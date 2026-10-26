//! Short reveal of received text only. Never generates text or schedules network I/O.
use super::*;
#[derive(Default)]
pub(super) struct ReplyMotion {
    entries: HashMap<(String, usize), Reveal>,
    pub live: std::collections::HashSet<String>,
}
struct Reveal {
    target: String,
    from: usize,
    started: Instant,
}
impl ReplyMotion {
    pub fn track(&mut self, key: String, index: usize, old: &str, text: &str) {
        if old == text || text.is_empty() {
            return;
        }
        // Rewrites/protocol changes appear atomically; only append-only text is revealed.
        if !text.starts_with(old) {
            self.entries.remove(&(key, index));
            return;
        }
        self.entries.insert(
            (key, index),
            Reveal {
                target: text.into(),
                from: old.len(),
                started: Instant::now(),
            },
        );
        if self.entries.len() > 16 {
            self.entries
                .retain(|_, r| r.started.elapsed() < Duration::from_millis(240));
        }
    }
    pub fn active(&self) -> bool {
        self.entries
            .values()
            .any(|r| r.started.elapsed() < Duration::from_millis(240))
    }
    pub fn prune(&mut self) {
        self.entries
            .retain(|_, r| r.started.elapsed() < Duration::from_millis(240));
    }
}
fn visible_end(text: &str, from: usize, t: f32) -> usize {
    if t >= 1. {
        return text.len();
    }
    let mut end = from + (((text.len() - from) as f32) * (1. - (1. - t).powi(2))) as usize;
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    end
}
impl Companion {
    pub(super) fn live_reply_text(&self, index: usize, text: &str) -> String {
        if self.settings.value.reduced_motion
            || self.selected_text.is_some()
            || self.context_menu.is_some()
        {
            return text.into();
        }
        let Some(s) = self.amoo_sessions.get(self.amoo_session_idx) else {
            return text.into();
        };
        let Some(r) = self
            .reply_motion
            .entries
            .get(&(s.session_key.clone(), index))
        else {
            return text.into();
        };
        if r.target != text {
            return text.into();
        }
        let end = visible_end(
            text,
            r.from,
            (r.started.elapsed().as_secs_f32() / 0.24).min(1.),
        );
        text[..end].into()
    }
    pub(super) fn tick_reply_motion(&mut self, ctx: &mut ViewContext<Self>) {
        if self.reply_motion.entries.is_empty() {
            return;
        }
        self.reply_motion.prune();
        if !self.settings.hidden && matches!(self.page, Page::Amoo | Page::Work) {
            ctx.notify();
        }
    }
    pub(super) fn chat_waiting(&self) -> bool {
        matches!(self.page, Page::Amoo | Page::Work)
            && self
                .amoo_sessions
                .get(self.amoo_session_idx)
                .is_some_and(|s| self.amoo_pending_reply.contains_key(&s.session_key))
    }
    pub(super) fn footer_detail(&self) -> String {
        if self.error {
            return self.status.clone();
        }
        if self.initializing {
            return "正在连接手机…".into();
        }
        if self.busy {
            return self.status.clone();
        }
        if self.page == Page::Work && self.work.running {
            return self.work.status.clone();
        }
        if self.chat_waiting() {
            return "Amoo 正在处理 · 等待手机返回内容…".into();
        }
        if self.amoo_ui.snapshot_job.is_some() && !self.amoo_gateway_online {
            return "正在连接手机 Amoo…".into();
        }
        if matches!(self.page, Page::Amoo) {
            if let Some(run) = &self.workspace.running {
                let secs = run.started_at.elapsed().as_secs();
                return format!("正在执行任务（{}s）", secs);
            }
        }
        if matches!(self.status.as_str(), "就绪" | "") {
            return if self.selected.is_some() {
                "设备已连接"
            } else {
                "等待设备连接"
            }
            .into();
        }
        self.status.clone()
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reveals_only_received_utf8_append_and_finishes_quickly() {
        let s = "你好🌟，这是回复";
        for i in 0..=100 {
            let n = visible_end(s, "你好".len(), i as f32 / 100.);
            assert!(s.is_char_boundary(n));
            assert!(n >= "你好".len());
        }
        assert_eq!(visible_end(s, 0, 1.), s.len());
    }
    #[test]
    fn history_and_rewrites_are_not_typed_again() {
        let mut r = ReplyMotion::default();
        r.track("x".into(), 0, "old", "new");
        assert!(!r.active());
        r.track("x".into(), 0, "你好", "你好呀");
        assert!(r.active());
    }
}
