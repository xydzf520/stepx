//! Dedicated Visual Calendar Grid (`Page::Calendar`) and Dedicated Alarm Center (`Page::Alarms`).
//! Provides a full 7×6 interactive month calendar with cross-platform system calendar sync
//! (Linux / Windows / macOS 与手机日历双向同步), selected-day schedule inspector,
//! and a dedicated visual alarm clock page with desktop-to-phone alarm creation & toggles.

use super::super::{
    backscreen_state::BackScreenAction, icons::UiIcon, theme::*, widgets::*, Action, Companion,
    PushAction, F_ALARM_LABEL, F_ALARM_TIME, F_CALENDAR_TIME, F_PHONE_SCHEDULE_TITLE,
};
use super::{calendar_style::DayMarks, home::home_event_date_bounds};
use chrono::{Datelike, Days, Local, NaiveDate, Timelike};
use facloud_ui::{
    color::ColorU,
    elements::*,
    fonts::{Properties, Weight},
    platform::Cursor,
    text_layout::ClipConfig,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum CalendarEventValidity {
    OngoingToday,
    UpcomingToday { mins_until: Option<i64> },
    UpcomingFuture { days_until: i64 },
    EndedToday,
    ManuallyExpired,
    Completed,
    PastDate,
}

impl CalendarEventValidity {
    #[inline]
    fn is_valid_unexpired(self) -> bool {
        matches!(
            self,
            Self::OngoingToday | Self::UpcomingToday { .. } | Self::UpcomingFuture { .. }
        )
    }

    #[inline]
    fn is_today_valid_unexpired(self) -> bool {
        matches!(self, Self::OngoingToday | Self::UpcomingToday { .. })
    }

    #[inline]
    fn sort_rank(self) -> u8 {
        match self {
            Self::OngoingToday => 0,
            Self::UpcomingToday { .. } => 1,
            Self::UpcomingFuture { .. } => 2,
            Self::EndedToday => 3,
            Self::Completed => 4,
            Self::ManuallyExpired => 5,
            Self::PastDate => 6,
        }
    }

    fn badge(self, p: &Palette) -> (String, ColorU, ColorU) {
        match self {
            Self::OngoingToday => ("进行中".into(), p.success_text, p.success_soft),
            Self::UpcomingToday { mins_until } => {
                let label = match mins_until {
                    Some(mins) if mins >= 60 => {
                        if mins % 60 == 0 {
                            format!("{} 小时后", mins / 60)
                        } else {
                            format!("{} 小时 {} 分后", mins / 60, mins % 60)
                        }
                    }
                    Some(mins) => format!("{} 分钟后", mins.max(1)),
                    None => "今天".into(),
                };
                (label, p.accent, p.accent_soft)
            }
            Self::UpcomingFuture { days_until } => {
                let label = match days_until {
                    1 => "明天".into(),
                    2 => "后天".into(),
                    d => format!("{d} 天后"),
                };
                (label, p.accent, p.accent_soft)
            }
            Self::EndedToday => ("已结束".into(), p.text3, p.card2),
            Self::ManuallyExpired => ("已过期".into(), p.text3, p.card2),
            Self::Completed => ("已完成".into(), p.success_text, p.success_soft),
            Self::PastDate => ("已过期".into(), p.text3, p.card2),
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum AlarmValidity {
    TodayValidUnexpired { diff_mins: u32 },
    PassedTodayRepeating,
    FutureDayRepeating,
    ExpiredOneShot,
    ManuallyExpired,
    Disabled,
}

fn fmt_alarm_countdown(diff_mins: u32) -> String {
    let mins = diff_mins.max(1);
    if mins >= 60 {
        if mins % 60 == 0 {
            format!("{} 小时后", mins / 60)
        } else {
            format!("{} 小时 {} 分后", mins / 60, mins % 60)
        }
    } else {
        format!("{mins} 分钟后")
    }
}

impl AlarmValidity {
    #[inline]
    fn is_today_unexpired(self) -> bool {
        matches!(self, Self::TodayValidUnexpired { .. })
    }

    #[inline]
    fn is_valid_active(self) -> bool {
        matches!(
            self,
            Self::TodayValidUnexpired { .. }
                | Self::PassedTodayRepeating
                | Self::FutureDayRepeating
        )
    }

    #[inline]
    fn sort_rank(self) -> u8 {
        match self {
            Self::TodayValidUnexpired { .. } => 0,
            Self::PassedTodayRepeating => 1,
            Self::FutureDayRepeating => 2,
            Self::ExpiredOneShot => 3,
            Self::ManuallyExpired => 4,
            Self::Disabled => 5,
        }
    }

    fn badge(self, p: &Palette) -> (String, ColorU, ColorU) {
        match self {
            Self::TodayValidUnexpired { diff_mins } => (
                fmt_alarm_countdown(diff_mins),
                p.success_text,
                p.success_soft,
            ),
            Self::PassedTodayRepeating => ("明天再响".into(), p.warning, p.warning_soft),
            Self::FutureDayRepeating => ("今天不响".into(), p.accent, p.accent_soft),
            Self::ExpiredOneShot => ("已过".into(), p.text3, p.card2),
            Self::ManuallyExpired => ("已过期".into(), p.text3, p.card2),
            Self::Disabled => ("未开启".into(), p.text3, p.card2),
        }
    }
}

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

#[inline]
fn truncate_chars(s: &str, max_chars: usize) -> String {
    stepx_companion::home_hub::truncate_chars(s, max_chars)
}

impl Companion {
    #[allow(dead_code)]
    pub(in super::super) fn schedule_page(&self) -> El {
        self.calendar_page()
    }

    fn calendar_event_validity(
        &self,
        ev: &stepx_companion::toolbox::PhoneCalendarEvent,
        today: NaiveDate,
        now_ms: i64,
    ) -> CalendarEventValidity {
        if self.calendar_event_is_completed(ev) {
            return CalendarEventValidity::Completed;
        }
        if self.calendar_event_is_suppressed(ev, now_ms) {
            return CalendarEventValidity::ManuallyExpired;
        }
        let (ey, em, ed) = ev.date_ymd();
        let Some(target_date) = NaiveDate::from_ymd_opt(ey, em, ed) else {
            return CalendarEventValidity::PastDate;
        };
        let effective_end_ms = if ev.dtend > ev.dtstart {
            ev.dtend
        } else {
            ev.dtstart + 3_600_000
        };
        let day_diff = (target_date - today).num_days();
        if day_diff < 0 {
            if !ev.all_day && ev.dtstart <= now_ms && effective_end_ms > now_ms {
                CalendarEventValidity::OngoingToday
            } else {
                CalendarEventValidity::PastDate
            }
        } else if day_diff == 0 {
            if ev.all_day {
                CalendarEventValidity::UpcomingToday { mins_until: None }
            } else if ev.dtstart <= now_ms && effective_end_ms > now_ms {
                CalendarEventValidity::OngoingToday
            } else if ev.dtstart > now_ms {
                let mins = ((ev.dtstart - now_ms) / 60_000).max(1);
                CalendarEventValidity::UpcomingToday {
                    mins_until: Some(mins),
                }
            } else {
                CalendarEventValidity::EndedToday
            }
        } else {
            CalendarEventValidity::UpcomingFuture {
                days_until: day_diff,
            }
        }
    }

    fn alarm_validity(
        &self,
        a: &stepx_companion::toolbox::PhoneAlarmItem,
        now_ms: i64,
        now_min: u32,
        weekday_idx: u32,
    ) -> AlarmValidity {
        let is_suppressed = self.reminder_governance.is_any_key_suppressed(
            &[format!("alarm:{}", a.id), format!("capsule:alarm-{}", a.id)],
            None,
            None,
            None,
            now_ms,
        );
        if is_suppressed {
            return AlarmValidity::ManuallyExpired;
        }
        if !a.enabled {
            return AlarmValidity::Disabled;
        }
        let matches_today =
            a.days_of_week == 0 || (a.days_of_week & (1u32 << weekday_idx.min(6))) != 0;
        let alarm_min = (a.hour as u32) * 60 + (a.minutes as u32);
        if matches_today && alarm_min > now_min {
            AlarmValidity::TodayValidUnexpired {
                diff_mins: alarm_min - now_min,
            }
        } else if matches_today && a.days_of_week != 0 {
            AlarmValidity::PassedTodayRepeating
        } else if !matches_today && a.days_of_week != 0 {
            AlarmValidity::FutureDayRepeating
        } else {
            AlarmValidity::ExpiredOneShot
        }
    }

    pub(super) fn calendar_actions(&self) -> Vec<El> {
        vec![
            self.button_icon(
                "cal-banner-autosync-toggle",
                "自动同步",
                if self.prefs.auto_sync_calendar {
                    UiIcon::Check
                } else {
                    UiIcon::Pause
                },
                Action::BackScreen(BackScreenAction::ToggleAutoSyncCalendar),
                Btn::Ghost,
            ),
            if self.back_screen.stepos_suite_loading {
                self.disabled_button("同步中…")
            } else {
                self.button_icon(
                    "cal-hdr-refresh",
                    "同步",
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
        let now_local = Local::now();
        let today = now_local.date_naive();
        let now_ms = now_local.timestamp_millis();

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

        let visible_calendar_events: Vec<_> = suite
            .calendar_events
            .iter()
            .filter(|event| !event.is_e2e_probe())
            .collect();
        let month_days = self.home_month_days();
        let mut marks = [DayMarks::default(); 32];
        for day in month_days.iter().filter(|d| (d.date.year(), d.date.month()) == (year, month)) {
            marks[day.date.day() as usize] = day.marks;
        }
        let first_day = NaiveDate::from_ymd_opt(year, month, 1).unwrap_or(today);
        let last_day = NaiveDate::from_ymd_opt(year, month, days_in_month).unwrap_or(first_day);

        let mut by_day: [Vec<&stepx_companion::toolbox::PhoneCalendarEvent>; 32] =
            std::array::from_fn(|_| Vec::new());
        let mut today_valid_unexpired_count = 0usize;
        for ev in &visible_calendar_events {
            let Some((start, end)) = home_event_date_bounds(ev) else {
                continue;
            };
            if today >= start && today <= end
                && self.calendar_event_validity(ev, today, now_ms).is_today_valid_unexpired()
            {
                today_valid_unexpired_count += 1;
            }
            if start <= last_day && end >= first_day {
                for day in start.max(first_day).day()..=end.min(last_day).day() {
                    by_day[day as usize].push(*ev);
                }
            }
        }
        for day_vec in &mut by_day {
            day_vec.sort_by_key(|ev| {
                let v = self.calendar_event_validity(ev, today, now_ms);
                (v.sort_rank(), ev.dtstart, ev.id)
            });
        }
        let selected_day_events = &by_day[selected_day as usize];
        let selected_day_valid_count = selected_day_events
            .iter()
            .filter(|ev| {
                self.calendar_event_validity(ev, today, now_ms)
                    .is_valid_unexpired()
            })
            .count();

        // 2. Left Column: Visual 7×6 Month Calendar Grid
        let month_nav_bar = self.between(
            self.hstack(
                S2,
                vec![
                    self.heading(format!("{year} 年 {month:02} 月")),
                    self.pill_badge(
                        format!("今日 {today_valid_unexpired_count} 项"),
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
                        UiIcon::ChevronLeft,
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
            .finish();
            weekday_header_row = weekday_header_row.with_child(Expanded::new(1., cell).finish());
        }

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
                    let adjacent = if day_num < 1 {
                        first_day.checked_sub_days(Days::new((1 - day_num) as u64))
                    } else {
                        first_day.checked_add_days(Days::new((day_num - 1) as u64))
                    };
                    let content = adjacent.map_or_else(
                        || Empty::new().finish(),
                        |date| {
                            let marks = month_days.iter().find(|day| day.date == date)
                                .map_or(DayMarks::default(), |day| day.marks);
                            self.hstack(
                                5.,
                                vec![
                                    self.calendar_day_number(date.day(), date == today, false, false),
                                    self.calendar_day_marks(marks),
                                ],
                            )
                        },
                    );
                    ConstrainedBox::new(
                        Container::new(Align::new(content).top_left().finish())
                            .with_uniform_padding(6.)
                            .finish(),
                    )
                    .with_height(80.)
                    .finish()
                } else {
                    let d = day_num as u32;
                    let is_today =
                        year == today.year() && month == today.month() && d == today.day();
                    let is_selected = d == selected_day;

                    let day_events = &by_day[d as usize];
                    let day_marks = marks[d as usize];
                    let previews: Vec<(i64, String, bool, bool)> = day_events
                        .iter()
                        .take(2)
                        .map(|ev| {
                            let validity = self.calendar_event_validity(ev, today, now_ms);
                            let is_valid = validity.is_valid_unexpired();
                            let raw_label = match validity {
                                CalendarEventValidity::Completed => {
                                    format!("✓ {} {}", ev.hm_label(), ev.title)
                                }
                                _ if !is_valid => {
                                    format!("[已过] {} {}", ev.hm_label(), ev.title)
                                }
                                _ => format!("{} {}", ev.hm_label(), ev.title),
                            };
                            let label = truncate_chars(&raw_label, 8);
                            let is_sys = ev.source.contains("system")
                                || ev.source.contains("desktop")
                                || ev.source.contains("local")
                                || ev.source.contains("电脑")
                                || ev.source.contains("双端");
                            (ev.id, label, is_sys, is_valid)
                        })
                        .collect();

                    let state = self.mouse_state(format!("cal-cell:{year}:{month}:{d}"));
                    let font = self.font;

                    Hoverable::new(state, move |mouse| {
                        let t = self.hover_t(mouse);
                        let bg = if is_selected {
                            p.accent_soft
                        } else if mouse.is_clicked() {
                            p.press_color(p.hover)
                        } else {
                            lerp_color(p.bg, p.hover, t)
                        };
                        let border_color = if is_selected {
                            p.accent
                        } else {
                            lerp_color(p.border, p.border_strong, t)
                        };
                        let date = self.calendar_day_number(d, is_today, is_selected, true);
                        let top_row = self.hstack(5., vec![date, self.calendar_day_marks(day_marks)]);
                        #[cfg(feature = "ui-capture")]
                        let top_row = crate::ui::scenario_check::control(
                            if is_today {
                                "cal-cell-today".to_string()
                            } else {
                                format!("cal-cell-{year}-{month}-{d}")
                            },
                            top_row,
                        );

                        let mut cell_items: Vec<El> = vec![top_row];
                        for &(ev_id, ref preview_text, is_sys, is_valid) in &previews {
                            let pill_state =
                                self.mouse_state(format!("cal-cell-ev:{year}:{month}:{d}:{ev_id}"));
                            let pill_text = preview_text.clone();
                            let pill = Hoverable::new(pill_state, move |pill_mouse| {
                                let pill_hovered = pill_mouse.is_mouse_over_element();
                                let pill_bg = if pill_mouse.is_clicked() || pill_hovered {
                                    p.hover
                                } else if !is_valid {
                                    p.card2
                                } else if is_selected {
                                    p.bg
                                } else {
                                    p.accent_soft
                                };
                                let pill_fg = if !is_valid {
                                    p.text3
                                } else if is_sys {
                                    p.blue
                                } else {
                                    p.text
                                };
                                Container::new(
                                    Text::new(pill_text.clone(), font, MICRO)
                                        .with_color(pill_fg)
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
                                .with_border(Border::all(1.).with_border_color(border_color))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish(),
                        )
                        .with_height(80.)
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

        let left_calendar_card = self.panel(vec![
            month_nav_bar,
            weekday_header_row.finish(),
            self.calendar_content_enter(self.vstack(6., week_rows), true),
            self.bounded_wrap(self.calendar_mark_legend()),
        ]);

        // 3. Right Column: Selected Day Inspector & Event Creator
        // Card 1: Selected Date Header & Create Event Form
        let (sel_rel_label, sel_rel_fg, sel_rel_bg) =
            relative_day_badge(year, month, selected_day, today, &p);
        let is_editing = self.back_screen.editing_calendar_event_id.is_some();
        let current_cal_time = self.fields[F_CALENDAR_TIME].text.trim();
        let cal_time_presets: [(&'static str, &'static str); 4] = [
            ("09:00", "09:00-09:30"),
            ("14:00", "14:00-15:00"),
            ("16:00", "16:00-17:00"),
            ("全天", "09:00-18:00"),
        ];
        let mut cal_time_preset_chips: Vec<El> = Vec::new();
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

        let mut create_card_items: Vec<El> = vec![self.hstack(
            S2,
            vec![
                self.heading(format!("{month}月{selected_day}日")),
                self.pill_badge(sel_rel_label, sel_rel_fg, sel_rel_bg),
            ],
        )];

        if is_editing {
            create_card_items.push(self.status_banner_box(
                self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.content_icon(UiIcon::Edit, 14., p.warning),
                            self.line_w("正在修改日程", CAPTION, p.text, Weight::Semibold),
                        ],
                    ),
                    self.button(
                        "cal-edit-banner-cancel",
                        "取消编辑",
                        Action::BackScreen(BackScreenAction::CalendarCancelEdit),
                        Btn::Secondary,
                    ),
                ),
                p.warning_soft,
                p.warning,
            ));
        }

        create_card_items.push(self.input(F_PHONE_SCHEDULE_TITLE, "日程标题", None));
        create_card_items.push(self.input(F_CALENDAR_TIME, "时间，如 09:30", None));
        create_card_items.push(self.wrap(cal_time_preset_chips));

        let mut submit_btns = vec![self.button_icon(
            "cal-create-selected-custom",
            if is_editing { "保存" } else { "添加" },
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
        }
        create_card_items.push(self.wrap(submit_btns));

        let create_event_card = self.panel(create_card_items);

        // Card 2: Selected Date Events & Upcoming Agenda
        let mut selected_day_hdr_right = vec![self.pill_badge(
            format!("{selected_day_valid_count}/{}", selected_day_events.len()),
            if selected_day_valid_count > 0 {
                p.success_text
            } else {
                p.accent
            },
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
            self.section_heading(UiIcon::Calendar, "当日日程"),
            self.hstack(S2, selected_day_hdr_right),
        )];

        if selected_day_events.is_empty() {
            agenda_rows.push(self.surface_note_box("暂无日程", p.text2));
        } else {
            let page_key = format!("calendar-day:{year}:{month}:{selected_day}");
            let range = self.list_page_range(&page_key,selected_day_events.len(),20);
            agenda_rows.push(self.list_page_controls(&page_key,selected_day_events.len(),20));
            for (_i, event) in selected_day_events.iter().enumerate().skip(range.start).take(range.len()) {
                let row_el = self.render_calendar_event_row(event, "sel", today, now_ms);
                #[cfg(feature = "ui-capture")]
                let row_el =
                    crate::ui::scenario_check::control(format!("cal-agenda-row-{_i}"), row_el);
                agenda_rows.push(row_el);
            }
        }

        if let Some(day) = month_days.iter().find(|day| day.date.day() == selected_day
            && (day.date.year(), day.date.month()) == (year, month))
        {
            let rows = self.calendar_day_context_rows(day);
            if !rows.is_empty() {
                agenda_rows.push(self.divider());
                agenda_rows.extend(rows);
            }
        }
        let selected_agenda = self.calendar_content_enter(self.vstack(S2, agenda_rows), false);
        let mut agenda_rows = vec![selected_agenda];

        let mut upcoming_valid_events: Vec<&stepx_companion::toolbox::PhoneCalendarEvent> =
            visible_calendar_events
                .iter()
                .copied()
                .filter(|ev| {
                    self.calendar_event_validity(ev, today, now_ms)
                        .is_valid_unexpired()
                })
                .collect();
        upcoming_valid_events.sort_by_key(|ev| (ev.dtstart, ev.id));

        agenda_rows.push(self.divider());
        agenda_rows.push(self.section_heading(UiIcon::Calendar, "近期日程"));

        if upcoming_valid_events.is_empty() {
            agenda_rows.push(self.surface_note_box(
                if suite.checked {
                    "暂无近期日程"
                } else {
                    "正在读取…"
                },
                p.text2,
            ));
        } else {
            let range = self.list_page_range("calendar-upcoming",upcoming_valid_events.len(),8);
            agenda_rows.push(self.list_page_controls("calendar-upcoming",upcoming_valid_events.len(),8));
            for (_i, event) in upcoming_valid_events.iter().enumerate().skip(range.start).take(range.len()) {
                let row_el = self.render_calendar_event_row(event, "all", today, now_ms);
                #[cfg(feature = "ui-capture")]
                let row_el = if selected_day_events.is_empty() {
                    crate::ui::scenario_check::control(format!("cal-agenda-row-{_i}"), row_el)
                } else {
                    crate::ui::scenario_check::control(format!("cal-all-row-{_i}"), row_el)
                };
                agenda_rows.push(row_el);
            }
        }

        let agenda_card = self.panel(agenda_rows);
        let right_col = self.vstack(S4, vec![create_event_card, agenda_card]);
        let right_col = super::super::motion::enter(
            right_col,
            self.motion_started,
            !self.settings.value.reduced_motion && !self.settings.hidden,
        );

        let main_split = self.responsive_columns(
            860.,
            S4,
            S3,
            CrossAxisAlignment::Start,
            vec![(1.35, left_calendar_card), (1., right_col)],
        );

        Container::new(main_split).with_padding_bottom(S2).finish()
    }

    fn render_calendar_event_row(
        &self,
        event: &stepx_companion::toolbox::PhoneCalendarEvent,
        prefix: &str,
        today: NaiveDate,
        now_ms: i64,
    ) -> El {
        let p = self.p();
        let time_badge = event.time_range_label();
        let src_label = event.source_label();
        let (ey, em, ed) = event.date_ymd();
        let validity = self.calendar_event_validity(event, today, now_ms);
        let (val_label, val_fg, val_bg) = validity.badge(&p);
        let is_done = self.calendar_event_is_completed(event);
        let is_valid = validity.is_valid_unexpired();
        let title_color = if is_valid { p.text } else { p.text3 };

        let mut header_badges = vec![
            self.line_w(&event.title, BODY, title_color, Weight::Semibold),
            self.pill_badge(val_label, val_fg, val_bg),
            self.pill_badge(&time_badge, p.accent, p.accent_soft),
            self.pill_badge(src_label, p.blue, p.surface),
        ];
        if is_done && !matches!(validity, CalendarEventValidity::Completed) {
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

        let is_editing_this = self.back_screen.editing_calendar_event_id == Some(event.id);
        let row_key = format!("cal-ev-row:{prefix}:{}:{}", event.id, event.dtstart);
        let done_id = format!("cal-{prefix}-done-{}:{}", event.id, event.dtstart);
        let edit_id = format!("cal-{prefix}-edit-{}:{}", event.id, event.dtstart);
        let del_id = format!("cal-{prefix}-del-{}:{}", event.id, event.dtstart);
        let jump_id = format!("cal-{prefix}-jump-{}:{}", event.id, event.dtstart);
        let phone_only = self.calendar_event_action_limit(event.id).is_some();
        let done_btn = if phone_only {
            self.button_icon(done_id, "手机处理", UiIcon::Smartphone,
                Action::OpenAppInMirror("com.android.calendar".into()), Btn::Secondary)
        } else { self.button_icon(
            done_id,
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
        ) };

        let top_row = self.between(self.vstack(S1, detail_col), done_btn);
        let mut row_children = vec![top_row];

        if !phone_only {
            let mut secondary_actions = vec![
                self.button_icon(
                    edit_id,
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
                    del_id,
                    "删除",
                    Action::BackScreen(BackScreenAction::CalendarDelete(event.id)),
                    Btn::Ghost,
                ),
            ];
            if prefix == "all" {
                secondary_actions.insert(
                    0,
                    self.button(
                        jump_id,
                        "定位",
                        Action::BackScreen(BackScreenAction::CalendarSelectDate(ey, em, ed)),
                        Btn::Ghost,
                    ),
                );
            }
            row_children.push(self.wrap(secondary_actions));
        } else {
            row_children.push(self.text("需在手机日历中选择处理本次或整个系列", MICRO, p.text2));
        }

        let row_body = self.vstack(S2, row_children);
        let state = self.mouse_state(row_key);
        let event_id = event.id;
        Hoverable::new(state, move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let hover_t = self.hover_t(&mouse);
            let row_bg = if is_editing_this {
                p.accent_soft
            } else {
                p.surface
            };
            let bg_col = if mouse.is_clicked() {
                p.press_color(p.hover)
            } else {
                lerp_color(row_bg, p.hover, hover_t)
            };
            let border_col = if mouse.is_clicked() || is_editing_this {
                p.accent
            } else if hovered {
                p.border_strong
            } else {
                lerp_color(p.border, p.border_strong, hover_t)
            };
            Container::new(row_body)
                .with_uniform_padding(S3)
                .with_background_color(bg_col)
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
            ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::CalendarEditEvent(
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
        vec![if self.back_screen.stepos_suite_loading {
            self.disabled_button("同步中…")
        } else {
            self.button_icon(
                "alarms-hdr-refresh",
                "刷新",
                UiIcon::Refresh,
                Action::BackScreen(BackScreenAction::InspectStepOsSuite),
                Btn::Primary,
            )
        }]
    }

    pub(super) fn alarms_page(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;

        let now = Local::now();
        let now_ms = now.timestamp_millis();
        let now_min = now.hour() * 60 + now.minute();
        let weekday_idx = now.weekday().num_days_from_monday();
        let now_hm = now.format("%H:%M").to_string();
        let weekday_cn = match weekday_idx {
            0 => "星期一",
            1 => "星期二",
            2 => "星期三",
            3 => "星期四",
            4 => "星期五",
            5 => "星期六",
            _ => "星期日",
        };
        let now_date = format!("{} · {}", now.format("%Y年%m月%d日"), weekday_cn);

        let mut visible_alarms: Vec<&stepx_companion::toolbox::PhoneAlarmItem> = suite
            .alarms
            .iter()
            .filter(|a| !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&a.message))
            .collect();
        visible_alarms.sort_by_key(|a| {
            let v = self.alarm_validity(a, now_ms, now_min, weekday_idx);
            let alarm_min = a.hour.saturating_mul(60).saturating_add(a.minutes);
            (v.sort_rank(), alarm_min, a.id)
        });

        let mut today_unexpired_count = 0usize;
        let mut valid_active_count = 0usize;
        let mut next_today_alarm: Option<(&stepx_companion::toolbox::PhoneAlarmItem, u32)> = None;

        for &a in &visible_alarms {
            let v = self.alarm_validity(a, now_ms, now_min, weekday_idx);
            if let AlarmValidity::TodayValidUnexpired { diff_mins } = v {
                today_unexpired_count += 1;
                if next_today_alarm.is_none_or(|(_, best)| diff_mins < best) {
                    next_today_alarm = Some((a, diff_mins));
                }
            }
            if v.is_valid_active() {
                valid_active_count += 1;
            }
        }

        // 1. Top Digital Clock & Alarm Summary Banner
        let clock_display = Container::new(self.mono(now_hm, 26., p.text))
            .with_horizontal_padding(S4)
            .with_vertical_padding(S2)
            .with_background_color(p.surface)
            .with_border(Border::all(1.5).with_border_color(p.accent))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();

        let banner_info = vec![self.line(now_date, BODY, p.text2)];
        let mut banner_pills: Vec<El> = Vec::new();
        if let Some((next_alarm, diff_mins)) = next_today_alarm {
            banner_pills.push(self.pill_badge(
                format!(
                    "下一个 {:02}:{:02} · {}",
                    next_alarm.hour,
                    next_alarm.minutes,
                    fmt_alarm_countdown(diff_mins)
                ),
                p.accent,
                p.accent_soft,
            ));
        }

        let top_banner = self.panel(vec![self.between(
            self.hstack(S4, vec![clock_display, self.vstack(S1, banner_info)]),
            self.hstack(S2, banner_pills),
        )]);

        // 2. Left Column: Visual Alarm Creator & Pomodoro Focus
        let quick_times: [&'static str; 6] = ["06:30", "07:00", "07:30", "08:00", "08:30", "09:00"];
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

        let alarm_scenario_presets: [(UiIcon, &'static str, &'static str, &'static str); 4] = [
            (UiIcon::Clock, "5 分钟后", "+5m", "倒计时"),
            (UiIcon::Coffee, "15 分钟小憩", "+15m", "小憩"),
            (UiIcon::Clock, "25 分钟专注", "+25m", "专注"),
            (UiIcon::Clock, "1 小时后", "+60m", "提醒"),
        ];
        let mut alarm_scenario_chips: Vec<El> = Vec::new();
        for (idx, (icon, btn_label, qt_str, tpl_label)) in
            alarm_scenario_presets.into_iter().enumerate()
        {
            let active_scen =
                current_alarm_time == qt_str && self.fields[F_ALARM_LABEL].text.trim() == tpl_label;
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
                            self.content_icon(
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
                    ctx.dispatch_typed_action(Action::Focus(F_ALARM_LABEL));
                    ctx.dispatch_typed_action(Action::ClearField(F_ALARM_LABEL));
                    ctx.dispatch_typed_action(Action::Insert(note_str.clone()));
                })
                .finish(),
            );
        }

        let is_editing_alarm = self.back_screen.editing_alarm_id.is_some();
        let mut create_alarm_items: Vec<El> = vec![self.section_heading(UiIcon::Clock, "新建闹钟")];
        if is_editing_alarm {
            create_alarm_items.push(self.status_banner_box(
                self.between(
                    self.hstack(
                        8.,
                        vec![
                            self.content_icon(UiIcon::Edit, 14., p.warning),
                            self.line_w("正在修改闹钟", CAPTION, p.text, Weight::Semibold),
                        ],
                    ),
                    self.button(
                        "alarm-edit-banner-cancel",
                        "取消编辑",
                        Action::BackScreen(BackScreenAction::AlarmCancelEdit),
                        Btn::Secondary,
                    ),
                ),
                p.warning_soft,
                p.warning,
            ));
        }
        create_alarm_items.push(self.input(F_ALARM_TIME, "07:30 或 +15m", None));
        create_alarm_items.push(self.wrap(quick_time_chips));
        create_alarm_items.push(self.input(F_ALARM_LABEL, "标签（可选）", None));
        let mut alarm_submit_btns = vec![self.button_icon(
            "alarm-create-custom",
            if is_editing_alarm { "保存" } else { "添加" },
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
        }
        create_alarm_items.push(self.wrap(alarm_submit_btns));
        if !is_editing_alarm {
            create_alarm_items.push(self.wrap(alarm_scenario_chips));
        }
        let create_alarm_card = self.panel(create_alarm_items);

        let focus_card = self.panel(vec![
            if self.back_screen.pomodoro_active {
                self.between(
                    self.section_heading(UiIcon::Coffee, "专注"),
                    self.pill_badge(
                        format!(
                            "剩余 {} 分钟",
                            self.back_screen
                                .pomodoro_remaining_secs()
                                .div_ceil(60)
                                .max(1)
                        ),
                        p.accent,
                        p.accent_soft,
                    ),
                )
            } else {
                self.section_heading(UiIcon::Coffee, "专注")
            },
            self.wrap({
                let mut focus_btns = vec![
                    self.button(
                        "alarm-pomo-start-15",
                        "15 分钟",
                        Action::BackScreen(BackScreenAction::StartPomodoro(15)),
                        Btn::Secondary,
                    ),
                    self.button(
                        "alarm-pomo-start",
                        "25 分钟",
                        Action::BackScreen(BackScreenAction::StartPomodoro(25)),
                        if self.back_screen.pomodoro_active {
                            Btn::Secondary
                        } else {
                            Btn::Primary
                        },
                    ),
                    self.button(
                        "alarm-pomo-start-45",
                        "45 分钟",
                        Action::BackScreen(BackScreenAction::StartPomodoro(45)),
                        Btn::Secondary,
                    ),
                ];
                if self.back_screen.pomodoro_active {
                    focus_btns.push(self.button(
                        "alarm-pomo-stop",
                        "停止",
                        Action::BackScreen(BackScreenAction::StopPomodoro),
                        Btn::Danger,
                    ));
                }
                focus_btns.push(self.button(
                    "alarm-dnd-toggle",
                    if suite.controls.dnd_on {
                        "勿扰：开"
                    } else {
                        "勿扰：关"
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
            format!(
                "今日 {today_unexpired_count} · 有效 {valid_active_count}/{}",
                visible_alarms.len()
            ),
            if today_unexpired_count > 0 {
                p.success_text
            } else if valid_active_count > 0 {
                p.accent
            } else {
                p.text2
            },
            if today_unexpired_count > 0 {
                p.success_soft
            } else if valid_active_count > 0 {
                p.accent_soft
            } else {
                p.surface
            },
        )];
        if !visible_alarms.is_empty() {
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
        let mut alarm_list_rows: Vec<El> = vec![self.between(
            self.section_heading(UiIcon::Clock, "全部闹钟"),
            self.hstack(S2, alarm_hdr_right),
        )];

        if visible_alarms.is_empty() {
            alarm_list_rows.push(self.surface_note_box("暂无闹钟", p.text2));
        } else {
            let range = self.list_page_range("phone-alarms",visible_alarms.len(),20);
            alarm_list_rows.push(self.list_page_controls("phone-alarms",visible_alarms.len(),20));
            for (_i, &alarm) in visible_alarms.iter().enumerate().skip(range.start).take(range.len()) {
                let validity = self.alarm_validity(alarm, now_ms, now_min, weekday_idx);
                let is_valid_active = validity.is_valid_active();
                let is_today_unexpired = validity.is_today_unexpired();
                let (v_label, v_fg, v_bg) = validity.badge(&p);

                let border_color = if is_today_unexpired {
                    p.accent
                } else if is_valid_active {
                    p.border_strong
                } else {
                    p.border
                };
                let border_w = if is_today_unexpired { 1.5 } else { 1.0 };
                let is_editing_this_alarm = self.back_screen.editing_alarm_id == Some(alarm.id);

                let left_info = self.hstack(
                    S3,
                    vec![
                        self.mono(
                            alarm.time_label(),
                            22.,
                            if is_valid_active { p.text } else { p.text3 },
                        ),
                        self.hstack(
                            S2,
                            vec![
                                self.line_w(
                                    alarm.display_title(),
                                    BODY,
                                    if is_valid_active { p.text } else { p.text3 },
                                    if is_valid_active {
                                        Weight::Semibold
                                    } else {
                                        Weight::Medium
                                    },
                                ),
                                self.pill_badge(v_label, v_fg, v_bg),
                            ],
                        ),
                    ],
                );

                let alarm_id = alarm.id;
                let row_key = format!("alarm-card-row:{alarm_id}");
                let toggle_id = format!("alarm-card-toggle-{alarm_id}");
                let edit_id = format!("alarm-card-edit-{alarm_id}");
                let shift_id = format!("alarm-card-shift10-{alarm_id}");
                let del_id = format!("alarm-card-del-{alarm_id}");
                let hover_keys = [
                    row_key.clone(),
                    toggle_id.clone(),
                    format!("btn:{edit_id}"),
                    format!("btn:{shift_id}"),
                    format!("btn:{del_id}"),
                ];
                let is_card_hovered = {
                    let map = self.hover_states.borrow();
                    hover_keys.iter().any(|k| {
                        map.get(k.as_str())
                            .is_some_and(|(_, h)| h.lock().unwrap().is_mouse_over_element())
                    })
                };

                let toggle_sw = self.toggle_switch(
                    toggle_id,
                    alarm.enabled,
                    Action::BackScreen(BackScreenAction::AlarmToggle(alarm.id, !alarm.enabled)),
                );

                let repeat_fg = if is_valid_active { p.text2 } else { p.text3 };
                let bottom_row = if is_card_hovered || is_editing_this_alarm {
                    let secondary_controls = self.hstack(
                        S2,
                        vec![
                            self.button_icon(
                                edit_id,
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
                                shift_id,
                                "+10分",
                                Action::BackScreen(BackScreenAction::AlarmShiftMinutes(
                                    alarm.id, 10,
                                )),
                                Btn::Secondary,
                            ),
                            self.button(
                                del_id,
                                "删除",
                                Action::BackScreen(BackScreenAction::AlarmDelete(alarm.id)),
                                Btn::Ghost,
                            ),
                        ],
                    );
                    self.between(
                        self.line(alarm.repeat_label(), CAPTION, repeat_fg),
                        secondary_controls,
                    )
                } else {
                    self.line(alarm.repeat_label(), CAPTION, repeat_fg)
                };

                let state = self.mouse_state(row_key);
                let row_inner =
                    self.vstack(S2, vec![self.between(left_info, toggle_sw), bottom_row]);
                let card_el = Hoverable::new(state, move |mouse| {
                    let hovered = mouse.is_mouse_over_element();
                    let hover_t = self.hover_t(&mouse);
                    let bg = if mouse.is_clicked() {
                        p.press_color(p.hover)
                    } else {
                        lerp_color(p.surface, p.hover, hover_t)
                    };
                    let bc = if mouse.is_clicked() {
                        p.accent
                    } else if hovered && !is_today_unexpired {
                        p.border_strong
                    } else {
                        border_color
                    };
                    Container::new(row_inner)
                        .with_uniform_padding(S3)
                        .with_background_color(bg)
                        .with_border(Border::all(border_w).with_border_color(bc))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                        .finish()
                })
                .with_defer_events_to_children()
                .with_cursor(Cursor::PointingHand)
                .on_double_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::AlarmEdit(
                        alarm_id,
                    )));
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::AlarmItem(alarm_id),
                        at,
                    ));
                })
                .finish();
                let card_el =
                    SavePosition::new(card_el, &format!("search-alarm-{alarm_id}")).finish();
                #[cfg(feature = "ui-capture")]
                let card_el =
                    crate::ui::scenario_check::control(format!("alarm-card-{_i}"), card_el);
                alarm_list_rows.push(card_el);
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

#[cfg(test)]
mod calendar_layout_review_tests {
    use super::*;
    use pathfinder_geometry::vector::vec2f;

    struct CalendarRowOnly {
        view: Companion,
        event: stepx_companion::toolbox::PhoneCalendarEvent,
    }
    impl facloud_ui::Entity for CalendarRowOnly { type Event = (); }
    impl facloud_ui::TypedActionView for CalendarRowOnly { type Action = (); }
    impl facloud_ui::View for CalendarRowOnly {
        fn ui_name() -> &'static str { "calendar_hover_layout_review" }
        fn render(&self, _: &facloud_ui::AppContext) -> El {
            let now = Local::now();
            Stack::new().with_child(SavePosition::new(self.view.render_calendar_event_row(
                &self.event, "all", now.date_naive(), now.timestamp_millis()), "calendar-row-probe").finish()).finish()
        }
    }

    #[test]
    fn calendar_hover_keeps_row_height_and_actions_inside_the_safe_margin() {
        facloud_ui::App::test((), |mut app| async move {
            for width in [440., 760.] {
                let start = Local::now().timestamp_millis() + 3_600_000;
                let event = stepx_companion::toolbox::PhoneCalendarEvent {
                    id: 71, title: "项目评审会议".into(), dtstart: start,
                    dtend: start + 3_600_000, recurring: Some(false), ..Default::default()
                };
                let mut companion = Companion::new_for_test();
                companion.back_screen.stepos_suite.calendar_events = vec![event.clone()];
                let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                    |_| CalendarRowOnly { view: companion, event });
                let root = app.root_view_id(window).unwrap();
                app.update(move |ctx| {
                    let mut presenter = facloud_ui::Presenter::new(window);
                    let invalidate = || facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]), ..Default::default()
                    };
                    presenter.invalidate(invalidate(), ctx);
                    presenter.build_scene(vec2f(width, 1200.), 1., None, ctx);
                    let before = presenter.position_cache().get_position("calendar-row-probe").unwrap();
                    let pointer = before.origin() + vec2f(20., 12.);
                    presenter.dispatch_event(facloud_ui::Event::MouseMoved {
                        position: pointer, cmd: false, shift: false, is_synthetic: false,
                    }, ctx);
                    presenter.invalidate(invalidate(), ctx);
                    presenter.build_scene(vec2f(width, 1200.), 1., None, ctx);
                    let positions = presenter.position_cache();
                    let after = positions.get_position("calendar-row-probe").unwrap();
                    assert!((before.height() - after.height()).abs() < 0.5,
                        "hover must not insert a row of actions: {before:?} -> {after:?}");
                    assert!(after.max_x() <= width + 0.5);
                    #[cfg(feature = "ui-capture")]
                    if std::env::var_os("STEPX_SCENARIO_DIR").is_some() {
                    for control in [format!("cal-all-edit-71:{start}"), format!("cal-all-del-71:{start}")] {
                        let rect = positions.get_position(&control).unwrap_or_else(|| panic!("missing {control}"));
                        assert!(rect.origin().x() >= after.origin().x() && rect.max_x() <= after.max_x());
                        assert!(rect.max_y() <= after.max_y() - S3 + 0.5,
                            "controls need bottom safe padding: {rect:?}/{after:?}");
                    }
                    }
                });
            }
        });
    }
}
