//! Per-character 90ms staggered glyph bloom ("晕开") for received streaming replies.
//! Never generates text or schedules network I/O.
use super::*;

pub const CHAR_BLOOM_MS: f32 = 90.0;
pub const BASE_STAGGER_MS: f32 = 16.0;
pub const MAX_FRONTIER_CHARS: usize = 16;

#[derive(Clone, Debug)]
struct Entry {
    settled_chars: usize,
    target_chars: usize,
    wave_started_at: Instant,
    stagger_ms: f32,
    active: bool,
}

#[derive(Default)]
pub(super) struct ReplyMotionState {
    entries: HashMap<String, Entry>,
    render_blooms: HashMap<String, facloud_ui::text_layout::GlyphBloomSchedule>,
}

#[derive(Default)]
pub(super) struct ReplyMotion {
    inner: std::cell::RefCell<ReplyMotionState>,
    pub live: std::collections::HashSet<String>,
}

fn slice_chars(text: &str, max_chars: usize) -> &str {
    if max_chars == 0 {
        return "";
    }
    match text.char_indices().nth(max_chars) {
        Some((byte_idx, _)) => &text[..byte_idx],
        None => text,
    }
}

impl ReplyMotionState {
    fn compute_stagger_ms(pending: usize) -> f32 {
        if pending > 12 {
            (120.0 / pending as f32).clamp(4.0, BASE_STAGGER_MS)
        } else {
            BASE_STAGGER_MS
        }
    }

    fn advance_entry(entry: &mut Entry, now: Instant) -> f32 {
        if !entry.active {
            return 0.0;
        }
        let mut elapsed_ms = now
            .saturating_duration_since(entry.wave_started_at)
            .as_secs_f32()
            * 1000.0;
        while entry.settled_chars < entry.target_chars && elapsed_ms >= CHAR_BLOOM_MS {
            entry.settled_chars += 1;
            if entry.settled_chars >= entry.target_chars {
                entry.active = false;
                return 0.0;
            }
            let step_ms = entry.stagger_ms;
            entry.wave_started_at += Duration::from_secs_f32(step_ms / 1000.0);
            elapsed_ms = (elapsed_ms - step_ms).max(0.0);
        }
        if entry.settled_chars >= entry.target_chars {
            entry.active = false;
            0.0
        } else {
            elapsed_ms
        }
    }

    pub fn sync(&mut self, key: &str, text: &str, animate: bool) {
        self.sync_at(key, text, animate, Instant::now());
    }

    pub(super) fn sync_at(&mut self, key: &str, text: &str, animate: bool, now: Instant) {
        let char_count = text.chars().count();
        if !animate || char_count == 0 {
            self.entries.insert(
                key.to_string(),
                Entry {
                    settled_chars: char_count,
                    target_chars: char_count,
                    wave_started_at: now,
                    stagger_ms: BASE_STAGGER_MS,
                    active: false,
                },
            );
            return;
        }
        if let Some(entry) = self.entries.get_mut(key) {
            Self::advance_entry(entry, now);
            if char_count < entry.settled_chars {
                entry.settled_chars = char_count;
                entry.target_chars = char_count;
                entry.wave_started_at = now;
                entry.stagger_ms = BASE_STAGGER_MS;
                entry.active = false;
            } else if char_count > entry.target_chars {
                if !entry.active {
                    entry.settled_chars = entry.target_chars;
                    entry.wave_started_at = now;
                }
                if char_count.saturating_sub(entry.settled_chars) > MAX_FRONTIER_CHARS * 2 {
                    entry.settled_chars = char_count.saturating_sub(MAX_FRONTIER_CHARS);
                    entry.wave_started_at = now;
                }
                entry.target_chars = char_count;
                let pending = entry.target_chars.saturating_sub(entry.settled_chars);
                entry.stagger_ms = Self::compute_stagger_ms(pending);
                entry.active = pending > 0;
            }
        } else {
            let settled_chars = char_count.saturating_sub(MAX_FRONTIER_CHARS);
            let pending = char_count.saturating_sub(settled_chars);
            let stagger_ms = Self::compute_stagger_ms(pending);
            self.entries.insert(
                key.to_string(),
                Entry {
                    settled_chars,
                    target_chars: char_count,
                    wave_started_at: now,
                    stagger_ms,
                    active: pending > 0,
                },
            );
        }
        if self.entries.len() > 64 {
            self.entries.retain(|_, e| e.active);
        }
    }

    pub fn bloom_schedule<'a>(
        &mut self,
        key: &str,
        text: &'a str,
        animate: bool,
    ) -> (&'a str, Option<facloud_ui::text_layout::GlyphBloomSchedule>) {
        self.bloom_schedule_at(key, text, animate, Instant::now())
    }

    pub(super) fn bloom_schedule_at<'a>(
        &mut self,
        key: &str,
        text: &'a str,
        animate: bool,
        now: Instant,
    ) -> (&'a str, Option<facloud_ui::text_layout::GlyphBloomSchedule>) {
        self.sync_at(key, text, animate, now);
        let Some(entry) = self.entries.get_mut(key) else {
            return (text, None);
        };
        if !entry.active {
            return (text, None);
        }
        let elapsed_ms = Self::advance_entry(entry, now);
        if !entry.active {
            return (text, None);
        }
        let visible_chars = (entry.settled_chars
            + (elapsed_ms / entry.stagger_ms).floor() as usize
            + 1)
            .min(entry.target_chars);
        let visible = slice_chars(text, visible_chars);
        let schedule = facloud_ui::text_layout::GlyphBloomSchedule {
            reveal_start_char: entry.settled_chars,
            elapsed_ms,
            stagger_ms: entry.stagger_ms,
            duration_ms: CHAR_BLOOM_MS,
        };
        (visible, Some(schedule))
    }

    pub fn visible_slice<'a>(&mut self, key: &str, text: &'a str, animate: bool) -> &'a str {
        self.bloom_schedule(key, text, animate).0
    }

    pub fn tick(&mut self) -> bool {
        self.tick_at(Instant::now())
    }

    pub(super) fn tick_at(&mut self, now: Instant) -> bool {
        let mut had_active = false;
        for entry in self.entries.values_mut() {
            if entry.active {
                had_active = true;
                Self::advance_entry(entry, now);
            }
        }
        if !had_active {
            self.render_blooms.clear();
        }
        had_active
    }

    pub fn has_active(&self) -> bool {
        self.entries.values().any(|e| e.active)
    }

    pub fn is_key_active(&self, key: &str) -> bool {
        self.entries.get(key).is_some_and(|e| e.active)
    }
}

impl ReplyMotion {
    pub fn borrow_mut(&self) -> std::cell::RefMut<'_, ReplyMotionState> {
        self.inner.borrow_mut()
    }

    pub fn sync(&self, key: &str, text: &str, animate: bool) {
        self.inner.borrow_mut().sync(key, text, animate);
    }

    pub fn sync_delta(&self, key: &str, old: &str, text: &str, animate: bool) {
        if old == text || text.is_empty() {
            return;
        }
        let mut state = self.inner.borrow_mut();
        if !animate || !text.starts_with(old) {
            state.sync(key, text, false);
            return;
        }
        let old_chars = old.chars().count();
        let needs_baseline = !old.is_empty()
            && state
                .entries
                .get(key)
                .map_or(true, |e| !e.active && e.target_chars != old_chars);
        if needs_baseline {
            state.sync(key, old, false);
        }
        state.sync(key, text, true);
    }

    pub fn sync_delta_with_elapsed(&self, key: &str, old: &str, text: &str, elapsed_ms: f32) {
        let mut state = self.inner.borrow_mut();
        let now = Instant::now();
        let old_chars = old.chars().count();
        let target_chars = text.chars().count();
        state.entries.insert(
            key.to_string(),
            Entry {
                settled_chars: old_chars,
                target_chars,
                wave_started_at: now
                    .checked_sub(Duration::from_secs_f32((elapsed_ms / 1000.0).max(0.0)))
                    .unwrap_or(now),
                stagger_ms: BASE_STAGGER_MS,
                active: target_chars > old_chars,
            },
        );
    }

    pub fn bloom_schedule<'a>(
        &self,
        key: &str,
        text: &'a str,
        animate: bool,
    ) -> (&'a str, Option<facloud_ui::text_layout::GlyphBloomSchedule>) {
        self.inner.borrow_mut().bloom_schedule(key, text, animate)
    }

    pub fn visible_slice<'a>(&self, key: &str, text: &'a str, animate: bool) -> &'a str {
        self.inner.borrow_mut().visible_slice(key, text, animate)
    }

    pub fn tick(&self) -> bool {
        self.inner.borrow_mut().tick()
    }

    pub fn has_active(&self) -> bool {
        self.inner.borrow().has_active()
    }

    pub fn active(&self) -> bool {
        self.has_active()
    }

    pub fn is_key_active(&self, key: &str) -> bool {
        self.inner.borrow().is_key_active(key)
    }

    pub fn prune(&self) {
        let _ = self.inner.borrow_mut().tick();
    }

    pub fn track(&self, key: String, index: usize, old: &str, text: &str) {
        if old == text || text.is_empty() {
            return;
        }
        let indexed_key = format!("{key}#{index}");
        let amoo_id = format!("amoo-{key}-{index}");
        let ws_phone_id = format!("ws-phone-reply-{key}-{index}");
        let animate = text.starts_with(old);
        self.sync_delta(&indexed_key, old, text, animate);
        self.sync_delta(&amoo_id, old, text, animate);
        self.sync_delta(&ws_phone_id, old, text, animate);
    }

    pub(super) fn take_render_bloom(
        &self,
        id: &str,
    ) -> Option<facloud_ui::text_layout::GlyphBloomSchedule> {
        self.inner.borrow_mut().render_blooms.remove(id)
    }
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
        let indexed_key = format!("{}#{index}", s.session_key);
        let animate = self.reply_motion.is_key_active(&indexed_key)
            || (index + 1 == s.items.len()
                && (self.amoo_pending_reply.contains_key(&s.session_key)
                    || self.reply_motion.live.contains(&s.session_key)));
        let (visible, schedule) = self
            .reply_motion
            .bloom_schedule(&indexed_key, text, animate);
        let mut state = self.reply_motion.borrow_mut();
        let amoo_render_id = format!("amoo-{}-{index}", s.session_key);
        let work_render_id = format!("message-{index}");
        if let Some(sched) = schedule {
            state.render_blooms.insert(amoo_render_id, sched);
            state.render_blooms.insert(work_render_id, sched);
        } else {
            state.render_blooms.remove(&amoo_render_id);
            state.render_blooms.remove(&work_render_id);
        }
        visible.into()
    }

    pub(super) fn tick_reply_motion(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.reply_motion.tick() {
            return;
        }
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
        if self.page == Page::Amoo && self.workspace.running.is_some() {
            return self.run_status_line();
        }
        if self.chat_waiting() {
            return "Amoo 正在处理 · 等待手机返回内容…".into();
        }
        if self.amoo_ui.snapshot_job.is_some() && !self.amoo_gateway_online {
            return "正在连接手机 Amoo…".into();
        }
        if self.page == Page::Amoo
            && matches!(
                self.status.as_str(),
                "已刷新后台对话" | "已在后台创建 Amoo 会话，可直接输入消息"
            )
        {
            if let Some(task) = self.workspace_task() {
                use stepx_companion::amoo_workspace::Phase;
                if let Some(step) = task.steps.last() {
                    return match step.phase {
                        Phase::Done if !task.outputs.is_empty() => {
                            "成果已保存在电脑，可预览或继续修改".into()
                        }
                        Phase::Done => "已收到 Amoo 回复".into(),
                        Phase::Stopped => "已停止，草稿和已有成果保留".into(),
                        Phase::Failed => "本次未完成，可查看原因后重试".into(),
                        Phase::Unconfirmed => "结果尚待确认，可同步查看".into(),
                        _ => "手机已连接，可以继续输入".into(),
                    };
                }
            }
            return "手机已连接，可以开始对话".into();
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
    fn char_bloom_duration_and_sequential_stagger_progress() {
        assert_eq!(CHAR_BLOOM_MS, 90.0);
        let mut state = ReplyMotionState::default();
        let t0 = Instant::now();
        let text = "你好🌟世界";

        // At t = 0ms: char 0 ('你') starts blooming; subsequent chars haven't started yet.
        let (vis0, sched0) = state.bloom_schedule_at("msg-1", text, true, t0);
        assert_eq!(vis0, "你");
        let sched0 = sched0.expect("bloom schedule active at t=0");
        assert_eq!(sched0.reveal_start_char, 0);
        assert_eq!(sched0.duration_ms, 90.0);
        assert_eq!(sched0.stagger_ms, BASE_STAGGER_MS);

        // At t = 20ms (> 16ms stagger): char 0 is at 20ms/90ms, char 1 ('好') has started (4ms/90ms).
        let (vis1, sched1) =
            state.bloom_schedule_at("msg-1", text, true, t0 + Duration::from_millis(20));
        assert_eq!(vis1, "你好");
        let sched1 = sched1.expect("bloom schedule active at t=20ms");
        assert_eq!(sched1.reveal_start_char, 0);
        assert!((sched1.elapsed_ms - 20.0).abs() < 1e-3);

        // At t = 95ms: char 0 (95ms >= 90ms) is settled; char 1 is now the frontier start (79ms/90ms).
        let (vis2, sched2) =
            state.bloom_schedule_at("msg-1", text, true, t0 + Duration::from_millis(95));
        assert_eq!(vis2, text);
        let sched2 = sched2.expect("bloom schedule active at t=95ms");
        assert_eq!(sched2.reveal_start_char, 1);
        assert!((sched2.elapsed_ms - (95.0 - BASE_STAGGER_MS)).abs() < 1e-2);
        assert_eq!(sched2.duration_ms, CHAR_BLOOM_MS);

        // After all 5 characters finish their 90ms bloom (4 * 16ms + 90ms = 154ms), schedule settles to None.
        let (vis_done, sched_done) =
            state.bloom_schedule_at("msg-1", text, true, t0 + Duration::from_millis(160));
        assert_eq!(vis_done, text);
        assert!(sched_done.is_none());
        assert!(!state.has_active());
    }

    #[test]
    fn history_and_rewrites_are_not_typed_again() {
        let r = ReplyMotion::default();
        r.track("x".into(), 0, "old", "new");
        assert!(!r.active());
        r.track("x".into(), 0, "你好", "你好呀");
        assert!(r.active());
    }
}
