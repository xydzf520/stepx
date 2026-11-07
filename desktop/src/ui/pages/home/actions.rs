#![allow(unused_imports)]
use crate::ui::{
    amoo_ui_state::AmooUiAction, backscreen_state::BackScreenAction, icons::UiIcon,
    led_grid::LedGrid, theme::*, widgets::*, work_state::WorkAction, Action, Companion, Page,
    PushAction, ToolPage, F_ALARM_TIME, F_AMOO, F_AMOO_GUI_APP, F_BACK_MARQUEE, F_CALENDAR_TIME,
    F_HOME_COMMAND, F_OV_TEXT, F_PHONE_SCHEDULE_TITLE, F_PUSH_REPLY, F_SMS_BODY, F_SMS_TO,
    F_WORK_PROMPT,
};
use super::*;
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

impl Companion {
    fn set_home_layout_edit_mode(&mut self, enabled: bool) {
        if enabled {
            if !self.home_module_edit_mode {
                self.home_pointer_registry.borrow_mut().animate_from_current(!self.animations_disabled());
            }
            self.home_pointer_registry.borrow_mut().begin_edit(&self.home_modules);
            self.home_module_edit_mode = true;
            self.home_edit_anim_started = std::time::Instant::now();
            self.status = "拖动调整位置；完成后保存，取消可恢复原布局".into();
        } else if self.home_module_edit_mode {
            self.save_home_modules_config();
            if self.error { return; }
            self.home_pointer_registry.borrow_mut().animate_from_current(!self.animations_disabled());
            self.home_module_edit_mode = false;
            self.home_pending_move = None;
            self.home_pointer_registry.borrow_mut().finish_edit();
            self.status = "首页布局已保存".into();
        }
    }

    pub(in crate::ui) fn apply_home_action(
        &mut self,
        action: &Action,
        mut ctx: Option<&mut ViewContext<Self>>,
    ) -> bool {
        match action {
            Action::HomeReminder(rem_action) => {
                self.error = false;
                let now_local = Local::now();
                let now_ms = now_local.timestamp_millis();
                let calendar_id = match rem_action {
                    HomeReminderAction::ManualExpire(target)
                    | HomeReminderAction::SnoozeMinutes(target, _)
                    | HomeReminderAction::SnoozeTonight20(target)
                    | HomeReminderAction::SnoozeTomorrow09(target)
                    | HomeReminderAction::CustomExpireHours(target, _)
                    | HomeReminderAction::CustomExpireTonight24(target) => target.calendar_event_id,
                    _ => None,
                };
                if let Some(reason) = calendar_id.and_then(|id| self.calendar_event_action_limit(id)) {
                    self.status = reason.into();
                    self.error = true;
                    return true;
                }
                match rem_action {
                    HomeReminderAction::ManualExpire(target) => {
                        self.reminder_governance
                            .manual_expire(target.spec.clone(), now_ms);
                        if !target.capsule_id.is_empty() {
                            self.back_screen
                                .dismissed_capsule_ids
                                .insert(target.capsule_id.clone());
                        }
                        if !target.otp_or_pickup_code.is_empty() {
                            let clean_code = target.otp_or_pickup_code.trim().to_uppercase();
                            if !clean_code.is_empty() {
                                self.push.prefs.dismissed_pickup_codes.insert(clean_code);
                                let _ = self.push.prefs.save();
                                self.sync_live_express_parcels();
                            }
                        }
                        if !target.sms_address.is_empty() {
                            self.mark_sms_thread_read(&target.sms_address);
                            let norm = stepx_companion::home_reminder_governance::normalize_phone_or_addr(
                                &target.sms_address,
                            );
                            for c in &mut self.call_logs {
                                if stepx_companion::home_reminder_governance::normalize_phone_or_addr(
                                    &c.number,
                                ) == norm
                                    && c.call_type == 3
                                {
                                    c.call_type = 1;
                                }
                            }
                        }
                        if !target.im_thread_id.is_empty() {
                            for t in &mut self.push.snapshot.threads {
                                if t.thread_id == target.im_thread_id {
                                    t.unread_count = 0;
                                }
                            }
                            for n in &mut self.push.snapshot.notifications {
                                if n.thread_id == target.im_thread_id {
                                    n.read = true;
                                }
                            }
                        }
                        if !target.notif_key.is_empty() {
                            for n in &mut self.push.snapshot.notifications {
                                if n.key == target.notif_key {
                                    n.read = true;
                                    n.dismissed = true;
                                    n.active = false;
                                }
                            }
                        }
                        if let Some(ev_id) = target.calendar_event_id {
                            self.back_screen.completed_calendar_events.insert(ev_id);
                            self.back_screen.save_completed_calendar_events();
                        }
                        if target.spec.primary_key == "timer:desktop-pomodoro"
                            || target.capsule_id == "desktop-pomodoro"
                        {
                            self.back_screen.pomodoro_active = false;
                            self.back_screen.pomodoro_started_at = None;
                            self.back_screen.pomodoro_last_remaining_secs = Some(0);
                        }
                        self.sync_notification_center();
                        self.status = format!(
                            "已将「{}」设为过期，不再提醒（可在首页提醒管理中随时恢复）",
                            target.spec.title
                        );
                    }
                    HomeReminderAction::SnoozeMinutes(target, mins) => {
                        let until_ms = now_ms + (*mins as i64) * 60_000;
                        self.reminder_governance
                            .snooze_until(target.spec.clone(), until_ms, now_ms);
                        self.status = format!(
                            "已将「{}」设为稍后提醒（{} 分钟后恢复）",
                            target.spec.title, mins
                        );
                    }
                    HomeReminderAction::SnoozeTonight20(target) => {
                        let today_naive = now_local.date_naive();
                        let tonight_20 = today_naive
                            .and_hms_opt(20, 0, 0)
                            .and_then(|dt| dt.and_local_timezone(Local).single())
                            .map(|dt| dt.timestamp_millis())
                            .unwrap_or(now_ms + 3 * 3_600_000);
                        let until_ms = if tonight_20 > now_ms + 15 * 60_000 {
                            tonight_20
                        } else {
                            now_ms + 3 * 3_600_000
                        };
                        self.reminder_governance
                            .snooze_until(target.spec.clone(), until_ms, now_ms);
                        self.status = format!(
                            "已将「{}」设为今晚 20:00 再提醒",
                            target.spec.title
                        );
                    }
                    HomeReminderAction::SnoozeTomorrow09(target) => {
                        let tomorrow_naive = now_local
                            .date_naive()
                            .succ_opt()
                            .unwrap_or_else(|| now_local.date_naive());
                        let tomorrow_09 = tomorrow_naive
                            .and_hms_opt(9, 0, 0)
                            .and_then(|dt| dt.and_local_timezone(Local).single())
                            .map(|dt| dt.timestamp_millis())
                            .unwrap_or(now_ms + 14 * 3_600_000);
                        self.reminder_governance
                            .snooze_until(target.spec.clone(), tomorrow_09, now_ms);
                        self.status = format!(
                            "已将「{}」设为明早 09:00 再提醒",
                            target.spec.title
                        );
                    }
                    HomeReminderAction::CustomExpireHours(target, hours) => {
                        let expire_at_ms = now_ms + (*hours as i64) * 3_600_000;
                        self.reminder_governance
                            .set_custom_expire_at(target.spec.clone(), expire_at_ms, now_ms);
                        for k in target.spec.all_keys() {
                            if let Some(code) = k.strip_prefix("code:") {
                                self.push.prefs.dismissed_pickup_codes.remove(code);
                            }
                            if let Some(ev_str) = k.strip_prefix("cal:") {
                                if let Ok(ev_id) = ev_str.parse::<i64>() {
                                    self.back_screen.completed_calendar_events.remove(&ev_id);
                                }
                            }
                        }
                        if !target.capsule_id.is_empty() {
                            self.back_screen
                                .dismissed_capsule_ids
                                .remove(&target.capsule_id);
                        }
                        self.status = format!(
                            "已将「{}」设为 {} 小时后自动过期",
                            target.spec.title, hours
                        );
                    }
                    HomeReminderAction::CustomExpireTonight24(target) => {
                        let today_naive = now_local.date_naive();
                        let tonight_24 = today_naive
                            .and_hms_opt(23, 59, 59)
                            .and_then(|dt| dt.and_local_timezone(Local).single())
                            .map(|dt| dt.timestamp_millis())
                            .unwrap_or(now_ms + 8 * 3_600_000);
                        let expire_at_ms = tonight_24.max(now_ms + 30 * 60_000);
                        self.reminder_governance
                            .set_custom_expire_at(target.spec.clone(), expire_at_ms, now_ms);
                        self.status = format!(
                            "已将「{}」设为今晚 24:00 自动过期",
                            target.spec.title
                        );
                    }
                    HomeReminderAction::RestoreReminder(key) => {
                        if let Some(spec) = self.reminder_governance.restore_reminder(key, now_ms) {
                            for k in spec.all_keys() {
                                if let Some(cap_id) = k.strip_prefix("capsule:") {
                                    self.back_screen.dismissed_capsule_ids.remove(cap_id);
                                }
                                if let Some(code) = k.strip_prefix("code:") {
                                    self.push.prefs.dismissed_pickup_codes.remove(code);
                                }
                                if let Some(parcel) = k.strip_prefix("parcel:") {
                                    self.push.prefs.dismissed_pickup_codes.remove(parcel);
                                }
                                if let Some(ev_str) = k.strip_prefix("cal:") {
                                    if let Ok(ev_id) = ev_str.parse::<i64>() {
                                        self.back_screen.completed_calendar_events.remove(&ev_id);
                                    }
                                }
                            }
                            let _ = self.push.prefs.save();
                            self.back_screen.save_completed_calendar_events();
                            self.sync_live_express_parcels();
                            self.sync_notification_center();
                            self.status = format!("已恢复提醒：「{}」", spec.title);
                        } else {
                            self.status = "该提醒已处于显示状态".into();
                        }
                    }
                    HomeReminderAction::MuteSource { source_key, label } => {
                        self.reminder_governance
                            .mute_source(source_key.clone(), label.clone(), now_ms);
                        self.status = format!(
                            "已屏蔽来源「{label}」的首页提醒（可在提醒管理面板随时解除）"
                        );
                    }
                    HomeReminderAction::UnmuteSource(source_key) => {
                        self.reminder_governance.unmute_source(source_key);
                        self.status = "已解除该来源的提醒屏蔽".into();
                    }
                    HomeReminderAction::SetCategoryTtlMins { category, ttl_mins } => {
                        self.reminder_governance
                            .set_category_ttl_mins(*category, *ttl_mins);
                        self.status = format!(
                            "已将「{}」自动过期时效设为 {}",
                            category.label(),
                            stepx_companion::home_reminder_governance::HomeReminderGovernance::format_ttl_label(
                                *ttl_mins
                            )
                        );
                    }
                    HomeReminderAction::OpenGovernanceDrawer(open) => {
                        self.home_reminder_drawer_open = *open;
                        self.status = if *open {
                            "已展开首页提醒与过期管理面板".into()
                        } else {
                            "已收起首页提醒与过期管理面板".into()
                        };
                    }
                    HomeReminderAction::SelectGovernanceTab(tab) => {
                        self.home_reminder_drawer_open = true;
                        self.home_reminder_drawer_tab = (*tab).min(2);
                        let tab_lbl = ["已过期 / 已忽略", "稍后提醒中", "各类提醒自动过期时效"]
                            [self.home_reminder_drawer_tab as usize];
                        self.status = format!("已切换至「{tab_lbl}」标签页");
                    }
                    HomeReminderAction::PurgeExpiredNow => {
                        let today = now_local.date_naive();
                        let now_min = now_local.hour() * 60 + now_local.minute();
                        let mut purged = 0usize;
                        let ended_events: Vec<_> = self
                            .back_screen
                            .stepos_suite
                            .calendar_events
                            .iter()
                            .filter(|ev| {
                                let (ey, em, ed) = ev.date_ymd();
                                if (ey, em, ed) != (today.year(), today.month(), today.day())
                                    || ev.all_day
                                    || self.calendar_event_action_limit(ev.id).is_some()
                                    || self.back_screen.completed_calendar_events.contains(&ev.id)
                                {
                                    return false;
                                }
                                let end_min =
                                    stepx_companion::home_hub::timestamp_to_local_ymd_parts(
                                        if ev.dtend > ev.dtstart {
                                            ev.dtend / 1000
                                        } else {
                                            ev.dtstart / 1000 + 3600
                                        },
                                    )
                                    .map(|(_, h, m)| h * 60 + m)
                                    .unwrap_or(23 * 60 + 59);
                                end_min <= now_min
                            })
                            .map(|ev| (ev.id, ev.title.clone(), ev.time_range_label()))
                            .collect();
                        for (ev_id, ev_title, ev_range) in ended_events {
                            self.back_screen.completed_calendar_events.insert(ev_id);
                            let spec =
                                stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                                    format!("cal:{ev_id}"),
                                    stepx_companion::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
                                    ev_title,
                                )
                                .with_aliases(
                                    stepx_companion::home_reminder_governance::canonical_keys_for_calendar_event(
                                        ev_id,
                                    ),
                                )
                                .with_subtitle(ev_range);
                            self.reminder_governance.manual_expire(spec, now_ms);
                            purged += 1;
                        }
                        if purged > 0 {
                            self.back_screen.save_completed_calendar_events();
                        }
                        self.status = if purged > 0 {
                            format!("已一键清理 {purged} 项今日已结束日程与过期提醒")
                        } else {
                            "已扫描并清理过期提醒，当前首页所有提醒均为有效活跃项".into()
                        };
                    }
                    HomeReminderAction::ClearAllGovernanceEntries => {
                        self.reminder_governance.clear_all_suppressed();
                        self.back_screen.dismissed_capsule_ids.clear();
                        self.status = "已清空全部手动过期与稍后提醒记录".into();
                    }
                }
            }
            Action::HomeToggleTools => {
                self.home_tools_expanded = !self.home_tools_expanded;
            }
            Action::HomeTogglePending => {
                self.home_pending_expanded = !self.home_pending_expanded;
            }
            Action::HomeSelectAiQuadrant(idx) => {
                self.error = false;
                self.home_ai_quadrant = (*idx).min(3);
                let label = ["日程负荷", "工作交付", "生活与守护", "待决与沟通"][self.home_ai_quadrant];
                self.status = format!("已聚焦所选周「{label}」");
            }
            Action::HomeModule(mod_action) => {
                self.error = false;
                if self.context_home_is_focused() {
                    self.handle_context_action(&crate::ui::context_state::ContextAction::Attention);
                }
                let before = self.home_modules.clone();
                if self.home_module_edit_mode {
                    self.home_pointer_registry.borrow_mut().begin_edit(&before);
                }
                match mod_action {
                    HomeModuleAction::ToggleEditMode => {
                        self.set_home_layout_edit_mode(!self.home_module_edit_mode);
                    }
                    HomeModuleAction::SetEditMode(on) => {
                        self.set_home_layout_edit_mode(*on);
                    }
                    HomeModuleAction::UndoEdit => {
                        let previous = self.home_pointer_registry.borrow_mut().edit_undo.pop();
                        if let Some(previous) = previous {
                            self.home_modules = previous;
                            self.home_pending_move = None;
                            self.home_pointer_registry.borrow_mut().active_op = None;
                            self.status = "已撤销上一步布局调整".into();
                        }
                    }
                    HomeModuleAction::CancelEdit => {
                        self.home_pointer_registry.borrow_mut().animate_from_current(!self.animations_disabled());
                        let previous = self.home_pointer_registry.borrow_mut().edit_original.take();
                        if let Some(previous) = previous { self.home_modules = previous; }
                        self.home_module_edit_mode = false;
                        self.home_pending_move = None;
                        self.home_pointer_registry.borrow_mut().finish_edit();
                        self.status = "已取消调整，恢复原布局".into();
                    }
                    HomeModuleAction::ToggleModuleLibrary => {
                        let mut registry = self.home_pointer_registry.borrow_mut();
                        registry.module_library_open = !registry.module_library_open;
                    }
                    HomeModuleAction::FilterCategory(cat) => {
                        self.home_module_filter = *cat;
                        self.status = match cat {
                            Some(c) => format!("模块筛选：{}", c.label()),
                            None => "模块筛选：全部模块".into(),
                        };
                    }
                    HomeModuleAction::AddModule(id) => {
                        if self.home_modules.add_module(*id) {
                            let rank = self.home_modules.enabled.len();
                            self.status = format!("已显示首页模块：「{}」（位于 #{rank}）", id.title());
                        } else {
                            self.status = format!("模块「{}」已在首页显示列表中", id.title());
                        }
                    }
                    HomeModuleAction::RemoveModule(id) => {
                        if self.home_modules.remove_module(*id) {
                            if self.home_pending_move == Some(*id) {
                                self.home_pending_move = None;
                            }
                            if !self.page_requires_svg() {
                                let _ = self.svg_slots.take_pending();
                            }
                            self.status = format!(
                                "已从首页隐藏模块：「{}」（可在模块库随时恢复显示）",
                                id.title()
                            );
                        } else {
                            self.error = true;
                            self.status = "首页至少需要保留 1 个功能模块，无法全部隐藏".into();
                        }
                    }
                    HomeModuleAction::ToggleModule(id) => {
                        if self.home_modules.is_enabled(*id) {
                            if self.home_modules.remove_module(*id) {
                                if self.home_pending_move == Some(*id) {
                                    self.home_pending_move = None;
                                }
                                if !self.page_requires_svg() {
                                    let _ = self.svg_slots.take_pending();
                                }
                                self.status = format!("已隐藏模块：「{}」", id.title());
                            } else {
                                self.error = true;
                                self.status = "首页至少需要保留 1 个功能模块，无法全部隐藏".into();
                            }
                        } else if self.home_modules.add_module(*id) {
                            let rank = self.home_modules.enabled.len();
                            self.status = format!("已显示模块：「{}」（位于 #{rank}）", id.title());
                        }
                    }
                    HomeModuleAction::SetCategoryEnabled(cat, enable) => {
                        let changed = self.home_modules.set_category_enabled(*cat, *enable);
                        if !self.page_requires_svg() {
                            let _ = self.svg_slots.take_pending();
                        }
                        self.status = if *enable {
                            format!("已开启「{}」分类下全部模块（共 {} 个）", cat.label(), self.home_modules.enabled.len())
                        } else if changed {
                            format!("已隐藏「{}」分类模块（当前显示 {} 个）", cat.label(), self.home_modules.enabled.len())
                        } else {
                            "首页至少需要保留 1 个显示模块".into()
                        };
                    }
                    HomeModuleAction::ShowAllModules => {
                        self.home_modules.show_all_modules();
                        self.status = format!(
                            "已显示全部 {} 个首页模块",
                            self.home_modules.enabled.len()
                        );
                    }
                    HomeModuleAction::SetModuleWidth(id, width) => {
                        self.home_modules.set_module_width(*id, *width);
                        self.status = format!(
                            "已将「{}」宽度设为 {}",
                            id.title(),
                            width.label()
                        );
                    }
                    HomeModuleAction::CycleModuleWidth(id) => {
                        let next_w = self.home_modules.cycle_module_width(*id);
                        self.status = format!(
                            "已切换「{}」宽度为 {}",
                            id.title(),
                            next_w.label()
                        );
                    }
                    HomeModuleAction::SetModuleHeight(id, height) => {
                        self.home_modules.set_module_height(*id, *height);
                        self.status = format!(
                            "已将「{}」高度设为 {}",
                            id.title(),
                            height.label()
                        );
                    }
                    HomeModuleAction::CycleModuleHeight(id) => {
                        let next_h = self.home_modules.cycle_module_height(*id);
                        self.status = format!(
                            "已切换「{}」高度为 {}",
                            id.title(),
                            next_h.label()
                        );
                    }
                    HomeModuleAction::SetModuleSize(id, width, height) => {
                        self.home_modules.set_module_size(*id, *width, *height);
                        self.status = format!(
                            "已调整「{}」尺寸为 {} · {}",
                            id.title(),
                            width.label(),
                            height.label()
                        );
                    }
                    HomeModuleAction::MoveUp(id) => {
                        if self.home_modules.move_up(*id) {
                            let rank = self.home_modules.index_of(*id).map(|i| i + 1).unwrap_or(1);
                            self.status = format!("已将「{}」上移至 #{rank}", id.title());
                        }
                    }
                    HomeModuleAction::MoveDown(id) => {
                        if self.home_modules.move_down(*id) {
                            let rank = self.home_modules.index_of(*id).map(|i| i + 1).unwrap_or(1);
                            self.status = format!("已将「{}」下移至 #{rank}", id.title());
                        }
                    }
                    HomeModuleAction::MoveToTop(id) => {
                        if self.home_modules.move_to_top(*id) {
                            self.status = format!("已将「{}」置顶到首页 #1", id.title());
                        }
                    }
                    HomeModuleAction::MoveToBottom(id) => {
                        if self.home_modules.move_to_bottom(*id) {
                            let rank = self.home_modules.enabled.len();
                            self.status = format!("已将「{}」移至首页底部 #{rank}", id.title());
                        }
                    }
                    HomeModuleAction::MoveToIndex(id, target_idx) => {
                        if self.home_modules.move_to_index(*id, *target_idx) {
                            let rank = self.home_modules.index_of(*id).map(|i| i + 1).unwrap_or(1);
                            self.status = format!("已将「{}」调整至 #{rank}", id.title());
                        }
                    }
                    HomeModuleAction::ToggleClickMove(id) => {
                        if self.home_pending_move == Some(*id) {
                            self.home_pending_move = None;
                            self.status = format!("已取消移动模块：「{}」", id.title());
                        } else {
                            self.home_pending_move = Some(*id);
                            self.status = format!(
                                "已选中「{}」· 点击目标模块的「移到此处」即可插入排序",
                                id.title()
                            );
                        }
                    }
                    HomeModuleAction::CancelClickMove => {
                        self.home_pending_move = None;
                        self.status = "已取消模块移动".into();
                    }
                    HomeModuleAction::SwapModules(a, b) => {
                        self.home_pending_move = None;
                        if self.home_modules.swap_modules(*a, *b) {
                            let ra = self.home_modules.index_of(*a).map(|i| i + 1).unwrap_or(1);
                            let rb = self.home_modules.index_of(*b).map(|i| i + 1).unwrap_or(1);
                            self.status = format!(
                                "已将「{}」(#{ra}) 与「{}」(#{rb}) 互换位置",
                                a.title(),
                                b.title()
                            );
                        }
                    }
                    HomeModuleAction::MoveBefore(source, target) => {
                        self.home_pending_move = None;
                        if self.home_modules.move_before(*source, *target) {
                            let r = self
                                .home_modules
                                .index_of(*source)
                                .map(|i| i + 1)
                                .unwrap_or(1);
                            self.status = format!(
                                "已将「{}」移至「{}」之前（位于 #{r}）",
                                source.title(),
                                target.title()
                            );
                        }
                    }
                    HomeModuleAction::BeginDrag(id) => {
                        self.status = format!(
                            "正在拖动「{}」· 移到插入线处松开即可排序",
                            id.title()
                        );
                    }
                    HomeModuleAction::UpdateDrag {
                        source,
                        hover_target,
                    } => {
                        self.status = if let Some(target) = hover_target {
                            format!(
                                "拖动「{}」→ 松开插入「{}」附近",
                                source.title(),
                                target.title()
                            )
                        } else {
                            format!("正在拖拽模块：「{}」", source.title())
                        };
                    }
                    HomeModuleAction::CommitDragSwap(source, target) => {
                        self.home_pending_move = None;
                        self.home_pointer_registry.borrow_mut().active_op = None;
                        if self.home_modules.swap_modules(*source, *target) {
                            let rs = self
                                .home_modules
                                .index_of(*source)
                                .map(|i| i + 1)
                                .unwrap_or(1);
                            let rt = self
                                .home_modules
                                .index_of(*target)
                                .map(|i| i + 1)
                                .unwrap_or(1);
                            self.status = format!(
                                "已拖拽互换「{}」(#{rs}) 与「{}」(#{rt})",
                                source.title(),
                                target.title()
                            );
                        }
                    }
                    HomeModuleAction::CommitDragInsert(source, target, after) => {
                        self.home_pending_move = None;
                        self.home_pointer_registry.borrow_mut().active_op = None;
                        if let (Some(source_index), Some(target_index)) =
                            (self.home_modules.index_of(*source), self.home_modules.index_of(*target))
                        {
                            if source != target {
                                let insertion = target_index - usize::from(source_index < target_index)
                                    + usize::from(*after);
                                self.home_modules.move_to_index(*source, insertion);
                                self.status = format!("「{}」已移到第 {} 位", source.short_title(),
                                    self.home_modules.index_of(*source).unwrap_or(0) + 1);
                            }
                        }
                    }
                    HomeModuleAction::CancelDrag => {
                        self.home_pending_move = None;
                        self.home_pointer_registry.borrow_mut().active_op = None;
                        self.status = "已取消拖拽排布".into();
                    }
                    HomeModuleAction::BeginResize(id) => {
                        self.status = format!("正在拖拽缩放模块：「{}」", id.title());
                    }
                    HomeModuleAction::CommitResize(id, width, height) => {
                        self.home_pointer_registry.borrow_mut().active_op = None;
                        self.home_modules.set_module_size(*id, *width, *height);
                        self.status = format!(
                            "已完成「{}」拖拽缩放：{} · {}",
                            id.title(),
                            width.label(),
                            height.label()
                        );
                    }
                    HomeModuleAction::ApplyPreset(preset) => {
                        self.home_pending_move = None;
                        self.home_pointer_registry.borrow_mut().active_op = None;
                        self.home_modules.apply_preset(*preset);
                        if !self.page_requires_svg() {
                            let _ = self.svg_slots.take_pending();
                        }
                        self.status = format!(
                            "已切换首页模块场景预设：「{}」（共 {} 个模块）",
                            preset.label(),
                            self.home_modules.enabled.len()
                        );
                    }
                    HomeModuleAction::ResetDefault => {
                        self.home_pending_move = None;
                        self.home_pointer_registry.borrow_mut().active_op = None;
                        self.home_modules.reset_default();
                        self.home_module_filter = None;
                        self.status = format!(
                            "已恢复首页默认模块布局（{} 个模块）",
                            self.home_modules.enabled.len()
                        );
                    }
                }
                if !matches!(
                    mod_action,
                    HomeModuleAction::ToggleEditMode
                        | HomeModuleAction::SetEditMode(_)
                        | HomeModuleAction::UndoEdit
                        | HomeModuleAction::CancelEdit
                        | HomeModuleAction::ToggleModuleLibrary
                        | HomeModuleAction::FilterCategory(_)
                        | HomeModuleAction::ToggleClickMove(_)
                        | HomeModuleAction::CancelClickMove
                        | HomeModuleAction::BeginDrag(_)
                        | HomeModuleAction::UpdateDrag { .. }
                        | HomeModuleAction::CancelDrag
                        | HomeModuleAction::BeginResize(_)
                ) {
                    if self.home_module_edit_mode {
                        self.home_pointer_registry.borrow_mut().record_edit(before, &self.home_modules);
                    } else if before != self.home_modules {
                        self.save_home_modules_config();
                    }
                }
            }
            Action::HomeShowHistory => {
                self.page = Page::Amoo;
                self.workspace.sub_view = 0;
                self.workspace.history = true;
                self.workspace.panel = 0;
                self.amoo_ui.screen_mode = false;
                self.scroll = Default::default();
            }
            Action::HomeOpenTaskOutput { task, path } => {
                let output_index = self.workspace.journal.tasks.iter()
                    .find(|candidate| &candidate.id == task && self.workspace_visible(candidate))
                    .and_then(|candidate| candidate.outputs.iter().position(|output| &output.path == path));
                let Some(index) = output_index else {
                    self.error = true;
                    self.status = "这份成果不属于当前记录，请重新打开对话查看".into();
                    return true;
                };
                self.apply_home_action(&Action::HomeResumeTask(task.clone()), ctx.as_deref_mut());
                if !self.error {
                    let still_selected = self.workspace.active.as_ref() == Some(task)
                        && self.workspace.journal.tasks.iter().find(|candidate| &candidate.id == task)
                            .and_then(|candidate| candidate.outputs.get(index))
                            .is_some_and(|output| &output.path == path);
                    if !still_selected {
                        self.error = true;
                        self.status = "任务内容已变化，请重新选择要打开的文件".into();
                        return true;
                    }
                    if let Some(c) = ctx {
                        self.handle_workspace(&crate::ui::workspace_state::WorkspaceAction::Preview(index), c);
                    }
                }
            }
            Action::HomeResumeTask(id) => {
                if let Some(session_id) = id.strip_prefix("phone-amoo:") {
                    let clean_session = session_id
                        .split_once(":topic:")
                        .map(|(base, _)| base)
                        .unwrap_or(session_id)
                        .trim();
                    let snapshot_idx = self.amoo_sessions.iter().position(|s| {
                        s.session_key == clean_session
                    });
                    if let Some(index) = snapshot_idx {
                        let _ = self.workspace_select_phone_session(index);
                        self.page = Page::Amoo;
                        self.workspace.sub_view = 0;
                        self.workspace.history = false;
                        self.workspace.panel = 0;
                        self.amoo_ui.screen_mode = false;
                        self.error = false;
                        self.status = "已接力手机 Amoo 会话，可在电脑继续对话".into();
                        if let Some(c) = ctx {
                            self.handle_ui_action(&Action::AmooSelectSession(index), c);
                            self.workspace_persist();
                        }
                        return true;
                    }
                    let existing_task_id = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| t.session.as_deref() == Some(clean_session))
                        .map(|t| t.id.clone());
                    let task_id = existing_task_id.unwrap_or_else(|| {
                        let target_prefix = format!("phone-amoo:{clean_session}");
                        let ledger_title = self
                            .context
                            .overview
                            .items
                            .iter()
                            .find(|it| {
                                it.id == *id
                                    || it.id == target_prefix
                                    || it.source_objects.iter().any(|so| {
                                        so.id == *id
                                            || so.id == target_prefix
                                            || so
                                                .id
                                                .strip_prefix(&target_prefix)
                                                .is_some_and(|rest| rest.starts_with(":topic:"))
                                    })
                            })
                            .and_then(|it| {
                                let t = it.title.trim();
                                if !t.is_empty() {
                                    Some(t.to_string())
                                } else {
                                    it.source_objects
                                        .iter()
                                        .map(|so| so.title.trim())
                                        .find(|s| !s.is_empty())
                                        .map(ToString::to_string)
                                }
                            })
                            .unwrap_or_else(|| format!("手机 Amoo 会话 {clean_session}"));
                        let mut task = stepx_companion::amoo_workspace::Task::new(
                            self.selected.as_ref().map(|d| d.serial.clone()),
                            self.amoo_ui.account_scope.clone(),
                        );
                        task.title = ledger_title;
                        task.session = Some(clean_session.to_string());
                        task.route_note = "接力自手机 Amoo 会话".into();
                        let tid = task.id.clone();
                        self.workspace.journal.tasks.insert(0, task);
                        tid
                    });

                    let _ = self.workspace_select_task(&task_id);
                    self.page = Page::Amoo;
                    self.workspace.sub_view = 0;
                    self.workspace.history = false;
                    self.workspace.panel = 0;
                    self.amoo_ui.screen_mode = false;
                    self.error = false;
                    self.status = "已接力手机 Amoo 会话，可在电脑继续对话".into();
                    if ctx.is_some() {
                        self.workspace_persist();
                    }
                    return true;
                }
                // Reuse the journal identity, draft and receipts. Opening a task does
                // not resubmit it, including an interrupted phone-delegated task.
                match self.workspace_select_task(id) {
                    Ok(index) => {
                        self.page = Page::Amoo;
                        self.workspace.sub_view = 0;
                        self.workspace.history = false;
                        self.workspace.panel = 0;
                        self.amoo_ui.screen_mode = false;
                        self.error = false;
                        self.status = "已打开原任务，草稿与成果保留；未重新执行".into();
                        if let Some(c) = ctx {
                            if let Some(index) = index {
                                self.handle_ui_action(&Action::AmooSelectSession(index), c);
                            }
                            self.workspace_persist();
                        }
                    }
                    Err(error) => {
                        self.error = true;
                        self.status = format!("无法继续此任务：{error:#}");
                    }
                }
            }
            Action::HomeRefreshAll => {
                if !self.demo_mode && self.selected.is_none() {
                    self.error = true;
                    self.status = "手机未连接，快照与草稿已保留；连接后可刷新首页".into();
                    return true;
                }
                if self.back_screen.stepos_suite_loading
                    && (self.demo_mode || self.busy || self.selected.is_none())
                {
                    self.error = false;
                    self.status = "首页正在同步，请稍候；不会重复发起读取".into();
                    return true;
                }
                if let Some(c) = ctx.as_deref_mut() {
                    if !self.push.syncing {
                        self.handle_push_action(&PushAction::Refresh, c);
                    }
                    if !self.back_screen.stepos_suite_loading {
                        self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, c);
                    }
                }
                self.handle_context_action(&crate::ui::context_state::ContextAction::Refresh);
                if self.selected.is_some() && !self.busy {
                    self.with_device("正在刷新首页消息、日程与手机状态…", |adb, d| {
                        let (summary_res, caps, sms, call_logs, contacts) =
                            std::thread::scope(|s| {
                                let h_sum = s.spawn(|| d.summary(adb));
                                let h_cap =
                                    s.spawn(|| stepx_companion::bridge::capabilities(adb, &d).ok());
                                let h_sms = s.spawn(|| {
                                    crate::ui::outcome_apply::read_result(d.messages(adb))
                                });
                                let h_log = s.spawn(|| {
                                    crate::ui::outcome_apply::read_result(d.call_logs(adb))
                                });
                                let h_con = s.spawn(|| {
                                    crate::ui::outcome_apply::read_result(d.contacts(adb))
                                });
                                (
                                    h_sum.join().unwrap(),
                                    h_cap.join().unwrap(),
                                    h_sms.join().unwrap(),
                                    h_log.join().unwrap(),
                                    h_con.join().unwrap(),
                                )
                            });
                        Ok(crate::ui::Outcome::HomeRefreshed(
                            summary_res?,
                            caps,
                            sms,
                            call_logs,
                            contacts,
                        ))
                    });
                }
                self.error = false;
                self.status = if self.demo_mode { "演示：已刷新首页示例数据" } else { "正在刷新首页消息、日程与手机状态…" }.into();
            }
            Action::HomeSubmitCommand => {
                let cmd = self.fields[F_HOME_COMMAND].text.trim().to_string();
                if cmd.is_empty() {
                    self.focus = Some(F_HOME_COMMAND);
                    if let Some(c) = ctx {
                        self.focus_field(F_HOME_COMMAND, c);
                    }
                    self.status = "请输入要执行的首页生活或工作指令".into();
                } else if self.settings.value.home_agent_enabled {
                    if let Some(c) = ctx {
                        self.run_agent_request(cmd, true, c);
                    }
                } else {
                    let intent = stepx_companion::home_hub::classify_command_intent(&cmd);
                    self.apply_home_action(&Action::HomeExecuteIntent(intent), ctx);
                    if !self.error {
                        self.fields[F_HOME_COMMAND].clear();
                    }
                }
            }
            Action::HomeRunGuiCommand => {
                let raw = self.fields[F_HOME_COMMAND].text.trim().to_string();
                if raw.is_empty() {
                    self.page = Page::Amoo;
                    self.handle_amoo_ui_action(&AmooUiAction::GuiMode(true));
                    self.focus = Some(F_AMOO);
                    if let Some(c) = ctx { self.focus_field(F_AMOO, c); }
                    self.status = "请输入希望手机完成的操作，再点击执行".into();
                } else {
                    self.fields[F_HOME_COMMAND].clear();
                    self.apply_home_action(
                        &Action::HomeExecuteIntent(HomeAiIntent::QuickGuiScenario {
                            app: "手机应用".into(),
                            task: raw,
                        }),
                        ctx,
                    );
                }
            }
            Action::HomeRunWorkCommand => {
                let raw = self.fields[F_HOME_COMMAND].text.trim().to_string();
                // The document entry must open the existing workbench even
                // before a command is entered. Period reviews have their own
                // explicit shortcuts; do not replace a saved document draft.
                if raw.is_empty() {
                    self.page = Page::Work;
                    self.scroll = Default::default();
                    self.focus = Some(F_WORK_PROMPT);
                    if let Some(c) = ctx {
                        self.focus_field(F_WORK_PROMPT, c);
                    }
                    return true;
                }
                let intent = {
                    let classified = stepx_companion::home_hub::classify_command_intent(&raw);
                    match classified {
                        HomeAiIntent::OpenPage(ref route) if route.starts_with("review-") => classified,
                        _ => HomeAiIntent::WorkPrompt(raw),
                    }
                };
                self.fields[F_HOME_COMMAND].clear();
                self.apply_home_action(&Action::HomeExecuteIntent(intent), ctx);
            }
            Action::HomeQuickSmsReply { number, body } => {
                self.fields[F_SMS_TO].set(number);
                self.fields[F_SMS_BODY].set(body);
                if self.demo_mode {
                    self.sms.push(stepx_companion::sms::Message {
                        provider_id: None,
                        address: number.clone(),
                        body: body.clone(),
                        date_ms: chrono::Local::now().timestamp_millis(),
                        outgoing: true,
                        kind: 2,
                        read: true,
                        subscription: self.selected_sim,
                    });
                    self.error = false;
                    self.status = format!("演示：已模拟回复短信至 {number}");
                } else if self.selected.is_none() {
                    self.page = Page::Sms;
                    self.error = true;
                    self.status = "手机未连接，短信未发送；收件人与正文已保留为草稿".into();
                } else {
                    self.handle_device_action(&Action::PrepareDirectSms);
                }
            }
            Action::HomeSelectReplyTarget {
                key,
                thread_id,
                package_name: _,
                title,
            } => {
                self.push.selected_key = Some(key.clone());
                self.push.replying_key = Some(key.clone());
                self.push.selected_im_thread = Some(thread_id.clone());
                self.focus = Some(F_PUSH_REPLY);
                if let Some(c) = ctx {
                    self.focus_field(F_PUSH_REPLY, c);
                }
                self.error = false;
                self.status = format!("已选中「{title}」，可在首页直接输入回复并回车发送");
            }
            Action::HomeToggleChecklistItem(key) => {
                self.ensure_calendar_state_scope();
                if self.back_screen.calendar_state_scope.is_none() {
                    self.error = true;
                    self.status = "请连接原手机后更新日程清单".into();
                    return true;
                }
                let clean_key = key.trim().to_string();
                let parsed_event_id = clean_key
                    .split_once(':')
                    .and_then(|(id_str, _)| id_str.trim().parse::<i64>().ok());
                if let Some(reason) = parsed_event_id.and_then(|id| self.calendar_event_action_limit(id)) {
                    self.status = reason.into();
                    self.error = true;
                    return true;
                }
                let item_label = clean_key
                    .split_once(':')
                    .map(|(_, item)| item)
                    .unwrap_or(&clean_key);
                let checked = if self.home_checked_checklist_items.contains(&clean_key) {
                    self.home_checked_checklist_items.remove(&clean_key);
                    false
                } else {
                    self.home_checked_checklist_items.insert(clean_key.clone());
                    true
                };
                if let Err(error) = self.save_home_checked_checklist_items() {
                    if checked {
                        self.home_checked_checklist_items.remove(&clean_key);
                    } else {
                        self.home_checked_checklist_items.insert(clean_key.clone());
                    }
                    self.error = true;
                    self.status = format!("日程清单未保存，已保留原状态：{error:#}");
                    return true;
                }
                if let Some(ev_id) = parsed_event_id {
                    self.sync_calendar_event_state_to_description(ev_id);
                }
                self.error = false;
                self.status = if checked {
                    format!("已勾选日程清单项：「{item_label}」")
                } else {
                    format!("已恢复待办清单项：「{item_label}」")
                };
            }
            Action::HomeQuickImReply {
                key,
                thread_id,
                package_name,
                title,
                text,
            } => {
                self.push.selected_key = Some(key.clone());
                self.push.replying_key = Some(key.clone());
                self.push.selected_im_thread = Some(thread_id.clone());
                if package_name == "com.android.mms"
                    || thread_id.starts_with("sms:")
                    || key.starts_with("sms:")
                    || package_name.contains("telecom")
                    || package_name.contains("incallui")
                    || package_name.contains("dialer")
                {
                    let number = thread_id
                        .strip_prefix("sms:")
                        .or_else(|| key.strip_prefix("sms:"))
                        .unwrap_or(title.as_str())
                        .trim()
                        .to_string();
                    self.fields[F_PUSH_REPLY].clear();
                    return self.apply_home_action(
                        &Action::HomeQuickSmsReply {
                            number,
                            body: text.clone(),
                        },
                        ctx,
                    );
                }
                self.fields[F_PUSH_REPLY].set(text);
                if let Some(c) = ctx {
                    self.handle_push_action(
                        &PushAction::ReplyInline {
                            key: key.clone(),
                            thread_id: thread_id.clone(),
                            package_name: package_name.clone(),
                            title: title.clone(),
                        },
                        c,
                    );
                } else {
                    self.fields[F_PUSH_REPLY].clear();
                    self.dispatch_im_reply(thread_id, package_name, title, text, Some(key));
                    self.status = format!("已发送回复至「{title}」：{text}");
                }
                self.error = false;
            }
            Action::HomeAiRefresh => {
                self.status = self.start_home_ai_brief();
                self.error = self.home_ai.error.is_some() && !self.home_ai.running();
            }
            Action::HomeAiCancel => {
                self.cancel_home_ai_brief();
                self.error = false;
                self.status = "已取消本次分析显示，正在请求手机停止".into();
            }
            Action::HomeExecuteIntent(intent) => match intent {
                HomeAiIntent::AmooPrompt(p) => {
                    if let Some(c) = ctx {
                        self.handle_ui_action(&Action::SendToAmoo(p.clone()), c);
                    } else {
                        self.page = Page::Amoo;
                        self.workspace.sub_view = 0;
                        self.workspace.panel = 0;
                        self.workspace.proposal = None;
                        self.fields[F_AMOO].set(p);
                        self.focus = Some(F_AMOO);
                        self.status = "已将内容带入 Amoo 对话，按 Enter 即可发送".into();
                        self.error = false;
                    }
                }
                HomeAiIntent::QuickGuiScenario { app, task } => {
                    if let Some(c) = ctx {
                        self.handle_ui_action(
                            &Action::QuickGuiScenario {
                                app: app.clone(),
                                goal: task.clone(),
                            },
                            c,
                        );
                    } else {
                        self.page = Page::Amoo;
                        self.handle_amoo_ui_action(&AmooUiAction::GuiMode(true));
                        self.fields[F_AMOO_GUI_APP].set(app);
                        self.fields[F_AMOO].set(task);
                        self.focus = Some(F_AMOO);
                        self.error = false;
                        self.status = format!("已填入手机操控指令「{task}」，点击执行即可开始");
                    }
                }
                HomeAiIntent::WorkPrompt(p) => {
                    self.fields[F_WORK_PROMPT].set(p);
                    self.page = Page::Work;
                    self.auto_load_for_page();
                    self.focus = Some(F_WORK_PROMPT);
                    self.work.error = None;
                    self.error = false;
                    self.status = format!("已带入文档与办公处理中枢：{p}");
                    if let Some(c) = ctx {
                        self.focus_field(F_WORK_PROMPT, c);
                        if (self.selected.is_some() && !self.demo_mode)
                            || stepx_companion::work::has_local_model_backend()
                        {
                            self.handle_work_action(&WorkAction::Send, c);
                        }
                    }
                }
                HomeAiIntent::BackScreenStatus(s) => {
                    let clean_s = {
                        let stripped = s.trim_start_matches(|c: char| {
                            c.is_whitespace()
                                || (c as u32) >= 0x1F000
                                || matches!(c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                        });
                        if stripped.trim().is_empty() {
                            s.trim().to_string()
                        } else {
                            stripped.trim().to_string()
                        }
                    };
                    self.fields[F_BACK_MARQUEE].set(&clean_s);
                    self.back_screen.stepos_suite.backscreen_signature = clean_s.clone();
                    self.matrix_marquee_on = true;
                    let frame = stepx_companion::backscreen::synthesize_chinese_signature_frame(
                        &clean_s,
                        self.back_screen.config.invert,
                    );
                    self.back_screen.native_feature = None;
                    self.back_screen.source =
                        stepx_companion::backscreen::Source::Image(frame.clone());
                    self.back_screen.frame = frame;
                    self.back_screen.title = format!("背屏状态 · {clean_s}");
                    self.back_screen.revision = self.back_screen.revision.wrapping_add(1);
                    let short_pill: String = clean_s.chars().take(10).collect();
                    self.back_screen.active_phone_capsule = Some((
                        "stepx_status".into(),
                        short_pill.clone(),
                        format!("桌面协同 · {clean_s}"),
                        std::time::Instant::now(),
                    ));
                    self.error = false;
                    self.status = format!("背屏与手机胶囊「{clean_s}」已在电脑预览，尚未应用到手机");
                    if let Some(c) = ctx {
                        if self.selected.is_some() && !self.demo_mode {
                            self.handle_backscreen_action(
                                &BackScreenAction::TriggerMcpQuickAction(format!(
                                    "backscreen_sig:{clean_s}"
                                )),
                                c,
                            );
                            self.handle_backscreen_action(
                                &BackScreenAction::PushPhoneLiveCapsule {
                                    tag: "stepx_status".into(),
                                    pill_text: short_pill,
                                    title: format!("桌面协同 · {clean_s}"),
                                    subtitle: "手机顶部实况胶囊与背屏 17×23 点阵已双向同步".into(),
                                    countdown_secs: None,
                                    sync_backscreen: false,
                                },
                                c,
                            );
                        }
                    }
                }
                HomeAiIntent::QueryExpress(no) => {
                    self.fields[F_OV_TEXT].set(no);
                    self.amoo_express_open = true;
                    if !self.demo_mode && self.selected.is_none() {
                        self.error = true;
                        self.status = "快递查询需要连接手机，尚未执行；单号已保留在输入框".into();
                    } else {
                        self.error = false;
                        self.status = "已打开快递查询，等待手机返回结果".into();
                        if let Some(c) = ctx {
                            self.handle_ui_action(&Action::AmooQueryExpress, c);
                        }
                    }
                }
                HomeAiIntent::AlipayCoupon => {
                    self.error = !self.demo_mode && self.selected.is_none();
                    self.status = if self.demo_mode { "演示：打开支付宝领券入口" } else { "支付宝领券需要连接手机，尚未执行" }.into();
                    if let Some(c) = ctx {
                        if self.selected.is_some() && !self.demo_mode {
                            self.handle_backscreen_action(
                                &BackScreenAction::TriggerMcpQuickAction(
                                    "open_alipay_coupons".into(),
                                ),
                                c,
                            );
                        }
                    }
                }
                HomeAiIntent::CapCutHandoff => {
                    // A generic Home action has no selected media. Continue in the
                    // gallery so an old schedule title cannot become a video request.
                    self.error = false;
                    if let Some(c) = ctx {
                        self.handle_ui_action(&Action::Page(Page::Gallery), c);
                    } else {
                        self.page = Page::Gallery;
                    }
                    self.status = "请在图库选择素材后交接剪映，并在手机确认制作".into();
                }
                HomeAiIntent::PullRemoteAgentFiles => {
                    self.error = !self.demo_mode && self.selected.is_none();
                    self.status = if self.demo_mode { "演示：查看手机智能体产出文件" } else { "取回手机产出需要连接手机，尚未执行" }.into();
                    if let Some(c) = ctx {
                        if self.selected.is_some() && !self.demo_mode {
                            self.handle_backscreen_action(
                                &BackScreenAction::TriggerMcpQuickAction(
                                    "pull_remote_agent_files".into(),
                                ),
                                c,
                            );
                        }
                    }
                }
                HomeAiIntent::StartPomodoro(mins) => {
                    let clamped_mins = (*mins).clamp(1, 60);
                    let dur = clamped_mins * 60;
                    self.back_screen.pomodoro_active = true;
                    self.back_screen.pomodoro_started_at = Some(std::time::Instant::now());
                    self.back_screen.pomodoro_duration_secs = dur;
                    self.back_screen.pomodoro_last_remaining_secs = Some(dur);
                    self.back_screen.title = format!("专注番茄钟 · {clamped_mins} 分钟");
                    self.error = false;
                    self.status = format!("已开启 {clamped_mins} 分钟桌面专注番茄钟");
                    if let Some(c) = ctx {
                        // BackScreenAction treats values > 60 as seconds; always pass the
                        // clamped minute count so "90 分钟" never becomes 90 seconds.
                        self.handle_backscreen_action(
                            &BackScreenAction::StartPomodoro(clamped_mins),
                            c,
                        );
                    }
                }
                HomeAiIntent::ToggleDnd(on) => {
                    if self.demo_mode {
                        self.back_screen.stepos_suite.controls.dnd_on = *on;
                        self.error = false;
                        self.status = if *on { "演示：勿扰已开启" } else { "演示：勿扰已关闭" }.into();
                    } else if self.selected.is_none() {
                        self.error = true;
                        self.status = "手机未连接，勿扰设置尚未修改".into();
                    } else if let Some(c) = ctx {
                        // Set the requested value on the phone and wait for its readback.
                        // Never invert a stale UI snapshot or claim success before receipt.
                        self.handle_backscreen_action(&BackScreenAction::TriggerMcpQuickAction(
                            if *on { "set_dnd_on" } else { "set_dnd_off" }.into()
                        ), c);
                    } else {
                        self.error = true;
                        self.status = "勿扰操作尚未执行，请从首页重试".into();
                    }
                }
                HomeAiIntent::OpenPage(p) => {
                    use stepx_companion::context_ledger::Period;
                    let period = match p.as_str() {
                        "review-day" => Some(Period::Day), "review-week" => Some(Period::Week),
                        "review-month" => Some(Period::Month), _ => None,
                    };
                    let target = if period.is_some() { Page::Home } else { Page::from_arg(p) };
                    if let Some(c) = ctx { self.handle_ui_action(&Action::Page(target), c); }
                    else { self.page = target; self.scroll = Default::default(); self.scroll_aux = Default::default(); }
                    if let Some(period) = period {
                        self.handle_context_action(&crate::ui::context_state::ContextAction::Period(period));
                        self.status = "已打开有来源的周期回顾；可先检查资料，再选择 AI 成稿".into();
                        self.error = false;
                    }
                }
                HomeAiIntent::CreatePhoneAlarm {
                    hour,
                    minute,
                    label,
                } => {
                    let h = (*hour).min(23);
                    let m = (*minute).min(59);
                    let clean_label = if label.trim().is_empty() {
                        "日程前置闹钟".to_string()
                    } else {
                        label.trim().to_string()
                    };
                    self.fields[F_ALARM_TIME].set(&format!("{h:02}:{m:02}"));
                    if let Some(existing) = self
                        .back_screen
                        .stepos_suite
                        .alarms
                        .iter_mut()
                        .find(|a| a.hour == h && a.minutes == m)
                    {
                        existing.enabled = true;
                        if existing.message.trim().is_empty() {
                            existing.message = clean_label.clone();
                        }
                    } else {
                        let next_id = self
                            .back_screen
                            .stepos_suite
                            .alarms
                            .iter()
                            .map(|a| a.id)
                            .max()
                            .unwrap_or(100)
                            + 1;
                        self.back_screen.stepos_suite.alarms.push(
                            stepx_companion::toolbox::PhoneAlarmItem {
                                id: next_id,
                                hour: h,
                                minutes: m,
                                enabled: true,
                                days_of_week: 0,
                                vibrate: true,
                                message: clean_label.clone(),
                            },
                        );
                    }
                    self.back_screen
                        .stepos_suite
                        .alarms
                        .sort_by_key(|a| (a.hour as u16) * 60 + (a.minutes as u16));
                    let msg = format!("已在首页原位设好 {:02}:{:02} 闹钟：「{clean_label}」", h, m);
                    self.back_screen.stepos_suite.last_action_note = msg.clone();
                    self.error = false;
                    self.status = msg;
                    if !self.demo_mode {
                        if let (Some(c), Some(device)) = (ctx, self.selected.clone()) {
                            let serial = device.serial.clone();
                            let label_for_adb = clean_label.clone();
                            let (tx, rx) = async_channel::bounded(1);
                            std::thread::spawn(move || {
                                let res = stepx_companion::adb::Adb::discover().and_then(|adb| {
                                    stepx_companion::toolbox::create_phone_alarm(
                                        &adb,
                                        &device,
                                        h,
                                        m,
                                        &label_for_adb,
                                    )
                                });
                                let _ = tx.send_blocking(res.map_err(|e| format!("{e:#}")));
                            });
                            c.spawn(async move { rx.recv().await }, move |me, result, ctx| {
                                if me.selected.as_ref().map(|d| d.serial.as_str())
                                    != Some(serial.as_str())
                                {
                                    return;
                                }
                                if let Ok(Ok((_id, alarms, note))) = result {
                                    me.apply(crate::ui::Outcome::StepOsAlarmsUpdated(
                                        alarms, note,
                                    ));
                                }
                                ctx.notify();
                            });
                        }
                    }
                }
            },
            Action::WeatherSelectCity(city) => {
                self.error = false;
                let clean_city = city.trim().to_string();
                if !clean_city.is_empty() {
                    if !self.demo_mode && ctx.is_some() {
                        stepx_companion::weather::save_preferred_city(&clean_city);
                    }
                    self.weather_snapshot =
                        stepx_companion::weather::cached_weather_snapshot_for_city_and_device(
                            &clean_city, self.selected.as_ref(),
                        )
                            .unwrap_or_else(|| {
                                stepx_companion::weather::WeatherSnapshot::default_for_city(
                                    &clean_city,
                                )
                            });
                    self.weather_selected_day = 0;
                    self.status = if self.demo_mode {
                        format!("已切换演示天气城市至「{clean_city}」")
                    } else if self.weather_snapshot.from_live_api {
                        format!(
                            "已切换至「{clean_city}」，显示{}缓存",
                            self.weather_snapshot.source_label()
                        )
                    } else {
                        format!("已选择「{clean_city}」，天气实况待更新")
                    };
                    if let Some(c) = ctx.filter(|_| !self.demo_mode) {
                        self.status.push_str("，正在后台更新…");
                        self.queue_live_weather_fetch(&clean_city, true, c);
                    }
                }
            }
            Action::WeatherRefresh => {
                self.error = false;
                if let Some(c) = ctx.filter(|_| !self.demo_mode) {
                    self.status = "正在后台更新天气…".into();
                    self.queue_live_weather_fetch("", true, c);
                } else {
                    self.status = if self.demo_mode {
                        "当前为演示天气，未查询设备或网络。".into()
                    } else {
                        "天气尚未刷新，已保留现有显示，请稍后重试。".into()
                    };
                }
            }
            Action::WeatherSelectDay(day_idx) => {
                self.error = false;
                let max_idx = self.weather_snapshot.daily.len().saturating_sub(1);
                self.weather_selected_day = (*day_idx).min(max_idx);
                if let Some(day) = self
                    .weather_snapshot
                    .daily
                    .get(self.weather_selected_day)
                    .cloned()
                {
                    let slot_desc = format!(
                        "{}（{}）{} {}°~{}°C，{}",
                        day.date_label, day.ymd, day.condition, day.low_c, day.high_c, day.advice
                    );
                    let prompt = self
                        .weather_snapshot
                        .build_weather_context_prompt(Some(&slot_desc), &[]);
                    self.fields[F_HOME_COMMAND].set(&prompt);
                    self.status = format!(
                        "已选中「{} · {}」：{} {}°~{}°C（{}）",
                        self.weather_snapshot.city,
                        day.date_label,
                        day.condition,
                        day.low_c,
                        day.high_c,
                        day.advice
                    );
                }
            }
            Action::WeatherBringToAmoo(focus) => {
                let schedule_titles: Vec<String> = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .filter(|ev| !ev.title.trim().is_empty())
                    .take(4)
                    .map(|ev| format!("{} {}", ev.time_range_label(), ev.title.trim()))
                    .collect();
                let prompt = self
                    .weather_snapshot
                    .build_weather_context_prompt(focus.as_deref(), &schedule_titles);
                self.fields[F_HOME_COMMAND].set(&prompt);
                self.fields[F_AMOO].set(&prompt);
                self.page = Page::Amoo;
                self.workspace.sub_view = 0;
                self.workspace.panel = 0;
                self.workspace.proposal = None;
                self.amoo_ui.screen_mode = false;
                self.focus = Some(F_AMOO);
                if let Some(c) = ctx {
                    self.focus_field(F_AMOO, c);
                }
                self.error = false;
                self.status = format!(
                    "已带入「{} {}°C {}」实时天气上下文，按 Enter 即可发送",
                    self.weather_snapshot.city,
                    self.weather_snapshot.temp_c,
                    self.weather_snapshot.condition
                );
            }
            Action::WeatherBringToHomeInput(focus) => {
                let schedule_titles: Vec<String> = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .filter(|ev| !ev.title.trim().is_empty())
                    .take(4)
                    .map(|ev| format!("{} {}", ev.time_range_label(), ev.title.trim()))
                    .collect();
                let prompt = self
                    .weather_snapshot
                    .build_weather_context_prompt(focus.as_deref(), &schedule_titles);
                self.fields[F_HOME_COMMAND].set(&prompt);
                self.fields[F_AMOO].set(&prompt);
                self.focus = Some(F_HOME_COMMAND);
                if let Some(c) = ctx {
                    self.focus_field(F_HOME_COMMAND, c);
                }
                self.error = false;
                self.status = format!(
                    "已将「{} {}°C {}」天气带入首页指令框",
                    self.weather_snapshot.city,
                    self.weather_snapshot.temp_c,
                    self.weather_snapshot.condition
                );
            }
            Action::WeatherBringToSchedule(focus) => {
                let note = self
                    .weather_snapshot
                    .build_weather_schedule_note(focus.as_deref());
                self.fields[F_PHONE_SCHEDULE_TITLE].set(&note);
                self.fields[F_HOME_COMMAND].set(&note);
                self.page = Page::Calendar;
                self.focus = Some(F_PHONE_SCHEDULE_TITLE);
                if let Some(c) = ctx {
                    self.focus_field(F_PHONE_SCHEDULE_TITLE, c);
                }
                self.error = false;
                self.status = format!("已将天气「{note}」带入日程安排");
            }
            Action::WeatherPushBackScreen => {
                let badge = self.weather_snapshot.backscreen_badge.clone();
                self.apply_home_action(
                    &Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus(badge)),
                    ctx,
                );
            }
            _ => return false,
        }
        true
    }
}

#[cfg(test)]
mod weather_action_tests {
    use super::*;

    #[test]
    fn weather_refresh_without_async_context_preserves_the_last_snapshot() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.weather_snapshot = stepx_companion::weather::WeatherSnapshot::default_for_city("南京");
        app.weather_snapshot.from_live_api = true;
        let before = app.weather_snapshot.clone();
        assert!(app.apply_home_action(&Action::WeatherRefresh, None));
        assert_eq!(app.weather_snapshot, before);
        assert!(app.status.contains("尚未刷新"));
        assert!(!app.status.contains("已更新"));
        assert!(!app.status.contains("已从手机系统天气"));
    }

    #[test]
    fn weather_city_selection_without_context_uses_local_state_and_has_no_success_claim() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        assert!(app.apply_home_action(&Action::WeatherSelectCity("南京".into()), None));
        assert_eq!(app.weather_snapshot.city, "南京");
        assert_eq!(app.weather_selected_day, 0);
        assert!(!app.status.contains("已从手机系统天气"));
        assert!(!app.status.contains("已更新"));
    }
}

