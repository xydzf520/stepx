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

#[derive(Clone, Debug)]
pub struct HomeReminderContextTarget {
    pub spec: stepx_companion::home_reminder_governance::ReminderTargetSpec,
    pub primary_action: Option<(String, Box<Action>)>,
    pub copy_text: String,
    pub amoo_prompt: String,
    pub otp_or_pickup_code: String,
    pub sms_address: String,
    pub im_thread_id: String,
    pub notif_key: String,
    pub calendar_event_id: Option<i64>,
    pub capsule_id: String,
    pub mute_source_key: String,
    pub mute_source_label: String,
}

impl Default for HomeReminderContextTarget {
    fn default() -> Self {
        Self {
            spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                "",
                stepx_companion::home_reminder_governance::ReminderCategory::LedgerAndTaskIssue,
                "",
            ),
            primary_action: None,
            copy_text: String::new(),
            amoo_prompt: String::new(),
            otp_or_pickup_code: String::new(),
            sms_address: String::new(),
            im_thread_id: String::new(),
            notif_key: String::new(),
            calendar_event_id: None,
            capsule_id: String::new(),
            mute_source_key: String::new(),
            mute_source_label: String::new(),
        }
    }
}

#[derive(Clone, Debug)]
pub enum HomeReminderAction {
    /// Immediately mark this reminder as manually expired & sync across all Home modules.
    ManualExpire(HomeReminderContextTarget),
    /// Snooze this reminder for `minutes` (e.g., 30, 120).
    SnoozeMinutes(HomeReminderContextTarget, u32),
    /// Snooze until tonight 20:00 (or +3h if already past 19:30).
    SnoozeTonight20(HomeReminderContextTarget),
    /// Snooze until tomorrow morning 09:00.
    SnoozeTomorrow09(HomeReminderContextTarget),
    /// Set a custom per-item auto-expire deadline `hours` from now (e.g., 1h, 4h).
    CustomExpireHours(HomeReminderContextTarget, u32),
    /// Set a custom per-item auto-expire deadline at tonight 24:00 (end of today).
    CustomExpireTonight24(HomeReminderContextTarget),
    /// Restore a previously expired or snoozed reminder by primary key.
    RestoreReminder(String),
    /// Mute a noisy reminder source (app package, delivery platform, or SMS sender).
    MuteSource { source_key: String, label: String },
    /// Unmute a previously muted source.
    UnmuteSource(String),
    /// Configure category default auto-expiration TTL in minutes (`None` = reset to default).
    SetCategoryTtlMins {
        category: stepx_companion::home_reminder_governance::ReminderCategory,
        ttl_mins: Option<u32>,
    },
    /// Toggle or set the Home Reminder Governance Drawer (`提醒与过期管理`).
    OpenGovernanceDrawer(bool),
    /// Switch tab inside the Governance Drawer (`0 = 已过期/稍后提醒`, `1 = 自动过期规则`, `2 = 屏蔽来源`).
    SelectGovernanceTab(u8),
    /// One-click purge of all stale/completed items on `Page::Home`.
    PurgeExpiredNow,
    /// Clear all manual-expired & snoozed records.
    ClearAllGovernanceEntries,
}

impl Companion {
    pub(crate) fn wrap_home_reminder_target(
        &self,
        id_slug: impl Into<String>,
        target: HomeReminderContextTarget,
        el: El,
    ) -> El {
        let slug = id_slug.into();
        let ctx_wrapped = self.context_target(
            slug.clone(),
            crate::ui::context_menu::Target::HomeReminder(target.clone()),
            el,
        );
        Box::new(HomeBoundsRecorder {
            child: ctx_wrapped,
            target: HomeBoundsTarget::Reminder(slug, target),
            registry: self.home_pointer_registry.clone(),
        })
    }

    pub(in crate::ui) fn active_home_countdown_timers(
        &self,
        snapshot: &PushSnapshot,
    ) -> Vec<stepx_companion::push::LiveCountdownTimer> {
        let now_ms = Local::now().timestamp_millis();
        let mut timers: Vec<stepx_companion::push::LiveCountdownTimer> =
            stepx_companion::push::extract_active_countdown_timers(snapshot, now_ms)
                .into_iter()
                .filter(|t| {
                    let keys = vec![format!("timer:{}", t.key), format!("notif:{}", t.key)];
                    !self
                        .reminder_governance
                        .is_any_key_suppressed(&keys, None, None, None, now_ms)
                })
                .collect();
        if self.back_screen.pomodoro_active
            && !self.reminder_governance.is_any_key_suppressed(
                &[
                    "timer:desktop-pomodoro".to_string(),
                    "capsule:desktop-pomodoro".to_string(),
                ],
                None,
                None,
                None,
                now_ms,
            )
        {
            let dur = self.back_screen.pomodoro_duration_secs.max(1);
            let rem = self.live_pomodoro_remaining_secs().max(1);
            let elapsed = dur.saturating_sub(rem);
            let progress_pct = ((elapsed as u64 * 100) / (dur as u64)).clamp(4, 99) as u8;
            timers.push(stepx_companion::push::LiveCountdownTimer {
                key: "desktop-pomodoro".into(),
                package_name: "com.stepx.companion".into(),
                app_name: "专注计时".into(),
                title: format!("{} 分钟专注倒计时", (dur / 60).max(1)),
                total_secs: dur,
                remaining_secs: rem,
                remaining_label: stepx_companion::push::format_countdown_remaining_cn(rem),
                mm_ss_label: stepx_companion::push::format_countdown_mm_ss(rem),
                progress_pct,
                is_pomodoro: true,
            });
        }
        timers
    }

    pub(in crate::ui) fn render_home_live_countdown_banner(
        &self,
        snapshot: &PushSnapshot,
    ) -> Option<El> {
        let timers = self.active_home_countdown_timers(snapshot);
        if timers.is_empty() {
            return None;
        }
        let p = self.p();
        let width = self.home_module_content_width(HomeModuleId::LiveCountdown);
        let height = self.home_modules.module_height(HomeModuleId::LiveCountdown);
        let eff_h = self.resolve_effective_height(HomeModuleId::LiveCountdown, height);
        let profile =
            self.compute_home_module_value_profile(HomeModuleId::LiveCountdown, width, height);

        let mut header_copy = vec![self.wrap(vec![
                self.home_section_heading(UiIcon::Clock, "实时倒计时"),
                self.pill_badge(format!("{} 项进行中", timers.len()), p.text2, p.card2),
            ])];
        if eff_h != HomeModuleHeight::Compact {
            header_copy.push(self.text(
                Self::truncate_chars(
                    "手机倒计时与桌面专注计时同步，可推送至手机胶囊或背屏",
                    profile.char_budget,
                ),
                CAPTION,
                p.text2,
            ));
        }
        let header = self.responsive_columns(480., S2, S2, CrossAxisAlignment::Center, vec![
            (3., self.vstack(2., header_copy)),
            (1., Align::new(self.button_icon(
                "home-countdown-open-alarms",
                "闹钟与计时",
                UiIcon::Clock,
                Action::Page(Page::Alarms),
                Btn::Ghost,
            )).top_right().finish()),
        ]);

        let mut timer_cards: Vec<El> = Vec::with_capacity(timers.len().min(profile.max_items));
        for (idx, timer) in timers.iter().take(profile.max_items).enumerate() {
            let tint = if timer.is_pomodoro {
                p.indigo
            } else {
                p.warning
            };
            let tint_soft = if timer.is_pomodoro {
                p.indigo_soft
            } else {
                p.warning_soft
            };
            let total_mins = (timer.total_secs.div_ceil(60)).max(1);
            let display_title = Self::truncate_chars(&timer.title, profile.char_budget);
            let title_row = self.wrap(vec![
                self.text_w(&display_title, BODY + 1., p.text, Weight::Semibold),
                self.pill_badge(&timer.app_name, p.text2, p.card2),
                self.mono(&timer.mm_ss_label, SUBHEAD, tint),
            ]);
            let mut inner_stack = vec![title_row];
            if eff_h != HomeModuleHeight::Compact {
                inner_stack.push(self.vstack(
                    4.,
                    vec![
                        self.progress(Some(timer.progress_pct), tint),
                        self.text(
                            format!(
                                "{} · 已进行 {}%（总长 {} 分钟）",
                                timer.remaining_label, timer.progress_pct, total_mins
                            ),
                            MICRO,
                            p.text2,
                        ),
                    ],
                ));
            }

            let mut actions = Vec::new();
            if eff_h != HomeModuleHeight::Compact
                && (width != HomeModuleWidth::OneThird || eff_h == HomeModuleHeight::Tall)
            {
                actions.push(self.chip(
                    format!("home-countdown-push-capsule-{idx}"),
                    "推手机胶囊",
                    Some(UiIcon::Smartphone),
                    vec![Action::BackScreen(BackScreenAction::PushPhoneLiveCapsule {
                        tag: "stepx_timer".into(),
                        pill_text: timer.mm_ss_label.clone(),
                        title: timer.title.clone(),
                        subtitle: format!("{} · 桌面首页与手机顶部胶囊同步", timer.remaining_label),
                        countdown_secs: Some(timer.remaining_secs as u64),
                        sync_backscreen: true,
                    })],
                    true,
                    false,
                ));
            }
            if eff_h != HomeModuleHeight::Compact
                && matches!(width, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full)
            {
                actions.push(self.chip(
                    format!("home-countdown-pin-{idx}"),
                    "钉到背屏",
                    Some(UiIcon::Matrix),
                    vec![Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus(
                        format!("{} {}", timer.title, timer.remaining_label),
                    ))],
                    true,
                    false,
                ));
            }
            if timer.is_pomodoro {
                actions.push(self.button(
                    format!("home-countdown-stop-pomo-{idx}"),
                    "停止",
                    Action::BackScreen(BackScreenAction::StopPomodoro),
                    Btn::Danger,
                ));
            } else if !timer.key.is_empty() {
                actions.push(self.chip(
                    format!("home-countdown-dismiss-{idx}"),
                    "关闭提醒",
                    Some(UiIcon::Close),
                    vec![Action::Push(PushAction::DismissNotification(
                        timer.key.clone(),
                    ))],
                    false,
                    false,
                ));
            }

            let card_row = self.responsive_columns(600., S3, S2, CrossAxisAlignment::Start, vec![
                (2., Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                    .with_spacing(S2)
                    .with_child(self.icon_badge(UiIcon::Clock, 16., tint, tint_soft, 34.))
                    .with_child(Expanded::new(1., self.vstack(6., inner_stack)).finish())
                    .finish()),
                (1., Align::new(self.wrap(actions)).top_right().finish()),
            ]);
            let card_box_el = self.card_box(
                card_row,
                p.card,
                with_alpha_f(tint, 0.38),
                R_MD,
                12.,
                if eff_h == HomeModuleHeight::Compact {
                    8.
                } else {
                    12.
                },
            );
            let timer_primary_key = format!("timer:{}", timer.key);
            let mut timer_keys = vec![timer_primary_key.clone()];
            if !timer.key.is_empty() && timer.key != "desktop-pomodoro" {
                timer_keys.push(format!("notif:{}", timer.key));
                timer_keys.push(format!("capsule:phone-timer-{idx}"));
            } else if timer.is_pomodoro {
                timer_keys.push("capsule:desktop-pomodoro".into());
            }
            let timer_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    timer_primary_key,
                    stepx_companion::home_reminder_governance::ReminderCategory::LiveTimer,
                    timer.title.clone(),
                )
                .with_aliases(timer_keys)
                .with_subtitle(format!("{} · {}", timer.app_name, timer.remaining_label))
                .with_source(timer.app_name.clone(), None),
                primary_action: Some((
                    if timer.is_pomodoro {
                        "停止计时".into()
                    } else {
                        "关闭提醒".into()
                    },
                    Box::new(if timer.is_pomodoro {
                        Action::BackScreen(BackScreenAction::StopPomodoro)
                    } else if !timer.key.is_empty() {
                        Action::Push(PushAction::DismissNotification(timer.key.clone()))
                    } else {
                        Action::Page(Page::Alarms)
                    }),
                )),
                copy_text: format!("{} · {}", timer.title, timer.remaining_label),
                amoo_prompt: format!(
                    "请根据当前计时「{}（{}）」安排后续事项",
                    timer.title, timer.remaining_label
                ),
                otp_or_pickup_code: String::new(),
                sms_address: String::new(),
                im_thread_id: String::new(),
                notif_key: if timer.is_pomodoro {
                    String::new()
                } else {
                    timer.key.clone()
                },
                calendar_event_id: None,
                capsule_id: if timer.is_pomodoro {
                    "desktop-pomodoro".into()
                } else {
                    format!("phone-timer-{idx}")
                },
                mute_source_key: String::new(),
                mute_source_label: String::new(),
            };
            timer_cards.push(self.wrap_home_reminder_target(
                format!("home-rem-timer-{idx}"),
                timer_target,
                card_box_el,
            ));
        }

        let mut children = vec![header];
        if profile.columns >= 2 && timer_cards.len() >= 2 {
            let cols = profile.columns.min(timer_cards.len());
            children.push(self.grid_cols(timer_cards, cols, S2));
        } else {
            children.extend(timer_cards);
        }

        if eff_h == HomeModuleHeight::Tall {
            children.push(self.wrap(vec![
                self.pill_badge("快速加时/预设", p.indigo, p.indigo_soft),
                self.chip(
                    "home-countdown-preset-25",
                    "25分钟番茄钟",
                    Some(UiIcon::Clock),
                    vec![Action::BackScreen(BackScreenAction::StartPomodoro(25))],
                    false,
                    false,
                ),
                self.chip(
                    "home-countdown-preset-15",
                    "15分钟冲刺",
                    Some(UiIcon::Clock),
                    vec![Action::BackScreen(BackScreenAction::StartPomodoro(15))],
                    false,
                    false,
                ),
                self.chip(
                    "home-countdown-preset-45",
                    "45分钟深度工作",
                    Some(UiIcon::Clock),
                    vec![Action::BackScreen(BackScreenAction::StartPomodoro(45))],
                    false,
                    false,
                ),
                self.chip(
                    "home-countdown-preset-5",
                    "+5m 小憩",
                    Some(UiIcon::Clock),
                    vec![Action::BackScreen(BackScreenAction::StartPomodoro(5))],
                    false,
                    false,
                ),
            ]));
        }

        Some(self.panel(children))
    }

    /// 头部胶囊 chip 的短标签，例如「叮咚买菜 · 已送达」「取件码 4-2-1086」「@我 · 星辰停泊港」。
    pub(in crate::ui) fn home_capsule_short_label(
        cap: &crate::ui::shell::TopBarLiveCapsuleItem,
    ) -> String {
        use crate::ui::shell::TopBarCapsuleKind as K;
        let clean = |s: &str| -> String {
            s.trim_start_matches(|c: char| {
                !c.is_ascii_alphanumeric() && !('\u{4e00}'..='\u{9fff}').contains(&c)
            })
            .trim()
            .to_string()
        };
        let title = clean(&cap.title);
        // 去掉与徽标重复的前缀，只取第一段
        let rest = title
            .strip_prefix(cap.domain_badge.as_str())
            .map(|s| s.trim_start_matches([' ', '·']).to_string())
            .unwrap_or_else(|| title.clone());
        let first_seg = |s: &str| -> String {
            let seg = s
                .split(" · ")
                .next()
                .unwrap_or(s)
                .trim()
                .trim_end_matches('…')
                .trim();
            seg.to_string()
        };
        match cap.kind {
            K::PickupOrOtp => {
                if let Some(code) = cap.pickup_code.as_deref().filter(|c| !c.trim().is_empty()) {
                    format!("取件码 {}", Self::truncate_chars(code.trim(), 12))
                } else if !cap.metric.trim().is_empty() {
                    format!(
                        "{} {}",
                        cap.domain_badge,
                        Self::truncate_chars(cap.metric.trim(), 12)
                    )
                } else {
                    cap.domain_badge.clone()
                }
            }
            K::DirectMention => {
                let t = rest
                    .trim_start_matches("@提及")
                    .trim_start_matches([' ', '·']);
                format!("@我 · {}", Self::truncate_chars(&first_seg(t), 10))
            }
            K::ScheduleOrAlarm => format!(
                "{} {}",
                cap.metric.trim(),
                Self::truncate_chars(&first_seg(&title), 8)
            )
            .trim()
            .to_string(),
            _ => {
                let seg = first_seg(&rest);
                if seg.is_empty() {
                    cap.domain_badge.clone()
                } else {
                    format!("{} · {}", cap.domain_badge, Self::truncate_chars(&seg, 8))
                }
            }
        }
    }

    pub(crate) fn render_home_reminder_governance_panel(&self) -> El {
        use stepx_companion::home_reminder_governance::{
            recent_auto_expired_notices, ReminderCategory, ReminderDisposition,
        };
        let p = self.p();
        let entry_count = self.reminder_governance.entries.len();
        let muted_count = self.reminder_governance.muted_sources.len();
        let auto_notices = recent_auto_expired_notices();

        let header = row_between_expanded(
            self.vstack(
                2.,
                vec![
                    self.hstack(
                        S2,
                        vec![
                            self.content_icon(UiIcon::Bell, 15., p.accent),
                            self.text_w("提醒与过期管理", SUBHEAD, p.text, Weight::Semibold),
                            self.pill_badge(
                                format!(
                                    "已处理 {entry_count} · 自动过期 {} · 屏蔽来源 {muted_count}",
                                    auto_notices.len()
                                ),
                                p.accent,
                                p.accent_soft,
                            ),
                        ],
                    ),
                    self.line(
                        "在首页任意提醒上右键，可立即过期、稍后提醒、自定义过期时间或修改分类默认有效期；所有操作跨模块实时生效并支持随时恢复。",
                        MICRO,
                        p.text2,
                    ),
                ],
            ),
            self.hstack(
                6.,
                vec![
                    self.button_icon(
                        "home-gov-purge-btn",
                        "一键清理已结束提醒",
                        UiIcon::Check,
                        Action::HomeReminder(HomeReminderAction::PurgeExpiredNow),
                        Btn::Secondary,
                    ),
                    self.button(
                        "home-gov-clear-btn",
                        "清空记录",
                        Action::HomeReminder(HomeReminderAction::ClearAllGovernanceEntries),
                        Btn::Ghost,
                    ),
                    self.button(
                        "home-gov-close-btn",
                        "收起",
                        Action::HomeReminder(HomeReminderAction::OpenGovernanceDrawer(false)),
                        Btn::Primary,
                    ),
                ],
            ),
            S2,
        );

        let tabs_row = self.wrap(vec![
            self.chip(
                "home-gov-tab-0",
                format!("已过期 / 稍后提醒 ({entry_count})"),
                Some(UiIcon::Check),
                vec![Action::HomeReminder(
                    HomeReminderAction::SelectGovernanceTab(0),
                )],
                self.home_reminder_drawer_tab == 0,
                false,
            ),
            self.chip(
                "home-gov-tab-1",
                format!("自动过期规则与记录 ({})", ReminderCategory::ALL.len()),
                Some(UiIcon::Clock),
                vec![Action::HomeReminder(
                    HomeReminderAction::SelectGovernanceTab(1),
                )],
                self.home_reminder_drawer_tab == 1,
                false,
            ),
            self.chip(
                "home-gov-tab-2",
                format!("已屏蔽来源 ({muted_count})"),
                Some(UiIcon::Ban),
                vec![Action::HomeReminder(
                    HomeReminderAction::SelectGovernanceTab(2),
                )],
                self.home_reminder_drawer_tab == 2,
                false,
            ),
        ]);

        let body_el = match self.home_reminder_drawer_tab {
            1 => {
                let mut rule_rows: Vec<El> = Vec::with_capacity(ReminderCategory::ALL.len() + 2);
                for (cat_idx, cat) in ReminderCategory::ALL.into_iter().enumerate() {
                    let cur_mins = self.reminder_governance.effective_ttl_mins(cat);
                    let is_custom = self
                        .reminder_governance
                        .category_ttl_mins
                        .contains_key(&cat);
                    let presets: &[(u32, &str)] = match cat {
                        ReminderCategory::OtpCode => &[
                            (10, "10分钟"),
                            (15, "15分钟"),
                            (30, "30分钟"),
                            (60, "1小时"),
                        ],
                        ReminderCategory::ScheduleAndMeeting => &[
                            (15, "15分钟"),
                            (30, "30分钟"),
                            (60, "1小时"),
                            (180, "3小时"),
                        ],
                        ReminderCategory::LiveTimer => &[
                            (30, "30分钟"),
                            (60, "1小时"),
                            (120, "2小时"),
                            (240, "4小时"),
                        ],
                        ReminderCategory::TakeoutAndMobility => &[
                            (60, "1小时"),
                            (120, "2小时"),
                            (240, "4小时"),
                            (480, "8小时"),
                        ],
                        ReminderCategory::ExpressPickup => &[
                            (360, "6小时"),
                            (720, "12小时"),
                            (1440, "24小时"),
                            (2880, "48小时"),
                        ],
                        ReminderCategory::WeatherAndLife => &[
                            (120, "2小时"),
                            (360, "6小时"),
                            (720, "12小时"),
                            (1440, "24小时"),
                        ],
                        ReminderCategory::LedgerAndTaskIssue => &[
                            (720, "12小时"),
                            (1440, "24小时"),
                            (2880, "48小时"),
                            (4320, "72小时"),
                        ],
                        _ => &[
                            (60, "1小时"),
                            (240, "4小时"),
                            (720, "12小时"),
                            (1440, "24小时"),
                        ],
                    };
                    let rule_desc = match cat {
                        ReminderCategory::OtpCode => "复制后或超过时效自动从首页与胶囊收起",
                        ReminderCategory::TakeoutAndMobility => {
                            "已送达读后 30 分钟或超过时效自动收起"
                        }
                        ReminderCategory::ExpressPickup => "取件完成或超过时效自动收起",
                        ReminderCategory::ExpressInTransit => "签收或超过在途时效自动收起",
                        ReminderCategory::DirectMentionAndCareer => {
                            "已读 30 分钟后或超过时效自动收起"
                        }
                        ReminderCategory::UnreadMessage => "会话已读立即消解，或超过时效自动收起",
                        ReminderCategory::MissedCall => "回拨/短信回复立即消解，或超过时效自动收起",
                        ReminderCategory::ScheduleAndMeeting => {
                            "今日会议结束后超过缓冲时效自动收起"
                        }
                        ReminderCategory::ScheduleConflict => "冲突日程结束后自动收起",
                        ReminderCategory::WeatherAndLife => "天气与出行提示超过时效自动收起",
                        ReminderCategory::LedgerAndTaskIssue => "台账与待办事项超过时效自动收起",
                        ReminderCategory::LiveTimer => "倒计时/番茄钟归零后超过时效自动收起",
                    };
                    let mut chips: Vec<El> = presets
                        .iter()
                        .map(|&(mins, label)| {
                            self.chip(
                                format!("home-ttl-preset-{cat_idx}-{mins}"),
                                label,
                                None,
                                vec![Action::HomeReminder(
                                    HomeReminderAction::SetCategoryTtlMins {
                                        category: cat,
                                        ttl_mins: Some(mins),
                                    },
                                )],
                                cur_mins == mins,
                                false,
                            )
                        })
                        .collect();
                    if is_custom {
                        chips.push(self.chip(
                            format!("home-ttl-reset-{cat_idx}"),
                            "恢复默认",
                            None,
                            vec![Action::HomeReminder(
                                HomeReminderAction::SetCategoryTtlMins {
                                    category: cat,
                                    ttl_mins: None,
                                },
                            )],
                            false,
                            false,
                        ));
                    }
                    rule_rows.push(
                        Container::new(row_between_expanded(
                            self.hstack(
                                S2,
                                vec![
                                    self.line_w(cat.label(), CAPTION, p.text, Weight::Semibold),
                                    self.pill_badge(
                                        if cur_mins >= 60 && cur_mins % 60 == 0 {
                                            format!("当前 {} 小时", cur_mins / 60)
                                        } else {
                                            format!("当前 {cur_mins} 分钟")
                                        },
                                        p.accent,
                                        p.accent_soft,
                                    ),
                                    self.line(rule_desc, MICRO, p.text3),
                                ],
                            ),
                            self.wrap(chips),
                            S2,
                        ))
                        .with_horizontal_padding(S2)
                        .with_vertical_padding(5.)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                    );
                }
                if !auto_notices.is_empty() {
                    let mut notice_chips: Vec<El> = Vec::with_capacity(auto_notices.len().min(8));
                    for n in auto_notices.iter().take(8) {
                        notice_chips.push(self.pill_badge(
                            format!(
                                "{} · {}（{}）",
                                n.category.label(),
                                Self::truncate_chars(&n.title, 14),
                                n.reason
                            ),
                            p.text2,
                            p.card,
                        ));
                    }
                    rule_rows.push(
                        Container::new(self.vstack(
                            4.,
                            vec![
                                self.line_w(
                                    "最近自动过期拦截记录（已自动从首页隐藏）",
                                    MICRO,
                                    p.text2,
                                    Weight::Semibold,
                                ),
                                self.wrap(notice_chips),
                            ],
                        ))
                        .with_uniform_padding(S2)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                    );
                }
                self.vstack(4., rule_rows)
            }
            2 => {
                if self.reminder_governance.muted_sources.is_empty() {
                    Container::new(self.line(
                        "当前未屏蔽任何提醒来源。右键首页提醒选择「🔕 屏蔽此类提醒」即可在此管理。",
                        CAPTION,
                        p.text3,
                    ))
                    .with_uniform_padding(S2)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                } else {
                    let rows: Vec<El> = self
                        .reminder_governance
                        .muted_sources
                        .values()
                        .enumerate()
                        .map(|(idx, src)| {
                            Container::new(row_between_expanded(
                                self.hstack(
                                    S2,
                                    vec![
                                        self.pill_badge("已屏蔽", p.danger, p.danger_soft),
                                        self.line_w(&src.label, CAPTION, p.text, Weight::Semibold),
                                        self.mono(&src.source_key, MICRO, p.text3),
                                    ],
                                ),
                                self.chip(
                                    format!("home-unmute-source-{idx}"),
                                    "取消屏蔽",
                                    Some(UiIcon::Restore),
                                    vec![Action::HomeReminder(HomeReminderAction::UnmuteSource(
                                        src.source_key.clone(),
                                    ))],
                                    true,
                                    false,
                                ),
                                S2,
                            ))
                            .with_horizontal_padding(S2)
                            .with_vertical_padding(5.)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                        })
                        .collect();
                    self.vstack(4., rows)
                }
            }
            _ => {
                if self.reminder_governance.entries.is_empty() {
                    Container::new(self.line(
                        "暂无已手动过期或稍后提醒的条目。右键首页任意提醒卡片，或点击卡片上的「不再提醒」即可立即过期归档。",
                        CAPTION,
                        p.text3,
                    ))
                    .with_uniform_padding(S2)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                } else {
                    let rows: Vec<El> = self
                        .reminder_governance
                        .entries
                        .values()
                        .take(16)
                        .enumerate()
                        .map(|(idx, entry)| {
                            let (disp_label, disp_fg, disp_bg) = match entry.disposition {
                                ReminderDisposition::ManualExpired { .. } => {
                                    ("已手动过期".to_string(), p.text2, p.card)
                                }
                                ReminderDisposition::Snoozed { until_ms } => {
                                    let hm =
                                        chrono::TimeZone::timestamp_millis_opt(&Local, until_ms)
                                            .single()
                                            .map(|dt| dt.format("%m-%d %H:%M").to_string())
                                            .unwrap_or_else(|| "稍后".into());
                                    (format!("稍后提醒 · {hm}"), p.indigo, p.indigo_soft)
                                }
                                ReminderDisposition::CustomExpireAt { expire_at_ms } => {
                                    let hm = chrono::TimeZone::timestamp_millis_opt(
                                        &Local,
                                        expire_at_ms,
                                    )
                                    .single()
                                    .map(|dt| dt.format("%m-%d %H:%M").to_string())
                                    .unwrap_or_else(|| "定时".into());
                                    (format!("将于 {hm} 过期"), p.warning, p.warning_soft)
                                }
                            };
                            Container::new(row_between_expanded(
                                self.hstack(
                                    S2,
                                    vec![
                                        self.pill_badge(
                                            entry.spec.category.label(),
                                            p.accent,
                                            p.accent_soft,
                                        ),
                                        self.pill_badge(disp_label, disp_fg, disp_bg),
                                        self.line_w(
                                            Self::truncate_chars(&entry.spec.title, 26),
                                            CAPTION,
                                            p.text,
                                            Weight::Semibold,
                                        ),
                                        self.line(
                                            Self::truncate_chars(&entry.spec.subtitle, 32),
                                            MICRO,
                                            p.text3,
                                        ),
                                    ],
                                ),
                                self.chip(
                                    format!("home-reminder-restore-{idx}"),
                                    "恢复提醒",
                                    Some(UiIcon::Restore),
                                    vec![Action::HomeReminder(
                                        HomeReminderAction::RestoreReminder(
                                            entry.spec.primary_key.clone(),
                                        ),
                                    )],
                                    true,
                                    false,
                                ),
                                S2,
                            ))
                            .with_horizontal_padding(S2)
                            .with_vertical_padding(5.)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                        })
                        .collect();
                    self.vstack(4., rows)
                }
            }
        };

        let panel = Container::new(self.vstack(S2, vec![header, tabs_row, body_el]))
            .with_uniform_padding(S2 + 2.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.accent))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();
        home_geometry_probe("home-reminder-governance-panel", panel)
    }
}
