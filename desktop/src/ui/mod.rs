//! Desktop shell for StepX Companion. Layout and visual rules: `desktop/DESIGN.md`.
//!
//! Module map:
//! - `theme`   design tokens (spacing, type, colour, radius) and font loading
//! - `widgets` reusable components built only from tokens
//! - `shell`   sidebar, page header and status bar
//! - `pages`   one file per page
//! - `actions` input handling and background jobs

#![allow(dead_code)]

mod actions;
mod app_store_state;
mod desktop_update_state;
mod device_actions;
mod github_stars;
mod keyboard_input;
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
mod amoo_composer;
mod amoo_display;
mod amoo_sync;
mod amoo_ui_state;
mod connection_watch;
mod context_menu;
mod expansion_state;
mod reply_motion;
mod speech_state;
mod text_interaction;
use amoo_ui_state::{AmooUiAction, AmooUiOutcome, AmooUiState};
mod backscreen_state;
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
mod studio_lab_state;
mod studio_state;
mod theme;
mod update_state;
mod widgets;
#[cfg(feature = "ui-capture")]
mod window_memory_check;
mod work_state;
mod workspace_state;
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
    apps::{self, AppInfo},
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
const FIELD_COUNT: usize = 40;

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
            _ => icons::asset(path)
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
    let instance_dir = if capture.is_some() {
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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum Page {
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
            Self::Overview => "首页",
            Self::Expansion => "存储扩容",
            Self::BackScreen => "背屏",
            Self::Amoo => "Amoo 对话",
            Self::Calendar | Self::Schedule => "日历",
            Self::Alarms => "闹钟",
            Self::Gallery => "图库",
            Self::Account => "账号与用量",
            Self::About => "设置",
            Self::Settings => "设置",
            Self::Work => "文件处理",
            Self::Studio => "我的作品",
            Self::Sms => "消息与短信",
            Self::Notifications => "手机推送",
            Self::Phone => "电话",
            Self::Files => "文件",
            Self::AppStore => "应用商店",
            Self::Apps => "应用",
            Self::Tools => "小工具",
            Self::Connect => "连接设备",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::Overview => "◈",
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
            Self::Connect
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
            "tools" | "toolbox" => Self::Tools,
            "account" => Self::Account,
            "about" => Self::About,
            "settings" => Self::Settings,
            "pair" | "connect" => Self::Connect,
            _ => Self::Overview,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[allow(dead_code)]
enum ToolPage {
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
    const ALL: [Self; 14] = [
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
            _ => 3,
        }
    }
    fn from_arg(value: &str) -> Option<Self> {
        match value {
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
#[allow(dead_code)]
enum Action {
    ReviewConfirm,
    ReviewCancel,
    ReviewHistory,
    Expansion(expansion_state::ExpansionAction),
    AmooUi(AmooUiAction),
    AmooToggleSchedules,
    AmooToggleDetails,
    AmooOpenScheduledSession(String),
    // Navigation & appearance & window
    Page(Page),
    AppStore(AppStoreAction),
    Push(PushAction),
    Work(WorkAction),
    Workspace(WorkspaceAction),
    BackScreen(BackScreenAction),
    Updates(UpdateAction),
    DesktopUpdate(DesktopUpdateAction),
    CycleTheme,
    Settings(SettingsAction),
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
    InputPointer(usize, usize, bool, u32),
    InputPointerUp,
    ExtendSelection(i8),
    MessageSelection(Option<String>),
    ClearField(usize),
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
    AmooRetryMessage { session_key: String, text: String },
    AmooToggleToolRecords(String),
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
    CompanionInspect,
    CompanionInstall,
    NeoInspect,
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
    FillDetectedPhoneIp(String),
    BackupPhoneFileToDesktop(String),
    QuickGuiScenario { app: String, goal: String },
    ScreenshotToBackScreen,
    OpenNewTextFile,
    // Clipboard
    Copy,
    CopyText(String, String),
    ClearContent,
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

#[derive(Clone, Debug)]
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
                        "3. 通讯录匹配「李明」(138 0013 8000)，拟定短信正文并触发敏感操作确认".into(),
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
            number: "138 0013 8000".into(),
            name: "李明 · 产品组".into(),
            call_type: 1,
            date_ms: now - 18 * min,
            duration_sec: 265,
        },
        CallRecord {
            number: "139 1024 6688".into(),
            name: "王磊 · 架构师".into(),
            call_type: 2,
            date_ms: now - 95 * min,
            duration_sec: 412,
        },
        CallRecord {
            number: "186 2100 5520".into(),
            name: "顺丰派件员".into(),
            call_type: 3,
            date_ms: now - 180 * min,
            duration_sec: 0,
        },
        CallRecord {
            number: "135 8801 9200".into(),
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
            number: "138 0013 8000".into(),
        },
        ContactEntry {
            name: "王磊 · 架构师".into(),
            number: "139 1024 6688".into(),
        },
        ContactEntry {
            name: "张晓楠 · 设计组".into(),
            number: "135 8801 9200".into(),
        },
        ContactEntry {
            name: "陈思远 · 硬件研发".into(),
            number: "137 6012 3390".into(),
        },
        ContactEntry {
            name: "顺丰速运园区网点".into(),
            number: "186 2100 5520".into(),
        },
    ]
}

struct Companion {
    font: FamilyId,
    mono_font: FamilyId,
    theme_mode: ThemeMode,
    settings: SettingsUi,
    shortcuts: shortcuts::Shortcuts,
    github_stars: github_stars::GithubStars,
    desktop_update: DesktopUpdateState,
    interaction: interaction::InteractionUi,
    motion_started: Instant,
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
    phone_link: Option<stepx_companion::companion_link::Link>,
    expansion: expansion_state::ExpansionUi,
    phone_link_notice: Option<(String, bool)>,
    preferences: Preferences,
    prefs: Preferences,
    allow_first_connect: bool,
    pairing_hosts: Vec<std::net::IpAddr>,
    pair_tab: PairTab,
    connect_guide_step: Option<u8>,
    adb_guide_step: Option<u8>,
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
    tools_tab: u8,
    tools_result_page: usize,
    system_compression: Option<toolbox::SystemCompression>,
    tools_last_result: Option<String>,
    capabilities: Option<Capabilities>,
    mirror: Option<MirrorSession>,
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
    amoo_agent_idx: usize,
    amoo_session_idx: usize,
    amoo_sessions: Vec<AmooSessionItem>,
    amoo_expanded_tool_records: std::collections::HashSet<String>,
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
    sms_loaded: bool,
    phone_loaded: bool,
    sms_error: Option<String>,
    call_logs_error: Option<String>,
    contacts_error: Option<String>,
    sms_filter: SmsFilter,
    selected_thread: Option<String>,
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
    directory: String,
    files: Vec<Entry>,
    file_page: usize,
    file_sort: FileSort,
    selected_file: Option<usize>,
    file_modal: Option<FileModal>,
    trash_confirm: bool,
    permanent_delete: bool,
    trashed: Option<Trashed>,
    // Apps & Privileged System Management
    pub(super) app_store: AppStoreUiState,
    apps: Vec<AppInfo>,
    apps_loaded: bool,
    apps_error: Option<String>,
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
    text_selection: SelectionHandle,
    selected_text: Option<String>,
    context_menu: Option<context_menu::Menu>,
    reply_motion: reply_motion::ReplyMotion,
    speech: speech_state::SpeechUi,
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
    hover_states: RefCell<HashMap<String, (u64, MouseStateHandle)>>,
    render_epoch: Cell<u64>,
    chat_scroll_metrics: std::rc::Rc<std::cell::Cell<(f32, f32)>>,
    chat_tail_requested: std::rc::Rc<std::cell::Cell<bool>>,
    wheel_motions: RefCell<HashMap<(&'static str, u32), smooth_scroll::WheelHandle>>,
    capture: Option<PathBuf>,
    capture_at: Instant,
    last_clock_second: u64,
    activation_requests: Arc<std::sync::atomic::AtomicBool>,
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
        let page = std::env::args()
            .position(|arg| arg == "--page")
            .and_then(|i| std::env::args().nth(i + 1))
            .map(|p| Page::from_arg(&p))
            .unwrap_or(if std::env::args().any(|a| a == "--pair-qr") {
                Page::Connect
            } else if std::env::args().any(|a| a == "--mini-ai") {
                Page::Amoo
            } else if std::env::args().any(|a| a == "--mini-notify") {
                Page::Notifications
            } else {
                Page::Overview
            });
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

        let demo_mode = cfg!(feature = "ui-capture") && std::env::args().any(|a| a == "--demo");
        Self {
            font,
            mono_font,
            theme_mode,
            settings: SettingsUi::default(),
            shortcuts: Default::default(),
            github_stars: Default::default(),
            desktop_update: Default::default(),
            interaction: Default::default(),
            motion_started: Instant::now(),
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
            phone_link: None,
            expansion: Default::default(),
            phone_link_notice: None,
            allow_first_connect: true,
            connect_guide_step: if preferences.first_connect_done {
                None
            } else {
                Some(0)
            },
            adb_guide_step: (!demo_mode && preferences.should_show_adb_guide()).then_some(0),
            prefs: preferences.clone(),
            preferences,
            pairing_hosts: vec![],
            pair_tab: PairTab::Qr,
            show_secret: false,
            qr: None,
            summary: String::new(),
            storage_report: None,
            tools_pending: None,
            tool_page: std::env::args()
                .position(|a| a == "--tool")
                .and_then(|i| std::env::args().nth(i + 1))
                .and_then(|a| ToolPage::from_arg(&a)),
            tool_category: 0,
            tool_confirmation: None,
            tools_feedback: None,
            job_tool: None,
            google_progress: Default::default(),
            tools_tab: 0,
            tools_result_page: 0,
            system_compression: None,
            tools_last_result: None,
            capabilities: None,
            mirror: None,
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
            amoo_memories: vec![
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
            ],
            amoo_skills: default_amoo_skills(),
            amoo_skill_manage: false,
            amoo_safety_policy_idx: 0,
            amoo_guard_count: if demo_mode { 12 } else { 0 },
            studio: StudioUiState::default(),
            studio_lab: StudioLabUiState::default(),
            push: PushUiState::default(),
            sms: Vec::new(),
            sms_loaded: demo_mode,
            phone_loaded: demo_mode,
            sms_error: None,
            call_logs_error: None,
            contacts_error: None,
            sms_filter: SmsFilter::All,
            selected_thread: None,
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
                Some("138 0013 8000".into())
            } else {
                None
            },
            directory: "/sdcard".into(),
            files: vec![],
            file_page: 0,
            file_sort: FileSort::Name,
            selected_file: None,
            file_modal: None,
            trash_confirm: false,
            permanent_delete: false,
            trashed: None,
            app_store: AppStoreUiState::new(),
            apps: vec![],
            apps_loaded: false,
            apps_error: None,
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
            text_selection: Default::default(),
            selected_text: None,
            context_menu: None,
            reply_motion: Default::default(),
            speech: Default::default(),
            fields,
            focus: None,
            scroll: Default::default(),
            scroll_aux: Default::default(),
            push_list_scroll: Default::default(),
            push_detail_scroll: Default::default(),
            sms_threads_scroll: Default::default(),
            sms_messages_scroll: Default::default(),
            hover_states: RefCell::new(HashMap::new()),
            render_epoch: Cell::new(0),
            wheel_motions: Default::default(),
            chat_scroll_metrics: Default::default(),
            chat_tail_requested: Default::default(),
            capture,
            capture_at: Instant::now(),
            last_clock_second: 0,
            activation_requests,
        }
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
        ctx.focus_self();
        me.initialize_shortcut(ctx);
        me.initialize_stars(ctx);
        me.initialize_desktop_updates(ctx);
        me.schedule_tick(ctx);
        me
    }

    fn schedule_tick(&self, ctx: &mut ViewContext<Self>) {
        let interval = if self.reply_motion.active()
            && !self.settings.hidden
            && matches!(self.page, Page::Amoo | Page::Work)
        {
            32
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

    fn mouse_state(&self, id: impl Into<String>) -> MouseStateHandle {
        let mut states = self.hover_states.borrow_mut();
        let entry = states.entry(id.into()).or_default();
        entry.0 = self.render_epoch.get();
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
        std::thread::spawn(move || {
            let outcome = Adb::discover().and_then(|adb| job(&adb));
            let _ = tx.send(outcome);
        });
    }

    pub(super) fn with_device(
        &mut self,
        label: &str,
        job: impl FnOnce(&Adb, Device) -> Result<Outcome> + Send + 'static,
    ) {
        match self.selected.clone() {
            Some(device) => self.submit(label, move |adb| job(adb, device)),
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
        let Some(device) = self.selected.clone() else {
            self.page = Page::Connect;
            self.status = "请先连接设备".into();
            self.error = true;
            return;
        };
        let page = self.page;
        let tool = self.tool_page;
        let label_str = label.to_string();
        self.spawn_bulk(label_str, move || {
            let result = Adb::discover().and_then(|adb| job(&adb, device));
            match result {
                Ok(outcome) => outcome,
                Err(error) => Outcome::BulkFailed(page, tool, format!("{error:#}")),
            }
        });
    }

    fn tick(&mut self, ctx: &mut ViewContext<Self>) {
        let _timing = perf::Span::new("tick");
        #[cfg(all(feature = "ui-capture", target_os = "linux"))]
        self.verify_text_interaction(ctx);
        #[cfg(all(feature = "ui-capture", target_os = "linux"))]
        self.verify_shell_details(ctx);
        self.poll_desktop_update(ctx);
        self.tick_speech(ctx);
        self.tick_reply_motion(ctx);
        self.tick_connection_watch(ctx);
        self.tick_settings(ctx);
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
        if !self.demo_mode && !link_matches {
            self.phone_link = None;
            if let Some(device) = self.selected.clone() {
                if let Ok(adb) = Adb::discover() {
                    self.phone_link =
                        Some(stepx_companion::companion_link::Link::start(adb, device));
                }
            }
        }
        if let Some(event) = self.phone_link.as_ref().and_then(|link| link.event()) {
            if !self.busy {
                self.status = event.message.clone();
            }
            self.phone_link_notice = Some((event.message, event.error));
            ctx.notify();
        }
        if self.tick_expansion() && matches!(self.page, Page::Overview | Page::Expansion) {
            ctx.notify();
        }
        let mut preview_visible = self.page == Page::BackScreen && !self.settings.hidden;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(window) =
            facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id())
        {
            preview_visible &= !window.is_minimized().unwrap_or(false);
        }
        if self.poll_backscreen(preview_visible) && preview_visible {
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
        if !self.demo_mode && self.page == Page::AppStore {
            self.ensure_store_online_catalog_synced();
        }
        if !self.demo_mode
            && self.page == Page::AppStore
            && self.apps_loaded
            && !self.apps.is_empty()
            && !self.app_store.update_checked_once
            && !self.app_store.checking_updates
        {
            self.handle_app_store_action(AppStoreAction::CheckUpdates, ctx);
        }
        if self.poll_work() && matches!(self.page, Page::Work | Page::Amoo) {
            ctx.notify();
        }
        if self.poll_amoo_gui() && self.page == Page::Amoo {
            ctx.notify();
        }
        if !self.demo_mode
            && self.page == Page::Overview
            && self.selected.is_some()
            && !self.busy
            && !self.amoo_ui.usage_attempted
        {
            self.start_account_usage_sync();
            ctx.notify();
        }
        if !self.demo_mode
            && matches!(
                self.page,
                Page::Overview | Page::Tools | Page::Calendar | Page::Schedule | Page::Alarms
            )
            && self.selected.is_some()
            && !self.back_screen.stepos_suite.checked
            && !self.back_screen.stepos_suite_loading
        {
            self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
        }
        if !self.demo_mode
            && self.page == Page::Tools
            && self.tool_page == Some(ToolPage::Screen)
            && self.selected.is_some()
        {
            if !self.lan_cast.is_running() && self.lan_cast.status_note.is_none() {
                if let Ok(stats) = self.ensure_lan_cast_server_started() {
                    self.lan_cast.status_note = Some((
                        format!("局域网多协议投屏网关已启动：{} · MPEG-TS {}", stats.web_url, stats.ts_url),
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
        if !self.demo_mode
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
            && matches!(self.page, Page::Notifications | Page::Sms)
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
                self.record_notice("操作未完成", format!("{e:#}"));
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
                    self.status = format!("{e:#}");
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
            if !self.busy && self.selected.is_some() && self.page != completed_page {
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
                            if matches!(completed_page, Page::Apps | Page::AppStore) {
                                self.apps_loaded = true;
                                self.apps_error = Some(format!("{e:#}"));
                            }
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
        if !self.demo_mode
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
        if let Some(child) = &mut self.mirror {
            if let Ok(Some(status)) = child.try_wait() {
                self.error = !status.success();
                self.status = if status.success() {
                    "投屏已结束".into()
                } else {
                    format!("投屏已中断：{}", child.exit_detail(status))
                };
                self.mirror = None;
                self.tools_feedback =
                    Some((Some(ToolPage::Screen), self.status.clone(), self.error));
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
        let overview_needs_amoo_init = self.page == Page::Overview
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
            && (!sync_pending.is_empty()
                || matches!(self.page, Page::Amoo | Page::Work)
                || overview_needs_amoo_init)
            && self.last_amoo_poll.elapsed()
                >= if needs_initial_amoo_fetch {
                    Duration::from_millis(400)
                } else if self.amoo_poll_remaining > 0 || !self.amoo_pending_reply.is_empty() {
                    Duration::from_millis(if !self.settings.hidden && self.chat_waiting() {
                        1200
                    } else {
                        2400
                    })
                } else if sync_pending.is_empty() && !self.amoo_gateway_online {
                    Duration::from_secs(30)
                } else {
                    Duration::from_secs(10)
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
                .is_some_and(|d| d.serial == "ZYPZ261A01M692000LP")
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
            })
            && self.capture_at.elapsed() > Duration::from_millis(900)
            && (self.demo_mode
                || (!self.busy
                    && self.selected.is_some()
                    && !self.work_previews_loading()
                    && (!(matches!(
                        self.page,
                        Page::Overview | Page::Calendar | Page::Schedule | Page::Alarms
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
                    && (self.page != Page::Notifications
                        || self.push.loaded
                        || self.push.error.is_some())
                    && (self.page != Page::Expansion
                        || self.expansion.snapshot.online
                        || self.expansion.snapshot.error)
                    && (self.page != Page::Studio
                        || self.studio.web.is_some()
                        || self.studio.web_error.is_some())
                    && (!(self.page == Page::Overview
                        || (self.page == Page::Amoo && self.amoo_ui.screen_mode))
                        || bezel_screen_ready
                        || self.capture_at.elapsed() > Duration::from_millis(7500))
                    && (!(self.page == Page::Tools
                        && self.tool_page == Some(ToolPage::Screen))
                        || (!self.lan_cast.scanning
                            && self.lan_cast.stats().frames_captured > 0)
                        || self.capture_at.elapsed() > Duration::from_millis(5500)))
                || self.capture_at.elapsed()
                    > Duration::from_millis(
                        if matches!(
                            self.page,
                            Page::Tools | Page::Work | Page::Amoo | Page::Studio
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
                && self.page != Page::Overview
            {
                self.no_device()
            } else if matches!(
                self.page,
                Page::Tools
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
        } else if let Some(toast_overlay) = self.push_toast_overlay() {
            Stack::new()
                .with_child(tree)
                .with_child(toast_overlay)
                .finish()
        } else {
            tree
        };
        Keyboard {
            accept_attachments: self.page == Page::Amoo
                && !self.busy
                && !self.workspace_busy()
                && self.context_menu.is_none()
                && self.workspace.proposal.is_none()
                && !has_interaction_overlay
                && !has_image_viewer,
            context_menu: self.context_menu.is_some(),
            child: perf::Measured(tree).finish(),
            shortcut: self.settings.value.compact_shortcut.clone(),
            recording_shortcut: self.shortcuts.recording,
            active: self.focus.is_some(),
            multiline: matches!(
                self.focus,
                Some(F_WORK_PROMPT | F_AMOO | F_SMS_BODY | F_LAB_PROMPT | F_BACK_PROMPT)
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
