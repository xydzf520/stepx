//! The capsule consumes local fulfillment evidence and optional scoped Agent reviews.
use super::*;
use std::{
    collections::{hash_map::DefaultHasher, VecDeque},
    hash::{Hash, Hasher},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
};
use stepx_companion::{
    capsule_fulfillment as rules,
    home_reminder_governance::{ReminderCategory, ReminderTargetSpec},
    model_source::Source,
};

pub(crate) const LIVE_CAPSULE_PROXIMITY_MS: i64 = 60 * 60_000;
const MAX_FULFILLMENT_REVIEWS: usize = 128;

pub(crate) fn stable_phone_timer_capsule_id(
    timer: &stepx_companion::push::LiveCountdownTimer,
) -> String {
    if timer.is_pomodoro || timer.key == "desktop-pomodoro" {
        return "desktop-pomodoro".into();
    }
    let raw = timer.key.trim();
    if !raw.is_empty() {
        let sanitized: String = raw
            .chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | ':') {
                    c
                } else {
                    '_'
                }
            })
            .collect();
        format!("phone-timer-{sanitized}")
    } else {
        format!(
            "phone-timer-{}",
            rules::digest(&[&timer.package_name, &timer.app_name, &timer.title])
        )
    }
}

fn signals_share_fulfillment_identity(a: &rules::Signal, b: &rules::Signal) -> bool {
    if a.id == b.id {
        return true;
    }
    if let (Some(ta), Some(tb)) = (
        a.tracking.as_deref().map(str::trim).filter(|s| !s.is_empty()),
        b.tracking.as_deref().map(str::trim).filter(|s| !s.is_empty()),
    ) {
        if ta.eq_ignore_ascii_case(tb) {
            return true;
        }
    }
    if a.kind == b.kind && a.provider == b.provider {
        if let (Some(sa), Some(sb)) = (
            a.order_suffix
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
            b.order_suffix
                .as_deref()
                .map(str::trim)
                .filter(|s| !s.is_empty()),
        ) {
            return sa.eq_ignore_ascii_case(sb);
        }
        if a.tracking.is_none()
            && b.tracking.is_none()
            && a.order_suffix.is_none()
            && b.order_suffix.is_none()
            && (a.at - b.at).abs() <= 45 * 60_000
        {
            return true;
        }
    }
    false
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scope {
    device: String,
    account: Option<String>,
    source: Source,
    prompt: String,
}

struct ReviewJob {
    rx: mpsc::Receiver<(HashMap<String, bool>, bool)>,
    cancel: Arc<AtomicBool>,
}

impl Drop for ReviewJob {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(super) struct FulfillmentUi {
    scope: Option<Scope>,
    job: Option<ReviewJob>,
    reviews: VecDeque<(String, bool)>,
    next_poll: Option<Instant>,
    next_expiry: Cell<Option<i64>>,
    cached_signals: RefCell<Option<(u64, Vec<rules::Signal>)>>,
    pub status: String,
}

impl Companion {
    fn fulfillment_signals_cache_key(&self, now: i64) -> u64 {
        let mut h = DefaultHasher::new();
        self.selected
            .as_ref()
            .map(|d| d.serial.as_str())
            .hash(&mut h);
        self.last_connected_serial.as_deref().hash(&mut h);
        (now / 15_000).hash(&mut h);
        self.sms.len().hash(&mut h);
        for msg in self.sms.iter().take(96) {
            msg.outgoing.hash(&mut h);
            msg.date_ms.hash(&mut h);
            msg.address.hash(&mut h);
            msg.body.hash(&mut h);
        }
        self.push.snapshot.notifications.len().hash(&mut h);
        for notif in self.push.snapshot.notifications.iter().take(96) {
            notif.key.hash(&mut h);
            notif.is_group.hash(&mut h);
            notif.post_time_ms.hash(&mut h);
            notif.read.hash(&mut h);
            notif.dismissed.hash(&mut h);
            notif.package_name.hash(&mut h);
            notif.app_name.hash(&mut h);
            notif.title.hash(&mut h);
            notif.text.hash(&mut h);
            notif.sub_text.hash(&mut h);
            self.notification_center
                .read_ids
                .contains(&format!("push-notif:{}", notif.key))
                .hash(&mut h);
        }
        self.push.snapshot.threads.len().hash(&mut h);
        for thread in self.push.snapshot.threads.iter().take(48) {
            thread.thread_id.hash(&mut h);
            thread.is_group.hash(&mut h);
            thread.last_timestamp_ms.hash(&mut h);
            thread.unread_count.hash(&mut h);
            thread.package_name.hash(&mut h);
            thread.app_name.hash(&mut h);
            thread.title.hash(&mut h);
            thread.last_message.hash(&mut h);
            thread.messages.last().map(|m| m.outgoing).hash(&mut h);
            self.notification_center
                .read_ids
                .contains(&format!("im-thread:{}", thread.thread_id))
                .hash(&mut h);
        }
        h.finish()
    }

    pub(super) fn fulfillment_signals(&self, now: i64) -> Vec<rules::Signal> {
        let cache_key = self.fulfillment_signals_cache_key(now);
        if let Some((cached_key, cached)) = self.fulfillment.cached_signals.borrow().as_ref() {
            if *cached_key == cache_key {
                return cached.clone();
            }
        }
        let mut signals = Vec::new();
        for msg in self.sms.iter().take(96).filter(|m| !m.outgoing) {
            signals.extend(rules::classify(
                "",
                "",
                &msg.address,
                &msg.body,
                rules::Origin::Sms(msg.address.clone()),
                msg.date_ms,
                // SMS read state is updated for the whole sender. The capsule
                // acknowledges the exact order when its action opens the SMS thread.
                false,
                now,
            ));
        }
        for notif in self
            .push
            .snapshot
            .notifications
            .iter()
            .take(96)
            .filter(|n| !n.key.is_empty() && !n.is_group)
        {
            let body = if notif.sub_text.trim().is_empty() || notif.text.contains(&notif.sub_text) {
                notif.text.clone()
            } else {
                format!("{} {}", notif.text, notif.sub_text)
            };
            signals.extend(rules::classify(
                &notif.package_name,
                &notif.app_name,
                &notif.title,
                &body,
                rules::Origin::Notification(notif.key.clone()),
                notif.post_time_ms,
                notif.read
                    || notif.dismissed
                    || self
                        .notification_center
                        .read_ids
                        .contains(&format!("push-notif:{}", notif.key)),
                now,
            ));
        }
        for thread in self.push.snapshot.threads.iter().take(48).filter(|t| {
            !t.is_group && !t.thread_id.is_empty() && !t.messages.last().is_some_and(|m| m.outgoing)
        }) {
            signals.extend(rules::classify(
                &thread.package_name,
                &thread.app_name,
                &thread.title,
                &thread.last_message,
                rules::Origin::Thread(thread.thread_id.clone()),
                thread.last_timestamp_ms,
                thread.unread_count == 0
                    || self
                        .notification_center
                        .read_ids
                        .contains(&format!("im-thread:{}", thread.thread_id)),
                now,
            ));
        }
        let reconciled = rules::reconcile(signals);
        *self.fulfillment.cached_signals.borrow_mut() = Some((cache_key, reconciled.clone()));
        reconciled
    }

    fn fulfillment_key(&self, signal: &rules::Signal) -> String {
        let serial = self
            .selected
            .as_ref()
            .map(|d| d.serial.as_str())
            .or(self.last_connected_serial.as_deref())
            .unwrap_or("");
        format!(
            "capsule:fulfillment-{}",
            rules::digest(&[serial, &signal.id])
        )
    }

    fn fulfillment_ttl(&self, signal: &rules::Signal) -> i64 {
        let category = if signal.kind == rules::Kind::Parcel {
            ReminderCategory::ExpressInTransit
        } else {
            ReminderCategory::TakeoutAndMobility
        };
        self.reminder_governance.effective_ttl_ms(category)
    }

    pub(super) fn fulfillment_correlated_keys(&self, signal: &rules::Signal) -> Vec<String> {
        let mut keys = vec![
            self.fulfillment_key(signal),
            format!("fulfillment:{}", signal.id),
        ];
        let push_unique = |list: &mut Vec<String>, k: String| {
            if !k.is_empty() && !list.contains(&k) {
                list.push(k);
            }
        };
        let (sms_addr, notif_key, thread_id) = match &signal.origin {
            rules::Origin::Sms(address) => (Some(address.as_str()), None, None),
            rules::Origin::Notification(key) => {
                let linked_tid = self
                    .push
                    .snapshot
                    .notifications
                    .iter()
                    .find(|n| n.key == *key)
                    .map(|n| n.thread_id.as_str())
                    .filter(|s| !s.trim().is_empty());
                (None, Some(key.as_str()), linked_tid)
            }
            rules::Origin::Thread(id) => (None, None, Some(id.as_str())),
        };
        let pickup_code = stepx_companion::home_hub::extract_pickup_code(&signal.detail);
        for alias in stepx_companion::home_reminder_governance::canonical_keys_for_delivery(
            &signal.provider,
            notif_key,
            thread_id,
            sms_addr,
        ) {
            push_unique(&mut keys, alias);
        }
        for alias in stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
            pickup_code.as_deref(),
            signal.tracking.as_deref(),
            notif_key,
            sms_addr,
        ) {
            push_unique(&mut keys, alias);
        }
        if let Some(tracking) = signal
            .tracking
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            push_unique(&mut keys, format!("parcel:{}", tracking.to_uppercase()));
        }
        if let Some(suffix) = signal
            .order_suffix
            .as_deref()
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            push_unique(
                &mut keys,
                format!("order:{}:{}", signal.provider.trim(), suffix.to_uppercase()),
            );
        }
        keys
    }

    pub(crate) fn is_capsule_within_proximity(
        &self,
        cap: &super::shell::TopBarLiveCapsuleItem,
        now_ms: i64,
    ) -> bool {
        if let Some(rest) = cap.id.strip_prefix("cal-") {
            if !cap.id.starts_with("cal-notif-") {
                let (id_part, start_part) = match rest.split_once(':') {
                    Some((i, s)) => (i, s.parse::<i64>().ok()),
                    None => (rest, None),
                };
                if let Ok(ev_id) = id_part.parse::<i64>() {
                    if let Some(ev) = self
                        .back_screen
                        .stepos_suite
                        .calendar_events
                        .iter()
                        .find(|e| e.id == ev_id && start_part.is_none_or(|ts| e.dtstart == ts))
                    {
                        if crate::ui::pages::home::calendar_event_is_ended_at(ev, now_ms) {
                            return false;
                        }
                        if ev.all_day {
                            return ev.dtstart > 0
                                && (ev.dtstart - now_ms).abs() <= LIVE_CAPSULE_PROXIMITY_MS;
                        }
                        if ev.dtstart > now_ms {
                            return ev.dtstart - now_ms <= LIVE_CAPSULE_PROXIMITY_MS;
                        }
                        return true;
                    }
                }
            }
        }
        if let Some(rest) = cap.id.strip_prefix("alarm-") {
            if let Ok(alarm_id) = rest.parse::<i64>() {
                if let Some(alarm) = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter()
                    .find(|a| a.id == alarm_id)
                {
                    let now_local = chrono::TimeZone::timestamp_millis_opt(&chrono::Local, now_ms)
                        .single()
                        .unwrap_or_else(chrono::Local::now);
                    let now_min = chrono::Timelike::hour(&now_local) * 60
                        + chrono::Timelike::minute(&now_local);
                    let weekday_idx =
                        chrono::Datelike::weekday(&now_local).num_days_from_monday();
                    let status = crate::ui::pages::home::resolve_alarm_status(
                        alarm,
                        now_ms,
                        now_min,
                        weekday_idx,
                        &self.reminder_governance,
                    );
                    return status.next_offset_mins().is_some_and(|mins| {
                        (mins as i64).saturating_mul(60_000) <= LIVE_CAPSULE_PROXIMITY_MS
                    });
                }
            }
        }
        true
    }

    pub(super) fn fulfillment_visible(&self, signal: &rules::Signal, now: i64) -> bool {
        if !signal.current(now, self.fulfillment_ttl(signal))
            || self.push.parcel_receipts.retires_capsule(signal, now)
            || signal.tracking.as_ref().is_some_and(|tracking| {
                self.push
                    .parcel_receipts
                    .is_completed("", tracking, signal.at)
            })
            || (signal.kind == rules::Kind::Parcel
                && signal.stage == rules::Stage::Ready
                && stepx_companion::home_hub::extract_pickup_code(&signal.detail).is_some())
        {
            return false;
        }
        if let rules::Origin::Notification(key) = &signal.origin {
            if let Some(notif) = self
                .push
                .snapshot
                .notifications
                .iter()
                .find(|n| n.key == *key)
            {
                if self.push.prefs.effective_popup_mode(&notif.package_name)
                    == stepx_companion::desktop_notify::PopupMode::Silent
                    || self
                        .reminder_governance
                        .muted_sources
                        .contains_key(&format!("pkg:{}", notif.package_name))
                {
                    return false;
                }
            }
        }
        let provider_key = format!("delivery:{}", signal.provider);
        let keys = self.fulfillment_correlated_keys(signal);
        !self.reminder_governance.is_any_key_suppressed(
            &keys,
            Some(signal.at),
            None,
            Some(&provider_key),
            now,
        )
    }

    pub(super) fn fulfillment_capsules(
        &self,
        now: i64,
    ) -> Vec<super::shell::TopBarLiveCapsuleItem> {
        use super::shell::{CapsuleTrack, TopBarCapsuleKind, TopBarLiveCapsuleItem};
        use rules::{Direction, Kind, Stage};
        let mut next_expiry = None;
        let items = self
            .fulfillment_signals(now)
            .into_iter()
            .filter(|signal| {
                let inferred = self.settings.value.capsule_agent_enabled
                    && self.fulfillment.scope.as_ref().is_some_and(|scope| {
                        self.selected
                            .as_ref()
                            .is_some_and(|d| d.serial == scope.device)
                            && scope.account == self.amoo_ui.account_scope
                            && scope.prompt == self.settings.value.capsule_agent_prompt
                    })
                    && signal.agent_candidate()
                    && self
                        .fulfillment
                        .reviews
                        .iter()
                        .any(|(k, v)| *v && *k == signal.agent_key());
                (signal.direction == Direction::Receive || inferred)
                    && self.fulfillment_visible(signal, now)
            })
            .take(5)
            .map(|signal| {
                let ttl = self.fulfillment_ttl(&signal);
                let expire = signal.at.saturating_add(if signal.stage == Stage::Arrived {
                    ttl.min(rules::ARRIVAL_TTL_MS)
                } else {
                    ttl
                });
                next_expiry = Some(next_expiry.map_or(expire, |old: i64| old.min(expire)));
                let (icon, kind) = match signal.kind {
                    Kind::Parcel => (icons::UiIcon::Package, TopBarCapsuleKind::PickupOrOtp),
                    Kind::Instant => (icons::UiIcon::Coffee, TopBarCapsuleKind::InstantDelivery),
                    Kind::Travel => (icons::UiIcon::Pin, TopBarCapsuleKind::TravelMobility),
                };
                let arrived = signal.stage == Stage::Arrived;
                let important = arrived || signal.stage == Stage::Ready;
                let (pill, metric) = if signal.kind == Kind::Parcel
                    && signal.dispatching
                    && signal.stage == Stage::Moving
                {
                    ("派件中".into(), "派件中".into())
                } else if signal.kind == Kind::Parcel
                    && signal.stage == Stage::Moving
                    && ["已发货", "揽收发出"]
                        .iter()
                        .any(|cue| signal.detail.contains(cue))
                {
                    ("已发货".into(), "已发货".into())
                } else if signal.kind == Kind::Instant && signal.stage <= Stage::Moving {
                    stepx_companion::home_hub::extract_eta_window_from_text(
                        &signal.detail,
                        signal.at,
                        now,
                    )
                    .filter(|(end, _, _)| *end > now)
                    .map(|(end, range, short)| {
                        next_expiry = Some(next_expiry.map_or(end, |old| old.min(end)));
                        (short, format!("预计 {range} 送达"))
                    })
                    .unwrap_or_else(|| {
                        let label = signal.stage.label(signal.kind).to_owned();
                        (label.clone(), label)
                    })
                } else {
                    let label = signal.stage.label(signal.kind).to_owned();
                    (label.clone(), label)
                };
                let id = self
                    .fulfillment_key(&signal)
                    .trim_start_matches("capsule:")
                    .to_owned();
                let (sms_address, im_thread_id, notif_key) = match &signal.origin {
                    rules::Origin::Sms(addr) => (Some(addr.clone()), None, None),
                    rules::Origin::Thread(id) => (None, Some(id.clone()), None),
                    rules::Origin::Notification(key) => (None, None, Some(key.clone())),
                };
                let detail = if arrived {
                    "查看后收起".into()
                } else {
                    Self::truncate_chars(&signal.detail, 32)
                };
                let subtitle = signal.order_suffix.as_ref().map_or_else(
                    || detail.clone(),
                    |suffix| format!("尾号 {suffix} · {detail}"),
                );
                TopBarLiveCapsuleItem {
                    id: id.clone(),
                    tag: if signal.kind == Kind::Parcel {
                        "stepx_pickup".into()
                    } else {
                        "stepx_delivery".into()
                    },
                    icon,
                    kind,
                    track: if important {
                        CapsuleTrack::ImportantMessage
                    } else {
                        CapsuleTrack::LiveActivity
                    },
                    domain_badge: signal.provider.clone(),
                    strategy_tag: if signal.direction == Direction::Receive {
                        "本地规则".into()
                    } else {
                        "规则＋Agent 复核".into()
                    },
                    score: if signal.stage == Stage::Ready {
                        98
                    } else if arrived {
                        96
                    } else if signal.kind == Kind::Parcel {
                        88
                    } else {
                        95
                    },
                    pill_text: pill.clone(),
                    title: signal.title.clone(),
                    subtitle,
                    metric,
                    source_tag: if important { "提醒" } else { "实况" },
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: pill,
                    primary_label: if arrived {
                        "查看送达"
                    } else {
                        "查看详情"
                    },
                    primary_action: Action::BackScreen(BackScreenAction::OpenFulfillment { id }),
                    copy_value: None,
                    complete_label: if arrived { "已知晓" } else { "收起" },
                    pickup_code: None,
                    sms_address,
                    im_thread_id,
                    notif_key,
                }
            })
            .collect();
        self.fulfillment.next_expiry.set(next_expiry);
        items
    }

    fn find_fulfillment(&self, id: &str, now: i64) -> Option<rules::Signal> {
        // Re-resolve the painted identity after refresh. List slots can be reused.
        self.fulfillment_signals(now).into_iter().find(|signal| {
            self.fulfillment_key(signal).strip_prefix("capsule:") == Some(id)
                && self.fulfillment_visible(signal, now)
        })
    }

    fn acknowledge_fulfillment(&mut self, signal: &rules::Signal, now: i64) {
        let category = if signal.kind == rules::Kind::Parcel {
            ReminderCategory::ExpressInTransit
        } else {
            ReminderCategory::TakeoutAndMobility
        };
        let mut aliases = self.fulfillment_correlated_keys(signal);
        let mut max_source_ts = signal.at;
        for other in self.fulfillment_signals(now) {
            if signals_share_fulfillment_identity(signal, &other) {
                max_source_ts = max_source_ts.max(other.at);
                for alias in self.fulfillment_correlated_keys(&other) {
                    if !aliases.contains(&alias) {
                        aliases.push(alias);
                    }
                }
            }
        }
        for alias in &aliases {
            if let Some(cap_id) = alias.strip_prefix("capsule:") {
                self.back_screen
                    .dismissed_capsule_ids
                    .insert(cap_id.to_string());
            }
        }
        self.reminder_governance.manual_expire(
            ReminderTargetSpec::new(self.fulfillment_key(signal), category, signal.title.clone())
                .with_aliases(aliases)
                .with_source(&signal.provider, None)
                .with_source_ts(now.max(max_source_ts)),
            now,
        );
        self.status = "已收起这条胶囊".into();
        self.error = false;
    }

    pub(super) fn acknowledge_fulfillment_id(&mut self, id: &str) {
        let now = chrono::Local::now().timestamp_millis();
        if let Some(signal) = self.find_fulfillment(id, now) {
            self.acknowledge_fulfillment(&signal, now);
        }
    }

    pub(super) fn open_fulfillment_capsule(&mut self, id: &str, ctx: &mut ViewContext<Self>) {
        use facloud_ui::TypedActionView;
        let now = chrono::Local::now().timestamp_millis();
        let Some(signal) = self.find_fulfillment(id, now) else {
            return;
        };
        self.back_screen.live_capsule_popover_open = false;
        match signal.origin.clone() {
            rules::Origin::Notification(key) => {
                self.handle_action(&Action::Page(Page::Notifications), ctx);
                if self.page != Page::Notifications {
                    return;
                }
                self.fields[F_PUSH_SEARCH].clear();
                self.handle_action(&Action::Push(PushAction::ResetNotificationFilters), ctx);
                self.handle_action(&Action::Push(PushAction::SelectNotification(key)), ctx);
            }
            rules::Origin::Sms(address) => self.handle_action(
                &Action::BackScreen(BackScreenAction::OpenCapsuleTarget {
                    sms_address: Some(address),
                    im_thread_id: None,
                    open_calendar: false,
                    open_alarms: false,
                }),
                ctx,
            ),
            rules::Origin::Thread(id) => self.handle_action(
                &Action::BackScreen(BackScreenAction::OpenCapsuleTarget {
                    sms_address: None,
                    im_thread_id: Some(id),
                    open_calendar: false,
                    open_alarms: false,
                }),
                ctx,
            ),
        }
        if signal.stage == rules::Stage::Arrived {
            self.acknowledge_fulfillment(&signal, now);
        }
    }

    pub(super) fn tick_capsule_fulfillment(&mut self, ctx: &mut ViewContext<Self>) {
        if self
            .fulfillment
            .next_expiry
            .get()
            .is_some_and(|expiry| chrono::Local::now().timestamp_millis() >= expiry)
        {
            self.fulfillment.next_expiry.set(None);
            ctx.notify();
        }
        if self.is_demo_device()
            || !self.settings.value.capsule_agent_enabled
            || self.selected.is_none()
        {
            if self.fulfillment.scope.is_some() {
                self.fulfillment = FulfillmentUi::default();
                ctx.notify();
            }
            return;
        }
        if self
            .fulfillment
            .next_poll
            .is_some_and(|next| Instant::now() < next)
        {
            return;
        }
        self.fulfillment.next_poll = Some(Instant::now() + Duration::from_secs(3));
        let Some(device) = self.selected.clone() else {
            return;
        };
        let Ok(prefs) = stepx_companion::model_source::load() else {
            self.fulfillment.scope = None;
            self.fulfillment.job = None;
            self.fulfillment.status = "模型来源配置无效，当前由规则判断".into();
            ctx.notify();
            return;
        };
        let account = self.amoo_ui.account_scope.clone();
        let scope = Scope {
            device: device.serial.clone(),
            account,
            source: prefs.source,
            prompt: self.settings.value.capsule_agent_prompt.clone(),
        };
        if self.fulfillment.scope.as_ref() != Some(&scope) {
            self.fulfillment.scope = Some(scope.clone());
            self.fulfillment.job = None;
            self.fulfillment.reviews.clear();
            self.fulfillment.status.clear();
        }
        if let Some(job) = &self.fulfillment.job {
            match job.rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => return,
                result => {
                    self.fulfillment.job = None;
                    self.fulfillment.status = match result {
                        Ok((reviews, verified)) => {
                            self.fulfillment.reviews.extend(reviews);
                            while self.fulfillment.reviews.len() > MAX_FULFILLMENT_REVIEWS {
                                self.fulfillment.reviews.pop_front();
                            }
                            if verified {
                                "最近一批已复核".into()
                            } else {
                                "本次复核未取得有效结果，当前由规则判断".into()
                            }
                        }
                        _ => "本次复核未取得有效结果，当前由规则判断".into(),
                    };
                    self.fulfillment.next_poll = Some(Instant::now() + Duration::from_secs(30));
                    ctx.notify();
                    return;
                }
            }
        }
        let capabilities = stepx_companion::home_ai::source_capabilities_for(
            scope.source,
            &scope.device,
            scope.account.as_deref(),
        );
        if !capabilities.automatic_available {
            let status = format!("规则运行中 · {}暂不可后台复核", scope.source.label());
            if self.fulfillment.status != status {
                self.fulfillment.status = status;
                ctx.notify();
            }
            return;
        }
        let now = chrono::Local::now().timestamp_millis();
        let mut seen = std::collections::HashSet::new();
        let batch: Vec<_> = self
            .fulfillment_signals(now)
            .into_iter()
            .filter(|s| s.agent_candidate() && self.fulfillment_visible(s, now))
            .filter(|s| {
                let key = s.agent_key();
                !self.fulfillment.reviews.iter().any(|(k, _)| *k == key) && seen.insert(key)
            })
            .take(6)
            .collect();
        if batch.is_empty() {
            return;
        }
        let (tx, rx) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.fulfillment.job = Some(ReviewJob {
            rx,
            cancel: cancel.clone(),
        });
        self.fulfillment.status = format!("正在用{}复核物流方向", scope.source.label());
        ctx.notify();
        std::thread::spawn(move || {
            let inputs: Vec<_> = batch.iter().collect();
            let prompt = rules::agent_request(&scope.prompt, &inputs);
            let run = || -> anyhow::Result<_> {
                anyhow::ensure!(
                    stepx_companion::model_source::load()?.source == scope.source,
                    "模型来源已变化",
                );
                let response = match scope.source {
                    Source::IndependentApi => stepx_companion::home_ai::generate_local_json(&prompt, &cancel)?,
                    Source::PhoneAccount => stepx_companion::home_ai::generate_selected_json_automatic_recorded_for_account(
                        &Adb::discover()?, &device, scope.source, scope.account.as_deref(),
                        &prompt, &cancel, &mut |_| Ok(()),
                    )?.brief,
                };
                rules::validate_agent_response(&response.markdown, &inputs)
                    .ok_or_else(|| anyhow::anyhow!("复核响应没有通过证据校验"))
            };
            // An invalid or failed response must not cause recurring requests
            // for the same evidence. The next distinct snapshot can be reviewed.
            let reviews = match run() {
                Ok(reviews) => (reviews, true),
                Err(_) => (
                    batch.iter().map(|s| (s.agent_key(), false)).collect(),
                    false,
                ),
            };
            let _ = tx.send(reviews);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stable_phone_timer_capsule_id_ignores_ticking_countdown_seconds() {
        let mut timer_a = stepx_companion::push::LiveCountdownTimer {
            key: "0|com.android.deskclock|77|timer|101".into(),
            package_name: "com.android.deskclock".into(),
            app_name: "时钟".into(),
            title: "手冲咖啡焖蒸".into(),
            total_secs: 180,
            remaining_secs: 142,
            remaining_label: "还剩 2 分 22 秒".into(),
            mm_ss_label: "02:22".into(),
            progress_pct: 21,
            is_pomodoro: false,
        };
        let id_initial = stable_phone_timer_capsule_id(&timer_a);
        assert!(id_initial.starts_with("phone-timer-"));
        assert!(!id_initial.contains("02:22") && !id_initial.contains("142"));

        timer_a.remaining_secs = 59;
        timer_a.remaining_label = "还剩 59 秒".into();
        timer_a.mm_ss_label = "00:59".into();
        timer_a.progress_pct = 67;
        assert_eq!(
            stable_phone_timer_capsule_id(&timer_a),
            id_initial,
            "ticking countdown text/seconds must never mutate timer capsule ID"
        );
    }

    #[test]
    fn acknowledge_fulfillment_propagates_aliases_across_secondary_keys() {
        let mut app = Companion::new_for_test();
        let now = 1_762_000_000_000i64;
        app.push.snapshot.notifications = vec![stepx_companion::push::PushNotification {
            key: "notif-meituan-8821".into(),
            package_name: "com.sankuai.meituan.takeoutnew".into(),
            app_name: "美团外卖".into(),
            title: "美团外卖".into(),
            text: "您的美团外卖订单尾号 8821 骑手正在配送中，预计 12:30~12:45 送达".into(),
            post_time_ms: now - 120_000,
            active: true,
            ..Default::default()
        }];
        app.sms = vec![stepx_companion::sms::Message {
            provider_id: Some(1),
            address: "10690123".into(),
            body: "【美团外卖】您的订单尾号8821骑手已接单正在配送中".into(),
            date_ms: now - 100_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];
        let caps_before = app.fulfillment_capsules(now);
        assert_eq!(caps_before.len(), 1);
        let cap_id = caps_before[0].id.clone();

        let signal = app.find_fulfillment(&cap_id, now).expect("active fulfillment");
        app.acknowledge_fulfillment(&signal, now);

        // Even if primary notification disappears and only the secondary SMS remains,
        // correlated order/provider/SMS aliases keep it suppressed.
        app.push.snapshot.notifications.clear();
        let caps_after = app.fulfillment_capsules(now);
        assert!(
            caps_after.is_empty(),
            "acknowledged fulfillment must not re-surface under secondary SMS/order key: {caps_after:?}"
        );
        assert!(app.reminder_governance.is_any_key_suppressed(
            &["order:美团外卖:8821".to_string()],
            Some(now - 90_000),
            None,
            None,
            now,
        ));
        assert!(app.reminder_governance.is_any_key_suppressed(
            &[format!("fulfillment:{}", signal.id)],
            Some(now - 90_000),
            None,
            None,
            now,
        ));
        assert!(
            !app.reminder_governance.is_any_key_suppressed(
                &["delivery:美团外卖".to_string()],
                Some(now - 90_000),
                None,
                None,
                now,
            ),
            "item-level acknowledgment must not poison channel-wide delivery:美团外卖 alias"
        );
        // Unrelated concurrent order on the same channel must remain visible.
        app.sms.push(stepx_companion::sms::Message {
            provider_id: Some(2),
            address: "10690123".into(),
            body: "【美团外卖】您的订单尾号8822骑手已接单正在配送中".into(),
            date_ms: now - 95_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        let caps_other_order = app.fulfillment_capsules(now);
        assert_eq!(caps_other_order.len(), 1);
    }

    #[test]
    fn capsule_proximity_window_enforces_60_minutes_for_calendar_and_alarms() {
        let mut app = Companion::new_for_test();
        let now_local = chrono::Local::now();
        let now_ms = now_local.timestamp_millis();

        app.back_screen.stepos_suite.calendar_events = vec![
            stepx_companion::toolbox::PhoneCalendarEvent {
                id: 901,
                title: "6小时后的远期评审".into(),
                dtstart: now_ms + 6 * 3_600_000,
                dtend: now_ms + 7 * 3_600_000,
                all_day: false,
                ..Default::default()
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
                id: 902,
                title: "35分钟后的临近会议".into(),
                dtstart: now_ms + 35 * 60_000,
                dtend: now_ms + 95 * 60_000,
                all_day: false,
                ..Default::default()
            },
            stepx_companion::toolbox::PhoneCalendarEvent {
                id: 903,
                title: "正在进行中的会议".into(),
                dtstart: now_ms - 15 * 60_000,
                dtend: now_ms + 45 * 60_000,
                all_day: false,
                ..Default::default()
            },
        ];

        let mk_cap = |id: &str| super::super::shell::TopBarLiveCapsuleItem {
            id: id.into(),
            tag: "stepx_schedule".into(),
            icon: icons::UiIcon::Calendar,
            kind: super::super::shell::TopBarCapsuleKind::ScheduleOrAlarm,
            track: super::super::shell::CapsuleTrack::ImportantMessage,
            domain_badge: "今日日程".into(),
            strategy_tag: String::new(),
            score: 90,
            pill_text: "10:00".into(),
            title: id.into(),
            subtitle: String::new(),
            metric: "10:00".into(),
            source_tag: "重要消息",
            progress_pct: 0,
            countdown_secs: None,
            short_sig: "日程".into(),
            primary_label: "查看",
            primary_action: Action::Page(Page::Calendar),
            copy_value: None,
            complete_label: "已知晓",
            pickup_code: None,
            sms_address: None,
            im_thread_id: None,
            notif_key: None,
        };

        assert!(
            !app.is_capsule_within_proximity(&mk_cap("cal-901"), now_ms),
            "meeting 6 hours away must not hog the live capsule"
        );
        assert!(
            app.is_capsule_within_proximity(&mk_cap("cal-902"), now_ms),
            "meeting within 60 minutes must be promoted to live capsule"
        );
        assert!(
            app.is_capsule_within_proximity(&mk_cap("cal-903"), now_ms),
            "ongoing meeting must stay in live capsule"
        );
    }
}
