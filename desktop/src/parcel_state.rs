//! Reconcile delivery receipts, not read flags, against the same parcel.
//! Only identifiers and event times are retained; no notification content or tokens.
use crate::{
    capsule_fulfillment::{Direction, Signal, Stage},
    home_hub,
    push::{ImThread, PushNotification},
    sms::Message,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const ACTIVE_CODE_WINDOW_MS: i64 = 7 * 86_400_000;
const COMPLETED_RETENTION_MS: i64 = 90 * 86_400_000;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompletionIndex {
    completed: BTreeMap<String, i64>,
    #[serde(default)]
    ambiguous_codes: BTreeMap<String, i64>,
    #[serde(default)]
    capsule_receipts: BTreeMap<String, (Stage, i64)>,
    #[serde(default)]
    outgoing_capsules: BTreeMap<String, i64>,
}

fn identifiers(text: &str) -> Vec<String> {
    let mut keys = Vec::new();
    if let Some(code) = home_hub::extract_pickup_code(text) {
        keys.push(format!("code:{}", code.trim().to_ascii_uppercase()));
    }
    if let Some((_, tracking)) = home_hub::extract_express_from_text_flexible(text, true) {
        keys.push(format!("parcel:{}", tracking.trim().to_ascii_uppercase()));
    }
    keys
}

impl CompletionIndex {
    pub fn record_capsule_signals(&mut self, signals: &[Signal], now: i64) {
        let cutoff = now.saturating_sub(72 * 3_600_000);
        self.capsule_receipts.retain(|_, (_, at)| *at >= cutoff);
        self.outgoing_capsules.retain(|_, at| *at >= cutoff);
        for signal in signals.iter().filter(|signal| signal.strong) {
            if signal.direction == Direction::Send {
                self.outgoing_capsules
                    .entry(signal.id.clone())
                    .and_modify(|at| *at = (*at).max(signal.at))
                    .or_insert(signal.at);
            }
            if signal.stage >= Stage::Arrived {
                self.capsule_receipts
                    .entry(signal.id.clone())
                    .and_modify(|old| *old = (*old).max((signal.stage, signal.at)))
                    .or_insert((signal.stage, signal.at));
                if let Some(prev_id) = &signal.prev_day_tail_id {
                    self.capsule_receipts
                        .entry(prev_id.clone())
                        .and_modify(|old| *old = (*old).max((signal.stage, signal.at)))
                        .or_insert((signal.stage, signal.at));
                }
            }
        }
    }

    pub fn retires_capsule(&self, signal: &Signal, now: i64) -> bool {
        let cutoff = now.saturating_sub(72 * 3_600_000);
        signal.strong
            && (self
                .outgoing_capsules
                .get(&signal.id)
                .is_some_and(|at| *at >= cutoff)
                || self
                    .capsule_receipts
                    .get(&signal.id)
                    .is_some_and(|(stage, at)| *at >= cutoff && *stage > signal.stage))
    }

    pub fn collect(
        sms: &[Message],
        notifications: &[PushNotification],
        threads: &[ImThread],
    ) -> Self {
        Self::reconcile(&Self::default(), sms, notifications, threads)
    }

    pub fn reconcile(
        previous: &Self,
        sms: &[Message],
        notifications: &[PushNotification],
        threads: &[ImThread],
    ) -> Self {
        let mut events = Vec::new();
        let mut add = |text: &str, at: i64| {
            if at <= 0 || home_hub::is_synthetic_e2e_probe_text(text) {
                return;
            }
            let keys = identifiers(text);
            if !keys.is_empty() {
                events.push((keys, at, home_hub::is_completed_express_text(text)));
            }
        };
        for message in sms.iter().filter(|m| !m.outgoing) {
            add(&message.body, message.date_ms);
        }
        // Read/dismissed receipts remain evidence. Group conversations are not logistics receipts.
        for notification in notifications.iter().filter(|n| !n.is_group) {
            add(
                &format!("{} {}", notification.title, notification.text),
                notification.post_time_ms,
            );
        }
        for thread in threads.iter().filter(|t| {
            !t.is_group
                && [
                    "快递",
                    "速递",
                    "速运",
                    "物流",
                    "菜鸟",
                    "丰巢",
                    "驿站",
                    "京东",
                    "淘宝",
                    "天猫",
                    "拼多多",
                ]
                .iter()
                .any(|cue| t.app_name.contains(cue) || t.title.contains(cue))
        }) {
            // A summary can be an outgoing reply; never treat that as a service receipt.
            if thread.messages.is_empty() {
                add(
                    &format!("{} {}", thread.title, thread.last_message),
                    thread.last_timestamp_ms,
                );
            }
            for message in thread
                .messages
                .iter()
                .filter(|m| !m.outgoing && !m.recalled)
            {
                // observed_at is the sync time, not the parcel event time.
                add(&message.text, message.timestamp_ms);
            }
        }
        let mut index = previous.clone();
        // Record explicit waybill completions first so already-completed waybills never
        // make a reused locker/shelf slot appear ambiguous for a newer parcel arrival.
        for (keys, at, terminal) in &events {
            if *terminal {
                for key in keys.iter().filter(|k| k.starts_with("parcel:")) {
                    index.record(key.clone(), *at);
                }
            }
        }

        // Process events chronologically (arrivals before terminal receipts at identical ms)
        // and track active uncompleted waybills per pickup code within a 7-day window.
        let mut sorted_events = events.clone();
        sorted_events.sort_by_key(|(_, at, terminal)| (*at, *terminal));

        let mut active_waybills_by_code: BTreeMap<String, BTreeMap<String, i64>> = BTreeMap::new();
        let prune_active_for_code =
            |code: &str,
             ref_at: i64,
             active_map: &mut BTreeMap<String, BTreeMap<String, i64>>,
             completed: &BTreeMap<String, i64>| {
                if let Some(waybills) = active_map.get_mut(code) {
                    waybills.retain(|parcel_key, arr_at| {
                        let within_window = ref_at.saturating_sub(*arr_at) <= ACTIVE_CODE_WINDOW_MS;
                        let completed_before_ref = completed
                            .get(parcel_key)
                            .is_some_and(|done| *done >= *arr_at && *done < ref_at);
                        within_window && !completed_before_ref
                    });
                }
            };

        for (keys, at, terminal) in &sorted_events {
            let codes: Vec<&String> = keys.iter().filter(|k| k.starts_with("code:")).collect();
            let parcels: Vec<&String> = keys.iter().filter(|k| k.starts_with("parcel:")).collect();
            if !*terminal {
                for code in codes {
                    prune_active_for_code(
                        code,
                        *at,
                        &mut active_waybills_by_code,
                        &index.completed,
                    );
                    if !parcels.is_empty() {
                        let entry = active_waybills_by_code.entry(code.clone()).or_default();
                        for parcel in &parcels {
                            entry
                                .entry((*parcel).clone())
                                .and_modify(|old| *old = (*old).max(*at))
                                .or_insert(*at);
                        }
                        if entry.len() > 1 {
                            index.ambiguous_codes.insert(code.clone(), *at);
                            index.completed.remove(code);
                        } else if index
                            .ambiguous_codes
                            .get(code)
                            .is_some_and(|amb_at| *amb_at < *at)
                        {
                            // Older waybills sharing `code` were completed or aged out; rebind cleanly.
                            index.ambiguous_codes.remove(code);
                        }
                    } else if active_waybills_by_code
                        .get(code)
                        .is_none_or(|m| m.is_empty())
                        && index
                            .ambiguous_codes
                            .get(code)
                            .is_some_and(|amb_at| *amb_at < *at)
                    {
                        index.ambiguous_codes.remove(code);
                    }
                }
            } else {
                for parcel in &parcels {
                    index.record((*parcel).clone(), *at);
                }
                for code in codes {
                    prune_active_for_code(
                        code,
                        *at,
                        &mut active_waybills_by_code,
                        &index.completed,
                    );
                    let active_len = active_waybills_by_code.get(code).map_or(0, |m| m.len());
                    if active_len <= 1
                        && index.ambiguous_codes.get(code).is_some_and(|amb_at| {
                            at.saturating_sub(*amb_at) > ACTIVE_CODE_WINDOW_MS
                        })
                    {
                        index.ambiguous_codes.remove(code);
                    }
                    index.record(code.clone(), *at);
                    if !index.ambiguous_codes.contains_key(code) {
                        if let Some(waybills) = active_waybills_by_code.get_mut(code) {
                            if waybills.len() == 1 {
                                if let Some((sole_parcel, arr_at)) =
                                    waybills.iter().next().map(|(k, v)| (k.clone(), *v))
                                {
                                    if arr_at <= *at {
                                        index.record(sole_parcel, *at);
                                        waybills.clear();
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }

        if let Some(max_at) = sorted_events.last().map(|(_, at, _)| *at) {
            index.ambiguous_codes.retain(|code, amb_at| {
                max_at.saturating_sub(*amb_at) <= ACTIVE_CODE_WINDOW_MS
                    || active_waybills_by_code.get(code).map_or(0, |m| m.len()) > 1
            });
        }

        // Shelf codes are not globally unique. An ambiguous code must not close
        // two distinct active waybills; an exact waybill receipt can still close its own.
        for code in index.ambiguous_codes.keys() {
            index.completed.remove(code);
        }
        // A pickup notice can bind a code to an actual waybill. A subsequent waybill
        // receipt (including one persisted across restart) closes the non-ambiguous code too.
        for (keys, at, terminal) in &events {
            if !terminal {
                let parcel_receipt = keys
                    .iter()
                    .filter(|k| k.starts_with("parcel:"))
                    .filter_map(|k| index.completed.get(k).copied())
                    .filter(|done| *done >= *at)
                    .max();
                if let Some(done) = parcel_receipt {
                    for code in keys.iter().filter(|k| k.starts_with("code:")) {
                        index.record(code.clone(), done);
                    }
                }
            }
        }
        index
    }

    fn record(&mut self, key: String, at: i64) {
        if self.ambiguous_codes.contains_key(&key) {
            return;
        }
        self.completed
            .entry(key)
            .and_modify(|old| *old = (*old).max(at))
            .or_insert(at);
    }

    pub fn merge(&mut self, other: &Self, now_ms: i64) {
        let amb_cutoff = now_ms.saturating_sub(ACTIVE_CODE_WINDOW_MS);
        for (code, at) in &other.ambiguous_codes {
            if *at >= amb_cutoff && !self.completed.get(code).is_some_and(|done| *done > *at) {
                self.ambiguous_codes
                    .entry(code.clone())
                    .and_modify(|old| *old = (*old).max(*at))
                    .or_insert(*at);
                self.completed.remove(code);
            }
        }
        for (key, at) in &other.completed {
            if key.starts_with("code:")
                && self
                    .ambiguous_codes
                    .get(key)
                    .is_some_and(|amb_at| *at > *amb_at && at.saturating_sub(*amb_at) > ACTIVE_CODE_WINDOW_MS)
            {
                self.ambiguous_codes.remove(key);
            }
            self.record(key.clone(), *at);
        }
        // Keep terminal evidence much longer than pending reminder windows, even
        // when the original phone notification has left its history window.
        self.completed
            .retain(|_, at| *at >= now_ms.saturating_sub(COMPLETED_RETENTION_MS));
        self.ambiguous_codes.retain(|_, at| *at >= amb_cutoff);
    }

    pub fn is_completed(&self, code: &str, tracking: &str, event_ms: i64) -> bool {
        [("code", code), ("parcel", tracking)]
            .iter()
            .any(|(prefix, value)| {
                !value.trim().is_empty()
                    && self
                        .completed
                        .get(&format!("{prefix}:{}", value.trim().to_ascii_uppercase()))
                        .is_some_and(|done| event_ms <= 0 || *done >= event_ms)
            })
    }

    pub fn text_is_completed(&self, text: &str, event_ms: i64) -> bool {
        identifiers(text).iter().any(|key| {
            self.completed
                .get(key)
                .is_some_and(|done| event_ms <= 0 || *done >= event_ms)
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn persisted_capsule_receipts_prevent_stale_progress_after_snapshot_removal() {
        use crate::capsule_fulfillment::{classify, Origin};
        let now = 1_790_000_000_000;
        let signal = |body, at| {
            classify(
                "com.yaya.zone",
                "叮咚买菜",
                "订单提醒",
                body,
                Origin::Notification("fixture".into()),
                at,
                false,
                now,
            )
            .unwrap()
        };
        let delivered = signal("订单号 10012345 已送达", now - 30_000);
        let replay = signal("订单号 10012345 正在配送", now);
        let another = signal("订单号 20012345 正在配送", now);
        let mut index = CompletionIndex::default();
        index.record_capsule_signals(&[delivered.clone()], now);
        let encoded = serde_json::to_string(&index).unwrap();
        let loaded: CompletionIndex = serde_json::from_str(&encoded).unwrap();
        assert!(!encoded.contains("10012345"));
        assert!(loaded.retires_capsule(&replay, now));
        assert!(!loaded.retires_capsule(&another, now));
        assert!(!loaded.retires_capsule(&delivered, now));
        assert!(!loaded.retires_capsule(&replay, now + 73 * 3_600_000));
    }
    fn sms(body: &str, at: i64) -> Message {
        Message {
            provider_id: None,
            address: "10690001".into(),
            body: body.into(),
            date_ms: at,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }
    }
    fn receipt(text: &str, at: i64) -> PushNotification {
        PushNotification {
            key: "receipt".into(),
            app_name: "丰巢".into(),
            text: text.into(),
            post_time_ms: at,
            read: true,
            dismissed: true,
            active: false,
            ..Default::default()
        }
    }

    #[test]
    fn parcel_receipt_closes_old_sms_even_when_receipt_read_and_dismissed() {
        let now = chrono::Utc::now().timestamp_millis();
        let pending = vec![sms("【丰巢】取件码31744725，请及时取件", now - 60_000)];
        let receipts = vec![receipt("【丰巢】取件码31744725的包裹已取出", now)];
        assert_eq!(
            home_hub::extract_live_express_parcels_with_threads(&pending, &[], &[], "", &[]).len(),
            1
        );
        assert!(home_hub::extract_live_express_parcels_with_threads(
            &pending,
            &receipts,
            &[],
            "",
            &[]
        )
        .is_empty());
        let index = CompletionIndex::collect(&pending, &receipts, &[]);
        assert!(!index.is_completed("31744726", "", now - 60_000));
        assert!(
            !index.is_completed("31744725", "", now + 60_000),
            "new pickup reusing a code is a new event"
        );
    }

    #[test]
    fn parcel_waybill_receipt_reconciles_code_and_survives_restart() {
        let now = chrono::Utc::now().timestamp_millis();
        let receipt = vec![receipt("顺丰快件SF1628890123456已签收", now)];
        let persisted =
            serde_json::to_string(&CompletionIndex::collect(&[], &receipt, &[])).unwrap();
        let restored = serde_json::from_str(&persisted).unwrap();
        let pending = vec![sms(
            "顺丰快件SF1628890123456，取件码31744725，请到丰巢取件",
            now - 60_000,
        )];
        let index = CompletionIndex::reconcile(&restored, &pending, &[], &[]);
        assert!(index.is_completed("31744725", "", now - 60_000));
        assert!(home_hub::extract_live_express_parcels_with_receipts(
            &pending,
            &[],
            &[],
            "",
            &index
        )
        .is_empty());
    }

    #[test]
    fn parcel_read_flags_and_ambiguous_status_are_not_completion() {
        let now = chrono::Utc::now().timestamp_millis();
        for body in [
            "快递员已取件，取件码31744725",
            "取件码31744725，如已取件请忽略",
            "取件码31744725，尚未取件",
            "包裹已由驿站代签收，取件码31744725",
            "驿站已签收SF1628890123456",
        ] {
            assert!(!home_hub::is_completed_express_text(body), "{body}");
        }
        let mut read = sms("丰巢取件码31744725，请及时取件", now);
        read.read = true;
        assert_eq!(
            home_hub::extract_live_express_parcels_with_threads(&[read], &[], &[], "", &[]).len(),
            1
        );
        let index = CompletionIndex::collect(&[], &[receipt("丰巢包裹已取出", now)], &[]);
        assert!(
            !index.is_completed("31744725", "", now),
            "no identifier cannot close another parcel"
        );
    }

    #[test]
    fn parcel_receipt_uses_service_history_not_sync_time_or_outgoing_chat() {
        let now = chrono::Utc::now().timestamp_millis();
        let message = crate::push::ImMessage {
            text: "取件码31744725包裹已取出".into(),
            timestamp_ms: now - 120_000,
            observed_at_ms: now,
            ..Default::default()
        };
        let mut thread = ImThread {
            title: "丰巢".into(),
            last_message: "广告".into(),
            messages: vec![message],
            ..Default::default()
        };
        let index = CompletionIndex::collect(&[], &[], &[thread.clone()]);
        assert!(index.is_completed("31744725", "", now - 180_000));
        assert!(!index.is_completed("31744725", "", now - 60_000));
        thread.messages[0].outgoing = true;
        assert!(!CompletionIndex::collect(&[], &[], &[thread]).is_completed("31744725", "", 0));
    }

    #[test]
    fn parcel_shared_shelf_code_does_not_close_another_waybill() {
        let now = chrono::Utc::now().timestamp_millis();
        let pending = vec![
            sms(
                "顺丰快件SF1628890123456，取件码18-2-2009，请及时取件",
                now - 60_000,
            ),
            sms(
                "中通包裹ZTO123456789012，取件码18-2-2009，请及时取件",
                now - 60_000,
            ),
        ];
        let done = vec![receipt("顺丰快件SF1628890123456已取出", now)];
        let index = CompletionIndex::collect(&pending, &done, &[]);
        assert!(index.is_completed("18-2-2009", "SF1628890123456", now - 60_000));
        assert!(!index.is_completed("18-2-2009", "ZTO123456789012", now - 60_000));
        let parcels =
            home_hub::extract_live_express_parcels_with_receipts(&pending, &done, &[], "", &index);
        assert_eq!(parcels.len(), 1);
        assert_eq!(parcels[0].tracking_no, "ZTO123456789012");
    }

    #[test]
    fn reused_shelf_slot_from_last_week_does_not_poison_code_only_completion_today() {
        let now = chrono::Utc::now().timestamp_millis();
        let last_week = now - 8 * 86_400_000;
        let today_arrival = now - 120_000;

        // Last week: parcel SF111 arrived on shelf 3-1-1002 and was picked up (even after colliding last week)
        let messages = vec![
            sms(
                "【菜鸟驿站】顺丰快件SF1628890000111已到站，凭取件码 3-1-1002 取件",
                last_week,
            ),
            sms(
                "【菜鸟驿站】圆通包裹YT2200112233445已到站，凭取件码 3-1-1002 取件",
                last_week + 10_000,
            ),
            sms(
                "【菜鸟驿站】顺丰快件SF1628890000111已取出",
                last_week + 300_000,
            ),
            sms(
                "【菜鸟驿站】圆通包裹YT2200112233445已取出",
                last_week + 360_000,
            ),
            // Today: new parcel ZTO arrives on reused shelf slot 3-1-1002
            sms(
                "【菜鸟驿站】中通包裹ZTO987654321098已到站，请凭取件码 3-1-1002 及时取件",
                today_arrival,
            ),
        ];

        // Before pickup today: today's ZTO parcel must be active (not falsely closed by last week)
        let before_pickup = CompletionIndex::collect(&messages, &[], &[]);
        assert!(!before_pickup.is_completed("3-1-1002", "ZTO987654321098", today_arrival));

        // Now user picks up today's parcel and receives a code-only completion SMS
        let mut after_messages = messages.clone();
        after_messages.push(sms("【菜鸟驿站】取件码 3-1-1002 已取件", now));

        let after_pickup = CompletionIndex::collect(&after_messages, &[], &[]);
        assert!(
            after_pickup.is_completed("3-1-1002", "ZTO987654321098", today_arrival),
            "Shelf slot 3-1-1002 picked up last week must not block code-only completion SMS today"
        );
        assert!(home_hub::extract_live_express_parcels_with_receipts(
            &after_messages,
            &[],
            &[],
            "",
            &after_pickup,
        )
        .is_empty());

        // Same-week sequential reuse (completed 2 days ago via code-only SMS, reused today) also works
        let two_days_ago = now - 2 * 86_400_000;
        let sequential_reuse = vec![
            sms(
                "【菜鸟驿站】韵达包裹YD1234567890123已到站，取件码 3-1-1002",
                two_days_ago,
            ),
            sms(
                "【菜鸟驿站】取件码 3-1-1002 已取件",
                two_days_ago + 1_800_000,
            ),
            sms(
                "【菜鸟驿站】极兔包裹JT7654321098765已到站，取件码 3-1-1002",
                today_arrival,
            ),
            sms("【菜鸟驿站】取件码 3-1-1002 已取件", now),
        ];
        let seq_index = CompletionIndex::collect(&sequential_reuse, &[], &[]);
        assert!(seq_index.is_completed("3-1-1002", "JT7654321098765", today_arrival));
    }
}
