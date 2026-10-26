//! Cross-platform interactive desktop popup notification cards (`push_toast.rs`).
//!
//! Renders an interactive floating toast stack in the bottom-right corner of the window
//! whenever new phone notifications arrive, supporting:
//! - Direct inline text input & Enter-to-send reply (`F_TOAST_REPLY`)
//! - Quick 1-click reply phrases & AI smart draft
//! - 1-click OTP / verification code copy & URL open
//! - Quick jump to `ShellMode::MiniNotify` or `ShellMode::Full`
//! - Privacy mask (`防窥脱敏`) and alert scope toggles

use super::{theme::*, widgets::*, Action, Companion, PushAction, ShellMode, F_TOAST_REPLY};
use facloud_ui::{elements::*, fonts::Weight};
use pathfinder_geometry::vector::vec2f;
use stepx_companion::desktop_notify::DesktopToastItem;

impl Companion {
    /// Returns `Some(El)` when there are active interactive desktop toast cards to display
    /// as a floating bottom-right overlay on top of the current shell.
    pub(super) fn push_toast_overlay(&self) -> Option<El> {
        if self.push.active_toasts.is_empty() {
            return None;
        }

        let p = self.p();
        let total = self.push.active_toasts.len();

        let mut stack_items: Vec<El> = Vec::new();

        // Header bar when multiple toasts are queued
        if total > 1 {
            let banner = ConstrainedBox::new(
                Container::new(self.between(
                    self.hstack(
                        S1,
                        vec![
                            self.dot(p.blue),
                            self.line_w(
                                format!("{total} 条新通知"),
                                CAPTION,
                                p.text,
                                Weight::Semibold,
                            ),
                        ],
                    ),
                    self.hstack(
                        S1,
                        vec![
                            self.button(
                                "toast-all-to-mini",
                                "查看全部",
                                Action::Push(PushAction::SetShellMode(ShellMode::MiniNotify)),
                                Btn::Ghost,
                            ),
                            self.button(
                                "toast-dismiss-all",
                                "全部忽略",
                                Action::Push(PushAction::DismissAllToasts),
                                Btn::Ghost,
                            ),
                        ],
                    ),
                ))
                .with_horizontal_padding(S3)
                .with_vertical_padding(S1 + 2.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border_strong))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            )
            .with_width(360.)
            .finish();
            stack_items.push(banner);
        }

        for (idx, item) in self.push.active_toasts.iter().take(3).enumerate() {
            stack_items.push(self.interactive_toast_card(idx, item, idx == 0));
        }

        let toast_column = ConstrainedBox::new(self.vstack(S2, stack_items))
            .with_width(360.)
            .finish();

        Some(
            Stack::new()
                .with_child(Empty::new().finish())
                .with_positioned_child(
                    toast_column,
                    OffsetPositioning::offset_from_parent(
                        vec2f(-16., -36.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::BottomRight,
                        ChildAnchor::BottomRight,
                    ),
                )
                .finish(),
        )
    }

    fn interactive_toast_card(&self, idx: usize, item: &DesktopToastItem, is_primary: bool) -> El {
        let p = self.p();
        let (app_fg, app_bg) = Self::im_package_colors(&item.package_name, &p);
        let app_badge = Self::short_app_badge(&item.package_name, &item.app_name);
        let time_str = Self::format_rel_time(item.timestamp_ms);

        let mut header_left: Vec<El> = vec![
            self.pill_badge(app_badge, app_fg, app_bg),
            self.line_w(&item.title, HEADING, p.text, Weight::Semibold),
        ];
        if item.can_reply {
            header_left.push(self.pill_badge("可回复", p.success_text, p.success_soft));
        } else if item.otp_code.is_some() {
            header_left.push(self.pill_badge("验证码", p.warning, p.warning_soft));
        }

        let header_right = self.hstack(
            S1,
            vec![
                self.line(time_str, MICRO, p.text3),
                self.button(
                    format!("toast-open-mini-{idx}"),
                    "详情",
                    Action::Push(PushAction::ToastOpenInMain(item.id.clone())),
                    Btn::Ghost,
                ),
                self.button(
                    format!("toast-dismiss-{idx}"),
                    "忽略",
                    Action::Push(PushAction::DismissToast(item.id.clone())),
                    Btn::Ghost,
                ),
            ],
        );

        let display_body = if self.push.prefs.privacy_mask {
            &item.masked_body
        } else {
            &item.body
        };

        let mut rows: Vec<El> = vec![
            self.between(self.hstack(S1, header_left), header_right),
            self.text(display_body.clone(), BODY, p.text),
        ];

        // One-click OTP copy or URL open bar
        if item.otp_code.is_some() || item.url_hint.is_some() {
            let mut action_btns: Vec<El> = Vec::new();
            if let Some(code) = &item.otp_code {
                action_btns.push(self.button(
                    format!("toast-copy-otp-{idx}"),
                    format!("复制验证码 {code}"),
                    Action::Push(PushAction::CopyOtp(code.clone())),
                    Btn::Primary,
                ));
            }
            if let Some(url) = &item.url_hint {
                action_btns.push(self.button(
                    format!("toast-open-url-{idx}"),
                    "打开链接",
                    Action::Push(PushAction::OpenUrl(url.clone())),
                    Btn::Secondary,
                ));
            }
            action_btns.push(self.button(
                format!("toast-open-phone-{idx}"),
                "手机查看",
                Action::Push(PushAction::OpenOnPhone(item.notification_key.clone())),
                Btn::Ghost,
            ));
            rows.push(self.wrap(action_btns));
        }

        // Interactive inline reply composer for IM / SMS toasts
        if item.can_reply && is_primary {
            let submit_msg = Action::Push(PushAction::ToastSendReply(item.id.clone()));
            let input_el = self.input(
                F_TOAST_REPLY,
                &format!("直接回复「{}」，按回车发送…", item.title),
                None,
            );
            let send_btn = self.button(
                format!("toast-send-reply-{idx}"),
                "发送",
                submit_msg,
                Btn::Primary,
            );

            let reply_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(S1)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_child(Expanded::new(1., input_el).finish())
                .with_child(send_btn)
                .finish();

            rows.push(reply_row);
        }

        let border_col = if is_primary {
            p.border_strong
        } else {
            p.border
        };

        ConstrainedBox::new(
            Container::new(self.vstack(S2, rows))
                .with_uniform_padding(S3)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(border_col))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
        )
        .with_width(360.)
        .finish()
    }
}
