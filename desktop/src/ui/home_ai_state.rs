//! Home hub runtime state: rasterized SVG chart slots and the phone-AI (Amoo) briefing job.
//!
//! SVG charts are uploaded into the asset cache under one fixed raw id per *slot*. When a
//! chart's markup changes (theme switch, new minute, new data) the next tick replaces the
//! slot in place, which also evicts the previous raster, so memory stays bounded by the
//! number of slots instead of growing with every distinct chart state.
use super::{widgets::El, Companion, Page};
use chrono::{DateTime, Local};
use facloud_ui::{
    assets::asset_cache::{AssetCache, AssetSource},
    elements::*,
    image_cache::ImageType,
    AfterLayoutContext, AppContext, EventContext, LayoutContext, PaintContext, SizeConstraint,
    SingletonEntity, ViewContext,
};
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::{
    cell::{Cell, RefCell},
    collections::{hash_map::DefaultHasher, HashMap},
    hash::{Hash, Hasher},
    sync::mpsc,
    time::{Duration, Instant},
};
use stepx_companion::home_ai;

#[derive(Default)]
pub(super) struct SvgSlots {
    inserted: RefCell<HashMap<String, u64>>,
    pending: RefCell<HashMap<String, (u64, String)>>,
    last_flushed_at: Cell<Option<Instant>>,
}

fn svg_hash(svg: &str) -> u64 {
    let mut h = DefaultHasher::new();
    svg.hash(&mut h);
    h.finish()
}

pub(super) fn slot_asset_id(slot: &str) -> String {
    format!("stepx-home-svg:{slot}")
}

#[derive(Clone, Copy, Debug)]
struct SvgCanvasGeometry {
    outer: Vector2F,
    image: Vector2F,
    offset: Vector2F,
}

/// A fixed chart may be given less room on either axis, or a larger tight
/// parent. Resolve its image rectangle uniformly rather than independently
/// squeezing a square ring into the parent's rectangular bounds.
fn svg_canvas_geometry(
    width: Option<f32>,
    height: f32,
    stretch: bool,
    constraint: SizeConstraint,
) -> SvgCanvasGeometry {
    let positive = |value: f32| if value.is_finite() && value > 0. { value } else { 1. };
    let maximum = |value: f32| if value.is_nan() { 0. } else { value.max(0.) };
    let minimum = |value: f32| if value.is_finite() { value.max(0.) } else { 0. };
    let max = vec2f(maximum(constraint.max.x()), maximum(constraint.max.y()));
    let requested = vec2f(
        width.map(positive).unwrap_or_else(|| {
            if max.x().is_finite() { max.x() } else { minimum(constraint.min.x()).max(1.) }
        }),
        positive(height),
    );
    let outer = vec2f(
        requested.x().max(minimum(constraint.min.x())).min(max.x()),
        requested.y().max(minimum(constraint.min.y())).min(max.y()),
    );
    let image = if width.is_some() && !stretch {
        let mut scale = (outer.x() / requested.x()).min(outer.y() / requested.y()).min(1.);
        let mut image = requested * scale;
        // Division and multiplication may round a limiting dimension one ULP
        // above its outer bound (e.g. a 176px chart fitting a 12px height).
        // Move the shared scale inward instead of clamping axes separately:
        // circles and rectangular charts retain their original aspect ratio.
        while scale > 0. && (image.x() > outer.x() || image.y() > outer.y()) {
            scale = f32::from_bits(scale.to_bits() - 1);
            image = requested * scale;
        }
        image
    } else {
        outer
    };
    SvgCanvasGeometry { outer, image, offset: (outer - image) / 2. }
}

/// Keeps the loading placeholder and loaded chart on the same layout contract.
/// The child receives a strict image viewport; an outer clip is the final
/// boundary, so cached rasters cannot paint into neighbouring labels/cards.
struct SvgCanvasFrame {
    child: El,
    width: Option<f32>,
    height: f32,
    stretch: bool,
    geometry: Option<SvgCanvasGeometry>,
    origin: Option<Point>,
}

impl Element for SvgCanvasFrame {
    fn layout(&mut self, constraint: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let geometry = svg_canvas_geometry(self.width, self.height, self.stretch, constraint);
        self.child.layout(SizeConstraint::strict(geometry.image), ctx, app);
        self.geometry = Some(geometry);
        geometry.outer
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        if let Some(geometry) = self.geometry.filter(|g| g.image.x() > 0. && g.image.y() > 0.) {
            self.child.paint(origin + geometry.offset, ctx, app);
        }
    }

    fn dispatch_event(&mut self, event: &facloud_ui::event::DispatchedEvent, ctx: &mut EventContext, app: &AppContext) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }

    fn size(&self) -> Option<Vector2F> { self.geometry.map(|geometry| geometry.outer) }
    fn origin(&self) -> Option<Point> { self.origin }
}

impl SvgSlots {
    /// Records the desired markup for `slot`; returns whether *some* raster is already
    /// available for the slot (possibly the previous version until the next tick).
    pub(super) fn request(&self, slot: &str, svg: String) -> bool {
        let hash = svg_hash(&svg);
        let ready = self.inserted.borrow().contains_key(slot);
        if self.inserted.borrow().get(slot) == Some(&hash) {
            self.pending.borrow_mut().remove(slot);
        } else {
            self.pending
                .borrow_mut()
                .insert(slot.to_string(), (hash, svg));
        }
        ready
    }

    pub(super) fn take_pending(&self) -> Vec<(String, u64, String)> {
        self.pending
            .borrow_mut()
            .drain()
            .map(|(slot, (hash, svg))| (slot, hash, svg))
            .collect()
    }

    pub(super) fn mark_inserted(&self, slot: &str, hash: u64) {
        self.inserted.borrow_mut().insert(slot.to_string(), hash);
        self.last_flushed_at.set(Some(Instant::now()));
    }

    pub(super) fn has_pending(&self) -> bool {
        !self.pending.borrow().is_empty()
    }

    /// True once all requested SVG slots have been flushed into `AssetCache` and given a
    /// frame to decode in `ImageCache`.
    pub(super) fn ready_for_capture(&self, require_any: bool) -> bool {
        if !self.pending.borrow().is_empty() {
            return false;
        }
        if self.inserted.borrow().is_empty() {
            return !require_any;
        }
        self.last_flushed_at
            .get()
            .is_some_and(|t| t.elapsed() >= Duration::from_millis(250))
    }

    #[cfg(test)]
    pub(super) fn pending_len(&self) -> usize {
        self.pending.borrow().len()
    }
}

/// Phone-AI briefing UI state.
#[derive(Default)]
pub(super) struct HomeAiUi {
    pub(super) job: Option<home_ai::Job>,
    pub(super) status: String,
    pub(super) brief: Option<home_ai::HomeAiBrief>,
    pub(super) brief_at: Option<DateTime<Local>>,
    pub(super) error: Option<String>,
    /// Device serial for which the last briefing was requested.
    pub(super) auto_for: Option<String>,
    device_serial: Option<String>,
}

impl HomeAiUi {
    fn sync_device(&mut self, serial: Option<&str>) -> bool {
        if self.device_serial.as_deref() == serial {
            return false;
        }
        // Dropping Job signals cancellation before dropping its receiver. Any late
        // worker response can no longer enter the newly selected device's state.
        *self = Self {
            device_serial: serial.map(str::to_string),
            ..Self::default()
        };
        true
    }

    pub(super) fn running(&self) -> bool {
        self.job.is_some()
    }

    /// Short relative age label such as "刚刚" / "12 分钟前".
    pub(super) fn age_label(&self, now: DateTime<Local>) -> Option<String> {
        let at = self.brief_at?;
        let mins = (now - at).num_minutes().max(0);
        Some(if mins < 1 {
            "刚刚".into()
        } else if mins < 60 {
            format!("{mins} 分钟前")
        } else {
            format!("{} 小时前", mins / 60)
        })
    }
}

/// Canned reply used only by `--demo` captures; it goes through the real parser.
const DEMO_BRIEF_REPLY: &str = r#"{"headline":"评审会前，先清掉产品群 3 条未读",
"suggestions":[
{"title":"汇总产品群并起草回复","reason":"产品研发群 3 条未读含今晚发版安排","action":"amoo","arg":"请汇总产品研发群未读消息的要点，并为需要我确认的内容起草简短回复","button":"汇总"},
{"title":"会前专注 45 分钟","reason":"距评审会约 1 小时，今日已亮屏 164 分钟","action":"focus","arg":"45","button":"开始"},
{"title":"顺路取驿站快递","reason":"菜鸟驿站包裹已到站待取","action":"express","arg":"","button":"查看"}
]}"#;

impl Companion {
    /// A failed refresh retains the last useful Home snapshot. Do not label that
    /// snapshot with today's time and send it as fresh evidence to the phone AI.
    fn home_ai_context_blocker(&self, now: Instant) -> Option<&'static str> {
        if Self::stepos_read_error_is_fatal(self.back_screen.stepos_suite_error.as_deref())
            || Self::stepos_read_error_is_fatal(
                self.back_screen.stepos_suite.read_error.as_deref(),
            )
        {
            return Some("手机数据未完整读取，请先刷新首页再生成 AI 建议");
        }
        if self.push.error.is_some() {
            return Some("消息同步未成功，请先刷新首页再生成 AI 建议");
        }
        if !self.back_screen.stepos_suite.checked
            || self.back_screen.stepos_suite_loading
            || !self.push.loaded
            || self.push.syncing
        {
            return Some("首页数据还在同步，完成后再生成 AI 建议");
        }
        let Some(synced_at) = self.back_screen.stepos_suite_synced_at else {
            return Some("尚未成功读取手机数据，请先刷新首页再生成 AI 建议");
        };
        let max_age = Duration::from_secs(5 * 60);
        if now.saturating_duration_since(synced_at) >= max_age
            || now.saturating_duration_since(self.push.last_sync) >= max_age
        {
            return Some("首页快照已超过 5 分钟，请先刷新首页再生成 AI 建议");
        }
        None
    }

    /// Rasterized SVG chart. Fixed charts preserve aspect ratio even when a
    /// parent squeezes one dimension. `stretch` is reserved for non-geometric
    /// horizontal spectra; circles, text and time-axis markers must not use it.
    pub(super) fn svg_canvas(
        &self,
        slot: &str,
        svg: String,
        width: Option<f32>,
        height: f32,
        stretch: bool,
    ) -> El {
        let ready = self.svg_slots.request(slot, svg);
        let framed = |child: El| -> El {
            Clipped::new(Box::new(SvgCanvasFrame {
                child, width, height, stretch, geometry: None, origin: None,
            })).finish()
        };
        if !ready {
            return framed(Empty::new().finish());
        }
        let mut image = Image::new(
            AssetSource::Raw {
                id: slot_asset_id(slot),
            },
            CacheOption::BySize,
        )
        .contain()
        .before_load(Empty::new().finish());
        if stretch {
            image = image.stretch();
        }
        framed(image.finish())
    }

    fn flush_svg_slots(&mut self, ctx: &mut ViewContext<Self>) {
        let pending = self.svg_slots.take_pending();
        if pending.is_empty() {
            return;
        }
        for (slot, hash, svg) in pending {
            AssetCache::handle(ctx).update(ctx, |cache, cache_ctx| {
                cache.insert_raw_asset_bytes::<ImageType>(
                    slot_asset_id(&slot),
                    svg.as_bytes(),
                    cache_ctx,
                );
            });
            self.svg_slots.mark_inserted(&slot, hash);
        }
        ctx.notify();
    }

    /// Starts a phone-AI briefing. Returns a user-facing status message.
    pub(super) fn start_home_ai_brief(&mut self) -> String {
        self.home_ai
            .sync_device(self.selected.as_ref().map(|d| d.serial.as_str()));
        if self.home_ai.running() {
            return "手机 Amoo 正在分析首页上下文…".into();
        }
        // A previous model answer may have used an older/ambiguous parcel
        // snapshot. Do not keep presenting it while the new evidence is checked.
        self.home_ai.brief = None;
        self.home_ai.brief_at = None;
        if self.demo_mode {
            self.apply_demo_home_ai_brief();
            return "演示模式：已载入示例手机 Amoo 建议".into();
        }
        let Some(device) = self.selected.clone() else {
            self.home_ai.error = Some("连接手机后即可调用手机 Amoo 生成建议".into());
            return "连接手机后即可调用手机 Amoo 生成建议".into();
        };
        if let Some(reason) = self.home_ai_context_blocker(Instant::now()) {
            let msg = reason.to_string();
            self.home_ai.error = Some(msg.clone());
            return msg;
        }
        let adb = match stepx_companion::adb::Adb::discover() {
            Ok(adb) => adb,
            Err(e) => {
                let msg = format!("无法调用手机 Amoo：{e:#}");
                self.home_ai.error = Some(msg.clone());
                return msg;
            }
        };
        let context = self.home_ai_context();
        self.home_ai.error = None;
        self.home_ai.status = "正在连接手机 Amoo…".into();
        self.home_ai.auto_for = Some(device.serial.clone());
        self.home_ai.job = Some(home_ai::start(adb, device, context));
        "已请手机 Amoo 结合首页上下文生成建议".into()
    }

    pub(super) fn cancel_home_ai_brief(&mut self) {
        if let Some(job) = self.home_ai.job.take() {
            job.cancel();
        }
        self.home_ai.status.clear();
    }

    fn apply_demo_home_ai_brief(&mut self) {
        if let Ok(brief) = home_ai::parse_brief(DEMO_BRIEF_REPLY, "手机 Amoo · 演示") {
            self.home_ai.brief = Some(brief);
            self.home_ai.brief_at = Some(Local::now());
            self.home_ai.error = None;
        }
    }

    fn poll_home_ai(&mut self) -> bool {
        let device_changed = self
            .home_ai
            .sync_device(self.selected.as_ref().map(|d| d.serial.as_str()));
        let mut events = Vec::new();
        let mut finished = false;
        if let Some(job) = &self.home_ai.job {
            loop {
                match job.receiver.try_recv() {
                    Ok(event) => events.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if !events.iter().any(|e| {
                            matches!(
                                e,
                                home_ai::Event::Complete(_)
                                    | home_ai::Event::Failed(_)
                                    | home_ai::Event::Cancelled
                            )
                        }) {
                            events.push(home_ai::Event::Failed(
                                "手机 Amoo 连接已结束，请重试".into(),
                            ));
                        }
                        break;
                    }
                }
            }
        }
        let changed = device_changed || !events.is_empty();
        for event in events {
            match event {
                home_ai::Event::Status(s) => self.home_ai.status = s,
                home_ai::Event::Complete(brief) => {
                    self.error = false;
                    self.status = if brief.session_cleaned {
                        format!("已收到手机 Amoo 的 {} 条建议", brief.suggestions.len())
                    } else {
                        "已收到手机 Amoo 建议，但临时会话未确认删除，可在 Amoo 会话列表清理"
                            .into()
                    };
                    self.home_ai.brief = Some(brief);
                    self.home_ai.brief_at = Some(Local::now());
                    self.home_ai.error = None;
                    finished = true;
                }
                home_ai::Event::Failed(e) => {
                    self.home_ai.error = Some(e);
                    finished = true;
                }
                home_ai::Event::Cancelled => finished = true,
            }
        }
        if finished {
            self.home_ai.job = None;
            self.home_ai.status.clear();
        }
        changed
    }

    /// Demo captures can render a canned briefing without a network call. Real AI
    /// analysis is requested explicitly, so opening Home does not upload context or
    /// consume the phone account's model quota.
    fn maybe_auto_home_ai(&mut self) -> bool {
        if self.page != Page::Home || self.home_ai.running() {
            return false;
        }
        if self.demo_mode {
            if self.home_ai.brief.is_none() {
                self.apply_demo_home_ai_brief();
                return true;
            }
            return false;
        }
        false
    }

    pub(super) fn tick_home(&mut self, ctx: &mut ViewContext<Self>) {
        let mut changed = self.poll_home_ai();
        changed |= self.maybe_auto_home_ai();
        if changed {
            ctx.notify();
        }
        self.flush_svg_slots(ctx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fixed_chart_viewports_stay_square_and_inside_tight_or_unbounded_parents() {
        // Parent constraints are deliberately asymmetric: the former pair of
        // independent width/height clamps could allocate a rectangular image.
        for requested in [36., 42., 56., 96., 176.] {
            for max_width in [0., 8., 24., 36., 120., 4096., f32::INFINITY] {
                for max_height in [0., 12., 24., 36., 180., f32::INFINITY] {
                    for tight in [false, true] {
                        let max = vec2f(max_width, max_height);
                        let min = if tight { max } else { Vector2F::zero() };
                        let g = svg_canvas_geometry(Some(requested), requested, false, SizeConstraint::new(min, max));
                        assert!(g.outer.x().is_finite() && g.outer.y().is_finite());
                        assert!(g.image.x().is_finite() && g.image.y().is_finite());
                        assert!((g.image.x() - g.image.y()).abs() < 0.001, "ring became oval: {g:?}");
                        assert!(g.image.x() <= requested && g.image.y() <= requested);
                        assert!(g.offset.x() >= 0. && g.offset.y() >= 0., "negative chart offset for requested={requested}, constraint={constraint:?}: {g:?}", constraint = SizeConstraint::new(min, max));
                        assert!(g.offset.x() + g.image.x() <= g.outer.x() + 0.001);
                        assert!(g.offset.y() + g.image.y() <= g.outer.y() + 0.001);
                        assert!(g.outer.x() <= max_width && g.outer.y() <= max_height);
                    }
                }
            }
        }
    }

    #[test]
    fn rectangular_charts_keep_aspect_and_strip_placeholders_keep_the_full_track() {
        let rounded = svg_canvas_geometry(Some(176.), 176., false,
            SizeConstraint::new(Vector2F::zero(), vec2f(120., 12.)));
        assert_eq!(rounded.image.x(), rounded.image.y());
        assert!(rounded.image.y() <= 12. && rounded.offset.x() >= 0. && rounded.offset.y() >= 0.);
        let constrained = SizeConstraint::strict(vec2f(300., 12.));
        let g = svg_canvas_geometry(Some(180.), 96., false, constrained);
        assert!((g.image.x() / g.image.y() - 180. / 96.).abs() < 0.001);
        assert_eq!(g.outer, vec2f(300., 12.));
        assert_eq!(g.image, vec2f(22.5, 12.));
        assert_eq!(g.offset, vec2f(138.75, 0.));

        let c = SizeConstraint::new(Vector2F::zero(), vec2f(1024., f32::INFINITY));
        let stripe = svg_canvas_geometry(None, 12., true, c);
        assert_eq!(stripe.outer, vec2f(1024., 12.));
        assert_eq!(stripe.image, stripe.outer);
        assert_eq!(stripe.offset, Vector2F::zero());
        let unbounded = svg_canvas_geometry(None, 12., true, SizeConstraint::new(Vector2F::zero(), Vector2F::splat(f32::INFINITY)));
        assert!(unbounded.outer.x().is_finite() && unbounded.outer.y().is_finite());
    }

    #[test]
    fn svg_slots_track_pending_until_inserted() {
        let slots = SvgSlots::default();
        assert!(!slots.request("dial", "<svg>a</svg>".into()));
        assert_eq!(slots.pending_len(), 1);
        let pending = slots.take_pending();
        assert_eq!(pending.len(), 1);
        slots.mark_inserted(&pending[0].0, pending[0].1);
        // Same markup → ready, nothing pending.
        assert!(slots.request("dial", "<svg>a</svg>".into()));
        assert_eq!(slots.pending_len(), 0);
        // Changed markup → still ready (old raster) but pending replacement.
        assert!(slots.request("dial", "<svg>b</svg>".into()));
        assert_eq!(slots.pending_len(), 1);
        // Re-requesting the inserted markup cancels the pending replacement.
        assert!(slots.request("dial", "<svg>a</svg>".into()));
        assert_eq!(slots.pending_len(), 0);
    }

    #[test]
    fn demo_brief_parses_and_age_label() {
        let brief = home_ai::parse_brief(DEMO_BRIEF_REPLY, "demo").unwrap();
        assert_eq!(brief.suggestions.len(), 3);
        let mut ui = HomeAiUi::default();
        assert!(ui.age_label(Local::now()).is_none());
        ui.brief_at = Some(Local::now() - chrono::Duration::minutes(12));
        assert_eq!(ui.age_label(Local::now()).as_deref(), Some("12 分钟前"));
        ui.brief_at = Some(Local::now() - chrono::Duration::minutes(130));
        assert_eq!(ui.age_label(Local::now()).as_deref(), Some("2 小时前"));
        ui.brief_at = Some(Local::now() + chrono::Duration::minutes(5));
        assert_eq!(ui.age_label(Local::now()).as_deref(), Some("刚刚"));
    }

    #[test]
    fn home_ai_without_device_reports_error_and_demo_loads_brief() {
        let mut app = Companion::new_for_test();
        app.selected = None;
        app.demo_mode = false;
        let msg = app.start_home_ai_brief();
        assert!(msg.contains("连接手机"));
        assert!(app.home_ai.error.is_some());
        assert!(!app.home_ai.running());

        app.demo_mode = true;
        let _ = app.start_home_ai_brief();
        assert!(app.home_ai.brief.is_some());
        assert!(app.home_ai.error.is_none());
    }

    #[test]
    fn home_ai_rejects_incomplete_or_failed_snapshots_before_contacting_phone() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(stepx_companion::device::Device {
            serial: "home-ai-test-no-phone".into(),
            ..Default::default()
        });
        app.back_screen.stepos_suite.checked = true;
        app.push.loaded = true;
        app.push.last_sync = Instant::now();
        assert!(app.start_home_ai_brief().contains("尚未成功读取"));
        assert!(!app.home_ai.running());

        app.back_screen.stepos_suite_synced_at = Some(Instant::now());
        app.back_screen.stepos_suite_error = Some("partial calendar read".into());
        assert!(app.start_home_ai_brief().contains("未完整读取"));
        assert!(!app.home_ai.running());

        app.back_screen.stepos_suite_error = None;
        app.back_screen.stepos_suite.read_error = Some("partial bridge read".into());
        assert!(app.start_home_ai_brief().contains("未完整读取"));
        assert!(!app.home_ai.running());

        app.back_screen.stepos_suite.read_error = None;
        app.push.error = Some("notification permission unavailable".into());
        assert!(app.start_home_ai_brief().contains("消息同步未成功"));
        assert!(!app.home_ai.running());
    }

    #[test]
    fn home_ai_requires_recent_snapshots_but_accepts_real_zero_values() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(stepx_companion::device::Device {
            serial: "home-ai-test-no-phone".into(),
            ..Default::default()
        });
        let now = Instant::now();
        app.back_screen.stepos_suite.checked = true;
        app.back_screen.stepos_suite.calendar_events.clear();
        app.back_screen.stepos_suite.alarms.clear();
        app.back_screen.stepos_suite.wellbeing.screen_time_minutes = 0;
        app.back_screen.stepos_suite.wellbeing.unlock_count = 0;
        app.back_screen.stepos_suite.wellbeing.top_apps.clear();
        app.back_screen.stepos_suite_synced_at = Some(now);
        app.push.loaded = true;
        app.push.last_sync = now;
        // Readiness depends on successful reads, not nonempty lists or positive counts.
        assert!(app.home_ai_context_blocker(now).is_none());

        app.back_screen.stepos_suite_synced_at = Some(now - Duration::from_secs(301));
        assert!(app.start_home_ai_brief().contains("超过 5 分钟"));
        assert!(!app.home_ai.running());
        app.back_screen.stepos_suite_synced_at = Some(now);
        app.push.last_sync = now - Duration::from_secs(301);
        assert!(app.start_home_ai_brief().contains("超过 5 分钟"));
        assert!(!app.home_ai.running());

        app.push.last_sync = now;
        app.push.syncing = true;
        assert!(app.start_home_ai_brief().contains("还在同步"));
        assert!(!app.home_ai.running());
    }

    #[test]
    fn device_change_cancels_job_drops_late_results_and_clears_brief() {
        let (sender, receiver) = mpsc::channel();
        let cancelled = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let mut state = HomeAiUi::default();
        assert!(state.sync_device(Some("phone-a")));
        state.job = Some(home_ai::Job {
            receiver,
            cancel: cancelled.clone(),
        });
        state.brief = Some(home_ai::parse_brief(DEMO_BRIEF_REPLY, "demo").unwrap());
        state.brief_at = Some(Local::now());
        state.auto_for = Some("phone-a".into());
        assert!(!state.sync_device(Some("phone-a")));
        assert!(state.running());
        assert!(state.sync_device(Some("phone-b")));
        assert!(cancelled.load(std::sync::atomic::Ordering::Acquire));
        assert!(sender
            .send(home_ai::Event::Status("old result".into()))
            .is_err());
        assert!(!state.running());
        assert!(state.brief.is_none());
        assert!(state.brief_at.is_none());
        assert!(state.auto_for.is_none());
        assert!(state.sync_device(None));
        assert!(!state.sync_device(None));
    }

    #[test]
    fn real_home_never_automatically_uploads_context() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.page = Page::Home;
        app.back_screen.stepos_suite.checked = true;
        app.push.loaded = true;
        assert!(!app.maybe_auto_home_ai());
        assert!(!app.home_ai.running());
    }

    #[test]
    fn rendered_home_svgs_parse_with_resvg() {
        use facloud_ui::assets::asset_cache::Asset;
        let mut app = Companion::new_for_test();
        app.init_demo_push();
        app.home_tools_expanded = true;
        app.home_modules
            .apply_preset(crate::ui::pages::home::HomeModulePreset::CompleteAll);
        let _ = app.home_page();
        let pending = app.svg_slots.take_pending();
        let slots: std::collections::HashSet<_> = pending.iter().map(|(slot, _, _)| slot.as_str()).collect();
        for expected in ["dial", "usage-donut"] {
            assert!(slots.contains(expected), "home must retain fixed SVG chart {expected}: {slots:?}");
        }
        for native in ["timeline", "weekly-bars", "ctx-home-flow", "ctx-home-spectrum"] {
            assert!(!slots.contains(native), "responsive {native} must use actual-width native layout");
        }
        for (slot, _, svg) in pending {
            assert!(
                ImageType::try_from_bytes(svg.as_bytes()).is_ok(),
                "slot {slot} produced SVG that resvg cannot parse: {svg}"
            );
        }
    }

    #[test]
    fn complete_home_overview_tools_and_context_svgs_parse_with_resvg() {
        use facloud_ui::assets::asset_cache::Asset;
        let mut app = Companion::new_for_test();
        app.init_demo_push();
        assert!(
            app.home_tools_expanded,
            "home tools must be expanded by default"
        );
        app.home_modules
            .apply_preset(crate::ui::pages::home::HomeModulePreset::CompleteAll);
        app.selected = Some(stepx_companion::device::Device {
            serial: "test-svg-device".into(),
            model: "StepOS 12".into(),
            ..Default::default()
        });
        app.context.overview.evidence_count = 4;
        let _ = app.home_page();
        let _ = app.overview_page();
        let _ = app.gallery_page();
        for tool in [
            crate::ui::ToolPage::McpHub,
            crate::ui::ToolPage::AiSafety,
            crate::ui::ToolPage::Wellbeing,
        ] {
            app.tool_page = Some(tool);
            let _ = app.tools_page();
        }
        let pending = app.svg_slots.take_pending();
        let slots: std::collections::HashSet<String> =
            pending.iter().map(|(s, _, _)| s.clone()).collect();
        for expected in [
            "dial",
            "usage-donut",
            "ov-ring-link",
            "ov-ring-battery",
            "ov-ring-temp",
            "ov-ring-storage",
            "tool-mcp-spectrum",
            "tool-wellbeing-spectrum",
            "tool-wb-ring-screen",
            "tool-wb-ring-unlock",
            "tool-wb-ring-apps",
        ] {
            assert!(
                slots.contains(expected),
                "expected SVG slot {expected} to be rendered (got {slots:?})"
            );
        }
        for native in ["timeline", "weekly-bars", "ctx-home-flow", "ctx-home-spectrum"] {
            assert!(!slots.contains(native), "responsive {native} must not return to stretched SVG");
        }
        assert!(
            !slots.contains("tool-ai-safety-spectrum"),
            "AI safety renders phone capability evidence without a synthetic spectrum"
        );
        for (slot, _, svg) in pending {
            assert!(
                ImageType::try_from_bytes(svg.as_bytes()).is_ok(),
                "slot {slot} produced SVG that resvg cannot parse: {svg}"
            );
        }
    }

    #[test]
    fn non_fatal_calendar_action_note_does_not_reject_stepos_snapshot() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(stepx_companion::device::Device {
            serial: "home-ai-test-calendar-note".into(),
            ..Default::default()
        });
        let now = Instant::now();
        let mut report = stepx_companion::toolbox::default_stepos_ai_suite_report();
        report.checked = true;
        report.read_error = Some("部分电脑日历文件未能读取；双端日历未能保存到电脑".into());
        report.last_action_note =
            "已读取 2 项手机日程 · 读取电脑本地日历遇到问题：test.ics".into();
        assert!(
            !app.reject_incomplete_stepos_snapshot(&report),
            "non-fatal desktop .ics calendar warning must not reject phone StepOS snapshot"
        );

        app.back_screen.stepos_suite = report.clone();
        app.back_screen.stepos_suite_synced_at = Some(now);
        app.back_screen.stepos_suite_error = Some("部分电脑日程未能写入手机".into());
        app.push.loaded = true;
        app.push.last_sync = now;
        app.push.error = None;
        assert!(
            app.home_ai_context_blocker(now).is_none(),
            "non-fatal desktop calendar warning in read_error or stepos_suite_error must not block home_ai_context"
        );

        report.read_error = Some("部分电脑日历文件未能读取；亮屏统计未读取".into());
        assert!(
            app.reject_incomplete_stepos_snapshot(&report),
            "real phone read error mixed with desktop warning must still reject incomplete snapshot"
        );
        assert!(app.home_ai_context_blocker(now).is_some());
    }
}
