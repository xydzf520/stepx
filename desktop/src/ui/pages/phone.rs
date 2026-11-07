use super::super::{
    icons::UiIcon, theme::*, widgets::*, Action, Companion, Page, PushAction, F_CONTACT_SEARCH,
    F_PHONE_CONTACT_NAME, F_PHONE_DIAL,
};
use chrono::{Local, TimeZone};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use std::collections::HashSet;
use stepx_companion::device::{CallRecord, ContactEntry};

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
    unseen_missed: bool,
    followed_up: bool,
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

fn extract_sms_bracket_brand(body: &str) -> Option<String> {
    let mut rest = body.trim();
    while let Some(start) = rest.find('【') {
        let after_start = &rest[start + '【'.len_utf8()..];
        let Some(end) = after_start.find('】') else {
            break;
        };
        let raw = after_start[..end].trim();
        if !raw.is_empty()
            && raw.chars().count() <= 16
            && !matches!(
                raw,
                "验证码" | "通知" | "提醒" | "温馨提示" | "重要通知" | "系统通知" | "账单"
            )
        {
            return Some(raw.to_string());
        }
        rest = &after_start[end + '】'.len_utf8()..];
    }
    None
}

fn known_service_contact_name(num: &str, norm: &str) -> Option<String> {
    match norm {
        "10086" => Some("中国移动".into()),
        "10010" => Some("中国联通".into()),
        "10000" => Some("中国电信".into()),
        "10106688" => Some("翼支付/电信客服".into()),
        "1888" | "8531888" => Some("中国电信服务热线".into()),
        "1886" => Some("中国电信增值热线".into()),
        "1889" => Some("中国电信话费查询".into()),
        "66050000" => Some("中国电信留言信箱".into()),
        "66050003" => Some("中国电信接驳铃声".into()),
        "114" => Some("号码百事通".into()),
        "12306" => Some("铁路12306".into()),
        "95338" => Some("顺丰速运".into()),
        "950616" => Some("京东物流".into()),
        "95588" => Some("工商银行".into()),
        "95533" => Some("建设银行".into()),
        "95599" => Some("农业银行".into()),
        "95566" => Some("中国银行".into()),
        "95555" => Some("招商银行".into()),
        "95595" => Some("光大银行".into()),
        "95558" => Some("中信银行".into()),
        "95568" => Some("民生银行".into()),
        "95528" => Some("浦发银行".into()),
        "95511" => Some("平安银行".into()),
        "95508" => Some("广发银行".into()),
        "95561" => Some("兴业银行".into()),
        "4008111333" => Some("光大白金客服".into()),
        _ if norm.starts_with("400") || num.trim().starts_with("400") => {
            Some(format!("客服热线 · {}", num.trim()))
        }
        _ if norm.starts_with("010") && norm.len() >= 10 => Some(format!(
            "北京固话 · {}",
            &norm[norm.len().saturating_sub(4)..]
        )),
        _ if norm.starts_with("021") && norm.len() >= 10 => Some(format!(
            "上海固话 · {}",
            &norm[norm.len().saturating_sub(4)..]
        )),
        _ if norm.starts_with("020") && norm.len() >= 10 => Some(format!(
            "广州固话 · {}",
            &norm[norm.len().saturating_sub(4)..]
        )),
        _ if norm.starts_with("0755") && norm.len() >= 11 => Some(format!(
            "深圳固话 · {}",
            &norm[norm.len().saturating_sub(4)..]
        )),
        _ if norm.starts_with("00") && norm.len() >= 10 => Some(format!(
            "境外来电 · {}",
            &norm[norm.len().saturating_sub(4)..]
        )),
        _ => None,
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

#[allow(dead_code)]
/// 1-based SIM slot number for display. Handles 0-based or duplicated slot
/// values by falling back to the subscription's position.
pub(super) fn sim_slot_number(subs: &[stepx_companion::bridge::SimSubscription], id: i32) -> i32 {
    let pos = subs.iter().position(|s| s.id == id).unwrap_or(0);
    let slot = subs.get(pos).map(|s| s.slot).unwrap_or(1);
    let zero_based = subs.iter().any(|s| s.slot <= 0);
    let duplicated = subs
        .iter()
        .enumerate()
        .any(|(i, a)| subs.iter().skip(i + 1).any(|b| b.slot == a.slot));
    if zero_based || duplicated {
        pos as i32 + 1
    } else {
        slot
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
    pub(crate) fn is_missed_call_followed_up(&self, record: &CallRecord) -> bool {
        if record.call_type != 3 {
            return false;
        }
        let target_norm = norm_phone(&record.number);
        if target_norm.is_empty() {
            return false;
        }
        let call_followed_up = self.call_logs.iter().any(|c| {
            c.date_ms > record.date_ms
                && norm_phone(&c.number) == target_norm
                && (c.call_type == 2 || (c.call_type == 1 && c.duration_sec > 0))
        });
        if call_followed_up {
            return true;
        }
        self.sms.iter().any(|m| {
            m.outgoing && m.date_ms > record.date_ms && norm_phone(&m.address) == target_norm
        })
    }

    pub(crate) fn is_missed_call_unseen(&self, record: &CallRecord) -> bool {
        if record.call_type != 3 {
            return false;
        }
        if record.date_ms <= 0
            || Local::now()
                .timestamp_millis()
                .saturating_sub(record.date_ms)
                > 86_400_000
        {
            return false;
        }
        if self.is_missed_call_followed_up(record) {
            return false;
        }
        let raw_trimmed = record.number.trim();
        let num = if raw_trimmed.is_empty() {
            "unknown"
        } else {
            raw_trimmed
        };
        let item_id = format!("missed-call:{num}:{}", record.date_ms);
        let raw_item_id = format!("missed-call:{raw_trimmed}:{}", record.date_ms);
        !self.notification_center.read_ids.contains(&item_id)
            && !self.notification_center.dismissed_ids.contains(&item_id)
            && !self.notification_center.read_ids.contains(&raw_item_id)
            && !self
                .notification_center
                .dismissed_ids
                .contains(&raw_item_id)
    }

    pub(crate) fn unseen_missed_call_count(&self) -> usize {
        self.call_logs
            .iter()
            .filter(|r| self.is_missed_call_unseen(r))
            .count()
    }

    pub(crate) fn effective_contacts(&self) -> Vec<ContactEntry> {
        let mut out = self.contacts.clone();
        let mut seen: HashSet<String> = out
            .iter()
            .map(|c| norm_phone(&c.number))
            .filter(|n| !n.is_empty())
            .collect();

        // Also include contacts whose real names are already stored on the phone's call logs
        for r in &self.call_logs {
            let num = r.number.trim();
            let r_name = r.name.trim();
            if num.is_empty()
                || r_name.is_empty()
                || r_name.eq_ignore_ascii_case("null")
                || r_name == num
            {
                continue;
            }
            let norm = norm_phone(num);
            if norm.is_empty() || !seen.insert(norm) {
                continue;
            }
            out.push(ContactEntry {
                name: r_name.to_string(),
                number: num.to_string(),
            });
        }

        out
    }

    pub(in super::super) fn phone_actions(&self) -> Vec<El> {
        vec![self.button_icon(
            "ph-refresh",
            "同步记录",
            UiIcon::Refresh,
            Action::ReadPhone,
            Btn::Secondary,
        )]
    }

    pub(in super::super) fn filtered_contacts(&self) -> Vec<ContactEntry> {
        let contacts = self.effective_contacts();
        let search_q = self.fields[F_CONTACT_SEARCH].text.trim().to_lowercase();
        let dial_q = self.fields[F_PHONE_DIAL].text.trim().to_lowercase();
        let q = if !search_q.is_empty() {
            search_q
        } else {
            dial_q
        };
        if q.is_empty() {
            return contacts;
        }
        let norm_q = norm_phone(&q);
        contacts
            .into_iter()
            .filter(|c| {
                c.name.to_lowercase().contains(&q)
                    || c.number.to_lowercase().contains(&q)
                    || (!norm_q.is_empty() && norm_phone(&c.number).contains(&norm_q))
            })
            .collect()
    }

    pub(in super::super) fn phone_top_bar(&self) -> El {
        let p = self.p();
        let total_calls_count = self.call_logs.len();
        let unseen_missed = self.unseen_missed_call_count();
        let _total_missed = self.call_logs.iter().filter(|r| r.call_type == 3).count();
        let contacts_count = self.effective_contacts().len();

        let mut left_items = vec![
            self.content_icon(UiIcon::Phone, 15., p.accent),
            if unseen_missed > 0 {
                self.pill_badge(format!("{unseen_missed} 未接"), p.danger, p.danger_soft)
            } else {
                self.pill_badge("无新未接", p.success_text, p.success_soft)
            },
            self.pill_badge(format!("{total_calls_count} 通话"), p.accent, p.accent_soft),
            self.pill_badge(format!("{contacts_count} 联系人"), p.text2, p.surface),
        ];
        if let Some(ref num) = self.active_call_number {
            left_items.push(self.pill_badge(
                format!("通话中 {num}"),
                p.success_text,
                p.success_soft,
            ));
        }

        let left = self.hstack(S2, left_items);
        let right = self.button_icon(
            "ph-bar-mirror",
            "手机投屏",
            UiIcon::Monitor,
            Action::Mirror,
            Btn::Ghost,
        );

        Container::new(self.between(left, right))
            .with_horizontal_padding(S5)
            .with_vertical_padding(S2 + 2.)
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish()
    }

    pub(in super::super) fn phone_page(&self) -> El {
        let p = self.p();
        if !self.demo_mode && self.selected.is_none() {
            return Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    Expanded::new(
                        1.,
                        self.empty(
                            "连接手机后使用电话",
                            "连接后即可查看通话记录、搜索联系人并从电脑发起呼叫。",
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
        let effective_contacts = self.effective_contacts();
        let all_contacts = self.filtered_contacts();
        let loaded = self.phone_loaded
            || self.demo_mode
            || !self.call_logs.is_empty()
            || !effective_contacts.is_empty();

        // Only records returned by the phone belong in this list.
        let mut all_calls: Vec<DisplayCallRow> = Vec::new();
        for r in &self.call_logs {
            let has_valid_name =
                !r.name.trim().is_empty() && !r.name.trim().eq_ignore_ascii_case("null");
            let resolved_name = if has_valid_name {
                r.name.clone()
            } else {
                let norm_r = norm_phone(&r.number);
                if let Some(c) = effective_contacts
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
            let followed_up = self.is_missed_call_followed_up(r);
            let unseen_missed = self.is_missed_call_unseen(r);
            let label = if r.call_type == 3 && followed_up {
                "未接 · 已回联".into()
            } else {
                format!("{} · {}", r.type_label(), r.format_duration())
            };
            all_calls.push(DisplayCallRow {
                name: resolved_name,
                num: r.number.clone(),
                outgoing: r.call_type == 2,
                missed: r.call_type == 3,
                unseen_missed,
                followed_up,
                label,
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

        let col1_header = Container::new(self.vstack(
            8.,
            vec![
                self.section_heading(UiIcon::Phone, "通话记录"),
                filter_pills,
            ],
        ))
        .with_uniform_padding(14.)
        .with_border(Border::bottom(1.).with_border_color(p.border))
        .finish();

        let search_q = self.fields[F_CONTACT_SEARCH].text.trim().to_lowercase();
        let norm_search_q = norm_phone(&search_q);
        let mut call_rows: Vec<El> = Vec::new();
        let matching_calls: Vec<_> = all_calls.iter().enumerate().filter(|(_,c)| {
            let matches_filter = match active_call_filter {1=>c.missed,2=>!c.outgoing&&!c.missed,3=>c.outgoing,_=>true};
            let matches_search = search_q.is_empty() || c.name.to_lowercase().contains(&search_q) || c.num.to_lowercase().contains(&search_q)
                || (!norm_search_q.is_empty() && norm_phone(&c.num).contains(&norm_search_q));
            matches_filter && matches_search
        }).collect();
        let call_page_key = format!("phone-calls:{active_call_filter}:{search_q}");
        let call_range = self.list_page_range(&call_page_key,matching_calls.len(),40);
        for &(i,c) in &matching_calls[call_range] {
            let arrow_icon = if c.outgoing {
                UiIcon::Upload
            } else {
                UiIcon::Download
            };
            let _followed_up = c.followed_up;
            let row_color = if c.unseen_missed { p.danger } else { p.text };
            let icon_color = if c.unseen_missed {
                p.danger
            } else if c.outgoing {
                p.success_text
            } else {
                p.text2
            };

            let mut info_lines = vec![self.line_w(&c.name, CAPTION, row_color, Weight::Semibold)];
            if !c.num.is_empty() && c.name != c.num {
                info_lines.push(self.mono(&c.num, CAPTION, p.text2));
            }
            info_lines.push(self.line(&c.label, CAPTION, p.text2));

            let left_info = Flex::row()
                .with_spacing(10.)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(self.content_icon(arrow_icon, 14., icon_color))
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
                            .with_child(self.content_icon(UiIcon::Phone, 14., p.text2))
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

            let row_key = format!("ph-call-row:{i}");
            let copy_id = format!("ph-call-copy-{i}");
            let sms_id = format!("ph-call-sms-{i}");
            let hover_keys = [
                row_key.clone(),
                format!("ph-cb:{i}"),
                format!("btn:{copy_id}"),
                format!("btn:{sms_id}"),
            ];
            let row_hovered = {
                let map = self.hover_states.borrow();
                hover_keys.iter().any(|k| {
                    map.get(k.as_str())
                        .is_some_and(|(_, h)| h.lock().unwrap().is_mouse_over_element())
                })
            };

            let mut row_btns: Vec<El> = Vec::new();
            if row_hovered && !c.num.trim().is_empty() {
                row_btns.push(self.msg_inline_svg_action_btn(
                    copy_id,
                    UiIcon::Copy,
                    false,
                    Action::CopyText(c.num.clone(), format!("已复制号码 {}", c.num)),
                ));
                row_btns.push(self.msg_inline_svg_action_btn(
                    sms_id,
                    UiIcon::Sms,
                    false,
                    Action::PhoneSmsTo(c.num.clone()),
                ));
            }
            row_btns.push(cb_btn);

            let right_actions_row = ConstrainedBox::new(
                Flex::row()
                    .with_spacing(4.)
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_children(row_btns)
                    .finish(),
            )
            .with_height(28.)
            .finish();

            let right_info = Flex::column()
                .with_spacing(6.)
                .with_cross_axis_alignment(CrossAxisAlignment::End)
                .with_child(self.mono(&c.time, CAPTION, p.text2))
                .with_child(right_actions_row)
                .finish();

            let row_state = self.mouse_state(row_key);
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
            #[cfg(feature = "ui-capture")]
            let call_row_el =
                crate::ui::scenario_check::control(format!("ph-call-row-{i}"), call_row_el);

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
                .with_child(self.list_page_controls(&call_page_key,matching_calls.len(),40))
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
                self.heading(
                    if loaded && (self.contacts_error.is_none() || !all_contacts.is_empty()) {
                        format!("联系人 ({})", all_contacts.len())
                    } else {
                        "联系人".into()
                    },
                ),
                self.input(F_CONTACT_SEARCH, "搜索联系人", None),
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

        let sel_avatar = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(self.content_icon(UiIcon::User, 16., p.accent_fg))
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
                    self.button_icon(
                        "ph-sel-copy",
                        "复制",
                        UiIcon::Copy,
                        Action::CopyText(
                            sel.number.clone(),
                            format!("已复制联系人号码 {}", sel.number),
                        ),
                        Btn::Ghost,
                    ),
                    self.button_icon(
                        "ph-sel-sms",
                        "短信",
                        UiIcon::Sms,
                        Action::PhoneSmsTo(sel.number.clone()),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "ph-sel-call",
                        "呼叫",
                        UiIcon::Phone,
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
        let contact_page_key = format!("phone-contacts:{}:{}",search_q,self.fields[F_PHONE_DIAL].text);
        let contact_range = self.list_page_range(&contact_page_key,all_contacts.len(),40);
        for (i, ct) in all_contacts.iter().enumerate().skip(contact_range.start).take(contact_range.len()) {
            let active = i == active_idx;
            let badge_el = ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(self.content_icon(
                            UiIcon::User,
                            14.,
                            if p.dark {
                                p.text2
                            } else {
                                rgb(0x52, 0x52, 0x5B)
                            },
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

            let row_key = format!("ph-ct-row:{i}");
            let call_btn_key = format!("btn:ph-ct-call-{i}");
            let sms_btn_key = format!("btn:ph-ct-sms-{i}");
            let copy_btn_key = format!("btn:ph-ct-copy-{i}");
            let row_hovered =
                self.is_hovered_any(&[&row_key, &call_btn_key, &sms_btn_key, &copy_btn_key]);

            let right_inner = if row_hovered && !ct.number.trim().is_empty() {
                self.hstack(
                    4.,
                    vec![
                        self.msg_inline_svg_action_btn(
                            format!("ph-ct-sms-{i}"),
                            UiIcon::Sms,
                            false,
                            Action::PhoneSmsTo(ct.number.clone()),
                        ),
                        self.msg_inline_svg_action_btn(
                            format!("ph-ct-copy-{i}"),
                            UiIcon::Copy,
                            false,
                            Action::CopyText(
                                ct.number.clone(),
                                format!("已复制 {} 的号码", ct.name),
                            ),
                        ),
                        self.msg_inline_svg_action_btn(
                            format!("ph-ct-call-{i}"),
                            UiIcon::Phone,
                            active,
                            Action::PhoneCall(ct.number.clone()),
                        ),
                    ],
                )
            } else {
                self.mono(&ct.number, CAPTION, p.text2)
            };
            let right_el = ConstrainedBox::new(
                Flex::row()
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(right_inner)
                    .finish(),
            )
            .with_height(24.)
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
                right_el,
            );
            let state = self.mouse_state(row_key);
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
            .with_defer_events_to_children()
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
            #[cfg(feature = "ui-capture")]
            let row_el = crate::ui::scenario_check::control(format!("ph-ct-row-{i}"), row_el);
            contact_rows.push(row_el);
        }

        let col2_scroll = if !contact_rows.is_empty() {
            self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(self.vstack(2., contact_rows))
                    .with_horizontal_padding(S2)
                    .with_padding_top(S2)
                    .with_padding_bottom(S3)
                    .finish(),
            )
        } else if let Some(error) = &self.contacts_error {
            self.phone_empty(
                "通讯录读取失败",
                error,
                Some(("ph-retry-contacts", "重新读取", Action::ReadPhone)),
            )
        } else {
            let searching = !self.fields[F_CONTACT_SEARCH].text.trim().is_empty()
                || !self.fields[F_PHONE_DIAL].text.trim().is_empty();
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
                        "phone-save-new-contact",
                        "保存到通讯录",
                        Action::PhoneCreateContact,
                    ))
                },
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
                .with_child(self.list_page_controls(&contact_page_key,all_contacts.len(),40))
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
                effective_contacts
                    .iter()
                    .find(|c| norm_phone(&c.number) == norm_dial)
                    .map(|c| format!("已匹配：{}", c.name))
                    .or_else(|| {
                        (norm_dial.len() >= 3).then(|| {
                            effective_contacts
                                .iter()
                                .find(|c| norm_phone(&c.number).contains(&norm_dial))
                                .map(|c| format!("已匹配：{} ({})", c.name, c.number))
                        })?
                    })
            })
            .flatten()
            .unwrap_or_default();

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
                            sim_slot_number(&c.subscriptions, s.id)
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
            .unwrap_or_default();

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
                                sim_slot_number(&caps.subscriptions, sim.id),
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
            let peer_label = effective_contacts
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
                                self.line_w("正在呼叫…", CAPTION, p.text, Weight::Medium),
                            ],
                        ),
                        self.text(
                            format!("{peer_label} · 通话状态以手机为准"),
                            CAPTION,
                            p.text2,
                        ),
                        self.wrap(vec![
                            self.button_icon(
                                "ph-active-sms",
                                "发短信",
                                UiIcon::Sms,
                                Action::PhoneSmsTo(active_num.clone()),
                                Btn::Secondary,
                            ),
                            self.button_icon(
                                "ph-hangup",
                                "挂断",
                                UiIcon::PhoneOff,
                                Action::PhoneHangup,
                                Btn::DangerSolid,
                            ),
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
                            self.button_icon(
                                "ph-cancel-call",
                                "取消",
                                UiIcon::Close,
                                Action::PhoneCancelCall,
                                Btn::Secondary,
                            ),
                            self.button_icon(
                                "ph-confirm-call",
                                "确认呼叫",
                                UiIcon::Phone,
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
            bottom_call_items.push(self.input(F_PHONE_CONTACT_NAME, "姓名（选填）", None));
            let save_ct_btn = self.button_icon(
                "phone-create-contact",
                "保存到通讯录",
                UiIcon::Plus,
                Action::PhoneCreateContact,
                Btn::Secondary,
            );

            bottom_call_items.push(self.wrap(vec![
                self.button_icon(
                    "ph-call",
                    "呼叫",
                    UiIcon::Phone,
                    Action::PhoneCall(dial_number.clone()),
                    Btn::Primary,
                ),
                self.button_icon(
                    "ph-dial-sms",
                    "发短信",
                    UiIcon::Sms,
                    Action::PhoneSmsTo(dial_number.clone()),
                    Btn::Secondary,
                ),
                save_ct_btn,
            ]));
        }

        let call_remote_bar = Container::new(self.vstack(
            6.,
            vec![
                self.line_w("手机音量与投屏", MICRO, p.text, Weight::Semibold),
                self.wrap(vec![
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
                        "手机投屏",
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
        for (idx, ct) in effective_contacts.iter().take(4).enumerate() {
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
                    num_display_box,
                    self.vstack(6., keypad_rows),
                    self.vstack(8., bottom_call_items),
                    speed_dial_bar,
                    call_remote_bar,
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
            .with_child(self.phone_top_bar())
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

#[cfg(test)]
mod tests {
    use crate::ui::Companion;
    use stepx_companion::{
        device::{CallRecord, Device},
        push::{ImMessage, ImThread},
        sms::Message,
    };

    #[test]
    fn phone_disconnect_reconnect_only_prompts_for_new_items_and_synthesizes_contacts() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::ui::TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.notification_center.connect_baseline_enabled = true;
        let now_ms = chrono::Local::now().timestamp_millis();

        // 1. Connect device PZ261A
        app.selected = Some(Device {
            serial: "10.0.0.42:5555".into(),
            model: "StepX PZ261A".into(),
            ..Default::default()
        });
        app.last_connected_serial = Some("10.0.0.42:5555".into());

        // Historical call logs on the phone (including a missed call followed up by a later answered call,
        // and a 2-day-old missed call) + empty system contacts
        let initial_calls = vec![
            CallRecord {
                number: "+12025550101".into(),
                name: String::new(),
                call_type: 1,
                date_ms: now_ms - 12 * 3_600_000,
                duration_sec: 15,
            },
            CallRecord {
                number: "+12025550101".into(),
                name: String::new(),
                call_type: 3,
                date_ms: now_ms - 14 * 3_600_000,
                duration_sec: 0,
            },
            CallRecord {
                number: "+12025550102".into(),
                name: String::new(),
                call_type: 3,
                date_ms: now_ms - 40 * 3_600_000,
                duration_sec: 0,
            },
            CallRecord {
                number: "10086".into(),
                name: String::new(),
                call_type: 1,
                date_ms: now_ms - 5 * 3_600_000,
                duration_sec: 28,
            },
        ];
        app.apply_phone_data(Ok(initial_calls.clone()), Ok(Vec::new()));

        let initial_sms = vec![Message {
            provider_id: Some(1),
            address: "10690001".into(),
            body: "【叮咚买菜】您的订单已送达".into(),
            date_ms: now_ms - 10 * 3_600_000,
            outgoing: false,
            kind: 1,
            read: true,
            subscription: None,
        }];
        app.apply_sms_data(Ok(initial_sms.clone()));

        // Historical read WeChat threads ("叮咚买菜 优惠券到账提醒" & "顺丰速运+ 账户变动提醒")
        app.push.snapshot.threads = vec![
            ImThread {
                thread_id: "com.tencent.mm::叮咚买菜 美食水果蔬菜海鲜早餐".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "叮咚买菜 美食水果蔬菜海鲜早餐".into(),
                last_message: "优惠券到账提醒".into(),
                last_timestamp_ms: now_ms - 22 * 60_000,
                unread_count: 0,
                ..Default::default()
            },
            ImThread {
                thread_id: "com.tencent.mm::顺丰速运+".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "顺丰速运+".into(),
                last_message: "账户变动提醒".into(),
                last_timestamp_ms: 0,
                unread_count: 0,
                messages: vec![ImMessage {
                    id: "capture_sf_1".into(),
                    sender: "顺丰速运+".into(),
                    text: "账户变动提醒".into(),
                    timestamp_ms: 0,
                    observed_at_ms: now_ms - 3 * 86_400_000,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];
        app.sync_live_express_parcels();
        app.sync_notification_center();

        // Verify on initial connect: 0 unseen missed calls, real contacts synced, 0 stale capsules, 0 unread notifications
        assert_eq!(app.unseen_missed_call_count(), 0);
        assert!(app.is_missed_call_followed_up(&initial_calls[1]));
        assert!(
            app.collect_top_bar_live_capsules().is_empty(),
            "Read coupon/account notices must not produce stale live capsules"
        );
        assert_eq!(app.notification_unread_count(), 0);

        // 2. Disconnect and reconnect the same phone
        app.selected = None;
        app.clear_device_state();
        assert!(app.selected.is_none());

        app.selected = Some(Device {
            serial: "10.0.0.42:5555".into(),
            model: "StepX PZ261A".into(),
            ..Default::default()
        });
        app.apply_phone_data(Ok(initial_calls.clone()), Ok(Vec::new()));
        app.apply_sms_data(Ok(initial_sms.clone()));
        app.sync_notification_center();

        // After reconnect: still 0 unseen missed calls, 0 unread notifications, 0 popups
        assert_eq!(app.unseen_missed_call_count(), 0);
        assert_eq!(app.notification_unread_count(), 0);
        assert_eq!(app.notification_center.popup_queue_len(), 0);

        // 3. Now a genuinely NEW missed call arrives AFTER connection is established
        let mut updated_calls = initial_calls;
        updated_calls.insert(
            0,
            CallRecord {
                number: "+12025550103".into(),
                name: "张总".into(),
                call_type: 3,
                date_ms: now_ms,
                duration_sec: 0,
            },
        );
        app.apply_phone_data(Ok(updated_calls), Ok(Vec::new()));
        app.sync_notification_center();

        // Only the 1 brand-new missed call triggers the unread missed call badge and notification center unread count!
        assert_eq!(app.unseen_missed_call_count(), 1);
        assert!(app.notification_unread_count() >= 1);

        // Viewing Page::Phone marks the new missed call as seen
        app.mark_missed_calls_seen();
        assert_eq!(app.unseen_missed_call_count(), 0);
    }
}
