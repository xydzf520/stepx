
use super::*;

fn assert_plot_stays_inside(rects: &[HomePlotRect], width: f32, height: f32) {
    assert!(!rects.is_empty(), "the chart should remain visible");
    for rect in rects {
        assert!(rect.x.is_finite() && rect.y.is_finite());
        assert!(rect.width > 0. && rect.height > 0.);
        assert!(
            rect.x >= 0. && rect.y >= 0.,
            "negative chart edge: {rect:?}"
        );
        assert!(
            rect.x + rect.width <= width + 0.001,
            "right overflow: {rect:?}"
        );
        assert!(
            rect.y + rect.height <= height + 0.001,
            "bottom overflow: {rect:?}"
        );
        assert!(rect.radius <= rect.width.min(rect.height) / 2. + 0.001);
    }
}

#[test]
fn home_timeline_actual_width_keeps_pins_and_overlapping_events_in_bounds() {
    let alarm_color = rgb(230, 120, 0);
    let plot = HomePlotContent::Timeline(DayTimeline {
        // Deliberately unlike the layout width: no raster/design width may
        // determine the native plot's position or stroke thickness.
        width: 960.,
        height: 40.,
        start_hour: 6,
        end_hour: 24,
        now_min: 1439,
        events: vec![
            DialEvent {
                start_min: 0,
                end_min: 420,
                paint: Paint::rgb(20, 90, 200),
            },
            DialEvent {
                start_min: 360,
                end_min: 720,
                paint: Paint::rgb(30, 100, 210),
            },
            DialEvent {
                start_min: 370,
                end_min: 700,
                paint: Paint::rgb(40, 110, 220),
            },
            DialEvent {
                start_min: 1400,
                end_min: 1600,
                paint: Paint::rgb(50, 120, 230),
            },
            DialEvent {
                start_min: 1439,
                end_min: 0,
                paint: Paint::rgb(60, 130, 240),
            },
        ],
        alarms: vec![
            DialAlarm {
                minute_of_day: 360,
                enabled: true,
            },
            DialAlarm {
                minute_of_day: 1439,
                enabled: true,
            },
            DialAlarm {
                minute_of_day: 300,
                enabled: true,
            },
        ],
        track: Paint::rgb(220, 220, 220),
        elapsed: Paint::rgb(170, 170, 170),
        tick: Paint::rgb(100, 100, 100),
        now: Paint::rgb(240, 30, 30),
        alarm: Paint::rgb(230, 120, 0),
        matte: Paint::rgb(255, 255, 255),
    });
    for width in [24., 120., 320., 640., 1400.] {
        for height in [24., 40.] {
            let rects = plot.rectangles(vec2f(width, height));
            assert_plot_stays_inside(&rects, width, height);
            let pins: Vec<_> = rects.iter().filter(|r| r.fill == alarm_color).collect();
            assert_eq!(
                pins.len(),
                2,
                "only alarms in the displayed time range belong here"
            );
            assert!(pins.iter().all(|r| (r.width - 3.).abs() < 0.001));
        }
    }
}

#[test]
fn home_week_chart_uses_available_width_without_stretching_bars() {
    let track = rgb(230, 230, 230);
    let plot = HomePlotContent::Week {
        days: (0..7)
            .map(|i| WeeklyBarDay {
                work_count: if i == 3 { u32::MAX } else { i * 2 },
                life_count: i + 1,
                issue_count: 2,
                selected: i == 6,
                today: i == 0,
            })
            .collect(),
        fills: [rgb(40, 110, 240), rgb(245, 160, 0), rgb(235, 50, 160)],
        track,
        selected: rgb(20, 150, 100),
    };
    for width in [84., 280., 700., 1400.] {
        let rects = plot.rectangles(vec2f(width, 54.));
        assert_plot_stays_inside(&rects, width, 54.);
        let bars: Vec<_> = rects.iter().filter(|r| r.fill == track).collect();
        assert_eq!(bars.len(), 7);
        for (index, bar) in bars.iter().enumerate() {
            assert!(
                bar.width <= 28.001,
                "wide windows should add spacing, not stretch bars"
            );
            assert!((bar.radius - 3.).abs() < 0.001);
            let expected_center = (index as f32 + 0.5) * width / 7.;
            assert!((bar.x + bar.width / 2. - expected_center).abs() < 0.001);
        }
    }
    assert!(plot.rectangles(vec2f(f32::NAN, 54.)).is_empty());
    assert!(plot.rectangles(vec2f(200., 0.)).is_empty());
}

#[test]
fn home_native_chart_modules_do_not_wait_for_svg_rasterization() {
    assert!(!HomeModuleId::CalendarPanorama.uses_svg());
    assert!(!HomeModuleId::WeeklyAndAi.uses_svg());
    assert!(!HomeModuleId::LifeModule.uses_svg());
    assert!(HomeModuleId::PulseAndKpi.uses_svg());
    assert!(HomeModuleId::BackScreenAndHabits.uses_svg());
    assert!(HomeModuleId::ContextLedgerHub.uses_svg());
}

#[test]
fn home_period_shortcuts_open_evidence_reviews_without_sending_a_prompt() {
    let mut app = Companion::new_for_test();
    app.fields[F_WORK_PROMPT].set("保留我的文档草稿");
    app.apply_home_action(&Action::HomeRunWorkCommand, None);
    assert_eq!(app.page, Page::Work);
    assert_eq!(app.fields[F_WORK_PROMPT].text, "保留我的文档草稿");
    assert!(!app.work.running && !app.home_ai.running());
    assert!(app.workspace.running.is_none());
    for route in ["review-day", "review-week", "review-month"] {
        app.page = Page::Work;
        assert!(app.apply_home_action(
            &Action::HomeExecuteIntent(HomeAiIntent::OpenPage(route.into())),
            None
        ));
        assert_eq!(app.page, Page::Home);
        assert_eq!(app.fields[F_WORK_PROMPT].text, "保留我的文档草稿");
        assert!(!app.work.running && !app.home_ai.running());
        assert!(app.workspace.running.is_none());
        assert!(app.status.contains("周期回顾"));
    }
}

#[test]
fn home_does_not_count_arbitrary_workspace_files_as_finished_outputs() {
    use crate::ui::work_state::{WorkArtifactItem, WorkFileItem};
    let mut app = Companion::new_for_test();
    app.work.files.push(WorkFileItem {
        path: "/tmp/reference.txt".into(),
        name: "参考资料".into(),
        bytes: 1,
        is_dir: false,
    });
    assert_eq!(app.home_local_output_count(), 0);
    app.work.artifacts.push(WorkArtifactItem {
        path: "/tmp/result.md".into(),
        name: "方案".into(),
        kind: "document".into(),
    });
    assert_eq!(app.home_local_output_count(), 1);
}

#[test]
fn home_and_terminal_pages_and_routing_match_product_spec() {
    assert_eq!(Page::Home.title(), "首页");
    assert!(!Page::Home.needs_device());
    assert_eq!(Page::Overview.title(), "终端");
    assert_eq!(Page::from_arg("home"), Page::Home);
    assert_eq!(Page::from_arg("overview"), Page::Overview);
    assert_eq!(Page::from_arg("terminal"), Page::Overview);

    let mut app = Companion::new_for_test();
    assert_eq!(app.page, Page::Home);
    let _ = app.home_page();
    let _ = app.home_actions();
    let _ = app.sidebar();

    // Populate rich connected state and verify rendering across all Home sections
    app.init_demo_push();
    app.back_screen.stepos_suite.checked = true;
    app.back_screen.stepos_suite.wellbeing.screen_time_minutes = 185;
    app.back_screen.stepos_suite.wellbeing.unlock_count = 42;
    app.back_screen.stepos_suite.wellbeing.top_apps = vec![
        ("com.tencent.mm".into(), "微信".into(), 68),
        ("com.ss.android.lark".into(), "飞书".into(), 54),
    ];
    app.back_screen.stepos_suite.phone_clipboard =
        "顺丰速运 SF1682904182291 您的快件已存放在丰巢智能柜，取件码 3-2-1048".into();
    let _ = app.page_content();
    let _ = app.page_actions();

    app.page = Page::Overview;
    assert_eq!(app.page.title(), "终端");
    let _ = app.page_content();
}

#[test]
fn home_ui_actions_update_companion_state_accurately() {
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();

    // 1. Action::HomeSelectReplyTarget updates selected_key, replying_key, selected_im_thread, and focuses F_PUSH_REPLY
    assert!(app.apply_home_action(
        &Action::HomeSelectReplyTarget {
            key: "notif-feishu-1".into(),
            thread_id: "im:com.ss.android.lark:product".into(),
            package_name: "com.ss.android.lark".into(),
            title: "产品研发对齐群".into(),
        },
        None,
    ));
    assert_eq!(app.push.selected_key.as_deref(), Some("notif-feishu-1"));
    assert_eq!(app.push.replying_key.as_deref(), Some("notif-feishu-1"));
    assert_eq!(
        app.push.selected_im_thread.as_deref(),
        Some("im:com.ss.android.lark:product")
    );
    assert_eq!(app.focus, Some(F_PUSH_REPLY));
    assert!(app.status.contains("产品研发对齐群"));
    assert!(!app.error);

    // 2. Action::HomeQuickSmsReply sets SMS fields and appends outgoing reply in offline/demo state
    let prev_sms_len = app.sms.len();
    assert!(app.apply_home_action(
        &Action::HomeQuickSmsReply {
            number: "13800009367".into(),
            body: "放门口即可，谢谢".into(),
        },
        None,
    ));
    assert_eq!(app.fields[F_SMS_TO].text, "13800009367");
    assert_eq!(app.fields[F_SMS_BODY].text, "放门口即可，谢谢");
    assert_eq!(app.sms.len(), prev_sms_len + 1);
    let last_sms = app.sms.last().expect("outgoing SMS recorded");
    assert_eq!(last_sms.address, "13800009367");
    assert_eq!(last_sms.body, "放门口即可，谢谢");
    assert!(last_sms.outgoing);
    assert!(app.status.contains("演示：已模拟回复短信至 13800009367"));

    // 3. Action::HomeSubmitCommand routes natural-language commands via classify_command_intent
    // 3a. BackScreen status command also synthesizes LED glyph frame immediately
    app.fields[F_HOME_COMMAND].set("帮我把背屏设置成去吃饭");
    assert!(app.apply_home_action(&Action::HomeSubmitCommand, None));
    assert!(app.fields[F_HOME_COMMAND].text.is_empty());
    assert_eq!(app.fields[F_BACK_MARQUEE].text, "去吃饭");
    assert!(app.matrix_marquee_on);
    assert!(app.back_screen.title.contains("去吃饭"));
    assert!(
        app.back_screen.frame.iter().any(|&b| b > 0),
        "BackScreenStatus should synthesize non-empty glyph frame into back_screen.frame"
    );

    // 3b. Pomodoro command
    app.fields[F_HOME_COMMAND].set("开启25分钟番茄钟");
    assert!(app.apply_home_action(&Action::HomeSubmitCommand, None));
    assert!(app.back_screen.pomodoro_active);
    assert_eq!(app.back_screen.pomodoro_duration_secs, 25 * 60);

    // 3c. Express tracking query command
    app.fields[F_HOME_COMMAND].set("查一下快递 SF1682904182291");
    assert!(app.apply_home_action(&Action::HomeSubmitCommand, None));
    assert_eq!(app.fields[F_OV_TEXT].text, "SF1682904182291");
    assert!(app.amoo_express_open);

    // 4. Action::HomeRunGuiCommand switches to Page::Amoo in GUI screen mode and populates F_AMOO
    app.page = Page::Home;
    app.fields[F_HOME_COMMAND].set("打开美团看看外卖骑手到哪了");
    assert!(app.apply_home_action(&Action::HomeRunGuiCommand, None));
    assert!(app.fields[F_HOME_COMMAND].text.is_empty());
    assert_eq!(app.page, Page::Amoo);
    assert!(app.amoo_ui.screen_mode);
    assert_eq!(app.fields[F_AMOO_GUI_APP].text, "手机应用");
    assert_eq!(app.fields[F_AMOO].text, "打开美团看看外卖骑手到哪了");

    // 5. Action::HomeRunWorkCommand switches to Page::Work and populates F_WORK_PROMPT without error
    app.page = Page::Home;
    app.fields[F_HOME_COMMAND].set("帮我写一份产品发布方案文档");
    assert!(app.apply_home_action(&Action::HomeRunWorkCommand, None));
    assert!(app.fields[F_HOME_COMMAND].text.is_empty());
    assert_eq!(app.page, Page::Work);
    assert_eq!(app.fields[F_WORK_PROMPT].text, "帮我写一份产品发布方案文档");
    assert!(!app.error);
    assert!(app.work.error.is_none());

    // 6. Action::HomeExecuteIntent across remaining intents (AmooPrompt, AlipayCoupon, CapCutHandoff, ToggleDnd, OpenPage)
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::AmooPrompt("汇总未读工作消息".into())),
        None,
    ));
    assert_eq!(app.page, Page::Amoo);
    assert_eq!(app.fields[F_AMOO].text, "汇总未读工作消息");
    assert_eq!(app.focus, Some(F_AMOO));

    assert!(app.apply_home_action(&Action::HomeExecuteIntent(HomeAiIntent::AlipayCoupon), None));
    assert!(app.status.contains("支付宝"));

    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::CapCutHandoff),
        None
    ));
    assert!(app.status.contains("剪映"));

    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::ToggleDnd(true)),
        None,
    ));
    assert!(app.back_screen.stepos_suite.controls.dnd_on);

    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::OpenPage("calendar".into())),
        None,
    ));
    assert_eq!(app.page, Page::Calendar);

    // 7. Action::HomeRefreshAll updates status cleanly
    assert!(app.apply_home_action(&Action::HomeRefreshAll, None));
    assert!(app.status.contains("演示：已刷新首页"));
    assert!(!app.error);
}

#[test]
fn home_calendar_backscreen_and_unread_alignment_regressions() {
    let mut app = Companion::new_for_test();
    app.init_demo_push();
    let snapshot = demo_push_snapshot();
    let express_parcels = default_express_parcels();
    let input = HomeHubInput {
        hour: 15,
        minute: 0,
        today_ymd: "2026-10-29",
        device_connected: true,
        device_model: "StepX Phone",
        firmware_profile: "StepOS",
        threads: &snapshot.threads,
        notifications: &snapshot.notifications,
        otp_banners: &snapshot.otp_banners,
        sms: &app.sms,
        call_logs: &app.call_logs,
        express_parcels: &express_parcels,
        calendar_events: &app.back_screen.stepos_suite.calendar_events,
        alarms: &app.back_screen.stepos_suite.alarms,
        wellbeing: &app.back_screen.stepos_suite.wellbeing,
        phone_clipboard: &app.back_screen.stepos_suite.phone_clipboard,
        remote_agent_files: &[],
        work_deliverables_count: 0,
        pomodoro_running: false,
        pomodoro_remaining_secs: 0,
        dnd_on: false,
    };
    let report = build_home_hub_report(&input);

    // 1. Default "STEPX AI" placeholder in F_BACK_MARQUEE is ignored so Home BackScreen Studio uses suggested scene status
    assert_eq!(app.fields[F_BACK_MARQUEE].text, "STEPX AI");
    assert_eq!(
        app.current_backscreen_signature(&report),
        report.scene.suggested_backscreen_status.as_str()
    );

    // 2. HomeAiIntent::BackScreenStatus updates F_BACK_MARQUEE and synthesizes LED glyph frame
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus("休息一下".into())),
        None,
    ));
    assert_eq!(app.fields[F_BACK_MARQUEE].text, "休息一下");
    assert_eq!(app.current_backscreen_signature(&report), "休息一下");
    assert!(
        app.back_screen.frame.iter().any(|&b| b > 0),
        "BackScreenStatus must populate non-empty LED frame"
    );

    // 3. Selecting a different date on the 7-day strip updates selected_calendar_day and renders cleanly
    app.back_screen.calendar_year = 2026;
    app.back_screen.calendar_month = 10;
    app.back_screen.selected_calendar_day = 30;
    let _ = app.home_page();

    // 4. resolve_home_reply_target skips raw JSON system notifications
    let (_, _, target_title, _) = app.resolve_home_reply_target(&snapshot);
    assert!(!target_title.trim_start().starts_with('{'));
}

fn ms(y: i32, mo: u32, d: u32, h: u32, mi: u32) -> i64 {
    chrono::TimeZone::with_ymd_and_hms(&Local, y, mo, d, h, mi, 0)
        .single()
        .unwrap()
        .timestamp_millis()
}

fn event(
    id: i64,
    start: i64,
    end: i64,
    all_day: bool,
) -> stepx_companion::toolbox::PhoneCalendarEvent {
    stepx_companion::toolbox::PhoneCalendarEvent {
        recurring: Some(false),
        id,
        title: format!("事件{id}"),
        dtstart: start,
        dtend: end,
        all_day,
        location: String::new(),
        description: String::new(),
        source: String::new(),
    }
}

#[test]
fn today_event_spans_normalize_edges() {
    let today = NaiveDate::from_ymd_opt(2026, 10, 29).unwrap();
    let events = vec![
        event(1, ms(2026, 10, 29, 19, 13), ms(2026, 10, 29, 20, 0), false),
        // Crosses midnight into tomorrow → clipped to 24:00.
        event(2, ms(2026, 10, 29, 23, 0), ms(2026, 10, 30, 1, 0), false),
        // Missing / inverted end → one hour.
        event(3, ms(2026, 10, 29, 9, 0), 0, false),
        // All-day and other days are excluded.
        event(4, ms(2026, 10, 29, 0, 0), ms(2026, 10, 30, 0, 0), true),
        event(5, ms(2026, 10, 30, 9, 0), ms(2026, 10, 30, 10, 0), false),
        // Started yesterday at 23:00 and continues to 01:00 today → clamped to (0, 60).
        event(6, ms(2026, 10, 28, 23, 0), ms(2026, 10, 29, 1, 0), false),
        // Ended at exactly 00:00 today → excluded.
        event(7, ms(2026, 10, 28, 22, 0), ms(2026, 10, 29, 0, 0), false),
    ];
    let spans = today_event_spans(&events, today);
    let simple: Vec<(u32, u32, i64)> = spans.iter().map(|(s, e, ev)| (*s, *e, ev.id)).collect();
    assert_eq!(
        simple,
        vec![(0, 60, 6), (540, 600, 3), (1153, 1200, 1), (1380, 1440, 2)],
        "sorted, clipped and defaulted"
    );
    assert!(today_event_spans(&[], today).is_empty());
    assert_eq!(fmt_minutes(5), "5 分钟");
    assert_eq!(fmt_minutes(65), "1h 05m");
}

#[test]
fn home_ai_context_carries_live_context_without_codes() {
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();
    app.back_screen.stepos_suite.wellbeing.screen_time_minutes = 164;
    app.back_screen.stepos_suite.wellbeing.top_apps =
        vec![("com.tencent.mm".into(), "微信".into(), 46)];
    let ctx = app.home_ai_context();
    assert!(ctx.unread_total > 0);
    assert!(!ctx.threads.is_empty());
    assert_eq!(ctx.screen_minutes, 164);
    assert_eq!(ctx.top_apps, vec![("微信".to_string(), 46)]);
    let prompt = stepx_companion::home_ai::build_prompt(&ctx);
    assert!(
        !prompt.contains("839201"),
        "OTP never reaches the phone AI prompt"
    );
    assert!(
        !prompt.contains("4-2-1088"),
        "pickup codes never reach the prompt"
    );
}

#[test]
fn home_ai_actions_and_brief_rendering_states() {
    let mut app = Companion::new_for_test();
    app.init_demo_push();
    // Offline: refresh reports a clear error and stays on local rules.
    app.selected = None;
    app.demo_mode = false;
    assert!(app.apply_home_action(&Action::HomeAiRefresh, None));
    assert!(app.status.contains("连接手机"));
    assert!(app.error);
    assert!(app.home_ai.brief.is_none());
    let _ = app.home_page();

    // Running state renders progress and can be cancelled.
    let (tx, rx) = std::sync::mpsc::channel();
    app.home_ai.job = Some(stepx_companion::home_ai::Job {
        receiver: rx,
        cancel: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
    });
    app.home_ai.status = "手机 Amoo 正在阅读首页上下文…".into();
    let _ = app.home_page();
    tx.send(stepx_companion::home_ai::Event::Status("x".into()))
        .unwrap();
    assert!(app.apply_home_action(&Action::HomeAiCancel, None));
    assert!(!app.home_ai.running());
    assert!(app.status.contains("已取消"));

    // Demo: brief loaded through the real parser and rendered.
    app.demo_mode = true;
    assert!(app.apply_home_action(&Action::HomeAiRefresh, None));
    let brief = app.home_ai.brief.clone().expect("demo brief");
    assert_eq!(brief.suggestions.len(), 3);
    let _ = app.home_page();

    // Every suggestion intent executes through the existing Home intent router.
    for rec in brief.suggestions {
        app.page = Page::Home;
        assert!(app.apply_home_action(&Action::HomeExecuteIntent(rec.intent.clone()), None));
    }
    assert!(
        app.back_screen.pomodoro_active,
        "focus suggestion started the pomodoro"
    );
}

#[test]
fn home_renders_in_dark_theme_and_with_empty_phone_data() {
    let mut app = Companion::new_for_test();
    app.context.overview.evidence_count = 4;
    assert!(app.home_tools_expanded);
    let _ = app.home_page();
    assert!(
        !app.svg_slots.take_pending().is_empty(),
        "the full default preserves chart modules"
    );
    app.home_modules.apply_preset(HomeModulePreset::CompleteAll);
    let _ = app.home_page();
    assert!(
        !app.svg_slots.take_pending().is_empty(),
        "the complete preset renders its fixed SVG charts"
    );
    app.apply_home_action(&Action::HomeToggleTools, None);
    assert!(!app.home_tools_expanded);
    let _ = app.home_page();
    assert!(
        !app.svg_slots.take_pending().is_empty(),
        "collapsing quick tools must preserve the other visible chart modules"
    );
    app.apply_home_action(&Action::HomeToggleTools, None);
    assert!(app.home_tools_expanded);
    app.home_modules.apply_preset(HomeModulePreset::CompleteAll);
    app.palette.set(Palette::for_theme(true));
    app.back_screen.stepos_suite.calendar_events.clear();
    app.back_screen.stepos_suite.alarms.clear();
    app.back_screen.stepos_suite.wellbeing.screen_time_minutes = 0;
    app.back_screen.stepos_suite.wellbeing.unlock_count = 0;
    app.back_screen.stepos_suite.wellbeing.top_apps.clear();
    let _ = app.home_page();
    let light: Vec<_> = app.svg_slots.take_pending();
    assert!(!light.iter().any(|(slot, _, _)| slot == "dial"),
        "narrow telemetry prioritizes statistics instead of stacking a large dial");
    assert!(light.iter().any(|(slot, _, _)| slot == "usage-donut"));
    assert_eq!(
        light
            .iter()
            .filter(|(s, _, _)| s.starts_with("kpi-"))
            .count(),
        0,
        "semantic KPI tiles must not register arbitrary-target progress rings"
    );
    assert!(light
        .iter()
        .all(|(slot, _, _)| slot != "timeline" && slot != "weekly-bars"));
    for (slot, hash, _) in &light {
        app.svg_slots.mark_inserted(slot, *hash);
    }
    // Re-render with identical data → nothing pending (no churn every tick).
    let _ = app.home_page();
    assert!(
        app.svg_slots
            .take_pending()
            .iter()
            .all(|(slot, _, _)| slot == "dial"),
        "only clock-dependent charts may change between renders"
    );
    // Parcel markers and their labels use native layout even with live data.
    app.demo_mode = true;
    let _ = app.home_page();
    let demo_slots: Vec<_> = app.svg_slots.take_pending();
    assert_eq!(
        demo_slots
            .iter()
            .filter(|(s, _, _)| s.starts_with("parcel-step-"))
            .count(),
        0
    );
}

#[test]
fn home_pomodoro_clamping_dnd_idempotency_and_stale_calendar_isolation() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;

    // 1. StartPomodoro(90) clamps to 60 minutes (3600 seconds) so BackScreenAction never treats it as 90 seconds.
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::StartPomodoro(90)),
        None,
    ));
    assert!(app.back_screen.pomodoro_active);
    assert_eq!(app.back_screen.pomodoro_duration_secs, 60 * 60);
    assert!(app.status.contains("60 分钟"));

    // 2. An offline snapshot cannot prove the current phone state.
    app.back_screen.stepos_suite.controls.dnd_on = true;
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::ToggleDnd(true)),
        None,
    ));
    assert!(app.back_screen.stepos_suite.controls.dnd_on);
    assert!(app.status.contains("手机未连接"));
    assert!(app.error);

    // 3. Past calendar events from prior dates are excluded from home_ai_context.events.
    app.back_screen.stepos_suite.calendar_events = vec![event(
        99,
        ms(2025, 4, 2, 10, 0),
        ms(2025, 4, 2, 11, 0),
        false,
    )];
    let ctx = app.home_ai_context();
    assert!(
        ctx.events.is_empty(),
        "stale past-date calendar events must not be sent as today's schedule"
    );

    // 4. Real-device empty snapshot does not fall back to demo '微信工作群'.
    let empty_snap = PushSnapshot::default();
    let (_, _, _, title) = app.resolve_home_reply_target(&empty_snap);
    assert_ne!(title, "微信工作群");
}

#[test]
fn offline_home_actions_preserve_drafts_and_do_not_invent_success() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    app.selected = None;
    let messages = app.sms.len();
    app.apply_home_action(
        &Action::HomeQuickSmsReply {
            number: "10086".into(),
            body: "稍后回复".into(),
        },
        None,
    );
    assert_eq!(
        app.sms.len(),
        messages,
        "offline must not append a sent message"
    );
    assert_eq!(app.fields[F_SMS_BODY].text, "稍后回复");
    assert_eq!(app.page, Page::Sms);
    assert!(app.error && app.status.contains("未发送"));

    app.fields[F_HOME_COMMAND].set("开启勿扰");
    let before = app.back_screen.stepos_suite.controls.dnd_on;
    app.apply_home_action(&Action::HomeSubmitCommand, None);
    assert_eq!(app.fields[F_HOME_COMMAND].text, "开启勿扰");
    assert_eq!(app.back_screen.stepos_suite.controls.dnd_on, before);
    assert!(app.error);

    app.apply_home_action(&Action::HomeRefreshAll, None);
    assert!(app.error && app.status.contains("手机未连接"));
    assert!(!app.back_screen.stepos_suite_loading);

    // A duplicate refresh must not clear another reader's in-flight guard.
    app.demo_mode = true;
    app.back_screen.stepos_suite_loading = true;
    app.apply_home_action(&Action::HomeRefreshAll, None);
    assert!(app.back_screen.stepos_suite_loading);
    assert!(app.status.contains("不会重复"));
}

#[test]
fn home_handoffs_describe_the_actual_next_step() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    app.selected = None;
    app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus("开会".into())),
        None,
    );
    assert!(app.status.contains("尚未应用"));
    app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::CapCutHandoff),
        None,
    );
    assert_eq!(app.page, Page::Gallery);
    assert!(app.status.contains("选择素材"));
    for intent in [
        HomeAiIntent::AlipayCoupon,
        HomeAiIntent::PullRemoteAgentFiles,
    ] {
        app.apply_home_action(&Action::HomeExecuteIntent(intent), None);
        assert!(app.error && app.status.contains("尚未执行"));
    }
}

#[test]
fn failed_partial_snapshot_keeps_last_known_phone_data_and_age() {
    let mut app = Companion::new_for_test();
    app.back_screen.stepos_suite.wellbeing.screen_time_minutes = 80;
    let before = std::time::Instant::now();
    app.back_screen.stepos_suite_synced_at = Some(before);
    let mut partial = stepx_companion::toolbox::default_stepos_ai_suite_report();
    partial.read_error = Some("日历读取失败".into());
    app.apply(crate::ui::Outcome::StepOsSuite(partial));
    assert_eq!(
        app.back_screen.stepos_suite.wellbeing.screen_time_minutes,
        80
    );
    assert_eq!(app.back_screen.stepos_suite_synced_at, Some(before));
    assert!(app.back_screen.stepos_suite_error.is_some());
    assert!(app.error && app.status.contains("保留上次快照"));
}

#[test]
fn home_keeps_recent_discussion_above_duplicate_unsent_drafts() {
    use stepx_companion::amoo_workspace::{Executor, Phase, Step, Task, Turn};
    let mut app = Companion::new_for_test();
    let mut draft = Task::new(None, None);
    draft.title = "活动方案".into();
    draft.draft = "想办一场读书会".into();
    draft.updated = 10;
    let mut duplicate = draft.clone();
    duplicate.id = "duplicate-draft".into();
    duplicate.updated = 11;
    let mut discussion = Task::new(None, None);
    discussion.title = "场地安排".into();
    discussion.updated = 12;
    discussion.turns.push(Turn {
        user: true,
        text: "预算改为480元，请继续讨论方案".into(),
        at_ms: Some(12000),
    });
    let mut interrupted = Task::new(None, None);
    interrupted.updated = 1;
    interrupted.steps.push(Step {
        executor: Executor::Phone,
        phase: Phase::Running,
        note: "旧进度".into(),
    });
    app.workspace.journal.tasks = vec![
        draft.clone(),
        duplicate.clone(),
        discussion.clone(),
        interrupted,
    ];
    let home = app.home_recent_tasks();
    assert_eq!(
        home.iter().map(|task| task.id.as_str()).collect::<Vec<_>>(),
        vec![discussion.id.as_str(), duplicate.id.as_str()]
    );
    assert_eq!(
        app.workspace.journal.tasks.len(),
        4,
        "Home must not delete saved drafts or interrupted work"
    );
    app.workspace.journal.tasks[0].draft = "不同的活动设想".into();
    app.workspace.journal.tasks[0].updated = 20;
    assert_eq!(
        app.home_recent_tasks()[0].id,
        draft.id,
        "distinct drafts stay available"
    );
}

#[test]
fn home_all_conversations_opens_history_without_sending_or_clearing() {
    let mut app = Companion::new_for_test();
    app.fields[F_AMOO].set("保留未发送内容");
    app.workspace.history = false;
    app.apply_home_action(&Action::HomeShowHistory, None);
    assert_eq!(app.page, Page::Amoo);
    assert!(app.workspace.history);
    assert_eq!(app.fields[F_AMOO].text, "保留未发送内容");
    assert!(app.workspace.running.is_none());
    app.apply_home_action(&Action::HomeShowHistory, None);
    assert!(app.workspace.history, "opening history is not a toggle");
}

#[test]
fn home_stale_output_does_not_switch_task_or_start_work() {
    use stepx_companion::amoo_workspace::Task;
    let mut app = Companion::new_for_test();
    let task = Task::new(None, None);
    app.workspace.active = Some(task.id.clone());
    app.fields[F_AMOO].set("还没发送的内容");
    app.workspace.journal.tasks.push(task.clone());
    app.apply_home_action(
        &Action::HomeOpenTaskOutput {
            task: task.id.clone(),
            path: "/tmp/not-recorded.md".into(),
        },
        None,
    );
    assert!(app.error);
    assert_eq!(app.workspace.active.as_deref(), Some(task.id.as_str()));
    assert_eq!(app.fields[F_AMOO].text, "还没发送的内容");
    assert!(app.workspace.running.is_none() && !app.work.running);
}

#[test]
fn home_continues_original_task_without_resubmitting_or_losing_draft() {
    use stepx_companion::amoo_workspace::{Executor, Phase, Step, Task};
    let mut app = Companion::new_for_test();
    let mut first = Task::new(None, None);
    first.draft = "原对话的草稿".into();
    let mut phone = Task::new(Some("phone-a".into()), None);
    phone.id = "phone-work-existing".into();
    phone.title = "手机发起的报表".into();
    phone.draft = "继续核对第二页".into();
    phone.steps.push(Step {
        executor: Executor::Files,
        phase: Phase::Unconfirmed,
        note: "等待核对".into(),
    });
    app.workspace.active = Some(first.id.clone());
    app.fields[F_AMOO].set("原对话的未发送修改");
    app.workspace.journal.tasks = vec![first.clone(), phone.clone()];
    assert_eq!(app.home_recent_tasks()[0].id, phone.id);
    app.apply_home_action(&Action::HomeResumeTask(phone.id.clone()), None);
    assert_eq!(app.workspace.active.as_deref(), Some(phone.id.as_str()));
    assert_eq!(app.fields[F_AMOO].text, "继续核对第二页");
    assert_eq!(
        app.workspace
            .journal
            .tasks
            .iter()
            .find(|t| t.id == first.id)
            .unwrap()
            .draft,
        "原对话的未发送修改"
    );
    assert_eq!(app.page, Page::Amoo);
    assert!(app.workspace.running.is_none());
    assert!(!app.work.running);

    app.selected = Some(stepx_companion::device::Device {
        serial: "phone-b".into(),
        transport: "phone-b".into(),
        model: "PZ261A".into(),
        android: "16".into(),
        product: "pandora".into(),
    });
    assert!(app.home_recent_tasks().iter().all(|t| t.id != phone.id));
    app.apply_home_action(&Action::HomeResumeTask(phone.id), None);
    assert!(app.error && app.status.contains("其他设备"));
}

#[test]
fn live_mode_syncs_express_parcels_from_sms_and_rejects_offline_express_query() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    app.selected = None;
    app.express_parcels.clear();

    // Offline express query must not fabricate a local placeholder parcel or claim success
    app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::QueryExpress("SF1234567890123".into())),
        None,
    );
    assert!(app.error);
    assert!(app.status.contains("尚未执行"));
    assert_eq!(app.fields[F_OV_TEXT].text, "SF1234567890123");
    assert!(app.express_parcels.is_empty());

    // Receiving real SMS in live mode automatically populates express_parcels without demo data
    app.apply(crate::ui::Outcome::Sms(
        Ok(vec![stepx_companion::sms::Message {
            provider_id: Some(7),
            address: "10690000".into(),
            body: "【菜鸟驿站】您的京东包裹JD10293847561已到站，请凭取件码8-3-1024取件".into(),
            date_ms: 1711867800000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: Some(1),
        }]),
        None,
    ));
    assert_eq!(app.express_parcels.len(), 1);
    assert_eq!(app.express_parcels[0].tracking_no, "JD10293847561");
    assert_eq!(app.express_parcels[0].pickup_code, "8-3-1024");

    // StepOsSuite with phone_clipboard and backscreen_signature populates express_parcels and LED frame in live mode
    let mut suite = stepx_companion::toolbox::default_stepos_ai_suite_report();
    suite.checked = true;
    suite.phone_clipboard = "YT9876543210123".into();
    suite.backscreen_signature = "开会勿扰".into();
    app.apply(crate::ui::Outcome::StepOsSuite(suite));
    assert_eq!(app.express_parcels.len(), 2);
    assert!(
        app.back_screen.frame.iter().any(|&b| b > 0),
        "StepOsSuite with backscreen_signature must synthesize LED glyph frame"
    );
    app.with_home_report(|report, _, _| {
        assert_eq!(app.current_backscreen_signature(report), "开会勿扰");
    });

    // Clearing device state in live mode clears express_parcels
    app.clear_device_state();
    assert!(app.express_parcels.is_empty());
}

#[test]
fn demo_mode_sets_stepos_suite_synced_at_and_device_switch_clears_bg_receivers() {
    let mut app = Companion::new_for_test();
    #[cfg(feature = "ui-capture")]
    {
        app.demo_mode = true;
        app.load_demo();
        assert!(
                app.back_screen.stepos_suite_synced_at.is_some(),
                "load_demo() must set stepos_suite_synced_at so the Home header reflects synced calendar events"
            );
        assert!(!app.back_screen.stepos_suite_loading);
        assert!(app.back_screen.stepos_suite_error.is_none());
    }

    let (_tx, rx) = std::sync::mpsc::channel();
    app.bg_receivers.push((Page::Home, None, rx));
    assert_eq!(app.bg_receivers.len(), 1);
    app.clear_device_state();
    assert!(
            app.bg_receivers.is_empty(),
            "clear_device_state() must clear bg_receivers so stale background outcomes from a previous device cannot apply"
        );
}

#[test]
fn home_calendar_date_selection_updates_panorama_weekly_bars_and_context_life_cards() {
    use chrono::TimeZone;
    let mut app = Companion::new_for_test();
    app.home_tools_expanded = true;

    let target_date = NaiveDate::from_ymd_opt(2026, 11, 4).unwrap();
    let target_start_ms = Local
        .from_local_datetime(&target_date.and_hms_opt(14, 0, 0).unwrap())
        .single()
        .unwrap()
        .timestamp_millis();
    app.back_screen.stepos_suite.calendar_events.push(
        stepx_companion::toolbox::PhoneCalendarEvent {
            id: 901,
            title: "周末读书会场地踏勘".into(),
            location: "静安区愚园路空间".into(),
            dtstart: target_start_ms,
            dtend: target_start_ms + 3_600_000,
            all_day: false,
            ..Default::default()
        },
    );
    app.context
        .overview
        .items
        .push(stepx_companion::context_ledger::PreparedItem {
            id: "rain-query".into(),
            title: "降雨查询".into(),
            status: "关注11月4日小雨天气节点".into(),
            summary: "近期多次关注天气变化，重点确认11月4日是否下雨及具体降雨时段".into(),
            updated_ms: target_start_ms,
            ..Default::default()
        });

    app.back_screen.calendar_year = 2026;
    app.back_screen.calendar_month = 11;
    app.back_screen.selected_calendar_day = 4;
    assert_eq!(app.home_selected_date(), target_date);

    app.with_home_report(|report, snapshot, express_parcels| {
        assert_eq!(report.life_context_cards.len(), 3);
        assert!(
            report
                .life_context_cards
                .iter()
                .all(|c| c.context_link.contains("关联")),
            "All 3 life context cards must carry explicit context_link explanations"
        );
        assert!(
            report
                .life_context_cards
                .iter()
                .any(|c| c.domain == HomeLifeDomain::WeatherAndCommute
                    && c.title == "降雨提醒已联动「周末读书会场地踏勘」"
                    && c.context_link.contains("台账「降雨查询」")
                    && c.context_link.contains("日历「周末读书会场地踏勘」")),
            "WeatherAndCommute life card must link the ledger weather query to the selected schedule"
        );

        let (panorama, weekly) = app.build_home_panorama_and_weekly(snapshot, express_parcels);
        assert_eq!(panorama.date_ymd, "2026-11-04");
        assert!(panorama.schedule_count >= 1);
        assert!(
            panorama
                .entries
                .iter()
                .any(|e| e.title.contains("周末读书会场地踏勘")),
            "Selected date panorama must include the event on 2026-11-04"
        );
        assert_eq!(weekly.days.len(), 7);
        assert!(weekly.days.iter().any(|d| d.is_selected && d.day == 4));
        assert_eq!(weekly.pillars.len(), 4);
    });

    app.home_modules.apply_preset(HomeModulePreset::CompleteAll);
    let _ = app.home_page();
    let slots = app.svg_slots.take_pending();
    for fixed in ["usage-donut"] {
        assert!(
            slots.iter().any(|(slot, _, _)| slot == fixed),
            "missing fixed SVG chart {fixed}"
        );
    }
    assert!(
        slots.iter().all(|(slot, _, _)| {
            slot != "timeline"
                && slot != "weekly-bars"
                && !slot.starts_with("parcel-step-")
                && !slot.starts_with("kpi-")
        }),
        "responsive charts must use native geometry"
    );
}

#[test]
fn real_ledger_prepared_items_extract_evidence_quotes_titles_and_date_mentions() {
    use chrono::TimeZone;
    use stepx_companion::context_ledger::SourceObject;
    use stepx_companion::context_review::{MatterAction, MatterPresentation};

    let mut app = Companion::new_for_test();
    app.back_screen.calendar_year = 2026;
    app.back_screen.calendar_month = 11;
    app.back_screen.selected_calendar_day = 1;

    let rain_item = stepx_companion::context_ledger::PreparedItem {
            id: "48cb59bb0929f3a958595ef513c005f9".into(),
            title: "降雨查询".into(),
            status: "资料已整理".into(),
            summary: "1 个来源上下文 · 0 个问题 · 0 条承诺候选 · 0 份文件".into(),
            updated_ms: Local::now().timestamp_millis() - 3_600_000,
            source_count: 1,
            source_objects: vec![SourceObject {
                id: "s1".into(),
                source: "手机 Amoo 讨论".into(),
                title: "降雨查询".into(),
                task_id: None,
            }],
            prompt: "<evidence>\n- 来源：手机 Amoo 讨论 · 降雨查询；时间：2026-10-28 22:28；作者：用户\n  > 明天会下雨吗\n- 来源：手机 Amoo 讨论 · 降雨查询；时间：2026-10-28 22:28；作者：AI 回复（未经独立核验）\n  > 今天不会下雨：你现在所在的位置是上海奉贤，目前是阴天，17℃，空气为优。  往后看，真正有雨的是这几个时段： - **明天（10/29）夜间**有小雨 - **11月1日（周日）夜间**有小雨 - **11月4日（周三）夜间**有小雨，**11月5日（周四）白天**有小雨\n</evidence>".into(),
            presentation: Some(MatterPresentation {
                headline: "降雨查询".into(),
                priority: 26,
                what_happened: "已保留讨论资料，尚未找到可以直接推荐的当前行动。".into(),
                next_step: "查看来源".into(),
                primary_action: MatterAction::ReviewSources,
                ..Default::default()
            }),
            ..Default::default()
        };

    let book_club_item = stepx_companion::context_ledger::PreparedItem {
            id: "7afc008a33a21b91ce4c771325bc52a1".into(),
            title: "bookclub-20261030-a".into(),
            status: "有变更，需核对".into(),
            summary: "2 个来源上下文 · 2 个问题 · 2 条承诺候选 · 0 份文件".into(),
            updated_ms: Local::now().timestamp_millis() - 1_800_000,
            source_count: 2,
            source_objects: vec![
                SourceObject {
                    id: "s2".into(),
                    source: "微信读书群".into(),
                    title: "[项目: BOOKCLUB-20261030-A] 读书会安排更新".into(),
                    task_id: None,
                },
                SourceObject {
                    id: "s3".into(),
                    source: "手机 Amoo 讨论".into(),
                    title: "社区读书会筹备".into(),
                    task_id: None,
                },
            ],
            prompt: "<evidence>\n- 来源：微信读书群；时间：2026-10-30 10:19；作者：用户\n  > [项目: BOOKCLUB-20261030-A] 时间改到2026年11月1日14:00；场地询价由小林负责，手机报名入口由我负责。\n- 来源：手机 Amoo 讨论 · 社区读书会筹备；时间：2026-10-30 10:19；作者：用户\n  > [项目: BOOKCLUB-20261030-A] 本周读书会原定2026年10月31日15:00，预算上限500元，尚未订场地、尚未开放报名。\n</evidence>".into(),
            presentation: Some(MatterPresentation {
                headline: "信息变更".into(),
                priority: 95,
                what_happened: "[项目: BOOKCLUB-20261030-A] 时间改到2026年11月1日14:00；场地询价由小林负责，手机报名入口由我负责。".into(),
                next_step: "核对最新安排".into(),
                primary_action: MatterAction::ContinueDiscussion,
                ..Default::default()
            }),
            ..Default::default()
        };

    let synthetic_probe_item = stepx_companion::context_ledger::PreparedItem {
        id: "probe-e2e-1".into(),
        title: "stepx-auto-20261030-a".into(),
        status: "有变更，需核对".into(),
        summary: "2 个来源上下文 · 2 个问题".into(),
        updated_ms: Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 10, 30)
                    .unwrap()
                    .and_hms_opt(10, 20, 0)
                    .unwrap(),
            )
            .single()
            .unwrap()
            .timestamp_millis(),
        source_count: 1,
        source_objects: vec![SourceObject {
            id: "s-probe".into(),
            source: "StepX 上下文验收".into(),
            title: "[项目: STEPX-AUTO-20261030-A] 验收资料".into(),
            task_id: None,
        }],
        ..Default::default()
    };

    let cross_date_calendar_item = stepx_companion::context_ledger::PreparedItem {
        id: "cal-oct31-roadtrip".into(),
        title: "自驾出发".into(),
        status: "资料已整理".into(),
        summary: "1 个来源上下文 · 0 个问题".into(),
        updated_ms: Local
            .from_local_datetime(
                &NaiveDate::from_ymd_opt(2026, 10, 30)
                    .unwrap()
                    .and_hms_opt(9, 0, 0)
                    .unwrap(),
            )
            .single()
            .unwrap()
            .timestamp_millis(),
        source_count: 1,
        source_objects: vec![SourceObject {
            id: "s-trip".into(),
            source: "出行备忘".into(),
            title: "自驾出发".into(),
            task_id: None,
        }],
        presentation: Some(MatterPresentation {
            headline: "自驾出发".into(),
            priority: 40,
            what_happened: "10-31 07:00 · 自驾出发".into(),
            next_step: "查看日程".into(),
            primary_action: MatterAction::PrepareSchedule,
            ..Default::default()
        }),
        ..Default::default()
    };

    assert!(
        Companion::is_synthetic_ledger_item(&synthetic_probe_item),
        "Synthetic E2E probe item must be identified as synthetic"
    );
    assert!(
        !Companion::is_synthetic_ledger_item(&book_club_item),
        "Real book club item must not be flagged as synthetic"
    );

    let rain_detail = Companion::ledger_item_display_detail(&rain_item);
    assert!(
            rain_detail.contains("10/29") && rain_detail.contains("11月1日") && !rain_detail.contains("个来源上下文"),
            "Rain item detail must extract evidence quote instead of generic counter summary: {rain_detail}"
        );
    let book_title = Companion::ledger_item_display_title(&book_club_item);
    assert!(
        book_title.contains("社区读书会筹备"),
        "ASCII project slug item should include human-readable source title: {book_title}"
    );

    app.context.overview.items = vec![
        synthetic_probe_item,
        book_club_item,
        rain_item,
        cross_date_calendar_item,
    ];

    app.with_home_report(|report, snapshot, express_parcels| {
        assert!(
            report
                .life_context_cards
                .iter()
                .all(|c| !c.title.contains("stepx-auto") && !c.detail.contains("STEPX-AUTO")),
            "Synthetic E2E probe must not leak into life context cards"
        );
        let weather_card = report
            .life_context_cards
            .iter()
            .find(|c| c.domain == HomeLifeDomain::WeatherAndCommute)
            .unwrap();
        assert!(
            weather_card.detail.contains("小雨") && !weather_card.detail.contains("个来源上下文"),
            "Weather card detail must show actual rain forecast instead of counter string: {}",
            weather_card.detail
        );
        let leisure_card = report
            .life_context_cards
            .iter()
            .find(|c| c.domain == HomeLifeDomain::LeisureAndWellbeing)
            .unwrap();
        assert!(
            leisure_card.title.contains("读书会") && !leisure_card.detail.contains("个来源上下文"),
            "Leisure card must link to book club item with clean detail: {} / {}",
            leisure_card.title,
            leisure_card.detail
        );

        // Selecting 2026-11-01 should show both the 11月1日 rain mention AND the 2026年11月1日 book club reschedule!
        let (pan_nov1, _) = app.build_home_panorama_and_weekly(snapshot, express_parcels);
        assert_eq!(pan_nov1.date_ymd, "2026-11-01");
        assert!(
            pan_nov1
                .entries
                .iter()
                .any(|e| e.title.contains("降雨查询")),
            "11/01 panorama must include projected rain forecast node"
        );
        assert!(
            pan_nov1.entries.iter().any(|e| e.title.contains("读书会")),
            "11/01 panorama must include projected book club reschedule node"
        );
        assert!(
            pan_nov1
                .entries
                .iter()
                .all(|e| !e.title.contains("stepx-auto")),
            "11/01 panorama must exclude synthetic E2E probes"
        );
    });

    // Verify cross-date calendar item ("10-31 07:00 · 自驾出发", updated on 10-30) does NOT pollute 2026-10-30
    app.back_screen.calendar_year = 2026;
    app.back_screen.calendar_month = 10;
    app.back_screen.selected_calendar_day = 30;
    app.with_home_report(|_, snapshot, express_parcels| {
        let (pan_oct30, _) = app.build_home_panorama_and_weekly(snapshot, express_parcels);
        assert_eq!(pan_oct30.date_ymd, "2026-10-30");
        assert!(
            pan_oct30
                .entries
                .iter()
                .all(|e| !e.title.contains("自驾出发")),
            "10-31 calendar item updated on 10-30 must not pollute 2026-10-30 panorama"
        );
        assert!(
            pan_oct30
                .entries
                .iter()
                .all(|e| !e.title.contains("stepx-auto")),
            "Synthetic E2E probe must not appear in 2026-10-30 panorama"
        );
    });

    // Selecting 2026-10-31 must include "自驾出发"
    app.back_screen.selected_calendar_day = 31;
    app.with_home_report(|_, snapshot, express_parcels| {
        let (pan_oct31, _) = app.build_home_panorama_and_weekly(snapshot, express_parcels);
        assert_eq!(pan_oct31.date_ymd, "2026-10-31");
        assert!(
            pan_oct31
                .entries
                .iter()
                .any(|e| e.title.contains("自驾出发")),
            "10-31 calendar item must appear on 2026-10-31 panorama"
        );
    });

    // Verify Dec <-> Jan year-boundary date mention resolution
    let jan_from_dec =
        Companion::extract_month_day_mentions_with_anchor("01-02 10:00 元旦复盘", 2026, 12);
    assert_eq!(
        jan_from_dec,
        vec![NaiveDate::from_ymd_opt(2027, 1, 2).unwrap()],
        "January mention from December anchor must resolve to next year"
    );
    let dec_from_jan =
        Companion::extract_month_day_mentions_with_anchor("12月30日 晚间总结", 2027, 1);
    assert_eq!(
        dec_from_jan,
        vec![NaiveDate::from_ymd_opt(2026, 12, 30).unwrap()],
        "December mention from January anchor must resolve to previous year"
    );

    // Verify synthetic E2E probes (compact_live_check SMS & synthetic CSV files) are filtered
    assert!(stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "我: StepX Companion 简洁模式回复验证（收到请忽略）"
    ));
    assert!(stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "/sdcard/Download/StepX-retry.csv"
    ));
    assert!(!stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "/sdcard/Download/AI终端与移动生产力深度调研报告.md"
    ));

    // Verify select_diverse_panorama_entries includes Schedule even when 4 morning items precede an evening schedule
    let sample_entries = vec![
        stepx_companion::home_hub::SelectedDateEntry {
            id: "1".into(),
            kind: SelectedDateEntryKind::WorkAndMatter,
            time_hm: "08:45".into(),
            sort_minute: 525,
            badge: "事项".into(),
            title: "Morning Work".into(),
            detail: String::new(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
            checklist_items: Vec::new(),
        },
        stepx_companion::home_hub::SelectedDateEntry {
            id: "2".into(),
            kind: SelectedDateEntryKind::Life,
            time_hm: "09:05".into(),
            sort_minute: 545,
            badge: "天气".into(),
            title: "Morning Life".into(),
            detail: String::new(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
            checklist_items: Vec::new(),
        },
        stepx_companion::home_hub::SelectedDateEntry {
            id: "3".into(),
            kind: SelectedDateEntryKind::Issue,
            time_hm: "09:15".into(),
            sort_minute: 555,
            badge: "待处理".into(),
            title: "Morning Issue 1".into(),
            detail: String::new(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
            checklist_items: Vec::new(),
        },
        stepx_companion::home_hub::SelectedDateEntry {
            id: "4".into(),
            kind: SelectedDateEntryKind::Issue,
            time_hm: "09:20".into(),
            sort_minute: 560,
            badge: "待处理".into(),
            title: "Morning Issue 2".into(),
            detail: String::new(),
            action_label: "打开".into(),
            intent: HomeAiIntent::OpenPage("work".into()),
            checklist_items: Vec::new(),
        },
        stepx_companion::home_hub::SelectedDateEntry {
            id: "5".into(),
            kind: SelectedDateEntryKind::Schedule,
            time_hm: "20:00".into(),
            sort_minute: 1200,
            badge: "日程".into(),
            title: "Evening Schedule".into(),
            detail: String::new(),
            action_label: "背屏".into(),
            intent: HomeAiIntent::OpenPage("calendar".into()),
            checklist_items: Vec::new(),
        },
    ];
    let picked = Companion::select_diverse_panorama_entries(&sample_entries, 4);
    assert_eq!(picked.len(), 4);
    assert!(
        picked
            .iter()
            .any(|e| e.kind == SelectedDateEntryKind::Schedule && e.title == "Evening Schedule"),
        "Category-diverse panorama selection must preserve evening Schedule in 4-card strip"
    );
}

#[test]
fn home_live_unsynced_authenticity_and_wireless_transport_failover_verification() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    app.selected = Some(stepx_companion::device::Device {
        serial: "HOMEVERIFYUNIT001".into(),
        transport: "10.0.0.42:34257".into(),
        model: "StepPhone".into(),
        android: "16".into(),
        product: "Basic".into(),
    });
    app.page = Page::Home;
    app.home_tools_expanded = false;

    // 1. Before StepOS suite and Push sync complete in live mode, home_page() must render honest waiting states without panicking
    assert!(!app.back_screen.stepos_suite.checked);
    assert!(!app.push.loaded);
    let _ = app.home_page();
    let _ = app.render_home_tools_summary_panel();

    // 2. Synthetic E2E probe text in notifications/SMS/clipboard must not leak into extract_live_express_parcels
    let probe_sms = vec![stepx_companion::sms::Message {
            provider_id: Some(1),
            address: "10690000".into(),
            body: "[项目: STEPX-AUTO-20261030-A] 【菜鸟驿站】您的包裹已到站，请凭取件码 9-9-9999 取件，运单号 SF1234567890123".into(),
            date_ms: 1_793_318_400_000,
            kind: 1,
            read: false,
            outgoing: false,
            subscription: Some(1),
        }];
    let real_sms = vec![stepx_companion::sms::Message {
        provider_id: Some(2),
        address: "10698888".into(),
        body: "【丰巢】您的顺丰快递 SF1357924680123 已存入上海奉贤丰巢柜，取件码 3-2-1088".into(),
        date_ms: Local::now().timestamp_millis() - 3_600_000,
        kind: 1,
        read: false,
        outgoing: false,
        subscription: Some(1),
    }];
    let parcels_from_probe = stepx_companion::home_hub::extract_live_express_parcels(
        &probe_sms,
        &[],
        "STEPX_E2E_PROBE SF9999999999999",
        &[],
    );
    assert!(
        parcels_from_probe.is_empty(),
        "Synthetic E2E probe SMS/clipboard must not be extracted as live express parcels"
    );
    let parcels_from_real =
        stepx_companion::home_hub::extract_live_express_parcels(&real_sms, &[], "", &[]);
    assert_eq!(parcels_from_real.len(), 1);
    assert_eq!(parcels_from_real[0].tracking_no, "SF1357924680123");
    assert_eq!(parcels_from_real[0].pickup_code, "3-2-1088");

    // 3. Verify USB-to-wireless transport failover across Device methods and Outcome::Transports
    #[cfg(unix)]
    {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let log_path = temp.path().join("adb_calls.log");
        let fake_adb = temp.path().join("adb");
        // Legacy DIAL is permitted only after successful read-only discovery
        // confirms that this phone has no native dialpad tool.
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let catalog_peer = std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            stream
                .set_write_timeout(Some(std::time::Duration::from_secs(3)))
                .unwrap();
            for command in ["connect", "ability.list"] {
                let mut length = [0; 4];
                stream.read_exact(&mut length).unwrap();
                let length = u32::from_be_bytes(length) as usize;
                assert!(length < 1024 * 1024);
                let mut body = vec![0; length];
                stream.read_exact(&mut body).unwrap();
                let request: serde_json::Value = serde_json::from_slice(&body).unwrap();
                let response = if command == "connect" {
                    assert_eq!(request["method"], "connect");
                    serde_json::json!({"type":"res","id":request["id"],"ok":true,
                        "payload":{"protocol":3,"server":{"version":"test"}}})
                } else {
                    assert_eq!(request["payload"]["command"], "ability.list");
                    serde_json::json!({"type":"req","method":"node.invoke.result",
                        "params":{"id":request["payload"]["id"],"ok":true,
                            "payload":{"ok":true,"abilities":[]}}})
                };
                let body = serde_json::to_vec(&response).unwrap();
                stream.write_all(&(body.len() as u32).to_be_bytes()).unwrap();
                stream.write_all(&body).unwrap();
            }
        });
        let script = format!(
            r#"#!/bin/sh
printf '%s\n' "$*" >> "{log}"
if [ "$1" = "devices" ]; then
  printf '%s\n' "List of devices attached" "10.0.0.42:34257       device product:Basic model:StepPhone device:P01 transport_id:7"
  exit 0
fi
if [ "$1" = "-s" ] && [ "$2" = "10.0.0.42:34257" ]; then
  shift 2
  if [ "$1" = "forward" ] && [ "$2" = "tcp:0" ]; then
    echo "{port}"
    exit 0
  fi
  if [ "$1" = "forward" ] && [ "$2" = "--remove" ]; then
    exit 0
  fi
  if [ "$1" = "shell" ]; then
    shift
    case "$*" in
      *"getprop"*"ro.serialno"*)
        echo "HOMEVERIFYUNIT001"
        exit 0
        ;;
      *"am"*"start"*|*"pm"*"list"*"packages"*)
        echo "package:com.stepos.ailauncher"
        exit 0
        ;;
    esac
  fi
  printf 'unsupported fake adb command: %s\n' "$*" >&2
  exit 64
fi
echo "error: device '$2' not found" >&2
exit 1
"#,
            log = log_path.display()
        );
        std::fs::write(&fake_adb, script).unwrap();
        let mut perms = std::fs::metadata(&fake_adb).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake_adb, perms).unwrap();

        let adb = stepx_companion::adb::Adb { path: fake_adb };
        // Validate and start the process fixture before exercising the bounded
        // recovery path. An env-selected Bash cold start on a busy CI host is
        // not a phone identity response; use the platform POSIX shell and keep
        // the production 900 ms identity probe and six-second budget unchanged.
        let fixture_identity = adb
            .run_timeout(
                &["-s", "10.0.0.42:34257", "shell", "getprop", "ro.serialno"],
                std::time::Duration::from_secs(3),
            )
            .expect("fake adb must start and return its explicit device identity");
        assert_eq!(fixture_identity.trim(), "HOMEVERIFYUNIT001");
        // Device initially has USB transport ("HOMEVERIFYUNIT001"), with wireless backup ("10.0.0.42:34257") online
        let usb_device_with_wireless_standby = stepx_companion::device::Device {
            serial: "HOMEVERIFYUNIT001".into(),
            transport: "HOMEVERIFYUNIT001".into(),
            model: "StepPhone".into(),
            android: "16".into(),
            product: "Basic".into(),
        };
        usb_device_with_wireless_standby.set_backup_transports(vec!["10.0.0.42:34257".into()]);
        let resolved = usb_device_with_wireless_standby
            .resolve_active_transport(&adb)
            .unwrap_or_else(|error| {
                panic!(
                    "Must fail over to online wireless backup transport: {error:#}\nFake ADB calls:\n{}",
                    std::fs::read_to_string(&log_path).unwrap_or_default()
                )
            });
        assert_eq!(resolved, "10.0.0.42:34257");
        assert_eq!(usb_device_with_wireless_standby.serial, "HOMEVERIFYUNIT001");

        // Calling Device::dial_number and Device::apps_with_filter must route via resolved 10.0.0.42:34257
        usb_device_with_wireless_standby
            .dial_number(&adb, "10086")
            .expect("dial_number must succeed via wireless failover transport");
        catalog_peer.join().unwrap();
        let pkgs = usb_device_with_wireless_standby
            .apps_with_filter(&adb, true)
            .expect("apps_with_filter must succeed via wireless failover transport");
        assert!(pkgs.contains(&"com.stepos.ailauncher".to_string()));

        let logged = std::fs::read_to_string(&log_path).unwrap();
        assert!(
            logged.contains("-s 10.0.0.42:34257 shell")
                && logged.contains(
                    "-s 10.0.0.42:34257 forward tcp:0 localabstract:agentbus"
                )
                && logged.contains("android.intent.action.DIAL"),
            "dial_number must pass -s 10.0.0.42:34257: {logged}"
        );
    }
}

#[test]
fn format_time_zero_or_negative_timestamp_returns_empty_string() {
    assert_eq!(crate::ui::pages::format_time(0), "");
    assert_eq!(crate::ui::pages::format_time(-1), "");
    assert!(!crate::ui::pages::format_time(1_730_000_000_000).is_empty());
}

#[test]
fn home_in_place_alarm_checklist_and_ledger_echo_filtering_work_end_to_end() {
    use chrono::TimeZone;
    use stepx_companion::context_ledger::{PreparedItem, SourceObject};

    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.page = Page::Home;
    app.back_screen.calendar_year = 2026;
    app.back_screen.calendar_month = 10;
    app.back_screen.selected_calendar_day = 30;

    // 1. Add a morning trip at 2026-10-30 07:00 without a matching early alarm -> conflict detected
    let dt_start = chrono::Local
        .with_ymd_and_hms(2026, 10, 30, 7, 0, 0)
        .unwrap()
        .timestamp_millis();
    let dt_end = dt_start + 3_600_000;
    app.back_screen.stepos_suite.checked = true;
    app.back_screen.stepos_suite.alarms.clear();
    app.back_screen.stepos_suite.calendar_events =
        vec![stepx_companion::toolbox::PhoneCalendarEvent {
            recurring: Some(false),
            id: 301,
            title: "07:00 自驾出发".into(),
            dtstart: dt_start,
            dtend: dt_end,
            all_day: false,
            location: "虹桥枢纽".into(),
            description: "准备：身份证、充电宝、雨伞".into(),
            source: "手机日历".into(),
        }];

    app.with_home_report(|report, _, _| {
        assert_eq!(
            report.schedule_conflicts.len(),
            1,
            "Early 07:00 trip without alarm must surface a schedule conflict"
        );
        assert_eq!(report.schedule_conflicts[0].suggested_alarm_hour, 6);
        assert_eq!(report.schedule_conflicts[0].suggested_alarm_minute, 0);
    });

    // Execute in-place CreatePhoneAlarm intent and verify it stays on Page::Home and resolves the conflict
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::CreatePhoneAlarm {
            hour: 6,
            minute: 0,
            label: "07:00 自驾出发".into(),
        }),
        None,
    ));
    assert_eq!(app.page, Page::Home);
    assert!(
        app.back_screen
            .stepos_suite
            .alarms
            .iter()
            .any(|a| a.hour == 6 && a.minutes == 0 && a.enabled),
        "06:00 alarm must be created and enabled in-place"
    );
    app.with_home_report(|report, _, _| {
        assert!(
            report.schedule_conflicts.is_empty(),
            "Creating the 06:00 alarm must resolve the schedule conflict"
        );
    });

    // 2. Verify interactive checklist toggling and unchecked backscreen text synthesis
    let checklists = stepx_companion::home_hub::extract_schedule_checklists_for_date(
        "2026-10-30",
        &app.back_screen.stepos_suite.calendar_events,
    );
    assert_eq!(checklists.len(), 1);
    let cl = &checklists[0];
    assert!(cl.items.iter().any(|i| i.contains("身份证")));
    let first_item = cl.items[0].clone();
    let state_key = Companion::checklist_item_state_key(cl.event_id, &first_item);
    assert!(!app.is_checklist_item_checked(cl.event_id, &first_item));
    assert!(app.apply_home_action(&Action::HomeToggleChecklistItem(state_key.clone()), None));
    assert!(app.is_checklist_item_checked(cl.event_id, &first_item));
    let bs_text = app.build_unchecked_checklist_backscreen_text(cl);
    assert!(
        !bs_text.contains(&first_item),
        "Backscreen checklist text must exclude checked item {first_item}: {bs_text}"
    );

    // 3. Verify historical calendar echo (`calendar:双端已同步:25:1793257200000` = 2026-10-29 15:00)
    // and `STEPX-E2E-20261029` in `item.prompt` do not pollute 2026-10-30 panorama or issues
    let oct30_noon_ms = chrono::Local
        .with_ymd_and_hms(2026, 10, 30, 12, 0, 0)
        .unwrap()
        .timestamp_millis();
    let historical_cal_echo = PreparedItem {
        id: "item-cal-echo-25".into(),
        title: "日程 · 15:00 产品周会".into(),
        status: "有变更，需核对".into(),
        summary: "来自日历同步".into(),
        updated_ms: oct30_noon_ms,
        source_count: 1,
        source_objects: vec![SourceObject {
            id: "calendar:双端已同步:25:1793257200000".into(),
            source: "手机日历".into(),
            title: "15:00 产品周会".into(),
            task_id: None,
        }],
        ..Default::default()
    };
    let prompt_probe_item = PreparedItem {
        id: "item-prompt-probe".into(),
        title: "日常工作".into(),
        status: "处理中".into(),
        summary: "正常摘要".into(),
        updated_ms: oct30_noon_ms,
        prompt: "请处理 STEPX-E2E-20261029 自动化验证".into(),
        ..Default::default()
    };
    assert!(Companion::is_pure_calendar_ledger_echo(
        &historical_cal_echo
    ));
    assert!(Companion::is_synthetic_ledger_item(&prompt_probe_item));

    app.context.overview.items = vec![historical_cal_echo, prompt_probe_item];
    app.back_screen.selected_calendar_day = 30;
    app.with_home_report(|_, snapshot, express_parcels| {
            let (pan_oct30, _) = app.build_home_panorama_and_weekly(snapshot, express_parcels);
            assert!(
                pan_oct30
                    .entries
                    .iter()
                    .all(|e| !e.title.contains("产品周会") && !e.title.contains("日常工作")),
                "2026-10-29 calendar echo and STEPX-E2E prompt probe must not pollute 2026-10-30 panorama"
            );
        });
    app.back_screen.selected_calendar_day = 29;
    app.with_home_report(|_, snapshot, express_parcels| {
            let (pan_oct29, _) = app.build_home_panorama_and_weekly(snapshot, express_parcels);
            assert!(
                pan_oct29
                    .entries
                    .iter()
                    .all(|e| !e.title.contains("产品周会")),
                "Deleted phone calendar event echo in context ledger must not resurrect on Page::Home when absent from calendar_events"
            );
        });
}

#[test]
fn boost_architecture_performance_benchmark_suite() {
    use std::hint::black_box;
    use std::time::Instant;

    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.page = Page::Home;
    app.home_tools_expanded = true;
    app.init_demo_push();
    app.express_parcels = stepx_companion::device::default_express_parcels();
    app.back_screen.stepos_suite = stepx_companion::toolbox::default_stepos_ai_suite_report();

    // Warm up caches once
    let _ = black_box(app.home_page());
    let _ = black_box(crate::app_store::curated_catalog());

    // 1. Page::Home full frame widget + SVG + report construction (50 frames)
    let t0 = Instant::now();
    for _ in 0..50 {
        black_box(app.home_page());
    }
    let home_page_avg_us = t0.elapsed().as_micros() as f64 / 50.0;

    // 2. HomeHub weekly + panorama + life context pipeline (100 runs)
    let desktop_entries = app.home_desktop_date_entries();
    let selected_ymd = app.home_selected_date().format("%Y-%m-%d").to_string();
    let now = Local::now();
    let today_ymd = now.format("%Y-%m-%d").to_string();
    let suite = &app.back_screen.stepos_suite;
    let input = HomeHubInput {
        hour: 10,
        minute: 30,
        today_ymd: &today_ymd,
        device_connected: true,
        device_model: "StepX Phone",
        firmware_profile: "StepOS 12",
        threads: &app.push.snapshot.threads,
        notifications: &app.push.snapshot.notifications,
        otp_banners: &app.push.snapshot.otp_banners,
        sms: &app.sms,
        call_logs: &app.call_logs,
        express_parcels: &app.express_parcels,
        calendar_events: &suite.calendar_events,
        alarms: &suite.alarms,
        wellbeing: &suite.wellbeing,
        phone_clipboard: &suite.phone_clipboard,
        remote_agent_files: &[],
        work_deliverables_count: 3,
        pomodoro_running: false,
        pomodoro_remaining_secs: 0,
        dnd_on: false,
    };
    let t1 = Instant::now();
    for _ in 0..100 {
        let pan = build_selected_date_panorama(&selected_ymd, &input, &desktop_entries);
        let weekly = build_weekly_overview_report(&selected_ymd, &input, &desktop_entries);
        let life = stepx_companion::home_hub::build_life_context_cards_with_desktop(
            &selected_ymd,
            &input,
            &desktop_entries,
        );
        black_box((pan, weekly, life));
    }
    let home_hub_pipeline_avg_us = t1.elapsed().as_micros() as f64 / 100.0;

    // 3. Synthetic E2E probe text filtering (500 x 10 realistic tuples)
    let sample_triples = [
        (
            "微信",
            "产品群：张经理",
            "下午三点评审下周版本排期与交付清单",
        ),
        ("飞书", "研发协同", "已完成接口联调，请查收测试报告"),
        (
            "顺丰速运",
            "派件通知",
            "您的快件已到达丰巢快递柜，取件码 882910",
        ),
        (
            "手机日历",
            "10:00 架构评审会议",
            "准备：设计文档、压测报告、上线Checklist",
        ),
        (
            "短信",
            "10698000",
            "【菜鸟驿站】您的包裹已到站，凭 3-2-1048 取件",
        ),
        (
            "系统通知",
            "天气预警",
            "今日傍晚有阵雨，出门请携带雨伞并注意路况",
        ),
        ("企业微信", "运维值班群", "核心集群 CPU 负载平稳，零告警"),
        ("钉钉", "审批通知", "您的差旅报销单已通过部门主管审批"),
        (
            "高德地图",
            "通勤提醒",
            "前往虹桥火车站当前路况畅通，预计 35 分钟",
        ),
        (
            "Amoo",
            "工作台任务",
            "整理本周跨端协同演进复盘与性能指标对比",
        ),
    ];
    let t2 = Instant::now();
    for _ in 0..500 {
        let mut hits = 0usize;
        for (a, b, c) in &sample_triples {
            if stepx_companion::home_hub::any_synthetic_e2e_probe(&[*a, *b, *c]) {
                hits += 1;
            }
        }
        black_box(hits);
    }
    let probe_filter_avg_us = t2.elapsed().as_micros() as f64 / 500.0;

    // 4. AppStore curated_catalog() construction & cache check (50 runs)
    let t3 = Instant::now();
    for _ in 0..50 {
        black_box(crate::app_store::curated_catalog());
    }
    let curated_catalog_avg_us = t3.elapsed().as_micros() as f64 / 50.0;

    // 5. AppStore search_instant_local across 3 queries (30 runs)
    let t4 = Instant::now();
    for _ in 0..30 {
        black_box(crate::app_store::search_instant_local("微信"));
        black_box(crate::app_store::search_instant_local("nas"));
        black_box(crate::app_store::search_instant_local("鸿蒙"));
    }
    let instant_search_avg_us = t4.elapsed().as_micros() as f64 / 30.0;

    // 6. AppStore sub-tag classification across catalog (100 runs)
    let catalog = crate::app_store::curated_catalog();
    let tags = crate::app_store::category_sub_tags(crate::app_store::StoreCategory::All);
    let t5 = Instant::now();
    for _ in 0..100 {
        let mut matched = 0usize;
        for &tag in tags {
            for store_app in &catalog {
                if crate::app_store::app_matches_sub_tag(store_app, tag) {
                    matched += 1;
                }
            }
        }
        black_box(matched);
    }
    let subtag_filter_avg_us = t5.elapsed().as_micros() as f64 / 100.0;

    // 7. Page::AppStore filtered_store_apps + app_store_page render (50 frames)
    app.page = Page::AppStore;
    app.app_store.sub_tag = Some("AI 助手".into());
    let t6 = Instant::now();
    for _ in 0..50 {
        black_box(app.filtered_store_apps());
        black_box(app.app_store_page());
    }
    let app_store_page_avg_us = t6.elapsed().as_micros() as f64 / 50.0;

    eprintln!(
        "BOOST_BENCHMARK_METRICS {}",
        serde_json::json!({
            "home_page_frame_us": (home_page_avg_us * 10.0).round() / 10.0,
            "home_hub_pipeline_us": (home_hub_pipeline_avg_us * 10.0).round() / 10.0,
            "probe_filter_10x_us": (probe_filter_avg_us * 100.0).round() / 100.0,
            "curated_catalog_us": (curated_catalog_avg_us * 10.0).round() / 10.0,
            "instant_search_3q_us": (instant_search_avg_us * 10.0).round() / 10.0,
            "subtag_filter_all_us": (subtag_filter_avg_us * 10.0).round() / 10.0,
            "app_store_page_frame_us": (app_store_page_avg_us * 10.0).round() / 10.0,
        })
    );
}

#[test]
fn boost_architecture_edge_cases_and_consolidations_contract() {
    use stepx_companion::home_hub::{
        any_synthetic_e2e_probe, contains_ascii_ignore_case, is_synthetic_e2e_probe_text,
        truncate_chars,
    };

    // 1. contains_ascii_ignore_case edge cases
    assert!(contains_ascii_ignore_case("", b""));
    assert!(contains_ascii_ignore_case("任意文本", b""));
    assert!(!contains_ascii_ignore_case("", b"stepx"));
    assert!(!contains_ascii_ignore_case("step", b"stepx"));
    assert!(contains_ascii_ignore_case("StEpX", b"stepx"));
    assert!(contains_ascii_ignore_case(
        "前缀中文[项目: StEpX-AuTo-42]后缀",
        "[项目: stepx-".as_bytes()
    ));
    assert!(!contains_ascii_ignore_case(
        "正常生产环境汇报",
        b"stepx-auto-"
    ));

    // 2. truncate_chars edge cases
    assert_eq!(truncate_chars("", 0), "");
    assert_eq!(truncate_chars("", 5), "");
    assert_eq!(truncate_chars("你好世界", 0), "…");
    assert_eq!(truncate_chars("你好世界", 4), "你好世界");
    assert_eq!(truncate_chars("你好世界", 3), "你好世…");
    assert_eq!(truncate_chars("A🚀B🎉C", 3), "A🚀B…");

    // 3. Synthetic probe detection
    assert!(is_synthetic_e2e_probe_text("任务 STEPX-E2E-2026 验证"));
    assert!(any_synthetic_e2e_probe(&[
        "正常标题",
        "含 StepX_Probe 标记"
    ]));
    assert!(!any_synthetic_e2e_probe(&[
        "微信",
        "张经理",
        "下午三点开会"
    ]));

    // 4. AppStore sub-tag matching & version comparison edge cases
    let catalog = crate::app_store::curated_catalog();
    assert!(!catalog.is_empty());
    let mut sample_app = catalog[0].clone();
    assert!(crate::app_store::app_matches_sub_tag(&sample_app, ""));
    assert!(crate::app_store::app_matches_sub_tag(
        &sample_app,
        "  全部 "
    ));
    // Mutating description to include NAS keyword must immediately reflect in sub-tag match
    sample_app.description = "支持 Synology 与私有云同步".to_string();
    assert!(crate::app_store::app_matches_sub_tag(
        &sample_app,
        "NAS 私有云"
    ));
    // Custom unknown sub-tag fallback
    assert!(crate::app_store::app_matches_sub_tag(
        &sample_app,
        "SYNOLOGY"
    ));
    assert!(!crate::app_store::app_matches_sub_tag(
        &sample_app,
        "不存在的冷门子标签xyz"
    ));

    assert_eq!(
        crate::app_store::compare_android_versions("v1.2.10", "1.2.9"),
        std::cmp::Ordering::Greater
    );
    assert_eq!(
        crate::app_store::compare_android_versions("2.0", "2.0.0"),
        std::cmp::Ordering::Equal
    );
    assert_eq!(
        crate::app_store::compare_android_versions("1.2.3.4.5.6.7.8", "1.2.3.4.5.6.7.9"),
        std::cmp::Ordering::Less
    );
}

#[test]
fn ui_render_path_consolidation_and_speedup_contract() {
    use std::hint::black_box;
    use std::time::Instant;

    // 1. theme::lerp_color boundary & transparent-endpoint edge cases
    let transparent = facloud_ui::color::ColorU::transparent_black();
    let accent = crate::ui::theme::rgb(0x3B, 0x82, 0xF6);
    assert_eq!(
        crate::ui::theme::lerp_color(transparent, accent, -0.5),
        transparent
    );
    assert_eq!(
        crate::ui::theme::lerp_color(transparent, accent, 0.0),
        transparent
    );
    assert_eq!(
        crate::ui::theme::lerp_color(transparent, accent, 1.0),
        accent
    );
    assert_eq!(
        crate::ui::theme::lerp_color(transparent, accent, 1.5),
        accent
    );
    let mid_from_trans = crate::ui::theme::lerp_color(transparent, accent, 0.5);
    assert_eq!(
        (mid_from_trans.r, mid_from_trans.g, mid_from_trans.b),
        (accent.r, accent.g, accent.b)
    );
    assert!(mid_from_trans.a > 120 && mid_from_trans.a < 135);
    let mid_to_trans = crate::ui::theme::lerp_color(accent, transparent, 0.5);
    assert_eq!(
        (mid_to_trans.r, mid_to_trans.g, mid_to_trans.b),
        (accent.r, accent.g, accent.b)
    );
    assert_eq!(mid_to_trans.a, mid_from_trans.a);

    // 2. extract_month_day_mentions_with_anchor empty / non-digit fast-path & boundary values
    assert!(Companion::extract_month_day_mentions_with_anchor("", 2026, 10).is_empty());
    assert!(Companion::extract_month_day_mentions_with_anchor(
        "纯中文无数字日程描述，测试无数字快速返回路径",
        2026,
        10
    )
    .is_empty());
    assert_eq!(
        Companion::extract_month_day_mentions_with_anchor("跨年活动 1月2日 举行", 2026, 12),
        vec![NaiveDate::from_ymd_opt(2027, 1, 2).unwrap()]
    );
    assert_eq!(
        Companion::extract_month_day_mentions_with_anchor("复盘 12月30日 记录", 2026, 1),
        vec![NaiveDate::from_ymd_opt(2025, 12, 30).unwrap()]
    );
    // Invalid calendar date (2026 is not a leap year) is rejected while subsequent valid date is kept
    assert_eq!(
        Companion::extract_month_day_mentions_with_anchor(
            "原定 2月29日 取消，改期到 3月1日 评审",
            2026,
            2
        ),
        vec![NaiveDate::from_ymd_opt(2026, 3, 1).unwrap()]
    );

    // 3. cached_file_modified_ms edge cases (missing file vs existing file)
    assert_eq!(
        crate::ui::widgets::cached_file_modified_ms(std::path::Path::new(
            "/nonexistent/stepx_missing_probe_file.bin"
        )),
        None
    );
    let cargo_manifest = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("Cargo.toml");
    let mod_ms_1 = crate::ui::widgets::cached_file_modified_ms(&cargo_manifest);
    let mod_ms_2 = crate::ui::widgets::cached_file_modified_ms(&cargo_manifest);
    assert!(mod_ms_1.is_some() && mod_ms_1 == mod_ms_2);

    // 4. Before vs. After microbenchmarks & consolidation contracts on Companion UI pages
    let mut app = Companion::new_for_test();
    app.home_tools_expanded = true;

    // Verify shared widget & tool helpers execute cleanly
    let p = app.p();
    let _ = black_box(app.surface_note_box("共享空白提示框验证", p.text2));
    let _ = black_box(app.status_banner_box(
        app.line("状态横幅验证", crate::ui::theme::CAPTION, p.text),
        p.warning_soft,
        p.warning,
    ));

    // Populate realistic installed apps so store_updatable_count / store_installed_count exercise real work
    for (i, cat_app) in app.app_store.catalog.iter().take(45).enumerate() {
        app.apps.push(stepx_companion::apps::AppInfo {
            package: cat_app.package.clone(),
            label: cat_app.name.clone(),
            version_name: if i % 3 == 0 {
                "0.0.1".to_string()
            } else {
                cat_app.version_name().to_string()
            },
            version_code: if i % 3 == 0 {
                1
            } else {
                cat_app.version_code().max(100)
            },
            system: false,
            updated_system: false,
            enabled: true,
            launchable: true,
            ..Default::default()
        });
    }

    // 4a. AppStore counts: uncached O(N*M) vs cached store_updatable_count + store_installed_count
    let t_store_uncached = Instant::now();
    for _ in 0..200 {
        let u = app
            .app_store
            .catalog
            .iter()
            .filter(|ca| {
                matches!(
                    crate::app_store::check_store_install_state(&app.apps, ca),
                    crate::app_store::StoreInstallState::Updatable { .. }
                )
            })
            .count();
        let inst = app
            .app_store
            .catalog
            .iter()
            .filter(|ca| app.apps.iter().any(|a| a.package == ca.package))
            .count();
        black_box((u, inst));
    }
    let store_counts_uncached_us = t_store_uncached.elapsed().as_nanos() as f64 / 200_000.0;

    let t_store_cached = Instant::now();
    for _ in 0..200 {
        let u = app.store_updatable_count();
        let inst = app.store_installed_count();
        black_box((u, inst));
    }
    let store_counts_cached_us = t_store_cached.elapsed().as_nanos() as f64 / 200_000.0;
    let initial_updatable = app.store_updatable_count();
    assert_eq!(app.store_installed_count(), 45);
    assert!(initial_updatable > 0);

    // Regression test: in-place mutation of an INTERIOR element of `app.apps` (index 21, which had i % 3 == 0)
    // without reallocating `app.apps` must immediately invalidate STORE_COUNTS_CACHE.
    let apps_ptr_before = app.apps.as_ptr();
    let target_cat_ver = app.app_store.catalog[21].version_name().to_string();
    let target_cat_code = app.app_store.catalog[21].version_code().max(100);
    app.apps[21].version_name = target_cat_ver;
    app.apps[21].version_code = target_cat_code;
    assert_eq!(app.apps.as_ptr(), apps_ptr_before);
    assert_eq!(app.store_updatable_count(), initial_updatable - 1);
    // Restore interior element in-place and verify count recovers immediately
    app.apps[21].version_name = "0.0.1".to_string();
    app.apps[21].version_code = 1;
    assert_eq!(app.store_updatable_count(), initial_updatable);

    // Regression test: in-place mutation of an INTERIOR catalog element via `mutate_app`
    let cat_ptr_before = app.app_store.catalog.as_ptr();
    let mid_pkg = app.app_store.catalog[22].package.clone();
    let orig_ver = app.app_store.catalog[22].sources[0].version_name.clone();
    app.app_store.mutate_app(&mid_pkg, |ca| {
        ca.sources[0].version_name = "999.99.9".to_string();
    });
    assert_eq!(app.app_store.catalog.as_ptr(), cat_ptr_before);
    assert_eq!(app.store_updatable_count(), initial_updatable + 1);
    app.app_store.mutate_app(&mid_pkg, |ca| {
        ca.sources[0].version_name = orig_ver.clone();
    });
    assert_eq!(app.store_updatable_count(), initial_updatable);

    // 4b. Home page render: legacy double-pass (with_home_report + build_home_panorama_and_weekly) vs single-pass home_page()
    app.demo_mode = true;
    app.init_demo_push();
    app.express_parcels = stepx_companion::device::default_express_parcels();
    app.back_screen.stepos_suite = stepx_companion::toolbox::default_stepos_ai_suite_report();
    let now_ms = chrono::Utc::now().timestamp_millis();
    for i in 0..24 {
        app.context
            .overview
            .items
            .push(stepx_companion::context_ledger::PreparedItem {
                id: format!("ledger-bench-{i}"),
                title: format!("跨端协同评审事项 #{i}"),
                status: if i % 4 == 0 {
                    "待确认".into()
                } else {
                    "推进中".into()
                },
                summary: format!("围绕 10月{}日 版本发布与性能回归验证展开讨论", (i % 28) + 1),
                updated_ms: now_ms - (i as i64) * 3_600_000,
                prompt: format!(
                    "<evidence>\n> [研发协同] 确认第 {i} 项模块的帧率与内存预算达标\n</evidence>"
                ),
                source_count: 2,
                source_objects: vec![stepx_companion::context_ledger::SourceObject {
                    id: format!("msg:{i}"),
                    source: "飞书".into(),
                    title: format!("架构组讨论 #{i}"),
                    ..Default::default()
                }],
                ..Default::default()
            });
    }
    let _ = black_box(app.home_page());

    let t_home_legacy = Instant::now();
    for _ in 0..50 {
        let metrics = app.cached_device_metrics();
        let el = app.with_home_report(|report, snapshot, express_parcels| {
            let (_panorama, weekly) = app.build_home_panorama_and_weekly(snapshot, express_parcels);
            let s1 = app.render_home_calendar_hub_and_panorama();
            let s2 = app.render_home_weekly_and_ai_row(report, &weekly);
            let s3 = app.render_home_modular_action_center(report, snapshot, express_parcels);
            let s4 = app.render_home_footer_device_dock(report, &metrics);
            let s5 = app.render_home_tools_summary_panel();
            app.vstack(S2, vec![s1, s2, s3, s4, s5])
        });
        black_box(el);
    }
    let home_legacy_us = t_home_legacy.elapsed().as_micros() as f64 / 50.0;

    let t_home_opt = Instant::now();
    for _ in 0..50 {
        black_box(app.home_page());
    }
    let home_opt_us = t_home_opt.elapsed().as_micros() as f64 / 50.0;

    // 4c. Amoo live mode (!demo_mode) vs demo mode render
    app.demo_mode = false;
    let t_amoo_live = Instant::now();
    for _ in 0..50 {
        black_box(app.amoo_page());
    }
    let amoo_live_us = t_amoo_live.elapsed().as_micros() as f64 / 50.0;

    app.demo_mode = true;
    let t_amoo_demo = Instant::now();
    for _ in 0..50 {
        black_box(app.amoo_page());
    }
    let amoo_demo_us = t_amoo_demo.elapsed().as_micros() as f64 / 50.0;

    // 4d. Schedule / Calendar page, Workspace page, and AppStore page render
    let t_sched = Instant::now();
    for _ in 0..50 {
        black_box(app.schedule_page());
    }
    let schedule_page_us = t_sched.elapsed().as_micros() as f64 / 50.0;

    let t_ws = Instant::now();
    for _ in 0..50 {
        black_box(app.workspace_page(false));
    }
    let workspace_page_us = t_ws.elapsed().as_micros() as f64 / 50.0;

    let t_store_page = Instant::now();
    for _ in 0..50 {
        black_box(app.app_store_page());
    }
    let app_store_page_us = t_store_page.elapsed().as_micros() as f64 / 50.0;

    eprintln!(
        "UI_CONSOLIDATION_BENCHMARK {}",
        serde_json::json!({
            "store_counts_uncached_us": (store_counts_uncached_us * 100.0).round() / 100.0,
            "store_counts_cached_us": (store_counts_cached_us * 1000.0).round() / 1000.0,
            "home_page_legacy_double_pass_us": (home_legacy_us * 10.0).round() / 10.0,
            "home_page_optimized_single_pass_us": (home_opt_us * 10.0).round() / 10.0,
            "amoo_page_live_us": (amoo_live_us * 10.0).round() / 10.0,
            "amoo_page_demo_us": (amoo_demo_us * 10.0).round() / 10.0,
            "schedule_page_us": (schedule_page_us * 10.0).round() / 10.0,
            "workspace_page_us": (workspace_page_us * 10.0).round() / 10.0,
            "app_store_page_us": (app_store_page_us * 10.0).round() / 10.0,
        })
    );
}

#[test]
fn home_resume_phone_amoo_and_collapsed_capsules_work() {
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();
    let item = stepx_companion::context_ledger::PreparedItem {
        id: "a1b2c3d4e5f60123456789abcdef".into(),
        title: "手机端技术架构评审".into(),
        status: "推进中".into(),
        summary: "讨论 MCP 跨端同步方案".into(),
        source_objects: vec![stepx_companion::context_ledger::SourceObject {
            id: "phone-amoo:s-88:topic:9999".into(),
            title: "手机端技术架构评审".into(),
            ..Default::default()
        }],
        ..Default::default()
    };
    let (primary_lbl, primary_act) = app.context_item_primary(&item);
    assert_eq!(primary_lbl, "在电脑继续对话");
    assert!(matches!(
        primary_act,
        Action::HomeResumeTask(ref t) if t == "phone-amoo:s-88"
    ));
    app.context.overview.items.push(item);

    let handled = app.apply_home_action(
        &Action::HomeResumeTask("phone-amoo:s-88:topic:9999".into()),
        None,
    );
    assert!(handled);
    assert!(!app.error);
    assert_eq!(app.page, Page::Amoo);
    let resumed_task = app
        .workspace
        .journal
        .tasks
        .iter()
        .find(|t| t.session.as_deref() == Some("s-88"))
        .expect("resumed phone-amoo task");
    assert_eq!(resumed_task.title, "手机端技术架构评审");

    let mut phone_work = stepx_companion::amoo_workspace::Task::new(None, None);
    phone_work.id = "phone-work-77".into();
    phone_work.title = "周会纪要整理".into();
    phone_work.route_note = "手机发起 · 桌面已完成，成果待回传手机".into();
    phone_work.note = "待回传手机 · 已回传 1/2 项成果".into();
    phone_work
        .steps
        .push(stepx_companion::amoo_workspace::Step {
            phase: stepx_companion::amoo_workspace::Phase::Running,
            executor: stepx_companion::amoo_workspace::Executor::Files,
            note: "已回传 1/2 项成果".into(),
        });

    app.workspace.journal.tasks.insert(0, phone_work);

    let ai_ctx = app.home_ai_context();
    assert!(!ai_ctx.ledger_items().is_empty());
    assert!(!ai_ctx.sms_cards().is_empty());
    assert!(ai_ctx
        .pending_phone_tasks()
        .iter()
        .any(|(t, n)| t == "周会纪要整理" && n.contains("待回传手机")));

    app.page = Page::Home;
    app.home_tools_expanded = false;
    app.back_screen.stepos_suite.phone_clipboard = "https://meet.feishu.cn/j/99887766".into();
    app.back_screen.stepos_suite_error = Some("读取 StepOS 套件超时".into());
    let _ = std::hint::black_box(app.home_page());

    // When clipboard equals an OTP, it is deduplicated in collapsed mode without panic
    app.back_screen.stepos_suite.phone_clipboard = "482910".into();
    let _ = std::hint::black_box(app.home_page());

    app.home_tools_expanded = true;
    app.context.show_all_items = true;
    let _ = std::hint::black_box(app.home_page());
}

#[test]
fn home_default_upgrade_keeps_every_existing_module_and_custom_size() {
    let old = vec![HomeModuleId::IssuesAndReplyModule, HomeModuleId::WeatherModule,
        HomeModuleId::RecentFiles, HomeModuleId::ProductivityHub,
        HomeModuleId::ContextLedgerHub, HomeModuleId::CalendarPanorama,
        HomeModuleId::WorkModule, HomeModuleId::LifeModule];
    let mut layout = HomeModuleLayoutConfig {
        enabled: old.clone(), preset: Some(HomeModulePreset::DefaultAll),
        sizes: Default::default(), ..Default::default()
    };
    layout.sizes.insert(HomeModuleId::WorkModule, HomeModuleSize {
        width: Some(HomeModuleWidth::TwoThirds), height: Some(HomeModuleHeight::Tall),
    });
    layout.sanitize();
    assert_eq!(layout.enabled.len(), HomeModuleId::ALL.len());
    assert_eq!(&layout.enabled[..old.len()], old.as_slice(), "upgrade cannot reorder existing cards");
    assert_eq!(layout.module_width(HomeModuleId::WorkModule), HomeModuleWidth::TwoThirds);
    assert_eq!(layout.module_height(HomeModuleId::WorkModule), HomeModuleHeight::Tall);
    let saved = layout.clone();
    layout.sanitize();
    assert_eq!(layout, saved, "migration must be idempotent");
}

#[test]
fn home_default_layout_keeps_all_functions_and_preserves_customizations() {
    let defaults = HomeModulePreset::DefaultAll.modules();
    assert_eq!(defaults.len(), HomeModuleId::ALL.len());
    for module in HomeModuleId::ALL { assert!(defaults.contains(&module)); }
    assert_eq!(defaults[0], HomeModuleId::CalendarPanorama);
    assert_eq!(defaults[1], HomeModuleId::RecentFiles);
    assert_eq!(defaults[2], HomeModuleId::IssuesAndReplyModule);
    assert_eq!(defaults[3], HomeModuleId::WorkModule);
    assert_eq!(defaults[4], HomeModuleId::ContinuityWorkspace);
    assert_eq!(defaults[5], HomeModuleId::QuickStartRequest);
    let old = vec![HomeModuleId::CalendarPanorama, HomeModuleId::IssuesAndReplyModule,
        HomeModuleId::WorkModule, HomeModuleId::RecentFiles, HomeModuleId::ContinuityWorkspace,
        HomeModuleId::QuickStartRequest, HomeModuleId::LifeModule, HomeModuleId::WeatherModule];
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("layout.json");
    let previous = HomeModuleLayoutConfig { enabled: old.clone(), ..Default::default() };
    std::fs::write(&path, serde_json::to_vec(&previous).unwrap()).unwrap();
    let mut app = Companion::new_for_test();
    assert!(app.load_home_modules_config_from(&path));
    assert_eq!(app.home_modules.enabled.len(), 16);
    assert_eq!(&app.home_modules.enabled[..old.len()], old.as_slice());
    assert!(app.home_modules.available_to_add().is_empty());
    // Hiding and reordering are user decisions, and survive subsequent loads.
    app.home_modules.remove_module(HomeModuleId::ContextLedgerHub);
    app.home_modules.move_to_top(HomeModuleId::AlarmsAndControl);
    app.home_modules.set_module_size(HomeModuleId::WorkModule,
        HomeModuleWidth::TwoThirds, HomeModuleHeight::Tall);
    let customized = app.home_modules.clone();
    app.save_home_modules_config_to(&path).unwrap();
    assert!(app.load_home_modules_config_from(&path));
    assert_eq!(app.home_modules, customized);
    assert!(app.home_modules.show_all_modules());
    assert_eq!(&app.home_modules.enabled[..customized.enabled.len()], customized.enabled.as_slice());
    assert_eq!(app.home_modules.sizes, customized.sizes, "show all must retain custom sizes");
}

#[test]
fn home_office_order_upgrades_only_exact_174_and_175_defaults() {
    let baseline = HomeModuleLayoutConfig::default();
    let order_174 = vec![HomeModuleId::CalendarPanorama, HomeModuleId::IssuesAndReplyModule,
        HomeModuleId::WorkModule, HomeModuleId::RecentFiles, HomeModuleId::ContinuityWorkspace,
        HomeModuleId::QuickStartRequest, HomeModuleId::ProductivityHub, HomeModuleId::ContextLedgerHub,
        HomeModuleId::WeeklyAndAi, HomeModuleId::LiveCountdown, HomeModuleId::AlarmsAndControl,
        HomeModuleId::ToolsSummaryDock, HomeModuleId::LifeModule, HomeModuleId::WeatherModule,
        HomeModuleId::BackScreenAndHabits, HomeModuleId::PulseAndKpi];
    let order_175 = vec![HomeModuleId::CalendarPanorama, HomeModuleId::RecentFiles,
        HomeModuleId::QuickStartRequest, HomeModuleId::IssuesAndReplyModule, HomeModuleId::WorkModule,
        HomeModuleId::ContinuityWorkspace, HomeModuleId::ProductivityHub, HomeModuleId::ContextLedgerHub,
        HomeModuleId::WeeklyAndAi, HomeModuleId::LiveCountdown, HomeModuleId::AlarmsAndControl,
        HomeModuleId::ToolsSummaryDock, HomeModuleId::LifeModule, HomeModuleId::WeatherModule,
        HomeModuleId::BackScreenAndHabits, HomeModuleId::PulseAndKpi];
    for (version, order) in [(174, order_174), (175, order_175)] {
        let mut old = HomeModuleLayoutConfig { enabled: order, ..baseline.clone() };
        if version == 175 {
            old.sizes.insert(HomeModuleId::RecentFiles, HomeModuleSize {
                width: Some(HomeModuleWidth::TwoThirds), height: Some(HomeModuleHeight::Compact),
            });
            old.sizes.insert(HomeModuleId::ContinuityWorkspace, HomeModuleSize {
                width: None, height: Some(HomeModuleHeight::Compact),
            });
        }
        let mut custom = old.clone();
        custom.sizes.insert(HomeModuleId::WorkModule, HomeModuleSize {
            width: Some(HomeModuleWidth::TwoThirds), height: Some(HomeModuleHeight::Tall),
        });
        let saved_order = custom.enabled.clone();
        let saved_sizes = custom.sizes.clone();
        custom.sanitize();
        assert_eq!(custom.enabled, saved_order, "{version}: custom order must remain");
        assert_eq!(custom.sizes, saved_sizes, "{version}: custom sizes must remain");
        assert_eq!(custom.preset, None);
        let mut hidden = old.clone();
        hidden.remove_module(HomeModuleId::WeatherModule);
        hidden.sanitize();
        assert!(!hidden.is_enabled(HomeModuleId::WeatherModule));
        old.sanitize();
        assert_eq!(old, baseline, "{version}: shipped default should adopt new scene order");
        old.sanitize();
        assert_eq!(old, baseline, "migration must be idempotent");
    }
    let rows = baseline.pack_rows();
    assert_eq!(rows[1].len(), 1, "recent files needs its own row for filenames and transfer");
    assert_eq!(rows[1][0].0, HomeModuleId::RecentFiles);
    assert_eq!(rows[2].iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        vec![HomeModuleId::IssuesAndReplyModule, HomeModuleId::WorkModule]);
    assert_eq!(rows[3].iter().map(|(id, _, _)| *id).collect::<Vec<_>>(),
        vec![HomeModuleId::ContinuityWorkspace, HomeModuleId::QuickStartRequest]);
}

#[test]
fn office_layout_is_calendar_first_compact_and_preserves_later_customization() {
    let layout = HomeModuleLayoutConfig::default();
    assert_eq!(layout.enabled.first(), Some(&HomeModuleId::CalendarPanorama));
    assert!(layout.is_enabled(HomeModuleId::RecentFiles));
    assert_eq!(layout.module_width(HomeModuleId::CalendarPanorama), HomeModuleWidth::Full);
    assert_eq!(layout.module_height(HomeModuleId::RecentFiles), HomeModuleHeight::Compact);
    assert_eq!(layout.module_width(HomeModuleId::WeatherModule), HomeModuleWidth::OneThird);
    assert_eq!(layout.module_height(HomeModuleId::WeatherModule), HomeModuleHeight::Compact);
    assert!(HomeModuleHeight::Compact.min_card_height().is_none());
    for row in layout.pack_rows() {
        assert_eq!(row.iter().map(|(_, width, _)| width.units() as u32).sum::<u32>(), 12,
            "default rows must not reserve blank filler columns");
    }
    for preset in HomeModulePreset::ALL {
        assert_eq!(preset.modules().first(), Some(&HomeModuleId::CalendarPanorama));
    }

    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("home.json");
    let mut custom = HomeModuleLayoutConfig {
        enabled: vec![HomeModuleId::WeatherModule, HomeModuleId::RecentFiles, HomeModuleId::WorkModule],
        sizes: Default::default(), preset: None, office_layout_introduced: false,
        ..Default::default()
    };
    custom.set_module_size(HomeModuleId::WorkModule, HomeModuleWidth::TwoThirds, HomeModuleHeight::Tall);
    let original_order = custom.enabled.clone();
    let original_sizes = custom.sizes.clone();
    let mut legacy = serde_json::to_value(custom).unwrap();
    legacy.as_object_mut().unwrap().remove("office_layout_introduced");
    std::fs::write(&path, serde_json::to_vec(&legacy).unwrap()).unwrap();
    let mut app = Companion::new_for_test();
    assert!(app.load_home_modules_config_from(&path));
    assert_eq!(app.home_modules.enabled, original_order);
    assert!(!app.home_modules.is_enabled(HomeModuleId::CalendarPanorama));
    assert_eq!(app.home_modules.sizes, original_sizes);
    assert!(app.home_modules.office_layout_introduced);
    // A later explicit reorder/hide is saved and must never be undone on reload.
    app.home_modules.move_to_top(HomeModuleId::RecentFiles);
    app.home_modules.remove_module(HomeModuleId::CalendarPanorama);
    let customized = app.home_modules.clone();
    app.save_home_modules_config_to(&path).unwrap();
    assert!(app.load_home_modules_config_from(&path));
    assert_eq!(app.home_modules, customized);
    assert_eq!(app.home_modules.enabled[0], HomeModuleId::RecentFiles);
}

#[test]
fn home_lone_custom_card_fills_row_without_changing_saved_span() {
    struct HomeOnly(Companion);
    impl facloud_ui::Entity for HomeOnly { type Event = (); }
    impl facloud_ui::View for HomeOnly {
        fn ui_name() -> &'static str { "home_lone_card_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El { self.0.home_page() }
    }
    impl facloud_ui::TypedActionView for HomeOnly { type Action = (); }

    facloud_ui::App::test((), |mut app| async move {
        for (viewport, paired) in [(1200., false), (390., false), (1200., true)] {
            let mut companion = Companion::new_for_test();
            companion.home_modules.enabled = vec![HomeModuleId::WeatherModule];
            companion.home_modules.set_module_width(HomeModuleId::WeatherModule, HomeModuleWidth::OneThird);
            if paired {
                companion.home_modules.enabled.push(HomeModuleId::LifeModule);
                companion.home_modules.set_module_width(HomeModuleId::LifeModule, HomeModuleWidth::TwoThirds);
            }
            let saved_layout = companion.home_modules.clone();
            let registry = companion.home_pointer_registry.clone();
            let (window_id, view) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| HomeOnly(companion));
            let root = app.root_view_id(window_id).unwrap();
            let width = app.update(move |ctx| {
                let mut presenter = facloud_ui::Presenter::new(window_id);
                presenter.invalidate(facloud_ui::WindowInvalidation {
                    updated: std::collections::HashSet::from([root]), ..Default::default()
                }, ctx);
                presenter.build_scene(vec2f(viewport, 1000.), 1., None, ctx);
                // App::test's platform window is always 1024px wide. Read this
                // explicit viewport's result before update flushes its own frame.
                let measured_width = registry.borrow().card_rects[&HomeModuleId::WeatherModule].width();
                measured_width
            });
            if viewport >= 760. && paired {
                let expected = layout_config::home_grid_span_width(viewport, S3, 4.);
                assert!((width - expected).abs() < 1., "actual 1/3 width, paired={paired}: {width}/{expected}");
            } else {
                assert!((width - viewport).abs() < 1., "lone card fills available row: {width}/{viewport}");
            }
            view.read(&app, |view, _| assert_eq!(view.0.home_modules, saved_layout));
        }
    });
}

#[test]
fn home_density_uses_current_canvas_space_and_never_promotes_for_background_arrivals() {
    use super::layout_config::{home_canvas_content_width, HOME_CONTENT_MAX_WIDTH};
    let mut app = Companion::new_for_test();
    app.page = Page::Home;
    app.workspace.viewport = vec2f(1000., 650.);
    // Includes the actual shell borders, sidebar, content padding and scroll track.
    assert_eq!(home_canvas_content_width(1000.), 722.);
    assert_eq!(home_canvas_content_width(3440.), HOME_CONTENT_MAX_WIDTH);
    for id in HomeModuleId::ALL {
        assert_eq!(app.home_module_available_width(id), 722., "narrow canvas stacks {id:?}");
        assert_eq!(app.resolve_effective_height(id, HomeModuleHeight::Auto), HomeModuleHeight::Compact);
        assert_eq!(app.resolve_effective_height(id, HomeModuleHeight::Standard), HomeModuleHeight::Standard);
        assert_eq!(app.resolve_effective_height(id, HomeModuleHeight::Tall), HomeModuleHeight::Tall);
    }
    app.workspace.viewport = vec2f(1440., 900.);
    let full = home_canvas_content_width(1440.);
    assert_eq!(app.home_module_available_width(HomeModuleId::RecentFiles), full);
    let continuation = app.home_module_available_width(HomeModuleId::ContinuityWorkspace);
    let quick = app.home_module_available_width(HomeModuleId::QuickStartRequest);
    assert!((continuation + quick + S3 - full).abs() < 0.1);
    assert!(((continuation + S3) / (quick + S3) - 2.).abs() < 0.01);
    app.init_demo_push();
    for id in HomeModuleId::ALL {
        assert_eq!(app.resolve_effective_height(id, HomeModuleHeight::Auto), HomeModuleHeight::Standard);
        assert!(app.compute_home_module_value_profile(id, HomeModuleWidth::Full, HomeModuleHeight::Auto).max_items > 0);
    }
    assert_eq!(app.home_module_content_width(HomeModuleId::QuickStartRequest), HomeModuleWidth::OneThird);
    // A lone card uses its row; the saved span survives for later pairing.
    app.home_modules.enabled = vec![HomeModuleId::QuickStartRequest];
    assert_eq!(app.home_module_available_width(HomeModuleId::QuickStartRequest), full);
    assert_eq!(app.home_module_content_width(HomeModuleId::QuickStartRequest), HomeModuleWidth::Full);
    assert_eq!(app.home_modules.module_width(HomeModuleId::QuickStartRequest), HomeModuleWidth::OneThird);
    assert!(HomeModuleHeight::Tall.min_card_height().is_none());
}

#[test]
fn home_width_budget_matches_packed_rows_for_custom_layouts_without_cached_rects() {
    use super::layout_config::home_canvas_content_width;
    let mut app = Companion::new_for_test();
    app.page = Page::Home;
    for offset in 0..HomeModuleId::ALL.len() {
        app.home_modules.enabled = HomeModuleId::ALL.into_iter().cycle().skip(offset).take(16).collect();
        for (index, id) in HomeModuleId::ALL.into_iter().enumerate() {
            app.home_modules.set_module_width(id, HomeModuleWidth::ALL[(index + offset) % 4]);
        }
        for viewport in [1000., 1100., 1290., 1320., 1440., 3440.] {
            app.workspace.viewport = vec2f(viewport, 900.);
            let content = home_canvas_content_width(viewport);
            for row in app.home_modules.pack_rows() {
                let stacked = content < if row.len() >= 3 { 1040. } else { 760. };
                for (id, width, _) in &row {
                    let expected = if stacked || row.len() == 1 { content } else {
                        layout_config::home_grid_span_width(content, S3, width.flex_weight())
                    };
                    assert!((app.home_module_available_width(*id) - expected).abs() < 0.01,
                        "packing mismatch at viewport {viewport}, row {row:?}, module {id:?}");
                }
            }
        }
    }
}

#[test]
fn home_reply_targets_require_background_routes_and_never_retarget_stale_drafts() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    let mut snapshot = PushSnapshot::default();
    snapshot.threads = vec![
        stepx_companion::push::ImThread { thread_id: "installer:done".into(), package_name: "com.stepos.appstore".into(),
            title: "安装成功".into(), last_message: "叮咚买菜".into(), unread_count: 1, ..Default::default() },
        stepx_companion::push::ImThread { thread_id: "xhs:promo".into(), package_name: "com.xingin.xhs".into(),
            title: "小红书".into(), last_message: "赏秋攻略".into(), unread_count: 1, can_ui_reply: true, ..Default::default() },
    ];
    assert!(app.resolve_home_actionable_reply_target(&snapshot).is_none());
    snapshot.threads.push(stepx_companion::push::ImThread { thread_id: "wechat:team".into(), package_name: "com.tencent.mm".into(),
        title: "协作群".into(), last_message: "请确认会议时间".into(), unread_count: 1,
        can_remote_reply: true, notification_key: "real-reply-handle".into(), ..Default::default() });
    assert_eq!(app.resolve_home_actionable_reply_target(&snapshot).unwrap().1, "wechat:team");
    app.push.selected_im_thread = Some("installer:done".into());
    assert!(app.resolve_home_actionable_reply_target(&snapshot).is_none(), "invalid explicit target cannot redirect to another group");
    app.push.selected_im_thread = Some("wechat:team".into());
    assert_eq!(app.resolve_home_actionable_reply_target(&snapshot).unwrap().0, "real-reply-handle");
    // A notification can carry a real route before its thread is consolidated.
    snapshot.threads.clear();
    snapshot.notifications.push(stepx_companion::push::PushNotification {
        key: "notification-reply".into(), thread_id: "wechat:team".into(), package_name: "com.tencent.mm".into(),
        title: "协作群".into(), can_reply: true, ..Default::default()
    });
    assert_eq!(app.resolve_home_actionable_reply_target(&snapshot).unwrap().0, "notification-reply");
    snapshot.notifications[0].can_reply = false;
    assert!(app.resolve_home_actionable_reply_target(&snapshot).is_none());
    app.push.selected_im_thread = Some("sms:10086".into());
    assert_eq!(app.resolve_home_actionable_reply_target(&snapshot).unwrap().2, "com.android.mms");
}

#[test]
fn home_install_notifications_do_not_expand_the_pending_card() {
    struct PendingOnly(Companion);
    impl facloud_ui::Entity for PendingOnly { type Event = (); }
    impl facloud_ui::View for PendingOnly {
        fn ui_name() -> &'static str { "home_pending_noise_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El { self.0.home_page() }
    }
    impl facloud_ui::TypedActionView for PendingOnly { type Action = (); }
    facloud_ui::App::test((), |mut app| async move {
        let mut heights = Vec::new();
        for count in [0, 20] {
            let mut companion = Companion::new_for_test();
            companion.demo_mode = false;
            companion.selected = None;
            companion.home_modules.enabled = vec![HomeModuleId::IssuesAndReplyModule];
            companion.home_modules.set_module_size(HomeModuleId::IssuesAndReplyModule,
                HomeModuleWidth::Full, HomeModuleHeight::Standard);
            companion.push.snapshot.threads = (0..count).map(|i| stepx_companion::push::ImThread {
                thread_id: format!("installer:{i}"), package_name: "com.stepos.appstore".into(),
                title: "安装成功".into(), last_message: "叮咚买菜".into(), unread_count: 1,
                ..Default::default()
            }).collect();
            let registry = companion.home_pointer_registry.clone();
            let (window_id, _view) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| PendingOnly(companion));
            let root = app.root_view_id(window_id).unwrap();
            let height = app.update(move |ctx| {
                let mut presenter = facloud_ui::Presenter::new(window_id);
                presenter.invalidate(facloud_ui::WindowInvalidation {
                    updated: std::collections::HashSet::from([root]), ..Default::default()
                }, ctx);
                presenter.build_scene(vec2f(800., 1000.), 1., None, ctx);
                let height = registry.borrow().card_rects[&HomeModuleId::IssuesAndReplyModule].height();
                height
            });
            assert!(height < 200., "noise must leave only the compact heading and empty-state hint: {height}");
            heights.push(height);
        }
        assert!((heights[0] - heights[1]).abs() < 1., "installation notifications cannot inflate the pending card: {heights:?}");
    });
}

#[test]
fn home_reply_enter_without_background_route_preserves_the_draft() {
    facloud_ui::App::test((), |mut app| async move {
        let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus, |_| Companion::new_for_test());
        owner.update(&mut app, |view, ctx| {
            view.demo_mode = false;
            view.selected = None;
            view.page = Page::Home;
            view.push.snapshot = PushSnapshot::default();
            view.push.selected_im_thread = Some("installer:done".into());
            view.focus = Some(F_PUSH_REPLY);
            view.fields[F_PUSH_REPLY].set("保留这条草稿");
            facloud_ui::TypedActionView::handle_action(view, &Action::SubmitFocused, ctx);
            assert_eq!(view.fields[F_PUSH_REPLY].text, "保留这条草稿");
            assert!(view.error && view.status.contains("草稿已保留"));
            assert_eq!(view.push.selected_im_thread.as_deref(), Some("installer:done"));
        });
    });
}

#[test]
fn home_toolbar_weather_distinguishes_cold_live_and_demo_data() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    app.weather_snapshot.from_live_api = false;
    assert_eq!(app.home_toolbar_weather_label(), "天气待更新");
    app.weather_snapshot.from_live_api = true;
    assert!(app.home_toolbar_weather_label().contains("°C"));
    app.weather_snapshot.from_live_api = false;
    app.demo_mode = true;
    assert!(app.home_toolbar_weather_label().contains("°C"));
}

#[test]
fn home_reorder_motion_only_starts_for_layout_edits_and_honors_reduced_motion() {
    let mut registry = HomePointerRegistry::default();
    let signature = vec![
        (HomeModuleId::CalendarPanorama, HomeModuleWidth::Full, HomeModuleHeight::Standard),
        (HomeModuleId::RecentFiles, HomeModuleWidth::Full, HomeModuleHeight::Compact),
    ];
    registry.card_rects.insert(HomeModuleId::RecentFiles, RectF::new(vec2f(0., 400.), vec2f(800., 180.)));
    registry.observe_layout(signature.clone(), true);
    assert!(registry.reflow_started.is_none(), "initial load is steady");
    registry.card_rects.insert(HomeModuleId::RecentFiles, RectF::new(vec2f(0., 410.), vec2f(800., 184.)));
    registry.observe_layout(signature.clone(), true);
    assert!(registry.reflow_started.is_none(), "new content must not start a layout animation");
    let reordered = signature.into_iter().rev().collect::<Vec<_>>();
    registry.observe_layout(reordered.clone(), true);
    let started = registry.reflow_started.expect("explicit reorder animates");
    assert_eq!(registry.reflow_from[&HomeModuleId::RecentFiles].origin().y(), 410.);
    registry.observe_layout(reordered.clone(), true);
    assert_eq!(registry.reflow_started, Some(started), "refresh never restarts movement");
    registry.observe_layout(reordered, false);
    assert!(registry.reflow_started.is_none() && registry.reflow_from.is_empty());
}

#[test]
fn home_modular_architecture_editor_and_persistence_contract() {
    // 1. Verify complete 16-module inventory and metadata
    assert_eq!(HomeModuleId::ALL.len(), 16);
    assert_eq!(HomeModuleCategory::ALL.len(), 5);
    for m in HomeModuleId::ALL {
        assert!(!m.slug().is_empty());
        assert!(!m.title().is_empty());
        assert!(!m.short_title().is_empty());
        assert!(!m.subtitle().is_empty());
        assert!(!m.category().label().is_empty());
    }

    // 2. All existing functions remain visible until the user hides them.
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();
    assert_eq!(app.home_modules.preset, Some(HomeModulePreset::DefaultAll));
    assert_eq!(app.home_modules.enabled.len(), 16);
    assert!(app.home_modules.available_to_add().is_empty());
    assert!(app.home_modules.is_enabled(HomeModuleId::ProductivityHub));
    assert!(app
        .home_modules
        .is_enabled(HomeModuleId::IssuesAndReplyModule));
    assert!(app.home_modules.is_enabled(HomeModuleId::WeatherModule));
    assert!(app.home_modules.is_enabled(HomeModuleId::ContinuityWorkspace));
    assert!(app.home_modules.is_enabled(HomeModuleId::QuickStartRequest));
    assert!(app.home_modules.is_enabled(HomeModuleId::ContextLedgerHub));

    // 3. Test ToggleEditMode & Category Filtering
    assert!(!app.home_module_edit_mode);
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ToggleEditMode), None));
    assert!(app.home_module_edit_mode);
    let _ = std::hint::black_box(app.home_page());

    for cat in HomeModuleCategory::ALL {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::FilterCategory(Some(cat))),
            None
        ));
        assert_eq!(app.home_module_filter, Some(cat));
        let _ = std::hint::black_box(app.home_page());
    }
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::FilterCategory(None)),
        None
    ));
    assert_eq!(app.home_module_filter, None);

    // 4. Hiding a module keeps it in the library; adding it restores it.
    assert!(app.home_modules.remove_module(HomeModuleId::ProductivityHub));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::AddModule(
            HomeModuleId::ProductivityHub
        )),
        None
    ));
    assert_eq!(app.home_modules.enabled.len(), 16);
    assert_eq!(
        app.home_modules.enabled.last(),
        Some(&HomeModuleId::ProductivityHub)
    );
    assert_eq!(app.home_modules.preset, None);
    // Adding duplicate module is idempotent
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::AddModule(
            HomeModuleId::ProductivityHub
        )),
        None
    ));
    assert_eq!(app.home_modules.enabled.len(), 16);

    // 5. Test Reordering Modules (置顶 / 上移 / 下移 / 置底 / 指定位置)
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveToTop(
            HomeModuleId::ProductivityHub
        )),
        None
    ));
    assert_eq!(
        app.home_modules.enabled[0],
        HomeModuleId::ProductivityHub
    );
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveDown(
            HomeModuleId::ProductivityHub
        )),
        None
    ));
    assert_eq!(
        app.home_modules.enabled[1],
        HomeModuleId::ProductivityHub
    );
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveUp(HomeModuleId::ProductivityHub)),
        None
    ));
    assert_eq!(
        app.home_modules.enabled[0],
        HomeModuleId::ProductivityHub
    );
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveToBottom(
            HomeModuleId::ProductivityHub
        )),
        None
    ));
    assert_eq!(
        app.home_modules.enabled.last(),
        Some(&HomeModuleId::ProductivityHub)
    );
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveToIndex(
            HomeModuleId::IssuesAndReplyModule,
            0
        )),
        None
    ));
    assert_eq!(
        app.home_modules.enabled[0],
        HomeModuleId::IssuesAndReplyModule
    );
    let _ = std::hint::black_box(app.home_page());

    // 6. Test Removing Modules (减少/隐藏模块) and Minimum-1-Module Guard
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::RemoveModule(HomeModuleId::LifeModule)),
        None
    ));
    assert!(!app.home_modules.is_enabled(HomeModuleId::LifeModule));
    assert!(app
        .home_modules
        .available_to_add()
        .contains(&HomeModuleId::LifeModule));

    // Remove until only 1 non-SVG module (`WorkModule`) remains
    app.home_modules.enabled = vec![HomeModuleId::WorkModule, HomeModuleId::PulseAndKpi];
    assert!(app.page_requires_svg());
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::RemoveModule(HomeModuleId::PulseAndKpi)),
        None
    ));
    assert_eq!(app.home_modules.enabled, vec![HomeModuleId::WorkModule]);
    assert!(!app.page_requires_svg());
    // Attempting to remove the last remaining module must be rejected safely
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::RemoveModule(HomeModuleId::WorkModule)),
        None
    ));
    assert!(app.error);
    assert_eq!(app.home_modules.enabled, vec![HomeModuleId::WorkModule]);
    let _ = std::hint::black_box(app.home_page());

    // 7. Test All 5 Presets & ResetDefault + Per-Preset Frame Timing
    app.home_module_edit_mode = false;
    let mut preset_metrics = serde_json::Map::new();
    for preset in HomeModulePreset::ALL {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::ApplyPreset(preset)),
            None
        ));
        assert_eq!(app.home_modules.preset, Some(preset));
        assert_eq!(app.home_modules.enabled, preset.modules());
        let _ = std::hint::black_box(app.home_page());
        let t0 = std::time::Instant::now();
        for _ in 0..10 {
            let _ = std::hint::black_box(app.home_page());
        }
        let avg_us = (t0.elapsed().as_secs_f64() * 1_000_000.0) / 10.0;
        preset_metrics.insert(
            format!("{:?}_us", preset),
            serde_json::json!((avg_us * 10.0).round() / 10.0),
        );
    }
    println!(
        "HOME_MODULAR_PRESET_BENCHMARK {}",
        serde_json::Value::Object(preset_metrics)
    );
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ResetDefault), None));
    assert_eq!(app.home_modules.preset, Some(HomeModulePreset::DefaultAll));
    assert_eq!(app.home_modules.enabled.len(), 16);

    // 7b. Test Free Width, Free Height, Free Show/Hide & Cross-Category Free Combination (`pack_rows`)
    assert_eq!(
        app.home_modules
            .module_width(HomeModuleId::CalendarPanorama),
        HomeModuleWidth::Full
    );
    assert_eq!(
        app.home_modules.module_width(HomeModuleId::WorkModule),
        HomeModuleWidth::OneHalf
    );
    // Set CalendarPanorama (Schedule category) to 1/2 width and PulseAndKpi (Device category) to 1/2 width,
    // and place them next to each other — verify pack_rows combines them into a single 2-column row!
    app.home_modules.enabled = vec![
        HomeModuleId::CalendarPanorama,
        HomeModuleId::PulseAndKpi,
        HomeModuleId::WorkModule,
        HomeModuleId::BackScreenAndHabits,
        HomeModuleId::WeeklyAndAi,
    ];
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetModuleWidth(
            HomeModuleId::CalendarPanorama,
            HomeModuleWidth::OneHalf,
        )),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetModuleWidth(
            HomeModuleId::PulseAndKpi,
            HomeModuleWidth::OneHalf,
        )),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetModuleWidth(
            HomeModuleId::WorkModule,
            HomeModuleWidth::TwoThirds,
        )),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetModuleWidth(
            HomeModuleId::BackScreenAndHabits,
            HomeModuleWidth::OneThird,
        )),
        None,
    ));
    let packed = app.home_modules.pack_rows();
    assert_eq!(packed.len(), 3);
    assert_eq!(
        packed[0],
        vec![
            (
                HomeModuleId::CalendarPanorama,
                HomeModuleWidth::OneHalf,
                HomeModuleHeight::Standard
            ),
            (
                HomeModuleId::PulseAndKpi,
                HomeModuleWidth::OneHalf,
                HomeModuleHeight::Standard
            ),
        ]
    );
    assert_eq!(
        packed[1],
        vec![
            (
                HomeModuleId::WorkModule,
                HomeModuleWidth::TwoThirds,
                HomeModuleHeight::Standard
            ),
            (
                HomeModuleId::BackScreenAndHabits,
                HomeModuleWidth::OneThird,
                HomeModuleHeight::Standard
            ),
        ]
    );
    assert_eq!(
        packed[2],
        vec![(
            HomeModuleId::WeeklyAndAi,
            HomeModuleWidth::Full,
            HomeModuleHeight::Standard
        )]
    );

    // Cycle width & height and verify item_limit / min_height_px
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetModuleHeight(
            HomeModuleId::WorkModule,
            HomeModuleHeight::Compact,
        )),
        None,
    ));
    assert_eq!(
        app.home_modules.module_height(HomeModuleId::WorkModule),
        HomeModuleHeight::Compact
    );
    assert_eq!(HomeModuleHeight::Compact.item_limit(4), 2);
    assert_eq!(HomeModuleHeight::Tall.item_limit(4), 6);
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::CycleModuleHeight(
            HomeModuleId::WorkModule
        )),
        None,
    ));
    assert_eq!(
        app.home_modules.module_height(HomeModuleId::WorkModule),
        HomeModuleHeight::Standard
    );
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::CycleModuleWidth(HomeModuleId::WorkModule)),
        None,
    ));
    assert_eq!(
        app.home_modules.module_width(HomeModuleId::WorkModule),
        HomeModuleWidth::Full
    );

    // ToggleModule, SetCategoryEnabled, ShowAllModules, and verify all 14 modules render across all HomeModuleHeight variants
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ShowAllModules), None,));
    assert_eq!(app.home_modules.enabled.len(), HomeModuleId::ALL.len());
    for height in HomeModuleHeight::ALL {
        for m in HomeModuleId::ALL {
            app.home_modules.set_module_height(m, height);
        }
        let _ = std::hint::black_box(app.home_page());
    }
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetCategoryEnabled(
            HomeModuleCategory::DeviceAndTelemetry,
            false,
        )),
        None,
    ));
    assert!(!app.home_modules.is_enabled(HomeModuleId::PulseAndKpi));
    assert!(!app.home_modules.is_enabled(HomeModuleId::AlarmsAndControl));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ToggleModule(HomeModuleId::PulseAndKpi)),
        None,
    ));
    assert!(app.home_modules.is_enabled(HomeModuleId::PulseAndKpi));

    // 8. Test JSON Persistence Round-Trip & Sanitize Resilience (including custom width & height)
    let tmp_path = std::env::temp_dir().join(format!(
        "stepx-home-modules-test-{}.json",
        std::process::id()
    ));
    app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ApplyPreset(HomeModulePreset::WorkFocus)),
        None,
    );
    app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveToTop(HomeModuleId::ContextLedgerHub)),
        None,
    );
    app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetModuleWidth(
            HomeModuleId::ContextLedgerHub,
            HomeModuleWidth::TwoThirds,
        )),
        None,
    );
    app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SetModuleHeight(
            HomeModuleId::ContextLedgerHub,
            HomeModuleHeight::Tall,
        )),
        None,
    );
    app.save_home_modules_config_to(&tmp_path)
        .expect("save home modules config");
    let saved_order = app.home_modules.enabled.clone();

    let mut loaded_app = Companion::new_for_test();
    assert!(loaded_app.load_home_modules_config_from(&tmp_path));
    assert_eq!(loaded_app.home_modules.enabled, saved_order);
    assert_eq!(loaded_app.home_modules.preset, None);
    assert_eq!(
        loaded_app
            .home_modules
            .module_width(HomeModuleId::ContextLedgerHub),
        HomeModuleWidth::TwoThirds
    );
    assert_eq!(
        loaded_app
            .home_modules
            .module_height(HomeModuleId::ContextLedgerHub),
        HomeModuleHeight::Tall
    );

    // Corrupt JSON or empty module list should be handled gracefully
    std::fs::write(&tmp_path, r#"{"enabled":[],"preset":null}"#).unwrap();
    assert!(loaded_app.load_home_modules_config_from(&tmp_path));
    assert_eq!(
        loaded_app.home_modules.enabled,
        HomeModulePreset::DefaultAll.modules()
    );
    std::fs::write(&tmp_path, "not-valid-json").unwrap();
    assert!(!loaded_app.load_home_modules_config_from(&tmp_path));
    let _ = std::fs::remove_file(&tmp_path);

    // 9. Verify ContextLedgerHub SVG gating and interleaved module orderings
    assert!(HomeModuleId::ContextLedgerHub.uses_svg());
    app.home_modules.enabled = vec![HomeModuleId::WorkModule, HomeModuleId::ContextLedgerHub];
    assert!(app.page_requires_svg());
    app.home_modules.enabled = vec![
        HomeModuleId::WorkModule,
        HomeModuleId::CalendarPanorama,
        HomeModuleId::LifeModule,
        HomeModuleId::PulseAndKpi,
        HomeModuleId::WeeklyAndAi,
        HomeModuleId::AlarmsAndControl,
    ];
    app.home_module_edit_mode = true;
    let _ = std::hint::black_box(app.home_page());
    app.home_module_edit_mode = false;
    let _ = std::hint::black_box(app.home_page());

    // 10. Verify in-place mutation of interior Task step phase / output name / ledger presentation
    // immediately invalidates HOME_DESKTOP_ENTRIES_CACHE and HOME_ENRICHED_REPORT_CACHE
    let mut t = stepx_companion::amoo_workspace::Task::new(None, None);
    t.id = "cache-probe-task".into();
    t.title = "缓存内联突变验证任务".into();
    t.updated = chrono::Utc::now().timestamp();
    t.steps.push(stepx_companion::amoo_workspace::Step {
        phase: stepx_companion::amoo_workspace::Phase::Running,
        executor: stepx_companion::amoo_workspace::Executor::Files,
        note: "正在执行".into(),
    });
    t.outputs.push(stepx_companion::amoo_workspace::Output {
        name: "draft_v1.md".into(),
        path: std::path::PathBuf::from("/tmp/draft_v1.md"),
        kind: "文档".into(),
        lab_id: None,
    });
    app.workspace.journal.tasks.push(t);
    let entries_before = app.home_desktop_date_entries();
    let probe_before = entries_before
        .iter()
        .find(|e| e.id == "task-cache-probe-task")
        .expect("probe task entry");
    assert_eq!(
        probe_before.kind,
        stepx_companion::home_hub::SelectedDateEntryKind::WorkAndMatter
    );
    assert!(probe_before.detail.contains("draft_v1.md"));

    // Mutate step phase and output name in-place without changing vec lengths or task.updated
    let last_task = app.workspace.journal.tasks.last_mut().unwrap();
    last_task.steps[0].phase = stepx_companion::amoo_workspace::Phase::Failed;
    last_task.outputs[0].name = "final_v2.md".into();

    let entries_after = app.home_desktop_date_entries();
    let probe_after = entries_after
        .iter()
        .find(|e| e.id == "task-cache-probe-task")
        .expect("probe task entry after in-place mutation");
    assert_eq!(
        probe_after.kind,
        stepx_companion::home_hub::SelectedDateEntryKind::Issue
    );
    assert_eq!(probe_after.badge, "需重试");
    assert!(probe_after.detail.contains("final_v2.md"));
}

#[test]
fn home_real_mode_interactions_and_frame_budget_regression() {
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    app.page = Page::Home;

    // 1. Preset changes leave the focused context view and preserve the tools disclosure.
    app.home_tools_expanded = false;
    app.handle_context_action(&crate::ui::context_state::ContextAction::Manage);
    assert!(app.context_home_is_focused());
    // Even while context_home_is_focused() is true, home_page() must render without panic (with the modular toolbar strip)
    let _ = std::hint::black_box(app.home_page());

    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ApplyPreset(HomeModulePreset::WorkFocus)),
        None,
    ));
    assert!(!app.home_tools_expanded);
    assert!(!app.context_home_is_focused());
    assert_eq!(app.home_modules.preset, Some(HomeModulePreset::WorkFocus));

    // 2. No reply route means no invented recipient. Service notifications
    // and missed calls cannot become reply targets; explicit SMS keeps its bridge.
    let mut noisy_snap = stepx_companion::push::PushSnapshot::default();
    noisy_snap.threads.push(stepx_companion::push::ImThread {
        thread_id: "com.tencent.mm::微信支付".into(),
        package_name: "com.tencent.mm".into(),
        app_name: "微信".into(),
        title: "微信支付".into(),
        last_message: "已支付¥36.90".into(),
        last_timestamp_ms: 1775021000000,
        unread_count: 0,
        ..Default::default()
    });
    noisy_snap.threads.push(stepx_companion::push::ImThread {
        thread_id: "com.tencent.mm::搜索 账号/手机号".into(),
        package_name: "com.tencent.mm".into(),
        app_name: "微信".into(),
        title: "搜索 账号/手机号".into(),
        last_message: "搜索 账号/手机号".into(),
        last_timestamp_ms: 0,
        unread_count: 0,
        ..Default::default()
    });
    noisy_snap.threads.push(stepx_companion::push::ImThread {
        thread_id: "com.android.server.telecom::138 0000 6424".into(),
        package_name: "com.android.server.telecom".into(),
        app_name: "更多社交".into(),
        title: "138 0000 6424".into(),
        last_message: "上海 移动".into(),
        last_timestamp_ms: 1775021100000,
        unread_count: 1,
        ..Default::default()
    });
    assert!(app.resolve_home_actionable_reply_target(&noisy_snap).is_none());
    assert_eq!(app.resolve_home_reply_target(&noisy_snap), Default::default());

    // Also verify parse_dumpsys_notifications does not turn telecom, Dianping, or arbitrary non-IM notifications into fake unread IM threads
    let telecom_and_dianping_dumpsys = r#"
  NotificationRecord(0x01: pkg=com.android.server.telecom user=UserHandle{0} id=1 tag=missed_call importance=3 key=0|com.android.server.telecom|1|missed_call|1000: Notification(channel=phone_missed_call flags=0x10 category=missed_call))
    extras={
      android.title=String (138 0000 6424)
      android.text=String (上海 移动)
    }
    postTime=1775021100000
  NotificationRecord(0x02: pkg=com.dianping.v1 user=UserHandle{0} id=2 tag=push importance=3 key=0|com.dianping.v1|2|push|10200: Notification(channel=push flags=0x10))
    extras={
      android.title=String (商家已接单)
      android.text=String (请耐心等待配送)
    }
    postTime=1775021200000
  NotificationRecord(0x03: pkg=com.example.weather user=UserHandle{0} id=3 tag=push importance=3 key=0|com.example.weather|3|push|10201: Notification(channel=push flags=0x10))
    extras={
      android.title=String (今日天气晴)
      android.text=String (气温 18-24℃)
    }
    postTime=1775021300000
"#;
    let (dump_notifs, dump_threads) =
        stepx_companion::push::parse_dumpsys_notifications(telecom_and_dianping_dumpsys);
    assert_eq!(dump_notifs.len(), 2);
    assert!(dump_notifs.iter().any(|n| n.category == "other"));
    assert!(
        dump_threads.is_empty(),
        "Missed calls and lifestyle notifications must not be turned into fake unread IM threads"
    );

    let empty_snap = stepx_companion::push::PushSnapshot::default();
    app.sms.push(stepx_companion::sms::Message {
        provider_id: Some(1),
        address: "10086".into(),
        body: "您本月的流量剩余 15GB".into(),
        date_ms: chrono::Utc::now().timestamp_millis(),
        outgoing: false,
        kind: 1,
        read: false,
        subscription: Some(1),
    });
    assert!(app.apply_home_action(
        &Action::HomeSelectReplyTarget {
            key: "sms:10086".into(),
            thread_id: "sms:10086".into(),
            package_name: "com.android.mms".into(),
            title: "10086".into(),
        },
        None,
    ));
    let (sms_key, sms_tid, sms_pkg, sms_title) = app.resolve_home_reply_target(&empty_snap);
    assert_eq!(sms_key, "sms:10086");
    assert_eq!(sms_tid, "sms:10086");
    assert_eq!(sms_pkg, "com.android.mms");
    assert_eq!(sms_title, "10086");

    // 3. Exercise every preset and its cached render path in all build profiles.
    // Wall-clock budgets belong to the isolated release probe below, not a
    // parallel, unoptimized interaction suite competing with other CI tests.
    for preset in HomeModulePreset::ALL {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::ApplyPreset(preset)),
            None,
        ));
        assert_eq!(app.home_modules.preset, Some(preset));
        for _ in 0..2 {
            let _ = std::hint::black_box(app.home_page());
        }
    }

    // 4. Emoji-prefixed backscreen signatures are sanitized and inline IM reply stays on Page::Home
    app.fields[F_BACK_MARQUEE].clear();
    app.back_screen.stepos_suite.backscreen_signature = "📲30分钟计".into();
    let clean_sig =
        app.with_home_report(|report, _, _| app.current_backscreen_signature(report).to_string());
    assert_eq!(clean_sig, "30分钟计");

    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus("📲 专注中".into())),
        None,
    ));
    assert_eq!(app.fields[F_BACK_MARQUEE].text, "专注中");
    assert_eq!(app.back_screen.stepos_suite.backscreen_signature, "专注中");

    app.page = Page::Home;
    app.demo_mode = true;
    assert!(app.apply_home_action(
        &Action::HomeQuickImReply {
            key: "home-default-reply".into(),
            thread_id: "im:com.tencent.mm:文件传输助手".into(),
            package_name: "com.tencent.mm".into(),
            title: "文件传输助手".into(),
            text: "收到，稍后处理".into(),
        },
        None,
    ));
    assert_eq!(app.page, Page::Home);
    assert!(app
        .push
        .snapshot
        .threads
        .iter()
        .any(|t| t.title == "文件传输助手" && t.last_message == "收到，稍后处理"));

    assert!(stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "StepX-Transfer-Probe-1774871580.txt"
    ));
    assert!(stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "StepX-待办-快递与取件线索 · SF14298019283-1774871580970.md"
    ));
    assert!(stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "真机端到端回归日程"
    ));
    assert!(stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "真机端到端测试闹钟"
    ));
    assert!(stepx_companion::home_hub::is_synthetic_e2e_probe_text(
        "SF1357924680123"
    ));
}

#[test]
fn home_e2e_click_all_14_modules_and_layout_actions_contract() {
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();
    app.page = Page::Home;

    // 1. Layout Editor Studio: exercise all 5 presets, ToggleEditMode, SetModuleWidth, SetModuleHeight,
    //    MoveUp/MoveDown/MoveToTop/MoveToBottom, AddModule/RemoveModule, SetCategoryEnabled
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ToggleEditMode), None,));
    assert!(app.home_module_edit_mode);
    let _ = std::hint::black_box(app.home_page());

    for preset in HomeModulePreset::ALL {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::ApplyPreset(preset)),
            None,
        ));
        assert_eq!(app.home_modules.preset, Some(preset));
        let _ = std::hint::black_box(app.home_page());
    }

    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ShowAllModules), None,));
    assert_eq!(app.home_modules.enabled.len(), 16);

    for width in HomeModuleWidth::ALL {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::SetModuleWidth(
                HomeModuleId::CalendarPanorama,
                width,
            )),
            None,
        ));
        assert_eq!(
            app.home_modules
                .module_width(HomeModuleId::CalendarPanorama),
            width
        );
    }
    for height in HomeModuleHeight::ALL {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::SetModuleHeight(
                HomeModuleId::CalendarPanorama,
                height,
            )),
            None,
        ));
        assert_eq!(
            app.home_modules
                .module_height(HomeModuleId::CalendarPanorama),
            height
        );
    }

    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveToBottom(
            HomeModuleId::ProductivityHub
        )),
        None,
    ));
    assert_eq!(
        app.home_modules.enabled.last(),
        Some(&HomeModuleId::ProductivityHub)
    );
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveUp(HomeModuleId::ProductivityHub)),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveDown(HomeModuleId::ProductivityHub)),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveToTop(HomeModuleId::ProductivityHub)),
        None,
    ));
    assert_eq!(
        app.home_modules.enabled.first(),
        Some(&HomeModuleId::ProductivityHub)
    );

    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::RemoveModule(
            HomeModuleId::QuickStartRequest
        )),
        None,
    ));
    assert!(!app.home_modules.is_enabled(HomeModuleId::QuickStartRequest));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::AddModule(HomeModuleId::QuickStartRequest)),
        None,
    ));
    assert!(app.home_modules.is_enabled(HomeModuleId::QuickStartRequest));

    for cat in HomeModuleCategory::ALL {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::SetCategoryEnabled(cat, false)),
            None,
        ));
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::SetCategoryEnabled(cat, true)),
            None,
        ));
    }

    // Render all 14 modules enabled together
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ShowAllModules), None,));
    assert_eq!(app.home_modules.enabled.len(), 16);
    let _ = std::hint::black_box(app.home_page());

    // 2. ProductivityHub: click all 4 tabs (0..=3) and verify rendering across all tabs
    for tab in 0..=3 {
        app.productivity.tab = tab;
        let _ = std::hint::black_box(app.render_productivity_hub());
    }
    app.productivity.tab = 0;

    // 3. CalendarPanorama: select date, jump to today, render selected date schedule
    app.back_screen.calendar_year = 2026;
    app.back_screen.calendar_month = 11;
    app.back_screen.selected_calendar_day = 5;
    assert_eq!(
        app.home_selected_date(),
        NaiveDate::from_ymd_opt(2026, 11, 5).unwrap()
    );
    let _ = std::hint::black_box(app.home_page());

    // 4. WeeklyAndAi: click all 4 AI quadrant pillars and verify selection state updates & renders
    for q in 0..4 {
        assert!(app.apply_home_action(&Action::HomeSelectAiQuadrant(q), None));
        assert_eq!(app.home_ai_quadrant, q);
        let _ = std::hint::black_box(app.home_page());
    }

    // 5. WorkModule, LifeModule, IssuesAndReplyModule: toggle checklist, query express, safe IM reply
    assert!(app.apply_home_action(
        &Action::HomeToggleChecklistItem("work-checklist-0".into()),
        None,
    ));
    assert!(app
        .home_checked_checklist_items
        .contains("work-checklist-0"));
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::QueryExpress("SF14298019283".into())),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeQuickImReply {
            key: "home-e2e-reply".into(),
            thread_id: "im:com.tencent.mm:文件传输助手".into(),
            package_name: "com.tencent.mm".into(),
            title: "文件传输助手".into(),
            text: "已核对完毕".into(),
        },
        None,
    ));

    // 6. AlarmsAndControl & BackScreenAndHabits: create custom alarm, toggle DND, trigger back-screen status & Tab(3)
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::CreatePhoneAlarm {
            hour: 8,
            minute: 15,
            label: "晨会提醒".into(),
        }),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::ToggleDnd(true)),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeExecuteIntent(HomeAiIntent::BackScreenStatus("会议中".into())),
        None,
    ));
    app.back_screen.tab = 3;
    app.page = Page::BackScreen;
    assert_eq!(app.page, Page::BackScreen);
    assert_eq!(app.back_screen.tab, 3);
    app.page = Page::Home;

    // 7. ContextLedgerHub: switch to Period(Day) / Manage and verify focused view renders & returns cleanly
    app.handle_context_action(&crate::ui::context_state::ContextAction::Period(
        stepx_companion::context_ledger::Period::Day,
    ));
    assert!(app.context_home_is_focused());
    let _ = std::hint::black_box(app.home_page());
    app.handle_context_action(&crate::ui::context_state::ContextAction::Manage);
    assert!(app.context_home_is_focused());
    let _ = std::hint::black_box(app.home_page());
    app.handle_context_action(&crate::ui::context_state::ContextAction::Attention);
    assert!(!app.context_home_is_focused());
    let _ = std::hint::black_box(app.home_page());

    // 8. ToolsSummaryDock & collapsed/expanded mode: verify ToolsSummaryDock renders in both states
    app.home_tools_expanded = true;
    let _ = std::hint::black_box(app.render_home_tools_summary_panel());
    assert!(app.apply_home_action(&Action::HomeToggleTools, None));
    assert!(!app.home_tools_expanded);
    let _ = std::hint::black_box(app.home_page());
    assert!(app.apply_home_action(&Action::HomeToggleTools, None));
    assert!(app.home_tools_expanded);

    // 9. Custom side-by-side free combination across categories (1/2 + 1/2, 1/3 + 2/3)
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ApplyPreset(HomeModulePreset::DefaultAll)),
        None,
    ));
    for m in [
        HomeModuleId::CalendarPanorama,
        HomeModuleId::PulseAndKpi,
        HomeModuleId::ProductivityHub,
        HomeModuleId::WeeklyAndAi,
    ] {
        assert!(app.apply_home_action(
            &Action::HomeModule(HomeModuleAction::SetModuleWidth(
                m,
                HomeModuleWidth::OneHalf
            )),
            None,
        ));
    }
    let _ = std::hint::black_box(app.home_page());
}

#[test]
fn home_mouse_drag_reorder_drag_resize_click_move_and_edit_anim_contract() {
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();
    app.page = Page::Home;

    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ApplyPreset(HomeModulePreset::DefaultAll)),
        None,
    ));
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ToggleEditMode), None,));
    assert!(app.home_module_edit_mode);

    // 1. Click-to-move: ToggleClickMove -> SwapModules / MoveBefore -> CancelClickMove
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ToggleClickMove(HomeModuleId::WorkModule)),
        None,
    ));
    assert_eq!(app.home_pending_move, Some(HomeModuleId::WorkModule));
    let _ = std::hint::black_box(app.home_page());

    let work_idx_before = app
        .home_modules
        .enabled
        .iter()
        .position(|m| *m == HomeModuleId::WorkModule)
        .unwrap();
    let life_idx_before = app
        .home_modules
        .enabled
        .iter()
        .position(|m| *m == HomeModuleId::LifeModule)
        .unwrap();
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::SwapModules(
            HomeModuleId::WorkModule,
            HomeModuleId::LifeModule,
        )),
        None,
    ));
    assert_eq!(app.home_pending_move, None);
    assert_eq!(
        app.home_modules.enabled[work_idx_before],
        HomeModuleId::LifeModule
    );
    assert_eq!(
        app.home_modules.enabled[life_idx_before],
        HomeModuleId::WorkModule
    );

    // MoveBefore via Click-to-Move
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ApplyPreset(HomeModulePreset::CompleteAll)),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::ToggleClickMove(HomeModuleId::PulseAndKpi)),
        None,
    ));
    assert_eq!(app.home_pending_move, Some(HomeModuleId::PulseAndKpi));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::MoveBefore(
            HomeModuleId::PulseAndKpi,
            HomeModuleId::CalendarPanorama,
        )),
        None,
    ));
    assert_eq!(app.home_pending_move, None);
    let pulse_pos = app
        .home_modules
        .enabled
        .iter()
        .position(|m| *m == HomeModuleId::PulseAndKpi)
        .unwrap();
    let cal_pos = app
        .home_modules
        .enabled
        .iter()
        .position(|m| *m == HomeModuleId::CalendarPanorama)
        .unwrap();
    assert_eq!(pulse_pos + 1, cal_pos);

    // 2. Mouse Drag-and-Drop Reorder lifecycle: BeginDrag -> UpdateDrag -> CommitDragSwap
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::BeginDrag(HomeModuleId::CalendarPanorama)),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::UpdateDrag {
            source: HomeModuleId::CalendarPanorama,
            hover_target: Some(HomeModuleId::WeeklyAndAi),
        }),
        None,
    ));
    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::CommitDragSwap(
            HomeModuleId::CalendarPanorama,
            HomeModuleId::WeeklyAndAi,
        )),
        None,
    ));
    let cal_after = app
        .home_modules
        .enabled
        .iter()
        .position(|m| *m == HomeModuleId::CalendarPanorama)
        .unwrap();
    let wk_after = app
        .home_modules
        .enabled
        .iter()
        .position(|m| *m == HomeModuleId::WeeklyAndAi)
        .unwrap();
    assert!(wk_after < cal_after);

    // 3. Mouse Drag-Resize quantization & CommitResize
    let (qw_wide, qh_tall) = HomePointerRegistry::compute_resized_dimensions(
        HomeModuleWidth::OneThird,
        HomeModuleHeight::Compact,
        180.0,
        140.0,
    );
    assert_eq!(qw_wide, HomeModuleWidth::TwoThirds);
    assert_eq!(qh_tall, HomeModuleHeight::Tall);

    let (qw_narrow, qh_compact) = HomePointerRegistry::compute_resized_dimensions(
        HomeModuleWidth::Full,
        HomeModuleHeight::Tall,
        -400.0,
        -160.0,
    );
    assert_eq!(qw_narrow, HomeModuleWidth::OneThird);
    assert_eq!(qh_compact, HomeModuleHeight::Compact);

    assert!(app.apply_home_action(
        &Action::HomeModule(HomeModuleAction::CommitResize(
            HomeModuleId::WorkModule,
            qw_wide,
            qh_tall,
        )),
        None,
    ));
    assert_eq!(
        app.home_modules.module_width(HomeModuleId::WorkModule),
        HomeModuleWidth::TwoThirds
    );
    assert_eq!(
        app.home_modules.module_height(HomeModuleId::WorkModule),
        HomeModuleHeight::Tall
    );

    // 4. Exiting edit mode clears pending move and active pointer operation
    app.home_pending_move = Some(HomeModuleId::LifeModule);
    app.home_pointer_registry.borrow_mut().active_op = Some(HomePointerOp::DragModule {
        source: HomeModuleId::LifeModule,
        start_pos: Vector2F::new(100., 100.),
        current_pos: Vector2F::new(140., 120.),
        hover_target: Some(HomeModuleId::WorkModule),
        moved: true,
    });
    assert!(app.home_drag_active());
    let _ = std::hint::black_box(app.home_page());
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ToggleEditMode), None,));
    assert!(!app.home_module_edit_mode);
    assert_eq!(app.home_pending_move, None);
    assert!(!app.home_drag_active());
}

#[test]
fn home_all_modules_value_by_size_and_stable_auto_height_contract() {
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();
    app.page = Page::Overview;

    let all_modules = HomeModuleId::ALL;

    for &m in &all_modules {
        let p_small = app.compute_home_module_value_profile(
            m,
            HomeModuleWidth::OneThird,
            HomeModuleHeight::Compact,
        );
        let p_med = app.compute_home_module_value_profile(
            m,
            HomeModuleWidth::OneHalf,
            HomeModuleHeight::Standard,
        );
        let p_large =
            app.compute_home_module_value_profile(m, HomeModuleWidth::Full, HomeModuleHeight::Tall);

        assert!(
            p_small.max_items <= p_med.max_items && p_med.max_items <= p_large.max_items,
            "Module {:?} max_items did not scale monotonically: small={} med={} large={}",
            m,
            p_small.max_items,
            p_med.max_items,
            p_large.max_items
        );
        assert!(
            p_small.columns <= p_large.columns && p_large.columns >= 2,
            "Module {:?} columns did not scale: small={} large={}",
            m,
            p_small.columns,
            p_large.columns
        );
        assert!(
            p_small.char_budget < p_large.char_budget,
            "Module {:?} char_budget did not scale: small={} large={}",
            m,
            p_small.char_budget,
            p_large.char_budget
        );

        for width in HomeModuleWidth::ALL {
            for height in HomeModuleHeight::ALL {
                app.home_modules.set_module_size(m, width, height);
                let prof = app.compute_home_module_value_profile(m, width, height);
                assert!(!prof.density_tier.is_empty());
                assert!(prof.max_items >= 1);
                assert!(prof.columns >= 1);
            }
        }
    }

    app.page = Page::Home;
    // Render full Home page across all 4 (Width, Height) combinations in both normal and edit mode
    assert!(app.apply_home_action(&Action::HomeModule(HomeModuleAction::ShowAllModules), None,));
    for (width, height) in [
        (HomeModuleWidth::OneThird, HomeModuleHeight::Compact),
        (HomeModuleWidth::OneHalf, HomeModuleHeight::Standard),
        (HomeModuleWidth::TwoThirds, HomeModuleHeight::Tall),
        (HomeModuleWidth::Full, HomeModuleHeight::Auto),
    ] {
        for &m in &all_modules {
            app.home_modules.set_module_size(m, width, height);
        }
        app.home_module_edit_mode = false;
        let _ = std::hint::black_box(app.home_page());
        app.home_module_edit_mode = true;
        let _ = std::hint::black_box(app.home_page());
    }

    // Auto starts from the standard budget; shrinking chooses compact rather
    // than first expanding to Tall. Pure horizontal moves retain Auto.
    let (_, qh_from_auto_to_compact) = HomePointerRegistry::compute_resized_dimensions(
        HomeModuleWidth::OneHalf,
        HomeModuleHeight::Auto,
        0.0,
        -60.0,
    );
    assert_eq!(qh_from_auto_to_compact, HomeModuleHeight::Compact);
    let (_, qh_horizontal_auto) = HomePointerRegistry::compute_resized_dimensions(
        HomeModuleWidth::OneHalf,
        HomeModuleHeight::Auto,
        130.0,
        0.0,
    );
    assert_eq!(qh_horizontal_auto, HomeModuleHeight::Auto);

    // Background arrivals must not change the chosen density or suddenly expand the layout.
    app.express_parcels.clear();
    app.push.snapshot.otp_banners.clear();
    assert_eq!(
        app.home_module_active_signal_count(HomeModuleId::LifeModule),
        0
    );
    assert_eq!(
        app.resolve_effective_height(HomeModuleId::LifeModule, HomeModuleHeight::Auto),
        HomeModuleHeight::Standard
    );
    app.express_parcels
        .push(stepx_companion::device::ExpressParcelInfo {
            carrier: "顺丰速运".into(),
            tracking_no: "SF1001".into(),
            status: "派送中".into(),
            pickup_code: "1-2-3001".into(),
            station: "丰巢快递柜".into(),
            updated_at: "10:00".into(),
        });
    assert_eq!(
        app.resolve_effective_height(HomeModuleId::LifeModule, HomeModuleHeight::Auto),
        HomeModuleHeight::Standard
    );
    for i in 0..3 {
        app.express_parcels
            .push(stepx_companion::device::ExpressParcelInfo {
                carrier: "京东快递".into(),
                tracking_no: format!("JD200{i}"),
                status: "待取件".into(),
                pickup_code: format!("2-3-400{i}"),
                station: "菜鸟驿站".into(),
                updated_at: "10:05".into(),
            });
    }
    assert!(app.home_module_active_signal_count(HomeModuleId::LifeModule) >= 4);
    assert_eq!(
        app.resolve_effective_height(HomeModuleId::LifeModule, HomeModuleHeight::Auto),
        HomeModuleHeight::Standard
    );
}

#[test]
fn home_weather_module_standalone_and_click_brings_weather_context_e2e() {
    let mut app = Companion::new_for_test();
    app.demo_mode = true;
    app.init_demo_push();
    app.page = Page::Home;

    // 1. Weather remains available as a compact secondary module after daily work.
    assert!(app.home_modules.is_enabled(HomeModuleId::WeatherModule));
    assert_eq!(
        app.home_modules.index_of(HomeModuleId::WeatherModule),
        Some(10)
    );
    let _ = std::hint::black_box(app.home_page());

    // 2. Legacy custom layouts retain a deliberately hidden weather module.
    let legacy_fourteen: Vec<HomeModuleId> = HomeModuleId::ALL
        .into_iter()
        .filter(|id| *id != HomeModuleId::WeatherModule)
        .collect();
    let mut legacy_custom = HomeModuleLayoutConfig {
        enabled: legacy_fourteen,
        preset: None,
        sizes: std::collections::BTreeMap::new(),
        recent_files_introduced: true,
        recent_files_home_restored: true,
        office_layout_introduced: false,
    };
    legacy_custom.sanitize();
    assert_eq!(legacy_custom.enabled.len(), 15);
    assert!(!legacy_custom.is_enabled(HomeModuleId::WeatherModule));
    assert!(legacy_custom.office_layout_introduced);
    // A deliberate removal after migration survives persistence and reload.
    legacy_custom.remove_module(HomeModuleId::WeatherModule);
    let saved = serde_json::to_vec(&legacy_custom).unwrap();
    let mut restored: HomeModuleLayoutConfig = serde_json::from_slice(&saved).unwrap();
    restored.sanitize();
    assert_eq!(restored, legacy_custom);
    assert!(!restored.is_enabled(HomeModuleId::WeatherModule));

    // 3. Render WeatherModule across all width/height combinations
    for width in HomeModuleWidth::ALL {
        for height in HomeModuleHeight::ALL {
            app.home_modules
                .set_module_size(HomeModuleId::WeatherModule, width, height);
            let prof =
                app.compute_home_module_value_profile(HomeModuleId::WeatherModule, width, height);
            assert!(prof.max_items >= 2);
            assert!(prof.columns >= 1);
            let _ = std::hint::black_box(app.home_page());
        }
    }

    // 4. Switch city and select forecast day
    assert!(app.apply_home_action(&Action::WeatherSelectCity("杭州".into()), None));
    assert_eq!(app.weather_snapshot.city, "杭州");
    assert!(app.apply_home_action(&Action::WeatherSelectDay(2), None));
    assert_eq!(app.weather_selected_day, 2);
    let _ = std::hint::black_box(app.home_page());

    // 5. Click-to-carry weather to Amoo (both default and specific hourly/daily focus slot)
    assert!(app.apply_home_action(&Action::WeatherBringToAmoo(None), None));
    assert_eq!(app.page, Page::Amoo);
    assert!(app.fields[F_AMOO].text.contains("杭州"));
    assert!(app.fields[F_AMOO].text.contains("°C"));

    app.page = Page::Home;
    assert!(app.apply_home_action(
        &Action::WeatherBringToAmoo(Some("15:00 小雨 17°C（降水概率 60%）".into())),
        None
    ));
    assert_eq!(app.page, Page::Amoo);
    assert!(app.fields[F_AMOO].text.contains("15:00 小雨 17°C"));

    // 6. Click-to-carry weather into Home command input
    app.page = Page::Home;
    assert!(app.apply_home_action(
        &Action::WeatherBringToHomeInput(Some("18:00 晚高峰通勤".into())),
        None
    ));
    assert_eq!(app.page, Page::Home);
    assert!(app.fields[F_HOME_COMMAND].text.contains("杭州"));
    assert!(app.fields[F_HOME_COMMAND].text.contains("18:00 晚高峰通勤"));

    // 7. Click-to-carry weather into Calendar Schedule reminder
    assert!(app.apply_home_action(
        &Action::WeatherBringToSchedule(Some("明日早高峰".into())),
        None
    ));
    assert_eq!(app.page, Page::Calendar);
    assert!(app.fields[F_PHONE_SCHEDULE_TITLE].text.contains("杭州"));
    assert!(app.fields[F_PHONE_SCHEDULE_TITLE]
        .text
        .contains("明日早高峰"));

    // 7b. Tomorrow advice & tomorrow schedule note handoffs
    let tmr_advice = app.weather_snapshot.tomorrow_advice();
    assert!(!tmr_advice.is_empty());
    assert!(!app.weather_snapshot.tomorrow_headline().is_empty());
    app.page = Page::Home;
    assert!(app.apply_home_action(&Action::WeatherBringToAmoo(Some(tmr_advice.clone())), None));
    assert_eq!(app.page, Page::Amoo);
    assert!(app.fields[F_AMOO].text.contains(&tmr_advice));

    app.page = Page::Home;
    let tmr_note = app.weather_snapshot.build_tomorrow_schedule_note();
    assert!(app.apply_home_action(
        &Action::WeatherBringToSchedule(Some(tmr_note.clone())),
        None
    ));
    assert_eq!(app.page, Page::Calendar);
    assert!(app.fields[F_PHONE_SCHEDULE_TITLE].text.contains(&tmr_note));

    // 8. Push weather badge to BackScreen
    assert!(app.apply_home_action(&Action::WeatherPushBackScreen, None));
    assert!(!app.back_screen.stepos_suite.backscreen_signature.is_empty());
}

#[test]
fn layout_upgrade_preserves_legacy_custom_visibility_and_order() {
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("layout.json");
    std::fs::write(&path, r#"{"enabled":["WeatherModule","CalendarPanorama","WorkModule"],"preset":null,"recent_files_introduced":true,"sizes":{"WorkModule":{"width":"OneHalf"}}}"#).unwrap();
    let mut app = Companion::new_for_test();
    assert!(app.recent_files_view);
    assert!(app.load_home_modules_config_from(&path));
    let expected = vec![HomeModuleId::WeatherModule, HomeModuleId::CalendarPanorama,
        HomeModuleId::WorkModule];
    assert_eq!(app.home_modules.enabled, expected);
    assert!(app.home_modules.office_layout_introduced);
    assert!(app.home_modules.recent_files_home_restored);
    assert_eq!(app.home_modules.enabled.iter().copied()
        .filter(|id| !matches!(id, HomeModuleId::CalendarPanorama | HomeModuleId::RecentFiles))
        .collect::<Vec<_>>(), vec![HomeModuleId::WeatherModule, HomeModuleId::WorkModule],
        "migration preserves the relative order of all other custom modules");
    assert_eq!(
        app.home_modules.module_width(HomeModuleId::WorkModule),
        HomeModuleWidth::OneHalf
    );
    assert!(HomeModuleId::ALL.contains(&HomeModuleId::RecentFiles));
    assert!(app.load_home_modules_config_from(&path));
    assert_eq!(app.home_modules.enabled, expected);
    // Explicit removal after the repair survives saving and loading again.
    app.home_modules
        .enabled
        .retain(|id| *id != HomeModuleId::RecentFiles);
    app.save_home_modules_config_to(&path).unwrap();
    assert!(app.load_home_modules_config_from(&path));
    assert!(!app.home_modules.is_enabled(HomeModuleId::RecentFiles));
    // Older layouts which still contain the card retain its exact position.
    std::fs::write(
        &path,
        r#"{"enabled":["CalendarPanorama","RecentFiles","WorkModule"],"preset":null}"#,
    )
    .unwrap();
    assert!(app.load_home_modules_config_from(&path));
    assert_eq!(
        app.home_modules.enabled,
        vec![
            HomeModuleId::CalendarPanorama,
            HomeModuleId::RecentFiles,
            HomeModuleId::WorkModule
        ]
    );
}

#[test]
fn home_tools_disclosure_preserves_custom_module_visibility_order_and_sizes() {
    let mut app = Companion::new_for_test();
    app.home_modules.remove_module(HomeModuleId::BackScreenAndHabits);
    app.home_modules.move_to_top(HomeModuleId::AlarmsAndControl);
    app.home_modules.set_module_size(HomeModuleId::PulseAndKpi,
        HomeModuleWidth::TwoThirds, HomeModuleHeight::Tall);
    let saved = app.home_modules.clone();
    for expanded in [false, true, false, true] {
        app.home_tools_expanded = !expanded;
        assert!(app.apply_home_action(&Action::HomeToggleTools, None));
        assert_eq!(app.home_tools_expanded, expanded);
        assert_eq!(app.home_modules, saved,
            "a local disclosure must neither hide unrelated modules nor restore hidden ones");
    }
    app.home_tools_expanded = false;
    app.apply_home_action(&Action::HomeModule(HomeModuleAction::ToggleEditMode), None);
    assert!(!app.home_tools_expanded, "layout editing must preserve the tools disclosure");
}

#[test]
fn home_pending_expansion_is_independent_from_context_ledger() {
    let mut app = Companion::new_for_test();
    assert!(!app.home_pending_expanded);
    assert!(app.apply_home_action(&Action::HomeTogglePending, None));
    assert!(app.home_pending_expanded);
    assert!(!app.context.show_all_items);
    app.handle_context_action(&crate::ui::context_state::ContextAction::ToggleItems);
    assert!(app.context.show_all_items);
    assert!(app.apply_home_action(&Action::HomeTogglePending, None));
    assert!(!app.home_pending_expanded);
    assert!(app.context.show_all_items);
}

#[cfg(feature = "ui-capture")]
#[test]
fn home_tools_normal_view_can_expand_and_collapse_at_every_card_height() {
    struct ToolsOnly(Companion);
    impl facloud_ui::Entity for ToolsOnly { type Event = (); }
    impl facloud_ui::TypedActionView for ToolsOnly { type Action = (); }
    impl facloud_ui::View for ToolsOnly {
        fn ui_name() -> &'static str { "home_tools_disclosure_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El {
            Stack::new().with_child(self.0.render_home_tools_summary_panel()).finish()
        }
    }
    facloud_ui::App::test((), |mut app| async move {
        for height in [HomeModuleHeight::Compact, HomeModuleHeight::Standard, HomeModuleHeight::Tall] {
            for expanded in [false, true] {
                let mut companion = Companion::new_for_test();
                companion.home_module_edit_mode = false;
                companion.home_tools_expanded = expanded;
                companion.home_modules.set_module_height(HomeModuleId::ToolsSummaryDock, height);
                let (window_id, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                    |_| ToolsOnly(companion));
                let root = app.root_view_id(window_id).unwrap();
                let (toggle_present, quick_action_present) = app.update(move |ctx| {
                    let mut presenter = facloud_ui::Presenter::new(window_id);
                    presenter.invalidate(facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]), ..Default::default()
                    }, ctx);
                    presenter.build_scene(vec2f(520., 1100.), 1., None, ctx);
                    let positions = presenter.position_cache();
                    (positions.get_position("home-toggle-tools").is_some(),
                     positions.get_position("home-dock-quick-backup").is_some())
                });
                assert!(toggle_present, "normal mode must offer disclosure at {height:?}");
                assert_eq!(quick_action_present, expanded,
                    "compact and tall cards must obey the disclosure state: {height:?}");
            }
        }
    });
}

// Compiled only with the production optimization profile. The companion script
// builds first, then runs this exact test alone in fresh processes. It enforces
// the original 15 ms budget on warm median/P95 and can compare same-host results.
// This measures View-tree construction and destruction, not layout/GPU painting,
// network synchronization, real-device latency, or a claim of 60 fps rendering.
#[cfg(all(not(debug_assertions), target_os = "linux"))]
#[test]
fn home_release_frame_budget_regression() {
    const WARMUP: usize = 20;
    const SAMPLES: usize = 80;
    // Use one long-lived app, as in production. In particular, do not accumulate
    // 30 startup reapers or terminate normal global workers to improve timings.
    // The Linux reaper sleeps for 150 + 500 + 1500 ms. Wait beyond that interval,
    // then verify that additional workers are parked and their CPU counters
    // have not changed for 300 ms. A live idle worker is not contamination.
    let initial_threads: std::collections::BTreeSet<_> = std::fs::read_dir("/proc/self/task")
        .expect("Linux benchmark needs /proc/self/task to verify isolation")
        .map(|entry| entry.expect("thread entry").file_name())
        .collect();
    #[derive(Debug, PartialEq, Eq, serde::Serialize)]
    struct WorkerSample {
        cpu_ticks: u64,
        state: String,
        wait: String,
    }
    let worker_samples = || {
        let mut workers = std::collections::BTreeMap::new();
        for entry in std::fs::read_dir("/proc/self/task").expect("thread inventory") {
            let entry = entry.expect("thread entry");
            if initial_threads.contains(&entry.file_name()) { continue; }
            // A startup worker may exit between inventory and reading stat.
            let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else { continue; };
            let fields: Vec<_> = stat.rsplit_once(") ").expect("proc stat format").1
                .split_whitespace().collect();
            let Ok(wait) = std::fs::read_to_string(entry.path().join("wchan")) else { continue; };
            workers.insert(entry.file_name().to_string_lossy().into_owned(), WorkerSample {
                cpu_ticks: fields[11].parse::<u64>().expect("utime")
                    + fields[12].parse::<u64>().expect("stime"),
                state: fields[0].to_owned(),
                wait: wait.trim().to_owned(),
            });
        }
        workers
    };
    let parked = |workers: &std::collections::BTreeMap<String, WorkerSample>| {
        // /proc wchan exposes kernel-version-dependent function names, not a stable ABI.
        // Accept only known sleeping futex helpers; unchanged ticks and inventory are
        // still checked before/after sampling, and unknown/hidden wait channels fail.
        workers.values().all(|w| w.state == "S" && matches!(w.wait.as_str(),
            "futex_do_wait" | "futex_wait_queue" | "futex_wait_queue_me"))
    };
    let mut app = Companion::new_for_test();
    app.demo_mode = false;
    app.page = Page::Home;
    let settling = std::time::Instant::now();
    std::thread::sleep(std::time::Duration::from_millis(2300));
    let workers_before = loop {
        let before = worker_samples();
        std::thread::sleep(std::time::Duration::from_millis(300));
        let after = worker_samples();
        if before == after && parked(&after) { break after; }
        assert!(settling.elapsed() < std::time::Duration::from_secs(8),
            "Startup workers are active; refusing contaminated measurements: {after:?}");
    };
    let settle_ms = settling.elapsed().as_millis();
    let mut rows = Vec::new();
    for populated in [false, true] {
        if populated {
            app.push.snapshot = stepx_companion::push::demo_push_snapshot();
            app.express_parcels = stepx_companion::device::default_express_parcels();
            app.back_screen.stepos_suite =
                stepx_companion::toolbox::default_stepos_ai_suite_report();
            // Exercise synchronized lists with local fixtures and no device job.
            let source = app.push.snapshot.threads.clone();
            for i in 0..120 {
                let mut thread = source[i % source.len()].clone();
                thread.thread_id = format!("home-perf-{i}");
                thread.title = format!("项目沟通 {i}");
                app.push.snapshot.threads.push(thread);
            }
        }
        for (width, height) in [(1000, 650), (1440, 900), (1920, 1080)] {
            for preset in HomeModulePreset::ALL {
                // Match real resizing/preset selection on one long-lived app.
                // first_us is the first build after switching this case; global
                // caches may be warm, so it is not application cold-start time.
                app.workspace.viewport = vec2f(width as f32, height as f32);
                app.home_modules = HomeModuleLayoutConfig::from_preset(preset);
                let first = std::time::Instant::now();
                drop(std::hint::black_box(app.home_page()));
                let first_us = first.elapsed().as_secs_f64() * 1_000_000.;
                for _ in 0..WARMUP {
                    drop(std::hint::black_box(app.home_page()));
                }
                let mut samples = Vec::with_capacity(SAMPLES);
                for _ in 0..SAMPLES {
                    let started = std::time::Instant::now();
                    drop(std::hint::black_box(app.home_page()));
                    samples.push(started.elapsed().as_secs_f64() * 1_000_000.);
                }
                samples.sort_by(f64::total_cmp);
                let median_us = (samples[SAMPLES / 2 - 1] + samples[SAMPLES / 2]) / 2.;
                let p95_us = samples[(SAMPLES * 95).div_ceil(100) - 1];
                rows.push(serde_json::json!({
                    "fixture": if populated { "populated-v1" } else { "empty-v1" },
                    "width": width, "height": height, "preset": format!("{preset:?}"),
                    "samples": SAMPLES, "warmup": WARMUP, "first_us": first_us,
                    "median_us": median_us, "p95_us": p95_us,
                    "mean_us": samples.iter().sum::<f64>() / SAMPLES as f64,
                    "min_us": samples[0], "max_us": samples[SAMPLES - 1],
                }));
            }
        }
    }
    assert_eq!(rows.len(), 30, "both fixtures, three viewports, all five presets");
    let workers_after = worker_samples();
    assert_eq!(workers_before, workers_after,
        "Background worker CPU/state changed during sampling; retry on an idle host");
    println!("STEPX_HOME_PERF_JSON {}", serde_json::json!({
        "schema": 2, "debug_assertions": cfg!(debug_assertions),
        "background_quiesced": true,
        "isolation": {
            "protocol": "one-companion-idle-workers-v1", "settle_ms": settle_ms,
            "workers_before": workers_before, "workers_after": workers_after,
        },
        "metric": "home_view_tree_build_and_drop_us", "cases": rows,
    }));
}
