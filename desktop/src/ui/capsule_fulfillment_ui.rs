//! The capsule consumes local fulfillment evidence and optional scoped Agent reviews.
use super::*;
use std::{
    collections::VecDeque,
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
    pub status: String,
}

impl Companion {
    pub(super) fn fulfillment_signals(&self, now: i64) -> Vec<rules::Signal> {
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
        rules::reconcile(signals)
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
        let key = self.fulfillment_key(signal);
        let provider_key = format!("delivery:{}", signal.provider);
        let mut keys = vec![key, provider_key.clone()];
        match &signal.origin {
            rules::Origin::Sms(address) => keys.push(format!(
                "sms:{}",
                stepx_companion::sms::normalize_address(address)
            )),
            rules::Origin::Notification(key) => {
                keys.push(format!("notif:{key}"));
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
            rules::Origin::Thread(id) => keys.push(format!("im:{id}")),
        }
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
        self.reminder_governance.manual_expire(
            ReminderTargetSpec::new(self.fulfillment_key(signal), category, signal.title.clone())
                .with_source(&signal.provider, None)
                .with_source_ts(signal.at),
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
                            while self.fulfillment.reviews.len() > 192 {
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
