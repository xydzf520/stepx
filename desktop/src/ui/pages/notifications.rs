use super::super::{
    icons::UiIcon, theme::*, widgets::*, Action, Companion, Page, PushAction, F_PUSH_REPLY,
    F_PUSH_SEARCH,
};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use stepx_companion::push::{
    demo_push_snapshot, ImChannel, PushCategory, PushNotification, PushSnapshot,
};

fn app_badge_style(package_name: &str, app_name: &str) -> (String, ColorU, ColorU) {
    let ch = ImChannel::from_package(package_name);
    match ch {
        ImChannel::WeChat => ("微".into(), rgb(0x07, 0xC1, 0x60), ColorU::white()),
        ImChannel::WeCom => ("企".into(), rgb(0x2B, 0x6C, 0xB0), ColorU::white()),
        ImChannel::DingTalk => ("钉".into(), rgb(0x00, 0x89, 0xFF), ColorU::white()),
        ImChannel::Feishu => ("飞".into(), rgb(0x33, 0x70, 0xFF), ColorU::white()),
        ImChannel::QqTim => ("Q".into(), rgb(0x02, 0x84, 0xC7), ColorU::white()),
        ImChannel::Sms => ("短".into(), rgb(0x10, 0xB9, 0x81), ColorU::white()),
        _ => {
            if package_name.contains("cainiao") || app_name.contains("菜鸟") {
                ("菜".into(), rgb(0xF9, 0x73, 0x16), ColorU::white())
            } else if package_name.contains("cmb")
                || app_name.contains("招行")
                || app_name.contains("招商")
            {
                ("招".into(), rgb(0xE1, 0x1D, 0x48), ColorU::white())
            } else if package_name.contains("Alipay") || app_name.contains("支付宝") {
                ("支".into(), rgb(0x02, 0x84, 0xC7), ColorU::white())
            } else if package_name.contains("xhs") || app_name.contains("小红书") {
                ("红".into(), rgb(0xE1, 0x1D, 0x48), ColorU::white())
            } else if package_name.contains("telegram") {
                ("TG".into(), rgb(0x02, 0x84, 0xC7), ColorU::white())
            } else {
                let first = app_name
                    .trim()
                    .chars()
                    .next()
                    .unwrap_or('推')
                    .to_uppercase()
                    .to_string();
                (first, rgb(0x63, 0x66, 0xF1), ColorU::white())
            }
        }
    }
}

fn category_badge_colors(cat: PushCategory, p: &Palette) -> (ColorU, ColorU) {
    match cat {
        PushCategory::Im => (p.success_text, p.success_soft),
        PushCategory::Finance => (p.warning, p.warning_soft),
        PushCategory::DeliveryTravel => (
            p.blue,
            if p.dark {
                rgba(0x3B, 0x82, 0xF6, 36)
            } else {
                rgb(0xEF, 0xF6, 0xFF)
            },
        ),
        PushCategory::Social => (
            p.pink,
            if p.dark {
                rgba(0xEC, 0x48, 0x99, 36)
            } else {
                rgb(0xFD, 0xF2, 0xF8)
            },
        ),
        PushCategory::SystemSecurity => (p.danger, p.danger_soft),
        PushCategory::All | PushCategory::Other => (p.text2, p.tag_bg),
    }
}

fn mask_notification_text(notif: &PushNotification) -> String {
    let app = if notif.app_name.is_empty() {
        "手机应用"
    } else {
        notif.app_name.as_str()
    };
    if notif_verification_code(notif).is_some() {
        format!("[{app}] 收到一条含验证码/取件码的通知（已开启防窥保护）")
    } else if !notif.title.trim().is_empty() {
        format!(
            "[{app}] 收到来自「{}」的新消息（已开启防窥保护）",
            notif.title.trim()
        )
    } else {
        format!("[{app}] 收到 1 条新推送通知（已开启防窥保护）")
    }
}

fn notif_verification_code(n: &PushNotification) -> Option<String> {
    n.otp_code
        .clone()
        .filter(|c| !c.trim().is_empty())
        .or_else(|| super::sms::extract_verification_code(&n.text))
        .or_else(|| super::sms::extract_verification_code(&n.title))
}

fn is_pickup_code(code: &str, n: &PushNotification) -> bool {
    super::sms::is_pickup_code_text(code, &n.text)
        || super::sms::is_pickup_code_text(code, &n.title)
        || super::sms::is_pickup_code_text(code, &n.app_name)
        || super::sms::is_pickup_code_text(code, &n.sender)
        || n.package_name.to_lowercase().contains("cainiao")
}

fn matches_notif_quick_group(n: &PushNotification, group: u8) -> bool {
    match group {
        1 => notif_verification_code(n).is_some(),
        2 => {
            matches!(
                PushCategory::from_str_code(&n.category),
                PushCategory::Im | PushCategory::Social
            ) || n.can_reply
        }
        3 => {
            n.is_ongoing
                || matches!(
                    PushCategory::from_str_code(&n.category),
                    PushCategory::SystemSecurity
                        | PushCategory::DeliveryTravel
                        | PushCategory::Finance
                        | PushCategory::Other
                )
        }
        _ => true,
    }
}

fn mix_color(a: ColorU, b: ColorU, t: f32) -> ColorU {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    if a.a == 0 {
        return ColorU::new(b.r, b.g, b.b, mix(0, b.a));
    }
    if b.a == 0 {
        return ColorU::new(a.r, a.g, a.b, mix(a.a, 0));
    }
    ColorU::new(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a))
}

impl Companion {
    pub(in super::super) fn notifications_actions(&self) -> Vec<El> {
        let mask_label = if self.push.prefs.privacy_mask {
            "显示原文"
        } else {
            "防窥保护"
        };
        vec![
            self.button(
                "push-toggle-privacy",
                mask_label,
                Action::Push(PushAction::TogglePrivacyMask),
                Btn::Secondary,
            ),
            self.button(
                "push-dismiss-all",
                "清空推送",
                Action::Push(PushAction::DismissAll),
                Btn::Secondary,
            ),
            self.button(
                "push-refresh",
                "刷新",
                Action::Push(PushAction::Refresh),
                Btn::Primary,
            ),
        ]
    }

    fn push_pill_btn(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        icon: Option<UiIcon>,
        active: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("push-pill:{}", id.into()));
        let label = label.into();
        let font = self.font;
        Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let t = if self.settings.value.reduced_motion {
                if hovered {
                    1.0
                } else {
                    0.0
                }
            } else {
                mouse.hover_progress().clamp(0.0, 1.0)
            };
            let (bg, border, fg, weight) = if active {
                let bg = if mouse.is_clicked() {
                    p.hover
                } else {
                    mix_color(p.accent_soft, p.hover, t * 0.5)
                };
                (bg, p.accent, p.text, Weight::Semibold)
            } else {
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else {
                    mix_color(p.surface, p.hover, t)
                };
                let border = if mouse.is_clicked() {
                    p.border_strong
                } else {
                    mix_color(p.border, p.border_strong, t)
                };
                let fg = if mouse.is_clicked() || hovered {
                    p.text
                } else {
                    mix_color(p.text2, p.text, t)
                };
                (bg, border, fg, Weight::Normal)
            };
            let text_el = ui_label(label.clone(), font, MICRO, fg, weight);
            let inner = match icon {
                Some(ic) => self.hstack(5., vec![self.ui_icon(ic, 13., fg), text_el]),
                None => text_el,
            };
            Container::new(inner)
                .with_horizontal_padding(10.)
                .with_vertical_padding(4.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    pub(in super::super) fn notifications_page(&self) -> El {
        let p = self.p();
        if !self.demo_mode && self.selected.is_none() {
            return Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.comm_mode_bar())
                .with_child(
                    Expanded::new(
                        1.,
                        self.empty(
                            "连接手机后查看全部推送",
                            "连接同一 Wi-Fi 或 USB 后，将实时同步手机通知栏的全部应用推送、验证码与快捷回复。",
                            Some(self.button(
                                "push-connect",
                                "连接设备",
                                Action::Page(Page::Connect),
                                Btn::Primary,
                            )),
                        ),
                    )
                    .finish(),
                )
                .finish();
        }

        let demo_fallback: PushSnapshot;
        let snapshot: &PushSnapshot = if self.demo_mode
            && self.push.snapshot.notifications.is_empty()
            && self.push.snapshot.threads.is_empty()
        {
            demo_fallback = demo_push_snapshot();
            &demo_fallback
        } else {
            &self.push.snapshot
        };

        let base_notifications: Vec<&PushNotification> = snapshot
            .notifications
            .iter()
            .filter(|n| {
                !n.dismissed
                    && !stepx_companion::push::should_hide_notification(n)
                    && (!self.push.collapse_ongoing || !n.is_ongoing)
            })
            .collect();

        let mut category_pills: Vec<El> = PushCategory::ALL
            .into_iter()
            .map(|cat| {
                let count = base_notifications
                    .iter()
                    .filter(|n| cat.matches(&n.category))
                    .count();
                self.push_pill_btn(
                    format!("cat-{}", cat.code()),
                    if count > 0 {
                        format!("{} {count}", cat.label())
                    } else {
                        cat.label().to_string()
                    },
                    None,
                    self.push.category == cat,
                    Action::Push(PushAction::SelectCategory(cat)),
                )
            })
            .collect();
        if let Some(app_flt) = &self.push.app_filter {
            category_pills.push(self.push_pill_btn(
                "clear-app-filter",
                format!("应用: {app_flt}"),
                Some(UiIcon::Close),
                true,
                Action::Push(PushAction::SelectAppFilter(None)),
            ));
        }

        let quick_group_tabs: [(u8, Option<UiIcon>, &str); 4] = [
            (0u8, None, "全部通知"),
            (1u8, Some(UiIcon::Key), "验证码/取件码"),
            (2u8, Some(UiIcon::Chat), "社交通讯"),
            (3u8, Some(UiIcon::Settings), "系统与服务"),
        ];
        let quick_group_pills: Vec<El> = quick_group_tabs
            .into_iter()
            .map(|(grp, icon, label)| {
                let count = base_notifications
                    .iter()
                    .filter(|n| matches_notif_quick_group(n, grp))
                    .count();
                self.push_pill_btn(
                    format!("notif-grp-{grp}"),
                    if count > 0 {
                        format!("{label} {count}")
                    } else {
                        label.to_string()
                    },
                    icon,
                    self.push.notif_category == grp,
                    Action::Push(PushAction::SetNotifCategory(grp)),
                )
            })
            .collect();

        // Top OTP & Pickup Code Summary Bar
        let top_otp_pickup_banner: Option<El> = if self.push.prefs.privacy_mask {
            None
        } else {
            let newest_otp = base_notifications
                .iter()
                .find_map(|n| notif_verification_code(n).filter(|code| !is_pickup_code(code, n)))
                .or_else(|| {
                    snapshot
                        .otp_banners
                        .iter()
                        .find(|b| !super::sms::is_pickup_code_text(&b.code, &b.raw_text))
                        .map(|b| b.code.clone())
                });

            let newest_pickup = base_notifications
                .iter()
                .find_map(|n| notif_verification_code(n).filter(|code| is_pickup_code(code, n)))
                .or_else(|| {
                    snapshot
                        .otp_banners
                        .iter()
                        .find(|b| super::sms::is_pickup_code_text(&b.code, &b.raw_text))
                        .map(|b| b.code.clone())
                });

            if newest_otp.is_some() || newest_pickup.is_some() {
                let mut left_items = vec![
                    self.dot(p.accent),
                    self.line_w("最新验证码 / 取件码速取", CAPTION, p.text, Weight::Semibold),
                ];
                let mut right_actions = Vec::new();
                if let Some(code) = &newest_otp {
                    left_items.push(self.pill_badge_icon(
                        format!("验证码 {code}"),
                        UiIcon::Key,
                        p.accent,
                        p.accent_soft,
                    ));
                    right_actions.push(self.button_icon(
                        "push-top-copy-otp",
                        format!("复制验证码 {code}"),
                        UiIcon::Clipboard,
                        Action::Push(PushAction::CopyOtp(code.clone())),
                        Btn::Primary,
                    ));
                }
                if let Some(pickup) = &newest_pickup {
                    left_items.push(self.pill_badge_icon(
                        format!("取件码 {pickup}"),
                        UiIcon::Package,
                        p.warning,
                        p.warning_soft,
                    ));
                    right_actions.push(self.button_icon(
                        "push-top-copy-pickup",
                        format!("复制取件码 {pickup}"),
                        UiIcon::Package,
                        Action::CopyText(pickup.clone(), format!("已复制取件码 {pickup}")),
                        if newest_otp.is_none() {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ));
                    right_actions.push(self.button_icon(
                        "push-top-send-backscreen",
                        "投送背屏灯牌",
                        UiIcon::Matrix,
                        Action::AmooPushPickupCode(pickup.clone()),
                        Btn::Secondary,
                    ));
                }
                Some(
                    Container::new(
                        self.between(self.hstack(8., left_items), self.hstack(8., right_actions)),
                    )
                    .with_horizontal_padding(S5)
                    .with_vertical_padding(S2)
                    .with_background_color(p.surface)
                    .with_border(Border::bottom(1.).with_border_color(p.border))
                    .finish(),
                )
            } else {
                None
            }
        };

        // Filter notifications for the feed
        let search_q = self.fields[F_PUSH_SEARCH].text.trim().to_lowercase();
        let filtered_notifications: Vec<&PushNotification> = base_notifications
            .into_iter()
            .filter(|n| matches_notif_quick_group(n, self.push.notif_category))
            .filter(|n| self.push.category.matches(&n.category))
            .filter(|n| {
                if let Some(app_flt) = &self.push.app_filter {
                    n.app_name == *app_flt || n.package_name == *app_flt
                } else {
                    true
                }
            })
            .filter(|n| {
                if search_q.is_empty() {
                    return true;
                }
                n.app_name.to_lowercase().contains(&search_q)
                    || n.package_name.to_lowercase().contains(&search_q)
                    || n.title.to_lowercase().contains(&search_q)
                    || n.sender.to_lowercase().contains(&search_q)
                    || n.text.to_lowercase().contains(&search_q)
                    || n.sub_text.to_lowercase().contains(&search_q)
                    || notif_verification_code(n)
                        .as_ref()
                        .is_some_and(|c| c.to_lowercase().contains(&search_q))
            })
            .collect();

        let selected_notif: Option<&PushNotification> = self
            .push
            .selected_key
            .as_deref()
            .and_then(|k| filtered_notifications.iter().copied().find(|n| n.key == k))
            .or_else(|| filtered_notifications.first().copied());
        let active_key = selected_notif.map(|n| n.key.as_str()).unwrap_or("");

        // 3. Left Column: Scrollable Notification Cards Feed
        let mut card_rows: Vec<El> = Vec::new();
        for (i, n) in filtered_notifications.iter().enumerate() {
            let is_selected = n.key == active_key;
            let app_label = if n.app_name.is_empty() {
                n.package_name.as_str()
            } else {
                n.app_name.as_str()
            };
            let (badge_txt, badge_bg, badge_fg) = app_badge_style(&n.package_name, app_label);
            let avatar = ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(self.line_w(badge_txt, CAPTION, badge_fg, Weight::Semibold))
                        .finish(),
                )
                .with_background_color(badge_bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(18.)))
                .finish(),
            )
            .with_width(36.)
            .with_height(36.)
            .finish();

            let cat_enum = PushCategory::from_str_code(&n.category);
            let (cat_fg, cat_bg) = category_badge_colors(cat_enum, &p);

            let detected_card_otp = notif_verification_code(n);
            let is_card_pickup = detected_card_otp
                .as_deref()
                .is_some_and(|code| is_pickup_code(code, n));
            let mut top_badges = vec![
                self.line_w(app_label, CAPTION, p.text, Weight::Semibold),
                self.pill_badge(cat_enum.label(), cat_fg, cat_bg),
            ];
            if n.can_reply {
                top_badges.push(self.pill_badge("可回复", p.success_text, p.success_soft));
            }
            if let Some(code) = &detected_card_otp {
                let otp_badge_text = if self.push.prefs.privacy_mask {
                    if is_card_pickup {
                        "含取件码".to_string()
                    } else {
                        "含验证码".to_string()
                    }
                } else if is_card_pickup {
                    format!("取件码 {code}")
                } else {
                    format!("验证码 {code}")
                };
                top_badges.push(self.pill_badge(otp_badge_text, p.warning, p.warning_soft));
            }
            if n.is_ongoing {
                top_badges.push(self.pill_badge("常驻", p.text3, p.tag_bg));
            }

            let time_label = super::format_time(n.post_time_ms);
            let header_line =
                self.between(self.wrap(top_badges), self.mono(time_label, MICRO, p.text3));

            let title_text = if !n.sender.is_empty() && n.sender != n.title && !n.title.is_empty() {
                format!("{} · {}", n.title, n.sender)
            } else if !n.title.is_empty() {
                n.title.clone()
            } else {
                app_label.to_string()
            };

            let display_body = if self.push.prefs.privacy_mask {
                mask_notification_text(n)
            } else {
                let single_line: String = n
                    .text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .collect::<Vec<_>>()
                    .join(" ");
                if single_line.chars().count() > 64 {
                    format!("{}…", single_line.chars().take(64).collect::<String>())
                } else {
                    single_line
                }
            };

            let mut right_col_items = vec![
                header_line,
                self.line_w(title_text, BODY, p.text, Weight::Medium),
                self.line(display_body, CAPTION, p.text2),
            ];
            let mut card_quick_btns: Vec<El> = Vec::new();
            if !self.push.prefs.privacy_mask {
                if let Some(code) = &detected_card_otp {
                    card_quick_btns.push(self.button_icon(
                        format!("push-card-otp-{i}"),
                        if is_card_pickup {
                            format!("复制取件码: {code}")
                        } else {
                            format!("复制验证码: {code}")
                        },
                        if is_card_pickup {
                            UiIcon::Package
                        } else {
                            UiIcon::Copy
                        },
                        Action::CopyText(
                            code.clone(),
                            if is_card_pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ),
                        Btn::Primary,
                    ));
                    if is_card_pickup {
                        card_quick_btns.push(self.button_icon(
                            format!("push-card-backscreen-{i}"),
                            "投送背屏",
                            UiIcon::Matrix,
                            Action::AmooPushPickupCode(code.clone()),
                            Btn::Secondary,
                        ));
                    }
                }
            }
            if is_selected {
                card_quick_btns.push(self.button_icon(
                    format!("push-card-mirror-{i}"),
                    "投屏查看",
                    UiIcon::Monitor,
                    Action::Mirror,
                    Btn::Ghost,
                ));
            }
            if !card_quick_btns.is_empty() {
                right_col_items.push(self.wrap(card_quick_btns));
            }

            let right_col = Flex::column()
                .with_spacing(4.)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_children(right_col_items)
                .finish();

            let row_inner = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(12.)
                .with_child(avatar)
                .with_child(Expanded::new(1., right_col).finish())
                .finish();

            let state = self.mouse_state(format!("push-row:{i}"));
            let notif_key = n.key.clone();
            let notif_key_dbl = n.key.clone();
            let notif_key_rc = n.key.clone();
            let otp_dbl = if self.push.prefs.privacy_mask {
                None
            } else {
                detected_card_otp.clone()
            };
            let row_el = Hoverable::new(state, move |mouse| {
                let hovered = mouse.is_mouse_over_element();
                let t = if self.settings.value.reduced_motion {
                    if hovered {
                        1.0
                    } else {
                        0.0
                    }
                } else {
                    mouse.hover_progress().clamp(0.0, 1.0)
                };
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else if is_selected {
                    mix_color(p.selected, p.hover, t * 0.5)
                } else {
                    mix_color(ColorU::transparent_black(), p.hover, t)
                };
                Container::new(row_inner)
                    .with_uniform_padding(12.)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            })
            .with_defer_events_to_children()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectNotification(
                    notif_key.clone(),
                )))
            })
            .on_double_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectNotification(
                    notif_key_dbl.clone(),
                )));
                if let Some(code) = &otp_dbl {
                    ctx.dispatch_typed_action(Action::CopyText(
                        code.clone(),
                        if is_card_pickup {
                            format!("已复制取件码 {code}")
                        } else {
                            format!("已复制验证码 {code}")
                        },
                    ));
                } else {
                    ctx.dispatch_typed_action(Action::Push(PushAction::OpenOnPhone(
                        notif_key_dbl.clone(),
                    )));
                }
            })
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectNotification(
                    notif_key_rc.clone(),
                )));
                ctx.dispatch_typed_action(Action::ContextOpen(
                    crate::ui::context_menu::Target::Notification(notif_key_rc.clone()),
                    at,
                ));
            })
            .finish();

            card_rows.push(row_el);
        }

        if let Some(err) = &self.push.error {
            card_rows.insert(
                0,
                self.empty(
                    "手机推送同步异常",
                    err.clone(),
                    Some(self.button(
                        "push-retry-err",
                        "重试同步",
                        Action::Push(PushAction::Refresh),
                        Btn::Secondary,
                    )),
                ),
            );
        } else if card_rows.is_empty() {
            card_rows.push(self.empty(
                if self.push.syncing {
                    "正在同步手机通知栏推送…"
                } else if snapshot.notifications.is_empty() {
                    "手机通知栏暂无推送消息"
                } else {
                    "没有符合筛选条件的推送"
                },
                if !snapshot.listener_connected {
                    "请开启通知监听权限，同步手机允许共享的通知。"
                } else if snapshot.notifications.is_empty() {
                    "手机收到微信、钉钉、飞书、验证码或账单通知时将在此实时呈现。"
                } else {
                    "请切换上方分类标签、应用过滤或清空搜索关键词。"
                },
                if !snapshot.listener_connected {
                    Some(self.button(
                        "push-empty-grant",
                        "开启通知监听权限",
                        Action::Push(PushAction::EnsurePermission),
                        Btn::Primary,
                    ))
                } else {
                    None
                },
            ));
        }

        let left_top_controls = Container::new(self.vstack(
            8.,
            vec![
                self.input(F_PUSH_SEARCH, "搜索推送通知或验证码…", None),
                self.wrap(quick_group_pills),
                self.wrap(category_pills),
                self.between(
                    self.checkbox_row(
                        "push-collapse-ongoing",
                        self.push.collapse_ongoing,
                        "折叠常驻",
                        Action::Push(PushAction::ToggleCollapseOngoing),
                    ),
                    self.checkbox_row(
                        "push-auto-copy-otp",
                        self.push.prefs.auto_copy_otp,
                        "自动复制验证码",
                        Action::Push(PushAction::ToggleAutoCopyOtp),
                    ),
                ),
            ],
        ))
        .with_horizontal_padding(S4)
        .with_vertical_padding(S3)
        .with_border(Border::bottom(1.).with_border_color(p.border))
        .finish();

        let left_feed = self.pane_scroll(
            self.push_list_scroll.clone(),
            Container::new(self.vstack(2., card_rows))
                .with_horizontal_padding(S2)
                .with_vertical_padding(S2)
                .finish(),
        );

        let left_column = self.fixed_w(
            LIST_PANE_W + 40.,
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(left_top_controls)
                    .with_child(Expanded::new(1., left_feed).finish())
                    .finish(),
            )
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::right(1.).with_border_color(p.border))
            .finish(),
        );

        // 4. Right Column: Selected Notification Detail & Direct Inline Reply
        let right_column = if let Some(n) = selected_notif {
            let app_label = if n.app_name.is_empty() {
                n.package_name.as_str()
            } else {
                n.app_name.as_str()
            };
            let (badge_txt, badge_bg, badge_fg) = app_badge_style(&n.package_name, app_label);
            let detail_avatar = ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(self.line_w(badge_txt, SUBHEAD, badge_fg, Weight::Semibold))
                        .finish(),
                )
                .with_background_color(badge_bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(20.)))
                .finish(),
            )
            .with_width(40.)
            .with_height(40.)
            .finish();

            let cat_enum = PushCategory::from_str_code(&n.category);
            let (cat_fg, cat_bg) = category_badge_colors(cat_enum, &p);

            let mut meta_badges = vec![
                self.pill_badge(cat_enum.label(), cat_fg, cat_bg),
                self.pill_badge(
                    if n.is_ongoing {
                        "常驻后台通知"
                    } else if n.is_clearable {
                        "仍在手机通知栏"
                    } else {
                        "系统通知"
                    },
                    p.text2,
                    p.tag_bg,
                ),
            ];
            if n.can_reply {
                meta_badges.push(self.pill_badge("支持通知回复", p.success_text, p.success_soft));
            }

            let detected_detail_otp = notif_verification_code(n);
            let is_detail_pickup = detected_detail_otp
                .as_deref()
                .is_some_and(|code| is_pickup_code(code, n));
            let is_im_notif =
                !n.thread_id.is_empty() || n.category.eq_ignore_ascii_case("im") || n.can_reply;
            let mut action_btns: Vec<El> = Vec::new();

            if let Some(code) = &detected_detail_otp {
                let otp_btn_label = if self.push.prefs.privacy_mask {
                    if is_detail_pickup {
                        "复制取件码".to_string()
                    } else {
                        "复制验证码".to_string()
                    }
                } else if is_detail_pickup {
                    format!("复制取件码: {code}")
                } else {
                    format!("复制验证码: {code}")
                };
                action_btns.push(self.button_icon(
                    "push-detail-copy-otp",
                    otp_btn_label,
                    if is_detail_pickup {
                        UiIcon::Package
                    } else {
                        UiIcon::Copy
                    },
                    if is_detail_pickup {
                        Action::CopyText(code.clone(), format!("已复制取件码 {code}"))
                    } else {
                        Action::Push(PushAction::CopyOtp(code.clone()))
                    },
                    Btn::Primary,
                ));
            }
            if let Some(url) = &n.url_hint {
                action_btns.push(self.button(
                    "push-detail-open-url",
                    "打开链接",
                    Action::Push(PushAction::OpenUrl(url.clone())),
                    Btn::Secondary,
                ));
            }
            if is_im_notif {
                let target_tid = if n.thread_id.trim().is_empty() {
                    format!("{}::{}", n.package_name, n.title)
                } else {
                    n.thread_id.clone()
                };
                let state = self.mouse_state("push-jump-im-dialog");
                let font = self.font;
                let jump_btn = Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let t = if self.settings.value.reduced_motion {
                        if hovered {
                            1.0
                        } else {
                            0.0
                        }
                    } else {
                        mouse.hover_progress().clamp(0.0, 1.0)
                    };
                    let bg = if mouse.is_clicked() {
                        p.hover
                    } else {
                        mix_color(p.accent_soft, p.hover, t)
                    };
                    let border = if mouse.is_clicked() {
                        p.border_strong
                    } else {
                        mix_color(p.accent, p.border_strong, t)
                    };
                    Container::new(
                        ConstrainedBox::new(
                            Align::new(
                                facloud_ui::elements::Text::new("打开聊天会话", font, BODY)
                                    .with_color(p.text)
                                    .with_style(
                                        facloud_ui::fonts::Properties::default()
                                            .weight(Weight::Medium),
                                    )
                                    .soft_wrap(false)
                                    .finish(),
                            )
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
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::SelectSmsThread(None));
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImChannel(
                        ImChannel::All,
                    )));
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                        target_tid.clone(),
                    )));
                    ctx.dispatch_typed_action(Action::Page(Page::Sms));
                })
                .finish();
                action_btns.push(jump_btn);
            }
            if is_detail_pickup {
                if let Some(code) = &detected_detail_otp {
                    action_btns.push(self.button_icon(
                        "push-detail-send-backscreen",
                        "投送背屏灯牌",
                        UiIcon::Matrix,
                        Action::AmooPushPickupCode(code.clone()),
                        Btn::Secondary,
                    ));
                }
            }
            action_btns.push(self.button(
                "push-open-phone",
                "手机打开",
                Action::Push(PushAction::OpenOnPhone(n.key.clone())),
                Btn::Secondary,
            ));
            action_btns.push(self.button_icon(
                "push-open-mirror",
                "投屏查看",
                UiIcon::Monitor,
                Action::Mirror,
                Btn::Secondary,
            ));
            action_btns.push(self.button_icon(
                "push-copy-text",
                "复制内容",
                UiIcon::Clipboard,
                Action::CopyText(n.text.clone(), "已复制推送正文".into()),
                Btn::Secondary,
            ));
            action_btns.push(self.button(
                "push-dismiss-one",
                "清除此条",
                Action::Push(PushAction::DismissNotification(n.key.clone())),
                Btn::Ghost,
            ));

            let detail_header = self.panel(vec![
                self.between(
                    self.hstack(
                        12.,
                        vec![
                            detail_avatar,
                            self.vstack(
                                4.,
                                vec![
                                    self.hstack(
                                        8.,
                                        vec![
                                            self.line_w(
                                                if n.title.is_empty() {
                                                    app_label
                                                } else {
                                                    n.title.as_str()
                                                },
                                                SUBHEAD,
                                                p.text,
                                                Weight::Semibold,
                                            ),
                                            self.mono(&n.package_name, MICRO, p.text3),
                                        ],
                                    ),
                                    self.wrap(meta_badges),
                                ],
                            ),
                        ],
                    ),
                    self.mono(super::format_time(n.post_time_ms), CAPTION, p.text2),
                ),
                self.divider(),
                self.vstack(
                    S2,
                    vec![
                        if !n.sender.is_empty() && n.sender != n.title {
                            self.line_w(
                                format!("发送人：{}", n.sender),
                                CAPTION,
                                p.text2,
                                Weight::Medium,
                            )
                        } else {
                            Empty::new().finish()
                        },
                        self.text(
                            if self.push.prefs.privacy_mask {
                                mask_notification_text(n)
                            } else {
                                n.text.clone()
                            },
                            BODY,
                            p.text,
                        ),
                        if !n.sub_text.trim().is_empty() {
                            self.line(format!("补充信息：{}", n.sub_text), MICRO, p.text3)
                        } else {
                            Empty::new().finish()
                        },
                    ],
                ),
                self.divider(),
                self.wrap(action_btns),
            ]);

            let mut detail_sections: Vec<El> = vec![detail_header];

            // Direct Inline Reply Box when `can_reply` is true
            if n.can_reply {
                let reply_tid = if n.thread_id.trim().is_empty() {
                    format!("{}::{}", n.package_name, n.title)
                } else {
                    n.thread_id.clone()
                };

                let quick_phrases = ["收到", "好的，马上处理", "稍后回复"];
                let mut chip_els: Vec<El> = quick_phrases
                    .into_iter()
                    .enumerate()
                    .map(|(idx, phrase)| {
                        self.button(
                            format!("push-quick-reply-{idx}"),
                            phrase,
                            Action::Push(PushAction::QuickReplyPhrase {
                                thread_id: reply_tid.clone(),
                                package_name: n.package_name.clone(),
                                title: n.title.clone(),
                                text: phrase.to_string(),
                            }),
                            Btn::Secondary,
                        )
                    })
                    .collect();

                chip_els.push(self.button(
                    "push-ai-draft",
                    "建议回复",
                    Action::Push(PushAction::AiDraftReply {
                        thread_id: reply_tid.clone(),
                        package_name: n.package_name.clone(),
                        title: n.title.clone(),
                    }),
                    Btn::Secondary,
                ));

                let mut reply_items = vec![
                    self.between(
                        self.hstack(S2, vec![self.dot(p.success), self.heading("内联回复")]),
                        self.pill_badge(app_label, p.success_text, p.success_soft),
                    ),
                    self.wrap(chip_els),
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(S2)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(
                            Expanded::new(
                                1.,
                                self.input(F_PUSH_REPLY, "输入回复内容，按回车发送…", None),
                            )
                            .finish(),
                        )
                        .with_child(self.button(
                            "push-inline-send",
                            "发送",
                            Action::Push(PushAction::ReplyInline {
                                key: n.key.clone(),
                                thread_id: reply_tid,
                                package_name: n.package_name.clone(),
                                title: n.title.clone(),
                            }),
                            Btn::Primary,
                        ))
                        .finish(),
                ];

                if let Some(status) = &self.push.last_reply_status {
                    reply_items.push(self.line(status, CAPTION, p.success_text));
                }

                detail_sections.push(self.panel(reply_items));
            }

            self.pane_scroll(
                self.push_detail_scroll.clone(),
                Container::new(self.vstack(S4, detail_sections))
                    .with_horizontal_padding(S5)
                    .with_padding_top(S5)
                    .with_padding_bottom(S6)
                    .finish(),
            )
        } else {
            Container::new(
                Align::new(self.empty(
                    "选择一条手机推送查看详情",
                    "点击左侧列表中的任意推送，即可查看完整正文、复制验证码或内联回复。",
                    None,
                ))
                .finish(),
            )
            .with_uniform_padding(S5)
            .finish()
        };

        let split = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(left_column)
            .with_child(Expanded::new(1., right_column).finish())
            .finish();

        let mut page_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.comm_mode_bar());
        if let Some(banner) = top_otp_pickup_banner {
            page_col = page_col.with_child(banner);
        }
        page_col
            .with_child(Expanded::new(1., self.sub_enter(split)).finish())
            .finish()
    }
}
