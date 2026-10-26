//! Short, paint-only motion. No per-frame data fetch or hit-region rebuild.
use super::*;
use facloud_ui::animation::schedule_animation_tick;
use facloud_ui::color::ColorU;
use pathfinder_geometry::rect::RectF;
pub(super) fn enter(child: Box<dyn Element>, started: Instant, enabled: bool) -> Box<dyn Element> {
    if !enabled || started.elapsed().as_millis() >= 160 {
        return child;
    }
    Box::new(Entrance {
        child,
        started,
        duration_secs: 0.16,
        base_opacity: 0.76,
    })
}

pub(super) fn sub_enter(
    child: Box<dyn Element>,
    started: Instant,
    enabled: bool,
) -> Box<dyn Element> {
    if !enabled || started.elapsed().as_millis() >= 130 {
        return child;
    }
    Box::new(Entrance {
        child,
        started,
        duration_secs: 0.13,
        base_opacity: 0.82,
    })
}

pub(super) fn entrance_opacity(elapsed_secs: f32, duration_secs: f32, base_opacity: f32) -> f32 {
    let t = (elapsed_secs / duration_secs).clamp(0.0, 1.0);
    let span = 1.0 - base_opacity;
    base_opacity + span * (1.0 - (1.0 - t).powi(3))
}

struct Entrance {
    child: Box<dyn Element>,
    started: Instant,
    duration_secs: f32,
    base_opacity: f32,
}
impl Element for Entrance {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        self.child.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app)
    }
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        let snapshot = ctx.scene.layer_snapshot();
        self.child.paint(origin, ctx, app);
        let elapsed = self.started.elapsed().as_secs_f32();
        let t = (elapsed / self.duration_secs).min(1.);
        if t < 1. {
            ctx.scene.apply_opacity_since(
                &snapshot,
                entrance_opacity(elapsed, self.duration_secs, self.base_opacity),
            );
            schedule_animation_tick(ctx, 16);
        }
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(e, ctx, app)
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}
struct LoadingLine {
    start: Instant,
    animate: bool,
    track: ColorU,
    ink: ColorU,
    size: Option<Vector2F>,
    origin: Option<Point>,
}
impl Element for LoadingLine {
    fn layout(&mut self, c: SizeConstraint, _: &mut LayoutContext, _: &AppContext) -> Vector2F {
        let size = vec2f(c.max.x().min(800.).max(c.min.x()), 3.);
        self.size = Some(size);
        size
    }
    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}
    fn paint(&mut self, o: Vector2F, ctx: &mut PaintContext, _: &AppContext) {
        self.origin = Some(Point::from_vec2f(o, ctx.scene.z_index()));
        let size = self.size.unwrap();
        ctx.scene
            .draw_rect_without_hit_recording(RectF::new(o, size))
            .with_background(Fill::Solid(self.track));
        if self.animate {
            let t = (self.start.elapsed().as_secs_f32() / 1.4) % 1.;
            let x = (1. - (t * std::f32::consts::TAU).cos()) * 0.5 * (size.x() * 0.72);
            ctx.scene
                .draw_rect_without_hit_recording(RectF::new(
                    o + vec2f(x, 0.),
                    vec2f(size.x() * 0.28, 3.),
                ))
                .with_background(Fill::Solid(self.ink));
            schedule_animation_tick(ctx, 32);
        }
    }
    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
    fn size(&self) -> Option<Vector2F> {
        self.size
    }
    fn origin(&self) -> Option<Point> {
        self.origin
    }
}
impl Companion {
    pub(super) fn animations_disabled(&self) -> bool {
        self.settings.value.reduced_motion || self.settings.hidden
    }
    pub(super) fn pulse_t(&self) -> f32 {
        if self.animations_disabled() {
            return 1.0;
        }
        let secs = self.motion_started.elapsed().as_secs_f32();
        0.625 + 0.375 * (secs * std::f32::consts::TAU / 1.6).sin()
    }
    pub(super) fn loading_line(&self) -> Box<dyn Element> {
        let p = self.p();
        Box::new(LoadingLine {
            start: self.job_started,
            animate: !self.animations_disabled(),
            track: p.border,
            ink: p.text3,
            size: None,
            origin: None,
        })
    }
    pub(super) fn loading_feedback(&self, body: Box<dyn Element>) -> Box<dyn Element> {
        // Loading never adds rows above the content. The fixed footer owns feedback.
        body
    }
    pub(super) fn footer_loading(&self) -> bool {
        !self.animations_disabled()
            && (self.initializing
                || self.busy
                || self.chat_waiting()
                || (matches!(self.page, Page::Amoo | Page::Work)
                    && (self.work.running
                        || self.workspace.running.is_some()
                        || self.studio_lab.handle.is_some()))
                || (self.page == Page::Studio && self.studio_lab.handle.is_some())
                || (self.page == Page::Amoo && self.amoo_ui.gui_running)
                || (self.amoo_ui.snapshot_job.is_some() && !self.amoo_gateway_online)
                || (matches!(self.page, Page::Notifications | Page::Sms)
                    && self.push.syncing
                    && !self.push.loaded))
    }
}

/// A bounded paint-only interpolation; no layout rebuild or polling animation.
struct UpdateProgress {
    from: f32,
    to: f32,
    at: Instant,
    animate: bool,
    track: ColorU,
    ink: ColorU,
    size: Option<Vector2F>,
}
impl Element for UpdateProgress {
    fn layout(&mut self, c: SizeConstraint, _: &mut LayoutContext, _: &AppContext) -> Vector2F {
        let size = vec2f(c.max.x().min(800.).max(c.min.x()), 5.);
        self.size = Some(size);
        size
    }
    fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}
    fn paint(&mut self, o: Vector2F, ctx: &mut PaintContext, _: &AppContext) {
        let size = self.size.unwrap();
        let t = if self.animate {
            (self.at.elapsed().as_secs_f32() / 0.24).min(1.)
        } else {
            1.
        };
        let f = self.from + (self.to - self.from) * (1. - (1. - t).powi(3));
        ctx.scene
            .draw_rect_without_hit_recording(RectF::new(o, size))
            .with_background(Fill::Solid(self.track));
        ctx.scene
            .draw_rect_without_hit_recording(RectF::new(o, vec2f(size.x() * f, 5.)))
            .with_background(Fill::Solid(self.ink));
        if t < 1. {
            schedule_animation_tick(ctx, 16);
        }
    }
    fn dispatch_event(
        &mut self,
        _: &DispatchedEvent,
        _: &mut EventContext,
        _: &AppContext,
    ) -> bool {
        false
    }
    fn size(&self) -> Option<Vector2F> {
        self.size
    }
    fn origin(&self) -> Option<Point> {
        None
    }
}
impl Companion {
    pub(super) fn trigger_subtab_switch(&mut self) {
        let now = Instant::now();
        self.sub_motion_started = now;
        self.workspace.panel_entered = now;
    }

    pub(super) fn sub_enter(&self, child: Box<dyn Element>) -> Box<dyn Element> {
        sub_enter(
            child,
            self.sub_motion_started.max(self.motion_started),
            !self.settings.value.reduced_motion && !self.settings.hidden,
        )
    }

    pub(super) fn update_progress(&self) -> Box<dyn Element> {
        let u = &self.desktop_update;
        let p = self.p();
        Box::new(UpdateProgress {
            from: u.fraction_from,
            to: u.fraction,
            at: u.fraction_at,
            animate: !self.settings.value.reduced_motion && !self.settings.hidden,
            track: p.border,
            ink: p.success,
            size: None,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::theme::{lerp_color, rgba, Palette};
    use std::time::Duration;

    struct DummyElement {
        marker: f32,
    }
    impl Element for DummyElement {
        fn layout(&mut self, _: SizeConstraint, _: &mut LayoutContext, _: &AppContext) -> Vector2F {
            vec2f(self.marker, 10.)
        }
        fn after_layout(&mut self, _: &mut AfterLayoutContext, _: &AppContext) {}
        fn paint(&mut self, _: Vector2F, _: &mut PaintContext, _: &AppContext) {}
        fn dispatch_event(
            &mut self,
            _: &DispatchedEvent,
            _: &mut EventContext,
            _: &AppContext,
        ) -> bool {
            false
        }
        fn size(&self) -> Option<Vector2F> {
            Some(vec2f(self.marker, 10.))
        }
        fn origin(&self) -> Option<Point> {
            None
        }
    }

    #[test]
    fn lerp_color_blends_and_clamps_channels() {
        let a = rgba(20, 40, 60, 0);
        let b = rgba(120, 140, 160, 200);
        assert_eq!(lerp_color(a, b, -0.5), a);
        assert_eq!(lerp_color(a, b, 0.0), a);
        assert_eq!(lerp_color(a, b, 0.5), rgba(70, 90, 110, 100));
        assert_eq!(lerp_color(a, b, 1.0), b);
        assert_eq!(lerp_color(a, b, 2.0), b);

        let light = Palette::for_theme(false);
        let dark = Palette::for_theme(true);
        assert_ne!(light.press_color(light.bg), light.bg);
        assert_ne!(dark.press_color(dark.bg), dark.bg);
    }

    #[test]
    fn enter_and_sub_enter_respect_reduced_motion_and_curves() {
        let now = Instant::now();
        let disabled = enter(Box::new(DummyElement { marker: 42. }), now, false);
        assert_eq!(disabled.size(), Some(vec2f(42., 10.)));

        let active = enter(Box::new(DummyElement { marker: 64. }), now, true);
        assert_eq!(active.size(), Some(vec2f(64., 10.)));

        let expired = enter(
            Box::new(DummyElement { marker: 96. }),
            now - Duration::from_millis(200),
            true,
        );
        assert_eq!(expired.size(), Some(vec2f(96., 10.)));

        let sub_disabled = sub_enter(Box::new(DummyElement { marker: 12. }), now, false);
        assert_eq!(sub_disabled.size(), Some(vec2f(12., 10.)));
        let sub_active = sub_enter(Box::new(DummyElement { marker: 24. }), now, true);
        assert_eq!(sub_active.size(), Some(vec2f(24., 10.)));

        assert!((entrance_opacity(0.0, 0.16, 0.76) - 0.76).abs() < 1e-5);
        assert!((entrance_opacity(0.16, 0.16, 0.76) - 1.0).abs() < 1e-5);
        assert!(entrance_opacity(0.08, 0.16, 0.76) > 0.90);

        assert!((entrance_opacity(0.0, 0.13, 0.82) - 0.82).abs() < 1e-5);
        assert!((entrance_opacity(0.13, 0.13, 0.82) - 1.0).abs() < 1e-5);
    }

    #[test]
    fn push_seen_toast_keys_and_work_previews_and_rotations_stay_bounded() {
        use crate::ui::backscreen_state::{BackScreenUiState, MAX_GALLERY_ROTATIONS};
        use crate::ui::expansion_state::{ExpansionUi, MAX_MEDIA_ROTATIONS};
        use crate::ui::push_state::{PushUiState, MAX_SEEN_TOAST_KEYS};
        use crate::ui::work_state::{
            WorkUiState, IDLE_WORK_IMAGE_PREVIEWS, MAX_WORK_IMAGE_PREVIEWS,
        };
        use stepx_companion::push::PushSnapshot;

        // 1. Push seen_toast_keys bounded at 2048
        let mut push = PushUiState::default();
        let snap = PushSnapshot::default();
        for i in 0..2500 {
            push.insert_seen_toast_key(format!("toast-key-{i}"), &snap);
        }
        assert!(push.seen_toast_keys.len() <= MAX_SEEN_TOAST_KEYS);
        assert_eq!(push.seen_toast_keys.len(), 2048);

        for i in 2500..3000 {
            push.seen_toast_keys.insert(format!("raw-key-{i}"));
        }
        push.cap_seen_toast_keys(&snap);
        assert!(push.seen_toast_keys.len() <= MAX_SEEN_TOAST_KEYS);

        // 2. Work image_previews bounded at 16 and trimmed to <= 4 on idle
        let mut work = WorkUiState::default();
        work.image_viewer = Some("pinned-preview".into());
        work.insert_image_preview("pinned-preview".into(), PathBuf::from("/tmp/pinned.png"));
        for i in 0..32 {
            work.insert_image_preview(
                format!("preview-{i}"),
                PathBuf::from(format!("/tmp/p-{i}.png")),
            );
        }
        assert!(work.image_previews.len() <= MAX_WORK_IMAGE_PREVIEWS);
        assert_eq!(work.image_previews.len(), 16);
        assert!(work.image_previews.contains_key("pinned-preview"));

        work.preview_errors.insert("err-1".into(), "failed".into());
        work.trim_idle_previews();
        assert!(work.image_previews.len() <= IDLE_WORK_IMAGE_PREVIEWS);
        assert!(work.image_previews.contains_key("pinned-preview"));
        assert!(work.preview_errors.is_empty());

        // 3. Expansion media_rotations and BackScreen gallery_rotations bounded at 256
        let mut exp = ExpansionUi::default();
        exp.selected_media_id = Some("keep-media".into());
        exp.insert_media_rotation("keep-media".into(), 90);
        for i in 0..320 {
            exp.insert_media_rotation(format!("media-{i}"), 180);
        }
        assert!(exp.media_rotations.len() <= MAX_MEDIA_ROTATIONS);
        assert_eq!(exp.media_rotations.len(), 256);
        assert_eq!(exp.media_rotations.get("keep-media"), Some(&90));

        let mut bs = BackScreenUiState::default();
        for i in 0..320 {
            bs.insert_gallery_rotation(format!("/sdcard/DCIM/img_{i}.jpg"), 90);
        }
        assert!(bs.gallery_rotations.len() <= MAX_GALLERY_ROTATIONS);
        assert_eq!(bs.gallery_rotations.len(), 256);
    }
}
