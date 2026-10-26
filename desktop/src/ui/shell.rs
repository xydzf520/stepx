//! Window chrome: sidebar, page header, status bar and shared page states.

use super::{
    icons::{self, UiIcon},
    theme::*,
    widgets::*,
    *,
};
use facloud_ui::{color::ColorU, fonts::Weight, platform::Cursor};

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
                format!("{model} (USB)")
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
        .with_padding_top(if first { 0. } else { 10. })
        .with_padding_bottom(4.)
        .finish()
    }

    pub(super) fn nav_item_badge(
        &self,
        id: &str,
        icon: &'static str,
        label: &'static str,
        badge: Option<String>,
        active: bool,
        action: Action,
    ) -> El {
        let resolved_icon = if icon == "✦" {
            UiIcon::Star
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
        badge: Option<String>,
    ) -> El {
        self.nav_item_icon(id, icon, label, badge, is_active, Action::Page(target))
    }

    pub(super) fn nav_item_icon(
        &self,
        id: &str,
        icon: UiIcon,
        label: &'static str,
        badge: Option<String>,
        active: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("nav:{id}"));
        let font = self.font;
        let mono_font = self.mono_font;
        Hoverable::new(state, move |mouse| {
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
            let row = match &badge {
                Some(txt) => {
                    let (badge_bg, badge_fg) = if active {
                        if p.dark {
                            (rgba(0xFF, 0xFF, 0xFF, 38), ColorU::white())
                        } else {
                            (rgb(0xF4, 0xF4, 0xF5), rgb(0x18, 0x18, 0x1B))
                        }
                    } else if p.dark {
                        (rgba(0xFF, 0xFF, 0xFF, 14), rgb(0x71, 0x71, 0x7A))
                    } else {
                        (rgba(0x00, 0x00, 0x00, 13), rgb(0x71, 0x71, 0x7A))
                    };
                    let badge_node = Container::new(
                        facloud_ui::elements::Text::new(txt.clone(), mono_font, 10.)
                            .with_color(badge_fg)
                            .with_style(
                                facloud_ui::fonts::Properties::default().weight(Weight::Medium),
                            )
                            .soft_wrap(false)
                            .finish(),
                    )
                    .with_horizontal_padding(6.)
                    .with_vertical_padding(1.)
                    .with_background_color(badge_bg)
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
                None => left,
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
        .finish()
    }

    pub(super) fn cached_device_metrics(&self) -> stepx_companion::device::DeviceMetrics {
        thread_local! {
            static CACHE: std::cell::RefCell<Option<(String, stepx_companion::device::DeviceMetrics)>> =
                const { std::cell::RefCell::new(None) };
        }
        CACHE.with(|cache| {
            let mut borrow = cache.borrow_mut();
            if let Some((cached_summary, metrics)) = borrow.as_ref() {
                if cached_summary == &self.summary {
                    return metrics.clone();
                }
            }
            let metrics = stepx_companion::device::DeviceMetrics::from_summary(&self.summary);
            *borrow = Some((self.summary.clone(), metrics.clone()));
            metrics
        })
    }

    fn device_block(&self) -> El {
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

        let content = match &self.selected {
            Some(d) => {
                let metrics = self.cached_device_metrics();
                let has_backup = !d.backup_transports().is_empty();
                let transport = if has_backup {
                    "双通道连接"
                } else if is_wireless(&d.transport) {
                    "无线直连"
                } else {
                    "USB 连接"
                };
                let detail = match metrics.battery_level {
                    Some(b) => format!("{transport} · {b}%"),
                    None => format!("{transport} · 已就绪"),
                };
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
                            .with_child(self.ui_icon(UiIcon::Smartphone, 15., p.text))
                            .finish(),
                    )
                    .with_background_color(icon_bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                )
                .with_width(32.)
                .with_height(32.)
                .finish();

                let text_col = Flex::column()
                    .with_spacing(2.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(self.lead(
                        6.,
                        self.dot(p.success),
                        self.line_w(title, CAPTION, p.text, Weight::Semibold),
                    ))
                    .with_child(self.line(detail, CAPTION, p.text2))
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
                            .with_child(self.ui_icon(
                                UiIcon::Smartphone,
                                15.,
                                if reconnecting { p.warning } else { p.text3 },
                            ))
                            .finish(),
                    )
                    .with_background_color(icon_bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                )
                .with_width(32.)
                .with_height(32.)
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

        Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let bg = if mouse.is_clicked() {
                p.press_color(p.hover)
            } else {
                lerp_color(card_bg, p.hover, t)
            };
            let border_col = if self.page == Page::Connect {
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
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Page(Page::Connect)))
        .finish()
    }

    pub(super) fn sidebar(&self) -> El {
        let p = self.p();
        let im_unread: usize = self
            .push
            .snapshot
            .threads
            .iter()
            .map(|t| t.unread_count)
            .sum();
        let sms_unread = if self.sms_loaded && self.sms_error.is_none() {
            self.sms.iter().filter(|m| m.kind == 1 && !m.read).count()
        } else {
            0
        };
        let push_unread = if self.push.loaded {
            self.push
                .snapshot
                .notifications
                .iter()
                .filter(|n| !n.dismissed && !n.read && !n.is_ongoing)
                .count()
        } else {
            0
        };
        let total_unread = im_unread + sms_unread + push_unread;
        let msg_badge = (total_unread > 0).then(|| total_unread.to_string());
        let cal_badge = (!self.back_screen.stepos_suite.calendar_events.is_empty()).then(|| {
            self.back_screen
                .stepos_suite
                .calendar_events
                .len()
                .to_string()
        });
        let alarm_badge = {
            let c = self
                .back_screen
                .stepos_suite
                .alarms
                .iter()
                .filter(|a| a.enabled)
                .count();
            (c > 0).then(|| c.to_string())
        };
        let gallery_badge = (!self.back_screen.gallery_items.is_empty())
            .then(|| self.back_screen.gallery_items.len().to_string());
        let app_badge = (self.apps_loaded && self.apps_error.is_none()).then(|| {
            self.apps
                .iter()
                .filter(|app| app.in_default_scope())
                .count()
                .to_string()
        });
        let amoo_badge = self.workspace_busy().then(|| "运行中".to_string());

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

        let brand_row = Container::new(self.hstack(
            10.,
            vec![
                logo_box,
                self.vstack(
                    2.,
                    vec![
                        self.line_w("StepX Companion", 15., p.text, Weight::Semibold),
                        self.mono(
                            format!("桌面伴侣 · {}", stepx_companion::build_info::VERSION),
                            11.,
                            p.text3,
                        ),
                    ],
                ),
            ],
        ))
        .with_horizontal_padding(6.)
        .with_padding_top(2.)
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
                        self.nav_group_title("AI 与工作台", true),
                        self.nav_item_with_match(
                            "overview",
                            UiIcon::Home,
                            "概览",
                            Page::Overview,
                            matches!(self.page, Page::Overview | Page::Account),
                            None,
                        ),
                        // 唯一的 AI 入口：聊天、生成文件、做应用都在这里，由系统自动判断。
                        // Page::Work 不再单独出现在侧边栏（只从「我的作品」/成果卡进入），
                        // 打开时仍高亮 Amoo。「我的作品」是作品与文件库。
                        self.nav_item_with_match(
                            "amoo",
                            UiIcon::Chat,
                            "Amoo 对话",
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
                            None,
                        ),
                        self.nav_group_title("日历、闹钟与通信", false),
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
                            "消息与通知",
                            Page::Sms,
                            matches!(self.page, Page::Sms | Page::Notifications | Page::Phone),
                            msg_badge,
                        ),
                        self.nav_group_title("媒体与设备", false),
                        self.nav_item_with_match(
                            "gallery",
                            UiIcon::Image,
                            "图库",
                            Page::Gallery,
                            self.page == Page::Gallery,
                            gallery_badge,
                        ),
                        self.nav_item_with_match(
                            "backscreen",
                            UiIcon::Matrix,
                            "背屏",
                            Page::BackScreen,
                            self.page == Page::BackScreen,
                            Some("点阵".into()),
                        ),
                        self.nav_item_with_match(
                            "apps",
                            UiIcon::Apps,
                            "应用",
                            Page::Apps,
                            self.page == Page::Apps,
                            app_badge,
                        ),
                        self.nav_item_with_match(
                            "files",
                            UiIcon::Folder,
                            "文件与扩容",
                            Page::Files,
                            matches!(self.page, Page::Files | Page::Expansion),
                            None,
                        ),
                        self.nav_item_with_match(
                            "tools",
                            UiIcon::Tools,
                            "小工具",
                            Page::Tools,
                            self.page == Page::Tools,
                            None,
                        ),
                    ])
                    .finish(),
            )
            .finish();

        self.fixed_w(
            SIDEBAR_W,
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
                        None,
                    ))
                    .finish(),
            )
            .with_uniform_padding(12.)
            .with_background_color(p.sidebar)
            .with_border(Border::right(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_top_left(Radius::Pixels(12.)))
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
        Hoverable::new(state, move |mouse| {
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
        .finish()
    }

    fn page_subtitle(&self) -> String {
        if self.selected.is_none() && self.page.needs_device() {
            return String::new();
        }
        match self.page {
            Page::Overview => {
                let android = self
                    .selected
                    .as_ref()
                    .map(|d| d.android.as_str())
                    .unwrap_or("16");
                format!(
                    "{} · Android {android} · 设备已连接",
                    self.selected
                        .as_ref()
                        .map(|d| d.model.as_str())
                        .unwrap_or("设备")
                )
            }
            Page::BackScreen => "17 列 × 23 行 · 379 颗单色灯珠 · 创作与手机联动".into(),
            Page::Amoo => "".into(),
            Page::Calendar | Page::Schedule => format!(
                "电脑与手机双向日历 · {} 项日程 · {}",
                self.back_screen.stepos_suite.calendar_events.len(),
                if self.preferences.auto_sync_calendar {
                    "自动同步开启"
                } else {
                    "手动管理"
                }
            ),
            Page::Alarms => {
                let enabled = self
                    .back_screen
                    .stepos_suite
                    .alarms
                    .iter()
                    .filter(|a| a.enabled)
                    .count();
                format!(
                    "{enabled}/{} 个手机闹钟开启 · 专注番茄钟与自动化提醒",
                    self.back_screen.stepos_suite.alarms.len()
                )
            }
            Page::Gallery => format!(
                "手机真实相册与截图 · {} 张照片 · 电脑预览与双向传输",
                self.back_screen.gallery_items.len()
            ),
            Page::Account => "手机账号 · 真实消息 Token 用量".into(),
            Page::About | Page::Settings => "更顺手的使用习惯".into(),
            Page::Expansion => "电脑保存原件 · 手机保留入口".into(),
            Page::Work => "用文件夹里的文件干活 · 结果同步回 Amoo 对话".into(),
            Page::Studio => "做过的文档、应用和小游戏都在这里".into(),
            Page::Sms => {
                let im_threads = self.push.snapshot.threads.len();
                let im_unread: usize = self
                    .push
                    .snapshot
                    .threads
                    .iter()
                    .map(|t| t.unread_count)
                    .sum();
                if self.sms_error.is_some() && im_threads == 0 {
                    "短信读取失败，请刷新重试".into()
                } else if !self.sms_loaded && !self.push.loaded {
                    "正在读取手机聊天会话与短信…".into()
                } else {
                    let sms_threads = self
                        .sms
                        .iter()
                        .map(|m| stepx_companion::sms::normalize_address(&m.address))
                        .collect::<std::collections::HashSet<_>>()
                        .len();
                    let sms_unread = self.sms.iter().filter(|m| m.kind == 1 && !m.read).count();
                    format!(
                        "{im_threads} 个聊天会话 ({im_unread} 未读) · {} 条短信 ({sms_threads} 会话 / {sms_unread} 未读)",
                        self.sms.len()
                    )
                }
            }
            Page::Notifications => {
                let total = self
                    .push
                    .snapshot
                    .notifications
                    .iter()
                    .filter(|n| !n.dismissed)
                    .count();
                let unread = self
                    .push
                    .snapshot
                    .notifications
                    .iter()
                    .filter(|n| !n.dismissed && !n.read && !n.is_ongoing)
                    .count();
                let otps = self.push.snapshot.otp_banners.len();
                format!("来自手机全部应用的实时推送 · {total} 条通知 · {unread} 条未读 · {otps} 个验证码/取件码")
            }
            Page::Phone => "通话记录、手机通讯录与桌面直接拨号".into(),
            Page::Files => {
                let m = self.cached_device_metrics();
                if m.storage_total.is_empty() {
                    "内部存储 · 容量暂不可用".into()
                } else {
                    format!("数据分区 · 可用 {} / {}", m.storage_avail, m.storage_total)
                }
            }
            Page::Apps => "管理手机端已安装应用与 APK 安装".into(),
            Page::Tools => self
                .tool_page
                .map(|t| t.description().to_string())
                .unwrap_or_else(|| "空间整理、应用安装与设备工具".into()),
            Page::Connect => "管理 USB 通道、局域网无线配对与已信任设备".into(),
        }
    }

    pub(super) fn header(&self) -> El {
        let p = self.p();
        let mut actions =
            if self.selected.is_none() && self.page.needs_device() && self.page != Page::Overview {
                vec![
                    self.button("hdr-rescan", "重新扫描", Action::Refresh, Btn::Secondary),
                    self.button_icon(
                        "hdr-connect",
                        "连接设备",
                        UiIcon::Connect,
                        Action::Page(Page::Connect),
                        Btn::Primary,
                    ),
                ]
            } else {
                self.page_actions()
            };
        actions.push(self.button(
            "operation-reminders",
            "操作提醒",
            Action::ReviewHistory,
            Btn::Ghost,
        ));
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
        if !actions.is_empty() {
            actions.push(
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(p.border)
                        .finish(),
                )
                .with_width(1.)
                .with_height(16.)
                .finish(),
            );
        }
        let controls = self.hstack(
            S2,
            vec![
                self.window_control_btn("min", "—", false, Action::MinimizeWindow),
                self.window_control_btn("max", "□", false, Action::MaximizeWindow),
                self.window_control_btn("close", "×", true, Action::CloseWindow),
            ],
        );

        let header_bg = if p.dark { p.sidebar } else { p.bg };
        ConstrainedBox::new(
            Container::new(
                self.between(
                    Clipped::new(
                        self.between(
                            Flex::row()
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_spacing(S3)
                                .with_child(self.line_w(
                                    if self.page == Page::Tools {
                                        self.tool_page.map(ToolPage::title).unwrap_or("小工具")
                                    } else {
                                        self.page.title()
                                    },
                                    TITLE,
                                    p.text,
                                    Weight::Semibold,
                                ))
                                .with_child(
                                    Expanded::new(
                                        1.,
                                        self.line(self.page_subtitle(), CAPTION, p.text2),
                                    )
                                    .finish(),
                                )
                                .finish(),
                            self.hstack(S2, actions),
                        ),
                    )
                    .finish(),
                    controls,
                ),
            )
            .with_horizontal_padding(S5)
            .with_vertical_padding(10.)
            .with_background_color(header_bg)
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_top_right(Radius::Pixels(12.)))
            .finish(),
        )
        .with_height(52.)
        .finish()
    }

    pub(super) fn status_bar(&self) -> El {
        let p = self.p();
        let transport = match &self.selected {
            Some(d) if is_wireless(&d.transport) => "无线",
            Some(_) => "USB",
            None => "ADB",
        };
        let detail = self.footer_detail();
        let left = self.lead(
            S2,
            self.fixed_w(
                18.,
                Align::new(if self.footer_loading() {
                    self.loading_line()
                } else {
                    self.dot(if self.error {
                        p.danger
                    } else if self.busy || self.initializing {
                        p.warning
                    } else if self.selected.is_some() {
                        p.success
                    } else {
                        p.text3
                    })
                })
                .finish(),
            ),
            self.line(
                format!("{transport} · {detail}"),
                CAPTION,
                if self.error { p.danger } else { p.text2 },
            ),
        );
        let right = self.hstack(
            S3,
            vec![
                self.desktop_update_link(),
                self.stars_link(),
                self.mono(
                    format!(
                        "v{} · {}",
                        stepx_companion::build_info::VERSION,
                        stepx_companion::build_info::compiled_id()
                    ),
                    CAPTION,
                    p.text3,
                ),
            ],
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
}
