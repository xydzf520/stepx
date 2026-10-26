use super::super::{
    icons::UiIcon, theme::*, widgets::*, Action, Companion, Page, PushAction, SmsFilter,
    F_IM_REPLY, F_SMS_BODY, F_SMS_SEARCH, F_SMS_TO,
};
use chrono::{Local, TimeZone};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use stepx_companion::push::{
    demo_push_snapshot, extract_first_url, extract_otp_code, ImChannel, ImThread, PushSnapshot,
};

struct DisplaySmsThread {
    name: String,
    num: String,
    time: String,
    preview: String,
    last_body: String,
    unread: bool,
    badge: String,
    count: usize,
    otp_code: Option<String>,
}

fn norm_addr(s: &str) -> String {
    stepx_companion::sms::normalize_address(s)
}

pub(in super::super) fn extract_pickup_code(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let has_pickup_kw = lower.contains("取件")
        || lower.contains("驿站")
        || lower.contains("菜鸟")
        || lower.contains("丰巢")
        || lower.contains("快递柜")
        || lower.contains("快件")
        || lower.contains("提货码")
        || lower.contains("包裹");
    if !has_pickup_kw {
        return None;
    }

    // Look for hyphenated pickup codes first, e.g. "4-2-1088", "A1-2-108", "12-304"
    for raw_token in text.split(|c: char| {
        c.is_whitespace()
            || matches!(
                c,
                '，' | '。'
                    | '；'
                    | '：'
                    | ':'
                    | '、'
                    | '！'
                    | '？'
                    | ','
                    | '.'
                    | ';'
                    | '【'
                    | '】'
                    | '('
                    | ')'
                    | '（'
                    | '）'
                    | '「'
                    | '」'
                    | '"'
                    | '\''
            )
    }) {
        let token = raw_token.trim_matches('-');
        if token.contains('-') && token.len() >= 4 && token.len() <= 14 {
            let parts: Vec<&str> = token.split('-').collect();
            let all_valid = parts.len() >= 2
                && parts
                    .iter()
                    .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_alphanumeric()));
            let digit_count = token.chars().filter(|c| c.is_ascii_digit()).count();
            let looks_like_date =
                parts.len() == 3 && parts[0].len() == 4 && parts[0].starts_with("20");
            if all_valid && digit_count >= 3 && !looks_like_date {
                return Some(token.to_string());
            }
        }
    }

    // Otherwise look for a 4..=8 digit code near pickup keywords
    let pickup_kws = ["取件码", "提货码", "取件", "凭码", "凭"];
    let bytes = text.as_bytes();
    let mut candidates: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if (4..=8).contains(&len) {
                let prev_char = text[..start].chars().next_back();
                let next_char = text[i..].chars().next();
                let boundary_bad = matches!(prev_char, Some('.' | ':' | '/'))
                    || matches!(next_char, Some('.' | ':' | '/' | '年' | '月' | '日' | '元'));
                if !boundary_bad {
                    candidates.push((start, text[start..i].to_string()));
                }
            }
        } else {
            i += 1;
        }
    }
    if let Some(kw_pos) = pickup_kws.iter().filter_map(|kw| lower.find(kw)).min() {
        if let Some((_, code)) = candidates
            .iter()
            .find(|(pos, _)| *pos >= kw_pos)
            .or_else(|| candidates.first())
        {
            return Some(code.clone());
        }
    }
    candidates.into_iter().next().map(|(_, c)| c)
}

pub(in super::super) fn extract_verification_code(text: &str) -> Option<String> {
    let lower = text.to_lowercase();
    let substring_keywords = [
        "验证码",
        "校验码",
        "动态码",
        "确认码",
        "安全码",
        "授权码",
        "登录码",
        "认证码",
        "verification",
        "passcode",
    ];
    let has_word_kw = lower
        .split(|c: char| !c.is_ascii_alphabetic() && c != '_')
        .any(|w| w == "code" || w == "otp" || w == "pin");
    let has_kw = has_word_kw || substring_keywords.iter().any(|kw| lower.contains(kw));
    if !has_kw {
        return extract_otp_code(text).or_else(|| extract_pickup_code(text));
    }
    let bytes = text.as_bytes();
    let mut candidates: Vec<(usize, String)> = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if (4..=8).contains(&len) {
                let prev_char = text[..start].chars().next_back();
                let next_char = text[i..].chars().next();
                let boundary_bad = matches!(prev_char, Some('.' | ':' | '/'))
                    || matches!(next_char, Some('.' | ':' | '/' | '年' | '月' | '日' | '元'));
                if !boundary_bad {
                    candidates.push((start, text[start..i].to_string()));
                }
            }
        } else {
            i += 1;
        }
    }
    let word_pos = ["code", "otp", "pin"]
        .iter()
        .filter_map(|w| {
            let mut offset = 0usize;
            while let Some(rel) = lower[offset..].find(w) {
                let abs = offset + rel;
                let end = abs + w.len();
                let prev_ok = lower[..abs]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_ascii_alphabetic() && c != '_');
                let next_ok = lower[end..]
                    .chars()
                    .next()
                    .is_none_or(|c| !c.is_ascii_alphabetic() && c != '_');
                if prev_ok && next_ok {
                    return Some(abs);
                }
                offset = end;
            }
            None
        })
        .min();
    let sub_pos = substring_keywords
        .iter()
        .filter_map(|kw| lower.find(kw))
        .min();
    let kw_pos = match (sub_pos, word_pos) {
        (Some(a), Some(b)) => Some(a.min(b)),
        (Some(a), None) => Some(a),
        (None, Some(b)) => Some(b),
        (None, None) => None,
    };
    if let Some(kw_pos) = kw_pos {
        if let Some((_, code)) = candidates
            .iter()
            .find(|(pos, _)| *pos >= kw_pos)
            .or_else(|| candidates.first())
        {
            return Some(code.clone());
        }
    }
    candidates
        .into_iter()
        .next()
        .map(|(_, c)| c)
        .or_else(|| extract_otp_code(text))
        .or_else(|| extract_pickup_code(text))
}

fn format_sms_time(ms: i64) -> String {
    if ms <= 0 {
        return "时间未知".into();
    }
    if let Some(dt) = Local.timestamp_millis_opt(ms).single() {
        dt.format("%m-%d %H:%M").to_string()
    } else {
        "时间未知".into()
    }
}

fn resolve_sender_info(
    addr: &str,
    contacts: &[stepx_companion::device::ContactEntry],
) -> (String, String) {
    let norm = norm_addr(addr);
    if let Some(c) = contacts.iter().find(|c| norm_addr(&c.number) == norm) {
        let badge = c.name.chars().next().unwrap_or('联').to_string();
        return (c.name.clone(), badge);
    }
    let label = if addr.is_empty() {
        "未知号码"
    } else {
        addr
    };
    (
        label.into(),
        label.chars().next().unwrap_or('#').to_string(),
    )
}

fn im_avatar_style(package_name: &str, app_name: &str) -> (String, ColorU, ColorU) {
    match ImChannel::from_package(package_name) {
        ImChannel::WeChat => ("微".into(), rgb(0x07, 0xC1, 0x60), ColorU::white()),
        ImChannel::WeCom => ("企".into(), rgb(0x2B, 0x6C, 0xB0), ColorU::white()),
        ImChannel::DingTalk => ("钉".into(), rgb(0x00, 0x89, 0xFF), ColorU::white()),
        ImChannel::Feishu => ("飞".into(), rgb(0x33, 0x70, 0xFF), ColorU::white()),
        ImChannel::QqTim => ("Q".into(), rgb(0x02, 0x84, 0xC7), ColorU::white()),
        ImChannel::Sms => ("短".into(), rgb(0x10, 0xB9, 0x81), ColorU::white()),
        ImChannel::SocialDm => ("私".into(), rgb(0xE1, 0x1D, 0x48), ColorU::white()),
        _ => {
            if package_name.contains("telegram") {
                ("TG".into(), rgb(0x02, 0x84, 0xC7), ColorU::white())
            } else if package_name.contains("whatsapp") {
                ("WA".into(), rgb(0x16, 0xA3, 0x4A), ColorU::white())
            } else {
                let first = app_name
                    .trim()
                    .chars()
                    .next()
                    .unwrap_or('聊')
                    .to_uppercase()
                    .to_string();
                (first, rgb(0x63, 0x66, 0xF1), ColorU::white())
            }
        }
    }
}

pub(super) fn is_pickup_code_text(code: &str, text: &str) -> bool {
    if code.contains('-') {
        return true;
    }
    let lower = text.to_lowercase();
    lower.contains("取件")
        || lower.contains("驿站")
        || lower.contains("菜鸟")
        || lower.contains("丰巢")
        || lower.contains("快递柜")
        || lower.contains("快件")
}

fn compact_thread_title(title: &str, max_chars: usize) -> String {
    let trimmed = title.trim();
    if trimmed.chars().count() > max_chars {
        format!("{}…", trimmed.chars().take(max_chars).collect::<String>())
    } else {
        trimmed.to_string()
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
    pub(in super::super) fn sms_actions(&self) -> Vec<El> {
        if self.push.im_channel == ImChannel::Sms {
            vec![
                self.button(
                    "sms-open-phone",
                    "手机打开短信",
                    Action::OpenSms,
                    Btn::Secondary,
                ),
                self.button(
                    "sms-refresh-native",
                    "刷新短信",
                    Action::ReadSms,
                    Btn::Secondary,
                ),
                self.button("sms-new", "新建短信", Action::NewSms, Btn::Primary),
            ]
        } else {
            vec![
                self.button(
                    "im-sync-chat-win",
                    "同步聊天屏",
                    Action::Push(PushAction::SyncActivePhoneChatWindow),
                    Btn::Secondary,
                ),
                self.button(
                    "im-refresh-push",
                    "刷新",
                    Action::Push(PushAction::Refresh),
                    Btn::Secondary,
                ),
                self.button("sms-new", "新建短信", Action::NewSms, Btn::Primary),
            ]
        }
    }

    pub(in super::super) fn comm_mode_bar(&self) -> El {
        let p = self.p();
        let im_unread: usize = self
            .push
            .snapshot
            .threads
            .iter()
            .map(|t| t.unread_count)
            .sum();
        let sms_unread: usize = self.sms.iter().filter(|m| m.kind == 1 && !m.read).count();
        let push_unread: usize = self
            .push
            .snapshot
            .notifications
            .iter()
            .filter(|n| !n.dismissed && !n.read && !n.is_ongoing)
            .count();
        let missed_calls: usize = self.call_logs.iter().filter(|r| r.call_type == 3).count();

        let mode_seg = self.segmented(
            "comm-mode-bar",
            vec![
                (
                    "聊天与短信".into(),
                    self.page == Page::Sms,
                    Action::Page(Page::Sms),
                ),
                (
                    "手机推送通知".into(),
                    self.page == Page::Notifications,
                    Action::Page(Page::Notifications),
                ),
                (
                    "电话与通讯录".into(),
                    self.page == Page::Phone,
                    Action::Page(Page::Phone),
                ),
            ],
        );

        let mut right_badges: Vec<El> = Vec::new();
        if im_unread > 0 {
            right_badges.push(self.pill_badge(
                format!("{im_unread} 未读聊天"),
                p.success_text,
                p.success_soft,
            ));
        }
        if sms_unread > 0 {
            right_badges.push(self.pill_badge(
                format!("{sms_unread} 未读短信"),
                p.accent,
                p.accent_soft,
            ));
        }
        if push_unread > 0 {
            right_badges.push(self.pill_badge(
                format!("{push_unread} 未读通知"),
                p.warning,
                p.warning_soft,
            ));
        }
        if missed_calls > 0 {
            right_badges.push(self.pill_badge(
                format!("{missed_calls} 未接来电"),
                p.danger,
                p.danger_soft,
            ));
        }
        if self.push.prefs.privacy_mask {
            right_badges.push(self.pill_badge("防窥已开启", p.warning, p.warning_soft));
        }

        Container::new(self.between(mode_seg, self.hstack(S2, right_badges)))
            .with_horizontal_padding(S5)
            .with_vertical_padding(S2 + 2.)
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish()
    }

    fn im_channel_pill(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        active: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("im-ch-pill:{}", id.into()));
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
            Container::new(
                facloud_ui::elements::Text::new(label.clone(), font, MICRO)
                    .with_color(fg)
                    .with_style(facloud_ui::fonts::Properties::default().weight(weight))
                    .soft_wrap(false)
                    .finish(),
            )
            .with_horizontal_padding(8.)
            .with_vertical_padding(3.)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            if matches!(&action, Action::Push(PushAction::SelectImChannel(_))) {
                ctx.dispatch_typed_action(Action::SelectSmsThread(None));
            }
            ctx.dispatch_typed_action(action.clone());
        })
        .finish()
    }

    fn im_thread_dialog(&self, t: &ImThread, snapshot: &PushSnapshot) -> El {
        let p = self.p();
        let app_label = if t.app_name.is_empty() {
            t.channel().label()
        } else {
            t.app_name.as_str()
        };
        let (badge_txt, badge_bg, badge_fg) = im_avatar_style(&t.package_name, app_label);
        let header_avatar = ConstrainedBox::new(
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

        let channel_status = if t.can_remote_reply {
            self.pill_badge("静默秒回", p.success_text, p.success_soft)
        } else if t.can_ui_reply {
            self.pill_badge("看屏投递", p.warning, p.warning_soft)
        } else {
            self.pill_badge("待激活", p.text2, p.tag_bg)
        };

        let title_row = self.hstack(
            8.,
            vec![
                self.line_w(&t.title, SUBHEAD, p.text, Weight::Semibold),
                self.pill_badge(
                    format!(
                        "{} · {}",
                        app_label,
                        if t.is_group { "群聊" } else { "私聊" }
                    ),
                    p.text2,
                    p.tag_bg,
                ),
                channel_status,
            ],
        );

        let latest_im_otp = if self.push.prefs.privacy_mask {
            None
        } else {
            t.messages
                .iter()
                .rev()
                .find_map(|m| {
                    extract_verification_code(&m.text)
                        .map(|code| (code.clone(), is_pickup_code_text(&code, &m.text)))
                })
                .or_else(|| {
                    extract_verification_code(&t.last_message)
                        .map(|code| (code.clone(), is_pickup_code_text(&code, &t.last_message)))
                })
        };

        let mut hdr_btns = Vec::new();
        if let Some((code, pickup)) = &latest_im_otp {
            hdr_btns.push(self.button_icon(
                "im-hdr-copy-otp",
                if *pickup {
                    format!("复制取件码: {code}")
                } else {
                    format!("复制验证码: {code}")
                },
                if *pickup {
                    UiIcon::Package
                } else {
                    UiIcon::Copy
                },
                Action::CopyText(
                    code.clone(),
                    if *pickup {
                        format!("已复制取件码 {code}")
                    } else {
                        format!("已复制验证码 {code}")
                    },
                ),
                Btn::Primary,
            ));
        }
        hdr_btns.push(self.button(
            "im-hdr-open-phone",
            "手机打开",
            Action::Push(PushAction::OpenOnPhone(t.notification_key.clone())),
            Btn::Secondary,
        ));
        let header_actions = self.hstack(6., hdr_btns);

        let dialog_header = Container::new(self.between(
            self.hstack(
                10.,
                vec![
                    header_avatar,
                    self.vstack(
                        2.,
                        vec![
                            title_row,
                            self.line(
                                if !snapshot.screen_interactive || snapshot.keyguard_locked {
                                    format!("{} · 熄屏锁屏可后台无感秒回", t.package_name)
                                } else {
                                    format!("{} · 支持静默秒回与屏上气泡同步", t.package_name)
                                },
                                MICRO,
                                p.text3,
                            ),
                        ],
                    ),
                ],
            ),
            header_actions,
        ))
        .with_horizontal_padding(S5)
        .with_vertical_padding(10.)
        .with_background_color(p.bg)
        .with_border(Border::bottom(1.).with_border_color(p.border))
        .finish();

        // Message Bubble Stream
        let mut msg_items: Vec<El> = Vec::new();
        msg_items.push(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_child(
                    Container::new(self.line(
                        format!(
                            "{}会话 · 共 {} 条已同步聊天气泡",
                            app_label,
                            t.messages.len()
                        ),
                        MICRO,
                        p.text3,
                    ))
                    .with_horizontal_padding(10.)
                    .with_vertical_padding(3.)
                    .with_background_color(p.tag_bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                    .finish(),
                )
                .finish(),
        );

        for (idx, m) in t.messages.iter().enumerate() {
            let ts = super::format_time(m.timestamp_ms);
            let msg_text = if self.push.prefs.privacy_mask && !m.outgoing {
                "[防窥模式已开启，已隐藏消息内容]".to_string()
            } else {
                m.text.clone()
            };

            if m.outgoing {
                let status_txt = match m.delivery_status.as_str() {
                    "sending" => "发送中",
                    "failed" => "发送失败 · 草稿已保留",
                    "unknown" => "发送状态未确认 · 请核对手机",
                    _ if m.via == "remote_input" => "已提交给手机应用",
                    _ if m.via == "ui_bridge" => "手机会话已显示",
                    _ => "已发送",
                };
                let meta = if ts.is_empty() {
                    format!("我 · {status_txt}")
                } else {
                    format!("{ts} · {status_txt}")
                };
                let bubble_box = Container::new(self.text(msg_text, BODY, p.accent_fg))
                    .with_horizontal_padding(16.)
                    .with_vertical_padding(10.)
                    .with_background_color(p.accent)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
                    .finish();
                let out_actions: Vec<El> = vec![self.button(
                    format!("im-out-copy-{idx}"),
                    "复制",
                    Action::CopyText(m.text.clone(), "已复制消息内容".into()),
                    Btn::Ghost,
                )];
                let out_row = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::End)
                    .with_child(
                        Flex::column()
                            .with_spacing(4.)
                            .with_cross_axis_alignment(CrossAxisAlignment::End)
                            .with_child(
                                ConstrainedBox::new(bubble_box)
                                    .with_max_width(560.)
                                    .finish(),
                            )
                            .with_child(self.hstack(
                                8.,
                                vec![
                                    Container::new(self.mono(meta, MICRO, p.text3))
                                        .with_padding_right(4.)
                                        .finish(),
                                    self.wrap(out_actions),
                                ],
                            ))
                            .finish(),
                    )
                    .finish();
                let state = self.mouse_state(format!("im-msg-out:{}:{idx}", t.thread_id));
                let txt_dbl = m.text.clone();
                let otp_rc = extract_verification_code(&m.text);
                let tid_rc = t.thread_id.clone();
                let pkg_rc = t.package_name.clone();
                let title_rc = t.title.clone();
                let txt_rc = m.text.clone();
                let nkey_rc = t.notification_key.clone();
                msg_items.push(
                    Hoverable::new(state, move |_| out_row)
                        .with_defer_events_to_children()
                        .with_cursor(Cursor::PointingHand)
                        .on_double_click(move |ctx, _, _| {
                            if let Some(code) = extract_verification_code(&txt_dbl) {
                                let pickup = is_pickup_code_text(&code, &txt_dbl);
                                ctx.dispatch_typed_action(Action::CopyText(
                                    code.clone(),
                                    if pickup {
                                        format!("已复制取件码 {code}")
                                    } else {
                                        format!("已复制验证码 {code}")
                                    },
                                ));
                            } else {
                                ctx.dispatch_typed_action(Action::CopyText(
                                    txt_dbl.clone(),
                                    "已复制消息内容".into(),
                                ));
                            }
                        })
                        .on_right_click(move |ctx, _, at| {
                            ctx.dispatch_typed_action(Action::ContextOpen(
                                crate::ui::context_menu::Target::ImMessage {
                                    thread_id: tid_rc.clone(),
                                    package_name: pkg_rc.clone(),
                                    title: title_rc.clone(),
                                    sender: "我".into(),
                                    text: txt_rc.clone(),
                                    notification_key: nkey_rc.clone(),
                                    otp_code: otp_rc.clone(),
                                },
                                at,
                            ));
                        })
                        .finish(),
                );
            } else {
                let sender_name = if m.sender.trim().is_empty() {
                    t.title.as_str()
                } else {
                    m.sender.as_str()
                };
                let meta = if ts.is_empty() {
                    sender_name.to_string()
                } else {
                    format!("{sender_name} · {ts}")
                };

                let bubble_box = Container::new(self.text(msg_text.clone(), BODY, p.text))
                    .with_horizontal_padding(16.)
                    .with_vertical_padding(10.)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
                    .finish();

                let mut msg_col = Flex::column()
                    .with_spacing(4.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_child(
                        Container::new(self.line_w(meta, MICRO, p.text2, Weight::Medium))
                            .with_padding_left(4.)
                            .finish(),
                    )
                    .with_child(
                        ConstrainedBox::new(bubble_box)
                            .with_max_width(560.)
                            .finish(),
                    );

                if !self.push.prefs.privacy_mask {
                    let mut inline_actions: Vec<El> = Vec::new();
                    if let Some(code) = extract_verification_code(&m.text) {
                        let pickup = is_pickup_code_text(&code, &m.text);
                        inline_actions.push(self.button_icon(
                            format!("im-msg-otp-{idx}"),
                            if pickup {
                                format!("复制取件码: {code}")
                            } else {
                                format!("复制验证码: {code}")
                            },
                            if pickup {
                                UiIcon::Package
                            } else {
                                UiIcon::Copy
                            },
                            Action::CopyText(
                                code.clone(),
                                if pickup {
                                    format!("已复制取件码 {code}")
                                } else {
                                    format!("已复制验证码 {code}")
                                },
                            ),
                            Btn::Primary,
                        ));
                    }
                    if let Some(url) = extract_first_url(&m.text) {
                        inline_actions.push(self.button(
                            format!("im-msg-url-{idx}"),
                            "打开链接",
                            Action::StudioOpenUrl(url),
                            Btn::Secondary,
                        ));
                    }
                    inline_actions.push(self.button(
                        format!("im-msg-copy-{idx}"),
                        "复制",
                        Action::CopyText(m.text.clone(), "已复制消息内容".into()),
                        Btn::Ghost,
                    ));
                    msg_col = msg_col.with_child(self.wrap(inline_actions));
                }

                let in_el = msg_col.finish();
                let state = self.mouse_state(format!("im-msg-in:{}:{idx}", t.thread_id));
                let masked = self.push.prefs.privacy_mask;
                let txt_dbl = if masked {
                    msg_text.clone()
                } else {
                    m.text.clone()
                };
                let otp_rc = if masked {
                    None
                } else {
                    extract_verification_code(&m.text)
                };
                let tid_rc = t.thread_id.clone();
                let pkg_rc = t.package_name.clone();
                let title_rc = t.title.clone();
                let sender_rc = sender_name.to_string();
                let txt_rc = txt_dbl.clone();
                let nkey_rc = t.notification_key.clone();
                msg_items.push(
                    Hoverable::new(state, move |_| in_el)
                        .with_defer_events_to_children()
                        .with_cursor(Cursor::PointingHand)
                        .on_double_click(move |ctx, _, _| {
                            if !masked {
                                if let Some(code) = extract_verification_code(&txt_dbl) {
                                    let pickup = is_pickup_code_text(&code, &txt_dbl);
                                    ctx.dispatch_typed_action(Action::CopyText(
                                        code.clone(),
                                        if pickup {
                                            format!("已复制取件码 {code}")
                                        } else {
                                            format!("已复制验证码 {code}")
                                        },
                                    ));
                                    return;
                                }
                            }
                            ctx.dispatch_typed_action(Action::CopyText(
                                txt_dbl.clone(),
                                "已复制消息内容".into(),
                            ));
                        })
                        .on_right_click(move |ctx, _, at| {
                            ctx.dispatch_typed_action(Action::ContextOpen(
                                crate::ui::context_menu::Target::ImMessage {
                                    thread_id: tid_rc.clone(),
                                    package_name: pkg_rc.clone(),
                                    title: title_rc.clone(),
                                    sender: sender_rc.clone(),
                                    text: txt_rc.clone(),
                                    notification_key: nkey_rc.clone(),
                                    otp_code: otp_rc.clone(),
                                },
                                at,
                            ));
                        })
                        .finish(),
                );
            }
        }

        let stream_scroll = self.pane_scroll(
            self.sms_messages_scroll.clone(),
            Container::new(self.vstack(14., msg_items))
                .with_horizontal_padding(S5)
                .with_padding_top(S4)
                .with_padding_bottom(S5)
                .finish(),
        );

        // Bottom Direct Reply Composer
        let quick_phrases = [
            ("收到，谢谢", "收到，谢谢"),
            ("稍后回电", "稍后回电"),
            ("正在开会，稍后联系", "正在开会，稍后联系"),
            ("快递请放门口/驿站", "快递请放门口/驿站"),
            ("我马上到", "我马上到"),
            ("好的，没问题", "好的，没问题"),
        ];
        let mut quick_chips: Vec<El> = quick_phrases
            .into_iter()
            .enumerate()
            .map(|(idx, (btn_label, phrase))| {
                self.button(
                    format!("im-qreply-{idx}"),
                    btn_label,
                    Action::Push(PushAction::QuickReplyPhrase {
                        thread_id: t.thread_id.clone(),
                        package_name: t.package_name.clone(),
                        title: t.title.clone(),
                        text: phrase.to_string(),
                    }),
                    Btn::Secondary,
                )
            })
            .collect();

        quick_chips.push(self.button(
            "im-qreply-ai",
            "AI 帮回",
            Action::Push(PushAction::AiDraftReply {
                thread_id: t.thread_id.clone(),
                package_name: t.package_name.clone(),
                title: t.title.clone(),
            }),
            Btn::Secondary,
        ));

        let is_sending = self.push.sending_thread.as_deref() == Some(t.thread_id.as_str());
        let status_node = if is_sending {
            Some(self.hstack(
                6.,
                vec![
                    self.dot(p.warning),
                    self.line("正在发送回复…", MICRO, p.warning),
                ],
            ))
        } else {
            self.push.last_reply_status.as_ref().map(|st| {
                self.line(
                    st,
                    MICRO,
                    if st.starts_with("回复未完成") {
                        p.danger
                    } else {
                        p.text2
                    },
                )
            })
        };

        let quick_bar = match status_node {
            Some(st) => self.between(self.wrap(quick_chips), st),
            None => self.wrap(quick_chips),
        };

        let composer_input_row = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(
                    Expanded::new(
                        1.,
                        self.input(F_IM_REPLY, "输入回复内容，按回车发送…", None),
                    )
                    .finish(),
                )
                .with_child(self.button(
                    "im-send-reply",
                    if is_sending { "发送中" } else { "发送" },
                    Action::Push(PushAction::SendImReply),
                    Btn::Primary,
                ))
                .finish(),
        )
        .with_uniform_padding(8.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let mut composer_rows = vec![quick_bar];
        if !t.can_reply() {
            composer_rows.push(self.button(
                "im-fallback-open-app",
                "通知快捷句柄已过期 · 在手机打开应用回复",
                Action::OpenApp(t.package_name.clone()),
                Btn::Secondary,
            ));
        }
        composer_rows.push(composer_input_row);

        let bottom_composer = Container::new(self.vstack(8., composer_rows))
            .with_horizontal_padding(S5)
            .with_padding_top(S3)
            .with_padding_bottom(S4)
            .with_background_color(p.bg)
            .with_border(Border::top(1.).with_border_color(p.border))
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(dialog_header)
            .with_child(Expanded::new(1., stream_scroll).finish())
            .with_child(bottom_composer)
            .finish()
    }

    pub(in super::super) fn sms_page(&self) -> El {
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
                            "连接手机后查看消息与短信",
                            "连接后将同步微信、企业微信、钉钉、飞书会话以及手机原生短信。",
                            Some(self.button(
                                "sms-connect",
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
            && self.push.snapshot.threads.is_empty()
            && self.push.snapshot.notifications.is_empty()
        {
            demo_fallback = demo_push_snapshot();
            &demo_fallback
        } else {
            &self.push.snapshot
        };

        let loaded = self.sms_loaded || self.demo_mode;
        let search_q = self.fields[F_SMS_SEARCH].text.trim().to_lowercase();
        let filtered = self.sms_filter != SmsFilter::All || !search_q.is_empty();

        // Build native SMS threads from the phone provider snapshot.
        let mut display_threads: Vec<DisplaySmsThread> = Vec::new();
        for rt in self.sms_threads() {
            let (name, badge) = resolve_sender_info(&rt.address, &self.contacts);
            let one_line: String = rt
                .last_body
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let preview = if one_line.chars().count() > 38 {
                format!("{}…", one_line.chars().take(38).collect::<String>())
            } else {
                one_line
            };
            let otp_code = extract_verification_code(&rt.last_body);
            display_threads.push(DisplaySmsThread {
                name,
                num: rt.address,
                time: format_sms_time(rt.last_ms),
                preview,
                last_body: rt.last_body,
                unread: rt.unread,
                badge,
                count: rt.count,
                otp_code,
            });
        }

        // Filter IM threads from `snapshot.threads`
        let matching_im_threads: Vec<&ImThread> = if self.push.im_channel == ImChannel::Sms {
            Vec::new()
        } else {
            snapshot
                .threads
                .iter()
                .filter(|t| {
                    !stepx_companion::push::is_excluded_im_package(&t.package_name)
                        && !(t.last_message.trim().is_empty()
                            && t.messages.iter().all(|m| m.text.trim().is_empty()))
                        && !t.thread_id.contains("stepx_e2e_test")
                        && !t.thread_id.contains("stepx_test_tag")
                })
                .filter(|t| self.push.im_channel.matches_package(&t.package_name))
                .filter(|t| {
                    if search_q.is_empty() {
                        return true;
                    }
                    t.title.to_lowercase().contains(&search_q)
                        || t.app_name.to_lowercase().contains(&search_q)
                        || t.last_sender.to_lowercase().contains(&search_q)
                        || t.last_message.to_lowercase().contains(&search_q)
                        || t.messages
                            .iter()
                            .any(|m| m.text.to_lowercase().contains(&search_q))
                })
                .collect()
        };

        let is_new_sms = self.selected_thread.as_deref() == Some("__new__");

        // Determine whether an IM thread or a native SMS thread is currently active in the right pane
        let active_im_thread: Option<&ImThread> =
            if is_new_sms || self.push.im_channel == ImChannel::Sms {
                None
            } else if let Some(sel_id) = self.push.selected_im_thread.as_deref() {
                if sel_id.is_empty() {
                    None
                } else {
                    matching_im_threads
                        .iter()
                        .copied()
                        .find(|t| t.thread_id == sel_id)
                        .or_else(|| snapshot.threads.iter().find(|t| t.thread_id == sel_id))
                }
            } else if self.selected_thread.is_none() || self.push.im_channel != ImChannel::All {
                matching_im_threads.first().copied()
            } else {
                None
            };

        let active_im_id = active_im_thread.map(|t| t.thread_id.as_str()).unwrap_or("");

        let active_addr = if is_new_sms {
            "__new__".to_string()
        } else if active_im_thread.is_some() {
            String::new()
        } else if let Some(sel) = &self.selected_thread {
            sel.clone()
        } else if let Some(first) = display_threads.first() {
            first.num.clone()
        } else {
            String::new()
        };

        // Top Channel Bar (`ImChannel`)
        let channel_tabs = [
            (ImChannel::All, "全部消息"),
            (ImChannel::WeChat, "微信"),
            (ImChannel::WeCom, "企微"),
            (ImChannel::DingTalk, "钉钉"),
            (ImChannel::Feishu, "飞书"),
            (ImChannel::QqTim, "QQ/TIM"),
            (ImChannel::GlobalIm, "国际IM"),
            (ImChannel::Sms, "原生短信"),
        ];
        let channel_pills: Vec<El> = channel_tabs
            .into_iter()
            .map(|(ch, label)| {
                self.im_channel_pill(
                    format!("ch-{ch:?}"),
                    label,
                    self.push.im_channel == ch,
                    Action::Push(PushAction::SelectImChannel(ch)),
                )
            })
            .collect();

        let filter_bar = self.segmented(
            "sms-filter-seg",
            vec![
                (
                    if loaded {
                        format!("全部 {}", self.sms.len())
                    } else {
                        "全部".into()
                    },
                    self.sms_filter == SmsFilter::All,
                    Action::SmsFilter(SmsFilter::All),
                ),
                (
                    if loaded {
                        format!("收件箱 {}", self.sms.iter().filter(|m| m.kind == 1).count())
                    } else {
                        "收件箱".into()
                    },
                    self.sms_filter == SmsFilter::Incoming,
                    Action::SmsFilter(SmsFilter::Incoming),
                ),
                (
                    if loaded {
                        format!("已发送 {}", self.sms.iter().filter(|m| m.kind == 2).count())
                    } else {
                        "已发送".into()
                    },
                    self.sms_filter == SmsFilter::Outgoing,
                    Action::SmsFilter(SmsFilter::Outgoing),
                ),
            ],
        );

        let mut thread_rows: Vec<El> = Vec::new();

        // 1. Render matching IM threads first when channel != ImChannel::Sms
        for (i, t) in matching_im_threads.iter().enumerate() {
            let active = !is_new_sms && t.thread_id == active_im_id;
            let app_label = if t.app_name.is_empty() {
                t.channel().label()
            } else {
                t.app_name.as_str()
            };
            let (badge_txt, badge_bg, badge_fg) = im_avatar_style(&t.package_name, app_label);
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

            let display_title = compact_thread_title(&t.title, 11);
            let is_long_title = t.title.trim().chars().count() > 11;
            let mut title_badges = vec![
                self.line_w(display_title, CAPTION, p.text, Weight::Semibold),
                self.pill_badge(app_label, p.text2, p.tag_bg),
            ];
            if !is_long_title {
                if t.can_remote_reply {
                    title_badges.push(self.pill_badge("可秒回", p.success_text, p.success_soft));
                } else if t.can_ui_reply {
                    title_badges.push(self.pill_badge("看屏回", p.text2, p.tag_bg));
                }
            }

            let raw_preview = if self.push.prefs.privacy_mask {
                "[防窥保护已开启]".to_string()
            } else if !t.last_sender.is_empty()
                && t.last_sender != t.title
                && !t.last_message.starts_with(&t.last_sender)
            {
                format!("{}: {}", t.last_sender, t.last_message)
            } else {
                t.last_message.clone()
            };
            let single_line: String = raw_preview
                .lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .collect::<Vec<_>>()
                .join(" ");
            let preview_str = if single_line.chars().count() > 36 {
                format!("{}…", single_line.chars().take(36).collect::<String>())
            } else {
                single_line
            };

            let has_unread = t.unread_count > 0;
            let preview_el = self.line_w(
                preview_str,
                CAPTION,
                if has_unread { p.text } else { p.text2 },
                if has_unread {
                    Weight::Medium
                } else {
                    Weight::Normal
                },
            );

            let bottom_row = if has_unread {
                let unread_pill =
                    self.pill_badge(format!("{}", t.unread_count), ColorU::white(), p.danger);
                self.between(preview_el, unread_pill)
            } else {
                preview_el
            };

            let im_time_el = facloud_ui::elements::Text::new(
                super::format_time(t.last_timestamp_ms),
                self.mono_font,
                MICRO,
            )
            .with_color(p.text3)
            .soft_wrap(false)
            .finish();

            let right_col = Flex::column()
                .with_spacing(4.)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.between(self.hstack(6., title_badges), im_time_el))
                .with_child(bottom_row)
                .finish();

            let row_inner = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_spacing(12.)
                .with_child(avatar)
                .with_child(Expanded::new(1., right_col).finish())
                .finish();

            let state = self.mouse_state(format!("im-th:{i}"));
            let tid = t.thread_id.clone();
            let tid_dbl = t.thread_id.clone();
            let tid_rc = t.thread_id.clone();
            let pkg_rc = t.package_name.clone();
            let title_rc = t.title.clone();
            let last_msg_rc = t.last_message.clone();
            let last_msg_dbl = t.last_message.clone();
            let nkey_dbl = t.notification_key.clone();
            let nkey_rc = t.notification_key.clone();
            let im_otp = if self.push.prefs.privacy_mask {
                None
            } else {
                t.messages
                    .iter()
                    .rev()
                    .find_map(|m| extract_verification_code(&m.text))
                    .or_else(|| extract_verification_code(&t.last_message))
            };
            let im_otp_dbl = im_otp.clone();
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
                } else if active {
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
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::SelectSmsThread(None));
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(tid.clone())));
            })
            .on_double_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::SelectSmsThread(None));
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                    tid_dbl.clone(),
                )));
                if let Some(code) = &im_otp_dbl {
                    let pickup = is_pickup_code_text(code, &last_msg_dbl);
                    ctx.dispatch_typed_action(Action::CopyText(
                        code.clone(),
                        if pickup {
                            format!("已复制取件码 {code}")
                        } else {
                            format!("已复制验证码 {code}")
                        },
                    ));
                } else if !nkey_dbl.is_empty() {
                    ctx.dispatch_typed_action(Action::Push(PushAction::OpenOnPhone(
                        nkey_dbl.clone(),
                    )));
                }
            })
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::SelectSmsThread(None));
                ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(tid_rc.clone())));
                ctx.dispatch_typed_action(Action::ContextOpen(
                    crate::ui::context_menu::Target::ImThread {
                        thread_id: tid_rc.clone(),
                        package_name: pkg_rc.clone(),
                        title: title_rc.clone(),
                        last_message: last_msg_rc.clone(),
                        notification_key: nkey_rc.clone(),
                        otp_code: im_otp.clone(),
                    },
                    at,
                ));
            })
            .finish();

            thread_rows.push(row_el);
        }

        // 2. Render native SMS threads when channel is `All` or `Sms`
        let show_native_sms = matches!(self.push.im_channel, ImChannel::All | ImChannel::Sms);
        if show_native_sms {
            for (i, t) in display_threads.iter().enumerate() {
                let active = !is_new_sms
                    && active_im_thread.is_none()
                    && !active_addr.is_empty()
                    && norm_addr(&active_addr) == norm_addr(&t.num);
                let avatar_bg = if active {
                    p.accent
                } else if p.dark {
                    rgb(0x27, 0x27, 0x2A)
                } else {
                    rgb(0xEF, 0xEF, 0xF2)
                };
                let avatar_fg = if active {
                    p.accent_fg
                } else if p.dark {
                    p.text2
                } else {
                    rgb(0x52, 0x52, 0x5B)
                };
                let avatar = ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Center)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(self.line_w(&t.badge, CAPTION, avatar_fg, Weight::Semibold))
                            .finish(),
                    )
                    .with_background_color(avatar_bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(18.)))
                    .finish(),
                )
                .with_width(36.)
                .with_height(36.)
                .finish();

                let preview_line = self.line_w(
                    &t.preview,
                    CAPTION,
                    if t.unread { p.text } else { p.text2 },
                    if t.unread {
                        Weight::Medium
                    } else {
                        Weight::Normal
                    },
                );
                let bottom_row = if t.unread {
                    self.between(preview_line, self.dot(rgb(0x25, 0x63, 0xEB)))
                } else {
                    preview_line
                };

                let display_name = compact_thread_title(&t.name, 11);
                let is_long_name = t.name.trim().chars().count() > 11;
                let mut title_items =
                    vec![self.line_w(display_name, CAPTION, p.text, Weight::Semibold)];
                if let Some(code) = &t.otp_code {
                    let pickup = is_pickup_code_text(code, &t.preview);
                    title_items.push(self.pill_badge(
                        if pickup {
                            format!("取件码 {code}")
                        } else {
                            format!("验证码 {code}")
                        },
                        p.warning,
                        p.warning_soft,
                    ));
                } else {
                    title_items.push(self.pill_badge(
                        if filtered {
                            format!("短信 · 匹配 {}", t.count)
                        } else if is_long_name {
                            "短信".to_string()
                        } else {
                            format!("短信 · {} 条", t.count)
                        },
                        p.text2,
                        p.tag_bg,
                    ));
                }

                let sms_time_el =
                    facloud_ui::elements::Text::new(t.time.clone(), self.mono_font, MICRO)
                        .with_color(p.text2)
                        .soft_wrap(false)
                        .finish();

                let has_distinct_name = !t.name.trim().is_empty()
                    && t.name.trim() != t.num.trim()
                    && norm_addr(&t.name) != norm_addr(&t.num);

                let mut right_col = Flex::column()
                    .with_spacing(3.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.between(self.hstack(6., title_items), sms_time_el));
                if has_distinct_name {
                    right_col = right_col.with_child(self.mono(&t.num, CAPTION, p.text2));
                }
                let right_col = right_col.with_child(bottom_row).finish();

                let row_inner = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(12.)
                    .with_child(avatar)
                    .with_child(Expanded::new(1., right_col).finish())
                    .finish();

                let state = self.mouse_state(format!("sms-th:{i}"));
                let num_str = t.num.clone();
                let num_dbl = t.num.clone();
                let otp_dbl = t.otp_code.clone();
                let body_dbl = t.last_body.clone();
                let num_rc = t.num.clone();
                let name_rc = t.name.clone();
                let body_rc = t.last_body.clone();
                let otp_rc = t.otp_code.clone();
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
                        .with_uniform_padding(12.)
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                        String::new(),
                    )));
                    ctx.dispatch_typed_action(Action::SelectSmsThread(Some(num_str.clone())));
                })
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                        String::new(),
                    )));
                    ctx.dispatch_typed_action(Action::SelectSmsThread(Some(num_dbl.clone())));
                    if let Some(code) = &otp_dbl {
                        let pickup = is_pickup_code_text(code, &body_dbl);
                        ctx.dispatch_typed_action(Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ));
                    }
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectImThread(
                        String::new(),
                    )));
                    ctx.dispatch_typed_action(Action::SelectSmsThread(Some(num_rc.clone())));
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::SmsThread {
                            address: num_rc.clone(),
                            name: name_rc.clone(),
                            last_body: body_rc.clone(),
                            otp_code: otp_rc.clone(),
                        },
                        at,
                    ));
                })
                .finish();
                thread_rows.push(row_el);
            }
        }

        if show_native_sms {
            if let Some(error) = &self.sms_error {
                if thread_rows.is_empty() {
                    thread_rows.insert(
                        0,
                        self.empty(
                            "短信读取失败",
                            error.clone(),
                            Some(self.button(
                                "sms-read-retry",
                                "重试读取",
                                Action::ReadSms,
                                Btn::Secondary,
                            )),
                        ),
                    );
                }
            } else if !loaded && thread_rows.is_empty() {
                thread_rows.push(self.empty(
                    if self.busy {
                        "正在读取手机短信…"
                    } else {
                        "尚未读取短信"
                    },
                    "读取完成后显示会话与短信数量。",
                    if self.busy {
                        None
                    } else {
                        Some(self.button(
                            "sms-first-read",
                            "读取短信",
                            Action::ReadSms,
                            Btn::Secondary,
                        ))
                    },
                ));
            }
        }

        if thread_rows.is_empty() {
            let is_sms_only = self.push.im_channel == ImChannel::Sms;
            thread_rows.push(self.empty(
                if is_sms_only {
                    if self.sms.is_empty() {
                        "手机暂无短信"
                    } else {
                        "没有匹配的短信"
                    }
                } else {
                    "暂无匹配的聊天会话"
                },
                if is_sms_only {
                    "收到或发送短信后，刷新即可查看。"
                } else {
                    "当手机收到微信、企微、钉钉、飞书或 QQ 消息时将自动生成会话并支持直接秒回。"
                },
                Some(self.button(
                    if is_sms_only {
                        "sms-empty-refresh"
                    } else {
                        "im-empty-sync-win"
                    },
                    if is_sms_only {
                        "刷新短信"
                    } else {
                        "同步聊天屏"
                    },
                    if is_sms_only {
                        Action::ReadSms
                    } else {
                        Action::Push(PushAction::SyncActivePhoneChatWindow)
                    },
                    Btn::Secondary,
                )),
            ));
        }

        let left_scroll = self.pane_scroll(
            self.sms_threads_scroll.clone(),
            Container::new(self.vstack(2., thread_rows))
                .with_horizontal_padding(S2)
                .with_vertical_padding(S2)
                .finish(),
        );

        let search_and_new_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(8.)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(
                Expanded::new(
                    1.,
                    self.input(
                        F_SMS_SEARCH,
                        "搜索微信/钉钉/飞书会话、联系人、号码或验证码…",
                        None,
                    ),
                )
                .finish(),
            )
            .with_child(self.button_icon(
                "sms-left-new",
                "新建短信",
                UiIcon::Plus,
                Action::NewSms,
                Btn::Primary,
            ))
            .finish();

        let mut left_top_controls = vec![search_and_new_row, self.wrap(channel_pills)];
        if self.push.im_channel == ImChannel::Sms {
            left_top_controls.push(filter_bar);
        }
        left_top_controls.push(self.line(
            format!(
                "{} 个即时通讯会话 · {} 个短信会话{}",
                matching_im_threads.len(),
                display_threads.len(),
                if filtered { "（已筛选）" } else { "" }
            ),
            11.,
            p.text3,
        ));

        // Top Latest Verification / Pickup Code Quick-Copy Bar
        let top_code_bar: Option<El> = if self.push.prefs.privacy_mask {
            None
        } else {
            let latest_otp = self
                .sms
                .iter()
                .find_map(|m| {
                    extract_verification_code(&m.body)
                        .filter(|code| !is_pickup_code_text(code, &m.body))
                })
                .or_else(|| {
                    display_threads.iter().find_map(|t| {
                        t.otp_code
                            .as_ref()
                            .filter(|c| !is_pickup_code_text(c, &t.last_body))
                            .cloned()
                    })
                })
                .or_else(|| {
                    snapshot
                        .otp_banners
                        .iter()
                        .find(|b| !is_pickup_code_text(&b.code, &b.raw_text))
                        .map(|b| b.code.clone())
                });

            let latest_pickup = self
                .sms
                .iter()
                .find_map(|m| extract_pickup_code(&m.body))
                .or_else(|| {
                    display_threads
                        .iter()
                        .find_map(|t| extract_pickup_code(&t.last_body))
                })
                .or_else(|| {
                    snapshot
                        .otp_banners
                        .iter()
                        .find(|b| is_pickup_code_text(&b.code, &b.raw_text))
                        .map(|b| b.code.clone())
                })
                .or_else(|| {
                    snapshot
                        .notifications
                        .iter()
                        .filter(|n| !n.dismissed)
                        .find_map(|n| extract_pickup_code(&n.text))
                });

            if latest_otp.is_some() || latest_pickup.is_some() {
                let mut left_info = vec![
                    self.dot(p.accent),
                    self.line_w("最新验证码 / 取件码速取", CAPTION, p.text, Weight::Semibold),
                ];
                let mut right_btns = Vec::new();
                if let Some(code) = &latest_otp {
                    left_info.push(self.pill_badge_icon(
                        format!("验证码 {code}"),
                        UiIcon::Key,
                        p.accent,
                        p.accent_soft,
                    ));
                    right_btns.push(self.button_icon(
                        "sms-top-copy-otp",
                        format!("复制验证码 {code}"),
                        UiIcon::Clipboard,
                        Action::CopyText(code.clone(), format!("已复制验证码 {code}")),
                        Btn::Primary,
                    ));
                }
                if let Some(pickup) = &latest_pickup {
                    left_info.push(self.pill_badge_icon(
                        format!("取件码 {pickup}"),
                        UiIcon::Package,
                        p.warning,
                        p.warning_soft,
                    ));
                    right_btns.push(self.button_icon(
                        "sms-top-copy-pickup",
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
                        "sms-top-push-pickup-back",
                        "投送到手机背屏",
                        UiIcon::Matrix,
                        Action::AmooPushPickupCode(pickup.clone()),
                        Btn::Secondary,
                    ));
                }
                Some(
                    Container::new(
                        self.between(self.hstack(8., left_info), self.hstack(8., right_btns)),
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

        let left_pane = self.fixed_w(
            LIST_PANE_W + 24.,
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(
                        Container::new(self.vstack(8., left_top_controls))
                            .with_horizontal_padding(S4)
                            .with_vertical_padding(S3)
                            .with_border(Border::bottom(1.).with_border_color(p.border))
                            .finish(),
                    )
                    .with_child(Expanded::new(1., left_scroll).finish())
                    .finish(),
            )
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::right(1.).with_border_color(p.border))
            .finish(),
        );

        // If an IM thread is selected, render the IM chat bubble dialog & direct reply composer
        if let Some(im_thread) = active_im_thread {
            let right_pane = self.im_thread_dialog(im_thread, snapshot);
            let split = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(left_pane)
                .with_child(Expanded::new(1., right_pane).finish())
                .finish();
            let mut page_col = Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.comm_mode_bar());
            if let Some(bar) = top_code_bar {
                page_col = page_col.with_child(bar);
            }
            return page_col
                .with_child(Expanded::new(1., split).finish())
                .finish();
        }

        // Otherwise render the native SMS conversation & SMS composer unchanged
        let selected_thread_info = display_threads
            .iter()
            .find(|t| norm_addr(&t.num) == norm_addr(&active_addr));

        let (peer_name, peer_num) = if is_new_sms {
            let typed = self.fields[F_SMS_TO].text.trim();
            (
                "新建短信会话".to_string(),
                if typed.is_empty() {
                    String::new()
                } else {
                    typed.to_string()
                },
            )
        } else if let Some(info) = selected_thread_info {
            (info.name.clone(), info.num.clone())
        } else if active_addr.is_empty() {
            ("短信会话".into(), String::new())
        } else {
            let (n, _) = resolve_sender_info(&active_addr, &self.contacts);
            (n, active_addr.clone())
        };

        let sims = self
            .capabilities
            .as_ref()
            .map(|c| c.subscriptions.as_slice())
            .unwrap_or(&[]);
        let selected_sim = self
            .selected_sim
            .or_else(|| self.capabilities.as_ref().map(|c| c.default_subscription));
        let real_sim_badge = sims
            .iter()
            .find(|s| Some(s.id) == selected_sim)
            .map(|s| {
                format!(
                    "卡 {} · {}",
                    s.slot.max(1),
                    stepx_companion::bridge::friendly_carrier_name(&s.label)
                )
            })
            .unwrap_or_else(|| "请选择可用 SIM".into());
        let can_send = self.capabilities.as_ref().is_some_and(|c| c.send_sms)
            && sims.iter().any(|s| Some(s.id) == selected_sim);
        let has_recipient = !peer_num.trim().is_empty();
        let real_msgs = self.thread_messages(&active_addr);
        let latest_sms_otp = real_msgs.iter().rev().find_map(|m| {
            extract_verification_code(&m.body)
                .map(|code| (code.clone(), is_pickup_code_text(&code, &m.body)))
        });

        let mut peer_labels = vec![self.line_w(&peer_name, CAPTION, p.text, Weight::Semibold)];
        if has_recipient {
            if peer_name != peer_num {
                peer_labels.push(self.mono(&peer_num, CAPTION, p.text2));
            }
            peer_labels.push(self.tag(&real_sim_badge, p.text2));
        }
        let mut peer_actions = Vec::new();
        if is_new_sms {
            peer_actions.push(self.button(
                "sms-cancel-new",
                "取消新建",
                Action::SelectSmsThread(None),
                Btn::Secondary,
            ));
        }
        if let Some((code, pickup)) = &latest_sms_otp {
            peer_actions.push(self.button_icon(
                "sms-hdr-copy-otp",
                if *pickup {
                    format!("复制取件码: {code}")
                } else {
                    format!("复制验证码: {code}")
                },
                if *pickup {
                    UiIcon::Package
                } else {
                    UiIcon::Copy
                },
                Action::CopyText(
                    code.clone(),
                    if *pickup {
                        format!("已复制取件码 {code}")
                    } else {
                        format!("已复制验证码 {code}")
                    },
                ),
                Btn::Primary,
            ));
        }
        if has_recipient {
            peer_actions.push(self.button_icon(
                "sms-call-peer",
                "拨号呼叫",
                UiIcon::Phone,
                Action::PhoneCall(peer_num.clone()),
                Btn::Secondary,
            ));
            peer_actions.push(self.button_icon(
                "sms-copy-num",
                "复制号码",
                UiIcon::Clipboard,
                Action::CopyText(peer_num.clone(), format!("已复制号码 {peer_num}")),
                Btn::Secondary,
            ));
        }

        // Right Conversation Detail Pane
        let subheader = ConstrainedBox::new(
            Container::new(
                self.between(self.hstack(10., peer_labels), self.hstack(8., peer_actions)),
            )
            .with_horizontal_padding(S5)
            .with_vertical_padding(8.)
            .with_background_color(p.bg)
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish(),
        )
        .with_height(48.)
        .finish();

        let date_pill = |label: &str| -> El {
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_child(
                    Container::new(self.line(label, CAPTION, p.text2))
                        .with_horizontal_padding(10.)
                        .with_vertical_padding(2.)
                        .with_background_color(if p.dark {
                            rgb(0x22, 0x22, 0x27)
                        } else {
                            rgb(0xEF, 0xEF, 0xF2)
                        })
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                        .finish(),
                )
                .finish()
        };

        let incoming_bubble = |idx: usize, txt: &str, meta: &str| -> El {
            let b = Container::new(self.text(txt.to_string(), CAPTION, p.text))
                .with_horizontal_padding(16.)
                .with_vertical_padding(10.)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
                .finish();
            let mut bubble_actions: Vec<El> = Vec::new();
            let detected_otp = extract_verification_code(txt);
            if let Some(code) = &detected_otp {
                let pickup = is_pickup_code_text(code, txt);
                bubble_actions.push(self.button_icon(
                    format!("sms-in-otp-{idx}"),
                    if pickup {
                        format!("复制取件码: {code}")
                    } else {
                        format!("复制验证码: {code}")
                    },
                    if pickup {
                        UiIcon::Package
                    } else {
                        UiIcon::Copy
                    },
                    Action::CopyText(
                        code.clone(),
                        if pickup {
                            format!("已复制取件码 {code}")
                        } else {
                            format!("已复制验证码 {code}")
                        },
                    ),
                    Btn::Primary,
                ));
            }
            if let Some(url) = extract_first_url(txt) {
                bubble_actions.push(self.button(
                    format!("sms-in-url-{idx}"),
                    "打开链接",
                    Action::StudioOpenUrl(url),
                    Btn::Secondary,
                ));
            }
            bubble_actions.push(self.button(
                format!("sms-in-copy-{idx}"),
                "复制",
                Action::CopyText(txt.to_string(), "已复制短信正文".into()),
                Btn::Ghost,
            ));
            let col = Flex::column()
                .with_spacing(6.)
                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                .with_child(ConstrainedBox::new(b).with_max_width(540.).finish())
                .with_child(self.wrap(bubble_actions))
                .with_child(
                    Container::new(self.mono(meta.to_string(), CAPTION, p.text2))
                        .with_padding_left(4.)
                        .finish(),
                )
                .finish();
            let state = self.mouse_state(format!("sms-bub-in:{idx}"));
            let txt_dbl = txt.to_string();
            let txt_rc = txt.to_string();
            let otp_rc = detected_otp;
            let addr_rc = peer_num.clone();
            let sender_rc = peer_name.clone();
            Hoverable::new(state, move |_| col)
                .with_defer_events_to_children()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    if let Some(code) = extract_verification_code(&txt_dbl) {
                        let pickup = is_pickup_code_text(&code, &txt_dbl);
                        ctx.dispatch_typed_action(Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ));
                    } else {
                        ctx.dispatch_typed_action(Action::CopyText(
                            txt_dbl.clone(),
                            "已复制短信全文".into(),
                        ));
                    }
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::SmsMessage {
                            address: addr_rc.clone(),
                            sender: sender_rc.clone(),
                            body: txt_rc.clone(),
                            otp_code: otp_rc.clone(),
                        },
                        at,
                    ));
                })
                .finish()
        };

        let outgoing_bubble = |idx: usize, txt: &str, meta: &str| -> El {
            let b = Container::new(self.text(txt.to_string(), CAPTION, p.accent_fg))
                .with_horizontal_padding(16.)
                .with_vertical_padding(10.)
                .with_background_color(p.accent)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
                .finish();
            let bubble_actions: Vec<El> = vec![self.button(
                format!("sms-out-copy-{idx}"),
                "复制",
                Action::CopyText(txt.to_string(), "已复制短信正文".into()),
                Btn::Ghost,
            )];
            let row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::End)
                .with_child(
                    Flex::column()
                        .with_spacing(6.)
                        .with_cross_axis_alignment(CrossAxisAlignment::End)
                        .with_child(ConstrainedBox::new(b).with_max_width(540.).finish())
                        .with_child(self.hstack(
                            8.,
                            vec![
                                Container::new(self.mono(meta.to_string(), CAPTION, p.text2))
                                    .with_padding_right(4.)
                                    .finish(),
                                self.wrap(bubble_actions),
                            ],
                        ))
                        .finish(),
                )
                .finish();
            let state = self.mouse_state(format!("sms-bub-out:{idx}"));
            let txt_dbl = txt.to_string();
            let txt_rc = txt.to_string();
            let otp_rc = extract_verification_code(txt);
            let addr_rc = peer_num.clone();
            let sender_rc = peer_name.clone();
            Hoverable::new(state, move |_| row)
                .with_defer_events_to_children()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    if let Some(code) = extract_verification_code(&txt_dbl) {
                        let pickup = is_pickup_code_text(&code, &txt_dbl);
                        ctx.dispatch_typed_action(Action::CopyText(
                            code.clone(),
                            if pickup {
                                format!("已复制取件码 {code}")
                            } else {
                                format!("已复制验证码 {code}")
                            },
                        ));
                    } else {
                        ctx.dispatch_typed_action(Action::CopyText(
                            txt_dbl.clone(),
                            "已复制短信全文".into(),
                        ));
                    }
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::SmsMessage {
                            address: addr_rc.clone(),
                            sender: sender_rc.clone(),
                            body: txt_rc.clone(),
                            otp_code: otp_rc.clone(),
                        },
                        at,
                    ));
                })
                .finish()
        };

        let mut timeline_children: Vec<El> = Vec::new();

        if is_new_sms {
            timeline_children.push(date_pill("新建短信 · 输入号码与正文后确认发送"));
        } else if !active_addr.is_empty() && real_msgs.is_empty() {
            timeline_children.push(date_pill("暂无短信，请选择会话或新建短信"));
        }

        if !real_msgs.is_empty() {
            timeline_children.push(date_pill(&format!("此会话共 {} 条短信", real_msgs.len())));
            for (idx, m) in real_msgs.iter().enumerate() {
                let ts = format_sms_time(m.date_ms);
                let sim = sims
                    .iter()
                    .find(|s| Some(s.id) == m.subscription)
                    .map(|s| {
                        format!(
                            "卡 {} · {}",
                            s.slot.max(1),
                            stepx_companion::bridge::friendly_carrier_name(&s.label)
                        )
                    })
                    .unwrap_or_else(|| "SIM 未知".into());
                let meta = format!("{ts} · {} · {sim}", m.status_label());
                if m.outgoing {
                    timeline_children.push(outgoing_bubble(idx, &m.body, &meta));
                } else {
                    timeline_children.push(incoming_bubble(idx, &m.body, &meta));
                }
            }
        }

        let timeline = if active_addr.is_empty() && !is_new_sms {
            Container::new(
                Align::new(self.empty(
                    if self.sms_error.is_some() {
                        "短信暂时无法读取"
                    } else if !loaded {
                        "等待短信同步"
                    } else if self.sms.is_empty() {
                        "手机暂无短信"
                    } else {
                        "请选择一个会话"
                    },
                    if self.sms_error.is_some() {
                        "请重试读取手机短信。"
                    } else if !loaded {
                        "同步完成后可在左侧选择会话。"
                    } else {
                        "也可以点击“新建短信”填写收件人与内容。"
                    },
                    None,
                ))
                .finish(),
            )
            .with_uniform_padding(S5)
            .finish()
        } else {
            let timeline_inner = Container::new(self.vstack(14., timeline_children))
                .with_horizontal_padding(S5)
                .with_padding_top(S4)
                .with_padding_bottom(S5)
                .finish();
            self.pane_scroll(self.sms_messages_scroll.clone(), timeline_inner)
        };

        // Inline Send Safety Confirmation Bar + Composer
        let sim_label = &real_sim_badge;
        let next_sim = match sims.iter().position(|s| Some(s.id) == selected_sim) {
            Some(i) => sims.get((i + 1) % sims.len()).map(|s| s.id),
            None => sims.first().map(|s| s.id),
        };

        let mut composer_parts: Vec<El> = Vec::new();

        if let Some(plan) = &self.pending_sms_plan {
            let safety_banner = Container::new(self.between(
                self.hstack(
                    8.,
                    vec![
                        self.dot(p.warning),
                        self.line_w("敏感操作二次确认：", CAPTION, p.text, Weight::Semibold),
                        self.line("将通过", CAPTION, p.text2),
                        self.line_w(&plan.subscription_label, CAPTION, p.text, Weight::Semibold),
                        self.line("发送至", CAPTION, p.text2),
                        self.mono(&plan.number, CAPTION, p.text),
                        self.pill_badge(format!("共 {} 段", plan.parts), p.warning, p.warning_soft),
                    ],
                ),
                self.hstack(
                    8.,
                    vec![
                        self.button(
                            "sms-cancel",
                            "取消",
                            Action::CancelDirectSms,
                            Btn::Secondary,
                        ),
                        self.button_icon(
                            "sms-confirm",
                            "确认立即发送",
                            UiIcon::Check,
                            Action::ConfirmDirectSms,
                            Btn::Primary,
                        ),
                    ],
                ),
            ))
            .with_horizontal_padding(14.)
            .with_vertical_padding(8.)
            .with_background_color(p.warning_soft)
            .with_border(Border::all(1.).with_border_color(p.warning))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();
            composer_parts.push(safety_banner);
        } else if !can_send {
            composer_parts.push(self.hstack(
                6.,
                vec![
                    self.dot(p.text3),
                    self.line(
                        if self.capabilities.is_none() {
                            "正在确认手机短信发送能力…"
                        } else {
                            "请确认手机短信权限与可用 SIM 卡，或点击右侧「填入手机草稿」"
                        },
                        MICRO,
                        p.text2,
                    ),
                ],
            ));
        }

        if (is_new_sms || !has_recipient) && !self.contacts.is_empty() {
            let typed_to = self.fields[F_SMS_TO].text.trim().to_lowercase();
            let matching_contacts: Vec<_> = self
                .contacts
                .iter()
                .filter(|c| {
                    typed_to.is_empty()
                        || c.name.to_lowercase().contains(&typed_to)
                        || c.number.to_lowercase().contains(&typed_to)
                })
                .take(8)
                .collect();
            if !matching_contacts.is_empty() {
                let mut contact_chips: Vec<El> = vec![self.line("联系人速选：", MICRO, p.text2)];
                for (c_idx, c) in matching_contacts.into_iter().enumerate() {
                    let active_c = norm_addr(&self.fields[F_SMS_TO].text) == norm_addr(&c.number)
                        && !c.number.trim().is_empty();
                    let label = if c.name.trim().is_empty() || c.name.trim() == c.number.trim() {
                        c.number.clone()
                    } else {
                        format!("{} ({})", c.name, c.number)
                    };
                    contact_chips.push(self.button_icon(
                        format!("sms-contact-pick-{c_idx}"),
                        label,
                        UiIcon::User,
                        Action::PhoneSmsTo(c.number.clone()),
                        if active_c {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ));
                }
                composer_parts.push(self.wrap(contact_chips));
            }
        }

        let sim_btn = self.button(
            "sms-sim-switch",
            format!("{sim_label} · 切换"),
            Action::SelectSim(next_sim),
            Btn::Secondary,
        );

        let mut composer_row_children: Vec<El> = vec![sim_btn];
        if is_new_sms || !has_recipient {
            composer_row_children.push(self.input(F_SMS_TO, "输入收件人号码…", Some(160.)));
        }
        composer_row_children.push(
            Expanded::new(
                1.,
                self.input(F_SMS_BODY, "输入短信内容，按 Enter 或点击发送…", None),
            )
            .finish(),
        );
        composer_row_children.push(self.button(
            "sms-send",
            if self.pending_sms_plan.is_some() {
                "确认发送"
            } else {
                "发送"
            },
            if self.pending_sms_plan.is_some() {
                Action::ConfirmDirectSms
            } else {
                Action::PrepareDirectSms
            },
            Btn::Primary,
        ));

        let composer_box = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(8.)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_children(composer_row_children)
                .finish(),
        )
        .with_uniform_padding(8.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let sms_quick_templates = [
            "收到，谢谢",
            "稍后回电",
            "正在开会，稍后联系",
            "快递请放门口/驿站",
            "我马上到",
            "好的，没问题",
        ];
        let mut sms_template_chips: Vec<El> = vec![self.line("快捷回复：", MICRO, p.text2)];
        for (idx, tpl) in sms_quick_templates.into_iter().enumerate() {
            let active_tpl = self.fields[F_SMS_BODY].text.trim() == tpl;
            sms_template_chips.push(self.button(
                format!("sms-tpl-chip-{idx}"),
                tpl,
                Action::Push(PushAction::FillSmsTemplate(tpl.to_string())),
                if active_tpl {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ));
        }
        if !self.fields[F_SMS_BODY].text.trim().is_empty() {
            sms_template_chips.push(self.button(
                "sms-tpl-clear",
                "清空输入",
                Action::ClearField(F_SMS_BODY),
                Btn::Ghost,
            ));
        }
        let sms_templates_bar = self.between(
            self.wrap(sms_template_chips),
            self.button(
                "sms-draft-phone",
                "填入手机草稿",
                Action::DraftSms,
                Btn::Ghost,
            ),
        );

        composer_parts.push(sms_templates_bar);
        if let Some(status) = &self.last_sms_status {
            composer_parts.push(self.between(
                self.text(status.summary_label(), CAPTION, p.text2),
                self.button(
                    "sms-check-status",
                    "查询发送状态",
                    Action::CheckSmsStatus(status.id.clone()),
                    Btn::Secondary,
                ),
            ));
        }
        composer_parts.push(composer_box);
        let bottom_area = Container::new(self.vstack(8., composer_parts))
            .with_horizontal_padding(S5)
            .with_padding_top(S3)
            .with_padding_bottom(S4)
            .with_background_color(p.bg)
            .with_border(Border::top(1.).with_border_color(p.border))
            .finish();

        let mut right_pane = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(subheader)
            .with_child(Expanded::new(1., timeline).finish());
        if has_recipient || is_new_sms {
            right_pane = right_pane.with_child(bottom_area);
        }
        let right_pane = right_pane.finish();

        let split = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(left_pane)
            .with_child(Expanded::new(1., right_pane).finish())
            .finish();

        let mut page_col = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(self.comm_mode_bar());
        if let Some(bar) = top_code_bar {
            page_col = page_col.with_child(bar);
        }
        page_col
            .with_child(Expanded::new(1., self.sub_enter(split)).finish())
            .finish()
    }
}
