//! Desktop shell for StepX Companion. Layout and visual rules: `desktop/DESIGN.md`.
//!
//! Module map:
//! - `theme`   design tokens (spacing, type, colour, radius) and font loading
//! - `widgets` reusable components built only from tokens
//! - `shell`   sidebar, page header and status bar
//! - `pages`   one file per page
//! - `actions` input handling and background jobs

#![allow(dead_code)]

use anyhow::Context;

mod actions;
mod app_store_state;
mod desktop_update_state;
mod release_notes_view;
mod device_actions;
mod github_stars;
mod global_search;
mod keyboard_input;
mod recent_files_state;
mod file_dialog_state;
use recent_files_state::{RecentFilesAction, RecentFilesState};
mod header_bar;
use global_search::{SearchAction, SearchState};
mod lan_cast_state;
mod outcome_apply;
#[cfg(all(feature = "ui-capture", target_os = "linux"))]
mod shell_details_check;
mod shortcuts;
use app_store_state::{AppStoreAction, AppStoreUiState};
use desktop_update_state::{DesktopUpdateAction, DesktopUpdateState};
use device_actions::parent_dir;
use keyboard_input::{Input, Keyboard};
use lan_cast_state::{LanCastAction, LanCastUiState};
use outcome_apply::{read_result, Outcome};
mod adb_onboarding;
mod agent_workbench;
mod live_calls;
mod amoo_composer;
mod amoo_display;
mod amoo_sync;
mod amoo_ui_state;
mod connection_watch;
mod battery_state;
mod google_recovery_state;
mod context_menu;
mod context_state;
mod expansion_state;
mod message_window;
mod productivity_state;
mod reply_motion;
mod speech_state;
mod voice_input_state;
mod voice_capability_state;
use voice_input_state::VoiceInputAction;
mod text_interaction;
use amoo_ui_state::{AmooUiAction, AmooUiOutcome, AmooUiState};
mod backscreen_state;
mod capsule_motion;
mod capsule_fulfillment_ui;
#[cfg(feature = "ui-capture")]
mod chat_first_check;
#[cfg(feature = "ui-capture")]
mod compact_live_check;
#[cfg(feature = "ui-capture")]
mod composer_check;
#[cfg(feature = "ui-capture")]
mod demo;
#[cfg(feature = "ui-capture")]
mod enter_check;
mod home_ai_state;
mod icon_style;
mod icons;
mod interaction;
mod led_grid;
mod mini_modes;
mod mirror_controls;
mod motion;
mod pages;
mod perf;
#[cfg(feature = "ui-capture")]
mod performance_check;
mod preview_cache;
mod push_state;
mod push_toast;
#[cfg(feature = "ui-capture")]
mod responsive_check;
#[cfg(feature = "ui-capture")]
mod responsive_live_check;
#[cfg(feature = "ui-capture")]
mod scenario_check;
#[cfg(feature = "ui-capture")]
mod settings_check;
mod settings_state;
mod shell;
mod smooth_scroll;
#[cfg(feature = "ui-capture")]
mod workspace_check;
use settings_state::{SettingsAction, SettingsUi};
pub mod auto_sync;
mod studio_lab_state;
mod studio_state;
mod theme;
mod update_state;
mod widgets;
#[cfg(feature = "ui-capture")]
mod window_memory_check;
mod work_state;
mod weather_state;
mod workspace_state;
pub(crate) use auto_sync::*;
pub mod notification_center_state;
pub(crate) use notification_center_state::*;
pub mod desktop_popup_window;
use backscreen_state::{BackScreenAction, BackScreenUiState};
use push_state::{PushAction, PushUiState, ShellMode};
use studio_lab_state::{StudioLabAction, StudioLabOutcome, StudioLabUiState};
use studio_state::{StudioAction, StudioOutcome, StudioUiState};
use update_state::{UpdateAction, UpdateState};
use work_state::{WorkAction, WorkOutcome, WorkUiState};
use workspace_state::{WorkspaceAction, WorkspaceUi};

use anyhow::Result;
use facloud_ui::{
    elements::*,
    event::{DispatchedEvent, Event},
    fonts::{Cache as FontCache, FamilyId},
    platform::{self, WindowBounds},
    AppContext, Element, Entity, SingletonEntity, View, ViewContext, ViewHandle,
};
use pathfinder_geometry::vector::{vec2f, Vector2F};
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    path::PathBuf,
    sync::{mpsc, Arc},
    time::{Duration, Instant},
};
use stepx_companion::{
    adb::Adb,
    amoo,
    apps::{self, AppInfo, AppRuntimeDetails},
    bridge::{self, AmooSnapshot, Capabilities, SmsPlan, SmsSendStatus},
    companion_app::{self, CompanionStatus},
    device::{
        default_express_parcels, default_system_services, parse_devices, BackScreenPattern,
        CallRecord, ContactEntry, Device, ExpressParcelInfo, GoogleSuiteStatus, SystemServiceInfo,
        Transport,
    },
    mirror::{MirrorAudioRoute, MirrorSession},
    openclaw::{self, RealAmooData},
    pairing::{self, Preferences, QrSession, Service},
    phone,
    sms::Message,
    storage::{self, Entry, Trashed},
    toolbox::{self, StorageReport, ToolOperation},
};
use theme::{Palette, ThemeMode};

// Indices into `Companion::fields`.
const F_PAIR_ADDR: usize = 0;
const F_PAIR_CODE: usize = 1;
const F_CONNECT: usize = 2;
const F_SMS_TO: usize = 3;
const F_SMS_BODY: usize = 4;
const F_AMOO: usize = 5;
const F_SMS_SEARCH: usize = 6;
const F_FILE_SEARCH: usize = 7;
const F_APP_SEARCH: usize = 8;
const F_FILE_NAME: usize = 9;
const F_OV_TEXT: usize = 10;
const F_BACK_PROMPT: usize = 11;
const F_BACK_MARQUEE: usize = 12;
const F_STUDIO_NAME: usize = 13;
const F_STUDIO_URL: usize = 14;
const F_PHONE_DIAL: usize = 15;
const F_CONTACT_SEARCH: usize = 16;
const F_AMOO_SEARCH: usize = 17;
const F_WORK_PROMPT: usize = 18;
const F_WORK_SEARCH: usize = 19;
const F_WORK_CAP_SEARCH: usize = 20;
const F_WORK_AGENT_NAME: usize = 21;
const F_AMOO_GUI_APP: usize = 22;
const F_STUDIO_SEARCH: usize = 23;
const F_LAB_ENDPOINT: usize = 24;
const F_LAB_KEY: usize = 25;
const F_LAB_MODEL: usize = 26;
const F_LAB_PROMPT: usize = 27;
const F_PUSH_SEARCH: usize = 28;
const F_PUSH_REPLY: usize = 29;
const F_IM_REPLY: usize = 30;
const F_TOAST_REPLY: usize = 31;
const F_PHONE_SCHEDULE_TITLE: usize = 32;
const F_AMOO_MEMORY_INPUT: usize = 33;
const F_CALENDAR_TIME: usize = 34;
const F_ALARM_TIME: usize = 35;
const F_GALLERY_SEARCH: usize = 36;
const F_STORE_SEARCH: usize = 37;
const F_ALARM_LABEL: usize = 38;
const F_PHONE_CONTACT_NAME: usize = 39;
const F_HOME_COMMAND: usize = 40;
const F_PRODUCTIVITY_NOTE: usize = 41;
const F_GLOBAL_SEARCH: usize = 42;
const F_CAPSULE_PROMPT: usize = 43;
const F_HOME_AGENT_PROMPT: usize = 44;
const F_AGENT_REQUEST: usize = 45;
const FIELD_COUNT: usize = 46;

const FILES_PER_PAGE: usize = 100;

struct DesktopAssets;
impl facloud_ui::assets::AssetProvider for DesktopAssets {
    fn get(&self, path: &str) -> Result<std::borrow::Cow<'_, [u8]>> {
        match path {
            "stepx_companion_icon.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/stepx_companion_icon.png"
            ))),
            "neo_front_frame.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/neo_front_frame.png"
            ))),
            "neo_rear_housing.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/neo_rear_housing.png"
            ))),
            "stepx_home_screen.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/stepx_home_screen.png"
            ))),
            "amoo_desktop_icon.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/stepx_companion_icon.png"
            ))),
            "amoo_icon.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/amoo_icon.png"
            ))),
            "qr_personal.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/qr_personal.png"
            ))),
            "qr_group.png" => Ok(std::borrow::Cow::Borrowed(include_bytes!(
                "../../assets/qr_group.png"
            ))),
            _ => stepx_companion::release_notes::image_asset(path).or_else(|| stepx_companion::connection_guide::image_asset(path)).or_else(|| icons::asset(path))
                .map(std::borrow::Cow::Borrowed)
                .ok_or_else(|| anyhow::anyhow!("找不到内置资源：{path}")),
        }
    }
}

pub fn run() -> Result<()> {
    let capture = std::env::args()
        .position(|arg| arg == "--capture-ui")
        .and_then(|i| std::env::args().nth(i + 1))
        .map(PathBuf::from);
    let instance_dir = if capture.is_some() || std::env::args().any(|a| a == "--verify-home-live") {
        std::env::temp_dir().join(format!("stepx-companion-capture-{}", std::process::id()))
    } else {
        stepx_companion::resources::user_config().join("stepx-companion/runtime")
    };
    let stepx_companion::single_instance::Claim::Primary(instance) =
        stepx_companion::single_instance::Instance::claim(&instance_dir)?
    else {
        return Ok(());
    };
    let activation = instance.activation_requests();
    let mut builder = platform::AppBuilder::new(
        platform::AppCallbacks {
            // The framework only tears down closed windows when this callback is set.
            on_window_will_close: Some(Box::new(|_, _| {})),
            ..Default::default()
        },
        Box::new(DesktopAssets),
        None,
    );
    #[cfg(target_os = "linux")]
    {
        use facloud_ui::platform::linux::AppBuilderExt;
        builder.set_window_class("stepx-companion".into());
        // Share XWayland coordinates with scrcpy so its sidebar can follow it.
        if std::env::var_os("DISPLAY").is_some() {
            builder.force_x11(true);
        }
        builder.set_window_icon_png(std::borrow::Cow::Borrowed(include_bytes!(
            "../../assets/stepx_companion_icon.png"
        )));
    }
    let size = mini_modes::WindowMemory::new().preferences.full;
    let (default_w, default_h) = (size.width, size.height);
    builder.run(move |ctx| {
        ctx.add_window(
            facloud_ui::AddWindowOptions {
                title: Some("StepX Companion".into()),
                // Capture runs may ask for a taller window so a long panel is
                // not cut off at the fold.
                window_bounds: WindowBounds::ExactSize(vec2f(
                    std::env::var("STEPX_CAPTURE_WIDTH")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(default_w),
                    std::env::var("STEPX_CAPTURE_HEIGHT")
                        .ok()
                        .and_then(|value| value.parse().ok())
                        .unwrap_or(default_h),
                )),
                ..Default::default()
            },
            move |ctx| Root {
                child: ctx.add_typed_action_view(|ctx| Companion::new(ctx, capture, activation)),
            },
        );
    })?;
    drop(instance);
    Ok(())
}

struct Root {
    child: ViewHandle<Companion>,
}
impl Entity for Root {
    type Event = ();
}
impl facloud_ui::TypedActionView for Root {
    type Action = ();
}
impl View for Root {
    fn ui_name() -> &'static str {
        "StepXRoot"
    }
    fn render(&self, _: &AppContext) -> Box<dyn Element> {
        ChildView::new(&self.child).finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[allow(dead_code)]
pub(crate) enum Page {
    Search,
    Home,
    Overview,
    Expansion,
    BackScreen,
    Amoo,
    Calendar,
    Schedule,
    Alarms,
    Gallery,
    Account,
    About,
    Settings,
    Work,
    Studio,
    Sms,
    Notifications,
    Phone,
    Files,
    AppStore,
    Apps,
    Tools,
    Connect,
}

#[allow(dead_code)]
impl Page {
    fn title(self) -> &'static str {
        match self {
            Self::Search => "搜索",
            Self::Home => "首页",
            Self::Overview => "终端",
            Self::Expansion => "存储扩容",
            Self::BackScreen => "背屏",
            Self::Amoo => "Amoo",
            Self::Calendar | Self::Schedule => "日历",
            Self::Alarms => "闹钟",
            Self::Gallery => "图库",
            Self::Account => "账号与用量",
            Self::About => "设置",
            Self::Settings => "设置",
            Self::Work => "文件处理",
            Self::Studio => "我的作品",
            Self::Sms => "消息",
            Self::Notifications => "消息",
            Self::Phone => "电话",
            Self::Files => "文件",
            Self::AppStore => "应用商店",
            Self::Apps => "应用",
            Self::Tools => "小工具",
            Self::Connect => "设备与同步",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Search => "⌕",
            Self::Home => "🏠",
            Self::Overview => "📱",
            Self::Expansion => "💾",
            Self::BackScreen => "❖",
            Self::Amoo => "✦",
            Self::Calendar | Self::Schedule => "📅",
            Self::Alarms => "⏰",
            Self::Gallery => "🖼",
            Self::Account => "◉",
            Self::About => "◉",
            Self::Settings => "⚙",
            Self::Work => "💼",
            Self::Studio => "⚡",
            Self::Sms => "✉",
            Self::Notifications => "🔔",
            Self::Phone => "☎",
            Self::Files => "▤",
            Self::AppStore => "🛍",
            Self::Apps => "⊞",
            Self::Tools => "⊟",
            Self::Connect => "⇄",
        }
    }

    /// Pages that need a connected phone to show anything useful.
    fn needs_device(self) -> bool {
        !matches!(
            self,
            Self::Search
                | Self::Home
                | Self::Connect
                | Self::Tools
                | Self::Calendar
                | Self::Schedule
                | Self::Alarms
                | Self::Gallery
                | Self::Work
                | Self::Amoo
                | Self::Account
                | Self::About
                | Self::Settings
                | Self::Studio
                | Self::Expansion
                | Self::AppStore
        )
    }

    /// Pages that manage their own 0-padding panes (split columns).
    fn fills_height(self) -> bool {
        matches!(
            self,
            Self::Overview
                | Self::Settings
                | Self::About
                | Self::Amoo
                | Self::Work
                | Self::Studio
                | Self::Sms
                | Self::Notifications
                | Self::Phone
                | Self::Account
        )
    }

    fn from_arg(arg: &str) -> Self {
        match arg {
            "search" => Self::Search,
            "home" | "index" | "hub" => Self::Home,
            "overview" | "terminal" | "device" => Self::Overview,
            "backscreen" | "back" => Self::BackScreen,
            "amoo" => Self::Amoo,
            "calendar" | "schedule" => Self::Calendar,
            "alarm" | "alarms" => Self::Alarms,
            "gallery" | "photos" => Self::Gallery,
            "work" | "amoo-work" => Self::Work,
            "studio" => Self::Studio,
            "sms" | "chat" | "im" => Self::Sms,
            "notifications" | "push" | "notify" => Self::Notifications,
            "phone" | "call" => Self::Phone,
            "files" => Self::Files,
            "expansion" => Self::Expansion,
            "app-store" | "store" | "appstore" => Self::AppStore,
            "apps" => Self::Apps,
            "tools" | "toolbox" | "weather" | "forecast" | "commute" => Self::Tools,
            "account" => Self::Account,
            "about" => Self::About,
            "settings" => Self::Settings,
            "pair" | "connect" => Self::Connect,
            _ => Self::Home,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
pub(crate) enum ToolPage {
    Weather,
    Updates,
    Storage,
    Google,
    Companion,
    Neo,
    Apk,
    Screen,
    Clipboard,
    McpHub,
    AiSafety,
    Wellbeing,
    Services,
    InputMethod,
    Device,
}
#[allow(dead_code)]
impl ToolPage {
    const ALL: [Self; 15] = [
        Self::Weather,
        Self::Updates,
        Self::Storage,
        Self::Google,
        Self::Companion,
        Self::Neo,
        Self::Apk,
        Self::Screen,
        Self::Clipboard,
        Self::McpHub,
        Self::AiSafety,
        Self::Wellbeing,
        Self::Services,
        Self::InputMethod,
        Self::Device,
    ];
    fn title(self) -> &'static str {
        match self {
            Self::Weather => "实时天气与出行",
            Self::Updates => "手机系统更新",
            Self::Storage => "释放空间",
            Self::Google => "Google Play",
            Self::Companion => "StepX伴侣",
            Self::Neo => "内置 GUI Agent",
            Self::Apk => "APK 安装",
            Self::Screen => "截图与投屏",
            Self::Clipboard => "双端剪贴板",
            Self::McpHub => "MCP 原生能力",
            Self::AiSafety => "端侧模型与安全",
            Self::Wellbeing => "数字健康与分身",
            Self::Services => "系统服务",
            Self::InputMethod => "手机输入法",
            Self::Device => "设备检测",
        }
    }
    fn description(self) -> &'static str {
        match self {
            Self::Weather => "查看实时气温、24小时逐时趋势与未来5日预报；点击任意天气时段即可将天气带入 Amoo 对话、日程提醒或手机背屏。",
            Self::Updates => "查询手机 OTA、测试渠道和安装条件。",
            Self::Storage => "清理缓存、重复文件，压缩文档并在需要时还原。",
            Self::Google => "检测 Google 服务，安装适配套件或打开 Play 商店。",
            Self::Companion => "安装手机伴侣，管理后台连接、扫码配对与背屏权限。",
            Self::Neo => "更新 StepX伴侣内置 GUI Agent，手机独立看屏、规划与执行。",
            Self::Apk => "从电脑选择 APK，安装到当前连接的手机。",
            Self::Screen => "本机高清投屏、局域网多协议跨机投屏（DLNA / AirPlay / Web / Miracast）与截图。",
            Self::Clipboard => "电脑与手机剪贴板实时双向秒传、验证码提取、链接跨端打开与同步记录。",
            Self::McpHub => "StepOS 20 项系统级 MCP 服务状态监测、快捷系统控制（音量/亮度/勿扰/网络）与跨应用调度。",
            Self::AiSafety => "管理端侧大模型 NPU 引擎（AIEngine）、AI 安全审计台账与四大核心感知操控权限一键修复。",
            Self::Wellbeing => "查看今日手机亮屏时长、解锁次数、高频应用可视化排行、背屏专注番茄钟与 AlphaMe 个人画像。",
            Self::Services => "查看可管理的系统服务，按需停用或恢复。",
            Self::InputMethod => "安装小企鹅中文输入法，切换默认并停用搜狗；支持恢复。",
            Self::Device => "读取设备、电量与存储信息，复制检测结果。",
        }
    }
    fn category(self) -> u8 {
        match self {
            Self::Storage => 1,
            Self::Google
            | Self::Neo
            | Self::Apk
            | Self::McpHub
            | Self::AiSafety
            | Self::Services
            | Self::InputMethod => 2,
            Self::Weather | _ => 3,
        }
    }
    fn from_arg(value: &str) -> Option<Self> {
        match value {
            "weather" | "forecast" | "commute" => Some(Self::Weather),
            "updates" => Some(Self::Updates),
            "storage" => Some(Self::Storage),
            "google" => Some(Self::Google),
            "companion" => Some(Self::Companion),
            "neo" | "gui-agent" => Some(Self::Neo),
            "apk" => Some(Self::Apk),
            "screen" => Some(Self::Screen),
            "clipboard" | "clip" => Some(Self::Clipboard),
            "mcp" | "mcp-hub" | "mcphub" => Some(Self::McpHub),
            "ai-safety" | "aisafety" | "aiengine" | "safety" => Some(Self::AiSafety),
            "wellbeing" | "alphame" | "health" => Some(Self::Wellbeing),
            "services" => Some(Self::Services),
            "input-method" => Some(Self::InputMethod),
            "device" => Some(Self::Device),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum PairTab {
    #[default]
    Qr,
    Code,
    Manual,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SmsFilter {
    #[default]
    All,
    Incoming,
    Outgoing,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum FileSort {
    #[default]
    Name,
    Size,
    Kind,
    Time,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum FileModal {
    NewFolder,
    NewTextFile,
    Rename(usize),
}

#[derive(Clone, Debug)]
#[allow(dead_code, private_interfaces)]
pub(crate) enum Action {
    ReviewConfirm,
    ReviewCancel,
    ReviewHistory,
    RetryOperation(String),
    CopyDiagnostic(String),
    MoreDiagnostics,
    ListPage(String, usize),
    StorageMaintain,
    Expansion(expansion_state::ExpansionAction),
    AmooUi(AmooUiAction),
    AmooToggleSchedules,
    AmooToggleDetails,
    AmooOpenScheduledSession(String),
    // Navigation & appearance & window
    Page(Page),
    AppStore(AppStoreAction),
    Push(PushAction),
    NotificationCenter(NotificationCenterAction),
    AutoSync(AutoSyncAction),
    Work(WorkAction),
    Workspace(WorkspaceAction),
    BackScreen(BackScreenAction),
    Updates(UpdateAction),
    DesktopUpdate(DesktopUpdateAction),
    CycleTheme,
    Settings(SettingsAction),
    Agent(agent_workbench::AgentAction),
    LiveCall(live_calls::LiveCallAction),
    QuitApplication,
    ContextRefresh,
    ScanStorage,
    OpenStorageSettings,
    ProbeSystemCompression,
    PrepareToolOperation(ToolOperation),
    ConfirmToolOperation,
    CancelToolOperation,
    LocateToolFile(String),
    ToolTab(u8),
    ToolResultPage(usize),
    MinimizeWindow,
    MaximizeWindow,
    CloseWindow,
    // Connection
    Refresh,
    Select(String),
    Forget,
    SetPreferredDevice(String),
    RemoveRememberedDevice(String),
    ConnectEndpoint(String),
    PairTab(PairTab),
    ToggleShowSecret,
    PairCode,
    Connect,
    Qr,
    // Text input
    Focus(usize),
    VoiceInput(VoiceInputAction),
    VoiceCapabilityRefresh,
    InputPointer(usize, usize, bool, u32),
    InputPointerUp,
    ExtendSelection(i8),
    MessageSelection(Option<String>),
    ClearField(usize),
    GlobalSearch(SearchAction),
    RecentFiles(RecentFilesAction),
    FileView(bool), // true: recent phone + desktop files; false: phone directory
    HeaderActions(bool),
    FocusSearch,
    NextField(bool),
    SubmitFocused,
    Escape,
    Insert(String),
    Preedit(String),
    Backspace,
    BackspaceLine,
    Delete,
    Left,
    Right,
    Home,
    End,
    SelectAll,
    CopyInput,
    UndoInput,
    RedoInput,
    ContextOpen(context_menu::Target, Vector2F),
    ContextClose,
    ContextNavigate(i8),
    ContextActivate,
    ContextRun(Box<Action>),
    Speak(String, String),
    CutInput,
    Paste,
    // Device & Overview
    Summary,
    Mirror,
    StopMirror,
    Screenshot,
    SaveScreenshot,
    Key(u32),
    ScreenTapNormalized(u16, u16),
    ScreenGesture(u8),
    ToggleLiveScreenStream,
    SetMirrorQuality(u8),
    SetMirrorAudioRoute(MirrorAudioRoute),
    CycleMirrorAudioRoute,
    ToggleMirrorAlwaysOnTop,
    ToggleMirrorTurnScreenOff,
    ToggleMirrorStayAwake,
    ToggleMirrorShowTouches,
    LanCast(LanCastAction),
    OverviewBackPreview(bool),
    SendFocusText,
    // Amoo
    Amoo,
    AmooSnapshot,
    AmooNewChat,
    AmooPrepare,
    AmooSend,
    AmooTemplate(String),
    AmooSelectCap(usize, String),
    AmooSelectAgent(usize),
    AmooSelectSession(usize),
    AmooToggleTts,
    AmooToggleVoicePlay,
    AmooToggleThinking,
    AmooPushArtifactBolt,
    AmooToggleInvoiceDetail,
    AmooConfirmSafety(bool),
    AmooRetryReply,
    AmooRetryMessage {
        session_key: String,
        text: String,
    },
    AmooToggleToolRecords(String),
    /// Generic page-level "更多" toggle keyed by string (studio / gallery menus).
    ToggleUiExpanded(String),
    AmooTogglePlus,
    AmooAttachFile(bool),
    AmooDropFiles(Vec<PathBuf>),
    AmooAttachPhoneAlbum,
    AmooRemoveAttachment(usize),
    AmooToggleRecording,
    AmooCancelRecording,
    AmooSendRecording,
    AmooToggleMemory,
    AmooCycleStyle,
    AmooCycleCommute,
    AmooAddMemory,
    AmooUseMemory(String),
    AmooToggleSkillManage,
    AmooClickSkill(usize),
    AmooCycleSafetyPolicy,
    AmooUndoSafety,
    AmooExportMemory,
    AmooOpenMemory,
    AmooOpenPersona,
    AmooOpenSafety,
    AmooOpenModels,
    AmooOpenVoiceSettings,
    AmooOpenTasks,
    AmooResetSession,
    ReadVisible,
    AppendVisibleLine(String),
    Studio(StudioAction),
    StudioLab(StudioLabAction),
    StudioOpenUrl(String),
    // SMS
    ReadSms,
    OpenSms,
    SmsFilter(SmsFilter),
    SelectSim(Option<i32>),
    SelectSmsThread(Option<String>),
    NewSms,
    PrepareDirectSms,
    ConfirmDirectSms,
    CancelDirectSms,
    CheckSmsStatus(String),
    DraftSms,
    // Phone & Contacts
    ReadPhone,
    PhoneFilterMissed(bool),
    PhoneSelectContact(usize),
    PhoneDialAppend(String),
    PhoneDialBackspace,
    PhoneDialClear,
    PhoneCall(String),
    PhoneConfirmCall,
    PhoneCancelCall,
    PhoneSelectSim(i32),
    PhoneSmsTo(String),
    // Files
    Browse(String),
    FileSort(FileSort),
    SelectFile(usize),
    ActivateFile(usize),
    OpenFile(usize),
    DownloadFile(usize),
    CompactChat,
    MoveFileSelection(isize),
    ActivateSelectedFile,
    ParentDirectory,
    OpenNewFolder,
    OpenRename,
    RenameFile(usize),
    TrashFile(usize),
    SubmitFileModal,
    CancelFileModal,
    FilePage(usize),
    Upload,
    TransferFiles(Vec<PathBuf>, Option<String>),
    Download,
    Trash,
    DeleteFile,
    ConfirmTrash,
    CancelTrash,
    Restore,
    // Apps & Privileged System Management
    Apps,
    ToggleSystemApps(bool),
    ExportApp(String),
    AppSettings(String),
    InspectApp(Option<String>),
    ToggleAppPermission {
        package: String,
        permission: String,
        grant: bool,
    },
    ToggleAppEnabled {
        package: String,
        enable: bool,
    },
    ClearAppStorage {
        package: String,
        clear_data: bool,
    },
    PreviewSelectedFile(usize),
    InstallRemoteApk(String),
    RequestUninstallApp(String),
    CancelUninstallApp,
    ConfirmUninstallApp(String),
    InstallApk,
    OpenApp(String),
    ForceStopApp(String),
    ToggleSystemService(String, bool),
    DebloatPresetServices,
    RestoreAllSystemServices,
    SwitchInputMethodFcitx5(bool),
    InspectInputMethod,
    OpenTool(Option<ToolPage>),
    ToolCategory(u8),
    PrepareUtility(Box<Action>),
    ConfirmUtility,
    CancelUtility,
    InspectServices,
    OpenInputSettings,
    GoogleSuiteInstall,
    GoogleSuiteInspect,
    GoogleSuiteMaintain,
    GoogleSuiteRollback,
    GoogleSuiteSnapshot,
    GoogleAutoRepairToggle,
    GoogleAutoRepairNow,
    CompanionInspect,
    CompanionInstall,
    NeoInspect,
    NeoExecutionMode(bool),
    NeoConnectAmoo,
    NeoInstall,
    NeoActivate,
    NeoRestore,
    NeoOpenPhone,
    NeoIme(bool),
    CompanionBackground(bool),
    CompanionSendFiles,
    CompanionReceiveFolder,
    ConnectGuide(Option<u8>),
    AdbGuide(Option<u8>),
    AdbGuideContinue,
    GrantDeepAmooPerms,
    GrantDeepBackScreenPerms,
    BackScreenToggleHwBreath,
    // Amoo Desktop Experience Enhancements
    AmooSelectModel(usize),
    AmooToggleLightbox,
    AmooToggleLightboxZoom,
    AmooSaveLightboxImage,
    AmooToggleExpressCard,
    AmooQueryExpress,
    AmooPushPickupCode(String),
    AmooTogglePcControl,
    AmooToggleBgKeepAlive,
    AmooExecPcCommand(String),
    // Phone extra
    PhoneHangup,
    PhoneCreateContact,
    PhoneDialNumber(String),
    // Files extra
    FilePushToBackScreen(String),
    RevealLocalPath(PathBuf),
    MoveHorizontalSelection(isize),
    // Cross-module human actions
    SendToAmoo(String),
    EditUserMessage(String),
    OpenAppInMirror(String),
    EnableUsbWirelessBridge,
    SwitchPreferredTransport(String),
    HealConnection,
    FillDetectedConnectionEndpoint(String),
    ConnectionManage(bool),
    ConnectionPairing(bool),
    BackupPhoneFileToDesktop(String),
    QuickGuiScenario {
        app: String,
        goal: String,
    },
    ScreenshotToBackScreen,
    OpenNewTextFile,
    // Clipboard
    Copy,
    CopyText(String, String),
    ClearContent,
    // Home Hub Actions
    HomeRefreshAll,
    HomeToggleTools,
    HomeTogglePending,
    HomeSelectAiQuadrant(usize),
    HomeResumeTask(String),
    HomeShowHistory,
    HomeOpenTaskOutput {
        task: String,
        path: PathBuf,
    },
    HomeFillDraft(String),
    HomeSubmitCommand,
    HomeRunGuiCommand,
    HomeRunWorkCommand,
    HomeQuickSmsReply {
        number: String,
        body: String,
    },
    HomeSelectReplyTarget {
        key: String,
        thread_id: String,
        package_name: String,
        title: String,
    },
    HomeExecuteIntent(stepx_companion::home_hub::HomeAiIntent),
    HomeToggleChecklistItem(String),
    HomeQuickImReply {
        key: String,
        thread_id: String,
        package_name: String,
        title: String,
        text: String,
    },
    HomeAiRefresh,
    HomeAiCancel,
    HomeModule(pages::home::HomeModuleAction),
    HomeReminder(pages::home::HomeReminderAction),
    Context(context_state::ContextAction),
    Productivity(productivity_state::ProductivityAction),
    RunDoctorHarness {
        topic: stepx_companion::doctor_harness::DoctorTopic,
        query: String,
    },
    ExecuteDoctorQuickAction(stepx_companion::doctor_harness::DoctorActionKind),
    // Weather & Commute
    WeatherSelectCity(String),
    WeatherRefresh,
    WeatherSelectDay(usize),
    WeatherBringToAmoo(Option<String>),
    WeatherBringToHomeInput(Option<String>),
    WeatherBringToSchedule(Option<String>),
    WeatherPushBackScreen,
}

type Job = Box<dyn FnOnce(&Adb) -> Result<Outcome> + Send>;

/// One conversation in the SMS list.
struct SmsThread {
    address: String,
    count: usize,
    last_ms: i64,
    last_body: String,
    unread: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct AmooAttachment {
    name: String,
    size_label: String,
    is_image: bool,
    local_path: Option<PathBuf>,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
enum SafetyDecision {
    #[default]
    Pending,
    Approved,
    Rejected,
}

#[derive(Clone, Debug, PartialEq, Eq)]
enum AmooChatItem {
    Attachment {
        name: String,
        kind: String,
        mime: String,
        url: String,
        size_bytes: Option<u64>,
        path: String,
    },
    ToolRecords {
        key: String,
        records: Vec<amoo_display::ToolRecord>,
    },
    UserText {
        attachment: Option<(String, String, bool)>,
        text: String,
    },
    UserVoice {
        transcript: String,
        duration_sec: u32,
    },
    AssistantRich {
        thinking_time: String,
        thinking_steps: Vec<String>,
        tools_summary: String,
        text: Option<String>,
        has_bolt_artifact: bool,
        has_invoice_artifact: bool,
        has_safety_confirm: bool,
        time_label: String,
    },
}

#[derive(Clone, Debug)]
struct AmooSessionItem {
    session_key: String,
    title: String,
    agent: String,
    status: String,
    status_kind: u8,
    meta: String,
    items: Vec<AmooChatItem>,
}

#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
struct AmooMemoryEntry {
    title: String,
    sub: String,
}

#[derive(Clone, Debug)]
struct AmooSkillEntry {
    skill_key: String,
    name: String,
    enabled: bool,
    prompt: String,
}

fn default_amoo_agents() -> Vec<String> {
    vec![
        "Amoo 全能助手".into(),
        "桌面交付伙伴".into(),
        "办公协同专员".into(),
        "极简总结官".into(),
    ]
}

fn default_amoo_sessions() -> Vec<AmooSessionItem> {
    vec![
        AmooSessionItem {
            session_key: "demo:1".into(),
            title: "发票入册与背屏图案生成".into(),
            agent: "Amoo 助手".into(),
            status: "待确认".into(),
            status_kind: 1,
            meta: "2 个产物 · 刚刚".into(),
            items: vec![
                AmooChatItem::UserText {
                    attachment: Some(("报销发票_1021.png".into(), "1.4 MB · 已上传".into(), true)),
                    text: "把这张发票归档到报销表，再帮我生成一个赛博闪电像素图放到手机背屏显示。".into(),
                },
                AmooChatItem::UserVoice {
                    transcript: "“顺便发短信告诉李明我马上过去”".into(),
                    duration_sec: 4,
                },
                AmooChatItem::AssistantRich {
                    thinking_time: "1.8s".into(),
                    thinking_steps: vec![
                        "1. 视觉识别「报销发票_1021.png」：提取高德打车电子发票 ¥68.00，归档至 10 月报销单（累计 ¥386.00）".into(),
                        "2. 背屏点阵生成：合成 23×17「赛博闪电」像素矩阵（391 颗 LED）".into(),
                        "3. 通讯录匹配「李明」(138 0000 9367)，拟定短信正文并触发敏感操作确认".into(),
                    ],
                    tools_summary: "归档发票 · 生成背屏图 · 拟定短信".into(),
                    text: None,
                    has_bolt_artifact: true,
                    has_invoice_artifact: true,
                    has_safety_confirm: true,
                    time_label: "刚刚".into(),
                },
            ],
        },
        AmooSessionItem {
            session_key: "demo:2".into(),
            title: "金融热点整理与周报草拟".into(),
            agent: "深度研究".into(),
            status: "执行中".into(),
            status_kind: 2,
            meta: "步骤 3/4 · 14:22".into(),
            items: vec![
                AmooChatItem::UserText {
                    attachment: Some(("评审方案_v2.pdf".into(), "640 KB · 已上传".into(), false)),
                    text: "结合这份评审方案，整理本周端侧大模型与智能硬件热点并草拟部门周报。".into(),
                },
                AmooChatItem::AssistantRich {
                    thinking_time: "3.2s".into(),
                    thinking_steps: vec![
                        "1. 解析附件「评审方案_v2.pdf」核心指标与里程碑".into(),
                        "2. 检索本周端侧 AI 与智能硬件行业动态（共 14 篇信源）".into(),
                        "3. 正在生成结构化周报草稿与下周排期建议（步骤 3/4）".into(),
                    ],
                    tools_summary: "文档解析 · 网页检索 · 周报生成".into(),
                    text: Some("已完成前 3 步信源聚合：\n• 端侧 Qwen3 混合推理延迟降低 28%，NPU 算力利用率达 82%\n• 背屏 23×17 光显矩阵新增 4 项互动场景\n• 正在为你汇总成飞书周报文档…".into()),
                    has_bolt_artifact: false,
                    has_invoice_artifact: false,
                    has_safety_confirm: false,
                    time_label: "14:22".into(),
                },
            ],
        },
        AmooSessionItem {
            session_key: "demo:3".into(),
            title: "同步飞书日历与下午会议".into(),
            agent: "飞书协同".into(),
            status: "已完成".into(),
            status_kind: 0,
            meta: "已写日历 · 11:40".into(),
            items: vec![
                AmooChatItem::UserText {
                    attachment: None,
                    text: "把下午两点 3 楼大会议室的产品评审同步到飞书日历，并提前 15 分钟在背屏闪烁提醒。".into(),
                },
                AmooChatItem::AssistantRich {
                    thinking_time: "1.1s".into(),
                    thinking_steps: vec![
                        "1. 校验飞书日历今日日程冲突：14:00 - 15:00 空闲".into(),
                        "2. 写入日程「第四季度产品评审 · 3 楼大会议室」".into(),
                        "3. 注册 13:45 背屏消息闪烁与语音提醒".into(),
                    ],
                    tools_summary: "飞书日历 · 背屏提醒".into(),
                    text: Some("已为你写入飞书日历「14:00 第四季度产品评审（3 楼大会议室）」，并已设置 13:45 手机背屏点阵闪烁提醒。".into()),
                    has_bolt_artifact: true,
                    has_invoice_artifact: false,
                    has_safety_confirm: false,
                    time_label: "11:40".into(),
                },
            ],
        },
        AmooSessionItem {
            session_key: "demo:4".into(),
            title: "整理相册截图并提取待办".into(),
            agent: "Amoo 助手".into(),
            status: "已完成".into(),
            status_kind: 0,
            meta: "4 条待办 · 昨天".into(),
            items: vec![
                AmooChatItem::UserText {
                    attachment: Some(("会议白板截图.jpg".into(), "3.2 MB · 手机相册".into(), true)),
                    text: "帮我从今天拍摄的会议白板和聊天截图里提取所有待办事项。".into(),
                },
                AmooChatItem::AssistantRich {
                    thinking_time: "2.4s".into(),
                    thinking_steps: vec![
                        "1. 扫描手机相册 /sdcard/DCIM/Screenshots 最新 6 张图片".into(),
                        "2. 端侧 OCR 识别白板手写字迹与群聊截图".into(),
                        "3. 去重并按优先级归类为 4 条待办事项".into(),
                    ],
                    tools_summary: "相册扫描 · 端侧识屏 · 待办提取".into(),
                    text: Some("已从相册最新截图中提取 4 条待办：\n1. 周三前完成背屏点阵工坊 4 款预设联调\n2. 核对 10 月差旅电子发票并归档报销单（¥386.00）\n3. 周四 14:00 参加 3 楼大会议室版本规划会\n4. 验证 OpenClaw :18789 局域网反向代理连通性".into()),
                    has_bolt_artifact: false,
                    has_invoice_artifact: true,
                    has_safety_confirm: false,
                    time_label: "昨天".into(),
                },
            ],
        },
    ]
}

fn default_amoo_skills() -> Vec<AmooSkillEntry> {
    vec![
        AmooSkillEntry {
            skill_key: "feishu-calendar".into(),
            name: "飞书日历".into(),
            enabled: true,
            prompt: "查询今日飞书日历安排，并把下午的会议同步到背屏提醒".into(),
        },
        AmooSkillEntry {
            skill_key: "wecom".into(),
            name: "企业微信".into(),
            enabled: true,
            prompt: "汇总企业微信未读工作群消息并提取核心待办".into(),
        },
        AmooSkillEntry {
            skill_key: "amap".into(),
            name: "高德出行".into(),
            enabled: true,
            prompt: "查询现在去公司的实时路况，并规划最快通勤路线".into(),
        },
        AmooSkillEntry {
            skill_key: "web-search".into(),
            name: "网页检索".into(),
            enabled: true,
            prompt: "深度检索今日科技与金融热点并整理 3 条核心摘要".into(),
        },
        AmooSkillEntry {
            skill_key: "call-helper".into(),
            name: "通话助手".into(),
            enabled: true,
            prompt: "查看最近未接来电并帮我拟定礼貌回电短信".into(),
        },
        AmooSkillEntry {
            skill_key: "backscreen-matrix".into(),
            name: "背屏点阵".into(),
            enabled: true,
            prompt: "帮我设计一个 23×17 的背屏赛博闪电像素图案并同步到背屏".into(),
        },
    ]
}

fn default_demo_call_logs() -> Vec<CallRecord> {
    let now = chrono::Local::now().timestamp_millis();
    let min = 60_000;
    vec![
        CallRecord {
            number: "138 0000 9367".into(),
            name: "李明 · 产品组".into(),
            call_type: 1,
            date_ms: now - 18 * min,
            duration_sec: 265,
        },
        CallRecord {
            number: "138 0000 0041".into(),
            name: "王磊 · 架构师".into(),
            call_type: 2,
            date_ms: now - 95 * min,
            duration_sec: 412,
        },
        CallRecord {
            number: "138 0000 9765".into(),
            name: "顺丰派件员".into(),
            call_type: 3,
            date_ms: now - 180 * min,
            duration_sec: 0,
        },
        CallRecord {
            number: "138 0000 1985".into(),
            name: "张晓楠 · 设计组".into(),
            call_type: 1,
            date_ms: now - 420 * min,
            duration_sec: 148,
        },
    ]
}

fn default_demo_contacts() -> Vec<ContactEntry> {
    vec![
        ContactEntry {
            name: "李明 · 产品组".into(),
            number: "138 0000 9367".into(),
        },
        ContactEntry {
            name: "王磊 · 架构师".into(),
            number: "138 0000 0041".into(),
        },
        ContactEntry {
            name: "张晓楠 · 设计组".into(),
            number: "138 0000 1985".into(),
        },
        ContactEntry {
            name: "陈思远 · 硬件研发".into(),
            number: "138 0000 0362".into(),
        },
        ContactEntry {
            name: "顺丰速运园区网点".into(),
            number: "138 0000 9765".into(),
        },
    ]
}

struct Companion {
    file_dialog: file_dialog_state::FileDialogState,
    search: SearchState,
    recent_files: RecentFilesState,
    header_actions_open: bool,
    font: FamilyId,
    mono_font: FamilyId,
    theme_mode: ThemeMode,
    settings: SettingsUi,
    shortcuts: shortcuts::Shortcuts,
    github_stars: github_stars::GithubStars,
    desktop_update: DesktopUpdateState,
    interaction: interaction::InteractionUi,
    motion_started: Instant,
    capsule_motion: std::rc::Rc<RefCell<capsule_motion::Carousel>>,
    fulfillment: capsule_fulfillment_ui::FulfillmentUi,
    agent_workbench: agent_workbench::AgentWorkbench,
    live_calls: live_calls::LiveCallUi,
    sub_motion_started: Instant,
    job_started: Instant,
    palette: Cell<Palette>,
    page: Page,
    compact: mini_modes::CompactWindow,
    window_memory: mini_modes::WindowMemory,
    compact_start_pending: bool,
    // Connection
    devices: Vec<Transport>,
    mdns_services: Vec<Service>,
    selected: Option<Device>,
    pub(crate) last_connected_serial: Option<String>,
    phone_link: Option<stepx_companion::companion_link::Link>,
    expansion: expansion_state::ExpansionUi,
    phone_link_notice: Option<(String, bool)>,
    preferences: Preferences,
    prefs: Preferences,
    allow_first_connect: bool,
    pairing_hosts: Vec<std::net::IpAddr>,
    pair_tab: PairTab,
    connection_manage_open: bool,
    connection_pairing_open: bool,
    connect_guide_step: Option<u8>,
    adb_guide_step: Option<u8>,
    adb_guide_scroll: ClippedScrollStateHandle,
    show_secret: bool,
    qr: Option<Arc<QrSession>>,
    // Device & Overview
    summary: String,
    storage_report: Option<StorageReport>,
    tools_pending: Option<ToolOperation>,
    tool_page: Option<ToolPage>,
    tool_category: u8,
    tool_confirmation: Option<Box<Action>>,
    tools_feedback: Option<(Option<ToolPage>, String, bool)>,
    job_tool: Option<ToolPage>,
    google_progress: Arc<std::sync::Mutex<String>>,
    google_recovery: google_recovery_state::GoogleRecoveryUi,
    tools_tab: u8,
    tools_result_page: usize,
    system_compression: Option<toolbox::SystemCompression>,
    tools_last_result: Option<String>,
    capabilities: Option<Capabilities>,
    mirror: Option<MirrorSession>,
    pub(super) mirror_interrupted_at: Option<Instant>,
    mirror_controls: Option<mirror_controls::ControlWindow>,
    pub(super) mirror_options: stepx_companion::mirror::MirrorOptions,
    pub(super) lan_cast: LanCastUiState,
    pub(super) live_screen_stream: bool,
    pub(super) last_live_screen_poll: Instant,
    pub(super) live_screen_rx: Option<mpsc::Receiver<(tempfile::TempDir, PathBuf)>>,
    screenshot: Option<(tempfile::TempDir, PathBuf)>,
    overview_back_preview: bool,
    // BackScreen (23×17 Dot-Matrix)
    back_screen: BackScreenUiState,
    updates: UpdateState,
    matrix_marquee_on: bool,
    // Amoo Work
    work: WorkUiState,
    workspace: WorkspaceUi,
    // Amoo
    demo_mode: bool,
    amoo_gateway_version: String,
    amoo_gateway_online: bool,
    amoo_cron_count: usize,
    amoo_cron_jobs: Vec<openclaw::RealCronInfo>,
    amoo_show_schedules: bool,
    amoo_show_details: bool,
    amoo_ui: AmooUiState,
    amoo_bm25_entries: usize,
    amoo_total_skills: usize,
    amoo_snapshot: Option<AmooSnapshot>,
    amoo_poll_remaining: u8,
    amoo_pending_reply: HashMap<String, amoo::PendingReply>,
    last_amoo_poll: Instant,
    amoo_agents: Vec<String>,
    amoo_agent_idx: usize,
    amoo_session_idx: usize,
    amoo_sessions: Vec<AmooSessionItem>,
    amoo_expanded_tool_records: std::collections::HashSet<String>,
    ui_expanded: std::collections::HashSet<String>,
    amoo_cap_idx: usize,
    amoo_tts_on: bool,
    amoo_voice_playing: bool,
    amoo_thinking_expanded: bool,
    amoo_bolt_pushed: bool,
    amoo_invoice_open: bool,
    amoo_safety_decision: SafetyDecision,
    amoo_plus_open: bool,
    amoo_recording: bool,
    amoo_record_secs: u32,
    amoo_attachments: Vec<AmooAttachment>,
    amoo_memory_enabled: bool,
    amoo_style_idx: usize,
    amoo_commute_idx: usize,
    amoo_memories: Vec<AmooMemoryEntry>,
    amoo_skills: Vec<AmooSkillEntry>,
    amoo_skill_manage: bool,
    amoo_safety_policy_idx: usize,
    amoo_guard_count: usize,
    // Studio & Reverse Proxy
    studio: StudioUiState,
    studio_lab: StudioLabUiState,
    // Push Notifications & IM Chat Hub & Compact Modes
    push: PushUiState,
    // SMS
    sms: Vec<Message>,
    pub(crate) local_sent_sms: Vec<Message>,
    sms_loaded: bool,
    phone_loaded: bool,
    sms_error: Option<String>,
    call_logs_error: Option<String>,
    contacts_error: Option<String>,
    sms_filter: SmsFilter,
    selected_thread: Option<String>,
    pub(crate) read_sms_addresses: std::collections::HashSet<String>,
    selected_sim: Option<i32>,
    pending_sms_plan: Option<SmsPlan>,
    last_sms_status: Option<SmsSendStatus>,
    // Phone & Contacts
    call_logs: Vec<CallRecord>,
    contacts: Vec<ContactEntry>,
    selected_contact: usize,
    call_filter_missed: bool,
    active_call_number: Option<String>,
    pending_call: Option<phone::DialPlan>,
    selected_call_sim: Option<i32>,
    // Files
    recent_files_view: bool,
    directory: String,
    files: Vec<Entry>,
    file_page: usize,
    file_sort: FileSort,
    selected_file: Option<usize>,
    pub(super) selected_file_preview_path: Option<String>,
    pub(super) selected_file_preview_text: Option<String>,
    pub(super) selected_file_preview_image: Option<PathBuf>,
    file_modal: Option<FileModal>,
    trash_confirm: bool,
    permanent_delete: bool,
    trashed: Option<Trashed>,
    // Apps & Privileged System Management
    pub(super) app_store: AppStoreUiState,
    apps: Vec<AppInfo>,
    apps_loaded: bool,
    apps_error: Option<String>,
    pub(super) selected_app: Option<String>,
    pub(super) selected_app_details: Option<AppRuntimeDetails>,
    pub(super) selected_app_loading: bool,
    pending_app_uninstall: Option<String>,
    include_system_apps: bool,
    google_suite: GoogleSuiteStatus,
    companion_status: CompanionStatus,
    neo_status: stepx_companion::neo_agent::NeoStatus,
    neo_enable_ime: bool,
    system_services: Vec<SystemServiceInfo>,
    fcitx5_active: bool,
    input_method: stepx_companion::input_method::Status,
    deep_amoo_granted: bool,
    deep_backscreen_granted: bool,
    backscreen_hw_breath: bool,
    // Amoo Desktop Experience Enhancements
    amoo_model_idx: usize,
    amoo_lightbox_open: bool,
    amoo_lightbox_zoom: u8,
    amoo_express_open: bool,
    express_parcels: Vec<ExpressParcelInfo>,
    amoo_pc_control_enabled: bool,
    amoo_bg_keepalive: bool,
    // Job output
    content: String,
    output_page: Page,
    job_page: Page,
    // Status & jobs
    status: String,
    /// Last observed `status` text and when it first appeared; used to make
    /// non-error status messages transient (auto-cleared after a few seconds).
    status_seen: (String, Instant),
    error: bool,
    busy: bool,
    initializing: bool,
    loading_frame: usize,
    sender: mpsc::Sender<Job>,
    receiver: mpsc::Receiver<Result<Outcome>>,
    bg_receivers: Vec<(Page, Option<ToolPage>, mpsc::Receiver<Result<Outcome>>)>,
    pub(super) bulk_rx: Option<mpsc::Receiver<Outcome>>,
    pub(super) bulk_busy: bool,
    pub(super) bulk_status: String,
    pub(super) bulk_page: Page,
    pub(super) bulk_tool: Option<ToolPage>,
    pub(super) bulk_started_at: Option<Instant>,
    last_scan: Instant,
    connection_watch: connection_watch::ConnectionWatch,
    battery: battery_state::BatteryUi,
    text_selection: SelectionHandle,
    selected_text: Option<String>,
    context_menu: Option<context_menu::Menu>,
    reply_motion: reply_motion::ReplyMotion,
    speech: speech_state::SpeechUi,
    voice_input: voice_input_state::VoiceInputUi,
    voice_capability: voice_capability_state::VoiceCapabilityUi,
    // Input
    fields: Vec<Input>,
    focus: Option<usize>,
    // View state
    scroll: ClippedScrollStateHandle,
    scroll_aux: ClippedScrollStateHandle,
    push_list_scroll: ClippedScrollStateHandle,
    push_detail_scroll: ClippedScrollStateHandle,
    sms_threads_scroll: ClippedScrollStateHandle,
    sms_messages_scroll: ClippedScrollStateHandle,
    sms_detail_scroll: ClippedScrollStateHandle,
    hover_states: RefCell<HashMap<String, (u64, MouseStateHandle)>>,
    render_epoch: Cell<u64>,
    chat_scroll_metrics: std::rc::Rc<std::cell::Cell<(f32, f32)>>,
    chat_tail_requested: std::rc::Rc<std::cell::Cell<bool>>,
    wheel_motions: RefCell<HashMap<(&'static str, u32), smooth_scroll::WheelHandle>>,
    capture: Option<PathBuf>,
    capture_at: Instant,
    last_clock_second: u64,
    activation_requests: Arc<std::sync::atomic::AtomicBool>,
    svg_slots: home_ai_state::SvgSlots,
    home_ai: home_ai_state::HomeAiUi,
    home_tools_expanded: bool,
    home_pending_expanded: bool,
    pub(super) home_modules: pages::home::HomeModuleLayoutConfig,
    pub(super) home_module_edit_mode: bool,
    pub(super) home_module_filter: Option<pages::home::HomeModuleCategory>,
    pub(super) home_ai_quadrant: usize,
    pub(super) home_checked_checklist_items: std::collections::BTreeSet<String>,
    pub(super) home_pending_move: Option<pages::home::HomeModuleId>,
    pub(super) home_edit_anim_started: Instant,
    pub(super) home_pointer_registry:
        std::rc::Rc<std::cell::RefCell<pages::home::HomePointerRegistry>>,
    pub(crate) reminder_governance:
        stepx_companion::home_reminder_governance::HomeReminderGovernance,
    pub(crate) home_reminder_drawer_open: bool,
    pub(crate) home_reminder_drawer_tab: u8,
    context: context_state::ContextUi,
    productivity: productivity_state::ProductivityUi,
    pub(crate) notification_center: notification_center_state::NotificationCenterState,
    pub(crate) desktop_popup: desktop_popup_window::DesktopPopupController,
    pub(crate) auto_sync: auto_sync::AutoSyncState,
    pub(super) doctor_report: Option<stepx_companion::doctor_harness::DoctorReport>,
    pub(super) doctor_running: bool,
    pub(in crate::ui) weather_snapshot: stepx_companion::weather::WeatherSnapshot,
    pub(in crate::ui) weather_selected_day: usize,
    weather_fetch: weather_state::WeatherFetchState,
}

fn is_stepx_model(model: &str) -> bool {
    let m = model.to_lowercase();
    m.contains("pz261a") || m.contains("stepx") || m.contains("pandora")
}

fn is_wireless(transport: &str) -> bool {
    transport.contains(':') || transport.contains("_tcp")
}

/// Reaps any zombie `ps` child process left behind by Winit's startup
/// `look_for_wayland_compositor()` probe on Linux (`<defunct>`).
#[cfg(target_os = "linux")]
fn reap_orphan_startup_zombies() {
    let pid = std::process::id();
    let children_path = format!("/proc/{pid}/task/{pid}/children");
    let Ok(children) = std::fs::read_to_string(children_path) else {
        return;
    };
    for token in children.split_whitespace() {
        let Ok(child_pid) = token.parse::<i32>() else {
            continue;
        };
        if child_pid <= 0 {
            continue;
        }
        let status_path = format!("/proc/{child_pid}/status");
        let Ok(status) = std::fs::read_to_string(status_path) else {
            continue;
        };
        let is_ps = status.lines().any(|l| l == "Name:\tps");
        let is_zombie = status.lines().any(|l| l.starts_with("State:\tZ"));
        if is_ps && is_zombie {
            unsafe {
                libc::waitpid(child_pid, std::ptr::null_mut(), libc::WNOHANG);
            }
        }
    }
}

impl Companion {
    #[cfg(test)]
    pub(super) fn new_for_test() -> Self {
        // Initialize the shared library/UI test directory before any settings load.
        let _ = ui_user_config_dir();
        Self::build_initial(
            FamilyId(0),
            FamilyId(1),
            None,
            Arc::new(std::sync::atomic::AtomicBool::new(false)),
        )
    }

    fn build_initial(
        font: FamilyId,
        mono_font: FamilyId,
        capture: Option<PathBuf>,
        activation_requests: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        #[cfg(target_os = "linux")]
        std::thread::spawn(|| {
            for delay_ms in [150u64, 500, 1500] {
                std::thread::sleep(Duration::from_millis(delay_ms));
                reap_orphan_startup_zombies();
            }
        });
        let (sender, jobs) = mpsc::channel::<Job>();
        let (results, receiver) = mpsc::channel();
        std::thread::spawn(move || {
            while let Ok(job) = jobs.recv() {
                let outcome = Adb::discover().and_then(|adb| job(&adb));
                if results.send(outcome).is_err() {
                    break;
                }
            }
        });
        let (preferences, warning) = match Preferences::load() {
            Ok(p) => (p, None),
            Err(e) => (Preferences::default(), Some(e.to_string())),
        };
        let mut theme_mode = match std::env::var("STEPX_THEME").ok().as_deref() {
            Some("dark") => ThemeMode::Dark,
            Some("light") => ThemeMode::Light,
            _ => match stepx_companion::settings::load().appearance {
                stepx_companion::settings::Appearance::System => ThemeMode::System,
                stepx_companion::settings::Appearance::Light => ThemeMode::Light,
                stepx_companion::settings::Appearance::Dark => ThemeMode::Dark,
            },
        };
        if std::env::args().any(|arg| arg == "--dark") {
            theme_mode = ThemeMode::Dark;
        } else if std::env::args().any(|arg| arg == "--light") {
            theme_mode = ThemeMode::Light;
        }
        // Paint the home/loading state before device discovery or ADB work.
        let devices: Vec<Transport> = Vec::new();
        let page_arg = std::env::args()
            .position(|arg| arg == "--page")
            .and_then(|i| std::env::args().nth(i + 1))
            .or_else(|| std::env::var("STEPX_PAGE").ok());
        let tool_from_cli = std::env::args()
            .position(|a| a == "--tool")
            .and_then(|i| std::env::args().nth(i + 1))
            .or_else(|| std::env::var("STEPX_TOOL").ok())
            .and_then(|a| ToolPage::from_arg(&a))
            .or_else(|| {
                page_arg
                    .as_deref()
                    .filter(|p| matches!(*p, "weather" | "forecast" | "commute"))
                    .and_then(ToolPage::from_arg)
            });
        let page = page_arg.as_deref().map(Page::from_arg).unwrap_or(
            if std::env::args().any(|a| a == "--pair-qr") {
                Page::Connect
            } else if std::env::args().any(|a| a == "--mini-ai") {
                Page::Amoo
            } else if std::env::args().any(|a| a == "--mini-notify") {
                Page::Notifications
            } else if tool_from_cli.is_some() {
                Page::Tools
            } else {
                Page::Home
            },
        );
        let status = warning.clone().unwrap_or_else(|| {
            if devices.is_empty() {
                "未连接设备".into()
            } else {
                format!("发现 {} 台设备", devices.len())
            }
        });
        let mut fields: Vec<Input> = (0..FIELD_COUNT).map(|_| Input::default()).collect();
        if let Ok(q) = std::env::var("STEPX_STORE_QUERY") {
            if !q.trim().is_empty() {
                fields[F_STORE_SEARCH].set(q.trim());
            }
        }
        fields[F_BACK_PROMPT].set("让猫咪眨眼陪我工作，亮度 4 级，播放 30 秒");
        fields[F_BACK_MARQUEE].set("STEPX AI");
        fields[F_CALENDAR_TIME].set("15:00");
        fields[F_ALARM_TIME].set("08:00");
        let settings = SettingsUi::default();
        fields[F_CAPSULE_PROMPT].set(&settings.value.capsule_agent_prompt);
        fields[F_HOME_AGENT_PROMPT].set(&settings.value.home_agent_prompt);

        let demo_mode = cfg!(feature = "ui-capture") && std::env::args().any(|a| a == "--demo");
        let mut s = Self {
            search: SearchState::default(),
            recent_files: RecentFilesState::default(),
            file_dialog: file_dialog_state::FileDialogState::default(),
            header_actions_open: false,
            font,
            mono_font,
            theme_mode,
            settings,
            shortcuts: Default::default(),
            github_stars: Default::default(),
            desktop_update: Default::default(),
            interaction: Default::default(),
            motion_started: Instant::now(),
            capsule_motion: Default::default(),
            fulfillment: Default::default(),
            agent_workbench: Default::default(),
            live_calls: Default::default(),
            sub_motion_started: Instant::now(),
            job_started: Instant::now(),
            palette: Cell::new(Palette::for_theme(false)),
            page,
            compact: Default::default(),
            window_memory: mini_modes::WindowMemory::new(),
            compact_start_pending: mini_modes::WindowMemory::new().startup_mode()
                != ShellMode::Full,
            devices,
            mdns_services: vec![],
            selected: None,
            last_connected_serial: preferences.selected_serial.clone(),
            phone_link: None,
            expansion: Default::default(),
            phone_link_notice: None,
            allow_first_connect: true,
            connect_guide_step: if preferences.first_connect_done {
                None
            } else {
                Some(0)
            },
            adb_guide_scroll: Default::default(),
            adb_guide_step: (!demo_mode && preferences.should_show_adb_guide()).then_some(0),
            prefs: preferences.clone(),
            preferences,
            pairing_hosts: vec![],
            pair_tab: PairTab::Qr,
            connection_manage_open: false,
            connection_pairing_open: false,
            show_secret: false,
            qr: None,
            summary: String::new(),
            storage_report: None,
            tools_pending: None,
            tool_page: tool_from_cli,
            tool_category: 0,
            tool_confirmation: None,
            tools_feedback: None,
            job_tool: None,
            google_progress: Default::default(),
            google_recovery: Default::default(),
            tools_tab: 0,
            tools_result_page: 0,
            system_compression: None,
            tools_last_result: None,
            capabilities: None,
            mirror: None,
            mirror_interrupted_at: None,
            mirror_controls: None,
            mirror_options: stepx_companion::mirror::MirrorOptions::default(),
            lan_cast: LanCastUiState::default(),
            live_screen_stream: false,
            last_live_screen_poll: Instant::now(),
            live_screen_rx: None,
            screenshot: None,
            overview_back_preview: std::env::var_os("STEPX_BACK_PREVIEW").is_some(),
            back_screen: BackScreenUiState::default(),
            updates: UpdateState::default(),
            matrix_marquee_on: true,
            demo_mode,
            work: WorkUiState::default(),
            workspace: WorkspaceUi::default(),
            amoo_gateway_version: String::new(),
            amoo_gateway_online: false,
            amoo_cron_count: 0,
            amoo_cron_jobs: Vec::new(),
            amoo_show_schedules: std::env::args().any(|arg| arg == "--amoo-tasks"),
            amoo_show_details: std::env::args().any(|arg| arg == "--amoo-details"),
            amoo_ui: AmooUiState::default(),
            amoo_bm25_entries: 0,
            amoo_total_skills: 0,
            amoo_snapshot: None,
            amoo_poll_remaining: 0,
            amoo_pending_reply: HashMap::new(),
            last_amoo_poll: Instant::now(),
            amoo_expanded_tool_records: Default::default(),
            ui_expanded: Default::default(),
            amoo_agents: default_amoo_agents(),
            amoo_agent_idx: 0,
            amoo_session_idx: 0,
            amoo_sessions: if demo_mode {
                default_amoo_sessions()
            } else {
                Vec::new()
            },
            amoo_cap_idx: 1,
            amoo_tts_on: false,
            amoo_voice_playing: false,
            amoo_thinking_expanded: false,
            amoo_bolt_pushed: false,
            amoo_invoice_open: false,
            amoo_safety_decision: SafetyDecision::Pending,
            amoo_plus_open: std::env::var_os("STEPX_AMOO_INTERACT").is_some(),
            amoo_recording: std::env::var_os("STEPX_AMOO_INTERACT").is_some(),
            amoo_record_secs: 5,
            amoo_attachments: if demo_mode {
                vec![
                    AmooAttachment {
                        name: "活动海报.jpg".into(),
                        size_label: "2.1M".into(),
                        is_image: true,
                        local_path: None,
                    },
                    AmooAttachment {
                        name: "评审方案_v2.pdf".into(),
                        size_label: "640K".into(),
                        is_image: false,
                        local_path: None,
                    },
                ]
            } else {
                Vec::new()
            },
            amoo_memory_enabled: true,
            amoo_style_idx: 0,
            amoo_commute_idx: 0,
            amoo_memories: if demo_mode || cfg!(test) {
                vec![
                    AmooMemoryEntry {
                        title: "Agent 称呼用户为正法，交流语气简洁直接".into(),
                        sub: "交流偏好 · 手机 Amoo 记忆同步".into(),
                    },
                    AmooMemoryEntry {
                        title: "偏好消费低、人口多、车程 5 小时以内的自驾县城".into(),
                        sub: "关于我 · 出行与生活画像".into(),
                    },
                    AmooMemoryEntry {
                        title: "高德打车默认开具电子发票 · 每周五 16:00 提醒汇总周报".into(),
                        sub: "备忘事项 · 办公自动化习惯".into(),
                    },
                ]
            } else {
                Vec::new()
            },
            amoo_skills: if demo_mode || cfg!(test) {
                default_amoo_skills()
            } else {
                Vec::new()
            },
            amoo_skill_manage: false,
            amoo_safety_policy_idx: 0,
            amoo_guard_count: if demo_mode { 12 } else { 0 },
            studio: StudioUiState::default(),
            studio_lab: StudioLabUiState::default(),
            push: PushUiState::default(),
            sms: Vec::new(),
            local_sent_sms: Vec::new(),
            sms_loaded: demo_mode,
            phone_loaded: demo_mode,
            sms_error: None,
            call_logs_error: None,
            contacts_error: None,
            sms_filter: SmsFilter::All,
            selected_thread: None,
            read_sms_addresses: std::collections::HashSet::new(),
            selected_sim: None,
            pending_sms_plan: None,
            last_sms_status: None,
            call_logs: if demo_mode {
                default_demo_call_logs()
            } else {
                Vec::new()
            },
            contacts: if demo_mode {
                default_demo_contacts()
            } else {
                Vec::new()
            },
            selected_contact: 0,
            call_filter_missed: false,
            pending_call: None,
            selected_call_sim: None,
            active_call_number: if demo_mode {
                Some("138 0000 9367".into())
            } else {
                None
            },
            recent_files_view: true,
            directory: "/sdcard".into(),
            files: vec![],
            file_page: 0,
            file_sort: FileSort::Name,
            selected_file: None,
            selected_file_preview_path: None,
            selected_file_preview_text: None,
            selected_file_preview_image: None,
            file_modal: None,
            trash_confirm: false,
            permanent_delete: false,
            trashed: None,
            app_store: AppStoreUiState::new(),
            apps: vec![],
            apps_loaded: false,
            apps_error: None,
            selected_app: None,
            selected_app_details: None,
            selected_app_loading: false,
            pending_app_uninstall: None,
            include_system_apps: false,
            google_suite: if demo_mode {
                GoogleSuiteStatus::default_verified()
            } else {
                GoogleSuiteStatus::default()
            },
            companion_status: CompanionStatus::default(),
            neo_status: Default::default(),
            neo_enable_ime: false,
            system_services: if demo_mode {
                default_system_services()
            } else {
                Vec::new()
            },
            fcitx5_active: demo_mode,
            input_method: Default::default(),
            deep_amoo_granted: demo_mode,
            deep_backscreen_granted: demo_mode,
            backscreen_hw_breath: false,
            amoo_model_idx: 0,
            amoo_lightbox_open: std::env::var_os("STEPX_AMOO_LIGHTBOX").is_some()
                || std::env::args().any(|a| a == "--amoo-lightbox"),
            amoo_lightbox_zoom: if std::env::var_os("STEPX_AMOO_LIGHTBOX").is_some()
                || std::env::args().any(|a| a == "--amoo-lightbox")
            {
                150
            } else {
                100
            },
            amoo_express_open: demo_mode,
            express_parcels: if demo_mode {
                default_express_parcels()
            } else {
                Vec::new()
            },
            amoo_pc_control_enabled: demo_mode,
            amoo_bg_keepalive: demo_mode,
            content: String::new(),
            output_page: Page::Overview,
            job_page: Page::Overview,
            status,
            status_seen: (String::new(), Instant::now()),
            error: warning.is_some(),
            busy: false,
            initializing: !demo_mode,
            loading_frame: 0,
            sender,
            receiver,
            bg_receivers: Vec::new(),
            bulk_rx: None,
            bulk_busy: false,
            bulk_status: String::new(),
            bulk_page: Page::Overview,
            bulk_tool: None,
            bulk_started_at: None,
            last_scan: Instant::now(),
            connection_watch: Default::default(),
            battery: Default::default(),
            text_selection: Default::default(),
            selected_text: None,
            context_menu: None,
            reply_motion: Default::default(),
            speech: Default::default(),
            voice_input: Default::default(),
            voice_capability: Default::default(),
            fields,
            focus: None,
            scroll: Default::default(),
            scroll_aux: Default::default(),
            push_list_scroll: Default::default(),
            push_detail_scroll: Default::default(),
            sms_threads_scroll: Default::default(),
            sms_messages_scroll: Default::default(),
            sms_detail_scroll: Default::default(),
            hover_states: RefCell::new(HashMap::new()),
            render_epoch: Cell::new(0),
            wheel_motions: Default::default(),
            chat_scroll_metrics: Default::default(),
            chat_tail_requested: Default::default(),
            capture,
            capture_at: Instant::now(),
            last_clock_second: 0,
            activation_requests,
            svg_slots: Default::default(),
            home_ai: Default::default(),
            home_tools_expanded: true,
            home_pending_expanded: false,
            home_modules: pages::home::HomeModuleLayoutConfig::default(),
            home_module_edit_mode: std::env::var_os("STEPX_HOME_EDIT_MODULES").is_some()
                || std::env::args().any(|a| a == "--home-edit-modules"),
            home_module_filter: None,
            home_ai_quadrant: 0,
            home_checked_checklist_items: std::collections::BTreeSet::new(),
            home_pending_move: None,
            home_edit_anim_started: Instant::now(),
            home_pointer_registry: Default::default(),
            reminder_governance: if cfg!(test) {
                let gov =
                    stepx_companion::home_reminder_governance::HomeReminderGovernance::default();
                stepx_companion::home_reminder_governance::set_active_governance(&gov);
                gov
            } else {
                stepx_companion::home_reminder_governance::HomeReminderGovernance::load()
            },
            home_reminder_drawer_open: false,
            home_reminder_drawer_tab: 0,
            context: context_state::ContextUi::new(!demo_mode && !cfg!(test)),
            productivity: Default::default(),
            notification_center: Default::default(),
            desktop_popup: Default::default(),
            auto_sync: auto_sync::AutoSyncState::new(),
            doctor_report: demo_mode.then(|| {
                stepx_companion::doctor_harness::default_demo_report(
                    stepx_companion::doctor_harness::DoctorTopic::GooglePlayGuide,
                    "google play 安装不了，不会用",
                )
            }),
            doctor_running: false,
            weather_snapshot: if cfg!(test) {
                stepx_companion::weather::WeatherSnapshot::default_for_city("上海")
            } else {
                stepx_companion::weather::cached_weather_snapshot().unwrap_or_else(||
                    stepx_companion::weather::WeatherSnapshot::default_for_city("上海"))
            },
            weather_selected_day: 0,
            weather_fetch: Default::default(),
        };
        if !cfg!(test) {
            s.load_amoo_persona_state();
            s.load_home_checked_checklist_items();
            s.load_home_modules_config();
        }
        if let Some(preset_name) = std::env::var("STEPX_HOME_PRESET").ok().or_else(|| {
            std::env::args()
                .position(|a| a == "--home-preset")
                .and_then(|i| std::env::args().nth(i + 1))
        }) {
            if let Some(preset) = pages::home::HomeModulePreset::from_cli_slug(&preset_name) {
                s.home_modules = pages::home::HomeModuleLayoutConfig::from_preset(preset);
            }
        }
        #[cfg(feature = "ui-capture")]
        s.load_context_capture_fixture();
        if s.demo_mode && !s.push.loaded {
            s.init_demo_push();
        }
        s.reconcile_auto_sync_domains_from_state();
        s.sync_notification_center();
        s
    }

    fn new(
        ctx: &mut ViewContext<Self>,
        capture: Option<PathBuf>,
        activation_requests: Arc<std::sync::atomic::AtomicBool>,
    ) -> Self {
        let (font, mono_font) =
            FontCache::handle(ctx).update(ctx, |cache, _| theme::load_fonts(cache));
        #[cfg(feature = "ui-capture")]
        let demo_mode = std::env::args().any(|a| a == "--demo");
        let mut me = Self::build_initial(font, mono_font, capture, activation_requests);
        #[cfg(feature = "ui-capture")]
        if me.page == Page::Expansion && std::env::args().any(|a| a == "--expansion-settings") {
            me.expansion.settings = true;
        }
        #[cfg(feature = "ui-capture")]
        if let Ok(tab_str) = std::env::var("STEPX_SETTINGS_TAB") {
            if let Ok(tab_idx) = tab_str.parse::<u8>() {
                me.settings.tab = tab_idx;
            }
        }
        #[cfg(feature = "ui-capture")]
        if let Ok(preview_mode) = std::env::var("STEPX_POPUP_PREVIEW") {
            let mode = match preview_mode.trim().to_ascii_lowercase().as_str() {
                "silent" | "none" | "0" => stepx_companion::desktop_notify::PopupMode::Silent,
                "banner" | "alert" | "1" => stepx_companion::desktop_notify::PopupMode::Banner,
                _ => stepx_companion::desktop_notify::PopupMode::FullReply,
            };
            me.push.prefs.popup_mode = mode;
            me.settings.notify_draft.popup_mode = mode;
            me.notification_center.alert_mode = match mode {
                stepx_companion::desktop_notify::PopupMode::Silent => NotificationAlertMode::Muted,
                stepx_companion::desktop_notify::PopupMode::Banner
                | stepx_companion::desktop_notify::PopupMode::FullReply => {
                    NotificationAlertMode::PopupAndBadge
                }
            };
        }
        #[cfg(feature = "ui-capture")]
        if me.page == Page::Studio {
            if let Some(i) = std::env::args().position(|a| a == "--studio-source") {
                me.studio.source = match std::env::args().nth(i + 1).as_deref() {
                    Some("phone") => 1,
                    Some("saved") => 2,
                    _ => 0,
                };
            }
            if let Some(i) = std::env::args().position(|a| a == "--studio-section") {
                me.studio.tab = match std::env::args().nth(i + 1).as_deref() {
                    Some("create") => 2,
                    Some("settings") => 3,
                    _ => 0,
                };
            }
            if demo_mode && std::env::args().any(|a| a == "--verify-studio-key-mask") {
                // Deliberately non-secret fixture; never load real keys into a capture.
                me.studio.tab = 3;
                me.fields[F_LAB_KEY].set("MASK_TEST_NOT_SECRET");
                me.fields[F_LAB_KEY].preedit = "输入测试".into();
                me.focus = Some(F_LAB_KEY);
            }
        }
        me.back_screen
            .set_preview_visible(me.page == Page::BackScreen);
        if me.page == Page::Amoo {
            me.reveal_latest_chat();
        } else if me.page == Page::Sms {
            Self::scroll_to_end(&me.scroll_aux);
        }
        let has_stepx_candidate = me
            .devices
            .iter()
            .any(|d| d.state == "device" && is_stepx_model(&d.model));
        #[cfg(feature = "ui-capture")]
        if let Some(panel) = std::env::args()
            .position(|a| a == "--studio-tab")
            .and_then(|i| std::env::args().nth(i + 1))
        {
            me.studio.tab = match panel.as_str() {
                "work" => 2,
                "settings" => 3,
                _ => 0,
            };
            if demo_mode && me.page == Page::Studio {
                me.studio_lab.loaded = true;
                me.studio_lab.artifacts =
                    stepx_companion::studio_lab::list_artifacts().unwrap_or_default();
                me.studio_lab.selected = (!me.studio_lab.artifacts.is_empty()).then_some(0);
                if let Some(a) = me.studio_lab.artifacts.first() {
                    if let Ok(report) = stepx_companion::studio_lab::read_report(a) {
                        me.studio_lab.report = Some((a.id.clone(), report));
                    }
                }
            }
        }
        #[cfg(feature = "ui-capture")]
        if let Some(px) = std::env::var("STEPX_CAPTURE_SCROLL")
            .ok()
            .and_then(|v| v.parse::<f32>().ok())
        {
            me.scroll.scroll_to(facloud_ui::units::Pixels::new(px));
        }
        #[cfg(feature = "ui-capture")]
        if std::env::args().any(|a| a == "--amoo-screen-view") {
            me.amoo_ui.screen_mode = true;
            me.amoo_ui.gui_desktop = true;
            me.fields[F_AMOO_GUI_APP].set("设置");
            me.fields[F_AMOO].set("打开关于手机，查看 Android 版本，不修改设置");
        }
        #[cfg(feature = "ui-capture")]
        if std::env::args().any(|a| a == "--overview-screen-menu") {
            me.context_menu = Some(context_menu::Menu {
                target: context_menu::Target::OverviewCard(
                    context_menu::OverviewContextKind::ScreenPreview,
                ),
                at: Vector2F::new(260., 180.),
                text: None,
                selected: Some(0),
            });
        }
        #[cfg(feature = "ui-capture")]
        if std::env::args().any(|a| a == "--demo") {
            me.load_demo();
            me.sync_demo_auto_sync_domains();
            me.sync_notification_center();
            if let Some(index) = std::env::args().position(|a| a == "--adb-guide-preview") {
                me.adb_guide_step = Some(
                    std::env::args()
                        .nth(index + 1)
                        .and_then(|s| s.parse::<u8>().ok())
                        .unwrap_or(0)
                        .min(2),
                );
            }
            if let Some(index) = std::env::args().position(|arg| arg == "--ota-report") {
                if let Some(path) = std::env::args().nth(index + 1) {
                    if let Ok(bytes) = std::fs::read(path) {
                        if let Ok(report) =
                            serde_json::from_slice::<stepx_companion::ota::OtaReport>(&bytes)
                        {
                            me.updates.phone = Some(stepx_companion::updates::PhoneVersion {
                                serial: report.serial.clone(),
                                firmware: report.current_version.clone(),
                                security_patch: "—".into(),
                                has_update_page: true,
                            });
                            me.updates.report = Some(report);
                        }
                    }
                }
            }
            ctx.focus_self();
            me.spawn_live_weather_fetch("", ctx);
            me.initialize_shortcut(ctx);
            me.initialize_stars(ctx);
            me.initialize_desktop_updates(ctx);
            me.schedule_tick(ctx);
            return me;
        }
        if has_stepx_candidate {
            me.auto_select();
        } else {
            me.refresh();
        }
        me.sync_notification_center();
        ctx.focus_self();
        me.spawn_live_weather_fetch("", ctx);
        me.initialize_shortcut(ctx);
        me.initialize_stars(ctx);
        me.initialize_desktop_updates(ctx);
        me.schedule_tick(ctx);
        me
    }

    fn schedule_tick(&self, ctx: &mut ViewContext<Self>) {
        let interval = if self.desktop_update.whats_new_open && self.desktop_update.video_playing {
            65
        } else if self.reply_motion.active()
            && !self.settings.hidden
            && matches!(self.page, Page::Amoo | Page::Work)
        {
            12
        } else if self.search.open
            || self.search.closing.is_some()
            || self.page == Page::Search
            || std::env::var_os("STEPX_SCENARIO_DIR").is_some()
        {
            // Live acceptance polls real phone workers too. Running the entire
            // tick at 60 Hz can starve native input/present events in debug builds.
            100
        } else if self.mirror.is_some()
            || self.live_screen_stream
            || self.live_screen_rx.is_some()
            || self.busy
            || self.bulk_busy
            || self.work.running
            || self.workspace.running.is_some()
            || self.studio_lab.handle.is_some()
            || self.amoo_ui.gui_running
            || self.work_previews_loading()
            || self.amoo_ui.snapshot_job.is_some()
            || self.app_store.has_active_work()
            || (self.page == Page::BackScreen && self.back_screen.preview_running)
            || self.svg_slots.has_pending()
            || (self.capture.is_some()
                && !self.svg_slots.ready_for_capture(self.page_requires_svg()))
            || self.home_ai.running()
            || self.context.running()
            || self.auto_sync.is_syncing()
            || self.doctor_running
        {
            100
        } else {
            500
        };
        ctx.spawn(
            async move {
                facloud_ui::r#async::Timer::after(Duration::from_millis(interval)).await;
            },
            |me, _, ctx| {
                me.tick(ctx);
                me.schedule_tick(ctx);
            },
        );
    }

    fn page_requires_svg(&self) -> bool {
        match self.page {
            Page::Home => {
                self.context_home_is_focused()
                    || self.home_modules.enabled.iter().any(|m| m.uses_svg())
            }
            Page::Overview | Page::Gallery => self.selected.is_some(),
            Page::Tools => {
                self.selected.is_some()
                    && matches!(
                        self.tool_page,
                        Some(ToolPage::McpHub | ToolPage::AiSafety | ToolPage::Wellbeing)
                    )
            }
            _ => false,
        }
    }

    fn mouse_state(&self, id: impl Into<String>) -> MouseStateHandle {
        let key = id.into();
        let epoch = self.render_epoch.get();
        let mut states = self.hover_states.borrow_mut();
        if let Some((last_epoch, _)) = states.get(&key) {
            if *last_epoch == epoch {
                let alt_key = format!("{key}#alt");
                let alt_entry = states.entry(alt_key).or_default();
                alt_entry.0 = epoch;
                return alt_entry.1.clone();
            }
        }
        let entry = states.entry(key).or_default();
        entry.0 = epoch;
        entry.1.clone()
    }

    pub(super) fn demote_busy_if_background(&mut self) {
        if self.busy
            && !self.initializing
            && self.selected.is_some()
            && !matches!(self.job_page, Page::Amoo | Page::Work)
            && self.google_progress.lock().is_ok_and(|p| p.is_empty())
        {
            let (_tx, dummy_rx) = mpsc::channel();
            let old_rx = std::mem::replace(&mut self.receiver, dummy_rx);
            self.bg_receivers
                .push((self.job_page, self.job_tool, old_rx));
            self.busy = false;
            self.loading_frame = 0;
        }
    }

    pub(super) fn submit(
        &mut self,
        label: &str,
        job: impl FnOnce(&Adb) -> Result<Outcome> + Send + 'static,
    ) {
        let (tx, rx) = mpsc::channel();
        if self.busy {
            let old_rx = std::mem::replace(&mut self.receiver, rx);
            self.bg_receivers
                .push((self.job_page, self.job_tool, old_rx));
        } else {
            self.receiver = rx;
        }
        if matches!(self.page, Page::Amoo | Page::Work) {
            self.amoo_ui.snapshot_generation = self.amoo_ui.snapshot_generation.wrapping_add(1);
        }
        self.busy = true;
        self.job_page = self.page;
        self.job_started = Instant::now();
        self.loading_frame = 0;
        self.job_tool = self.tool_page;
        self.error = false;
        self.status = label.into();
        let mut operation = self
            .interaction
            .active_operation
            .clone()
            .unwrap_or_else(|| {
                interaction::OperationFailure::for_action(
                    &Action::ReviewHistory,
                    self.page,
                    self.selected.as_ref().map(|d| d.serial.clone()),
                )
            });
        operation.label = label
            .trim_start_matches("正在")
            .trim_end_matches(['…', '.', ' '])
            .to_string();
        std::thread::spawn(move || {
            let outcome = Adb::discover()
                .and_then(|adb| job(&adb))
                .with_context(|| operation);
            let _ = tx.send(outcome);
        });
    }

    pub(super) fn with_device(
        &mut self,
        label: &str,
        job: impl FnOnce(&Adb, Device) -> Result<Outcome> + Send + 'static,
    ) {
        match self.selected.clone() {
            Some(mut device) => self.submit(label, move |adb| {
                device.transport = device.resolve_active_transport(adb)?;
                job(adb, device)
            }),
            None => {
                self.page = Page::Connect;
                self.status = "请先连接设备".into();
                self.error = true;
            }
        }
    }

    pub(super) fn spawn_bulk(
        &mut self,
        status: impl Into<String>,
        f: impl FnOnce() -> Outcome + Send + 'static,
    ) {
        let label = status.into();
        let (tx, rx) = mpsc::channel();
        self.bulk_rx = Some(rx);
        self.bulk_busy = true;
        self.bulk_status = label.clone();
        self.bulk_page = self.page;
        self.bulk_tool = self.tool_page;
        self.bulk_started_at = Some(Instant::now());
        self.error = false;
        if !self.busy {
            self.status = label;
        }
        std::thread::spawn(move || {
            let _ = tx.send(f());
        });
    }

    pub(super) fn with_device_bulk(
        &mut self,
        label: &str,
        job: impl FnOnce(&Adb, Device) -> Result<Outcome> + Send + 'static,
    ) {
        let Some(mut device) = self.selected.clone() else {
            self.page = Page::Connect;
            self.status = "请先连接设备".into();
            self.error = true;
            return;
        };
        let page = self.page;
        let tool = self.tool_page;
        let label_str = label.to_string();
        let mut operation = self
            .interaction
            .active_operation
            .clone()
            .unwrap_or_else(|| {
                interaction::OperationFailure::for_action(
                    &Action::ReviewHistory,
                    self.page,
                    self.selected.as_ref().map(|d| d.serial.clone()),
                )
            });
        operation.label = label
            .trim_start_matches("正在")
            .trim_end_matches(['…', '.', ' '])
            .to_string();
        self.spawn_bulk(label_str, move || {
            let result = Adb::discover()
                .and_then(|adb| {
                    device.transport = device.resolve_active_transport(&adb)?;
                    job(&adb, device)
                })
                .with_context(|| operation);
            match result {
                Ok(outcome) => outcome,
                Err(error) => Outcome::JobFailed(page, tool, error),
            }
        });
    }

    pub(super) fn is_demo_device(&self) -> bool {
        self.demo_mode
            || self
                .selected
                .as_ref()
                .is_some_and(|d| d.serial == "STEPX_DEMO_DEVICE")
    }

    /// Status-bar messages are transient: a non-error, non-busy message is
    /// reset to the idle state ~6s after it first appeared.
    fn tick_status_expiry(&mut self, ctx: &mut ViewContext<Self>) {
        const STATUS_TTL: Duration = Duration::from_secs(6);
        if self.status_seen.0 != self.status {
            self.status_seen = (self.status.clone(), Instant::now());
            return;
        }
        if self.error || self.busy || self.initializing || self.bulk_busy {
            self.status_seen.1 = Instant::now();
            return;
        }
        if self.status.is_empty() || self.status == "就绪" {
            return;
        }
        if self.status_seen.1.elapsed() >= STATUS_TTL {
            self.status = "就绪".into();
            self.status_seen = (self.status.clone(), Instant::now());
            ctx.notify();
        }
    }

    fn tick(&mut self, ctx: &mut ViewContext<Self>) {
        self.poll_file_dialog(ctx);
        self.poll_storage_maintenance(ctx);
        self.poll_search(ctx);
        self.tick_capsule_fulfillment(ctx);
        self.tick_agent_workbench(ctx);
        self.tick_live_calls(ctx);
        self.poll_recent_files(ctx);
        let _timing = perf::Span::new("tick");
        #[cfg(all(feature = "ui-capture", target_os = "linux"))]
        self.verify_text_interaction(ctx);
        #[cfg(all(feature = "ui-capture", target_os = "linux"))]
        self.verify_shell_details(ctx);
        self.poll_desktop_update(ctx);
        self.tick_speech(ctx);
        self.tick_voice_capability(ctx);
        self.tick_reply_motion(ctx);
        self.tick_status_expiry(ctx);
        self.tick_connection_watch(ctx);
        self.tick_battery(ctx);
        self.tick_google_recovery(ctx);
        self.tick_weather_fetch(ctx);
        self.ensure_calendar_state_scope();
        self.tick_settings(ctx);
        self.tick_home(ctx);
        self.tick_context(ctx);
        self.tick_productivity(ctx);
        #[cfg(feature = "ui-capture")]
        self.verify_settings(ctx);
        self.remember_window(ctx, false);
        #[cfg(feature = "ui-capture")]
        self.verify_window_memory(ctx);
        if self
            .activation_requests
            .swap(false, std::sync::atomic::Ordering::AcqRel)
        {
            let id = ctx.window_id();
            self.settings.hidden = false;
            ctx.windows().show_window_and_focus_app(id);
        }
        let link_matches = self
            .selected
            .as_ref()
            .zip(self.phone_link.as_ref())
            .is_some_and(|(device, link)| {
                device.serial == link.serial && device.transport == link.transport
            });
        if !self.is_demo_device() && !link_matches {
            self.phone_link = None;
            if let Some(device) = self.selected.clone() {
                if let Ok(adb) = Adb::discover() {
                    self.phone_link =
                        Some(stepx_companion::companion_link::Link::start(adb, device));
                }
            }
        }
        if let Some(event) = self.phone_link.as_ref().and_then(|link| link.event()) {
            if !event.phone_work.is_empty() {
                for mut incoming in event.phone_work {
                    if let Some(existing) = self
                        .workspace
                        .journal
                        .tasks
                        .iter_mut()
                        .find(|t| t.id == incoming.id)
                    {
                        incoming.draft = existing.draft.clone();
                        incoming.attachments = existing.attachments.clone();
                        *existing = incoming;
                    } else {
                        self.workspace.journal.tasks.push(incoming);
                    }
                }
                self.workspace_persist();
            }
            if !event.message.is_empty() {
                if !self.busy {
                    self.status = event.message.clone();
                }
                self.phone_link_notice = Some((event.message, event.error));
            }
            ctx.notify();
        }
        if self.tick_expansion()
            && matches!(self.page, Page::Home | Page::Overview | Page::Expansion)
        {
            ctx.notify();
        }
        let mut preview_visible = self.page == Page::BackScreen && !self.settings.hidden;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(window) =
            facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id())
        {
            preview_visible &= !window.is_minimized().unwrap_or(false);
        }
        if self.poll_backscreen(preview_visible) {
            ctx.notify();
        }
        if self.poll_updates() && self.page == Page::Tools {
            ctx.notify();
        }
        if self.tick_lan_cast() {
            ctx.notify();
        }
        if self.poll_app_store(ctx) {
            ctx.notify();
        }
        if !self.is_demo_device()
            && (self.page == Page::AppStore
                || self.apps_loaded
                || self.auto_sync.initial_wave_completed())
        {
            self.ensure_store_online_catalog_synced();
        }
        if !self.is_demo_device()
            && (self.page == Page::AppStore
                || self.apps_loaded
                || self.auto_sync.initial_wave_completed())
            && self.apps_loaded
            && !self.apps.is_empty()
            && !self.app_store.catalog.is_empty()
            && !self.app_store.update_checked_once
            && !self.app_store.checking_updates
        {
            self.handle_app_store_action(AppStoreAction::CheckUpdates, ctx);
        }
        if self.poll_work() && matches!(self.page, Page::Home | Page::Work | Page::Amoo) {
            ctx.notify();
        }
        if self.poll_amoo_gui() && self.page == Page::Amoo {
            ctx.notify();
        }
        if !self.is_demo_device()
            && self.page == Page::Home
            && !self.auto_sync.enabled
            && self.selected.is_some()
            && !self.busy
            && !self
                .bg_receivers
                .iter()
                .any(|(p, _, _)| matches!(p, Page::Home | Page::Sms | Page::Overview))
            && (self.summary.is_empty()
                || (!self.sms_loaded && self.sms_error.is_none())
                || (!self.phone_loaded && self.call_logs_error.is_none()))
        {
            self.auto_load_for_page();
            ctx.notify();
        } else if !self.is_demo_device()
            && self.page == Page::Overview
            && self.selected.is_some()
            && !self.busy
            && self.summary.is_empty()
        {
            self.handle_device_action(&Action::Summary);
            ctx.notify();
        }
        if !self.is_demo_device()
            && matches!(self.page, Page::Home | Page::Overview)
            && self.selected.is_some()
            && !self.amoo_ui.usage_attempted
        {
            self.start_account_usage_sync();
            ctx.notify();
        }
        let stepos_stale_on_home = self.page == Page::Home
            && self.focus.is_none()
            && self.bg_receivers.is_empty()
            && self.back_screen.stepos_suite_error.is_none()
            && self
                .back_screen
                .stepos_suite_synced_at
                .is_some_and(|t| t.elapsed() >= Duration::from_secs(180));
        if !self.is_demo_device()
            && matches!(
                self.page,
                Page::Home
                    | Page::Overview
                    | Page::Tools
                    | Page::Calendar
                    | Page::Schedule
                    | Page::Alarms
            )
            && self.selected.is_some()
            && (!self.back_screen.stepos_suite.checked || stepos_stale_on_home)
            && !self.back_screen.stepos_suite_loading
        {
            self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
        }
        if !self.is_demo_device()
            && self.page == Page::Home
            && self.selected.is_some()
            && !self.push.loaded
            && !self.push.syncing
            && self.push.error.is_none()
        {
            self.handle_push_action(&PushAction::Refresh, ctx);
        }
        if !self.is_demo_device()
            && self.page == Page::Tools
            && self.tool_page == Some(ToolPage::Screen)
            && self.selected.is_some()
        {
            if !self.lan_cast.is_running() && self.lan_cast.status_note.is_none() {
                if let Ok(stats) = self.ensure_lan_cast_server_started() {
                    self.lan_cast.status_note = Some((
                        format!(
                            "局域网多协议投屏网关已启动：{} · MPEG-TS {}",
                            stats.web_url, stats.ts_url
                        ),
                        false,
                    ));
                    ctx.notify();
                }
            }
            if !self.lan_cast.scanning
                && self
                    .lan_cast
                    .discovery
                    .as_ref()
                    .is_none_or(|d| d.scanned_at_hms.is_empty())
            {
                self.trigger_lan_cast_discovery();
                ctx.notify();
            }
        }
        if !self.is_demo_device()
            && self.page == Page::Gallery
            && self.selected.is_some()
            && !self.back_screen.gallery_checked
            && !self.back_screen.gallery_loading
        {
            self.handle_backscreen_action(&BackScreenAction::GalleryRefresh, ctx);
        }
        self.tick_auto_clipboard_sync(ctx);
        if self.poll_account_usage_sync() {
            ctx.notify();
        }
        if self.push.syncing
            && !self.push.loaded
            && self.push.sync_started.elapsed() > Duration::from_millis(150)
            && self.push.sync_started.elapsed() < Duration::from_millis(750)
            && matches!(self.page, Page::Home | Page::Notifications | Page::Sms)
        {
            ctx.notify();
        }
        if self.page == Page::Amoo {
            self.demote_busy_if_background();
        }
        if self.busy
            && self.loading_frame == 0
            && self.job_started.elapsed() > Duration::from_millis(150)
        {
            self.loading_frame = 1;
            ctx.notify();
        }
        while let Ok(result) = self.receiver.try_recv() {
            let completed_page = self.job_page;
            let completed_tool = self.job_tool;
            let tool_result = result.as_ref().map_or(true, |outcome| {
                !matches!(
                    outcome,
                    Outcome::Transports(..) | Outcome::Selected(..) | Outcome::Reattached(..)
                )
            });
            self.busy = false;
            if let Ok(mut progress) = self.google_progress.lock() {
                progress.clear();
            }
            self.error = result.is_err();
            if let Err(e) = &result {
                self.record_operation_error(e);
            }
            match result {
                Err(e) => {
                    self.initializing = false;
                    if matches!(self.job_page, Page::Apps | Page::AppStore) {
                        self.apps_loaded = true;
                        self.apps_error = Some(format!("{e:#}"));
                    }
                    // A transient read error must not discard an accepted request.
                    if matches!(self.job_page, Page::Amoo | Page::Work) {
                        self.amoo_gateway_online = false;
                    }
                    if self.job_page == Page::Work {
                        self.work.catalog_loading = false;
                        self.work.error = Some(format!("{e:#}"));
                    }
                    self.status = self.connection_failure_status(&format!("{e:#}"));
                    if self.workspace.running.as_ref().is_some_and(|r| {
                        !r.accepted
                            && matches!(
                                r.executor,
                                stepx_companion::amoo_workspace::Executor::Phone
                                    | stepx_companion::amoo_workspace::Executor::Image
                            )
                    }) {
                        self.workspace_finish(
                            stepx_companion::amoo_workspace::Phase::Failed,
                            self.status.clone(),
                        );
                    }
                }
                Ok(outcome) => {
                    self.status = "就绪".into();
                    self.apply(outcome);
                    if self.bulk_busy && !self.bulk_status.is_empty() && self.status == "就绪" {
                        self.status = self.bulk_status.clone();
                    }
                }
            }
            if completed_page == Page::Tools
                && completed_tool.is_some()
                && tool_result
                && !self.busy
            {
                self.tools_feedback = Some((completed_tool, self.status.clone(), self.error));
            }
            if !self.busy
                && self.selected.is_some()
                && !(self.page == Page::Home && self.auto_sync.enabled)
                && (self.page != completed_page
                    || (matches!(self.page, Page::Home | Page::Overview)
                        && self.summary.is_empty()))
            {
                self.auto_load_for_page();
            }
            ctx.notify();
        }
        let mut bg_idx = 0;
        while bg_idx < self.bg_receivers.len() {
            match self.bg_receivers[bg_idx].2.try_recv() {
                Ok(result) => {
                    let (completed_page, completed_tool, _) = self.bg_receivers.swap_remove(bg_idx);
                    match result {
                        Ok(outcome) => {
                            let prev_status = self.status.clone();
                            let prev_error = self.error;
                            self.apply(outcome);
                            if self.busy || self.page == Page::Amoo {
                                self.status = prev_status;
                                self.error = prev_error;
                            }
                            if completed_page == Page::Tools
                                && completed_tool.is_some()
                                && !self.busy
                            {
                                self.tools_feedback =
                                    Some((completed_tool, self.status.clone(), false));
                            }
                        }
                        Err(e) => {
                            self.initializing = false;
                            self.record_operation_error(&e);
                            let err_str = format!("{e:#}");
                            if matches!(completed_page, Page::Apps | Page::AppStore) {
                                self.apps_loaded = true;
                                self.apps_error = Some(err_str.clone());
                            }
                            self.on_bg_job_failed(completed_page, &err_str);
                        }
                    }
                    ctx.notify();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.bg_receivers.swap_remove(bg_idx);
                }
                Err(mpsc::TryRecvError::Empty) => {
                    bg_idx += 1;
                }
            }
        }
        if let Some(rx) = &self.bulk_rx {
            match rx.try_recv() {
                Ok(outcome) => {
                    let completed_page = self.bulk_page;
                    let completed_tool = self.bulk_tool;
                    self.bulk_busy = false;
                    self.bulk_started_at = None;
                    self.bulk_rx = None;
                    self.bulk_tool = None;
                    let prev_bulk_status = std::mem::take(&mut self.bulk_status);
                    if !self.busy {
                        self.error = false;
                        self.status = "就绪".into();
                    }
                    self.apply(outcome);
                    if completed_page == Page::Tools
                        && completed_tool.is_some()
                        && !self.busy
                        && self.status != prev_bulk_status
                    {
                        self.tools_feedback =
                            Some((completed_tool, self.status.clone(), self.error));
                    }
                    ctx.notify();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.bulk_busy = false;
                    self.bulk_started_at = None;
                    self.bulk_rx = None;
                    self.bulk_tool = None;
                    self.bulk_status.clear();
                    ctx.notify();
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if let Some(rx) = &self.live_screen_rx {
            match rx.try_recv() {
                Ok((dir, path)) => {
                    if self.amoo_ui.screen_mode
                        || self.amoo_ui.gui_running
                        || self.amoo_ui.gui_paused
                    {
                        self.amoo_ui.gui_screenshot = Some(path.clone());
                    }
                    self.screenshot = Some((dir, path));
                    self.live_screen_rx = None;
                    ctx.notify();
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.live_screen_rx = None;
                }
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        if !self.is_demo_device()
            && !self.settings.hidden
            && self.selected.is_some()
            && self.live_screen_rx.is_none()
            && ((self.screenshot.is_none()
                && matches!(self.page, Page::Overview | Page::Tools | Page::Amoo))
                || (self.live_screen_stream
                    && matches!(self.page, Page::Overview | Page::Tools | Page::Amoo)
                    && self.last_live_screen_poll.elapsed() >= Duration::from_millis(1200))
                || (!self.busy
                    && self.screenshot.is_none()
                    && matches!(self.page, Page::Overview | Page::Tools)
                    && self.last_live_screen_poll.elapsed() >= Duration::from_millis(1500)))
        {
            if let Some(device) = self.selected.clone() {
                self.last_live_screen_poll = Instant::now();
                let fast_jpeg = self
                    .lan_cast
                    .server
                    .as_ref()
                    .map(|srv| srv.latest_jpeg_bytes())
                    .filter(|b| !b.is_empty());
                let (tx, rx) = mpsc::channel();
                self.live_screen_rx = Some(rx);
                std::thread::spawn(move || {
                    if let Some(jpeg) = fast_jpeg {
                        if let Ok(dir) = tempfile::tempdir() {
                            let path = dir.path().join("screen.jpg");
                            if std::fs::write(&path, jpeg).is_ok() {
                                let _ = tx.send((dir, path));
                                return;
                            }
                        }
                    }
                    if let Ok(adb) = Adb::discover() {
                        let transport = device
                            .resolve_active_transport(&adb)
                            .unwrap_or_else(|_| device.transport.clone());
                        if let Ok(bytes) = adb.screenshot(&transport) {
                            if let Ok(dir) = tempfile::tempdir() {
                                let path = dir.path().join("screen.png");
                                if std::fs::write(&path, bytes).is_ok() {
                                    let _ = tx.send((dir, path));
                                }
                            }
                        }
                    }
                });
            }
        }
        if self.poll_amoo_sync() {
            ctx.notify();
        }
        if self.page == Page::Studio && !self.studio.loaded && !self.busy {
            self.refresh_studio();
        }
        if self.tick_studio_lab() && matches!(self.page, Page::Studio | Page::Amoo) {
            ctx.notify();
        }
        self.tick_studio_cloud();
        if self.tick_workspace(ctx) {
            ctx.notify();
        }
        if self.compact_start_pending {
            self.compact_start_pending = false;
            let mode = self.window_memory.startup_mode();
            self.set_compact_mode(mode, ctx);
        }
        self.tick_push_sync(ctx);
        self.tick_auto_sync(ctx);
        self.tick_notification_center();
        if let Some(code) = self.notification_center.pending_clipboard_otp.take() {
            ctx.clipboard()
                .write(facloud_ui::clipboard::ClipboardContent::plain_text(
                    code.clone(),
                ));
            self.status = format!("已自动复制验证码 {code} 到剪贴板");
        }
        let now_popup_inst = Instant::now();
        let popup_elapsed_ms = self
            .notification_center
            .last_popup_timer_tick
            .map(|prev| {
                (now_popup_inst.saturating_duration_since(prev).as_millis() as u32).clamp(1, 1000)
            })
            .unwrap_or(16);
        self.notification_center.last_popup_timer_tick = Some(now_popup_inst);
        if self.tick_notification_popup_timer(popup_elapsed_ms) {
            ctx.notify();
        }
        self.sync_desktop_popup_window(ctx);
        if let Some(child) = &mut self.mirror {
            if let Ok(Some(status)) = child.try_wait() {
                let interrupted = !status.success();
                self.error = interrupted;
                self.status = if status.success() {
                    "投屏已结束".into()
                } else {
                    format!("投屏已中断：{}", child.exit_detail(status))
                };
                self.mirror = None;
                self.tools_feedback =
                    Some((Some(ToolPage::Screen), self.status.clone(), self.error));
                if interrupted && self.selected.is_some() {
                    self.mirror_interrupted_at = Some(Instant::now());
                    self.begin_connection_watch();
                }
                ctx.notify();
            }
        }
        #[cfg(feature = "ui-capture")]
        self.verify_ui_performance(ctx);
        #[cfg(feature = "ui-capture")]
        self.verify_compact_window(ctx);
        #[cfg(feature = "ui-capture")]
        self.verify_compact_live(ctx);
        #[cfg(feature = "ui-capture")]
        self.verify_composer(ctx);
        #[cfg(feature = "ui-capture")]
        self.verify_workspace(ctx);
        #[cfg(feature = "ui-capture")]
        self.verify_home_live(ctx);
        self.sync_mirror_controls(ctx);
        #[cfg(feature = "ui-capture")]
        self.verify_mirror_controls(ctx);
        let clock_second = self.capture_at.elapsed().as_secs();
        let clock_changed = clock_second != self.last_clock_second;
        self.last_clock_second = clock_second;
        let active_page_task_animating = !self.animations_disabled()
            && ((matches!(self.page, Page::Amoo | Page::Work)
                && (self.workspace.running.is_some()
                    || self.work.running
                    || self.studio_lab.handle.is_some()
                    || !self.amoo_pending_reply.is_empty()))
                || (self.page == Page::Studio && self.studio_lab.handle.is_some()));
        if active_page_task_animating {
            ctx.notify();
        } else if clock_changed && self.page == Page::Amoo && self.workspace.running.is_some() {
            ctx.notify();
        }
        if clock_changed && self.qr.is_some() && self.page == Page::Connect {
            // Countdown.
            ctx.notify();
        }
        if clock_changed && self.amoo_recording && self.capture.is_none() {
            self.amoo_record_secs = (self.amoo_record_secs + 1).min(59);
            ctx.notify();
        }
        let selected_session_empty = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .is_some_and(|s| s.items.is_empty());
        let overview_needs_amoo_init = matches!(self.page, Page::Home | Page::Overview)
            && self.selected.is_some()
            && !self.amoo_gateway_online
            && self.amoo_sessions.is_empty();
        let needs_initial_amoo_fetch = (self.page == Page::Amoo
            && (self.amoo_sessions.is_empty()
                || (selected_session_empty && self.amoo_poll_remaining > 0)))
            || (overview_needs_amoo_init && self.amoo_poll_remaining > 0);
        let sync_pending = self.workspace_sync_pending_keys();
        let amoo_fg_busy = self.busy && matches!(self.job_page, Page::Amoo | Page::Work);
        if !amoo_fg_busy
            && !self.demo_mode
            && self.selected.is_some()
            && self.amoo_ui.snapshot_job.is_none()
            && !self.work.running
            && self.last_amoo_poll.elapsed()
                >= if needs_initial_amoo_fetch {
                    Duration::from_millis(400)
                } else if self.amoo_poll_remaining > 0 || !self.amoo_pending_reply.is_empty() {
                    Duration::from_millis(if !self.settings.hidden && self.chat_waiting() {
                        1200
                    } else {
                        2400
                    })
                } else if !sync_pending.is_empty() || matches!(self.page, Page::Amoo | Page::Work) {
                    if sync_pending.is_empty() && !self.amoo_gateway_online {
                        Duration::from_secs(30)
                    } else {
                        Duration::from_secs(10)
                    }
                } else {
                    Duration::from_secs(180)
                }
        {
            self.amoo_poll_remaining = self.amoo_poll_remaining.saturating_sub(1);
            self.last_amoo_poll = Instant::now();
            // 文件处理页有自己的会话，停在该页时优先同步它的回复。
            let work_key = (self.page == Page::Work && self.workspace.running.is_none())
                .then(|| self.work_session().map(|s| s.session_key.clone()))
                .flatten();
            let key = work_key.or_else(|| self.current_amoo_sync_key());
            self.begin_amoo_sync(key, sync_pending, needs_initial_amoo_fetch);
        }
        if !self.busy
            && self.amoo_ui.snapshot_job.is_none()
            && self.last_scan.elapsed() > Duration::from_secs(5)
        {
            if self.page == Page::Connect
                && self.pair_tab == PairTab::Qr
                && (self.selected.is_none()
                    || self.connection_pairing_open
                    || self.connect_guide_step.is_some())
                && self.connect_guide_step.is_none_or(|step| step >= 4)
            {
                if let Some(qr) = self.qr.clone() {
                    if qr.expired() {
                        self.qr = QrSession::new().ok().map(Arc::new);
                        ctx.notify();
                    } else {
                        self.last_scan = Instant::now();
                        self.submit("等待扫码…", move |adb| {
                            Ok(Outcome::Qr(qr.try_pair(adb)?))
                        });
                    }
                } else if !self.error {
                    self.refresh();
                }
            }
        }
        if !self.demo_mode && self.last_scan.elapsed() > Duration::from_secs(5) {
            self.last_scan = Instant::now();
            self.begin_connection_watch();
        }
        #[cfg(feature = "ui-capture")]
        if std::env::args().any(|a| a == "--verify-google-cert") && !self.initializing {
            static SENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SENT.swap(true, std::sync::atomic::Ordering::SeqCst) {
                // Land on Tools → Google so the certification panel is on screen
                // when --capture-ui fires.
                self.page = Page::Tools;
                self.tool_page = Some(ToolPage::Google);
                eprintln!("GOOGLE_CERT_UI_VERIFICATION: landing on Tools → Google");
            }
        }
        #[cfg(feature = "ui-capture")]
        if std::env::args().any(|a| a == "--verify-amoo-gui")
            && !self.busy
            && !self.initializing
            && self.page == Page::Amoo
            && self
                .selected
                .as_ref()
                .is_some_and(|d| std::env::var("STEPX_VERIFY_SERIAL").ok().is_some_and(|serial| !serial.is_empty() && (d.serial == serial || d.transport == serial)))
        {
            static SENT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
            if !SENT.swap(true, std::sync::atomic::Ordering::SeqCst) {
                self.amoo_ui.screen_mode = false;
                self.fields[F_AMOO].set("打开京东，我的订单");
                eprintln!("GUI_UI_VERIFICATION: normal Amoo composer sends user request");
                self.send_amoo_draft();
            }
        }
        #[cfg(feature = "ui-capture")]
        if !self.initializing && !self.busy && self.selected.is_some() {
            if std::env::args().any(|a| a == "--amoo-screen-view") {
                if !self.amoo_ui.screen_mode {
                    self.amoo_ui.screen_mode = true;
                    self.amoo_ui.gui_desktop = true;
                    self.fields[F_AMOO_GUI_APP].set("设置");
                    self.fields[F_AMOO].set("打开关于手机，查看 Android 版本，不修改设置");
                    ctx.notify();
                }
                if self.amoo_ui.gui_screenshot.is_none() {
                    if let Some((_, p)) = &self.screenshot {
                        self.amoo_ui.gui_screenshot = Some(p.clone());
                        ctx.notify();
                    }
                }
            }
            if std::env::args().any(|a| a == "--overview-screen-menu")
                && self.context_menu.is_none()
            {
                self.context_menu = Some(context_menu::Menu {
                    target: context_menu::Target::OverviewCard(
                        context_menu::OverviewContextKind::ScreenPreview,
                    ),
                    at: Vector2F::new(260., 180.),
                    text: None,
                    selected: Some(0),
                });
                ctx.notify();
            }
        }
        let popup_preview_ready = {
            #[cfg(feature = "ui-capture")]
            {
                static POPUP_READY_AT: std::sync::Mutex<Option<Instant>> =
                    std::sync::Mutex::new(None);
                if std::env::var_os("STEPX_POPUP_PREVIEW").is_some() {
                    if let Ok(preview_mode) = std::env::var("STEPX_POPUP_PREVIEW") {
                        let mode = match preview_mode.trim().to_ascii_lowercase().as_str() {
                            "silent" | "none" | "0" => {
                                stepx_companion::desktop_notify::PopupMode::Silent
                            }
                            "banner" | "alert" | "1" => {
                                stepx_companion::desktop_notify::PopupMode::Banner
                            }
                            _ => stepx_companion::desktop_notify::PopupMode::FullReply,
                        };
                        self.push.prefs.popup_mode = mode;
                        self.push.prefs.app_popup_modes.clear();
                        self.settings.notify_draft.popup_mode = mode;
                        self.settings.notify_draft.app_popup_modes.clear();
                        self.notification_center.alert_mode = match mode {
                            stepx_companion::desktop_notify::PopupMode::Silent => {
                                NotificationAlertMode::Muted
                            }
                            stepx_companion::desktop_notify::PopupMode::Banner
                            | stepx_companion::desktop_notify::PopupMode::FullReply => {
                                NotificationAlertMode::PopupAndBadge
                            }
                        };
                    }
                    let has_phone_items = (self.sms_loaded || self.push.loaded)
                        && self.notification_center.items.iter().any(|i| {
                            matches!(
                                i.category,
                                NotificationCategory::PhoneApps | NotificationCategory::PhoneComm
                            )
                        });
                    if has_phone_items
                        && self.notification_active_popup().is_none()
                        && self.push.prefs.popup_mode
                            != stepx_companion::desktop_notify::PopupMode::Silent
                    {
                        self.apply_notification_center_action(
                            NotificationCenterAction::ReplayLatestPopups,
                        );
                        self.sync_desktop_popup_window(ctx);
                        ctx.notify();
                    }
                    if has_phone_items
                        && (self.push.prefs.popup_mode
                            == stepx_companion::desktop_notify::PopupMode::Silent
                            || self.notification_active_popup().is_some())
                    {
                        let mut guard = POPUP_READY_AT.lock().unwrap();
                        let t = *guard.get_or_insert_with(Instant::now);
                        ctx.notify();
                        t.elapsed() > Duration::from_millis(450)
                    } else {
                        false
                    }
                } else {
                    true
                }
            }
            #[cfg(not(feature = "ui-capture"))]
            {
                true
            }
        };
        let bezel_screen_ready = {
            static SHOT_READY_AT: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
            if self.screenshot.is_some() {
                let mut guard = SHOT_READY_AT.lock().unwrap();
                let t = *guard.get_or_insert_with(Instant::now);
                t.elapsed() > Duration::from_millis(650)
            } else {
                *SHOT_READY_AT.lock().unwrap() = None;
                false
            }
        };
        if self.capture.is_some()
            && !std::env::args().any(|a| {
                a == "--verify-settings"
                    || a == "--verify-compact"
                    || a == "--verify-compact-live"
                    || a == "--verify-composer"
                    || a == "--verify-workspace"
                    || a == "--verify-workspace-live"
                    || a == "--verify-window-memory"
                    || a == "--verify-home-live"
            })
            && self.capture_at.elapsed() > Duration::from_millis(900)
            && popup_preview_ready
            && self.svg_slots.ready_for_capture(self.page_requires_svg())
            && (self.demo_mode
                || (std::env::var_os("STEPX_POPUP_PREVIEW").is_some() && popup_preview_ready)
                || (self.page == Page::Tools && self.tool_page == Some(ToolPage::Weather))
                || (!self.busy
                    && self.selected.is_some()
                    && !self.work_previews_loading()
                    && (!(matches!(
                        self.page,
                        Page::Home
                            | Page::Overview
                            | Page::Calendar
                            | Page::Schedule
                            | Page::Alarms
                    ) || (self.page == Page::Tools
                        && matches!(
                            self.tool_page,
                            None | Some(
                                ToolPage::Clipboard
                                    | ToolPage::McpHub
                                    | ToolPage::AiSafety
                                    | ToolPage::Wellbeing
                            )
                        )))
                        || (self.back_screen.stepos_suite.checked
                            && !self.back_screen.stepos_suite_loading))
                    && (self.page != Page::Gallery
                        || (self.back_screen.gallery_checked
                            && !self.back_screen.gallery_loading))
                    && (self.page != Page::Home
                        || (!self.home_ai.running()
                            && (self.demo_mode
                                || self.push.loaded
                                || self.push.error.is_some()
                                || self.capture_at.elapsed() > Duration::from_millis(15000))))
                    && (self.page != Page::Overview
                        || (self.demo_mode
                            || (!self.amoo_ui.usage_loading
                                && self.amoo_ui.snapshot_job.is_none()
                                && (self.amoo_gateway_online
                                    || !self.amoo_sessions.is_empty()
                                    || self.amoo_ui.usage_error.is_some())))
                        || self.capture_at.elapsed() > Duration::from_millis(18000))
                    && (self.page != Page::Notifications
                        || self.push.loaded
                        || self.push.error.is_some())
                    && (std::env::var_os("STEPX_POPUP_PREVIEW").is_none()
                        || ((self.push.loaded || self.push.error.is_some())
                            && (self.push.prefs.popup_mode
                                == stepx_companion::desktop_notify::PopupMode::Silent
                                || self.notification_active_popup().is_some())))
                    && (self.page != Page::Expansion
                        || self.expansion.snapshot.online
                        || self.expansion.snapshot.error)
                    && (self.page != Page::Studio
                        || self.studio.web.is_some()
                        || self.studio.web_error.is_some())
                    && (!(self.page == Page::Overview
                        || (self.page == Page::Amoo && self.amoo_ui.screen_mode))
                        || bezel_screen_ready
                        || self.capture_at.elapsed() > Duration::from_millis(15000))
                    && (!(self.page == Page::Tools && self.tool_page == Some(ToolPage::Screen))
                        || (!self.lan_cast.scanning && self.lan_cast.stats().frames_captured > 0)
                        || self.capture_at.elapsed() > Duration::from_millis(5500)))
                || self.capture_at.elapsed()
                    > Duration::from_millis(
                        if matches!(
                            self.page,
                            Page::Tools
                                | Page::Work
                                | Page::Amoo
                                | Page::Studio
                                | Page::Home
                                | Page::Overview
                        ) {
                            60000
                        } else {
                            12000
                        },
                    ))
        {
            let path = self.capture.take().unwrap();
            if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                window
                    .as_ctx()
                    .request_frame_capture(Box::new(move |frame| {
                        let result = image::save_buffer(
                            &path,
                            &frame.data,
                            frame.width,
                            frame.height,
                            image::ColorType::Rgba8,
                        );
                        if let Err(e) = result {
                            eprintln!("UI capture failed: {e}");
                            std::process::exit(1);
                        }
                        std::process::exit(0);
                    }));
                ctx.notify();
            }
        }
    }

    /// Chat-like panes start at the newest message; the scroll view clamps the offset.
    pub(super) fn scroll_to_end(state: &ClippedScrollStateHandle) {
        state.scroll_to(facloud_ui::units::Pixels::new(1.0e7));
    }

    pub(super) fn is_excluded_im_package(pkg: &str) -> bool {
        stepx_companion::push::is_excluded_im_package(pkg)
    }

    /// Returns true if an incoming SMS message is genuinely unread, unviewed in this session,
    /// and arrived within the last 12 hours.
    pub(crate) fn is_sms_entry_fresh_unread(&self, m: &Message) -> bool {
        if m.kind != 1 || m.read || m.date_ms <= 0 {
            return false;
        }
        let norm = stepx_companion::sms::normalize_address(&m.address);
        let addr_key = if norm.is_empty() {
            m.address.trim().to_string()
        } else {
            norm
        };
        if !addr_key.is_empty() && self.read_sms_addresses.contains(&addr_key) {
            return false;
        }
        let now_ms = chrono::Local::now().timestamp_millis().max(0) as u64;
        now_ms.saturating_sub(m.date_ms as u64) <= 12 * 3600 * 1000
    }

    /// Number of distinct SMS threads that have at least one fresh unread message.
    pub(crate) fn fresh_unread_sms_thread_count(&self) -> usize {
        if !self.sms_loaded || self.sms_error.is_some() {
            return 0;
        }
        let mut threads = std::collections::HashSet::new();
        for m in &self.sms {
            if self.is_sms_entry_fresh_unread(m) {
                let norm = stepx_companion::sms::normalize_address(&m.address);
                let key = if norm.is_empty() {
                    m.address.trim().to_string()
                } else {
                    norm
                };
                if !key.is_empty() {
                    threads.insert(key);
                }
            }
        }
        threads.len()
    }

    /// Total unread count across non-excluded IM threads (excluding the currently active IM thread on Sms page).
    pub(crate) fn active_im_unread_count(&self) -> usize {
        let active_im_id = if self.page == Page::Sms {
            self.push
                .selected_im_thread
                .as_deref()
                .filter(|id| !id.is_empty())
        } else {
            None
        };
        self.push
            .snapshot
            .threads
            .iter()
            .filter(|t| {
                !Self::is_excluded_im_package(&t.package_name)
                    && !t.title.trim_start().starts_with('{')
                    && !stepx_companion::push::is_timer_or_countdown_notification(
                        &t.package_name,
                        &t.app_name,
                        &t.title,
                        &t.last_message,
                    )
                    && active_im_id != Some(t.thread_id.as_str())
            })
            .map(|t| t.unread_count)
            .sum()
    }

    /// Total unread notifications in `消息与通知 -> 通知`, including ongoing live delivery/mobility
    /// notifications (e.g. 美团外卖, 顺丰派送) and synthesized push notifications.
    pub(crate) fn unread_notifications(&self) -> usize {
        self.effective_push_notifications()
            .into_iter()
            .filter(|n| {
                !n.dismissed
                    && !n.read
                    && (!n.is_ongoing
                        || stepx_companion::home_hub::classify_delivery_or_mobility_signal(
                            &n.package_name,
                            &n.app_name,
                            &n.title,
                            &n.text,
                        )
                        .is_some())
                    && !stepx_companion::push::should_hide_notification(n)
            })
            .count()
    }

    /// Marks a specific SMS thread address as viewed by the user and syncs local SMS read state.
    pub(crate) fn mark_sms_thread_read(&mut self, addr: &str) {
        let trimmed = addr.trim();
        if !trimmed.is_empty() && trimmed != "__new__" {
            let norm = stepx_companion::sms::normalize_address(trimmed);
            self.read_sms_addresses.insert(if norm.is_empty() {
                trimmed.to_string()
            } else {
                norm
            });
        }
        self.sync_read_sms_state();
    }

    /// Synchronizes local SMS read state: marks messages in viewed threads or older than 12 hours
    /// as `read = true` so periodic ADB re-polls never resurrect stale unread counts.
    pub(crate) fn sync_read_sms_state(&mut self) {
        let now_ms = chrono::Local::now().timestamp_millis().max(0) as u64;
        for m in &mut self.sms {
            if m.kind == 1 && !m.read {
                let norm = stepx_companion::sms::normalize_address(&m.address);
                let key = if norm.is_empty() {
                    m.address.trim().to_string()
                } else {
                    norm
                };
                let is_stale =
                    m.date_ms <= 0 || now_ms.saturating_sub(m.date_ms as u64) > 12 * 3600 * 1000;
                if is_stale || (!key.is_empty() && self.read_sms_addresses.contains(&key)) {
                    m.read = true;
                }
            }
        }
    }
}

impl Entity for Companion {
    type Event = ();
}

impl View for Companion {
    fn ui_name() -> &'static str {
        "StepXCompanion"
    }

    fn on_window_closed(&mut self, ctx: &mut ViewContext<Self>) {
        self.remember_window(ctx, true);
        self.stop_mirror();
        self.close_mirror_controls(ctx);
        self.qr = None;
    }

    fn active_cursor_position(&self, ctx: &ViewContext<Self>) -> Option<facloud_ui::CursorInfo> {
        let index = self.focus?;
        Some(facloud_ui::CursorInfo {
            position: ctx.element_position_by_id(format!("input-caret:{index}"))?,
            font_size: theme::BODY,
        })
    }

    fn render(&self, app: &AppContext) -> Box<dyn Element> {
        let _timing = perf::Span::new("render");
        let epoch = self.render_epoch.get().wrapping_add(1);
        self.render_epoch.set(epoch);
        if self.hover_states.borrow().len() > 512 {
            self.hover_states
                .borrow_mut()
                .retain(|_, (last, _)| epoch.wrapping_sub(*last) <= 2);
        }
        let p = Palette::for_theme(self.theme_mode.is_dark(app.system_theme()));
        self.palette.set(p);

        let tree = if self.push.shell_mode != ShellMode::Full {
            self.compact_shell()
        } else {
            let body = if self.initializing && self.busy && self.page.needs_device() {
                self.loading_page()
            } else if self.page.needs_device()
                && self.selected.is_none()
                && !matches!(self.page, Page::Home | Page::Overview)
            {
                self.no_device()
            } else if matches!(
                self.page,
                Page::Search
                    | Page::Home
                    | Page::Tools
                    | Page::Connect
                    | Page::Calendar
                    | Page::Schedule
                    | Page::Alarms
                    | Page::Gallery
            ) {
                self.scrollable(self.scroll.clone(), self.page_content())
            } else if self.page.fills_height() {
                self.page_content()
            } else {
                Container::new(self.page_content())
                    .with_uniform_padding(theme::S5)
                    .finish()
            };

            let body = self.loading_feedback(body);
            let body = motion::enter(
                body,
                self.motion_started,
                !self.settings.value.reduced_motion && !self.settings.hidden,
            );
            let body = self.with_notification_center_overlay(body);
            let body = self.with_live_capsule_popover_overlay(body);
            let body = if self.search.open || self.search.closing.is_some() {
                adb_onboarding::Inert(body).finish()
            } else {
                body
            };
            let main = Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(self.header())
                    .with_child(Expanded::new(1., body).finish())
                    .finish(),
            )
            .with_background_color(p.workspace)
            .with_corner_radius(CornerRadius::with_top_right(Radius::Pixels(theme::R_PANEL)))
            .finish();
            let main = self.with_header_actions_overlay(main);
            let main = self.with_search_overlay(main);

            let top = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.sidebar())
                .with_child(Expanded::new(1., main).finish())
                .finish();

            let outer_border_color = if p.dark {
                p.border_strong
            } else {
                theme::rgb(0xDC, 0xDC, 0xE2)
            };
            let tree = Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(Expanded::new(1., top).finish())
                    .with_child(self.status_bar())
                    .finish(),
            )
            .with_background_color(p.workspace)
            .with_border(Border::all(1.).with_border_color(outer_border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish();

            tree
        };
        #[cfg(feature = "ui-capture")]
        let tree = self.text_test_fixture().unwrap_or(tree);
        #[cfg(feature = "ui-capture")]
        let tree = SavePosition::new(tree, "test-window-layout")
            .for_single_frame()
            .finish();
        // SavePosition commits in a Stack paint scope, including the pinned composer.
        let tree = Stack::new().with_child(tree).finish();
        let mut has_interaction_overlay = false;
        let mut has_image_viewer = false;
        let tree = if let Some(sheet) = self.desktop_update_overlay() {
            Stack::new()
                .with_child(adb_onboarding::Inert(tree).finish())
                .with_child(sheet)
                .finish()
        } else if let Some(sheet) = self.whats_new_overlay() {
            Stack::new()
                .with_child(adb_onboarding::Inert(tree).finish())
                .with_child(sheet)
                .finish()
        } else if let Some(menu) = self.context_overlay() {
            Stack::new()
                .with_child(adb_onboarding::Inert(tree).finish())
                .with_child(menu)
                .finish()
        } else if let Some(sheet) = self.interaction_overlay() {
            has_interaction_overlay = true;
            Stack::new()
                .with_child(adb_onboarding::Inert(tree).finish())
                .with_child(sheet)
                .finish()
        } else if let Some(step) = self.adb_guide_step {
            Stack::new()
                .with_child(adb_onboarding::Inert(tree).finish())
                .with_child(self.adb_guide_modal(step))
                .finish()
        } else if let Some(viewer) = self.amoo_image_viewer() {
            has_image_viewer = true;
            Stack::new()
                .with_child(adb_onboarding::Inert(tree).finish())
                .with_child(viewer)
                .finish()
        } else {
            tree
        };
        let has_global_modal = self.desktop_update.open
            || self.desktop_update.whats_new_open
            || has_interaction_overlay
            || self.adb_guide_step.is_some()
            || has_image_viewer;
        Keyboard {
            search: self.focus == Some(F_GLOBAL_SEARCH) && !has_global_modal,
            accept_attachments: self.page == Page::Amoo
                && !self.busy
                && !self.workspace_busy()
                && self.context_menu.is_none()
                && self.workspace.proposal.is_none()
                && !has_global_modal,
            context_menu: self.context_menu.is_some(),
            interaction: self.interaction.review.is_some(),
            child: perf::Measured(tree).finish(),
            shortcut: self.settings.value.compact_shortcut.clone(),
            recording_shortcut: self.shortcuts.recording,
            active: self.focus.is_some(),
            multiline: matches!(
                self.focus,
                Some(F_WORK_PROMPT | F_AMOO | F_SMS_BODY | F_LAB_PROMPT | F_BACK_PROMPT | F_CAPSULE_PROMPT | F_HOME_AGENT_PROMPT | F_AGENT_REQUEST)
            ),
            composing: self
                .focus
                .is_some_and(|i| !self.fields[i].preedit.is_empty()),
        }
        .finish()
    }
}

impl Drop for Companion {
    fn drop(&mut self) {
        self.workspace_flush();
        self.stop_mirror();
    }
}

#[cfg(feature = "ui-capture")]
impl Companion {
    pub(super) fn verify_home_live(&mut self, ctx: &mut ViewContext<Self>) {
        use facloud_ui::TypedActionView;
        if !std::env::args().any(|a| a == "--verify-home-live") {
            return;
        }
        static STEP: std::sync::atomic::AtomicUsize = std::sync::atomic::AtomicUsize::new(0);
        static LAST_STEP_MS: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        let step = STEP.load(std::sync::atomic::Ordering::SeqCst);
        if step >= 99 {
            return;
        }
        let elapsed_ms = self.capture_at.elapsed().as_millis() as u64;
        if elapsed_ms > 90_000 {
            eprintln!(
                "HOME_LIVE_E2E timeout at step={} status={} suite_loading={}",
                step, self.status, self.back_screen.stepos_suite_loading
            );
            std::process::exit(2);
        }
        let prev_ms = LAST_STEP_MS.load(std::sync::atomic::Ordering::SeqCst);
        if elapsed_ms.saturating_sub(prev_ms) < 1100 {
            return;
        }
        if self.initializing
            || self.busy
            || self.selected.is_none()
            || !self.back_screen.stepos_suite.checked
            || self.back_screen.stepos_suite_loading
            || (!self.push.loaded && self.push.error.is_none())
            || self.home_ai.running()
            || !self.svg_slots.ready_for_capture(self.page_requires_svg())
        {
            return;
        }
        assert!(!self.demo_mode, "verify_home_live must run in real mode");
        let serial = self
            .selected
            .as_ref()
            .map(|d| d.serial.as_str())
            .unwrap_or("");
        assert_eq!(
            serial, "STEPXTEST8DB89E4B8927",
            "must connect to STEPXTEST8DB89E4B8927"
        );
        let target_dir = PathBuf::from(
            "/home/stepx-user/.gemini/antigravity/brain/23f6eb3f-adb6-4b5e-b038-9d4d067d6cd7",
        );
        let _ = std::fs::create_dir_all(&target_dir);
        let out_dir = self.capture.clone().unwrap_or_else(|| target_dir.clone());
        let save_shot = |ctx: &mut ViewContext<Self>, name: &str| {
            let path = out_dir.join(name);
            let brain_path = target_dir.join(name);
            if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                window
                    .as_ctx()
                    .request_frame_capture(Box::new(move |frame| {
                        image::save_buffer(
                            &path,
                            &frame.data,
                            frame.width,
                            frame.height,
                            image::ColorType::Rgba8,
                        )
                        .expect("save live home screenshot");
                        if brain_path != path {
                            let _ = image::save_buffer(
                                &brain_path,
                                &frame.data,
                                frame.width,
                                frame.height,
                                image::ColorType::Rgba8,
                            );
                        }
                    }));
            }
        };

        static ORIG_CLIPBOARD: std::sync::Mutex<Option<String>> = std::sync::Mutex::new(None);
        LAST_STEP_MS.store(elapsed_ms, std::sync::atomic::Ordering::SeqCst);
        match step {
            0 => {
                let raw_clip = self
                    .back_screen
                    .stepos_suite
                    .phone_clipboard
                    .trim()
                    .to_string();
                let clean_orig_clip = if raw_clip.contains("SF1357924680123")
                    || raw_clip.contains("SF14298019283")
                    || stepx_companion::home_hub::is_synthetic_e2e_probe_text(&raw_clip)
                {
                    self.back_screen.stepos_suite.phone_clipboard.clear();
                    self.express_parcels.retain(|p| {
                        p.tracking_no != "SF1357924680123" && p.tracking_no != "SF14298019283"
                    });
                    String::new()
                } else {
                    raw_clip
                };
                if let Ok(mut guard) = ORIG_CLIPBOARD.lock() {
                    *guard = Some(clean_orig_clip);
                }
                let sig_stripped = self
                    .back_screen
                    .stepos_suite
                    .backscreen_signature
                    .trim_start_matches(|c: char| {
                        c.is_whitespace()
                            || (c as u32) >= 0x1F000
                            || matches!(c, '📲' | '⏱' | '📦' | '✨' | '🔥' | '🔔')
                    })
                    .trim()
                    .to_string();
                if sig_stripped.is_empty()
                    || sig_stripped != self.back_screen.stepos_suite.backscreen_signature.trim()
                {
                    let clean_sig = if sig_stripped.is_empty() {
                        "专注勿扰".to_string()
                    } else {
                        sig_stripped
                    };
                    self.back_screen.stepos_suite.backscreen_signature = clean_sig.clone();
                    self.fields[F_BACK_MARQUEE].set(&clean_sig);
                    self.back_screen.frame =
                        stepx_companion::backscreen::synthesize_chinese_signature_frame(
                            &clean_sig,
                            self.back_screen.config.invert,
                        );
                }

                self.context.overview.items.retain(|it| {
                    !stepx_companion::home_hub::any_synthetic_e2e_probe(&[
                        &it.id,
                        &it.title,
                        &it.summary,
                        &it.prompt,
                    ])
                });
                self.workspace.journal.tasks.retain(|t| {
                    !stepx_companion::home_hub::any_synthetic_e2e_probe(&[
                        &t.id, &t.title, &t.draft, &t.note,
                    ])
                });
                self.page = Page::Home;
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::ApplyPreset(
                        pages::home::HomeModulePreset::DefaultAll,
                    )),
                    ctx,
                );
                self.home_module_edit_mode = false;
                self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.0));

                // Warm up cache once, then measure 10 consecutive real-mode Page::Home renders against live snapshot.
                let _ = std::hint::black_box(self.home_page());
                let mut total_us = 0u128;
                let mut max_us = 0u128;
                for _ in 0..10 {
                    let t0 = Instant::now();
                    let _ = self.home_page();
                    let dt = t0.elapsed().as_micros();
                    total_us += dt;
                    max_us = max_us.max(dt);
                }
                let avg_ms = (total_us as f64) / 10_000.0;
                let max_ms = (max_us as f64) / 1_000.0;
                eprintln!(
                    "HOME_LIVE_FRAME_TIMING: avg={:.2}ms max={:.2}ms",
                    avg_ms, max_ms
                );
                assert!(
                    avg_ms < 8.0 && max_ms < 16.0,
                    "Home render exceeded frame budget"
                );

                let (key, _tid, pkg, label) = self.resolve_home_reply_target(&self.push.snapshot);
                eprintln!(
                    "HOME_LIVE_REPLY_TARGET: key={} pkg={} label={}",
                    key, pkg, label
                );
                assert!(
                    !label.contains("微信支付")
                        && !label.contains("顺丰速运")
                        && !label.contains("ImageView")
                        && !label.contains("搜索 账号"),
                    "polluted reply target: {label}"
                );
                save_shot(ctx, "home_v0296_1_default_clean_live.png");
                STEP.store(1, std::sync::atomic::Ordering::SeqCst);
                ctx.notify();
            }
            1 => {
                // Stage 2: Module Editor Studio (home_module_edit_mode = true)
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::ApplyPreset(
                        pages::home::HomeModulePreset::DefaultAll,
                    )),
                    ctx,
                );
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::SetEditMode(true)),
                    ctx,
                );
                self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.0));
                save_shot(ctx, "home_v0296_2_free_editor_studio_live.png");
                STEP.store(2, std::sync::atomic::Ordering::SeqCst);
                ctx.notify();
            }
            2 => {
                // Stage 3: Custom Free Combination across categories (CalendarPanorama 1/2 + PulseAndKpi 1/2 on Row 1,
                // ProductivityHub 1/2 + WeeklyAndAi 1/2 on Row 2)
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::SetEditMode(false)),
                    ctx,
                );
                self.home_modules.enabled = vec![
                    pages::home::HomeModuleId::CalendarPanorama,
                    pages::home::HomeModuleId::PulseAndKpi,
                    pages::home::HomeModuleId::ProductivityHub,
                    pages::home::HomeModuleId::WeeklyAndAi,
                    pages::home::HomeModuleId::WorkModule,
                    pages::home::HomeModuleId::LifeModule,
                    pages::home::HomeModuleId::IssuesAndReplyModule,
                ];
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::SetModuleWidth(
                        pages::home::HomeModuleId::CalendarPanorama,
                        pages::home::HomeModuleWidth::OneHalf,
                    )),
                    ctx,
                );
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::SetModuleWidth(
                        pages::home::HomeModuleId::PulseAndKpi,
                        pages::home::HomeModuleWidth::OneHalf,
                    )),
                    ctx,
                );
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::SetModuleWidth(
                        pages::home::HomeModuleId::ProductivityHub,
                        pages::home::HomeModuleWidth::OneHalf,
                    )),
                    ctx,
                );
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::SetModuleWidth(
                        pages::home::HomeModuleId::WeeklyAndAi,
                        pages::home::HomeModuleWidth::OneHalf,
                    )),
                    ctx,
                );
                self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.0));
                save_shot(ctx, "home_v0296_3_custom_free_combination_live.png");
                STEP.store(3, std::sync::atomic::Ordering::SeqCst);
                ctx.notify();
            }
            3 => {
                // Stage 4: Interactive Clicked Modules (ProductivityHub tab switched to Actions,
                // WeeklyAndAi pillar #1 selected, ContextLedgerHub & ToolsSummaryDock visible)
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::ApplyPreset(
                        pages::home::HomeModulePreset::DefaultAll,
                    )),
                    ctx,
                );
                self.handle_action(
                    &Action::Productivity(productivity_state::ProductivityAction::Tab(2)),
                    ctx,
                );
                self.handle_action(&Action::HomeSelectAiQuadrant(1), ctx);
                self.home_modules.enabled = vec![
                    pages::home::HomeModuleId::ProductivityHub,
                    pages::home::HomeModuleId::WeeklyAndAi,
                    pages::home::HomeModuleId::ContextLedgerHub,
                    pages::home::HomeModuleId::ToolsSummaryDock,
                ];
                for m in [
                    pages::home::HomeModuleId::ProductivityHub,
                    pages::home::HomeModuleId::WeeklyAndAi,
                    pages::home::HomeModuleId::ContextLedgerHub,
                    pages::home::HomeModuleId::ToolsSummaryDock,
                ] {
                    self.home_modules
                        .set_module_width(m, pages::home::HomeModuleWidth::OneHalf);
                    self.home_modules
                        .set_module_height(m, pages::home::HomeModuleHeight::Compact);
                }
                self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.0));
                save_shot(ctx, "home_v0296_4_interactive_clicked_modules_live.png");
                STEP.store(4, std::sync::atomic::Ordering::SeqCst);
                ctx.notify();
            }
            4 => {
                // Restore clean DefaultAll preset and save config before exiting
                self.handle_action(
                    &Action::Productivity(productivity_state::ProductivityAction::Tab(0)),
                    ctx,
                );
                self.handle_action(&Action::HomeSelectAiQuadrant(0), ctx);
                self.handle_action(
                    &Action::HomeModule(pages::home::HomeModuleAction::ApplyPreset(
                        pages::home::HomeModulePreset::DefaultAll,
                    )),
                    ctx,
                );
                self.home_module_edit_mode = false;
                if let Some(p) = Self::home_modules_config_path() {
                    let _ = std::fs::remove_file(p);
                }
                self.fields[F_OV_TEXT].clear();
                self.fields[F_HOME_COMMAND].clear();
                self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.0));
                eprintln!("HOME_LIVE_E2E_VERIFIED: all 4 screenshots captured and DefaultAll restored cleanly");
                STEP.store(99, std::sync::atomic::Ordering::SeqCst);
                std::process::exit(0);
            }
            _ => {}
        }
    }
}

#[cfg(test)]
pub(super) static TEST_CONFIG_DIR_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

#[cfg(test)]
thread_local! {
    static TEST_UI_CONFIG_DIR: std::cell::RefCell<Option<std::path::PathBuf>> =
        const { std::cell::RefCell::new(None) };
    static TEST_THREAD_DEFAULT_CONFIG_DIR: (tempfile::TempDir, stepx_companion::resources::ConfigDirOverride) = {
        let dir = tempfile::Builder::new()
            .prefix("stepx-ui-test-")
            .tempdir()
            .expect("create per-thread test config dir");
        let guard = stepx_companion::resources::override_config_dir_for_current_thread(dir.path().to_path_buf());
        (dir, guard)
    };
}

pub(super) fn ui_user_config_dir() -> std::path::PathBuf {
    #[cfg(test)]
    {
        if let Some(dir) = TEST_UI_CONFIG_DIR.with(|c| c.borrow().clone()) {
            return dir;
        }
        return TEST_THREAD_DEFAULT_CONFIG_DIR.with(|d| d.0.path().to_path_buf());
    }
    #[cfg(not(test))]
    {
        stepx_companion::resources::user_config()
    }
}

#[cfg(test)]
pub(super) struct TestUiConfigDirGuard {
    previous: Option<std::path::PathBuf>,
    _library_config: stepx_companion::resources::ConfigDirOverride,
}

#[cfg(test)]
impl TestUiConfigDirGuard {
    pub(super) fn set(path: &std::path::Path) -> Self {
        let prev = TEST_UI_CONFIG_DIR.with(|c| c.borrow_mut().replace(path.to_path_buf()));
        Self {
            previous: prev,
            _library_config: stepx_companion::resources::override_config_dir_for_current_thread(
                path.to_path_buf(),
            ),
        }
    }
}

#[cfg(test)]
impl Drop for TestUiConfigDirGuard {
    fn drop(&mut self) {
        let prev = self.previous.take();
        TEST_UI_CONFIG_DIR.with(|c| {
            *c.borrow_mut() = prev;
        });
    }
}
