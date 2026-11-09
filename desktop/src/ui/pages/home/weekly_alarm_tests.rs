use super::*;
use chrono::TimeZone;
use stepx_companion::toolbox::{PhoneAlarmItem, PhoneCalendarEvent};

fn fixture() -> Companion {
    let mut app = Companion::new_for_test();
    app.push.snapshot = PushSnapshot::default();
    app.sms.clear();
    app.call_logs.clear();
    app.express_parcels.clear();
    app.workspace.journal.tasks.clear();
    app.back_screen.stepos_suite.remote_agent_files.clear();
    app.back_screen.stepos_suite.calendar_events.clear();
    app.back_screen.stepos_suite.alarms.clear();
    app.home_modules
        .set_module_height(HomeModuleId::WeeklyAndAi, HomeModuleHeight::Standard);
    app.home_modules
        .set_module_height(HomeModuleId::AlarmsAndControl, HomeModuleHeight::Standard);
    // A saved full-width choice must not override actual narrow constraints.
    app.home_modules
        .set_module_width(HomeModuleId::AlarmsAndControl, HomeModuleWidth::Full);
    app
}

fn timestamp(date: NaiveDate) -> i64 {
    Local
        .from_local_datetime(&date.and_hms_opt(9, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp_millis()
}

fn calendar_event(id: i64, date: NaiveDate, title: &str) -> PhoneCalendarEvent {
    PhoneCalendarEvent {
        id,
        title: title.into(),
        dtstart: timestamp(date),
        dtend: timestamp(date) + 3_600_000,
        ..Default::default()
    }
}

fn select_date(app: &mut Companion, date: NaiveDate) {
    app.back_screen.calendar_year = date.year();
    app.back_screen.calendar_month = date.month();
    app.back_screen.selected_calendar_day = date.day();
}

#[test]
fn weekly_details_stay_in_selected_week_and_refresh_source_edits() {
    let mut app = fixture();
    let old = Local::now().date_naive() - ChronoDuration::days(14);
    let monday = old - ChronoDuration::days(old.weekday().num_days_from_monday() as i64);
    select_date(&mut app, monday);
    app.back_screen.stepos_suite.calendar_events = vec![
        calendar_event(701, monday + ChronoDuration::days(1), "设计评审"),
        calendar_event(702, monday + ChronoDuration::days(4), "交付确认"),
        calendar_event(703, monday - ChronoDuration::days(1), "上一周的会议"),
        calendar_event(704, monday + ChronoDuration::days(8), "下一周的会议"),
    ];
    let (_, weekly) = app.build_home_panorama_and_weekly(&app.push.snapshot, &app.express_parcels);
    assert!(!weekly.days.iter().any(|day| day.is_today));
    let entries = app.home_weekly_detail_entries(&weekly, 0);
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].1.title, "交付确认");
    assert_eq!(entries[1].1.title, "设计评审");
    assert!(entries.iter().all(
        |(date, entry)| weekly.days.iter().any(|d| &d.date_ymd == date)
            && entry.kind == SelectedDateEntryKind::Schedule
    ));
    assert!(std::rc::Rc::ptr_eq(
        &entries,
        &app.home_weekly_detail_entries(&weekly, 0)
    ));
    app.back_screen.stepos_suite.calendar_events[1].title = "交付时间已调整".into();
    let changed = app.home_weekly_detail_entries(&weekly, 0);
    assert!(!std::rc::Rc::ptr_eq(&entries, &changed));
    assert_eq!(changed[0].1.title, "交付时间已调整");
    // Today's remote files and screen-time counters cannot become historic details.
    assert!(app.home_weekly_detail_entries(&weekly, 1).is_empty());
    assert!(app.home_weekly_detail_entries(&weekly, 2).is_empty());
}

#[test]
fn weekly_life_and_communication_details_use_dated_sources() {
    let mut app = fixture();
    let selected = Local::now().date_naive() - ChronoDuration::days(14);
    select_date(&mut app, selected);
    app.sms = vec![
        stepx_companion::sms::Message {
            address: "10690001".into(),
            body: "快递已到丰巢，取件码 123456".into(),
            date_ms: timestamp(selected),
            outgoing: false,
            kind: 1,
            read: true,
            provider_id: None,
            subscription: None,
        },
        stepx_companion::sms::Message {
            address: "13800000000".into(),
            body: "请确认合同交付日期".into(),
            date_ms: timestamp(selected),
            outgoing: false,
            kind: 1,
            read: true,
            provider_id: None,
            subscription: None,
        },
        stepx_companion::sms::Message {
            address: "13800000000".into(),
            body: "本周新消息不属于所选周".into(),
            date_ms: timestamp(selected + ChronoDuration::days(14)),
            outgoing: false,
            kind: 1,
            read: false,
            provider_id: None,
            subscription: None,
        },
    ];
    let (_, weekly) = app.build_home_panorama_and_weekly(&app.push.snapshot, &app.express_parcels);
    let life = app.home_weekly_detail_entries(&weekly, 2);
    assert!(life
        .iter()
        .any(|(_, entry)| entry.detail.contains("123456")));
    let communication = app.home_weekly_detail_entries(&weekly, 3);
    assert!(communication
        .iter()
        .any(|(_, entry)| entry.detail.contains("合同交付日期")));
    assert!(!communication
        .iter()
        .any(|(_, entry)| entry.detail.contains("本周新消息")));
}

struct ModuleOnly {
    app: Companion,
    weekly: bool,
}
impl facloud_ui::Entity for ModuleOnly {
    type Event = ();
}
impl facloud_ui::TypedActionView for ModuleOnly {
    type Action = ();
}
impl facloud_ui::View for ModuleOnly {
    fn ui_name() -> &'static str {
        "weekly_alarm_regression"
    }
    fn render(&self, _: &facloud_ui::AppContext) -> El {
        let child = if self.weekly {
            let (_, weekly) = self
                .app
                .build_home_panorama_and_weekly(&self.app.push.snapshot, &self.app.express_parcels);
            self.app.render_home_weekly_summary_card(&weekly)
        } else {
            let report = self.app.with_home_hub_input(
                &self.app.push.snapshot,
                &self.app.express_parcels,
                |input, _, _| build_home_hub_report(input),
            );
            self.app.render_home_alarms_and_control_column(&report)
        };
        Stack::new()
            .with_child(Align::new(child).top_left().finish())
            .finish()
    }
}

#[test]
fn alarms_fit_actual_width_and_weekly_standard_exposes_details() {
    facloud_ui::App::test((), |mut host| async move {
        for width in [300., 420., 690., 900.] {
            let mut app = fixture();
            app.back_screen.stepos_suite.alarms = (0..3)
                .map(|i| PhoneAlarmItem {
                    id: 801 + i,
                    hour: 9 + i as u8,
                    minutes: 30,
                    enabled: true,
                    days_of_week: 31,
                    message: "工作日项目晨会及待办确认".into(),
                    ..Default::default()
                })
                .collect();
            let (window, _) = host
                .add_window(facloud_ui::platform::WindowStyle::NotStealFocus, |_| {
                    ModuleOnly { app, weekly: false }
                });
            let root = host.root_view_id(window).unwrap();
            host.update(move |ctx| {
                let mut presenter = facloud_ui::Presenter::new(window);
                presenter.invalidate(
                    facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]),
                        ..Default::default()
                    },
                    ctx,
                );
                presenter.build_scene(vec2f(width, 1500.), 1., None, ctx);
                let positions = presenter.position_cache();
                let rect = |id: &str| {
                    positions
                        .get_position(id)
                        .unwrap_or_else(|| panic!("missing {id}"))
                };
                let card = rect("home-alarms-card");
                for id in [
                    "home-alarm-presets",
                    "home-alarm-time-input",
                    "home-alarm-create-button",
                    "home-alarm-tile-801",
                    "home-alarm-tile-802",
                    "home-alarm-tile-803",
                ] {
                    let child = rect(id);
                    assert!(
                        child.origin().x() >= card.origin().x()
                            && child.max_x() <= card.max_x() + 1.,
                        "{id} overflows at {width}: {child:?} / {card:?}"
                    );
                }
                let input = rect("home-alarm-time-input");
                let create = rect("home-alarm-create-button");
                assert!(
                    input.width() >= 60.,
                    "time input squeezed at {width}: {input:?}"
                );
                assert!(
                    input.max_x() <= create.origin().x(),
                    "create button overlaps input"
                );
                if width < 600. {
                    // Upcoming alarms precede elapsed ones, so their order
                    // changes when the host clock crosses a fixture's time.
                    // Verify every pair is vertically disjoint in either order.
                    let tiles = [801, 802, 803].map(|id| rect(&format!("home-alarm-tile-{id}")));
                    for (index, first) in tiles.iter().enumerate() {
                        for second in &tiles[index + 1..] {
                            assert!(
                                first.max_y() <= second.origin().y()
                                    || second.max_y() <= first.origin().y(),
                                "narrow alarms must stack at {width}: {first:?} / {second:?}"
                            );
                        }
                    }
                }
            });
        }
        for quadrant in 0..4 {
            let mut app = fixture();
            app.home_ai_quadrant = quadrant;
            let (window, _) = host
                .add_window(facloud_ui::platform::WindowStyle::NotStealFocus, |_| {
                    ModuleOnly { app, weekly: true }
                });
            let root = host.root_view_id(window).unwrap();
            host.update(move |ctx| {
                let mut presenter = facloud_ui::Presenter::new(window);
                presenter.invalidate(
                    facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]),
                        ..Default::default()
                    },
                    ctx,
                );
                presenter.build_scene(vec2f(320., 1500.), 1., None, ctx);
                let drilldown = presenter
                    .position_cache()
                    .get_position("home-weekly-drilldown")
                    .expect("standard-height selected quadrant must have visible details");
                assert!(drilldown.height() > 20. && drilldown.max_x() <= 320.);
            });
        }
    });
}
