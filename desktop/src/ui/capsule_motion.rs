//! Stable-identity carousel. Rotation and transitions advance in paint, independent
//! of phone polling or root view rebuilds. Only the 420 ms transition needs frames.
use super::{icons::UiIcon, theme::*, widgets::El, *};
use facloud_ui::{color::ColorU, platform::Cursor, ClipBounds};
use pathfinder_geometry::rect::RectF;
use std::rc::Rc;

const DWELL: Duration = Duration::from_millis(5500);
const TRANSITION: Duration = Duration::from_millis(420);
const HEIGHT: f32 = 30.;
const PAD: f32 = 7.;

#[derive(Clone)]
pub(super) struct Visual {
    pub id: String,
    pub revision: Option<String>,
    pub icon: UiIcon,
    pub accent: ColorU,
    pub domain: String,
    pub metric: String,
    pub live: bool,
    pub secondary: Vec<(UiIcon, ColorU)>,
    pub total: usize,
}

impl Visual {
    pub fn paint_key(&self) -> String {
        self.revision.as_ref().map_or_else(
            || self.id.clone(),
            |revision| format!("{}:{revision}", self.id),
        )
    }
}

#[derive(Default)]
pub(super) struct Carousel {
    items: Vec<Visual>,
    #[cfg(feature = "ui-capture")]
    preview: Option<Vec<Visual>>,
    current: Option<Visual>,
    outgoing: Option<Visual>,
    started: Option<Instant>,
    next: Option<Instant>,
    enabled: bool,
    popover_open: bool,
    hovered: bool,
    pressed: bool,
    leaving: bool,
}

struct Frame {
    from: Option<String>,
    to: String,
    progress: f32,
    accent: ColorU,
    next_paint: Option<Duration>,
    opacity: f32,
}

fn smooth(t: f32) -> f32 {
    let t = t.clamp(0., 1.);
    t * t * (3. - 2. * t)
}

impl Carousel {
    pub fn sync(&mut self, items: Vec<Visual>, enabled: bool, popover_open: bool, now: Instant) {
        #[cfg(feature = "ui-capture")]
        let items = self.preview.clone().unwrap_or(items);
        if items.is_empty() {
            self.items.clear();
            self.outgoing = None;
            self.pressed = false;
            self.hovered = false;
            self.next = None;
            self.popover_open = popover_open;
            if enabled && self.current.is_some() {
                if !self.leaving {
                    self.started = Some(now);
                    self.leaving = true;
                }
            } else {
                self.started = None;
                self.current = None;
                self.leaving = false;
            }
            return;
        }
        if self.leaving {
            self.leaving = false;
            self.started = None;
        }
        if enabled != self.enabled
            || popover_open != self.popover_open
            || (self.items.len() <= 1 && items.len() > 1)
        {
            self.next = Some(now + DWELL);
        }
        self.enabled = enabled;
        self.popover_open = popover_open;
        self.items = items;
        if !enabled {
            self.outgoing = None;
            self.started = None;
        }
        if let Some(current) = &self.current {
            if let Some(updated) = self.items.iter().find(|v| v.id == current.id).cloned() {
                // Data refreshes do not restart the reading interval or change selection.
                if enabled && self.outgoing.is_none() && current.revision != updated.revision {
                    self.select(updated, now);
                } else {
                    self.current = Some(updated);
                }
            } else if self.outgoing.is_none() {
                self.select(self.items[0].clone(), now);
            }
        } else {
            self.current = Some(self.items[0].clone());
            self.next = Some(now + DWELL);
        }
    }

    /// Local render fixture only: never creates a phone alarm, notification or persisted record.
    #[cfg(feature = "ui-capture")]
    pub fn preview(&mut self, enabled: bool) {
        *self = Self::default();
        if enabled {
            self.preview = Some(vec![
                Visual {
                    id: "motion-preview-alarm".into(),
                    revision: None,
                    icon: UiIcon::Clock,
                    accent: rgb(96, 165, 250),
                    domain: "闹钟".into(),
                    metric: "11:50".into(),
                    live: true,
                    secondary: vec![],
                    total: 3,
                },
                Visual {
                    id: "motion-preview-pickup".into(),
                    revision: None,
                    icon: UiIcon::Package,
                    accent: rgb(45, 212, 191),
                    domain: "取件码".into(),
                    metric: "31744725".into(),
                    live: false,
                    secondary: vec![],
                    total: 3,
                },
                Visual {
                    id: "motion-preview-delivery".into(),
                    revision: None,
                    icon: UiIcon::Package,
                    accent: rgb(52, 211, 153),
                    domain: "美团外卖".into(),
                    metric: "20:30~20:50".into(),
                    live: true,
                    secondary: vec![],
                    total: 3,
                },
            ]);
        }
    }

    #[cfg(feature = "ui-capture")]
    pub fn diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"preview": self.preview.is_some(),
            "current": self.current.as_ref().map(|v| &v.id),
            "outgoing": self.outgoing.as_ref().map(|v| &v.id),
            "hovered": self.hovered, "popover_open": self.popover_open,
            "enabled": self.enabled,
            "elapsed_ms": self.started.map(|start| start.elapsed().as_millis())})
    }

    fn select(&mut self, next: Visual, now: Instant) {
        self.outgoing = self.current.take().filter(|_| self.enabled);
        self.current = Some(next);
        self.started = self.outgoing.as_ref().map(|_| now);
        self.next = Some(now + TRANSITION + DWELL);
    }

    fn set_hovered(&mut self, hovered: bool, now: Instant) -> bool {
        if self.hovered == hovered {
            return false;
        }
        self.hovered = hovered;
        // Returning from an interaction always gives a full reading interval.
        self.next = Some(now + DWELL);
        true
    }

    pub fn presentations(&self) -> Vec<Visual> {
        let mut rows = self.items.clone();
        for visual in [&self.current, &self.outgoing].into_iter().flatten() {
            if !rows
                .iter()
                .any(|v| v.id == visual.id && v.revision == visual.revision)
            {
                rows.push(visual.clone());
            }
        }
        rows
    }

    fn frame(&mut self, now: Instant) -> Option<Frame> {
        if self
            .started
            .is_some_and(|start| now.saturating_duration_since(start) >= TRANSITION)
        {
            if self.leaving {
                self.current = None;
                self.leaving = false;
            }
            self.outgoing = None;
            self.started = None;
            self.next = Some(now + DWELL);
        }
        if self.outgoing.is_none() {
            let index = self
                .current
                .as_ref()
                .and_then(|current| self.items.iter().position(|v| v.id == current.id));
            if index.is_none() && !self.items.is_empty() {
                self.select(self.items[0].clone(), now);
            } else if self.enabled
                && !self.hovered
                && !self.popover_open
                && self.items.len() > 1
                && self.next.is_some_and(|next| now >= next)
            {
                let next = (index.unwrap_or(0) + 1) % self.items.len();
                // After suspension advance just once; never catch up skipped rotations.
                self.select(self.items[next].clone(), now);
            }
        }
        let current = self.current.as_ref()?;
        let progress = self
            .started
            .map(|start| {
                now.saturating_duration_since(start).as_secs_f32() / TRANSITION.as_secs_f32()
            })
            .unwrap_or(1.)
            .clamp(0., 1.);
        let accent = self
            .outgoing
            .as_ref()
            .map(|old| lerp_color(old.accent, current.accent, smooth(progress)))
            .unwrap_or(current.accent);
        let next_paint = if self.started.is_some() {
            Some(Duration::from_millis(16))
        } else if self.enabled && !self.hovered && !self.popover_open && self.items.len() > 1 {
            self.next.map(|next| {
                next.saturating_duration_since(now)
                    .max(Duration::from_millis(1))
            })
        } else {
            None
        };
        Some(Frame {
            from: self.outgoing.as_ref().map(Visual::paint_key),
            to: current.paint_key(),
            progress,
            accent,
            next_paint,
            opacity: if self.leaving {
                1. - smooth(progress)
            } else {
                1.
            },
        })
    }
}

pub(super) struct CapsuleMotion {
    state: Rc<RefCell<Carousel>>,
    rows: Vec<(String, El)>,
    dark: bool,
    size: Option<Vector2F>,
    origin: Option<Point>,
    visual_bounds: Option<RectF>,
}

impl CapsuleMotion {
    pub fn new(state: Rc<RefCell<Carousel>>, rows: Vec<(String, El)>, dark: bool) -> Self {
        Self {
            state,
            rows,
            dark,
            size: None,
            origin: None,
            visual_bounds: None,
        }
    }

    fn row_width(&self, id: &str) -> f32 {
        self.rows
            .iter()
            .find(|(key, _)| key == id)
            .and_then(|(_, row)| row.size())
            .map(|size| size.x() + PAD * 2.)
            .unwrap_or(148.)
            .min(self.size.unwrap_or(vec2f(310., HEIGHT)).x())
    }

    fn paint_row(
        &mut self,
        id: &str,
        origin: Vector2F,
        alpha: f32,
        y: f32,
        ctx: &mut PaintContext,
        app: &AppContext,
    ) {
        if alpha <= 0.001 {
            return;
        }
        if let Some((_, row)) = self.rows.iter_mut().find(|(key, _)| key == id) {
            let height = row.size().map(|size| size.y()).unwrap_or(20.);
            let snapshot = ctx.scene.layer_snapshot();
            row.paint(origin + vec2f(PAD, (HEIGHT - height) * 0.5 + y), ctx, app);
            ctx.scene.apply_opacity_since(&snapshot, alpha);
        }
    }

    fn hit(&self, position: Vector2F, ctx: &EventContext) -> bool {
        self.visual_bounds
            .is_some_and(|rect| rect.contains_point(position))
            && self.origin.is_some_and(|origin| {
                !ctx.is_covered(Point::from_vec2f(position, origin.z_index()))
            })
    }
}

impl Element for CapsuleMotion {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let mut width: f32 = 0.;
        for (_, row) in &mut self.rows {
            let size = row.layout(
                SizeConstraint::new(vec2f(0., 0.), vec2f((c.max.x() - PAD * 2.).max(0.), HEIGHT)),
                ctx,
                app,
            );
            width = width.max(size.x() + PAD * 2.);
        }
        // Reserve the widest presentation. Cycling never moves the adjacent toolbar.
        let size = vec2f(
            width.min(c.max.x()).max(c.min.x()),
            HEIGHT.min(c.max.y()).max(c.min.y()),
        );
        self.size = Some(size);
        size
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for (_, row) in &mut self.rows {
            row.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let Some(frame) = self.state.borrow_mut().frame(Instant::now()) else {
            self.visual_bounds = None;
            return;
        };
        let target_width = self.row_width(&frame.to);
        let from_width = frame
            .from
            .as_ref()
            .map(|id| self.row_width(id))
            .unwrap_or(target_width);
        let width = from_width + (target_width - from_width) * smooth(frame.progress);
        let bounds = RectF::new(origin, vec2f(width, HEIGHT));
        self.visual_bounds =
            (frame.opacity >= 1. && !self.state.borrow().leaving).then_some(bounds);
        super::shell::set_capsule_pill_bounds(bounds);
        let state = self.state.borrow();
        let bg = if state.pressed {
            rgba(15, 23, 42, 255)
        } else if state.hovered {
            rgba(28, 39, 62, 246)
        } else if self.dark {
            rgba(24, 33, 52, 245)
        } else {
            rgba(15, 23, 42, 238)
        };
        let accent = frame.accent;
        let border = if state.popover_open {
            ColorU::new(accent.r, accent.g, accent.b, 200)
        } else {
            lerp_color(
                rgba(255, 255, 255, 42),
                ColorU::new(accent.r, accent.g, accent.b, 145),
                0.6,
            )
        };
        drop(state);
        let fading = self.state.borrow().leaving;
        let snapshot = fading.then(|| ctx.scene.layer_snapshot());
        let surface = if fading {
            ctx.scene.draw_rect_without_hit_recording(bounds)
        } else {
            ctx.scene.draw_rect_with_hit_recording(bounds)
        };
        surface
            .with_background(Fill::Solid(bg))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(HEIGHT * 0.5)))
            .with_border(Border::all(1.).with_border_color(border));
        let clip_origin = Point::from_vec2f(origin + vec2f(PAD, 0.), ctx.scene.z_index());
        if let Some(clip) = ctx
            .scene
            .visible_rect(clip_origin, vec2f((width - PAD * 2.).max(0.), HEIGHT))
        {
            ctx.scene.start_layer(ClipBounds::BoundedBy(clip));
            if let Some(from) = &frame.from {
                let old_alpha = 1. - smooth(frame.progress / 0.4);
                let new_alpha = smooth((frame.progress - 0.35) / 0.65);
                self.paint_row(
                    from,
                    origin,
                    old_alpha,
                    -3. * smooth(frame.progress),
                    ctx,
                    app,
                );
                self.paint_row(
                    &frame.to,
                    origin,
                    new_alpha,
                    3. * (1. - smooth(frame.progress)),
                    ctx,
                    app,
                );
            } else {
                self.paint_row(&frame.to, origin, 1., 0., ctx, app);
            }
            ctx.scene.stop_layer();
        }
        if let Some(snapshot) = snapshot {
            ctx.scene.apply_opacity_since(&snapshot, frame.opacity);
        }
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.max_active_z_index()));
        if let Some(delay) = frame.next_paint {
            if delay <= Duration::from_millis(16) {
                facloud_ui::animation::schedule_animation_tick(ctx, 16);
            } else {
                ctx.repaint_after(delay);
            }
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size
    }
    fn origin(&self) -> Option<Point> {
        self.origin
    }
    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        match event.raw_event() {
            Event::MouseMoved { position, .. } => {
                let hit = self.hit(*position, ctx);
                if hit {
                    if let Some(origin) = self.origin {
                        ctx.set_cursor(Cursor::PointingHand, origin.z_index());
                    }
                }
                if self.state.borrow_mut().set_hovered(hit, Instant::now()) {
                    if !hit {
                        ctx.reset_cursor();
                    }
                    ctx.notify();
                }
                false
            }
            Event::LeftMouseDown { position, .. } => {
                let hit = self.hit(*position, ctx);
                self.state.borrow_mut().pressed = hit;
                if hit {
                    ctx.notify();
                }
                hit
            }
            Event::LeftMouseUp { position, .. } => {
                let pressed = std::mem::take(&mut self.state.borrow_mut().pressed);
                if pressed {
                    ctx.notify();
                }
                if pressed && self.hit(*position, ctx) {
                    ctx.dispatch_typed_action(Action::BackScreen(
                        BackScreenAction::ToggleLiveCapsulePopover,
                    ));
                    return true;
                }
                false
            }
            Event::LeftMouseDragged { .. } => self.state.borrow().pressed,
            _ => false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn visual(id: &str) -> Visual {
        Visual {
            id: id.into(),
            revision: None,
            icon: UiIcon::Clock,
            accent: rgb(80, 160, 240),
            domain: id.into(),
            metric: "11:50".into(),
            live: true,
            secondary: vec![],
            total: 2,
        }
    }
    fn carousel(now: Instant) -> Carousel {
        let mut state = Carousel::default();
        state.sync(vec![visual("alarm"), visual("pickup")], true, false, now);
        state
    }

    #[test]
    fn first_item_gets_full_reading_interval_then_continuous_transition() {
        let now = Instant::now();
        let mut state = carousel(now);
        let first = state.frame(now).unwrap();
        assert_eq!(first.to, "alarm");
        assert!(first.from.is_none());
        assert_eq!(first.next_paint, Some(DWELL));
        let start = state.frame(now + DWELL).unwrap();
        assert_eq!(start.from.as_deref(), Some("alarm"));
        assert_eq!(start.to, "pickup");
        assert_eq!(start.progress, 0.);
        let middle = state.frame(now + DWELL + TRANSITION / 2).unwrap();
        assert!((middle.progress - 0.5).abs() < 0.001);
        let settled = state.frame(now + DWELL + TRANSITION).unwrap();
        assert!(settled.from.is_none());
        assert_eq!(settled.next_paint, Some(DWELL));
    }

    #[test]
    fn reordering_and_refreshing_do_not_change_item_or_restart_clock() {
        let now = Instant::now();
        let mut state = carousel(now);
        let mut refreshed = visual("alarm");
        refreshed.metric = "11:51".into();
        state.sync(
            vec![visual("new"), visual("pickup"), refreshed],
            true,
            false,
            now + Duration::from_secs(2),
        );
        assert_eq!(
            state.frame(now + Duration::from_secs(2)).unwrap().to,
            "alarm"
        );
        assert_eq!(state.current.as_ref().unwrap().metric, "11:51");
        assert_eq!(state.frame(now + DWELL).unwrap().to, "new");
    }

    #[test]
    fn hover_and_popover_pause_and_resume_with_full_dwell() {
        let now = Instant::now();
        let mut state = carousel(now);
        state.set_hovered(true, now + Duration::from_secs(1));
        let paused = state.frame(now + Duration::from_secs(60)).unwrap();
        assert_eq!(paused.to, "alarm");
        assert!(paused.next_paint.is_none());
        state.set_hovered(false, now + Duration::from_secs(60));
        assert_eq!(
            state
                .frame(now + Duration::from_secs(60))
                .unwrap()
                .next_paint,
            Some(DWELL)
        );
        state.sync(
            vec![visual("alarm"), visual("pickup")],
            true,
            true,
            now + Duration::from_secs(61),
        );
        assert!(state
            .frame(now + Duration::from_secs(120))
            .unwrap()
            .next_paint
            .is_none());
        state.sync(
            vec![visual("alarm"), visual("pickup")],
            true,
            false,
            now + Duration::from_secs(120),
        );
        assert_eq!(
            state
                .frame(now + Duration::from_secs(120))
                .unwrap()
                .next_paint,
            Some(DWELL)
        );
    }

    #[test]
    fn hovering_during_transition_finishes_instead_of_freezing_half_visible() {
        let now = Instant::now();
        let mut state = carousel(now);
        state.frame(now + DWELL);
        state.set_hovered(true, now + DWELL + Duration::from_millis(100));
        assert!(state
            .frame(now + DWELL + Duration::from_millis(200))
            .unwrap()
            .next_paint
            .is_some());
        let settled = state.frame(now + DWELL + TRANSITION).unwrap();
        assert_eq!(settled.to, "pickup");
        assert!(settled.from.is_none());
        assert!(settled.next_paint.is_none());
    }

    #[test]
    fn removing_current_retains_outgoing_presentation_until_fade_finishes() {
        let now = Instant::now();
        let mut state = carousel(now);
        state.sync(
            vec![visual("pickup")],
            true,
            false,
            now + Duration::from_secs(1),
        );
        let frame = state.frame(now + Duration::from_secs(1)).unwrap();
        assert_eq!(frame.from.as_deref(), Some("alarm"));
        assert_eq!(state.presentations().len(), 2);
        let settled = state
            .frame(now + Duration::from_secs(1) + TRANSITION)
            .unwrap();
        assert!(settled.next_paint.is_none());
        assert_eq!(state.presentations().len(), 1);
    }

    #[test]
    fn fulfillment_revision_crossfades_and_last_item_fades_out() {
        let now = Instant::now();
        let mut state = Carousel::default();
        let mut order = visual("order");
        order.revision = Some("配送中".into());
        let moving = order.paint_key();
        state.sync(vec![order.clone()], true, false, now);
        order.revision = Some("已送达".into());
        let arrived = order.paint_key();
        let updated_at = now + Duration::from_secs(1);
        state.sync(vec![order], true, false, updated_at);
        let mid = state.frame(updated_at + TRANSITION / 2).unwrap();
        assert_eq!(mid.from, Some(moving));
        assert_eq!(mid.to, arrived);
        assert!((mid.progress - 0.5).abs() < 0.001);
        let removed_at = updated_at + TRANSITION;
        state.frame(removed_at);
        state.sync(vec![], true, false, removed_at);
        let mid = state.frame(removed_at + TRANSITION / 2).unwrap();
        assert!((mid.opacity - 0.5).abs() < 0.001);
        assert!(state.frame(removed_at + TRANSITION).is_none());
    }

    #[test]
    fn disabled_motion_and_single_item_have_no_animation_timer() {
        let now = Instant::now();
        let mut state = carousel(now);
        state.frame(now + DWELL);
        state.sync(
            vec![visual("alarm"), visual("pickup")],
            false,
            false,
            now + DWELL,
        );
        let frame = state.frame(now + Duration::from_secs(3600)).unwrap();
        assert_eq!(frame.to, "pickup");
        assert!(frame.from.is_none());
        assert!(frame.next_paint.is_none());
        state.sync(vec![visual("pickup")], true, false, now);
        assert!(state
            .frame(now + Duration::from_secs(3600))
            .unwrap()
            .next_paint
            .is_none());
        state.sync(vec![], false, false, now);
        assert!(state.frame(now).is_none());
    }

    #[test]
    fn resuming_after_long_delay_advances_once_without_catchup() {
        let now = Instant::now();
        let mut state = carousel(now);
        let resumed = state.frame(now + Duration::from_secs(3600)).unwrap();
        assert_eq!(resumed.to, "pickup");
        assert_eq!(resumed.progress, 0.);
        assert_eq!(
            state
                .frame(now + Duration::from_secs(3600) + TRANSITION)
                .unwrap()
                .next_paint,
            Some(DWELL)
        );
    }

    #[test]
    fn width_curve_is_monotonic_with_no_squeeze_or_overshoot() {
        let mut prev = 0.;
        for n in 0..=100 {
            let value = smooth(n as f32 / 100.);
            assert!((prev..=1.).contains(&value));
            prev = value;
        }
        assert_eq!(smooth(0.), 0.);
        assert_eq!(smooth(1.), 1.);
    }
}
