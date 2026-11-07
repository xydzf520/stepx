//! Reconcile delivery receipts, not read flags, against the same parcel.
//! Only identifiers and event times are retained; no notification content or tokens.
use crate::{
    capsule_fulfillment::{Direction, Signal, Stage},
    home_hub,
    push::{ImThread, PushNotification},
    sms::Message,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

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
        let mut bindings: BTreeMap<String, (BTreeSet<String>, i64)> = BTreeMap::new();
        for (keys, at, _) in &events {
            if let Some(code) = keys.iter().find(|k| k.starts_with("code:")) {
                for tracking in keys.iter().filter(|k| k.starts_with("parcel:")) {
                    let binding = bindings.entry(code.clone()).or_default();
                    binding.0.insert(tracking.clone());
                    binding.1 = binding.1.max(*at);
                }
            }
        }
        for (code, (tracking, at)) in bindings {
            if tracking.len() > 1 {
                index.ambiguous_codes.insert(code, at);
            }
        }
        // Shelf codes are not globally unique. An ambiguous code must not close
        // two distinct waybills; an exact waybill receipt can still close its own.
        for code in index.ambiguous_codes.keys() {
            index.completed.remove(code);
        }
        for (keys, at, terminal) in &events {
            if *terminal {
                for key in keys {
                    index.record(key.clone(), *at);
                }
            }
        }
        // A pickup notice can bind a code to an actual waybill. A subsequent waybill
        // receipt then closes the code too; neither app name nor sender is an identity.
        for (keys, at, terminal) in &events {
            if !terminal {
                let receipt = keys
                    .iter()
                    .filter_map(|k| index.completed.get(k).copied())
                    .filter(|done| *done >= *at)
                    .max();
                if let Some(done) = receipt {
                    for key in keys {
                        index.record(key.clone(), done);
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
        for (code, at) in &other.ambiguous_codes {
            self.ambiguous_codes.insert(code.clone(), *at);
            self.completed.remove(code);
        }
        for (key, at) in &other.completed {
            self.record(key.clone(), *at);
        }
        // Keep terminal evidence much longer than pending reminder windows, even
        // when the original phone notification has left its history window.
        self.completed
            .retain(|_, at| *at >= now_ms.saturating_sub(90 * 86_400_000));
        self.ambiguous_codes
            .retain(|_, at| *at >= now_ms.saturating_sub(90 * 86_400_000));
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
}
