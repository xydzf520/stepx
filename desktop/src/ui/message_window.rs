//! Keep native layout bounded without discarding history or moving the reader on sync.
use std::ops::Range;
use stepx_companion::push::{ImMessage, ImThread};

pub(super) const PAGE_SIZE: usize = 80;

pub(super) fn message_key(m: &ImMessage) -> String {
    if !m.id.is_empty() {
        return m.id.clone();
    }
    serde_json::to_string(&(m.timestamp_ms, &m.sender, m.outgoing, &m.text)).unwrap_or_default()
}

pub(super) fn has_key(m: &ImMessage, key: &str) -> bool {
    if m.id.is_empty() {
        message_key(m) == key
    } else {
        m.id == key
    }
}

#[derive(Default)]
pub(super) struct MessageWindow {
    thread: String,
    first: Option<String>,
    browsing_history: bool,
}

impl MessageWindow {
    pub fn open(&mut self, thread: &str, first: Option<String>) {
        self.thread = thread.into();
        self.browsing_history = first.is_some();
        self.first = first;
    }

    pub fn visible_range(&mut self, t: &ImThread, follow_latest: bool) -> Range<usize> {
        if self.thread != t.thread_id {
            self.open(&t.thread_id, None);
        }
        let start = if follow_latest && !self.browsing_history {
            t.messages.len().saturating_sub(PAGE_SIZE)
        } else {
            self.first
                .as_ref()
                .and_then(|key| t.messages.iter().position(|m| has_key(m, key)))
                .unwrap_or_else(|| t.messages.len().saturating_sub(PAGE_SIZE))
        };
        self.first = t.messages.get(start).map(message_key);
        start..(start + PAGE_SIZE).min(t.messages.len())
    }
}

pub(super) fn is_outgoing(m: &ImMessage) -> bool {
    m.outgoing || matches!(m.sender.trim(), "我" | "Me" | "me" | "自己")
}

pub(super) fn sender<'a>(t: &'a ImThread, m: &'a ImMessage) -> &'a str {
    if is_outgoing(m) {
        return "我";
    }
    if !m.sender.trim().is_empty()
        && !stepx_companion::push::is_generic_sender_name(&m.sender, &t.title)
    {
        return m.sender.trim();
    }
    if t.is_group {
        "未知成员"
    } else {
        t.title.trim()
    }
}

pub(super) fn same_burst(t: &ImThread, a: &ImMessage, b: &ImMessage) -> bool {
    if a.recalled
        || b.recalled
        || a.delivery_status == "failed"
        || b.delivery_status == "failed"
        || is_outgoing(a) != is_outgoing(b)
        || sender(t, a) != sender(t, b)
        || (t.is_group
            && (sender(t, a) == "未知成员"
                || a.sender.trim().is_empty()
                || b.sender.trim().is_empty()))
    {
        return false;
    }
    match (a.occurred_at_ms(), b.occurred_at_ms()) {
        (Some(a), Some(b)) => b >= a && b - a <= 120_000,
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn message(id: usize, sender: &str) -> ImMessage {
        ImMessage {
            id: id.to_string(),
            sender: sender.into(),
            timestamp_ms: 1_800_000_000_000 + id as i64 * 1000,
            ..Default::default()
        }
    }
    #[test]
    fn history_is_bounded_and_reading_position_survives_refresh_and_prepend() {
        let mut thread = ImThread {
            thread_id: "one".into(),
            messages: (0..5000).map(|i| message(i, "甲")).collect(),
            ..Default::default()
        };
        let mut window = MessageWindow::default();
        assert_eq!(window.visible_range(&thread, true), 4920..5000);
        thread
            .messages
            .extend((5000..5010).map(|i| message(i, "甲")));
        assert_eq!(
            window.visible_range(&thread, false),
            4920..5000,
            "new messages must not displace a reader"
        );
        assert_eq!(window.visible_range(&thread, true), 4930..5010);
        window.open("one", Some("4840".into()));
        thread.messages.insert(0, message(9999, "乙"));
        assert_eq!(
            window.visible_range(&thread, true),
            4841..4921,
            "history stays anchored by identity, not array index"
        );
        thread.thread_id = "two".into();
        assert_eq!(window.visible_range(&thread, true), 4931..5011);
    }
    #[test]
    fn bursts_require_known_sender_direction_and_time() {
        let thread = ImThread {
            is_group: true,
            title: "群".into(),
            last_sender: "乙".into(),
            ..Default::default()
        };
        let a = message(0, "甲");
        let mut b = message(60, "甲");
        assert!(same_burst(&thread, &a, &b));
        b.sender = "乙".into();
        assert!(!same_burst(&thread, &a, &b));
        b.sender = "".into();
        assert_eq!(sender(&thread, &b), "未知成员");
        assert!(!same_burst(&thread, &b, &b));
        b = message(121, "甲");
        assert!(!same_burst(&thread, &a, &b));
        b = message(1, "甲");
        b.outgoing = true;
        assert!(!same_burst(&thread, &a, &b));
        b.outgoing = false;
        b.time_known = Some(false);
        b.timestamp_ms = 0;
        assert!(!same_burst(&thread, &a, &b));
    }
}
