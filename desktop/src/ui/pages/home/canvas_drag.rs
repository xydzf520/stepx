#![allow(unused_imports)]
use crate::ui::{
    amoo_ui_state::AmooUiAction, backscreen_state::BackScreenAction, icons::UiIcon,
    led_grid::LedGrid, theme::*, widgets::*, work_state::WorkAction, Action, Companion, Page,
    PushAction, ToolPage, F_ALARM_TIME, F_AMOO, F_AMOO_GUI_APP, F_BACK_MARQUEE, F_CALENDAR_TIME,
    F_HOME_COMMAND, F_OV_TEXT, F_PHONE_SCHEDULE_TITLE, F_PUSH_REPLY, F_SMS_BODY, F_SMS_TO,
    F_WORK_PROMPT,
};
use super::*;
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, Timelike};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor, ViewContext};
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use stepx_companion::{
    device::default_express_parcels,
    home_hub::{
        build_home_hub_report, build_selected_date_panorama, build_weekly_overview_report,
        quick_replies_for_message_with_context, resolve_top_app_display_name, DesktopDateEntry,
        HomeAiIntent, HomeAiRecommendation, HomeCardKind, HomeHubInput, HomeLifeDomain,
        SelectedDateEntryKind,
    },
    home_visuals::{
        day_dial, donut, parcel_progress, ring_gauge, DayDial, DayTimeline, DialAlarm, DialEvent,
        Paint, WeeklyBarDay,
    },
    push::{demo_push_snapshot, ImChannel, PushSnapshot},
};

/// Live mouse pointer operation on `Page::Home` (drag-and-drop reorder or drag-resize).
#[derive(Clone, Debug, PartialEq)]
pub enum HomePointerOp {
    DragModule {
        source: HomeModuleId,
        start_pos: Vector2F,
        current_pos: Vector2F,
        hover_target: Option<HomeModuleId>,
        moved: bool,
    },
    ResizeModule {
        module: HomeModuleId,
        start_pos: Vector2F,
        current_pos: Vector2F,
        start_rect: RectF,
        start_row_width: f32,
        initial_width: HomeModuleWidth,
        initial_height: HomeModuleHeight,
        preview_width: HomeModuleWidth,
        preview_height: HomeModuleHeight,
        moved: bool,
    },
}

/// Shared hit-rect and pointer-drag registry for `Page::Home` modules.
#[derive(Clone, Debug, Default)]
pub struct HomePointerRegistry {
    pub card_rects: std::collections::BTreeMap<HomeModuleId, RectF>,
    pub drag_handle_rects: std::collections::BTreeMap<HomeModuleId, RectF>,
    pub resize_handle_rects: std::collections::BTreeMap<HomeModuleId, RectF>,
    pub reminder_rects: Vec<(String, RectF, HomeReminderContextTarget)>,
    pub last_canvas_post_z: Option<facloud_ui::elements::ZIndex>,
    pub active_op: Option<HomePointerOp>,
    pub anim_frame_count: u64,
    pub anim_last_phase: f32,
    pub last_anim_tick: Option<std::time::Instant>,
    pub layout_signature: Vec<(HomeModuleId, HomeModuleWidth, HomeModuleHeight)>,
    pub reflow_from: std::collections::BTreeMap<HomeModuleId, RectF>,
    pub reflow_started: Option<std::time::Instant>,
    /// The visible layout stays a draft until the user chooses Done.
    pub edit_original: Option<HomeModuleLayoutConfig>,
    pub edit_undo: Vec<HomeModuleLayoutConfig>,
    pub module_library_open: bool,
    /// Keep layout feedback beside its controls instead of letting background
    /// synchronization overwrite the user's confirmation in the status bar.
    pub edit_feedback: Option<String>,
    pub viewport: Option<Vector2F>,
    pub canvas_width: Option<f32>,
    pub pointer_redraw_due: Option<std::time::Instant>,
}

impl HomePointerRegistry {
    pub fn begin_edit(&mut self, layout: &HomeModuleLayoutConfig) {
        if self.edit_original.is_none() {
            self.edit_original = Some(layout.clone());
            self.edit_undo.clear();
            self.module_library_open = false;
            self.edit_feedback = None;
        }
    }

    pub fn animate_from_current(&mut self, animate: bool) {
        self.reflow_started = (animate && !self.card_rects.is_empty())
            .then(std::time::Instant::now);
        self.reflow_from = if self.reflow_started.is_some() {
            self.card_rects.clone()
        } else { Default::default() };
    }

    pub fn finish_edit(&mut self) {
        self.edit_original = None;
        self.edit_undo.clear();
        self.module_library_open = false;
        self.edit_feedback = None;
        self.active_op = None;
        self.pointer_redraw_due = None;
    }

    pub fn record_edit(&mut self, before: HomeModuleLayoutConfig, after: &HomeModuleLayoutConfig) {
        if self.edit_original.is_some() && before != *after {
            self.edit_undo.push(before);
            if self.edit_undo.len() > 64 { self.edit_undo.remove(0); }
        }
    }

    /// Window resizing changes both the grid and content density. A pointer
    /// gesture based on the previous geometry must not resize a different card.
    pub fn observe_viewport(&mut self, viewport: Vector2F) {
        if self.viewport.is_some_and(|previous| (previous - viewport).length() > 1.) {
            self.active_op = None;
            self.pointer_redraw_due = None;
            self.reflow_started = None;
            self.reflow_from.clear();
        }
        self.viewport = Some(viewport);
    }

    fn observe_canvas_geometry(&mut self, viewport: Vector2F, width: f32) {
        self.observe_viewport(viewport);
        if width.is_finite() && width > 0. {
            if self.canvas_width.is_some_and(|previous| (previous - width).abs() > 1.) {
                self.active_op = None;
                self.pointer_redraw_due = None;
                self.reflow_started = None;
                self.reflow_from.clear();
            }
            self.canvas_width = Some(width);
        }
    }

    fn notify_pointer_motion(&mut self, ctx: &mut EventContext) {
        // Native pointer events can arrive far faster than display frames. Keep
        // the latest position, with one pending view update for the next frame.
        if self.pointer_redraw_due.is_some_and(|due| due > std::time::Instant::now()) {
            return;
        }
        let (_, due) = ctx.notify_after(std::time::Duration::from_millis(16));
        self.pointer_redraw_due = Some(due);
    }

    fn shares_row(&self, target: HomeModuleId) -> bool {
        let Some(rect) = self.card_rects.get(&target) else { return false; };
        self.card_rects.iter().any(|(id, other)| {
            *id != target
                && (other.origin().y() - rect.origin().y()).abs() < 16.
                && (other.max_x() <= rect.origin().x() || other.origin().x() >= rect.max_x())
        })
    }

    fn content_width(&self) -> f32 {
        if let Some(width) = self.canvas_width.filter(|width| width.is_finite() && *width > 0.) {
            return width;
        }
        let left = self.card_rects.values().map(|rect| rect.origin().x()).reduce(f32::min);
        let right = self.card_rects.values().map(|rect| rect.max_x()).reduce(f32::max);
        left.zip(right).map(|(left, right)| (right - left).max(120.)).unwrap_or(480.)
    }

    /// A horizontal row uses left/right insertion. Full rows use top/bottom.
    pub fn drop_after(&self, target: HomeModuleId, pos: Vector2F) -> bool {
        let Some(rect) = self.card_rects.get(&target) else { return false; };
        if self.shares_row(target) {
            pos.x() >= rect.origin().x() + rect.size().x() * 0.5
        } else {
            pos.y() >= rect.origin().y() + rect.size().y() * 0.5
        }
    }

    pub(in crate::ui) fn observe_layout(
        &mut self,
        signature: Vec<(HomeModuleId, HomeModuleWidth, HomeModuleHeight)>,
        animate: bool,
    ) {
        if self.layout_signature != signature {
            let reordered = !self.layout_signature.is_empty();
            self.layout_signature = signature;
            self.reflow_started = (animate && reordered && !self.card_rects.is_empty())
                .then(std::time::Instant::now);
            self.reflow_from = if self.reflow_started.is_some() {
                self.card_rects.clone()
            } else { Default::default() };
        }
        if !animate {
            self.reflow_started = None;
            self.reflow_from.clear();
        }
    }

    #[inline]
    pub fn rect_contains_with_pad(rect: RectF, pos: Vector2F, pad: f32) -> bool {
        pos.x() >= rect.origin().x() - pad
            && pos.x() <= rect.max_x() + pad
            && pos.y() >= rect.origin().y() - pad
            && pos.y() <= rect.max_y() + pad
    }

    pub fn hit_reminder(&self, pos: Vector2F) -> Option<HomeReminderContextTarget> {
        self.reminder_rects
            .iter()
            .rev()
            .find(|(_, rect, _)| Self::rect_contains_with_pad(*rect, pos, 2.0))
            .map(|(_, _, target)| target.clone())
    }

    pub fn hit_module_card(&self, pos: Vector2F) -> Option<HomeModuleId> {
        self.card_rects
            .iter()
            .find(|(_, &rect)| Self::rect_contains_with_pad(rect, pos, 0.0))
            .map(|(&id, _)| id)
    }

    pub fn hit_resize_handle(&self, pos: Vector2F) -> Option<HomeModuleId> {
        if let Some((&id, _)) = self
            .resize_handle_rects
            .iter()
            .find(|(_, &rect)| Self::rect_contains_with_pad(rect, pos, 4.0))
        {
            return Some(id);
        }
        self.card_rects
            .iter()
            .find(|(_, &rect)| {
                let br_grip = RectF::new(
                    vec2f(rect.max_x() - 32.0, rect.max_y() - 28.0),
                    vec2f(36.0, 32.0),
                );
                Self::rect_contains_with_pad(br_grip, pos, 2.0)
            })
            .map(|(&id, _)| id)
    }

    pub fn resolve_drop_target(&self, source: HomeModuleId, pos: Vector2F) -> Option<HomeModuleId> {
        // Returning to the original card cancels the move, even if a neighbour
        // lies within the gap tolerance. Never commit a stale hover target.
        if self.card_rects.get(&source).is_some_and(|rect| Self::rect_contains_with_pad(*rect, pos, 0.)) {
            return None;
        }
        if let Some((&id, _)) = self
            .card_rects
            .iter()
            .find(|(&id, &rect)| id != source && Self::rect_contains_with_pad(rect, pos, 0.0))
        {
            return Some(id);
        }
        let mut best: Option<(HomeModuleId, f32)> = None;
        for (&id, &rect) in &self.card_rects {
            if id == source {
                continue;
            }
            let cx = pos.x().clamp(rect.origin().x(), rect.max_x());
            let cy = pos.y().clamp(rect.origin().y(), rect.max_y());
            let dist = (pos - vec2f(cx, cy)).length();
            if dist <= 60.0 && best.map_or(true, |(_, d)| dist < d) {
                best = Some((id, dist));
            }
        }
        best.map(|(id, _)| id)
    }

    pub fn compute_resized_dimensions(
        initial_width: HomeModuleWidth,
        initial_height: HomeModuleHeight,
        dx: f32,
        dy: f32,
    ) -> (HomeModuleWidth, HomeModuleHeight) {
        Self::compute_resized_dimensions_for_geometry(initial_width, initial_height,
            vec2f(480., 320.), 480., vec2f(dx, dy))
    }

    pub fn compute_resized_dimensions_for_geometry(
        initial_width: HomeModuleWidth,
        initial_height: HomeModuleHeight,
        start_size: Vector2F,
        row_width: f32,
        delta: Vector2F,
    ) -> (HomeModuleWidth, HomeModuleHeight) {
        let row_width = if row_width.is_finite() { row_width.max(120.) } else { 480. };
        let requested_units = initial_width.units() as f32 + delta.x() * 12. / row_width;
        let new_w = HomeModuleWidth::ALL.into_iter().min_by(|a, b| {
            (a.units() as f32 - requested_units).abs()
                .total_cmp(&(b.units() as f32 - requested_units).abs())
        }).unwrap_or(initial_width);
        let base_h: isize = match initial_height {
            HomeModuleHeight::Compact => 0,
            HomeModuleHeight::Standard => 1,
            HomeModuleHeight::Tall => 2,
            HomeModuleHeight::Auto => 1,
        };
        let height_step = (start_size.y() * 0.28).clamp(64., 168.);
        let step_y = (delta.y() / height_step).round().clamp(-2., 2.) as isize;
        let new_h = if step_y == 0 { initial_height } else {
            HomeModuleHeight::ALL[(base_h + step_y).clamp(0, 2) as usize]
        };

        (new_w, new_h)
    }
}

#[derive(Clone, Debug)]
pub enum HomeBoundsTarget {
    Card(HomeModuleId),
    DragHandle(HomeModuleId),
    ResizeHandle(HomeModuleId),
    Reminder(String, HomeReminderContextTarget),
}

pub(in crate::ui) struct HomeBoundsRecorder {
    pub(in crate::ui) child: El,
    pub(in crate::ui) target: HomeBoundsTarget,
    pub(in crate::ui) registry: std::rc::Rc<std::cell::RefCell<HomePointerRegistry>>,
}

impl HomeBoundsRecorder {
    fn record_rect(&self, rect: RectF) {
        let mut reg = self.registry.borrow_mut();
        match &self.target {
            HomeBoundsTarget::Card(id) => {
                reg.card_rects.insert(*id, rect);
            }
            HomeBoundsTarget::DragHandle(id) => {
                reg.drag_handle_rects.insert(*id, rect);
            }
            HomeBoundsTarget::ResizeHandle(id) => {
                reg.resize_handle_rects.insert(*id, rect);
            }
            HomeBoundsTarget::Reminder(slug, target) => {
                if let Some(existing) = reg
                    .reminder_rects
                    .iter_mut()
                    .find(|(s, _, _)| s == slug)
                {
                    existing.1 = rect;
                    existing.2 = target.clone();
                } else {
                    reg.reminder_rects
                        .push((slug.clone(), rect, target.clone()));
                }
            }
        }
    }
}

impl facloud_ui::Element for HomeBoundsRecorder {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        self.child.layout(constraint, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.child.after_layout(ctx, app);
        if let Some(sz) = self.child.size() {
            let orig = self
                .child
                .origin()
                .map(|p| vec2f(p.x(), p.y()))
                .unwrap_or_else(Vector2F::zero);
            self.record_rect(RectF::new(orig, sz));
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        if let Some(sz) = self.child.size() {
            self.record_rect(RectF::new(origin, sz));
        }
        self.child.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &facloud_ui::event::DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}

fn home_reflow_offset(delta: Vector2F, elapsed: std::time::Duration) -> Vector2F {
    let progress = (elapsed.as_secs_f32() / 0.22).clamp(0., 1.);
    delta * (1. - progress).powi(3)
}

pub(in crate::ui) struct HomeInteractiveCanvas {
    pub(in crate::ui) child: El,
    pub(in crate::ui) floating_badge: Option<El>,
    pub(in crate::ui) editing: bool,
    pub(in crate::ui) registry: std::rc::Rc<std::cell::RefCell<HomePointerRegistry>>,
    pub(in crate::ui) module_sizes: std::collections::BTreeMap<HomeModuleId, (HomeModuleWidth, HomeModuleHeight)>,
    pub(in crate::ui) accent: ColorU,
    pub(in crate::ui) surface: ColorU,
    pub(in crate::ui) size: Option<Vector2F>,
    pub(in crate::ui) origin: Option<Point>,
}

impl HomeInteractiveCanvas {
    fn contains_visible_pointer(&self, position: Vector2F, ctx: &EventContext) -> bool {
        !self.registry.borrow().last_canvas_post_z
            .is_some_and(|z| ctx.is_covered(Point::from_vec2f(position, z)))
            && self.origin.zip(self.size).and_then(|(origin, size)| ctx.visible_rect(origin, size))
            .is_some_and(|rect| HomePointerRegistry::rect_contains_with_pad(rect, position, 0.))
    }
}

impl facloud_ui::Element for HomeInteractiveCanvas {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        // Layout runs on native resize before the next business tick. Use its
        // actual viewport, including height, rather than the cached workspace.
        self.registry.borrow_mut().observe_canvas_geometry(ctx.window_size, constraint.max.x());
        let sz = self.child.layout(constraint, ctx, app);
        if let Some(badge) = &mut self.floating_badge {
            let _ = badge.layout(
                SizeConstraint::new(Vector2F::zero(), vec2f(420.0, 48.0)),
                ctx,
                app,
            );
        }
        self.size = Some(sz);
        sz
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.child.after_layout(ctx, app);
        if let Some(badge) = &mut self.floating_badge {
            badge.after_layout(ctx, app);
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        {
            let mut registry = self.registry.borrow_mut();
            // Enabled modules can temporarily have no content. Hit testing must
            // reflect this frame, never a hidden card's previous position.
            registry.card_rects.clear();
            registry.drag_handle_rects.clear();
            registry.resize_handle_rects.clear();
            registry.reminder_rects.clear();
        }
        self.child.paint(origin, ctx, app);
        let reg_snapshot = self.registry.borrow();
        match reg_snapshot.active_op.clone() {
            Some(HomePointerOp::DragModule {
                source: _,
                current_pos,
                hover_target,
                moved: true,
                ..
            }) => {
                if let Some(target_id) = hover_target {
                    if let Some(&target_rect) = reg_snapshot.card_rects.get(&target_id) {
                        let after = reg_snapshot.drop_after(target_id, current_pos);
                        let marker = if reg_snapshot.shares_row(target_id) {
                            RectF::new(vec2f(if after { target_rect.max_x() + 3. } else { target_rect.origin().x() - 6. }, target_rect.origin().y()),
                                vec2f(3., target_rect.size().y()))
                        } else {
                            RectF::new(vec2f(target_rect.origin().x(), if after { target_rect.max_y() + 3. } else { target_rect.origin().y() - 6. }),
                                vec2f(target_rect.size().x(), 3.))
                        };
                        ctx.scene
                            .draw_rect_without_hit_recording(marker)
                            .with_background(Fill::Solid(self.accent))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(1.5)));
                    }
                }
                let badge_w = self
                    .floating_badge
                    .as_ref()
                    .and_then(|b| b.size())
                    .map(|s| s.x())
                    .unwrap_or(184.0);
                let max_x = self
                    .size
                    .map(|sz| origin.x() + sz.x() - badge_w - 8.0)
                    .unwrap_or(current_pos.x() + 12.0);
                let ghost_x = (current_pos.x() + 12.0).min(max_x).max(origin.x() + 6.0);
                let ghost_origin = vec2f(ghost_x, current_pos.y() - 18.0);
                if let Some(badge) = &mut self.floating_badge {
                    badge.paint(ghost_origin, ctx, app);
                } else {
                    let ghost_size = vec2f(184.0, 34.0);
                    ctx.scene
                        .draw_rect_without_hit_recording(RectF::new(
                            ghost_origin + vec2f(0., 4.),
                            ghost_size,
                        ))
                        .with_background(Fill::Solid(rgba(0, 0, 0, 41)))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)));
                    ctx.scene
                        .draw_rect_without_hit_recording(RectF::new(ghost_origin, ghost_size))
                        .with_background(Fill::Solid(with_alpha_f(self.surface, 0.95)))
                        .with_border(Border::all(1.8).with_border_color(self.accent))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)));
                }
            }
            Some(HomePointerOp::ResizeModule {
                module,
                current_pos,
                start_pos,
                start_rect,
                moved: true,
                ..
            }) => {
                if reg_snapshot.card_rects.contains_key(&module) {
                    let max_width = self.size.map(|size| origin.x() + size.x() - start_rect.origin().x()).unwrap_or(f32::MAX);
                    let preview_size = vec2f(
                        (start_rect.size().x() + current_pos.x() - start_pos.x()).min(max_width).max(120.),
                        (start_rect.size().y() + current_pos.y() - start_pos.y()).max(120.),
                    );
                    ctx.scene
                        .draw_rect_without_hit_recording(RectF::new(
                            start_rect.origin() - vec2f(2., 2.),
                            preview_size + vec2f(4., 4.),
                        ))
                        .with_background(Fill::Solid(with_alpha_f(self.accent, 0.06)))
                        .with_border(Border::all(2.2).with_border_color(self.accent))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_LG + 2.)));
                }
                let badge_origin = current_pos + vec2f(-68.0, -28.0);
                if let Some(badge) = &mut self.floating_badge {
                    badge.paint(badge_origin, ctx, app);
                } else {
                    ctx.scene
                        .draw_rect_without_hit_recording(RectF::new(
                            badge_origin,
                            vec2f(84.0, 24.0),
                        ))
                        .with_background(Fill::Solid(with_alpha_f(self.surface, 0.94)))
                        .with_border(Border::all(1.5).with_border_color(self.accent))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)));
                }
            }
            _ => {}
        }
        drop(reg_snapshot);
        // Cards and the preview badge can paint their own child layers. The
        // active layer returns to this canvas afterwards; using that index
        // would mistake another card's handle for an overlay covering a drop.
        // Include all canvas layers, while later external overlays still block.
        self.registry.borrow_mut().last_canvas_post_z = Some(ctx.scene.max_active_z_index());
    }

    fn size(&self) -> Option<Vector2F> {
        self.size.or_else(|| self.child.size())
    }

    fn origin(&self) -> Option<Point> {
        self.origin.or_else(|| self.child.origin())
    }

    fn dispatch_event(
        &mut self,
        event: &facloud_ui::event::DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        match event.raw_event() {
            Event::LeftMouseDown { position, .. } => {
                // Explicit handles use the same painted hit testing as buttons.
                // The canvas only falls back to the card's corner resize grip.
                if self.child.dispatch_event(event, ctx, app) {
                    return true;
                }
                let is_covered = self
                    .registry
                    .borrow()
                    .last_canvas_post_z
                    .is_some_and(|post_z| ctx.is_covered(Point::from_vec2f(*position, post_z)));
                if self.editing && !is_covered && self.contains_visible_pointer(*position, ctx) {
                    let hit_resize = self.registry.borrow().hit_resize_handle(*position);
                    if let Some(mod_id) = hit_resize {
                        let (init_w, init_h) = self
                            .module_sizes
                            .get(&mod_id)
                            .copied()
                            .unwrap_or_else(|| (mod_id.default_width(), mod_id.default_height()));
                        let start_rect = self
                            .registry
                            .borrow()
                            .card_rects
                            .get(&mod_id)
                            .copied()
                            .unwrap_or_else(|| RectF::new(*position, vec2f(320., 220.)));
                        let canvas_post_z = self.registry.borrow().last_canvas_post_z;
                        let start_row_width = self.registry.borrow().content_width();
                        self.registry.borrow_mut().active_op = Some(HomePointerOp::ResizeModule {
                            module: mod_id,
                            start_pos: *position,
                            current_pos: *position,
                            start_rect,
                            start_row_width,
                            initial_width: init_w,
                            initial_height: init_h,
                            preview_width: init_w,
                            preview_height: init_h,
                            moved: false,
                        });
                        if let Some(z) = canvas_post_z {
                            ctx.set_cursor(Cursor::ResizeUpDown, z);
                        }
                        ctx.dispatch_typed_action(Action::HomeModule(
                            HomeModuleAction::BeginResize(mod_id),
                        ));
                        ctx.notify();
                        return true;
                    }

                }
                return false;
            }
            Event::LeftMouseDragged { position, .. }
            | Event::MouseMoved {
                position,
                is_synthetic: false,
                ..
            } => {
                let (active, canvas_post_z) = {
                    let reg = self.registry.borrow();
                    (reg.active_op.clone(), reg.last_canvas_post_z)
                };
                match active {
                    Some(HomePointerOp::DragModule {
                        source,
                        start_pos,
                        hover_target: prev_target,
                        moved: prev_moved,
                        ..
                    }) => {
                        let moved = prev_moved || (*position - start_pos).length() >= 4.0;
                        let new_target = if moved && self.contains_visible_pointer(*position, ctx) {
                            self.registry.borrow().resolve_drop_target(source, *position)
                        } else {
                            None
                        };
                        self.registry.borrow_mut().active_op = Some(HomePointerOp::DragModule {
                            source,
                            start_pos,
                            current_pos: *position,
                            hover_target: new_target,
                            moved,
                        });
                        if let Some(z) = canvas_post_z {
                            ctx.set_cursor(Cursor::ClosedHand, z);
                        }
                        if new_target != prev_target || moved != prev_moved {
                            ctx.dispatch_typed_action(Action::HomeModule(
                                HomeModuleAction::UpdateDrag {
                                    source,
                                    hover_target: new_target,
                                },
                            ));
                        }
                        self.registry.borrow_mut().notify_pointer_motion(ctx);
                        return true;
                    }
                    Some(HomePointerOp::ResizeModule {
                        module,
                        start_pos,
                        start_rect,
                        start_row_width,
                        initial_width,
                        initial_height,
                        moved: prev_moved,
                        ..
                    }) => {
                        let delta = *position - start_pos;
                        let moved = prev_moved || delta.length() >= 4.0;
                        let (new_w, new_h) = if moved {
                            HomePointerRegistry::compute_resized_dimensions_for_geometry(
                                initial_width,
                                initial_height,
                                start_rect.size(),
                                start_row_width,
                                delta,
                            )
                        } else {
                            (initial_width, initial_height)
                        };
                        self.registry.borrow_mut().active_op = Some(HomePointerOp::ResizeModule {
                            module,
                            start_pos,
                            current_pos: *position,
                            start_rect,
                            start_row_width,
                            initial_width,
                            initial_height,
                            preview_width: new_w,
                            preview_height: new_h,
                            moved,
                        });
                        if let Some(z) = canvas_post_z {
                            ctx.set_cursor(Cursor::ResizeUpDown, z);
                        }
                        // Preview only: no layout mutation, disk write or reflow
                        // until the pointer is released.
                        self.registry.borrow_mut().notify_pointer_motion(ctx);
                        return true;
                    }
                    None => {}
                }
            }
            Event::LeftMouseUp { position, .. } => {
                self.registry.borrow_mut().pointer_redraw_due = None;
                let op = self.registry.borrow_mut().active_op.take();
                match op {
                    Some(HomePointerOp::DragModule {
                        source,
                        start_pos,
                        moved,
                        ..
                    }) => {
                        let did_move = moved || (*position - start_pos).length() >= 4.0;
                        if did_move {
                            let final_target = self.contains_visible_pointer(*position, ctx)
                                .then(|| self.registry.borrow().resolve_drop_target(source, *position))
                                .flatten();
                            if let Some(target) = final_target.filter(|&t| t != source) {
                                let after = self.registry.borrow().drop_after(target, *position);
                                ctx.dispatch_typed_action(Action::HomeModule(
                                    HomeModuleAction::CommitDragInsert(source, target, after),
                                ));
                            } else {
                                ctx.dispatch_typed_action(Action::HomeModule(
                                    HomeModuleAction::CancelDrag,
                                ));
                            }
                        } else {
                            ctx.dispatch_typed_action(Action::HomeModule(
                                HomeModuleAction::ToggleClickMove(source),
                            ));
                        }
                        ctx.notify();
                        return true;
                    }
                    Some(HomePointerOp::ResizeModule {
                        module,
                        start_pos,
                        start_rect,
                        start_row_width,
                        initial_width,
                        initial_height,
                        preview_width,
                        preview_height,
                        moved,
                        ..
                    }) => {
                        let delta = *position - start_pos;
                        let did_move = moved || delta.length() >= 4.0;
                        if did_move {
                            let (final_w, final_h) =
                                HomePointerRegistry::compute_resized_dimensions_for_geometry(
                                    initial_width,
                                    initial_height,
                                    start_rect.size(),
                                    start_row_width,
                                    delta,
                                );
                            let _ = (preview_width, preview_height);
                            ctx.dispatch_typed_action(Action::HomeModule(
                                HomeModuleAction::CommitResize(module, final_w, final_h),
                            ));
                        } else {
                            ctx.dispatch_typed_action(Action::ContextOpen(
                                crate::ui::context_menu::Target::HomeModule(module),
                                *position,
                            ));
                        }
                        ctx.notify();
                        return true;
                    }
                    None => {}
                }
            }
            Event::RightMouseDown { position, .. } => {
                let is_covered = self
                    .registry
                    .borrow()
                    .last_canvas_post_z
                    .is_some_and(|post_z| ctx.is_covered(Point::from_vec2f(*position, post_z)));
                if self.child.dispatch_event(event, ctx, app) {
                    return true;
                }
                if !is_covered && self.contains_visible_pointer(*position, ctx) {
                    let hit_rem = self.registry.borrow().hit_reminder(*position);
                    let target = if let Some(rem) = hit_rem {
                        crate::ui::context_menu::Target::HomeReminder(rem)
                    } else if let Some(mod_id) = self.registry.borrow().hit_module_card(*position) {
                        crate::ui::context_menu::Target::HomeModule(mod_id)
                    } else {
                        crate::ui::context_menu::Target::HomeCanvas
                    };
                    ctx.dispatch_typed_action(Action::ContextOpen(target, *position));
                    ctx.notify();
                    return true;
                }
                return false;
            }
            _ => {}
        }
        self.child.dispatch_event(event, ctx, app)
    }
}

/// Custom animated frame wrapper for `Page::Home` module cards during edit/move/drag mode.
pub(in crate::ui) struct HomeEditAnimatedCard {
    pub(in crate::ui) child: El,
    pub(in crate::ui) rank_idx: usize,
    pub(in crate::ui) started: std::time::Instant,
    pub(in crate::ui) reflow: Option<(std::time::Instant, RectF)>,
    pub(in crate::ui) animate: bool,
    pub(in crate::ui) accent: ColorU,
    pub(in crate::ui) is_active_source: bool,
    pub(in crate::ui) is_drop_target: bool,
    pub(in crate::ui) registry: std::rc::Rc<std::cell::RefCell<HomePointerRegistry>>,
    pub(in crate::ui) size: Option<Vector2F>,
    pub(in crate::ui) origin: Option<Point>,
}

impl facloud_ui::Element for HomeEditAnimatedCard {
    fn layout(
        &mut self,
        constraint: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        let sz = self.child.layout(constraint, ctx, app);
        self.size = Some(sz);
        sz
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        // Only a user layout change starts this short movement. New messages,
        // fresh files and data refreshes never replay an entrance animation.
        let offset = self.reflow.filter(|(started, _)| self.animate
            && self.registry.borrow().reflow_started == Some(*started)).map(|(started, previous)| {
            let t = (started.elapsed().as_secs_f32() / 0.22).clamp(0., 1.);
            if t < 1. { ctx.repaint_after(ctx.display_aligned_interval(std::time::Duration::from_millis(16))); }
            home_reflow_offset(previous.origin() - origin, started.elapsed())
        }).unwrap_or_else(|| vec2f(0., 0.));
        let anim_origin = origin + offset;

        self.origin = Some(Point::from_vec2f(anim_origin, ctx.scene.z_index()));
        self.child.paint(anim_origin, ctx, app);

        if self.is_active_source || self.is_drop_target {
            if let Some(sz) = self.size.or_else(|| self.child.size()) {
                // Clean soft-blue highlight ring only for the active drag/move source or drop target
                let ring_color = rgb(0x3B, 0x82, 0xF6);
                let border_w = if self.is_active_source { 2.0 } else { 1.5 };
                let border_alpha = if self.is_active_source { 0.85 } else { 0.55 };
                ctx.scene
                    .draw_rect_without_hit_recording(RectF::new(
                        anim_origin - vec2f(1., 1.),
                        sz + vec2f(2., 2.),
                    ))
                    .with_border(
                        Border::all(border_w)
                            .with_border_color(with_alpha_f(ring_color, border_alpha)),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)));
            }
        }

        if self.rank_idx == 0 {
            let mut reg = self.registry.borrow_mut();
            reg.anim_frame_count = reg.anim_frame_count.saturating_add(1);
            reg.anim_last_phase = if offset.length() > 0.1 { 1. } else { 0. };
        }
    }

    fn size(&self) -> Option<Vector2F> {
        self.size.or_else(|| self.child.size())
    }

    fn origin(&self) -> Option<Point> {
        self.origin.or_else(|| self.child.origin())
    }

    fn dispatch_event(
        &mut self,
        event: &facloud_ui::event::DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}


impl Companion {
    pub(super) fn home_toolbar_weather_label(&self) -> String {
        if !self.weather_snapshot.from_live_api && !self.demo_mode {
            "天气待更新".into()
        } else {
            format!("{} {}°C {}", self.weather_snapshot.city,
                self.weather_snapshot.temp_c, self.weather_snapshot.condition)
        }
    }

    pub(in crate::ui) fn render_home_modular_toolbar_strip(&self) -> El {
        let p = self.p();
        let layout = self.home_modules.preset.map(|preset| preset.label()).unwrap_or("自定义布局");
        let row_count = self.home_modules.pack_rows().len();
        let capsules = self.collect_top_bar_live_capsules();
        let total_caps = capsules.len();
        let live_caps = capsules
            .iter()
            .filter(|c| c.track == crate::ui::shell::CapsuleTrack::LiveActivity)
            .count();
        let imp_caps = capsules
            .iter()
            .filter(|c| c.track == crate::ui::shell::CapsuleTrack::ImportantMessage)
            .count();

        let mut right_btns = Vec::new();
        let gov_count = self.reminder_governance.entries.len();
        let gov_label = if gov_count > 0 {
            format!("提醒管理 ({gov_count})")
        } else {
            "提醒管理".to_string()
        };
        right_btns.push(self.button_icon(
            "home-reminder-governance-btn",
            gov_label,
            UiIcon::Bell,
            Action::HomeReminder(HomeReminderAction::OpenGovernanceDrawer(
                !self.home_reminder_drawer_open,
            )),
            if self.home_reminder_drawer_open {
                Btn::Primary
            } else {
                Btn::Secondary
            },
        ));
        if total_caps > 0 {
            right_btns.push(self.button_icon(
                "home-open-capsule-hub",
                format!("胶囊仓 ({total_caps})"),
                UiIcon::Smartphone,
                Action::BackScreen(BackScreenAction::ToggleLiveCapsulePopover),
                Btn::Secondary,
            ));
        }
        right_btns.push(self.button_icon(
            "home-mod-bar-edit-toggle",
            if self.home_module_edit_mode { "完成编辑" } else { "调整布局" },
            if self.home_module_edit_mode { UiIcon::Check } else { UiIcon::Layers },
            Action::HomeModule(HomeModuleAction::ToggleEditMode),
            if self.home_module_edit_mode { Btn::Primary } else { Btn::Secondary },
        ));

        let mut title_row = vec![
            self.text_w("今天", SUBHEAD, p.text, Weight::Semibold),
            self.chip(
                "home-toolbar-weather-chip",
                self.home_toolbar_weather_label(),
                Some(UiIcon::Sun),
                vec![Action::OpenTool(Some(ToolPage::Weather))],
                self.tool_page == Some(ToolPage::Weather),
                false,
            ),
        ];
        if imp_caps > 0 {
            title_row.push(self.pill_badge(format!("{imp_caps} 条重要"), p.accent, p.accent_soft));
        }
        if self.home_module_edit_mode {
            // 仅编辑态显示布局信息
            title_row.push(self.pill_badge(
                format!("{layout} · {} 个模块 · {row_count} 行", self.home_modules.enabled.len()),
                p.text2,
                p.surface,
            ));
        }
        let _ = live_caps;
        let mut content = vec![row_between_expanded(
            self.hstack(S2, title_row),
            self.hstack(S2, right_btns),
            S2,
        )];
        if !self.home_module_edit_mode && !capsules.is_empty() {
            let mut cap_chips: Vec<El> = Vec::with_capacity(4);
            for (idx, cap) in capsules.iter().take(4).enumerate() {
                let chip_label = Self::home_capsule_short_label(cap);
                let chip_el = self.chip(
                    format!("home-capsule-chip-{idx}"),
                    chip_label,
                    Some(cap.icon),
                    vec![cap.primary_action.clone()],
                    false,
                    false,
                );
                let rem_target = self.reminder_target_for_capsule(cap);
                cap_chips.push(self.wrap_home_reminder_target(
                    format!("home-rem-capsule-{idx}"),
                    rem_target,
                    chip_el,
                ));
            }
            let strip = self.bounded_wrap(cap_chips);
            #[cfg(not(feature = "ui-capture"))]
            let strip = SavePosition::new(strip, "home-important-capsule-strip")
                .for_single_frame()
                .finish();
            #[cfg(feature = "ui-capture")]
            let strip = crate::ui::scenario_check::control(
                "home-important-capsule-strip".into(),
                strip,
            );
            content.push(strip);
        }
        if self.home_reminder_drawer_open {
            content.push(self.render_home_reminder_governance_panel());
        }
        if self.home_module_edit_mode {
            content.push(self.wrap(HomeModulePreset::ALL.into_iter().map(|preset| {
                self.chip(
                    format!("home-mod-bar-preset-{:?}", preset), preset.label(), None,
                    vec![Action::HomeModule(HomeModuleAction::ApplyPreset(preset))],
                    self.home_modules.preset == Some(preset), false,
                )
            }).collect()));
        }
        Container::new(self.vstack(S2, content))
            .with_horizontal_padding(S3).with_vertical_padding(7.)
            .with_background_color(p.card)
            .with_border(Border::all(1.).with_border_color(if self.home_module_edit_mode { p.accent } else { p.border }))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL))).finish()
    }


    pub(in crate::ui) fn render_home_module_editor_studio(&self) -> El {
        let p = self.p();
        let registry = self.home_pointer_registry.borrow();
        let library_open = registry.module_library_open;
        let can_undo = !registry.edit_undo.is_empty();
        let mut sections = vec![self.bounded_wrap(vec![
            self.hstack(S2, vec![self.ui_icon(UiIcon::Layers, 18., p.accent),
                self.line_w("调整布局", SUBHEAD, p.text, Weight::Semibold)]),
            self.button_icon("home-mod-studio-add", "添加", UiIcon::Plus,
                Action::HomeModule(HomeModuleAction::ToggleModuleLibrary),
                if library_open { Btn::Primary } else { Btn::Secondary }),
            if can_undo {
                self.button_icon("home-mod-studio-undo", "撤销", UiIcon::Retry,
                    Action::HomeModule(HomeModuleAction::UndoEdit), Btn::Secondary)
            } else { self.disabled_button("撤销") },
            self.button("home-mod-studio-cancel", "取消",
                Action::HomeModule(HomeModuleAction::CancelEdit), Btn::Ghost),
            self.button_icon("home-mod-studio-done", "完成并保存", UiIcon::Check,
                Action::HomeModule(HomeModuleAction::SetEditMode(false)), Btn::Primary),
        ]), self.text("拖动或点选「移动」排序；点击「尺寸」选择宽度与内容量。独占一行时自动铺满，同排时使用所选比例。", CAPTION, p.text2)];
        sections.push(self.line(
            registry.edit_feedback.as_deref().unwrap_or("可随时撤销调整；隐藏的模块可从「添加」恢复。"),
            CAPTION, if registry.edit_feedback.is_some() { p.accent } else { p.text2 },
        ));

        if library_open {
            let modules = HomeModuleId::ALL.into_iter().map(|module| {
                let enabled = self.home_modules.is_enabled(module);
                self.chip(format!("home-mod-matrix-toggle-{}", module.slug()), module.short_title(),
                    Some(if enabled { UiIcon::Check } else { UiIcon::Plus }),
                    vec![Action::HomeModule(HomeModuleAction::ToggleModule(module))], enabled, false)
            }).collect();
            sections.push(self.divider());
            sections.push(self.line_w(format!("全部模块 · 已显示 {}/{}", self.home_modules.enabled.len(), HomeModuleId::ALL.len()),
                CAPTION, p.text, Weight::Semibold));
            sections.push(self.bounded_wrap(modules));
            sections.push(self.line("也可以选择一个起点，再按需要调整", CAPTION, p.text2));
            sections.push(self.bounded_wrap(HomeModulePreset::ALL.into_iter().map(|preset| {
                self.chip(format!("home-mod-bar-preset-{:?}", preset), preset.label(), Some(UiIcon::Layers),
                    vec![Action::HomeModule(HomeModuleAction::ApplyPreset(preset))],
                    self.home_modules.preset == Some(preset), false)
            }).collect()));
        }
        self.panel(sections)
    }

    pub(in crate::ui) fn wrap_module_with_edit_controls(
        &self,
        mod_id: HomeModuleId,
        rank_idx: usize,
        total_enabled: usize,
        body: El,
    ) -> El {
        let p = self.p();
        let slug = mod_id.slug();
        let cur_w = self.home_modules.module_width(mod_id);
        let cur_h = self.home_modules.module_height(mod_id);
        let is_pending_source = self.home_pending_move == Some(mod_id);
        let is_pending_target = self
            .home_pending_move
            .is_some_and(|src| src != mod_id);

        let (drag_active_src, drag_hover_tgt) = {
            let reg = self.home_pointer_registry.borrow();
            match reg.active_op {
                Some(HomePointerOp::DragModule {
                    source,
                    hover_target,
                    ..
                }) => (source == mod_id, hover_target == Some(mod_id)),
                Some(HomePointerOp::ResizeModule { module, .. }) => (module == mod_id, false),
                None => (false, false),
            }
        };

        let drag_pill_inner = Container::new(self.hstack(
            4.,
            vec![
                self.ui_icon(UiIcon::Menu, 14., p.text2),
                self.line_w(format!("移动 · {}", mod_id.short_title()), CAPTION, p.text, Weight::Semibold),
            ],
        ))
        .with_horizontal_padding(7.)
        .with_vertical_padding(2.5)
        .with_background_color(if is_pending_source || drag_active_src {
            p.accent_soft
        } else {
            p.surface
        })
        .with_border(Border::all(1.).with_border_color(if is_pending_source || drag_active_src {
            p.accent
        } else {
            p.border
        }))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_SM)))
        .finish();
        let drag_registry = self.home_pointer_registry.clone();
        let drag_pill_inner = Hoverable::new(self.mouse_state(format!("home-drag-press-{slug}")), move |_| drag_pill_inner)
            .with_cursor(Cursor::OpenHand)
            .on_mouse_down(move |ctx, _, position| {
                drag_registry.borrow_mut().active_op = Some(HomePointerOp::DragModule {
                    source: mod_id,
                    start_pos: position,
                    current_pos: position,
                    hover_target: None,
                    moved: false,
                });
                ctx.dispatch_typed_action(Action::HomeModule(HomeModuleAction::BeginDrag(mod_id)));
                ctx.notify();
            })
            .finish();
        let drag_handle = home_geometry_probe(
            format!("home-drag-handle-{slug}"),
            Box::new(HomeBoundsRecorder {
                child: drag_pill_inner,
                target: HomeBoundsTarget::DragHandle(mod_id),
                registry: self.home_pointer_registry.clone(),
            }),
        );

        let resize_pill_inner = Container::new(self.hstack(4., vec![
            self.ui_icon(UiIcon::Maximize, 14., p.text2),
            self.line(format!("尺寸 {} · {}", cur_w.short_label(), cur_h.short_label()), CAPTION, p.text),
        ]))
        .with_horizontal_padding(7.)
        .with_vertical_padding(5.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_SM)))
        .finish();
        let resize_registry = self.home_pointer_registry.clone();
        let resize_pill_inner = Hoverable::new(self.mouse_state(format!("home-resize-press-{slug}")), move |_| resize_pill_inner)
            .with_cursor(Cursor::PointingHand)
            .on_mouse_down(move |ctx, _, position| {
                let mut registry = resize_registry.borrow_mut();
                let start_rect = registry.card_rects.get(&mod_id).copied()
                    .unwrap_or_else(|| RectF::new(position, vec2f(320., 220.)));
                let start_row_width = registry.content_width();
                registry.active_op = Some(HomePointerOp::ResizeModule {
                    module: mod_id,
                    start_pos: position,
                    current_pos: position,
                    start_rect,
                    start_row_width,
                    initial_width: cur_w,
                    initial_height: cur_h,
                    preview_width: cur_w,
                    preview_height: cur_h,
                    moved: false,
                });
                drop(registry);
                ctx.dispatch_typed_action(Action::HomeModule(HomeModuleAction::BeginResize(mod_id)));
                ctx.notify();
            })
            .finish();
        let resize_handle = home_geometry_probe(
            format!("home-resize-handle-{slug}"),
            Box::new(HomeBoundsRecorder {
                child: resize_pill_inner,
                target: HomeBoundsTarget::ResizeHandle(mod_id),
                registry: self.home_pointer_registry.clone(),
            }),
        );

        let menu_position = self.home_pointer_registry.borrow().card_rects.get(&mod_id)
            .map(|rect| vec2f(rect.origin().x() + 16., rect.origin().y() + 40.))
            .unwrap_or_else(Vector2F::zero);
        // Reuse the ordering slot in click-to-move mode. Adding another button
        // here used to wrap narrow toolbars and move the user's target away.
        let ordering = match self.home_pending_move {
            Some(source) if source == mod_id => self.chip(format!("home-click-move-{slug}"),
                "取消移动", Some(UiIcon::Close),
                vec![Action::HomeModule(HomeModuleAction::CancelClickMove)], true, false),
            Some(source) => self.chip(format!("home-drop-insert-{slug}"),
                "移到此处", Some(UiIcon::ArrowDown),
                vec![Action::HomeModule(HomeModuleAction::MoveBefore(source, mod_id))], true, false),
            None => self.chip(format!("home-module-menu-{slug}"), "调整排序", Some(UiIcon::Menu),
                vec![Action::ContextOpen(crate::ui::context_menu::Target::HomeModule(mod_id), menu_position)], false, false),
        };
        let mut actions: Vec<El> = vec![ordering];
        if total_enabled > 1 {
            actions.push(self.chip(format!("home-mod-inline-hide-{slug}"), "隐藏", Some(UiIcon::Minus),
                vec![Action::HomeModule(HomeModuleAction::RemoveModule(mod_id))], false, false));
        }
        #[cfg(test)]
        let actions: Vec<El> = actions.into_iter().enumerate().map(|(i, action)|
            SavePosition::new(action, &format!("home-edit-test-action-{i}")).finish()
        ).collect();
        let active_highlight = is_pending_source || drag_active_src || drag_hover_tgt;
        let mut controls = vec![drag_handle, resize_handle];
        controls.extend(actions);
        let ctrl_bar = Container::new(self.bounded_wrap(controls))
            .with_horizontal_padding(S2)
            .with_vertical_padding(4.)
            .with_background_color(if active_highlight { p.accent_soft } else { p.surface })
            .with_border(Border::all(1.).with_border_color(if active_highlight { p.accent } else { p.border }))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();

        // 拖拽手柄 / 移动 / 尺寸控件条仅在布局编辑态（或正在移动、拖拽中）显示；
        // 普通态模块只渲染自身卡片，不额外加控件条。
        let show_ctrl_bar = self.home_module_edit_mode
            || self.home_pending_move.is_some()
            || self.home_drag_active();
        let card_stack = if show_ctrl_bar {
            self.home_surface_stack(4., vec![ctrl_bar, body])
        } else {
            body
        };
        let recorded_card: El = home_geometry_probe(
            format!("home-module-card-{slug}"),
            Box::new(HomeBoundsRecorder {
                child: card_stack,
                target: HomeBoundsTarget::Card(mod_id),
                registry: self.home_pointer_registry.clone(),
            }),
        );

        let reflow = {
            let registry = self.home_pointer_registry.borrow();
            registry.reflow_started
                .filter(|started| started.elapsed() < std::time::Duration::from_millis(220))
                .and_then(|started| registry.reflow_from.get(&mod_id).copied().map(|rect| (started, rect)))
        };
        if self.home_module_edit_mode || self.home_pending_move.is_some() || self.home_drag_active() || reflow.is_some()
        {
            Box::new(HomeEditAnimatedCard {
                child: recorded_card,
                rank_idx,
                started: self.home_edit_anim_started,
                reflow,
                animate: !self.animations_disabled(),
                accent: p.accent,
                is_active_source: is_pending_source || drag_active_src,
                is_drop_target: is_pending_target || drag_hover_tgt,
                registry: self.home_pointer_registry.clone(),
                size: None,
                origin: None,
            })
        } else {
            recorded_card
        }
    }

}

#[cfg(test)]
mod responsive_home_controls_tests {
    use super::*;

    fn edit(app: &mut Companion, action: HomeModuleAction) {
        assert!(app.apply_home_action(&Action::HomeModule(action), None));
    }

    #[test]
    fn home_edit_draft_undo_cancel_and_done_preserve_layout_and_input() {
        let mut app = Companion::new_for_test();
        app.fields[F_HOME_COMMAND].set("保留正在输入的安排");
        let original = app.home_modules.clone();
        let module = original.enabled[0];
        edit(&mut app, HomeModuleAction::SetEditMode(true));
        edit(&mut app, HomeModuleAction::RemoveModule(module));
        let feedback = app.home_pointer_registry.borrow().edit_feedback.clone();
        assert!(feedback.as_ref().is_some_and(|message| message.contains("隐藏")));
        app.status = "新消息已同步".into();
        assert_eq!(app.home_pointer_registry.borrow().edit_feedback, feedback,
            "background sync must not replace layout feedback");
        let hidden = app.home_modules.clone();
        assert_eq!(app.home_pointer_registry.borrow().edit_original.as_ref(), Some(&original));
        edit(&mut app, HomeModuleAction::CommitResize(module, HomeModuleWidth::OneThird, HomeModuleHeight::Compact));
        assert_eq!(app.home_pointer_registry.borrow().edit_undo.len(), 2);
        edit(&mut app, HomeModuleAction::UndoEdit);
        assert_eq!(app.home_modules, hidden);
        edit(&mut app, HomeModuleAction::CancelEdit);
        assert_eq!(app.home_modules, original);
        assert!(!app.home_module_edit_mode);
        assert!(app.home_pointer_registry.borrow().edit_original.is_none());
        assert!(app.home_pointer_registry.borrow().edit_undo.is_empty());
        assert!(app.home_pointer_registry.borrow().edit_feedback.is_none());
        assert_eq!(app.fields[F_HOME_COMMAND].text, "保留正在输入的安排");

        edit(&mut app, HomeModuleAction::SetEditMode(true));
        edit(&mut app, HomeModuleAction::RemoveModule(module));
        edit(&mut app, HomeModuleAction::SetEditMode(false));
        assert_eq!(app.home_modules, hidden, "Done retains the edited configuration");
        assert!(!app.home_module_edit_mode);
        assert!(app.home_pointer_registry.borrow().edit_original.is_none());
        edit(&mut app, HomeModuleAction::SetEditMode(true));
        edit(&mut app, HomeModuleAction::ShowAllModules);
        edit(&mut app, HomeModuleAction::CancelEdit);
        assert_eq!(app.home_modules, hidden, "the next editing session starts from the last saved draft");
    }

    #[test]
    fn home_edit_insert_keeps_intervening_order_and_ignores_noops() {
        let mut app = Companion::new_for_test();
        let original = app.home_modules.enabled.clone();
        let first = original[0];
        let target = original[3];
        edit(&mut app, HomeModuleAction::SetEditMode(true));
        edit(&mut app, HomeModuleAction::CommitDragInsert(first, target, true));
        assert_eq!(&app.home_modules.enabled[..4], &[original[1], original[2], target, first]);
        assert_eq!(&app.home_modules.enabled[4..], &original[4..]);
        edit(&mut app, HomeModuleAction::CommitDragInsert(first, original[1], false));
        assert_eq!(app.home_modules.enabled, original);
        let undo_count = app.home_pointer_registry.borrow().edit_undo.len();
        edit(&mut app, HomeModuleAction::CommitDragInsert(first, first, false));
        edit(&mut app, HomeModuleAction::AddModule(first));
        assert_eq!(app.home_pointer_registry.borrow().edit_undo.len(), undo_count);
        edit(&mut app, HomeModuleAction::ToggleClickMove(first));
        edit(&mut app, HomeModuleAction::CancelDrag);
        assert!(app.home_pending_move.is_none());
        assert_eq!(app.home_modules.enabled, original);
    }

    #[test]
    fn home_drop_insertion_uses_row_direction_and_resize_preserves_auto_without_vertical_drag() {
        let mut registry = HomePointerRegistry::default();
        let a = HomeModuleId::CalendarPanorama;
        let b = HomeModuleId::RecentFiles;
        registry.card_rects.insert(a, RectF::new(vec2f(0., 0.), vec2f(800., 300.)));
        assert!(!registry.drop_after(a, vec2f(700., 100.)));
        assert!(registry.drop_after(a, vec2f(100., 200.)));
        registry.card_rects.insert(b, RectF::new(vec2f(820., 0.), vec2f(300., 300.)));
        assert!(!registry.drop_after(a, vec2f(100., 200.)));
        assert!(registry.drop_after(a, vec2f(700., 100.)));
        let (width, height) = HomePointerRegistry::compute_resized_dimensions(
            HomeModuleWidth::OneHalf, HomeModuleHeight::Auto, 180., 0.);
        assert_eq!(width, HomeModuleWidth::Full);
        assert_eq!(height, HomeModuleHeight::Auto);
        let (_, taller) = HomePointerRegistry::compute_resized_dimensions(
            HomeModuleWidth::OneHalf, HomeModuleHeight::Auto, 0., 80.);
        assert_eq!(taller, HomeModuleHeight::Tall);
    }

    #[test]
    fn home_reflow_follows_full_distance_and_settles_in_220ms() {
        use std::time::Duration;
        let distance = vec2f(640., -900.);
        assert_eq!(home_reflow_offset(distance, Duration::ZERO), distance);
        let midpoint = home_reflow_offset(distance, Duration::from_millis(110));
        assert!((midpoint.x() - 80.).abs() < 0.01);
        assert!((midpoint.y() + 112.5).abs() < 0.01);
        assert_eq!(home_reflow_offset(distance, Duration::from_millis(220)), Vector2F::zero());
        assert_eq!(home_reflow_offset(distance, Duration::from_secs(1)), Vector2F::zero());
    }

    #[test]
    fn home_drop_return_to_source_and_distant_release_cancel_for_all_grid_widths() {
        for width in [320., 760., 1280., 2400.] {
            let mut registry = HomePointerRegistry::default();
            let source = HomeModuleId::RecentFiles;
            let target = HomeModuleId::WeatherModule;
            let third = HomeModuleId::AlarmsAndControl;
            let half = (width - 16.) / 2.;
            registry.card_rects.insert(source, RectF::new(vec2f(0., 0.), vec2f(half, 220.)));
            registry.card_rects.insert(target, RectF::new(vec2f(half + 16., 0.), vec2f(half, 220.)));
            registry.card_rects.insert(third, RectF::new(vec2f(0., 236.), vec2f(width, 260.)));
            assert_eq!(registry.resolve_drop_target(source, vec2f(width - 20., 40.)), Some(target));
            assert_eq!(registry.resolve_drop_target(source, vec2f(half - 1., 40.)), None,
                "returning to the source edge must not choose a nearby card at {width}");
            assert_eq!(registry.resolve_drop_target(source, vec2f(width + 80., 40.)), None);
            assert_eq!(registry.resolve_drop_target(source, vec2f(30., 380.)), Some(third));
            assert!(!registry.drop_after(third, vec2f(width - 10., 250.)),
                "full-width target uses vertical insertion, even on a wide window");
            assert!(registry.drop_after(third, vec2f(10., 480.)));
            assert!(!registry.drop_after(target, vec2f(half + 20., 210.)));
            assert!(registry.drop_after(target, vec2f(width - 10., 10.)));
        }
    }

    #[test]
    fn home_resize_scales_with_grid_width_and_actual_card_height() {
        for row_width in [320., 760., 1280., 2400.] {
            for card_height in [180., 320., 600.] {
                for initial in HomeModuleWidth::ALL {
                    for desired in HomeModuleWidth::ALL {
                        let dx = (desired.units() as f32 - initial.units() as f32) * row_width / 12.;
                        let (width, height) = HomePointerRegistry::compute_resized_dimensions_for_geometry(
                            initial, HomeModuleHeight::Auto,
                            vec2f(row_width * initial.units() as f32 / 12., card_height), row_width,
                            vec2f(dx, 0.),
                        );
                        assert_eq!(width, desired, "row={row_width}, initial={initial:?}");
                        assert_eq!(height, HomeModuleHeight::Auto, "horizontal resizing must preserve Auto");
                    }
                }
                let height_step = (card_height * 0.28).clamp(64., 168.);
                for (fraction, desired) in [(0.1, HomeModuleHeight::Auto), (1., HomeModuleHeight::Tall), (-1., HomeModuleHeight::Compact)] {
                    let (width, height) = HomePointerRegistry::compute_resized_dimensions_for_geometry(
                        HomeModuleWidth::OneHalf, HomeModuleHeight::Auto,
                        vec2f(row_width / 2., card_height), row_width,
                        vec2f(0., height_step * fraction),
                    );
                    assert_eq!(width, HomeModuleWidth::OneHalf);
                    assert_eq!(height, desired, "height={card_height}");
                }
            }
        }
    }

    #[test]
    fn home_window_resize_cancels_pointer_geometry_without_discarding_draft() {
        let mut registry = HomePointerRegistry::default();
        let original = HomeModuleLayoutConfig::default();
        registry.begin_edit(&original);
        registry.edit_undo.push(original.clone());
        registry.observe_viewport(vec2f(1200., 800.));
        registry.active_op = Some(HomePointerOp::DragModule {
            source: HomeModuleId::RecentFiles, start_pos: vec2f(100., 100.),
            current_pos: vec2f(400., 400.), hover_target: Some(HomeModuleId::WeatherModule), moved: true,
        });
        registry.reflow_started = Some(std::time::Instant::now());
        registry.reflow_from.insert(HomeModuleId::RecentFiles, RectF::new(vec2f(10., 10.), vec2f(300., 200.)));
        registry.observe_viewport(vec2f(1200., 800.));
        assert!(registry.active_op.is_some(), "normal renders keep the current gesture");
        registry.observe_viewport(vec2f(2400., 600.));
        assert!(registry.active_op.is_none());
        assert!(registry.reflow_started.is_none());
        assert!(registry.reflow_from.is_empty());
        assert_eq!(registry.edit_original.as_ref(), Some(&original));
        assert_eq!(registry.edit_undo, vec![original]);
    }

    struct CanvasResizeOnly { app: Companion }
    impl facloud_ui::Entity for CanvasResizeOnly { type Event = (); }
    impl facloud_ui::TypedActionView for CanvasResizeOnly { type Action = (); }
    impl facloud_ui::View for CanvasResizeOnly {
        fn ui_name() -> &'static str { "home_resize_release_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El {
            let app = &self.app;
            Box::new(HomeInteractiveCanvas {
                child: Box::new(HomeBoundsRecorder {
                    child: app.panel(vec![app.text("缩放验收模块", BODY, app.p().text)]),
                    target: HomeBoundsTarget::Card(HomeModuleId::RecentFiles),
                    registry: app.home_pointer_registry.clone(),
                }),
                floating_badge: None, editing: true,
                registry: app.home_pointer_registry.clone(),
                module_sizes: Default::default(), accent: app.p().accent, surface: app.p().card,
                size: None, origin: None,
            })
        }
    }

    #[test]
    fn home_native_layout_resize_then_immediate_release_cannot_commit_stale_geometry() {
        facloud_ui::App::test((), |mut app| async move {
            for next_size in [vec2f(1600., 800.), vec2f(1200., 500.), vec2f(720., 500.)] {
                let companion = Companion::new_for_test();
                let original = companion.home_modules.clone();
                let registry = companion.home_pointer_registry.clone();
                registry.borrow_mut().begin_edit(&original);
                let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                    |_| CanvasResizeOnly { app: companion });
                let root = app.root_view_id(window).unwrap();
                app.update(move |ctx| {
                    let mut presenter = facloud_ui::Presenter::new(window);
                    presenter.invalidate(facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]), ..Default::default()
                    }, ctx);
                    presenter.build_scene(vec2f(1200., 800.), 1., None, ctx);
                    let start_rect = registry.borrow().card_rects[&HomeModuleId::RecentFiles];
                    registry.borrow_mut().active_op = Some(HomePointerOp::ResizeModule {
                        module: HomeModuleId::RecentFiles,
                        start_pos: start_rect.origin(), current_pos: vec2f(120., 120.),
                        start_rect, start_row_width: 1200.,
                        initial_width: HomeModuleWidth::OneHalf, initial_height: HomeModuleHeight::Auto,
                        preview_width: HomeModuleWidth::TwoThirds, preview_height: HomeModuleHeight::Tall,
                        moved: true,
                    });
                    presenter.build_scene(vec2f(1200., 800.), 1., None, ctx);
                    assert!(registry.borrow().active_op.is_some(), "ordinary layout must not cancel a valid gesture");
                    // No view invalidation, no timer or workspace.viewport update.
                    presenter.build_scene(next_size, 1., None, ctx);
                    assert!(registry.borrow().active_op.is_none(), "native layout must cancel stale geometry for {next_size:?}");
                    let release = presenter.dispatch_event(Event::LeftMouseUp {
                        position: vec2f(200., 200.), modifiers: Default::default(),
                    }, ctx);
                    assert!(release.actions.is_empty(), "immediate release cannot commit the cancelled resize");
                    assert_eq!(registry.borrow().edit_original.as_ref(), Some(&original));
                });
            }
        });
    }

    struct ReflowPaintOnly {
        app: Companion,
        renders: std::rc::Rc<std::cell::Cell<usize>>,
    }

    struct InteractiveEditorOnly { app: Companion }
    impl facloud_ui::Entity for InteractiveEditorOnly { type Event = (); }
    impl facloud_ui::TypedActionView for InteractiveEditorOnly { type Action = (); }
    impl facloud_ui::View for InteractiveEditorOnly {
        fn ui_name() -> &'static str { "home_edit_gesture_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El {
            let app = &self.app;
            let cards = [HomeModuleId::RecentFiles, HomeModuleId::WeatherModule]
                .into_iter().enumerate().map(|(i, id)| app.wrap_module_with_edit_controls(
                    id, i, 2, app.panel(vec![app.text("模块内容", BODY, app.p().text)])))
                .collect();
            Box::new(HomeInteractiveCanvas {
                child: app.vstack(12., cards),
                floating_badge: Some(app.panel(vec![app.text("松开以移动模块", BODY, app.p().text)])),
                editing: true, registry: app.home_pointer_registry.clone(),
                module_sizes: Default::default(), accent: app.p().accent, surface: app.p().card,
                size: None, origin: None,
            })
        }
    }

    fn typed_actions(actions: &[facloud_ui::presenter::DispatchedAction]) -> Vec<&Action> {
        actions.iter().filter_map(|action| match &action.kind {
            facloud_ui::presenter::DispatchedActionKind::Typed(action) => action.as_ref().as_any().downcast_ref::<Action>(),
            _ => None,
        }).collect()
    }

    #[test]
    fn home_size_click_opens_choices_while_drag_commits_and_preview_never_blocks_drop() {
        facloud_ui::App::test((), |mut app| async move {
            let mut companion = Companion::new_for_test();
            companion.home_module_edit_mode = true;
            let registry = companion.home_pointer_registry.clone();
            let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| InteractiveEditorOnly { app: companion });
            let root = app.root_view_id(window).unwrap();
            app.update(move |ctx| {
                let mut presenter = facloud_ui::Presenter::new(window);
                presenter.invalidate(facloud_ui::WindowInvalidation {
                    updated: std::collections::HashSet::from([root]), ..Default::default()
                }, ctx);
                presenter.build_scene(vec2f(700., 800.), 1., None, ctx);
                let rect = registry.borrow().resize_handle_rects[&HomeModuleId::RecentFiles];
                let position = rect.origin() + rect.size() * 0.5;
                presenter.dispatch_event(Event::LeftMouseDown { position, modifiers: Default::default(),
                    click_count: 1, is_first_mouse: false }, ctx);
                assert!(matches!(registry.borrow().active_op, Some(HomePointerOp::ResizeModule { moved: false, .. })));
                let release = presenter.dispatch_event(Event::LeftMouseUp { position, modifiers: Default::default() }, ctx);
                assert!(typed_actions(&release.actions).iter().any(|action| matches!(action,
                    Action::ContextOpen(crate::ui::context_menu::Target::HomeModule(HomeModuleId::RecentFiles), _))),
                    "a size click must visibly open choices rather than silently do nothing");
                assert!(registry.borrow().active_op.is_none());
                presenter.dispatch_event(Event::LeftMouseDown { position, modifiers: Default::default(),
                    click_count: 1, is_first_mouse: false }, ctx);
                let release = presenter.dispatch_event(Event::LeftMouseUp {
                    position: position + vec2f(160., 0.), modifiers: Default::default() }, ctx);
                assert!(typed_actions(&release.actions).iter().any(|action| matches!(action,
                    Action::HomeModule(HomeModuleAction::CommitResize(HomeModuleId::RecentFiles, _, _)))));
                assert!(!typed_actions(&release.actions).iter().any(|action| matches!(action, Action::ContextOpen(..))));

                // The real badge clamps against the right canvas edge and can
                // cover the pointer. Dropping there must still reach the card.
                let source = HomeModuleId::RecentFiles;
                let target = HomeModuleId::WeatherModule;
                let target_rect = registry.borrow().card_rects[&target];
                let target_handle = registry.borrow().drag_handle_rects[&target];
                for drop_position in [
                    vec2f(target_rect.max_x() - 15., target_rect.origin().y() + 15.),
                    target_handle.origin() + target_handle.size() * 0.5,
                ] {
                    registry.borrow_mut().active_op = Some(HomePointerOp::DragModule {
                        source, start_pos: position, current_pos: drop_position,
                        hover_target: Some(target), moved: true,
                    });
                    presenter.build_scene(vec2f(700., 800.), 1., None, ctx);
                    let release = presenter.dispatch_event(Event::LeftMouseUp {
                        position: drop_position, modifiers: Default::default() }, ctx);
                    assert!(typed_actions(&release.actions).iter().any(|action| matches!(action,
                        Action::HomeModule(HomeModuleAction::CommitDragInsert(s, t, _)) if *s == source && *t == target)),
                        "the drag preview or target handle must not intercept its own drop");
                }
            });
        });
    }
    impl facloud_ui::Entity for ReflowPaintOnly { type Event = (); }
    impl facloud_ui::TypedActionView for ReflowPaintOnly { type Action = (); }
    impl facloud_ui::View for ReflowPaintOnly {
        fn ui_name() -> &'static str { "home_reflow_paint_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El {
            self.renders.set(self.renders.get() + 1);
            let card = self.app.wrap_module_with_edit_controls(HomeModuleId::RecentFiles,
                0, 16, self.app.text("实际模块内容", BODY, self.app.p().text));
            Stack::new().with_child(Align::new(card).top_left().finish()).finish()
        }
    }

    #[test]
    fn home_reflow_moves_the_painted_card_and_repaints_without_rebuilding_the_view() {
        facloud_ui::App::test((), |mut app| async move {
            let mut companion = Companion::new_for_test();
            companion.home_module_edit_mode = true;
            let registry = companion.home_pointer_registry.clone();
            let previous = vec2f(220., 360.);
            {
                let mut registry = registry.borrow_mut();
                // A future start gives a deterministic first frame; this checks
                // actual paint coordinates, not only the easing helper.
                registry.reflow_started = Some(std::time::Instant::now() + std::time::Duration::from_secs(5));
                registry.reflow_from.insert(HomeModuleId::RecentFiles,
                    RectF::new(previous, vec2f(400., 160.)));
            }
            let renders = std::rc::Rc::new(std::cell::Cell::new(0));
            let render_count = renders.clone();
            let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| ReflowPaintOnly { app: companion, renders });
            let root = app.root_view_id(window).unwrap();
            app.update(move |ctx| {
                let mut presenter = facloud_ui::Presenter::new(window);
                presenter.invalidate(facloud_ui::WindowInvalidation {
                    updated: std::collections::HashSet::from([root]), ..Default::default()
                }, ctx);
                presenter.build_scene(vec2f(1200., 800.), 1., None, ctx);
                let painted = registry.borrow().card_rects[&HomeModuleId::RecentFiles];
                assert!((painted.origin() - previous).length() < 1., "the card must start at its real old position: {painted:?}");
                let renders_before = render_count.get();
                presenter.build_scene(vec2f(1200., 800.), 1., None, ctx);
                assert_eq!(render_count.get(), renders_before,
                    "animation frames must repaint the existing tree, without rebuilding homepage business data");
                let registry = registry.borrow();
                inside(registry.drag_handle_rects[&HomeModuleId::RecentFiles], painted, "animated drag handle");
                inside(registry.resize_handle_rects[&HomeModuleId::RecentFiles], painted, "animated resize handle");
            });
        });
    }

    struct CardOnly { app: Companion, editor: bool }
    impl facloud_ui::Entity for CardOnly { type Event = (); }
    impl facloud_ui::TypedActionView for CardOnly { type Action = (); }
    impl facloud_ui::View for CardOnly {
        fn ui_name() -> &'static str { "home_responsive_controls_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El {
            let card = if self.editor {
                self.app.wrap_module_with_edit_controls(HomeModuleId::CalendarPanorama,
                    1, 16, self.app.text("模块正文", BODY, self.app.p().text))
            } else { self.app.home_page() };
            Stack::new().with_child(Align::new(card).top_left().finish()).finish()
        }
    }

    fn inside(child: RectF, parent: RectF, label: &str) {
        assert!(child.origin().x() >= parent.origin().x() - 1.
            && child.max_x() <= parent.max_x() + 1.
            && child.origin().y() >= parent.origin().y() - 1.
            && child.max_y() <= parent.max_y() + 1.,
            "{label} {child:?} outside {parent:?}");
    }

    #[test]
    fn full_card_edit_actions_wrap_inside_narrow_windows() {
        facloud_ui::App::test((), |mut app| async move {
            for width in [260., 390., 720., 1100.] {
                let mut companion = Companion::new_for_test();
                companion.home_module_edit_mode = true;
                companion.home_edit_anim_started = std::time::Instant::now() - std::time::Duration::from_secs(1);
                let registry = companion.home_pointer_registry.clone();
                let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                    |_| CardOnly { app: companion, editor: true });
                let root = app.root_view_id(window).unwrap();
                app.update(move |ctx| {
                    let mut presenter = facloud_ui::Presenter::new(window);
                    presenter.invalidate(facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]), ..Default::default()
                    }, ctx);
                    presenter.build_scene(vec2f(width, 1600.), 1., None, ctx);
                    let reg = registry.borrow();
                    let panel = reg.card_rects[&HomeModuleId::CalendarPanorama];
                    assert!(panel.max_x() <= width + 1., "card at {width}: {panel:?}; drag={:?}, resize={:?}", reg.drag_handle_rects, reg.resize_handle_rects);
                    inside(reg.drag_handle_rects[&HomeModuleId::CalendarPanorama], panel, "drag handle");
                    inside(reg.resize_handle_rects[&HomeModuleId::CalendarPanorama], panel, "resize handle");
                    let positions = presenter.position_cache();
                    let mut count = 0;
                    while let Some(rect) = positions.get_position(&format!("home-edit-test-action-{count}")) {
                        inside(rect, panel, "edit action");
                        count += 1;
                    }
                    assert_eq!(count, 2, "uncommon sizing/order actions belong in the menu, alongside one hide button");
                });
            }
        });
    }

    #[test]
    fn today_data_tiles_and_dial_fit_all_card_widths() {
        facloud_ui::App::test((), |mut app| async move {
            for width in [260., 390., 560., 720., 1100.] {
                for height in [HomeModuleHeight::Compact, HomeModuleHeight::Standard, HomeModuleHeight::Tall] {
                    let mut companion = Companion::new_for_test();
                    companion.home_modules.enabled = vec![HomeModuleId::PulseAndKpi];
                    companion.home_modules.set_module_size(HomeModuleId::PulseAndKpi, HomeModuleWidth::OneHalf, height);
                    // This fixture paints the canvas alone. Supply the shell
                    // width that allocates exactly this canvas width so render
                    // density and native layout receive the same geometry.
                    companion.workspace.viewport = vec2f(width + 2. + SIDEBAR_W + S5 * 2.
                        + ScrollbarWidth::Auto.as_f32() + 4., 5000.);
                    let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                        |_| CardOnly { app: companion, editor: false });
                    let root = app.root_view_id(window).unwrap();
                    app.update(move |ctx| {
                        let mut presenter = facloud_ui::Presenter::new(window);
                        presenter.invalidate(facloud_ui::WindowInvalidation {
                            updated: std::collections::HashSet::from([root]), ..Default::default()
                        }, ctx);
                        presenter.build_scene(vec2f(width, 5000.), 1., None, ctx);
                        let positions = presenter.position_cache();
                        let rect = |id: &str| positions.get_position(id).unwrap_or_else(|| panic!("missing {id}"));
                        let panel = rect("home-pulse-panel");
                        assert!(panel.max_x() <= width + 1., "card exceeds {width}: {panel:?}");
                        let stats = rect("home-pulse-stats");
                        inside(stats, panel, "stats");
                        for id in ["home-kpi-messages", "home-kpi-life", "home-kpi-meetings", "home-kpi-screen"] {
                            inside(rect(id), stats, id);
                        }
                        let shows_dial = height == HomeModuleHeight::Tall
                            || (height != HomeModuleHeight::Compact && panel.size().x() - 34. >= 560.);
                        if shows_dial {
                            let dial = rect("home-pulse-dial");
                            inside(dial, panel, "dial");
                            assert!(dial.max_x() <= stats.origin().x() || dial.max_y() <= stats.origin().y(),
                                "dial overlaps facts at {width}: {dial:?}/{stats:?}");
                        } else {
                            assert!(positions.get_position("home-pulse-dial").is_none(),
                                "compact and narrow standard cards prioritize the facts without a dial");
                        }
                    });
                }
            }
        });
    }
}
