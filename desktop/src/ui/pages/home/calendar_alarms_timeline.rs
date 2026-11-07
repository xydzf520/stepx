#![allow(unused_imports)]
use super::*;
use crate::ui::pages::calendar_style::{day_domain, DayDomain, DayMarks};
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

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
enum MonthEntryKind {
    Calendar,
    Record,
    Alarm,
    Life,
}

impl MonthEntryKind {
    fn is_record(self) -> bool {
        matches!(self, Self::Record | Self::Life)
    }
}

#[derive(Clone)]
struct HomeMonthEntry {
    id: String,
    time: String,
    minute: u32,
    title: String,
    detail: String,
    kind: MonthEntryKind,
    domain: Option<DayDomain>,
    actions: Vec<Action>,
}

#[derive(Clone)]
pub(in crate::ui::pages) struct HomeMonthDay {
    pub(in crate::ui::pages) date: NaiveDate,
    pub(in crate::ui::pages) marks: DayMarks,
    entries: Vec<HomeMonthEntry>,
}

fn home_month_geometry_probe(id: impl Into<String>, element: El) -> El {
    let id = id.into();
    #[cfg(test)]
    {
        SavePosition::new(element, &id).for_single_frame().finish()
    }
    #[cfg(not(test))]
    {
        home_geometry_probe(id, element)
    }
}

fn home_month_dates(selected: NaiveDate) -> Vec<NaiveDate> {
    let first = selected.with_day(1).unwrap_or(selected);
    let next = if first.month() == 12 {
        NaiveDate::from_ymd_opt(first.year() + 1, 1, 1)
    } else {
        NaiveDate::from_ymd_opt(first.year(), first.month() + 1, 1)
    }
    .unwrap_or(first);
    let leading = first.weekday().num_days_from_monday() as usize;
    let days = (next - first).num_days().max(28) as usize;
    let cells = ((leading + days + 6) / 7).max(5).min(6) * 7;
    let start = first - ChronoDuration::days(leading as i64);
    (0..cells)
        .map(|i| start + ChronoDuration::days(i as i64))
        .collect()
}

pub(in crate::ui::pages) fn home_event_date_bounds(
    event: &stepx_companion::toolbox::PhoneCalendarEvent,
) -> Option<(NaiveDate, NaiveDate)> {
    let ((y, m, d), _, _) = stepx_companion::home_hub::timestamp_to_local_ymd_parts(event.dtstart)?;
    let first = NaiveDate::from_ymd_opt(y, m, d)?;
    // Calendar end times are exclusive. Do not mark the following date when an
    // event ends at midnight; multi-day all-day events retain every occupied day.
    let end_ms = if event.dtend > event.dtstart {
        event.dtend - 1
    } else {
        event.dtstart
    };
    let ((y, m, d), _, _) = stepx_companion::home_hub::timestamp_to_local_ymd_parts(end_ms)?;
    Some((first, NaiveDate::from_ymd_opt(y, m, d)?.max(first)))
}

fn home_work_event_end_ms(event: &stepx_companion::toolbox::PhoneCalendarEvent) -> i64 {
    if event.dtend > event.dtstart {
        event.dtend
    } else {
        event.dtstart.saturating_add(3_600_000)
    }
}

fn home_work_agenda<'a>(
    events: impl Iterator<Item = &'a stepx_companion::toolbox::PhoneCalendarEvent>,
    selected: NaiveDate,
    today: NaiveDate,
    now_ms: i64,
    limit: usize,
) -> (Vec<&'a stepx_companion::toolbox::PhoneCalendarEvent>, bool, usize) {
    let mut on_date = Vec::new();
    let mut upcoming = Vec::new();
    for event in events {
        let Some((first, last)) = home_event_date_bounds(event) else { continue };
        if first <= selected && selected <= last {
            on_date.push(event);
        } else if first > selected {
            upcoming.push(event);
        }
    }
    let fallback = on_date.is_empty() && !upcoming.is_empty();
    let mut events = if fallback { upcoming } else { on_date };
    // Rank before limiting: database IDs describe insertion order, not the
    // next meeting. Historical selections retain their chronological order.
    events.sort_by_key(|event| {
        let rank = if selected != today || fallback {
            0
        } else if event.dtstart <= now_ms && home_work_event_end_ms(event) > now_ms {
            0
        } else if event.dtstart > now_ms {
            1
        } else {
            2
        };
        (rank, event.dtstart, event.id)
    });
    let total = events.len();
    events.truncate(limit);
    (events, fallback, total)
}

fn home_work_location(event: &stepx_companion::toolbox::PhoneCalendarEvent) -> &str {
    let location = event.location.trim();
    if location.is_empty() { "地点未填写" } else { location }
}

#[cfg(test)]
mod home_work_agenda_tests {
    use super::*;
    use chrono::TimeZone;

    fn event(id: i64, day: u32, hour: u32, duration_hours: i64) -> stepx_companion::toolbox::PhoneCalendarEvent {
        let start = Local.with_ymd_and_hms(2026, 11, day, hour, 0, 0).single().unwrap().timestamp_millis();
        stepx_companion::toolbox::PhoneCalendarEvent { id, dtstart: start,
            dtend: start + duration_hours * 3_600_000, ..Default::default() }
    }

    #[test]
    fn work_agenda_prioritizes_live_and_next_events_before_limiting() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let now = event(0, 7, 10, 1).dtstart;
        let events = [event(900, 7, 8, 1), event(800, 7, 14, 1), event(1, 7, 9, 2), event(2, 7, 11, 1)];
        let (visible, fallback, total) = home_work_agenda(events.iter(), date, date, now, 2);
        assert_eq!(visible.iter().map(|event| event.id).collect::<Vec<_>>(), vec![1, 2]);
        assert!(!fallback);
        assert_eq!(total, 4);
        let (historical, _, _) = home_work_agenda(events.iter(), date, date.succ_opt().unwrap(), now, 4);
        assert_eq!(historical.iter().map(|event| event.id).collect::<Vec<_>>(), vec![900, 1, 2, 800]);
    }

    #[test]
    fn work_agenda_includes_overnight_events_and_sorts_future_fallback() {
        let date = NaiveDate::from_ymd_opt(2026, 11, 7).unwrap();
        let overnight = [event(1, 6, 23, 2), event(2, 6, 23, 1)];
        let now = event(0, 7, 0, 1).dtstart;
        let (visible, fallback, _) = home_work_agenda(overnight.iter(), date, date, now, 3);
        assert_eq!(visible.iter().map(|event| event.id).collect::<Vec<_>>(), vec![1]);
        assert!(!fallback);
        assert!(home_work_event_end_ms(visible[0]) > now);
        let future = [event(90, 9, 10, 1), event(2, 8, 15, 1), event(1, 8, 9, 1)];
        let (visible, fallback, total) = home_work_agenda(future.iter(), date, date, now, 2);
        assert!(fallback);
        assert_eq!(total, 3);
        assert_eq!(visible.iter().map(|event| event.id).collect::<Vec<_>>(), vec![1, 2]);
        assert_eq!(home_work_location(&future[0]), "地点未填写");
    }
}

fn home_alarm_on_date(
    alarm: &stepx_companion::toolbox::PhoneAlarmItem,
    date: NaiveDate,
    today: NaiveDate,
    now_minute: u32,
) -> bool {
    alarm.enabled
        && !alarm.is_e2e_probe()
        && date >= today
        && alarm.matches_weekday(date.weekday().num_days_from_monday())
        && (alarm.days_of_week != 0 || (date == today && alarm.total_minutes() >= now_minute))
}

fn home_month_preview(text: &str, user: bool) -> Option<String> {
    // Normalize before shortening. Transport feedback can use the "user" role
    // without being an actual user request, so also remove protocol envelopes.
    let visible = stepx_companion::work_protocol::display_text(text, user)?;
    let visible = stepx_companion::work_protocol::display_text(&visible, false)?;
    let visible = visible.split_whitespace().collect::<Vec<_>>().join(" ");
    let visible = ["你讨论过：", "记录更新为：", "讨论中提到："]
        .into_iter()
        .find_map(|prefix| visible.strip_prefix(prefix))
        .unwrap_or(&visible)
        .trim()
        .to_owned();
    (!visible.is_empty()
        && !visible.contains("<amoo_work_")
        && !visible.contains("</amoo_work_")
        && !visible.starts_with("<stepx_gui_receipt>"))
    .then_some(visible)
}

fn home_month_task_preview(task: &stepx_companion::amoo_workspace::Task) -> Option<String> {
    if let Some(output) = task.outputs.last() {
        return Some(format!(
            "已生成 {} 份文件 · {}",
            task.outputs.len(),
            output.name
        ));
    }
    home_month_preview(&task.draft, true)
        .or_else(|| {
            task.turns
                .iter()
                .rev()
                .filter(|turn| turn.user)
                .find_map(|turn| home_month_preview(&turn.text, true))
        })
        .or_else(|| {
            task.turns
                .iter()
                .rev()
                .filter(|turn| !turn.user)
                .find_map(|turn| home_month_preview(&turn.text, false))
        })
}

#[cfg(test)]
mod month_calendar_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn month_grid_covers_leap_days_and_year_boundaries_without_missing_dates() {
        for year in [2024, 2025, 2026, 2027] {
            for month in 1..=12 {
                let selected = NaiveDate::from_ymd_opt(year, month, 15).unwrap();
                let dates = home_month_dates(selected);
                assert!(matches!(dates.len(), 35 | 42));
                assert_eq!(dates[0].weekday(), chrono::Weekday::Mon);
                assert_eq!(dates.last().unwrap().weekday(), chrono::Weekday::Sun);
                assert!(dates
                    .windows(2)
                    .all(|pair| pair[1] - pair[0] == ChronoDuration::days(1)));
                for day in 1..=31 {
                    if let Some(date) = NaiveDate::from_ymd_opt(year, month, day) {
                        assert_eq!(
                            dates.iter().filter(|&&candidate| candidate == date).count(),
                            1
                        );
                    }
                }
            }
        }
        let february = home_month_dates(NaiveDate::from_ymd_opt(2024, 2, 1).unwrap());
        assert!(february.contains(&NaiveDate::from_ymd_opt(2024, 2, 29).unwrap()));
        assert_eq!(
            home_month_dates(NaiveDate::from_ymd_opt(2026, 11, 1).unwrap()).len(),
            42
        );
        assert_eq!(
            home_month_dates(NaiveDate::from_ymd_opt(2026, 2, 1).unwrap()).len(),
            35
        );
    }

    #[test]
    fn month_events_keep_overnight_days_but_exclude_midnight_end_date() {
        let start = Local
            .with_ymd_and_hms(2026, 11, 6, 23, 30, 0)
            .single()
            .unwrap();
        let midnight = Local
            .with_ymd_and_hms(2026, 11, 7, 0, 0, 0)
            .single()
            .unwrap();
        let mut event = stepx_companion::toolbox::PhoneCalendarEvent {
            dtstart: start.timestamp_millis(),
            dtend: midnight.timestamp_millis(),
            ..Default::default()
        };
        assert_eq!(
            home_event_date_bounds(&event),
            Some((start.date_naive(), start.date_naive()))
        );
        event.dtend += 3_600_000;
        assert_eq!(
            home_event_date_bounds(&event),
            Some((start.date_naive(), midnight.date_naive()))
        );
        event.all_day = true;
        event.dtstart = Local
            .with_ymd_and_hms(2026, 11, 5, 0, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        event.dtend = midnight.timestamp_millis();
        assert_eq!(
            home_event_date_bounds(&event),
            Some((
                NaiveDate::from_ymd_opt(2026, 11, 5).unwrap(),
                start.date_naive()
            ))
        );
        event.dtstart = 0;
        assert!(home_event_date_bounds(&event).is_none());
    }

    #[test]
    fn month_alarm_projection_does_not_invent_historical_or_one_shot_recurrence() {
        let friday = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let mut alarm = stepx_companion::toolbox::PhoneAlarmItem {
            enabled: true,
            hour: 9,
            days_of_week: 31,
            ..Default::default()
        };
        assert!(home_alarm_on_date(&alarm, friday, friday, 600));
        assert!(!home_alarm_on_date(
            &alarm,
            friday - ChronoDuration::days(1),
            friday,
            600
        ));
        assert!(!home_alarm_on_date(
            &alarm,
            friday + ChronoDuration::days(1),
            friday,
            600
        ));
        assert!(home_alarm_on_date(
            &alarm,
            friday + ChronoDuration::days(3),
            friday,
            600
        ));
        alarm.days_of_week = 0;
        assert!(home_alarm_on_date(&alarm, friday, friday, 500));
        assert!(!home_alarm_on_date(&alarm, friday, friday, 600));
        assert!(!home_alarm_on_date(
            &alarm,
            friday + ChronoDuration::days(1),
            friday,
            500
        ));
        alarm.enabled = false;
        assert!(!home_alarm_on_date(&alarm, friday, friday, 500));
    }

    #[test]
    fn month_rows_bind_original_calendar_ids_and_invalidate_changed_source() {
        let mut app = Companion::new_for_test();
        app.workspace.journal.tasks.clear();
        app.work.artifacts.clear();
        app.context.overview.items.clear();
        app.back_screen.stepos_suite.alarms.clear();
        app.back_screen.calendar_year = 2026;
        app.back_screen.calendar_month = 11;
        app.back_screen.selected_calendar_day = 6;
        let date = NaiveDate::from_ymd_opt(2026, 11, 6).unwrap();
        let timestamp = Local
            .with_ymd_and_hms(2026, 11, 6, 10, 0, 0)
            .single()
            .unwrap()
            .timestamp_millis();
        app.back_screen.stepos_suite.calendar_events =
            vec![stepx_companion::toolbox::PhoneCalendarEvent {
                id: 718,
                title: "项目评审".into(),
                dtstart: timestamp,
                dtend: timestamp + 3_600_000,
                ..Default::default()
            }];
        let data = app.home_month_days();
        let day = data.iter().find(|day| day.date == date).unwrap();
        assert_eq!(day.entries.len(), 1);
        assert_eq!(day.entries[0].title, "项目评审");
        assert_eq!(day.marks.domains().collect::<Vec<_>>(), [DayDomain::Work]);
        assert!(matches!(
            day.entries[0].actions.as_slice(),
            [
                Action::Page(Page::Calendar),
                Action::BackScreen(BackScreenAction::CalendarEditEvent(718))
            ]
        ));
        assert!(std::rc::Rc::ptr_eq(&data, &app.home_month_days()));
        app.back_screen.selected_calendar_day = 7;
        assert!(std::rc::Rc::ptr_eq(&data, &app.home_month_days()));
        app.back_screen.stepos_suite.calendar_events[0].title = "已调整的项目评审".into();
        let changed = app.home_month_days();
        assert_eq!(
            changed.iter().find(|day| day.date == date).unwrap().entries[0].title,
            "已调整的项目评审"
        );
        app.back_screen.stepos_suite.calendar_events.clear();
        assert!(app
            .home_month_days()
            .iter()
            .all(|day| day.entries.is_empty()));
    }
    #[test]
    fn month_task_preview_skips_internal_user_feedback_without_changing_source() {
        use stepx_companion::amoo_workspace::{Task, Turn};
        let request = "请整理本周会议纪要，保留负责人和截止日期。";
        let feedback = "<amoo_work_protocol>不能宣布完成：电脑没有收到文件写入请求。请返回 write_document 的 JSON 计划。</amoo_work_protocol>";
        let mut task = Task::new(None, None);
        task.title = "本周会议纪要".into();
        task.turns = vec![
            Turn {
                user: true,
                text: request.into(),
                at_ms: None,
            },
            Turn {
                user: true,
                text: feedback.into(),
                at_ms: None,
            },
        ];
        let original = task.clone();
        assert_eq!(home_month_task_preview(&task).as_deref(), Some(request));
        assert_eq!(task, original);
        assert!(home_month_preview(feedback, true).is_none());
        assert!(home_month_preview(&format!("你讨论过：{feedback}"), false).is_none());
        assert!(
            home_month_preview("<amoo_work_protocol>不能宣布完成：尚未收到文件…", true).is_none()
        );
        let ordinary = r#"普通说明：请核对 {"total": 42}"#;
        assert_eq!(
            home_month_preview(ordinary, true).as_deref(),
            Some(ordinary)
        );
        assert_eq!(home_month_preview(r#"<amoo_work_protocol>{"amoo_work":1,"summary":"已整理三项待办","tools":[]}</amoo_work_protocol>"#, false).as_deref(), Some("已整理三项待办"));

        let mut app = Companion::new_for_test();
        app.workspace.journal.tasks = vec![task];
        app.work.artifacts.clear();
        app.context.overview.items.clear();
        app.back_screen.stepos_suite.calendar_events.clear();
        app.back_screen.stepos_suite.alarms.clear();
        let today = Local::now().date_naive();
        app.back_screen.calendar_year = today.year();
        app.back_screen.calendar_month = today.month();
        app.back_screen.selected_calendar_day = today.day();
        let data = app.home_month_days();
        let entry = data
            .iter()
            .flat_map(|day| &day.entries)
            .find(|entry| entry.id == format!("task-{}", original.id))
            .unwrap();
        assert!(entry.detail.contains(request));
        assert!(!entry.detail.contains("amoo_work_protocol"));
        assert!(!entry.detail.contains("不能宣布完成"));
        assert_eq!(app.workspace.journal.tasks[0], original);
        // A historical turn may be filled in without changing the last
        // transport-user feedback, turn count, or task update timestamp.
        app.workspace.journal.tasks[0].turns[0].text =
            "补齐后的会议请求：请保留三位负责人。".into();
        let changed = app.home_month_days();
        assert!(!std::rc::Rc::ptr_eq(&data, &changed));
        let revised = changed
            .iter()
            .flat_map(|day| &day.entries)
            .find(|entry| entry.id == format!("task-{}", original.id))
            .unwrap();
        assert!(revised.detail.contains("补齐后的会议请求"));
    }

    #[test]
    fn month_calendar_keeps_all_42_days_in_narrow_and_short_windows() {
        struct MonthOnly(Companion);
        impl facloud_ui::Entity for MonthOnly {
            type Event = ();
        }
        impl facloud_ui::TypedActionView for MonthOnly {
            type Action = ();
        }
        impl facloud_ui::View for MonthOnly {
            fn ui_name() -> &'static str {
                "home_month_width_test"
            }
            fn render(&self, _: &facloud_ui::AppContext) -> El {
                // SavePosition registers inside the paint namespace supplied
                // by the real page's Stack/scrollable host.
                Stack::new().with_child(self.0.render_home_calendar_hub_and_panorama()).finish()
            }
        }
        facloud_ui::App::test((), |mut app| async move {
            for (width, height) in [(390., 560.), (540., 650.), (660., 650.), (746., 650.), (1200., 900.)] {
                let mut companion = Companion::new_for_test();
                companion.page = Page::Home;
                // The isolated view receives the content area, not the outer
                // desktop shell. Density still uses the real window budget.
                companion.workspace.viewport = vec2f(width + 278., height);
                let today = Local::now().date_naive();
                companion.back_screen.calendar_year = today.year();
                companion.back_screen.calendar_month = today.month();
                companion.back_screen.selected_calendar_day = today.day();
                let days = companion.home_month_days();
                assert_eq!(days.len(), 42);
                let day_ids = days.iter().map(|day|
                    format!("home-month-day-{}", day.date.format("%Y-%m-%d"))).collect::<Vec<_>>();
                let (window_id, _) = app
                    .add_window(facloud_ui::platform::WindowStyle::NotStealFocus, |_| {
                        MonthOnly(companion)
                    });
                let root = app.root_view_id(window_id).unwrap();
                let (calendar, agenda, title, create, card, day_rects) = app.update(move |ctx| {
                    let mut presenter = facloud_ui::Presenter::new(window_id);
                    presenter.invalidate(
                        facloud_ui::WindowInvalidation {
                            updated: std::collections::HashSet::from([root]),
                            ..Default::default()
                        },
                        ctx,
                    );
                    presenter.build_scene(vec2f(width, height), 1., None, ctx);
                    let positions = presenter.position_cache();
                    let rect = |id: &str| {
                        positions
                            .get_position(id)
                            .unwrap_or_else(|| panic!("missing month layout probe: {id}"))
                    };
                    (
                        rect("home-calendar-strip"),
                        rect("home-day-agenda-column"),
                        rect("home-month-agenda-heading"),
                        rect("home-month-create"),
                        rect("home-calendar-card"),
                        day_ids.iter().map(|id| rect(id)).collect::<Vec<_>>(),
                    )
                });
                for day in &day_rects {
                    assert!(day.origin().x() >= calendar.origin().x() - 1.
                        && day.max_x() <= calendar.max_x() + 1.
                        && day.origin().y() >= calendar.origin().y()
                        && day.max_y() <= calendar.max_y(),
                        "day escaped its full-month grid at {width}×{height}: {day:?}");
                }
                let expected_cell = if height < 740. { 32. } else { 38. };
                assert!((day_rects[0].height() - expected_cell - 2.).abs() < 1.,
                    "calendar uses height budget: {}", day_rects[0].height());
                if width > 700. {
                    assert!(
                        agenda.origin().x() >= calendar.max_x(),
                        "714px content must use two columns"
                    );
                    assert!((agenda.origin().y() - calendar.origin().y()).abs() < 1.);
                    assert!(
                        card.height() < if height < 740. { 380. } else { 500. },
                        "calendar must not become a tall stacked card: {}",
                        card.height()
                    );
                } else {
                    assert!(
                        agenda.origin().y() >= calendar.max_y(),
                        "narrow content must stack"
                    );
                }
                for rect in [title, create] {
                    assert!(
                        rect.origin().x() >= agenda.origin().x() - 1.
                            && rect.max_x() <= agenda.max_x() + 1.,
                        "agenda control overflow: {rect:?}, {agenda:?}"
                    );
                }
                assert!(
                    title.max_x() <= create.origin().x(),
                    "date heading cannot overlap new-schedule button"
                );
            }
        });
    }
}
impl Companion {
    pub(in crate::ui) fn home_local_output_count(&self) -> usize {
        if self.work.artifacts.is_empty() && self.workspace.journal.tasks.is_empty() {
            return 0;
        }
        self.work
            .artifacts
            .iter()
            .map(|f| &f.path)
            .chain(
                self.workspace
                    .journal
                    .tasks
                    .iter()
                    .filter(|task| self.workspace_visible(task))
                    .flat_map(|task| task.outputs.iter().map(|output| &output.path)),
            )
            .collect::<std::collections::HashSet<_>>()
            .len()
    }

    pub(in crate::ui) fn home_selected_date(&self) -> NaiveDate {
        let today = Local::now().date_naive();
        NaiveDate::from_ymd_opt(
            self.back_screen.calendar_year,
            self.back_screen.calendar_month,
            self.back_screen.selected_calendar_day.clamp(1, 31),
        )
        .unwrap_or(today)
    }

    pub(in crate::ui) fn extract_month_day_mentions(
        text: &str,
        default_year: i32,
    ) -> Vec<NaiveDate> {
        Self::extract_month_day_mentions_with_anchor(text, default_year, 0)
    }

    pub(in crate::ui) fn extract_month_day_mentions_with_anchor(
        text: &str,
        anchor_year: i32,
        anchor_month: u32,
    ) -> Vec<NaiveDate> {
        if !text.bytes().any(|b| b.is_ascii_digit()) {
            return Vec::new();
        }
        let mut out = Vec::new();
        let chars: Vec<char> = text.chars().collect();
        let mut i = 0;
        while i < chars.len() {
            // 1. Check for explicit 4-digit year: YYYY年M月D日 or YYYY-MM-DD / YYYY/MM/DD
            if i + 4 < chars.len()
                && (i == 0 || !chars[i - 1].is_ascii_digit())
                && chars[i..i + 4].iter().all(|c| c.is_ascii_digit())
            {
                let y_val = chars[i..i + 4]
                    .iter()
                    .fold(0i32, |acc, c| acc * 10 + c.to_digit(10).unwrap_or(0) as i32);
                let sep1 = chars[i + 4];
                if (2000..=2100).contains(&y_val) && (sep1 == '年' || sep1 == '-' || sep1 == '/') {
                    let mut j = i + 5;
                    let m_start = j;
                    let mut m_val: u32 = 0;
                    while j < chars.len() && chars[j].is_ascii_digit() && (j - m_start) < 2 {
                        m_val = m_val * 10 + chars[j].to_digit(10).unwrap_or(0);
                        j += 1;
                    }
                    let ok_sep2 = j > m_start
                        && j < chars.len()
                        && ((sep1 == '年' && chars[j] == '月')
                            || ((sep1 == '-' || sep1 == '/') && chars[j] == sep1));
                    if ok_sep2 && (1..=12).contains(&m_val) {
                        j += 1;
                        let d_start = j;
                        let mut d_val: u32 = 0;
                        while j < chars.len() && chars[j].is_ascii_digit() && (j - d_start) < 2 {
                            d_val = d_val * 10 + chars[j].to_digit(10).unwrap_or(0);
                            j += 1;
                        }
                        let valid_end = if sep1 == '年' {
                            j > d_start && j < chars.len() && (chars[j] == '日' || chars[j] == '号')
                        } else {
                            j > d_start && (j == chars.len() || !chars[j].is_ascii_digit())
                        };
                        if valid_end && (1..=31).contains(&d_val) {
                            if let Some(date) = NaiveDate::from_ymd_opt(y_val, m_val, d_val) {
                                if !out.contains(&date) {
                                    out.push(date);
                                }
                            }
                            i = j + usize::from(sep1 == '年');
                            continue;
                        }
                    }
                }
            }

            // 2. Check for M月D日 / M月D号 or M/D or MM-DD using anchor_year (resolving Dec<->Jan boundary)
            if chars[i].is_ascii_digit() && (i == 0 || !chars[i - 1].is_ascii_digit()) {
                let mut m_val: u32 = 0;
                let mut j = i;
                while j < chars.len() && chars[j].is_ascii_digit() && (j - i) < 2 {
                    m_val = m_val * 10 + chars[j].to_digit(10).unwrap_or(0);
                    j += 1;
                }
                if j < chars.len()
                    && (chars[j] == '月' || chars[j] == '/' || chars[j] == '-')
                    && (1..=12).contains(&m_val)
                {
                    let sep = chars[j];
                    j += 1;
                    let d_start = j;
                    let mut d_val: u32 = 0;
                    while j < chars.len() && chars[j].is_ascii_digit() && (j - d_start) < 2 {
                        d_val = d_val * 10 + chars[j].to_digit(10).unwrap_or(0);
                        j += 1;
                    }
                    let valid_end = if sep == '月' {
                        j > d_start && j < chars.len() && (chars[j] == '日' || chars[j] == '号')
                    } else {
                        j > d_start
                            && (i == 0
                                || (!chars[i - 1].is_ascii_alphanumeric()
                                    && chars[i - 1] != '/'
                                    && chars[i - 1] != '-'))
                            && (j == chars.len()
                                || (!chars[j].is_ascii_alphanumeric()
                                    && chars[j] != '/'
                                    && chars[j] != '-'))
                    };
                    if valid_end && (1..=31).contains(&d_val) {
                        let resolved_year = if anchor_month == 12 && m_val == 1 {
                            anchor_year + 1
                        } else if anchor_month == 1 && m_val == 12 {
                            anchor_year - 1
                        } else {
                            anchor_year
                        };
                        if let Some(date) = NaiveDate::from_ymd_opt(resolved_year, m_val, d_val) {
                            if !out.contains(&date) {
                                out.push(date);
                            }
                        }
                        i = j;
                        continue;
                    }
                }
            }
            i += 1;
        }
        out
    }

    pub(in crate::ui) fn strip_project_prefix(s: &str) -> &str {
        let trimmed = s.trim();
        if trimmed.starts_with('[') {
            if let Some(end_idx) = trimmed.find(']') {
                let rest = trimmed[end_idx + 1..].trim();
                if !rest.is_empty() {
                    return rest;
                }
            }
        }
        trimmed
    }

    pub(in crate::ui) fn extract_ledger_evidence_quotes(prompt: &str) -> Vec<String> {
        let section = if let Some(start) = prompt.find("<evidence>") {
            let after = &prompt[start + "<evidence>".len()..];
            if let Some(end) = after.find("</evidence>") {
                &after[..end]
            } else {
                after
            }
        } else {
            prompt
        };
        let mut quotes = Vec::new();
        for raw_line in section.lines() {
            let trimmed = raw_line.trim();
            if let Some(quote) = trimmed.strip_prefix('>') {
                let cleaned = Self::strip_project_prefix(&quote.replace("**", ""))
                    .split_whitespace()
                    .collect::<Vec<_>>()
                    .join(" ");
                if !cleaned.is_empty() && !quotes.contains(&cleaned) {
                    quotes.push(cleaned);
                }
            }
        }
        quotes
    }

    pub(in crate::ui) fn ledger_item_display_title(
        item: &stepx_companion::context_ledger::PreparedItem,
    ) -> String {
        let raw_title = item.title.trim();
        let is_ascii_slug = !raw_title.is_empty()
            && raw_title
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_');
        if is_ascii_slug {
            let clean_non_bracket = item
                .source_objects
                .iter()
                .map(|so| so.title.trim())
                .find(|t| !t.is_empty() && !t.starts_with('[') && !t.chars().all(|c| c.is_ascii()));
            let stripped_fallback = item
                .source_objects
                .iter()
                .map(|so| Self::strip_project_prefix(&so.title))
                .find(|t| !t.is_empty() && !t.chars().all(|c| c.is_ascii()));
            if let Some(human_src) = clean_non_bracket.or(stripped_fallback) {
                return format!("{human_src} · {raw_title}");
            }
        }
        raw_title.to_string()
    }

    pub(in crate::ui) fn ledger_item_display_detail(
        item: &stepx_companion::context_ledger::PreparedItem,
    ) -> String {
        let what = item
            .presentation
            .as_ref()
            .map(|p| Self::strip_project_prefix(&p.what_happened))
            .unwrap_or("");
        let is_generic_what =
            what.is_empty() || what.starts_with("已保留讨论资料") || what.contains("个来源上下文");
        if !is_generic_what {
            return what.to_string();
        }
        let quotes = Self::extract_ledger_evidence_quotes(&item.prompt);
        if let Some(best) = quotes
            .iter()
            .max_by_key(|q| q.chars().count())
            .filter(|q| !q.is_empty())
        {
            return best.clone();
        }
        item.summary.trim().to_string()
    }

    pub(in crate::ui) fn ledger_item_search_text(
        item: &stepx_companion::context_ledger::PreparedItem,
    ) -> String {
        let what = item
            .presentation
            .as_ref()
            .map(|p| p.what_happened.as_str())
            .unwrap_or("");
        let quotes = Self::extract_ledger_evidence_quotes(&item.prompt);
        let mut out = String::with_capacity(
            item.title.len()
                + item.summary.len()
                + item.status.len()
                + what.len()
                + item.source_objects.len() * 24
                + 64,
        );
        out.push_str(&item.title);
        out.push(' ');
        out.push_str(&item.summary);
        out.push(' ');
        out.push_str(&item.status);
        out.push(' ');
        out.push_str(what);
        out.push(' ');
        for (i, so) in item.source_objects.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(&so.source);
            out.push(' ');
            out.push_str(&so.title);
        }
        out.push(' ');
        for (i, q) in quotes.iter().enumerate() {
            if i > 0 {
                out.push(' ');
            }
            out.push_str(q);
        }
        out
    }

    pub(in crate::ui) fn is_synthetic_ledger_item(
        item: &stepx_companion::context_ledger::PreparedItem,
    ) -> bool {
        let what = item
            .presentation
            .as_ref()
            .map(|p| p.what_happened.as_str())
            .unwrap_or("");
        stepx_companion::home_hub::any_synthetic_e2e_probe(&[
            &item.id,
            &item.title,
            &item.summary,
            &item.status,
            &item.prompt,
            what,
        ]) || item
            .source_objects
            .iter()
            .any(|so| stepx_companion::home_hub::any_synthetic_e2e_probe(&[&so.source, &so.title]))
    }

    pub(in crate::ui) fn extract_ledger_calendar_start_ms(
        item: &stepx_companion::context_ledger::PreparedItem,
    ) -> Option<i64> {
        for so in &item.source_objects {
            if so.id.starts_with("calendar:") || so.source.contains("日历") {
                if let Some(tail) = so.id.rsplit(':').next() {
                    if let Ok(ms) = tail.trim().parse::<i64>() {
                        if ms > 100_000_000_000 {
                            return Some(ms);
                        }
                    }
                }
            }
        }
        None
    }

    pub(in crate::ui) fn is_pure_calendar_ledger_echo(
        item: &stepx_companion::context_ledger::PreparedItem,
    ) -> bool {
        !item.source_objects.is_empty()
            && item
                .source_objects
                .iter()
                .all(|so| so.id.starts_with("calendar:") || so.source.contains("日历"))
    }

    pub(in crate::ui) fn compute_desktop_entries_hash(
        &self,
        selected_date: NaiveDate,
        default_year: i32,
    ) -> u64 {
        use std::hash::{Hash, Hasher};
        let mut h = std::collections::hash_map::DefaultHasher::new();
        selected_date.year().hash(&mut h);
        selected_date.month().hash(&mut h);
        selected_date.day().hash(&mut h);
        default_year.hash(&mut h);
        self.selected
            .as_ref()
            .map(|d| d.serial.as_str())
            .hash(&mut h);
        self.amoo_ui.account_scope.hash(&mut h);
        self.workspace
            .running
            .as_ref()
            .map(|r| r.task.as_str())
            .hash(&mut h);
        self.workspace.journal.tasks.len().hash(&mut h);
        for t in &self.workspace.journal.tasks {
            t.id.hash(&mut h);
            t.updated.hash(&mut h);
            t.device.hash(&mut h);
            t.account.hash(&mut h);
            t.title.hash(&mut h);
            t.draft.hash(&mut h);
            t.outputs.len().hash(&mut h);
            for out in &t.outputs {
                out.path.hash(&mut h);
                out.name.hash(&mut h);
            }
            t.turns.len().hash(&mut h);
            if let Some(last_user_turn) = t.turns.iter().rev().find(|turn| turn.user) {
                last_user_turn.text.hash(&mut h);
            }
            t.steps.len().hash(&mut h);
            if let Some(last_step) = t.steps.last() {
                (last_step.phase as u8).hash(&mut h);
                last_step.note.hash(&mut h);
            }
        }
        self.work.artifacts.len().hash(&mut h);
        for a in &self.work.artifacts {
            a.path.hash(&mut h);
            a.name.hash(&mut h);
            a.kind.hash(&mut h);
        }
        self.context.overview.items.len().hash(&mut h);
        for it in &self.context.overview.items {
            it.id.hash(&mut h);
            it.updated_ms.hash(&mut h);
            it.title.hash(&mut h);
            it.summary.hash(&mut h);
            it.status.hash(&mut h);
            it.prompt.hash(&mut h);
            it.source_count.hash(&mut h);
            if let Some(ref pres) = it.presentation {
                pres.headline.hash(&mut h);
                pres.what_happened.hash(&mut h);
                pres.next_step.hash(&mut h);
                pres.priority.hash(&mut h);
                (pres.primary_action as u8).hash(&mut h);
            }
            it.source_objects.len().hash(&mut h);
            for so in &it.source_objects {
                so.id.hash(&mut h);
                so.source.hash(&mut h);
                so.title.hash(&mut h);
                so.task_id.hash(&mut h);
            }
        }
        self.back_screen
            .stepos_suite
            .calendar_events
            .len()
            .hash(&mut h);
        for ev in &self.back_screen.stepos_suite.calendar_events {
            ev.id.hash(&mut h);
            ev.title.hash(&mut h);
            ev.dtstart.hash(&mut h);
            ev.dtend.hash(&mut h);
            ev.location.hash(&mut h);
            ev.description.hash(&mut h);
        }
        self.sms.is_empty().hash(&mut h);
        self.push.snapshot.notifications.is_empty().hash(&mut h);
        self.push.snapshot.threads.is_empty().hash(&mut h);
        h.finish()
    }

    pub(in crate::ui) fn home_desktop_date_entries(&self) -> Vec<DesktopDateEntry> {
        use chrono::TimeZone;
        use stepx_companion::amoo_workspace::Phase;
        use stepx_companion::context_review::MatterAction;

        thread_local! {
            static HOME_DESKTOP_ENTRIES_CACHE: std::cell::RefCell<Option<(u64, Vec<DesktopDateEntry>)>> =
                const { std::cell::RefCell::new(None) };
        }

        let default_year = self.back_screen.calendar_year;
        let selected_date = self.home_selected_date();
        let cache_hash = self.compute_desktop_entries_hash(selected_date, default_year);
        if let Some(hit) = HOME_DESKTOP_ENTRIES_CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|(k, v)| (*k == cache_hash).then(|| v.clone()))
        }) {
            return hit;
        }

        let now = Local::now();
        let now_ts = now.timestamp();
        let mut entries = Vec::new();
        let mut seen_output_paths: std::collections::HashSet<&std::path::Path> =
            std::collections::HashSet::new();

        for task in self
            .workspace
            .journal
            .tasks
            .iter()
            .filter(|t| self.workspace_visible(t))
        {
            for out in &task.outputs {
                seen_output_paths.insert(out.path.as_path());
            }
            if stepx_companion::home_hub::is_synthetic_e2e_probe_text(&task.title)
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&task.draft)
            {
                continue;
            }
            if task.turns.is_empty()
                && task.draft.trim().is_empty()
                && task.outputs.is_empty()
                && task.steps.is_empty()
            {
                continue;
            }
            let ts = if task.updated > 0 {
                task.updated
            } else {
                now_ts
            };
            let last_phase = task.steps.last().map(|s| s.phase);
            let is_issue = matches!(last_phase, Some(Phase::Failed | Phase::Unconfirmed));
            let badge = if self
                .workspace
                .running
                .as_ref()
                .is_some_and(|run| run.task == task.id)
            {
                "执行中"
            } else if matches!(last_phase, Some(Phase::Failed)) {
                "需重试"
            } else if matches!(last_phase, Some(Phase::Unconfirmed)) {
                "待确认"
            } else if !task.outputs.is_empty() {
                "已有成果"
            } else if !task.draft.trim().is_empty() {
                "草稿留存"
            } else {
                "工作讨论"
            };
            let detail = if !task.outputs.is_empty() {
                format!(
                    "已生成 {} 份文件 · {}",
                    task.outputs.len(),
                    task.outputs
                        .last()
                        .map(|o| o.name.as_str())
                        .unwrap_or("成果文件")
                )
            } else if !task.draft.trim().is_empty() {
                Self::truncate_chars(task.draft.trim(), 64)
            } else {
                task.turns
                    .iter()
                    .rev()
                    .find(|t| t.user)
                    .map(|t| Self::truncate_chars(t.text.trim(), 64))
                    .unwrap_or_else(|| "跨端工作台连续任务".into())
            };
            entries.push(DesktopDateEntry {
                id: format!("task-{}", task.id),
                timestamp: ts,
                kind: if is_issue {
                    SelectedDateEntryKind::Issue
                } else {
                    SelectedDateEntryKind::WorkAndMatter
                },
                badge: badge.into(),
                title: Self::truncate_chars(&task.title, 48),
                detail,
                action_label: if is_issue {
                    "处理任务".into()
                } else {
                    "继续推进".into()
                },
                intent: HomeAiIntent::WorkPrompt(task.title.clone()),
            });
        }

        // Also include standalone local work artifacts (`self.work.artifacts`) not already in task outputs
        for (idx, art) in self.work.artifacts.iter().enumerate() {
            if !seen_output_paths.insert(art.path.as_path()) {
                continue;
            }
            let ts = crate::ui::widgets::cached_file_modified_ms(&art.path)
                .map(|ms| ms / 1000)
                .filter(|&s| s > 0)
                .unwrap_or(now_ts);
            entries.push(DesktopDateEntry {
                id: format!("work-artifact-{idx}"),
                timestamp: ts,
                kind: SelectedDateEntryKind::WorkAndMatter,
                badge: "本地成果".into(),
                title: Self::truncate_chars(&art.name, 48),
                detail: format!("{} · {}", art.kind, art.path.display()),
                action_label: "查看成果".into(),
                intent: HomeAiIntent::OpenPage("work".into()),
            });
        }

        let mut seen_ledger_ids: std::collections::HashSet<&str> = std::collections::HashSet::new();
        for item in &self.context.overview.items {
            if !seen_ledger_ids.insert(item.id.as_str())
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&item.id)
                || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&item.prompt)
            {
                continue;
            }
            let combined_text = Self::ledger_item_search_text(item);
            if stepx_companion::home_hub::is_synthetic_e2e_probe_text(&combined_text) {
                continue;
            }
            let cal_start_ms = Self::extract_ledger_calendar_start_ms(item);
            let is_pure_cal_echo = Self::is_pure_calendar_ledger_echo(item);
            let ts_secs = if let Some(cms) = cal_start_ms {
                cms / 1000
            } else if item.updated_ms > 1_000_000_000_000 {
                item.updated_ms / 1000
            } else if item.updated_ms > 0 {
                item.updated_ms
            } else {
                now_ts
            };
            let pres = item.presentation.as_ref();
            let display_title = Self::ledger_item_display_title(item);
            let display_detail = Self::ledger_item_display_detail(item);
            let is_calendar_source = is_pure_cal_echo
                || item
                    .source_objects
                    .iter()
                    .any(|so| so.source.contains("日历") || so.id.starts_with("calendar:"));

            // Skip deleted/missing-source ledger items or calendar-mirrored ledger items:
            // live phone calendar events are already rendered directly from `self.back_screen.stepos_suite.calendar_events`,
            // so deleted phone calendar events must never resurrect from `self.context.overview.items`.
            if item.status.contains("已不见") || is_calendar_source || is_pure_cal_echo {
                continue;
            }

            let is_weather_or_life = [
                "降雨",
                "下雨",
                "小雨",
                "暴雨",
                "天气",
                "取件",
                "快递",
                "订场地",
                "读书会",
                "自驾",
            ]
            .iter()
            .any(|kw| combined_text.contains(kw));
            let is_schedule = is_pure_cal_echo
                || pres.is_some_and(|p| p.primary_action == MatterAction::PrepareSchedule);
            let is_issue = !is_pure_cal_echo
                && (pres.is_some_and(|p| p.priority >= 80)
                    || item.status.contains("待")
                    || item.status.contains("需核对")
                    || item.status.contains("变更")
                    || item.status.contains("未成功"));
            let kind = if is_schedule {
                SelectedDateEntryKind::Schedule
            } else if is_issue && !is_weather_or_life {
                SelectedDateEntryKind::Issue
            } else if is_weather_or_life {
                SelectedDateEntryKind::Life
            } else if is_issue {
                SelectedDateEntryKind::Issue
            } else {
                SelectedDateEntryKind::WorkAndMatter
            };
            let badge_str = if is_pure_cal_echo {
                "日程同步".to_string()
            } else if !item.status.trim().is_empty() {
                Self::truncate_chars(item.status.trim(), 16)
            } else {
                format!("台账 · {}条来源", item.source_count.max(1))
            };
            let intent = HomeAiIntent::WorkPrompt(format!(
                "围绕事项「{}」继续跟进：{}",
                display_title,
                Self::truncate_chars(&display_detail, 72)
            ));

            let primary_date = chrono::DateTime::from_timestamp(ts_secs, 0)
                .map(|dt| dt.with_timezone(&Local).date_naive());
            let mut mentioned_dates = Self::extract_month_day_mentions_with_anchor(
                &combined_text,
                selected_date.year(),
                selected_date.month(),
            );
            if let Some(pd) = primary_date {
                if (pd.year(), pd.month()) != (selected_date.year(), selected_date.month()) {
                    for d in Self::extract_month_day_mentions_with_anchor(
                        &combined_text,
                        pd.year(),
                        pd.month(),
                    ) {
                        if !mentioned_dates.contains(&d) {
                            mentioned_dates.push(d);
                        }
                    }
                }
            }
            if default_year != selected_date.year() || matches!(selected_date.month(), 1 | 12) {
                for d in Self::extract_month_day_mentions(&combined_text, default_year) {
                    if !mentioned_dates.contains(&d) {
                        mentioned_dates.push(d);
                    }
                }
            }

            let is_only_phone_msg_source = !item.source_objects.is_empty()
                && item.source_objects.iter().all(|so| {
                    so.source.contains("短信")
                        || so.source.contains("通知")
                        || so.source.contains("消息")
                })
                && (!self.sms.is_empty()
                    || !self.push.snapshot.notifications.is_empty()
                    || !self.push.snapshot.threads.is_empty());

            // For calendar-sourced or schedule-tagged items with explicit event dates, only place them on their
            // scheduled date(s) rather than the ledger sync timestamp (`ts_secs`). Also skip raw phone-msg mirrors
            // on primary_date that are already included from push/sms.
            let include_primary_ts = !is_only_phone_msg_source
                && (cal_start_ms.is_some()
                    || !(is_calendar_source || is_schedule)
                    || mentioned_dates.is_empty()
                    || primary_date.is_some_and(|pd| mentioned_dates.contains(&pd)));
            if include_primary_ts {
                entries.push(DesktopDateEntry {
                    id: format!("ledger-{}", item.id),
                    timestamp: ts_secs,
                    kind,
                    badge: badge_str.clone(),
                    title: Self::truncate_chars(&display_title, 48),
                    detail: Self::truncate_chars(&display_detail, 68),
                    action_label: "跟进事项".into(),
                    intent: intent.clone(),
                });
            }

            // Also project explicit YYYY年M月D日 / M月D日 / M/D / MM-DD mentions onto those calendar dates
            for mentioned_date in mentioned_dates {
                if include_primary_ts && Some(mentioned_date) == primary_date {
                    continue;
                }
                if let Some(dt_local) = mentioned_date
                    .and_hms_opt(9, 0, 0)
                    .and_then(|ndt| Local.from_local_datetime(&ndt).single())
                {
                    entries.push(DesktopDateEntry {
                        id: format!("ledger-{}-{}", item.id, mentioned_date.format("%Y%m%d")),
                        timestamp: dt_local.timestamp(),
                        kind: if is_weather_or_life {
                            SelectedDateEntryKind::Life
                        } else {
                            kind
                        },
                        badge: format!("关联节点 · {}", badge_str),
                        title: Self::truncate_chars(&display_title, 48),
                        detail: Self::truncate_chars(&display_detail, 68),
                        action_label: "查看关联".into(),
                        intent: intent.clone(),
                    });
                }
            }
        }

        HOME_DESKTOP_ENTRIES_CACHE.with(|c| {
            *c.borrow_mut() = Some((cache_hash, entries.clone()));
        });
        entries
    }

    pub(in crate::ui) fn enrich_report_with_ledger_context(
        &self,
        report: &mut stepx_companion::home_hub::HomeHubReport,
    ) {
        use stepx_companion::home_reminder_governance::ReminderCategory;
        let _selected_date = self.home_selected_date();
        let now_ms = Local::now().timestamp_millis();
        let weather_ttl_ms = self
            .reminder_governance
            .effective_ttl_ms(ReminderCategory::WeatherAndLife);
        let genuine_items: Vec<(&stepx_companion::context_ledger::PreparedItem, String)> = self
            .context
            .overview
            .items
            .iter()
            .filter_map(|it| {
                if it.status.contains("已不见")
                    || it.status.contains("已完成")
                    || it.status.contains("已结项")
                    || it.status.contains("已归档")
                    || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&it.id)
                    || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&it.prompt)
                {
                    return None;
                }
                let l_key = format!("ledger:{}", it.id);
                if (it.updated_ms > 0
                    && now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(it.updated_ms) > weather_ttl_ms
                    && !self.reminder_governance.is_force_kept(&l_key, now_ms))
                    || self.reminder_governance.is_any_key_suppressed(
                        &[l_key],
                        (it.updated_ms > 0).then_some(it.updated_ms),
                        None,
                        None,
                        now_ms,
                    )
                {
                    return None;
                }
                let text = Self::ledger_item_search_text(it);
                if stepx_companion::home_hub::is_synthetic_e2e_probe_text(&text) {
                    return None;
                }
                let is_cal = Self::is_pure_calendar_ledger_echo(it)
                    || it
                        .source_objects
                        .iter()
                        .any(|so| so.source.contains("日历") || so.id.starts_with("calendar:"));
                if is_cal {
                    return None;
                }
                Some((it, text))
            })
            .collect();
        if genuine_items.is_empty() {
            return;
        }

        let weather_item = genuine_items
            .iter()
            .find(|(_, text)| {
                ["降雨", "下雨", "小雨", "暴雨", "大风", "降温", "天气"]
                    .iter()
                    .any(|kw| text.contains(kw))
            })
            .map(|(it, _)| *it);
        let outdoor_or_trip_item = genuine_items
            .iter()
            .find(|(_, text)| {
                ["自驾", "出行", "出发", "读书会", "活动", "订场地", "外出"]
                    .iter()
                    .any(|kw| text.contains(kw))
            })
            .map(|(it, _)| *it);
        // Exclude pure calendar-mirrored items that already exist in `suite.calendar_events`
        // so `要愉悦` is not overwritten into a duplicate of the calendar schedule card.
        let leisure_item = genuine_items
            .iter()
            .filter(|(it, _)| {
                let is_pure_cal_mirror = !it.source_objects.is_empty()
                    && it
                        .source_objects
                        .iter()
                        .all(|so| so.source.contains("日历"));
                !is_pure_cal_mirror
            })
            .find(|(_, text)| {
                ["读书会", "订场地", "预约", "报名", "自驾", "聚餐", "活动"]
                    .iter()
                    .any(|kw| text.contains(kw))
            })
            .map(|(it, _)| *it);

        for card in &mut report.life_context_cards {
            match card.domain {
                HomeLifeDomain::WeatherAndCommute => {
                    if card.badge.contains("×") {
                        if let Some(w_it) = weather_item {
                            let w_title = Self::ledger_item_display_title(w_it);
                            let ev_detail = Self::ledger_item_display_detail(w_it);
                            card.title = format!("{w_title} · 自驾天气");
                            if !ev_detail.trim().is_empty() {
                                card.detail = Self::truncate_chars(&ev_detail, 32);
                            }
                        }
                        continue;
                    }
                    if let Some(w_it) = weather_item {
                        let w_title = Self::ledger_item_display_title(w_it);
                        let ev_detail = Self::ledger_item_display_detail(w_it);
                        card.badge = "降雨与出行提醒".into();
                        card.title = format!("{w_title} · 降雨与出行安排");
                        card.detail = Self::truncate_chars(&ev_detail, 76);
                        card.context_link = if let Some(trip) = outdoor_or_trip_item {
                            let trip_title = Self::ledger_item_display_title(trip);
                            format!(
                                "关联出行「{}」· 天气记录「{}」，建议备伞并提前规划路线",
                                Self::truncate_chars(&trip_title, 18),
                                Self::truncate_chars(&ev_detail, 28)
                            )
                        } else {
                            format!(
                                "关联天气记录「{}」，建议外出备伞并在背屏开启天气提醒",
                                Self::truncate_chars(&ev_detail, 36)
                            )
                        };
                    }
                }
                HomeLifeDomain::LeisureAndWellbeing => {
                    if let Some(l_it) = leisure_item {
                        let l_title = Self::ledger_item_display_title(l_it);
                        let l_detail = Self::ledger_item_display_detail(l_it);
                        let status_hint = if !l_it.status.trim().is_empty() {
                            l_it.status.as_str()
                        } else {
                            "进行中"
                        };
                        card.badge = "活动与日程衔接".into();
                        card.title = format!("{l_title} · 预约与日常安排");
                        card.detail = Self::truncate_chars(&l_detail, 76);
                        card.context_link = format!(
                            "关联事项「{}」（{}），可跟进场地预订与行前准备",
                            Self::truncate_chars(&l_title, 20),
                            Self::truncate_chars(status_hint, 24)
                        );
                        card.action_label = "跟进筹备".into();
                        card.intent = HomeAiIntent::WorkPrompt(format!(
                            "围绕事项「{}」继续跟进：{}",
                            l_title,
                            Self::truncate_chars(&l_detail, 72)
                        ));
                    }
                }
                HomeLifeDomain::ExpressAndPickup => {
                    if let Some(w_it) = weather_item {
                        let ev_detail = Self::ledger_item_display_detail(w_it);
                        if !card.context_link.contains("雨") {
                            let base = card
                                .context_link
                                .split('：')
                                .next()
                                .unwrap_or(&card.context_link)
                                .trim();
                            card.context_link = format!(
                                "{} · 天气：{}",
                                Self::truncate_chars(base, 36),
                                Self::truncate_chars(&ev_detail, 24)
                            );
                        }
                    }
                }
            }
        }
    }

    pub(in crate::ui) fn with_home_hub_input<R>(
        &self,
        snapshot: &PushSnapshot,
        express_parcels: &[stepx_companion::device::ExpressParcelInfo],
        f: impl FnOnce(&HomeHubInput<'_>, &str, &str) -> R,
    ) -> R {
        let now = Local::now();
        let today = now.date_naive();
        let today_ymd = format!(
            "{:04}-{:02}-{:02}",
            today.year(),
            today.month(),
            today.day()
        );
        let sel = self.home_selected_date();
        let selected_ymd = format!("{:04}-{:02}-{:02}", sel.year(), sel.month(), sel.day());
        let suite = &self.back_screen.stepos_suite;
        let remote_file_paths: Vec<String> = suite
            .remote_agent_files
            .iter()
            .filter(|f| !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&f.path))
            .map(|f| f.path.clone())
            .collect();
        let work_deliverables_count = self.home_local_output_count() + remote_file_paths.len();
        let pomodoro_remaining_secs = self.live_pomodoro_remaining_secs();
        let cached_metrics = (!self.summary.is_empty()).then(|| self.cached_device_metrics());
        let device_model = self
            .selected
            .as_ref()
            .map(|d| d.model.as_str())
            .unwrap_or("StepX Phone");
        let firmware_profile = if let Some(m) = cached_metrics
            .as_ref()
            .filter(|m| !m.firmware_version.is_empty())
        {
            m.firmware_version.as_str()
        } else if !suite.firmware_profile.is_empty() {
            suite.firmware_profile.as_str()
        } else {
            "StepOS"
        };

        let input = HomeHubInput {
            hour: now.hour(),
            minute: now.minute(),
            today_ymd: &today_ymd,
            device_connected: self.selected.is_some(),
            device_model,
            firmware_profile,
            threads: &snapshot.threads,
            notifications: &snapshot.notifications,
            otp_banners: &snapshot.otp_banners,
            sms: &self.sms,
            call_logs: &self.call_logs,
            express_parcels,
            calendar_events: &suite.calendar_events,
            alarms: &suite.alarms,
            wellbeing: &suite.wellbeing,
            phone_clipboard: &suite.phone_clipboard,
            remote_agent_files: &remote_file_paths,
            work_deliverables_count,
            pomodoro_running: self.back_screen.pomodoro_active,
            pomodoro_remaining_secs,
            dnd_on: suite.controls.dnd_on,
        };
        f(&input, &today_ymd, &selected_ymd)
    }

    pub(in crate::ui) fn build_home_panorama_and_weekly(
        &self,
        snapshot: &PushSnapshot,
        express_parcels: &[stepx_companion::device::ExpressParcelInfo],
    ) -> (
        stepx_companion::home_hub::SelectedDatePanorama,
        stepx_companion::home_hub::WeeklyOverviewReport,
    ) {
        self.with_home_hub_input(
            snapshot,
            express_parcels,
            |input, _today_ymd, selected_ymd| {
                let desktop_entries = self.home_desktop_date_entries();
                let panorama = build_selected_date_panorama(selected_ymd, input, &desktop_entries);
                let weekly = build_weekly_overview_report(selected_ymd, input, &desktop_entries);
                (panorama, weekly)
            },
        )
    }

    pub(in crate::ui) fn build_report_from_input(
        &self,
        input: &HomeHubInput<'_>,
        today_ymd: &str,
        selected_ymd: &str,
        desktop_entries: &[DesktopDateEntry],
    ) -> stepx_companion::home_hub::HomeHubReport {
        use std::hash::{Hash, Hasher};

        thread_local! {
            static HOME_ENRICHED_REPORT_CACHE: std::cell::RefCell<
                Option<(u64, stepx_companion::home_hub::HomeHubReport)>,
            > = const { std::cell::RefCell::new(None) };
        }

        let now_ms = Local::now().timestamp_millis().max(0) as u64;
        let cache_key = {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            stepx_companion::home_hub::compute_home_hub_input_hash(
                99,
                selected_ymd,
                input,
                desktop_entries,
            )
            .hash(&mut h);
            self.compute_desktop_entries_hash(
                self.home_selected_date(),
                self.back_screen.calendar_year,
            )
            .hash(&mut h);
            today_ymd.hash(&mut h);
            (now_ms / 60_000).hash(&mut h);
            self.demo_mode.hash(&mut h);
            self.selected.is_some().hash(&mut h);
            self.reminder_governance.revision.hash(&mut h);
            h.finish()
        };

        if let Some(hit) = HOME_ENRICHED_REPORT_CACHE.with(|c| {
            c.borrow()
                .as_ref()
                .and_then(|(k, v)| (*k == cache_key).then(|| v.clone()))
        }) {
            return hit;
        }

        let mut report = build_home_hub_report(input);
        let fresh_unread_sms = input
            .sms
            .iter()
            .filter(|m| {
                m.kind == 1
                    && !m.read
                    && now_ms.saturating_sub(m.date_ms.max(0) as u64) <= 12 * 3600 * 1000
                    && !stepx_companion::home_hub::any_synthetic_e2e_probe(&[&m.address, &m.body])
            })
            .count();
        let unread_threads_sum: usize = input
            .threads
            .iter()
            .filter(|t| {
                !t.title.trim_start().starts_with('{')
                    && !stepx_companion::push::is_excluded_im_package(&t.package_name)
                    && !stepx_companion::home_hub::any_synthetic_e2e_probe(&[
                        &t.app_name,
                        &t.title,
                        &t.last_message,
                    ])
            })
            .map(|t| t.unread_count)
            .sum();
        report.unread_messages_count = unread_threads_sum + fresh_unread_sms;
        report.life_context_cards = if selected_ymd == today_ymd {
            stepx_companion::home_hub::enrich_life_context_cards_with_desktop(
                report.life_context_cards,
                selected_ymd,
                input,
                desktop_entries,
            )
        } else {
            stepx_companion::home_hub::build_life_context_cards_with_desktop(
                selected_ymd,
                input,
                desktop_entries,
            )
        };
        if report.schedule_conflicts.is_empty()
            && (selected_ymd > today_ymd || (self.demo_mode && selected_ymd != today_ymd))
        {
            report.schedule_conflicts = stepx_companion::home_hub::detect_schedule_alarm_conflicts(
                selected_ymd,
                0,
                0,
                input.calendar_events,
                input.alarms,
            );
        }
        self.enrich_report_with_ledger_context(&mut report);
        if !self.demo_mode && self.selected.is_none() {
            report.scene.title = "把正在做的事继续下去".into();
            report.scene.badge = "电脑工作区".into();
            report.scene.subtitle =
                "手机离线，可继续整理草稿和查看本地成果；手机信息以最后快照为准。".into();
        }
        HOME_ENRICHED_REPORT_CACHE.with(|c| {
            *c.borrow_mut() = Some((cache_key, report.clone()));
        });
        report
    }

    /// Builds the live Home hub report and hands it (plus the effective push snapshot and
    /// parcel list, which fall back to demo fixtures when the phone has none) to `f`.
    pub(in crate::ui) fn with_home_report<R>(
        &self,
        f: impl FnOnce(
            &stepx_companion::home_hub::HomeHubReport,
            &PushSnapshot,
            &[stepx_companion::device::ExpressParcelInfo],
        ) -> R,
    ) -> R {
        let demo_push: PushSnapshot;
        let snapshot: &PushSnapshot = if self.demo_mode
            && self.push.snapshot.notifications.is_empty()
            && self.push.snapshot.threads.is_empty()
        {
            demo_push = demo_push_snapshot();
            &demo_push
        } else {
            &self.push.snapshot
        };

        let fallback_parcels;
        let express_parcels = if self.demo_mode && self.express_parcels.is_empty() {
            fallback_parcels = default_express_parcels();
            &fallback_parcels
        } else {
            &self.express_parcels
        };

        self.with_home_hub_input(
            snapshot,
            express_parcels,
            |input, today_ymd, selected_ymd| {
                let desktop_entries = self.home_desktop_date_entries();
                let report =
                    self.build_report_from_input(input, today_ymd, selected_ymd, &desktop_entries);
                f(&report, snapshot, express_parcels)
            },
        )
    }

    pub(in crate::ui) fn current_backscreen_signature<'a>(
        &'a self,
        report: &'a stepx_companion::home_hub::HomeHubReport,
    ) -> &'a str {
        let custom = self.fields[F_BACK_MARQUEE].text.trim();
        let raw = if !custom.is_empty() && custom != "STEPX AI" {
            custom
        } else if !self
            .back_screen
            .stepos_suite
            .backscreen_signature
            .trim()
            .is_empty()
        {
            self.back_screen.stepos_suite.backscreen_signature.trim()
        } else {
            report.scene.suggested_backscreen_status.as_str()
        };
        let stripped = raw.trim_start_matches(|c: char| {
            c.is_whitespace()
                || (c as u32) >= 0x1F000
                || matches!(c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
        });
        if stripped.is_empty() {
            raw
        } else {
            stripped
        }
    }

    /// Compact, privacy-reduced context for the phone's Amoo (no OTP / pickup codes).
    pub(in crate::ui) fn home_ai_context(&self) -> stepx_companion::home_ai::HomeAiContext {
        self.with_home_report(|report, snapshot, express_parcels| {
            let suite = &self.back_screen.stepos_suite;
            let now = Local::now();
            let weekday = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
                [now.weekday().num_days_from_monday() as usize];
            let mut threads: Vec<_> = snapshot
                .threads
                .iter()
                .filter(|t| {
                    !t.title.trim_start().starts_with('{')
                        && !stepx_companion::push::is_excluded_im_package(&t.package_name)
                        && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!(
                            "{} {} {}",
                            t.app_name, t.title, t.last_message
                        ))
                })
                .collect();
            threads.sort_by_key(|t| {
                (
                    std::cmp::Reverse(t.unread_count),
                    std::cmp::Reverse(t.last_timestamp_ms),
                )
            });
            let today = now.date_naive();
            let genuine_cal: Vec<_> = suite
                .calendar_events
                .iter()
                .filter(|ev| {
                    !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!(
                        "{} {} {}",
                        ev.title, ev.location, ev.description
                    ))
                })
                .cloned()
                .collect();
            let mut events: Vec<(String, String)> = today_event_spans(&genuine_cal, today)
                .into_iter()
                .map(|(s, _, ev)| (format!("{:02}:{:02}", s / 60, s % 60), ev.title.clone()))
                .collect();
            if events.is_empty() {
                events = genuine_cal
                    .iter()
                    .filter(|ev| {
                        let (ey, em, ed) = ev.date_ymd();
                        ey == today.year() && em == today.month() && ed == today.day()
                    })
                    .take(3)
                    .map(|ev| (ev.time_range_label(), ev.title.clone()))
                    .collect();
            }
            let ledger_items: Vec<(String, String)> = self
                .context
                .overview
                .items
                .iter()
                .filter(|it| {
                    !Self::is_synthetic_ledger_item(it) && !Self::is_pure_calendar_ledger_echo(it)
                })
                .take(5)
                .map(|it| (Self::ledger_item_display_title(it), it.status.clone()))
                .collect();
            let sms_cards: Vec<(String, String)> = report
                .priority_cards
                .iter()
                .filter(|c| {
                    matches!(
                        c.kind,
                        HomeCardKind::OtpAndPickup
                            | HomeCardKind::TakeoutAndDelivery
                            | HomeCardKind::ExpressParcel
                    )
                })
                .take(5)
                .map(|c| (c.badge.clone(), c.title.clone()))
                .collect();
            let pending_phone_tasks: Vec<(String, String)> = self
                .workspace
                .journal
                .tasks
                .iter()
                .filter(|t| t.id.starts_with("phone-work-") && self.workspace_visible(t))
                .take(5)
                .map(|t| (t.title.clone(), t.note.clone()))
                .collect();
            // Share the capsule's reconciled per-order direction evidence instead
            // of reducing every dispatch notification to "the user must receive".
            let signals = self.fulfillment_signals(now.timestamp_millis());
            let mut parcel_evidence: Vec<_> = signals.iter()
                .filter(|s| s.kind != stepx_companion::capsule_fulfillment::Kind::Travel
                    && s.current(now.timestamp_millis(), 72 * 3_600_000)
                    && !self.push.parcel_receipts.retires_capsule(s, now.timestamp_millis())
                    && !s.tracking.as_ref().is_some_and(|number| self.push.parcel_receipts.is_completed("", number, s.at)))
                .take(8)
                .map(stepx_companion::home_ai::HomeAiParcelEvidence::from_signal)
                .collect();
            for parcel in express_parcels.iter().take(4) {
                if parcel_evidence.len() >= 8 || signals.iter().any(|s| s.tracking.as_deref() == Some(parcel.tracking_no.as_str())) { continue; }
                let direction = stepx_companion::capsule_fulfillment::parcel_direction(&format!("{} {}", parcel.status, parcel.station));
                parcel_evidence.push(stepx_companion::home_ai::HomeAiParcelEvidence {
                    id: stepx_companion::capsule_fulfillment::digest(&["home-parcel", &parcel.carrier, &parcel.tracking_no, &parcel.pickup_code]),
                    provider: parcel.carrier.clone(), status: parcel.status.clone(), direction,
                    pickup_ready: !parcel.pickup_code.trim().is_empty()
                        && ["待取", "到站", "驿站", "柜机"].iter().any(|stage| parcel.status.contains(stage))
                        && direction == stepx_companion::capsule_fulfillment::Direction::Receive,
                    completed: stepx_companion::home_hub::is_completed_express_text(&parcel.status),
                });
            }
            stepx_companion::home_ai::HomeAiContext {
                now_label: format!(
                    "{} {weekday} {}",
                    now.format("%Y-%m-%d"),
                    now.format("%H:%M")
                ),
                scene: report.scene.badge.clone(),
                device_model: self
                    .selected
                    .as_ref()
                    .map(|d| d.model.clone())
                    .unwrap_or_default(),
                unread_total: report.unread_messages_count,
                threads: threads
                    .into_iter()
                    .take(5)
                    .map(|t| {
                        let (_, channel, _, _) =
                            channel_avatar_spec(&t.package_name, &t.app_name, &self.p());
                        let snippet = if t.last_sender.trim().is_empty() {
                            t.last_message.clone()
                        } else {
                            format!("{}：{}", t.last_sender.trim(), t.last_message)
                        };
                        (channel, t.title.clone(), snippet, t.unread_count as u32)
                    })
                    .collect(),
                missed_calls: self
                    .call_logs
                    .iter()
                    .filter(|c| {
                        c.call_type == 3
                            && (c.date_ms <= 0
                                || Local::now().timestamp_millis().saturating_sub(c.date_ms)
                                    <= 86_400_000)
                    })
                    .count(),
                parcels: express_parcels
                    .iter()
                    .take(4)
                    .map(|p| (p.carrier.clone(), p.status.clone()))
                    .collect(),
                parcel_evidence,
                events,
                alarms: suite
                    .alarms
                    .iter()
                    .take(5)
                    .map(|a| {
                        format!(
                            "{} {}",
                            a.time_label(),
                            if a.enabled { "已开启" } else { "已关闭" }
                        )
                    })
                    .collect(),
                screen_minutes: suite.wellbeing.screen_time_minutes,
                unlocks: suite.wellbeing.unlock_count,
                top_apps: suite
                    .wellbeing
                    .top_apps
                    .iter()
                    .take(4)
                    .map(|(pkg, label, mins)| {
                        (resolve_top_app_display_name(pkg, label).to_string(), *mins)
                    })
                    .collect(),
                dnd_on: suite.controls.dnd_on,
                pomodoro_running: self.back_screen.pomodoro_active,
                backscreen_status: self.current_backscreen_signature(report).to_string(),
                clipboard_hint: if stepx_companion::home_hub::is_synthetic_e2e_probe_text(
                    &suite.phone_clipboard,
                ) {
                    String::new()
                } else {
                    suite.phone_clipboard.clone()
                },
            }
            .with_synergy_context(ledger_items, sms_cards, pending_phone_tasks)
        })
    }

    // =========================================================================
    // SECTION 1: 全月日历与所选日期的真实安排
    // =========================================================================
    pub(in crate::ui::pages) fn home_month_days(&self) -> std::rc::Rc<Vec<HomeMonthDay>> {
        use std::hash::{Hash, Hasher};
        thread_local! {
            static MONTH_CACHE: std::cell::RefCell<Option<(u64, std::rc::Rc<Vec<HomeMonthDay>>)>> = const { std::cell::RefCell::new(None) };
        }
        let now = Local::now();
        let selected = self.home_selected_date();
        let suite = &self.back_screen.stepos_suite;
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        self.compute_desktop_entries_hash(selected.with_day(1).unwrap_or(selected), selected.year())
            .hash(&mut hasher);
        // Preview can fall back past an internal last-user turn. Hash the
        // selected display value as well, then reuse it on a cache miss.
        let task_previews: std::collections::HashMap<_, _> = self
            .workspace
            .journal
            .tasks
            .iter()
            .map(|task| {
                let preview = home_month_task_preview(task);
                task.id.hash(&mut hasher);
                preview.hash(&mut hasher);
                (task.id.as_str(), preview)
            })
            .collect();
        (now.timestamp() / 60).hash(&mut hasher);
        self.reminder_governance.revision.hash(&mut hasher);
        for id in &self.back_screen.completed_calendar_events {
            id.hash(&mut hasher);
        }
        for event in &suite.calendar_events {
            event.all_day.hash(&mut hasher);
            event.source.hash(&mut hasher);
            event.recurring.hash(&mut hasher);
        }
        for alarm in &suite.alarms {
            (
                alarm.id,
                alarm.hour,
                alarm.minutes,
                alarm.enabled,
                alarm.days_of_week,
                &alarm.message,
            )
                .hash(&mut hasher);
        }
        let key = hasher.finish();
        if let Some(hit) = MONTH_CACHE.with(|cache| {
            cache
                .borrow()
                .as_ref()
                .and_then(|(k, v)| (*k == key).then(|| v.clone()))
        }) {
            return hit;
        }
        let mut days: Vec<_> = home_month_dates(selected)
            .into_iter()
            .map(|date| HomeMonthDay {
                date,
                marks: DayMarks::default(),
                entries: vec![],
            })
            .collect();
        let first = days[0].date;
        let last = days[days.len() - 1].date;
        for event in &suite.calendar_events {
            if event.is_e2e_probe() {
                continue;
            }
            let Some((start, end)) = home_event_date_bounds(event) else {
                continue;
            };
            if start > last || end < first {
                continue;
            }
            let completed = self.calendar_event_is_completed(event);
            let domain = day_domain(&event.source, &event.title);
            let detail = [
                event.source_label().to_string(),
                event.location.trim().to_string(),
                if completed {
                    "已完成".into()
                } else {
                    String::new()
                },
            ]
            .into_iter()
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
            for day in days
                .iter_mut()
                .filter(|day| day.date >= start && day.date <= end)
            {
                let time = if event.all_day {
                    "全天".into()
                } else if day.date > start {
                    "延续".into()
                } else {
                    event.hm_label()
                };
                let minute = if event.all_day || day.date > start {
                    0
                } else {
                    stepx_companion::home_hub::timestamp_to_local_ymd_parts(event.dtstart)
                        .map(|(_, h, m)| h * 60 + m)
                        .unwrap_or(0)
                };
                day.entries.push(HomeMonthEntry {
                    id: format!("calendar:{}:{}", event.id, event.dtstart),
                    time,
                    minute,
                    title: if event.title.trim().is_empty() {
                        "未命名日程".into()
                    } else {
                        event.title.clone()
                    },
                    detail: detail.clone(),
                    kind: MonthEntryKind::Calendar,
                    domain,
                    actions: if self.calendar_event_action_limit(event.id).is_some() { vec![
                        Action::Page(Page::Calendar),
                        Action::BackScreen(BackScreenAction::CalendarSelectDate(
                            day.date.year(), day.date.month(), day.date.day())),
                    ] } else { vec![
                        Action::Page(Page::Calendar),
                        Action::BackScreen(BackScreenAction::CalendarEditEvent(event.id)),
                    ] },
                });
            }
        }
        let ledger_ids: std::collections::HashSet<_> = self
            .context
            .overview
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect();
        let tasks: std::collections::HashMap<_, _> = self
            .workspace
            .journal
            .tasks
            .iter()
            .map(|task| (task.id.as_str(), task))
            .collect();
        for entry in self.home_desktop_date_entries() {
            let domain = day_domain("", &entry.title).or_else(|| {
                entry.id.starts_with("work-artifact-").then_some(DayDomain::Work)
            });
            let kind = match entry.kind {
                SelectedDateEntryKind::WorkAndMatter | SelectedDateEntryKind::Issue
                | SelectedDateEntryKind::Schedule if domain == Some(DayDomain::Life) => MonthEntryKind::Life,
                SelectedDateEntryKind::WorkAndMatter | SelectedDateEntryKind::Issue
                | SelectedDateEntryKind::Schedule => MonthEntryKind::Record,
                SelectedDateEntryKind::Life if domain == Some(DayDomain::Life) => MonthEntryKind::Life,
                _ => continue,
            };
            let Some(((y, m, d), h, minute)) =
                stepx_companion::home_hub::timestamp_to_local_ymd_parts(entry.timestamp)
            else {
                continue;
            };
            let Some(date) = NaiveDate::from_ymd_opt(y, m, d) else {
                continue;
            };
            let index = (date - first).num_days();
            if index < 0 || index as usize >= days.len() {
                continue;
            }
            let raw_ledger_id = entry.id.strip_prefix("ledger-");
            let ledger_id = raw_ledger_id
                .filter(|id| ledger_ids.contains(*id))
                .or_else(|| {
                    raw_ledger_id
                        .and_then(|id| id.strip_suffix(&format!("-{}", date.format("%Y%m%d"))))
                        .filter(|id| ledger_ids.contains(*id))
                });
            let actions = if let Some(task) = entry.id.strip_prefix("task-") {
                vec![
                    Action::Page(Page::Amoo),
                    Action::Workspace(crate::ui::workspace_state::WorkspaceAction::Select(
                        task.into(),
                    )),
                ]
            } else if let Some(id) = ledger_id {
                vec![Action::Context(
                    crate::ui::context_state::ContextAction::ItemDetails(id.into()),
                )]
            } else if let Some(index) = entry
                .id
                .strip_prefix("work-artifact-")
                .and_then(|index| index.parse::<usize>().ok())
            {
                vec![
                    Action::Page(Page::Work),
                    Action::Work(WorkAction::PreviewArtifact(index)),
                ]
            } else {
                vec![Action::Page(Page::Work)]
            };
            let task = entry
                .id
                .strip_prefix("task-")
                .and_then(|id| tasks.get(id))
                .copied();
            let default_label = match domain {
                Some(DayDomain::Life) => "生活事项",
                Some(DayDomain::Work) => "工作事项",
                None => "事项记录",
            };
            let title = home_month_preview(
                task.map_or(entry.title.as_str(), |task| task.title.as_str()),
                true,
            )
            .unwrap_or_else(|| default_label.into());
            let detail = if let Some(task) = task {
                task_previews.get(task.id.as_str()).cloned().flatten()
            } else {
                home_month_preview(&entry.detail, false)
            };
            let badge =
                home_month_preview(&entry.badge, false).unwrap_or_else(|| default_label.into());
            let detail =
                detail.map_or_else(|| badge.clone(), |detail| format!("{badge} · {detail}"));
            days[index as usize].entries.push(HomeMonthEntry {
                id: entry.id,
                time: format!("{h:02}:{minute:02}"),
                minute: h * 60 + minute,
                title,
                detail,
                kind,
                domain,
                actions,
            });
        }
        for alarm in &suite.alarms {
            if alarm.is_suppressed_at(now.timestamp_millis(), &self.reminder_governance) {
                continue;
            }
            let domain = day_domain("", alarm.display_title());
            for day in &mut days {
                if !home_alarm_on_date(
                    alarm,
                    day.date,
                    now.date_naive(),
                    now.hour() * 60 + now.minute(),
                ) {
                    continue;
                }
                day.entries.push(HomeMonthEntry {
                    id: format!("alarm:{}", alarm.id),
                    time: alarm.time_label(),
                    minute: alarm.total_minutes(),
                    title: alarm.display_title().into(),
                    detail: format!("手机闹钟 · {}", alarm.repeat_label()),
                    kind: MonthEntryKind::Alarm,
                    domain,
                    actions: vec![
                        Action::Page(Page::Alarms),
                        Action::BackScreen(BackScreenAction::AlarmEdit(alarm.id)),
                    ],
                });
            }
        }
        for day in &mut days {
            day.entries.sort_by(|a, b| {
                a.minute
                    .cmp(&b.minute)
                    .then(a.kind.cmp(&b.kind))
                    .then(a.id.cmp(&b.id))
            });
            day.entries.dedup_by(|a, b| a.id == b.id);
            for domain in day.entries.iter().filter_map(|entry| entry.domain) {
                day.marks.include(domain);
            }
        }
        let days = std::rc::Rc::new(days);
        MONTH_CACHE.with(|cache| *cache.borrow_mut() = Some((key, days.clone())));
        days
    }

    #[cfg(feature = "ui-capture")]
    pub(in crate::ui) fn calendar_mark_diagnostics(&self) -> serde_json::Value {
        use DayDomain::{Life, Work};
        let days: std::collections::BTreeMap<_, Vec<_>> = self.home_month_days().iter()
            .map(|day| {
                let marks = day.marks.domains().map(|domain| match domain {
                    Work => "work",
                    Life => "life",
                }).collect();
                (day.date.to_string(), marks)
            })
            .collect();
        serde_json::json!({
            "selected": self.home_selected_date().to_string(),
            "days": days,
            "reduced_motion": self.animations_disabled(),
        })
    }

    pub(in crate::ui) fn render_home_calendar_hub_and_panorama(&self) -> El {
        let today = Local::now().date_naive();
        let selected = self.home_selected_date();
        // Both columns show one snapshot. Even a month-cache hit validates the
        // complete input, so obtain its Rc once instead of hashing it twice.
        let days = self.home_month_days();
        let calendar = home_month_geometry_probe(
            "home-calendar-strip",
            self.render_home_top_calendar_strip(selected, today, days.as_slice()),
        );
        let agenda = home_month_geometry_probe(
            "home-day-agenda-column",
            self.render_selected_date_panorama_strip(days.as_slice()),
        );
        home_month_geometry_probe(
            "home-calendar-card",
            self.panel(vec![
                self.render_home_top_command_and_calendar_header(selected, today),
                home_geometry_probe(
                    "home-calendar-columns",
                    self.responsive_columns(
                        680.,
                        S5,
                        S3,
                        CrossAxisAlignment::Start,
                        vec![(1.25, calendar), (1., agenda)],
                    ),
                ),
            ]),
        )
    }

    pub(in crate::ui) fn render_home_top_command_and_calendar_header(
        &self,
        selected_date: NaiveDate,
        today: NaiveDate,
    ) -> El {
        let p = self.p();
        let title = self.hstack(
            S2,
            vec![
                self.home_section_heading(UiIcon::Calendar, format!(
                    "{}年{}月",
                    selected_date.year(),
                    selected_date.month()
                )),
                self.line("月历", CAPTION, p.text3),
            ],
        );
        let controls = self.hstack(
            2.,
            vec![
                self.icon_action(
                    "home-cal-prev-month",
                    "上个月",
                    UiIcon::ArrowLeft,
                    vec![Action::BackScreen(BackScreenAction::CalendarPrevMonth)],
                    false,
                ),
                self.button(
                    "home-cal-today",
                    "今天",
                    Action::BackScreen(BackScreenAction::CalendarToday),
                    if selected_date == today {
                        Btn::Ghost
                    } else {
                        Btn::Secondary
                    },
                ),
                self.icon_action(
                    "home-cal-next-month",
                    "下个月",
                    UiIcon::ArrowRight,
                    vec![Action::BackScreen(BackScreenAction::CalendarNextMonth)],
                    false,
                ),
                self.icon_action(
                    "home-cal-sync",
                    if self.back_screen.stepos_suite_loading {
                        "同步中…"
                    } else {
                        "同步日历"
                    },
                    UiIcon::Refresh,
                    vec![Action::BackScreen(BackScreenAction::SyncCalendarAndSuite)],
                    false,
                ),
                self.icon_action(
                    "home-cal-open-page",
                    "打开日历",
                    UiIcon::ExternalLink,
                    vec![Action::Page(Page::Calendar)],
                    false,
                ),
            ],
        );
        self.responsive_columns(
            490.,
            S3,
            S1,
            CrossAxisAlignment::Center,
            vec![(1., title), (1., controls)],
        )
    }

    fn render_home_top_calendar_strip(
        &self,
        selected_date: NaiveDate,
        today: NaiveDate,
        days: &[HomeMonthDay],
    ) -> El {
        let p = self.p();
        let module = HomeModuleId::CalendarPanorama;
        let cell_height = match self.resolve_effective_height(module, self.home_modules.module_height(module)) {
            HomeModuleHeight::Compact => 32.,
            HomeModuleHeight::Tall => 46.,
            HomeModuleHeight::Auto | HomeModuleHeight::Standard => 38.,
        };
        let weekdays = self.grid_cols(
            ["一", "二", "三", "四", "五", "六", "日"]
                .into_iter()
                .enumerate()
                .map(|(i, label)| {
                    Align::new(self.line(label, CAPTION, if i >= 5 { p.text3 } else { p.text2 }))
                        .finish()
                })
                .collect(),
            7,
            3.,
        );
        let mut rows = vec![weekdays];
        for week in days.chunks(7) {
            let mut cells = Vec::with_capacity(7);
            for day in week {
                let date = day.date;
                let is_today = date == today;
                let is_selected = date == selected_date;
                let is_current_month =
                    date.month() == selected_date.month() && date.year() == selected_date.year();
                let marks = day.marks;
                let id = format!("home-month-day-{}", date.format("%Y-%m-%d"));
                let cell = Hoverable::new(self.mouse_state(&id), move |mouse| {
                    let date_label = self.calendar_day_number(date.day(), is_today, is_selected, is_current_month);
                    let markers = self.calendar_day_marks(marks);
                    Container::new(
                        ConstrainedBox::new(
                            Flex::column()
                                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                .with_main_axis_alignment(MainAxisAlignment::Center)
                                .with_spacing(1.)
                                .with_child(date_label)
                                .with_child(markers)
                                .finish(),
                        )
                        .with_height(cell_height)
                        .finish(),
                    )
                    .with_background_color(if is_selected {
                        p.accent_soft
                    } else {
                        lerp_color(p.card, p.hover, self.hover_t(&mouse))
                    })
                    .with_border(Border::all(1.).with_border_color(if is_selected {
                        p.accent
                    } else {
                        ColorU::transparent_black()
                    }))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::BackScreen(
                        BackScreenAction::CalendarSelectDate(date.year(), date.month(), date.day()),
                    ))
                })
                .finish();
                cells.push(home_geometry_probe(id, cell));
            }
            rows.push(self.grid_cols(cells, 7, 3.));
        }
        let mut legend = self.calendar_mark_legend();
        legend.push(self.line(
            if self.back_screen.stepos_suite_loading {
                "同步中…"
            } else if self.selected.is_none() {
                "手机离线 · 已保存记录"
            } else if !self.back_screen.stepos_suite.checked {
                "日历待同步"
            } else {
                "已同步记录"
            },
            MICRO,
            p.text3,
        ));
        rows.push(self.bounded_wrap(legend));
        self.calendar_content_enter(self.vstack(3., rows), true)
    }

    fn render_selected_date_panorama_strip(&self, days: &[HomeMonthDay]) -> El {
        let p = self.p();
        let selected = self.home_selected_date();
        let profile = self.compute_home_module_value_profile(HomeModuleId::CalendarPanorama,
            self.home_modules.module_width(HomeModuleId::CalendarPanorama),
            self.home_modules.module_height(HomeModuleId::CalendarPanorama));
        let page_size = if self.home_module_available_width(HomeModuleId::CalendarPanorama) < 680. {
            profile.max_items.min(2)
        } else { profile.max_items };
        let compact = profile.effective_density == HomeDensityTier::HeadlineTop1;
        let entries = days
            .iter()
            .find(|day| day.date == selected)
            .map(|day| day.entries.as_slice())
            .unwrap_or(&[]);
        let weekday = ["周一", "周二", "周三", "周四", "周五", "周六", "周日"]
            [selected.weekday().num_days_from_monday() as usize];
        let today = selected == Local::now().date_naive();
        let mut rows = vec![self.between(
            home_month_geometry_probe(
                "home-month-agenda-heading",
                self.vstack(
                    2.,
                    vec![
                        self.line_w(
                            format!(
                                "{}月{}日 {weekday}{}",
                                selected.month(),
                                selected.day(),
                                if today { " · 今天" } else { "" }
                            ),
                            BODY,
                            p.text,
                            Weight::Semibold,
                        ),
                        self.text(
                            [
                                (MonthEntryKind::Calendar, "项日程"),
                                (MonthEntryKind::Record, "条事项记录"),
                                (MonthEntryKind::Life, "条生活记录"),
                                (MonthEntryKind::Alarm, "个闹钟"),
                            ]
                                .into_iter()
                                .filter_map(|(kind, label)| {
                                    let count = entries.iter().filter(|entry| entry.kind == kind).count();
                                    (count > 0).then(|| format!("{count} {label}"))
                                })
                                .collect::<Vec<_>>()
                                .join(" · "),
                            MICRO,
                            p.text3,
                        ),
                    ],
                ),
            ),
            home_month_geometry_probe(
                "home-month-create",
                self.chip(
                    "home-panorama-add-cal",
                    "新建日程",
                    Some(UiIcon::Plus),
                    vec![
                        Action::Page(Page::Calendar),
                        Action::BackScreen(BackScreenAction::CalendarCancelEdit),
                        Action::Focus(F_PHONE_SCHEDULE_TITLE),
                    ],
                    false,
                    false,
                ),
            ),
        )];
        if entries.is_empty() {
            rows.push(
                Container::new(self.vstack(
                    S1,
                    vec![
                        self.line("这一天没有已记录的安排", CAPTION, p.text2),
                        if compact { Empty::new().finish() } else {
                            self.text("选择日期查看工作与生活安排，也可以为这一天添加新日程。", MICRO, p.text3)
                        },
                    ],
                ))
                .with_vertical_padding(S3)
                .finish(),
            );
        } else {
            let page_key = format!("home-month-agenda:{selected}");
            let range = self.list_page_range(&page_key, entries.len(), page_size);
            // The agenda answers what is scheduled first. Keep time order
            // within the scheduled items and within the work history.
            let scheduled = entries.iter().filter(|entry| !entry.kind.is_record());
            let history = entries.iter().filter(|entry| entry.kind.is_record());
            let mixed = scheduled.clone().next().is_some() && history.clone().next().is_some();
            let mut last_group = None;
            for entry in scheduled.chain(history).skip(range.start).take(range.len()) {
                let history = entry.kind.is_record();
                if mixed && !compact && last_group != Some(history) {
                    rows.push(self.line_w(
                        if history { "工作与生活记录" } else { "日程与闹钟" },
                        CAPTION,
                        p.text2,
                        Weight::Medium,
                    ));
                }
                last_group = Some(history);
                rows.push(self.month_agenda_row(entry));
            }
            if entries.len() > page_size {
                rows.push(self.list_page_controls(&page_key, entries.len(), page_size));
            }
        }
        // Give a quiet day useful context from the same cached month. Date
        // shortcuts keep the selected day's list and its count independent.
        if entries.len() <= 2 && !compact {
            let mut other_days: Vec<_> = days.iter().filter_map(|day| {
                if day.date == selected || day.date.month() != selected.month() {
                    return None;
                }
                let mut scheduled = day.entries.iter().filter(|entry| entry.kind == MonthEntryKind::Calendar);
                let first = scheduled.next()?;
                Some((day.date, first, 1 + scheduled.count()))
            }).collect();
            other_days.sort_by_key(|(date, _, _)| {
                (*date < selected, (*date - selected).num_days().abs())
            });
            if !other_days.is_empty() {
                rows.push(self.line_w("本月其他安排", CAPTION, p.text2, Weight::Semibold));
                for (date, first, count) in other_days.into_iter().take(3) {
                    let label = if count == 1 {
                        Self::truncate_chars(&first.title, 24)
                    } else {
                        format!("{} 等 {count} 项", Self::truncate_chars(&first.title, 18))
                    };
                    rows.push(self.home_agenda_link(
                        format!("home-month-related-{date}"),
                        self.fixed_w(38., self.mono(date.format("%m/%d").to_string(), MICRO, p.text2)),
                        self.line(label, CAPTION, p.text),
                        vec![Action::BackScreen(BackScreenAction::CalendarSelectDate(
                            date.year(), date.month(), date.day(),
                        ))],
                    ));
                }
            }
        }
        self.calendar_content_enter(self.vstack(6., rows), false)
    }

    fn month_agenda_row(&self, entry: &HomeMonthEntry) -> El {
        let p = self.p();
        let icon = match entry.kind {
            MonthEntryKind::Calendar => UiIcon::Calendar,
            MonthEntryKind::Alarm => UiIcon::Clock,
            MonthEntryKind::Life => UiIcon::Coffee,
            MonthEntryKind::Record if entry.domain.is_some() => UiIcon::Briefcase,
            MonthEntryKind::Record => UiIcon::File,
        };
        let tint = entry.domain.map_or(p.text2, |domain| domain.tint(p));
        home_geometry_probe(
            format!("home-month-entry-{}", entry.id),
            self.home_agenda_link(
                format!("home-month-open-{}", entry.id),
                self.fixed_w(38., self.vstack(3., vec![
                    self.mono(&entry.time, MICRO, tint),
                    self.content_icon(icon, 12., tint),
                ])),
                self.vstack(2., vec![
                    self.line_w(Self::truncate_chars(&entry.title, 38), CAPTION, p.text, Weight::Medium),
                    self.line(Self::truncate_chars(&entry.detail, 54), MICRO, p.text2),
                ]),
                entry.actions.clone(),
            ),
        )
    }

    pub(in crate::ui::pages) fn calendar_day_context_rows(&self, day: &HomeMonthDay) -> Vec<El> {
        let records: Vec<_> = day.entries.iter()
            .filter(|entry| entry.kind != MonthEntryKind::Calendar)
            .collect();
        if records.is_empty() {
            return Vec::new();
        }
        let key = format!("calendar-day-context:{}", day.date);
        let range = self.list_page_range(&key, records.len(), 5);
        let mut rows = vec![self.section_heading(UiIcon::Layers, "工作与生活记录")];
        rows.extend(records[range].iter().map(|entry| self.month_agenda_row(entry)));
        if records.len() > 5 {
            rows.push(self.list_page_controls(&key, records.len(), 5));
        }
        rows
    }

    fn home_agenda_link(&self, id: String, leading: El, content: El, actions: Vec<Action>) -> El {
        let p = self.p();
        let row = Hoverable::new(self.mouse_state(&id), move |mouse| {
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(S2)
                    .with_child(leading)
                    .with_child(Expanded::new(1., content).finish())
                    .with_child(self.ui_icon(UiIcon::ChevronRight, 13., p.text2))
                    .finish(),
            )
            .with_horizontal_padding(S1)
            .with_vertical_padding(6.)
            .with_background_color(lerp_color(p.bg, p.hover, self.hover_t(&mouse)))
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| {
            for action in &actions {
                ctx.dispatch_typed_action(action.clone());
            }
        })
        .finish();
        home_geometry_probe(id, row)
    }

    pub(in crate::ui) fn select_diverse_panorama_entries<'a>(
        entries: &'a [stepx_companion::home_hub::SelectedDateEntry],
        limit: usize,
    ) -> Vec<&'a stepx_companion::home_hub::SelectedDateEntry> {
        if entries.len() <= limit {
            return entries.iter().collect();
        }
        let mut chosen_indices: Vec<usize> = Vec::with_capacity(limit);
        let bucket_predicates: [fn(SelectedDateEntryKind) -> bool; 4] = [
            |k| k == SelectedDateEntryKind::Schedule,
            |k| k == SelectedDateEntryKind::WorkAndMatter,
            |k| k == SelectedDateEntryKind::Life,
            |k| {
                matches!(
                    k,
                    SelectedDateEntryKind::Issue | SelectedDateEntryKind::Communication
                )
            },
        ];
        for pred in bucket_predicates {
            if chosen_indices.len() >= limit {
                break;
            }
            if let Some((idx, _)) = entries
                .iter()
                .enumerate()
                .find(|(idx, e)| !chosen_indices.contains(idx) && pred(e.kind))
            {
                chosen_indices.push(idx);
            }
        }
        for idx in 0..entries.len() {
            if chosen_indices.len() >= limit {
                break;
            }
            if !chosen_indices.contains(&idx) {
                chosen_indices.push(idx);
            }
        }
        chosen_indices.sort_unstable();
        chosen_indices
            .into_iter()
            .map(|idx| &entries[idx])
            .collect()
    }

    /// 24-hour SVG dial: night band, selected date's calendar arcs, alarm dots, elapsed arc and a
    /// now-hand, with the clock and day progress drawn natively in the centre.
    pub(in crate::ui) fn render_home_day_dial(&self) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let now = Local::now();
        let now_ms = now.timestamp_millis();
        let today = now.date_naive();
        let now_min = now.hour() * 60 + now.minute();
        let spans: Vec<_> = today_event_spans(&suite.calendar_events, today)
            .into_iter()
            .filter(|(_, _, ev)| {
                !self.calendar_event_is_completed(ev)
                    && !self.calendar_event_is_suppressed(ev, now_ms)
            })
            .collect();
        let colors = event_colors(&p);
        let size = 152.;
        let spec = DayDial {
            size,
            now_min,
            events: spans
                .iter()
                .enumerate()
                .map(|(i, (s, e, _))| DialEvent {
                    start_min: *s,
                    end_min: *e,
                    paint: paint_of(colors[i % colors.len()]),
                })
                .collect(),
            alarms: suite
                .alarms
                .iter()
                .filter(|a| {
                    a.enabled
                        && !a.is_e2e_probe()
                        && !a.is_suppressed_at(now_ms, &self.reminder_governance)
                })
                .map(|a| DialAlarm {
                    minute_of_day: a.hour as u32 * 60 + a.minutes as u32,
                    enabled: a.is_today_valid_unexpired_at(now_ms, &self.reminder_governance),
                })
                .collect(),
            track: solid(p.text3, 0.16, p.bg),
            night: solid(p.indigo, if p.dark { 0.42 } else { 0.24 }, p.bg),
            elapsed: solid(p.text2, 0.5, p.surface),
            tick: solid(p.text3, 0.85, p.surface),
            hand: paint_of(p.text),
            alarm: paint_of(p.warning),
            face: paint_of(p.surface),
            matte: paint_of(p.bg).with_alpha(1.0),
        };
        let dial = self.svg_canvas("dial", day_dial(&spec), Some(size), size, false);

        let day_pct = now_min * 100 / 1440;
        let sub_label = format!("今日已过 {day_pct}%");
        let center = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(0.)
            .with_child(self.line_w(
                now.format("%H:%M").to_string(),
                19.,
                p.text,
                Weight::Semibold,
            ))
            .with_child(self.line(sub_label, 10., p.text2))
            .finish();
        let top_label = Align::new(
            Container::new(self.line("0", 10., p.text3))
                .with_padding_top(34.)
                .finish(),
        )
        .top_center()
        .finish();
        let bottom_label = Align::new(
            Container::new(self.line("12", 10., p.text3))
                .with_padding_bottom(34.)
                .finish(),
        )
        .bottom_center()
        .finish();
        let dial_stack = overlay(size, size, dial, vec![center, top_label, bottom_label]);

        let state = self.mouse_state("home-day-dial");
        let dial_click = Hoverable::new(state, move |_| dial_stack)
            .with_cursor(Cursor::PointingHand)
            .on_click(|ctx, _, _| {
                ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::CalendarToday));
                ctx.dispatch_typed_action(Action::Page(Page::Calendar));
            })
            .finish();

        Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(4.)
            .with_child(dial_click)
            .finish()
    }

    // =========================================================================
    // SECTION 2: 一周全局总结 (7-Day Stacked SVG Bar Chart + 2×2 Pillars) & 智能建议 (Phone Amoo + Context Rules)
    // =========================================================================
    pub(in crate::ui) fn render_home_weekly_and_ai_row(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
        weekly: &stepx_companion::home_hub::WeeklyOverviewReport,
    ) -> El {
        let mod_w = self.home_module_content_width(HomeModuleId::WeeklyAndAi);
        let effective_height = self.resolve_effective_height(HomeModuleId::WeeklyAndAi,
            self.home_modules.module_height(HomeModuleId::WeeklyAndAi));
        if effective_height == HomeModuleHeight::Compact {
            // One compact surface: week rhythm, review entry points and the most
            // useful next step. The full quadrants remain in Standard / Tall.
            let p = self.p();
            let header = row_between_expanded(
                self.home_section_heading(UiIcon::Activity, "本周与建议"),
                self.line(&weekly.week_range_label, MICRO, p.text2), S2);
            let review = self.wrap([
                ("day", "今日总结", stepx_companion::context_ledger::Period::Day),
                ("week", "本周复盘", stepx_companion::context_ledger::Period::Week),
                ("month", "本月总结", stepx_companion::context_ledger::Period::Month),
            ].into_iter().map(|(id, label, period)| self.chip(
                format!("home-weekly-review-{id}"), label, Some(UiIcon::File),
                vec![Action::Context(crate::ui::context_state::ContextAction::Period(period))],
                false, false)).collect());
            let days = weekly.days.iter().map(|d| WeeklyBarDay {
                work_count: (d.schedule_count + d.work_count) as u32,
                life_count: d.life_count as u32,
                issue_count: (d.issue_count + d.communication_count) as u32,
                selected: d.is_selected, today: d.is_today,
            }).collect();
            let chart = home_native_plot("home-weekly-plot", HomePlotContent::Week {
                days, fills: [p.blue, p.warning, p.pink],
                track: lerp_color(p.surface, p.text3, 0.14), selected: p.accent,
            }, 38.);
            let labels = Flex::row().with_main_axis_size(MainAxisSize::Max)
                .with_children(weekly.days.iter().enumerate().map(|(idx, d)| {
                    let action = Action::BackScreen(BackScreenAction::CalendarSelectDate(d.year, d.month, d.day));
                    let label = format!("{} {}", d.weekday_label.trim_start_matches('周'), d.total_count);
                    let label = Align::new(self.line_w(label, 10.,
                        if d.is_selected { p.accent } else { p.text2 }, Weight::Medium)).finish();
                    let cell = Hoverable::new(self.mouse_state(format!("home-weekly-day-{idx}")), move |_| label)
                        .with_cursor(Cursor::PointingHand)
                        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone())).finish();
                    Expanded::new(1., home_geometry_probe(format!("home-weekly-day-{idx}"), cell)).finish()
                })).finish();
            return home_geometry_probe("home-weekly-and-ai-compact", self.panel(vec![
                header, review, self.vstack(3., vec![chart, labels]),
                self.render_home_ai_brief_card(report),
            ]));
        }
        if matches!(mod_w, HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf) {
            let weekly_card = self.render_home_weekly_summary_card(weekly);
            let ai_card = self.render_home_ai_brief_card(report);
            return home_geometry_probe(
                "home-weekly-and-ai-compact",
                self.vstack(S2, vec![weekly_card, self.panel(vec![ai_card])]),
            );
        }
        let left = self.render_home_weekly_summary_card(weekly);
        let right = self.render_home_ai_brief_panel(report);
        self.responsive_columns(
            760.,
            S3,
            S3,
            CrossAxisAlignment::Stretch,
            vec![(1.14, left), (1.0, right)],
        )
    }

    /// Bounded, lazy detail for the selected week. Reuse the same dated source
    /// filtering as the daily agenda instead of mixing in today's unrelated data.
    fn home_weekly_detail_entries(
        &self,
        weekly: &stepx_companion::home_hub::WeeklyOverviewReport,
        quadrant: usize,
    ) -> std::rc::Rc<Vec<(String, stepx_companion::home_hub::SelectedDateEntry)>> {
        type Entries = std::rc::Rc<Vec<(String, stepx_companion::home_hub::SelectedDateEntry)>>;
        thread_local! {
            static DETAILS: std::cell::RefCell<[Option<(u64, Entries)>; 4]> =
                const { std::cell::RefCell::new([None, None, None, None]) };
            static EMPTY_DETAILS: Entries = std::rc::Rc::new(Vec::new());
        }
        let quadrant = quadrant.min(3);
        // The loop below skips every zero-count day. In that case its result is
        // necessarily empty; avoid cloning and hashing unrelated source data.
        // Nonempty details retain the complete-content cache key below.
        if weekly.days.iter().all(|day| match quadrant {
            0 => day.schedule_count == 0,
            1 => day.work_count == 0,
            2 => day.life_count == 0,
            _ => day.issue_count == 0 && day.communication_count == 0,
        }) {
            return EMPTY_DETAILS.with(std::rc::Rc::clone);
        }
        self.with_home_hub_input(&self.push.snapshot, &self.express_parcels, |input, _, _| {
            let desktop_entries = self.home_desktop_date_entries();
            let monday = weekly
                .days
                .first()
                .map(|d| d.date_ymd.as_str())
                .unwrap_or("");
            let key = stepx_companion::home_hub::compute_home_hub_input_hash(
                210 + quadrant as u8,
                monday,
                input,
                &desktop_entries,
            );
            if let Some(hit) = DETAILS.with(|cache| {
                cache.borrow()[quadrant]
                    .as_ref()
                    .and_then(|(old, entries)| (*old == key).then(|| entries.clone()))
            }) {
                return hit;
            }
            let mut entries = Vec::new();
            let mut seen = std::collections::HashSet::new();
            for day in weekly.days.iter().rev() {
                let count = match quadrant {
                    0 => day.schedule_count,
                    1 => day.work_count,
                    2 => day.life_count,
                    _ => day.issue_count + day.communication_count,
                };
                if count == 0 {
                    continue;
                }
                let panorama = build_selected_date_panorama(&day.date_ymd, input, &desktop_entries);
                for entry in panorama.entries.into_iter().rev() {
                    let belongs = match quadrant {
                        0 => entry.kind == SelectedDateEntryKind::Schedule,
                        1 => entry.kind == SelectedDateEntryKind::WorkAndMatter,
                        2 => entry.kind == SelectedDateEntryKind::Life,
                        _ => matches!(
                            entry.kind,
                            SelectedDateEntryKind::Issue | SelectedDateEntryKind::Communication
                        ),
                    };
                    if belongs && seen.insert(entry.id.clone()) {
                        entries.push((day.date_ymd.clone(), entry));
                        if entries.len() == 3 {
                            break;
                        }
                    }
                }
                if entries.len() == 3 {
                    break;
                }
            }
            let entries = std::rc::Rc::new(entries);
            DETAILS.with(|cache| cache.borrow_mut()[quadrant] = Some((key, entries.clone())));
            entries
        })
    }

    pub(in crate::ui) fn render_home_weekly_summary_card(
        &self,
        weekly: &stepx_companion::home_hub::WeeklyOverviewReport,
    ) -> El {
        let p = self.p();

        let header_title = self.wrap(vec![
            self.home_section_heading(UiIcon::Activity, if weekly.days.iter().any(|d| d.is_today) {
                "本周概览"
            } else {
                "所选周概览"
            }),
            self.pill_badge(
                &weekly
                    .days
                    .first()
                    .map(|d| format!("{} · {}", d.year, weekly.week_range_label))
                    .unwrap_or_else(|| weekly.week_range_label.clone()),
                p.indigo,
                soft_tint(p.indigo, p.dark),
            ),
        ]);
        let review_actions = self.wrap(vec![
            self.chip(
                "home-weekly-review-day",
                "今日总结",
                None,
                vec![Action::Context(
                    crate::ui::context_state::ContextAction::Period(
                        stepx_companion::context_ledger::Period::Day,
                    ),
                )],
                false,
                false,
            ),
            self.chip(
                "home-weekly-review-week",
                "本周复盘",
                Some(UiIcon::Calendar),
                vec![Action::Context(
                    crate::ui::context_state::ContextAction::Period(
                        stepx_companion::context_ledger::Period::Week,
                    ),
                )],
                false,
                false,
            ),
            self.chip(
                "home-weekly-review-month",
                "本月总结",
                None,
                vec![Action::Context(
                    crate::ui::context_state::ContextAction::Period(
                        stepx_companion::context_ledger::Period::Month,
                    ),
                )],
                false,
                false,
            ),
        ]);
        let header = self.responsive_columns(
            620.,
            S2,
            S2,
            CrossAxisAlignment::Center,
            vec![(1., header_title), (1.2, review_actions)],
        );

        let bar_days: Vec<WeeklyBarDay> = weekly
            .days
            .iter()
            .map(|d| WeeklyBarDay {
                work_count: (d.schedule_count + d.work_count) as u32,
                life_count: d.life_count as u32,
                issue_count: (d.issue_count + d.communication_count) as u32,
                selected: d.is_selected,
                today: d.is_today,
            })
            .collect();
        let raw_h = self.home_modules.module_height(HomeModuleId::WeeklyAndAi);
        let eff_h = self.resolve_effective_height(HomeModuleId::WeeklyAndAi, raw_h);
        let plot_h = match eff_h {
            HomeModuleHeight::Compact => 42.,
            HomeModuleHeight::Tall => 68.,
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => 54.,
        };
        let chart_el = home_native_plot(
            "home-weekly-plot",
            HomePlotContent::Week {
                days: bar_days,
                fills: [p.blue, p.warning, p.pink],
                track: lerp_color(p.surface, p.text3, 0.14),
                selected: p.accent,
            },
            plot_h,
        );

        let day_labels = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_children(weekly.days.iter().enumerate().map(|(idx, d)| {
                let (dy, dm, dd) = (d.year, d.month, d.day);
                let is_sel = d.is_selected;
                let is_tod = d.is_today;
                let lbl = format!(
                    "{}({})",
                    d.weekday_label.trim_start_matches('周'),
                    d.total_count
                );
                let day_id = format!("home-weekly-day-{idx}");
                let state = self.mouse_state(&day_id);
                let cell = Hoverable::new(state, move |mouse| {
                    let t = self.hover_t(&mouse);
                    let fg = if is_sel {
                        p.accent
                    } else if is_tod {
                        p.blue
                    } else {
                        lerp_color(p.text3, p.text, t)
                    };
                    Align::new(self.line_w(
                        lbl.clone(),
                        10.,
                        fg,
                        if is_sel || is_tod {
                            Weight::Semibold
                        } else {
                            Weight::Normal
                        },
                    ))
                    .top_center()
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::BackScreen(
                        BackScreenAction::CalendarSelectDate(dy, dm, dd),
                    ))
                })
                .finish();
                let cell = home_geometry_probe(day_id, cell);
                Expanded::new(1., cell).finish()
            }))
            .finish();

        let week_total: usize = weekly.days.iter().map(|d| d.total_count).sum();
        let chart_legend = row_between_expanded(
            self.line_w(
                format!("共 {week_total} 项"),
                MICRO,
                p.text,
                Weight::Semibold,
            ),
            self.hstack(
                6.,
                vec![
                    self.hstack(3., vec![self.dot(p.blue), self.line("工作", 10., p.text2)]),
                    self.hstack(
                        3.,
                        vec![self.dot(p.warning), self.line("生活", 10., p.text2)],
                    ),
                    self.hstack(3., vec![self.dot(p.pink), self.line("问题", 10., p.text2)]),
                ],
            ),
            4.,
        );

        let chart_box = home_geometry_probe(
            "home-weekly-chart-card",
            Container::new(self.vstack(3., vec![chart_legend, chart_el, day_labels]))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(6.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        );

        let peak_summary = weekly
            .days
            .iter()
            .max_by_key(|d| (d.total_count, d.schedule_count))
            .filter(|d| d.total_count > 0)
            .map(|d| format!("峰值：{} {} 项", d.weekday_label, d.total_count))
            .unwrap_or_else(|| "所选周暂无已同步记录".to_string());

        if eff_h == HomeModuleHeight::Compact {
            let compact_peak = self.line(
                format!(
                    "{peak_summary} · 日程 {} · 待核对 {}",
                    weekly.total_schedules, weekly.total_issues
                ),
                MICRO,
                p.text2,
            );
            return home_geometry_probe(
                "home-weekly-card",
                self.panel(vec![header, chart_box, compact_peak]),
            );
        }

        let pillar_meta = [
            (UiIcon::Calendar, p.indigo),
            (UiIcon::Briefcase, p.blue),
            (UiIcon::Package, p.warning),
            (UiIcon::Alert, p.pink),
        ];
        let active_quadrant = self.home_ai_quadrant.min(3);
        let mut pillar_tiles: Vec<El> = weekly
            .pillars
            .iter()
            .enumerate()
            .take(4)
            .map(|(i, (name, stat, _detail))| {
                let (icon, tint) = pillar_meta[i % pillar_meta.len()];
                let is_active = active_quadrant == i;
                let short_stat = stat
                    .split('项')
                    .next()
                    .map(|n| format!("{}项", n.trim()))
                    .unwrap_or_else(|| stat.clone());
                let name_str = name.clone();
                let pillar_id = format!("home-weekly-pillar-{i}");
                let state = self.mouse_state(&pillar_id);
                let tile = Hoverable::new(state, move |mouse| {
                    let t = self.hover_t(&mouse);
                    let bg = if is_active {
                        soft_tint(tint, p.dark)
                    } else {
                        lerp_color(p.surface, p.hover, t)
                    };
                    let border = if is_active {
                        tint
                    } else {
                        lerp_color(p.border, tint, t * 0.5)
                    };
                    Container::new(self.hstack(
                        6.,
                        vec![
                            self.ui_icon_box(icon, 22., soft_tint(tint, p.dark), tint, None),
                            Expanded::new(
                                1.,
                                self.vstack(
                                    1.,
                                    vec![row_between_expanded(
                                        self.line_w(&name_str, MICRO, p.text, Weight::Semibold),
                                        self.pill_badge(
                                            &short_stat,
                                            tint,
                                            soft_tint(tint, p.dark),
                                        ),
                                        4.,
                                    )],
                                ),
                            )
                            .finish(),
                        ],
                    ))
                    .with_horizontal_padding(S2)
                    .with_vertical_padding(4.)
                    .with_background_color(bg)
                    .with_border(
                        Border::all(if is_active { 1.5 } else { 1. }).with_border_color(border),
                    )
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::HomeSelectAiQuadrant(i))
                })
                .finish();
                home_geometry_probe(pillar_id, tile)
            })
            .collect();

        let mut pillar_rows = Vec::new();
        while !pillar_tiles.is_empty() {
            let first = pillar_tiles.remove(0);
            if pillar_tiles.is_empty() {
                pillar_rows.push(first);
            } else {
                let second = pillar_tiles.remove(0);
                pillar_rows.push(self.responsive_columns(
                    340.,
                    6.,
                    6.,
                    CrossAxisAlignment::Stretch,
                    vec![(1., first), (1., second)],
                ));
            }
        }
        let pillars_grid = self.vstack(6., pillar_rows);
        let body_row = self.responsive_columns(
            540.,
            S2,
            S2,
            CrossAxisAlignment::Start,
            vec![(1.02, chart_box), (1.0, pillars_grid)],
        );

        let (q_icon, q_tint) = pillar_meta[active_quadrant];
        let entries = self.home_weekly_detail_entries(weekly, active_quadrant);
        let (q_name, empty_text, source_label, page) = match active_quadrant {
            0 => ("日程", "所选周暂无已同步日程", "日历", Page::Calendar),
            1 => ("工作与成果", "所选周暂无已同步工作记录", "作品", Page::Studio),
            2 => ("生活服务", "所选周暂无已同步生活记录", "消息", Page::Notifications),
            _ => (
                "沟通与待确认",
                "所选周暂无已同步沟通记录",
                "消息",
                Page::Notifications,
            ),
        };
        let mut detail_rows = vec![row_between_expanded(
            self.wrap(vec![
                self.content_icon(q_icon, 14., q_tint),
                self.line_w(q_name, MICRO, p.text, Weight::Semibold),
                self.line("所选周 · 最近记录", MICRO, p.text3),
            ]),
            self.link("home-weekly-source-page", source_label, Action::Page(page)),
            6.,
        )];
        let limit = if eff_h == HomeModuleHeight::Tall {
            3
        } else {
            2
        };
        for (idx, (date, entry)) in entries.iter().take(limit).enumerate() {
            let mut actions = Vec::new();
            if entry.kind == SelectedDateEntryKind::Schedule {
                if let Ok(day) = NaiveDate::parse_from_str(date, "%Y-%m-%d") {
                    actions.push(Action::BackScreen(BackScreenAction::CalendarSelectDate(
                        day.year(), day.month(), day.day(),
                    )));
                }
            }
            actions.push(Action::HomeExecuteIntent(entry.intent.clone()));
            let open = self.chip(
                format!("home-weekly-detail-open-{idx}"),
                &entry.action_label,
                Some(UiIcon::ExternalLink),
                actions,
                false,
                false,
            );
            let content = self.vstack(
                2.,
                vec![
                    self.line_w(
                        format!("{} {} · {}", &date[5..], entry.time_hm, entry.title),
                        CAPTION,
                        p.text,
                        Weight::Medium,
                    ),
                    self.line(&entry.detail, MICRO, p.text2),
                ],
            );
            detail_rows.push(home_month_geometry_probe(
                format!("home-weekly-detail-{idx}"),
                row_between_expanded(content, open, S2),
            ));
        }
        if entries.is_empty() {
            detail_rows.push(self.line(empty_text, MICRO, p.text3));
        }
        let drilldown = home_month_geometry_probe(
            "home-weekly-drilldown",
            Container::new(self.vstack(S2, detail_rows))
                .with_uniform_padding(S2)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
        );
        let card_children = vec![header, body_row, drilldown];

        home_geometry_probe("home-weekly-card", self.panel(card_children))
    }

    pub(in crate::ui) fn render_home_ai_brief_panel(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        home_geometry_probe(
            "home-ai-brief-card",
            self.panel(vec![self.render_home_ai_brief_card(report)]),
        )
    }

    /// The phone-AI briefing card: genuine suggestions from the phone's Amoo when available,
    /// otherwise the context-linked local recommendations, clearly labelled as such.
    pub(in crate::ui) fn render_home_ai_brief_card(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        let p = self.p();
        let ai = &self.home_ai;
        let running = ai.running();
        let brief = ai.brief.as_ref();

        let status_pill =
            running.then(|| self.pill_badge("更新中…", p.blue, soft_tint(p.blue, p.dark)));

        let refresh = if running {
            self.button_icon(
                "home-ai-cancel",
                "停止",
                UiIcon::Close,
                Action::HomeAiCancel,
                Btn::Ghost,
            )
        } else {
            self.button_icon(
                "home-ai-refresh",
                "刷新",
                UiIcon::Refresh,
                Action::HomeAiRefresh,
                Btn::Secondary,
            )
        };

        let mut header_items = vec![
            self.ui_icon_box(UiIcon::Bell, 22., soft_tint(p.blue, p.dark), p.blue, None),
            home_geometry_probe("home-ai-brief-title", self.heading("建议")),
        ];
        if let Some(pill) = status_pill {
            header_items.push(pill);
        }
        header_items.push(home_geometry_probe("home-ai-brief-action", refresh));
        let header = home_geometry_probe("home-ai-brief-header", self.wrap(header_items));

        let mut children = vec![header];
        if running {
            let status = if ai.status.is_empty() {
                "手机 Amoo 正在阅读首页上下文…".to_string()
            } else {
                ai.status.clone()
            };
            children.push(self.vstack(
                3.,
                vec![
                    self.line(status, MICRO, p.text2),
                    self.progress(Some(((self.loading_frame % 20) * 5) as u8), p.blue),
                ],
            ));
        }
        let mod_w = self.home_module_content_width(HomeModuleId::WeeklyAndAi);
        let raw_h = self.home_modules.module_height(HomeModuleId::WeeklyAndAi);
        let profile =
            self.compute_home_module_value_profile(HomeModuleId::WeeklyAndAi, mod_w, raw_h);
        let now_ms_ai = Local::now().timestamp_millis();
        let rec_limit = profile.max_items;
        let (headline, recs): (String, Vec<&HomeAiRecommendation>) = match brief {
            Some(b) => (
                b.headline.clone(),
                b.suggestions
                    .iter()
                    .filter(|r| {
                        !self.reminder_governance.is_any_key_suppressed(
                            &[format!("ai_rec:{}", r.id), format!("ai_rec:{}", r.title)],
                            None,
                            None,
                            None,
                            now_ms_ai,
                        )
                    })
                    .take(rec_limit)
                    .collect(),
            ),
            None => (
                report.scene.subtitle.clone(),
                report
                    .ai_recommendations
                    .iter()
                    .filter(|r| {
                        !self.reminder_governance.is_any_key_suppressed(
                            &[format!("ai_rec:{}", r.id), format!("ai_rec:{}", r.title)],
                            None,
                            None,
                            None,
                            now_ms_ai,
                        )
                    })
                    .take(rec_limit)
                    .collect(),
            ),
        };
        if !running || brief.is_some() {
            children.push(self.line_w(
                Self::truncate_chars(&headline, profile.char_budget.clamp(16, 28)),
                MICRO,
                p.text2,
                Weight::Medium,
            ));
        }
        if let (Some(err), false) = (ai.error.as_ref(), running) {
            children.push(self.line(err.clone(), MICRO, p.warning));
        }
        if recs.is_empty() && !running {
            children.push(self.line("暂无建议", CAPTION, p.text3));
        } else {
            for (idx, rec) in recs.into_iter().enumerate() {
                children.push(self.render_home_ai_suggestion_row(rec, idx == 0));
            }
        }

        self.vstack(6., children)
    }

    pub(in crate::ui) fn render_home_ai_suggestion_row(
        &self,
        rec: &HomeAiRecommendation,
        primary: bool,
    ) -> El {
        let p = self.p();
        let mod_w = self.home_module_content_width(HomeModuleId::WeeklyAndAi);
        let (title_budget, reason_budget) = match mod_w {
            HomeModuleWidth::OneThird => (20, 30),
            HomeModuleWidth::OneHalf => (30, 44),
            HomeModuleWidth::TwoThirds => (40, 60),
            HomeModuleWidth::Full => (56, 80),
        };
        let (icon, tint) = intent_visual(&rec.intent, &p);
        let clean_title = rec.title.trim_start_matches("⚠️ ").trim();
        let clean_reason = if rec.reason.starts_with("检测到") && rec.reason.contains('，') {
            rec.reason
                .splitn(2, '，')
                .nth(1)
                .unwrap_or(&rec.reason)
                .trim_start_matches("但")
                .trim()
        } else {
            rec.reason.trim()
        };
        let clean_btn = rec
            .action_label
            .trim_start_matches("⏰ ")
            .replace("手机闹钟", "闹钟");
        let text_col = self.vstack(
            1.,
            vec![
                self.line_w(
                    Self::truncate_chars(clean_title, title_budget),
                    CAPTION,
                    p.text,
                    Weight::Semibold,
                ),
                self.line(
                    Self::truncate_chars(clean_reason, reason_budget),
                    10.,
                    p.text2,
                ),
            ],
        );
        let row_el = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(S2)
                .with_child(self.ui_icon_box(icon, 24., soft_tint(tint, p.dark), tint, None))
                .with_child(Expanded::new(1., text_col).finish())
                .with_child(self.button(
                    format!("home-ai-rec-btn-{}", rec.id),
                    clean_btn.clone(),
                    Action::HomeExecuteIntent(rec.intent.clone()),
                    if primary {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ))
                .finish(),
        )
        .with_horizontal_padding(S2)
        .with_vertical_padding(4.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();
        let p_key = format!("ai_rec:{}", rec.id);
        let rem_target = HomeReminderContextTarget {
            spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                p_key.clone(),
                stepx_companion::home_reminder_governance::ReminderCategory::LedgerAndTaskIssue,
                clean_title.to_string(),
            )
            .with_aliases(vec![p_key, format!("ai_rec:{}", rec.title)])
            .with_subtitle(clean_reason.to_string())
            .with_source("处理建议", None),
            primary_action: Some((
                clean_btn,
                Box::new(Action::HomeExecuteIntent(rec.intent.clone())),
            )),
            copy_text: format!("{clean_title} · {clean_reason}"),
            amoo_prompt: format!("请执行处理建议：{clean_title}（{clean_reason}）"),
            otp_or_pickup_code: String::new(),
            sms_address: String::new(),
            im_thread_id: String::new(),
            notif_key: String::new(),
            calendar_event_id: None,
            capsule_id: String::new(),
            mute_source_key: String::new(),
            mute_source_label: String::new(),
        };
        self.wrap_home_reminder_target(format!("home-rem-ai-rec-{}", rec.id), rem_target, row_el)
    }

    /// Counts are facts, not progress toward arbitrary daily targets. Use
    /// semantic SVG icons and adapt to the actual available card width.
    pub(in crate::ui) fn render_home_kpi_tiles(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let now = Local::now();
        let spans = today_event_spans(&suite.calendar_events, now.date_naive());
        let screen = suite.wellbeing.screen_time_minutes;

        let message_known = self.demo_mode || self.push.loaded;
        let suite_known = self.demo_mode || self.back_screen.stepos_suite_synced_at.is_some();
        let cells = [
            (
                "home-kpi-messages",
                UiIcon::Chat,
                if message_known {
                    report.unread_messages_count.to_string()
                } else {
                    "—".into()
                },
                "未读".to_string(),
                if !message_known {
                    "待同步".to_string()
                } else {
                    String::new()
                },
                p.blue,
                Action::Page(Page::Notifications),
            ),
            (
                "home-kpi-life",
                UiIcon::Package,
                if message_known {
                    report.active_life_cards_count.to_string()
                } else {
                    "—".into()
                },
                "生活提醒".to_string(),
                String::new(),
                p.warning,
                Action::Page(Page::Notifications),
            ),
            (
                "home-kpi-meetings",
                UiIcon::Calendar,
                if suite_known {
                    spans.len().to_string()
                } else {
                    "—".into()
                },
                "日程".to_string(),
                if !suite_known {
                    "待同步".to_string()
                } else {
                    String::new()
                },
                p.indigo,
                Action::Page(Page::Calendar),
            ),
            (
                "home-kpi-screen",
                UiIcon::Phone,
                if !suite_known || (!self.demo_mode && screen == 0) {
                    "—".into()
                } else if screen >= 60 {
                    format!("{}h{:02}", screen / 60, screen % 60)
                } else {
                    format!("{screen}m")
                },
                "亮屏".to_string(),
                if !suite_known || (!self.demo_mode && screen == 0) {
                    "待同步".into()
                } else {
                    String::new()
                },
                p.success_text,
                Action::OpenTool(Some(ToolPage::Wellbeing)),
            ),
        ];

        let mut tiles: Vec<El> = Vec::new();
        for (id, icon, value, label, sub, color, action) in cells
        {
            let state = self.mouse_state(id);
            let tile = Hoverable::new(state, move |mouse| {
                let t = self.hover_t(&mouse);
                let mut col = Flex::column()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(3.)
                    .with_child(self.icon_badge(icon, 16., color, soft_tint(color, p.dark), 28.))
                    .with_child(self.line_w(value, SUBHEAD, p.text, Weight::Semibold))
                    .with_child(self.line_w(label, MICRO, p.text, Weight::Medium));
                if !sub.is_empty() {
                    col = col.with_child(self.line(sub, 10., p.text3));
                }
                Container::new(col.finish())
                    .with_vertical_padding(S2)
                    .with_horizontal_padding(S1)
                    .with_background_color(lerp_color(p.bg, p.hover, t))
                    .with_border(Border::all(1.).with_border_color(lerp_color(
                        p.border,
                        color,
                        t * 0.6,
                    )))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                if id == "home-kpi-meetings" {
                    ctx.dispatch_typed_action(Action::BackScreen(BackScreenAction::CalendarToday));
                }
                ctx.dispatch_typed_action(action.clone());
            })
            .finish();
            #[cfg(test)]
            let tile = SavePosition::new(tile, id).finish();
            tiles.push(Expanded::new(1., tile).finish());
        }
        let mut rows = Vec::new();
        let mut iter = tiles.into_iter().peekable();
        while iter.peek().is_some() {
            let mut row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(S2);
            for tile in iter.by_ref().take(2) {
                row = row.with_child(tile);
            }
            rows.push(row.finish());
        }
        self.responsive_columns(360., S2, S2, CrossAxisAlignment::Start,
            rows.into_iter().map(|row| (1., row)).collect())
    }

    pub(in crate::ui) fn render_home_day_timeline_inner(
        &self,
        _report: &stepx_companion::home_hub::HomeHubReport,
        selected_date: NaiveDate,
        today: NaiveDate,
    ) -> El {
        let p = self.p();
        let suite = &self.back_screen.stepos_suite;
        let now = Local::now();
        let now_ms = now.timestamp_millis();
        let now_min = now.hour() * 60 + now.minute();
        let spans: Vec<_> = today_event_spans(&suite.calendar_events, selected_date)
            .into_iter()
            .filter(|(_, _, ev)| {
                !self.calendar_event_is_completed(ev)
                    && !self.calendar_event_is_suppressed(ev, now_ms)
            })
            .collect();
        let colors = event_colors(&p);
        const START_H: u32 = 6;
        const END_H: u32 = 24;
        let spec = DayTimeline {
            width: 960.,
            height: 40.,
            start_hour: START_H,
            end_hour: END_H,
            now_min,
            events: spans
                .iter()
                .enumerate()
                .map(|(i, (s, e, _))| DialEvent {
                    start_min: *s,
                    end_min: *e,
                    paint: paint_of(colors[i % colors.len()]),
                })
                .collect(),
            alarms: suite
                .alarms
                .iter()
                .filter(|a| {
                    a.enabled
                        && !a.is_e2e_probe()
                        && !a.is_suppressed_at(now_ms, &self.reminder_governance)
                })
                .map(|a| DialAlarm {
                    minute_of_day: a.hour as u32 * 60 + a.minutes as u32,
                    enabled: a.is_today_valid_unexpired_at(now_ms, &self.reminder_governance),
                })
                .collect(),
            track: solid(p.text3, 0.12, p.bg),
            elapsed: solid(p.text3, 0.28, p.bg),
            tick: solid(p.text3, 0.8, p.bg),
            now: paint_of(p.danger),
            alarm: paint_of(p.warning),
            matte: paint_of(p.bg).with_alpha(1.0),
        };
        let chart = home_native_plot(
            "home-day-timeline-plot",
            HomePlotContent::Timeline(spec),
            40.,
        );

        // Hour labels: one cell per 3h interval, left-aligned on the tick.
        let mut labels = Flex::row().with_main_axis_size(MainAxisSize::Max);
        let mut h = START_H;
        while h < END_H {
            labels = labels.with_child(
                Expanded::new(
                    1.,
                    Align::new(self.mono(format!("{h:02}"), 10., p.text3))
                        .left()
                        .finish(),
                )
                .finish(),
            );
            h += 3;
        }
        labels = labels.with_child(self.mono("24", 10., p.text3));

        // Next / current event summary.
        let current = (selected_date == today)
            .then(|| spans.iter().find(|(s, e, _)| *s <= now_min && now_min < *e))
            .flatten();
        let next = if selected_date == today {
            spans.iter().find(|(s, _, _)| *s > now_min)
        } else {
            spans.first()
        };
        let summary: El = match (current, next) {
            (Some((_, e, ev)), _) => self.line(
                format!(
                    "进行中 {} · 还剩 {}",
                    Self::truncate_chars(ev.title.trim(), 12),
                    fmt_minutes(e - now_min)
                ),
                CAPTION,
                p.text2,
            ),
            (None, Some((s, _, ev))) => {
                let hm = format!("{:02}:{:02}", s / 60, s % 60);
                let t = Self::truncate_chars(ev.title.trim(), 12);
                self.line(
                    if selected_date == today && *s > now_min {
                        format!("下一项 {hm} {t} · {}后", fmt_minutes(s - now_min))
                    } else {
                        format!("首项 {hm} {t}")
                    },
                    CAPTION,
                    p.text2,
                )
            }
            (None, None) => self.line(
                if spans.is_empty() {
                    "暂无日程"
                } else {
                    "日程已结束"
                },
                CAPTION,
                p.text2,
            ),
        };

        let header = home_geometry_probe(
            "home-day-timeline-header",
            home_geometry_probe(
                "home-day-timeline-title",
                self.hstack(
                    S2,
                    vec![
                        self.content_icon(UiIcon::Clock, 15., p.indigo),
                        self.heading(if selected_date == today {
                            "时间轴".to_string()
                        } else {
                            format!("{} 时间轴", selected_date.format("%m-%d"))
                        }),
                    ],
                ),
            ),
        );
        let summary = home_geometry_probe("home-day-timeline-summary", summary);

        let legend: Vec<El> = spans
            .iter()
            .take(3)
            .enumerate()
            .map(|(i, (s, _, ev))| {
                let title: String = Self::truncate_chars(ev.title.trim(), 10);
                self.hstack(
                    4.,
                    vec![
                        self.dot(colors[i % colors.len()]),
                        self.mono(format!("{:02}:{:02}", s / 60, s % 60), MICRO, p.text2),
                        self.line(title, MICRO, p.text2),
                    ],
                )
            })
            .collect();

        let axis = Container::new(labels.finish())
            .with_horizontal_padding(6.)
            .finish();
        let mut children = vec![header, summary, self.vstack(2., vec![chart, axis])];
        if !legend.is_empty() {
            children.push(home_geometry_probe(
                "home-day-timeline-event-legend",
                self.wrap(legend),
            ));
        }
        self.vstack(S2, children)
    }

    // =========================================================================
    // ROW 5 LEFT: SVG APP-USAGE DONUT
    // =========================================================================
    pub(in crate::ui) fn render_home_habit_donut_card(&self) -> El {
        let p = self.p();
        let raw_h = self
            .home_modules
            .module_height(HomeModuleId::BackScreenAndHabits);
        let eff_h = self.resolve_effective_height(HomeModuleId::BackScreenAndHabits, raw_h);
        let top_take = if eff_h == HomeModuleHeight::Tall {
            5
        } else {
            3
        };
        let suite = &self.back_screen.stepos_suite;
        let suite_known =
            self.demo_mode || (suite.checked && self.back_screen.stepos_suite_synced_at.is_some());
        let wb = &suite.wellbeing;
        let colors = [p.blue, p.success_text, p.indigo, p.pink, p.warning];
        let total = wb.screen_time_minutes;
        let top: Vec<(String, u32)> = wb
            .top_apps
            .iter()
            .take(top_take)
            .map(|(pkg, label, mins)| (resolve_top_app_display_name(pkg, label).to_string(), *mins))
            .collect();
        let top_sum: u32 = top.iter().map(|(_, m)| *m).sum();
        let other = total.saturating_sub(top_sum);
        let mut segments: Vec<(f32, Paint)> = top
            .iter()
            .enumerate()
            .map(|(i, (_, m))| (*m as f32, paint_of(colors[i % colors.len()])))
            .collect();
        if other > 0 {
            segments.push((other as f32, solid(p.text3, 0.35, p.bg)));
        }
        let size = 88.;
        let donut_svg = donut(size, 13., &segments, solid(p.text3, 0.12, p.bg), 0.012);
        let ring = self.svg_canvas("usage-donut", donut_svg, Some(size), size, false);
        let center = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::Center)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_child(self.line_w(
                if suite_known {
                    total.to_string()
                } else {
                    "—".into()
                },
                16.,
                p.text,
                Weight::Semibold,
            ))
            .with_child(self.line(if suite_known { "分钟" } else { "待同步" }, 9., p.text2))
            .finish();
        let donut_el = overlay(size, size, ring, vec![center]);

        let denom = total.max(top_sum).max(1);
        let mut legend_rows: Vec<El> = top
            .iter()
            .enumerate()
            .map(|(i, (name, mins))| {
                let color = colors[i % colors.len()];
                let name_str = name.clone();
                let stat_str = format!("{mins}m · {}%", mins * 100 / denom);
                let row_id = format!("home-habit-top-app-{i}");
                let state = self.mouse_state(&row_id);
                let row_el = Hoverable::new(state, move |mouse| {
                    let t = self.hover_t(&mouse);
                    Container::new(row_between_expanded(
                        self.hstack(
                            6.,
                            vec![
                                self.dot(color),
                                self.line_w(&name_str, CAPTION, p.text, Weight::Medium),
                            ],
                        ),
                        self.mono(&stat_str, MICRO, lerp_color(p.text2, p.text, t)),
                        S2,
                    ))
                    .with_horizontal_padding(4.)
                    .with_vertical_padding(2.)
                    .with_background_color(lerp_color(ColorU::transparent_black(), p.hover, t))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(|ctx, _, _| {
                    ctx.dispatch_typed_action(Action::OpenTool(Some(ToolPage::Wellbeing)))
                })
                .finish();
                home_geometry_probe(row_id, row_el)
            })
            .collect();
        if other > 0 {
            legend_rows.push(row_between_expanded(
                self.hstack(
                    6.,
                    vec![
                        self.dot(lerp_color(p.bg, p.text3, 0.5)),
                        self.line("其他", CAPTION, p.text2),
                    ],
                ),
                self.mono(
                    format!("{other}m · {}%", other * 100 / denom),
                    MICRO,
                    p.text3,
                ),
                S2,
            ));
        }
        if legend_rows.is_empty() {
            legend_rows.push(self.line("暂无数据", CAPTION, p.text3));
        }

        let header = row_between_expanded(
            self.hstack(
                S2,
                vec![
                    self.home_section_heading(UiIcon::Activity, "屏幕时长"),
                    self.pill_badge(
                        if suite_known {
                            format!("解锁 {} 次", wb.unlock_count)
                        } else {
                            "等待同步".into()
                        },
                        p.text2,
                        p.surface,
                    ),
                ],
            ),
            self.hstack(
                S1,
                vec![
                    self.icon_action(
                        "home-rec-refresh-suite",
                        "刷新使用习惯",
                        UiIcon::Refresh,
                        vec![Action::BackScreen(BackScreenAction::InspectStepOsSuite)],
                        false,
                    ),
                    self.link_muted(
                        "home-rec-open-wellbeing",
                        "详情 →",
                        Action::OpenTool(Some(ToolPage::Wellbeing)),
                    ),
                ],
            ),
            S2,
        );

        let body = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S3)
            .with_child(donut_el)
            .with_child(Expanded::new(1., self.vstack(4., legend_rows)).finish())
            .finish();

        self.panel(vec![header, body])
    }

    pub(in crate::ui) fn render_home_device_and_backscreen_studio_card(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
        _metrics: &stepx_companion::device::DeviceMetrics,
    ) -> El {
        thread_local! {
            static HOME_SIG_FRAME_CACHE: std::cell::RefCell<Option<(String, bool, Vec<u8>)>> =
                const { std::cell::RefCell::new(None) };
        }
        let p = self.p();
        let raw_h = self
            .home_modules
            .module_height(HomeModuleId::BackScreenAndHabits);
        let eff_h = self.resolve_effective_height(HomeModuleId::BackScreenAndHabits, raw_h);
        let active_sig = self.current_backscreen_signature(report);
        let synthesized_frame;
        let frame: &[u8] = if self.back_screen.frame.iter().any(|&b| b > 0) {
            &self.back_screen.frame
        } else {
            let inv = self.back_screen.config.invert;
            synthesized_frame = HOME_SIG_FRAME_CACHE.with(|c| {
                if let Some((ref sig, prev_inv, ref cached)) = *c.borrow() {
                    if sig == active_sig && prev_inv == inv {
                        return cached.clone();
                    }
                }
                let built = stepx_companion::backscreen::synthesize_chinese_signature_frame(
                    active_sig, inv,
                );
                *c.borrow_mut() = Some((active_sig.to_string(), inv, built.clone()));
                built
            });
            &synthesized_frame
        };

        let header = row_between_expanded(
            self.home_section_heading(UiIcon::Matrix, "背屏"),
            self.link(
                "home-open-backscreen-page",
                "更多 →",
                Action::Page(Page::BackScreen),
            ),
            S2,
        );

        // Visual Phone Rear Housing + Live 17x23 LED Matrix Canvas side-by-side
        let rear_art = self.phone_rear_housing(54., 110., frame, 1.9, 0.55);
        let led_matrix_box = Container::new(
            LedGrid::new(frame, 2.25, 0.65)
                .on_pixel_click(|x, y| Action::BackScreen(BackScreenAction::TogglePixel(x, y)))
                .finish(),
        )
        .with_uniform_padding(5.)
        .with_background_color(rgb(20, 20, 26))
        .with_border(Border::all(1.).with_border_color(rgb(46, 46, 56)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();

        let pomo_label = if self.back_screen.pomodoro_active {
            let rem = self.live_pomodoro_remaining_secs();
            format!("番茄钟 {:02}:{:02}", rem / 60, rem % 60)
        } else {
            format!("签名：{}", Self::truncate_chars(active_sig, 8))
        };

        let matrix_side = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(2.)
            .with_child(led_matrix_box)
            .with_child(self.line_w(pomo_label, 10., p.text2, Weight::Medium))
            .finish();

        let visual_showcase = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::Center)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(S3)
                .with_child(rear_art)
                .with_child(matrix_side)
                .finish(),
        )
        .with_vertical_padding(6.)
        .with_horizontal_padding(S2)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();

        let status_presets = ["开会勿扰", "专注中", "去吃饭", "休息一下"];
        let preset_take = 2;
        let mut sig_chips: Vec<El> = status_presets
            .iter()
            .take(preset_take)
            .enumerate()
            .map(|(idx, preset)| {
                let active = active_sig == *preset;
                self.chip(
                    format!("home-bs-sig-{idx}"),
                    *preset,
                    None,
                    vec![Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus(
                        (*preset).into(),
                    ))],
                    active,
                    false,
                )
            })
            .collect();
        sig_chips.push(self.chip(
            "home-bs-open-signature",
            "签名",
            Some(UiIcon::Clock),
            vec![Action::BackScreen(BackScreenAction::Tab(3))],
            false,
            false,
        ));

        let mut panel_children = vec![header, visual_showcase, self.wrap(sig_chips)];
        if eff_h == HomeModuleHeight::Tall {
            let custom_sig_row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(6.)
                .with_child(
                    Expanded::new(1., self.input(F_BACK_MARQUEE, "自定义签名…", None)).finish(),
                )
                .with_child(self.button_icon(
                    "home-bs-apply-custom-sig",
                    "推送",
                    UiIcon::Matrix,
                    Action::BackScreen(BackScreenAction::SaveMarquee),
                    Btn::Secondary,
                ))
                .finish();
            panel_children.push(custom_sig_row);
        }

        self.panel(panel_children)
    }

    // =========================================================================
    // MODULE 1: 💼 工作模块 (所选日日程与会议 · 跨端连续工作区与产出 · 番茄钟专注)
    // =========================================================================
    pub(in crate::ui) fn live_pomodoro_remaining_secs(&self) -> u32 {
        if !self.back_screen.pomodoro_active {
            return self.back_screen.pomodoro_last_remaining_secs.unwrap_or(0);
        }
        if self.back_screen.pomodoro_started_at.is_some() {
            let live = self.back_screen.pomodoro_remaining_secs();
            if let Some(cached) = self.back_screen.pomodoro_last_remaining_secs {
                live.min(cached)
            } else {
                live
            }
        } else {
            self.back_screen
                .pomodoro_last_remaining_secs
                .unwrap_or(self.back_screen.pomodoro_duration_secs)
        }
    }

    pub(in crate::ui) fn home_checked_checklist_items_path(&self) -> Option<std::path::PathBuf> {
        self.back_screen
            .calendar_state_dir()
            .map(|dir| dir.join("home-checked-checklist-items.json"))
    }

    pub(in crate::ui) fn save_home_checked_checklist_items(&self) -> anyhow::Result<()> {
        let path = self.home_checked_checklist_items_path()
            .ok_or_else(|| anyhow::anyhow!("请连接原手机后保存日程清单"))?;
        let json = serde_json::to_vec_pretty(&self.home_checked_checklist_items)?;
        crate::ui::backscreen_state::persist_calendar_state(&path, &json)
    }

    pub(in crate::ui) fn load_home_checked_checklist_items(&mut self) {
        self.home_checked_checklist_items.clear();
        let Some(path) = self.home_checked_checklist_items_path() else {
            return;
        };
        if path.is_symlink() || std::fs::metadata(&path).is_ok_and(|m| m.len() > 512 * 1024) {
            return;
        }
        let Ok(raw) = std::fs::read_to_string(path) else {
            return;
        };
        if let Ok(items) = serde_json::from_str::<std::collections::BTreeSet<String>>(&raw) {
            self.home_checked_checklist_items = items;
        }
    }

    pub(in crate::ui) fn checklist_item_state_key(event_id: i64, item: &str) -> String {
        format!("{event_id}:{}", item.trim())
    }

    pub(in crate::ui) fn is_checklist_item_checked(&self, event_id: i64, item: &str) -> bool {
        let key = Self::checklist_item_state_key(event_id, item);
        self.home_checked_checklist_items.contains(&key)
    }

    pub(in crate::ui) fn build_unchecked_checklist_backscreen_text(
        &self,
        checklist: &stepx_companion::home_hub::HomeScheduleChecklist,
    ) -> String {
        let unchecked: Vec<&str> = checklist
            .items
            .iter()
            .map(|s| s.as_str())
            .filter(|item| !self.is_checklist_item_checked(checklist.event_id, item))
            .collect();
        if unchecked.is_empty() {
            format!(
                "{} 清单已齐",
                Self::truncate_chars(&checklist.event_title, 6)
            )
        } else {
            format!(
                "待办: {}",
                unchecked
                    .into_iter()
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" · ")
            )
        }
    }

    pub(in crate::ui) fn render_home_calendar_and_work_column(
        &self,
        _report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        let p = self.p();
        let now_local = Local::now();
        let now_work_ms = now_local.timestamp_millis();
        let today = now_local.date_naive();
        let suite = &self.back_screen.stepos_suite;
        let selected_date = self.home_selected_date();
        let sel_y = selected_date.year();
        let sel_m = selected_date.month();
        let sel_d = selected_date.day();
        let selected_ymd = format!("{sel_y:04}-{sel_m:02}-{sel_d:02}");
        let schedule_checklists: Vec<_> =
            stepx_companion::home_hub::extract_schedule_checklists_for_date(
                &selected_ymd,
                &suite.calendar_events,
            )
            .into_iter()
            .filter(|cl| {
                self.calendar_event_action_limit(cl.event_id).is_none() && !self
                    .back_screen
                    .completed_calendar_events
                    .contains(&cl.event_id)
                    && !self.reminder_governance.is_any_key_suppressed(
                        &[
                            format!("checklist:{}", cl.event_title),
                            format!("cal:{}", cl.event_id),
                        ],
                        None,
                        None,
                        None,
                        now_work_ms,
                    )
            })
            .collect();

        let schedule_ttl_ms = self.reminder_governance.effective_ttl_ms(
            stepx_companion::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
        );
        let eligible_events = suite.calendar_events.iter().filter(|ev| {
            let expired_today = selected_date == today
                && !ev.all_day
                && ev.dtstart > 0
                && now_work_ms.saturating_sub(home_work_event_end_ms(ev)) > schedule_ttl_ms;
            !expired_today
                && !self.calendar_event_is_completed(ev)
                && !self.calendar_event_is_suppressed(ev, now_work_ms)
                && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!(
                    "{} {} {}", ev.title, ev.location, ev.description
                ))
        });

        let genuine_remote_files: Vec<_> = suite
            .remote_agent_files
            .iter()
            .filter(|f| !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&f.path))
            .collect();

        let header = row_between_expanded(
            self.home_section_heading(UiIcon::Briefcase, "工作安排"),
            self.hstack(
                2.,
                vec![
                    self.button(
                        "home-continue-amoo",
                        "历史",
                        Action::HomeShowHistory,
                        Btn::Ghost,
                    ),
                    self.button(
                        "home-work-open-page",
                        "工作台",
                        Action::Page(Page::Work),
                        Btn::Ghost,
                    ),
                ],
            ),
            4.,
        );

        // 1. Quick Inline Event Creator on Selected Date
        let quick_create_row = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(6.)
            .with_child(
                Expanded::new(1., self.input(F_PHONE_SCHEDULE_TITLE, "添加日程…", None)).finish(),
            )
            .with_child(self.input(F_CALENDAR_TIME, "15:00", Some(64.)))
            .with_child(self.button_icon(
                "home-cal-create-btn",
                "添加",
                UiIcon::Plus,
                Action::BackScreen(BackScreenAction::CalendarCreateOnSelectedDate(None)),
                Btn::Secondary,
            ))
            .finish();

        // Show the actual next appointments, independently of insertion order.
        let mod_w = self.home_module_content_width(HomeModuleId::WorkModule);
        let raw_h = self.home_modules.module_height(HomeModuleId::WorkModule);
        let mod_h = self.resolve_effective_height(HomeModuleId::WorkModule, raw_h);
        let (title_budget, loc_budget) = match mod_w {
            HomeModuleWidth::OneThird => (18, 14),
            HomeModuleWidth::OneHalf => (26, 20),
            HomeModuleWidth::TwoThirds => (34, 26),
            HomeModuleWidth::Full => (48, 36),
        };
        let ev_chip_limit = match mod_w {
            HomeModuleWidth::OneThird => 2,
            HomeModuleWidth::OneHalf => 3,
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 3,
        };
        let cl_cols = match mod_w {
            HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 2,
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 3,
        };
        let max_sel_events = match (mod_h, mod_w) {
            (HomeModuleHeight::Compact, HomeModuleWidth::OneThird) => 1,
            (HomeModuleHeight::Compact, _) => 2,
            (HomeModuleHeight::Standard | HomeModuleHeight::Auto, _) => 3,
            (HomeModuleHeight::Tall, _) => 5,
        };
        let (display_events, showing_fallback, available_events) = home_work_agenda(
            eligible_events,
            selected_date,
            today,
            now_work_ms,
            max_sel_events,
        );
        let visible_events = display_events.len();

        let mut event_rows: Vec<El> = Vec::new();
        for (idx, ev) in display_events.into_iter().enumerate() {
            let loc = home_work_location(ev);
            let is_ended_today = !showing_fallback
                && selected_date == today
                && !ev.all_day
                && home_work_event_end_ms(ev) <= now_work_ms;
            let is_ongoing_today = !showing_fallback
                && selected_date == today
                && !ev.all_day
                && ev.dtstart <= now_work_ms
                && home_work_event_end_ms(ev) > now_work_ms;
            let cal_keys =
                stepx_companion::home_reminder_governance::canonical_keys_for_calendar_event(ev.id);
            let cal_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("cal:{}", ev.id),
                    stepx_companion::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
                    ev.title.clone(),
                )
                .with_aliases(cal_keys)
                .with_subtitle(format!("{} · {loc}", ev.time_range_label())),
                calendar_event_id: Some(ev.id),
                ..Default::default()
            };
            let amoo_prompt = format!(
                "请帮我围绕会议日程「{}（{}，地点：{loc}）」准备一份简明议程与会议纪要模板",
                ev.title,
                ev.time_range_label()
            );
            let bar_col = if is_ended_today {
                p.text3
            } else if idx == 0 {
                p.blue
            } else {
                p.indigo
            };
            let accent_bar = ConstrainedBox::new(
                Rect::new()
                    .with_background_color(bar_col)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.)))
                    .finish(),
            )
            .with_width(4.)
            .with_height(32.)
            .finish();

            let (_, em, ed) = ev.date_ymd();
            let sub_date_label = if showing_fallback {
                format!("{em:02}/{ed:02} {}", ev.source_label())
            } else if ev.date_ymd() < (sel_y, sel_m, sel_d) {
                "跨日延续".into()
            } else {
                ev.source_label().replace("双端已同步", "已同步")
            };

            let time_col = ConstrainedBox::new(self.vstack(
                1.,
                vec![
                    self.mono(
                        ev.hm_label(),
                        CAPTION,
                        if is_ended_today { p.text3 } else { p.text },
                    ),
                    self.line(sub_date_label, 10., p.text3),
                ],
            ))
            .with_width(48.)
            .finish();

            let clean_time_range = ev.time_range_label();
            let short_time_range = clean_time_range.trim_start_matches("今天 ");
            let mut title_row_items = vec![self.line_w(
                Self::truncate_chars(&ev.title, title_budget),
                CAPTION,
                if is_ended_today { p.text3 } else { p.text },
                Weight::Semibold,
            )];
            if is_ended_today {
                title_row_items.push(self.pill_badge("已结束", p.text3, p.bg));
            } else if is_ongoing_today {
                title_row_items.push(self.pill_badge("进行中", p.success_text, p.success_soft));
            }
            let detail_col = self.vstack(
                1.,
                vec![
                    self.hstack(4., title_row_items),
                    self.line(
                        format!(
                            "{short_time_range} · {}",
                            Self::truncate_chars(&loc, loc_budget)
                        ),
                        MICRO,
                        if is_ended_today { p.text3 } else { p.text2 },
                    ),
                ],
            );

            let meeting_url = stepx_companion::home_hub::extract_meeting_url(&format!(
                "{} {}",
                ev.location, ev.description
            ));
            let mut ev_chips = Vec::new();
            if let Some(url) = meeting_url {
                ev_chips.push(self.chip(
                    format!("home-cal-join-{}", ev.id),
                    "入会",
                    Some(UiIcon::ArrowUpRight),
                    vec![Action::Push(PushAction::OpenUrl(url))],
                    true,
                    false,
                ));
            }
            ev_chips.push(self.chip(
                format!("home-cal-bs-{}", ev.id),
                "钉背屏",
                None,
                vec![Action::BackScreen(
                    BackScreenAction::CalendarSendReminderToPhone(ev.id),
                )],
                false,
                false,
            ));
            ev_chips.push(self.chip(
                format!("home-cal-amoo-{}", ev.id),
                "纪要",
                None,
                vec![Action::SendToAmoo(amoo_prompt)],
                false,
                false,
            ));
            ev_chips.push(self.chip(
                format!("home-cal-expire-{}", ev.id),
                "不再提醒",
                Some(UiIcon::Check),
                vec![Action::HomeReminder(HomeReminderAction::ManualExpire(
                    cal_target.clone(),
                ))],
                false,
                false,
            ));
            ev_chips.push(self.chip(
                format!("home-cal-del-{}", ev.id),
                "删除",
                None,
                vec![Action::BackScreen(BackScreenAction::CalendarDelete(ev.id))],
                false,
                false,
            ));
            ev_chips.truncate(ev_chip_limit);
            let actions_col = self.hstack(4., ev_chips);

            let row_el = Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(accent_bar)
                    .with_child(time_col)
                    .with_child(Expanded::new(1., detail_col).finish())
                    .with_child(actions_col)
                    .finish(),
            )
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(5.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();
            event_rows.push(self.wrap_home_reminder_target(
                format!("home-rem-work-cal-{}", ev.id),
                cal_target,
                row_el,
            ));
        }

        if event_rows.is_empty() {
            event_rows.push(
                Container::new(self.line("暂无日程", MICRO, p.text3))
                    .with_horizontal_padding(S2 + 2.)
                    .with_vertical_padding(5.)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            );
        }

        event_rows.push(row_between_expanded(
            self.line(
                if showing_fallback {
                    format!("后续安排 · 显示 {visible_events}/{available_events} 项")
                } else if available_events > visible_events {
                    format!("当日安排 · 显示 {visible_events}/{available_events} 项")
                } else {
                    format!("{sel_m:02}/{sel_d:02} · {available_events} 项日程")
                },
                MICRO,
                p.text3,
            ),
            self.button_icon(
                "home-work-all-calendar",
                "全部日程",
                UiIcon::Calendar,
                Action::Page(Page::Calendar),
                Btn::Ghost,
            ),
            6.,
        ));

        // 2b. Interactive Schedule Action Checklists (P0/P1 Actionable Checklist Extraction)
        let max_checklists = match mod_h {
            HomeModuleHeight::Compact => 0,
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => 1,
            HomeModuleHeight::Tall => 2,
        };
        let mut checklist_cards: Vec<El> = Vec::new();
        for (c_idx, cl) in schedule_checklists.iter().take(max_checklists).enumerate() {
            let total_count = cl.items.len();
            let done_count = cl
                .items
                .iter()
                .filter(|it| self.is_checklist_item_checked(cl.event_id, it))
                .count();
            let bs_text = self.build_unchecked_checklist_backscreen_text(cl);
            let cl_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("checklist:{}", cl.event_title),
                    stepx_companion::home_reminder_governance::ReminderCategory::ScheduleAndMeeting,
                    format!("会前准备 · {}", cl.event_title),
                )
                .with_subtitle(format!("已完成 {done_count}/{total_count}")),
                ..Default::default()
            };
            let item_pills: Vec<El> = cl
                .items
                .iter()
                .enumerate()
                .map(|(i, item)| {
                    let checked = self.is_checklist_item_checked(cl.event_id, item);
                    let key = Self::checklist_item_state_key(cl.event_id, item);
                    let id = format!("home-cal-chk-{}-{i}", cl.event_id);
                    let state = self.mouse_state(id);
                    let concise_item = item
                        .replace("行李打包、孩子衣物与常备药", "行李打包、衣物与常备药")
                        .replace("检查油量/电量、胎压、玻璃水", "检查油电、胎压与玻璃水")
                        .replace("手机与车机导航离线地图下载", "手机与车机离线地图下载")
                        .replace("证件、ETC、充电线、充电宝", "证件、ETC 与充电宝");
                    let label = Self::truncate_chars(&concise_item, loc_budget.max(13));
                    Hoverable::new(state, move |mouse| {
                        let t = self.hover_t(&mouse);
                        let bg = if checked {
                            p.selected
                        } else {
                            lerp_color(p.surface, p.hover, t)
                        };
                        let border = if checked {
                            p.accent
                        } else {
                            lerp_color(p.border, p.border_strong, t)
                        };
                        let fg = if checked { p.accent } else { p.text };
                        Container::new(self.hstack(
                            4.,
                            vec![
                                self.content_icon(
                                    if checked {
                                        UiIcon::Check
                                    } else {
                                        UiIcon::Square
                                    },
                                    11.,
                                    fg,
                                ),
                                self.line_w(&label, 10.5, fg, Weight::Medium),
                            ],
                        ))
                        .with_horizontal_padding(5.)
                        .with_vertical_padding(3.5)
                        .with_background_color(bg)
                        .with_border(Border::all(1.).with_border_color(border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                        .finish()
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(move |ctx, _, _| {
                        ctx.dispatch_typed_action(Action::HomeToggleChecklistItem(key.clone()))
                    })
                    .finish()
                })
                .collect();
            let cl_header = row_between_expanded(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(self.pill_badge(
                        format!("已完成 {done_count}/{total_count}"),
                        if done_count == total_count {
                            p.success_text
                        } else {
                            p.blue
                        },
                        if done_count == total_count {
                            p.success_soft
                        } else {
                            soft_tint(p.blue, p.dark)
                        },
                    ))
                    .with_child(
                        Expanded::new(
                            1.,
                            self.hstack(
                                4.,
                                vec![
                                    self.content_icon(UiIcon::File, 12., p.blue),
                                    self.line_w(
                                        Self::truncate_chars(&cl.event_title, title_budget),
                                        CAPTION,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                ],
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
                self.chip(
                    format!("home-cal-chk-bs-{}", cl.event_id),
                    "推背屏",
                    Some(UiIcon::Matrix),
                    vec![Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus(
                        bs_text,
                    ))],
                    true,
                    false,
                ),
                6.,
            );
            let cl_card = Container::new(
                self.vstack(5., vec![cl_header, self.grid_cols(item_pills, cl_cols, 3.)]),
            )
            .with_horizontal_padding(7.)
            .with_vertical_padding(5.)
            .with_background_color(soft_tint(p.blue, p.dark))
            .with_border(Border::all(1.).with_border_color(p.blue))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();
            checklist_cards.push(self.wrap_home_reminder_target(
                format!("home-rem-checklist-{c_idx}"),
                cl_target,
                cl_card,
            ));
        }

        // 3. Resumable Workspace Tasks (`接着上次做`)
        let recent_tasks: Vec<_> = self
            .home_recent_tasks()
            .into_iter()
            .filter(|task| {
                !self.reminder_governance.is_any_key_suppressed(
                    &[format!("task:{}", task.id)],
                    None,
                    None,
                    None,
                    now_work_ms,
                )
            })
            .collect();
        let max_recent_tasks = match mod_h {
            HomeModuleHeight::Compact => 0,
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => 1,
            HomeModuleHeight::Tall => 2,
        };
        let mut task_rows: Vec<El> = Vec::new();
        for (index, task) in recent_tasks.iter().take(max_recent_tasks).enumerate() {
            let is_phone = task.id.starts_with("phone-work-");
            let awaiting_delivery = is_phone
                && (task.route_note.contains("待回传")
                    || task.note.contains("待回传")
                    || task.outputs.iter().any(|o| o.kind.contains("待回传")));
            let queued = is_phone && task.route_note.contains("排队");
            let badge_txt = if awaiting_delivery {
                "待回传手机"
            } else if queued {
                "排队待处理"
            } else if !task.outputs.is_empty() {
                "已有成果"
            } else if !task.draft.trim().is_empty() {
                "草稿待发"
            } else {
                "讨论留存"
            };

            let mut badges = vec![self.pill_badge(badge_txt, p.accent, p.accent_soft)];
            if is_phone {
                badges.push(self.pill_badge("手机发起", p.blue, soft_tint(p.blue, p.dark)));
            }
            badges.push(self.line_w(
                Self::truncate_chars(&task.title, title_budget),
                CAPTION,
                p.text,
                Weight::Semibold,
            ));
            let sub_meta = if is_phone && !task.note.trim().is_empty() {
                format!(
                    "{} · 更新于 {}",
                    Self::truncate_chars(&task.note, loc_budget.max(20)),
                    home_updated_label(task.updated)
                )
            } else {
                format!("更新于 {}", home_updated_label(task.updated))
            };
            let task_target = HomeReminderContextTarget {
                spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                    format!("task:{}", task.id),
                    stepx_companion::home_reminder_governance::ReminderCategory::LedgerAndTaskIssue,
                    task.title.clone(),
                )
                .with_subtitle(sub_meta.clone()),
                ..Default::default()
            };
            let task_row_el = Container::new(row_between_expanded(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(self.ui_icon_box(
                        if is_phone {
                            UiIcon::Smartphone
                        } else {
                            UiIcon::File
                        },
                        24.,
                        soft_tint(p.accent, p.dark),
                        p.accent,
                        None,
                    ))
                    .with_child(
                        Expanded::new(
                            1.,
                            self.vstack(
                                1.,
                                vec![self.hstack(4., badges), self.line(sub_meta, 10., p.text3)],
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
                self.button_icon(
                    format!("home-work-mod-resume-{}", task.id),
                    if !task.draft.trim().is_empty() {
                        "继续写"
                    } else {
                        "接着做"
                    },
                    UiIcon::ArrowUpRight,
                    Action::HomeResumeTask(task.id.clone()),
                    if index == 0 {
                        Btn::Primary
                    } else {
                        Btn::Secondary
                    },
                ),
                6.,
            ))
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(5.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();
            task_rows.push(self.wrap_home_reminder_target(
                format!("home-rem-work-task-{}", task.id),
                task_target,
                task_row_el,
            ));
        }

        // 4. Visual Work & Remote Agent Deliverables Card
        let mut file_chips: Vec<El> = Vec::new();
        if !genuine_remote_files.is_empty() {
            file_chips.push(self.chip(
                "home-agent-file-pull-0",
                "拉取产出",
                Some(UiIcon::Download),
                vec![Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                    "pull_remote_agent_files".into(),
                ))],
                false,
                false,
            ));
        }
        file_chips.push(self.chip(
            "home-work-weekly-doc",
            "周回顾",
            Some(UiIcon::Calendar),
            vec![Action::HomeExecuteIntent(HomeAiIntent::OpenPage(
                "review-week".into(),
            ))],
            true,
            false,
        ));
        file_chips.push(self.chip(
            "home-work-studio",
            "作品库",
            Some(UiIcon::Package),
            vec![Action::Page(Page::Studio)],
            false,
            false,
        ));

        let mut work_right: Vec<El> = Vec::new();
        if !self.back_screen.pomodoro_active {
            work_right.push(self.chip(
                "home-pomodoro-start-25",
                "番茄钟",
                Some(UiIcon::Clock),
                vec![Action::BackScreen(BackScreenAction::StartPomodoro(25))],
                false,
                false,
            ));
        }

        let work_header = row_between_expanded(
            self.hstack(
                6.,
                vec![
                    self.ui_icon_box(
                        UiIcon::Briefcase,
                        22.,
                        soft_tint(p.blue, p.dark),
                        p.blue,
                        None,
                    ),
                    self.line_w("文档与产出", CAPTION, p.text, Weight::Semibold),
                ],
            ),
            self.hstack(6., work_right),
            6.,
        );

        let work_box = Container::new(self.vstack(5., vec![work_header, self.wrap(file_chips)]))
            .with_horizontal_padding(S2 + 2.)
            .with_vertical_padding(5.)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();

        // 5. Visual Pomodoro Focus Bar (shown when active)
        let pomo_card_opt = if self.back_screen.pomodoro_active {
            let dur = self.back_screen.pomodoro_duration_secs.max(1);
            let rem = self.live_pomodoro_remaining_secs();
            let elapsed_pct = (((dur.saturating_sub(rem)) * 100) / dur).min(100) as u8;
            let pomo_title = format!("背屏专注 {:02}:{:02}", rem / 60, rem % 60);
            let pomo_btn = self.button(
                "home-pomodoro-stop",
                "停止",
                Action::BackScreen(BackScreenAction::StopPomodoro),
                Btn::Danger,
            );
            Some(
                Container::new(self.vstack(
                    4.,
                    vec![
                        row_between_expanded(
                            self.hstack(
                                6.,
                                vec![
                                    self.ui_icon_box(
                                        UiIcon::Clock,
                                        22.,
                                        soft_tint(p.indigo, p.dark),
                                        p.indigo,
                                        None,
                                    ),
                                    self.line_w(pomo_title, CAPTION, p.text, Weight::Semibold),
                                ],
                            ),
                            pomo_btn,
                            6.,
                        ),
                        self.progress(Some(elapsed_pct), p.indigo),
                    ],
                ))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(6.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            )
        } else {
            None
        };

        if task_rows.is_empty()
            && (mod_h == HomeModuleHeight::Tall
                || matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full))
        {
            let fallback_limit = if mod_h == HomeModuleHeight::Tall {
                2
            } else {
                1
            };
            for (idx, item) in self
                .context
                .overview
                .items
                .iter()
                .filter(|it| {
                    !self.reminder_governance.is_any_key_suppressed(
                        &[format!("ledger:{}", it.id)],
                        None,
                        None,
                        None,
                        now_work_ms,
                    ) && !stepx_companion::home_hub::is_synthetic_e2e_probe_text(&format!(
                        "{} {}",
                        it.title, it.summary
                    ))
                })
                .take(fallback_limit)
                .enumerate()
            {
                let disp_title = Self::ledger_item_display_title(item);
                let disp_detail = Self::ledger_item_display_detail(item);
                let path = item.path.clone();
                let ctx_target = HomeReminderContextTarget {
                    spec: stepx_companion::home_reminder_governance::ReminderTargetSpec::new(
                        format!("ledger:{}", item.id),
                        stepx_companion::home_reminder_governance::ReminderCategory::LedgerAndTaskIssue,
                        disp_title.clone(),
                    )
                    .with_subtitle(disp_detail.clone()),
                    ..Default::default()
                };
                let ctx_row_el = Container::new(row_between_expanded(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_spacing(6.)
                        .with_child(self.ui_icon_box(
                            UiIcon::Layers,
                            24.,
                            soft_tint(p.accent, p.dark),
                            p.accent,
                            None,
                        ))
                        .with_child(
                            Expanded::new(
                                1.,
                                self.vstack(
                                    1.,
                                    vec![
                                        self.hstack(
                                            4.,
                                            vec![
                                                self.pill_badge(
                                                    "连续推进",
                                                    p.accent,
                                                    p.accent_soft,
                                                ),
                                                self.line_w(
                                                    Self::truncate_chars(&disp_title, title_budget),
                                                    CAPTION,
                                                    p.text,
                                                    Weight::Semibold,
                                                ),
                                            ],
                                        ),
                                        self.line(
                                            Self::truncate_chars(&disp_detail, loc_budget.max(24)),
                                            10.,
                                            p.text3,
                                        ),
                                    ],
                                ),
                            )
                            .finish(),
                        )
                        .finish(),
                    self.button_icon(
                        format!("home-work-ctx-open-{idx}"),
                        "跟进",
                        UiIcon::ArrowUpRight,
                        Action::Context(crate::ui::context_state::ContextAction::Open(path)),
                        if idx == 0 {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    ),
                    6.,
                ))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(5.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
                task_rows.push(self.wrap_home_reminder_target(
                    format!("home-rem-work-ledger-{idx}"),
                    ctx_target,
                    ctx_row_el,
                ));
            }
        }

        if checklist_cards.is_empty() && mod_h == HomeModuleHeight::Tall {
            let prep_pills: Vec<El> = [
                ("核对评审会议程与演示材料", "prep-agenda"),
                ("同步最新测试数据与台账证据", "prep-evidence"),
                ("会前 10 分钟开启背屏请勿打扰", "prep-dnd"),
                ("整理会后行动项与纪要分发名单", "prep-summary"),
            ]
            .into_iter()
            .enumerate()
            .map(|(i, (label, key_suffix))| {
                let key = format!("work-tall-{key_suffix}");
                let checked = self.home_checked_checklist_items.contains(&key);
                let state = self.mouse_state(format!("home-work-tall-chk-{i}"));
                let short_lbl = Self::truncate_chars(label, loc_budget.max(15));
                Hoverable::new(state, move |mouse| {
                    let t = self.hover_t(&mouse);
                    let bg = if checked {
                        p.selected
                    } else {
                        lerp_color(p.surface, p.hover, t)
                    };
                    let border = if checked {
                        p.accent
                    } else {
                        lerp_color(p.border, p.border_strong, t)
                    };
                    let fg = if checked { p.accent } else { p.text };
                    Container::new(self.hstack(
                        4.,
                        vec![
                            self.content_icon(
                                if checked {
                                    UiIcon::Check
                                } else {
                                    UiIcon::Square
                                },
                                11.,
                                fg,
                            ),
                            self.line_w(&short_lbl, 10.5, fg, Weight::Medium),
                        ],
                    ))
                    .with_horizontal_padding(5.)
                    .with_vertical_padding(3.5)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::HomeToggleChecklistItem(key.clone()))
                })
                .finish()
            })
            .collect();
            let prep_header = row_between_expanded(
                self.hstack(
                    6.,
                    vec![self.pill_badge("会前准备", p.blue, soft_tint(p.blue, p.dark))],
                ),
                self.chip(
                    "home-work-tall-prep-bs",
                    "推背屏",
                    Some(UiIcon::Matrix),
                    vec![Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus(
                        "会前准备：核对材料与开启免打扰".into(),
                    ))],
                    true,
                    false,
                ),
                6.,
            );
            checklist_cards.push(
                Container::new(
                    self.vstack(5., vec![prep_header, self.grid_cols(prep_pills, 2, 4.)]),
                )
                .with_horizontal_padding(7.)
                .with_vertical_padding(6.)
                .with_background_color(soft_tint(p.blue, p.dark))
                .with_border(Border::all(1.).with_border_color(p.blue))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        }

        if mod_h == HomeModuleHeight::Compact {
            let mut compact_children = vec![header, self.vstack(6., event_rows), work_box];
            if let Some(pomo_card) = pomo_card_opt {
                compact_children.push(pomo_card);
            }
            return self.panel(compact_children);
        }

        if matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full) {
            let mut left_parts = vec![quick_create_row, self.vstack(6., event_rows)];
            if !checklist_cards.is_empty() {
                left_parts.push(self.vstack(6., checklist_cards));
            }
            let left_col = self.vstack(6., left_parts);
            let mut right_parts = Vec::new();
            if !task_rows.is_empty() {
                right_parts.push(self.vstack(6., task_rows));
            }
            right_parts.push(work_box);
            if let Some(pomo_card) = pomo_card_opt {
                right_parts.push(pomo_card);
            } else if mod_h == HomeModuleHeight::Tall {
                let focus_dock = Container::new(row_between_expanded(
                    self.hstack(
                        6.,
                        vec![
                            self.ui_icon_box(
                                UiIcon::Clock,
                                22.,
                                soft_tint(p.indigo, p.dark),
                                p.indigo,
                                None,
                            ),
                            self.line_w("专注", CAPTION, p.text, Weight::Semibold),
                        ],
                    ),
                    self.hstack(
                        4.,
                        vec![
                            self.chip(
                                "home-work-tall-pomo-25",
                                "25m 专注",
                                Some(UiIcon::Clock),
                                vec![Action::BackScreen(BackScreenAction::StartPomodoro(25))],
                                true,
                                false,
                            ),
                            self.chip(
                                "home-work-tall-pomo-45",
                                "45m 深工",
                                None,
                                vec![Action::BackScreen(BackScreenAction::StartPomodoro(45))],
                                false,
                                false,
                            ),
                        ],
                    ),
                    6.,
                ))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(5.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
                right_parts.push(focus_dock);
            }
            let right_col = self.vstack(6., right_parts);
            let split_body = self.responsive_columns(
                740.,
                S2,
                S2,
                CrossAxisAlignment::Start,
                vec![(1.08, left_col), (1.0, right_col)],
            );
            return self.panel(vec![header, split_body]);
        }

        let mut panel_children = vec![header, quick_create_row, self.vstack(6., event_rows)];
        if !checklist_cards.is_empty() {
            panel_children.push(self.vstack(6., checklist_cards));
        }
        if !task_rows.is_empty() {
            panel_children.push(self.vstack(6., task_rows));
        }
        panel_children.push(work_box);
        if let Some(pomo_card) = pomo_card_opt {
            panel_children.push(pomo_card);
        }

        self.panel(panel_children)
    }

    // =========================================================================
    // SECTION 4: 全天作息、用机分布、闹钟与背屏工坊 (24h Dial + KPI Rings | Alarms/Control | BackScreen Studio + App Usage Donut)
    // =========================================================================
    pub(in crate::ui) fn render_home_pulse_telemetry_card(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        let raw_h = self.home_modules.module_height(HomeModuleId::PulseAndKpi);
        let mod_h = self.resolve_effective_height(HomeModuleId::PulseAndKpi, raw_h);
        let now = Local::now();
        let header = self.home_section_heading(UiIcon::Activity, "今日数据");
        let stats = self.render_home_kpi_tiles(report);
        #[cfg(test)]
        let stats = SavePosition::new(stats, "home-pulse-stats").finish();
        let body = if mod_h == HomeModuleHeight::Compact {
            stats
        } else {
            let dial = Align::new(self.fixed_w(162., self.render_home_day_dial())).finish();
            #[cfg(test)]
            let dial = SavePosition::new(dial, "home-pulse-dial").finish();
            self.responsive_columns(560., S3, S2, CrossAxisAlignment::Center,
                vec![(0.8, dial), (1.8, stats)])
        };
        let mut children = vec![header, body];
        if mod_h == HomeModuleHeight::Tall {
            let today = now.date_naive();
            children.push(self.render_home_day_timeline_inner(report, today, today));
        }
        let panel = self.panel(children);
        #[cfg(test)]
        let panel = SavePosition::new(panel, "home-pulse-panel").finish();
        panel
    }

    pub(in crate::ui) fn render_home_footer_device_dock(
        &self,
        report: &stepx_companion::home_hub::HomeHubReport,
        metrics: &stepx_companion::device::DeviceMetrics,
    ) -> El {
        let left_col = self.render_home_pulse_telemetry_card(report);
        let mid_col = self.render_home_alarms_and_control_column(report);
        let right_col = self.vstack(
            S2,
            vec![
                self.render_home_device_and_backscreen_studio_card(report, metrics),
                self.render_home_habit_donut_card(),
            ],
        );
        self.responsive_columns(
            980.,
            S3,
            S3,
            CrossAxisAlignment::Start,
            vec![(1.05, left_col), (1.05, mid_col), (0.95, right_col)],
        )
    }

    // =========================================================================
    // ROW 4 RIGHT: 2-COL VISUAL ALARMS (WITH TOGGLE SWITCH), POMODORO & CONTROL MATRIX
    // =========================================================================
    pub(in crate::ui) fn render_home_alarms_and_control_column(
        &self,
        _report: &stepx_companion::home_hub::HomeHubReport,
    ) -> El {
        let p = self.p();
        let mod_w = self.home_module_content_width(HomeModuleId::AlarmsAndControl);
        let raw_h = self
            .home_modules
            .module_height(HomeModuleId::AlarmsAndControl);
        let mod_h = self.resolve_effective_height(HomeModuleId::AlarmsAndControl, raw_h);
        let profile =
            self.compute_home_module_value_profile(HomeModuleId::AlarmsAndControl, mod_w, raw_h);
        let suite = &self.back_screen.stepos_suite;
        let suite_known =
            self.demo_mode || (suite.checked && self.back_screen.stepos_suite_synced_at.is_some());
        let ctrls = &suite.controls;

        let now_ms = Local::now().timestamp_millis();
        let mut header_left = vec![self.home_section_heading(UiIcon::Clock, "闹钟")];
        if !suite_known {
            header_left.push(self.pill_badge("待同步", p.text3, p.surface));
        }
        let header = row_between_expanded(
            self.hstack(S2, header_left),
            self.link("home-alarms-open-page", "全部闹钟", Action::Page(Page::Alarms)),
            S2,
        );

        // Presets and custom time share a row only when the actual card fits.
        let preset_bar = home_month_geometry_probe(
            "home-alarm-presets",
            self.wrap(vec![
                self.chip(
                    "home-alarm-15m",
                    "+15分",
                    None,
                    vec![Action::BackScreen(BackScreenAction::AlarmCreatePreset(0))],
                    false,
                    false,
                ),
                self.chip(
                    "home-alarm-30m",
                    "+30分",
                    None,
                    vec![Action::BackScreen(BackScreenAction::AlarmCreatePreset(1))],
                    false,
                    false,
                ),
            ]),
        );
        let custom_bar = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Center)
            .with_spacing(S2)
            .with_child(
                Expanded::new(
                    1.,
                    home_month_geometry_probe(
                        "home-alarm-time-input",
                        self.input(F_ALARM_TIME, "08:00", None),
                    ),
                )
                .finish(),
            )
            .with_child(home_month_geometry_probe(
                "home-alarm-create-button",
                self.button(
                    "home-alarm-custom-create",
                    "设闹钟",
                    Action::BackScreen(BackScreenAction::AlarmCreateCustom),
                    Btn::Secondary,
                ),
            ))
            .finish();
        let quick_alarm_bar = self.responsive_columns(
            420.,
            S2,
            S2,
            CrossAxisAlignment::Center,
            vec![(1., preset_bar), (1.6, custom_bar)],
        );

        // Alarm columns also follow available space, including resized windows.
        let mut alarm_grid_rows: Vec<El> = Vec::new();
        for (idx, cd) in self
            .active_home_countdown_timers(&self.push.snapshot)
            .iter()
            .enumerate()
        {
            alarm_grid_rows.push(
                Container::new(self.vstack(
                    4.,
                    vec![
                        row_between_expanded(
                            self.wrap(vec![
                                self.pill_badge_icon(
                                    "倒计时",
                                    UiIcon::Clock,
                                    p.warning,
                                    p.warning_soft,
                                ),
                                self.line_w(&cd.title, CAPTION, p.text, Weight::Semibold),
                                self.pill_badge(&cd.remaining_label, p.warning, p.warning_soft),
                            ]),
                            self.chip(
                                format!("home-alarm-col-cd-pin-{idx}"),
                                "钉背屏",
                                Some(UiIcon::Matrix),
                                vec![Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus(
                                    format!("{} {}", cd.title, cd.remaining_label),
                                ))],
                                true,
                                false,
                            ),
                            6.,
                        ),
                        self.progress(Some(cd.progress_pct), p.warning),
                    ],
                ))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(6.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(with_alpha_f(p.warning, 0.45)))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        }
        let mut prioritized_alarms: Vec<_> =
            suite.alarms.iter().filter(|a| !a.is_e2e_probe()).collect();
        prioritized_alarms.sort_by_key(|a| {
            (
                std::cmp::Reverse(a.is_today_valid_unexpired_at(now_ms, &self.reminder_governance)),
                std::cmp::Reverse(a.enabled),
                a.total_minutes(),
                std::cmp::Reverse(a.id),
            )
        });
        let max_alarms = profile.max_items.min(3);
        let alarms_slice: Vec<_> = prioritized_alarms.into_iter().take(max_alarms).collect();
        for pair in alarms_slice.chunks(2) {
            let first = self.render_visual_alarm_tile(pair[0]);
            if pair.len() == 1 {
                // A final unpaired alarm uses the full row rather than a blank cell.
                alarm_grid_rows.push(first);
            } else {
                alarm_grid_rows.push(self.responsive_columns(
                    600.,
                    S2,
                    S2,
                    CrossAxisAlignment::Stretch,
                    vec![(1., first), (1., self.render_visual_alarm_tile(pair[1]))],
                ));
            }
        }

        if alarm_grid_rows.is_empty() {
            alarm_grid_rows.push(
                Container::new(self.line("暂无闹钟", CAPTION, p.text3))
                    .with_uniform_padding(S3)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            );
        }

        // 3. Only DND is kept here; other phone controls live on their own pages.
        let dnd_cell = self.render_control_switch_cell(
            "home-ctrl-dnd",
            UiIcon::Bell,
            "勿扰",
            if !suite_known {
                "待同步"
            } else if ctrls.dnd_on {
                "已开启"
            } else {
                "关闭"
            },
            suite_known && ctrls.dnd_on,
            Action::BackScreen(BackScreenAction::McpToggleDnd),
        );

        if mod_h == HomeModuleHeight::Compact {
            return home_month_geometry_probe(
                "home-alarms-card",
                self.panel(vec![header, self.vstack(S2, alarm_grid_rows), dnd_cell]),
            );
        }

        if matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full) {
            let left_col = self.vstack(S2, vec![quick_alarm_bar, self.vstack(S2, alarm_grid_rows)]);
            let right_col = self.vstack(S2, vec![dnd_cell]);
            let split_body = self.responsive_columns(
                740.,
                S2,
                S2,
                CrossAxisAlignment::Start,
                vec![(1.4, left_col), (1.0, right_col)],
            );
            return home_month_geometry_probe(
                "home-alarms-card",
                self.panel(vec![header, split_body]),
            );
        }

        home_month_geometry_probe(
            "home-alarms-card",
            self.panel(vec![
                header,
                quick_alarm_bar,
                self.vstack(S2, alarm_grid_rows),
                dnd_cell,
            ]),
        )
    }

    pub(in crate::ui) fn render_visual_alarm_tile(
        &self,
        alarm: &stepx_companion::toolbox::PhoneAlarmItem,
    ) -> El {
        let p = self.p();
        let enabled = alarm.enabled;
        let now_ms = Local::now().timestamp_millis();
        let (status_label, is_today_pending) =
            alarm.validity_status_label_at(now_ms, &self.reminder_governance);
        let status_badge = self.pill_badge(
            status_label,
            if is_today_pending {
                p.success_text
            } else if enabled {
                p.warning
            } else {
                p.text3
            },
            if is_today_pending {
                p.success_soft
            } else if enabled {
                p.warning_soft
            } else {
                p.surface
            },
        );
        let toggle = self.toggle_switch(
            format!("home-alarm-toggle-{}", alarm.id),
            enabled,
            Action::BackScreen(BackScreenAction::AlarmToggle(alarm.id, !enabled)),
        );
        let del_btn = self.chip(
            format!("home-alarm-del-{}", alarm.id),
            "×",
            None,
            vec![Action::BackScreen(BackScreenAction::AlarmDelete(alarm.id))],
            false,
            false,
        );
        let card = Container::new(row_between_expanded(
            self.vstack(
                2.,
                vec![
                    self.wrap(vec![
                        self.mono(
                            alarm.time_label(),
                            19.,
                            if is_today_pending {
                                p.text
                            } else if enabled {
                                p.text2
                            } else {
                                p.text3
                            },
                        ),
                        status_badge,
                    ]),
                    self.line_w(
                        format!("{} · {}", alarm.display_title(), alarm.repeat_label()),
                        MICRO,
                        if enabled { p.text2 } else { p.text3 },
                        Weight::Medium,
                    ),
                ],
            ),
            self.hstack(4., vec![toggle, del_btn]),
            S2,
        ))
        .with_horizontal_padding(S3)
        .with_vertical_padding(S2)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(if is_today_pending {
            lerp_color(p.border, p.success_text, 0.45)
        } else if enabled {
            lerp_color(p.border, p.warning, 0.35)
        } else {
            p.border
        }))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();
        let alarm_id = alarm.id;
        let hover_state = self.mouse_state(&format!("home-alarm-tile-ctx-{alarm_id}"));
        home_month_geometry_probe(
            format!("home-alarm-tile-{alarm_id}"),
            Hoverable::new(hover_state, move |_| card)
                .with_defer_events_to_children()
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::AlarmItem(alarm_id),
                        at,
                    ));
                })
                .finish(),
        )
    }

    pub(in crate::ui) fn render_control_switch_cell(
        &self,
        id: &str,
        icon: UiIcon,
        title: &str,
        sub: &str,
        on: bool,
        action: Action,
    ) -> El {
        let p = self.p();
        let tint = if on { p.success_text } else { p.text2 };
        let icon_box = self.ui_icon_box(
            icon,
            28.,
            if on { p.success_soft } else { p.bg },
            tint,
            None,
        );
        let sw = self.toggle_switch(id, on, action);
        Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(S2)
                .with_child(icon_box)
                .with_child(
                    Expanded::new(
                        1.,
                        self.vstack(
                            1.,
                            vec![
                                self.line_w(title, CAPTION, p.text, Weight::Semibold),
                                self.line(sub, 10., p.text3),
                            ],
                        ),
                    )
                    .finish(),
                )
                .with_child(sw)
                .finish(),
        )
        .with_horizontal_padding(S2 + 2.)
        .with_vertical_padding(S1 + 2.)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish()
    }
}

#[cfg(test)]
#[path = "weekly_alarm_tests.rs"]
mod weekly_alarm_tests;
