//! Window chrome: sidebar, page header, status bar and shared page states.

use super::{
    backscreen_state::LiveCapsuleFilter,
    icons::{self, UiIcon},
    theme::*,
    widgets::*,
    *,
};
use facloud_ui::{color::ColorU, fonts::Weight, platform::Cursor, ClipBounds};

#[derive(Clone, Copy, Debug)]
pub(super) enum SidebarBadge {
    None,
    RedDot,
    Count(usize, ColorU),
    SvgIcon(UiIcon, ColorU),
    SvgIconWithDot(UiIcon, ColorU),
}

impl Companion {
    #[track_caller]
    pub(super) fn scrollable(&self, state: ClippedScrollStateHandle, content: El) -> El {
        let p = self.p();
        let thumb = ColorU::new(p.text3.r, p.text3.g, p.text3.b, 90);
        let thumb_active = ColorU::new(p.text3.r, p.text3.g, p.text3.b, 160);
        self.ease_scroll(
            ClippedScrollable::vertical(
                state.clone(),
                Container::new(content)
                    .with_horizontal_padding(S5)
                    .with_padding_top(S5)
                    .with_padding_bottom(S6)
                    .finish(),
                ScrollbarWidth::Auto,
                thumb.into(),
                thumb_active.into(),
                Fill::None,
            )
            .finish(),
            state,
        )
    }

    /// Same scroll container without page padding (for panes that draw their own).
    #[track_caller]
    pub(super) fn pane_scroll(&self, state: ClippedScrollStateHandle, content: El) -> El {
        let p = self.p();
        let thumb = ColorU::new(p.text3.r, p.text3.g, p.text3.b, 90);
        let thumb_active = ColorU::new(p.text3.r, p.text3.g, p.text3.b, 160);
        self.ease_scroll(
            ClippedScrollable::vertical(
                state.clone(),
                content,
                ScrollbarWidth::Auto,
                thumb.into(),
                thumb_active.into(),
                Fill::None,
            )
            .finish(),
            state,
        )
    }

    pub(super) fn loading_page(&self) -> El {
        let p = self.p();
        Align::new(self.vstack(
            S4,
            vec![
            Align::new(self.ui_icon(UiIcon::Smartphone, 36., p.text2)).finish(),
            Align::new(self.line_w("正在载入手机数据", TITLE, p.text, Weight::Semibold)).finish(),
            Align::new(self.line(&self.status, CAPTION, p.text2)).finish(),
        ],
        ))
        .finish()
    }

    pub(super) fn no_device(&self) -> El {
        let p = self.p();
        let ready = self.devices.iter().filter(|d| d.state == "device").count();
        let count_label = if ready > 0 {
            format!("{ready} 台待选")
        } else {
            "未检测到可用设备".to_string()
        };
        let candidate_label = self
            .devices
            .first()
            .map(|d| {
                let model = if d.model.is_empty() {
                    "未知型号".to_string()
                } else {
                    d.model.replace('_', " ")
                };
                let ch = if is_wireless(&d.id) { "无线" } else { "USB" };
                format!("{model} ({ch})")
            })
            .unwrap_or_else(|| "连接手机后将在这里显示".to_string());

        let icon_bg = if p.dark {
            rgb(0x27, 0x27, 0x2A)
        } else {
            rgb(0xF4, 0xF4, 0xF6)
        };
        let phone_icon = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(self.ui_icon(UiIcon::Smartphone, 22., p.text2))
                    .finish(),
            )
            .with_background_color(icon_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
            .finish(),
        )
        .with_width(48.)
        .with_height(48.)
        .finish();

        let channel_box = Container::new(self.vstack(
            S2,
            vec![
                self.between(
                    self.line(
                        if ready > 0 {
                            "检测到未选择通道"
                        } else {
                            "等待连接"
                        },
                        CAPTION,
                        p.text2,
                    ),
                    self.mono(count_label, CAPTION, p.warning),
                ),
                self.divider(),
                self.between(
                    self.mono(candidate_label, CAPTION, p.text),
                    self.link_action("nodev-pick", "前往选择", Action::Page(Page::Connect)),
                ),
            ],
        ))
        .with_uniform_padding(14.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.surface_border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let card = ConstrainedBox::new(
            Container::new(
                Flex::column()
                    .with_spacing(S4 + 4.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(phone_icon)
                    .with_child(
                        Flex::column()
                            .with_spacing(6.)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(self.line_w(
                                "尚未连接 StepX 设备",
                                TITLE,
                                p.text,
                                Weight::Semibold,
                            ))
                            .with_child(self.text(
                                "从 USB 调试授权开始，按引导安装 StepX伴侣，再切换无线扫码连接。已有系统无线调试配对也可直接连接。",
                                CAPTION,
                                p.text2,
                            ))
                            .finish(),
                    )
                    .with_child(
                        Flex::column()
                            .with_main_axis_size(MainAxisSize::Min)
                            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .with_child(channel_box)
                            .finish(),
                    )
                    .with_child(self.hstack(
                        10.,
                        vec![
                            self.button("nodev-refresh", "刷新检测", Action::Refresh, Btn::Secondary),
                            self.button(
                                "nodev-connect",
                                "开始连接引导",
                                Action::ConnectGuide(Some(0)),
                                Btn::Primary,
                            ),
                        ],
                    ))
                    .finish(),
            )
            .with_uniform_padding(28.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
            .finish(),
        )
        .with_width(440.)
        .finish();

        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(card)
                .finish(),
        )
        .with_uniform_padding(S5)
        .finish()
    }

    fn nav_group_title(&self, title: &'static str, first: bool) -> El {
        let p = self.p();
        Container::new(
            facloud_ui::elements::Text::new(title, self.font, 10.)
                .with_color(p.text3)
                .with_style(facloud_ui::fonts::Properties::default().weight(Weight::Semibold))
                .soft_wrap(false)
                .finish(),
        )
        .with_horizontal_padding(10.)
        .with_padding_top(if first { 0. } else { 8. })
        .with_padding_bottom(3.)
        .finish()
    }

    pub(super) fn nav_item_badge(
        &self,
        id: &str,
        icon: &'static str,
        label: &'static str,
        badge: SidebarBadge,
        active: bool,
        action: Action,
    ) -> El {
        let resolved_icon = if icon == "✦" {
            UiIcon::Sparkles
        } else {
            icons::symbol(icon).unwrap_or(UiIcon::Apps)
        };
        self.nav_item_icon(id, resolved_icon, label, badge, active, action)
    }

    pub(super) fn nav_item_with_match(
        &self,
        id: &str,
        icon: UiIcon,
        label: &'static str,
        target: Page,
        is_active: bool,
        badge: SidebarBadge,
    ) -> El {
        self.nav_item_icon(id, icon, label, badge, is_active, Action::Page(target))
    }

    pub(super) fn nav_item_icon(
        &self,
        id: &str,
        icon: UiIcon,
        label: &'static str,
        badge: SidebarBadge,
        active: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("nav:{id}"));
        let font = self.font;
        let el = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if active {
                if clicked {
                    lerp_color(p.selected, p.hover, 0.5)
                } else {
                    p.selected
                }
            } else if clicked {
                p.press_color(p.hover)
            } else {
                lerp_color(ColorU::transparent_black(), p.hover, t)
            };
            let border = if active && !p.dark {
                Border::all(1.).with_border_color(rgb(0xE4, 0xE4, 0xE7))
            } else if active {
                Border::all(1.).with_border_color(lerp_color(p.border, p.accent, 0.42))
            } else {
                Border::all(1.).with_border_color(ColorU::transparent_black())
            };
            let icon_fg = if active || clicked {
                p.text
            } else {
                lerp_color(p.text2, p.text, t * 0.65)
            };
            let resting_text = if p.dark {
                p.text2
            } else {
                rgb(0x52, 0x52, 0x5B)
            };
            let text_fg = if active || clicked {
                p.text
            } else {
                lerp_color(resting_text, p.text, t)
            };
            let left = Flex::row()
                .with_spacing(10.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    ConstrainedBox::new(Align::new(icons::draw_icon(icon, 16., icon_fg)).finish())
                        .with_width(20.)
                        .with_height(20.)
                        .finish(),
                )
                .with_child(
                    facloud_ui::elements::Text::new(label, font, CAPTION)
                        .with_color(text_fg)
                        .with_style(facloud_ui::fonts::Properties::default().weight(if active {
                            Weight::Semibold
                        } else {
                            Weight::Medium
                        }))
                        .soft_wrap(false)
                        .finish(),
                )
                .finish();
            let row = match badge {
                SidebarBadge::None => left,
                SidebarBadge::RedDot => {
                    let dot = ConstrainedBox::new(
                        Container::new(
                            Align::new(
                                ConstrainedBox::new(
                                    Container::new(Empty::new().finish())
                                        .with_background_color(p.danger)
                                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                            4.,
                                        )))
                                        .finish(),
                                )
                                .with_width(8.)
                                .with_height(8.)
                                .finish(),
                            )
                            .finish(),
                        )
                        .with_background_color(p.danger_soft)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(7.)))
                        .finish(),
                    )
                    .with_width(14.)
                    .with_height(14.)
                    .finish();
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(left)
                        .with_child(dot)
                        .finish()
                }
                SidebarBadge::SvgIcon(b_icon, b_color) => {
                    let badge_node = ConstrainedBox::new(
                        Align::new(icons::draw_icon(b_icon, 13., b_color)).finish(),
                    )
                    .with_width(20.)
                    .with_height(20.)
                    .finish();
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(left)
                        .with_child(badge_node)
                        .finish()
                }
                SidebarBadge::Count(count, b_color) => {
                    let badge_node = Container::new(
                        facloud_ui::elements::Text::new(count.to_string(), font, 10.)
                            .with_color(b_color)
                            .with_style(
                                facloud_ui::fonts::Properties::default().weight(Weight::Semibold),
                            )
                            .soft_wrap(false)
                            .finish(),
                    )
                    .with_horizontal_padding(5.)
                    .with_vertical_padding(1.)
                    .with_background_color(ColorU::new(b_color.r, b_color.g, b_color.b, 32))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                    .finish();
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(left)
                        .with_child(badge_node)
                        .finish()
                }
                SidebarBadge::SvgIconWithDot(b_icon, b_color) => {
                    let badge_node = Flex::row()
                        .with_spacing(4.)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(icons::draw_icon(b_icon, 13., b_color))
                        .with_child(
                            ConstrainedBox::new(
                                Container::new(Empty::new().finish())
                                    .with_background_color(p.danger)
                                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.5)))
                                    .finish(),
                            )
                            .with_width(7.)
                            .with_height(7.)
                            .finish(),
                        )
                        .finish();
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(left)
                        .with_child(badge_node)
                        .finish()
                }
            };
            Container::new(row)
                .with_horizontal_padding(10.)
                .with_vertical_padding(6.)
                .with_background_color(bg)
                .with_border(border)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        #[cfg(feature = "ui-capture")]
        let el = crate::ui::scenario_check::control(format!("nav-{id}"), el);
        el
    }

    pub(super) fn cached_device_metrics(&self) -> stepx_companion::device::DeviceMetrics {
        thread_local! {
            static CACHE: std::cell::RefCell<Option<(String, stepx_companion::device::DeviceMetrics)>> =
                const { std::cell::RefCell::new(None) };
        }
        let mut metrics = CACHE.with(|cache| {
            let mut borrow = cache.borrow_mut();
            if let Some((cached_summary, metrics)) = borrow.as_ref() {
                if cached_summary == &self.summary {
                    return metrics.clone();
                }
            }
            let metrics = stepx_companion::device::DeviceMetrics::from_summary(&self.summary);
            *borrow = Some((self.summary.clone(), metrics.clone()));
            metrics
        });
        if let Some(battery) = self.battery_reading() {
            metrics.battery_level = battery.level;
            metrics.charging_state = battery.label().into();
        }
        metrics
    }

    pub(crate) fn device_block(&self) -> El {
        let p = self.p();
        let state = self.mouse_state("device-block");
        let card_bg = if p.dark {
            rgb(0x1C, 0x1C, 0x21)
        } else {
            ColorU::white()
        };
        let card_border = if self.page == Page::Connect {
            p.accent
        } else if p.dark {
            rgb(0x2E, 0x2E, 0x36)
        } else {
            rgb(0xE4, 0xE4, 0xE7)
        };
        let icon_bg = if p.dark {
            rgb(0x27, 0x27, 0x2A)
        } else {
            rgb(0xF4, 0xF4, 0xF5)
        };

        let top_row = match &self.selected {
            Some(d) => {
                let metrics = self.cached_device_metrics();
                let detail = metrics
                    .battery_level
                    .map(|b| format!("{b}%"))
                    .unwrap_or_else(|| "—".into());
                let battery = self.battery_reading();
                let battery_icon = if battery.is_some_and(|b| b.charging()) {
                    UiIcon::BatteryCharging
                } else if battery.is_some_and(|b| b.plugged > 0 && !b.full()) {
                    UiIcon::BatteryPlugged
                } else {
                    match metrics.battery_level {
                        Some(0..=5) => UiIcon::BatteryEmpty,
                        Some(6..=25) => UiIcon::BatteryLow,
                        Some(26..=50) => UiIcon::BatteryHalf,
                        Some(51..=80) => UiIcon::Battery,
                        Some(81..=100) => UiIcon::BatteryFull,
                        _ => UiIcon::BatteryEmpty,
                    }
                };
                let battery_color = if battery.is_some_and(|b| b.charging() || b.full()) { p.success }
                    else if metrics.battery_level.is_some_and(|v| v <= 20) { p.warning }
                    else { p.text2 };
                let title = if d.model.contains("StepX") {
                    d.model.clone()
                } else {
                    format!("StepX {}", d.model)
                };
                let icon_box = ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(self.phone_front_icon(19., 36.))
                            .finish(),
                    )
                    .with_background_color(icon_bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                )
                .with_width(28.)
                .with_height(38.)
                .finish();

                let text_col = Expanded::new(
                    1.,
                    Flex::column()
                        .with_spacing(2.)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_child(self.lead(
                            6.,
                            self.dot(p.success),
                            self.line_w(title, CAPTION, p.text, Weight::Semibold),
                        ))
                        .with_child(self.hstack(
                            4.,
                            vec![
                                self.device_connection_icons("sidebar-connection", d),
                                self.status_icon(
                                    "sidebar-battery",
                                    &self.battery_hint(),
                                    battery_icon,
                                    battery_color,
                                ),
                                ConstrainedBox::new(self.line(detail, 10.5, battery_color)).with_width(32.).finish(),
                            ],
                        ))
                        .finish(),
                )
                .finish();

                self.hstack(10., vec![icon_box, text_col])
            }
            None => {
                let reconnecting = self.connection_watch.detached_serial.is_some()
                    && self
                        .connection_watch
                        .disconnected_at
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(30));
                let icon_box = ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(self.phone_front_icon(19., 36.))
                            .finish(),
                    )
                    .with_background_color(icon_bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                )
                .with_width(28.)
                .with_height(38.)
                .finish();

                let text_col = Flex::column()
                    .with_spacing(2.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(if reconnecting {
                        self.lead(
                            6.,
                            self.dot(p.warning),
                            self.line_w("正在自动重连…", CAPTION, p.text, Weight::Semibold),
                        )
                    } else {
                        self.line_w("未连接设备", CAPTION, p.text, Weight::Semibold)
                    })
                    .with_child(self.line(
                        if reconnecting {
                            "无线/USB 通道恢复中"
                        } else {
                            "点击前往配对"
                        },
                        CAPTION,
                        p.text2,
                    ))
                    .finish();

                self.between(
                    self.hstack(10., vec![icon_box, text_col]),
                    self.ui_icon(UiIcon::ChevronRight, 14., p.text3),
                )
            }
        };

        let connected = self.selected.is_some();
        let data_stalled = self.selected.as_ref().is_some_and(stepx_companion::push::message_data_stalled);
        let sync_pct = self.auto_sync.overall_percent();
        let details_open = self.page == Page::Connect;
        let (sync_line, sync_badge): (String, &'static str) = {
            let a = &self.auto_sync;
            let done = a.completed_count();
            let total = a.participating_count();
            if !connected || a.device_serial.is_none() {
                ("未连接 · 点击配对".into(), "配对")
            } else if !a.enabled {
                ("实时同步已暂停".into(), "已暂停")
            } else if data_stalled {
                ("数据连接恢复中 · 旧资料可用".into(), "详情")
            } else if total == 0 {
                ("资料采集已暂停".into(), "详情")
            } else if done >= total && a.partial_count() > 0 {
                (format!("已更新 · {}", a.coverage_summary()), "详情")
            } else if done >= total {
                ("已更新 · 自动守护中".into(), "详情")
            } else if a.is_syncing() {
                (format!("正在同步 · {done}/{total} ({sync_pct}%)"), "详情")
            } else {
                (format!("{done}/{total}项已更新 · 待重试"), "详情")
            }
        };
        let sync_fill_color = if !connected {
            p.border
        } else if data_stalled || !self.auto_sync.enabled {
            p.warning
        } else if self.auto_sync.partial_count() > 0 {
            p.warning
        } else if sync_pct >= 100 {
            p.success_text
        } else {
            p.accent
        };
        let track_w = 168.0_f32;
        let fill_w = if !connected {
            0.0
        } else {
            (track_w * (sync_pct as f32 / 100.0))
                .round()
                .max(if self.auto_sync.is_syncing() {
                    14.0
                } else {
                    8.0
                })
                .min(track_w)
        };
        let progress_track = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(
                        ConstrainedBox::new(
                            Container::new(Empty::new().finish())
                                .with_background_color(sync_fill_color)
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                                .finish(),
                        )
                        .with_width(fill_w)
                        .with_height(3.5)
                        .finish(),
                    )
                    .finish(),
            )
            .with_background_color(if p.dark {
                rgba(255, 255, 255, 22)
            } else {
                rgba(15, 23, 42, 18)
            })
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
            .finish(),
        )
        .with_width(track_w)
        .with_height(3.5)
        .finish();

        let sync_text_col = if connected && (data_stalled || self.auto_sync.partial_count() > 0) {
            p.warning
        } else if connected && sync_pct >= 100 {
            p.success_text
        } else if connected && !self.auto_sync.enabled {
            p.warning
        } else if connected {
            p.accent
        } else {
            p.text3
        };
        let sync_icon = if !connected {
            UiIcon::Connect
        } else if sync_pct >= 100 && self.auto_sync.enabled {
            UiIcon::Check
        } else {
            UiIcon::Refresh
        };
        let sync_state = self.mouse_state("top-bar-auto-sync-pill");
        let sync_footer = Hoverable::new(sync_state, move |mouse| {
            let t = self.hover_t(&mouse);
            let pill_bg = if details_open {
                p.accent_soft
            } else {
                lerp_color(ColorU::transparent_black(), p.hover, t * 0.85)
            };
            Container::new(
                Clipped::new(
                    Flex::column()
                        .with_spacing(4.)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(progress_track)
                        .with_child(self.between(
                            self.lead(
                                4.,
                                self.ui_icon(sync_icon, 10.5, sync_text_col),
                                self.line_w(sync_line.clone(), 10., sync_text_col, Weight::Medium),
                            ),
                            self.status_icon(
                                "sidebar-sync-details-hint",
                                sync_badge,
                                if connected {
                                    UiIcon::Info
                                } else {
                                    UiIcon::Connect
                                },
                                if details_open { p.accent } else { p.text3 },
                            ),
                        ))
                        .finish(),
                )
                .finish(),
            )
            .with_horizontal_padding(4.)
            .with_vertical_padding(3.)
            .with_background_color(pill_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            if connected {
                ctx.dispatch_typed_action(Action::AutoSync(
                    crate::ui::auto_sync::AutoSyncAction::ToggleDetails,
                ));
            } else {
                ctx.dispatch_typed_action(Action::Page(Page::Connect));
            }
        })
        .finish();
        let sync_footer = SavePosition::new(sync_footer, "top-bar-auto-sync-pill")
            .for_single_frame()
            .finish();
        #[cfg(feature = "ui-capture")]
        let sync_footer =
            crate::ui::scenario_check::control("top-bar-auto-sync-pill".to_string(), sync_footer);

        let top_row_clickable = Hoverable::new(self.mouse_state("sidebar-device-top"), move |_| {
            Container::new(top_row).finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Page(Page::Connect)))
        .finish();

        let content = Flex::column()
            .with_spacing(6.)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(top_row_clickable)
            .with_child(sync_footer)
            .finish();

        let card = ConstrainedBox::new(
            Hoverable::new(state, move |mouse| {
                let t = self.hover_t(&mouse);
                let bg = if mouse.is_clicked() {
                    p.press_color(p.hover)
                } else {
                    lerp_color(card_bg, p.hover, t * 0.5)
                };
                let border_col = if self.page == Page::Connect || details_open {
                    p.accent
                } else {
                    lerp_color(card_border, p.border_strong, t)
                };
                Container::new(content)
                    .with_uniform_padding(10.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border_col))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish()
            })
            .finish(),
        )
        .with_width(SIDEBAR_W - 25.)
        .finish();
        #[cfg(feature = "ui-capture")]
        let card = crate::ui::scenario_check::control("sidebar-device-card".to_string(), card);
        card
    }

    pub(super) fn sidebar(&self) -> El {
        let p = self.p();
        let im_unread = self.active_im_unread_count();
        let sms_unread = self.fresh_unread_sms_thread_count();
        let thread_ids: std::collections::HashSet<&str> = self
            .push
            .snapshot
            .threads
            .iter()
            .map(|t| t.thread_id.as_str())
            .collect();
        let push_unread = if self.push.loaded {
            self.push
                .snapshot
                .notifications
                .iter()
                .filter(|n| {
                    n.active
                        && !stepx_companion::push::should_hide_notification(n)
                        && !n.dismissed
                        && !n.read
                        && !n.is_ongoing
                        && !n.title.trim_start().starts_with('{')
                        && stepx_companion::push::ImChannel::from_package(&n.package_name)
                            != stepx_companion::push::ImChannel::Sms
                        && !Self::is_excluded_im_package(&n.package_name)
                        && !stepx_companion::push::is_timer_or_countdown_notification(
                            &n.package_name,
                            &n.app_name,
                            &n.title,
                            &n.text,
                        )
                        && (n.thread_id.trim().is_empty()
                            || !thread_ids.contains(n.thread_id.trim()))
                        && !self.notification_center.read_ids.contains(&n.key)
                        && !self
                            .notification_center
                            .read_ids
                            .contains(&format!("push-notif:{}", n.key))
                        && !self.notification_center.dismissed_ids.contains(&n.key)
                        && !self
                            .notification_center
                            .dismissed_ids
                            .contains(&format!("push-notif:{}", n.key))
                })
                .count()
        } else {
            0
        };
        let total_unread = im_unread + sms_unread + push_unread;
        let has_unread_msg = total_unread > 0
            || self.push.snapshot.threads.iter().any(|t| {
                t.channel() != stepx_companion::push::ImChannel::Sms
                    && !Self::is_excluded_im_package(&t.package_name)
                    && !t.title.trim_start().starts_with('{')
                    && !stepx_companion::push::is_timer_or_countdown_notification(
                        &t.package_name,
                        &t.app_name,
                        &t.title,
                        &t.last_message,
                    )
                    && t.unread_count > 0
            });
        let msg_badge = if has_unread_msg {
            SidebarBadge::RedDot
        } else {
            SidebarBadge::None
        };
        let updatable_apps = self.store_updatable_count();
        let store_badge = if self.app_store.active_task_count() > 0 {
            SidebarBadge::SvgIcon(UiIcon::Download, p.accent)
        } else if updatable_apps > 0 {
            SidebarBadge::Count(updatable_apps, p.indigo)
        } else {
            SidebarBadge::None
        };
        let amoo_badge = if self.workspace_busy() {
            SidebarBadge::SvgIconWithDot(UiIcon::Activity, p.accent)
        } else {
            SidebarBadge::None
        };
        let now_ms = chrono::Local::now().timestamp_millis();
        let today_valid_cal = self
            .back_screen
            .stepos_suite
            .calendar_events
            .iter()
            .filter(|e| {
                e.is_today_valid_unexpired_at(
                    now_ms,
                    &self.back_screen.completed_calendar_events,
                    &self.reminder_governance,
                )
            })
            .count();
        let cal_badge = if today_valid_cal > 0 {
            SidebarBadge::Count(today_valid_cal, p.indigo)
        } else {
            SidebarBadge::None
        };
        let today_unexpired_alarms = self
            .back_screen
            .stepos_suite
            .alarms
            .iter()
            .filter(|a| a.is_today_valid_unexpired_at(now_ms, &self.reminder_governance))
            .count();
        let alarm_badge = if today_unexpired_alarms > 0 {
            SidebarBadge::Count(today_unexpired_alarms, p.warning)
        } else {
            SidebarBadge::None
        };
        let missed_calls: usize = self
            .call_logs
            .iter()
            .filter(|r| {
                if r.call_type != 3
                    || r.date_ms <= 0
                    || (now_ms > 1_700_000_000_000 && now_ms.saturating_sub(r.date_ms) > 86_400_000)
                    || self.is_missed_call_followed_up(r)
                {
                    return false;
                }
                let id = format!("missed-call:{}:{}", r.number.trim(), r.date_ms);
                !self.notification_center.read_ids.contains(&id)
                    && !self.notification_center.dismissed_ids.contains(&id)
            })
            .count();
        let phone_badge = if missed_calls > 0 {
            SidebarBadge::Count(missed_calls, p.danger)
        } else {
            SidebarBadge::None
        };

        let logo_box = ConstrainedBox::new(
            Image::new(
                facloud_ui::assets::asset_cache::AssetSource::Bundled {
                    path: "stepx_companion_icon.png",
                },
                CacheOption::BySize,
            )
            .contain()
            .finish(),
        )
        .with_width(28.)
        .with_height(28.)
        .finish();

        let brand_row = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Start)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(10.)
                    .with_child(logo_box)
                    .with_child(self.line_w("StepX Companion", 15.5, p.text, Weight::Semibold))
                    .finish(),
            )
            .with_horizontal_padding(6.)
            .finish(),
        )
        .with_height(30.)
        .finish();

        let top_group = Flex::column()
            .with_spacing(14.)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(brand_row)
            .with_child(self.device_block())
            .with_child(
                Flex::column()
                    .with_spacing(2.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_children(vec![
                        self.nav_group_title("工作", true),
                        self.nav_item_with_match(
                            "home",
                            UiIcon::Home,
                            "首页",
                            Page::Home,
                            self.page == Page::Home,
                            SidebarBadge::None,
                        ),
                        self.nav_item_with_match(
                            "overview",
                            UiIcon::Smartphone,
                            "终端",
                            Page::Overview,
                            matches!(self.page, Page::Overview | Page::Account),
                            SidebarBadge::None,
                        ),
                        // 唯一的 AI 入口：聊天、生成文件、做应用都在这里，由系统自动判断。
                        // Page::Work 不再单独出现在侧边栏（只从「我的作品」/成果卡进入），
                        // 打开时仍高亮 Amoo。「我的作品」是作品与文件库。
                        self.nav_item_with_match(
                            "amoo",
                            UiIcon::Chat,
                            "Amoo",
                            Page::Amoo,
                            matches!(self.page, Page::Amoo | Page::Work),
                            amoo_badge,
                        ),
                        self.nav_item_with_match(
                            "studio",
                            UiIcon::Package,
                            "我的作品",
                            Page::Studio,
                            self.page == Page::Studio,
                            SidebarBadge::None,
                        ),
                        self.nav_group_title("通信", false),
                        self.nav_item_with_match(
                            "calendar",
                            UiIcon::Calendar,
                            "日历",
                            Page::Calendar,
                            matches!(self.page, Page::Calendar | Page::Schedule),
                            cal_badge,
                        ),
                        self.nav_item_with_match(
                            "alarms",
                            UiIcon::Clock,
                            "闹钟",
                            Page::Alarms,
                            self.page == Page::Alarms,
                            alarm_badge,
                        ),
                        self.nav_item_with_match(
                            "sms",
                            UiIcon::Sms,
                            "消息",
                            Page::Sms,
                            matches!(self.page, Page::Sms | Page::Notifications),
                            msg_badge,
                        ),
                        self.nav_item_with_match(
                            "phone",
                            UiIcon::Phone,
                            "电话",
                            Page::Phone,
                            self.page == Page::Phone,
                            phone_badge,
                        ),
                        self.nav_group_title("设备", false),
                        self.nav_item_with_match(
                            "gallery",
                            UiIcon::Image,
                            "图库",
                            Page::Gallery,
                            self.page == Page::Gallery,
                            SidebarBadge::None,
                        ),
                        self.nav_item_with_match(
                            "backscreen",
                            UiIcon::Matrix,
                            "背屏",
                            Page::BackScreen,
                            self.page == Page::BackScreen,
                            SidebarBadge::None,
                        ),
                        self.nav_item_with_match(
                            "apps",
                            UiIcon::Apps,
                            "应用",
                            Page::Apps,
                            self.page == Page::Apps,
                            SidebarBadge::None,
                        ),
                        self.nav_item_with_match(
                            "files",
                            UiIcon::Folder,
                            "文件与扩容",
                            Page::Files,
                            matches!(self.page, Page::Files | Page::Expansion),
                            SidebarBadge::None,
                        ),
                        self.nav_item_with_match(
                            "tools",
                            UiIcon::Tools,
                            "小工具",
                            Page::Tools,
                            self.page == Page::Tools,
                            SidebarBadge::None,
                        ),
                        self.nav_group_title("服务", false),
                        self.nav_item_with_match(
                            "app-store",
                            UiIcon::Download,
                            "应用商店",
                            Page::AppStore,
                            self.page == Page::AppStore,
                            store_badge,
                        ),
                    ])
                    .finish(),
            )
            .finish();

        self.fixed_w(
            SIDEBAR_W,
            Clipped::new(
                Container::new(
                    Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(top_group)
                        .with_child(self.nav_item_with_match(
                            "sidebar-settings",
                            UiIcon::Settings,
                            "设置",
                            Page::Settings,
                            matches!(self.page, Page::Settings | Page::About),
                            SidebarBadge::None,
                        ))
                        .finish(),
                )
                .with_uniform_padding(12.)
                .with_background_color(p.sidebar)
                .with_border(Border::right(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_top_left(Radius::Pixels(12.)))
                .finish(),
            )
            .finish(),
        )
    }

    pub(super) fn window_control_btn(
        &self,
        id: &str,
        symbol: &'static str,
        danger: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("win:{id}"));
        let el = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let target_bg = if danger { p.danger } else { p.hover };
            let bg = if clicked {
                if danger {
                    lerp_color(p.danger, ColorU::black(), 0.18)
                } else {
                    p.press_color(p.hover)
                }
            } else {
                lerp_color(ColorU::transparent_black(), target_bg, t)
            };
            let target_fg = if danger { ColorU::white() } else { p.text };
            let fg = if clicked {
                target_fg
            } else {
                lerp_color(p.text2, target_fg, t)
            };
            ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(icons::draw_icon(
                            match symbol {
                                "—" => UiIcon::Minus,
                                "□" => UiIcon::Maximize,
                                _ => UiIcon::Close,
                            },
                            14.,
                            fg,
                        ))
                        .finish(),
                )
                .with_background_color(bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .finish(),
            )
            .with_width(28.)
            .with_height(28.)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        #[cfg(feature = "ui-capture")]
        let el = crate::ui::scenario_check::control(format!("win-{id}"), el);
        el
    }

    /// Short (≤10 chars) top-bar subtitle; empty for most pages. Counts already
    /// shown inside each page are intentionally not repeated here.
    fn page_subtitle(&self) -> String {
        if self.selected.is_none() && self.page.needs_device() {
            return String::new();
        }
        match self.page {
            Page::Home => {
                if self.push.error.is_some() || self.back_screen.stepos_suite_error.is_some() {
                    "同步失败".into()
                } else {
                    String::new()
                }
            }
            Page::Calendar | Page::Schedule | Page::Alarms => {
                if self.back_screen.stepos_suite_error.is_some() {
                    "同步失败".into()
                } else {
                    String::new()
                }
            }
            Page::Sms => {
                if self.sms_error.is_some() && self.push.snapshot.threads.is_empty() {
                    "读取失败".into()
                } else if !self.sms_loaded
                    && !self.push.loaded
                    && self.sms.is_empty()
                    && self.push.snapshot.threads.is_empty()
                {
                    "同步中…".into()
                } else {
                    String::new()
                }
            }
            Page::AppStore => {
                if self.app_store.search_results.is_some()
                    || (self.app_store.searching && !self.app_store.last_search_query.is_empty())
                {
                    let q: String = self.app_store.last_search_query.chars().take(6).collect();
                    format!("搜索「{q}」")
                } else {
                    String::new()
                }
            }
            _ => String::new(),
        }
    }

    pub(super) fn header(&self) -> El {
        let p = self.p();
        let actions = SavePosition::new(
            self.hstack(S2, self.header_actions(true)),
            "page-header-actions",
        )
        .finish();
        let more = SavePosition::new(
            self.icon_action(
                "header-more",
                "更多操作",
                UiIcon::MoreHorizontal,
                vec![Action::HeaderActions(!self.header_actions_open)],
                self.header_actions_open,
            ),
            "header-more-anchor",
        )
        .for_single_frame()
        .finish();
        let compact = SavePosition::new(
            self.hstack(S2, vec![self.render_top_bar_notification_bell(), more]),
            "page-header-actions",
        )
        .finish();
        let controls = SavePosition::new(
            self.hstack(
                S2,
                vec![
                    self.window_control_btn("min", "—", false, Action::MinimizeWindow),
                    self.window_control_btn("max", "□", false, Action::MaximizeWindow),
                    self.window_control_btn("close", "×", true, Action::CloseWindow),
                ],
            ),
            "page-header-window-controls",
        )
        .finish();
        let row = super::header_bar::HeaderBar::new(
            SavePosition::new(self.render_top_bar_live_capsule(), "page-header-title").finish(),
            self.search_bar(),
            actions,
            compact,
            controls,
            self.search.input_width.clone(),
        );
        Container::new(Box::new(row))
            .with_horizontal_padding(S4)
            .with_vertical_padding(S3)
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish()
    }

    fn header_actions(&self, include_bell: bool) -> Vec<El> {
        let mut actions = if self.selected.is_none()
            && self.page.needs_device()
            && !matches!(self.page, Page::Home | Page::Overview)
        {
            vec![
                self.button("hdr-rescan", "重新扫描", Action::Refresh, Btn::Secondary),
                self.button_icon(
                    "hdr-connect",
                    "连接设备",
                    UiIcon::Connect,
                    Action::Page(Page::Connect),
                    Btn::Primary,
                ),
                self.render_top_bar_notification_bell(),
            ]
        } else {
            self.page_actions()
        };
        if !include_bell {
            actions.pop();
        }
        if self.mirror.is_some() {
            actions.push(self.button_icon(
                "hdr-mirror-audio",
                format!("声音: {}", self.mirror_options.audio_route.short_label()),
                UiIcon::Speaker,
                Action::CycleMirrorAudioRoute,
                Btn::Secondary,
            ));
            if self.page != Page::Overview {
                actions.push(self.button(
                    "hdr-mirror-stop",
                    "结束投屏",
                    Action::StopMirror,
                    Btn::Danger,
                ));
            }
        }
        actions.push(self.button_icon(
            "header-compact",
            "简洁",
            UiIcon::Restore,
            Action::Push(PushAction::SetShellMode(
                if matches!(self.page, Page::Sms | Page::Notifications) {
                    ShellMode::MiniNotify
                } else {
                    ShellMode::MiniAi
                },
            )),
            Btn::Ghost,
        ));
        actions
    }

    pub(super) fn with_header_actions_overlay(&self, base: El) -> El {
        if !self.header_actions_open {
            return base;
        }
        let p = self.p();
        let card = Container::new(self.wrap(self.header_actions(false)))
            .with_uniform_padding(S3)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();
        let card = ConstrainedBox::new(card).with_max_width(400.).finish();
        let backdrop = Hoverable::new(self.mouse_state("header-actions-backdrop"), |_| {
            Align::new(Empty::new().finish()).finish()
        })
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::HeaderActions(false)))
        .finish();
        Stack::new()
            .with_event_dispatch_mode(EventDispatchMode::Waterfall)
            .with_child(base)
            .with_child(backdrop)
            .with_positioned_child(
                card,
                OffsetPositioning::offset_from_save_position_element(
                    "page-header-actions",
                    vec2f(0., 12.),
                    PositionedElementOffsetBounds::ParentByPosition,
                    PositionedElementAnchor::BottomRight,
                    ChildAnchor::TopRight,
                ),
            )
            .finish()
    }

    pub(super) fn status_bar(&self) -> El {
        let p = self.p();
        let transport = match &self.selected {
            Some(device) => self.device_connection_icons("footer-connection", device),
            None => self.status_icon(
                "footer-connection",
                "未连接手机",
                UiIcon::Smartphone,
                p.text3,
            ),
        };
        let raw_detail = self.footer_detail();
        let detail = if raw_detail.chars().count() > 42 {
            let first = raw_detail
                .split(['；', '。'])
                .next()
                .unwrap_or(&raw_detail)
                .trim();
            if first.chars().count() > 48 {
                Self::truncate_chars(first, 46)
            } else {
                first.to_string()
            }
        } else {
            raw_detail
        };
        let left = self.lead(
            S2,
            self.fixed_w(
                18.,
                Align::new(if self.footer_loading() {
                    self.loading_line()
                } else {
                    self.dot(if self.initializing || self.connection_watch.misses > 0 {
                        p.warning
                    } else if self.selected.is_some() {
                        p.success
                    } else {
                        p.text3
                    })
                })
                .finish(),
            ),
            self.hstack(
                S1,
                vec![
                    transport,
                    self.line(detail, CAPTION, if self.error { p.danger } else { p.text2 }),
                    if self.error {
                        self.button_icon(
                            "status-failure-details",
                            "原因与重试",
                            UiIcon::Info,
                            Action::ReviewHistory,
                            Btn::Ghost,
                        )
                    } else {
                        Empty::new().finish()
                    },
                ],
            ),
        );
        let version_label = format!(
            "v{} · {}",
            stepx_companion::build_info::VERSION,
            stepx_companion::build_info::compiled_id()
        );
        let version_chip = Hoverable::new(
            self.mouse_state("status-bar-version-whats-new"),
            move |mouse| {
                let fg = if mouse.is_mouse_over_element() {
                    p.text
                } else {
                    p.text3
                };
                Container::new(self.mono(version_label.clone(), CAPTION, fg))
                    .with_horizontal_padding(6.)
                    .with_background_color(if mouse.is_mouse_over_element() {
                        p.hover
                    } else {
                        ColorU::transparent_black()
                    })
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish()
            },
        )
        .with_cursor(platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(Action::DesktopUpdate(DesktopUpdateAction::OpenWhatsNew))
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let version_chip =
            crate::ui::scenario_check::control("status-bar-version-whats-new".into(), version_chip);
        let right = self.hstack(
            S3,
            vec![self.desktop_update_link(), self.stars_link(), version_chip],
        );
        ConstrainedBox::new(
            Container::new(self.between(left, right))
                .with_horizontal_padding(S4)
                .with_vertical_padding(5.)
                .with_background_color(p.sidebar)
                .with_border(Border::top(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(12.)))
                .finish(),
        )
        .with_height(28.)
        .finish()
    }

    /// Gathers active Phone Live Capsules + Strategy/AI Important Message Capsules.
    pub(crate) fn collect_top_bar_live_capsules(&self) -> Vec<TopBarLiveCapsuleItem> {
        use stepx_companion::home_reminder_governance::{
            canonical_keys_for_calendar_event, canonical_keys_for_code_or_parcel,
            canonical_keys_for_im_thread, ReminderCategory,
        };
        let mut items: Vec<TopBarLiveCapsuleItem> = Vec::new();
        let now_ms = chrono::Local::now().timestamp_millis();
        let is_dismissed = |id: &str| self.back_screen.dismissed_capsule_ids.contains(id);

        // 1. Phone Live Countdown Timers
        let timers =
            stepx_companion::push::extract_active_countdown_timers(&self.push.snapshot, now_ms);
        for (idx, timer) in timers.into_iter().enumerate() {
            let cap_id = format!("phone-timer-{idx}");
            let mut timer_keys = vec![format!("capsule:{cap_id}")];
            if !timer.key.is_empty() {
                timer_keys.push(format!("timer:{}", timer.key));
                timer_keys.push(format!("notif:{}", timer.key));
            }
            if is_dismissed(&cap_id)
                || self.reminder_governance.is_any_key_suppressed(
                    &timer_keys,
                    None,
                    None,
                    None,
                    now_ms,
                )
            {
                continue;
            }
            let clean_title = timer
                .title
                .trim_start_matches('⏱')
                .trim_start_matches('🍅')
                .trim();
            let display_title = if clean_title.is_empty() {
                if timer.is_pomodoro {
                    "番茄计时".to_string()
                } else {
                    "倒计时".to_string()
                }
            } else {
                clean_title.to_string()
            };
            let short_sig: String = format!(
                "{} {}",
                if clean_title.is_empty() {
                    "计时"
                } else {
                    clean_title
                },
                timer.mm_ss_label
            )
            .chars()
            .take(6)
            .collect();
            let primary_action = if !timer.key.is_empty() {
                Action::Push(PushAction::DismissNotification(timer.key.clone()))
            } else {
                Action::Page(Page::Alarms)
            };
            items.push(TopBarLiveCapsuleItem {
                id: cap_id,
                tag: "stepx_timer".into(),
                icon: UiIcon::Clock,
                kind: if timer.is_pomodoro {
                    TopBarCapsuleKind::Pomodoro
                } else {
                    TopBarCapsuleKind::Timer
                },
                track: CapsuleTrack::LiveActivity,
                domain_badge: "手机计时".into(),
                strategy_tag: "手机实况·实时读秒".into(),
                score: 105,
                pill_text: timer.mm_ss_label.clone(),
                title: display_title,
                subtitle: format!(
                    "{} · {} · 双端实时读秒",
                    timer.app_name, timer.remaining_label
                ),
                metric: timer.mm_ss_label,
                source_tag: "手机实况",
                progress_pct: timer.progress_pct.max(1),
                countdown_secs: Some(timer.remaining_secs as u64),
                short_sig,
                primary_label: if !timer.key.is_empty() {
                    "关闭提醒"
                } else {
                    "查看"
                },
                primary_action,
                copy_value: None,
                complete_label: "✓ 已知晓",
                pickup_code: None,
                sms_address: None,
                im_thread_id: None,
                notif_key: (!timer.key.is_empty()).then(|| timer.key.clone()),
            });
        }

        // 2. Desktop Pomodoro Focus Timer
        let pomo_keys = vec![
            "capsule:desktop-pomodoro".to_string(),
            "timer:desktop-pomodoro".to_string(),
        ];
        if self.back_screen.pomodoro_active
            && !is_dismissed("desktop-pomodoro")
            && !self
                .reminder_governance
                .is_any_key_suppressed(&pomo_keys, None, None, None, now_ms)
        {
            let rem = self.live_pomodoro_remaining_secs();
            if rem > 0 {
                let total = self.back_screen.pomodoro_duration_secs.max(60);
                let mm_ss = stepx_companion::push::format_countdown_mm_ss(rem);
                let rem_cn = stepx_companion::push::format_countdown_remaining_cn(rem);
                let pct = (((total.saturating_sub(rem)) as f64 / total as f64) * 100.0)
                    .round()
                    .clamp(1.0, 100.0) as u8;
                items.push(TopBarLiveCapsuleItem {
                    id: "desktop-pomodoro".into(),
                    tag: "stepx_pomodoro".into(),
                    icon: UiIcon::Clock,
                    kind: TopBarCapsuleKind::Pomodoro,
                    track: CapsuleTrack::LiveActivity,
                    domain_badge: "番茄专注".into(),
                    strategy_tag: "手机实况·专注联动".into(),
                    score: 103,
                    pill_text: mm_ss.clone(),
                    title: "番茄专注".into(),
                    subtitle: format!("{rem_cn} · 桌面与手机背屏专注倒计时"),
                    metric: mm_ss,
                    source_tag: "手机实况",
                    progress_pct: pct,
                    countdown_secs: Some(rem as u64),
                    short_sig: "专注中".into(),
                    primary_label: "停止计时",
                    primary_action: Action::BackScreen(BackScreenAction::StopPomodoro),
                    copy_value: None,
                    complete_label: "✓ 结束",
                    pickup_code: None,
                    sms_address: None,
                    im_thread_id: None,
                    notif_key: None,
                });
            }
        }

        // 3. Reconcile fulfillment evidence before any item is shown.
        items.extend(self.fulfillment_capsules(now_ms));

        // 4. Pickup codes remain available until their receipt or explicit completion.
        // No-code shipments require direction evidence and are handled above.
        const MAX_PARCEL_CAPSULES: usize = 3;
        let express_pickup_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::ExpressPickup);
        let mut parcel_count = 0usize;
        for parcel in &self.express_parcels {
            if parcel_count >= MAX_PARCEL_CAPSULES {
                break;
            }
            let code = parcel.pickup_code.trim();
            let tracking = parcel.tracking_no.trim();
            if code.is_empty()
                || self.push.prefs.dismissed_pickup_codes.contains(&code.to_uppercase())
                || self.push.prefs.dismissed_pickup_codes.contains(&tracking.to_uppercase())
            {
                continue;
            }
            let parcel_ts = self.express_parcel_timestamp_ms(parcel, now_ms);
            if parcel_ts <= 0
                || now_ms.saturating_sub(parcel_ts) > express_pickup_ttl_ms
                || self.push.parcel_receipts.is_completed(code, tracking, parcel_ts)
            {
                continue;
            }
            let cap_id = format!("pickup-express-{code}");
            let mut keys =
                canonical_keys_for_code_or_parcel(Some(code), Some(tracking), None, None);
            keys.push(format!("capsule:{cap_id}"));
            if is_dismissed(&cap_id)
                || self.reminder_governance.is_any_key_suppressed(
                    &keys, Some(parcel_ts), Some(&parcel.status), None, now_ms,
                )
            {
                continue;
            }
            parcel_count += 1;
            let carrier = parcel.carrier.split('·').next().unwrap_or("快递").trim();
            items.push(TopBarLiveCapsuleItem {
                id: cap_id,
                tag: "stepx_pickup".into(),
                icon: UiIcon::Package,
                kind: TopBarCapsuleKind::PickupOrOtp,
                track: CapsuleTrack::ImportantMessage,
                domain_badge: "快递取件".into(),
                strategy_tag: "规则 · 待取件".into(),
                score: 93,
                pill_text: code.to_owned(),
                title: format!("取件码 · {carrier}"),
                subtitle: if parcel.station.trim().is_empty() {
                    "待取件".into()
                } else {
                    parcel.station.trim().to_owned()
                },
                metric: code.to_owned(),
                source_tag: "提醒",
                progress_pct: 0,
                countdown_secs: None,
                short_sig: code.chars().take(6).collect(),
                primary_label: "复制",
                primary_action: Action::CopyText(code.to_owned(), format!("已复制取件码 {code}")),
                copy_value: Some(code.to_owned()),
                complete_label: "已取件",
                pickup_code: Some(code.to_owned()),
                sms_address: None,
                im_thread_id: None,
                notif_key: None,
            });
        }
        if parcel_count < MAX_PARCEL_CAPSULES {
            if let Some((sender, code)) = self.extract_recent_sms_pickup_code() {
                let already_present = items.iter().any(|it| {
                    it.pill_text.trim().eq_ignore_ascii_case(&code)
                        || it
                            .pickup_code
                            .as_deref()
                            .is_some_and(|p| p.trim().eq_ignore_ascii_case(&code))
                });
                let cap_id = format!("pickup-sms-{code}");
                let mut p_keys =
                    canonical_keys_for_code_or_parcel(Some(&code), None, None, Some(&sender));
                p_keys.push(format!("capsule:{cap_id}"));
                if !already_present
                    && !is_dismissed(&cap_id)
                    && !self
                        .reminder_governance
                        .is_any_key_suppressed(&p_keys, None, None, None, now_ms)
                {
                    let short_sig: String = code.chars().take(6).collect();
                    items.push(TopBarLiveCapsuleItem {
                        id: cap_id,
                        tag: "stepx_pickup".into(),
                        icon: UiIcon::Package,
                        kind: TopBarCapsuleKind::PickupOrOtp,
                        track: CapsuleTrack::ImportantMessage,
                        domain_badge: "快递取件".into(),
                        strategy_tag: "策略·短信取件码提取".into(),
                        score: 93,
                        pill_text: code.clone(),
                        title: "取件码".into(),
                        subtitle: format!("{sender} · 短信取件码实时提取"),
                        metric: code.clone(),
                        source_tag: "重要消息",
                        progress_pct: 0,
                        countdown_secs: None,
                        short_sig,
                        primary_label: "复制",
                        primary_action: Action::CopyText(
                            code.clone(),
                            format!("已复制取件码 {code}"),
                        ),
                        copy_value: Some(code.clone()),
                        complete_label: "✓ 已取件",
                        pickup_code: Some(code),
                        sms_address: Some(sender),
                        im_thread_id: None,
                        notif_key: None,
                    });
                }
            }
        }

        // 4b. Public Utility / Community Emergency Notice Capsules (【停电通知】, 【停水通知】, 燃气通知)
        const UTILITY_ALERT_TTL_MS: i64 = 18 * 3_600_000;
        let mut utility_pills: std::collections::HashSet<String> = std::collections::HashSet::new();
        for notif in self
            .push
            .snapshot
            .notifications
            .iter()
            .filter(|n| !n.dismissed)
            .take(50)
        {
            if utility_pills.len() >= 2 {
                break;
            }
            if notif.post_time_ms <= 0
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&notif.text)
                || (now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(notif.post_time_ms) > UTILITY_ALERT_TTL_MS)
            {
                continue;
            }
            if let Some((badge, short_pill, headline)) =
                stepx_companion::home_hub::classify_public_utility_or_community_alert(
                    &notif.app_name,
                    &notif.title,
                    &notif.text,
                )
            {
                let cap_id = format!("utility-alert-{short_pill}");
                let n_keys = vec![format!("notif:{}", notif.key), format!("capsule:{cap_id}")];
                if is_dismissed(&cap_id)
                    || self.notification_center.dismissed_ids.contains(&notif.key)
                    || self.reminder_governance.is_any_key_suppressed(
                        &n_keys,
                        Some(notif.post_time_ms),
                        Some(&notif.text),
                        None,
                        now_ms,
                    )
                    || !utility_pills.insert(short_pill.clone())
                {
                    continue;
                }
                items.push(TopBarLiveCapsuleItem {
                    id: cap_id,
                    tag: "stepx_utility_alert".into(),
                    icon: UiIcon::Bell,
                    kind: TopBarCapsuleKind::ScheduleOrAlarm,
                    track: CapsuleTrack::ImportantMessage,
                    domain_badge: badge,
                    strategy_tag: "AI识别·民生应急提醒".into(),
                    score: 96,
                    pill_text: short_pill.clone(),
                    title: headline,
                    subtitle: Self::truncate_chars(notif.text.trim(), 42),
                    metric: "今日社区提醒".into(),
                    source_tag: "重要消息",
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: Self::truncate_chars(&short_pill, 4),
                    primary_label: "查看",
                    primary_action: if !notif.thread_id.is_empty() {
                        Action::BackScreen(BackScreenAction::OpenCapsuleTarget {
                            sms_address: None,
                            im_thread_id: Some(notif.thread_id.clone()),
                            open_calendar: false,
                            open_alarms: false,
                        })
                    } else {
                        Action::Page(Page::Sms)
                    },
                    copy_value: None,
                    complete_label: "✓ 已知晓",
                    pickup_code: None,
                    sms_address: None,
                    im_thread_id: (!notif.thread_id.is_empty()).then(|| notif.thread_id.clone()),
                    notif_key: (!notif.key.is_empty()).then(|| notif.key.clone()),
                });
            }
        }
        for thread in self.push.snapshot.threads.iter().take(30) {
            if utility_pills.len() >= 2 {
                break;
            }
            let last_msg_ts = thread
                .messages
                .iter()
                .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                .max()
                .unwrap_or(0);
            let effective_ts = thread.last_timestamp_ms.max(last_msg_ts);
            if effective_ts <= 0
                || thread.messages.last().is_some_and(|m| m.outgoing)
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&thread.last_message)
                || (now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(effective_ts) > UTILITY_ALERT_TTL_MS)
            {
                continue;
            }
            let tid_key = if thread.thread_id.trim().is_empty() {
                thread.title.trim()
            } else {
                thread.thread_id.trim()
            };
            let im_item_id = format!("im-thread:{tid_key}");
            if self.notification_center.dismissed_ids.contains(&im_item_id) {
                continue;
            }
            if let Some((badge, short_pill, headline)) =
                stepx_companion::home_hub::classify_public_utility_or_community_alert(
                    &thread.app_name,
                    &thread.title,
                    &thread.last_message,
                )
            {
                let cap_id = format!("utility-alert-{short_pill}");
                let mut t_keys = canonical_keys_for_im_thread(&thread.thread_id, None);
                t_keys.push(format!("capsule:{cap_id}"));
                if is_dismissed(&cap_id)
                    || self.reminder_governance.is_any_key_suppressed(
                        &t_keys,
                        Some(effective_ts),
                        Some(&thread.last_message),
                        None,
                        now_ms,
                    )
                    || !utility_pills.insert(short_pill.clone())
                {
                    continue;
                }
                items.push(TopBarLiveCapsuleItem {
                    id: cap_id,
                    tag: "stepx_utility_alert".into(),
                    icon: UiIcon::Bell,
                    kind: TopBarCapsuleKind::ScheduleOrAlarm,
                    track: CapsuleTrack::ImportantMessage,
                    domain_badge: badge,
                    strategy_tag: "AI识别·民生应急提醒".into(),
                    score: 96,
                    pill_text: short_pill.clone(),
                    title: headline,
                    subtitle: Self::truncate_chars(thread.last_message.trim(), 42),
                    metric: "今日社区提醒".into(),
                    source_tag: "重要消息",
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: Self::truncate_chars(&short_pill, 4),
                    primary_label: "查看",
                    primary_action: Action::BackScreen(BackScreenAction::OpenCapsuleTarget {
                        sms_address: None,
                        im_thread_id: Some(thread.thread_id.clone()),
                        open_calendar: false,
                        open_alarms: false,
                    }),
                    copy_value: None,
                    complete_label: "✓ 已知晓",
                    pickup_code: None,
                    sms_address: None,
                    im_thread_id: Some(thread.thread_id.clone()),
                    notif_key: None,
                });
            }
        }
        for msg in self.sms.iter().take(30) {
            if utility_pills.len() >= 2 {
                break;
            }
            if msg.outgoing
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&msg.body)
                || (msg.date_ms > 0
                    && now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(msg.date_ms) > UTILITY_ALERT_TTL_MS)
            {
                continue;
            }
            if let Some((badge, short_pill, headline)) =
                stepx_companion::home_hub::classify_public_utility_or_community_alert(
                    "",
                    &msg.address,
                    &msg.body,
                )
            {
                let cap_id = format!("utility-alert-{short_pill}");
                let s_keys = vec![format!("sms:{}", msg.address), format!("capsule:{cap_id}")];
                if is_dismissed(&cap_id)
                    || self.reminder_governance.is_any_key_suppressed(
                        &s_keys,
                        (msg.date_ms > 0).then_some(msg.date_ms),
                        Some(&msg.body),
                        None,
                        now_ms,
                    )
                    || !utility_pills.insert(short_pill.clone())
                {
                    continue;
                }
                items.push(TopBarLiveCapsuleItem {
                    id: cap_id,
                    tag: "stepx_utility_alert".into(),
                    icon: UiIcon::Bell,
                    kind: TopBarCapsuleKind::ScheduleOrAlarm,
                    track: CapsuleTrack::ImportantMessage,
                    domain_badge: badge,
                    strategy_tag: "AI识别·民生应急提醒".into(),
                    score: 96,
                    pill_text: short_pill.clone(),
                    title: headline,
                    subtitle: Self::truncate_chars(msg.body.trim(), 42),
                    metric: "今日社区提醒".into(),
                    source_tag: "重要消息",
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: Self::truncate_chars(&short_pill, 4),
                    primary_label: "查看",
                    primary_action: Action::BackScreen(BackScreenAction::OpenCapsuleTarget {
                        sms_address: Some(msg.address.clone()),
                        im_thread_id: None,
                        open_calendar: false,
                        open_alarms: false,
                    }),
                    copy_value: None,
                    complete_label: "✓ 已知晓",
                    pickup_code: None,
                    sms_address: Some(msg.address.clone()),
                    im_thread_id: None,
                    notif_key: None,
                });
            }
        }

        // 5. OTP Verification Code
        let otp_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::OtpCode);
        let sms_silent = self.push.prefs.effective_popup_mode("com.android.mms")
            == stepx_companion::desktop_notify::PopupMode::Silent;
        let otp_candidate: Option<(String, String)> = (!sms_silent)
            .then(|| {
                self.push
                    .snapshot
                    .otp_banners
                    .iter()
                    .find(|b| {
                        let code_trimmed = b.code.trim();
                        let code_up = code_trimmed.to_uppercase();
                        let o_keys = canonical_keys_for_code_or_parcel(
                            Some(&code_up),
                            None,
                            (!b.notification_key.is_empty()).then_some(b.notification_key.as_str()),
                            None,
                        );
                        !code_up.is_empty()
                            && !self
                                .notification_center
                                .read_ids
                                .contains(&format!("otp:{code_trimmed}"))
                            && !self
                                .notification_center
                                .dismissed_ids
                                .contains(&format!("otp:{code_trimmed}"))
                            && (b.source_app.trim().is_empty()
                                || self.push.prefs.effective_popup_mode(b.source_app.trim())
                                    != stepx_companion::desktop_notify::PopupMode::Silent)
                            && !items
                                .iter()
                                .any(|it| it.pill_text.trim().eq_ignore_ascii_case(&code_up))
                            && !self.push.prefs.dismissed_pickup_codes.contains(&code_up)
                            && !self.reminder_governance.is_any_key_suppressed(
                                &o_keys,
                                (b.timestamp_ms > 0).then_some(b.timestamp_ms),
                                Some(&b.code),
                                None,
                                now_ms,
                            )
                            && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&b.raw_text)
                            && !stepx_companion::home_hub::is_completed_express_text(&b.raw_text)
                            && !self
                                .push
                                .parcel_receipts
                                .text_is_completed(&b.raw_text, b.timestamp_ms)
                            && (b.timestamp_ms <= 0
                                || now_ms <= 1_700_000_000_000
                                || now_ms.saturating_sub(b.timestamp_ms) <= otp_ttl_ms)
                    })
                    .map(|b| {
                        let src = if !b.source_app.trim().is_empty() {
                            b.source_app.trim().to_string()
                        } else if !b.title.trim().is_empty() {
                            b.title.trim().to_string()
                        } else {
                            "手机验证码".to_string()
                        };
                        (b.code.trim().to_string(), src)
                    })
                    .or_else(|| self.extract_recent_sms_otp_code())
            })
            .flatten();

        if let Some((otp_code, otp_source)) = otp_candidate.filter(|(c, _)| {
            !self
                .notification_center
                .read_ids
                .contains(&format!("otp:{}", c.trim()))
                && !self
                    .notification_center
                    .dismissed_ids
                    .contains(&format!("otp:{}", c.trim()))
                && !items
                    .iter()
                    .any(|it| it.pill_text.trim().eq_ignore_ascii_case(c.trim()))
        }) {
            let cap_id = format!("otp-code-{otp_code}");
            let o_keys = vec![
                format!("code:{}", otp_code.trim().to_uppercase()),
                format!("capsule:{cap_id}"),
            ];
            if !is_dismissed(&cap_id)
                && !self
                    .reminder_governance
                    .is_any_key_suppressed(&o_keys, None, None, None, now_ms)
            {
                let short_sig: String = otp_code.chars().take(6).collect();
                items.push(TopBarLiveCapsuleItem {
                    id: cap_id,
                    tag: "stepx_otp".into(),
                    icon: UiIcon::Key,
                    kind: TopBarCapsuleKind::PickupOrOtp,
                    track: CapsuleTrack::ImportantMessage,
                    domain_badge: "验证码".into(),
                    strategy_tag: "策略·15分钟动态验证码".into(),
                    score: 102,
                    pill_text: otp_code.clone(),
                    title: format!("验证码 · {}", Self::truncate_chars(&otp_source, 8)),
                    subtitle: format!("{otp_source} · 一键复制或同步手机顶部胶囊"),
                    metric: otp_code.clone(),
                    source_tag: "重要消息",
                    progress_pct: 0,
                    countdown_secs: Some((otp_ttl_ms / 1000).clamp(60, 3600) as u64),
                    short_sig,
                    primary_label: "复制",
                    primary_action: Action::Push(PushAction::CopyOtp(otp_code.clone())),
                    copy_value: Some(otp_code.clone()),
                    complete_label: "✓ 已用",
                    pickup_code: Some(otp_code),
                    sms_address: None,
                    im_thread_id: None,
                    notif_key: None,
                });
            }
        }

        // 6. Direct `@` Mentions & BOSS直聘 / Career Recruitment & High-Priority Unread IM
        let im_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::UnreadMessage);
        let mut mention_count = 0usize;
        let mut career_count = 0usize;
        for thread in &self.push.snapshot.threads {
            let last_msg_ts = thread
                .messages
                .iter()
                .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                .max()
                .unwrap_or(0);
            let effective_ts = thread.last_timestamp_ms.max(last_msg_ts);
            let tid_key = if thread.thread_id.trim().is_empty() {
                thread.title.trim()
            } else {
                thread.thread_id.trim()
            };
            let im_item_id = format!("im-thread:{tid_key}");
            if effective_ts <= 0
                || thread.unread_count == 0
                || self.push.prefs.effective_popup_mode(&thread.package_name)
                    == stepx_companion::desktop_notify::PopupMode::Silent
                || self.notification_center.read_ids.contains(&im_item_id)
                || self.notification_center.dismissed_ids.contains(&im_item_id)
                || thread.messages.last().is_some_and(|m| m.outgoing)
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&thread.last_message)
                || (now_ms > 1_700_000_000_000 && now_ms.saturating_sub(effective_ts) > im_ttl_ms)
            {
                continue;
            }
            let mut t_keys = canonical_keys_for_im_thread(&thread.thread_id, None);
            t_keys.push(format!("capsule:mention-{}", thread.thread_id));
            t_keys.push(format!("capsule:career-{}", thread.thread_id));
            let mute_pkg = format!("pkg:{}", thread.package_name);
            if self.reminder_governance.is_any_key_suppressed(
                &t_keys,
                Some(effective_ts),
                Some(&thread.last_message),
                Some(&mute_pkg),
                now_ms,
            ) {
                continue;
            }
            if mention_count < 2 {
                if stepx_companion::push::thread_mentions_current_user(
                    thread,
                    &self.push.snapshot.notifications,
                ) {
                    let at_tok = "@我".to_string();
                    let cap_id = format!("mention-{}", thread.thread_id);
                    if !is_dismissed(&cap_id) {
                        mention_count += 1;
                        let clean_title =
                            Self::truncate_chars(thread.title.replace("[表情]", "").trim(), 10);
                        items.push(TopBarLiveCapsuleItem {
                            id: cap_id,
                            tag: "stepx_mention".into(),
                            icon: UiIcon::Sms,
                            kind: TopBarCapsuleKind::DirectMention,
                            track: CapsuleTrack::ImportantMessage,
                            domain_badge: "有人@我".into(),
                            strategy_tag: format!("策略+AI·直接{at_tok}"),
                            score: 96,
                            pill_text: at_tok.clone(),
                            title: format!("@提及 · {clean_title}"),
                            subtitle: Self::truncate_chars(&thread.last_message, 42),
                            metric: at_tok,
                            source_tag: "重要消息",
                            progress_pct: 0,
                            countdown_secs: None,
                            short_sig: "有人@".into(),
                            primary_label: "打开会话",
                            primary_action: Action::BackScreen(
                                BackScreenAction::OpenCapsuleTarget {
                                    sms_address: None,
                                    im_thread_id: Some(thread.thread_id.clone()),
                                    open_calendar: false,
                                    open_alarms: false,
                                },
                            ),
                            copy_value: None,
                            complete_label: "✓ 已知晓",
                            pickup_code: None,
                            sms_address: None,
                            im_thread_id: Some(thread.thread_id.clone()),
                            notif_key: None,
                        });
                        continue;
                    }
                }
            }
            if career_count < 2
                && stepx_companion::push::is_career_or_recruitment_signal(
                    &thread.package_name,
                    &thread.app_name,
                    &thread.title,
                    &thread.last_message,
                )
            {
                let cap_id = format!("career-{}", thread.thread_id);
                if !is_dismissed(&cap_id) {
                    career_count += 1;
                    let clean_title = Self::truncate_chars(thread.title.trim(), 10);
                    items.push(TopBarLiveCapsuleItem {
                        id: cap_id,
                        tag: "stepx_career".into(),
                        icon: UiIcon::Briefcase,
                        kind: TopBarCapsuleKind::CareerRecruitment,
                        track: CapsuleTrack::ImportantMessage,
                        domain_badge: "BOSS直聘".into(),
                        strategy_tag: "AI识别·职场招聘沟通".into(),
                        score: 92,
                        pill_text: "招聘沟通".into(),
                        title: format!("BOSS直聘 · {clean_title}"),
                        subtitle: Self::truncate_chars(&thread.last_message, 42),
                        metric: "HR沟通".into(),
                        source_tag: "重要消息",
                        progress_pct: 0,
                        countdown_secs: None,
                        short_sig: "BOSS".into(),
                        primary_label: "打开会话",
                        primary_action: Action::BackScreen(BackScreenAction::OpenCapsuleTarget {
                            sms_address: None,
                            im_thread_id: Some(thread.thread_id.clone()),
                            open_calendar: false,
                            open_alarms: false,
                        }),
                        copy_value: None,
                        complete_label: "✓ 已知晓",
                        pickup_code: None,
                        sms_address: None,
                        im_thread_id: Some(thread.thread_id.clone()),
                        notif_key: None,
                    });
                }
            }
        }
        if career_count == 0 {
            for notif in self.push.snapshot.notifications.iter().take(50) {
                if notif.dismissed
                    || notif.read
                    || self.push.prefs.effective_popup_mode(&notif.package_name)
                        == stepx_companion::desktop_notify::PopupMode::Silent
                    || (notif.post_time_ms > 0
                        && now_ms > 1_700_000_000_000
                        && now_ms.saturating_sub(notif.post_time_ms) > im_ttl_ms)
                {
                    continue;
                }
                if stepx_companion::push::is_career_or_recruitment_signal(
                    &notif.package_name,
                    &notif.app_name,
                    &notif.title,
                    &notif.text,
                ) || notif.text.contains("BOSS直聘")
                {
                    let cap_id = format!("career-notif-{}", notif.key);
                    let n_keys = vec![format!("notif:{}", notif.key), format!("capsule:{cap_id}")];
                    let mute_pkg = format!("pkg:{}", notif.package_name);
                    if !is_dismissed(&cap_id)
                        && !self.reminder_governance.is_any_key_suppressed(
                            &n_keys,
                            (notif.post_time_ms > 0).then_some(notif.post_time_ms),
                            Some(&notif.text),
                            Some(&mute_pkg),
                            now_ms,
                        )
                    {
                        let summary = if notif.title.contains("安装成功") {
                            "BOSS直聘已就绪 · HR 新消息将实时推送胶囊".to_string()
                        } else {
                            Self::truncate_chars(&format!("{} · {}", notif.title, notif.text), 40)
                        };
                        items.push(TopBarLiveCapsuleItem {
                            id: cap_id,
                            tag: "stepx_career".into(),
                            icon: UiIcon::Briefcase,
                            kind: TopBarCapsuleKind::CareerRecruitment,
                            track: CapsuleTrack::ImportantMessage,
                            domain_badge: "BOSS直聘".into(),
                            strategy_tag: "AI识别·职场招聘通道".into(),
                            score: 91,
                            pill_text: "BOSS直聘".into(),
                            title: "BOSS直聘 · 招聘动态".into(),
                            subtitle: summary,
                            metric: "BOSS".into(),
                            source_tag: "重要消息",
                            progress_pct: 0,
                            countdown_secs: None,
                            short_sig: "BOSS".into(),
                            primary_label: "查看消息",
                            primary_action: Action::Page(Page::Sms),
                            copy_value: None,
                            complete_label: "✓ 已知晓",
                            pickup_code: None,
                            sms_address: None,
                            im_thread_id: (!notif.thread_id.is_empty())
                                .then(|| notif.thread_id.clone()),
                            notif_key: (!notif.key.is_empty()).then(|| notif.key.clone()),
                        });
                        break;
                    }
                }
            }
        }

        // 7. Unread Message Aggregation (`消息未读`)
        let unread_im_threads: Vec<&stepx_companion::push::ImThread> = self
            .push
            .snapshot
            .threads
            .iter()
            .filter(|t| {
                let tid_key = if t.thread_id.trim().is_empty() {
                    t.title.trim()
                } else {
                    t.thread_id.trim()
                };
                let im_item_id = format!("im-thread:{tid_key}");
                let im_raw_id = format!("im-thread:{}", t.thread_id);
                t.unread_count > 0
                    && t.channel() != stepx_companion::push::ImChannel::Sms
                    && !Self::is_excluded_im_package(&t.package_name)
                    && self.push.prefs.effective_popup_mode(&t.package_name)
                        != stepx_companion::desktop_notify::PopupMode::Silent
                    && !self.notification_center.read_ids.contains(&im_item_id)
                    && !self.notification_center.dismissed_ids.contains(&im_item_id)
                    && !self.notification_center.read_ids.contains(&im_raw_id)
                    && !self.notification_center.dismissed_ids.contains(&im_raw_id)
                    && !t.title.trim_start().starts_with('{')
                    && !stepx_companion::push::is_timer_or_countdown_notification(
                        &t.package_name,
                        &t.app_name,
                        &t.title,
                        &t.last_message,
                    )
                    && !t.messages.last().is_some_and(|m| m.outgoing)
                    && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&t.last_message)
                    && !self.reminder_governance.is_any_key_suppressed(
                        &canonical_keys_for_im_thread(&t.thread_id, None),
                        (t.last_timestamp_ms > 0).then_some(t.last_timestamp_ms),
                        Some(&t.last_message),
                        Some(&format!("pkg:{}", t.package_name)),
                        now_ms,
                    )
            })
            .collect();
        let unread_sms_count = if sms_silent {
            0
        } else {
            self.fresh_unread_sms_thread_count()
        };
        let total_unread: usize = unread_im_threads
            .iter()
            .map(|t| t.unread_count.max(1))
            .sum::<usize>()
            + unread_sms_count;
        if total_unread > 0
            && !is_dismissed("unread-summary")
            && !self.reminder_governance.is_any_key_suppressed(
                &["capsule:unread-summary".to_string()],
                None,
                None,
                None,
                now_ms,
            )
        {
            let first_thread = unread_im_threads.first().copied();
            let title = if let Some(t) = first_thread {
                format!(
                    "未读 · {}",
                    Self::truncate_chars(&t.title.replace("[表情]", ""), 9)
                )
            } else {
                format!("消息未读 · {total_unread}条")
            };
            let subtitle = if let Some(t) = first_thread {
                format!(
                    "{} ({total_unread}条未读) · {}",
                    t.app_name,
                    Self::truncate_chars(&t.last_message, 28)
                )
            } else {
                format!("共 {total_unread} 条未读短信/消息待查看")
            };
            items.push(TopBarLiveCapsuleItem {
                id: "unread-summary".into(),
                tag: "stepx_unread".into(),
                icon: UiIcon::Sms,
                kind: TopBarCapsuleKind::UnreadMessage,
                track: CapsuleTrack::ImportantMessage,
                domain_badge: "消息未读".into(),
                strategy_tag: "策略+AI·重要未读聚合".into(),
                score: 89,
                pill_text: format!("{total_unread}未读"),
                title,
                subtitle,
                metric: format!("{total_unread}未读"),
                source_tag: "重要消息",
                progress_pct: 0,
                countdown_secs: None,
                short_sig: format!("{total_unread}未读"),
                primary_label: "去回复",
                primary_action: Action::BackScreen(BackScreenAction::OpenCapsuleTarget {
                    sms_address: None,
                    im_thread_id: first_thread.map(|t| t.thread_id.clone()),
                    open_calendar: false,
                    open_alarms: false,
                }),
                copy_value: None,
                complete_label: "✓ 已读",
                pickup_code: None,
                sms_address: None,
                im_thread_id: first_thread.map(|t| t.thread_id.clone()),
                notif_key: None,
            });
        }

        // 7b. Unread Missed Calls (`未接来电`)
        if let Some(missed_item) = self.notification_center.items.iter().find(|item| {
            let pkg = if item.package_name.trim().is_empty() {
                "com.android.dialer"
            } else {
                item.package_name.trim()
            };
            item.visual == super::notification_center_state::NotificationVisualKind::MissedCall
                && item.unread
                && !self.notification_center.read_ids.contains(&item.id)
                && !self.notification_center.dismissed_ids.contains(&item.id)
                && !is_dismissed(&item.id)
                && self.push.prefs.effective_popup_mode(pkg)
                    != stepx_companion::desktop_notify::PopupMode::Silent
        }) {
            let short_sig: String = missed_item.title.trim().chars().take(6).collect();
            items.push(TopBarLiveCapsuleItem {
                id: missed_item.id.clone(),
                tag: "stepx_missed_call".into(),
                icon: UiIcon::Phone,
                kind: TopBarCapsuleKind::MissedCall,
                track: CapsuleTrack::ImportantMessage,
                domain_badge: "未接来电".into(),
                strategy_tag: "策略·未接来电提醒".into(),
                score: 95,
                pill_text: "未接来电".into(),
                title: missed_item.title.clone(),
                subtitle: missed_item.summary.clone(),
                metric: "未接".into(),
                source_tag: "重要消息",
                progress_pct: 0,
                countdown_secs: None,
                short_sig: if short_sig.is_empty() {
                    "来电".into()
                } else {
                    short_sig
                },
                primary_label: "回拨",
                primary_action: match &missed_item.popup_widget {
                    super::notification_center_state::NotificationPopupWidget::CallerBadge { number, .. } =>
                        Action::LiveCall(super::live_calls::LiveCallAction::CallBack(number.clone())),
                    _ => Action::Page(Page::Phone),
                },
                copy_value: None,
                complete_label: "已读",
                pickup_code: None,
                sms_address: None,
                im_thread_id: None,
                notif_key: None,
            });
        }

        // 8. Active Amoo / Work / Phone GUI Task
        if (self.work.running
            || self.workspace_busy()
            || self.amoo_ui.gui_running
            || (self.busy && matches!(self.page, Page::Amoo | Page::Work)))
            && !is_dismissed("amoo-running")
            && !self.reminder_governance.is_any_key_suppressed(
                &["capsule:amoo-running".to_string()],
                None,
                None,
                None,
                now_ms,
            )
        {
            let metric = if self.amoo_ui.gui_running {
                "手机GUI".to_string()
            } else {
                "执行中".to_string()
            };
            items.push(TopBarLiveCapsuleItem {
                id: "amoo-running".into(),
                tag: "stepx_amoo".into(),
                icon: UiIcon::Sparkles,
                kind: TopBarCapsuleKind::AmooTask,
                track: CapsuleTrack::LiveActivity,
                domain_badge: "Amoo协同".into(),
                strategy_tag: "手机实况·跨端任务".into(),
                score: 97,
                pill_text: "Amoo执行".into(),
                title: "Amoo 协同执行中".into(),
                subtitle: "跨端任务正在执行 · 可同步进度到手机顶部胶囊与背屏".into(),
                metric,
                source_tag: "手机实况",
                progress_pct: 62,
                countdown_secs: None,
                short_sig: "协同中".into(),
                primary_label: "查看",
                primary_action: Action::Page(Page::Amoo),
                copy_value: None,
                complete_label: "✓ 已知晓",
                pickup_code: None,
                sms_address: None,
                im_thread_id: None,
                notif_key: None,
            });
        }

        // 9. Active Pinned Phone Capsule
        let live_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::LiveTimer);
        if let Some((tag, pill_text, title, pushed_at)) = &self.back_screen.active_phone_capsule {
            let duplicate_pomo = tag == "stepx_pomodoro" && self.back_screen.pomodoro_active;
            let cap_id = format!("pinned-{tag}");
            let pinned_keys = vec![format!("capsule:{cap_id}")];
            let completed_pickup = tag == "stepx_pickup"
                && (self.push.parcel_receipts.text_is_completed(
                    &format!("取件码 {pill_text}"),
                    now_ms.saturating_sub(pushed_at.elapsed().as_millis() as i64),
                ) || self
                    .push
                    .prefs
                    .dismissed_pickup_codes
                    .contains(&pill_text.trim().to_uppercase()));
            if !duplicate_pomo
                && !completed_pickup
                && !is_dismissed(&cap_id)
                && (pushed_at.elapsed().as_millis() as i64 <= live_ttl_ms
                    || self
                        .reminder_governance
                        .is_force_kept(&format!("capsule:{cap_id}"), now_ms))
                && !self.reminder_governance.is_any_key_suppressed(
                    &pinned_keys,
                    None,
                    None,
                    None,
                    now_ms,
                )
            {
                let clean_title = title.trim().trim_start_matches('📲').trim();
                let display_title = if clean_title.is_empty() {
                    format!("已同步手机胶囊 · {}", pill_text.trim())
                } else {
                    clean_title.to_string()
                };
                let clean_pill = pill_text
                    .trim_start_matches(|c: char| {
                        !c.is_ascii_alphanumeric() && !('\u{4e00}'..='\u{9fff}').contains(&c)
                    })
                    .trim();
                let short_sig: String = clean_pill
                    .chars()
                    .filter(|c| !c.is_whitespace())
                    .take(6)
                    .collect();
                items.push(TopBarLiveCapsuleItem {
                    id: cap_id,
                    tag: tag.clone(),
                    icon: UiIcon::Smartphone,
                    kind: TopBarCapsuleKind::PinnedPhone,
                    track: CapsuleTrack::LiveActivity,
                    domain_badge: "背屏常驻".into(),
                    strategy_tag: "手机实况·双端常驻".into(),
                    score: 90,
                    pill_text: clean_pill.to_string(),
                    title: display_title,
                    subtitle: "已同步手机胶囊 · 手机顶部状态栏与背屏点阵联动".into(),
                    metric: Self::truncate_chars(clean_pill, 9),
                    source_tag: "手机实况",
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: if short_sig.is_empty() {
                        "StepX".into()
                    } else {
                        short_sig
                    },
                    primary_label: "收起",
                    primary_action: Action::BackScreen(BackScreenAction::DismissPhoneLiveCapsule(
                        tag.clone(),
                    )),
                    copy_value: None,
                    complete_label: "✓ 收起",
                    pickup_code: None,
                    sms_address: None,
                    im_thread_id: None,
                    notif_key: None,
                });
            }
        }

        // 10. Upcoming Calendar Schedule / Calendar Notification / Active Alarm
        let sched_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::ScheduleAndMeeting);
        let now_local = chrono::Local::now();
        let today_ymd = (
            chrono::Datelike::year(&now_local),
            chrono::Datelike::month(&now_local),
            chrono::Datelike::day(&now_local),
        );
        let cal_silent = self.push.prefs.effective_popup_mode("com.android.calendar")
            == stepx_companion::desktop_notify::PopupMode::Silent;
        if !cal_silent {
            if let Some(ev) = self
                .back_screen
                .stepos_suite
                .calendar_events
                .iter()
                .find(|ev| {
                    let occurrence = self.calendar_event_action_limit(ev.id).is_some();
                    let suffix = if occurrence { format!("{}:{}", ev.id, ev.dtstart) } else { ev.id.to_string() };
                    let capsule_id = format!("cal-{suffix}");
                    let notification_id = format!("calendar:{suffix}");
                    let mut c_keys = if occurrence { vec![] } else { canonical_keys_for_calendar_event(ev.id) };
                    c_keys.push(format!("capsule:{capsule_id}"));
                    !ev.title.trim().is_empty()
                        && !self.calendar_event_is_completed(ev)
                        && !self
                            .notification_center
                            .read_ids
                            .contains(&notification_id)
                        && !self
                            .notification_center
                            .dismissed_ids
                            .contains(&notification_id)
                        && !is_dismissed(&capsule_id)
                        && !self
                            .reminder_governance
                            .is_any_key_suppressed(&c_keys, None, None, None, now_ms)
                        && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&ev.title)
                        && (self.demo_mode
                            || (ev.date_ymd() == today_ymd
                                && (ev.dtstart <= 0
                                    || (ev.dtend > 0 && now_ms <= ev.dtend + sched_ttl_ms)
                                    || (ev.dtend <= 0
                                        && now_ms.saturating_sub(ev.dtstart)
                                            <= 3_600_000 + sched_ttl_ms))))
                })
            {
                let hm = ev.hm_label();
                let (_y, m, d) = ev.date_ymd();
                let date_str = format!("{m:02}-{d:02}");
                let short_sig: String = ev.title.trim().chars().take(6).collect();
                items.push(TopBarLiveCapsuleItem {
                    id: if self.calendar_event_action_limit(ev.id).is_some() {
                        format!("cal-{}:{}", ev.id, ev.dtstart)
                    } else { format!("cal-{}", ev.id) },
                    tag: "stepx_schedule".into(),
                    icon: UiIcon::Calendar,
                    kind: TopBarCapsuleKind::ScheduleOrAlarm,
                    track: CapsuleTrack::ImportantMessage,
                    domain_badge: "今日日程".into(),
                    strategy_tag: "策略+AI·临近日程提醒".into(),
                    score: 91,
                    pill_text: hm.clone(),
                    title: Self::truncate_chars(ev.title.trim(), 14),
                    subtitle: format!("{date_str} {hm} · 双端日程提醒"),
                    metric: hm,
                    source_tag: "重要消息",
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: if short_sig.is_empty() {
                        "日程".into()
                    } else {
                        short_sig
                    },
                    primary_label: "查看日程",
                    primary_action: Action::BackScreen(BackScreenAction::CalendarOpenOccurrence(ev.id, ev.dtstart)),
                    copy_value: None,
                    complete_label: "✓ 已知晓",
                    pickup_code: None,
                    sms_address: None,
                    im_thread_id: None,
                    notif_key: None,
                });
            } else if let Some(cal_notif) = self.push.snapshot.notifications.iter().find(|n| {
                let n_keys = vec![
                    format!("notif:{}", n.key),
                    format!("capsule:cal-notif-{}", n.key),
                ];
                !n.dismissed
                    && !n.read
                    && self.push.prefs.effective_popup_mode(&n.package_name)
                        != stepx_companion::desktop_notify::PopupMode::Silent
                    && (n.package_name.contains("calendar")
                        || n.title.contains("日程")
                        || n.title.contains("行前准备"))
                    && !n.title.trim().is_empty()
                    && !is_dismissed(&format!("cal-notif-{}", n.key))
                    && !self.reminder_governance.is_any_key_suppressed(
                        &n_keys,
                        (n.post_time_ms > 0).then_some(n.post_time_ms),
                        Some(&n.text),
                        None,
                        now_ms,
                    )
            }) {
                let short_sig: String = cal_notif.title.trim().chars().take(6).collect();
                items.push(TopBarLiveCapsuleItem {
                    id: format!("cal-notif-{}", cal_notif.key),
                    tag: "stepx_schedule".into(),
                    icon: UiIcon::Calendar,
                    kind: TopBarCapsuleKind::ScheduleOrAlarm,
                    track: CapsuleTrack::ImportantMessage,
                    domain_badge: "日程提醒".into(),
                    strategy_tag: "策略·手机日历提醒".into(),
                    score: 87,
                    pill_text: "日程".into(),
                    title: Self::truncate_chars(cal_notif.title.trim(), 14),
                    subtitle: Self::truncate_chars(cal_notif.text.trim(), 38),
                    metric: "日程".into(),
                    source_tag: "重要消息",
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: if short_sig.is_empty() {
                        "日程".into()
                    } else {
                        short_sig
                    },
                    primary_label: "查看日程",
                    primary_action: Action::Page(Page::Calendar),
                    copy_value: None,
                    complete_label: "✓ 已知晓",
                    pickup_code: None,
                    sms_address: None,
                    im_thread_id: None,
                    notif_key: (!cal_notif.key.is_empty()).then(|| cal_notif.key.clone()),
                });
            }
        }

        let alarm_silent = self
            .push
            .prefs
            .effective_popup_mode("com.android.deskclock")
            == stepx_companion::desktop_notify::PopupMode::Silent
            || self
                .push
                .prefs
                .effective_popup_mode("com.android.alarmclock")
                == stepx_companion::desktop_notify::PopupMode::Silent;
        if !alarm_silent {
            if let Some(alarm) = self.back_screen.stepos_suite.alarms.iter().find(|a| {
                let a_keys = vec![format!("alarm:{}", a.id), format!("capsule:alarm-{}", a.id)];
                a.is_today_valid_unexpired_at(now_ms, &self.reminder_governance)
                    && !self
                        .notification_center
                        .read_ids
                        .contains(&format!("alarm:{}", a.id))
                    && !self
                        .notification_center
                        .dismissed_ids
                        .contains(&format!("alarm:{}", a.id))
                    && !is_dismissed(&format!("alarm-{}", a.id))
                    && !self
                        .reminder_governance
                        .is_any_key_suppressed(&a_keys, None, None, None, now_ms)
            }) {
                let hm = alarm.time_label();
                let label = alarm.display_title();
                items.push(TopBarLiveCapsuleItem {
                    id: format!("alarm-{}", alarm.id),
                    tag: "stepx_alarm".into(),
                    icon: UiIcon::Clock,
                    kind: TopBarCapsuleKind::ScheduleOrAlarm,
                    track: CapsuleTrack::LiveActivity,
                    domain_badge: "手机闹钟".into(),
                    strategy_tag: "手机实况·闹钟已开启".into(),
                    score: 85,
                    pill_text: hm.clone(),
                    title: Self::truncate_chars(label, 12),
                    subtitle: format!("手机闹钟 {hm} · 已开启双端提醒"),
                    metric: hm.clone(),
                    source_tag: "手机实况",
                    progress_pct: 0,
                    countdown_secs: None,
                    short_sig: hm,
                    primary_label: "查看",
                    primary_action: Action::Page(Page::Alarms),
                    copy_value: None,
                    complete_label: "✓ 已知晓",
                    pickup_code: None,
                    sms_address: None,
                    im_thread_id: None,
                    notif_key: None,
                });
            }
        }

        items.sort_by(|a, b| b.score.cmp(&a.score));
        items
    }

    fn extract_recent_sms_otp_code(&self) -> Option<(String, String)> {
        use stepx_companion::home_reminder_governance::{
            canonical_keys_for_code_or_parcel, ReminderCategory,
        };
        if self.push.prefs.effective_popup_mode("com.android.mms")
            == stepx_companion::desktop_notify::PopupMode::Silent
        {
            return None;
        }
        let now_ms = chrono::Local::now().timestamp_millis();
        let otp_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::OtpCode);
        for msg in self.sms.iter().take(12) {
            if msg.read
                || msg.outgoing
                || (msg.date_ms > 0
                    && now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(msg.date_ms) > otp_ttl_ms)
            {
                continue;
            }
            let body = msg.body.trim();
            if !(body.contains("验证码") || body.contains("校验码") || body.contains("动态码"))
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(body)
            {
                continue;
            }
            for token in body.split(|c: char| !c.is_ascii_digit()) {
                let keys =
                    canonical_keys_for_code_or_parcel(Some(token), None, None, Some(&msg.address));
                if (4..=8).contains(&token.len())
                    && !self
                        .notification_center
                        .read_ids
                        .contains(&format!("otp:{token}"))
                    && !self
                        .notification_center
                        .dismissed_ids
                        .contains(&format!("otp:{token}"))
                    && !self
                        .push
                        .prefs
                        .dismissed_pickup_codes
                        .contains(&token.to_uppercase())
                    && !self.reminder_governance.is_any_key_suppressed(
                        &keys,
                        (msg.date_ms > 0).then_some(msg.date_ms),
                        Some(&msg.body),
                        None,
                        now_ms,
                    )
                {
                    let sender = if msg.address.trim().is_empty() {
                        "短信验证码".to_string()
                    } else {
                        msg.address.trim().to_string()
                    };
                    return Some((token.to_string(), sender));
                }
            }
        }
        None
    }

    fn extract_recent_sms_pickup_code(&self) -> Option<(String, String)> {
        use stepx_companion::home_reminder_governance::{
            canonical_keys_for_code_or_parcel, ReminderCategory,
        };
        if self.push.prefs.effective_popup_mode("com.android.mms")
            == stepx_companion::desktop_notify::PopupMode::Silent
        {
            return None;
        }
        let now_ms = chrono::Local::now().timestamp_millis();
        let express_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::ExpressPickup);
        for msg in self.sms.iter().take(15) {
            if msg.outgoing
                || (msg.date_ms > 0
                    && now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(msg.date_ms) > express_ttl_ms)
            {
                continue;
            }
            let body = msg.body.trim();
            if !(body.contains("取件码") || body.contains("提货码") || body.contains("凭码"))
                || stepx_companion::capsule_fulfillment::parcel_direction(body)
                    == stepx_companion::capsule_fulfillment::Direction::Send
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(body)
                || stepx_companion::home_hub::is_completed_express_text(body)
                || self
                    .push
                    .parcel_receipts
                    .text_is_completed(body, msg.date_ms)
            {
                continue;
            }
            if let Some(code) = stepx_companion::home_hub::extract_pickup_code(body) {
                let t = code.as_str();
                let keys =
                    canonical_keys_for_code_or_parcel(Some(t), None, None, Some(&msg.address));
                if !self
                    .push
                    .prefs
                    .dismissed_pickup_codes
                    .contains(&t.to_uppercase())
                    && !self.reminder_governance.is_any_key_suppressed(
                        &keys,
                        (msg.date_ms > 0).then_some(msg.date_ms),
                        Some(&msg.body),
                        None,
                        now_ms,
                    )
                {
                    let sender = if msg.address.trim().is_empty() {
                        "驿站短信".to_string()
                    } else {
                        msg.address.trim().to_string()
                    };
                    return Some((sender, t.to_string()));
                }
            }
        }
        None
    }

    pub(crate) fn live_capsule_kind_tint(&self, kind: TopBarCapsuleKind) -> (ColorU, ColorU) {
        let p = self.p();
        match kind {
            TopBarCapsuleKind::Timer => (p.warning, p.warning_soft),
            TopBarCapsuleKind::Pomodoro => (p.indigo, p.indigo_soft),
            TopBarCapsuleKind::InstantDelivery => (p.success_text, p.success_soft),
            TopBarCapsuleKind::PickupOrOtp => (p.teal, p.teal_soft),
            TopBarCapsuleKind::TravelMobility => (p.blue, p.indigo_soft),
            TopBarCapsuleKind::DirectMention => (p.warning, p.warning_soft),
            TopBarCapsuleKind::CareerRecruitment => (p.indigo, p.indigo_soft),
            TopBarCapsuleKind::ScheduleOrAlarm => (p.blue, p.indigo_soft),
            TopBarCapsuleKind::UnreadMessage => (p.accent, p.accent_soft),
            TopBarCapsuleKind::MissedCall => (p.danger, p.danger_soft),
            TopBarCapsuleKind::AmooTask => (p.accent, p.accent_soft),
            TopBarCapsuleKind::PinnedPhone => (p.success_text, p.success_soft),
        }
    }

    pub(crate) fn live_capsule_item_tint(&self, item: &TopBarLiveCapsuleItem) -> (ColorU, ColorU) {
        if item.tag == "stepx_utility_alert" || item.domain_badge == "民生预警" {
            let warn = rgb(0xD9, 0x77, 0x06);
            let warn_soft = rgba(217, 119, 6, 32);
            return (warn, warn_soft);
        }
        self.live_capsule_kind_tint(item.kind)
    }

    pub(crate) fn live_capsule_island_accent(item: &TopBarLiveCapsuleItem) -> ColorU {
        if item.tag == "stepx_utility_alert" || item.domain_badge == "民生预警" {
            return rgb(0xF5, 0x9E, 0x0B);
        }
        match item.kind {
            TopBarCapsuleKind::Timer => rgb(0xF5, 0x9E, 0x0B),
            TopBarCapsuleKind::Pomodoro => rgb(0x81, 0x8C, 0xF8),
            TopBarCapsuleKind::InstantDelivery => rgb(0x34, 0xD3, 0x99),
            TopBarCapsuleKind::PickupOrOtp => rgb(0x2D, 0xD4, 0xBF),
            TopBarCapsuleKind::TravelMobility => rgb(0x38, 0xBD, 0xF8),
            TopBarCapsuleKind::DirectMention => rgb(0xFB, 0xBF, 0x24),
            TopBarCapsuleKind::CareerRecruitment => rgb(0x81, 0x8C, 0xF8),
            TopBarCapsuleKind::ScheduleOrAlarm => rgb(0x60, 0xA5, 0xFA),
            TopBarCapsuleKind::UnreadMessage => rgb(0x38, 0xBD, 0xF8),
            TopBarCapsuleKind::MissedCall => rgb(0xF8, 0x71, 0x71),
            TopBarCapsuleKind::AmooTask => rgb(0xA7, 0x8B, 0xFA),
            TopBarCapsuleKind::PinnedPhone => rgb(0x34, 0xD3, 0x99),
        }
    }

    fn live_capsule_compact_domain(item: &TopBarLiveCapsuleItem) -> String {
        if item.tag == "stepx_utility_alert" {
            return "民生预警".to_string();
        }
        match item.kind {
            TopBarCapsuleKind::InstantDelivery | TopBarCapsuleKind::TravelMobility => {
                let clean_title = item
                    .title
                    .trim_start_matches(|c: char| {
                        !c.is_ascii_alphanumeric() && !('\u{4e00}'..='\u{9fff}').contains(&c)
                    })
                    .trim();
                let short_sub = clean_title
                    .strip_prefix(item.domain_badge.as_str())
                    .map(|r| r.trim_start_matches([' ', '·']).trim())
                    .filter(|r| !r.is_empty() && r.chars().count() <= 5);
                if let Some(sub) = short_sub {
                    format!("{}·{}", Self::truncate_chars(&item.domain_badge, 4), sub)
                } else {
                    Self::truncate_chars(&item.domain_badge, 6)
                }
            }
            TopBarCapsuleKind::PickupOrOtp => {
                if item.domain_badge.contains("验证码") {
                    "验证码".to_string()
                } else if item.pickup_code.is_some() {
                    "快递取件".to_string()
                } else {
                    Self::truncate_chars(&item.domain_badge, 5)
                }
            }
            TopBarCapsuleKind::DirectMention => "有人@我".to_string(),
            TopBarCapsuleKind::CareerRecruitment => "BOSS直聘".to_string(),
            TopBarCapsuleKind::ScheduleOrAlarm => {
                if item.domain_badge.contains("闹钟") {
                    "闹钟提醒".to_string()
                } else {
                    "今日日程".to_string()
                }
            }
            TopBarCapsuleKind::Timer | TopBarCapsuleKind::Pomodoro => {
                Self::truncate_chars(&item.title, 6)
            }
            TopBarCapsuleKind::UnreadMessage => "未读消息".to_string(),
            TopBarCapsuleKind::MissedCall => "未接来电".to_string(),
            TopBarCapsuleKind::AmooTask => "Amoo".to_string(),
            TopBarCapsuleKind::PinnedPhone => "实况".to_string(),
        }
    }

    /// Budget names by their visual width; never cut an actionable code or time.
    fn capsule_short_text(text: &str, units: usize) -> String {
        let width = |c: char| if c.is_ascii() { 1 } else { 2 };
        if text.chars().map(width).sum::<usize>() <= units {
            return text.to_string();
        }
        let mut used = 0;
        let mut out = String::new();
        for c in text.chars() {
            used += width(c);
            if used + 2 > units {
                break;
            }
            out.push(c);
        }
        out.push('…');
        out
    }

    fn live_capsule_text(item: &TopBarLiveCapsuleItem) -> (String, String) {
        let short = Self::capsule_short_text;
        let pill = item.pill_text.trim();
        if item.tag == "stepx_utility_alert" {
            return ("预警".into(), short(pill, 24));
        }
        match item.kind {
            TopBarCapsuleKind::PickupOrOtp => {
                if item.domain_badge.contains("验证码") {
                    return (
                        "验证码".into(),
                        item.copy_value.as_deref().unwrap_or(pill).to_string(),
                    );
                }
                if let Some(code) = &item.pickup_code {
                    return ("取件码".into(), code.trim().to_string());
                }
                (short(&item.domain_badge, 10), short(pill, 20))
            }
            TopBarCapsuleKind::Timer => ("计时".into(), pill.to_string()),
            TopBarCapsuleKind::Pomodoro => ("专注".into(), pill.to_string()),
            TopBarCapsuleKind::InstantDelivery | TopBarCapsuleKind::TravelMobility => {
                let eta = item
                    .metric
                    .trim()
                    .strip_prefix("预计 ")
                    .and_then(|m| m.split_whitespace().next())
                    .filter(|m| m.contains(':'));
                (
                    short(&Self::live_capsule_compact_domain(item), 12),
                    eta.map(str::to_string).unwrap_or_else(|| short(pill, 24)),
                )
            }
            TopBarCapsuleKind::ScheduleOrAlarm => {
                if item.tag == "stepx_schedule" {
                    // Time comes first. The title uses the remaining room.
                    (
                        "日程".into(),
                        format!("{pill} · {}", short(&item.title, 12)),
                    )
                } else {
                    ("闹钟".into(), pill.to_string())
                }
            }
            TopBarCapsuleKind::DirectMention => (
                short(pill, 8),
                short(item.title.trim_start_matches("@提及 · "), 22),
            ),
            TopBarCapsuleKind::UnreadMessage => (
                short(pill, 10),
                short(item.title.trim_start_matches("未读 · "), 22),
            ),
            TopBarCapsuleKind::MissedCall => ("未接来电".into(), short(&item.title, 22)),
            TopBarCapsuleKind::CareerRecruitment => (
                "招聘".into(),
                short(item.title.trim_start_matches("BOSS直聘 · "), 24),
            ),
            TopBarCapsuleKind::AmooTask => ("Amoo".into(), short(&item.metric, 24)),
            TopBarCapsuleKind::PinnedPhone => ("实况".into(), short(pill, 26)),
        }
    }

    pub(crate) fn render_top_bar_live_capsule(&self) -> El {
        use super::capsule_motion::{CapsuleMotion, Visual};
        let capsules = self.collect_top_bar_live_capsules();
        let visuals = capsules
            .iter()
            .enumerate()
            .map(|(_index, item)| {
                let (domain, metric) = Self::live_capsule_text(item);
                Visual {
                    id: item.id.clone(),
                    revision: item.id.starts_with("fulfillment-").then(|| item.pill_text.clone()),
                    icon: item.icon,
                    accent: Self::live_capsule_island_accent(item),
                    domain,
                    metric,
                    live: item.track == CapsuleTrack::LiveActivity,
                    secondary: Vec::new(),
                    total: capsules.len(),
                }
            })
            .collect();
        let popover_open = self.back_screen.live_capsule_popover_open;
        let mut motion = self.capsule_motion.borrow_mut();
        motion.sync(
            visuals,
            !self.animations_disabled(),
            popover_open,
            Instant::now(),
        );
        let presentations = motion.presentations();
        drop(motion);
        if presentations.is_empty() {
            LAST_CAPSULE_PILL_BOUNDS.with(|b| b.set(None));
            return Empty::new().finish();
        }
        let rows = presentations
            .into_iter()
            .map(|visual| {
                let row = self.render_live_capsule_content(&visual);
                (visual.paint_key(), row)
            })
            .collect();
        let control = CapsuleMotion::new(self.capsule_motion.clone(), rows, self.p().dark).finish();
        #[cfg(not(feature = "ui-capture"))]
        let control = SavePosition::new(control, "top-bar-live-capsule")
            .for_single_frame()
            .finish();
        #[cfg(feature = "ui-capture")]
        let control = crate::ui::scenario_check::control("top-bar-live-capsule".into(), control);
        control
    }

    fn render_live_capsule_content(&self, visual: &super::capsule_motion::Visual) -> El {
        let font = self.font;
        let mono_font = self.mono_font;
        let icon = visual.icon;
        let accent = visual.accent;
        let is_live = visual.live;
        let total_count = visual.total;
        let orb_core = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(icons::draw_icon(icon, 11.5, accent))
                    .finish(),
            )
            .with_background_color(ColorU::new(accent.r, accent.g, accent.b, 54))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .finish(),
        )
        .with_width(20.)
        .with_height(20.)
        .finish();

        let left_orb = if is_live {
            let live_dot = ConstrainedBox::new(
                Container::new(Empty::new().finish())
                    .with_background_color(accent)
                    .with_border(Border::all(1.).with_border_color(rgba(15, 23, 42, 235)))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                    .finish(),
            )
            .with_width(6.)
            .with_height(6.)
            .finish();
            ConstrainedBox::new(
                Stack::new()
                    .with_child(orb_core)
                    .with_positioned_child(
                        live_dot,
                        OffsetPositioning::offset_from_parent(
                            vec2f(1., 1.),
                            ParentOffsetBounds::Unbounded,
                            ParentAnchor::BottomRight,
                            ChildAnchor::BottomRight,
                        ),
                    )
                    .finish(),
            )
            .with_width(20.)
            .with_height(20.)
            .finish()
        } else {
            orb_core
        };

        // 2. Concise domain label
        let domain_node = Text::new(visual.domain.clone(), font, 11.5)
            .with_color(rgb(0xF8, 0xFA, 0xFC))
            .with_style(facloud_ui::fonts::Properties::default().weight(Weight::Semibold))
            .soft_wrap(false)
            .finish();

        // 3. Highlighted tinted metric pill
        let pill_badge_el = Container::new(
            Text::new(
                visual.metric.clone(),
                if visual.metric.is_ascii() {
                    mono_font
                } else {
                    font
                },
                10.5,
            )
            .with_color(rgb(0xFF, 0xFF, 0xFF))
            .with_style(facloud_ui::fonts::Properties::default().weight(Weight::Bold))
            .soft_wrap(false)
            .finish(),
        )
        .with_horizontal_padding(6.)
        .with_vertical_padding(2.)
        .with_background_color(ColorU::new(accent.r, accent.g, accent.b, 62))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
        .finish();

        let left_cluster = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_children(vec![left_orb, domain_node, pill_badge_el])
            .finish();

        let mut row_children = vec![left_cluster];

        // 4. Keep other events discoverable without taking room from the key value.
        if total_count > 1 {
            let mut sec_nodes = Vec::with_capacity(visual.secondary.len() + 1);
            for &(sec_icon, sec_accent) in &visual.secondary {
                let sec_orb = ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(icons::draw_icon(sec_icon, 10., sec_accent))
                            .finish(),
                    )
                    .with_background_color(ColorU::new(
                        sec_accent.r,
                        sec_accent.g,
                        sec_accent.b,
                        46,
                    ))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
                    .finish(),
                )
                .with_width(18.)
                .with_height(18.)
                .finish();
                sec_nodes.push(sec_orb);
            }
            if total_count > 1 {
                let count_pill = ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Min)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(
                                Text::new(format!("+{}", total_count - 1), mono_font, 9.5)
                                    .with_color(rgba(241, 245, 249, 235))
                                    .with_style(
                                        facloud_ui::fonts::Properties::default()
                                            .weight(Weight::Bold),
                                    )
                                    .soft_wrap(false)
                                    .finish(),
                            )
                            .finish(),
                    )
                    .with_horizontal_padding(5.)
                    .with_background_color(rgba(255, 255, 255, 28))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(9.)))
                    .finish(),
                )
                .with_height(18.)
                .finish();
                sec_nodes.push(count_pill);
            }
            let sec_cluster = Flex::row()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(3.)
                .with_children(sec_nodes)
                .finish();
            row_children.push(sec_cluster);
        }

        let inner_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_children(row_children)
            .finish();

        inner_row
    }

    /// Interactive Dual-End Live & Important Message Capsule Hub (`id = "top-bar-live-capsule-popover"`).
    pub(crate) fn render_live_capsule_popover(&self) -> El {
        let p = self.p();
        let capsules = self.collect_top_bar_live_capsules();
        let active_count = capsules.len();
        let live_count = capsules
            .iter()
            .filter(|c| c.track == CapsuleTrack::LiveActivity)
            .count();
        let imp_count = capsules
            .iter()
            .filter(|c| c.track == CapsuleTrack::ImportantMessage)
            .count();
        let cur_filter = self.back_screen.live_capsule_filter;

        let count_badge = if active_count > 0 {
            self.pill_badge(format!("{active_count}"), p.accent, p.accent_soft)
        } else {
            self.pill_badge("待命", p.text3, p.tag_bg)
        };

        let filter_row = self.bounded_wrap(vec![
            self.chip(
                "live-capsule-filter-all",
                "全部",
                None,
                vec![Action::BackScreen(BackScreenAction::SetLiveCapsuleFilter(
                    LiveCapsuleFilter::All,
                ))],
                cur_filter == LiveCapsuleFilter::All,
                false,
            ),
            self.chip(
                "live-capsule-filter-live",
                if live_count > 0 {
                    format!("实况 {live_count}")
                } else {
                    "实况".to_string()
                },
                None,
                vec![Action::BackScreen(BackScreenAction::SetLiveCapsuleFilter(
                    LiveCapsuleFilter::LiveOnly,
                ))],
                cur_filter == LiveCapsuleFilter::LiveOnly,
                false,
            ),
            self.chip(
                "live-capsule-filter-important",
                if imp_count > 0 {
                    format!("提醒 {imp_count}")
                } else {
                    "提醒".to_string()
                },
                None,
                vec![Action::BackScreen(BackScreenAction::SetLiveCapsuleFilter(
                    LiveCapsuleFilter::ImportantOnly,
                ))],
                cur_filter == LiveCapsuleFilter::ImportantOnly,
                false,
            ),
        ]);

        let clear_btn = self.chip(
            "live-capsule-clear-phone",
            "清空手机胶囊",
            Some(UiIcon::Trash),
            vec![Action::BackScreen(
                BackScreenAction::ClearAllPhoneLiveCapsules,
            )],
            false,
            false,
        );

        let close_btn = self.icon_action(
            "live-capsule-popover-close",
            "关闭",
            UiIcon::Close,
            vec![Action::BackScreen(
                BackScreenAction::CloseLiveCapsulePopover,
            )],
            false,
        );

        let title_row = self.between(
            self.hstack(
                6.,
                vec![
                    self.ui_icon(UiIcon::Sparkles, 14., p.accent),
                    self.line_w("实况胶囊", SUBHEAD, p.text, Weight::Bold),
                    count_badge,
                ],
            ),
            self.hstack(4., vec![clear_btn, close_btn]),
        );
        let header = Container::new(self.vstack(S2, vec![title_row, filter_row]))
            .with_horizontal_padding(S3)
            .with_vertical_padding(10.)
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish();

        let filtered_capsules: Vec<TopBarLiveCapsuleItem> = capsules
            .into_iter()
            .filter(|c| match cur_filter {
                LiveCapsuleFilter::All => true,
                LiveCapsuleFilter::LiveOnly => c.track == CapsuleTrack::LiveActivity,
                LiveCapsuleFilter::ImportantOnly => c.track == CapsuleTrack::ImportantMessage,
            })
            .collect();

        let mut card_rows: Vec<El> = Vec::new();
        if filtered_capsules.is_empty() {
            card_rows.push(
                Container::new(self.hstack(
                    8.,
                    vec![
                        self.ui_icon(UiIcon::Sparkles, 16., p.text3),
                        self.line(
                            "暂无活跃实况，新配送、取件与提醒将自动汇聚于此",
                            CAPTION,
                            p.text2,
                        ),
                    ],
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                .finish(),
            );
        } else {
            for (_idx, item) in filtered_capsules.into_iter().enumerate() {
                let rem_target = self.reminder_target_for_capsule(&item);
                let (tint, tint_soft) = self.live_capsule_item_tint(&item);
                let clean_item_title = item
                    .title
                    .trim_start_matches(|c: char| {
                        !c.is_ascii_alphanumeric() && !('\u{4e00}'..='\u{9fff}').contains(&c)
                    })
                    .trim();
                let clean_item_title = clean_item_title
                    .strip_prefix(item.domain_badge.as_str())
                    .map(|rest| rest.trim_start_matches([' ', '·']).trim())
                    .filter(|rest| !rest.is_empty())
                    .unwrap_or(clean_item_title);

                // 34x34 circular tinted icon orb
                let icon_orb = ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(self.ui_icon(item.icon, 16., tint))
                            .finish(),
                    )
                    .with_background_color(tint_soft)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(17.)))
                    .finish(),
                )
                .with_width(34.)
                .with_height(34.)
                .finish();

                let pill_display = if !item.pill_text.trim().is_empty() {
                    &item.pill_text
                } else {
                    &item.metric
                };
                let right_pill = self.pill_badge(pill_display, tint, tint_soft);

                let top_line = self.between(
                    Expanded::new(
                        1.,
                        Clipped::new(self.hstack(
                            6.,
                            vec![
                                self.line_w(&item.domain_badge, MICRO, tint, Weight::Semibold),
                                self.line_w(
                                    Self::truncate_chars(clean_item_title, 15),
                                    CAPTION,
                                    p.text,
                                    Weight::Semibold,
                                ),
                            ],
                        ))
                        .finish(),
                    )
                    .finish(),
                    right_pill,
                );

                // At most 2 clean compact action pills (1 primary action, 1 subtle dismiss button)
                let primary_btn = if let Some(ref code) = item.copy_value {
                    self.chip(
                        format!("live-capsule-copy-{}", item.id),
                        "复制",
                        Some(UiIcon::Copy),
                        vec![Action::CopyText(code.clone(), format!("已复制 {}", code))],
                        true,
                        false,
                    )
                } else {
                    let short_primary = match item.primary_label {
                        "查看详情" | "查看短信" | "查看消息" | "查看日程" | "查看通话" => {
                            "查看"
                        }
                        "打开会话" => "打开",
                        "停止计时" | "关闭提醒" => "停止",
                        other => other,
                    };
                    self.chip(
                        format!("live-capsule-primary-{}", item.id),
                        short_primary,
                        None,
                        vec![item.primary_action],
                        true,
                        false,
                    )
                };
                let done_btn = self.icon_action(
                    format!("live-capsule-done-{}", item.id),
                    item.complete_label.trim_start_matches(['✓', ' ']),
                    UiIcon::Check,
                    vec![Action::BackScreen(BackScreenAction::CompleteCapsuleItem {
                        id: item.id.clone(),
                        pickup_code: item.pickup_code.clone(),
                        sms_address: item.sms_address.clone(),
                        im_thread_id: item.im_thread_id.clone(),
                        notif_key: item.notif_key.clone(),
                    })],
                    false,
                );

                #[cfg(feature = "ui-capture")]
                let (primary_btn, done_btn) = {
                    let action = if item.copy_value.is_some() { "copy" } else { "primary" };
                    (
                        crate::ui::scenario_check::control(format!("live-capsule-{action}-{_idx}"), primary_btn),
                        crate::ui::scenario_check::control(format!("live-capsule-done-{_idx}"), done_btn),
                    )
                };

                let detail_txt = if !item.metric.is_empty()
                    && item.metric != item.pill_text
                    && item.metric.contains("预计")
                {
                    format!("{} · {}", item.metric, item.subtitle)
                } else {
                    item.subtitle.clone()
                };

                let bottom_line = self.between(
                    Expanded::new(
                        1.,
                        Clipped::new(self.line(
                            Self::truncate_chars(&detail_txt, 24),
                            MICRO,
                            p.text2,
                        ))
                        .finish(),
                    )
                    .finish(),
                    self.hstack(4., vec![primary_btn, done_btn]),
                );

                let mut center_col = vec![top_line, bottom_line];
                if item.progress_pct > 0 && item.progress_pct < 100 {
                    center_col.push(self.progress(Some(item.progress_pct), tint));
                }

                let row_content = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(10.)
                    .with_child(icon_orb)
                    .with_child(Expanded::new(1., self.vstack(4., center_col)).finish())
                    .finish();

                let card_el = Container::new(row_content)
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(10.)
                    .with_background_color(p.surface)
                    .with_border(
                        Border::all(1.).with_border_color(lerp_color(p.border, tint, 0.28)),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                    .finish();
                let card_el = self.context_target(
                    format!("live-capsule-ctx-{}", item.id),
                    context_menu::Target::HomeReminder(rem_target),
                    card_el,
                );
                #[cfg(feature = "ui-capture")]
                let card_el =
                    crate::ui::scenario_check::control(format!("live-capsule-card-{_idx}"), card_el);
                card_rows.push(card_el);
            }
        }

        let scroll_list = self.pane_scroll(
            self.back_screen.live_capsule_scroll.clone(),
            Container::new(self.vstack(S2, card_rows))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2 + 2.)
                .finish(),
        );
        let body_section = ConstrainedBox::new(scroll_list)
            .with_max_height(340.)
            .finish();

        let footer_bar = Container::new(self.bounded_wrap(vec![
            self.chip(
                "live-capsule-quick-pomodoro",
                "专注 25m",
                Some(UiIcon::Clock),
                vec![Action::BackScreen(BackScreenAction::StartPomodoro(25))],
                true,
                false,
            ),
            self.chip(
                "live-capsule-quick-dnd",
                "勿扰 1h",
                Some(UiIcon::Bell),
                vec![Action::BackScreen(BackScreenAction::PushPhoneLiveCapsule {
                    tag: "stepx_status".into(),
                    pill_text: "会议勿扰".into(),
                    title: "桌面协同 · 会议勿扰".into(),
                    subtitle: "已同步手机顶部实况胶囊与背屏签名".into(),
                    countdown_secs: Some(3600),
                    sync_backscreen: true,
                })],
                false,
                false,
            ),
            self.chip(
                "live-capsule-quick-rest",
                "小憩 15m",
                Some(UiIcon::Coffee),
                vec![Action::BackScreen(BackScreenAction::PushPhoneLiveCapsule {
                    tag: "stepx_status".into(),
                    pill_text: "休息一下".into(),
                    title: "桌面协同 · 小憩休息".into(),
                    subtitle: "手机顶部实况胶囊与背屏点阵同步显示".into(),
                    countdown_secs: Some(900),
                    sync_backscreen: true,
                })],
                false,
                false,
            ),
        ]))
        .with_horizontal_padding(S3)
        .with_vertical_padding(8.)
        .with_background_color(p.surface)
        .with_border(Border::top(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_bottom(Radius::Pixels(11.)))
        .finish();

        let popover = ConstrainedBox::new(
            popover_surface(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(header)
                    .with_child(Shrinkable::new(1., body_section).finish())
                    .with_child(footer_bar)
                    .finish(),
                p,
            ),
        )
        .with_width(420.)
        .finish();

        #[cfg(not(feature = "ui-capture"))]
        let popover = SavePosition::new(popover, "top-bar-live-capsule-popover")
            .for_single_frame()
            .finish();
        #[cfg(feature = "ui-capture")]
        let popover =
            crate::ui::scenario_check::control("top-bar-live-capsule-popover".into(), popover);
        popover
    }

    pub(crate) fn with_live_capsule_popover_overlay(&self, inner: El) -> El {
        if !self.back_screen.live_capsule_popover_open {
            return inner;
        }
        LiveCapsuleOverlayStack {
            base: inner,
            popover: self.render_live_capsule_popover(),
            card_bounds: None,
        }
        .finish()
    }
}

thread_local! {
    static LAST_CAPSULE_PILL_BOUNDS: Cell<Option<pathfinder_geometry::rect::RectF>> =
        const { Cell::new(None) };
}

pub(super) fn set_capsule_pill_bounds(bounds: pathfinder_geometry::rect::RectF) {
    LAST_CAPSULE_PILL_BOUNDS.with(|b| b.set(Some(bounds)));
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum CapsuleTrack {
    LiveActivity,
    ImportantMessage,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum TopBarCapsuleKind {
    Timer,
    Pomodoro,
    InstantDelivery,
    PickupOrOtp,
    TravelMobility,
    DirectMention,
    CareerRecruitment,
    ScheduleOrAlarm,
    UnreadMessage,
    MissedCall,
    AmooTask,
    PinnedPhone,
}

#[derive(Clone, Debug)]
pub(crate) struct TopBarLiveCapsuleItem {
    pub id: String,
    pub tag: String,
    pub icon: UiIcon,
    pub kind: TopBarCapsuleKind,
    pub track: CapsuleTrack,
    pub domain_badge: String,
    pub strategy_tag: String,
    pub score: u8,
    pub pill_text: String,
    pub title: String,
    pub subtitle: String,
    pub metric: String,
    pub source_tag: &'static str,
    pub progress_pct: u8,
    pub countdown_secs: Option<u64>,
    pub short_sig: String,
    pub primary_label: &'static str,
    pub primary_action: Action,
    pub copy_value: Option<String>,
    pub complete_label: &'static str,
    pub pickup_code: Option<String>,
    pub sms_address: Option<String>,
    pub im_thread_id: Option<String>,
    pub notif_key: Option<String>,
}

struct LiveCapsuleOverlayStack {
    base: El,
    popover: El,
    card_bounds: Option<pathfinder_geometry::rect::RectF>,
}

impl facloud_ui::Element for LiveCapsuleOverlayStack {
    fn layout(
        &mut self,
        c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        let sz = self.base.layout(c, ctx, app);
        let available = (sz - vec2f(S3 * 2., S2 + S3)).max(vec2f(0., 0.));
        self.popover
            .layout(SizeConstraint::new(vec2f(0., 0.), available), ctx, app);
        sz
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.base.after_layout(ctx, app);
        self.popover.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        self.base.paint(origin, ctx, app);
        let (Some(base_sz), Some(pop_sz)) = (self.base.size(), self.popover.size()) else {
            return;
        };
        let pill_center_x = LAST_CAPSULE_PILL_BOUNDS
            .with(|c| c.get())
            .map(|r| r.origin().x() + r.width() * 0.5)
            .unwrap_or(origin.x() + base_sz.x() * 0.5);
        let min_x = origin.x() + S3;
        let max_x = (origin.x() + base_sz.x() - pop_sz.x() - S3).max(min_x);
        let pop_x = (pill_center_x - pop_sz.x() * 0.5).clamp(min_x, max_x);
        let pop_origin = vec2f(pop_x, origin.y() + S2);
        self.card_bounds = Some(pathfinder_geometry::rect::RectF::new(pop_origin, pop_sz));

        // The page's scroll areas and cards paint in child layers. The entire
        // popover must start above those layers, including its surface and chrome.
        ctx.scene.start_layer(ClipBounds::ActiveLayer);
        self.popover.paint(pop_origin, ctx, app);
        ctx.scene.stop_layer();
    }

    fn size(&self) -> Option<Vector2F> {
        self.base.size()
    }

    fn origin(&self) -> Option<Point> {
        self.base.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        if let Event::KeyDown { keystroke, .. } = event.raw_event() {
            if keystroke.key.eq_ignore_ascii_case("escape") {
                ctx.dispatch_typed_action(Action::BackScreen(
                    BackScreenAction::CloseLiveCapsulePopover,
                ));
                return true;
            }
        }

        let pointer_pos = match event.raw_event() {
            Event::ScrollWheel { position, .. }
            | Event::MouseMoved { position, .. }
            | Event::LeftMouseDown { position, .. }
            | Event::LeftMouseUp { position, .. }
            | Event::LeftMouseDragged { position, .. }
            | Event::RightMouseDown { position, .. } => Some(*position),
            _ => None,
        };

        if let Some(pos) = pointer_pos {
            if self.card_bounds.is_some_and(|rect| rect.contains_point(pos)) {
                let _ = self.popover.dispatch_event(event, ctx, app);
                return true;
            }
            match event.raw_event() {
                Event::ScrollWheel { .. } => return true,
                Event::LeftMouseDown { .. } | Event::RightMouseDown { .. } => {
                    ctx.dispatch_typed_action(Action::BackScreen(
                        BackScreenAction::CloseLiveCapsulePopover,
                    ));
                    return true;
                }
                _ => {
                    let _ = self.popover.dispatch_event(event, ctx, app);
                    return true;
                }
            }
        }

        if self.popover.dispatch_event(event, ctx, app) {
            return true;
        }
        self.base.dispatch_event(event, ctx, app)
    }
}

impl Companion {
    pub(crate) fn reminder_target_for_capsule(
        &self,
        item: &TopBarLiveCapsuleItem,
    ) -> pages::home::HomeReminderContextTarget {
        use stepx_companion::home_reminder_governance::{
            canonical_keys_for_code_or_parcel, canonical_keys_for_delivery,
            canonical_keys_for_im_thread, ReminderCategory, ReminderTargetSpec,
        };
        let fulfillment = item.id.starts_with("fulfillment-");
        let category = match item.kind {
            TopBarCapsuleKind::Timer
            | TopBarCapsuleKind::Pomodoro
            | TopBarCapsuleKind::PinnedPhone => ReminderCategory::LiveTimer,
            TopBarCapsuleKind::InstantDelivery | TopBarCapsuleKind::TravelMobility => {
                ReminderCategory::TakeoutAndMobility
            }
            TopBarCapsuleKind::PickupOrOtp => {
                if fulfillment {
                    ReminderCategory::ExpressInTransit
                } else if item.domain_badge.contains("验证码") || item.id.starts_with("otp-") {
                    ReminderCategory::OtpCode
                } else {
                    ReminderCategory::ExpressPickup
                }
            }
            TopBarCapsuleKind::DirectMention
            | TopBarCapsuleKind::CareerRecruitment
            | TopBarCapsuleKind::UnreadMessage
            | TopBarCapsuleKind::MissedCall => ReminderCategory::UnreadMessage,
            TopBarCapsuleKind::ScheduleOrAlarm => ReminderCategory::ScheduleAndMeeting,
            TopBarCapsuleKind::AmooTask => ReminderCategory::LedgerAndTaskIssue,
        };
        let mut keys = vec![format!("capsule:{}", item.id)];
        if !fulfillment && matches!(
            item.kind,
            TopBarCapsuleKind::InstantDelivery | TopBarCapsuleKind::TravelMobility
        ) {
            keys.extend(canonical_keys_for_delivery(
                &item.domain_badge,
                item.notif_key.as_deref(),
                item.im_thread_id.as_deref(),
                item.sms_address.as_deref(),
            ));
        }
        if let Some(ref code) = item.pickup_code {
            keys.extend(canonical_keys_for_code_or_parcel(
                Some(code),
                None,
                item.notif_key.as_deref(),
                item.sms_address.as_deref(),
            ));
        }
        if !fulfillment {
            if let Some(ref tid) = item.im_thread_id {
                keys.extend(canonical_keys_for_im_thread(tid, item.notif_key.as_deref()));
            }
            if let Some(ref nk) = item.notif_key {
                keys.push(format!("notif:{nk}"));
            }
        }
        if let Some(ev_str) = item.id.strip_prefix("cal-") {
            if let Ok(ev_id) = ev_str.parse::<i64>() {
                keys.push(format!("cal:{ev_id}"));
            }
        }
        let primary_key = keys
            .iter()
            .find(|k| !k.starts_with("capsule:"))
            .cloned()
            .unwrap_or_else(|| format!("capsule:{}", item.id));
        let (mute_source_key, mute_source_label) = if fulfillment || matches!(
            item.kind,
            TopBarCapsuleKind::InstantDelivery | TopBarCapsuleKind::TravelMobility
        ) {
            (
                format!("delivery:{}", item.domain_badge),
                item.domain_badge.clone(),
            )
        } else {
            (String::new(), String::new())
        };
        pages::home::HomeReminderContextTarget {
            spec: ReminderTargetSpec::new(primary_key, category, item.title.clone())
                .with_aliases(keys)
                .with_subtitle(item.subtitle.clone())
                .with_source(
                    item.domain_badge.clone(),
                    (!mute_source_key.is_empty()).then(|| mute_source_key.clone()),
                ),
            primary_action: Some((
                item.primary_label.to_string(),
                Box::new(item.primary_action.clone()),
            )),
            copy_text: format!("{} · {}", item.title, item.subtitle),
            amoo_prompt: format!(
                "请帮我跟进处理这条{}提醒：{}（{}）",
                item.domain_badge, item.title, item.subtitle
            ),
            otp_or_pickup_code: item.pickup_code.clone().unwrap_or_default(),
            sms_address: item.sms_address.clone().unwrap_or_default(),
            im_thread_id: item.im_thread_id.clone().unwrap_or_default(),
            notif_key: item.notif_key.clone().unwrap_or_default(),
            calendar_event_id: item
                .id
                .strip_prefix("cal-")
                .and_then(|s| s.parse::<i64>().ok()),
            capsule_id: item.id.clone(),
            mute_source_key,
            mute_source_label,
        }
    }

    pub fn complete_capsule_item(
        &mut self,
        id: &str,
        pickup_code: Option<&str>,
        sms_address: Option<&str>,
        im_thread_id: Option<&str>,
        notif_key: Option<&str>,
    ) {
        use stepx_companion::home_reminder_governance::{
            canonical_keys_for_code_or_parcel, canonical_keys_for_delivery,
            canonical_keys_for_im_thread, ReminderCategory, ReminderTargetSpec,
        };
        if id.starts_with("fulfillment-") {
            self.acknowledge_fulfillment_id(id);
            return;
        }
        let now_ms = chrono::Local::now().timestamp_millis();
        self.back_screen
            .dismissed_capsule_ids
            .insert(id.to_string());
        self.notification_center.read_ids.insert(id.to_string());
        let mut keys = vec![format!("capsule:{id}")];
        let mut category = ReminderCategory::LedgerAndTaskIssue;
        if let Some(badge) = id
            .strip_prefix("delivery-sms-")
            .or_else(|| id.strip_prefix("delivery-notif-"))
            .or_else(|| id.strip_prefix("delivery-thread-"))
        {
            category = ReminderCategory::TakeoutAndMobility;
            keys.extend(canonical_keys_for_delivery(
                badge,
                notif_key,
                im_thread_id,
                sms_address,
            ));
        } else if id.starts_with("otp-") {
            category = ReminderCategory::OtpCode;
            if let Some(code) = id.strip_prefix("otp-code-") {
                self.notification_center
                    .read_ids
                    .insert(format!("otp:{}", code.trim()));
            }
        } else if id.starts_with("pickup-") {
            category = ReminderCategory::ExpressPickup;
        } else if id.starts_with("cal-") {
            category = ReminderCategory::ScheduleAndMeeting;
            if let Some(Ok(ev_id)) = id.strip_prefix("cal-").map(|s| s.parse::<i64>()) {
                keys.push(format!("cal:{ev_id}"));
                self.notification_center
                    .read_ids
                    .insert(format!("calendar:{ev_id}"));
            }
        } else if id.starts_with("alarm-") {
            category = ReminderCategory::ScheduleAndMeeting;
            if let Some(Ok(alarm_id)) = id.strip_prefix("alarm-").map(|s| s.parse::<i64>()) {
                keys.push(format!("alarm:{alarm_id}"));
                self.notification_center
                    .read_ids
                    .insert(format!("alarm:{alarm_id}"));
            }
        } else if id.starts_with("utility-alert-") {
            category = ReminderCategory::WeatherAndLife;
        } else if id.starts_with("mention-")
            || id.starts_with("career-")
            || id == "unread-summary"
            || id.starts_with("missed-call:")
        {
            category = ReminderCategory::UnreadMessage;
        } else if id.contains("timer") || id.contains("pomodoro") || id.starts_with("pinned-") {
            category = ReminderCategory::LiveTimer;
        }
        if let Some(code) = pickup_code.filter(|s| !s.trim().is_empty()) {
            keys.extend(canonical_keys_for_code_or_parcel(
                Some(code),
                None,
                notif_key,
                sms_address,
            ));
            let norm = code.trim().to_uppercase();
            if self.push.prefs.dismissed_pickup_codes.insert(norm) {
                let _ = self.push.prefs.save();
            }
            self.sync_live_express_parcels();
        }
        if let Some(addr) = sms_address.filter(|s| !s.trim().is_empty()) {
            self.mark_sms_thread_read(addr);
        }
        if let Some(tid) = im_thread_id.filter(|s| !s.trim().is_empty()) {
            keys.extend(canonical_keys_for_im_thread(tid, notif_key));
            if let Some(thread) = self
                .push
                .snapshot
                .threads
                .iter_mut()
                .find(|t| t.thread_id == tid)
            {
                thread.unread_count = 0;
            }
            for n in &mut self.push.snapshot.notifications {
                if n.thread_id == tid {
                    n.read = true;
                }
            }
        }
        if let Some(k) = notif_key.filter(|s| !s.trim().is_empty()) {
            keys.push(format!("notif:{k}"));
            if let Some(n) = self
                .push
                .snapshot
                .notifications
                .iter_mut()
                .find(|n| n.key == k)
            {
                n.read = true;
                n.dismissed = true;
                n.active = false;
            }
        }
        let primary_key = keys
            .iter()
            .find(|k| !k.starts_with("capsule:"))
            .cloned()
            .unwrap_or_else(|| format!("capsule:{id}"));
        self.reminder_governance.manual_expire(
            ReminderTargetSpec::new(primary_key, category, format!("实况胶囊 · {id}"))
                .with_aliases(keys)
                .with_subtitle("已从胶囊条标记完成")
                .with_source(category.label(), None)
                .with_source_ts(now_ms),
            now_ms,
        );
        self.sync_notification_center();
        self.status = "已完成并归档该胶囊事项".into();
        self.error = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recurring_calendar_capsules_acknowledge_only_the_exact_occurrence() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        let start = chrono::Local::now().timestamp_millis() + 3_600_000;
        let first = stepx_companion::toolbox::PhoneCalendarEvent {
            id: 71, title: "例行项目周会".into(), dtstart: start,
            dtend: start + 3_600_000, recurring: Some(true), ..Default::default()
        };
        app.back_screen.stepos_suite.calendar_events = vec![first.clone(),
            stepx_companion::toolbox::PhoneCalendarEvent {
                dtstart: start + 7 * 86_400_000, dtend: start + 7 * 86_400_000 + 3_600_000,
                ..first.clone()
            }];
        // All legacy series-wide markers remain stored, but cannot hide a new occurrence.
        app.back_screen.completed_calendar_events.insert(71);
        app.back_screen.dismissed_capsule_ids.insert("cal-71".into());
        app.notification_center.read_ids.insert("calendar:71".into());
        let current = app.collect_top_bar_live_capsules().into_iter()
            .find(|item| item.id.starts_with("cal-71:")).unwrap();
        assert_eq!(current.id, format!("cal-71:{start}"));
        assert!(matches!(current.primary_action,
            Action::BackScreen(BackScreenAction::CalendarOpenOccurrence(71, at)) if at == start));
        app.complete_capsule_item(&current.id, None, None, None, None);
        let next = app.collect_top_bar_live_capsules().into_iter()
            .find(|item| item.id.starts_with("cal-71:")).unwrap();
        assert_eq!(next.id, format!("cal-71:{}", start + 7 * 86_400_000));
        assert!(app.back_screen.completed_calendar_events.contains(&71));
    }

    #[test]
    fn capsule_parcel_receipt_removes_sms_and_pinned_fallbacks() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = Companion::new_for_test();
        let now = chrono::Local::now().timestamp_millis();
        app.sms = vec![stepx_companion::sms::Message {
            provider_id: None,
            address: "10690001".into(),
            body: "丰巢取件码31744725，请及时取件".into(),
            date_ms: now - 60_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];
        app.sync_live_express_parcels();
        let capsule = app
            .collect_top_bar_live_capsules()
            .into_iter()
            .find(|c| c.pickup_code.as_deref() == Some("31744725"))
            .unwrap();
        assert_eq!(
            Companion::live_capsule_text(&capsule),
            ("取件码".into(), "31744725".into())
        );
        app.back_screen.active_phone_capsule = Some((
            "stepx_pickup".into(),
            "31744725".into(),
            "取件码".into(),
            Instant::now() - Duration::from_secs(30),
        ));
        app.push
            .snapshot
            .notifications
            .push(stepx_companion::push::PushNotification {
                key: "parcel-terminal".into(),
                app_name: "丰巢".into(),
                text: "取件码31744725的包裹已取出".into(),
                post_time_ms: now,
                read: true,
                dismissed: true,
                ..Default::default()
            });
        app.sync_live_express_parcels();
        assert!(app.express_parcels.is_empty());
        assert!(app.extract_recent_sms_pickup_code().is_none());
        assert!(!app
            .collect_top_bar_live_capsules()
            .iter()
            .any(|c| c.tag == "stepx_pickup"));
    }

    #[test]
    fn capsule_text_preserves_codes_times_and_contact_context() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.init_demo_push();
        let mut item = app
            .collect_top_bar_live_capsules()
            .into_iter()
            .next()
            .unwrap();
        item.tag = "stepx_pickup".into();
        item.kind = TopBarCapsuleKind::PickupOrOtp;
        item.domain_badge = "快递取件".into();
        item.pickup_code = Some("12-34-567890".into());
        item.pill_text = "12-34-5…".into();
        assert_eq!(Companion::live_capsule_text(&item).1, "12-34-567890");
        item.domain_badge = "验证码".into();
        item.copy_value = Some("12345678".into());
        assert_eq!(Companion::live_capsule_text(&item).1, "12345678");
        item.kind = TopBarCapsuleKind::InstantDelivery;
        item.metric = "预计 20:30~20:50 送达".into();
        assert_eq!(Companion::live_capsule_text(&item).1, "20:30~20:50");
        item.kind = TopBarCapsuleKind::MissedCall;
        item.title = "13800138000".into();
        assert_eq!(Companion::live_capsule_text(&item).1, "13800138000");
        item.kind = TopBarCapsuleKind::ScheduleOrAlarm;
        item.tag = "stepx_schedule".into();
        item.pill_text = "13:30".into();
        item.title = "产品与桌面协同评审会".into();
        assert_eq!(Companion::live_capsule_text(&item).1, "13:30 · 产品与桌面…");
    }

    #[test]
    fn top_bar_live_capsule_priority_and_popover_render_cleanly() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.init_demo_push();

        // 1. Default demo push + suite has pickup/OTP/schedule capsules
        let initial_capsules = app.collect_top_bar_live_capsules();
        assert!(!initial_capsules.is_empty());
        let _pill = app.render_top_bar_live_capsule();
        let _popover = app.render_live_capsule_popover();

        // 2. Starting desktop Pomodoro puts Pomodoro ahead of pickup/schedule
        app.back_screen.pomodoro_active = true;
        app.back_screen.pomodoro_duration_secs = 25 * 60;
        app.back_screen.pomodoro_started_at = Some(Instant::now());
        let with_pomo = app.collect_top_bar_live_capsules();
        assert_eq!(with_pomo[0].kind, TopBarCapsuleKind::Pomodoro);
        assert!(with_pomo[0].title.contains("番茄专注"));

        // 3. Adding an active phone countdown timer takes top priority (#1)
        let now_ms = chrono::Local::now().timestamp_millis();
        app.push
            .snapshot
            .notifications
            .push(stepx_companion::push::PushNotification {
                key: "0|com.android.deskclock|1|timer|1000".into(),
                package_name: "com.android.deskclock".into(),
                app_name: "时钟".into(),
                title: "30分钟计时".into(),
                text: "还剩 29 分钟".into(),
                post_time_ms: now_ms - 60_000,
                is_ongoing: true,
                ..Default::default()
            });
        let with_timer = app.collect_top_bar_live_capsules();
        assert_eq!(with_timer[0].kind, TopBarCapsuleKind::Timer);
        assert!(with_timer[0].title.contains("30分钟计时"));
        assert!(with_timer[0].progress_pct > 0);

        // 4. Toggle / Close popover state and render overlay
        app.back_screen.live_capsule_popover_open = true;
        let _wrapped_home = app.page_content();
        let _header = app.header();
    }

    #[test]
    fn fresh_unread_sms_and_sidebar_badges_ignore_stale_and_viewed_threads() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = Companion::new_for_test();
        let now_ms = chrono::Local::now().timestamp_millis();

        // 1 stale unread message (24h old) and 2 fresh unread messages in the same thread + 1 fresh in another thread
        app.apply_sms_data(Ok(vec![
            Message {
                provider_id: Some(1),
                address: "10690001".into(),
                body: "昨天的营销短信".into(),
                date_ms: now_ms - 24 * 3600 * 1000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
            Message {
                provider_id: Some(2),
                address: "+8613800138000".into(),
                body: "第一条新消息".into(),
                date_ms: now_ms - 60_000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
            Message {
                provider_id: Some(3),
                address: "13800138000".into(),
                body: "同一会话第二条新消息".into(),
                date_ms: now_ms - 30_000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
            Message {
                provider_id: Some(4),
                address: "95555".into(),
                body: "银行通知".into(),
                date_ms: now_ms - 10_000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
        ]));

        // Stale thread is marked read; 2 distinct fresh threads remain
        assert_eq!(app.fresh_unread_sms_thread_count(), 2);

        // Selecting 13800138000 records it in read_sms_addresses and persists across re-polls
        app.selected_thread = Some("13800138000".into());
        app.mark_sms_thread_read("13800138000");
        assert_eq!(app.fresh_unread_sms_thread_count(), 1);

        // Re-applying fresh ADB poll with read=false on 13800138000 keeps it read
        app.selected_thread = Some("95555".into());
        app.mark_sms_thread_read("95555");
        app.apply_sms_data(Ok(vec![
            Message {
                provider_id: Some(2),
                address: "+8613800138000".into(),
                body: "第一条新消息".into(),
                date_ms: now_ms - 60_000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
            Message {
                provider_id: Some(4),
                address: "95555".into(),
                body: "银行通知".into(),
                date_ms: now_ms - 10_000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
        ]));
        assert_eq!(app.fresh_unread_sms_thread_count(), 0);
        let _sidebar = app.sidebar();
    }

    #[test]
    fn strategy_ai_capsules_cover_dingdong_express_mention_boss_schedule_unread_and_complete() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = Companion::new_for_test();
        let now_ms = chrono::Local::now().timestamp_millis();

        // 1. Seed DingDong fresh grocery SMS + Cainiao express pickup SMS + unread SMS
        app.apply_sms_data(Ok(vec![
            Message {
                provider_id: Some(101),
                address: "106839909638768323".into(),
                body: "【叮咚买菜】您尾号8350的订单已代收（家门口），如果您有疑问，请随时联系骑手"
                    .into(),
                date_ms: now_ms - 45_000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
            Message {
                provider_id: Some(102),
                address: "106881517503453".into(),
                body: "【菜鸟驿站】您的包裹已到站，请凭取件码 4-2-1086 到阳光花园2栋菜鸟驿站领取。"
                    .into(),
                date_ms: now_ms - 120_000,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
        ]));
        app.sync_live_express_parcels();

        // 2. Seed WeChat direct @ mention + BOSS直聘 thread
        app.push.snapshot.threads = vec![
            stepx_companion::push::ImThread {
                thread_id: "wx_group_stepx_harbor".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "STEPX星辰停泊港".into(),
                last_message: "[有人@我] 窝-StepX: @倒影颠倒\u{2005}老大最近有啥新科研吗".into(),
                last_sender: "窝-StepX".into(),
                last_timestamp_ms: now_ms - 30_000,
                unread_count: 2,
                is_group: true,
                can_remote_reply: true,
                can_ui_reply: true,
                ..Default::default()
            },
            stepx_companion::push::ImThread {
                thread_id: "boss_hr_zhang".into(),
                package_name: "com.hpbr.bosszhipin".into(),
                app_name: "BOSS直聘".into(),
                title: "张经理 · 字节跳动 HR".into(),
                last_message: "请问今天下午方便聊一下架构师岗位吗？".into(),
                last_sender: "张经理".into(),
                last_timestamp_ms: now_ms - 90_000,
                unread_count: 1,
                is_group: false,
                can_remote_reply: true,
                can_ui_reply: true,
                ..Default::default()
            },
        ];

        // 3. An ongoing schedule stays on today's date even within 25 minutes of midnight.
        app.back_screen.stepos_suite.calendar_events =
            vec![stepx_companion::toolbox::PhoneCalendarEvent {
                recurring: Some(false),
                id: 1001,
                title: "第四季度产品与桌面协同评审会".into(),
                dtstart: now_ms,
                dtend: now_ms + 85 * 60_000,
                location: "StepX 3F 星辰会议室".into(),
                description: "评审策略+AI双轨胶囊引擎".into(),
                all_day: false,
                source: "StepX Calendar".into(),
            }];

        let capsules = app.collect_top_bar_live_capsules();

        // Verify DingDong grocery delivery capsule
        let dingdong = capsules
            .iter()
            .find(|c| c.kind == TopBarCapsuleKind::InstantDelivery && c.domain_badge == "叮咚买菜")
            .expect("Expected DingDong InstantDelivery capsule");
        assert_eq!(dingdong.complete_label, "已知晓");
        assert!(dingdong.title.contains("叮咚买菜"));

        // Verify WeChat direct @ mention capsule
        let mention = capsules
            .iter()
            .find(|c| c.kind == TopBarCapsuleKind::DirectMention)
            .expect("Expected WeChat DirectMention capsule");
        assert_eq!(mention.domain_badge, "有人@我");
        assert!(mention.subtitle.contains("老大最近有啥新科研吗"));

        // Verify BOSS直聘 recruitment capsule
        let boss = capsules
            .iter()
            .find(|c| c.kind == TopBarCapsuleKind::CareerRecruitment)
            .expect("Expected BOSS直聘 CareerRecruitment capsule");
        assert_eq!(boss.domain_badge, "BOSS直聘");
        assert!(boss.title.contains("张经理"));

        // Verify Express parcel / SMS pickup code capsule
        let express = capsules
            .iter()
            .find(|c| c.kind == TopBarCapsuleKind::PickupOrOtp)
            .expect("Expected Express PickupOrOtp capsule");
        assert!(express.metric.contains("4-2-1086") || express.title.contains("4-2-1086"));

        // Verify Unread message summary capsule
        let unread = capsules
            .iter()
            .find(|c| c.kind == TopBarCapsuleKind::UnreadMessage)
            .expect("Expected UnreadMessage capsule");
        assert_eq!(unread.domain_badge, "消息未读");

        // Verify Calendar schedule capsule
        let schedule = capsules
            .iter()
            .find(|c| c.kind == TopBarCapsuleKind::ScheduleOrAlarm)
            .expect("Expected ScheduleOrAlarm capsule");
        assert_eq!(schedule.domain_badge, "今日日程");
        assert!(schedule.title.contains("第四季度产品与桌面协同评审会"));

        // Verify completing DingDong capsule removes it from collect_top_bar_live_capsules()
        let dingdong_id = dingdong.id.clone();
        app.complete_capsule_item(
            &dingdong.id,
            dingdong.pickup_code.as_deref(),
            dingdong.sms_address.as_deref(),
            dingdong.im_thread_id.as_deref(),
            dingdong.notif_key.as_deref(),
        );

        let after_complete = app.collect_top_bar_live_capsules();
        assert!(
            !after_complete.iter().any(|c| c.id == dingdong_id),
            "Completed DingDong capsule should be removed from collect_top_bar_live_capsules()"
        );
        for ordinary in ["@所有人 今天开会", "@其他成员 请看一下"] {
            app.push
                .snapshot
                .threads
                .iter_mut()
                .filter(|t| t.package_name == "com.tencent.mm")
                .for_each(|t| t.last_message = ordinary.into());
            assert!(
                !app.collect_top_bar_live_capsules()
                    .iter()
                    .any(|c| c.kind == TopBarCapsuleKind::DirectMention),
                "{ordinary}"
            );
        }
    }

    #[test]
    fn live_capsules_coexist_calendar_and_alarm_include_missed_call_and_respect_silent_channels() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        let now_ms = chrono::Local::now().timestamp_millis();
        let now_local = chrono::Local::now();
        // Capsules show today's alarms. Keep the fixture on the same date even
        // when the suite runs after 23:50, instead of accidentally testing tomorrow.
        let alarm_dt = chrono::Timelike::with_hour(&now_local, 23)
            .and_then(|dt| chrono::Timelike::with_minute(&dt, 59))
            .expect("valid end-of-day time");

        app.back_screen.stepos_suite.calendar_events =
            vec![stepx_companion::toolbox::PhoneCalendarEvent {
                recurring: Some(false),
                id: 2001,
                title: "架构同步会".into(),
                dtstart: now_ms + 15 * 60_000,
                dtend: now_ms + 45 * 60_000,
                location: "A302".into(),
                description: String::new(),
                all_day: false,
                source: "StepX Calendar".into(),
            }];
        app.back_screen.stepos_suite.alarms = vec![stepx_companion::toolbox::PhoneAlarmItem {
            id: 3001,
            hour: chrono::Timelike::hour(&alarm_dt) as u8,
            minutes: chrono::Timelike::minute(&alarm_dt) as u8,
            enabled: true,
            days_of_week: 0x7f,
            vibrate: true,
            message: "出发去机场".into(),
        }];
        app.notification_center.items.push(
            super::notification_center_state::UnifiedNotificationItem {
                id: "missed-call:13800138000:100".into(),
                package_name: "com.android.server.telecom".into(),
                visual: super::notification_center_state::NotificationVisualKind::MissedCall,
                source_badge: "未接来电".into(),
                title: "李明 · 产品组".into(),
                summary: "138 0013 8000 · 点击快速回短信".into(),
                unread: true,
                ..Default::default()
            },
        );

        let capsules = app.collect_top_bar_live_capsules();
        assert!(
            capsules.iter().any(|c| c.id == "cal-2001"),
            "Expected calendar capsule"
        );
        assert!(
            capsules.iter().any(|c| c.id == "alarm-3001"),
            "Expected alarm capsule alongside calendar capsule"
        );
        assert!(
            capsules
                .iter()
                .any(|c| c.id == "missed-call:13800138000:100" && c.domain_badge == "未接来电"),
            "Expected missed call capsule"
        );

        // Suppressing calendar, deskclock, and dialer via PopupMode::Silent removes them from live capsules
        app.push.prefs.set_app_popup_mode(
            "com.android.calendar",
            Some(stepx_companion::desktop_notify::PopupMode::Silent),
        );
        app.push.prefs.set_app_popup_mode(
            "com.android.deskclock",
            Some(stepx_companion::desktop_notify::PopupMode::Silent),
        );
        app.push.prefs.set_app_popup_mode(
            "com.android.dialer",
            Some(stepx_companion::desktop_notify::PopupMode::Silent),
        );
        let after_silent = app.collect_top_bar_live_capsules();
        assert!(!after_silent.iter().any(|c| c.id == "cal-2001"));
        assert!(!after_silent.iter().any(|c| c.id == "alarm-3001"));
        assert!(!after_silent
            .iter()
            .any(|c| c.id == "missed-call:13800138000:100"));
    }

    #[test]
    fn dianping_eta_jd_express_and_power_outage_form_active_capsules_even_when_read() {
        let _guard = TEST_CONFIG_DIR_LOCK
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let mut app = Companion::new_for_test();
        let now_local = chrono::Local::now();
        let now_ms = now_local.timestamp_millis();
        let start_dt = now_local + chrono::Duration::minutes(20);
        let end_dt = now_local + chrono::Duration::minutes(40);
        let range_str = format!(
            "{:02}:{:02}~{:02}:{:02}",
            chrono::Timelike::hour(&start_dt),
            chrono::Timelike::minute(&start_dt),
            chrono::Timelike::hour(&end_dt),
            chrono::Timelike::minute(&end_dt)
        );
        let expected_end_hm = format!(
            "{:02}:{:02}送达",
            chrono::Timelike::hour(&end_dt),
            chrono::Timelike::minute(&end_dt)
        );

        // 1. Dianping order accepted notification & thread (already viewed: read = true, unread_count = 0)
        let dianping_text = format!("您的订单预计今天 {range_str}送达，请您耐心等待。");
        app.push.snapshot.notifications = vec![
            stepx_companion::push::PushNotification {
                key: "0|com.tencent.mm|101|dianping|1".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "大众点评".into(),
                text: format!("商家已接单：{dianping_text}"),
                post_time_ms: now_ms - 180_000,
                active: false,
                read: true,
                dismissed: false,
                ..Default::default()
            },
            stepx_companion::push::PushNotification {
                key: "0|com.jingdong.app.mall|102|express|1".into(),
                package_name: "com.jingdong.app.mall".into(),
                app_name: "京东".into(),
                title: "物流发出提醒".into(),
                text: "您购买的商品已由京东物流揽收发出，运单号 JD1234567890123".into(),
                post_time_ms: now_ms - 300_000,
                active: true,
                read: true,
                dismissed: false,
                ..Default::default()
            },
        ];
        app.push.snapshot.threads = vec![
            stepx_companion::push::ImThread {
                thread_id: "wx_dianping_service".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "大众点评".into(),
                last_message: format!("商家已接单 · {dianping_text}"),
                last_timestamp_ms: now_ms - 180_000,
                unread_count: 0,
                ..Default::default()
            },
            stepx_companion::push::ImThread {
                thread_id: "wx_community_grid".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "阳光物业服务中心".into(),
                last_message: "【停电通知】因本小区10V开万甲线计划检修，今日14:00-18:00临时停电，请提前做好准备。".into(),
                last_timestamp_ms: now_ms - 600_000,
                unread_count: 0,
                ..Default::default()
            },
        ];
        app.sync_live_express_parcels();
        // Simulate user having viewed the JD express item in notification center
        app.notification_center
            .read_ids
            .insert("express:JD1234567890123:".into());

        let capsules = app.collect_top_bar_live_capsules();

        // Verify Dianping delivery capsule is retained with explicit wall-clock ETA pill
        let dianping_cap = capsules
            .iter()
            .find(|c| c.domain_badge == "大众点评")
            .expect("Expected Dianping delivery capsule even when read");
        assert_eq!(dianping_cap.pill_text, expected_end_hm);
        assert!(dianping_cap.metric.contains(&range_str));
        // The ETA is a forecast, not a measured completion percentage.
        assert_eq!(dianping_cap.progress_pct, 0);

        // Verify JD in-transit express capsule is retained even when marked read
        let jd_cap = capsules
            .iter()
            .find(|c| c.kind == TopBarCapsuleKind::PickupOrOtp && c.title.contains("京东"))
            .expect("Expected JD in-transit express capsule even when read");
        assert_eq!(jd_cap.pill_text, "已发货");

        // Verify Public Utility / Power Outage notice capsule is formed
        let utility_cap = capsules
            .iter()
            .find(|c| c.tag == "stepx_utility_alert" && c.pill_text == "停电通知")
            .expect("Expected power outage public utility capsule");
        assert_eq!(utility_cap.domain_badge, "民生预警");
        assert_eq!(utility_cap.metric, "今日社区提醒");

        // Render both top-center pill and popover cleanly
        let _pill = app.render_top_bar_live_capsule();
        let _popover = app.render_live_capsule_popover();
    }
}
