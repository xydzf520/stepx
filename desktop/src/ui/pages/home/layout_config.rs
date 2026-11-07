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

/// Mirrors the native shell: 1 px border on both sides, 216 px sidebar,
/// page padding, and the scrollbar's 8 px track plus 2 px gutters on each side.
/// Keep this shared by the canvas and density budget; never use last-frame rects.
pub(in crate::ui) const HOME_CONTENT_MAX_WIDTH: f32 = 1420.;
pub(in crate::ui) fn home_canvas_content_width(window_width: f32) -> f32 {
    (window_width - 2. - SIDEBAR_W - S5 * 2. - ScrollbarWidth::Auto.as_f32() - 4.)
        .max(0.).min(HOME_CONTENT_MAX_WIDTH)
}

/// Functional domain category for `Page::Home` modular cards.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HomeModuleCategory {
    /// 日程全景、倒计时与全局指令
    ScheduleAndGlobal,
    /// 核心工作台：工作的、生活的、有问题的
    CoreWorkbench,
    /// 一周复盘、AI 建议与有来源的上下文台账
    AiAndReview,
    /// 昼夜表盘、核心指标、闹钟直控、背屏与用机习惯
    DeviceAndTelemetry,
    /// 连续工作区、快速发起新事与跨端快捷工具
    ExtendedWorkflow,
}

impl HomeModuleCategory {
    pub const ALL: [Self; 5] = [
        Self::ScheduleAndGlobal,
        Self::CoreWorkbench,
        Self::AiAndReview,
        Self::DeviceAndTelemetry,
        Self::ExtendedWorkflow,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::ScheduleAndGlobal => "日程与全局",
            Self::CoreWorkbench => "日常事项",
            Self::AiAndReview => "总结与资料",
            Self::DeviceAndTelemetry => "手机与状态",
            Self::ExtendedWorkflow => "快捷与接力",
        }
    }

    pub(in crate::ui) fn icon(self) -> UiIcon {
        match self {
            Self::ScheduleAndGlobal => UiIcon::Calendar,
            Self::CoreWorkbench => UiIcon::Grid,
            Self::AiAndReview => UiIcon::File,
            Self::DeviceAndTelemetry => UiIcon::Activity,
            Self::ExtendedWorkflow => UiIcon::Layers,
        }
    }
}

/// Complete inventory of modular functional blocks on `Page::Home`.
/// Users can freely add, remove/hide, reorder, resize width/height, or switch presets.
#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum HomeModuleId {
    /// Recent shared files from phone and desktop.
    RecentFiles,
    /// 手机资料、可追踪行动、跨端接管与会前准备，共享同一资料范围。
    ProductivityHub,
    /// 1. 实时倒计时与专注横幅 (Pomodoro / Takeout / Live phone capsules)
    LiveCountdown,
    /// Full month calendar with the selected day's paginated agenda.
    CalendarPanorama,
    /// 3. 本周概览与处理建议 (Native 7-Day Stacked Bars + 4 Pillars + Actionable Suggestions)
    WeeklyAndAi,
    /// 4. 工作安排 · 日程与产出 (Schedule, Action Checklists, Continuity Tasks, Deliverables)
    WorkModule,
    /// 5. 生活服务 · 快递与出行 (Express & Pickup, Rain/Commute, Leisure, Parcel Steppers)
    LifeModule,
    /// 6. 待确认与消息直回 (Schedule×Alarm Conflicts, Unresolved Ledger, Missed Calls, IM Inline Reply)
    IssuesAndReplyModule,
    /// 7. 全天作息与关键数据 (24h SVG Day Dial + 2×2 KPI Ring Gauges)
    PulseAndKpi,
    /// 8. 手机闹钟与快捷开关 (2-Col Alarm Toggles + Quick Alarm Creator + DND/Wi-Fi/BT/BackScreen Controls)
    AlarmsAndControl,
    /// 9. 背屏预览与屏幕时长 (17×23 LED Matrix Studio + Signature Push + App Usage SVG Donut)
    BackScreenAndHabits,
    /// 10. 手机快捷工具栏 (Quick Phone Tools Summary Dock & Expand/Collapse)
    ToolsSummaryDock,
    /// 11. 接着上次做 · 连续记录 (Dedicated Multi-Task Continuity & Recent Outputs Card)
    ContinuityWorkspace,
    /// 12. 讨论、成果与回顾 (Evidence-backed Day/Week/Month Context Ledger Hub)
    ContextLedgerHub,
    /// 13. 开始一件新事 · 快速发起 (Quick Task & Phone Scenario Launcher)
    QuickStartRequest,
    /// 14. 实时天气 · 预报与出行 (Live Weather, 24h Hourly, Multi-Day Forecast & One-Click Context Carry)
    WeatherModule,
}

impl HomeModuleId {
    pub const ALL: [Self; 16] = [
        Self::RecentFiles,
        Self::ProductivityHub,
        Self::LiveCountdown,
        Self::CalendarPanorama,
        Self::WeeklyAndAi,
        Self::WorkModule,
        Self::LifeModule,
        Self::IssuesAndReplyModule,
        Self::PulseAndKpi,
        Self::AlarmsAndControl,
        Self::BackScreenAndHabits,
        Self::ToolsSummaryDock,
        Self::ContinuityWorkspace,
        Self::ContextLedgerHub,
        Self::QuickStartRequest,
        Self::WeatherModule,
    ];

    pub fn slug(self) -> &'static str {
        match self {
            Self::ProductivityHub => "productivity_hub",
            Self::RecentFiles => "recent_files",
            Self::LiveCountdown => "live_countdown",
            Self::CalendarPanorama => "calendar_panorama",
            Self::WeeklyAndAi => "weekly_and_ai",
            Self::WorkModule => "work_module",
            Self::LifeModule => "life_module",
            Self::IssuesAndReplyModule => "issues_and_reply",
            Self::PulseAndKpi => "pulse_and_kpi",
            Self::AlarmsAndControl => "alarms_and_control",
            Self::BackScreenAndHabits => "backscreen_and_habits",
            Self::ToolsSummaryDock => "tools_summary_dock",
            Self::ContinuityWorkspace => "continuity_workspace",
            Self::ContextLedgerHub => "context_ledger_hub",
            Self::QuickStartRequest => "quick_start_request",
            Self::WeatherModule => "weather_module",
        }
    }

    pub fn title(self) -> &'static str {
        match self {
            Self::ProductivityHub => "资料、行动与协同工作",
            Self::RecentFiles => "最近文件 · 手机与电脑",
            Self::LiveCountdown => "实时倒计时与专注横幅",
            Self::CalendarPanorama => "月历与日程",
            Self::WeeklyAndAi => "本周概览与处理建议",
            Self::WorkModule => "工作安排 · 日程与产出",
            Self::LifeModule => "生活服务 · 快递与出行",
            Self::IssuesAndReplyModule => "待确认事项 · 冲突与直回",
            Self::PulseAndKpi => "全天作息与关键数据",
            Self::AlarmsAndControl => "手机闹钟与快捷开关",
            Self::BackScreenAndHabits => "背屏预览与屏幕时长",
            Self::ToolsSummaryDock => "手机快捷工具与状态速览",
            Self::ContinuityWorkspace => "接着上次做 · 连续记录",
            Self::ContextLedgerHub => "讨论、成果与回顾",
            Self::QuickStartRequest => "开始一件新事 · 快速发起",
            Self::WeatherModule => "实时天气 · 预报与出行",
        }
    }

    pub fn short_title(self) -> &'static str {
        match self {
            Self::ProductivityHub => "推进与交付",
            Self::RecentFiles => "最近文件",
            Self::LiveCountdown => "实时倒计时",
            Self::CalendarPanorama => "月历与日程",
            Self::WeeklyAndAi => "本周与建议",
            Self::WorkModule => "工作安排",
            Self::LifeModule => "生活服务",
            Self::IssuesAndReplyModule => "待确认与直回",
            Self::PulseAndKpi => "作息与数据",
            Self::AlarmsAndControl => "闹钟与开关",
            Self::BackScreenAndHabits => "背屏与时长",
            Self::ToolsSummaryDock => "快捷工具栏",
            Self::ContinuityWorkspace => "连续记录",
            Self::ContextLedgerHub => "讨论与成果",
            Self::QuickStartRequest => "快速发起",
            Self::WeatherModule => "实时天气",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::ProductivityHub => {
                "手机资料收件箱、行动跟踪、任务接管与会前准备包，按实际状态推进到成果"
            }
            Self::RecentFiles => "自动发现新下载、导出和拍摄的文件，打开或复制到另一端",
            Self::LiveCountdown => "查看进行中的专注计时、外卖配送与手机实况",
            Self::CalendarPanorama => "全月日历、所选日期的日程与待办，快速回到今天或安排新事项",
            Self::WeeklyAndAi => "7日事项分布图、四维复盘与手机待处理建议",
            Self::WorkModule => "当日会议日程、准备清单、接力记录与跨端工作文件",
            Self::LifeModule => "快递取件码、降雨出行提醒、日常补给与在途包裹物流进度",
            Self::IssuesAndReplyModule => {
                "核对日程与闹钟冲突，处理未接来电、未读会话和可后台回复的消息"
            }
            Self::PulseAndKpi => "查看今日安排、消息和生活提醒，可从数字跳到对应页面",
            Self::AlarmsAndControl => {
                "双列可视化手机闹钟开关、快速设闹钟与勿扰/Wi-Fi/蓝牙/背屏直控"
            }
            Self::BackScreenAndHabits => "17×23 背屏点阵实时预览、签名推送与今日应用时长环形图",
            Self::ToolsSummaryDock => "手机快捷工具折叠面板与页面模式切换",
            Self::ContinuityWorkspace => "独立展示最近电脑与手机接力对话、未发草稿与生成文件",
            Self::ContextLedgerHub => "关联手机与电脑资料，整理方案、待办和日周月总结",
            Self::QuickStartRequest => "一键发起电脑/手机协同任务、表格处理与常用手机操作",
            Self::WeatherModule => {
                "当前城市气温、24小时逐时预报、多日趋势，点击即可将天气带入对话与日程"
            }
        }
    }

    pub fn category(self) -> HomeModuleCategory {
        match self {
            Self::ProductivityHub => HomeModuleCategory::AiAndReview,
            Self::RecentFiles => HomeModuleCategory::ExtendedWorkflow,
            Self::LiveCountdown | Self::CalendarPanorama | Self::WeatherModule => {
                HomeModuleCategory::ScheduleAndGlobal
            }
            Self::WorkModule | Self::LifeModule | Self::IssuesAndReplyModule => {
                HomeModuleCategory::CoreWorkbench
            }
            Self::WeeklyAndAi | Self::ContextLedgerHub => HomeModuleCategory::AiAndReview,
            Self::PulseAndKpi | Self::AlarmsAndControl | Self::BackScreenAndHabits => {
                HomeModuleCategory::DeviceAndTelemetry
            }
            Self::ToolsSummaryDock | Self::ContinuityWorkspace | Self::QuickStartRequest => {
                HomeModuleCategory::ExtendedWorkflow
            }
        }
    }

    pub(in crate::ui) fn icon(self) -> UiIcon {
        match self {
            Self::ProductivityHub => UiIcon::Briefcase,
            Self::RecentFiles => UiIcon::Folder,
            Self::LiveCountdown => UiIcon::Clock,
            Self::CalendarPanorama => UiIcon::Calendar,
            Self::WeeklyAndAi => UiIcon::Layers,
            Self::WorkModule => UiIcon::Briefcase,
            Self::LifeModule => UiIcon::Package,
            Self::IssuesAndReplyModule => UiIcon::Alert,
            Self::PulseAndKpi => UiIcon::Activity,
            Self::AlarmsAndControl => UiIcon::Clock,
            Self::BackScreenAndHabits => UiIcon::Matrix,
            Self::ToolsSummaryDock => UiIcon::Grid,
            Self::ContinuityWorkspace => UiIcon::Layers,
            Self::ContextLedgerHub => UiIcon::File,
            Self::QuickStartRequest => UiIcon::Send,
            Self::WeatherModule => UiIcon::Sun,
        }
    }

    pub fn uses_svg(self) -> bool {
        matches!(
            self,
            Self::PulseAndKpi | Self::BackScreenAndHabits | Self::ContextLedgerHub
        )
    }

    pub fn is_core_workbench_col(self) -> bool {
        matches!(
            self,
            Self::WorkModule | Self::LifeModule | Self::IssuesAndReplyModule
        )
    }

    pub fn is_device_telemetry_col(self) -> bool {
        matches!(
            self,
            Self::PulseAndKpi | Self::AlarmsAndControl | Self::BackScreenAndHabits
        )
    }

    pub fn default_width(self) -> HomeModuleWidth {
        if self.is_core_workbench_col() || self.is_device_telemetry_col() {
            HomeModuleWidth::OneThird
        } else {
            HomeModuleWidth::Full
        }
    }

    pub fn default_height(self) -> HomeModuleHeight {
        HomeModuleHeight::Standard
    }
}

/// Free width tier for each module on `Page::Home`.
/// Consecutive non-Full modules automatically combine into multi-column rows up to 12 grid units.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HomeModuleWidth {
    /// 1/3 行宽 (4/12 grid units, up to 3 modules side-by-side)
    OneThird,
    /// 1/2 行宽 (6/12 grid units, 2 modules side-by-side)
    OneHalf,
    /// 2/3 行宽 (8/12 grid units, combines with a 1/3 module)
    TwoThirds,
    /// 整行全宽 (12/12 grid units)
    Full,
}

impl HomeModuleWidth {
    pub const ALL: [Self; 4] = [Self::OneThird, Self::OneHalf, Self::TwoThirds, Self::Full];

    pub fn label(self) -> &'static str {
        match self {
            Self::OneThird => "1/3 宽",
            Self::OneHalf => "1/2 宽",
            Self::TwoThirds => "2/3 宽",
            Self::Full => "全宽",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::OneThird => "1/3",
            Self::OneHalf => "1/2",
            Self::TwoThirds => "2/3",
            Self::Full => "全宽",
        }
    }

    pub fn units(self) -> u8 {
        match self {
            Self::OneThird => 4,
            Self::OneHalf => 6,
            Self::TwoThirds => 8,
            Self::Full => 12,
        }
    }

    pub fn flex_weight(self) -> f32 {
        self.units() as f32
    }

    pub fn next(self) -> Self {
        match self {
            Self::OneThird => Self::OneHalf,
            Self::OneHalf => Self::TwoThirds,
            Self::TwoThirds => Self::Full,
            Self::Full => Self::OneThird,
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::OneThird => "1/3",
            Self::OneHalf => "1/2",
            Self::TwoThirds => "2/3",
            Self::Full => "full",
        }
    }
}

/// Free height / vertical density tier for each module on `Page::Home`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HomeModuleHeight {
    /// 紧凑高度 (精简条目数量与纵向留白)
    Compact,
    /// 标准高度 (默认均衡密度)
    Standard,
    /// 舒展高度 (展开更多条目，不人为撑高空卡)
    Tall,
    /// 自适应高度 (根据视口预算稳定选择密度，不跟随后台数据升档)
    Auto,
}

impl HomeModuleHeight {
    pub const ALL: [Self; 4] = [Self::Compact, Self::Standard, Self::Tall, Self::Auto];

    pub fn label(self) -> &'static str {
        match self {
            Self::Compact => "紧凑",
            Self::Standard => "标准",
            Self::Tall => "舒展",
            Self::Auto => "自适应",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Compact => "紧凑",
            Self::Standard => "标准",
            Self::Tall => "舒展",
            Self::Auto => "自动",
        }
    }

    pub fn slug(self) -> &'static str {
        match self {
            Self::Compact => "compact",
            Self::Standard => "standard",
            Self::Tall => "tall",
            Self::Auto => "auto",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Compact => Self::Standard,
            Self::Standard => Self::Tall,
            Self::Tall => Self::Auto,
            Self::Auto => Self::Compact,
        }
    }

    pub fn min_card_height(self) -> Option<f32> {
        match self {
            Self::Compact | Self::Standard | Self::Auto => None,
            Self::Tall => None,
        }
    }

    pub fn item_limit(self, standard: usize) -> usize {
        match self {
            Self::Compact => (standard / 2).max(1),
            Self::Standard | Self::Auto => standard,
            Self::Tall => standard.saturating_add(2),
        }
    }
}

/// Per-module custom size overrides persisted in `HomeModuleLayoutConfig`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeModuleSize {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<HomeModuleWidth>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<HomeModuleHeight>,
}

impl HomeModuleSize {
    pub fn is_empty(&self) -> bool {
        self.width.is_none() && self.height.is_none()
    }
}

/// Curated layout presets for one-click switching on `Page::Home`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HomeModulePreset {
    /// 完整办公工作台：保留全部模块，用户自行调整顺序、尺寸与显隐。
    DefaultAll,
    /// 专注办公模式 (Schedule, Work, Issues/Reply, Continuity, Context Ledger, Quick Start, Weekly AI)
    WorkFocus,
    /// 生活与消息模式 (Countdown, Weather, Issues/IM Reply, Life/Express, Calendar, Alarms, BackScreen)
    LifeAndMessages,
    /// 极简脉搏模式 (Calendar Panorama, Work, Issues/Reply, Pulse KPI)
    MinimalPulse,
    /// 全量展开模式 (All 16 modules enabled)
    CompleteAll,
}

impl HomeModulePreset {
    pub const ALL: [Self; 5] = [
        Self::DefaultAll,
        Self::WorkFocus,
        Self::LifeAndMessages,
        Self::MinimalPulse,
        Self::CompleteAll,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::DefaultAll => "日常工作台",
            Self::WorkFocus => "专注办公",
            Self::LifeAndMessages => "生活与消息",
            Self::MinimalPulse => "极简概览",
            Self::CompleteAll => "全量展开",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::DefaultAll => {
                "保留全部 16 个模块，月历优先；按你的需要移动、排序或隐藏"
            }
            Self::WorkFocus => "资料、行动、会前准备与连续工作（10 个模块）",
            Self::LifeAndMessages => "置顶消息直回、快递天气、闹钟与背屏实况（8 个模块）",
            Self::MinimalPulse => "仅保留今日日程、工作/待办与全天数据（4 个模块）",
            Self::CompleteAll => "启用全部 16 个首页功能模块，自由调整宽高、顺序与显隐",
        }
    }

    pub fn from_cli_slug(slug: &str) -> Option<Self> {
        match slug.trim().to_ascii_lowercase().as_str() {
            "default" | "all-default" | "standard" => Some(Self::DefaultAll),
            "work" | "focus" | "office" => Some(Self::WorkFocus),
            "life" | "messages" | "im" => Some(Self::LifeAndMessages),
            "minimal" | "pulse" | "compact" => Some(Self::MinimalPulse),
            "complete" | "full" | "all" => Some(Self::CompleteAll),
            _ => None,
        }
    }

    pub fn modules(self) -> Vec<HomeModuleId> {
        let mut modules = match self {
            Self::DefaultAll => vec![
                HomeModuleId::CalendarPanorama,
                HomeModuleId::RecentFiles,
                HomeModuleId::IssuesAndReplyModule,
                HomeModuleId::WorkModule,
                HomeModuleId::ContinuityWorkspace,
                HomeModuleId::QuickStartRequest,
                HomeModuleId::ProductivityHub,
                HomeModuleId::ContextLedgerHub,
                HomeModuleId::WeeklyAndAi,
                HomeModuleId::LifeModule,
                HomeModuleId::WeatherModule,
                HomeModuleId::LiveCountdown,
                HomeModuleId::AlarmsAndControl,
                HomeModuleId::ToolsSummaryDock,
                HomeModuleId::BackScreenAndHabits,
                HomeModuleId::PulseAndKpi,
            ],
            Self::WorkFocus => vec![
                HomeModuleId::LiveCountdown,
                HomeModuleId::RecentFiles,
                HomeModuleId::ProductivityHub,
                HomeModuleId::ContextLedgerHub,
                HomeModuleId::ContinuityWorkspace,
                HomeModuleId::QuickStartRequest,
                HomeModuleId::CalendarPanorama,
                HomeModuleId::WorkModule,
                HomeModuleId::IssuesAndReplyModule,
                HomeModuleId::WeeklyAndAi,
            ],
            Self::LifeAndMessages => vec![
                HomeModuleId::LiveCountdown,
                HomeModuleId::WeatherModule,
                HomeModuleId::RecentFiles,
                HomeModuleId::IssuesAndReplyModule,
                HomeModuleId::LifeModule,
                HomeModuleId::CalendarPanorama,
                HomeModuleId::AlarmsAndControl,
                HomeModuleId::BackScreenAndHabits,
            ],
            Self::MinimalPulse => vec![
                HomeModuleId::CalendarPanorama,
                HomeModuleId::WorkModule,
                HomeModuleId::IssuesAndReplyModule,
                HomeModuleId::PulseAndKpi,
            ],
            Self::CompleteAll => std::iter::once(HomeModuleId::CalendarPanorama)
                .chain(HomeModuleId::ALL.into_iter().filter(|id| *id != HomeModuleId::CalendarPanorama))
                .collect(),
        };
        if let Some(at) = modules.iter().position(|id| *id == HomeModuleId::CalendarPanorama) {
            let calendar = modules.remove(at);
            modules.insert(0, calendar);
        }
        modules
    }

    fn sizes(self) -> std::collections::BTreeMap<HomeModuleId, HomeModuleSize> {
        if self != Self::DefaultAll { return Default::default(); }
        use HomeModuleHeight::Compact;
        use HomeModuleWidth::{OneHalf, OneThird, TwoThirds};
        [
            (HomeModuleId::IssuesAndReplyModule, HomeModuleSize { width: Some(OneHalf), height: None }),
            (HomeModuleId::WorkModule, HomeModuleSize { width: Some(OneHalf), height: None }),
            (HomeModuleId::RecentFiles, HomeModuleSize { width: None, height: Some(Compact) }),
            (HomeModuleId::ContinuityWorkspace, HomeModuleSize { width: Some(TwoThirds), height: Some(Compact) }),
            (HomeModuleId::QuickStartRequest, HomeModuleSize { width: Some(OneThird), height: Some(Compact) }),
            (HomeModuleId::LifeModule, HomeModuleSize { width: Some(TwoThirds), height: Some(Compact) }),
            (HomeModuleId::WeatherModule, HomeModuleSize { width: Some(OneThird), height: Some(Compact) }),
            (HomeModuleId::AlarmsAndControl, HomeModuleSize { width: Some(OneHalf), height: None }),
            (HomeModuleId::ToolsSummaryDock, HomeModuleSize { width: Some(OneHalf), height: Some(Compact) }),
            (HomeModuleId::BackScreenAndHabits, HomeModuleSize { width: Some(OneHalf), height: None }),
            (HomeModuleId::PulseAndKpi, HomeModuleSize { width: Some(OneHalf), height: None }),
        ].into_iter().collect()
    }
}

/// User-editable modular layout configuration for `Page::Home`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HomeModuleLayoutConfig {
    /// Ordered list of currently enabled modules on `Page::Home`.
    pub enabled: Vec<HomeModuleId>,
    #[serde(default)]
    pub recent_files_introduced: bool,
    /// One-time repair for layouts whose recent files card was removed in 0.2.148.
    #[serde(default)]
    pub recent_files_home_restored: bool,
    /// Legacy migration marker; custom order and visibility always remain authoritative.
    #[serde(default)]
    pub office_layout_introduced: bool,
    /// Active preset if the layout matches a preset; `None` when customized.
    pub preset: Option<HomeModulePreset>,
    /// Per-module custom width and height overrides.
    #[serde(default, skip_serializing_if = "std::collections::BTreeMap::is_empty")]
    pub sizes: std::collections::BTreeMap<HomeModuleId, HomeModuleSize>,
}

impl Default for HomeModuleLayoutConfig {
    fn default() -> Self {
        Self::from_preset(HomeModulePreset::DefaultAll)
    }
}

impl HomeModuleLayoutConfig {
    pub fn from_preset(preset: HomeModulePreset) -> Self {
        Self {
            enabled: preset.modules(),
            recent_files_introduced: true,
            recent_files_home_restored: true,
            office_layout_introduced: true,
            preset: Some(preset),
            sizes: preset.sizes(),
        }
    }

    pub fn normalize(&mut self) {
        self.sanitize();
    }

    pub fn sanitize(&mut self) {
        // Only a bundled default can gain modules during an upgrade. Custom
        // layouts are authoritative, even when old migration markers are absent.
        // Never remove a previously enabled module or replace its saved size.
        if self.preset == Some(HomeModulePreset::DefaultAll) {
            let order_174 = vec![HomeModuleId::CalendarPanorama, HomeModuleId::IssuesAndReplyModule,
                HomeModuleId::WorkModule, HomeModuleId::RecentFiles, HomeModuleId::ContinuityWorkspace,
                HomeModuleId::QuickStartRequest, HomeModuleId::ProductivityHub, HomeModuleId::ContextLedgerHub,
                HomeModuleId::WeeklyAndAi, HomeModuleId::LiveCountdown, HomeModuleId::AlarmsAndControl,
                HomeModuleId::ToolsSummaryDock, HomeModuleId::LifeModule, HomeModuleId::WeatherModule,
                HomeModuleId::BackScreenAndHabits, HomeModuleId::PulseAndKpi];
            let order_175 = vec![HomeModuleId::CalendarPanorama, HomeModuleId::RecentFiles,
                HomeModuleId::QuickStartRequest, HomeModuleId::IssuesAndReplyModule,
                HomeModuleId::WorkModule, HomeModuleId::ContinuityWorkspace,
                HomeModuleId::ProductivityHub, HomeModuleId::ContextLedgerHub,
                HomeModuleId::WeeklyAndAi, HomeModuleId::LiveCountdown, HomeModuleId::AlarmsAndControl,
                HomeModuleId::ToolsSummaryDock, HomeModuleId::LifeModule, HomeModuleId::WeatherModule,
                HomeModuleId::BackScreenAndHabits, HomeModuleId::PulseAndKpi];
            let sizes_174 = HomeModulePreset::DefaultAll.sizes();
            let mut sizes_175 = sizes_174.clone();
            sizes_175.insert(HomeModuleId::RecentFiles, HomeModuleSize { width: Some(HomeModuleWidth::TwoThirds), height: Some(HomeModuleHeight::Compact) });
            sizes_175.insert(HomeModuleId::ContinuityWorkspace, HomeModuleSize { width: None, height: Some(HomeModuleHeight::Compact) });
            // Upgrade only exact shipped defaults. Even a single custom width,
            // hidden module or reordered item keeps the user's layout intact.
            if (self.enabled == order_174 && self.sizes == sizes_174)
                || (self.enabled == order_175 && self.sizes == sizes_175)
            {
                self.enabled = HomeModulePreset::DefaultAll.modules();
                self.sizes = HomeModulePreset::DefaultAll.sizes();
            }
            let defaults = HomeModulePreset::DefaultAll;
            for id in defaults.modules() {
                if !self.enabled.contains(&id) {
                    self.enabled.push(id);
                    if let Some(size) = defaults.sizes().get(&id) {
                        self.sizes.entry(id).or_insert(*size);
                    }
                }
            }
            if !self.office_layout_introduced {
                self.enabled.retain(|id| *id != HomeModuleId::CalendarPanorama);
                self.enabled.insert(0, HomeModuleId::CalendarPanorama);
            }
        }
        let mut seen = Vec::with_capacity(self.enabled.len());
        for &m in &self.enabled {
            if HomeModuleId::ALL.contains(&m) && !seen.contains(&m) {
                seen.push(m);
            }
        }
        if seen.is_empty() {
            seen = HomeModulePreset::DefaultAll.modules();
            self.sizes = HomeModulePreset::DefaultAll.sizes();
            self.preset = Some(HomeModulePreset::DefaultAll);
        }
        self.enabled = seen;
        self.office_layout_introduced = true;
        self.recent_files_introduced = true;
        self.recent_files_home_restored = true;
        self.sizes.retain(|&id, sz| {
            let mut clean = *sz;
            if clean.width == Some(id.default_width()) {
                clean.width = None;
            }
            if clean.height == Some(id.default_height()) {
                clean.height = None;
            }
            *sz = clean;
            !sz.is_empty()
        });
        self.sync_matching_preset();
    }

    fn sync_matching_preset(&mut self) {
        self.preset = HomeModulePreset::ALL
            .into_iter()
            .find(|p| p.modules() == self.enabled && p.sizes() == self.sizes);
    }

    #[inline]
    pub fn is_enabled(&self, id: HomeModuleId) -> bool {
        self.enabled.contains(&id)
    }

    pub fn index_of(&self, id: HomeModuleId) -> Option<usize> {
        self.enabled.iter().position(|&m| m == id)
    }

    pub fn module_width(&self, id: HomeModuleId) -> HomeModuleWidth {
        self.sizes
            .get(&id)
            .and_then(|s| s.width)
            .unwrap_or_else(|| id.default_width())
    }

    pub fn set_module_width(&mut self, id: HomeModuleId, width: HomeModuleWidth) -> bool {
        let prev = self.module_width(id);
        if prev == width {
            return false;
        }
        let entry = self.sizes.entry(id).or_default();
        if width == id.default_width() {
            entry.width = None;
        } else {
            entry.width = Some(width);
        }
        if entry.is_empty() {
            self.sizes.remove(&id);
        }
        self.sync_matching_preset();
        true
    }

    pub fn cycle_module_width(&mut self, id: HomeModuleId) -> HomeModuleWidth {
        let next = self.module_width(id).next();
        self.set_module_width(id, next);
        next
    }

    pub fn module_height(&self, id: HomeModuleId) -> HomeModuleHeight {
        self.sizes
            .get(&id)
            .and_then(|s| s.height)
            .unwrap_or_else(|| id.default_height())
    }

    pub fn set_module_height(&mut self, id: HomeModuleId, height: HomeModuleHeight) -> bool {
        let prev = self.module_height(id);
        if prev == height {
            return false;
        }
        let entry = self.sizes.entry(id).or_default();
        if height == id.default_height() {
            entry.height = None;
        } else {
            entry.height = Some(height);
        }
        if entry.is_empty() {
            self.sizes.remove(&id);
        }
        self.sync_matching_preset();
        true
    }

    pub fn cycle_module_height(&mut self, id: HomeModuleId) -> HomeModuleHeight {
        let next = self.module_height(id).next();
        self.set_module_height(id, next);
        next
    }

    /// Packs enabled modules into visual rows according to each module's `HomeModuleWidth`.
    /// Any consecutive non-Full modules (regardless of category) combine into a multi-column row
    /// as long as their grid units sum to `<= 12` (up to 3 columns per row).
    pub fn pack_rows(&self) -> Vec<Vec<(HomeModuleId, HomeModuleWidth, HomeModuleHeight)>> {
        let mut rows: Vec<Vec<(HomeModuleId, HomeModuleWidth, HomeModuleHeight)>> = Vec::new();
        let mut cur_row: Vec<(HomeModuleId, HomeModuleWidth, HomeModuleHeight)> = Vec::new();
        let mut cur_units: u8 = 0;

        for &id in &self.enabled {
            let w = self.module_width(id);
            let h = self.module_height(id);
            let u = w.units();
            if w == HomeModuleWidth::Full {
                if !cur_row.is_empty() {
                    rows.push(std::mem::take(&mut cur_row));
                    cur_units = 0;
                }
                rows.push(vec![(id, w, h)]);
            } else if !cur_row.is_empty() && (cur_units + u > 12 || cur_row.len() >= 3) {
                rows.push(std::mem::take(&mut cur_row));
                cur_row.push((id, w, h));
                cur_units = u;
            } else {
                cur_row.push((id, w, h));
                cur_units += u;
                if cur_units >= 12 {
                    rows.push(std::mem::take(&mut cur_row));
                    cur_units = 0;
                }
            }
        }
        if !cur_row.is_empty() {
            rows.push(cur_row);
        }
        rows
    }

    /// Return (column count, row units, requested card units) without creating
    /// the nested row vectors for each module and each of its content sections.
    fn packed_row_metrics(&self, requested: HomeModuleId) -> Option<(usize, u8, u8)> {
        let (mut count, mut total, mut requested_units) = (0usize, 0u8, 0u8);
        for &id in &self.enabled {
            let units = self.module_width(id).units();
            if count > 0 && (units == 12 || total + units > 12 || count >= 3) {
                if requested_units > 0 { return Some((count, total, requested_units)); }
                count = 0;
                total = 0;
            }
            count += 1;
            total += units;
            if id == requested { requested_units = units; }
            if total >= 12 {
                if requested_units > 0 { return Some((count, total, requested_units)); }
                count = 0;
                total = 0;
            }
        }
        (requested_units > 0).then_some((count, total, requested_units))
    }

    /// Returns all modules from `HomeModuleId::ALL` that are currently hidden / not added to `Page::Home`.
    pub fn available_to_add(&self) -> Vec<HomeModuleId> {
        HomeModuleId::ALL
            .into_iter()
            .filter(|m| !self.enabled.contains(m))
            .collect()
    }

    /// Adds a module to the end of `enabled` (if not already present). Returns `true` if added.
    pub fn add_module(&mut self, id: HomeModuleId) -> bool {
        if self.enabled.contains(&id) {
            return false;
        }
        self.enabled.push(id);
        self.sync_matching_preset();
        true
    }

    /// Removes/hides a module from `enabled`. Preserves at least 1 module on `Page::Home`.
    pub fn remove_module(&mut self, id: HomeModuleId) -> bool {
        if self.enabled.len() <= 1 {
            return false;
        }
        let Some(pos) = self.index_of(id) else {
            return false;
        };
        self.enabled.remove(pos);
        self.sync_matching_preset();
        true
    }

    /// Toggles a module's visibility on `Page::Home`. Preserves at least 1 enabled module.
    pub fn toggle_module(&mut self, id: HomeModuleId) -> bool {
        if self.is_enabled(id) {
            self.remove_module(id)
        } else {
            self.add_module(id)
        }
    }

    /// Enables or hides all modules belonging to `category` (while keeping at least 1 module enabled overall).
    pub fn set_category_enabled(&mut self, category: HomeModuleCategory, enabled: bool) -> bool {
        let mut changed = false;
        if enabled {
            for m in HomeModuleId::ALL
                .into_iter()
                .filter(|m| m.category() == category)
            {
                if !self.enabled.contains(&m) {
                    self.enabled.push(m);
                    changed = true;
                }
            }
        } else {
            for m in HomeModuleId::ALL
                .into_iter()
                .filter(|m| m.category() == category)
            {
                if self.enabled.len() > 1 {
                    if let Some(pos) = self.index_of(m) {
                        self.enabled.remove(pos);
                        changed = true;
                    }
                }
            }
        }
        if changed {
            self.sync_matching_preset();
        }
        changed
    }

    /// Enables all modules while preserving the relative order of already-enabled modules.
    pub fn show_all_modules(&mut self) -> bool {
        let mut changed = false;
        for m in HomeModuleId::ALL {
            if !self.enabled.contains(&m) {
                self.enabled.push(m);
                changed = true;
            }
        }
        if changed {
            self.sync_matching_preset();
        }
        changed
    }

    /// Moves `id` one slot earlier in `enabled`.
    pub fn move_up(&mut self, id: HomeModuleId) -> bool {
        let Some(pos) = self.index_of(id) else {
            return false;
        };
        if pos == 0 {
            return false;
        }
        self.enabled.swap(pos, pos - 1);
        self.sync_matching_preset();
        true
    }

    /// Moves `id` one slot later in `enabled`.
    pub fn move_down(&mut self, id: HomeModuleId) -> bool {
        let Some(pos) = self.index_of(id) else {
            return false;
        };
        if pos + 1 >= self.enabled.len() {
            return false;
        }
        self.enabled.swap(pos, pos + 1);
        self.sync_matching_preset();
        true
    }

    /// Pins `id` to the first slot (`index 0`) of `enabled`.
    pub fn move_to_top(&mut self, id: HomeModuleId) -> bool {
        let Some(pos) = self.index_of(id) else {
            return false;
        };
        if pos == 0 {
            return false;
        }
        let item = self.enabled.remove(pos);
        self.enabled.insert(0, item);
        self.sync_matching_preset();
        true
    }

    /// Moves `id` to the last slot of `enabled`.
    pub fn move_to_bottom(&mut self, id: HomeModuleId) -> bool {
        let Some(pos) = self.index_of(id) else {
            return false;
        };
        if pos + 1 >= self.enabled.len() {
            return false;
        }
        let item = self.enabled.remove(pos);
        self.enabled.push(item);
        self.sync_matching_preset();
        true
    }

    /// Moves `id` to an arbitrary index `target_idx` in `enabled`.
    pub fn move_to_index(&mut self, id: HomeModuleId, target_idx: usize) -> bool {
        let Some(pos) = self.index_of(id) else {
            return false;
        };
        if self.enabled.is_empty() {
            return false;
        }
        let clamped = target_idx.min(self.enabled.len() - 1);
        if pos == clamped {
            return false;
        }
        let item = self.enabled.remove(pos);
        self.enabled.insert(clamped, item);
        self.sync_matching_preset();
        true
    }

    /// Swaps the positions of two enabled modules `a` and `b` in `self.enabled`.
    pub fn swap_modules(&mut self, a: HomeModuleId, b: HomeModuleId) -> bool {
        if a == b {
            return false;
        }
        let (Some(pos_a), Some(pos_b)) = (self.index_of(a), self.index_of(b)) else {
            return false;
        };
        self.enabled.swap(pos_a, pos_b);
        self.sync_matching_preset();
        true
    }

    /// Moves `source` to be positioned immediately before `target` in `self.enabled`.
    pub fn move_before(&mut self, source: HomeModuleId, target: HomeModuleId) -> bool {
        if source == target {
            return false;
        }
        let (Some(src_pos), Some(_)) = (self.index_of(source), self.index_of(target)) else {
            return false;
        };
        self.enabled.remove(src_pos);
        let Some(target_pos) = self.index_of(target) else {
            self.enabled.insert(src_pos.min(self.enabled.len()), source);
            return false;
        };
        self.enabled.insert(target_pos, source);
        self.sync_matching_preset();
        true
    }

    /// Sets both width and height of `id` in a single atomic operation.
    pub fn set_module_size(
        &mut self,
        id: HomeModuleId,
        width: HomeModuleWidth,
        height: HomeModuleHeight,
    ) -> bool {
        let w_changed = self.set_module_width(id, width);
        let h_changed = self.set_module_height(id, height);
        w_changed || h_changed
    }

    pub fn apply_preset(&mut self, preset: HomeModulePreset) {
        self.enabled = preset.modules();
        self.sizes = preset.sizes();
        self.office_layout_introduced = true;
        self.preset = Some(preset);
    }

    pub fn reset_default(&mut self) {
        self.apply_preset(HomeModulePreset::DefaultAll);
    }
}

/// Interactive actions for editing `Page::Home` modules (adding, removing, reordering, resizing, presets).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HomeModuleAction {
    ToggleEditMode,
    UndoEdit,
    CancelEdit,
    ToggleModuleLibrary,
    SetEditMode(bool),
    FilterCategory(Option<HomeModuleCategory>),
    AddModule(HomeModuleId),
    RemoveModule(HomeModuleId),
    ToggleModule(HomeModuleId),
    SetCategoryEnabled(HomeModuleCategory, bool),
    ShowAllModules,
    SetModuleWidth(HomeModuleId, HomeModuleWidth),
    CycleModuleWidth(HomeModuleId),
    SetModuleHeight(HomeModuleId, HomeModuleHeight),
    CycleModuleHeight(HomeModuleId),
    SetModuleSize(HomeModuleId, HomeModuleWidth, HomeModuleHeight),
    MoveUp(HomeModuleId),
    MoveDown(HomeModuleId),
    MoveToTop(HomeModuleId),
    MoveToBottom(HomeModuleId),
    MoveToIndex(HomeModuleId, usize),
    ToggleClickMove(HomeModuleId),
    CancelClickMove,
    SwapModules(HomeModuleId, HomeModuleId),
    MoveBefore(HomeModuleId, HomeModuleId),
    BeginDrag(HomeModuleId),
    UpdateDrag {
        source: HomeModuleId,
        hover_target: Option<HomeModuleId>,
    },
    CommitDragSwap(HomeModuleId, HomeModuleId),
    CommitDragInsert(HomeModuleId, HomeModuleId, bool),
    CancelDrag,
    BeginResize(HomeModuleId),
    CommitResize(HomeModuleId, HomeModuleWidth, HomeModuleHeight),
    ApplyPreset(HomeModulePreset),
    ResetDefault,
}

/// Semantic Value-by-Size density tier for `Page::Home` modules.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum HomeDensityTier {
    HeadlineTop1,
    BalancedWorkbench,
    DeepPanorama,
}

impl HomeDensityTier {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::HeadlineTop1 => "HeadlineTop1",
            Self::BalancedWorkbench => "BalancedWorkbench",
            Self::DeepPanorama => "DeepPanorama",
        }
    }
}

/// Deterministic Value-by-Size profile for any `HomeModuleId` at `(width, height)`.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ModuleValueProfile {
    pub module_id: String,
    pub width: String,
    pub height: String,
    pub resolved_height: String,
    pub columns: usize,
    pub internal_columns: usize,
    pub max_items: usize,
    pub primary_item_budget: usize,
    pub char_budget: usize,
    pub density_tier: String,
    pub effective_density: HomeDensityTier,
    pub show_side_detail_panel: bool,
    pub show_extended_subcards: bool,
    pub unlocked_sections: Vec<&'static str>,
}

impl Companion {
    pub(in crate::ui) fn home_modules_config_path() -> Option<PathBuf> {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".config")))?;
        Some(base.join("stepx").join("home-modules-layout.json"))
    }

    pub(in crate::ui) fn save_home_modules_config_to(&self, path: &Path) -> std::io::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let json = serde_json::to_string_pretty(&self.home_modules)
            .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
        use std::io::Write;
        let mut temporary =
            tempfile::NamedTempFile::new_in(path.parent().unwrap_or(Path::new(".")))?;
        temporary.write_all(json.as_bytes())?;
        temporary.as_file().sync_all()?;
        temporary.persist(path).map_err(|error| error.error)?;
        Ok(())
    }

    pub(in crate::ui) fn load_home_modules_config_from(&mut self, path: &Path) -> bool {
        let Ok(raw) = std::fs::read_to_string(path) else {
            return false;
        };
        let Ok(mut parsed) = serde_json::from_str::<HomeModuleLayoutConfig>(&raw) else {
            return false;
        };
        let before = parsed.clone();
        parsed.sanitize();
        let migrated = parsed != before;
        self.home_modules = parsed;
        if migrated {
            let _ = self.save_home_modules_config_to(path);
        }
        true
    }

    pub(in crate::ui) fn save_home_modules_config(&mut self) {
        if cfg!(test) {
            return;
        }
        if let Some(path) = Self::home_modules_config_path() {
            if let Err(error) = self.save_home_modules_config_to(&path) {
                self.error = true;
                self.status = format!("布局已调整，但保存失败：{error}。请检查配置目录后重试。");
            }
        }
    }

    pub(in crate::ui) fn load_home_modules_config(&mut self) {
        if let Some(path) = Self::home_modules_config_path() {
            let _ = self.load_home_modules_config_from(&path);
        }
    }

    pub(in crate::ui) fn home_drag_active(&self) -> bool {
        self.home_pointer_registry.borrow().active_op.is_some()
    }

    pub(in crate::ui) fn home_module_active_signal_count(&self, id: HomeModuleId) -> usize {
        let suite = &self.back_screen.stepos_suite;
        match id {
            HomeModuleId::RecentFiles => {
                self.recent_files.desktop.len() + self.recent_files.phone.len()
            }
            HomeModuleId::LiveCountdown => {
                usize::from(self.back_screen.pomodoro_active)
                    + self
                        .push
                        .snapshot
                        .notifications
                        .iter()
                        .filter(|n| n.category == "service" || n.category == "transport")
                        .count()
            }
            HomeModuleId::CalendarPanorama => {
                suite.calendar_events.len()
                    + suite.alarms.iter().filter(|a| a.enabled).count()
                    + self.workspace.journal.tasks.len().min(3)
            }
            HomeModuleId::WeeklyAndAi => {
                suite.calendar_events.len()
                    + self.workspace.journal.tasks.len()
                    + self.context.overview.items.len().min(3)
            }
            HomeModuleId::WorkModule => {
                suite.calendar_events.len()
                    + self.workspace.journal.tasks.len()
                    + self.work.artifacts.len()
                    + suite.remote_agent_files.len()
            }
            HomeModuleId::LifeModule => {
                self.express_parcels.len() + self.push.snapshot.otp_banners.len()
            }
            HomeModuleId::IssuesAndReplyModule => {
                self.push.snapshot.threads.len()
                    + self.push.snapshot.notifications.len()
                    + self.sms.len().min(3)
            }
            HomeModuleId::PulseAndKpi => {
                suite.calendar_events.len()
                    + suite.alarms.len()
                    + usize::from(self.selected.is_some())
            }
            HomeModuleId::AlarmsAndControl => {
                suite.alarms.len() + usize::from(self.back_screen.pomodoro_active)
            }
            HomeModuleId::BackScreenAndHabits => {
                suite.wellbeing.top_apps.len() + usize::from(!suite.backscreen_signature.is_empty())
            }
            HomeModuleId::ToolsSummaryDock => {
                usize::from(!self.sms.is_empty())
                    + usize::from(!self.push.snapshot.notifications.is_empty())
                    + usize::from(!suite.alarms.is_empty())
                    + usize::from(!self.work.artifacts.is_empty())
                    + self.push.snapshot.otp_banners.len()
            }
            HomeModuleId::ContinuityWorkspace => {
                self.workspace.journal.tasks.len() + self.work.artifacts.len()
            }
            HomeModuleId::QuickStartRequest => {
                usize::from(!self.fields[F_WORK_PROMPT].text.trim().is_empty())
                    + self.workspace.journal.tasks.len().min(4)
            }
            HomeModuleId::ProductivityHub => {
                self.context.overview.items.len() + self.workspace.journal.tasks.len()
            }
            HomeModuleId::ContextLedgerHub => self.context.overview.items.len(),
            HomeModuleId::WeatherModule => {
                1 + usize::from(self.weather_snapshot.umbrella_needed)
                    + self.weather_snapshot.hourly.len().min(2)
            }
        }
    }

    /// Match the same twelve-column packing and breakpoints used by the canvas.
    /// A saved half-width card becomes a full-width card when its row stacks;
    /// density must follow that available space rather than the saved enum.
    pub(in crate::ui) fn home_module_available_width(&self, id: HomeModuleId) -> f32 {
        let content = home_canvas_content_width(self.workspace.viewport.x());
        if content < 760. || self.home_modules.module_width(id) == HomeModuleWidth::Full {
            return content;
        }
        let Some((count, total, weight)) = self.home_modules.packed_row_metrics(id) else { return content; };
        if count <= 1 || (count >= 3 && content < 1040.) { return content; }
        (content - S3 * (count - 1) as f32) * f32::from(weight) / f32::from(total)
    }

    pub(in crate::ui) fn home_module_content_width(&self, id: HomeModuleId) -> HomeModuleWidth {
        if self.page != Page::Home { return self.home_modules.module_width(id); }
        match self.home_module_available_width(id) {
            w if w < 420. => HomeModuleWidth::OneThird,
            w if w < 620. => HomeModuleWidth::OneHalf,
            w if w < 960. => HomeModuleWidth::TwoThirds,
            _ => HomeModuleWidth::Full,
        }
    }

    pub(in crate::ui) fn resolve_effective_height(
        &self,
        _id: HomeModuleId,
        height: HomeModuleHeight,
    ) -> HomeModuleHeight {
        // Auto is a viewport budget, never a live-data count. A new message or
        // file must not promote a card to Tall and push the reader off-screen.
        // Tall unlocks more content; no universal minimum height adds blanks.
        match height {
            HomeModuleHeight::Compact | HomeModuleHeight::Tall => height,
            HomeModuleHeight::Auto | HomeModuleHeight::Standard if self.workspace.viewport.y() < 740. => HomeModuleHeight::Compact,
            _ => HomeModuleHeight::Standard,
        }
    }

    pub(in crate::ui) fn compute_home_module_value_profile(
        &self,
        id: HomeModuleId,
        width: HomeModuleWidth,
        height: HomeModuleHeight,
    ) -> ModuleValueProfile {
        let width = if self.page == Page::Home { self.home_module_content_width(id) } else { width };
        let eff_h = self.resolve_effective_height(id, height);
        let effective_density = match eff_h {
            HomeModuleHeight::Compact => HomeDensityTier::HeadlineTop1,
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                HomeDensityTier::BalancedWorkbench
            }
            HomeModuleHeight::Tall => HomeDensityTier::DeepPanorama,
        };
        let density_tier = effective_density.as_str().to_string();

        let (columns, max_items, char_budget, mut unlocked_sections) = match id {
            HomeModuleId::RecentFiles => {
                let cols = if matches!(width, HomeModuleWidth::Full | HomeModuleWidth::TwoThirds) {
                    2
                } else {
                    1
                };
                let count = eff_h.item_limit(6);
                (
                    cols,
                    count,
                    if cols == 2 { 96 } else { 48 },
                    vec!["recent_files", "source_filters", "verified_transfer"],
                )
            }
            HomeModuleId::LiveCountdown => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 3,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 1,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 2,
                    HomeModuleHeight::Tall => 4,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 28,
                    HomeModuleWidth::OneHalf => 42,
                    HomeModuleWidth::TwoThirds => 64,
                    HomeModuleWidth::Full => 96,
                };
                let mut secs = vec!["active_countdown_pill", "quick_toggle"];
                if matches!(
                    width,
                    HomeModuleWidth::OneHalf | HomeModuleWidth::TwoThirds | HomeModuleWidth::Full
                ) {
                    secs.push("pomodoro_takeout_pair");
                }
                if matches!(width, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full) {
                    secs.push("multi_capsule_strip");
                }
                if width == HomeModuleWidth::Full {
                    secs.push("inline_quick_controls");
                }
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("compact_single_line_pill"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("progress_bars")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("progress_bars");
                        secs.push("preset_duration_strip");
                        secs.push("focus_target_subtitle");
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::CalendarPanorama => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
                };
                let items = match eff_h { HomeModuleHeight::Compact => 3, HomeModuleHeight::Tall => 6, _ => 5 };
                let chars = match width {
                    HomeModuleWidth::OneThird => 28,
                    HomeModuleWidth::OneHalf => 40,
                    HomeModuleWidth::TwoThirds => 52,
                    HomeModuleWidth::Full => 72,
                };
                let secs = vec!["full_month_calendar", "month_navigation", "today_shortcut",
                    "selected_date_agenda", "agenda_pagination", "open_schedule_editor"];
                (cols, items, chars, secs)
            }
            HomeModuleId::WeeklyAndAi => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
                };
                let items = match (eff_h, width) {
                    (HomeModuleHeight::Compact, _) => 1,
                    (
                        HomeModuleHeight::Standard | HomeModuleHeight::Auto,
                        HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf,
                    ) => 2,
                    (HomeModuleHeight::Standard | HomeModuleHeight::Auto, _) => 3,
                    (HomeModuleHeight::Tall, _) => 5,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 30,
                    HomeModuleWidth::OneHalf => 44,
                    HomeModuleWidth::TwoThirds => 60,
                    HomeModuleWidth::Full => 80,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => {
                        vec!["compact_week_sparkline", "ai_suggestion_list"]
                    }
                    HomeModuleWidth::OneHalf => {
                        vec![
                            "compact_week_sparkline",
                            "ai_quadrant_tabs",
                            "ai_suggestion_list",
                        ]
                    }
                    HomeModuleWidth::TwoThirds => {
                        vec![
                            "weekly_7day_chart",
                            "ai_quadrant_briefing",
                            "ai_suggestion_list",
                        ]
                    }
                    HomeModuleWidth::Full => vec![
                        "weekly_7day_chart",
                        "completion_metrics",
                        "ai_quadrant_briefing",
                        "one_click_execution_buttons",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("kpi_headline_and_top1_ai"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("weekly_bars_and_ai_cards")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("full_7day_breakdown");
                        secs.push("selected_quadrant_drilldown");
                        secs.push("5_ai_suggestions");
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::WorkModule => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 2,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 4,
                    HomeModuleHeight::Tall => 7,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 30,
                    HomeModuleWidth::OneHalf => 46,
                    HomeModuleWidth::TwoThirds => 72,
                    HomeModuleWidth::Full => 108,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["work_task_list", "status_pill"],
                    HomeModuleWidth::OneHalf => {
                        vec!["work_task_list", "source_badges", "primary_cta"]
                    }
                    HomeModuleWidth::TwoThirds => {
                        vec![
                            "active_deliverables_col",
                            "workspace_resumable_col",
                            "source_badges",
                        ]
                    }
                    HomeModuleWidth::Full => vec![
                        "active_deliverables_col",
                        "workspace_resumable_col",
                        "inline_task_dispatch_bar",
                        "deliverable_artifact_links",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("top2_urgent_tasks"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("4_tasks_with_subtitles")
                    }
                    HomeModuleHeight::Tall => secs.push("7_tasks_with_steps_and_prompt_bar"),
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::LifeModule => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 3,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 2,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 3,
                    HomeModuleHeight::Tall => 6,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 28,
                    HomeModuleWidth::OneHalf => 44,
                    HomeModuleWidth::TwoThirds => 68,
                    HomeModuleWidth::Full => 100,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["pickup_otp_highlight", "top_parcel_card"],
                    HomeModuleWidth::OneHalf => {
                        vec!["pickup_otp_tickets", "parcel_cards", "life_domain_pills"]
                    }
                    HomeModuleWidth::TwoThirds => {
                        vec![
                            "pickup_otp_tickets",
                            "parcel_stepper_cards",
                            "life_domain_grid",
                        ]
                    }
                    HomeModuleWidth::Full => vec![
                        "3col_life_express_board",
                        "pickup_otp_tickets",
                        "parcel_stepper_cards",
                        "travel_weather_reminders",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("2_urgent_pickup_or_life_cards"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("3_life_cards_with_stepper")
                    }
                    HomeModuleHeight::Tall => secs.push("6_life_cards_with_route_history"),
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::IssuesAndReplyModule => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 2,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 4,
                    HomeModuleHeight::Tall => 7,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 30,
                    HomeModuleWidth::OneHalf => 48,
                    HomeModuleWidth::TwoThirds => 76,
                    HomeModuleWidth::Full => 116,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["urgent_issue_feed", "quick_reply_chip"],
                    HomeModuleWidth::OneHalf => {
                        vec!["urgent_issue_feed", "im_thread_list", "inline_reply_box"]
                    }
                    HomeModuleWidth::TwoThirds => vec![
                        "master_detail_im_split",
                        "issue_and_thread_list",
                        "conversation_context_pane",
                        "smart_reply_chips",
                    ],
                    HomeModuleWidth::Full => vec![
                        "master_detail_im_split",
                        "issue_and_thread_list",
                        "conversation_context_pane",
                        "smart_reply_chips",
                        "inline_reply_composer",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("top2_unread_threads"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("4_threads_with_3_smart_replies")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("7_threads_with_message_preview_and_composer")
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::PulseAndKpi => {
                let cols = match width {
                    HomeModuleWidth::OneThird => 1,
                    HomeModuleWidth::OneHalf | HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 3,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 2,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 4,
                    HomeModuleHeight::Tall => 6,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 28,
                    HomeModuleWidth::OneHalf => 44,
                    HomeModuleWidth::TwoThirds => 68,
                    HomeModuleWidth::Full => 96,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["compact_day_dial", "kpi_2x2_grid"],
                    HomeModuleWidth::OneHalf => vec!["day_dial_svg", "kpi_2x2_rings"],
                    HomeModuleWidth::TwoThirds => {
                        vec!["day_dial_svg", "kpi_2x2_rings", "next_up_countdown_callout"]
                    }
                    HomeModuleWidth::Full => vec![
                        "day_dial_svg",
                        "kpi_4_ring_strip",
                        "next_up_event_callout",
                        "device_telemetry_strip",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("compact_dial_128_and_kpi_pills"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("standard_dial_164_and_4_rings")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("tall_dial_196_with_legend_and_telemetry_breakdown")
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::AlarmsAndControl => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 3,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 2,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 4,
                    HomeModuleHeight::Tall => 6,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 26,
                    HomeModuleWidth::OneHalf => 40,
                    HomeModuleWidth::TwoThirds => 64,
                    HomeModuleWidth::Full => 92,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => {
                        vec!["next_alarms_list", "compact_control_switches"]
                    }
                    HomeModuleWidth::OneHalf => {
                        vec!["alarm_toggle_list", "pomodoro_bar", "control_switch_grid"]
                    }
                    HomeModuleWidth::TwoThirds => {
                        vec!["alarms_and_creator_col", "pomodoro_and_control_col"]
                    }
                    HomeModuleWidth::Full => vec![
                        "alarms_col",
                        "pomodoro_focus_col",
                        "device_control_matrix_col",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("top2_alarms_and_quick_toggles"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("4_alarms_plus_inline_alarm_creator")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("6_alarms_plus_creator_and_pomodoro_presets")
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::BackScreenAndHabits => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 3,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 5,
                    HomeModuleHeight::Tall => 8,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 28,
                    HomeModuleWidth::OneHalf => 42,
                    HomeModuleWidth::TwoThirds => 68,
                    HomeModuleWidth::Full => 98,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["led_matrix_preview", "habit_donut_summary"],
                    HomeModuleWidth::OneHalf => {
                        vec!["led_matrix_preview", "preset_chips", "habit_donut_summary"]
                    }
                    HomeModuleWidth::TwoThirds => {
                        vec!["backscreen_led_studio_col", "habit_donut_and_apps_col"]
                    }
                    HomeModuleWidth::Full => vec![
                        "backscreen_led_studio_col",
                        "marquee_and_pattern_controls",
                        "habit_donut_and_apps_col",
                        "category_usage_bars",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("led_preview_and_top3_apps"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("led_studio_presets_and_top5_apps")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("full_led_studio_marquee_push_and_top8_apps")
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::ToolsSummaryDock => {
                let cols = match width {
                    HomeModuleWidth::OneThird => 1,
                    HomeModuleWidth::OneHalf | HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 4,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 4,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 8,
                    HomeModuleHeight::Tall => 12,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 26,
                    HomeModuleWidth::OneHalf => 40,
                    HomeModuleWidth::TwoThirds => 60,
                    HomeModuleWidth::Full => 88,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["1col_summary_tiles"],
                    HomeModuleWidth::OneHalf => vec!["2x2_summary_tiles"],
                    HomeModuleWidth::TwoThirds => {
                        vec!["2x2_summary_tiles", "quick_tool_chip_row"]
                    }
                    HomeModuleWidth::Full => vec!["4col_summary_tiles", "quick_tool_chip_row"],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("4_headline_summary_tiles"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("4_summary_tiles_with_subtitles_and_4_tool_chips")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("4_summary_tiles_with_metrics_and_full_tool_drawer")
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::ContinuityWorkspace => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 3,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 2,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 4,
                    HomeModuleHeight::Tall => 6,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 32,
                    HomeModuleWidth::OneHalf => 48,
                    HomeModuleWidth::TwoThirds => 72,
                    HomeModuleWidth::Full => 108,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["resumable_task_stack"],
                    HomeModuleWidth::OneHalf => vec!["resumable_task_stack", "resume_cta"],
                    HomeModuleWidth::TwoThirds => {
                        vec!["2col_resumable_task_grid", "artifact_badges"]
                    }
                    HomeModuleWidth::Full => {
                        vec![
                            "3col_resumable_task_grid",
                            "artifact_badges",
                            "last_turn_snippet",
                        ]
                    }
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("top2_resumable_tasks"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("4_tasks_with_turn_count_and_timestamp")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("6_tasks_with_last_assistant_reply_and_actions")
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::QuickStartRequest => {
                let cols = match width {
                    HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                    HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 4,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 4,
                    HomeModuleHeight::Tall => 8,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 32,
                    HomeModuleWidth::OneHalf => 48,
                    HomeModuleWidth::TwoThirds => 76,
                    HomeModuleWidth::Full => 112,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["prompt_input_bar", "send_button"],
                    HomeModuleWidth::OneHalf => {
                        vec!["prompt_input_bar", "send_button", "quick_template_chips"]
                    }
                    HomeModuleWidth::TwoThirds => {
                        vec!["prompt_composer_col", "2x2_quick_template_matrix_col"]
                    }
                    HomeModuleWidth::Full => vec![
                        "prompt_composer_col",
                        "2x2_quick_template_matrix_col",
                        "device_context_toggles",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("single_line_composer_and_2_chips"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("composer_and_4_template_chips")
                    }
                    HomeModuleHeight::Tall => {
                        secs.push("expanded_composer_8_templates_and_recent_history")
                    }
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::ProductivityHub | HomeModuleId::ContextLedgerHub => {
                let cols = match width {
                    HomeModuleWidth::OneThird => 1,
                    HomeModuleWidth::OneHalf | HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 3,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 1,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 3,
                    HomeModuleHeight::Tall => 5,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 36,
                    HomeModuleWidth::OneHalf => 54,
                    HomeModuleWidth::TwoThirds => 84,
                    HomeModuleWidth::Full => 120,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["summary_column", "primary_cta"],
                    HomeModuleWidth::OneHalf => vec!["master_list", "detail_preview"],
                    HomeModuleWidth::TwoThirds => {
                        vec!["master_list", "detail_preview", "evidence_sources"]
                    }
                    HomeModuleWidth::Full => vec![
                        "master_list",
                        "detail_preview",
                        "evidence_sources",
                        "cross_device_actions",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("headline_top1"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("balanced_3_items")
                    }
                    HomeModuleHeight::Tall => secs.push("deep_5_items_with_timeline"),
                }
                (cols, items, chars, secs)
            }
            HomeModuleId::WeatherModule => {
                let cols = match width {
                    HomeModuleWidth::OneThird => 1,
                    HomeModuleWidth::OneHalf | HomeModuleWidth::TwoThirds => 2,
                    HomeModuleWidth::Full => 3,
                };
                let items = match eff_h {
                    HomeModuleHeight::Compact => 3,
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => 6,
                    HomeModuleHeight::Tall => 9,
                };
                let chars = match width {
                    HomeModuleWidth::OneThird => 30,
                    HomeModuleWidth::OneHalf => 48,
                    HomeModuleWidth::TwoThirds => 74,
                    HomeModuleWidth::Full => 110,
                };
                let mut secs = match width {
                    HomeModuleWidth::OneThird => vec!["live_weather_hero", "commute_advice"],
                    HomeModuleWidth::OneHalf => {
                        vec![
                            "live_weather_hero",
                            "hourly_forecast_strip",
                            "commute_advice",
                        ]
                    }
                    HomeModuleWidth::TwoThirds => vec![
                        "live_weather_hero",
                        "hourly_forecast_strip",
                        "multi_day_forecast",
                        "click_to_carry_actions",
                    ],
                    HomeModuleWidth::Full => vec![
                        "live_weather_hero",
                        "city_quick_switcher",
                        "hourly_forecast_strip",
                        "multi_day_forecast",
                        "schedule_weather_link",
                        "click_to_carry_actions",
                    ],
                };
                match eff_h {
                    HomeModuleHeight::Compact => secs.push("compact_weather_and_3_hourly"),
                    HomeModuleHeight::Standard | HomeModuleHeight::Auto => {
                        secs.push("standard_weather_6_hourly_and_3_days")
                    }
                    HomeModuleHeight::Tall => secs.push("tall_weather_8_hourly_and_5_days"),
                }
                (cols, items, chars, secs)
            }
        };

        if height == HomeModuleHeight::Auto {
            unlocked_sections.push("auto_adaptive_height");
        }

        let show_side_detail_panel =
            matches!(width, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full);
        let show_extended_subcards = eff_h != HomeModuleHeight::Compact;

        let primary_item_budget = match (effective_density, width) {
            (HomeDensityTier::HeadlineTop1, HomeModuleWidth::OneThird) => 1,
            _ => max_items,
        };

        ModuleValueProfile {
            module_id: id.slug().to_string(),
            width: width.slug().to_string(),
            height: height.slug().to_string(),
            resolved_height: eff_h.slug().to_string(),
            columns,
            internal_columns: columns,
            max_items,
            primary_item_budget,
            char_budget,
            density_tier,
            effective_density,
            show_side_detail_panel,
            show_extended_subcards,
            unlocked_sections,
        }
    }
}
