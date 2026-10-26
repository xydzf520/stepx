//! Dedicated Visual Calendar Grid (`Page::Calendar`) and Dedicated Alarm Center (`Page::Alarms`).
//! Provides a full 7×6 interactive month calendar with cross-platform system calendar sync
//! (Linux / Windows / macOS 与手机日历双向同步), selected-day schedule inspector,
//! and a dedicated visual alarm clock page with desktop-to-phone alarm creation & toggles.

use super::super::{
    backscreen_state::BackScreenAction, icons::UiIcon, theme::*, widgets::*,
    workspace_state::WorkspaceAction, Action, Companion, Page, PushAction, F_ALARM_TIME,
    F_CALENDAR_TIME, F_PHONE_SCHEDULE_TITLE,
};
use chrono::{Datelike, Local, NaiveDate};
use facloud_ui::{
    color::ColorU,
    elements::*,
    fonts::{Properties, Weight},
    platform::Cursor,
    text_layout::ClipConfig,
};

fn relative_day_badge(
    year: i32,
    month: u32,
    day: u32,
    today: NaiveDate,
    p: &Palette,
) -> (String, ColorU, ColorU) {
    let Some(target) = NaiveDate::from_ymd_opt(year, month, day) else {
        return ("日期未知".into(), p.text2, p.surface);
    };
    let diff = (target - today).num_days();
    match diff {
        0 => ("今天".into(), p.success_text, p.success_soft),
        1 => ("明天".into(), p.accent, p.accent_soft),
        2 => ("后天".into(), p.blue, p.surface),
        d if d < 0 => ("已过期".into(), p.danger, p.danger_soft),
        d => (format!("{d} 天后"), p.text2, p.surface),
    }
}

fn days_in_year_month(year: i32, month: u32) -> u32 {
    let (next_y, next_m) = if month >= 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let Some(first_of_this) = NaiveDate::from_ymd_opt(year, month, 1) else {
        return 30;
    };
    let Some(first_of_next) = NaiveDate::from_ymd_opt(next_y, next_m, 1) else {
        return 30;
    };
    (first_of_next - first_of_this).num_days().clamp(28, 31) as u32
}

fn truncate_chars(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{head}…")
    } else {
        head
    }
}

impl Companion {
    #[allow(dead_code)]
    pub(super) fn schedule_actions(&self) -> Vec<El> {
        self.calendar_actions()
    }

    #[allow(dead_code)]
    pub(in super::super) fn schedule_page(&self) -> El {
        self.calendar_page()
    }

    pub(super) fn calendar_actions(&self) -> Vec<El> {
        vec![
            self.button(
                "cal-hdr-today",
                "回到今天",
                Action::BackScreen(BackScreenAction::CalendarToday),
                Btn::Secondary,
            ),
            self.button_icon(
                "cal-hdr-open-phone",
                "在手机打开日历",
                UiIcon::Calendar,
                Action::BackScreen(BackScreenAction::McpQuickOpenCalendar),
                Btn::Ghost,
            ),
            if self.back_screen.stepos_suite_loading {
                self.disabled_button("正在同步日历…")
            } else {
                self.button_icon(
                    "cal-hdr-refresh",
                    "同步系统与手机日历",
                    UiIcon::Refresh,
                    Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                    Btn::Primary,
                )
            },
        ]
    }

    pub(super) fn calendar_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let today = Local::now().date_naive();

        let year = if self.back_screen.calendar_year >= 1970 {
            self.back_screen.calendar_year
        } else {
            today.year()
        };
        let month = if (1..=12).contains(&self.back_screen.calendar_month) {
            self.back_screen.calendar_month
        } else {
            today.month()
        };
        let days_in_month = days_in_year_month(year, month);
        let selected_day = self
            .back_screen
            .selected_calendar_day
            .clamp(1, days_in_month);

        let visible_calendar_events: Vec<&stepx_companion::toolbox::PhoneCalendarEvent> = suite
            .calendar_events
            .iter()
            .filter(|ev| {
                !ev.title.contains("StepX 日历网格验证") && !ev.title.contains("StepX E2E")
            })
            .collect();

        let mut by_day: [Vec<&stepx_companion::toolbox::PhoneCalendarEvent>; 32] =
            std::array::from_fn(|_| Vec::new());
        let mut month_event_count = 0usize;
        for ev in &visible_calendar_events {
            let (ey, em, ed) = ev.date_ymd();
            if ey == year && em == month && (1..=31).contains(&ed) {
                month_event_count += 1;
                by_day[ed as usize].push(*ev);
            }
        }
        let selected_day_events = &by_day[selected_day as usize];

        // 1. Top Sync & Status Banner
        let cal_note = if !suite.last_action_note.is_empty()
            && (suite.last_action_note.contains("日历") || suite.last_action_note.contains("日程"))
            && !suite.last_action_note.contains("屏幕使用")
            && !suite.last_action_note.contains("闹钟")
        {
            suite.last_action_note.clone()
        } else {
            format!(
                "当前选中 {year}-{month:02}-{selected_day:02} · 当日 {} 项日程，本月共 {} 项日程",
                selected_day_events.len(),
                month_event_count
            )
        };
        let banner_left = vec![
            self.hstack(
                S2,
                vec![
                    self.heading("双端日历工作台（电脑系统日历与手机日历双向同步）"),
                    self.pill_badge(
                        format!("共 {} 项双端日程", visible_calendar_events.len()),
                        p.accent,
                        p.accent_soft,
                    ),
                ],
            ),
            self.text(
                "默认自动读取当前电脑系统日历（Linux Evolution/GNOME/KDE · macOS Calendar · Windows Outlook/系统日历）并与手机日历双向同步，也可在右侧或设置中随时关闭。",
                CAPTION,
                p.text2,
            ),
            self.line(cal_note, MICRO, p.text3),
        ];

        let top_banner = self.panel(vec![self.between(
            self.vstack(S1, banner_left),
            self.hstack(
                S2,
                vec![
                    self.button(
                        "cal-banner-autosync-toggle",
                        if self.prefs.auto_sync_calendar {
                            "自动同步系统日历：已开启"
                        } else {
                            "自动同步系统日历：已关闭"
                        },
                        Action::BackScreen(BackScreenAction::ToggleAutoSyncCalendar),
                        if self.prefs.auto_sync_calendar {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                    self.button(
                        "cal-banner-goto-settings",
                        "前往设置",
                        Action::Page(Page::Settings),
                        Btn::Ghost,
                    ),
                ],
            ),
        )]);

        // 2. Left Column: Visual 7×6 Month Calendar Grid
        let month_nav_bar = self.between(
            self.hstack(
                S2,
                vec![
                    self.heading(format!("{year} 年 {month:02} 月")),
                    self.pill_badge(
                        format!("本月 {month_event_count} 项日程"),
                        p.accent,
                        p.accent_soft,
                    ),
                ],
            ),
            self.hstack(
                S2,
                vec![
                    self.button_icon(
                        "cal-nav-prev",
                        "上个月",
                        UiIcon::ArrowLeft,
                        Action::BackScreen(BackScreenAction::CalendarPrevMonth),
                        Btn::Secondary,
                    ),
                    self.button(
                        "cal-nav-today",
                        "今天",
                        Action::BackScreen(BackScreenAction::CalendarToday),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "cal-nav-next",
                        "下个月",
                        UiIcon::ChevronRight,
                        Action::BackScreen(BackScreenAction::CalendarNextMonth),
                        Btn::Secondary,
                    ),
                ],
            ),
        );

        let weekday_labels = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"];
        let mut weekday_header_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(6.);
        for (idx, label) in weekday_labels.iter().enumerate() {
            let cell = Container::new(
                Align::new(self.line_w(
                    *label,
                    CAPTION,
                    if idx >= 5 { p.text3 } else { p.text2 },
                    Weight::Semibold,
                ))
                .finish(),
            )
            .with_vertical_padding(6.)
            .with_background_color(p.surface)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish();
            weekday_header_row = weekday_header_row.with_child(Expanded::new(1., cell).finish());
        }

        let first_day = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(today);
        let leading_empty = first_day.weekday().num_days_from_monday() as i32;

        let mut week_rows: Vec<El> = Vec::with_capacity(6);
        for week in 0..6 {
            let mut row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(6.);

            for dow in 0..7 {
                let cell_idx = week * 7 + dow;
                let day_num = cell_idx - leading_empty + 1;

                let cell_el: El = if day_num < 1 || day_num > days_in_month as i32 {
                    ConstrainedBox::new(
                        Container::new(Empty::new().finish())
                            .with_background_color(p.workspace)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish(),
                    )
                    .with_height(78.)
                    .finish()
                } else {
                    let d = day_num as u32;
                    let is_today =
                        year == today.year() && month == today.month() && d == today.day();
                    let is_selected = d == selected_day;

                    let day_events = &by_day[d as usize];
                    let event_count = day_events.len();
                    let previews: Vec<(i64, String, bool)> = day_events
                        .iter()
                        .take(2)
                        .map(|ev| {
                            let label =
                                truncate_chars(&format!("{} {}", ev.hm_label(), ev.title), 10);
                            let is_sys = ev.source.contains("system")
                                || ev.source.contains("desktop")
                                || ev.source.contains("local")
                                || ev.source.contains("电脑")
                                || ev.source.contains("双端");
                            (ev.id, label, is_sys)
                        })
                        .collect();

                    let state = self.mouse_state(format!("cal-cell:{year}:{month}:{d}"));
                    let font = self.font;

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
                        let blend = |c1: ColorU, c2: ColorU| {
                            ColorU::new(
                                mix(c1.r, c2.r),
                                mix(c1.g, c2.g),
                                mix(c1.b, c2.b),
                                mix(c1.a, c2.a),
                            )
                        };
                        let bg = if mouse.is_clicked() || is_selected {
                            p.accent_soft
                        } else {
                            blend(p.surface, p.hover)
                        };
                        let border_color = if mouse.is_clicked() || is_selected {
                            p.accent
                        } else if is_today {
                            p.blue
                        } else {
                            blend(p.border, p.border_strong)
                        };
                        let border_w = if mouse.is_clicked() || is_selected || is_today {
                            1.5
                        } else {
                            1.0
                        };

                        let day_num_el = Text::new(format!("{d}"), font, BODY)
                            .with_color(if is_selected {
                                p.accent
                            } else if is_today {
                                p.blue
                            } else {
                                p.text
                            })
                            .with_style(Properties::default().weight(if is_selected || is_today {
                                Weight::Bold
                            } else {
                                Weight::Semibold
                            }))
                            .soft_wrap(false)
                            .finish();

                        let badge_el: El = if is_today {
                            Container::new(
                                Text::new("今".to_string(), font, MICRO)
                                    .with_color(p.on_accent)
                                    .with_style(Properties::default().weight(Weight::Semibold))
                                    .soft_wrap(false)
                                    .finish(),
                            )
                            .with_horizontal_padding(5.)
                            .with_vertical_padding(1.)
                            .with_background_color(p.blue)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                            .finish()
                        } else if event_count > 0 {
                            Container::new(
                                Text::new(format!("{event_count}项"), font, MICRO)
                                    .with_color(p.accent)
                                    .with_style(Properties::default().weight(Weight::Medium))
                                    .soft_wrap(false)
                                    .finish(),
                            )
                            .with_horizontal_padding(5.)
                            .with_vertical_padding(1.)
                            .with_background_color(if is_selected { p.bg } else { p.accent_soft })
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                            .finish()
                        } else {
                            Empty::new().finish()
                        };

                        let top_row = Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(day_num_el)
                            .with_child(badge_el)
                            .finish();

                        let mut cell_items: Vec<El> = vec![top_row];
                        for &(ev_id, ref preview_text, is_sys) in &previews {
                            let pill_state =
                                self.mouse_state(format!("cal-cell-ev:{year}:{month}:{d}:{ev_id}"));
                            let pill_text = preview_text.clone();
                            let pill = Hoverable::new(pill_state, move |pill_mouse| {
                                let pill_hovered = pill_mouse.is_mouse_over_element();
                                let pill_bg = if pill_mouse.is_clicked() || pill_hovered {
                                    p.hover
                                } else if is_selected {
                                    p.bg
                                } else {
                                    p.accent_soft
                                };
                                Container::new(
                                    Text::new(pill_text.clone(), font, MICRO)
                                        .with_color(if is_sys { p.blue } else { p.text })
                                        .soft_wrap(false)
                                        .with_clip(ClipConfig::ellipsis())
                                        .finish(),
                                )
                                .with_horizontal_padding(4.)
                                .with_vertical_padding(1.)
                                .with_background_color(pill_bg)
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                                .finish()
                            })
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::BackScreen(
                                    BackScreenAction::CalendarSelectDate(year, month, d),
                                ));
                            })
                            .on_double_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::BackScreen(
                                    BackScreenAction::CalendarEditEvent(ev_id),
                                ));
                            })
                            .on_right_click(move |ctx, _, at| {
                                ctx.dispatch_typed_action(Action::BackScreen(
                                    BackScreenAction::CalendarSelectDate(year, month, d),
                                ));
                                ctx.dispatch_typed_action(Action::ContextOpen(
                                    crate::ui::context_menu::Target::ScheduleEvent(ev_id),
                                    at,
                                ));
                            })
                            .finish();
                            cell_items.push(pill);
                        }

                        let inner = Flex::column()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                            .with_spacing(3.)
                            .with_children(cell_items)
                            .finish();

                        ConstrainedBox::new(
                            Container::new(inner)
                                .with_uniform_padding(6.)
                                .with_background_color(bg)
                                .with_border(Border::all(border_w).with_border_color(border_color))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish(),
                        )
                        .with_height(78.)
                        .finish()
                    })
                    .with_defer_events_to_children()
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(
                            BackScreenAction::CalendarSelectDate(year, month, d),
                        ))
                    })
                    .on_double_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(
                            BackScreenAction::CalendarStartCreateOnDate(year, month, d),
                        ));
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::BackScreen(
                            BackScreenAction::CalendarSelectDate(year, month, d),
                        ));
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::CalendarDate {
                                year,
                                month,
                                day: d,
                            },
                            at,
                        ));
                    })
                    .finish()
                };

                row = row.with_child(Expanded::new(1., cell_el).finish());
            }

            week_rows.push(row.finish());
        }

        let grid_footer = self.between(
            self.hstack(
                S2,
                vec![
                    self.pill_badge("今 · 今日日期", p.blue, p.surface),
                    self.pill_badge(
                        format!("当前选中：{year}-{month:02}-{selected_day:02}"),
                        p.accent,
                        p.accent_soft,
                    ),
                ],
            ),
            self.line(
                "左键选日期 · 双击新建 · 右键日期或日程直接添加/修改/删除",
                CAPTION,
                p.text3,
            ),
        );

        let left_calendar_card = self.panel(vec![
            month_nav_bar,
            weekday_header_row.finish(),
            self.vstack(6., week_rows),
            grid_footer,
        ]);

        // 3. Right Column: Selected Day Inspector & Event Creator
        // Card 1: Selected Date Header & Create Event Form
        let (sel_rel_label, sel_rel_fg, sel_rel_bg) =
            relative_day_badge(year, month, selected_day, today, &p);
        let is_editing = self.back_screen.editing_calendar_event_id.is_some();
        let current_cal_time = self.fields[F_CALENDAR_TIME].text.trim();
        let cal_time_presets: [(&'static str, &'static str); 6] = [
            ("09:00 晨会", "09:00-09:30"),
            ("10:00 评审", "10:00-11:00"),
            ("14:00 下午", "14:00-15:00"),
            ("16:00 沟通", "16:00-17:00"),
            ("19:30 晚间", "19:30-20:30"),
            ("全天日程", "09:00-18:00"),
        ];
        let mut cal_time_preset_chips: Vec<El> = vec![self.line("快捷时段：", MICRO, p.text2)];
        for (idx, (chip_label, time_val)) in cal_time_presets.into_iter().enumerate() {
            let active_chip = current_cal_time == time_val;
            cal_time_preset_chips.push(self.button(
                format!("cal-preset-chip-{idx}"),
                chip_label,
                Action::Push(PushAction::SelectCalendarQuickTime(time_val.to_string())),
                if active_chip {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ));
        }

        let scenario_templates: [(UiIcon, &'static str, &'static str, &'static str); 5] = [
            (
                UiIcon::Calendar,
                "15分钟后开会",
                "临时会议讨论",
                "10:00-10:30",
            ),
            (
                UiIcon::Package,
                "下班取快递",
                "下班驿站取快递",
                "18:30-19:00",
            ),
            (
                UiIcon::Activity,
                "今晚 20:00 运动",
                "晚间运动健身",
                "20:00-21:00",
            ),
            (
                UiIcon::Heart,
                "每日吃药提醒",
                "按时服药与饮水提醒",
                "21:30-21:45",
            ),
            (
                UiIcon::Gift,
                "纪念日/生日",
                "亲友生日与纪念日祝福",
                "09:00-18:00",
            ),
        ];
        let mut scenario_chips: Vec<El> = vec![self.line("场景模板：", MICRO, p.text2)];
        for (idx, (icon, btn_label, tpl_title, tpl_time)) in
            scenario_templates.into_iter().enumerate()
        {
            let is_active_tpl = self.fields[F_PHONE_SCHEDULE_TITLE].text.trim() == tpl_title;
            let state = self.mouse_state(format!("cal-scen-tpl:{idx}"));
            let font = self.font;
            let title_str = tpl_title.to_string();
            let time_str = tpl_time.to_string();
            let label_str = btn_label.to_string();
            scenario_chips.push(
                Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let bg = if mouse.is_clicked() || is_active_tpl {
                        p.accent_soft
                    } else if hovered {
                        p.hover
                    } else {
                        p.surface
                    };
                    let border = if is_active_tpl {
                        p.accent
                    } else if hovered {
                        p.border_strong
                    } else {
                        p.border
                    };
                    Container::new(self.hstack(
                        5.,
                        vec![
                            self.ui_icon(
                                icon,
                                13.,
                                if is_active_tpl { p.accent } else { p.text2 },
                            ),
                            Text::new(label_str.clone(), font, MICRO)
                                .with_color(p.text)
                                .with_style(Properties::default().weight(if is_active_tpl {
                                    Weight::Semibold
                                } else {
                                    Weight::Medium
                                }))
                                .soft_wrap(false)
                                .finish(),
                        ],
                    ))
                    .with_horizontal_padding(10.)
                    .with_vertical_padding(4.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Focus(F_PHONE_SCHEDULE_TITLE));
                    ctx.dispatch_typed_action(Action::ClearField(F_PHONE_SCHEDULE_TITLE));
                    ctx.dispatch_typed_action(Action::Insert(title_str.clone()));
                    ctx.dispatch_typed_action(Action::Push(PushAction::SelectCalendarQuickTime(
                        time_str.clone(),
                    )));
                })
                .finish(),
            );
        }

        let mut create_card_items: Vec<El> = vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.heading(format!("{year}年{month:02}月{selected_day:02}日 日程安排")),
                        self.pill_badge(sel_rel_label, sel_rel_fg, sel_rel_bg),
                    ],
                ),
                self.pill_badge(
                    format!("当日 {} 项", selected_day_events.len()),
                    p.accent,
                    p.accent_soft,
                ),
            ),
            self.text(
                "点击左侧日历任意日期即可切换选中日；在下方输入日程标题与时间（或点击场景模板/快捷时段），一键写入手机日历并与桌面同步。",
                CAPTION,
                p.text2,
            ),
        ];

        if is_editing {
            create_card_items.push(
                Container::new(self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.ui_icon(UiIcon::Edit, 14., p.warning),
                            self.line_w(
                                "正在修改日程（保存后将直接更新原日程）",
                                CAPTION,
                                p.text,
                                Weight::Semibold,
                            ),
                        ],
                    ),
                    self.button(
                        "cal-edit-banner-cancel",
                        "取消编辑",
                        Action::BackScreen(BackScreenAction::CalendarCancelEdit),
                        Btn::Secondary,
                    ),
                ))
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .with_background_color(p.warning_soft)
                .with_border(Border::all(1.).with_border_color(p.warning))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        }

        create_card_items.push(self.input(
            F_PHONE_SCHEDULE_TITLE,
            "输入日程标题（如：项目评审会 / 航班出行）…",
            None,
        ));
        create_card_items.push(self.input(
            F_CALENDAR_TIME,
            "输入日程时间（如 09:30 或 14:00-15:30，留空默认 09:30）…",
            None,
        ));
        create_card_items.push(self.wrap(scenario_chips));
        create_card_items.push(self.wrap(cal_time_preset_chips));

        let mut submit_btns = vec![self.button_icon(
            "cal-create-selected-custom",
            if is_editing {
                "保存日程修改"
            } else {
                "在选中日期添加日程"
            },
            if is_editing {
                UiIcon::Check
            } else {
                UiIcon::Plus
            },
            Action::BackScreen(BackScreenAction::CalendarCreateOnSelectedDate(None)),
            Btn::Primary,
        )];
        if is_editing {
            submit_btns.push(self.button(
                "cal-cancel-edit-btn",
                "取消编辑",
                Action::BackScreen(BackScreenAction::CalendarCancelEdit),
                Btn::Secondary,
            ));
        } else {
            submit_btns.push(self.button_icon(
                "cal-create-preset-1h",
                "1 小时后快速日程",
                UiIcon::Plus,
                Action::BackScreen(BackScreenAction::CalendarCreatePreset(0)),
                Btn::Secondary,
            ));
            submit_btns.push(self.button_icon(
                "cal-create-preset-18",
                "今晚 18:00 复盘",
                UiIcon::Plus,
                Action::BackScreen(BackScreenAction::CalendarCreatePreset(1)),
                Btn::Secondary,
            ));
            submit_btns.push(self.button_icon(
                "cal-create-preset-tmr",
                "明早 09:30 晨会",
                UiIcon::Plus,
                Action::BackScreen(BackScreenAction::CalendarCreatePreset(2)),
                Btn::Secondary,
            ));
        }
        create_card_items.push(self.wrap(submit_btns));

        let create_event_card = self.panel(create_card_items);

        // Card 2: Selected Date Events & Upcoming Agenda
        let mut selected_day_hdr_right = vec![self.pill_badge(
            format!("{} 项", selected_day_events.len()),
            p.accent,
            p.surface,
        )];
        if !selected_day_events.is_empty() {
            selected_day_hdr_right.push(self.button(
                "cal-clear-selected-day",
                "清空当日",
                Action::BackScreen(BackScreenAction::CalendarClearDay(
                    year,
                    month,
                    selected_day,
                )),
                Btn::Ghost,
            ));
        }
        let mut agenda_rows: Vec<El> = vec![self.between(
            self.heading(format!("{month:02}月{selected_day:02}日 当日日程明细")),
            self.hstack(S2, selected_day_hdr_right),
        )];

        if selected_day_events.is_empty() {
            agenda_rows.push(
                Container::new(self.text(
                    "选中日期暂无日程安排。可在上方填写标题后点击「在选中日期添加日程」或快捷时间按钮创建。",
                    BODY,
                    p.text2,
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        } else {
            for event in selected_day_events {
                agenda_rows.push(self.render_calendar_event_row(event, "sel"));
            }
        }

        agenda_rows.push(self.divider());
        agenda_rows.push(self.between(
            self.heading("近期全部日程"),
            self.pill_badge(
                format!("共 {} 项", visible_calendar_events.len()),
                p.text2,
                p.surface,
            ),
        ));

        if visible_calendar_events.is_empty() {
            agenda_rows.push(
                Container::new(self.text(
                    if suite.checked {
                        "当前暂无近期日程。系统日历（Evolution / GNOME / KDE / macOS / Windows）与手机日历中的新日程会自动汇总于此。"
                    } else {
                        "正在读取电脑系统日历与手机日历日程…"
                    },
                    BODY,
                    p.text2,
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        } else {
            for event in visible_calendar_events.iter().take(8) {
                agenda_rows.push(self.render_calendar_event_row(event, "all"));
            }
        }

        let agenda_card = self.panel(agenda_rows);
        let right_col = self.vstack(S4, vec![create_event_card, agenda_card]);
        let right_col = super::super::motion::enter(
            right_col,
            self.motion_started,
            !self.settings.value.reduced_motion && !self.settings.hidden,
        );

        let main_split = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S4)
            .with_child(Expanded::new(1.35, left_calendar_card).finish())
            .with_child(Expanded::new(1.0, right_col).finish())
            .finish();

        Container::new(self.vstack(S4, vec![top_banner, main_split]))
            .with_padding_bottom(S2)
            .finish()
    }

    fn render_calendar_event_row(
        &self,
        event: &stepx_companion::toolbox::PhoneCalendarEvent,
        prefix: &str,
    ) -> El {
        let p = self.p();
        let time_badge = event.time_range_label();
        let src_label = event.source_label();
        let (ey, em, ed) = event.date_ymd();
        let today = Local::now().date_naive();
        let (rel_label, rel_fg, rel_bg) = relative_day_badge(ey, em, ed, today, &p);
        let is_done = self
            .back_screen
            .completed_calendar_events
            .contains(&event.id);

        let mut header_badges = vec![
            self.line_w(&event.title, BODY, p.text, Weight::Semibold),
            self.pill_badge(rel_label, rel_fg, rel_bg),
            self.pill_badge(&time_badge, p.accent, p.accent_soft),
            self.pill_badge(src_label, p.blue, p.surface),
        ];
        if is_done {
            header_badges.push(self.pill_badge_icon(
                "已完成",
                UiIcon::Check,
                p.success_text,
                p.success_soft,
            ));
        }

        let mut detail_col = vec![self.wrap(header_badges)];

        if !event.location.is_empty() || !event.description.is_empty() {
            let sub = if !event.location.is_empty() && !event.description.is_empty() {
                format!("地点：{} · {}", event.location, event.description)
            } else if !event.location.is_empty() {
                format!("地点：{}", event.location)
            } else {
                event.description.clone()
            };
            detail_col.push(self.text(sub, CAPTION, p.text2));
        }

        let summary_text = if !event.location.is_empty() && !event.description.is_empty() {
            format!(
                "【日程】{} ({}) · 地点：{} · 备注：{}",
                event.title, time_badge, event.location, event.description
            )
        } else if !event.location.is_empty() {
            format!(
                "【日程】{} ({}) · 地点：{}",
                event.title, time_badge, event.location
            )
        } else if !event.description.is_empty() {
            format!(
                "【日程】{} ({}) · {}",
                event.title, time_badge, event.description
            )
        } else {
            format!("【日程】{} ({})", event.title, time_badge)
        };

        let is_editing_this = self.back_screen.editing_calendar_event_id == Some(event.id);
        let mut actions = vec![
            self.button_icon(
                format!("cal-{prefix}-done-{}", event.id),
                if is_done { "设为未完" } else { "完成" },
                if is_done {
                    UiIcon::Retry
                } else {
                    UiIcon::Check
                },
                Action::BackScreen(BackScreenAction::CalendarToggleDone(event.id)),
                if is_done {
                    Btn::Secondary
                } else {
                    Btn::Primary
                },
            ),
            self.button_icon(
                format!("cal-{prefix}-edit-{}", event.id),
                if is_editing_this {
                    "编辑中"
                } else {
                    "编辑"
                },
                UiIcon::Edit,
                Action::BackScreen(BackScreenAction::CalendarEditEvent(event.id)),
                if is_editing_this {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            ),
            self.button(
                format!("cal-{prefix}-postpone-{}", event.id),
                "顺延1天",
                Action::BackScreen(BackScreenAction::CalendarPostponeOneDay(event.id)),
                Btn::Secondary,
            ),
            self.button_icon(
                format!("cal-{prefix}-copy-{}", event.id),
                "复制日程摘要",
                UiIcon::Clipboard,
                Action::CopyText(summary_text, format!("已复制日程摘要：{}", event.title)),
                Btn::Secondary,
            ),
            self.button(
                format!("cal-{prefix}-amoo-{}", event.id),
                "交给 Amoo",
                Action::Workspace(WorkspaceAction::SendScheduleToAmoo(format!(
                    "{}（{}）",
                    event.title, time_badge
                ))),
                Btn::Secondary,
            ),
            self.button(
                format!("cal-{prefix}-del-{}", event.id),
                "删除",
                Action::BackScreen(BackScreenAction::CalendarDelete(event.id)),
                Btn::Ghost,
            ),
        ];
        if prefix == "all" {
            actions.insert(
                0,
                self.button(
                    format!("cal-{prefix}-jump-{}", event.id),
                    "定位",
                    Action::BackScreen(BackScreenAction::CalendarSelectDate(ey, em, ed)),
                    Btn::Ghost,
                ),
            );
        }

        let row_body = self.vstack(S2, vec![self.vstack(S1, detail_col), self.wrap(actions)]);
        let state = self.mouse_state(format!("cal-ev-row:{prefix}:{}", event.id));
        let event_id = event.id;
        Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let border_col = if mouse.is_clicked() {
                p.accent
            } else if hovered {
                p.border_strong
            } else {
                p.border
            };
            Container::new(row_body)
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(border_col))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_defer_events_to_children()
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::CalendarSelectDate(
                ey, em, ed,
            )));
        })
        .on_double_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::CalendarToggleDone(
                event_id,
            )));
        })
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::ScheduleEvent(event_id),
                at,
            ));
        })
        .finish()
    }

    pub(super) fn alarms_actions(&self) -> Vec<El> {
        vec![
            self.button_icon(
                "alarms-hdr-open-phone",
                "在手机打开时钟",
                UiIcon::Clock,
                Action::BackScreen(BackScreenAction::McpQuickOpenAlarm),
                Btn::Ghost,
            ),
            if self.back_screen.stepos_suite_loading {
                self.disabled_button("正在同步闹钟…")
            } else {
                self.button_icon(
                    "alarms-hdr-refresh",
                    "刷新手机闹钟",
                    UiIcon::Refresh,
                    Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                    Btn::Primary,
                )
            },
        ]
    }

    pub(super) fn alarms_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let enabled_alarms = suite.alarms.iter().filter(|a| a.enabled).count();

        let now = Local::now();
        let now_hm = now.format("%H:%M").to_string();
        let weekday_cn = match now.weekday().num_days_from_monday() {
            0 => "星期一",
            1 => "星期二",
            2 => "星期三",
            3 => "星期四",
            4 => "星期五",
            5 => "星期六",
            _ => "星期日",
        };
        let now_date = format!("{} · {}", now.format("%Y年%m月%d日"), weekday_cn);

        // 1. Top Digital Clock & Alarm Summary Banner
        let clock_display = Container::new(self.mono(now_hm, 26., p.text))
            .with_horizontal_padding(S4)
            .with_vertical_padding(S2)
            .with_background_color(p.surface)
            .with_border(Border::all(1.5).with_border_color(p.accent))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();

        let alarm_note = if !suite.last_action_note.is_empty()
            && suite.last_action_note.contains("闹钟")
            && !suite.last_action_note.contains("屏幕使用")
            && !suite.last_action_note.contains("手机日历")
            && !suite.last_action_note.contains("双端日历")
        {
            suite.last_action_note.clone()
        } else {
            format!(
                "桌面与手机系统时钟实时同步 · 当前已开启 {} 个闹钟",
                enabled_alarms
            )
        };
        let banner_info = vec![
            self.heading("手机与桌面闹钟中心"),
            self.line(now_date, BODY, p.text2),
            self.line(alarm_note, CAPTION, p.text3),
        ];

        let top_banner = self.panel(vec![self.between(
            self.hstack(S4, vec![clock_display, self.vstack(S1, banner_info)]),
            self.hstack(
                S2,
                vec![
                    self.pill_badge(
                        format!("{enabled_alarms}/{} 个闹钟已开启", suite.alarms.len()),
                        if enabled_alarms > 0 {
                            p.success
                        } else {
                            p.text2
                        },
                        p.surface,
                    ),
                    self.pill_badge(
                        if suite.controls.dnd_on {
                            "手机勿扰：已开启"
                        } else {
                            "手机铃声：正常"
                        },
                        if suite.controls.dnd_on {
                            p.warning
                        } else {
                            p.success
                        },
                        p.surface,
                    ),
                ],
            ),
        )]);

        // 2. Left Column: Visual Alarm Creator & Pomodoro Focus
        let quick_times: [&'static str; 8] = [
            "06:30", "07:00", "07:30", "08:00", "08:30", "09:00", "12:30", "22:00",
        ];
        let current_alarm_time = self.fields[F_ALARM_TIME].text.trim();
        let quick_time_chips: Vec<El> = quick_times
            .iter()
            .map(|&qt| {
                let selected = current_alarm_time == qt;
                self.button(
                    format!("alarm-qt-{qt}"),
                    qt,
                    Action::BackScreen(BackScreenAction::AlarmSelectQuickTime(qt)),
                    if selected {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                )
            })
            .collect();

        let alarm_scenario_presets: [(UiIcon, &'static str, &'static str, &'static str); 5] = [
            (UiIcon::Coffee, "15分钟小憩", "+15m", "15分钟午休小憩提醒"),
            (
                UiIcon::Clock,
                "25分钟番茄钟",
                "+25m",
                "25分钟专注番茄钟提醒",
            ),
            (UiIcon::Coffee, "5分钟泡面", "+5m", "5分钟泡面/倒计时提醒"),
            (
                UiIcon::Clock,
                "明早 07:30 起床",
                "07:30",
                "早间起床与日程提醒",
            ),
            (
                UiIcon::Clock,
                "明早 08:00 早会",
                "08:00",
                "晨会与重点任务提醒",
            ),
        ];
        let mut alarm_scenario_chips: Vec<El> = vec![self.line("场景速设：", MICRO, p.text2)];
        for (idx, (icon, btn_label, qt_str, tpl_label)) in
            alarm_scenario_presets.into_iter().enumerate()
        {
            let active_scen = current_alarm_time == qt_str
                && self.fields[F_PHONE_SCHEDULE_TITLE].text.trim() == tpl_label;
            let state = self.mouse_state(format!("alarm-scen-chip:{idx}"));
            let font = self.font;
            let label_str = btn_label.to_string();
            let note_str = tpl_label.to_string();
            alarm_scenario_chips.push(
                Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let bg = if mouse.is_clicked() || active_scen {
                        p.accent_soft
                    } else if hovered {
                        p.hover
                    } else {
                        p.surface
                    };
                    let border = if active_scen {
                        p.accent
                    } else if hovered {
                        p.border_strong
                    } else {
                        p.border
                    };
                    Container::new(self.hstack(
                        5.,
                        vec![
                            self.ui_icon(
                                icon,
                                13.,
                                if active_scen { p.accent } else { p.text2 },
                            ),
                            Text::new(label_str.clone(), font, MICRO)
                                .with_color(p.text)
                                .with_style(Properties::default().weight(if active_scen {
                                    Weight::Semibold
                                } else {
                                    Weight::Medium
                                }))
                                .soft_wrap(false)
                                .finish(),
                        ],
                    ))
                    .with_horizontal_padding(10.)
                    .with_vertical_padding(4.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::BackScreen(
                        BackScreenAction::AlarmSelectQuickTime(qt_str),
                    ));
                    ctx.dispatch_typed_action(Action::Focus(F_PHONE_SCHEDULE_TITLE));
                    ctx.dispatch_typed_action(Action::ClearField(F_PHONE_SCHEDULE_TITLE));
                    ctx.dispatch_typed_action(Action::Insert(note_str.clone()));
                })
                .finish(),
            );
        }

        let is_editing_alarm = self.back_screen.editing_alarm_id.is_some();
        let mut create_alarm_items: Vec<El> = vec![
            self.between(
                self.heading("新建闹钟（实时同步至手机系统时钟）"),
                self.pill_badge("桌面添加 · 手机响铃", p.accent, p.accent_soft),
            ),
            self.text(
                "直接点选下方场景速设、常用时间或输入 24 小时制时间与倒计时（如 07:30 或 +15m），点击「添加并开启闹钟」即可同步到手机闹钟。",
                CAPTION,
                p.text2,
            ),
        ];
        if is_editing_alarm {
            create_alarm_items.push(
                Container::new(self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.ui_icon(UiIcon::Edit, 14., p.warning),
                            self.line_w(
                                "正在修改闹钟（保存后将直接更新原闹钟时间与标签）",
                                CAPTION,
                                p.text,
                                Weight::Semibold,
                            ),
                        ],
                    ),
                    self.button(
                        "alarm-edit-banner-cancel",
                        "取消编辑",
                        Action::BackScreen(BackScreenAction::AlarmCancelEdit),
                        Btn::Secondary,
                    ),
                ))
                .with_horizontal_padding(12.)
                .with_vertical_padding(8.)
                .with_background_color(p.warning_soft)
                .with_border(Border::all(1.).with_border_color(p.warning))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        }
        create_alarm_items.push(self.input(
            F_ALARM_TIME,
            "输入闹钟时间（24小时制如 07:30，或倒计时如 +15m）…",
            None,
        ));
        create_alarm_items.push(self.wrap(alarm_scenario_chips));
        create_alarm_items.push(self.wrap(quick_time_chips));
        create_alarm_items.push(self.input(
            F_PHONE_SCHEDULE_TITLE,
            "输入闹钟标签备注（如：早起打卡 / 会议提醒）…",
            None,
        ));
        let mut alarm_submit_btns = vec![self.button_icon(
            "alarm-create-custom",
            if is_editing_alarm {
                "保存闹钟修改"
            } else {
                "添加并开启闹钟"
            },
            if is_editing_alarm {
                UiIcon::Check
            } else {
                UiIcon::Plus
            },
            Action::BackScreen(BackScreenAction::AlarmCreateCustom),
            Btn::Primary,
        )];
        if is_editing_alarm {
            alarm_submit_btns.push(self.button(
                "alarm-cancel-edit-btn",
                "取消编辑",
                Action::BackScreen(BackScreenAction::AlarmCancelEdit),
                Btn::Secondary,
            ));
        } else {
            alarm_submit_btns.push(self.button_icon(
                "alarm-create-preset-15m",
                "15 分钟小憩闹钟",
                UiIcon::Coffee,
                Action::BackScreen(BackScreenAction::AlarmCreatePreset(0)),
                Btn::Secondary,
            ));
            alarm_submit_btns.push(self.button_icon(
                "alarm-create-preset-30m",
                "30 分钟后响铃",
                UiIcon::Plus,
                Action::BackScreen(BackScreenAction::AlarmCreatePreset(1)),
                Btn::Secondary,
            ));
            alarm_submit_btns.push(self.button_icon(
                "alarm-create-preset-0730",
                "明早 07:30 起床",
                UiIcon::Clock,
                Action::BackScreen(BackScreenAction::AlarmCreatePreset(2)),
                Btn::Secondary,
            ));
            alarm_submit_btns.push(self.button_icon(
                "alarm-create-preset-0800",
                "明早 08:00 早会",
                UiIcon::Clock,
                Action::BackScreen(BackScreenAction::AlarmCreatePreset(3)),
                Btn::Secondary,
            ));
        }
        create_alarm_items.push(self.wrap(alarm_submit_btns));
        let create_alarm_card = self.panel(create_alarm_items);

        let focus_card = self.panel(vec![
            self.between(
                self.heading("专注番茄钟与手机免打扰"),
                self.pill_badge(
                    if self.back_screen.pomodoro_active {
                        format!(
                            "专注剩余 {} 分钟",
                            self.back_screen.pomodoro_remaining_secs().div_ceil(60).max(1)
                        )
                    } else {
                        "背屏像素倒计时联动".to_string()
                    },
                    p.accent,
                    p.accent_soft,
                ),
            ),
            self.text(
                "开启桌面专注番茄钟时，手机背屏将同步点亮像素倒计时；同时可一键切换手机勿扰模式，避免专注期间受到打扰。",
                BODY,
                p.text2,
            ),
            self.wrap({
                let mut focus_btns = vec![
                    self.button(
                        "alarm-pomo-start-15",
                        "开启 15 分钟专注",
                        Action::BackScreen(BackScreenAction::StartPomodoro(15)),
                        Btn::Secondary,
                    ),
                    self.button(
                        "alarm-pomo-start",
                        "开启 25 分钟专注",
                        Action::BackScreen(BackScreenAction::StartPomodoro(25)),
                        if self.back_screen.pomodoro_active {
                            Btn::Secondary
                        } else {
                            Btn::Primary
                        },
                    ),
                    self.button(
                        "alarm-pomo-start-45",
                        "开启 45 分钟深度专注",
                        Action::BackScreen(BackScreenAction::StartPomodoro(45)),
                        Btn::Secondary,
                    ),
                ];
                if self.back_screen.pomodoro_active {
                    focus_btns.push(self.button(
                        "alarm-pomo-stop",
                        "停止专注番茄钟",
                        Action::BackScreen(BackScreenAction::StopPomodoro),
                        Btn::Danger,
                    ));
                }
                focus_btns.push(self.button(
                    "alarm-dnd-toggle",
                    if suite.controls.dnd_on {
                        "勿扰模式：已开启（点击恢复响铃）"
                    } else {
                        "勿扰模式：正常响铃（点击开启勿扰）"
                    },
                    Action::BackScreen(BackScreenAction::McpToggleDnd),
                    if suite.controls.dnd_on {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ));
                focus_btns
            }),
        ]);

        let left_col = self.vstack(S4, vec![create_alarm_card, focus_card]);

        // 3. Right Column: Visual Alarm Cards List
        let mut alarm_hdr_right = vec![self.pill_badge(
            format!("已开启 {} 个", enabled_alarms),
            if enabled_alarms > 0 {
                p.success
            } else {
                p.text2
            },
            p.surface,
        )];
        if !suite.alarms.is_empty() {
            alarm_hdr_right.push(self.button(
                "alarms-batch-enable-all",
                "全部开启",
                Action::BackScreen(BackScreenAction::AlarmToggleAll(true)),
                Btn::Secondary,
            ));
            alarm_hdr_right.push(self.button(
                "alarms-batch-disable-all",
                "全部关闭",
                Action::BackScreen(BackScreenAction::AlarmToggleAll(false)),
                Btn::Ghost,
            ));
        }
        let mut alarm_list_rows: Vec<El> = vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.heading("全部闹钟"),
                        self.pill_badge(
                            format!("共 {} 个", suite.alarms.len()),
                            p.accent,
                            p.accent_soft,
                        ),
                    ],
                ),
                self.hstack(S2, alarm_hdr_right),
            ),
            self.text(
                "直观展示手机与桌面端添加的全部闹钟卡片，支持在桌面端直接开启/关闭、编辑、顺延或右键快捷操作。",
                CAPTION,
                p.text2,
            ),
        ];

        if suite.alarms.is_empty() {
            alarm_list_rows.push(
                Container::new(self.text(
                    "当前暂无闹钟。可在左侧输入时间或点选快捷时间后点击「添加并开启闹钟」，添加后将立即显示在此处。",
                    BODY,
                    p.text2,
                ))
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        } else {
            for alarm in &suite.alarms {
                let border_color = if alarm.enabled { p.accent } else { p.border };
                let border_w = if alarm.enabled { 1.5 } else { 1.0 };
                let is_editing_this_alarm = self.back_screen.editing_alarm_id == Some(alarm.id);

                let left_info = self.hstack(
                    S3,
                    vec![
                        self.mono(
                            alarm.time_label(),
                            22.,
                            if alarm.enabled { p.text } else { p.text3 },
                        ),
                        self.hstack(
                            S2,
                            vec![
                                self.line_w(
                                    alarm.display_title(),
                                    BODY,
                                    if alarm.enabled { p.text } else { p.text2 },
                                    Weight::Semibold,
                                ),
                                self.pill_badge(
                                    if alarm.enabled {
                                        "已开启"
                                    } else {
                                        "已关闭"
                                    },
                                    if alarm.enabled { p.success } else { p.text3 },
                                    if alarm.enabled { p.accent_soft } else { p.bg },
                                ),
                            ],
                        ),
                    ],
                );

                let toggle_btn = self.button(
                    format!("alarm-card-toggle-{}", alarm.id),
                    if alarm.enabled {
                        "关闭闹钟"
                    } else {
                        "开启闹钟"
                    },
                    Action::BackScreen(BackScreenAction::AlarmToggle(alarm.id, !alarm.enabled)),
                    if alarm.enabled {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                );

                let secondary_controls = self.hstack(
                    S2,
                    vec![
                        self.button_icon(
                            format!("alarm-card-edit-{}", alarm.id),
                            if is_editing_this_alarm {
                                "编辑中"
                            } else {
                                "编辑"
                            },
                            UiIcon::Edit,
                            Action::BackScreen(BackScreenAction::AlarmEdit(alarm.id)),
                            if is_editing_this_alarm {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                        self.button(
                            format!("alarm-card-shift10-{}", alarm.id),
                            "+10分",
                            Action::BackScreen(BackScreenAction::AlarmShiftMinutes(alarm.id, 10)),
                            Btn::Secondary,
                        ),
                        self.button(
                            format!("alarm-card-del-{}", alarm.id),
                            "删除",
                            Action::BackScreen(BackScreenAction::AlarmDelete(alarm.id)),
                            Btn::Ghost,
                        ),
                    ],
                );

                let alarm_id = alarm.id;
                let alarm_enabled = alarm.enabled;
                let state = self.mouse_state(format!("alarm-card-row:{alarm_id}"));
                let row_inner = self.vstack(
                    S2,
                    vec![
                        self.between(left_info, toggle_btn),
                        self.between(
                            self.line(alarm.repeat_label(), CAPTION, p.text2),
                            secondary_controls,
                        ),
                    ],
                );
                alarm_list_rows.push(
                    Hoverable::new(state, move |mouse| {
                        let hovered = mouse.is_mouse_over_element();
                        let bc = if mouse.is_clicked() {
                            p.accent
                        } else if hovered && !alarm_enabled {
                            p.border_strong
                        } else {
                            border_color
                        };
                        Container::new(row_inner)
                            .with_uniform_padding(S3)
                            .with_background_color(p.surface)
                            .with_border(Border::all(border_w).with_border_color(bc))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish()
                    })
                    .with_defer_events_to_children()
                    .with_cursor(Cursor::PointingHand)
                    .on_double_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::BackScreen(
                            BackScreenAction::AlarmToggle(alarm_id, !alarm_enabled),
                        ));
                    })
                    .on_right_click(move |ctx, _, at| {
                        ctx.dispatch_typed_action(Action::ContextOpen(
                            crate::ui::context_menu::Target::AlarmItem(alarm_id),
                            at,
                        ));
                    })
                    .finish(),
                );
            }
        }

        let right_alarms_card = self.panel(alarm_list_rows);

        let main_split = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S4)
            .with_child(Expanded::new(1.0, left_col).finish())
            .with_child(Expanded::new(1.25, right_alarms_card).finish())
            .finish();

        Container::new(self.vstack(S4, vec![top_banner, main_split]))
            .with_padding_bottom(S2)
            .finish()
    }
}
