//! Wheel easing only. Precision touchpads keep the OS's own momentum unchanged.
use super::*;
use facloud_ui::{animation::schedule_animation_tick, units::Pixels};
use std::rc::Rc;

pub const SNAP_ZONE_PX: f32 = 28.0;
pub const PIN_BOTTOM_ZONE_PX: f32 = 36.0;
pub const UNPIN_BOTTOM_ZONE_PX: f32 = 40.0;
pub const WHEEL_DURATION_SECS: f32 = 0.12;
pub const TAIL_FOLLOW_DURATION_SECS: f32 = 0.11;

/// Snaps a target scroll offset cleanly to `0.0` (top edge) or `max_y` (bottom edge, when `max_y > 0.0`)
/// when within `SNAP_ZONE_PX` (`28.0px`).
#[inline]
pub fn snap_to_magnetic_edge(target_y: f32, max_y: f32) -> f32 {
    let clamped = if max_y > 0.0 {
        target_y.clamp(0.0, max_y)
    } else {
        target_y.max(0.0)
    };
    if clamped <= SNAP_ZONE_PX {
        0.0
    } else if max_y > 0.0 && (max_y - clamped) <= SNAP_ZONE_PX {
        max_y
    } else {
        clamped
    }
}

/// Updates `pinned_to_bottom` with 36px/40px hysteresis so micro-fluctuations do not unpin.
#[inline]
pub fn update_bottom_pin(pinned_to_bottom: bool, scroll_y: f32, max_y: f32) -> bool {
    if max_y <= 0.0 {
        return true;
    }
    let dist = (max_y - scroll_y).max(0.0);
    if dist <= PIN_BOTTOM_ZONE_PX {
        true
    } else if dist > UNPIN_BOTTOM_ZONE_PX {
        false
    } else {
        pinned_to_bottom
    }
}

#[derive(Clone, Debug)]
pub struct TrackSlot {
    pub from_y: f32,
    pub target_y: f32,
    pub current_y: f32,
    pub max_y: f32,
    pub started_at: Option<Instant>,
    pub duration_secs: f32,
    pub pinned_to_bottom: bool,
}

impl Default for TrackSlot {
    fn default() -> Self {
        Self {
            from_y: 0.0,
            target_y: 0.0,
            current_y: 0.0,
            max_y: 0.0,
            started_at: None,
            duration_secs: WHEEL_DURATION_SECS,
            pinned_to_bottom: true,
        }
    }
}

impl TrackSlot {
    pub fn apply_wheel_impulse(&mut self, current_y: f32, delta_y: f32, max_y: f32, now: Instant) -> f32 {
        self.max_y = max_y.max(0.0);
        let remaining = if self.started_at.is_some() {
            self.target_y - current_y
        } else {
            0.0
        };
        let tail = if remaining.signum() == delta_y.signum() {
            remaining
        } else {
            0.0
        };
        let raw_target = (current_y + delta_y + tail).max(0.0);
        let snapped = snap_to_magnetic_edge(raw_target, self.max_y);
        self.from_y = current_y;
        self.target_y = snapped;
        self.current_y = current_y;
        self.duration_secs = WHEEL_DURATION_SECS;
        self.started_at = Some(now);
        self.pinned_to_bottom = update_bottom_pin(self.pinned_to_bottom, snapped, self.max_y);
        self.target_y
    }

    pub fn sample(&mut self, now: Instant) -> f32 {
        let Some(started_at) = self.started_at else {
            return self.current_y;
        };
        let dur = self.duration_secs.max(0.001);
        let raw_t = (now.saturating_duration_since(started_at).as_secs_f32() / dur).clamp(0.0, 1.0);
        let eased = crate::ui::motion::bezier_control(raw_t);
        self.current_y = self.from_y + (self.target_y - self.from_y) * eased;
        if raw_t >= 1.0 {
            self.current_y = self.target_y;
            self.started_at = None;
        }
        self.current_y
    }
}

#[derive(Default)]
pub struct SmoothScrollRegistry {
    slots: HashMap<&'static str, TrackSlot>,
}

impl SmoothScrollRegistry {
    pub fn slot_mut(&mut self, key: &'static str) -> &mut TrackSlot {
        self.slots.entry(key).or_default()
    }

    pub fn on_user_scroll(&mut self, key: &'static str, current_y: f32, delta_y: f32, max_y: f32) -> f32 {
        self.slot_mut(key)
            .apply_wheel_impulse(current_y, delta_y, max_y, Instant::now())
    }

    /// When streaming lines grow `new_max_y`, if `pinned_to_bottom` is `true`, smoothly animates
    /// `target_y` to `new_max_y` over `110ms` using `bezier_control` instead of a jarring instant jump.
    pub fn follow_tail(&mut self, key: &'static str, new_max_y: f32) -> Option<f32> {
        let slot = self.slots.entry(key).or_default();
        let clamped_max = new_max_y.max(0.0);
        slot.max_y = clamped_max;
        if !slot.pinned_to_bottom {
            return None;
        }
        let now = Instant::now();
        let cur = slot.sample(now);
        if (clamped_max - slot.target_y).abs() > 0.5 {
            slot.from_y = cur;
            slot.target_y = clamped_max;
            slot.duration_secs = TAIL_FOLLOW_DURATION_SECS;
            slot.started_at = Some(now);
        }
        Some(slot.sample(now))
    }
}

pub(super) struct WheelMotion {
    from: f32,
    to: f32,
    expected: f32,
    start: Option<Instant>,
    duration_secs: f32,
    max_y: f32,
    pinned_to_bottom: bool,
}

impl Default for WheelMotion {
    fn default() -> Self {
        Self {
            from: 0.0,
            to: 0.0,
            expected: 0.0,
            start: None,
            duration_secs: WHEEL_DURATION_SECS,
            max_y: 0.0,
            pinned_to_bottom: true,
        }
    }
}

pub(super) type WheelHandle = Rc<RefCell<WheelMotion>>;
pub(super) struct SmoothScroll {
    pub child: widgets::El,
    pub scroll: ClippedScrollStateHandle,
    pub motion: WheelHandle,
    pub enabled: bool,
}
impl Element for SmoothScroll {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let mut m = self.motion.borrow_mut();
        if let Some(start) = m.start {
            let current = self.scroll.scroll_start().as_f32();
            if !self.enabled || (current - m.expected).abs() > 1. {
                m.start = None;
            } else {
                let dur = if m.duration_secs > 0.0 {
                    m.duration_secs
                } else {
                    WHEEL_DURATION_SECS
                };
                let t = (start.elapsed().as_secs_f32() / dur).min(1.);
                let y = m.from + (m.to - m.from) * crate::ui::motion::bezier_control(t);
                self.scroll.scroll_to(Pixels::new(y));
                m.expected = y;
                if t >= 1. {
                    m.start = None;
                }
            }
        }
        drop(m);
        let size = self.child.layout(c, ctx, app);
        let mut m = self.motion.borrow_mut();
        let clamped = self.scroll.scroll_start().as_f32();
        if (clamped - m.expected).abs() > 1. {
            m.start = None;
        }
        size
    }
    fn after_layout(&mut self, c: &mut AfterLayoutContext, a: &AppContext) {
        self.child.after_layout(c, a)
    }
    fn paint(&mut self, o: Vector2F, c: &mut PaintContext, a: &AppContext) {
        self.child.paint(o, c, a);
        if self.motion.borrow().start.is_some() {
            schedule_animation_tick(c, 16);
        }
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        c: &mut EventContext,
        a: &AppContext,
    ) -> bool {
        let before = self.scroll.scroll_start().as_f32();
        let handled = self.child.dispatch_event(e, c, a);
        let after = self.scroll.scroll_start().as_f32();
        if let Event::ScrollWheel { precise, .. } = e.raw_event() {
            let mut m = self.motion.borrow_mut();
            if *precise || !self.enabled {
                m.start = None;
            } else if (after - before).abs() > 0.1 {
                let remaining = if m.start.is_some() { m.to - before } else { 0. };
                let delta = after - before;
                // Reversing the wheel cancels old momentum immediately.
                let tail = if remaining.signum() == delta.signum() {
                    remaining
                } else {
                    0.
                };
                let raw_target = (after + tail).max(0.);
                let target = snap_to_magnetic_edge(raw_target, m.max_y);
                m.from = before;
                m.to = target;
                m.expected = before;
                m.duration_secs = WHEEL_DURATION_SECS;
                m.start = Some(Instant::now());
                m.pinned_to_bottom = update_bottom_pin(m.pinned_to_bottom, target, m.max_y);
                self.scroll.scroll_to(Pixels::new(before));
                c.notify();
            }
        } else if matches!(e.raw_event(), Event::LeftMouseDown { .. }) {
            self.motion.borrow_mut().start = None;
        }
        handled
    }
}
impl Companion {
    pub(super) fn reveal_latest_chat(&self) {
        self.chat_tail_requested.set(true);
    }
    pub(super) fn chat_near_end(&self) -> bool {
        let (content, viewport) = self.chat_scroll_metrics.get();
        content - viewport - self.scroll.scroll_start().as_f32() <= 80.
    }
    #[track_caller]
    pub(super) fn chat_scroll(&self, content: widgets::El) -> widgets::El {
        let content = ChatExtent {
            child: content,
            metrics: self.chat_scroll_metrics.clone(),
            viewport: false,
            tail: None,
        }
        .finish();
        ChatExtent {
            child: self.chat_pane_scroll(content, false),
            metrics: self.chat_scroll_metrics.clone(),
            viewport: true,
            tail: Some((
                self.scroll.clone(),
                self.chat_tail_requested.clone(),
                self.selected_text.is_none(),
            )),
        }
        .finish()
    }
    // Overlay the scrollbar so messages and the pinned composer share one axis,
    // whether the conversation is empty, short or long enough to scroll.
    pub(super) fn chat_pane_scroll(&self, content: widgets::El, centered: bool) -> widgets::El {
        let p = self.p();
        let thumb = facloud_ui::color::ColorU::new(p.text3.r, p.text3.g, p.text3.b, 90);
        let active = facloud_ui::color::ColorU::new(p.text3.r, p.text3.g, p.text3.b, 160);
        let make = if centered {
            ClippedScrollable::vertical_centered
        } else {
            ClippedScrollable::vertical
        };
        self.ease_scroll(
            make(
                self.scroll.clone(),
                content,
                ScrollbarWidth::Auto,
                thumb.into(),
                active.into(),
                Fill::None,
            )
            .with_overlayed_scrollbar()
            .finish(),
            self.scroll.clone(),
        )
    }
    #[track_caller]
    pub(super) fn ease_scroll(
        &self,
        child: widgets::El,
        scroll: ClippedScrollStateHandle,
    ) -> widgets::El {
        let loc = std::panic::Location::caller();
        let handle = self
            .wheel_motions
            .borrow_mut()
            .entry((loc.file(), loc.line()))
            .or_default()
            .clone();
        SmoothScroll {
            child,
            scroll,
            motion: handle,
            enabled: !self.settings.value.reduced_motion,
        }
        .finish()
    }
}

struct ChatExtent {
    child: widgets::El,
    metrics: Rc<std::cell::Cell<(f32, f32)>>,
    viewport: bool,
    tail: Option<(ClippedScrollStateHandle, Rc<std::cell::Cell<bool>>, bool)>,
}

/// Pin only on entry or when a tail-following reader's layout grows/resizes.
/// An empty first layout stays at the tail when async history/images arrive.
fn tail_offset(
    old: (f32, f32),
    new: (f32, f32),
    offset: f32,
    requested: bool,
    follow: bool,
) -> Option<f32> {
    let changed = (old.0 - new.0).abs() > 0.5 || (old.1 - new.1).abs() > 0.5;
    (requested || (changed && follow && old.0 - old.1 - offset <= 80.))
        .then(|| (new.0 - new.1).max(0.))
}
impl Element for ChatExtent {
    fn layout(&mut self, c: SizeConstraint, x: &mut LayoutContext, a: &AppContext) -> Vector2F {
        let old = self.metrics.get();
        let before = self.tail.as_ref().map(|(scroll, request, follow)| {
            (
                scroll.scroll_start().as_f32(),
                request.replace(false),
                *follow,
            )
        });
        let size = self.child.layout(c, x, a);
        let (content, viewport) = self.metrics.get();
        self.metrics.set(if self.viewport {
            (content, size.y())
        } else {
            (size.y(), viewport)
        });
        if let (Some((scroll, _, _)), Some((offset, requested, follow))) = (&self.tail, before) {
            if let Some(target) = tail_offset(old, self.metrics.get(), offset, requested, follow) {
                scroll.scroll_to(Pixels::new(target));
            }
        }
        size
    }
    fn after_layout(&mut self, c: &mut AfterLayoutContext, a: &AppContext) {
        self.child.after_layout(c, a)
    }
    fn paint(&mut self, o: Vector2F, c: &mut PaintContext, a: &AppContext) {
        self.child.paint(o, c, a)
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        c: &mut EventContext,
        a: &AppContext,
    ) -> bool {
        self.child.dispatch_event(e, c, a)
    }
}

#[cfg(test)]
mod tail_tests {
    use super::{
        snap_to_magnetic_edge, tail_offset, update_bottom_pin, SmoothScrollRegistry, SNAP_ZONE_PX,
        TAIL_FOLLOW_DURATION_SECS,
    };
    #[test]
    fn initial_async_history_and_late_images_follow_latest() {
        assert_eq!(tail_offset((0., 0.), (0., 500.), 0., true, true), Some(0.));
        assert_eq!(
            tail_offset((0., 500.), (4000., 500.), 0., false, true),
            Some(3500.)
        );
        assert_eq!(
            tail_offset((4000., 500.), (4500., 500.), 3500., false, true),
            Some(4000.)
        );
    }
    #[test]
    fn manual_history_reading_and_selection_stay_put() {
        assert_eq!(
            tail_offset((4000., 500.), (4500., 500.), 2000., false, true),
            None
        );
        assert_eq!(
            tail_offset((4000., 500.), (4500., 500.), 3500., false, false),
            None
        );
        assert_eq!(
            tail_offset((4000., 500.), (4000., 500.), 3450., false, true),
            None
        );
    }
    #[test]
    fn mode_session_and_resize_land_on_latest() {
        assert_eq!(
            tail_offset((4000., 800.), (5000., 400.), 0., true, true),
            Some(4600.)
        );
        assert_eq!(
            tail_offset((4000., 800.), (5000., 400.), 3200., false, true),
            Some(4600.)
        );
        assert_eq!(
            tail_offset((5000., 400.), (4000., 800.), 4600., false, true),
            Some(3200.)
        );
    }
    #[test]
    fn magnetic_edge_snapping_and_tail_follow_hysteresis() {
        assert_eq!(SNAP_ZONE_PX, 28.0);
        // Snaps within 28px of top (0.0) and bottom (max_y)
        assert_eq!(snap_to_magnetic_edge(14.0, 1000.0), 0.0);
        assert_eq!(snap_to_magnetic_edge(28.0, 1000.0), 0.0);
        assert_eq!(snap_to_magnetic_edge(29.0, 1000.0), 29.0);
        assert_eq!(snap_to_magnetic_edge(972.0, 1000.0), 1000.0);
        assert_eq!(snap_to_magnetic_edge(985.0, 1000.0), 1000.0);
        assert_eq!(snap_to_magnetic_edge(970.0, 1000.0), 970.0);

        // Pin/unpin hysteresis: <= 36px pins, > 40px unpins
        assert!(update_bottom_pin(false, 964.0, 1000.0));
        assert!(update_bottom_pin(true, 961.0, 1000.0));
        assert!(!update_bottom_pin(true, 955.0, 1000.0));

        let mut reg = SmoothScrollRegistry::default();
        assert!(reg.follow_tail("chat", 400.0).is_some());
        let slot = reg.slot_mut("chat");
        assert!(slot.pinned_to_bottom);
        assert_eq!(slot.target_y, 400.0);
        assert!((slot.duration_secs - TAIL_FOLLOW_DURATION_SECS).abs() < 1e-6);

        // Scrolling upward away from max_y by > 40px unpins tail follow
        reg.on_user_scroll("chat", 400.0, -120.0, 400.0);
        assert!(!reg.slot_mut("chat").pinned_to_bottom);
        assert_eq!(reg.follow_tail("chat", 520.0), None);

        // Scrolling back within 36px of max_y re-pins tail follow and snaps to bottom if within 28px
        let target = reg.on_user_scroll("chat", 280.0, 225.0, 520.0);
        assert_eq!(target, 520.0);
        assert!(reg.slot_mut("chat").pinned_to_bottom);
        assert!(reg.follow_tail("chat", 640.0).is_some());
        assert_eq!(reg.slot_mut("chat").target_y, 640.0);
    }
}
