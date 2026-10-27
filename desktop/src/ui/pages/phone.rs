use super::super::{
    icons::UiIcon, theme::*, widgets::*, Action, Companion, Page, PushAction, F_CONTACT_SEARCH,
    F_PHONE_CONTACT_NAME, F_PHONE_DIAL,
};
use chrono::{Local, TimeZone};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use stepx_companion::device::ContactEntry;

const DIAL_KEYS: [(&str, &str); 12] = [
    ("1", ""),
    ("2", "ABC"),
    ("3", "DEF"),
    ("4", "GHI"),
    ("5", "JKL"),
    ("6", "MNO"),
    ("7", "PQRS"),
    ("8", "TUV"),
    ("9", "WXYZ"),
    ("*", ""),
    ("0", "+"),
    ("#", ""),
];

struct DisplayCallRow {
    name: String,
    num: String,
    outgoing: bool,
    missed: bool,
    label: String,
    time: String,
}

fn norm_phone(s: &str) -> String {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.len() == 13 && digits.starts_with("861") {
        return digits[2..].to_string();
    }
    if digits.len() == 15 && digits.starts_with("00861") {
        return digits[4..].to_string();
    }
    if digits.is_empty() {
        stepx_companion::sms::normalize_address(s)
    } else {
        digits
    }
}

fn format_call_time(ms: i64) -> String {
    if ms <= 0 {
        return "时间未知".into();
    }
    if let Some(dt) = Local.timestamp_millis_opt(ms).single() {
        dt.format("%m-%d %H:%M").to_string()
    } else {
        "时间未知".into()
    }
}

fn contact_badge(name: &str) -> String {
    name.trim()
        .chars()
        .next()
        .unwrap_or('联')
        .to_uppercase()
        .to_string()
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
    pub(in super::super) fn phone_actions(&self) -> Vec<El> {
        vec![self.button("ph-refresh", "同步记录", Action::ReadPhone, Btn::Secondary)]
    }

    pub(in super::super) fn filtered_contacts(&self) -> Vec<ContactEntry> {
        let q = self.fields[F_CONTACT_SEARCH].text.trim().to_lowercase();
        if q.is_empty() {
            return self.contacts.clone();
        }
        self.contacts
            .iter()
            .filter(|c| c.name.to_lowercase().contains(&q) || c.number.to_lowercase().contains(&q))
            .cloned()
            .collect()
    }

    pub(in super::super) fn phone_page(&self) -> El {
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
                            "连接手机后使用电话与通讯录",
                            "连接后可同步手机通话记录、实机通讯录，并支持从桌面直接发起拨号。",
                            Some(self.button(
                                "phone-connect",
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
        let all_contacts = self.filtered_contacts();
        let loaded = self.phone_loaded
            || self.demo_mode
            || !self.call_logs.is_empty()
            || !self.contacts.is_empty();

        // Only records returned by the phone belong in this list.
        let mut all_calls: Vec<DisplayCallRow> = Vec::new();
        for r in &self.call_logs {
            let has_valid_name =
                !r.name.trim().is_empty() && !r.name.trim().eq_ignore_ascii_case("null");
            let resolved_name = if has_valid_name {
                r.name.clone()
            } else {
                let norm_r = norm_phone(&r.number);
                if let Some(c) = self
                    .contacts
                    .iter()
                    .find(|c| norm_phone(&c.number) == norm_r)
                {
                    c.name.clone()
                } else if r.number.is_empty() {
                    "隐藏号码".into()
                } else {
                    r.number.clone()
                }
            };
            all_calls.push(DisplayCallRow {
                name: resolved_name,
                num: r.number.clone(),
                outgoing: r.call_type == 2,
                missed: r.call_type == 3,
                label: format!("{} · {}", r.type_label(), r.format_duration()),
                time: format_call_time(r.date_ms),
            });
        }
        let missed_count = all_calls.iter().filter(|c| c.missed).count();
        let incoming_count = all_calls
            .iter()
            .filter(|c| !c.outgoing && !c.missed)
            .count();
        let outgoing_count = all_calls.iter().filter(|c| c.outgoing).count();
        let total_calls_count = all_calls.len();
        let active_call_filter = if self.push.call_filter != 0 {
            self.push.call_filter
        } else if self.call_filter_missed {
            1
        } else {
            0
        };
        let show_counts = loaded && self.call_logs_error.is_none();

        // 1. Col 1-4: Call History (`通话记录`)
        let filter_pills = self.wrap(vec![
            self.button(
                "ph-flt-all",
                if show_counts {
                    format!("全部 ({total_calls_count})")
                } else {
                    "全部".into()
                },
                Action::Push(PushAction::SetCallFilter(0)),
                if active_call_filter == 0 {
                    Btn::Primary
                } else {
                    Btn::Ghost
                },
            ),
            self.button(
                "ph-flt-missed",
                if show_counts {
                    format!("未接来电 ({missed_count})")
                } else {
                    "未接来电".into()
                },
                Action::Push(PushAction::SetCallFilter(1)),
                if active_call_filter == 1 {
                    Btn::Primary
                } else {
                    Btn::Ghost
                },
            ),
            self.button(
                "ph-flt-incoming",
                if show_counts {
                    format!("呼入 ({incoming_count})")
                } else {
                    "呼入".into()
                },
                Action::Push(PushAction::SetCallFilter(2)),
                if active_call_filter == 2 {
                    Btn::Primary
                } else {
                    Btn::Ghost
                },
            ),
            self.button(
                "ph-flt-outgoing",
                if show_counts {
                    format!("呼出 ({outgoing_count})")
                } else {
                    "呼出".into()
                },
                Action::Push(PushAction::SetCallFilter(3)),
                if active_call_filter == 3 {
                    Btn::Primary
                } else {
                    Btn::Ghost
                },
            ),
        ]);

        let col1_header =
            Container::new(self.vstack(8., vec![self.heading("通话记录"), filter_pills]))
                .with_uniform_padding(14.)
                .with_border(Border::bottom(1.).with_border_color(p.border))
                .finish();

        let search_q = self.fields[F_CONTACT_SEARCH].text.trim().to_lowercase();
        let mut call_rows: Vec<El> = Vec::new();
        for (i, c) in all_calls.iter().enumerate() {
            let matches_filter = match active_call_filter {
                1 => c.missed,
                2 => !c.outgoing && !c.missed,
                3 => c.outgoing,
                _ => true,
            };
            let matches_search = search_q.is_empty()
                || c.name.to_lowercase().contains(&search_q)
                || c.num.to_lowercase().contains(&search_q);
            if !matches_filter || !matches_search {
                continue;
            }
            let arrow_icon = if c.outgoing {
                UiIcon::Upload
            } else {
                UiIcon::Download
            };
            let arrow_color = if c.missed {
                p.danger
            } else if c.outgoing {
                p.success
            } else {
                p.text2
            };

            let mut info_lines = vec![self.line_w(
                &c.name,
                CAPTION,
                if c.missed { p.danger } else { p.text },
                Weight::Semibold,
            )];
            if !c.num.is_empty() && c.name != c.num {
                info_lines.push(self.mono(&c.num, CAPTION, p.text2));
            }
            info_lines.push(self.line(&c.label, CAPTION, p.text2));

            let left_info = Flex::row()
                .with_spacing(10.)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(self.ui_icon(arrow_icon, 14., arrow_color))
                .with_child(
                    Flex::column()
                        .with_spacing(2.)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_children(info_lines)
                        .finish(),
                )
                .finish();

            let cb_state = self.mouse_state(format!("ph-cb:{i}"));
            let num_str = c.num.clone();
            let cb_btn = Hoverable::new(cb_state, move |mouse| {
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
                } else {
                    mix_color(p.surface, p.hover, t)
                };
                let border = if mouse.is_clicked() {
                    p.success
                } else {
                    mix_color(p.surface_border, p.success, t)
                };
                ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(self.ui_icon(UiIcon::Phone, 14., p.text2))
                            .finish(),
                    )
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
                    .finish(),
                )
                .with_width(28.)
                .with_height(28.)
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::PhoneCall(num_str.clone()))
            })
            .finish();

            let mut row_btns: Vec<El> = Vec::new();
            if !c.num.trim().is_empty() {
                row_btns.push(self.button(
                    format!("ph-call-copy-{i}"),
                    "复制",
                    Action::CopyText(c.num.clone(), format!("已复制号码 {}", c.num)),
                    Btn::Ghost,
                ));
                row_btns.push(self.button(
                    format!("ph-call-sms-{i}"),
                    "短信",
                    Action::PhoneSmsTo(c.num.clone()),
                    Btn::Ghost,
                ));
            }
            row_btns.push(cb_btn);

            let right_info = Flex::column()
                .with_spacing(6.)
                .with_cross_axis_alignment(CrossAxisAlignment::End)
                .with_child(self.mono(&c.time, CAPTION, p.text2))
                .with_child(self.hstack(4., row_btns))
                .finish();

            let row_state = self.mouse_state(format!("ph-call-row:{i}"));
            let num_click = c.num.clone();
            let num_dbl = c.num.clone();
            let name_rc = c.name.clone();
            let num_rc = c.num.clone();
            let row_body = self.between(left_info, right_info);
            let call_row_el = Hoverable::new(row_state, move |mouse| {
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
                } else {
                    mix_color(ColorU::transparent_black(), p.hover, t)
                };
                Container::new(row_body)
                    .with_horizontal_padding(S3)
                    .with_vertical_padding(10.)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            })
            .with_defer_events_to_children()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                if !num_click.is_empty() {
                    ctx.dispatch_typed_action(Action::PhoneDialNumber(num_click.clone()));
                }
            })
            .on_double_click(move |ctx, _, _| {
                if !num_dbl.is_empty() {
                    ctx.dispatch_typed_action(Action::PhoneCall(num_dbl.clone()));
                }
            })
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::ContextOpen(
                    crate::ui::context_menu::Target::PhoneEntry {
                        name: name_rc.clone(),
                        number: num_rc.clone(),
                    },
                    at,
                ));
            })
            .finish();

            call_rows.push(call_row_el);
        }

        let col1_scroll = if let Some(error) = &self.call_logs_error {
            self.phone_empty(
                "通话记录读取失败",
                error,
                Some(("ph-retry-calls", "重新读取", Action::ReadPhone)),
            )
        } else if call_rows.is_empty() {
            self.phone_empty(
                if !loaded {
                    "正在读取通话记录…"
                } else {
                    match active_call_filter {
                        1 => "暂无未接来电",
                        2 => "暂无呼入通话记录",
                        3 => "暂无呼出通话记录",
                        _ => "暂无通话记录",
                    }
                },
                if !loaded {
                    "正在与手机同步"
                } else {
                    "这里显示手机中的通话记录"
                },
                None,
            )
        } else {
            self.pane_scroll(
                self.scroll.clone(),
                Container::new(self.vstack(2., call_rows))
                    .with_horizontal_padding(S2)
                    .with_padding_top(S2)
                    .with_padding_bottom(S3)
                    .finish(),
            )
        };

        let col1_card = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(col1_header)
                .with_child(Expanded::new(1., col1_scroll).finish())
                .finish(),
        )
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        // 2. Col 5-8: Contacts Directory & Contact Detail Card
        let col2_top = Container::new(self.vstack(
            8.,
            vec![
                self.between(
                    self.heading(if loaded && self.contacts_error.is_none() {
                        format!("通讯录联系人 ({})", all_contacts.len())
                    } else {
                        "通讯录联系人".into()
                    }),
                    self.line("A–Z · 实机同步", MICRO, p.text2),
                ),
                self.input(
                    F_CONTACT_SEARCH,
                    "搜索姓名/手机号，或输入新联系人姓名…",
                    None,
                ),
            ],
        ))
        .with_uniform_padding(12.)
        .with_border(Border::bottom(1.).with_border_color(p.border))
        .finish();

        let fallback_contact = ContactEntry {
            name: "暂无联系人".into(),
            number: String::new(),
        };
        let active_idx = if all_contacts.is_empty() {
            0
        } else {
            self.selected_contact.min(all_contacts.len() - 1)
        };
        let sel = all_contacts.get(active_idx).unwrap_or(&fallback_contact);
        let sel_badge = contact_badge(&sel.name);

        let sel_avatar = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(self.line_w(&sel_badge, CAPTION, p.accent_fg, Weight::Semibold))
                    .finish(),
            )
            .with_background_color(p.accent)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(18.)))
            .finish(),
        )
        .with_width(36.)
        .with_height(36.)
        .finish();

        let detail_box = Container::new(self.vstack(
            10.,
            vec![
                self.hstack(
                    10.,
                    vec![
                        sel_avatar,
                        Flex::column()
                            .with_spacing(2.)
                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                            .with_child(self.line_w(&sel.name, CAPTION, p.text, Weight::Semibold))
                            .with_child(self.mono(&sel.number, CAPTION, p.text2))
                            .finish(),
                    ],
                ),
                self.wrap(vec![
                    self.button(
                        "ph-sel-copy",
                        "复制",
                        Action::CopyText(
                            sel.number.clone(),
                            format!("已复制联系人号码 {}", sel.number),
                        ),
                        Btn::Ghost,
                    ),
                    self.button(
                        "ph-sel-sms",
                        "短信",
                        Action::PhoneSmsTo(sel.number.clone()),
                        Btn::Secondary,
                    ),
                    self.button(
                        "ph-sel-call",
                        "呼叫",
                        Action::PhoneCall(sel.number.clone()),
                        Btn::Primary,
                    ),
                ]),
            ],
        ))
        .with_uniform_padding(14.)
        .with_background_color(p.surface)
        .with_border(Border::bottom(1.).with_border_color(p.border))
        .finish();

        let mut contact_rows: Vec<El> = Vec::new();
        for (i, ct) in all_contacts.iter().enumerate() {
            let active = i == active_idx;
            let badge = contact_badge(&ct.name);
            let badge_el = ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(self.line_w(
                            &badge,
                            CAPTION,
                            if p.dark {
                                p.text2
                            } else {
                                rgb(0x52, 0x52, 0x5B)
                            },
                            Weight::Semibold,
                        ))
                        .finish(),
                )
                .with_background_color(if p.dark {
                    rgb(0x27, 0x27, 0x2A)
                } else {
                    rgb(0xEF, 0xEF, 0xF2)
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
                .finish(),
            )
            .with_width(28.)
            .with_height(28.)
            .finish();

            let row_inner = self.between(
                self.hstack(
                    10.,
                    vec![
                        badge_el,
                        self.line_w(
                            &ct.name,
                            CAPTION,
                            p.text,
                            if active {
                                Weight::Semibold
                            } else {
                                Weight::Medium
                            },
                        ),
                    ],
                ),
                self.mono(&ct.number, CAPTION, p.text2),
            );
            let state = self.mouse_state(format!("ph-ct-row:{i}"));
            let ct_num_dbl = ct.number.clone();
            let ct_name_rc = ct.name.clone();
            let ct_num_rc = ct.number.clone();
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
                let active_bg = if p.dark {
                    rgb(0x22, 0x22, 0x28)
                } else {
                    rgb(0xF4, 0xF4, 0xF7)
                };
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else if active {
                    mix_color(active_bg, p.hover, t * 0.5)
                } else {
                    mix_color(ColorU::transparent_black(), p.hover, t)
                };
                Container::new(row_inner)
                    .with_horizontal_padding(S3)
                    .with_vertical_padding(9.)
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(Action::PhoneSelectContact(i)))
            .on_double_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::PhoneSelectContact(i));
                if !ct_num_dbl.is_empty() {
                    ctx.dispatch_typed_action(Action::PhoneCall(ct_num_dbl.clone()));
                }
            })
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::PhoneSelectContact(i));
                ctx.dispatch_typed_action(Action::ContextOpen(
                    crate::ui::context_menu::Target::PhoneEntry {
                        name: ct_name_rc.clone(),
                        number: ct_num_rc.clone(),
                    },
                    at,
                ));
            })
            .finish();
            contact_rows.push(row_el);
        }

        let col2_scroll = if let Some(error) = &self.contacts_error {
            self.phone_empty(
                "通讯录读取失败",
                error,
                Some(("ph-retry-contacts", "重新读取", Action::ReadPhone)),
            )
        } else if contact_rows.is_empty() {
            let searching = !self.fields[F_CONTACT_SEARCH].text.trim().is_empty();
            self.phone_empty(
                if !loaded {
                    "正在读取通讯录…"
                } else if searching {
                    "未找到匹配的联系人"
                } else {
                    "暂无联系人"
                },
                if !loaded {
                    "正在与手机同步"
                } else if searching {
                    "若需将该姓名存为新联系人，请在右侧输入号码后点击下方按钮"
                } else {
                    "在上方输入姓名并在右侧输入号码后可新建联系人"
                },
                if !loaded {
                    None
                } else {
                    Some((
                        "ph-empty-contact",
                        "存为新联系人",
                        Action::PhoneCreateContact,
                    ))
                },
            )
        } else {
            self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(self.vstack(2., contact_rows))
                    .with_horizontal_padding(S2)
                    .with_padding_top(S2)
                    .with_padding_bottom(S3)
                    .finish(),
            )
        };
        let mut contact_content = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(col2_top);
        if !all_contacts.is_empty() {
            contact_content = contact_content.with_child(detail_box);
        }
        let col2_card = Container::new(
            contact_content
                .with_child(Expanded::new(1., col2_scroll).finish())
                .finish(),
        )
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        // 3. Col 9-12: Desktop Dialpad & Active Call Controller
        let dial_number = self.fields[F_PHONE_DIAL].text.trim().to_string();
        let norm_dial = norm_phone(&dial_number);

        let matched_contact_name = (!dial_number.is_empty())
            .then(|| {
                self.contacts
                    .iter()
                    .find(|c| norm_phone(&c.number) == norm_dial)
                    .map(|c| format!("已匹配：{}", c.name))
            })
            .flatten()
            .unwrap_or_else(|| {
                if dial_number.is_empty() {
                    "输入号码或选择联系人".into()
                } else {
                    "未匹配联系人".into()
                }
            });

        let carrier_status = self
            .capabilities
            .as_ref()
            .map(|c| {
                c.subscriptions
                    .iter()
                    .find(|s| Some(s.id) == self.selected_call_sim)
                    .map(|s| {
                        format!(
                            "{} · 卡 {}",
                            stepx_companion::bridge::friendly_carrier_name(&s.label),
                            s.slot.max(1)
                        )
                    })
                    .unwrap_or_else(|| {
                        if c.subscriptions.is_empty() {
                            "未检测到通话 SIM".into()
                        } else {
                            "使用系统默认通话卡".into()
                        }
                    })
            })
            .unwrap_or_else(|| "SIM 信息未读取".into());

        let sim_choices: Vec<El> = self
            .capabilities
            .as_ref()
            .map(|caps| {
                caps.subscriptions
                    .iter()
                    .map(|sim| {
                        self.button(
                            format!("ph-sim-{}", sim.id),
                            format!(
                                "卡 {} · {}",
                                sim.slot.max(1),
                                stepx_companion::bridge::friendly_carrier_name(&sim.label)
                            ),
                            Action::PhoneSelectSim(sim.id),
                            if self.selected_call_sim == Some(sim.id) {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();

        let mut dial_top_rows = vec![
            self.between(
                self.line("桌面拨号", CAPTION, p.text2),
                self.hstack(
                    8.,
                    vec![
                        self.link_action("ph-dial-bksp", "退格", Action::PhoneDialBackspace),
                        self.link_action("ph-dial-clr", "清空", Action::PhoneDialClear),
                    ],
                ),
            ),
            self.input(F_PHONE_DIAL, "输入电话号码…", None),
            self.between(
                if sim_choices.is_empty() {
                    self.line(&carrier_status, MICRO, p.text2)
                } else {
                    self.line("选择通话 SIM 卡", MICRO, p.text2)
                },
                self.line_w(&matched_contact_name, MICRO, p.text2, Weight::Medium),
            ),
        ];
        if !sim_choices.is_empty() {
            dial_top_rows.push(self.wrap(sim_choices));
        }

        let num_display_box = Container::new(
            Flex::column()
                .with_spacing(4.)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_children(dial_top_rows)
                .finish(),
        )
        .with_uniform_padding(12.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.surface_border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let mut keypad_rows = Vec::with_capacity(4);
        for chunk in DIAL_KEYS.chunks(3) {
            let mut row_items = Vec::with_capacity(3);
            for &(digit, sub) in chunk {
                let state = self.mouse_state(format!("ph-pad:{digit}"));
                let d_str = digit.to_string();
                let mut col_children = vec![self.mono(digit, 14., p.text)];
                if !sub.is_empty() {
                    col_children.push(self.mono(sub, 10., p.text2));
                }
                let inner_col = Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(2.)
                    .with_children(col_children)
                    .finish();
                let key_btn = Hoverable::new(state, move |mouse| {
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
                    } else {
                        mix_color(p.surface, p.hover, t)
                    };
                    let border = if mouse.is_clicked() {
                        p.border_strong
                    } else {
                        mix_color(p.surface_border, p.border_strong, t)
                    };
                    ConstrainedBox::new(
                        Container::new(inner_col)
                            .with_background_color(bg)
                            .with_border(Border::all(1.).with_border_color(border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish(),
                    )
                    .with_height(40.)
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::PhoneDialAppend(d_str.clone()))
                })
                .finish();
                row_items.push(Expanded::new(1., key_btn).finish());
            }
            keypad_rows.push(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_spacing(6.)
                    .with_children(row_items)
                    .finish(),
            );
        }

        let mut bottom_call_items: Vec<El> = Vec::new();

        if let Some(active_num) = self.active_call_number.as_ref().filter(|_| !self.demo_mode) {
            let norm_active = norm_phone(active_num);
            let peer_label = self
                .contacts
                .iter()
                .find(|c| norm_phone(&c.number) == norm_active)
                .map(|c| format!("{} · {}", c.name, active_num))
                .unwrap_or_else(|| active_num.clone());
            bottom_call_items.push(
                Container::new(self.vstack(
                    S2,
                    vec![
                        self.hstack(
                            S2,
                            vec![
                                self.dot(p.success),
                                self.line_w("已提交呼叫", CAPTION, p.text, Weight::Medium),
                            ],
                        ),
                        self.text(
                            format!("{peer_label} · 通话状态以手机为准"),
                            CAPTION,
                            p.text2,
                        ),
                        self.wrap(vec![
                            self.button(
                                "ph-active-sms",
                                "发短信",
                                Action::PhoneSmsTo(active_num.clone()),
                                Btn::Secondary,
                            ),
                            self.button("ph-hangup", "挂断", Action::PhoneHangup, Btn::DangerSolid),
                        ]),
                    ],
                ))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S2)
                .with_background_color(p.success_soft)
                .with_border(Border::all(1.).with_border_color(rgba(0x10, 0xB9, 0x81, 76)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        }

        if let Some(plan) = &self.pending_call {
            bottom_call_items.push(
                Container::new(self.vstack(
                    S2,
                    vec![
                        self.line_w(
                            format!("确认呼叫 {}", plan.number),
                            BODY,
                            p.text,
                            Weight::Medium,
                        ),
                        self.text(
                            format!("{} · 确认后由手机直接发起通话", plan.subscription_label),
                            CAPTION,
                            p.text2,
                        ),
                        self.wrap(vec![
                            self.button(
                                "ph-cancel-call",
                                "取消",
                                Action::PhoneCancelCall,
                                Btn::Secondary,
                            ),
                            self.button(
                                "ph-confirm-call",
                                "确认呼叫",
                                Action::PhoneConfirmCall,
                                Btn::Primary,
                            ),
                        ]),
                    ],
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border_strong))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        } else {
            bottom_call_items.push(self.input(
                F_PHONE_CONTACT_NAME,
                "输入联系人姓名（选填，可直接存为联系人）",
                None,
            ));
            let col3_name_draft = self.fields[F_PHONE_CONTACT_NAME].text.trim().to_string();
            let col2_name_draft = self.fields[F_CONTACT_SEARCH].text.trim().to_string();
            let save_ct_state = self.mouse_state("ph-save-ct");
            let font = self.font;
            let save_ct_btn = Hoverable::new(save_ct_state, move |mouse| {
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
                    mix_color(p.surface, p.hover, t)
                };
                let border = if mouse.is_clicked() {
                    p.border_strong
                } else {
                    mix_color(p.surface_border, p.border_strong, t)
                };
                Container::new(
                    ConstrainedBox::new(
                        Align::new(
                            facloud_ui::elements::Text::new("存为联系人", font, BODY)
                                .with_color(p.text)
                                .with_style(
                                    facloud_ui::fonts::Properties::default().weight(Weight::Medium),
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
                if !col3_name_draft.is_empty() {
                    ctx.dispatch_typed_action(Action::Focus(F_CONTACT_SEARCH));
                    ctx.dispatch_typed_action(Action::ClearField(F_CONTACT_SEARCH));
                    ctx.dispatch_typed_action(Action::Insert(col3_name_draft.clone()));
                } else if col2_name_draft.is_empty() {
                    ctx.dispatch_typed_action(Action::Focus(F_PHONE_CONTACT_NAME));
                }
                ctx.dispatch_typed_action(Action::PhoneCreateContact);
            })
            .finish();

            bottom_call_items.push(self.wrap(vec![
                self.button_icon(
                    "ph-call",
                    "呼叫",
                    UiIcon::Phone,
                    Action::PhoneCall(dial_number.clone()),
                    Btn::Primary,
                ),
                self.button(
                    "ph-dial-sms",
                    "发短信",
                    Action::PhoneSmsTo(dial_number.clone()),
                    Btn::Secondary,
                ),
                save_ct_btn,
            ]));
        }

        let call_remote_bar = Container::new(self.vstack(
            6.,
            vec![
                self.between(
                    self.line_w("通话与免提遥控栏", MICRO, p.text, Weight::Semibold),
                    self.line("随时接挂 / 音量 / 投屏免提", MICRO, p.text3),
                ),
                self.wrap(vec![
                    self.button_icon(
                        "ph-rem-answer",
                        "接听来电",
                        UiIcon::Phone,
                        Action::Key(5),
                        Btn::Primary,
                    ),
                    self.button_icon(
                        "ph-rem-hangup",
                        "挂断电话",
                        UiIcon::PhoneOff,
                        Action::PhoneHangup,
                        Btn::DangerSolid,
                    ),
                    self.button_icon(
                        "ph-rem-vol-up",
                        "音量+",
                        UiIcon::Speaker,
                        Action::Key(24),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "ph-rem-vol-dn",
                        "音量-",
                        UiIcon::Speaker,
                        Action::Key(25),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "ph-rem-mute",
                        "静音切换",
                        UiIcon::Mute,
                        Action::Key(164),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "ph-rem-mirror",
                        "投屏免提通话",
                        UiIcon::Monitor,
                        Action::Mirror,
                        Btn::Secondary,
                    ),
                ]),
            ],
        ))
        .with_uniform_padding(10.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.surface_border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let mut speed_dial_chips: Vec<El> = vec![self.line("速拨：", MICRO, p.text2)];
        for (idx, ct) in self.contacts.iter().take(4).enumerate() {
            if ct.number.trim().is_empty() {
                continue;
            }
            let active_sd = norm_phone(&dial_number) == norm_phone(&ct.number);
            speed_dial_chips.push(self.button_icon(
                format!("ph-sd-ct-{idx}"),
                ct.name.clone(),
                UiIcon::User,
                Action::PhoneDialNumber(ct.number.clone()),
                if active_sd {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ));
        }
        for (srv_num, srv_label) in [
            ("10086", "10086 移动"),
            ("10010", "10010 联通"),
            ("10000", "10000 电信"),
        ] {
            let active_srv = dial_number == srv_num;
            speed_dial_chips.push(self.button(
                format!("ph-sd-srv-{srv_num}"),
                srv_label,
                Action::PhoneDialNumber(srv_num.to_string()),
                if active_srv { Btn::Primary } else { Btn::Ghost },
            ));
        }
        let speed_dial_bar = self.wrap(speed_dial_chips);

        let col3_scroll = self.pane_scroll(
            self.push_detail_scroll.clone(),
            Container::new(self.vstack(
                S3,
                vec![
                    call_remote_bar,
                    num_display_box,
                    self.vstack(6., keypad_rows),
                    speed_dial_bar,
                    self.vstack(8., bottom_call_items),
                ],
            ))
            .with_uniform_padding(S4)
            .with_padding_bottom(S4)
            .finish(),
        );

        let col3_card = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(Expanded::new(1., col3_scroll).finish())
                .finish(),
        )
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let columns = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(S4)
                .with_child(Expanded::new(4., col1_card).finish())
                .with_child(Expanded::new(4., col2_card).finish())
                .with_child(Expanded::new(4., col3_card).finish())
                .finish(),
        )
        .with_uniform_padding(S5)
        .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.comm_mode_bar())
            .with_child(Expanded::new(1., self.sub_enter(columns)).finish())
            .finish()
    }

    fn phone_empty(
        &self,
        title: &str,
        description: &str,
        action: Option<(&str, &str, Action)>,
    ) -> El {
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.empty(
                title,
                description,
                action.map(|(id, label, action)| self.button(id, label, action, Btn::Secondary)),
            ))
            .finish()
    }
}
