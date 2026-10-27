use super::super::{
    icons::UiIcon, is_wireless, theme::*, widgets::*, Action, Companion, Page, F_OV_TEXT,
};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use stepx_companion::device::DeviceMetrics;

impl Companion {
    fn overview_metric_card(&self, children: Vec<El>) -> El {
        // A bounded shared height keeps the metric row aligned even when one
        // sensor adds a second hint line or reports an unavailable value.
        ConstrainedBox::new(self.panel(children))
            .with_height(160.)
            .finish()
    }
    pub(in super::super) fn overview_actions(&self) -> Vec<El> {
        if self.selected.is_none() {
            return vec![
                self.button(
                    "ov-account",
                    "账号与用量",
                    Action::Page(Page::Account),
                    Btn::Secondary,
                ),
                self.button("ov-refresh", "重新扫描", Action::Refresh, Btn::Secondary),
                self.button_icon(
                    "ov-connect",
                    "连接设备",
                    UiIcon::Refresh,
                    Action::Page(Page::Connect),
                    Btn::Primary,
                ),
            ];
        }
        vec![
            self.button(
                "ov-account",
                "账号与用量",
                Action::Page(Page::Account),
                Btn::Secondary,
            ),
            self.button("ov-summary", "刷新状态", Action::Summary, Btn::Secondary),
            self.button_icon(
                "ov-shot",
                "截屏",
                UiIcon::Camera,
                Action::Screenshot,
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-lan-cast",
                if self.lan_cast.is_running() {
                    "局域网投屏中"
                } else {
                    "局域网投屏"
                },
                UiIcon::Wifi,
                Action::OpenTool(Some(super::super::ToolPage::Screen)),
                Btn::Secondary,
            ),
            if self.mirror.is_some() {
                self.button(
                    "ov-mirror-stop",
                    "结束投屏",
                    Action::StopMirror,
                    Btn::Danger,
                )
            } else {
                self.button_icon(
                    "ov-mirror",
                    "打开投屏",
                    UiIcon::Monitor,
                    Action::Mirror,
                    Btn::Primary,
                )
            },
        ]
    }

    fn key_row_btn(
        &self,
        id: &str,
        icon: UiIcon,
        title: &'static str,
        code: &'static str,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("ov-key:{id}"));
        let left = self.hstack(
            8.,
            vec![
                self.ui_icon(icon, 14., p.text2),
                self.line_w(title, CAPTION, p.text, Weight::Medium),
            ],
        );
        let right = self.mono(code, 11., p.text2);
        let row = self.between(left, right);
        Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            ConstrainedBox::new(
                Container::new(row)
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(8.)
                    .with_background_color(if hovered { p.hover } else { p.surface })
                    .with_border(Border::all(1.).with_border_color(if hovered {
                        p.border_strong
                    } else {
                        p.surface_border
                    }))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            )
            .with_height(36.)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    fn hw_spec_cell(&self, icon: &'static str, label: &'static str, val: String) -> El {
        let p = self.p();
        let icon_bg = if p.dark { p.bg } else { ColorU::white() };
        let icon_border = if p.dark {
            p.surface_border
        } else {
            rgb(0xE4, 0xE4, 0xE7)
        };
        let box_el = self.icon_box(icon, 28., icon_bg, p.text2, Some(icon_border));
        let text_col = Flex::column()
            .with_spacing(2.)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(self.line(label, 11., p.text2))
            .with_child(self.line_w(val, CAPTION, p.text, Weight::Medium))
            .finish();
        Container::new(self.hstack(10., vec![box_el, text_col]))
            .with_horizontal_padding(12.)
            .with_vertical_padding(8.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.surface_border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
    }

    pub(in super::super) fn overview_page(&self) -> El {
        let Some(device) = &self.selected else {
            return self.scrollable(
                self.scroll.clone(),
                self.vstack(S4, vec![self.overview_usage_card(), self.no_device()]),
            );
        };
        let p = self.p();
        let metrics = DeviceMetrics::from_summary(&self.summary);

        let battery_lvl = metrics.battery_level;
        let battery_val = battery_lvl
            .map(|v| format!("{v}%"))
            .unwrap_or_else(|| "未知".into());
        let temperature = if metrics.temperature.is_empty() {
            "未知".to_string()
        } else {
            metrics.temperature.clone()
        };
        let model_display = if device.model.contains("StepX") {
            device.model.clone()
        } else {
            format!("StepX {}", device.model)
        };
        let backups = device.backup_transports();
        let has_dual_standby = !backups.is_empty();

        // 1. Top 4-Column Metric Row
        let card_device = self.overview_metric_card(vec![
            self.between(
                self.hstack(
                    6.,
                    vec![
                        self.ui_icon(UiIcon::Smartphone, 15., p.text2),
                        self.line_w("当前设备", CAPTION, p.text2, Weight::Medium),
                    ],
                ),
                self.pill_badge(
                    if has_dual_standby {
                        "USB + 无线双通道热备"
                    } else if is_wireless(&device.transport) {
                        "无线连接"
                    } else {
                        "USB 连接"
                    },
                    p.success_text,
                    p.success_soft,
                ),
            ),
            Flex::column()
                .with_spacing(2.)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(self.line_w(model_display, TITLE, p.text, Weight::Semibold))
                .with_child(self.mono(&device.serial, CAPTION, p.text2))
                .finish(),
            Container::new(self.between(
                self.line("Amoo 网关", CAPTION, p.text2),
                self.mono(
                    if self.amoo_gateway_online {
                        "已连接"
                    } else {
                        "未连接"
                    },
                    CAPTION,
                    if self.amoo_gateway_online {
                        p.success
                    } else {
                        p.text2
                    },
                ),
            ))
            .with_padding_top(6.)
            .with_border(Border::top(1.).with_border_color(p.border))
            .finish(),
        ]);

        let card_battery = self.overview_metric_card(vec![
            self.between(
                self.hstack(
                    6.,
                    vec![
                        self.ui_icon(UiIcon::Bolt, 15., p.text2),
                        self.line_w("电池电量", CAPTION, p.text2, Weight::Medium),
                    ],
                ),
                self.line_w(
                    if metrics.charging_state.is_empty() {
                        "未检测"
                    } else {
                        &metrics.charging_state
                    },
                    CAPTION,
                    p.text2,
                    Weight::Medium,
                ),
            ),
            self.hstack(
                8.,
                vec![
                    self.line_w(battery_val, METRIC, p.text, Weight::Semibold),
                    self.line(
                        format!(
                            "健康：{}",
                            if metrics.battery_health.is_empty() {
                                "未检测"
                            } else {
                                &metrics.battery_health
                            }
                        ),
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            Flex::column()
                .with_spacing(4.)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.progress(battery_lvl, p.success))
                .with_child(self.between(
                    self.line(
                        if metrics.voltage.is_empty() {
                            "电压未检测"
                        } else {
                            &metrics.voltage
                        },
                        CAPTION,
                        p.text2,
                    ),
                    self.line("手机上报", CAPTION, p.text2),
                ))
                .finish(),
        ]);

        let card_temp = self.overview_metric_card(vec![
            self.hstack(
                6.,
                vec![
                    self.ui_icon(UiIcon::Temperature, 15., p.text2),
                    self.line_w("电池温度", CAPTION, p.text2, Weight::Medium),
                ],
            ),
            self.line_w(temperature, METRIC, p.text, Weight::Semibold),
            self.vstack(
                S1,
                vec![
                    self.line("来自手机电池传感器", CAPTION, p.text2),
                    self.line("刷新状态获取最新读数", MICRO, p.text3),
                ],
            ),
        ]);

        let card_storage = self.overview_metric_card(vec![
            self.between(
                self.hstack(
                    6.,
                    vec![
                        self.ui_icon(UiIcon::Storage, 15., p.text2),
                        self.line_w("内部存储", CAPTION, p.text2, Weight::Medium),
                    ],
                ),
                self.mono("数据分区", CAPTION, p.text2),
            ),
            self.hstack(
                6.,
                vec![
                    self.line_w(
                        if metrics.storage_used.is_empty() {
                            "未知"
                        } else {
                            &metrics.storage_used
                        },
                        METRIC,
                        p.text,
                        Weight::Semibold,
                    ),
                    self.line(
                        format!(
                            "/ {}",
                            if metrics.storage_total.is_empty() {
                                "未知"
                            } else {
                                &metrics.storage_total
                            }
                        ),
                        SUBHEAD,
                        p.text2,
                    ),
                ],
            ),
            Flex::column()
                .with_spacing(4.)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.progress(metrics.storage_percent, p.accent))
                .with_child(
                    self.between(
                        self.line(
                            metrics
                                .storage_percent
                                .map(|v| format!("已用 {v}%"))
                                .unwrap_or_else(|| "暂不可用".into()),
                            CAPTION,
                            p.text2,
                        ),
                        self.line(
                            format!(
                                "可用 {}",
                                if metrics.storage_avail.is_empty() {
                                    "未知"
                                } else {
                                    &metrics.storage_avail
                                }
                            ),
                            CAPTION,
                            p.text2,
                        ),
                    ),
                )
                .finish(),
        ]);

        let top_metrics = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S4)
            .with_child(Expanded::new(1., card_device).finish())
            .with_child(Expanded::new(1., card_battery).finish())
            .with_child(Expanded::new(1., card_temp).finish())
            .with_child(Expanded::new(1., card_storage).finish())
            .finish();

        // 2. Left 4 Cols: Interactive Front / Rear Screen Preview
        let preview_switcher = self.segmented(
            "ov-preview-mode",
            vec![
                (
                    "正面".into(),
                    !self.overview_back_preview,
                    Action::OverviewBackPreview(false),
                ),
                (
                    "背屏".into(),
                    self.overview_back_preview,
                    Action::OverviewBackPreview(true),
                ),
            ],
        );

        let phone_illustration = if self.overview_back_preview {
            let frame = self.back_screen.frame.clone();
            self.phone_rear_housing(170., 357., &frame, 4.3, 1.3)
        } else {
            self.phone_front_housing(170., 357.)
        };

        let left_preview = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(S5)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.heading("外观预览"),
                            self.line(
                                if self.overview_back_preview {
                                    "背屏设计稿"
                                } else {
                                    if self.screenshot.is_some() {
                                        "最近截图"
                                    } else {
                                        "外观示意"
                                    }
                                },
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                    preview_switcher,
                ))
                .with_child(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(phone_illustration)
                        .finish(),
                )
                .with_child(
                    Container::new(self.between(
                        self.link(
                            "ov-toggle-screen",
                            if self.overview_back_preview {
                                "← 切回正面预览"
                            } else {
                                "切换背面预览"
                            },
                            Action::OverviewBackPreview(!self.overview_back_preview),
                        ),
                        self.link_action(
                            "ov-go-backscreen",
                            "⚙ 背屏设置",
                            Action::Page(Page::BackScreen),
                        ),
                    ))
                    .with_padding_top(8.)
                    .with_border(Border::top(1.).with_border_color(p.border))
                    .finish(),
                )
                .finish(),
        )
        .with_uniform_padding(S4)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        // 3. Right 8 Cols: Remote Key Pad + Text Input + Hardware Grid
        let row1 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.)
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn("back", UiIcon::ArrowLeft, "返回", "ESC", Action::Key(4)),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn("home", UiIcon::Home, "主屏幕", "Home", Action::Key(3)),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn("recent", UiIcon::Menu, "多任务", "App", Action::Key(187)),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn("power", UiIcon::Lock, "锁屏", "Power", Action::Key(26)),
                )
                .finish(),
            )
            .finish();

        let row2 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.)
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn(
                        "voldown",
                        UiIcon::Speaker,
                        "音量减",
                        "Vol-",
                        Action::Key(25),
                    ),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn("volup", UiIcon::Speaker, "音量加", "Vol+", Action::Key(24)),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn("ai", UiIcon::Bolt, "Amoo", "", Action::Page(Page::Amoo)),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn(
                        "matrix",
                        UiIcon::Matrix,
                        "背屏工坊",
                        "LED",
                        Action::Page(Page::BackScreen),
                    ),
                )
                .finish(),
            )
            .finish();

        let focus_input_bar = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Expanded::new(
                    1.,
                    self.input(F_OV_TEXT, "向手机当前焦点输入框直接发送文字…", None),
                )
                .finish(),
            )
            .with_child(self.button(
                "ov-send-focus-text",
                "发送文本",
                Action::SendFocusText,
                Btn::Secondary,
            ))
            .finish();

        let ai_hint = self.line("按键控制当前连接的手机", CAPTION, p.text2);

        let control_card = self.panel(vec![
            self.between(self.heading("快捷控制"), ai_hint),
            row1,
            row2,
            focus_input_bar,
        ]);

        let hw_row1 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(S3)
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell("🛡", "系统版本", format!("Android {}", device.android)),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell(
                        "📱",
                        "屏幕分辨率",
                        if metrics.screen_size.is_empty() {
                            "未检测".into()
                        } else {
                            metrics.screen_size.clone()
                        },
                    ),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell("❖", "背屏设计画布", "17 × 23 · 非实况".into()),
                )
                .finish(),
            )
            .finish();

        let hw_row2 = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(S3)
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell("⚙", "产品型号", device.product.clone()),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell(
                        "⌘",
                        "Amoo 后台",
                        if self.amoo_gateway_online {
                            "已连接".into()
                        } else {
                            "未连接".into()
                        },
                    ),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell(
                        "📡",
                        "手机无线地址",
                        if metrics.wifi_ipv4.is_empty() {
                            "未检测".into()
                        } else {
                            metrics.wifi_ipv4.clone()
                        },
                    ),
                )
                .finish(),
            )
            .finish();

        let priv_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(S3)
            .with_child(
                Expanded::new(
                    1.,
                    self.panel(vec![
                        self.heading("Google Play"),
                        self.text("安装与检测手机上的 Google 服务", CAPTION, p.text2),
                        self.hstack(
                            S2,
                            vec![
                                self.button(
                                    "ov-gms-maintain",
                                    "安装与检测",
                                    Action::OpenTool(Some(super::super::ToolPage::Google)),
                                    Btn::Secondary,
                                ),
                                self.button(
                                    "ov-open-play",
                                    "打开商店",
                                    Action::OpenApp("com.android.vending".into()),
                                    Btn::Secondary,
                                ),
                            ],
                        ),
                    ]),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.panel(vec![
                        self.heading("手机管理"),
                        self.text("管理存储空间、应用与系统服务", CAPTION, p.text2),
                        self.hstack(
                            S2,
                            vec![
                                self.button(
                                    "ov-storage",
                                    "释放空间",
                                    Action::OpenTool(Some(super::super::ToolPage::Storage)),
                                    Btn::Secondary,
                                ),
                                self.button(
                                    "ov-tools",
                                    "全部工具",
                                    Action::OpenTool(None),
                                    Btn::Ghost,
                                ),
                            ],
                        ),
                    ]),
                )
                .finish(),
            )
            .finish();

        let hw_card = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(S2)
                .with_child(self.between(
                    self.heading("设备信息与小工具"),
                    self.line("设备信息来自手机", CAPTION, p.text2),
                ))
                .with_child(priv_row)
                .with_child(self.vstack(S2, vec![hw_row1, hw_row2]))
                .finish(),
        )
        .with_uniform_padding(S4)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let right_column = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S4)
            .with_child(control_card)
            .with_child(hw_card)
            .finish();

        let lower = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S4)
            .with_child(Expanded::new(4., left_preview).finish())
            .with_child(Expanded::new(8., right_column).finish())
            .finish();

        // The overview is a document-like dashboard. Its sections keep their
        // natural height; a short window scrolls rather than squeezing cards.
        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S4)
            .with_child(top_metrics)
            .with_child(self.overview_usage_card())
            .with_child(lower)
            .finish();
        self.scrollable(self.scroll.clone(), content)
    }
}
