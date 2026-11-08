#![allow(unused_imports)]
use super::*;
use crate::ui::{
    amoo_ui_state::AmooUiAction, backscreen_state::BackScreenAction, icons::UiIcon,
    led_grid::LedGrid, theme::*, widgets::*, work_state::WorkAction, Action, Companion, Page,
    PushAction, ToolPage, F_ALARM_TIME, F_AMOO, F_AMOO_GUI_APP, F_BACK_MARQUEE, F_CALENDAR_TIME,
    F_HOME_COMMAND, F_OV_TEXT, F_PHONE_SCHEDULE_TITLE, F_PUSH_REPLY, F_SMS_BODY, F_SMS_TO,
    F_WORK_PROMPT,
};
use chrono::{Datelike, Duration as ChronoDuration, Local, NaiveDate, Timelike};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor, ViewContext};
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use stepx_companion::{
    device::default_express_parcels,
    home_hub::{
        build_home_hub_report, build_selected_date_panorama, build_weekly_overview_report,
        quick_replies_for_message_with_context, resolve_top_app_display_name, DesktopDateEntry,
        HomeAiIntent, HomeAiRecommendation, HomeCardKind, HomeHubInput, HomeLifeDomain,
        SelectedDateEntryKind,
    },
    home_visuals::{
        day_dial, donut, parcel_progress, ring_gauge, DayDial, DayTimeline, DialAlarm, DialEvent,
        Paint, WeeklyBarDay,
    },
    push::{demo_push_snapshot, ImChannel, PushSnapshot},
};

pub(in crate::ui) fn channel_avatar_spec(
    package_name: &str,
    app_name: &str,
    p: &Palette,
) -> (UiIcon, String, ColorU, ColorU) {
    let channel = ImChannel::from_package(package_name);
    let style = crate::ui::icon_style::channel_source_style(channel, p);
    let label = match channel {
        ImChannel::WeChat
        | ImChannel::WeCom
        | ImChannel::DingTalk
        | ImChannel::Feishu
        | ImChannel::Sms => channel.label().to_string(),
        ImChannel::QqTim => "QQ".into(),
        _ if !app_name.trim().is_empty() => app_name.trim().to_string(),
        _ => "通知".into(),
    };
    (style.icon, label, style.background, style.foreground)
}

#[cfg(test)]
mod home_pending_message_tests {
    use super::*;

    #[test]
    fn unread_read_only_conversation_opens_desktop_detail_without_reply_or_gui_actions() {
        let mut app = Companion::new_for_test();
        app.reminder_governance = Default::default();
        let now = 1_794_000_000_000;
        let thread = stepx_companion::push::ImThread {
            thread_id: "wechat-project-review".into(), package_name: "com.tencent.mm".into(),
            app_name: "微信".into(), title: "项目评审群".into(), unread_count: 8,
            last_message: "请确认明天的评审时间".into(), last_timestamp_ms: now,
            can_remote_reply: false, ..Default::default()
        };
        let snapshot = PushSnapshot { threads: vec![thread], ..Default::default() };
        let visible = app.home_pending_im_threads(&snapshot, now);
        assert_eq!(visible.len(), 1);
        assert!(app.home_issue_reply_target(&snapshot, &visible).is_none());
        let actions = Companion::home_issue_thread_actions(visible[0]);
        assert_eq!(actions.len(), 2);
        assert!(matches!(actions[0], Action::Page(Page::Sms)));
        assert!(matches!(&actions[1], Action::Push(PushAction::SelectImThread(id)) if id == "wechat-project-review"));
    }

    #[test]
    fn pending_message_count_and_preview_share_expiration_mute_and_unread_filters() {
        let mut app = Companion::new_for_test();
        let now = 1_794_000_000_000;
        app.reminder_governance = Default::default();
        app.reminder_governance.category_ttl_mins.insert(
            stepx_companion::home_reminder_governance::ReminderCategory::UnreadMessage, 60);
        let threads = (0..4).map(|index| stepx_companion::push::ImThread {
            thread_id: format!("review-{index}"), package_name: "com.tencent.mm".into(),
            title: format!("项目评审{index}"), unread_count: if index == 2 { 0 } else { 8 },
            last_message: "请确认评审时间".into(),
            last_timestamp_ms: if index == 1 { now - 3_600_001 } else { now },
            can_remote_reply: true,
            ..Default::default()
        }).collect();
        let snapshot = PushSnapshot { threads, ..Default::default() };
        // Two pending conversations, not sixteen pending issues from their unread counts.
        assert_eq!(app.home_pending_im_threads(&snapshot, now).len(), 2);
        app.reminder_governance.mute_source("pkg:com.tencent.mm", "微信", now);
        assert!(app.home_pending_im_threads(&snapshot, now).is_empty());
        assert!(app.home_issue_reply_target(&snapshot, &[]).is_none());

        app.sms = (0..3).map(|index| stepx_companion::sms::Message {
            provider_id: None, address: "10086".into(), body: "业务提醒".into(),
            date_ms: if index == 1 { now - 3_600_001 } else { now },
            outgoing: false, kind: 1, read: index == 2, subscription: None,
        }).collect();
        assert_eq!(app.home_pending_sms(now).len(), 1);
        app.reminder_governance.mute_source("sms_addr:10086", "业务提醒", now);
        assert!(app.home_pending_sms(now).is_empty());
    }

    #[test]
    fn home_life_tickets_and_parcel_actions_fit_actual_narrow_columns() {
        struct LifeOnly(Companion);
        impl facloud_ui::Entity for LifeOnly { type Event = (); }
        impl facloud_ui::TypedActionView for LifeOnly { type Action = (); }
        impl facloud_ui::View for LifeOnly {
            fn ui_name() -> &'static str { "home_life_layout_test" }
            fn render(&self, _: &facloud_ui::AppContext) -> El {
                Stack::new().with_child(self.0.with_home_report(|report, snapshot, parcels|
                    self.0.render_home_life_and_express_column(snapshot, parcels, report))).finish()
            }
        }
        facloud_ui::App::test((), |mut app| async move {
            for (width, height) in [(340., 560.), (420., 650.), (600., 900.), (1000., 900.)] {
                for (case, state) in [(0, "待取件"), (1, "运输中"), (2, "本人已签收"), (3, "驿站已签收")] {
                    let mut companion = Companion::new_for_test();
                    companion.demo_mode = false;
                    companion.reminder_governance = Default::default();
                    companion.page = Page::Home;
                    companion.workspace.viewport = vec2f(width + 278., height);
                    companion.home_modules.enabled = vec![HomeModuleId::LifeModule];
                    companion.home_modules.set_module_size(HomeModuleId::LifeModule,
                        HomeModuleWidth::Full, if height < 740. { HomeModuleHeight::Compact } else { HomeModuleHeight::Standard });
                    companion.express_parcels = (0..if case == 0 { 2 } else { 1 }).map(|index|
                        stepx_companion::device::ExpressParcelInfo {
                            carrier: "京东快递".into(), tracking_no: format!("JD100000000000{index}"),
                            status: state.into(), pickup_code: if case == 1 { String::new() } else { format!("12-34-567{index}") },
                            station: "小区菜鸟驿站".into(), updated_at: "10:05".into(),
                        }).collect();
                    if case == 2 {
                        companion.push.snapshot.otp_banners.push(stepx_companion::push::OtpBanner {
                            code: "12-34-5670".into(), title: "待取件".into(), source_app: "菜鸟".into(),
                            raw_text: "快递请凭取件码12-34-5670领取".into(),
                            timestamp_ms: Local::now().timestamp_millis(), ..Default::default()
                        });
                    }
                    let (window_id, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                        |_| LifeOnly(companion));
                    let root = app.root_view_id(window_id).unwrap();
                    app.update(move |ctx| {
                        let mut presenter = facloud_ui::Presenter::new(window_id);
                        presenter.invalidate(facloud_ui::WindowInvalidation {
                            updated: std::collections::HashSet::from([root]), ..Default::default()
                        }, ctx);
                        presenter.build_scene(vec2f(width, height), 1., None, ctx);
                        let positions = presenter.position_cache();
                        let rect = |id: &str| positions.get_position(id)
                            .unwrap_or_else(|| panic!("missing life probe {id} at {width}×{height}, case {case}"));
                        if case == 2 {
                            assert!(positions.get_position("home-ticket-card-0").is_none(),
                                "completed parcel cannot revive its pickup code from an old notification");
                            assert!(positions.get_position("home-parcel-card-0").is_none());
                        } else if case == 1 {
                            let card = rect("home-parcel-card-0");
                            let actions = rect("home-parcel-actions-0");
                            assert!(actions.origin().x() >= card.origin().x()
                                && actions.max_x() <= card.max_x()
                                && actions.max_y() < card.max_y(), "parcel actions overflow: {actions:?} / {card:?}");
                        } else {
                            for index in 0..if case == 0 { 2 } else { 1 } {
                                let card = rect(&format!("home-ticket-card-{index}"));
                                let code = rect(&format!("home-ticket-code-{index}"));
                                let action = rect(&format!("home-ticket-action-{index}"));
                                assert!(card.origin().x() >= 0. && card.max_x() <= width,
                                    "ticket outside module: {card:?}, width {width}");
                                assert!(code.max_x() <= action.origin().x()
                                    && action.max_x() < card.max_x(),
                                    "pickup code and action overlap: {code:?}/{action:?}/{card:?}");
                            }
                            if case == 0 && width < 600. {
                                assert!(rect("home-ticket-card-1").origin().y() >= rect("home-ticket-card-0").max_y(),
                                    "narrow ticket columns must stack before truncating a pickup code");
                            }
                        }
                    });
                }
            }
        });
    }
}

impl Companion {
    // =========================================================================
    // ROW 3 LEFT: AVATAR-BASED IM & CALL DIRECT REPLY HUB
    // =========================================================================
    pub(in crate::ui) fn is_non_replyable_service_or_noise_thread(title: &str) -> bool {
        let t = title.trim();
        if t.is_empty()
            || t.starts_with('{')
            || stepx_companion::push::is_bogus_captured_view_text(t)
        {
            return true;
        }
        matches!(
            t,
            "微信支付"
                | "服务通知"
                | "公众号"
                | "订阅号"
                | "订阅号消息"
                | "服务号"
                | "腾讯新闻"
                | "微信运动"
                | "微信团队"
                | "顺丰速运"
                | "顺丰速运+"
                | "圆通速递"
                | "中通快递"
                | "韵达快递"
                | "极兔速递"
                | "京东快递"
                | "菜鸟"
                | "菜鸟裹裹"
                | "丰巢"
                | "搜索 账号/手机号"
                | "搜索小程序 搜索栏"
                | "最近"
                | "导入与导出"
                | "搜索"
                | "发现"
                | "通讯录"
                | "我"
        )
    }

    pub(in crate::ui) fn resolve_home_reply_target(
        &self,
        snapshot: &PushSnapshot,
    ) -> (String, String, String, String) {
        // Kept for read-only diagnostics. Rendering and submission use the
        // optional target so a missing route never invents a recipient.
        self.resolve_home_actionable_reply_target(snapshot).unwrap_or_default()
    }

    fn home_thread_supports_background_reply(t: &stepx_companion::push::ImThread) -> bool {
        (t.can_remote_reply || stepx_companion::push::is_native_sms_package_or_thread(&t.package_name, &t.thread_id))
                && !t.title.trim().is_empty()
                && !t.title.trim_start().starts_with('{')
                && !stepx_companion::push::is_excluded_im_package(&t.package_name)
                && !stepx_companion::push::is_scraped_ui_noise_thread(t)
                && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!(
                    "{} {} {}",
                    t.app_name, t.title, t.last_message
                ))
    }

    fn home_notification_supports_background_reply(n: &stepx_companion::push::PushNotification) -> bool {
        (n.can_reply || stepx_companion::push::is_native_sms_package_or_thread(&n.package_name, &n.thread_id))
            && !n.key.trim().is_empty() && !n.title.trim().is_empty()
            && !n.dismissed && n.removed_time_ms <= 0
            && !n.title.trim_start().starts_with('{')
            && !stepx_companion::push::is_excluded_im_package(&n.package_name)
            && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!("{} {}", n.title, n.text))
    }

    pub(in crate::ui) fn resolve_home_actionable_reply_target(
        &self,
        snapshot: &PushSnapshot,
    ) -> Option<(String, String, String, String)> {
        let is_valid_thread = Self::home_thread_supports_background_reply;
        if let Some(sel_thread) = &self.push.selected_im_thread {
            if let Some(number) = sel_thread.strip_prefix("sms:") {
                let clean_num = number.trim();
                if !clean_num.is_empty() {
                    return Some((
                        sel_thread.clone(),
                        sel_thread.clone(),
                        "com.android.mms".into(),
                        clean_num.to_string(),
                    ));
                }
            }
            if let Some(thread) = snapshot
                .threads
                .iter()
                .find(|t| &t.thread_id == sel_thread && is_valid_thread(t))
            {
                return Some((
                    if thread.notification_key.is_empty() {
                        thread.thread_id.clone()
                    } else {
                        thread.notification_key.clone()
                    },
                    thread.thread_id.clone(),
                    thread.package_name.clone(),
                    thread.title.clone(),
                ));
            }
            if let Some(notif) = snapshot.notifications.iter().find(|n|
                &n.thread_id == sel_thread && Self::home_notification_supports_background_reply(n)) {
                return Some((notif.key.clone(), notif.thread_id.clone(), notif.package_name.clone(), notif.title.clone()));
            }
            // A stale explicit selection must not redirect the user's draft
            // to another conversation merely because it has an unread message.
            return None;
        }

        if let Some(sel_key) = self
            .push
            .replying_key
            .as_ref()
            .or(self.push.selected_key.as_ref())
        {
            if let Some(number) = sel_key.strip_prefix("sms:") {
                let clean_num = number.trim();
                if !clean_num.is_empty() {
                    return Some((
                        sel_key.clone(),
                        sel_key.clone(),
                        "com.android.mms".into(),
                        clean_num.to_string(),
                    ));
                }
            }
            if let Some(thread) = snapshot.threads.iter().find(|t| {
                (&t.notification_key == sel_key || &t.thread_id == sel_key) && is_valid_thread(t)
            }) {
                return Some((
                    if thread.notification_key.is_empty() {
                        thread.thread_id.clone()
                    } else {
                        thread.notification_key.clone()
                    },
                    thread.thread_id.clone(),
                    thread.package_name.clone(),
                    thread.title.clone(),
                ));
            }
            if let Some(notif) = snapshot.notifications.iter().find(|n| {
                &n.key == sel_key && Self::home_notification_supports_background_reply(n)
            }) {
                return Some((
                    notif.key.clone(),
                    notif.thread_id.clone(),
                    notif.package_name.clone(),
                    notif.title.clone(),
                ));
            }
            return None;
        }

        let is_replyable_thread = |t: &stepx_companion::push::ImThread| -> bool {
            is_valid_thread(t)
        };

        if let Some(first_unread) = snapshot
            .threads
            .iter()
            .filter(|t| is_replyable_thread(t))
            .find(|t| t.unread_count > 0)
            .or_else(|| {
                snapshot
                    .threads
                    .iter()
                    .find(|t| is_valid_thread(t) && t.title.trim() == "文件传输助手")
            })
            .or_else(|| {
                self.demo_mode
                    .then(|| snapshot.threads.iter().find(|t| is_replyable_thread(t)))?
            })
        {
            return Some((
                if first_unread.notification_key.is_empty() {
                    first_unread.thread_id.clone()
                } else {
                    first_unread.notification_key.clone()
                },
                first_unread.thread_id.clone(),
                first_unread.package_name.clone(),
                first_unread.title.clone(),
            ));
        }
        snapshot.notifications.iter()
            .find(|n| !n.read && Self::home_notification_supports_background_reply(n))
            .map(|n| (n.key.clone(), n.thread_id.clone(), n.package_name.clone(), n.title.clone()))
    }

    pub(in crate::ui) fn render_channel_avatar(
        &self,
        ch_icon: UiIcon,
        bg: ColorU,
        fg: ColorU,
    ) -> El {
        ConstrainedBox::new(
            Container::new(Align::new(self.content_icon(ch_icon, 14.0, fg)).finish())
                .with_background_color(bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish(),
        )
        .with_width(28.)
        .with_height(28.)
        .finish()
    }

    // =========================================================================
    // SECTION 3: 当日待处理事项 · 三大结构化模块工作台 (1. 工作的 | 2. 生活的 | 3. 有问题的)
    // =========================================================================
    pub(in crate::ui) fn render_home_modular_action_center(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
        snapshot: &PushSnapshot,
        express_parcels: &[stepx_companion::device::ExpressParcelInfo],
    ) -> El {
        let p = self.p();
        let selected_date = self.home_selected_date();
        let today = Local::now().date_naive();
        let date_tag = if selected_date == today {
            format!("今日（{}）", selected_date.format("%m-%d"))
        } else {
            format!("所选日（{}）", selected_date.format("%m-%d"))
        };
        let genuine_ledger_count = self
            .context
            .overview
            .items
            .iter()
            .filter(|it| {
                !Self::is_synthetic_ledger_item(it)
                    && !Self::is_pure_calendar_ledger_echo(it)
                    && !it.status.contains("已不见")
                    && Self::is_unresolved_ledger_item(it)
            })
            .count();

        let section_banner = self.render_home_workbench_banner(
            "home-workbench-legacy",
            "今日事项 · 工作、生活与待确认".into(),
            date_tag,
            "按「工作安排 · 生活服务 · 待确认事项」三栏分流处理当日事务",
            vec![
                self.pill_badge_icon(
                    "1. 工作安排",
                    UiIcon::Briefcase,
                    p.blue,
                    soft_tint(p.blue, p.dark),
                ),
                self.pill_badge_icon("2. 生活服务", UiIcon::Package, p.warning, p.warning_soft),
                self.pill_badge_icon(
                    "3. 待确认事项",
                    UiIcon::Alert,
                    if report.unread_messages_count > 0 || genuine_ledger_count > 0 {
                        p.danger
                    } else {
                        p.teal
                    },
                    if report.unread_messages_count > 0 || genuine_ledger_count > 0 {
                        p.danger_soft
                    } else {
                        p.teal_soft
                    },
                ),
            ],
        );

        // True 3-Column Modular Grid on wide viewports (>= 980px), stacked vertically when narrower
        let modules_grid = self.responsive_columns(
            980.,
            S3,
            S3,
            CrossAxisAlignment::Start,
            vec![
                (1.08, self.render_home_calendar_and_work_column(report)),
                (
                    1.0,
                    self.render_home_life_and_express_column(snapshot, express_parcels, report),
                ),
                (
                    1.0,
                    self.render_home_issues_and_reply_column(snapshot, report),
                ),
            ],
        );

        home_geometry_probe(
            "home-workbench-legacy-card",
            self.vstack(S2, vec![section_banner, modules_grid]),
        )
    }

    pub(in crate::ui) fn is_unresolved_ledger_item(
        it: &stepx_companion::context_ledger::PreparedItem,
    ) -> bool {
        !it.status.contains("已不见")
            && (it.status.contains("待")
                || it.status.contains("需")
                || it.status.contains("问题")
                || it.status.contains("变更")
                || it.status.contains("冲突")
                || it.status.contains("核对")
                || it.status.contains("确认")
                || it.status.contains("未成功")
                || it.presentation.as_ref().is_some_and(|pr| pr.priority >= 75))
    }

    // =========================================================================
    // MODULE 3: 待确认与消息直回 (跨端待确认问题/冲突核对 · 未接来电 · 未读消息免解锁直回)
    // =========================================================================
    fn home_pending_im_threads<'a>(
        &self,
        snapshot: &'a PushSnapshot,
        now_ms: i64,
    ) -> Vec<&'a stepx_companion::push::ImThread> {
        let ttl_ms = self.reminder_governance.effective_ttl_ms(
            stepx_companion::home_reminder_governance::ReminderCategory::UnreadMessage,
        );
        let mut threads: Vec<_> = snapshot.threads.iter().filter(|thread| {
            // Reading a message does not require an Android RemoteInput action.
            // Reply capability is checked separately before displaying a composer.
            if thread.unread_count == 0
                || thread.thread_id.trim().is_empty()
                || Self::is_non_replyable_service_or_noise_thread(&thread.title)
                // Store notices belong to the updates feed, not the conversation queue.
                || matches!(thread.package_name.trim(), "com.stepos.appstore" | "com.stepx.appstore")
                || stepx_companion::push::is_excluded_im_package(&thread.package_name)
                || stepx_companion::push::is_scraped_ui_noise_thread(thread)
                || stepx_companion::home_hub::any_synthetic_e2e_probe(&[
                    &thread.app_name, &thread.title, &thread.last_message,
                ])
            {
                return false;
            }
            let keys = stepx_companion::home_reminder_governance::canonical_keys_for_im_thread(
                &thread.thread_id,
                (!thread.notification_key.is_empty()).then_some(thread.notification_key.as_str()),
            );
            !self.reminder_governance.is_any_key_suppressed(
                &keys,
                (thread.last_timestamp_ms > 0).then_some(thread.last_timestamp_ms),
                Some(&thread.last_message),
                Some(&format!("pkg:{}", thread.package_name)),
                now_ms,
            ) && (thread.last_timestamp_ms <= 0
                || now_ms <= 1_700_000_000_000
                || keys.iter().any(|key| self.reminder_governance.is_force_kept(key, now_ms))
                || now_ms.saturating_sub(thread.last_timestamp_ms) <= ttl_ms)
        }).collect();
        threads.sort_by(|left, right| right.last_timestamp_ms.cmp(&left.last_timestamp_ms)
            .then_with(|| left.thread_id.cmp(&right.thread_id)));
        threads
    }

    fn home_pending_sms<'a>(&'a self, now_ms: i64) -> Vec<&'a stepx_companion::sms::Message> {
        let ttl_ms = self.reminder_governance.effective_ttl_ms(
            stepx_companion::home_reminder_governance::ReminderCategory::UnreadMessage,
        );
        let mut messages: Vec<_> = self.sms.iter().filter(|message| {
            if message.kind != 1 || message.read
                || stepx_companion::home_hub::any_synthetic_e2e_probe(&[&message.address, &message.body])
            {
                return false;
            }
            let normalized = stepx_companion::home_reminder_governance::normalize_phone_or_addr(&message.address);
            let key = format!("sms_addr:{normalized}");
            !self.reminder_governance.is_any_key_suppressed(
                std::slice::from_ref(&key),
                (message.date_ms > 0).then_some(message.date_ms),
                Some(&message.body),
                Some(&key),
                now_ms,
            ) && (self.reminder_governance.is_force_kept(&key, now_ms)
                || now_ms.saturating_sub(message.date_ms.max(0)) <= ttl_ms.max(0))
        }).collect();
        messages.sort_by_key(|message| std::cmp::Reverse(message.date_ms));
        messages
    }

    fn home_issue_thread_actions(thread: &stepx_companion::push::ImThread) -> Vec<Action> {
        if Self::home_thread_supports_background_reply(thread) {
            vec![Action::HomeSelectReplyTarget {
                key: if thread.notification_key.is_empty() { thread.thread_id.clone() } else { thread.notification_key.clone() },
                thread_id: thread.thread_id.clone(),
                package_name: thread.package_name.clone(),
                title: thread.title.clone(),
            }]
        } else {
            vec![Action::Page(Page::Sms), Action::Push(PushAction::SelectImThread(thread.thread_id.clone()))]
        }
    }

    fn home_issue_reply_target(
        &self,
        snapshot: &PushSnapshot,
        pending_threads: &[&stepx_companion::push::ImThread],
    ) -> Option<(String, String, String, String)> {
        if self.push.selected_im_thread.is_some()
            || self.push.replying_key.is_some()
            || self.push.selected_key.is_some()
        {
            return self.resolve_home_actionable_reply_target(snapshot);
        }
        pending_threads.iter().find(|thread| Self::home_thread_supports_background_reply(thread))
            .map(|thread| (
                if thread.notification_key.is_empty() { thread.thread_id.clone() } else { thread.notification_key.clone() },
                thread.thread_id.clone(), thread.package_name.clone(), thread.title.clone(),
            ))
    }

    pub(in crate::ui) fn render_home_issues_and_reply_column(
        &self,
        snapshot: &PushSnapshot,
        report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        use stepx_companion::amoo_workspace::Phase;

        let p = self.p();
        let ov = &self.context.overview;
        let selected_date = self.home_selected_date();
        let today = Local::now().date_naive();
        let sel_ymd = (
            selected_date.year(),
            selected_date.month(),
            selected_date.day(),
        );

        let now_call_ms = Local::now().timestamp_millis();
        let task_ttl_ms = self.reminder_governance.effective_ttl_ms(
            stepx_companion::home_reminder_governance::ReminderCategory::LedgerAndTaskIssue,
        );
        let call_ttl_ms = self.reminder_governance.effective_ttl_ms(
            stepx_companion::home_reminder_governance::ReminderCategory::MissedCall,
        );
        let pending_threads = self.home_pending_im_threads(snapshot, now_call_ms);
        let pending_sms = self.home_pending_sms(now_call_ms);

        let failed_or_unconfirmed_tasks: Vec<_> = self
            .workspace
            .journal
            .tasks
            .iter()
            .filter(|t| self.workspace_visible(t))
            .filter(|t| !stepx_companion::home_hub::any_synthetic_e2e_probe(&[&t.title, &t.draft]))
            .filter(|t| {
                matches!(
                    t.steps.last().map(|s| s.phase),
                    Some(Phase::Failed | Phase::Unconfirmed)
                )
            })
            .filter(|t| {
                !self.reminder_governance.is_any_key_suppressed(
                    &[format!("task:{}", t.id)],
                    None,
                    None,
                    None,
                    now_call_ms,
                )
            })
            .collect();

        let default_year = self.back_screen.calendar_year;
        let mut genuine_ledger_items: Vec<_> = ov
            .items
            .iter()
            .filter_map(|it| {
                if !Self::is_unresolved_ledger_item(it) {
                    return None;
                }
                let ledger_key = format!("ledger:{}", it.id);
                let item_ts_ms = if it.updated_ms > 1_000_000_000_000 {
                    it.updated_ms
                } else if it.updated_ms > 0 {
                    it.updated_ms * 1000
                } else {
                    0
                };
                if self.reminder_governance.is_any_key_suppressed(
                    std::slice::from_ref(&ledger_key),
                    (item_ts_ms > 0).then_some(item_ts_ms),
                    None,
                    None,
                    now_call_ms,
                ) {
                    return None;
                }
                if item_ts_ms > 0
                    && now_call_ms > 1_700_000_000_000
                    && !self
                        .reminder_governance
                        .is_force_kept(&ledger_key, now_call_ms)
                    && now_call_ms.saturating_sub(item_ts_ms) > task_ttl_ms
                {
                    return None;
                }
                // A large saved prompt is irrelevant once the item has expired
                // or been dismissed. Do the cheap eligibility checks first.
                if Self::is_pure_calendar_ledger_echo(it) || Self::is_synthetic_ledger_item(it) {
                    return None;
                }
                let ts_secs = if it.updated_ms > 1_000_000_000_000 {
                    it.updated_ms / 1000
                } else {
                    it.updated_ms
                };
                let matches_updated =
                    stepx_companion::home_hub::timestamp_to_local_ymd_parts(ts_secs)
                        .map(|(ymd, _, _)| ymd == sel_ymd)
                        .unwrap_or(selected_date == today);
                let matches_sel = matches_updated || {
                    let search_txt = Self::ledger_item_search_text(it);
                    Self::extract_month_day_mentions(&search_txt, default_year)
                        .contains(&selected_date)
                };
                let prio = it.presentation.as_ref().map(|pr| pr.priority).unwrap_or(50);
                let sort_key = (
                    std::cmp::Reverse(true),
                    std::cmp::Reverse(matches_sel),
                    std::cmp::Reverse(prio),
                    std::cmp::Reverse(it.updated_ms),
                );
                Some((sort_key, it))
            })
            .collect();
        let active_conflicts: Vec<_> = report
            .schedule_conflicts
            .iter()
            .filter(|c| {
                !self.reminder_governance.is_any_key_suppressed(
                    &[format!("conflict:{}", c.headline)],
                    None,
                    None,
                    None,
                    now_call_ms,
                )
            })
            .collect();
        let missed_calls_count = self
            .call_logs
            .iter()
            .filter(|c| {
                if c.call_type != 3 {
                    return false;
                }
                let norm =
                    stepx_companion::home_reminder_governance::normalize_phone_or_addr(&c.number);
                let call_key = format!("call:{norm}");
                let src_key = format!("sms_addr:{norm}");
                if self.reminder_governance.is_any_key_suppressed(
                    std::slice::from_ref(&call_key),
                    (c.date_ms > 0).then_some(c.date_ms),
                    None,
                    Some(&src_key),
                    now_call_ms,
                ) {
                    return false;
                }
                self.reminder_governance
                    .is_force_kept(&call_key, now_call_ms)
                    || c.date_ms <= 0
                    || now_call_ms.saturating_sub(c.date_ms) <= call_ttl_ms
            })
            .count();
        let issue_total = active_conflicts.len()
            + genuine_ledger_items.len()
            + failed_or_unconfirmed_tasks.len()
            + missed_calls_count
            + pending_threads.len()
            + pending_sms.len();

        let header = row_between_expanded(
            self.hstack(
                6.,
                vec![
                    self.home_section_heading(UiIcon::Clipboard, "待处理"),
                    self.pill_badge(
                        format!("{issue_total}"),
                        if issue_total > 0 {
                            p.warning_text
                        } else {
                            p.text2
                        },
                        if issue_total > 0 {
                            p.warning_soft
                        } else {
                            p.card2
                        },
                    ),
                ],
            ),
            if issue_total > 0 || self.home_pending_expanded {
                self.chip(
                    "home-issues-open-ledger",
                    if self.home_pending_expanded { "收起" } else { "展开" },
                    Some(if self.home_pending_expanded { UiIcon::ChevronUp } else { UiIcon::ChevronDown }),
                    vec![Action::HomeTogglePending],
                    false,
                    false,
                )
            } else {
                Empty::new().finish()
            },
            4.,
        );

        let mut issue_cards: Vec<El> = Vec::new();

        // 0. P0 High-Priority Cross-Signal Conflict Card (Schedule × Alarm)
        for (c_idx, conflict) in active_conflicts.into_iter().enumerate() {
            let clean_badge = conflict
                .conflict_badge
                .trim_start_matches("⚠️ ")
                .trim_start_matches("⚠ ")
                .trim();
            let clean_headline = conflict
                .headline
                .trim_start_matches("⚠️ ")
                .trim_start_matches("⚠ ")
                .trim();
            let clean_detail = format!(
                "建议提前 60 分钟设好 {:02}:{:02} 闹钟并同步背屏提醒",
                conflict.suggested_alarm_hour, conflict.suggested_alarm_minute
            );
            let clean_primary_label = conflict
                .primary_action_label
                .trim_start_matches("⏰ ")
                .replace("手机闹钟", "闹钟");
            let conflict_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("conflict:{}", conflict.headline),
                    stepx_companion::home_reminder_governance::ReminderCategory::ScheduleConflict,
                    clean_headline.to_string(),
                )
                .with_subtitle(clean_detail.clone()),
                ..Default::default()
            };
            let top_row = row_between_expanded(
                self.hstack(
                    4.,
                    vec![
                        self.ui_icon_box(UiIcon::Alert, 22., p.danger_soft, p.danger, None),
                        self.pill_badge(
                            Self::truncate_chars(clean_badge, 6),
                            p.danger,
                            p.danger_soft,
                        ),
                    ],
                ),
                self.mono(&conflict.event_time_hm, 11., p.danger),
                4.,
            );
            let headline_el = self.line_w(
                Self::truncate_chars(clean_headline, 26),
                CAPTION,
                p.text,
                Weight::Semibold,
            );
            let actions_row = row_between_expanded(
                self.button_icon(
                    format!("home-conflict-primary-{c_idx}"),
                    clean_primary_label,
                    UiIcon::Clock,
                    Action::HomeExecuteIntent(conflict.primary_intent.clone()),
                    Btn::Primary,
                ),
                self.icon_action(
                    format!("home-conflict-expire-{c_idx}"),
                    "不再提醒",
                    UiIcon::Check,
                    vec![Action::HomeReminder(HomeReminderAction::ManualExpire(
                        conflict_target.clone(),
                    ))],
                    false,
                ),
                6.,
            );
            let conflict_card =
                Container::new(self.vstack(4., vec![top_row, headline_el, actions_row]))
                    .with_horizontal_padding(S2 + 2.)
                    .with_vertical_padding(6.)
                    .with_background_color(lerp_color(p.surface, p.danger_soft, 0.25))
                    .with_border(Border::all(1.5).with_border_color(p.danger))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish();
            issue_cards.push(self.wrap_home_reminder_target(
                format!("home-rem-conflict-{c_idx}"),
                conflict_target,
                conflict_card,
            ));
        }

        // 1. Failed / Unconfirmed workspace tasks needing user decision
        for task in failed_or_unconfirmed_tasks.iter().take(if self.home_pending_expanded { usize::MAX } else { 1 }) {
            let phase = task.steps.last().map(|s| s.phase);
            let is_failed = matches!(phase, Some(Phase::Failed));
            let badge_txt = if is_failed { "失败" } else { "待确认" };
            let tint = if is_failed { p.danger } else { p.warning };
            let note = task
                .steps
                .last()
                .map(|s| s.note.trim())
                .filter(|s| !s.is_empty())
                .unwrap_or("点击回到原任务查看上下文与草稿");
            let task_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("task:{}", task.id),
                    stepx_companion::home_reminder_governance::ReminderCategory::LedgerAndTaskIssue,
                    task.title.clone(),
                )
                .with_subtitle(note.to_string()),
                ..Default::default()
            };
            let task_card = Container::new(row_between_expanded(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(S2)
                    .with_child(self.ui_icon_box(
                        UiIcon::Alert,
                        24.,
                        soft_tint(tint, p.dark),
                        tint,
                        None,
                    ))
                    .with_child(
                        Expanded::new(
                            1.,
                            self.vstack(
                                2.,
                                vec![
                                    self.hstack(
                                        6.,
                                        vec![
                                            self.pill_badge(
                                                badge_txt,
                                                p.icon_foreground(tint),
                                                soft_tint(tint, p.dark),
                                            ),
                                            self.line_w(
                                                Self::truncate_chars(&task.title, 15),
                                                CAPTION,
                                                p.text,
                                                Weight::Semibold,
                                            ),
                                        ],
                                    ),
                                    self.line(Self::truncate_chars(note, 40), MICRO, p.text2),
                                ],
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
                self.button_icon(
                    format!("home-issue-resume-{}", task.id),
                    "处理",
                    UiIcon::ArrowUpRight,
                    Action::HomeResumeTask(task.id.clone()),
                    Btn::Secondary,
                ),
                6.,
            ))
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(5.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();
            issue_cards.push(self.wrap_home_reminder_target(
                format!("home-rem-issue-task-{}", task.id),
                task_target,
                task_card,
            ));
        }

        // 2. Genuine Context Ledger items (prioritizing unresolved issues and items matching selected_date)
        genuine_ledger_items.sort_by_key(|(sort_key, _)| *sort_key);

        let mod_w = self.home_module_content_width(HomeModuleId::IssuesAndReplyModule);
        let raw_h = self
            .home_modules
            .module_height(HomeModuleId::IssuesAndReplyModule);
        let mod_h = self.resolve_effective_height(HomeModuleId::IssuesAndReplyModule, raw_h);
        let (title_budget, summary_budget) = match mod_w {
            HomeModuleWidth::OneThird => (16, 26),
            HomeModuleWidth::OneHalf => (26, 40),
            HomeModuleWidth::TwoThirds => (36, 56),
            HomeModuleWidth::Full => (52, 78),
        };
        let phrase_take = 2;
        let short_phrase = |s: &str| s.chars().count() <= 10;
        let max_ledger_cards = if self.home_pending_expanded {
            12
        } else {
            match mod_h {
                HomeModuleHeight::Compact => 1,
                HomeModuleHeight::Standard | HomeModuleHeight::Auto => 2,
                HomeModuleHeight::Tall => 4,
            }
        };

        for (idx, (_, item)) in genuine_ledger_items
            .into_iter()
            .take(max_ledger_cards)
            .enumerate()
        {
            let prio = item
                .presentation
                .as_ref()
                .map(|pr| pr.priority)
                .unwrap_or(50);
            let (tint, cat_label) = if item.status.contains("未成功") || item.status.contains("失败") {
                (p.danger, "失败")
            } else if prio >= 90 || item.status.contains("变更") {
                (p.warning, "待核对")
            } else if prio >= 75 || item.status.contains("待") || item.status.contains("问题") {
                (p.warning, "待处理")
            } else {
                (p.accent, "重点")
            };
            let display_title = Self::ledger_item_display_title(item);
            let raw_summary = Self::ledger_item_display_detail(item);
            let base_summary = if raw_summary.starts_with("已根据「") && raw_summary.contains('」')
            {
                raw_summary
                    .splitn(2, '」')
                    .nth(1)
                    .map(|s| format!("已{}", s.trim()))
                    .unwrap_or(raw_summary)
            } else {
                raw_summary
            };
            let summary_txt = base_summary.replace(
                "背屏点阵图案预览，可在「背屏」页预览或直接应用",
                "背屏点阵预览，可在背屏页应用",
            );
            let ledger_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("ledger:{}", item.id),
                    stepx_companion::home_reminder_governance::ReminderCategory::LedgerAndTaskIssue,
                    display_title.clone(),
                )
                .with_subtitle(summary_txt.clone()),
                ..Default::default()
            };
            let (_, primary_act) = self.context_item_primary(item);
            let (act_label, act_action) = match primary_act {
                Action::HomeResumeTask(target) => ("继续对话", Action::HomeResumeTask(target)),
                _ => (
                    "跟进",
                    Action::Context(crate::ui::context_state::ContextAction::Open(
                        item.path.clone(),
                    )),
                ),
            };
            let top_row = row_between_expanded(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(self.ui_icon_box(
                        UiIcon::Layers,
                        22.,
                        soft_tint(tint, p.dark),
                        tint,
                        None,
                    ))
                    .with_child(self.pill_badge(cat_label, p.icon_foreground(tint), soft_tint(tint, p.dark)))
                    .with_child(
                        Expanded::new(
                            1.,
                            self.line_w(
                                Self::truncate_chars(&display_title, title_budget),
                                CAPTION,
                                p.text,
                                Weight::Semibold,
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
                self.hstack(
                    2.,
                    vec![
                        self.chip(
                            format!("home-issue-act-{idx}"),
                            act_label,
                            Some(UiIcon::ArrowUpRight),
                            vec![act_action],
                            false,
                            false,
                        ),
                        self.icon_action(
                            format!("home-ledger-expire-{idx}"),
                            "不再提醒",
                            UiIcon::Check,
                            vec![Action::HomeReminder(HomeReminderAction::ManualExpire(
                                ledger_target.clone(),
                            ))],
                            false,
                        ),
                    ],
                ),
                4.,
            );
            let bottom_row = self.line(
                Self::truncate_chars(&summary_txt, summary_budget),
                MICRO,
                p.text2,
            );
            let card = Container::new(self.vstack(2., vec![top_row, bottom_row]))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(5.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
            issue_cards.push(self.wrap_home_reminder_target(
                format!("home-rem-ledger-{idx}"),
                ledger_target,
                card,
            ));
        }

        let now_local = Local::now();
        let active_meeting_ctx = self
            .back_screen
            .stepos_suite
            .calendar_events
            .iter()
            .find(|ev| {
                ev.date_ymd() == (today.year(), today.month(), today.day())
                    && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!(
                        "{} {} {}",
                        ev.title, ev.location, ev.description
                    ))
                    && stepx_companion::home_hub::event_is_ongoing(
                        ev,
                        now_local.hour(),
                        now_local.minute(),
                    )
            })
            .map(stepx_companion::home_hub::format_active_meeting_context)
            .or_else(|| {
                report
                    .priority_cards
                    .iter()
                    .find(|c| {
                        c.kind == HomeCardKind::UpcomingMeeting
                            && matches!(c.badge.as_str(), "正在进行" | "即将开始")
                    })
                    .map(|c| {
                        self.back_screen
                            .stepos_suite
                            .calendar_events
                            .iter()
                            .find(|ev| ev.title.trim() == c.title.trim())
                            .map(stepx_companion::home_hub::format_active_meeting_context)
                            .unwrap_or_else(|| c.title.clone())
                    })
            });
        let active_meeting_title = active_meeting_ctx.as_deref();
        let pomo_mins = self.back_screen.pomodoro_active.then(|| {
            (self
                .back_screen
                .pomodoro_remaining_secs()
                .saturating_add(59)
                / 60)
                .max(1)
        });
        let dnd_on = self.back_screen.stepos_suite.controls.dnd_on;

        // 3. Missed Calls (call_type == 3, within TTL)
        for (idx, call) in self
            .call_logs
            .iter()
            .filter(|c| {
                if c.call_type != 3 {
                    return false;
                }
                let norm =
                    stepx_companion::home_reminder_governance::normalize_phone_or_addr(&c.number);
                let call_key = format!("call:{norm}");
                let src_key = format!("sms_addr:{norm}");
                if self.reminder_governance.is_any_key_suppressed(
                    std::slice::from_ref(&call_key),
                    (c.date_ms > 0).then_some(c.date_ms),
                    None,
                    Some(&src_key),
                    now_call_ms,
                ) {
                    return false;
                }
                self.reminder_governance
                    .is_force_kept(&call_key, now_call_ms)
                    || c.date_ms <= 0
                    || now_call_ms.saturating_sub(c.date_ms) <= call_ttl_ms
            })
            .take(1)
            .enumerate()
        {
            let caller = if call.name.trim().is_empty() {
                call.number.trim().to_string()
            } else {
                format!("{} ({})", call.name.trim(), call.number.trim())
            };
            let num = call.number.trim().to_string();
            let norm_num = stepx_companion::home_reminder_governance::normalize_phone_or_addr(&num);
            let call_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("call:{norm_num}"),
                    stepx_companion::home_reminder_governance::ReminderCategory::MissedCall,
                    format!("未接来电 · {caller}"),
                )
                .with_subtitle(num.clone())
                .with_source_ts(call.date_ms),
                sms_address: num.clone(),
                mute_source_key: format!("sms_addr:{norm_num}"),
                mute_source_label: caller.clone(),
                ..Default::default()
            };
            let call_replies = stepx_companion::home_hub::quick_replies_for_message_with_context(
                "电话",
                &caller,
                "未接来电",
                active_meeting_title,
                pomo_mins,
                dnd_on,
            );
            let sms_reply_body = call_replies
                .first()
                .cloned()
                .unwrap_or_else(|| "正在开会，稍后给您回电".to_string());
            let top_row = row_between_expanded(
                self.hstack(
                    4.,
                    vec![
                        self.ui_icon_box(UiIcon::Phone, 22., p.warning_soft, p.warning_text, None),
                        self.pill_badge("未接", p.warning_text, p.warning_soft),
                        self.line_w(
                            Self::truncate_chars(&caller, title_budget),
                            CAPTION,
                            p.text,
                            Weight::Semibold,
                        ),
                    ],
                ),
                self.button_icon(
                    format!("home-call-{idx}-dial"),
                    "回拨",
                    UiIcon::Phone,
                    Action::PhoneCall(num.clone()),
                    Btn::Secondary,
                ),
                4.,
            );
            let bottom_row = row_between_expanded(
                self.mono(crate::ui::pages::format_time(call.date_ms), 10., p.text3),
                self.hstack(
                    2.,
                    vec![
                        self.chip(
                            format!("home-call-{idx}-sms-meeting"),
                            "短信",
                            None,
                            vec![Action::HomeQuickSmsReply {
                                number: num,
                                body: sms_reply_body,
                            }],
                            false,
                            false,
                        ),
                        self.icon_action(
                            format!("home-call-expire-{idx}"),
                            "不再提醒",
                            UiIcon::Check,
                            vec![Action::HomeReminder(HomeReminderAction::ManualExpire(
                                call_target.clone(),
                            ))],
                            false,
                        ),
                    ],
                ),
                4.,
            );
            let call_card = Container::new(self.vstack(2., vec![top_row, bottom_row]))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(5.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
            issue_cards.push(self.wrap_home_reminder_target(
                format!("home-rem-call-{idx}"),
                call_target,
                call_card,
            ));
        }

        // 4. IM & SMS Direct Reply Section inside Module 3
        let reply_target = self.home_issue_reply_target(snapshot, &pending_threads);
        let has_reply_target = reply_target.is_some();
        let (target_key, target_thread_id, target_pkg, target_title) = reply_target.unwrap_or_default();
        let (target_icon, ch_label, ch_bg, ch_fg) = channel_avatar_spec(&target_pkg, "", &p);

        let mut msg_rows: Vec<El> = Vec::new();
        let mut selected_last_msg = String::new();
        let mut selected_unread = 0u32;
        let mut selected_im_target: Option<HomeReminderContextTarget> = None;

        for thread in snapshot.threads.iter().filter(|t| {
            has_reply_target
                && (t.thread_id == target_thread_id || t.notification_key == target_key)
                && !t.title.trim_start().starts_with('{')
                && !stepx_companion::push::is_excluded_im_package(&t.package_name)
                && !stepx_companion::home_hub::any_synthetic_e2e_probe(&[
                    &t.app_name, &t.title, &t.last_message,
                ])
        }) {
            let t_key = if thread.notification_key.is_empty() {
                thread.thread_id.clone()
            } else {
                thread.notification_key.clone()
            };
            if t_key == target_key || thread.thread_id == target_thread_id {
                selected_last_msg = thread.last_message.clone();
                selected_unread = thread.unread_count as u32;
                break;
            }
        }

        let max_thread_rows = match mod_h {
            _ if self.home_pending_expanded => 12,
            HomeModuleHeight::Compact => usize::from(issue_cards.is_empty()),
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => 1,
            HomeModuleHeight::Tall => 4,
        };

        for (idx, thread) in pending_threads
            .iter()
            .copied()
            .take(max_thread_rows + 1)
            .enumerate()
        {
            let (av_icon, badge_txt, badge_bg, badge_fg) =
                channel_avatar_spec(&thread.package_name, &thread.app_name, &p);
            let t_key = if thread.notification_key.is_empty() {
                thread.thread_id.clone()
            } else {
                thread.notification_key.clone()
            };
            let is_selected = t_key == target_key || thread.thread_id == target_thread_id;

            let sender = thread.last_sender.trim();
            let raw_msg = thread.last_message.trim();
            let preview = if !sender.is_empty()
                && sender != thread.title.trim()
                && !raw_msg.starts_with(&format!("{sender}:"))
                && !raw_msg.starts_with(&format!("{sender}："))
            {
                format!("{sender}: {raw_msg}")
            } else {
                raw_msg.to_string()
            };
            let im_keys = stepx_companion::home_reminder_governance::canonical_keys_for_im_thread(
                &thread.thread_id,
                (!thread.notification_key.is_empty()).then_some(thread.notification_key.as_str()),
            );
            let im_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("im:{}", thread.thread_id),
                    stepx_companion::home_reminder_governance::ReminderCategory::UnreadMessage,
                    format!("{} · {}", badge_txt, thread.title),
                )
                .with_aliases(im_keys)
                .with_subtitle(preview.clone())
                .with_source_ts(thread.last_timestamp_ms),
                im_thread_id: thread.thread_id.clone(),
                notif_key: thread.notification_key.clone(),
                mute_source_key: format!("pkg:{}", thread.package_name),
                mute_source_label: badge_txt.clone(),
                ..Default::default()
            };
            if is_selected {
                selected_im_target = Some(im_target);
                continue;
            }
            // The extra candidate only exists to allow for the selected
            // conversation's composer; never render it just to discard it.
            if msg_rows.len() >= max_thread_rows { continue; }

            let mut badges: Vec<El> = Vec::new();
            if thread.unread_count > 0 {
                badges.push(self.pill_badge(
                    format!("{} 未读", thread.unread_count),
                    p.accent,
                    p.accent_soft,
                ));
            }

            let top_line = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.)
                .with_child(
                    Expanded::new(
                        1.,
                        self.line_w(
                            Self::truncate_chars(&thread.title, title_budget.max(18)),
                            CAPTION,
                            p.text,
                            Weight::Semibold,
                        ),
                    )
                    .finish(),
                )
                .with_child(self.hstack(4., badges))
                .with_child(self.mono(
                    crate::ui::pages::format_time(thread.last_timestamp_ms),
                    10.,
                    p.text3,
                ))
                .finish();

            let bottom_line = self.line(
                Self::truncate_chars(&preview, summary_budget.max(32)),
                MICRO,
                p.text2,
            );
            let center_children = vec![top_line, bottom_line];

            let avatar = self.render_channel_avatar(av_icon, badge_bg, badge_fg);
            let center_col = self.vstack(3., center_children);

            let select_actions = Self::home_issue_thread_actions(thread);
            let state = self.mouse_state(format!("home-im-row-{idx}"));
            let row_el = Hoverable::new(state, move |mouse| {
                let t = self.hover_t(&mouse);
                let bg = if is_selected {
                    p.selected
                } else {
                    lerp_color(p.surface, p.hover, t)
                };
                let border = if is_selected {
                    p.accent
                } else {
                    lerp_color(p.border, p.border_strong, t)
                };
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                        .with_spacing(S2)
                        .with_child(avatar)
                        .with_child(Expanded::new(1., center_col).finish())
                        .finish(),
                )
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(5.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                for action in &select_actions { ctx.dispatch_typed_action(action.clone()); }
            })
            .finish();
            msg_rows.push(self.wrap_home_reminder_target(
                format!("home-rem-im-{idx}"),
                im_target,
                row_el,
            ));
        }

        if (msg_rows.is_empty() && issue_cards.len() < 3) || self.home_pending_expanded {
            for (idx, sms_msg) in pending_sms
                .iter()
                .copied()
                .take(if self.home_pending_expanded { 12 } else { 1 })
                .enumerate()
            {
                let number = sms_msg.address.trim().to_string();
                let norm_num =
                    stepx_companion::home_reminder_governance::normalize_phone_or_addr(&number);
                let sms_target = HomeReminderContextTarget {
                    spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                        format!("sms_addr:{norm_num}"),
                        stepx_companion::home_reminder_governance::ReminderCategory::UnreadMessage,
                        format!("未读短信 · {number}"),
                    )
                    .with_subtitle(sms_msg.body.trim().to_string())
                    .with_source_ts(sms_msg.date_ms),
                    sms_address: number.clone(),
                    mute_source_key: format!("sms_addr:{norm_num}"),
                    mute_source_label: number.clone(),
                    ..Default::default()
                };
                let phrases = quick_replies_for_message_with_context(
                    "短信",
                    &number,
                    &sms_msg.body,
                    active_meeting_title,
                    pomo_mins,
                    dnd_on,
                );
                let first_phrase = phrases
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| "收到，谢谢".into());

                let avatar =
                    self.render_channel_avatar(UiIcon::Sms, rgb(0x10, 0xB9, 0x81), ColorU::white());
                let top_line = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(
                        Expanded::new(1., self.line_w(&number, CAPTION, p.text, Weight::Semibold))
                            .finish(),
                    )
                    .with_child(self.pill_badge("短信", p.warning, p.warning_soft))
                    .with_child(self.mono(
                        crate::ui::pages::format_time(sms_msg.date_ms),
                        10.,
                        p.text3,
                    ))
                    .finish();

                let bottom_line = Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.line(
                                Self::truncate_chars(sms_msg.body.trim(), summary_budget.max(30)),
                                MICRO,
                                p.text2,
                            ),
                        )
                        .finish(),
                    )
                    .with_child(self.chip(
                        format!("home-sms-{idx}-quick-0"),
                        "回复",
                        Some(UiIcon::Send),
                        vec![Action::HomeQuickSmsReply {
                            number: number.clone(),
                            body: first_phrase,
                        }],
                        false,
                        false,
                    ))
                    .finish();

                let sms_key = format!("sms:{number}");
                let is_sms_selected = target_key == sms_key || target_thread_id == sms_key;
                let select_sms_act = Action::HomeSelectReplyTarget {
                    key: sms_key.clone(),
                    thread_id: sms_key,
                    package_name: "com.android.mms".into(),
                    title: number.clone(),
                };
                let sms_state = self.mouse_state(format!("home-sms-row-{idx}"));
                let sms_row_el = Hoverable::new(sms_state, move |mouse| {
                    let t = self.hover_t(&mouse);
                    let bg = if is_sms_selected {
                        p.selected
                    } else {
                        lerp_color(p.surface, p.hover, t)
                    };
                    let border = if is_sms_selected {
                        p.accent
                    } else {
                        lerp_color(p.border, p.border_strong, t)
                    };
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_spacing(S2)
                            .with_child(avatar)
                            .with_child(
                                Expanded::new(1., self.vstack(2., vec![top_line, bottom_line]))
                                    .finish(),
                            )
                            .finish(),
                    )
                    .with_horizontal_padding(S2 + 2.)
                    .with_vertical_padding(5.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(select_sms_act.clone()))
                .finish();
                msg_rows.push(self.wrap_home_reminder_target(
                    format!("home-rem-sms-{idx}"),
                    sms_target,
                    sms_row_el,
                ));
            }
        }

        // Active Conversation Composer Dock at the bottom of Module 3
        let composer_box = has_reply_target.then(|| {
            let mut active_phrases = if mod_h == HomeModuleHeight::Tall {
                quick_replies_for_message_with_context(
                &ch_label,
                &target_title,
                &selected_last_msg,
                active_meeting_title,
                pomo_mins,
                dnd_on,
                )
            } else { Vec::new() };
            for std_phrase in if mod_h == HomeModuleHeight::Tall { &[
                "收到，马上处理",
                "好的，稍后回复",
                "已确认，按计划推进",
                "稍后给你回电",
            ][..] } else { &[][..] } {
                if !active_phrases.iter().any(|s| s == *std_phrase) {
                    active_phrases.push(std_phrase.to_string());
                }
            }
            let active_phrase_chips: Vec<El> = active_phrases
                .into_iter()
                .filter(|s| short_phrase(s))
                .take(phrase_take)
                .enumerate()
                .map(|(p_idx, phrase)| {
                    let short_label = phrase
                        .replace("正在忙，稍后回复你", "稍后回复你")
                        .replace("好的，谢谢！", "好的，谢谢");
                    self.chip(
                        format!("home-composer-quick-{p_idx}"),
                        short_label,
                        None,
                        vec![Action::HomeQuickImReply {
                            key: target_key.clone(),
                            thread_id: target_thread_id.clone(),
                            package_name: target_pkg.clone(),
                            title: target_title.clone(),
                            text: phrase,
                        }],
                        false,
                        false,
                    )
                })
                .collect();
            let open_phone_chip = self.icon_action(
                "home-inline-reply-open-phone",
                "手机打开",
                UiIcon::ExternalLink,
                vec![Action::Push(PushAction::OpenOnPhone(target_key.clone()))],
                false,
            );
            let ai_draft_chip = self.icon_action(
                "home-inline-reply-aidraft",
                "草拟回复",
                UiIcon::File,
                vec![Action::Push(PushAction::AiDraftReply {
                    thread_id: target_thread_id.clone(),
                    package_name: target_pkg.clone(),
                    title: target_title.clone(),
                })],
                false,
            );

            let composer_input_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.)
                .with_child(Expanded::new(1., self.input(F_PUSH_REPLY, "回复…", None)).finish())
                .with_child(self.button_icon(
                    "home-inline-reply-send",
                    "发送",
                    UiIcon::Send,
                    Action::Push(PushAction::ReplyInline {
                        key: target_key.clone(),
                        thread_id: target_thread_id.clone(),
                        package_name: target_pkg.clone(),
                        title: target_title.clone(),
                    }),
                    Btn::Primary,
                ))
                .finish();

            let composer_header = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.)
                .with_child(self.render_channel_avatar(target_icon, ch_bg, ch_fg))
                .with_child(
                    Expanded::new(
                        1.,
                        self.vstack(
                            1.,
                            vec![
                                {
                                    let mut title_row = vec![self.line_w(
                                        Self::truncate_chars(&target_title, title_budget.max(18)),
                                        CAPTION,
                                        p.text,
                                        Weight::Semibold,
                                    )];
                                    if selected_unread > 0 {
                                        title_row.push(self.pill_badge(
                                            format!("{selected_unread} 未读"),
                                            p.danger,
                                            p.danger_soft,
                                        ));
                                    }
                                    self.hstack(4., title_row)
                                },
                                self.line(
                                    if selected_last_msg.trim().is_empty() {
                                        ch_label.to_string()
                                    } else {
                                        Self::truncate_chars(
                                            selected_last_msg.trim(),
                                            summary_budget.max(30),
                                        )
                                    },
                                    10.,
                                    p.text2,
                                ),
                            ],
                        ),
                    )
                    .finish(),
                )
                .with_child(ai_draft_chip)
                .with_child(open_phone_chip)
                .finish();

            let composer_children = if mod_h != HomeModuleHeight::Tall || active_phrase_chips.is_empty() {
                vec![composer_header, composer_input_row]
            } else {
                vec![
                    composer_header,
                    composer_input_row,
                    self.wrap(active_phrase_chips),
                ]
            };

            let composer_box = Container::new(self.vstack(4., composer_children))
                .with_uniform_padding(S2)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.accent))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
            match selected_im_target {
                Some(target) => {
                    self.wrap_home_reminder_target("home-rem-im-selected", target, composer_box)
                }
                None => composer_box,
            }
        });
        msg_rows.truncate(if self.home_pending_expanded { 24 } else { max_thread_rows });
        // The complete message stream remains one click away even when
        // summaries are omitted to keep the default workbench compact.
        let all_messages = Align::new(self.link("home-issues-all-messages", "查看全部消息",
            Action::Page(Page::Notifications))).top_left().finish();

        if matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full)
            && mod_h != HomeModuleHeight::Compact
            && !issue_cards.is_empty()
            && (!msg_rows.is_empty() || composer_box.is_some())
        {
            // Split only when both sides contain useful information. A wide
            // pending-work card must not reserve half its body for no messages.
            let left_col = self.vstack(5., issue_cards);
            let mut right_parts = Vec::new();
            if !msg_rows.is_empty() {
                right_parts.push(self.vstack(5., msg_rows));
            }
            if let Some(composer) = composer_box { right_parts.push(composer); }
            let right_col = self.vstack(5., right_parts);
            let split_body = self.responsive_columns(
                740.,
                S2,
                S2,
                CrossAxisAlignment::Start,
                vec![(1.0, left_col), (1.08, right_col)],
            );
            return self.home_frame_with_footer(HomeModuleId::IssuesAndReplyModule, vec![header, split_body], Some(all_messages));
        }

        if mod_h == HomeModuleHeight::Compact && mod_w == HomeModuleWidth::OneThird {
            let mut compact_children = vec![header];
            if !issue_cards.is_empty() {
                compact_children.push(self.vstack(5., issue_cards));
            } else if let Some(first_msg) = msg_rows.into_iter().next() {
                compact_children.push(first_msg);
            }
            if let Some(composer) = composer_box { compact_children.push(composer); }
            return self.home_frame_with_footer(HomeModuleId::IssuesAndReplyModule, compact_children, Some(all_messages));
        }

        let mut col_children = vec![header];
        if !issue_cards.is_empty() {
            col_children.push(self.vstack(5., issue_cards));
        }
        if !msg_rows.is_empty() {
            col_children.push(self.vstack(5., msg_rows));
        }
        if let Some(composer) = composer_box { col_children.push(composer); }

        self.home_frame_with_footer(HomeModuleId::IssuesAndReplyModule, col_children, Some(all_messages))
    }

    // =========================================================================
    // MODULE 2: 生活服务 · 快递与出行 (快递与取件 · 天气通勤 · 预约与生活安排 + 凭证与物流轨迹)
    // =========================================================================
    pub(in crate::ui) fn render_home_life_context_cards_section(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
        express_parcels: &[stepx_companion::device::ExpressParcelInfo],
        shown_codes: &[String],
    ) -> El {
        let p = self.p();
        let now_life_ms = Local::now().timestamp_millis();
        let mod_w = self.home_module_content_width(HomeModuleId::LifeModule);
        let raw_h = self.home_modules.module_height(HomeModuleId::LifeModule);
        let mod_h = self.resolve_effective_height(HomeModuleId::LifeModule, raw_h);
        let (title_budget, detail_budget) = match mod_w {
            HomeModuleWidth::OneThird => (15, 25),
            HomeModuleWidth::OneHalf => (24, 38),
            HomeModuleWidth::TwoThirds => (32, 52),
            HomeModuleWidth::Full => (46, 68),
        };
        let max_ctx_cards = match (mod_w, mod_h) {
            (_, HomeModuleHeight::Compact) => 1,
            (HomeModuleWidth::OneThird, HomeModuleHeight::Standard | HomeModuleHeight::Auto) => 1,
            (_, HomeModuleHeight::Standard | HomeModuleHeight::Auto) => 3,
            (_, HomeModuleHeight::Tall) => 4,
        };
        let mut cards: Vec<El> = Vec::with_capacity(max_ctx_cards);

        for (idx, card) in report
            .life_context_cards
            .iter()
            .filter(|card| {
                let mut keys = vec![
                    format!("life:{:?}:{}", card.domain, card.badge),
                    format!("life:{}", card.title),
                ];
                if let Some(code) = &card.code_or_number {
                    keys.extend(
                        stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
                            Some(code),
                            None,
                            None,
                            None,
                        ),
                    );
                }
                // Parcels / pickup codes already have their own cards above: don't repeat them.
                if card.domain == HomeLifeDomain::ExpressAndPickup
                    && (!express_parcels.is_empty()
                        || card.code_or_number.as_ref().is_some_and(|c| {
                            shown_codes.iter().any(|s| s.eq_ignore_ascii_case(c.trim()))
                        }))
                {
                    return false;
                }
                !self
                    .reminder_governance
                    .is_any_key_suppressed(&keys, None, None, None, now_life_ms)
            })
            .take(max_ctx_cards)
            .enumerate()
        {
            let (icon, tint, domain_tag) = match card.domain {
                HomeLifeDomain::ExpressAndPickup => (UiIcon::Package, p.warning, "快递"),
                HomeLifeDomain::WeatherAndCommute => (UiIcon::Sun, p.blue, "天气"),
                HomeLifeDomain::LeisureAndWellbeing => (UiIcon::Gift, p.pink, "生活"),
            };

            // P3 Zero-Noise Adaptive Collapse: when there are 0 active parcels and no pickup code on the
            // ExpressAndPickup card, collapse it to a compact single-line quiet status bar so Weather × Schedule
            // and Leisure cards get primary visual space.
            let is_idle_express = card.domain == HomeLifeDomain::ExpressAndPickup
                && express_parcels.is_empty()
                && card.code_or_number.is_none();
            if is_idle_express {
                cards.push(
                    Container::new(row_between_expanded(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_spacing(6.)
                            .with_child(self.pill_badge_icon(
                                "暂无快递",
                                UiIcon::Check,
                                p.success_text,
                                p.success_soft,
                            ))
                            .finish(),
                        self.chip(
                            format!("home-life-ctx-primary-{idx}"),
                            Self::truncate_chars(&card.action_label, 8),
                            None,
                            vec![Action::HomeExecuteIntent(card.intent.clone())],
                            false,
                            false,
                        ),
                        4.,
                    ))
                    .with_horizontal_padding(S2 + 2.)
                    .with_vertical_padding(4.)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                );
                continue;
            }

            let is_cross_linked_weather = card.domain == HomeLifeDomain::WeatherAndCommute
                && (card.badge.contains("×") || card.context_link.contains("跨源关联"));

            let display_action = if card.action_label.contains("推送雨天") {
                "推背屏清单".to_string()
            } else if card.action_label.contains("准备赴约") {
                "推背屏提醒".to_string()
            } else {
                Self::truncate_chars(&card.action_label, 7)
            };

            let primary_btn = self.button(
                format!("home-life-ctx-primary-{idx}"),
                display_action,
                Action::HomeExecuteIntent(card.intent.clone()),
                if idx == 0 || is_cross_linked_weather {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            );

            let badge_el = if is_cross_linked_weather {
                self.pill_badge_icon("天气×日程", UiIcon::Sun, tint, soft_tint(tint, p.dark))
            } else {
                self.pill_badge(domain_tag.to_string(), tint, soft_tint(tint, p.dark))
            };

            let clean_title = card
                .title
                .trim_start_matches("今日预约/生活安排 · ")
                .trim();

            let clean_detail = card
                .detail
                .trim_start_matches("时间：")
                .replace(" · 地点：已记录在日程", " · 双端日历已同步");

            let life_cat = match card.domain {
                HomeLifeDomain::ExpressAndPickup => {
                    stepx_companion::home_reminder_governance::ReminderCategory::ExpressPickup
                }
                _ => stepx_companion::home_reminder_governance::ReminderCategory::WeatherAndLife,
            };
            let mut life_aliases = vec![format!("life:{}", card.title)];
            if let Some(code) = &card.code_or_number {
                life_aliases.extend(
                    stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
                        Some(code),
                        None,
                        None,
                        None,
                    ),
                );
            }
            let life_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("life:{:?}:{}", card.domain, card.badge),
                    life_cat,
                    clean_title.to_string(),
                )
                .with_aliases(life_aliases)
                .with_subtitle(clean_detail.clone()),
                otp_or_pickup_code: card.code_or_number.clone().unwrap_or_default(),
                ..Default::default()
            };

            let top_row = row_between_expanded(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(4.)
                    .with_child(self.ui_icon_box(
                        icon,
                        24.,
                        soft_tint(tint, p.dark),
                        tint,
                        None,
                    ))
                    .with_child(badge_el)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.line_w(
                                Self::truncate_chars(clean_title, title_budget),
                                CAPTION,
                                p.text,
                                Weight::Semibold,
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
                {
                    let mut right_actions = Vec::new();
                    if card.domain == HomeLifeDomain::WeatherAndCommute {
                        right_actions.push(self.chip(
                            format!("home-life-weather-bring-{idx}"),
                            "带入天气",
                            Some(UiIcon::Sun),
                            vec![Action::WeatherBringToAmoo(Some(card.title.clone()))],
                            false,
                            false,
                        ));
                        right_actions.push(self.chip(
                            format!("home-life-weather-tool-{idx}"),
                            "独立天气",
                            None,
                            vec![Action::OpenTool(Some(ToolPage::Weather))],
                            false,
                            false,
                        ));
                    }
                    right_actions.push(primary_btn);
                    right_actions.push(self.icon_action(
                        format!("home-life-card-expire-{idx}"),
                        "不再提醒",
                        UiIcon::Check,
                        vec![Action::HomeReminder(HomeReminderAction::ManualExpire(
                            life_target.clone(),
                        ))],
                        false,
                    ));
                    self.hstack(2., right_actions)
                },
                4.,
            );

            let detail_row = self.line(
                Self::truncate_chars(&clean_detail, detail_budget + 4),
                MICRO,
                p.text2,
            );

            let card_el = Container::new(self.vstack(
                3.,
                vec![top_row, detail_row],
            ))
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(6.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(lerp_color(p.border, tint, 0.38)))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();

            cards.push(self.wrap_home_reminder_target(
                format!("home-rem-life-card-{idx}"),
                life_target,
                card_el,
            ));
        }

        self.vstack(6., cards)
    }

    pub(in crate::ui) fn render_home_life_and_express_column(
        &self,
        snapshot: &PushSnapshot,
        express_parcels: &[stepx_companion::device::ExpressParcelInfo],
        report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        let p = self.p();
        let now_ticket_ms = Local::now().timestamp_millis();
        let otp_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(stepx_companion::home_reminder_governance::ReminderCategory::OtpCode);
        let pickup_ttl_ms = self.reminder_governance.effective_ttl_ms(
            stepx_companion::home_reminder_governance::ReminderCategory::ExpressPickup,
        );
        // A completed parcel can still retain its old pickup code in an older
        // notification. Do not resurrect that code from a secondary source.
        let completed_codes: Vec<_> = express_parcels.iter()
            .filter(|parcel| stepx_companion::home_hub::is_completed_express_text(&parcel.status))
            .map(|parcel| parcel.pickup_code.trim())
            .filter(|code| !code.is_empty())
            .collect();
        let code_completed = |code: &str| completed_codes.iter()
            .any(|done| done.eq_ignore_ascii_case(code.trim()));
        let visible_parcels: Vec<_> = express_parcels
            .iter()
            .filter(|parcel| {
                if stepx_companion::home_hub::is_completed_express_text(&parcel.status) {
                    return false;
                }
                let keys =
                    stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
                        (!parcel.pickup_code.trim().is_empty())
                            .then_some(parcel.pickup_code.trim()),
                        (!parcel.tracking_no.trim().is_empty())
                            .then_some(parcel.tracking_no.trim()),
                        None,
                        None,
                    );
                !self.reminder_governance.is_any_key_suppressed(
                    &keys,
                    None,
                    None,
                    None,
                    now_ticket_ms,
                )
            })
            .cloned()
            .collect();
        let mod_w = self.home_module_content_width(HomeModuleId::LifeModule);
        let raw_h = self.home_modules.module_height(HomeModuleId::LifeModule);
        let mod_h = self.resolve_effective_height(HomeModuleId::LifeModule, raw_h);
        let (_, detail_budget) = match mod_w {
            HomeModuleWidth::OneThird => (15, 25),
            HomeModuleWidth::OneHalf => (24, 38),
            HomeModuleWidth::TwoThirds => (32, 52),
            HomeModuleWidth::Full => (46, 68),
        };

        let header = row_between_expanded(
            self.home_section_heading(UiIcon::Package, "快递与出行"),
            self.button(
                "home-otp-open-clip-tool",
                "剪贴板",
                Action::OpenTool(Some(ToolPage::Clipboard)),
                Btn::Ghost,
            ),
            4.,
        );

        // 1. Code tickets use their actual allocated column width below.
        let max_tickets = match mod_h {
            HomeModuleHeight::Tall => 4,
            HomeModuleHeight::Compact | HomeModuleHeight::Standard | HomeModuleHeight::Auto => 2,
        };
        let mut ticket_specs: Vec<(String, String, String, bool)> = Vec::new();
        for otp in snapshot.otp_banners.iter() {
            if stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!(
                "{} {} {}",
                otp.code, otp.title, otp.raw_text
            )) || stepx_companion::home_hub::is_completed_express_text(&otp.raw_text)
            {
                continue;
            }
            let code = otp.code.trim().to_string();
            let code_keys =
                stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
                    Some(&code),
                    None,
                    None,
                    None,
                );
            if code.is_empty() || code_completed(&code)
                || self
                    .push
                    .prefs
                    .dismissed_pickup_codes
                    .contains(&code.to_uppercase())
                || self.reminder_governance.is_any_key_suppressed(
                    &code_keys,
                    (otp.timestamp_ms > 0).then_some(otp.timestamp_ms),
                    Some(&code),
                    None,
                    now_ticket_ms,
                )
                || ticket_specs.iter().any(|t| t.0 == code)
            {
                continue;
            }
            let is_pickup = otp.title.contains("取件")
                || otp.raw_text.contains("取件")
                || otp.source_app.contains("驿站");
            let max_age_ms = if is_pickup { pickup_ttl_ms } else { otp_ttl_ms };
            let force_kept = code_keys
                .iter()
                .any(|k| self.reminder_governance.is_force_kept(k, now_ticket_ms));
            if !force_kept
                && otp.timestamp_ms > 0
                && now_ticket_ms > 1_700_000_000_000
                && now_ticket_ms.saturating_sub(otp.timestamp_ms) > max_age_ms
            {
                continue;
            }
            ticket_specs.push((code, otp.source_app.clone(), otp.title.clone(), is_pickup));
            if ticket_specs.len() >= max_tickets {
                break;
            }
        }
        if ticket_specs.len() < max_tickets {
            for card in report
                .priority_cards
                .iter()
                .filter(|c| c.kind == HomeCardKind::OtpAndPickup)
            {
                if let Some(code) = &card.code_or_number {
                    let code_trim = code.trim();
                    let code_keys =
                        stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
                            Some(code_trim),
                            None,
                            None,
                            None,
                        );
                    if !code_trim.is_empty() && !code_completed(code_trim)
                        && !self
                            .push
                            .prefs
                            .dismissed_pickup_codes
                            .contains(&code_trim.to_uppercase())
                        && !self.reminder_governance.is_any_key_suppressed(
                            &code_keys,
                            None,
                            Some(code_trim),
                            None,
                            now_ticket_ms,
                        )
                        && !ticket_specs.iter().any(|t| t.0 == code_trim)
                    {
                        let is_pickup = card.badge.contains("取件") || card.title.contains("取件");
                        ticket_specs.push((
                            code_trim.to_string(),
                            card.badge.clone(),
                            card.subtitle.clone(),
                            is_pickup,
                        ));
                        if ticket_specs.len() >= max_tickets {
                            break;
                        }
                    }
                }
            }
        }
        if ticket_specs.len() < max_tickets {
            for parcel in &visible_parcels {
                let code = parcel.pickup_code.trim().to_string();
                if !code.is_empty()
                    && !self
                        .push
                        .prefs
                        .dismissed_pickup_codes
                        .contains(&code.to_uppercase())
                    && !ticket_specs.iter().any(|t| t.0 == code)
                {
                    let short_carrier = parcel
                        .carrier
                        .split('·')
                        .next()
                        .unwrap_or(&parcel.carrier)
                        .trim()
                        .to_string();
                    ticket_specs.push((code, short_carrier, parcel.station.clone(), true));
                    if ticket_specs.len() >= max_tickets {
                        break;
                    }
                }
            }
        }

        ticket_specs.truncate(max_tickets);
        let shown_codes: Vec<String> = ticket_specs.iter().map(|t| t.0.clone()).collect();
        let covered_codes: Vec<String> = shown_codes.iter().cloned()
            .chain(completed_codes.iter().map(|code| code.to_string())).collect();
        let life_context_section =
            self.render_home_life_context_cards_section(report, &visible_parcels, &covered_codes);

        let mut ticket_cards: Vec<El> = Vec::new();
        for (idx, (code, badge, sub, is_pickup)) in ticket_specs.into_iter().enumerate() {
            let tint = if is_pickup { p.warning } else { p.blue };
            let icon = if is_pickup {
                UiIcon::Package
            } else {
                UiIcon::Key
            };
            let short_badge: String = badge
                .split('·')
                .next()
                .unwrap_or(&badge)
                .trim()
                .chars()
                .take(6)
                .collect();
            let code_font_size = if code.chars().count() > 8 { 15. } else { 18. };
            let ticket_keys =
                stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
                    Some(&code),
                    None,
                    None,
                    None,
                );
            let ticket_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("code:{}", code.trim().to_uppercase()),
                    if is_pickup {
                        stepx_companion::home_reminder_governance::ReminderCategory::ExpressPickup
                    } else {
                        stepx_companion::home_reminder_governance::ReminderCategory::OtpCode
                    },
                    format!("{} · {code}", if is_pickup { "取件码" } else { "验证码" }),
                )
                .with_aliases(ticket_keys)
                .with_subtitle(sub.clone()),
                otp_or_pickup_code: code.clone(),
                ..Default::default()
            };
            let fill_chip = self.chip(
                format!("home-ticket-fill-{idx}"),
                "填到手机",
                Some(UiIcon::Keyboard),
                vec![Action::BackScreen(BackScreenAction::FillOtpOnPhone(code.clone()))],
                false,
                false,
            );
            let copy_chip = self.chip(
                format!("home-ticket-copy-{idx}"),
                "复制",
                Some(UiIcon::Copy),
                vec![Action::CopyText(code.clone(), format!("已复制：{code}"))],
                false,
                false,
            );
            let code_text = home_geometry_probe(format!("home-ticket-code-{idx}"),
                self.mono(&code, code_font_size, p.text));
            let middle_row = if is_pickup {
                let done_chip = self.chip(
                    format!("home-ticket-done-{idx}"),
                    "标为已取",
                    Some(UiIcon::Check),
                    vec![
                        Action::Push(PushAction::DismissPickupCode(code.clone())),
                        Action::HomeReminder(HomeReminderAction::ManualExpire(
                            ticket_target.clone(),
                        )),
                    ],
                    false,
                    false,
                );
                #[cfg(feature = "ui-capture")]
                let done_chip = crate::ui::scenario_check::control(
                    format!("home-dismiss-pickup-{code}"),
                    done_chip,
                );
                row_between_expanded(code_text,
                    home_geometry_probe(format!("home-ticket-action-{idx}"), done_chip), 4.)
            } else {
                row_between_expanded(code_text,
                    home_geometry_probe(format!("home-ticket-action-{idx}"), fill_chip), 4.)
            };

            let top_row = row_between_expanded(
                self.hstack(
                    4.,
                    vec![
                        self.ui_icon_box(icon, 22., soft_tint(tint, p.dark), tint, None),
                        self.pill_badge(short_badge, tint, soft_tint(tint, p.dark)),
                    ],
                ),
                copy_chip,
                4.,
            );

            let bottom_row = self.line(
                Self::truncate_chars(&sub, detail_budget.max(16)),
                10.,
                p.text2,
            );

            let ticket_card =
                Container::new(self.vstack(3., vec![top_row, middle_row, bottom_row]))
                    .with_horizontal_padding(S2)
                    .with_vertical_padding(6.)
                    .with_background_color(p.surface)
                    .with_border(
                        Border::all(1.).with_border_color(lerp_color(p.border, tint, 0.45)),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish();

            ticket_cards.push(home_geometry_probe(format!("home-ticket-card-{idx}"), self.wrap_home_reminder_target(
                format!("home-rem-life-code-{idx}"),
                ticket_target,
                ticket_card,
            )));
        }

        // 2. Visual Express & Takeout Progress Tracker (with `stepper`)
        let express_search_actions = self.hstack(6., vec![self.button_icon(
                "home-express-query-btn",
                "查快递",
                UiIcon::Search,
                Action::AmooQueryExpress,
                Btn::Secondary,
            ), self.button(
                "home-express-pull-clip-btn",
                "读剪贴板",
                Action::BackScreen(BackScreenAction::PullPhoneClipboard),
                Btn::Ghost,
            )]);
        let express_search_row = self.responsive_columns(560., 6., 6.,
            CrossAxisAlignment::Center, vec![
                (1.2, self.input(F_OV_TEXT, "输入单号查快递…", None)),
                (1., express_search_actions),
            ]);

        let max_parcels = match (mod_w, mod_h) {
            (_, HomeModuleHeight::Compact) => 1,
            (HomeModuleWidth::OneThird, HomeModuleHeight::Standard | HomeModuleHeight::Auto) => 1,
            (_, HomeModuleHeight::Standard | HomeModuleHeight::Auto) => 2,
            (_, HomeModuleHeight::Tall) => 4,
        };
        let mut tracker_cards: Vec<El> = Vec::new();
        for (idx, parcel) in visible_parcels
            .iter()
            .filter(|parcel| {
                let code = parcel.pickup_code.trim();
                code.is_empty() || !shown_codes.iter().any(|s| s.eq_ignore_ascii_case(code))
            })
            .take(max_parcels)
            .enumerate()
        {
            let pickup = parcel.pickup_code.trim().to_string();
            let tracking = parcel.tracking_no.trim().to_string();
            let (done, total) = parcel_progress(&parcel.status);
            let short_carrier = parcel
                .carrier
                .split('·')
                .next()
                .unwrap_or(&parcel.carrier)
                .trim();
            let short_status = parcel
                .status
                .split('·')
                .next()
                .unwrap_or(&parcel.status)
                .trim();
            let third = if done >= 2 && !pickup.is_empty() {
                format!("待取 {pickup}")
            } else {
                "到站/派送".to_string()
            };
            let stage_labels = ["已揽收".to_string(), "运输中".into(), third, "签收".into()];
            let stage_labels: Vec<&str> = stage_labels.iter().map(String::as_str).collect();
            let stepper = self.native_flow_stepper(
                &format!("home-parcel-{idx}"),
                &stage_labels,
                done.min(total),
                false,
                p.accent,
            );

            let icon_box = self.ui_icon_box(
                UiIcon::Package,
                26.,
                soft_tint(p.success_text, p.dark),
                p.success_text,
                None,
            );

            let dismiss_key = if !pickup.is_empty() {
                pickup.clone()
            } else {
                tracking.clone()
            };
            let parcel_keys =
                stepx_companion::home_reminder_governance::canonical_keys_for_code_or_parcel(
                    (!pickup.is_empty()).then_some(pickup.as_str()),
                    (!tracking.is_empty()).then_some(tracking.as_str()),
                    None,
                    None,
                );
            let parcel_primary = parcel_keys
                .first()
                .cloned()
                .unwrap_or_else(|| format!("parcel:{}", dismiss_key.to_uppercase()));
            let track_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    parcel_primary,
                    stepx_companion::home_reminder_governance::ReminderCategory::ExpressPickup,
                    format!("{short_carrier} · {dismiss_key}"),
                )
                .with_aliases(parcel_keys)
                .with_subtitle(parcel.station.clone()),
                otp_or_pickup_code: dismiss_key.clone(),
                ..Default::default()
            };
            let top_bar = self.wrap(vec![
                    self.line_w(Self::truncate_chars(short_carrier, 12), CAPTION, p.text, Weight::Semibold),
                    self.pill_badge(
                        Self::truncate_chars(short_status, 7),
                        p.text2,
                        p.card2,
                    ),
            ]);
            let actions = home_geometry_probe(format!("home-parcel-actions-{idx}"), self.hstack(
                    4.,
                    vec![
                        self.chip(
                            format!("home-parcel-done-{idx}"),
                            if pickup.is_empty() { "不再显示" } else { "标为已取" },
                            Some(UiIcon::Check),
                            vec![
                                Action::Push(PushAction::DismissPickupCode(dismiss_key)),
                                Action::HomeReminder(HomeReminderAction::ManualExpire(
                                    track_target.clone(),
                                )),
                            ],
                            false,
                            false,
                        ),
                        self.chip(
                            format!("home-parcel-open-app-{idx}"),
                            "轨迹",
                            None,
                            vec![Action::QuickGuiScenario {
                                app: "菜鸟".into(),
                                goal: format!(
                                    "打开菜鸟或顺丰查看运单 {tracking} 的最新物流轨迹与取件码"
                                ),
                            }],
                            false,
                            true,
                        ),
                    ],
                ));

            let mut info = vec![top_bar,
                    self.mono(Self::truncate_chars(&tracking, 24), 10., p.text3),
                    self.line(
                        Self::truncate_chars(&parcel.station, detail_budget),
                        10.,
                        p.text3,
                    )];
            if mod_h != HomeModuleHeight::Compact
                && matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full) {
                info.push(stepper);
            }
            info.push(actions);
            let info_col = self.vstack(4., info);

            let tracker_el = Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(S2)
                    .with_child(icon_box)
                    .with_child(Expanded::new(1., info_col).finish())
                    .finish(),
            )
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(6.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();

            tracker_cards.push(home_geometry_probe(format!("home-parcel-card-{idx}"), self.wrap_home_reminder_target(
                format!("home-rem-life-track-{idx}"),
                track_target,
                tracker_el,
            )));
        }

        // 3. Visual Life Service Launcher Grid (`ui_icon_tile_btn`: 2x2 at OneThird, 4x1 at wider widths)
        let life_tile_items = vec![
            self.ui_icon_tile_btn(
                "home-life-alipay-coupon",
                UiIcon::Gift,
                "支付宝领券",
                Some("领外卖券"),
                true,
                Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                    "open_alipay_coupons".into(),
                )),
            ),
            self.ui_icon_tile_btn(
                "home-life-meituan-order",
                UiIcon::Clock,
                "美团外卖",
                Some("骑手位置"),
                false,
                Action::QuickGuiScenario {
                    app: "美团".into(),
                    goal: "打开美团外卖查看当前订单实时骑手位置与预计送达时间".into(),
                },
            ),
            self.ui_icon_tile_btn(
                "home-life-taobao-flash",
                UiIcon::Package,
                "淘宝闪购",
                Some("闪购进度"),
                false,
                Action::QuickGuiScenario {
                    app: "淘宝".into(),
                    goal: "打开淘宝闪购查看即时配送订单状态".into(),
                },
            ),
            self.ui_icon_tile_btn(
                "home-life-amap-taxi",
                UiIcon::ArrowUpRight,
                "高德打车",
                Some("路况打车"),
                false,
                Action::QuickGuiScenario {
                    app: "高德地图".into(),
                    goal: "查看当前回家/去公司的实时路况与打车预估".into(),
                },
            ),
        ];
        let tile_cols = if mod_w == HomeModuleWidth::OneThird {
            2
        } else {
            4
        };
        let life_tiles_row = self.grid_cols(life_tile_items, tile_cols, 6.);

        let tickets_grid = if !ticket_cards.is_empty() {
            let mut rows = Vec::new();
            let mut tickets = ticket_cards.into_iter();
            while let Some(first) = tickets.next() {
                let mut cells = vec![(1., first)];
                if let Some(second) = tickets.next() { cells.push((1., second)); }
                rows.push(self.responsive_columns(560., 6., 6., CrossAxisAlignment::Start, cells));
            }
            Some(self.vstack(6., rows))
        } else {
            None
        };

        if mod_h == HomeModuleHeight::Compact {
            let mut compact_children = vec![header];
            if let Some(tg) = tickets_grid {
                compact_children.push(tg);
            } else if !tracker_cards.is_empty() {
                compact_children.push(self.vstack(6., tracker_cards));
            }
            compact_children.push(life_context_section);
            return self.home_frame(HomeModuleId::LifeModule, compact_children);
        }

        if matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full) {
            let mut left_parts = Vec::new();
            if let Some(tg) = tickets_grid {
                left_parts.push(tg);
            }
            if !tracker_cards.is_empty() {
                left_parts.push(self.vstack(6., tracker_cards));
            }
            left_parts.push(express_search_row);
            let left_col = self.vstack(6., left_parts);
            let right_col = self.vstack(6., vec![life_context_section, life_tiles_row]);
            let split_body = self.responsive_columns(
                740.,
                S2,
                S2,
                CrossAxisAlignment::Start,
                vec![(1.0, left_col), (1.05, right_col)],
            );
            return self.home_frame(HomeModuleId::LifeModule, vec![header, split_body]);
        }

        let mut col_children = vec![header];
        if let Some(tg) = tickets_grid {
            col_children.push(tg);
        }
        if !tracker_cards.is_empty() {
            col_children.push(self.vstack(6., tracker_cards));
        }
        col_children.push(life_context_section);
        if mod_h == HomeModuleHeight::Tall || mod_w != HomeModuleWidth::OneThird {
            col_children.push(express_search_row);
            col_children.push(life_tiles_row);
        }

        self.home_frame(HomeModuleId::LifeModule, col_children)
    }

    // =========================================================================
    // MODULE 14: 天气 · 每日建议与预报
    // =========================================================================
    pub(in crate::ui) fn render_home_weather_module_card(
        &self,
        _report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        let p = self.p();
        let w = &self.weather_snapshot;
        if !self.demo_mode && !w.from_live_api {
            return self.home_frame(HomeModuleId::WeatherModule, vec![
                self.between(
                    self.home_section_heading(UiIcon::Sun, "天气"),
                    self.icon_action("home-weather-refresh", "刷新天气", UiIcon::Refresh,
                        vec![Action::WeatherRefresh], false),
                ),
                self.caption("天气待更新，其他安排可继续查看。"),
            ]);
        }
        let mod_w = self.home_module_content_width(HomeModuleId::WeatherModule);
        let raw_h = self.home_modules.module_height(HomeModuleId::WeatherModule);
        let mod_h = self.resolve_effective_height(HomeModuleId::WeatherModule, raw_h);

        let header = row_between_expanded(
            self.hstack(
                S2,
                vec![
                    self.home_section_heading(UiIcon::Sun, "天气"),
                    self.line(&w.city, CAPTION, p.text2),
                ],
            ),
            self.hstack(
                6.,
                vec![
                    self.button(
                        "home-weather-open-tool",
                        "详情",
                        Action::OpenTool(Some(ToolPage::Weather)),
                        Btn::Ghost,
                    ),
                    self.icon_action(
                        "home-weather-refresh",
                        "刷新",
                        UiIcon::Refresh,
                        vec![Action::WeatherRefresh],
                        false,
                    ),
                ],
            ),
            S2,
        );

        let cond_tint = if w.umbrella_needed { p.warning } else { p.blue };
        let hero_box = Container::new(self.vstack(
            6.,
            vec![
                row_between_expanded(
                    self.hstack(
                        S2,
                        vec![
                            self.text_w(format!("{}°C", w.temp_c), 26., p.text, Weight::Bold),
                            self.pill_badge(&w.condition, cond_tint, soft_tint(cond_tint, p.dark)),
                            self.line(w.range_label(), CAPTION, p.text2),
                        ],
                    ),
                    self.line(
                        format!(
                            "湿度 {}% · {} · 空气{}",
                            w.humidity_pct, w.wind_label, w.aqi_label
                        ),
                        MICRO,
                        p.text3,
                    ),
                    S2,
                ),
                self.line(&w.commute_advice, CAPTION, p.text2),
            ],
        ))
        .with_horizontal_padding(S3)
        .with_vertical_padding(8.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();

        let tmr_rainy = w.tomorrow_umbrella_needed();
        let tmr_tint = if tmr_rainy { p.warning } else { p.teal };
        let (tomorrow_advice, advice_source) = w.tomorrow_advice_with_source();
        let tomorrow_box = Container::new(self.vstack(
            5.,
            vec![
                row_between_expanded(
                    self.hstack(
                        6.,
                        vec![
                            self.pill_badge("明日建议", tmr_tint, soft_tint(tmr_tint, p.dark)),
                            self.line_w(w.tomorrow_headline(), CAPTION, p.text, Weight::Semibold),
                            self.pill_badge(
                                w.tomorrow_temp_delta_label(),
                                p.blue,
                                soft_tint(p.blue, p.dark),
                            ),
                        ],
                    ),
                    self.line(advice_source, MICRO, p.text3),
                    S2,
                ),
                self.text(tomorrow_advice, CAPTION, p.text),
            ],
        ))
        .with_horizontal_padding(S2 + 2.)
        .with_vertical_padding(7.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(lerp_color(p.border, tmr_tint, 0.35)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();

        let left_col = self.vstack(6., vec![hero_box, tomorrow_box]);

        let hourly_take = match mod_h {
            HomeModuleHeight::Compact => 4,
            HomeModuleHeight::Standard | HomeModuleHeight::Auto | HomeModuleHeight::Tall => 6,
        };
        let hourly_chips: Vec<El> = w
            .hourly
            .iter()
            .take(hourly_take)
            .map(|pt| {
                self.pill_badge(
                    format!("{} {}° {}", pt.hour_label, pt.temp_c, pt.condition),
                    p.text,
                    p.card,
                )
            })
            .collect();

        let hourly_box = Container::new(self.vstack(
            5.,
            vec![
                self.line_w("逐时预报", CAPTION, p.text2, Weight::Medium),
                self.wrap(hourly_chips),
            ],
        ))
        .with_horizontal_padding(S2 + 2.)
        .with_vertical_padding(6.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();

        let mut daily_rows: Vec<El> = Vec::with_capacity(4);
        daily_rows.push(self.line_w("近几日", CAPTION, p.text2, Weight::Medium));
        for day in w.daily.iter().take(3) {
            let row_el = row_between_expanded(
                self.hstack(
                    6.,
                    vec![
                        self.line_w(&day.date_label, CAPTION, p.text, Weight::Semibold),
                        self.pill_badge(
                            &day.condition,
                            if day.pop_pct >= 40 { p.warning } else { p.blue },
                            soft_tint(if day.pop_pct >= 40 { p.warning } else { p.blue }, p.dark),
                        ),
                        self.mono(format!("{}°~{}°C", day.low_c, day.high_c), 11., p.text),
                    ],
                ),
                self.line(Self::truncate_chars(&day.advice, 22), MICRO, p.text2),
                4.,
            );
            daily_rows.push(row_el);
        }

        let daily_box = Container::new(self.vstack(4., daily_rows))
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(6.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();

        let right_col = if mod_h == HomeModuleHeight::Compact && mod_w == HomeModuleWidth::OneThird
        {
            hourly_box
        } else {
            self.vstack(6., vec![hourly_box, daily_box])
        };

        let body = if matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full) {
            self.responsive_columns(
                760.,
                S2,
                S2,
                CrossAxisAlignment::Start,
                vec![(1.0, left_col), (1.05, right_col)],
            )
        } else {
            self.vstack(6., vec![left_col, right_col])
        };

        self.home_frame_with_footer(HomeModuleId::WeatherModule, vec![header, body],
            Some(self.text(format!("{} · {}", w.source_label(), w.updated_label), MICRO, p.text3)))
    }
}
