use super::super::{
    backscreen_state::BackScreenAction, icons::UiIcon, is_wireless, theme::*, widgets::*, Action,
    Companion, Page, PushAction, F_OV_TEXT,
};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};

impl Companion {
    fn overview_metric_card_interactive(
        &self,
        id: &'static str,
        children: Vec<El>,
        on_double: Action,
        kind: crate::ui::context_menu::OverviewContextKind,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("ov-metric:{id}"));
        let content = self.vstack(S3, children);
        Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let border_color = if mouse.is_clicked() {
                p.accent
            } else if hovered {
                p.border_strong
            } else {
                p.border
            };
            ConstrainedBox::new(
                Container::new(content)
                    .with_uniform_padding(S4)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(border_color))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
            )
            .with_height(146.)
            .finish()
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_double_click(move |ctx, _, _| ctx.dispatch_typed_action(on_double.clone()))
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::OverviewCard(kind.clone()),
                at,
            ))
        })
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
            let t = if self.settings.value.reduced_motion {
                if hovered {
                    1.
                } else {
                    0.
                }
            } else {
                mouse.hover_progress().clamp(0., 1.)
            };
            let mix = |a: u8, b: u8| (a as f32 + (b as f32 - a as f32) * t) as u8;
            let bg = if mouse.is_clicked() {
                p.accent_soft
            } else {
                ColorU::new(
                    mix(p.surface.r, p.hover.r),
                    mix(p.surface.g, p.hover.g),
                    mix(p.surface.b, p.hover.b),
                    mix(p.surface.a, p.hover.a),
                )
            };
            let border_color = if mouse.is_clicked() {
                p.accent
            } else {
                ColorU::new(
                    mix(p.surface_border.r, p.border_strong.r),
                    mix(p.surface_border.g, p.border_strong.g),
                    mix(p.surface_border.b, p.border_strong.b),
                    mix(p.surface_border.a, p.border_strong.a),
                )
            };
            ConstrainedBox::new(
                Container::new(row)
                    .with_horizontal_padding(12.)
                    .with_vertical_padding(8.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border_color))
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

    fn hw_spec_cell(&self, icon: UiIcon, label: &'static str, val: String) -> El {
        let p = self.p();
        let icon_bg = if p.dark { p.bg } else { ColorU::white() };
        let icon_border = if p.dark {
            p.surface_border
        } else {
            rgb(0xE4, 0xE4, 0xE7)
        };
        let box_el = self.ui_icon_box(icon, 34., icon_bg, p.text2, Some(icon_border));
        let text_col = Flex::column()
            .with_spacing(2.)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(self.line(label, 11., p.text2))
            .with_child(self.line_w(&val, CAPTION, p.text, Weight::Medium))
            .finish();
        let inner = self.hstack(10., vec![box_el, text_col]);
        let state = self.mouse_state(format!("ov-hw-spec:{label}"));
        let val_dbl = val.clone();
        let val_rc = val;
        Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let border_color = if mouse.is_clicked() {
                p.accent
            } else if hovered {
                p.border_strong
            } else {
                p.surface_border
            };
            Container::new(inner)
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(border_color))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::CopyText(
                val_dbl.clone(),
                format!("已复制{label}：{val_dbl}"),
            ));
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::CopyableText {
                    title: label.into(),
                    text: val_rc.clone(),
                },
                at,
            ));
        })
        .finish()
    }

    fn overview_activity_summary_card(&self) -> El {
        let p = self.p();

        // Left column: Upcoming Schedule & Alarms quick summary
        let mut sched_rows: Vec<El> = vec![self.between(
            self.hstack(
                6.,
                vec![
                    self.ui_icon(UiIcon::Calendar, 15., p.accent),
                    self.heading("近期日程与闹钟速览"),
                ],
            ),
            self.hstack(
                S2,
                vec![
                    self.link_action("ov-go-calendar", "打开日历", Action::Page(Page::Calendar)),
                    self.link_action("ov-go-alarms", "打开闹钟", Action::Page(Page::Alarms)),
                ],
            ),
        )];

        let events: Vec<_> = self
            .back_screen
            .stepos_suite
            .calendar_events
            .iter()
            .filter(|ev| {
                !ev.title.contains("StepX 日历网格验证") && !ev.title.contains("StepX E2E")
            })
            .collect();
        let alarms = &self.back_screen.stepos_suite.alarms;

        if events.is_empty() && alarms.is_empty() {
            let empty_inner = self.between(
                self.line(
                    "暂无近期日程或手机闹钟，双击前往日历新建安排",
                    CAPTION,
                    p.text2,
                ),
                self.pill_badge("双击新建", p.accent, p.accent_soft),
            );
            sched_rows.push(
                Hoverable::new(self.mouse_state("ov-sched-empty"), move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    Container::new(empty_inner)
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(10.)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(if hovered {
                            p.border_strong
                        } else {
                            p.surface_border
                        }))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Page(Page::Calendar)))
                .on_double_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Page(Page::Calendar))
                })
                .on_right_click(|ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::OverviewCard(
                            crate::ui::context_menu::OverviewContextKind::Device,
                        ),
                        at,
                    ))
                })
                .finish(),
            );
        } else {
            for ev in events.iter().take(2) {
                let ev_id = ev.id;
                let done = self.back_screen.completed_calendar_events.contains(&ev_id);
                let title = Self::truncate_chars(&ev.title, 22);
                let row_inner = self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.pill_badge(
                                if done { "✓ 已完成" } else { "待办" },
                                if done { p.success_text } else { p.accent },
                                if done { p.success_soft } else { p.accent_soft },
                            ),
                            self.line_w(
                                title,
                                CAPTION,
                                if done { p.text3 } else { p.text },
                                Weight::Medium,
                            ),
                        ],
                    ),
                    self.mono(ev.time_range_label(), MICRO, p.text2),
                );
                sched_rows.push(
                    Hoverable::new(
                        self.mouse_state(format!("ov-sched-ev-{ev_id}")),
                        move |mouse| {
                            let hovered = mouse.is_mouse_over_element();
                            let border_color = if mouse.is_clicked() {
                                p.accent
                            } else if hovered {
                                p.border_strong
                            } else {
                                p.surface_border
                            };
                            Container::new(row_inner)
                                .with_horizontal_padding(12.)
                                .with_vertical_padding(8.)
                                .with_background_color(p.surface)
                                .with_border(Border::all(1.).with_border_color(border_color))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish()
                        },
                    )
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(
                            BackScreenAction::CalendarToggleDone(ev_id),
                        ))
                    })
                    .on_double_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(Action::Page(Page::Calendar))
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::ScheduleEvent(ev_id),
                            at,
                        ))
                    })
                    .finish(),
                );
            }

            if let Some(alarm) = alarms.iter().find(|a| a.enabled).or_else(|| alarms.first()) {
                let alarm_id = alarm.id;
                let alarm_enabled = alarm.enabled;
                let alarm_inner = self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.pill_badge(
                                if alarm_enabled {
                                    "闹钟开启"
                                } else {
                                    "闹钟关闭"
                                },
                                if alarm_enabled { p.accent } else { p.text3 },
                                if alarm_enabled { p.accent_soft } else { p.bg },
                            ),
                            self.mono(alarm.time_label(), CAPTION, p.text),
                            self.line(
                                Self::truncate_chars(alarm.display_title(), 16),
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                    self.line(alarm.repeat_label(), MICRO, p.text3),
                );
                sched_rows.push(
                    Hoverable::new(
                        self.mouse_state(format!("ov-alarm-{alarm_id}")),
                        move |mouse| {
                            let hovered = mouse.is_mouse_over_element();
                            let border_color = if mouse.is_clicked() {
                                p.accent
                            } else if hovered {
                                p.border_strong
                            } else {
                                p.surface_border
                            };
                            Container::new(alarm_inner)
                                .with_horizontal_padding(12.)
                                .with_vertical_padding(8.)
                                .with_background_color(p.surface)
                                .with_border(Border::all(1.).with_border_color(border_color))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish()
                        },
                    )
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(
                            BackScreenAction::AlarmToggle(alarm_id, !alarm_enabled),
                        ))
                    })
                    .on_double_click(|ctx, _, _| {
                        ctx.dispatch_typed_action(Action::Page(Page::Alarms))
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::AlarmItem(alarm_id),
                            at,
                        ))
                    })
                    .finish(),
                );
            }
        }
        sched_rows.push(self.line(
            "单击切换完成/开关 · 双击跳转详情页 · 右键打开快捷菜单",
            MICRO,
            p.text3,
        ));
        let left_sched_card = self.panel(sched_rows);

        // Right column: Recent Notifications & SMS quick summary
        let mut msg_rows: Vec<El> = vec![self.between(
            self.hstack(
                6.,
                vec![
                    self.ui_icon(UiIcon::Bell, 15., p.accent),
                    self.heading("最新消息与通知速览"),
                ],
            ),
            self.hstack(
                S2,
                vec![
                    self.link_action(
                        "ov-go-notifications",
                        "手机推送",
                        Action::Page(Page::Notifications),
                    ),
                    self.link_action("ov-go-sms", "消息与短信", Action::Page(Page::Sms)),
                ],
            ),
        )];

        let masked = self.push.prefs.privacy_mask;
        let mut added_msg_items = 0usize;

        for n in self
            .push
            .snapshot
            .notifications
            .iter()
            .filter(|n| {
                !n.dismissed
                    && !stepx_companion::push::should_hide_notification(n)
                    && !(n.text.trim().is_empty() && n.sub_text.trim().is_empty())
            })
            .take(2)
        {
            added_msg_items += 1;
            let n_key = n.key.clone();
            let n_key_dbl = n.key.clone();
            let app_badge = if n.app_name.is_empty() {
                "通知".to_string()
            } else {
                Self::truncate_chars(&n.app_name, 10)
            };
            let body_text = if !n.text.trim().is_empty() {
                n.text.trim()
            } else {
                n.sub_text.trim()
            };
            let summary_text = if masked {
                "隐私遮罩已开启 · 点击查看通知".to_string()
            } else if n.title.trim().is_empty() {
                Self::truncate_chars(body_text, 26)
            } else {
                Self::truncate_chars(&format!("{}：{}", n.title.trim(), body_text), 26)
            };
            let row_inner = self.between(
                self.hstack(
                    8.,
                    vec![
                        self.pill_badge(app_badge, p.accent, p.accent_soft),
                        self.line_w(summary_text, CAPTION, p.text, Weight::Medium),
                    ],
                ),
                self.line("双击手机打开", MICRO, p.text3),
            );
            msg_rows.push(
                Hoverable::new(
                    self.mouse_state(format!("ov-notif-{}", n.key)),
                    move |mouse| {
                        let hovered = mouse.is_mouse_over_element();
                        let border_color = if mouse.is_clicked() {
                            p.accent
                        } else if hovered {
                            p.border_strong
                        } else {
                            p.surface_border
                        };
                        Container::new(row_inner)
                            .with_horizontal_padding(12.)
                            .with_vertical_padding(8.)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(border_color))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                    },
                )
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Page(Page::Notifications)))
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::OpenOnPhone(
                        n_key_dbl.clone(),
                    )))
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::Notification(n_key.clone()),
                        at,
                    ))
                })
                .finish(),
            );
        }

        if added_msg_items < 2 {
            if let Some(m) = self.sms.first() {
                added_msg_items += 1;
                let addr = m.address.clone();
                let body = m.body.clone();
                let otp_code = if masked {
                    None
                } else {
                    super::sms::extract_verification_code(&m.body)
                };
                let preview = if masked {
                    "隐私遮罩已开启 · 点击查看短信".to_string()
                } else {
                    Self::truncate_chars(&m.body, 26)
                };
                let sender_label = if masked {
                    "短信".to_string()
                } else {
                    Self::truncate_chars(&m.address, 12)
                };
                let sms_inner = self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.pill_badge(sender_label.clone(), p.text2, p.bg),
                            self.line_w(preview, CAPTION, p.text, Weight::Medium),
                        ],
                    ),
                    self.line("右键更多", MICRO, p.text3),
                );
                msg_rows.push(
                    Hoverable::new(self.mouse_state("ov-latest-sms"), move |mouse| {
                        let hovered = mouse.is_mouse_over_element();
                        let border_color = if mouse.is_clicked() {
                            p.accent
                        } else if hovered {
                            p.border_strong
                        } else {
                            p.surface_border
                        };
                        Container::new(sms_inner)
                            .with_horizontal_padding(12.)
                            .with_vertical_padding(8.)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(border_color))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Page(Page::Sms)))
                    .on_double_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Page(Page::Sms)))
                    .on_right_click(move |ctx, _, at| {
                        let target = if masked {
                            crate::ui::context_menu::Target::CopyableText {
                                title: "短信号码".into(),
                                text: addr.clone(),
                            }
                        } else {
                            crate::ui::context_menu::Target::SmsMessage {
                                address: addr.clone(),
                                sender: sender_label.clone(),
                                body: body.clone(),
                                otp_code: otp_code.clone(),
                            }
                        };
                        ctx.dispatch_typed_action(Action::ContextOpen(target, at));
                    })
                    .finish(),
                );
            }
        }

        if added_msg_items == 0 {
            let empty_msg = self.between(
                self.line(
                    "暂无未读手机推送或近期短信，双击查看消息中心",
                    CAPTION,
                    p.text2,
                ),
                self.pill_badge("消息同步中", p.text2, p.bg),
            );
            msg_rows.push(
                Hoverable::new(self.mouse_state("ov-msg-empty"), move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    Container::new(empty_msg)
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(10.)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(if hovered {
                            p.border_strong
                        } else {
                            p.surface_border
                        }))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::Page(Page::Notifications)))
                .on_double_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Page(Page::Notifications))
                })
                .on_right_click(|ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::OverviewCard(
                            crate::ui::context_menu::OverviewContextKind::Device,
                        ),
                        at,
                    ))
                })
                .finish(),
            );
        }
        msg_rows.push(self.line(
            "单击进入列表 · 双击在手机打开 · 右键复制全文 / 验证码 / 交给 Amoo 处理",
            MICRO,
            p.text3,
        ));
        let right_msg_card = self.panel(msg_rows);

        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S4)
            .with_child(Expanded::new(1., left_sched_card).finish())
            .with_child(Expanded::new(1., right_msg_card).finish())
            .finish()
    }

    pub(in super::super) fn overview_page(&self) -> El {
        let Some(device) = &self.selected else {
            return self.pane_scroll(
                self.scroll.clone(),
                Container::new(self.vstack(S4, vec![self.overview_usage_card(), self.no_device()]))
                    .with_horizontal_padding(S5)
                    .with_padding_top(S5)
                    .with_padding_bottom(S6)
                    .finish(),
            );
        };
        let p = self.p();
        let metrics = self.cached_device_metrics();

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
        let card_device = self.overview_metric_card_interactive(
            "device",
            vec![
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
            ],
            Action::OpenTool(Some(super::super::ToolPage::Device)),
            crate::ui::context_menu::OverviewContextKind::Device,
        );

        let card_battery = self.overview_metric_card_interactive(
            "battery",
            vec![
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
            ],
            Action::Summary,
            crate::ui::context_menu::OverviewContextKind::Battery,
        );

        let card_temp = self.overview_metric_card_interactive(
            "temp",
            vec![
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
            ],
            Action::Summary,
            crate::ui::context_menu::OverviewContextKind::Battery,
        );

        let card_storage = self.overview_metric_card_interactive(
            "storage",
            vec![
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
            ],
            Action::Page(Page::Files),
            crate::ui::context_menu::OverviewContextKind::Storage,
        );

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

        let raw_illustration = if self.overview_back_preview {
            let frame = self.back_screen.frame.clone();
            let rear_housing = self.phone_rear_housing(158., 328., &frame, 4.1, 1.2);
            let rear_dock = Container::new(self.wrap(vec![
                self.button(
                    "ov-rear-invert",
                    "反相",
                    Action::BackScreen(BackScreenAction::InvertFrame),
                    Btn::Secondary,
                ),
                self.button(
                    "ov-rear-left",
                    "左移",
                    Action::BackScreen(BackScreenAction::ShiftFrame(-1, 0)),
                    Btn::Secondary,
                ),
                self.button(
                    "ov-rear-right",
                    "右移",
                    Action::BackScreen(BackScreenAction::ShiftFrame(1, 0)),
                    Btn::Secondary,
                ),
                self.button(
                    "ov-rear-breath",
                    "呼吸灯",
                    Action::BackScreenToggleHwBreath,
                    Btn::Secondary,
                ),
                self.button(
                    "ov-rear-push",
                    "推送背屏",
                    Action::BackScreen(BackScreenAction::Play),
                    Btn::Primary,
                ),
            ]))
            .with_uniform_padding(4.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .finish();
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.)
                .with_child(rear_housing)
                .with_child(rear_dock)
                .finish()
        } else {
            self.interactive_phone_bezel("ov-front", 156., 324., None)
        };
        let is_back_preview = self.overview_back_preview;
        let phone_illustration =
            Hoverable::new(self.mouse_state("ov-phone-illustration"), move |_| {
                raw_illustration
            })
            .with_defer_events_to_children()
            .with_cursor(Cursor::PointingHand)
            .on_double_click(move |ctx, _, _| {
                if is_back_preview {
                    ctx.dispatch_typed_action(Action::Page(Page::BackScreen));
                } else {
                    ctx.dispatch_typed_action(Action::Mirror);
                }
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

        let left_preview = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Min)
                .with_spacing(S3)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.heading("外观预览"),
                            self.line(
                                if self.overview_back_preview {
                                    "背屏设计稿"
                                } else if self.live_screen_stream {
                                    "● 实时同步中"
                                } else if self.screenshot.is_some() {
                                    "单击直接触控"
                                } else {
                                    "单击直接触控"
                                },
                                CAPTION,
                                if !self.overview_back_preview && self.live_screen_stream {
                                    p.success
                                } else {
                                    p.text2
                                },
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
                    Container::new(if self.overview_back_preview {
                        self.between(
                            self.link_icon(
                                "ov-refresh-backscreen",
                                "刷新背屏状态",
                                UiIcon::Refresh,
                                Action::BackScreen(BackScreenAction::CompanionInspect),
                            ),
                            self.link_icon(
                                "ov-go-backscreen",
                                "背屏设置",
                                UiIcon::Settings,
                                Action::Page(Page::BackScreen),
                            ),
                        )
                    } else {
                        self.between(
                            self.hstack(
                                10.,
                                vec![
                                    self.link_icon(
                                        "ov-toggle-live-screen",
                                        if self.live_screen_stream {
                                            "暂停实时同步"
                                        } else {
                                            "实时画面同步"
                                        },
                                        UiIcon::Refresh,
                                        Action::ToggleLiveScreenStream,
                                    ),
                                    self.link_icon(
                                        "ov-refresh-shot",
                                        "刷新截屏",
                                        UiIcon::Camera,
                                        Action::Screenshot,
                                    ),
                                ],
                            ),
                            self.link_icon(
                                "ov-open-screen-studio",
                                "投屏工坊",
                                UiIcon::Monitor,
                                Action::OpenTool(Some(super::super::ToolPage::Screen)),
                            ),
                        )
                    })
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
                    self.key_row_btn("mute", UiIcon::Bell, "静音", "Mute", Action::Key(164)),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.key_row_btn(
                        "play-pause",
                        UiIcon::Play,
                        "播放/暂停",
                        "Play",
                        Action::Key(85),
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

        let quick_hub_bar = self.wrap(vec![
            self.button_icon(
                "ov-cmd-mirror",
                "投屏控制手机",
                UiIcon::Monitor,
                Action::Mirror,
                Btn::Primary,
            ),
            self.button_icon(
                "ov-cmd-screen-studio",
                "内嵌投屏工坊",
                UiIcon::Smartphone,
                Action::OpenTool(Some(super::super::ToolPage::Screen)),
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-screenshot",
                "截屏到电脑",
                UiIcon::Camera,
                Action::Screenshot,
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-wake",
                "唤醒亮屏",
                UiIcon::Sun,
                Action::Key(224),
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-sleep",
                "一键锁屏",
                UiIcon::Lock,
                Action::Key(223),
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-push-clip",
                "电脑剪贴板发手机",
                UiIcon::Upload,
                Action::BackScreen(BackScreenAction::PushDesktopClipboard),
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-pull-clip",
                "拉取手机剪贴板",
                UiIcon::Download,
                Action::BackScreen(BackScreenAction::PullPhoneClipboard),
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-send-files",
                "发送文件到手机",
                UiIcon::Folder,
                Action::CompanionSendFiles,
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-install-apk",
                "安装 APK",
                UiIcon::Package,
                Action::InstallApk,
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-dnd",
                "勿扰模式开关",
                UiIcon::Bell,
                Action::BackScreen(BackScreenAction::McpToggleDnd),
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-cmd-find-phone",
                "响铃找手机",
                UiIcon::Bell,
                Action::BackScreen(BackScreenAction::TriggerMcpQuickAction("voice_tts".into())),
                Btn::Secondary,
            ),
            self.button_icon(
                "ov-quick-capture-gallery",
                "截屏入手机相册",
                UiIcon::Smartphone,
                Action::BackScreen(BackScreenAction::GalleryCapturePhoneScreenshot),
                Btn::Ghost,
            ),
            self.button_icon(
                "ov-quick-open-gallery",
                "打开手机相册",
                UiIcon::Image,
                Action::Page(Page::Gallery),
                Btn::Ghost,
            ),
        ]);

        let control_card = self.panel(vec![
            self.between(self.heading("跨端快捷控制中心"), ai_hint),
            quick_hub_bar,
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
                    self.hw_spec_cell(
                        UiIcon::Shield,
                        "系统版本",
                        format!("Android {}", device.android),
                    ),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell(
                        UiIcon::Smartphone,
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
                    self.hw_spec_cell(UiIcon::Matrix, "背屏设计画布", "17 × 23 · 非实况".into()),
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
                    self.hw_spec_cell(UiIcon::Settings, "产品型号", device.product.clone()),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell(
                        UiIcon::Sun,
                        "屏幕亮度",
                        format!(
                            "{}%",
                            self.back_screen.stepos_suite.controls.brightness_percent
                        ),
                    ),
                )
                .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.hw_spec_cell(
                        UiIcon::Wifi,
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

        // Live Verification / Pickup Code Quick Banner on Overview
        let live_code_banner: Option<El> = if self.push.prefs.privacy_mask {
            None
        } else {
            let latest_otp = self
                .sms
                .iter()
                .find_map(|m| {
                    super::sms::extract_verification_code(&m.body)
                        .filter(|c| !super::sms::is_pickup_code_text(c, &m.body))
                })
                .or_else(|| {
                    self.push
                        .snapshot
                        .notifications
                        .iter()
                        .filter(|n| !n.dismissed)
                        .find_map(|n| {
                            n.otp_code
                                .clone()
                                .filter(|c| !c.trim().is_empty())
                                .or_else(|| super::sms::extract_verification_code(&n.text))
                                .filter(|c| !super::sms::is_pickup_code_text(c, &n.text))
                        })
                })
                .or_else(|| {
                    self.push
                        .snapshot
                        .otp_banners
                        .iter()
                        .find(|b| !super::sms::is_pickup_code_text(&b.code, &b.raw_text))
                        .map(|b| b.code.clone())
                });

            let latest_pickup = self
                .sms
                .iter()
                .find_map(|m| super::sms::extract_pickup_code(&m.body))
                .or_else(|| {
                    self.push
                        .snapshot
                        .notifications
                        .iter()
                        .filter(|n| !n.dismissed)
                        .find_map(|n| {
                            super::sms::extract_pickup_code(&n.text).or_else(|| {
                                n.otp_code
                                    .as_ref()
                                    .filter(|c| super::sms::is_pickup_code_text(c, &n.text))
                                    .cloned()
                            })
                        })
                })
                .or_else(|| {
                    self.push
                        .snapshot
                        .otp_banners
                        .iter()
                        .find(|b| super::sms::is_pickup_code_text(&b.code, &b.raw_text))
                        .map(|b| b.code.clone())
                });

            if latest_otp.is_some() || latest_pickup.is_some() {
                let mut left_items = vec![
                    self.dot(p.accent),
                    self.line_w("实时验证码 / 取件码速取", CAPTION, p.text, Weight::Semibold),
                ];
                let mut right_btns = Vec::new();
                if let Some(code) = &latest_otp {
                    let badge = self.pill_badge_icon(
                        format!("验证码 {code}"),
                        UiIcon::Key,
                        p.accent,
                        p.accent_soft,
                    );
                    let c_click = code.clone();
                    let c_dbl = code.clone();
                    let c_rc = code.clone();
                    left_items.push(
                        Hoverable::new(self.mouse_state("ov-badge-otp"), move |_| badge)
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::Push(PushAction::CopyOtp(
                                    c_click.clone(),
                                )))
                            })
                            .on_double_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::Push(PushAction::CopyOtp(
                                    c_dbl.clone(),
                                )))
                            })
                            .on_right_click(move |ctx, _, at| {
                                ctx.dispatch_typed_action(Action::ContextOpen(
                                    crate::ui::context_menu::Target::CopyableText {
                                        title: "实时验证码".into(),
                                        text: c_rc.clone(),
                                    },
                                    at,
                                ))
                            })
                            .finish(),
                    );
                    right_btns.push(self.button_icon(
                        "ov-top-copy-otp",
                        format!("复制验证码 {code}"),
                        UiIcon::Clipboard,
                        Action::Push(PushAction::CopyOtp(code.clone())),
                        Btn::Primary,
                    ));
                }
                if let Some(pickup) = &latest_pickup {
                    let badge = self.pill_badge_icon(
                        format!("取件码 {pickup}"),
                        UiIcon::Package,
                        p.warning,
                        p.warning_soft,
                    );
                    let p_click = pickup.clone();
                    let p_dbl = pickup.clone();
                    let p_rc = pickup.clone();
                    left_items.push(
                        Hoverable::new(self.mouse_state("ov-badge-pickup"), move |_| badge)
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::CopyText(
                                    p_click.clone(),
                                    format!("已复制取件码 {p_click}"),
                                ))
                            })
                            .on_double_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::CopyText(
                                    p_dbl.clone(),
                                    format!("已复制取件码 {p_dbl}"),
                                ))
                            })
                            .on_right_click(move |ctx, _, at| {
                                ctx.dispatch_typed_action(Action::ContextOpen(
                                    crate::ui::context_menu::Target::CopyableText {
                                        title: "实时取件码".into(),
                                        text: p_rc.clone(),
                                    },
                                    at,
                                ))
                            })
                            .finish(),
                    );
                    right_btns.push(self.button_icon(
                        "ov-top-copy-pickup",
                        format!("复制取件码 {pickup}"),
                        UiIcon::Package,
                        Action::CopyText(pickup.clone(), format!("已复制取件码 {pickup}")),
                        if latest_otp.is_none() {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ));
                    right_btns.push(self.button_icon(
                        "ov-top-send-backscreen",
                        "投送到手机背屏",
                        UiIcon::Matrix,
                        Action::AmooPushPickupCode(pickup.clone()),
                        Btn::Secondary,
                    ));
                }
                Some(self.panel(vec![
                    self.between(self.hstack(8., left_items), self.hstack(8., right_btns)),
                ]))
            } else {
                None
            }
        };

        // The overview is a document-like dashboard. Its sections keep their
        // natural height; a short window scrolls rather than squeezing cards.
        let mut content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S4);
        if let Some(banner) = live_code_banner {
            content = content.with_child(banner);
        }
        let content = content
            .with_child(top_metrics)
            .with_child(self.overview_usage_card())
            .with_child(lower)
            .with_child(self.overview_activity_summary_card())
            .finish();
        self.pane_scroll(
            self.scroll.clone(),
            Container::new(content)
                .with_horizontal_padding(S5)
                .with_padding_top(S5)
                .with_padding_bottom(S6)
                .finish(),
        )
    }
}
