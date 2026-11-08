//! Home card surfaces and row layout. Keep measuring at the allocated width:
//! measuring at an unconstrained width changes wrapping and produces stale heights.
use super::*;
use facloud_ui::{AppContext, Element, event::DispatchedEvent};

/// Content contracts, not user-visible modes or a second set of layout settings.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(in crate::ui) enum HomeFrameKind {
    Calendar,
    List,
    Summary,
    Metric,
    Control,
}

impl HomeModuleId {
    pub(in crate::ui) fn frame_kind(self) -> HomeFrameKind {
        match self {
            Self::CalendarPanorama => HomeFrameKind::Calendar,
            Self::RecentFiles
            | Self::IssuesAndReplyModule
            | Self::WorkModule
            | Self::LifeModule
            | Self::ContextLedgerHub => HomeFrameKind::List,
            Self::ProductivityHub | Self::ContinuityWorkspace | Self::WeeklyAndAi => {
                HomeFrameKind::Summary
            }
            Self::WeatherModule | Self::PulseAndKpi => HomeFrameKind::Metric,
            Self::QuickStartRequest
            | Self::LiveCountdown
            | Self::AlarmsAndControl
            | Self::ToolsSummaryDock
            | Self::BackScreenAndHabits => HomeFrameKind::Control,
        }
    }
}

impl Companion {
    /// Common border, radius and safe inset. A title is always followed by real
    /// content; only an explicitly supplied footer is anchored to the lower edge.
    pub(in crate::ui) fn home_frame(&self, module: HomeModuleId, sections: Vec<El>) -> El {
        self.home_frame_with_footer(module, sections, None)
    }

    pub(in crate::ui) fn home_frame_with_footer(
        &self,
        module: HomeModuleId,
        mut sections: Vec<El>,
        footer: Option<El>,
    ) -> El {
        let p = self.p();
        let footer_index = footer.map(|footer| {
            let index = sections.len();
            sections.push(footer);
            index
        });
        // List sections are denser, not their rows. Controls keep generous
        // separation; neither text nor buttons scale to fill an assigned height.
        let spacing = match module.frame_kind() {
            HomeFrameKind::List => S2,
            _ => S3,
        };
        Container::new(HomeSections::new(sections, spacing, None, footer_index).finish())
            .with_uniform_padding(S4)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }

    pub(in crate::ui) fn home_aligned_columns(
        &self,
        min_wide: f32,
        gap: f32,
        stack_gap: f32,
        columns: Vec<(f32, El)>,
    ) -> El {
        HomeColumns {
            columns,
            min_wide,
            gap,
            stack_gap,
            grid: false,
            offsets: Vec::new(),
            size: None,
            origin: None,
        }
        .finish()
    }

    /// Shared rows retain their grid spans. A configured singleton uses its
    /// whole row without changing the saved span used after a later reorder.
    /// Inner card columns continue to use relative weights via home_aligned_columns.
    pub(in crate::ui) fn home_canvas_columns(&self, configured_count: usize, columns: Vec<(f32, El)>) -> El {
        HomeColumns {
            min_wide: if configured_count >= 3 { 1040. } else { 760. },
            columns,
            gap: S3,
            stack_gap: S3,
            grid: configured_count > 1,
            offsets: Vec::new(),
            size: None,
            origin: None,
        }.finish()
    }

    /// A composite card (e.g. an edit bar and its body) must pass the row's
    /// remaining height to its last surface instead of stretching an empty wrapper.
    pub(in crate::ui) fn home_surface_stack(&self, gap: f32, children: Vec<El>) -> El {
        let grow = children.len().checked_sub(1);
        HomeSections::new(children, gap, grow, None).finish()
    }
}

struct HomeColumns {
    columns: Vec<(f32, El)>,
    min_wide: f32,
    gap: f32,
    stack_gap: f32,
    grid: bool,
    offsets: Vec<Vector2F>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}

impl Element for HomeColumns {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let width = if c.max.x().is_finite() {
            c.max.x()
        } else {
            c.min.x().max(self.min_wide)
        };
        let horizontal = width >= self.min_wide && (self.grid || self.columns.len() > 1);
        let gap = if horizontal { self.gap } else { self.stack_gap };
        let gaps = gap * self.columns.len().saturating_sub(1) as f32;
        let total_weight: f32 = self.columns.iter().map(|(w, _)| w.max(0.01)).sum();
        self.offsets.clear();
        let mut sizes = Vec::with_capacity(self.columns.len());
        let mut natural_height: f32 = 0.;
        for (weight, child) in &mut self.columns {
            let w = if horizontal {
                if self.grid {
                    layout_config::home_grid_span_width(width, self.gap, *weight)
                } else {
                    (width - gaps).max(0.) * weight.max(0.01) / total_weight
                }
            } else {
                width
            };
            let size = child.layout(
                SizeConstraint::new(vec2f(w, 0.), vec2f(w, f32::INFINITY)),
                ctx,
                app,
            );
            natural_height = if horizontal {
                natural_height.max(size.y())
            } else {
                natural_height + size.y()
            };
            sizes.push(size);
        }
        if !horizontal {
            natural_height += gaps;
        }
        // Home scrolls vertically: never clip a real control to a fixed height.
        let height = natural_height.max(c.min.y());
        let mut pos = Vector2F::zero();
        for (index, (_, child)) in self.columns.iter_mut().enumerate() {
            let size = sizes[index];
            let child_height = if horizontal {
                height
            } else if index + 1 == sizes.len() {
                size.y() + height - natural_height
            } else {
                size.y()
            };
            // Flex::Stretch only grows infinite-size children. Finite cards need
            // this final pass. Re-layout only short columns, never duplicate trees.
            if child_height > size.y() + 0.01 {
                child.layout(
                    SizeConstraint::strict(vec2f(size.x(), child_height)),
                    ctx,
                    app,
                );
            }
            self.offsets.push(pos);
            if horizontal {
                pos += vec2f(size.x() + gap, 0.);
            } else {
                pos += vec2f(0., child_height + gap);
            }
        }
        let size = vec2f(width, height);
        self.size = Some(size);
        size
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for (_, child) in &mut self.columns {
            child.after_layout(ctx, app);
        }
    }
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        for ((_, child), offset) in self.columns.iter_mut().zip(&self.offsets) {
            child.paint(origin + *offset, ctx, app);
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
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        for (_, child) in self.columns.iter_mut().rev() {
            if child.dispatch_event(e, ctx, app) {
                return true;
            }
        }
        false
    }
}

struct HomeSections {
    children: Vec<El>,
    gap: f32,
    grow: Option<usize>,
    footer: Option<usize>,
    offsets: Vec<Vector2F>,
    size: Option<Vector2F>,
    origin: Option<Point>,
}
impl HomeSections {
    fn new(children: Vec<El>, gap: f32, grow: Option<usize>, footer: Option<usize>) -> Self {
        Self {
            children,
            gap,
            grow,
            footer,
            offsets: Vec::new(),
            size: None,
            origin: None,
        }
    }
}
impl Element for HomeSections {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        let width = if c.max.x().is_finite() {
            c.max.x()
        } else {
            c.min.x()
        };
        let mut sizes = Vec::with_capacity(self.children.len());
        let mut natural = self.gap * self.children.len().saturating_sub(1) as f32;
        for child in &mut self.children {
            let size = child.layout(
                SizeConstraint::new(vec2f(width, 0.), vec2f(width, f32::INFINITY)),
                ctx,
                app,
            );
            natural += size.y();
            sizes.push(size);
        }
        let height = natural.max(c.min.y());
        let extra = height - natural;
        self.offsets.clear();
        let mut y = 0.;
        for (index, child) in self.children.iter_mut().enumerate() {
            if self.footer == Some(index) {
                y += extra;
            }
            self.offsets.push(vec2f(0., y));
            let mut h = sizes[index].y();
            if self.grow == Some(index) && extra > 0.01 {
                h += extra;
                child.layout(SizeConstraint::strict(vec2f(width, h)), ctx, app);
            }
            y += h + self.gap;
        }
        let size = vec2f(width, height);
        self.size = Some(size);
        size
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        for child in &mut self.children {
            child.after_layout(ctx, app);
        }
    }
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin = Some(Point::from_vec2f(origin, ctx.scene.z_index()));
        for (child, offset) in self.children.iter_mut().zip(&self.offsets) {
            child.paint(origin + *offset, ctx, app);
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
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        for child in self.children.iter_mut().rev() {
            if child.dispatch_event(e, ctx, app) {
                return true;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Frames(Companion);
    impl facloud_ui::Entity for Frames {
        type Event = ();
    }
    impl facloud_ui::TypedActionView for Frames {
        type Action = ();
    }
    impl facloud_ui::View for Frames {
        fn ui_name() -> &'static str {
            "home_frame_contract"
        }
        fn render(&self, _: &AppContext) -> El {
            let a = &self.0;
            let text =
                |id: &str, value: &str| home_geometry_probe(id, a.text(value, BODY, a.p().text));
            let tall = a.home_frame(
                HomeModuleId::WorkModule,
                vec![text(
                    "long-copy",
                    &"工作安排与会议，需要完整可读的真实内容。".repeat(28),
                )],
            );
            let short = home_geometry_probe(
                "short-surface",
                a.home_frame_with_footer(
                    HomeModuleId::RecentFiles,
                    vec![text("short-title", "最近文件")],
                    Some(text("footer", "查看全部文件")),
                ),
            );
            let edited = a.home_surface_stack(4., vec![text("edit-bar", "移动 / 调整尺寸"), short]);
            let nested = a.home_aligned_columns(
                520.,
                S2,
                S2,
                vec![
                    (
                        1.,
                        home_geometry_probe(
                            "nested-a",
                            a.home_frame(
                                HomeModuleId::BackScreenAndHabits,
                                vec![text("nested-copy", "背屏状态")],
                            ),
                        ),
                    ),
                    (
                        1.,
                        home_geometry_probe(
                            "nested-b",
                            a.home_frame(
                                HomeModuleId::PulseAndKpi,
                                vec![text("nested-copy-b", "使用情况")],
                            ),
                        ),
                    ),
                ],
            );
            let body = a.home_aligned_columns(
                760.,
                S3,
                S3,
                vec![
                    (1., home_geometry_probe("tall", tall)),
                    (1., home_geometry_probe("edited", edited)),
                    (2., home_geometry_probe("nested", nested)),
                ],
            );
            Stack::new()
                .with_child(Align::new(body).top_left().finish())
                .finish()
        }
    }

    #[test]
    fn home_frames_native_relayout_aligns_surfaces_and_footer_without_stretching_content() {
        facloud_ui::App::test((), |mut app| async move {
            let (window, _) = app
                .add_window(facloud_ui::platform::WindowStyle::NotStealFocus, |_| {
                    Frames(Companion::new_for_test())
                });
            let root = app.root_view_id(window).unwrap();
            app.update(move |ctx| {
                let mut presenter = facloud_ui::Presenter::new(window);
                presenter.invalidate(
                    facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]),
                        ..Default::default()
                    },
                    ctx,
                );
                // Reuse the exact element tree across native resize, including
                // both responsive breakpoints; no viewport-state update required.
                for width in [1400., 740., 1100., 320., 1400.] {
                    presenter.build_scene(vec2f(width, 5000.), 1., None, ctx);
                    let cache = presenter.position_cache();
                    let rect = |id| cache.get_position(id).unwrap();
                    let tall = rect("tall");
                    let edited = rect("edited");
                    let nested = rect("nested");
                    if width >= 760. {
                        assert!(
                            (tall.max_y() - edited.max_y()).abs() < 1.,
                            "row bottoms {width}: {tall:?}/{edited:?}"
                        );
                        assert!(
                            (tall.max_y() - nested.max_y()).abs() < 1.,
                            "nested column bottom {width}"
                        );
                    } else {
                        assert!(tall.max_y() + S3 - 1. <= edited.origin().y());
                        assert!(edited.max_y() + S3 - 1. <= nested.origin().y());
                    }
                    let short = rect("short-surface");
                    let footer = rect("footer");
                    let title = rect("short-title");
                    assert!(
                        (short.max_y() - edited.max_y()).abs() < 1.,
                        "editor wrapper must stretch the actual surface"
                    );
                    assert!(
                        (short.max_y() - footer.max_y() - S4).abs() < 2.,
                        "footer safe inset {width}: {footer:?}/{short:?}"
                    );
                    assert!(title.max_y() < footer.origin().y());
                    assert!(
                        title.size().y() < 30.,
                        "title must retain natural line height"
                    );
                    for id in [
                        "long-copy",
                        "short-title",
                        "footer",
                        "edit-bar",
                        "nested-a",
                        "nested-b",
                    ] {
                        let r = rect(id);
                        assert!(
                            r.origin().x() >= 0. && r.max_x() <= width + 1.,
                            "overflow {width} {id}: {r:?}"
                        );
                    }
                    let a = rect("nested-a");
                    let b = rect("nested-b");
                    assert!(
                        (b.max_y() - nested.max_y()).abs() < 1.,
                        "stacked inner surface must reach parent bottom"
                    );
                    if nested.size().x() >= 520. {
                        assert!((a.max_y() - b.max_y()).abs() < 1.);
                    } else {
                        assert!(a.max_y() + S2 - 1. <= b.origin().y());
                    }
                }
            });
        });
    }
}
