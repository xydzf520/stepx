//! Reusable components. Every visual value comes from `theme`.

use super::{
    icons::{self, UiIcon},
    theme::*,
    Action, Companion,
};
use facloud_ui::{
    color::ColorU,
    elements::*,
    fonts::{Properties, Weight},
    platform::Cursor,
    text_layout::ClipConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Btn {
    Primary,
    Secondary,
    Ghost,
    Danger,
    DangerSolid,
}

pub type El = Box<dyn Element>;

type FileProbeEntry = (std::time::Instant, bool, u64, Option<(u32, u32)>);

static FILE_PROBE_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<std::path::PathBuf, FileProbeEntry>>,
> = std::sync::OnceLock::new();

fn probe_file_cached(path: &std::path::Path, need_dims: bool) -> (bool, u64, Option<(u32, u32)>) {
    const TTL: std::time::Duration = std::time::Duration::from_secs(2);
    let cache =
        FILE_PROBE_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()));
    if let Ok(mut map) = cache.lock() {
        if let Some((at, is_file, len, dims)) = map.get_mut(path) {
            if at.elapsed() < TTL {
                if !need_dims || !*is_file || dims.is_some() {
                    return (*is_file, *len, dims.filter(|&(w, h)| w > 0 && h > 0));
                }
                let probed = image::image_dimensions(path).ok().unwrap_or((0, 0));
                *dims = Some(probed);
                return (
                    *is_file,
                    *len,
                    (probed.0 > 0 && probed.1 > 0).then_some(probed),
                );
            }
        }
    }
    let meta = std::fs::metadata(path).ok();
    let is_file = meta.as_ref().is_some_and(|m| m.is_file());
    let len = meta.as_ref().map(|m| m.len()).unwrap_or(0);
    let dims_raw = if need_dims && is_file {
        Some(image::image_dimensions(path).ok().unwrap_or((0, 0)))
    } else {
        None
    };
    if let Ok(mut map) = cache.lock() {
        if map.len() >= 512 {
            map.retain(|_, (at, ..)| at.elapsed() < TTL);
            if map.len() >= 512 {
                map.clear();
            }
        }
        map.insert(
            path.to_path_buf(),
            (std::time::Instant::now(), is_file, len, dims_raw),
        );
    }
    (is_file, len, dims_raw.filter(|&(w, h)| w > 0 && h > 0))
}

pub(super) fn cached_is_file(path: &std::path::Path) -> bool {
    probe_file_cached(path, false).0
}

pub(super) fn cached_file_len(path: &std::path::Path) -> Option<u64> {
    let (is_file, len, _) = probe_file_cached(path, false);
    is_file.then_some(len)
}

pub(super) fn cached_image_dimensions(path: &std::path::Path) -> Option<(u32, u32)> {
    probe_file_cached(path, true).2
}

pub(super) fn invalidate_cached_file(path: &std::path::Path) {
    if let Some(cache) = FILE_PROBE_CACHE.get() {
        if let Ok(mut map) = cache.lock() {
            map.remove(path);
        }
    }
}

pub(super) fn ui_label(
    text: String,
    font: facloud_ui::fonts::FamilyId,
    size: f32,
    color: ColorU,
    weight: Weight,
) -> El {
    let Some((icon, rest)) = icons::label_icon(&text) else {
        return Text::new(text, font, size)
            .with_color(color)
            .with_style(Properties::default().weight(weight))
            .soft_wrap(false)
            .with_clip(ClipConfig::ellipsis())
            .finish();
    };
    icon_label(icon, rest, font, size, color, weight)
}

fn icon_label(
    icon: UiIcon,
    rest: &str,
    font: facloud_ui::fonts::FamilyId,
    size: f32,
    color: ColorU,
    weight: Weight,
) -> El {
    let rest = match icons::label_icon(rest) {
        Some((_, stripped)) => stripped,
        None => rest,
    };
    let icon_size = size.max(14.);
    let image = icons::draw_icon(icon, icon_size, color);
    if rest.is_empty() {
        return image;
    }
    // A non-flex Text inside a row is measured with infinite width, losing its
    // ellipsis. Padding preserves the parent's width constraint while a
    // positioned icon still allows buttons to measure their intrinsic width.
    let label = Container::new(
        Text::new(rest.to_string(), font, size)
            .with_color(color)
            .with_style(Properties::default().weight(weight))
            .soft_wrap(false)
            .with_clip(ClipConfig::ellipsis())
            .finish(),
    )
    .with_padding_left(icon_size + 6.)
    .finish();
    Stack::new()
        .with_child(
            ConstrainedBox::new(label)
                .with_min_height(icon_size)
                .finish(),
        )
        .with_positioned_child(
            image,
            OffsetPositioning::offset_from_parent(
                pathfinder_geometry::vector::vec2f(0., 0.),
                ParentOffsetBounds::Unbounded,
                ParentAnchor::MiddleLeft,
                ChildAnchor::MiddleLeft,
            ),
        )
        .finish()
}

impl Companion {
    pub fn p(&self) -> Palette {
        self.palette.get()
    }

    pub fn hover_t(&self, mouse: &MouseState) -> f32 {
        if self.settings.value.reduced_motion {
            if mouse.is_mouse_over_element() {
                1.0
            } else {
                0.0
            }
        } else {
            let p = mouse.hover_progress().clamp(0.0, 1.0);
            1.0 - (1.0 - p).powi(2)
        }
    }

    pub fn toggle_switch(&self, id: impl Into<String>, on: bool, action: Action) -> El {
        let id = id.into();
        let p = self.p();
        let state = self.mouse_state(id.clone());
        let toggle = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let base_track = if on { p.success } else { p.border_strong };
            let hover_target = if on { p.success_text } else { p.text3 };
            let hover_track = lerp_color(base_track, hover_target, 0.35);
            let mut track_bg = lerp_color(base_track, hover_track, t);
            if clicked {
                track_bg = lerp_color(track_bg, p.accent, 0.18);
            }
            let thumb_w = if clicked { 20. } else { 18. };
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(if on {
                        MainAxisAlignment::End
                    } else {
                        MainAxisAlignment::Start
                    })
                    .with_child(
                        ConstrainedBox::new(
                            Container::new(Empty::new().finish())
                                .with_background_color(rgb(255, 255, 255))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
                                .finish(),
                        )
                        .with_width(thumb_w)
                        .with_height(18.)
                        .finish(),
                    )
                    .finish(),
            )
            .with_uniform_padding(3.)
            .with_background_color(track_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let control = ConstrainedBox::new(toggle)
            .with_width(42.)
            .with_height(24.)
            .finish();
        #[cfg(feature = "ui-capture")]
        let control = super::scenario_check::control(id, control);
        control
    }

    // ---- Text ----

    pub fn text(&self, text: impl Into<String>, size: f32, color: ColorU) -> El {
        Text::new(text.into(), self.font, size)
            .with_color(color)
            .with_line_height_ratio(1.45)
            .finish()
    }

    pub fn text_w(&self, text: impl Into<String>, size: f32, color: ColorU, weight: Weight) -> El {
        Text::new(text.into(), self.font, size)
            .with_color(color)
            .with_style(Properties::default().weight(weight))
            .finish()
    }

    /// Single line, truncated with an ellipsis when it does not fit.
    pub fn line(&self, text: impl Into<String>, size: f32, color: ColorU) -> El {
        ui_label(text.into(), self.font, size, color, Weight::Normal)
    }

    pub fn line_w(&self, text: impl Into<String>, size: f32, color: ColorU, weight: Weight) -> El {
        ui_label(text.into(), self.font, size, color, weight)
    }

    pub fn mono(&self, text: impl Into<String>, size: f32, color: ColorU) -> El {
        Text::new(text.into(), self.mono_font, size)
            .with_color(color)
            .soft_wrap(false)
            .with_clip(ClipConfig::ellipsis())
            .finish()
    }

    pub fn heading(&self, text: impl Into<String>) -> El {
        let p = self.p();
        self.line_w(text, HEADING, p.text, Weight::Semibold)
    }

    pub fn caption(&self, text: impl Into<String>) -> El {
        let p = self.p();
        let s = text.into();
        if !s.contains('\n') && icons::label_icon(&s).is_some() {
            return ui_label(s, self.font, CAPTION, p.text2, Weight::Normal);
        }
        self.text(s, CAPTION, p.text2)
    }

    // ---- Layout ----

    pub fn hstack(&self, spacing: f32, children: Vec<El>) -> El {
        Flex::row()
            .with_spacing(spacing)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_children(children)
            .finish()
    }

    pub fn vstack(&self, spacing: f32, children: Vec<El>) -> El {
        Flex::column()
            .with_spacing(spacing)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children(children)
            .finish()
    }

    pub fn wrap(&self, children: Vec<El>) -> El {
        Wrap::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S2)
            .with_run_spacing(S2)
            .with_children(children)
            .finish()
    }

    /// Left side takes the remaining width (and truncates/wraps); right side keeps its size.
    pub fn between(&self, left: El, right: El) -> El {
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
            .with_spacing(S3)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(Expanded::new(1., left).finish())
            .with_child(right)
            .finish()
    }

    pub fn fixed_w(&self, width: f32, child: El) -> El {
        ConstrainedBox::new(child).with_width(width).finish()
    }

    /// Fixed leading element followed by content side-by-side.
    pub fn lead(&self, spacing: f32, leading: El, rest: El) -> El {
        self.hstack(spacing, vec![leading, rest])
    }

    pub fn spacer(&self) -> El {
        Expanded::new(1., Empty::new().finish()).finish()
    }

    pub fn divider(&self) -> El {
        let p = self.p();
        ConstrainedBox::new(Rect::new().with_background_color(p.border).finish())
            .with_height(1.)
            .finish()
    }

    #[allow(dead_code)]
    pub fn vdivider(&self, height: f32) -> El {
        let p = self.p();
        ConstrainedBox::new(Rect::new().with_background_color(p.border).finish())
            .with_width(1.)
            .with_height(height)
            .finish()
    }

    // ---- Containers ----

    pub fn panel(&self, children: Vec<El>) -> El {
        let p = self.p();
        Container::new(self.vstack(S3, children))
            .with_uniform_padding(S4)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }

    pub(super) fn hover_panel(&self, id: impl Into<String>, content: El) -> El {
        self.hover_panel_interactive(id, content, None, None, None)
    }

    pub(super) fn hover_panel_interactive(
        &self,
        id: impl Into<String>,
        content: El,
        on_click: Option<Action>,
        on_double: Option<Action>,
        menu_target: Option<super::context_menu::Target>,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("hpanel:{}", id.into()));
        let interactive = on_click.is_some() || on_double.is_some() || menu_target.is_some();
        let mut h = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let border_col = lerp_color(p.border, p.border_strong, t);
            let bg_col = if interactive {
                let hover_bg = lerp_color(p.bg, p.hover, 0.45);
                if mouse.is_clicked() {
                    lerp_color(p.bg, p.press_color(p.hover), 0.55)
                } else {
                    lerp_color(p.bg, hover_bg, t)
                }
            } else {
                p.bg
            };
            Container::new(content)
                .with_uniform_padding(S4)
                .with_background_color(bg_col)
                .with_border(Border::all(1.).with_border_color(border_col))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish()
        })
        .with_defer_events_to_children();
        if interactive {
            h = h.with_cursor(Cursor::PointingHand);
        }
        if let Some(action) = on_click {
            h = h.on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()));
        }
        if let Some(action) = on_double {
            h = h.on_double_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()));
        }
        if let Some(target) = menu_target {
            h = h.on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::ContextOpen(target.clone(), at))
            });
        }
        h.finish()
    }

    pub(super) fn context_target(
        &self,
        id: impl Into<String>,
        target: super::context_menu::Target,
        content: El,
    ) -> El {
        let state = self.mouse_state(format!("ctx:{}", id.into()));
        Hoverable::new(state, move |_| content)
            .with_defer_events_to_children()
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::ContextOpen(target.clone(), at))
            })
            .finish()
    }

    /// Bordered list: rows separated by 1px dividers, no inner padding.
    pub fn list(&self, rows: Vec<El>) -> El {
        let p = self.p();
        let mut items = Vec::with_capacity(rows.len() * 2);
        for (i, row) in rows.into_iter().enumerate() {
            if i > 0 {
                items.push(self.divider());
            }
            items.push(row);
        }
        Container::new(self.vstack(0., items))
            .with_uniform_padding(1.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }

    /// Section: heading row (title + optional trailing control) above its body.
    pub fn section(&self, title: impl Into<String>, trailing: Option<El>, body: El) -> El {
        let header = match trailing {
            Some(t) => self.between(self.heading(title), t),
            None => self.heading(title),
        };
        self.vstack(S2 + 2., vec![header, body])
    }

    pub fn list_row(
        &self,
        id: impl Into<String>,
        content: El,
        selected: bool,
        on_click: Option<Action>,
        on_double: Option<Action>,
    ) -> El {
        self.list_row_with_menu(id, content, selected, on_click, on_double, None)
    }

    pub fn list_row_with_menu(
        &self,
        id: impl Into<String>,
        content: El,
        selected: bool,
        on_click: Option<Action>,
        on_double: Option<Action>,
        menu_target: Option<super::context_menu::Target>,
    ) -> El {
        let id = id.into();
        let p = self.p();
        let state = self.mouse_state(format!("row:{id}"));
        let interactive = on_click.is_some() || on_double.is_some() || menu_target.is_some();
        let mut row = Hoverable::new(state, move |mouse| {
            let t = if interactive {
                self.hover_t(&mouse)
            } else {
                0.0
            };
            let clicked = interactive && mouse.is_clicked();
            let bg = if selected {
                if clicked {
                    p.press_color(p.hover)
                } else {
                    lerp_color(p.selected, p.hover, t * 0.45)
                }
            } else if clicked {
                p.press_color(p.hover)
            } else {
                lerp_color(ColorU::transparent_black(), p.hover, t)
            };
            let border = if selected {
                Border::all(1.).with_border_color(lerp_color(p.border, p.border_strong, 0.65))
            } else {
                Border::all(1.).with_border_color(lerp_color(
                    ColorU::transparent_black(),
                    p.border,
                    t * 0.6,
                ))
            };
            Container::new(content)
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2 + 2.)
                .with_background_color(bg)
                .with_border(border)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_defer_events_to_children();
        if interactive {
            row = row.with_cursor(Cursor::PointingHand);
        }
        if let Some(action) = on_click {
            row = row.on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()));
        }
        if let Some(action) = on_double {
            row = row.on_double_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()));
        }
        if let Some(target) = menu_target {
            row = row.on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::ContextOpen(target.clone(), at))
            });
        }
        let control = row.finish();
        #[cfg(feature = "ui-capture")]
        let control = super::scenario_check::control(id, control);
        control
    }

    pub fn empty(
        &self,
        title: impl Into<String>,
        detail: impl Into<String>,
        action: Option<El>,
    ) -> El {
        let p = self.p();
        let mut items = vec![
            Align::new(self.text_w(title, HEADING, p.text, Weight::Medium)).finish(),
            Align::new(self.text(detail, CAPTION, p.text3)).finish(),
        ];
        if let Some(a) = action {
            items.push(Align::new(Container::new(a).with_margin_top(S2).finish()).finish());
        }
        Container::new(self.vstack(S1 + 2., items))
            .with_vertical_padding(S6)
            .with_horizontal_padding(S4)
            .finish()
    }

    /// Inline confirmation used for irreversible actions.
    pub fn confirm_bar(&self, message: impl Into<String>, danger: bool, buttons: Vec<El>) -> El {
        let p = self.p();
        Container::new(self.between(
            self.text(message, BODY, if danger { p.danger } else { p.text }),
            self.hstack(S2, buttons),
        ))
        .with_horizontal_padding(S3)
        .with_vertical_padding(S2)
        .with_background_color(if danger { p.danger_soft } else { p.accent_soft })
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish()
    }

    // ---- Small parts ----

    pub fn tag(&self, text: impl Into<String>, fg: ColorU) -> El {
        let p = self.p();
        Container::new(ui_label(text.into(), self.font, MICRO, fg, Weight::Medium))
            .with_horizontal_padding(6.)
            .with_vertical_padding(2.)
            .with_background_color(p.tag_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_TAG)))
            .finish()
    }

    pub fn dot(&self, color: ColorU) -> El {
        ConstrainedBox::new(
            Rect::new()
                .with_background_color(color)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
        )
        .with_width(8.)
        .with_height(8.)
        .finish()
    }

    pub fn progress(&self, percent: Option<u8>, color: ColorU) -> El {
        let p = self.p();
        let pct = percent.unwrap_or(0).min(100) as f32;
        let bar = |c: ColorU| {
            ConstrainedBox::new(Rect::new().with_background_color(c).finish())
                .with_height(6.)
                .finish()
        };
        let mut row = Flex::row().with_main_axis_size(MainAxisSize::Max);
        if pct > 0. {
            row = row.with_child(Expanded::new(pct, bar(color)).finish());
        }
        if pct < 100. {
            row = row.with_child(Expanded::new(100. - pct, bar(p.tag_bg)).finish());
        }
        Clipped::new(
            Container::new(row.finish())
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                .finish(),
        )
        .finish()
    }

    // ---- Controls ----

    pub fn button(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        action: Action,
        kind: Btn,
    ) -> El {
        self.button_control(id.into(), label.into(), None, action, kind)
    }

    pub fn button_icon(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        icon: UiIcon,
        action: Action,
        kind: Btn,
    ) -> El {
        self.button_control(id.into(), label.into(), Some(icon), action, kind)
    }

    fn button_control(
        &self,
        id: String,
        label: String,
        icon: Option<UiIcon>,
        action: Action,
        kind: Btn,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("btn:{id}"));
        let font = self.font;
        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let (rest_bg, hover_bg, press_bg, rest_border, hover_border, rest_fg, hover_fg) =
                match kind {
                    Btn::Primary => {
                        let press = lerp_color(p.accent, p.text2, 0.25);
                        (
                            p.accent,
                            p.accent_hover,
                            press,
                            p.accent,
                            p.accent_hover,
                            p.on_accent,
                            p.on_accent,
                        )
                    }
                    Btn::Secondary => (
                        p.bg,
                        p.hover,
                        p.press_color(p.hover),
                        p.border,
                        p.border_strong,
                        p.text,
                        p.text,
                    ),
                    Btn::Ghost => (
                        ColorU::transparent_black(),
                        p.hover,
                        p.accent_soft,
                        ColorU::transparent_black(),
                        ColorU::transparent_black(),
                        p.text2,
                        p.text,
                    ),
                    Btn::Danger => (
                        p.bg,
                        p.danger_soft,
                        lerp_color(p.danger_soft, p.danger, 0.22),
                        p.border,
                        lerp_color(p.border, p.danger, 0.35),
                        p.danger,
                        p.danger,
                    ),
                    Btn::DangerSolid => {
                        let hov = lerp_color(p.danger, ColorU::white(), 0.12);
                        let press = lerp_color(p.danger, ColorU::black(), 0.16);
                        (
                            p.danger,
                            hov,
                            press,
                            p.danger,
                            hov,
                            p.on_accent,
                            p.on_accent,
                        )
                    }
                };
            let bg = if clicked {
                press_bg
            } else {
                lerp_color(rest_bg, hover_bg, t)
            };
            let border = if clicked {
                match kind {
                    Btn::Primary | Btn::DangerSolid => press_bg,
                    Btn::Secondary => p.border_strong,
                    Btn::Ghost => ColorU::transparent_black(),
                    Btn::Danger => p.danger,
                }
            } else {
                lerp_color(rest_border, hover_border, t)
            };
            let fg = if clicked && matches!(kind, Btn::Ghost) {
                p.text
            } else {
                lerp_color(rest_fg, hover_fg, t)
            };
            Container::new(
                ConstrainedBox::new(
                    Align::new({
                        let weight = if matches!(kind, Btn::Primary | Btn::DangerSolid) {
                            Weight::Medium
                        } else {
                            Weight::Normal
                        };
                        match icon {
                            Some(icon) => icon_label(icon, &label, font, BODY, fg, weight),
                            None => ui_label(label.clone(), font, BODY, fg, weight),
                        }
                    })
                    .finish(),
                )
                .with_height(28.)
                .finish(),
            )
            .with_horizontal_padding(S3)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        #[cfg(feature = "ui-capture")]
        let control = super::scenario_check::control(id, control);
        control
    }

    /// Text-style link button (used inside rows and breadcrumbs).
    pub fn link(&self, id: impl Into<String>, label: impl Into<String>, action: Action) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("link:{}", id.into()));
        let label = label.into();
        let font = self.font;
        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let fg = if mouse.is_clicked() {
                p.text2
            } else {
                lerp_color(p.accent, p.accent_hover, t)
            };
            ui_label(label.clone(), font, BODY, fg, Weight::Normal)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    /// Quiet inline action (e.g. under a message). Muted until hovered.
    pub fn link_muted(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("mlink:{}", id.into()));
        let label = label.into();
        let font = self.font;
        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let fg = if mouse.is_clicked() {
                p.text
            } else {
                lerp_color(p.text3, p.accent, t)
            };
            ui_label(label.clone(), font, CAPTION, fg, Weight::Normal)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    pub fn disabled_button(&self, label: impl Into<String>) -> El {
        let p = self.p();
        Container::new(
            ConstrainedBox::new(Align::new(self.line(label, BODY, p.text3)).finish())
                .with_height(28.)
                .finish(),
        )
        .with_horizontal_padding(S3)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish()
    }

    /// Segmented control for 2–4 mutually exclusive options.
    pub fn segmented(&self, id: &str, items: Vec<(String, bool, Action)>) -> El {
        let p = self.p();
        let font = self.font;
        let children: Vec<El> = items
            .into_iter()
            .enumerate()
            .map(|(i, (label, active, action))| {
                let state = self.mouse_state(format!("seg:{id}:{i}"));
                Hoverable::new(state, move |mouse| {
                    let t = self.hover_t(&mouse);
                    let clicked = mouse.is_clicked();
                    let bg = if active {
                        if clicked {
                            lerp_color(p.bg, p.hover, 0.4)
                        } else {
                            p.bg
                        }
                    } else if clicked {
                        p.press_color(p.hover)
                    } else {
                        lerp_color(ColorU::transparent_black(), p.hover, t * 0.7)
                    };
                    let fg = if active || clicked {
                        p.text
                    } else {
                        lerp_color(p.text2, p.text, t)
                    };
                    let border_col = if active {
                        lerp_color(p.border, p.border_strong, t * 0.6)
                    } else if clicked {
                        p.border
                    } else {
                        ColorU::transparent_black()
                    };
                    Container::new(ui_label(
                        label.clone(),
                        font,
                        BODY,
                        fg,
                        if active {
                            Weight::Medium
                        } else {
                            Weight::Normal
                        },
                    ))
                    .with_horizontal_padding(S3)
                    .with_vertical_padding(4.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border_col))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL - 1.)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish()
            })
            .collect();
        Container::new(self.hstack(2., children))
            .with_uniform_padding(2.)
            .with_background_color(p.sidebar)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL + 1.)))
            .finish()
    }

    /// Shape each hard line separately: native paragraph shaping uses local
    /// character indices. Keep the active caret local too, including CJK/emoji.
    pub(super) fn multiline_field_text(
        &self,
        value: &str,
        caret: usize,
        index: usize,
        color: ColorU,
    ) -> El {
        let mut offset = 0;
        let mut byte_offset = 0;
        let paragraphs = value
            .split('\n')
            .map(|line| {
                let chars = line.chars().count();
                let text = Text::new(
                    if line.is_empty() { " " } else { line }.to_owned(),
                    self.font,
                    BODY,
                )
                .with_color(color)
                .with_line_height_ratio(1.45);
                let local_caret =
                    (caret >= offset && caret <= offset + chars).then(|| caret - offset);
                offset += chars + 1;
                let element = self.editable_line(text, line, byte_offset, index, local_caret);
                byte_offset += line.len() + 1;
                element
            })
            .collect();
        self.vstack(0., paragraphs)
    }

    /// Text field bound to `self.fields[index]`. Without a width it fills the parent.
    pub fn input(&self, index: usize, placeholder: &str, width: Option<f32>) -> El {
        let p = self.p();
        let field = &self.fields[index];
        let focused = self.focus == Some(index);
        let masked =
            (index == super::F_PAIR_CODE && !self.show_secret) || index == super::F_LAB_KEY;
        let mut value = if masked {
            "•".repeat(field.text.chars().count())
        } else {
            field.text.clone()
        };
        let mut caret_index = 0;
        if focused {
            let safe_cursor = field.cursor.min(field.text.len());
            let cursor = if masked {
                field.text[..safe_cursor].chars().count() * "•".len()
            } else {
                safe_cursor
            };
            let preedit = if masked {
                "•".repeat(field.preedit.chars().count())
            } else {
                field.preedit.clone()
            };
            caret_index = value[..cursor].chars().count() + preedit.chars().count();
            value.insert_str(cursor.min(value.len()), &preedit);
        }
        let is_empty = field.text.is_empty() && field.preedit.is_empty();
        if value.is_empty() {
            value = placeholder.into();
        }
        let text_color = if is_empty && !focused {
            p.text3
        } else {
            p.text
        };
        let selected_all = field.all;
        let font = self.font;
        let state = self.mouse_state(format!("input:{index}"));
        let field_box = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let border = if focused {
                p.accent
            } else {
                lerp_color(p.surface_border, p.border_strong, t)
            };
            Container::new({
                let text = Text::new(value.clone(), font, BODY)
                    .with_color(text_color)
                    .soft_wrap(false)
                    .with_clip(ClipConfig::ellipsis());
                self.editable_line(text, &value, 0, index, Some(caret_index))
            })
            .with_horizontal_padding(10.)
            .with_vertical_padding(6.)
            .with_background_color(if selected_all {
                p.accent_soft
            } else {
                p.surface
            })
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
        })
        .with_cursor(Cursor::IBeam)
        .with_defer_events_to_children()
        .on_mouse_down(move |ctx, _, _| ctx.dispatch_typed_action(Action::Focus(index)))
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::Input(index),
                at,
            ))
        })
        .finish();

        match width {
            Some(w) => self.fixed_w(w, field_box),
            None => field_box,
        }
    }

    pub fn checkbox_row(
        &self,
        id: impl Into<String>,
        checked: bool,
        label: impl Into<String>,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("chk:{}", id.into()));
        let label = label.into();
        let font = self.font;
        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let box_bg = if checked {
                if clicked {
                    lerp_color(p.accent, p.text2, 0.25)
                } else {
                    lerp_color(p.accent, p.accent_hover, t)
                }
            } else if clicked {
                p.accent_soft
            } else {
                lerp_color(p.bg, p.hover, t)
            };
            let box_border = if checked {
                box_bg
            } else {
                lerp_color(p.border_strong, p.text2, t * 0.5)
            };
            let label_fg = if clicked {
                p.text
            } else {
                lerp_color(p.text2, p.text, t)
            };
            let mark = ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(if checked {
                            icons::draw_icon(UiIcon::Check, 11., p.on_accent)
                        } else {
                            Empty::new().finish()
                        })
                        .finish(),
                )
                .with_background_color(box_bg)
                .with_border(Border::all(1.).with_border_color(box_border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
            )
            .with_width(16.)
            .with_height(16.)
            .finish();
            Flex::row()
                .with_spacing(S2)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(mark)
                .with_child(ui_label(
                    label.clone(),
                    font,
                    BODY,
                    label_fg,
                    Weight::Normal,
                ))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    pub fn icon_tile_btn(
        &self,
        id: impl Into<String>,
        icon: &str,
        title: &str,
        subtitle: Option<&str>,
        active: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("tile:{}", id.into()));
        let icon = icon.to_string();
        let title = title.to_string();
        let subtitle = subtitle.map(str::to_string);
        let font = self.font;
        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if active {
                if clicked {
                    lerp_color(p.accent_soft, p.hover, 0.6)
                } else {
                    p.accent_soft
                }
            } else if clicked {
                p.accent_soft
            } else {
                lerp_color(p.surface, p.hover, t)
            };
            let border = if active {
                p.accent
            } else if clicked {
                p.border_strong
            } else {
                lerp_color(p.border, p.border_strong, t)
            };
            let mut col = vec![
                ui_label(
                    icon.clone(),
                    font,
                    HEADING + 1.,
                    if active { p.accent } else { p.text },
                    Weight::Normal,
                ),
                Text::new(title.clone(), font, CAPTION)
                    .with_color(if active { p.accent } else { p.text })
                    .with_style(Properties::default().weight(Weight::Medium))
                    .soft_wrap(false)
                    .finish(),
            ];
            if let Some(sub) = &subtitle {
                col.push(
                    Text::new(sub.clone(), font, MICRO)
                        .with_color(p.text3)
                        .soft_wrap(false)
                        .finish(),
                );
            }
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(3.)
                    .with_children(col)
                    .finish(),
            )
            .with_horizontal_padding(S2)
            .with_vertical_padding(S2 + 2.)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    pub fn pill_badge(&self, text: impl Into<String>, fg: ColorU, bg: ColorU) -> El {
        Container::new(ui_label(text.into(), self.font, MICRO, fg, Weight::Medium))
            .with_horizontal_padding(8.)
            .with_vertical_padding(2.)
            .with_background_color(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
            .finish()
    }

    pub fn pill_badge_icon(
        &self,
        text: impl Into<String>,
        icon: UiIcon,
        fg: ColorU,
        bg: ColorU,
    ) -> El {
        let label = text.into();
        Container::new(icon_label(
            icon,
            &label,
            self.font,
            MICRO,
            fg,
            Weight::Medium,
        ))
        .with_horizontal_padding(8.)
        .with_vertical_padding(2.)
        .with_background_color(bg)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
        .finish()
    }

    pub fn link_action(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        action: Action,
    ) -> El {
        self.link(id, label, action)
    }

    pub fn link_icon(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        icon: UiIcon,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("link:{}", id.into()));
        let label = label.into();
        let font = self.font;
        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let fg = if mouse.is_clicked() {
                p.text2
            } else {
                lerp_color(p.accent, p.accent_hover, t)
            };
            icon_label(icon, &label, font, BODY, fg, Weight::Normal)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    pub fn ui_icon_box(
        &self,
        icon: UiIcon,
        size: f32,
        bg: ColorU,
        fg: ColorU,
        border: Option<ColorU>,
    ) -> El {
        let mut c = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(self.ui_icon(icon, 18., fg))
                .finish(),
        )
        .with_background_color(bg)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)));
        if let Some(b) = border {
            c = c.with_border(Border::all(1.).with_border_color(b));
        }
        ConstrainedBox::new(c.finish())
            .with_width(size)
            .with_height(size)
            .finish()
    }

    pub fn icon_box(
        &self,
        symbol: &str,
        size: f32,
        bg: ColorU,
        fg: ColorU,
        border: Option<ColorU>,
    ) -> El {
        let mut c = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    icons::symbol(symbol)
                        .map(|icon| self.ui_icon(icon, 18., fg))
                        .unwrap_or_else(|| self.line_w(symbol, BODY, fg, Weight::Semibold)),
                )
                .finish(),
        )
        .with_background_color(bg)
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)));
        if let Some(b) = border {
            c = c.with_border(Border::all(1.).with_border_color(b));
        }
        ConstrainedBox::new(c.finish())
            .with_width(size)
            .with_height(size)
            .finish()
    }

    /// Renders a 17×23 (391-pixel, 379 active LEDs) dot-matrix preview matching the white/silver LED hardware.
    pub fn matrix_grid(&self, frame: &[u8], dot_size: f32, gap: f32, dark_chassis: bool) -> El {
        let grid = super::led_grid::LedGrid::new(frame, dot_size, gap).finish();
        if !dark_chassis {
            return grid;
        }
        self.phone_rear_housing(182., 312., frame, dot_size, gap)
    }

    /// Orthographic device artwork. Screen/LED content is composited separately.
    fn phone_art(&self, name: &'static str, width: f32, height: f32) -> El {
        ConstrainedBox::new(
            Image::new(
                facloud_ui::assets::asset_cache::AssetSource::Bundled { path: name },
                CacheOption::BySize,
            )
            .contain()
            .finish(),
        )
        .with_width(width)
        .with_height(height)
        .finish()
    }

    pub fn phone_rear_housing(
        &self,
        width: f32,
        height: f32,
        frame: &[u8],
        _dot_size: f32,
        _gap: f32,
    ) -> El {
        let scale = (width / 866.).min(height / 1817.);
        let w = 866. * scale;
        let h = 1817. * scale;
        // The real Neo display sits to the right of the camera lenses.
        Stack::new()
            .with_child(self.phone_art("neo_rear_housing.png", w, h))
            .with_positioned_child(
                self.matrix_grid(frame, 7. * scale, 2.5 * scale, false),
                OffsetPositioning::offset_from_parent(
                    pathfinder_geometry::vector::vec2f(562. * scale, 180. * scale),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopLeft,
                    ChildAnchor::TopLeft,
                ),
            )
            .finish()
    }

    pub fn phone_front_housing(&self, width: f32, height: f32) -> El {
        let scale = (width / 866.).min(height / 1817.);
        let w = 866. * scale;
        let h = 1817. * scale;
        let source = if let Some((_, path)) = &self.screenshot {
            facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                path: path.to_string_lossy().into_owned(),
            }
        } else {
            facloud_ui::assets::asset_cache::AssetSource::Bundled {
                path: "stepx_home_screen.png",
            }
        };
        // Fit under the generated bezel, preserving screenshot aspect ratio.
        let screen = ConstrainedBox::new(
            Image::new(source, CacheOption::BySize)
                .cover()
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(77. * scale)))
                .finish(),
        )
        .with_width(738. * scale)
        .with_height(1660. * scale)
        .finish();
        ConstrainedBox::new(
            Stack::new()
                .with_child(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(w)
                        .with_height(h)
                        .finish(),
                )
                .with_positioned_child(
                    screen,
                    OffsetPositioning::offset_from_parent(
                        pathfinder_geometry::vector::vec2f(64. * scale, 80. * scale),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                )
                .with_positioned_child(
                    self.phone_art("neo_front_frame.png", w, h),
                    OffsetPositioning::offset_from_parent(
                        pathfinder_geometry::vector::vec2f(0., 0.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                )
                .finish(),
        )
        .with_width(w)
        .with_height(h)
        .finish()
    }

    /// Unified interactive phone studio unit: combines the live phone screen (with direct
    /// click-to-tap touch grid), an attached right-side hardware button rail, and an attached
    /// bottom Android navigation bar into a single cohesive physical unit.
    pub fn interactive_phone_bezel(
        &self,
        id_prefix: &str,
        width: f32,
        height: f32,
        custom_shot: Option<&std::path::Path>,
    ) -> El {
        let p = self.p();
        let font = self.font;
        let scale = (width / 866.).min(height / 1817.);
        let w = 866. * scale;
        let h = 1817. * scale;
        let sw = 738. * scale;
        let sh = 1660. * scale;
        let cell_w = sw / 5.;
        let cell_h = sh / 9.;

        let shot_path = custom_shot.or_else(|| self.screenshot.as_ref().map(|(_, p)| p.as_path()));
        let source = if let Some(path) = shot_path {
            facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                path: path.to_string_lossy().into_owned(),
            }
        } else {
            facloud_ui::assets::asset_cache::AssetSource::Bundled {
                path: "stepx_home_screen.png",
            }
        };

        let screen = ConstrainedBox::new(
            Image::new(source, CacheOption::BySize)
                .cover()
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(77. * scale)))
                .finish(),
        )
        .with_width(sw)
        .with_height(sh)
        .finish();

        // 5-col x 9-row interactive touch matrix over the phone screen area
        let mut grid_rows = Vec::with_capacity(9);
        for row in 0..9u16 {
            let mut row_cells = Vec::with_capacity(5);
            let norm_y = row * 111 + 56;
            for col in 0..5u16 {
                let norm_x = col * 200 + 100;
                let cell_id = format!("{id_prefix}-tap-{col}-{row}");
                let state = self.mouse_state(&cell_id);
                let cell = Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let clicked = mouse.is_clicked();
                    let bg = if clicked {
                        ColorU::new(p.accent.r, p.accent.g, p.accent.b, 58)
                    } else if hovered {
                        ColorU::new(p.accent.r, p.accent.g, p.accent.b, 28)
                    } else {
                        ColorU::transparent_black()
                    };
                    let mut c = Container::new(Empty::new().finish())
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)));
                    if hovered || clicked {
                        c = c.with_border(Border::all(1.).with_border_color(ColorU::new(
                            p.accent.r, p.accent.g, p.accent.b, 92,
                        )));
                    }
                    ConstrainedBox::new(c.finish())
                        .with_width(cell_w)
                        .with_height(cell_h)
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::ScreenTapNormalized(norm_x, norm_y));
                })
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Mirror);
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::OverviewCard(
                            crate::ui::context_menu::OverviewContextKind::ScreenPreview,
                        ),
                        at,
                    ));
                })
                .finish();
                #[cfg(feature = "ui-capture")]
                let cell = super::scenario_check::control(cell_id, cell);
                row_cells.push(cell);
            }
            let r = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(row_cells)
                .finish();
            grid_rows.push(r);
        }

        let touch_overlay = ConstrainedBox::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(grid_rows)
                .finish(),
        )
        .with_width(sw)
        .with_height(sh)
        .finish();

        let phone_chassis = ConstrainedBox::new(
            Stack::new()
                .with_child(
                    ConstrainedBox::new(Empty::new().finish())
                        .with_width(w)
                        .with_height(h)
                        .finish(),
                )
                .with_positioned_child(
                    screen,
                    OffsetPositioning::offset_from_parent(
                        pathfinder_geometry::vector::vec2f(64. * scale, 80. * scale),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                )
                .with_positioned_child(
                    self.phone_art("neo_front_frame.png", w, h),
                    OffsetPositioning::offset_from_parent(
                        pathfinder_geometry::vector::vec2f(0., 0.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                )
                .with_positioned_child(
                    touch_overlay,
                    OffsetPositioning::offset_from_parent(
                        pathfinder_geometry::vector::vec2f(64. * scale, 80. * scale),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopLeft,
                        ChildAnchor::TopLeft,
                    ),
                )
                .finish(),
        )
        .with_width(w)
        .with_height(h)
        .finish();

        // Attached right-side hardware control rail (docked flush against the right edge of the phone)
        let rail_specs = [
            ("wake", UiIcon::Sun, "亮屏", Action::Key(224)),
            ("lock", UiIcon::Lock, "锁屏", Action::Key(223)),
            ("vol-up", UiIcon::Plus, "音量+", Action::Key(24)),
            ("vol-down", UiIcon::Minus, "音量-", Action::Key(25)),
            ("mute", UiIcon::Mute, "静音", Action::Key(164)),
            (
                "swipe-up",
                UiIcon::ArrowUp,
                "上滑",
                Action::ScreenGesture(0),
            ),
            (
                "swipe-down",
                UiIcon::ArrowDown,
                "下滑",
                Action::ScreenGesture(1),
            ),
            ("notif", UiIcon::Bell, "通知", Action::ScreenGesture(4)),
            ("shot", UiIcon::Camera, "截屏", Action::Screenshot),
        ];

        let mut rail_buttons = Vec::with_capacity(rail_specs.len());
        for (suffix, icon, label, action) in rail_specs {
            let btn_id = format!("{id_prefix}-rail-{suffix}");
            let state = self.mouse_state(&btn_id);
            let lbl = label.to_string();
            let btn = Hoverable::new(state, move |mouse| {
                let t = self.hover_t(&mouse);
                let clicked = mouse.is_clicked();
                let bg = if clicked {
                    p.accent_soft
                } else {
                    lerp_color(p.bg, p.hover, t)
                };
                let fg = if clicked {
                    p.accent
                } else {
                    lerp_color(p.text2, p.text, t)
                };
                let border = if clicked {
                    p.accent
                } else {
                    lerp_color(p.border, p.border_strong, t)
                };
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Min)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(4.)
                        .with_child(icons::draw_icon(icon, 12., fg))
                        .with_child(ui_label(lbl.clone(), font, 10.5, fg, Weight::Medium))
                        .finish(),
                )
                .with_horizontal_padding(6.)
                .with_vertical_padding(4.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish();
            #[cfg(feature = "ui-capture")]
            let btn = super::scenario_check::control(btn_id, btn);
            rail_buttons.push(btn);
        }

        let side_rail = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(4.)
                .with_children(rail_buttons)
                .finish(),
        )
        .with_uniform_padding(4.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
        .finish();

        // Attached bottom Android navigation bar (flush underneath the phone unit)
        let nav_specs = [
            ("back", UiIcon::ArrowLeft, "◁ 返回", Action::Key(4)),
            ("home", UiIcon::Home, "● 主页", Action::Key(3)),
            ("recents", UiIcon::Apps, "▣ 多任务", Action::Key(187)),
        ];
        let mut nav_buttons = Vec::with_capacity(nav_specs.len());
        for (suffix, icon, label, action) in nav_specs {
            let btn_id = format!("{id_prefix}-nav-{suffix}");
            let state = self.mouse_state(&btn_id);
            let lbl = label.to_string();
            let btn = Hoverable::new(state, move |mouse| {
                let t = self.hover_t(&mouse);
                let clicked = mouse.is_clicked();
                let bg = if clicked {
                    p.accent_soft
                } else {
                    lerp_color(p.bg, p.hover, t)
                };
                let fg = if clicked {
                    p.accent
                } else {
                    lerp_color(p.text2, p.text, t)
                };
                let border = if clicked {
                    p.accent
                } else {
                    lerp_color(p.border, p.border_strong, t)
                };
                Container::new(
                    Align::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Min)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_spacing(4.)
                            .with_child(icons::draw_icon(icon, 12., fg))
                            .with_child(ui_label(lbl.clone(), font, 11., fg, Weight::Medium))
                            .finish(),
                    )
                    .finish(),
                )
                .with_horizontal_padding(6.)
                .with_vertical_padding(5.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
            .finish();
            #[cfg(feature = "ui-capture")]
            let btn = super::scenario_check::control(btn_id, btn);
            nav_buttons.push(Expanded::new(1., btn).finish());
        }

        let total_unit_width = w + 6. + 66.;
        let bottom_nav = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_spacing(4.)
                    .with_children(nav_buttons)
                    .finish(),
            )
            .with_uniform_padding(4.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .finish(),
        )
        .with_width(total_unit_width)
        .finish();

        let top_unit = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(phone_chassis)
            .with_child(side_rail)
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(top_unit)
            .with_child(bottom_nav)
            .finish()
    }

    // ---- Conversation helpers (chat-first Amoo) ----

    /// Small rounded chip. Every action runs in order on click, so a chip in
    /// a small menu can close the menu and perform the choice in one tap.
    /// `active` highlights the chip; `muted` renders it as a quiet hint.
    pub fn chip(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        icon: Option<UiIcon>,
        actions: Vec<Action>,
        active: bool,
        muted: bool,
    ) -> El {
        let id = id.into();
        let p = self.p();
        let label = label.into();
        let font = self.font;
        let control = Hoverable::new(self.mouse_state(format!("chip:{id}")), move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let rest_fg = if muted { p.text3 } else { p.text2 };
            let fg = if active {
                p.accent
            } else if clicked {
                p.text
            } else {
                lerp_color(rest_fg, p.text, t)
            };
            let bg = if active {
                if clicked {
                    p.press_color(p.hover)
                } else {
                    lerp_color(p.accent_soft, p.hover, t * 0.6)
                }
            } else if clicked {
                p.press_color(p.hover)
            } else {
                lerp_color(p.bg, p.hover, t)
            };
            let border = if active {
                Border::all(1.).with_border_color(lerp_color(p.accent, p.accent_hover, t))
            } else if clicked {
                Border::all(1.).with_border_color(p.border_strong)
            } else {
                Border::all(1.).with_border_color(lerp_color(p.border, p.border_strong, t))
            };
            let content = match icon {
                Some(icon) => icon_label(icon, &label, font, CAPTION, fg, Weight::Medium),
                None => ui_label(label.clone(), font, CAPTION, fg, Weight::Medium),
            };
            Container::new(
                ConstrainedBox::new(Align::new(content).finish())
                    .with_height(24.)
                    .finish(),
            )
            .with_horizontal_padding(S2 + 2.)
            .with_background_color(bg)
            .with_border(border)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            for action in &actions {
                ctx.dispatch_typed_action(action.clone());
            }
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let control = super::scenario_check::control(id, control);
        control
    }

    /// Icon-only button with a hover tooltip. Runs every action in order.
    pub fn icon_action(
        &self,
        id: impl Into<String>,
        tooltip: impl Into<String>,
        icon: UiIcon,
        actions: Vec<Action>,
        active: bool,
    ) -> El {
        let id = id.into();
        let p = self.p();
        let tooltip = tooltip.into();
        let control = Hoverable::new(self.mouse_state(format!("iconact:{id}")), move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if active {
                if clicked {
                    p.press_color(p.hover)
                } else {
                    lerp_color(p.accent_soft, p.hover, t * 0.55)
                }
            } else if clicked {
                p.press_color(p.hover)
            } else {
                lerp_color(ColorU::transparent_black(), p.hover, t)
            };
            let fg = if active {
                p.accent
            } else if clicked {
                p.text
            } else {
                lerp_color(p.text2, p.text, t)
            };
            let button = ConstrainedBox::new(
                Container::new(Align::new(self.ui_icon(icon, 16., fg)).finish())
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            )
            .with_width(30.)
            .with_height(30.)
            .finish();
            if !hovered || tooltip.is_empty() {
                return button;
            }
            let tip = Container::new(self.line(tooltip.clone(), MICRO, p.text))
                .with_horizontal_padding(S2)
                .with_vertical_padding(S1)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
            Stack::new()
                .with_child(button)
                .with_positioned_child(
                    tip,
                    OffsetPositioning::offset_from_parent(
                        pathfinder_geometry::vector::vec2f(0., S1),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                )
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            for action in &actions {
                ctx.dispatch_typed_action(action.clone());
            }
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let control = super::scenario_check::control(id, control);
        control
    }

    /// Compact inline step indicator: done (check), active (filled dot),
    /// pending (hollow dot) and failed (cross), each with a plain label.
    pub fn stepper(&self, steps: Vec<(String, StepMark)>) -> El {
        let p = self.p();
        let dot = |fill: Option<ColorU>, ring: ColorU| -> El {
            let mut c = Container::new(
                ConstrainedBox::new(Empty::new().finish())
                    .with_width(8.)
                    .with_height(8.)
                    .finish(),
            )
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .with_border(Border::all(1.).with_border_color(ring));
            if let Some(fill) = fill {
                c = c.with_background_color(fill);
            }
            c.finish()
        };
        let mut items = Vec::new();
        for (i, (label, mark)) in steps.into_iter().enumerate() {
            if i > 0 {
                items.push(
                    ConstrainedBox::new(Rect::new().with_background_color(p.border).finish())
                        .with_width(14.)
                        .with_height(1.)
                        .finish(),
                );
            }
            let (marker, fg, weight) = match mark {
                StepMark::Done => (
                    self.ui_icon(UiIcon::Check, 12., p.success),
                    p.text2,
                    Weight::Normal,
                ),
                StepMark::Active => {
                    let pulse = self.pulse_t();
                    let fill = lerp_color(p.accent_soft, p.accent, pulse);
                    (dot(Some(fill), p.accent), p.text, Weight::Semibold)
                }
                StepMark::Pending => (dot(None, p.text3), p.text3, Weight::Normal),
                StepMark::Failed => (
                    self.ui_icon(UiIcon::Close, 12., p.warning),
                    p.warning,
                    Weight::Normal,
                ),
            };
            items.push(self.hstack(
                S1 + 2.,
                vec![marker, self.line_w(label, CAPTION, fg, weight)],
            ));
        }
        self.wrap(items)
    }
}

/// State of one item in [`Companion::stepper`].
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StepMark {
    Pending,
    Active,
    Done,
    Failed,
}
