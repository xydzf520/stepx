//! Unified Notification Center state, 6-source aggregator, persistence, and action state machine.
//!
//! Aggregates and deduplicates desktop + phone notifications into a concise, human-centric
//! graphical model across 6 sources:
//! 1. `PhoneApps`: IM threads (WeChat, Feishu, DingTalk, QQ, etc.) & app push notifications
//! 2. `PhoneComm`: OTP codes, missed calls, unread SMS, express parcels, upcoming meetings/alarms
//! 3. `AppUpdates`: App Store updatable apps & APK download/install progress/results
//! 4. `CompanionAndTasks`: Software status/updates, `phone-work-*` tasks, GUI safety approvals,
//!    cross-device clipboard & expansion notices
//! 5. `System`: Device connection status, low battery (<20%), high storage (>85%), OTA alerts

use super::{
    amoo_ui_state::AmooUiAction, app_store_state::StoreTaskStage, is_wireless, Companion, Page,
    SafetyDecision, SmsFilter, F_SMS_TO,
};
use chrono::{Local, Timelike};
use facloud_ui::elements::ClippedScrollStateHandle;
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    path::{Path, PathBuf},
    time::Instant,
};
use stepx_companion::{
    app_store::{check_store_install_state, StoreCategory, StoreInstallState},
    device::ExpressParcelInfo,
    ota::OtaStatus,
    push::{extract_otp_code, ImChannel},
};

const MAX_PERSISTED_IDS: usize = 2048;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum NotificationAlertMode {
    #[default]
    #[serde(alias = "All", alias = "all")]
    PopupAndBadge,
    BadgeOnly,
    Muted,
}

impl NotificationAlertMode {
    pub const ALL: [Self; 3] = [Self::PopupAndBadge, Self::BadgeOnly, Self::Muted];

    pub fn label(self) -> &'static str {
        match self {
            Self::PopupAndBadge => "弹窗与角标",
            Self::BadgeOnly => "仅显示红点",
            Self::Muted => "关闭弹窗提醒",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::PopupAndBadge => "弹窗+角标",
            Self::BadgeOnly => "仅红点",
            Self::Muted => "免打扰",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::PopupAndBadge => "新消息在电脑右下角显示 StepX 弹窗，并在顶部图标显示未读数量",
            Self::BadgeOnly => "关闭弹窗打扰，仅在顶部消息图标保留未读红点",
            Self::Muted => "静默收纳全部消息，不弹悬浮窗、不显示红点",
        }
    }

    pub fn allows_popup(self) -> bool {
        matches!(self, Self::PopupAndBadge)
    }

    pub fn allows_badge(self) -> bool {
        matches!(self, Self::PopupAndBadge | Self::BadgeOnly)
    }

    pub fn next(self) -> Self {
        match self {
            Self::PopupAndBadge => Self::BadgeOnly,
            Self::BadgeOnly => Self::Muted,
            Self::Muted => Self::PopupAndBadge,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize, Default)]
pub(crate) enum NotificationCategory {
    #[default]
    All,
    PhoneApps,
    PhoneComm,
    AppUpdates,
    CompanionAndTasks,
    System,
}

impl NotificationCategory {
    pub const ALL: [Self; 6] = [
        Self::All,
        Self::PhoneApps,
        Self::PhoneComm,
        Self::AppUpdates,
        Self::CompanionAndTasks,
        Self::System,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::PhoneApps => "手机应用",
            Self::PhoneComm => "来电与短信",
            Self::AppUpdates => "应用更新",
            Self::CompanionAndTasks => "本机与协同",
            Self::System => "系统状态",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::PhoneApps => "应用",
            Self::PhoneComm => "通讯",
            Self::AppUpdates => "更新",
            Self::CompanionAndTasks => "协同",
            Self::System => "系统",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum NotificationVisualKind {
    WeChat,
    Feishu,
    DingTalk,
    ImChat,
    IncomingCall,
    MissedCall,
    SmsOtp,
    Express,
    CalendarAlarm,
    AppUpdate,
    TaskReady,
    SafetyApproval,
    DeviceLink,
    BatteryWarn,
    #[default]
    CompanionInfo,
    SystemAlert,
}

impl NotificationVisualKind {
    pub fn glyph(self) -> &'static str {
        match self {
            Self::WeChat => "微",
            Self::Feishu => "飞",
            Self::DingTalk => "钉",
            Self::ImChat => "信",
            Self::IncomingCall | Self::MissedCall => "电",
            Self::SmsOtp => "码",
            Self::Express => "件",
            Self::CalendarAlarm => "程",
            Self::AppUpdate => "升",
            Self::TaskReady => "协",
            Self::SafetyApproval => "盾",
            Self::DeviceLink => "连",
            Self::BatteryWarn => "电",
            Self::CompanionInfo => "端",
            Self::SystemAlert => "警",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::WeChat => "微信",
            Self::Feishu => "飞书",
            Self::DingTalk => "钉钉",
            Self::ImChat => "应用消息",
            Self::IncomingCall => "手机通话",
            Self::MissedCall => "未接来电",
            Self::SmsOtp => "短信验证码",
            Self::Express => "快递取件",
            Self::CalendarAlarm => "日程闹钟",
            Self::AppUpdate => "应用更新",
            Self::TaskReady => "协同任务",
            Self::SafetyApproval => "安全确认",
            Self::DeviceLink => "设备连接",
            Self::BatteryWarn => "电量提醒",
            Self::CompanionInfo => "本机软件",
            Self::SystemAlert => "系统状态",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize, Default)]
pub(crate) enum NotificationPriority {
    P0Immediate = 0, // OTP, SafetyApproval, MissedCall, Express, IM/Mention, <=15m Calendar/Alarm
    #[default]
    P1Digest = 1, // AppUpdates summary, TaskReady completion, ClipboardPreview, Low Battery/Storage
    P2Silent = 2,    // Companion status, Device connected/disconnected, Routine background info
}

impl NotificationPriority {
    #[allow(non_upper_case_globals)]
    pub const P1Important: Self = Self::P1Digest;

    pub fn allows_popup(self) -> bool {
        matches!(self, Self::P0Immediate | Self::P1Digest)
    }

    pub fn default_ttl_ms(self) -> u32 {
        match self {
            Self::P0Immediate => 60_000,
            Self::P1Digest => 60_000,
            Self::P2Silent => 0,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::P0Immediate => "P0 紧急即时",
            Self::P1Digest => "P1 摘要提醒",
            Self::P2Silent => "P2 静默收纳",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) enum NotificationPopupWidget {
    #[default]
    None,
    OtpCode {
        code: String,
        source_app: String,
    },
    ExpressPickup {
        pickup_code: String,
        carrier: String,
        station: String,
    },
    InlineImReply {
        thread_id: String,
        package_name: String,
        can_reply: bool,
        quick_phrases: Vec<String>,
    },
    CallerBadge {
        name: String,
        number: String,
    },
    LiveCall { id: String, ringing: bool },
    AppUpdateStrip {
        count: usize,
        app_names: Vec<String>,
        package_name: Option<String>,
    },
    SafetyConfirm {
        prompt: String,
    },
    ClipboardPreview {
        text: String,
    },
}

impl NotificationPopupWidget {
    pub fn kind_name(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::OtpCode { .. } => "otp_code",
            Self::ExpressPickup { .. } => "express_pickup",
            Self::InlineImReply { .. } => "inline_im_reply",
            Self::CallerBadge { .. } => "caller_badge",
            Self::LiveCall { .. } => "live_call",
            Self::AppUpdateStrip { .. } => "app_update_strip",
            Self::SafetyConfirm { .. } => "safety_confirm",
            Self::ClipboardPreview { .. } => "clipboard_preview",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum NotificationPrimaryAction {
    CopyText { label: String, text: String },
    OpenImThread { thread_id: String },
    OpenSms,
    OpenAppUpdates,
    UpdateSingleApp { package_name: String },
    ApproveGuiSafety(bool),
    ResumeTask { task_id: String },
    OpenPage(Page),
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub(crate) struct UnifiedNotificationItem {
    pub id: String,
    #[serde(default)]
    pub package_name: String,
    #[serde(default)]
    pub app_name: String,
    #[serde(default)]
    pub sender: String,
    #[serde(default)]
    pub full_body: String,
    #[serde(default)]
    pub banner_body: String,
    pub category: NotificationCategory,
    pub visual: NotificationVisualKind,
    pub priority: NotificationPriority,
    pub popup_widget: NotificationPopupWidget,
    pub source_badge: String,
    pub title: String,
    pub summary: String,
    pub time_label: String,
    pub sort_ts_ms: i64,
    pub unread: bool,
    pub status_tag: Option<String>,
    pub primary_action: Option<(String, NotificationPrimaryAction)>,
}

impl UnifiedNotificationItem {
    /// Only structured payment/chat payloads imply a red packet. A nickname,
    /// marketing copy or a receipt about an old packet is not a new packet.
    pub(crate) fn is_red_packet(&self) -> bool {
        let package = stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(&self.package_name);
        if !matches!(package.as_str(), "com.tencent.mm" | "com.tencent.mobileqq" | "com.tencent.tim" | "com.tencent.qqlite" | "com.eg.android.AlipayGphone" | "com.eg.android.alipaygphone") {
            return false;
        }
        let mut body = self.full_body.trim();
        if let Some((prefix, rest)) = body.split_once(']') {
            if prefix.strip_prefix('[').is_some_and(|v| v.ends_with('条') && v.trim_end_matches('条').chars().all(|c| c.is_ascii_digit())) {
                body = rest.trim();
            }
        }
        // Android notification text may include the sender before the payload.
        if let Some((sender, rest)) = body.split_once([':', '：']) {
            if sender.chars().count() <= 32 && !sender.contains(['[', ']', '“', '”', '"']) {
                body = rest.trim();
            }
        }
        if ["已领取", "已领完", "已过期", "已退回", "已退还", "红包提醒已撤回"].iter().any(|status| body.contains(status)) {
            return false;
        }
        ["[微信红包]", "[QQ红包]", "[支付宝红包]", "[红包]"].iter().any(|marker| body.starts_with(marker))
            || ["给你发了一个红包", "向你发来一个红包", "收到一个红包", "你收到一个红包"].contains(&body)
    }

    pub fn default_popup_ttl_ms(&self) -> u32 {
        match self.visual {
            NotificationVisualKind::SafetyApproval | NotificationVisualKind::MissedCall => 60_000,
            _ => self.priority.default_ttl_ms(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum NotificationCenterAction {
    ToggleDrawer,
    CloseDrawer,
    SetAlertMode(NotificationAlertMode),
    SelectCategory(NotificationCategory),
    TriggerItem(String),
    OpenPopupDetails(String),
    PopupMenu(Option<String>),
    MutePopupApp(String),
    SnoozeToday,
    MarkRead(String),
    MarkAllRead,
    DismissItem(String),
    ClearReadAndDismissed,
    DismissPopup,
    // Exact events painted in the menu, not permanent conversation mutes.
    ClearPopupBatch { receipts: Vec<(String, String)> },
    SwitchToBadgeOnlyFromPopup,
    PopupPrev,
    PopupNext,
    PopupHoverChanged(bool),
    SnoozePopups { duration_ms: i64 },
    ClearSnooze,
    PopupQuickReply { item_id: String, text: String },
    SendQuickReply(String, String),
    SubmitPopupReply(String),
    ReplayLatestPopups,
    PreviewItemPopup(String),
    MovePopupBy { dx: i32, dy: i32 },
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
struct PersistedNotificationCenter {
    alert_mode: NotificationAlertMode,
    snooze_until_ms: Option<i64>,
    read_ids: Vec<String>,
    dismissed_ids: Vec<String>,
    popup_shown_ids: Vec<String>,
    delivered_events: VecDeque<String>,
    cached_history: Vec<UnifiedNotificationItem>,
}

pub(crate) fn config_dir() -> PathBuf {
    super::ui_user_config_dir().join("stepx-companion")
}

pub(crate) fn persistence_path() -> PathBuf {
    config_dir().join("notification_center.json")
}

pub(crate) const MAX_POPUP_QUEUE_LEN: usize = 8;
const MAX_CACHED_HISTORY_LEN: usize = 80;

#[derive(Clone)]
pub(crate) struct NotificationCenterState {
    pub drawer_open: bool,
    pub drawer_list_started: Option<Instant>,
    pub alert_mode: NotificationAlertMode,
    pub selected_category: NotificationCategory,
    pub items: Vec<UnifiedNotificationItem>,
    pub active_popup_id: Option<String>,
    pub popup_queue: VecDeque<String>,
    pub popup_cursor: usize,
    // New arrivals wait here while a reply/IME/menu/pointer owns the card.
    pub popup_pending_front: Option<String>,
    pub popup_reply_item_id: Option<String>,
    pub popup_menu_item_id: Option<String>,
    pub popup_reply_drafts: HashMap<String, String>,
    pub popup_hovered: bool,
    pub popup_remaining_ms: u32,
    pub popup_total_ms: u32,
    pub snooze_until_ms: Option<i64>,
    pub popup_feedback_text: Option<(String, i64)>,
    pub initial_sync_seeded: bool,
    pub popup_session_started_ms: i64,
    pub popup_shown_ids: HashSet<String>,
    delivered_events: VecDeque<String>,
    pub read_ids: HashSet<String>,
    pub dismissed_ids: HashSet<String>,
    pub list_scroll: ClippedScrollStateHandle,
    pub popup_menu_scroll: ClippedScrollStateHandle,
    pub last_copied_text: Option<String>,
    pub popup_drag_offset: (f32, f32),
    pub popup_drag_last_pos: std::rc::Rc<std::cell::Cell<Option<(f32, f32)>>>,
    pub(crate) last_thread_unread: HashMap<String, (usize, i64)>,
    pub(crate) last_thread_sig: HashMap<String, String>,
    pub(crate) last_notif_sig: HashMap<String, String>,
    pub(crate) seen_updatable_apps: HashMap<String, (String, String)>,
    pub(crate) last_popup_timer_tick: Option<std::time::Instant>,
    pub(crate) last_materialized_at: Option<std::time::Instant>,
    pub(crate) pending_clipboard_otp: Option<String>,
    pub(crate) recent_clipboards: VecDeque<(String, i64)>,
    pub(crate) cached_history: Vec<UnifiedNotificationItem>,
    pub(crate) baseline_push_seeded_for_serial: Option<String>,
    pub(crate) baseline_sms_seeded_for_serial: Option<String>,
    pub(crate) baseline_calls_seeded_for_serial: Option<String>,
    pub(crate) baseline_apps_seeded_for_serial: Option<String>,
    pub(crate) baseline_suite_seeded_for_serial: Option<String>,
    pub(crate) baseline_clip_seeded_for_serial: Option<String>,
    pub(crate) connect_baseline_enabled: bool,
}

impl Default for NotificationCenterState {
    fn default() -> Self {
        Self::load()
    }
}

impl NotificationCenterState {
    pub fn new_empty() -> Self {
        Self {
            drawer_open: false,
            drawer_list_started: None,
            alert_mode: NotificationAlertMode::PopupAndBadge,
            selected_category: NotificationCategory::All,
            items: Vec::new(),
            active_popup_id: None,
            popup_queue: VecDeque::new(),
            popup_cursor: 0,
            popup_pending_front: None,
            popup_reply_item_id: None,
            popup_menu_item_id: None,
            popup_reply_drafts: HashMap::new(),
            popup_hovered: false,
            popup_remaining_ms: 0,
            popup_total_ms: 0,
            snooze_until_ms: None,
            popup_feedback_text: None,
            initial_sync_seeded: false,
            popup_session_started_ms: Local::now().timestamp_millis(),
            popup_shown_ids: HashSet::new(),
            delivered_events: VecDeque::new(),
            read_ids: HashSet::new(),
            dismissed_ids: HashSet::new(),
            list_scroll: ClippedScrollStateHandle::default(),
            popup_menu_scroll: ClippedScrollStateHandle::default(),
            last_copied_text: None,
            popup_drag_offset: (0.0, 0.0),
            popup_drag_last_pos: std::rc::Rc::new(std::cell::Cell::new(None)),
            last_thread_unread: HashMap::new(),
            last_thread_sig: HashMap::new(),
            last_notif_sig: HashMap::new(),
            seen_updatable_apps: HashMap::new(),
            last_popup_timer_tick: None,
            last_materialized_at: None,
            pending_clipboard_otp: None,
            recent_clipboards: VecDeque::new(),
            cached_history: Vec::new(),
            baseline_push_seeded_for_serial: None,
            baseline_sms_seeded_for_serial: None,
            baseline_calls_seeded_for_serial: None,
            baseline_apps_seeded_for_serial: None,
            baseline_suite_seeded_for_serial: None,
            baseline_clip_seeded_for_serial: None,
            connect_baseline_enabled: !cfg!(test),
        }
    }

    pub fn load() -> Self {
        let primary = persistence_path();
        if primary.is_file() {
            return Self::load_from_path(&primary);
        }
        let fallback = super::ui_user_config_dir().join("notification_center.json");
        if fallback.is_file() {
            return Self::load_from_path(&fallback);
        }
        Self::new_empty()
    }

    fn is_current_popup_event(&self, item: &UnifiedNotificationItem, now_ms: i64) -> bool {
        self.is_current_popup_time(item.sort_ts_ms, now_ms)
    }

    pub(super) fn is_current_popup_time(&self, timestamp_ms: i64, now_ms: i64) -> bool {
        timestamp_ms == 0
            || (timestamp_ms >= self.popup_session_started_ms.saturating_sub(600_000)
                && timestamp_ms >= now_ms.saturating_sub(600_000)
                && timestamp_ms <= now_ms.saturating_add(120_000))
    }

    pub fn load_from_path(path: &Path) -> Self {
        let mut state = Self::new_empty();
        let Ok(raw) = stepx_companion::local_store::read_bounded(path, 2 * 1024 * 1024) else {
            return state;
        };
        let Ok(parsed) = serde_json::from_slice::<PersistedNotificationCenter>(&raw) else {
            return state;
        };
        state.alert_mode = parsed.alert_mode;
        state.snooze_until_ms = parsed
            .snooze_until_ms
            .filter(|until| *until > Local::now().timestamp_millis());
        state.read_ids = parsed
            .read_ids
            .into_iter()
            .take(MAX_PERSISTED_IDS)
            .collect();
        state.dismissed_ids = parsed
            .dismissed_ids
            .into_iter()
            .take(MAX_PERSISTED_IDS)
            .collect();
        state.popup_shown_ids = parsed.popup_shown_ids.into_iter().take(MAX_PERSISTED_IDS).collect();
        state.delivered_events = parsed.delivered_events.into_iter()
            .filter(|key| key.len() <= 1024).take(MAX_PERSISTED_IDS).collect();
        state.cached_history = parsed
            .cached_history
            .into_iter()
            .filter(|i| !state.dismissed_ids.contains(&i.id))
            .take(MAX_CACHED_HISTORY_LEN)
            .map(|mut i| {
                if state.read_ids.contains(&i.id) {
                    i.unread = false;
                }
                // A persisted event receipt supersedes the legacy conversation
                // marker. Otherwise a later message in the same conversation
                // could be suppressed after restarting the desktop.
                let suffix = popup_delivery_event_suffix(&i);
                if state.delivered_events.iter().any(|key| key.ends_with(&suffix)) {
                    state.popup_shown_ids.remove(&i.id);
                } else {
                    state.popup_shown_ids.insert(i.id.clone());
                }
                i
            })
            .collect();
        state.items = state.cached_history.clone();
        state
    }

    pub fn save(&self) {
        // The legacy root path is read only for migration. Maintain one source of truth.
        if let Err(error) = self.save_to_path(&persistence_path()) {
            eprintln!("Notification state was not saved: {error:#}");
        }
    }

    pub fn save_to_path(&self, path: &Path) -> anyhow::Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let mut read_ids: Vec<String> = self.read_ids.iter().cloned().collect();
        read_ids.sort();
        read_ids.truncate(MAX_PERSISTED_IDS);

        let mut dismissed_ids: Vec<String> = self.dismissed_ids.iter().cloned().collect();
        dismissed_ids.sort();
        dismissed_ids.truncate(MAX_PERSISTED_IDS);

        let cached_history: Vec<UnifiedNotificationItem> = self
            .cached_history
            .iter()
            .filter(|i| !self.dismissed_ids.contains(&i.id))
            .take(MAX_CACHED_HISTORY_LEN)
            .cloned()
            .collect();

        let payload = PersistedNotificationCenter {
            alert_mode: self.alert_mode,
            snooze_until_ms: self.snooze_until_ms,
            read_ids,
            dismissed_ids,
            popup_shown_ids: self.popup_shown_ids.iter().take(MAX_PERSISTED_IDS).cloned().collect(),
            delivered_events: self.delivered_events.clone(),
            cached_history,
        };
        let bytes = serde_json::to_vec_pretty(&payload)?;
        stepx_companion::local_store::write_if_changed(path, &bytes)?;
        Ok(())
    }

    pub fn should_show_badge(&self) -> bool {
        self.alert_mode.allows_badge() && self.items.iter().any(|i| i.unread)
    }

    pub fn is_snoozed(&self, now_ms: i64) -> bool {
        self.snooze_until_ms.is_some_and(|until| until > now_ms)
    }

    pub fn is_currently_snoozed(&self) -> bool {
        self.is_snoozed(Local::now().timestamp_millis())
    }

    pub fn popup_queue_len(&self) -> usize {
        self.popup_queue.len()
    }

    pub fn popup_position_label(&self) -> String {
        let total = self.popup_queue.len();
        if total == 0 {
            "0 / 0".into()
        } else {
            let cur = (self.popup_cursor + 1).min(total);
            format!("{cur} / {total}")
        }
    }

    pub fn popup_progress_ratio(&self) -> f32 {
        if self.popup_total_ms == 0 {
            return 1.0;
        }
        (self.popup_remaining_ms as f32 / self.popup_total_ms as f32).clamp(0.0, 1.0)
    }

    pub fn active_feedback(&self, now_ms: i64) -> Option<&str> {
        self.popup_feedback_text
            .as_ref()
            .and_then(|(text, exp)| (*exp > now_ms).then_some(text.as_str()))
    }
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut hash: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        hash ^= b as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }
    hash
}

pub(crate) fn truncate_concise(text: &str, max_chars: usize) -> String {
    let collapsed: String = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let trimmed = collapsed.trim();
    if trimmed.chars().count() <= max_chars {
        return trimmed.to_string();
    }
    let keep = max_chars.saturating_sub(1);
    let prefix: String = trimmed.chars().take(keep).collect();
    format!("{}…", prefix.trim_end())
}

fn short_app_display_name(raw: &str) -> String {
    let base = raw
        .split('·')
        .next()
        .unwrap_or(raw)
        .split('(')
        .next()
        .unwrap_or(raw)
        .split('（')
        .next()
        .unwrap_or(raw)
        .trim();
    if base.is_empty() {
        "手机应用".into()
    } else {
        truncate_concise(base, 10)
    }
}

fn extract_sms_brand_signature(body: &str) -> Option<String> {
    let trimmed = body.trim();
    if let Some(rest) = trimmed.strip_prefix('【') {
        if let Some((brand, _)) = rest.split_once('】') {
            let b = brand.trim();
            let count = b.chars().count();
            if (2..=10).contains(&count) {
                return Some(b.to_string());
            }
        }
    }
    if let Some(before) = trimmed.strip_suffix('】') {
        if let Some((_, brand)) = before.rsplit_once('【') {
            let b = brand.trim();
            let count = b.chars().count();
            if (2..=10).contains(&count) {
                return Some(b.to_string());
            }
        }
    }
    None
}

pub(crate) fn is_real_pickup_code_label(code: &str) -> bool {
    let trimmed = code.trim();
    !trimmed.is_empty()
        && trimmed.chars().any(|c| c.is_ascii_digit())
        && !trimmed.starts_with("进度")
        && !matches!(
            trimmed,
            "已送达" | "已送达家门口" | "派送中" | "实时配送" | "配送中"
        )
}

fn visual_and_badge_for_package(package: &str, app_name: &str) -> (NotificationVisualKind, String) {
    let trimmed = package.trim();
    if matches!(
        trimmed,
        "com.android.calendar"
            | "com.google.android.calendar"
            | "com.coloros.calendar"
            | "com.miui.calendar"
            | "com.huawei.calendar"
            | "com.samsung.android.calendar"
    ) {
        return (NotificationVisualKind::CalendarAlarm, "日历".into());
    }
    if matches!(
        trimmed,
        "com.android.deskclock"
            | "com.google.android.deskclock"
            | "com.android.alarmclock"
            | "com.coloros.alarmclock"
            | "com.miui.deskclock"
    ) {
        return (NotificationVisualKind::CalendarAlarm, "闹钟".into());
    }
    if matches!(
        trimmed,
        "com.android.dialer"
            | "com.android.server.telecom"
            | "com.google.android.dialer"
            | "com.android.incallui"
            | "com.android.phone"
    ) {
        return (NotificationVisualKind::MissedCall, "电话".into());
    }
    match ImChannel::from_package(trimmed) {
        ImChannel::WeChat => (NotificationVisualKind::WeChat, "微信".into()),
        ImChannel::WeCom => (NotificationVisualKind::WeChat, "企微".into()),
        ImChannel::Feishu => (NotificationVisualKind::Feishu, "飞书".into()),
        ImChannel::DingTalk => (NotificationVisualKind::DingTalk, "钉钉".into()),
        ImChannel::QqTim => (NotificationVisualKind::ImChat, "QQ".into()),
        ImChannel::Sms => (NotificationVisualKind::SmsOtp, "短信".into()),
        _ => {
            let lower = trimmed.to_ascii_lowercase();
            if lower.contains("telegram") {
                (NotificationVisualKind::ImChat, "TG".into())
            } else if lower.contains("bosszhipin") {
                (NotificationVisualKind::ImChat, "BOSS直聘".into())
            } else if lower.contains("maimai") {
                (NotificationVisualKind::ImChat, "脉脉".into())
            } else if lower.contains("lietpin") {
                (NotificationVisualKind::ImChat, "猎聘".into())
            } else if lower.contains("yaya.zone") {
                (NotificationVisualKind::ImChat, "叮咚买菜".into())
            } else if lower.contains("wudaokou.hippo") {
                (NotificationVisualKind::ImChat, "盒马".into())
            } else if lower.contains("samsclub") {
                (NotificationVisualKind::ImChat, "山姆".into())
            } else if lower.contains("pupumall") {
                (NotificationVisualKind::ImChat, "朴朴".into())
            } else if lower.contains("meituan") {
                (NotificationVisualKind::ImChat, "美团".into())
            } else if lower == "me.ele" {
                (NotificationVisualKind::ImChat, "饿了么".into())
            } else if !app_name.trim().is_empty() {
                (
                    NotificationVisualKind::ImChat,
                    truncate_concise(&short_app_display_name(app_name), 4),
                )
            } else {
                (NotificationVisualKind::ImChat, "应用".into())
            }
        }
    }
}

fn format_concise_time(ts_ms: i64, now_ms: i64) -> String {
    if ts_ms <= 0 {
        return "刚刚".into();
    }
    let diff_ms = now_ms.saturating_sub(ts_ms);
    if diff_ms >= 0 && diff_ms < 90_000 {
        return "刚刚".into();
    }
    use chrono::TimeZone;
    let Some(dt) = Local.timestamp_millis_opt(ts_ms).single() else {
        return "今日".into();
    };
    let today = Local::now().date_naive();
    let d = dt.date_naive();
    if d == today {
        dt.format("%H:%M").to_string()
    } else if today.pred_opt() == Some(d) {
        "昨天".into()
    } else {
        dt.format("%m-%d").to_string()
    }
}

fn classify_ai_value_thread(
    title: &str,
    last_message: &str,
    mentions_me: bool,
) -> Option<(&'static str, NotificationPriority, Option<String>)> {
    let matches_any = |keywords: &[&str]| -> bool {
        keywords
            .iter()
            .any(|kw| title.contains(kw) || last_message.contains(kw))
    };
    if matches_any(&[
        "圆通速递",
        "顺丰",
        "京东快递",
        "菜鸟",
        "丰巢",
        "中通",
        "韵达",
        "极兔",
        "快递",
        "取件",
        "派件",
        "签收",
        "驿站",
    ]) {
        return Some((
            "AI 快递提醒",
            NotificationPriority::P0Immediate,
            Some(format!(
                "快递动态：{} · {}",
                title.trim(),
                last_message.trim()
            )),
        ));
    }
    if matches_any(&[
        "叮咚买菜",
        "盒马",
        "山姆",
        "朴朴",
        "已代收",
        "家门口",
        "订单已送达",
    ]) {
        return Some((
            "AI 买菜送达",
            NotificationPriority::P0Immediate,
            Some(format!(
                "生鲜送达：{} · {}",
                title.trim(),
                last_message.trim()
            )),
        ));
    }
    if matches_any(&[
        "霸王茶姬",
        "瑞幸",
        "喜茶",
        "星巴克",
        "奈雪",
        "美团",
        "饿了么",
        "取餐",
        "取餐码",
        "取茶",
    ]) {
        return Some(("AI 取餐提醒", NotificationPriority::P0Immediate, None));
    }
    if matches_any(&[
        "BOSS直聘",
        "Boss直聘",
        "脉脉",
        "猎聘",
        "智联招聘",
        "前程无忧",
        "面试邀约",
        "查看了你的简历",
        "交换微信",
    ]) {
        return Some(("AI 招聘消息", NotificationPriority::P0Immediate, None));
    }
    if mentions_me {
        return Some(("AI @提及我", NotificationPriority::P0Immediate, None));
    }
    if matches_any(&[
        "粤海水务",
        "停水",
        "停电",
        "燃气",
        "物业",
        "航班",
        "高铁",
        "登机口",
        "检票口",
    ]) {
        return Some(("AI 民生提醒", NotificationPriority::P0Immediate, None));
    }
    if matches_any(&[
        "微信支付",
        "支付宝",
        "已支付",
        "收款",
        "到账",
        "账单",
        "工资条",
    ]) {
        return Some(("AI 账单提醒", NotificationPriority::P1Important, None));
    }
    None
}

fn popup_delivery_event_suffix(item: &UnifiedNotificationItem) -> String {
    // Clipboard reads have no source event timestamp. Re-reading the same
    // value must not turn it into a new alert after restarting the desktop.
    let stamp = if item.id.starts_with("phone-clipboard:") { 0 } else { item.sort_ts_ms };
    format!("|{}|{:016x}", item.id, fnv1a64(format!("{}|{}|{stamp}", item.title, item.full_body).as_bytes()))
}

impl Companion {
    fn popup_delivery_key(&self, item: &UnifiedNotificationItem) -> String {
        let serial = self.selected.as_ref().map(|device| device.serial.as_str()).unwrap_or("desktop");
        format!("{serial}{}", popup_delivery_event_suffix(item))
    }

    pub(crate) fn popup_batch_receipts(&self) -> Vec<(String, String)> {
        self.notification_center.items.iter()
            .filter(|item| !item.id.starts_with("live-call:")
                && !matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. })
                && (self.notification_center.popup_queue.contains(&item.id)
                    || self.notification_center.active_popup_id.as_ref() == Some(&item.id)
                    || self.notification_center.popup_pending_front.as_ref() == Some(&item.id)))
            .map(|item| (item.id.clone(), self.popup_delivery_key(item)))
            .collect()
    }

    fn clear_popup_batch(&mut self, receipts: Vec<(String, String)>) {
        let events: HashSet<_> = receipts.iter().map(|(_, key)| key.as_str()).collect();
        let clear_ids: HashSet<_> = self.notification_center.items.iter()
            .filter(|item| !item.id.starts_with("live-call:")
                && !matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. })
                && events.contains(self.popup_delivery_key(item).as_str()))
            .map(|item| item.id.clone()).collect();
        // Persist the exact batch even if an item has since received a new
        // event. Only the captured event is acknowledged; the new one stays.
        for (id, key) in receipts {
            if id.starts_with("live-call:") { continue; }
            if !self.notification_center.delivered_events.contains(&key) {
                self.notification_center.delivered_events.push_front(key);
            }
        }
        self.notification_center.delivered_events.truncate(MAX_PERSISTED_IDS);
        self.notification_center.popup_queue.retain(|id| !clear_ids.contains(id));
        // A live-call card may already be visible before a queue rebuild has
        // completed. Keep that control surface even in this transitional state.
        let protected_calls: Vec<_> = self.notification_center.items.iter()
            .filter(|item| matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. })
                && (self.notification_center.active_popup_id.as_ref() == Some(&item.id)
                    || self.notification_center.popup_pending_front.as_ref() == Some(&item.id)))
            .map(|item| item.id.clone()).collect();
        for id in protected_calls {
            if !self.notification_center.popup_queue.contains(&id) {
                self.notification_center.popup_queue.push_front(id);
            }
        }
        if self.notification_center.popup_pending_front.as_ref().is_some_and(|id| clear_ids.contains(id)) {
            self.notification_center.popup_pending_front = None;
        }
        if let Some(active) = self.notification_center.active_popup_id.as_ref() {
            if let Some(index) = self.notification_center.popup_queue.iter().position(|id| id == active) {
                self.notification_center.popup_cursor = index;
            } else {
                self.notification_center.popup_cursor = 0;
            }
        }
        self.notification_center.popup_hovered = false;
        self.sync_active_popup_from_queue(false);
        // History, unread state, reply drafts and reminder preferences remain.
        self.notification_center.save();
    }
    pub(crate) fn active_express_parcels(&self) -> &[ExpressParcelInfo] {
        &self.express_parcels
    }

    pub(crate) fn notification_unread_count(&self) -> usize {
        if !self.notification_center.alert_mode.allows_badge() {
            return 0;
        }
        self.notification_center
            .items
            .iter()
            .filter(|i| i.unread)
            .count()
    }

    pub(crate) fn notification_raw_unread_count(&self) -> usize {
        self.notification_center
            .items
            .iter()
            .filter(|i| i.unread)
            .count()
    }

    pub(crate) fn notification_category_unread(&self, cat: NotificationCategory) -> usize {
        match cat {
            NotificationCategory::All => self.notification_raw_unread_count(),
            other => self
                .notification_center
                .items
                .iter()
                .filter(|i| i.unread && i.category == other)
                .count(),
        }
    }

    pub(crate) fn notification_visible_items(&self) -> Vec<&UnifiedNotificationItem> {
        let cat = self.notification_center.selected_category;
        let has_update_summary = self.notification_center.items.iter().any(|i| {
            i.id == "app-updates-summary" && !self.notification_center.dismissed_ids.contains(&i.id)
        });
        self.notification_center
            .items
            .iter()
            .filter(|item| {
                if self.notification_center.dismissed_ids.contains(&item.id) {
                    return false;
                }
                if cat == NotificationCategory::All {
                    if has_update_summary && item.id.starts_with("app-update:") && item.unread {
                        return false;
                    }
                    true
                } else {
                    item.category == cat
                }
            })
            .collect()
    }

    pub(crate) fn notification_active_popup(&self) -> Option<&UnifiedNotificationItem> {
        let now_ms = Local::now().timestamp_millis();
        let all_silent = self.push.prefs.popup_mode
            == stepx_companion::desktop_notify::PopupMode::Silent
            && self
                .push
                .prefs
                .app_popup_modes
                .values()
                .all(|m| *m == stepx_companion::desktop_notify::PopupMode::Silent);
        if self.notification_center.alert_mode != NotificationAlertMode::PopupAndBadge
            || all_silent
            || self.notification_center.is_snoozed(now_ms)
        {
            return None;
        }
        let popup_id = self
            .notification_center
            .active_popup_id
            .as_deref()
            .or_else(|| {
                self.notification_center
                    .popup_queue
                    .get(self.notification_center.popup_cursor)
                    .map(String::as_str)
            })?;
        self.notification_center.items.iter().find(|item| {
            item.id == popup_id
                && item.unread
                && self.push.prefs.effective_popup_mode(&item.package_name)
                    != stepx_companion::desktop_notify::PopupMode::Silent
        })
    }

    fn is_item_popup_eligible(
        item: &UnifiedNotificationItem,
        prefs: &stepx_companion::desktop_notify::NotifyPreferences,
        gov: &stepx_companion::home_reminder_governance::HomeReminderGovernance,
    ) -> bool {
        if !item.unread {
            return false;
        }
        if !matches!(
            item.priority,
            NotificationPriority::P0Immediate | NotificationPriority::P1Digest
        ) {
            return false;
        }
        let app_popup_mode = prefs.effective_popup_mode(&item.package_name);
        if app_popup_mode == stepx_companion::desktop_notify::PopupMode::Silent {
            return false;
        }
        let norm_pkg = stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(
            &item.package_name,
        );
        let raw_pkg = item.package_name.trim();
        if (!norm_pkg.is_empty()
            && (gov.muted_sources.contains_key(&norm_pkg)
                || gov.muted_sources.contains_key(&format!("pkg:{norm_pkg}"))))
            || (!raw_pkg.is_empty()
                && (gov.muted_sources.contains_key(raw_pkg)
                    || gov.muted_sources.contains_key(&format!("pkg:{raw_pkg}"))))
        {
            return false;
        }
        let is_otp = matches!(item.visual, NotificationVisualKind::SmsOtp)
            || matches!(
                item.popup_widget,
                NotificationPopupWidget::OtpCode { .. }
                    | NotificationPopupWidget::ExpressPickup { .. }
            );
        let is_im_or_comm = matches!(
            item.visual,
            NotificationVisualKind::WeChat
                | NotificationVisualKind::Feishu
                | NotificationVisualKind::DingTalk
                | NotificationVisualKind::ImChat
                | NotificationVisualKind::IncomingCall
                | NotificationVisualKind::MissedCall
                | NotificationVisualKind::SmsOtp
                | NotificationVisualKind::CalendarAlarm
                | NotificationVisualKind::Express
                | NotificationVisualKind::SafetyApproval
        ) || matches!(
            item.popup_widget,
            NotificationPopupWidget::ClipboardPreview { .. }
        );
        if !prefs.allows_channel_in_scope(&item.package_name, is_im_or_comm, is_otp, true) {
            return false;
        }
        true
    }

    fn sync_popup_reply_draft(&mut self, next_id: Option<String>) {
        if self.notification_center.popup_reply_item_id == next_id {
            return;
        }
        self.desktop_popup.reply_focused = false;
        if let Some(previous) = self.notification_center.popup_reply_item_id.take() {
            let draft = self.fields[super::F_TOAST_REPLY].text.clone();
            if draft.is_empty() {
                self.notification_center
                    .popup_reply_drafts
                    .remove(&previous);
            } else {
                self.notification_center
                    .popup_reply_drafts
                    .insert(previous, draft);
            }
        }
        let draft = next_id
            .as_ref()
            .and_then(|id| self.notification_center.popup_reply_drafts.get(id))
            .cloned()
            .unwrap_or_default();
        self.fields[super::F_TOAST_REPLY].set(&draft);
        self.notification_center.popup_reply_item_id = next_id;
    }

    fn sync_active_popup_from_queue(&mut self, _reset_timer_if_changed: bool) {
        let now_ms = Local::now().timestamp_millis();
        let all_silent = self.push.prefs.popup_mode
            == stepx_companion::desktop_notify::PopupMode::Silent
            && self
                .push
                .prefs
                .app_popup_modes
                .values()
                .all(|m| *m == stepx_companion::desktop_notify::PopupMode::Silent);
        let next_id = if self.notification_center.alert_mode != NotificationAlertMode::PopupAndBadge
            || all_silent
            || self.notification_center.is_snoozed(now_ms)
            || self.notification_center.popup_queue.is_empty()
        {
            self.notification_center.popup_cursor = 0;
            None
        } else {
            if self.notification_center.popup_cursor >= self.notification_center.popup_queue.len() {
                self.notification_center.popup_cursor = 0;
            }
            self.notification_center
                .popup_queue
                .get(self.notification_center.popup_cursor)
                .cloned()
        };
        self.sync_popup_reply_draft(next_id.clone());
        self.notification_center.active_popup_id = next_id;
        if let Some(item) = self.notification_center.items.iter().find(|item| Some(&item.id) == self.notification_center.active_popup_id.as_ref()) {
            let key = self.popup_delivery_key(item);
            if !self.notification_center.delivered_events.contains(&key) {
                self.notification_center.delivered_events.push_front(key);
                self.notification_center.delivered_events.truncate(MAX_PERSISTED_IDS);
                self.notification_center.save();
            }
        }
        // A notification stays until it is handled, closed or read on the phone.
        self.notification_center.popup_total_ms = 0;
        self.notification_center.popup_remaining_ms = 0;
    }

    fn select_pending_popup(&mut self) -> bool {
        let field = &self.fields[super::F_TOAST_REPLY];
        if self.desktop_popup.reply_focused || !field.text.is_empty() || !field.preedit.is_empty()
            || self.notification_center.popup_hovered || self.notification_center.popup_menu_item_id.is_some()
        {
            return false;
        }
        let Some(id) = self.notification_center.popup_pending_front.as_ref() else { return false; };
        // A live carrier call retains its controls until it ends.
        if self.notification_active_popup().is_some_and(|item| matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. }))
            && !id.starts_with("live-call:")
        {
            return false;
        }
        let index = self.notification_center.popup_queue.iter().position(|queued| queued == id);
        self.notification_center.popup_pending_front = None;
        if let Some(index) = index {
            self.notification_center.popup_cursor = index;
            true
        } else { false }
    }

    pub(crate) fn tick_notification_popup_timer(&mut self, _elapsed_ms: u32) -> bool {
        let now_ms = Local::now().timestamp_millis();
        let mut changed = false;
        if self.select_pending_popup() {
            self.sync_active_popup_from_queue(false);
            changed = true;
        }
        if self
            .notification_center
            .popup_feedback_text
            .as_ref()
            .is_some_and(|(_, exp)| *exp <= now_ms)
        {
            self.notification_center.popup_feedback_text = None;
            changed = true;
        }
        if self
            .notification_center
            .snooze_until_ms
            .is_some_and(|until| until <= now_ms)
        {
            self.notification_center.snooze_until_ms = None;
            changed = true;
        }
        changed
    }

    pub(crate) fn item_action_focuses_main_window(&self, item_id: &str) -> bool {
        if let Some(item) = self
            .notification_center
            .items
            .iter()
            .find(|i| i.id == item_id)
        {
            if let Some((_, ref action)) = item.primary_action {
                return match action {
                    NotificationPrimaryAction::CopyText { .. }
                    | NotificationPrimaryAction::ApproveGuiSafety(_) => false,
                    NotificationPrimaryAction::OpenImThread { .. }
                    | NotificationPrimaryAction::OpenSms
                    | NotificationPrimaryAction::OpenAppUpdates
                    | NotificationPrimaryAction::UpdateSingleApp { .. }
                    | NotificationPrimaryAction::ResumeTask { .. }
                    | NotificationPrimaryAction::OpenPage(_) => true,
                };
            }
        }
        !(item_id.starts_with("otp:")
            || item_id.starts_with("phone-clipboard:")
            || item_id == "gui-safety-approval"
            || (item_id.starts_with("express:") && !item_id.ends_with(':')))
    }

    pub(crate) fn express_parcel_timestamp_ms(
        &self,
        parcel: &stepx_companion::device::ExpressParcelInfo,
        now_ms: i64,
    ) -> i64 {
        let code = parcel.pickup_code.trim();
        let tracking = parcel.tracking_no.trim();
        let station = parcel.station.trim();

        let mut best_ts: i64 = 0;

        for m in &self.sms {
            if m.outgoing || m.date_ms <= 0 {
                continue;
            }
            let body = m.body.trim();
            let matched = (!code.is_empty() && body.contains(code))
                || (!tracking.is_empty()
                    && !tracking.starts_with("WX-")
                    && body.contains(tracking))
                || (code.is_empty()
                    && !station.is_empty()
                    && station != "手机通知提取"
                    && body.contains(station));
            if matched && m.date_ms > best_ts {
                best_ts = m.date_ms;
            }
        }

        for n in &self.push.snapshot.notifications {
            if n.post_time_ms <= 0 {
                continue;
            }
            let combined = format!("{} {}", n.title, n.text);
            let matched = (!code.is_empty() && combined.contains(code))
                || (!tracking.is_empty()
                    && !tracking.starts_with("WX-")
                    && combined.contains(tracking))
                || (code.is_empty()
                    && !station.is_empty()
                    && station != "手机通知提取"
                    && combined.contains(station));
            if matched && n.post_time_ms > best_ts {
                best_ts = n.post_time_ms;
            }
        }

        for t in &self.push.snapshot.threads {
            let last_msg_ts = t.messages.iter().map(|m| m.timestamp_ms).max().unwrap_or(0);
            let eff_ts = t.last_timestamp_ms.max(last_msg_ts);
            if eff_ts <= 0 {
                continue;
            }
            let wx_trk = format!("WX-{}", t.title.trim());
            let matched = (!tracking.is_empty() && tracking == wx_trk)
                || (!code.is_empty() && t.last_message.contains(code))
                || (!tracking.is_empty()
                    && !tracking.starts_with("WX-")
                    && t.last_message.contains(tracking));
            if matched && eff_ts > best_ts {
                best_ts = eff_ts;
            }
        }

        if best_ts > 0 {
            return best_ts;
        }

        let updated = parcel.updated_at.trim();
        if updated.is_empty() {
            return 0;
        }
        if !code.is_empty()
            && (updated == "刚刚"
                || updated == "今日"
                || updated.starts_with("今天")
                || (updated.contains(':') && !updated.contains('-')))
        {
            return now_ms.saturating_sub(60_000);
        }
        0
    }

    pub(crate) fn seed_connection_baseline(&mut self) {
        let current_serial = self
            .selected
            .as_ref()
            .map(|d| d.serial.clone())
            .or_else(|| self.auto_sync.device_serial.clone());
        let Some(serial) = current_serial else {
            self.notification_center.baseline_push_seeded_for_serial = None;
            self.notification_center.baseline_sms_seeded_for_serial = None;
            self.notification_center.baseline_calls_seeded_for_serial = None;
            self.notification_center.baseline_apps_seeded_for_serial = None;
            self.notification_center.baseline_suite_seeded_for_serial = None;
            self.notification_center.baseline_clip_seeded_for_serial = None;
            return;
        };

        let target_serial = Some(serial);

        let ready = !self.initializing || self.notification_center.connect_baseline_enabled;
        let now_ms = Local::now().timestamp_millis();

        // 1. Push baseline (threads, notifications, otp_banners, express_parcels)
        if (ready
            || !self.push.snapshot.threads.is_empty()
            || !self.push.snapshot.notifications.is_empty()
            || !self.push.snapshot.otp_banners.is_empty())
            && self.notification_center.baseline_push_seeded_for_serial != target_serial
        {
            self.notification_center.baseline_push_seeded_for_serial = target_serial.clone();
            // Seed history once for this connection. It also contains live notifications
            // after the first snapshot; re-seeding on every refresh would immediately
            // mark new messages read and withdraw their desktop popup before it paints.
            for hist in &self.notification_center.cached_history {
                self.notification_center.read_ids.insert(hist.id.clone());
                self.notification_center
                    .popup_shown_ids
                    .insert(hist.id.clone());
            }
            for (time_str, title, detail) in &self.interaction.notices {
                let notice_id = format!(
                    "interaction-notice:{:016x}",
                    fnv1a64(format!("{time_str}:{title}:{detail}").as_bytes())
                );
                self.notification_center.read_ids.insert(notice_id.clone());
                self.notification_center.popup_shown_ids.insert(notice_id);
            }
            for thread in &self.push.snapshot.threads {
                let trimmed_title = thread.title.trim();
                let tid_key = if thread.thread_id.trim().is_empty() {
                    trimmed_title.to_string()
                } else {
                    thread.thread_id.trim().to_string()
                };
                if tid_key.is_empty() {
                    continue;
                }
                let last_msg_ts = thread
                    .messages
                    .iter()
                    .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                    .max()
                    .unwrap_or(0);
                let ts_ms = thread.last_timestamp_ms.max(last_msg_ts);
                let item_id = format!("im-thread:{tid_key}");
                let msg_sig = format!(
                    "{}|{}|{}",
                    thread.unread_count,
                    ts_ms,
                    thread.last_message.trim()
                );
                self.notification_center.read_ids.insert(item_id.clone());
                self.notification_center.popup_shown_ids.insert(item_id);
                self.notification_center
                    .last_thread_unread
                    .insert(tid_key.clone(), (thread.unread_count, ts_ms));
                self.notification_center
                    .last_thread_sig
                    .insert(tid_key, msg_sig);
            }
            for notif in &self.push.snapshot.notifications {
                let item_id = format!(
                    "push-notif:{}",
                    if notif.key.trim().is_empty() {
                        format!("seq-{}", notif.seq_id)
                    } else {
                        notif.key.trim().to_string()
                    }
                );
                let notif_sig = format!(
                    "{}|{}|{}",
                    notif.post_time_ms,
                    notif.seq_id,
                    notif.text.trim()
                );
                self.notification_center.read_ids.insert(item_id.clone());
                self.notification_center
                    .popup_shown_ids
                    .insert(item_id.clone());
                self.notification_center
                    .last_notif_sig
                    .insert(item_id, notif_sig);
                if notif.post_time_ms <= 0 || now_ms.saturating_sub(notif.post_time_ms) > 300_000 {
                    if let Some(code) = notif
                        .otp_code
                        .clone()
                        .filter(|c| !c.trim().is_empty())
                        .or_else(|| extract_otp_code(&format!("{} {}", notif.title, notif.text)))
                    {
                        let otp_id = format!("otp:{}", code.trim());
                        self.notification_center.read_ids.insert(otp_id.clone());
                        self.notification_center.popup_shown_ids.insert(otp_id);
                    }
                }
            }
            for banner in &self.push.snapshot.otp_banners {
                let code = banner.code.trim();
                if !code.is_empty()
                    && (banner.timestamp_ms <= 0
                        || now_ms.saturating_sub(banner.timestamp_ms) > 300_000)
                {
                    let otp_id = format!("otp:{code}");
                    self.notification_center.read_ids.insert(otp_id.clone());
                    self.notification_center.popup_shown_ids.insert(otp_id);
                }
            }
            for (idx, parcel) in self.express_parcels.iter().enumerate() {
                let tracking = parcel.tracking_no.trim();
                let code = parcel.pickup_code.trim();
                let express_id = format!(
                    "express:{}:{}",
                    if tracking.is_empty() {
                        idx.to_string()
                    } else {
                        tracking.to_string()
                    },
                    code
                );
                self.notification_center.read_ids.insert(express_id.clone());
                self.notification_center.popup_shown_ids.insert(express_id);
            }
        }

        // 2. SMS baseline
        if (ready || !self.sms.is_empty())
            && self.notification_center.baseline_sms_seeded_for_serial != target_serial
        {
            self.notification_center.baseline_sms_seeded_for_serial = target_serial.clone();
            for msg in &self.sms {
                let sender = if msg.address.trim().is_empty() {
                    "未知号码"
                } else {
                    msg.address.trim()
                };
                let sms_id = format!("sms:{}:{}", sender, msg.date_ms);
                self.notification_center.read_ids.insert(sms_id.clone());
                self.notification_center.popup_shown_ids.insert(sms_id);
                if msg.date_ms <= 0 || now_ms.saturating_sub(msg.date_ms) > 300_000 {
                    if let Some(code) = extract_otp_code(&msg.body) {
                        let otp_id = format!("otp:{}", code.trim());
                        self.notification_center.read_ids.insert(otp_id.clone());
                        self.notification_center.popup_shown_ids.insert(otp_id);
                    }
                }
            }
            for (idx, parcel) in self.express_parcels.iter().enumerate() {
                let tracking = parcel.tracking_no.trim();
                let code = parcel.pickup_code.trim();
                let express_id = format!(
                    "express:{}:{}",
                    if tracking.is_empty() {
                        idx.to_string()
                    } else {
                        tracking.to_string()
                    },
                    code
                );
                self.notification_center.read_ids.insert(express_id.clone());
                self.notification_center.popup_shown_ids.insert(express_id);
            }
        }

        // 3. Calls baseline
        if (ready || !self.call_logs.is_empty())
            && self.notification_center.baseline_calls_seeded_for_serial != target_serial
        {
            self.notification_center.baseline_calls_seeded_for_serial = target_serial.clone();
            for call in &self.call_logs {
                let call_id = format!("missed-call:{}:{}", call.number.trim(), call.date_ms);
                self.notification_center.read_ids.insert(call_id.clone());
                self.notification_center.popup_shown_ids.insert(call_id);
            }
        }

        // 4. Apps baseline
        if (ready || !self.apps.is_empty())
            && self.notification_center.baseline_apps_seeded_for_serial != target_serial
        {
            self.notification_center.baseline_apps_seeded_for_serial = target_serial.clone();
            self.notification_center
                .read_ids
                .insert("app-updates-summary".into());
            self.notification_center
                .popup_shown_ids
                .insert("app-updates-summary".into());
            let installed_map: HashMap<&str, &stepx_companion::apps::AppInfo> =
                self.apps.iter().map(|a| (a.package.as_str(), a)).collect();
            for store_app in &self.app_store.catalog {
                if let Some(&installed_entry) = installed_map.get(store_app.package.as_str()) {
                    if let StoreInstallState::Updatable { latest_version, .. } =
                        check_store_install_state(std::slice::from_ref(installed_entry), store_app)
                    {
                        let short_name = short_app_display_name(&store_app.name);
                        self.notification_center
                            .seen_updatable_apps
                            .insert(store_app.package.clone(), (short_name, latest_version));
                        let item_id = format!("app-update:{}", store_app.package);
                        self.notification_center.read_ids.insert(item_id.clone());
                        self.notification_center.popup_shown_ids.insert(item_id);
                    }
                }
            }
        }

        // 5. Suite baseline (calendar, alarms, tasks)
        if (ready
            || !self.back_screen.stepos_suite.calendar_events.is_empty()
            || !self.back_screen.stepos_suite.alarms.is_empty()
            || !self.workspace.journal.tasks.is_empty())
            && self.notification_center.baseline_suite_seeded_for_serial != target_serial
        {
            self.notification_center.baseline_suite_seeded_for_serial = target_serial.clone();
            let now_ms = Local::now().timestamp_millis();
            for ev in &self.back_screen.stepos_suite.calendar_events {
                if (ev.dtend > 0 && ev.dtend <= now_ms)
                    || (ev.dtend <= 0 && now_ms.saturating_sub(ev.dtstart) > 3_600_000)
                {
                    let cal_id = format!("calendar:{}", ev.id);
                    self.notification_center.read_ids.insert(cal_id.clone());
                    self.notification_center.popup_shown_ids.insert(cal_id);
                }
            }
            for alarm in &self.back_screen.stepos_suite.alarms {
                if !alarm.enabled {
                    let alarm_id = format!("alarm:{}", alarm.id);
                    self.notification_center.read_ids.insert(alarm_id.clone());
                    self.notification_center.popup_shown_ids.insert(alarm_id);
                }
            }
            for task in &self.workspace.journal.tasks {
                let task_id = format!("task:{}", task.id);
                self.notification_center.read_ids.insert(task_id.clone());
                self.notification_center.popup_shown_ids.insert(task_id);
            }
        }

        // 6. Clipboard baseline
        let clip_a = self.back_screen.stepos_suite.phone_clipboard.trim();
        let clip_b = self.back_screen.last_synced_phone_clip.trim();
        if (!clip_a.is_empty() || !clip_b.is_empty())
            && self.notification_center.baseline_clip_seeded_for_serial != target_serial
        {
            self.notification_center.baseline_clip_seeded_for_serial = target_serial;
            for clip in [clip_a, clip_b] {
                if !clip.is_empty() {
                    let clip_id = format!("phone-clipboard:{:016x}", fnv1a64(clip.as_bytes()));
                    self.notification_center.read_ids.insert(clip_id.clone());
                    self.notification_center.popup_shown_ids.insert(clip_id);
                }
            }
        }
    }

    pub(crate) fn tick_notification_center(&mut self) {
        // Real data receipts and user actions call sync_notification_center
        // immediately. The timer only needs to refresh time-dependent reminders;
        // rebuilding all conversations at the animation rate stalls native input.
        if self
            .notification_center
            .last_materialized_at
            .is_none_or(|at| at.elapsed() >= std::time::Duration::from_secs(1))
        {
            self.sync_notification_center();
        }
    }

    pub(crate) fn sync_notification_center(&mut self) {
        self.notification_center.last_materialized_at = Some(std::time::Instant::now());
        let now_ms = Local::now().timestamp_millis();
        let mut raw_items: Vec<UnifiedNotificationItem> = Vec::with_capacity(24);
        let mut seen_ids: HashSet<String> = HashSet::with_capacity(32);

        if !self.demo_mode
            && (!cfg!(test)
                || (self.express_parcels.is_empty() && !self.push.snapshot.threads.is_empty()))
        {
            self.sync_live_express_parcels();
        }

        if self.selected.is_none() {
            self.notification_center.baseline_push_seeded_for_serial = None;
            self.notification_center.baseline_sms_seeded_for_serial = None;
            self.notification_center.baseline_calls_seeded_for_serial = None;
            self.notification_center.baseline_apps_seeded_for_serial = None;
            self.notification_center.baseline_suite_seeded_for_serial = None;
            self.notification_center.baseline_clip_seeded_for_serial = None;
        } else if !self.demo_mode
            && (self.notification_center.connect_baseline_enabled
                || self.push.loaded
                || self.sms_loaded
                || self.phone_loaded
                || self.apps_loaded)
        {
            self.seed_connection_baseline();
        }

        // =====================================================================
        // 1. PhoneApps: IM threads & phone app push notifications
        // =====================================================================
        let mut covered_thread_ids: HashSet<String> = HashSet::new();
        for thread in &self.push.snapshot.threads {
            let trimmed_title = thread.title.trim();
            if trimmed_title.starts_with('{')
                || (thread.thread_id.trim().is_empty() && trimmed_title.is_empty())
            {
                continue;
            }
            let tid_key = if thread.thread_id.trim().is_empty() {
                trimmed_title.to_string()
            } else {
                thread.thread_id.trim().to_string()
            };
            covered_thread_ids.insert(tid_key.clone());
            let item_id = format!("im-thread:{tid_key}");

            let ai_value = classify_ai_value_thread(
                trimmed_title,
                &thread.last_message,
                stepx_companion::push::thread_mentions_current_user(
                    thread,
                    &self.push.snapshot.notifications,
                ),
            )
            .or_else(|| {
                stepx_companion::push::is_career_or_recruitment_signal(
                    &thread.package_name,
                    &thread.app_name,
                    trimmed_title,
                    &thread.last_message,
                )
                .then_some((
                    "AI 招聘消息",
                    NotificationPriority::P0Immediate,
                    None,
                ))
            });

            let last_msg_ts = thread
                .messages
                .iter()
                .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                .max()
                .unwrap_or(0);
            let ts_ms = thread.last_timestamp_ms.max(last_msg_ts);
            if ts_ms <= 0 {
                continue;
            }

            let msg_sig = format!(
                "{}|{}|{}",
                thread.unread_count,
                ts_ms,
                thread.last_message.trim()
            );
            let prev_sig = self
                .notification_center
                .last_thread_sig
                .insert(tid_key.clone(), msg_sig.clone());
            let prev_state = self
                .notification_center
                .last_thread_unread
                .insert(tid_key.clone(), (thread.unread_count, ts_ms));
            let is_outgoing_tail = thread.last_sender.trim() == "我"
                || thread.messages.last().is_some_and(|m| m.outgoing);
            let marked_read_same_msg = thread.unread_count == 0
                && prev_state.is_some_and(|(prev_u, prev_ts)| prev_u > 0 && ts_ms == prev_ts)
                && prev_sig.as_deref().is_some_and(|s| {
                    s.ends_with(&format!("|{ts_ms}|{}", thread.last_message.trim()))
                });
            if prev_sig.as_deref() != Some(msg_sig.as_str())
                && !marked_read_same_msg
                && !thread.last_message.trim().is_empty()
                && !is_outgoing_tail
            {
                if prev_sig.is_some() {
                    self.notification_center.read_ids.remove(&item_id);
                    self.notification_center.dismissed_ids.remove(&item_id);
                    self.notification_center.popup_shown_ids.remove(&item_id);
                } else if thread.unread_count == 0 {
                    self.notification_center.read_ids.insert(item_id.clone());
                }
            } else if thread.unread_count == 0
                && (prev_sig.is_none() || marked_read_same_msg || is_outgoing_tail)
            {
                self.notification_center.read_ids.insert(item_id.clone());
            } else if let Some((prev_unread, prev_ts)) = prev_state {
                if (prev_unread == 0 && thread.unread_count > 0) || (ts_ms > prev_ts && prev_ts > 0)
                {
                    self.notification_center.read_ids.remove(&item_id);
                    self.notification_center.dismissed_ids.remove(&item_id);
                    self.notification_center.popup_shown_ids.remove(&item_id);
                }
            }

            let (visual, badge) =
                visual_and_badge_for_package(&thread.package_name, &thread.app_name);
            let display_title = if trimmed_title.is_empty() {
                badge.clone()
            } else {
                truncate_concise(trimmed_title, 20)
            };
            let raw_summary = if !thread.last_sender.trim().is_empty()
                && thread.last_sender.trim() != trimmed_title
            {
                format!(
                    "{}: {}",
                    thread.last_sender.trim(),
                    thread.last_message.trim()
                )
            } else if !thread.last_message.trim().is_empty() {
                thread.last_message.trim().to_string()
            } else {
                "收到新消息".into()
            };
            let unread = (thread.unread_count > 0
                || !self.notification_center.read_ids.contains(&item_id))
                && !self.notification_center.read_ids.contains(&item_id);
            let (priority, status_tag, ai_summary) =
                if let Some((ai_tag, ai_prio, ai_sum)) = ai_value {
                    (ai_prio, Some(ai_tag.into()), ai_sum)
                } else {
                    let default_tag = if unread {
                        Some(if thread.unread_count > 1 {
                            format!("{}条未读", thread.unread_count)
                        } else {
                            "未读".into()
                        })
                    } else {
                        Some("已读".into())
                    };
                    (NotificationPriority::P0Immediate, default_tag, None)
                };
            let summary_text = ai_summary.as_deref().unwrap_or(&raw_summary);
            let app_label = if !thread.app_name.trim().is_empty() {
                short_app_display_name(thread.app_name.trim())
            } else {
                badge.clone()
            };
            let sender_label = if !thread.last_sender.trim().is_empty() {
                thread.last_sender.trim().to_string()
            } else if !trimmed_title.is_empty() {
                trimmed_title.to_string()
            } else {
                app_label.clone()
            };
            let full_body = if !thread.last_message.trim().is_empty() {
                // Keep the original payload for exact media binding. Presentation
                // adds the sender separately; summaries may contain a prefix.
                thread.last_message.trim().to_owned()
            } else {
                summary_text.to_string()
            };
            let banner_body = if !sender_label.is_empty() {
                format!("{} · 收到来自「{}」的新消息", badge, sender_label)
            } else {
                format!(
                    "{} · 收到 {} 条新消息提醒",
                    badge,
                    thread.unread_count.max(1)
                )
            };

            let thread_category = if matches!(
                visual,
                NotificationVisualKind::CalendarAlarm | NotificationVisualKind::MissedCall
            ) {
                NotificationCategory::PhoneComm
            } else {
                NotificationCategory::PhoneApps
            };
            let thread_popup_widget = if matches!(
                visual,
                NotificationVisualKind::WeChat
                    | NotificationVisualKind::Feishu
                    | NotificationVisualKind::DingTalk
                    | NotificationVisualKind::ImChat
            ) {
                NotificationPopupWidget::InlineImReply {
                    thread_id: tid_key.clone(),
                    package_name: thread.package_name.clone(),
                    can_reply: thread.can_remote_reply,
                    quick_phrases: vec![
                        "收到，谢谢".into(),
                        "好的，马上处理".into(),
                        "稍后回复".into(),
                    ],
                }
            } else {
                NotificationPopupWidget::None
            };

            if seen_ids.insert(item_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: item_id,
                    category: thread_category,
                    visual,
                    priority,
                    popup_widget: thread_popup_widget,
                    package_name: thread.package_name.clone(),
                    app_name: app_label,
                    sender: sender_label,
                    full_body,
                    banner_body,
                    source_badge: badge,
                    title: display_title,
                    summary: truncate_concise(summary_text, 64),
                    time_label: format_concise_time(ts_ms, now_ms),
                    sort_ts_ms: ts_ms,
                    unread,
                    status_tag,
                    primary_action: Some((
                        "回复".into(),
                        NotificationPrimaryAction::OpenImThread {
                            thread_id: thread.thread_id.clone(),
                        },
                    )),
                });
            }
        }

        // Non-thread app push notifications in `self.push.snapshot.notifications`
        for notif in &self.push.snapshot.notifications {
            if (matches!(ImChannel::from_package(&notif.package_name), ImChannel::Sms)
                || stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(
                    &notif.package_name,
                ) == "com.android.mms")
                && !self.sms.is_empty()
            {
                continue;
            }
            let delivery_sig = stepx_companion::home_hub::classify_delivery_or_mobility_signal(
                &notif.package_name,
                &notif.app_name,
                &notif.title,
                &notif.text,
            );
            if !notif.active
                || notif.dismissed
                || (notif.is_ongoing && delivery_sig.is_none())
                || notif.post_time_ms <= 0
                || (now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(notif.post_time_ms) > 86_400_000)
            {
                continue;
            }
            if !notif.thread_id.trim().is_empty()
                && covered_thread_ids.contains(notif.thread_id.trim())
            {
                continue;
            }
            if notif.otp_code.is_some() || extract_otp_code(&notif.text).is_some() {
                continue;
            }
            let item_id = format!(
                "push-notif:{}",
                if notif.key.trim().is_empty() {
                    format!("seq-{}", notif.seq_id)
                } else {
                    notif.key.trim().to_string()
                }
            );
            let notif_sig = format!(
                "{}|{}|{}",
                notif.post_time_ms,
                notif.seq_id,
                notif.text.trim()
            );
            let prev_notif_sig = self
                .notification_center
                .last_notif_sig
                .insert(item_id.clone(), notif_sig.clone());
            if prev_notif_sig.as_deref() != Some(notif_sig.as_str())
                && prev_notif_sig.is_some()
                && !notif.text.trim().is_empty()
                && !notif.read
            {
                self.notification_center.read_ids.remove(&item_id);
                self.notification_center.dismissed_ids.remove(&item_id);
                self.notification_center.popup_shown_ids.remove(&item_id);
            }
            if notif.read {
                self.notification_center.read_ids.insert(item_id.clone());
            }
            let (mut visual, mut badge) =
                visual_and_badge_for_package(&notif.package_name, &notif.app_name);
            if let Some((ref d_badge, _, _, _)) = delivery_sig {
                if badge == "应用" || badge == "短信" || badge.is_empty() {
                    badge = d_badge.clone();
                }
            }
            let notif_pickup =
                super::pages::sms::extract_pickup_code(&format!("{} {}", notif.title, notif.text))
                    .map(|c| c.trim().to_string())
                    .filter(|c| {
                        !c.is_empty()
                            && !self
                                .push
                                .prefs
                                .dismissed_pickup_codes
                                .contains(&c.to_uppercase())
                    });
            if notif_pickup.is_some() {
                if let Some(brand) = extract_sms_brand_signature(&notif.text) {
                    if badge == "短信" || badge == "应用" {
                        badge = brand;
                    }
                }
            }
            let title = if !notif.title.trim().is_empty() {
                truncate_concise(notif.title.trim(), 24)
            } else {
                badge.clone()
            };
            let summary = if !notif.text.trim().is_empty() {
                truncate_concise(notif.text.trim(), 64)
            } else {
                "收到一条手机应用通知".into()
            };
            let ts_ms = notif.post_time_ms;
            let unread = !notif.read && !self.notification_center.read_ids.contains(&item_id);
            let primary_action = if !notif.thread_id.trim().is_empty() {
                Some((
                    "回复".into(),
                    NotificationPrimaryAction::OpenImThread {
                        thread_id: notif.thread_id.clone(),
                    },
                ))
            } else {
                Some((
                    "查看".into(),
                    NotificationPrimaryAction::OpenPage(Page::Notifications),
                ))
            };
            let reply_tid = if !notif.thread_id.trim().is_empty() {
                notif.thread_id.trim().to_string()
            } else {
                format!("{}::{}", notif.package_name.trim(), title)
            };
            let notif_ai = classify_ai_value_thread(
                &notif.title,
                &notif.text,
                stepx_companion::push::notification_mentions_current_user(notif),
            )
            .or_else(|| {
                stepx_companion::push::is_career_or_recruitment_signal(
                    &notif.package_name,
                    &notif.app_name,
                    &notif.title,
                    &notif.text,
                )
                .then_some((
                    "AI 招聘消息",
                    NotificationPriority::P0Immediate,
                    None,
                ))
            });
            let status_tag = if let Some((ai_tag, _, _)) = notif_ai {
                Some(ai_tag.into())
            } else if let Some((_, _, ref stag, _)) = delivery_sig {
                Some(stag.clone())
            } else {
                Some(if unread {
                    "新推送".into()
                } else {
                    "已读".into()
                })
            };
            let popup_widget = if let Some(pickup_code) = notif_pickup {
                visual = NotificationVisualKind::Express;
                NotificationPopupWidget::ExpressPickup {
                    pickup_code,
                    carrier: badge.clone(),
                    station: truncate_concise(&notif.text, 42),
                }
            } else if let Some((ref d_badge, ref _headline, ref detail, is_arrived)) = delivery_sig
            {
                visual = NotificationVisualKind::Express;
                let status_code = if is_arrived {
                    "已送达".into()
                } else if notif.progress_max > 0 {
                    format!(
                        "进度 {}%",
                        (notif.progress_current * 100 / notif.progress_max.max(1)).clamp(1, 99)
                    )
                } else {
                    "实时配送".into()
                };
                NotificationPopupWidget::ExpressPickup {
                    pickup_code: status_code,
                    carrier: d_badge.clone(),
                    station: detail.clone(),
                }
            } else if matches!(
                visual,
                NotificationVisualKind::WeChat
                    | NotificationVisualKind::Feishu
                    | NotificationVisualKind::DingTalk
                    | NotificationVisualKind::ImChat
            ) && (notif.can_reply
                || !notif.thread_id.trim().is_empty()
                || matches!(
                    ImChannel::from_package(&notif.package_name),
                    ImChannel::WeChat
                        | ImChannel::WeCom
                        | ImChannel::Feishu
                        | ImChannel::DingTalk
                        | ImChannel::QqTim
                ))
            {
                NotificationPopupWidget::InlineImReply {
                    thread_id: reply_tid,
                    package_name: notif.package_name.clone(),
                    can_reply: notif.can_reply,
                    quick_phrases: vec![
                        "收到，谢谢".into(),
                        "好的，马上处理".into(),
                        "稍后回复".into(),
                    ],
                }
            } else {
                NotificationPopupWidget::None
            };
            let app_label = if !notif.app_name.trim().is_empty() {
                short_app_display_name(notif.app_name.trim())
            } else {
                badge.clone()
            };
            let sender_label = if !notif.title.trim().is_empty() {
                notif.title.trim().to_string()
            } else {
                app_label.clone()
            };
            let full_body = if !notif.text.trim().is_empty() {
                notif.text.trim().to_string()
            } else {
                summary.clone()
            };
            let banner_body = format!("{} · 收到来自「{}」的新消息", badge, sender_label);
            let notif_category = if matches!(
                visual,
                NotificationVisualKind::CalendarAlarm | NotificationVisualKind::MissedCall
            ) {
                NotificationCategory::PhoneComm
            } else {
                NotificationCategory::PhoneApps
            };
            if seen_ids.insert(item_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: item_id,
                    category: notif_category,
                    visual,
                    priority: NotificationPriority::P0Immediate,
                    popup_widget,
                    package_name: notif.package_name.clone(),
                    app_name: app_label,
                    sender: sender_label,
                    full_body,
                    banner_body,
                    source_badge: badge,
                    title,
                    summary,
                    time_label: format_concise_time(ts_ms, now_ms),
                    sort_ts_ms: ts_ms,
                    unread,
                    status_tag,
                    primary_action,
                });
            }
        }

        // =====================================================================
        // 2. PhoneComm: OTP codes, Missed calls, Unread SMS, Express, Alarms/Calendar
        // =====================================================================
        let mut seen_otp_codes: HashSet<String> = HashSet::new();
        for banner in &self.push.snapshot.otp_banners {
            let code = banner.code.trim();
            if code.is_empty()
                || self
                    .push
                    .prefs
                    .dismissed_pickup_codes
                    .contains(&code.to_uppercase())
                || (banner.timestamp_ms > 0
                    && now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(banner.timestamp_ms) > 900_000)
                || !seen_otp_codes.insert(code.to_string())
            {
                continue;
            }
            let item_id = format!("otp:{code}");
            let ts_ms = if banner.timestamp_ms > 0 {
                banner.timestamp_ms
            } else {
                now_ms
            };
            let unread = !self.notification_center.read_ids.contains(&item_id);
            let app_label = if !banner.source_app.trim().is_empty() {
                short_app_display_name(&banner.source_app)
            } else if !banner.title.trim().is_empty() {
                short_app_display_name(&banner.title)
            } else {
                "手机验证".into()
            };
            let summary = if !banner.raw_text.trim().is_empty() {
                truncate_concise(banner.raw_text.trim(), 64)
            } else {
                format!("{app_label} · 点击一键复制验证码")
            };
            let full_body = if !banner.raw_text.trim().is_empty() {
                banner.raw_text.trim().to_string()
            } else {
                summary.clone()
            };
            let banner_body = format!("{app_label} · 收到一条新验证码提醒");
            if seen_ids.insert(item_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: item_id,
                    category: NotificationCategory::PhoneComm,
                    visual: NotificationVisualKind::SmsOtp,
                    priority: NotificationPriority::P0Immediate,
                    popup_widget: NotificationPopupWidget::OtpCode {
                        code: code.to_string(),
                        source_app: app_label.clone(),
                    },
                    package_name: "com.android.mms".into(),
                    app_name: app_label.clone(),
                    sender: app_label,
                    full_body,
                    banner_body,
                    source_badge: "验证码".into(),
                    title: truncate_concise(&format!("验证码 {code}"), 24),
                    summary,
                    time_label: format_concise_time(ts_ms, now_ms),
                    sort_ts_ms: ts_ms,
                    unread,
                    status_tag: Some(if unread {
                        "待复制".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "复制".into(),
                        NotificationPrimaryAction::CopyText {
                            label: format!("验证码 {code}"),
                            text: code.to_string(),
                        },
                    )),
                });
            }
        }

        // Extract OTP codes from push notifications if not already in `otp_banners`
        for notif in &self.push.snapshot.notifications {
            if !notif.active
                || notif.dismissed
                || (notif.post_time_ms > 0
                    && now_ms > 1_700_000_000_000
                    && now_ms.saturating_sub(notif.post_time_ms) > 900_000)
            {
                continue;
            }
            let extracted = notif
                .otp_code
                .clone()
                .filter(|c| !c.trim().is_empty())
                .or_else(|| extract_otp_code(&format!("{} {}", notif.title, notif.text)));
            let Some(code) = extracted else {
                continue;
            };
            let code = code.trim();
            if code.is_empty()
                || self
                    .push
                    .prefs
                    .dismissed_pickup_codes
                    .contains(&code.to_uppercase())
                || !seen_otp_codes.insert(code.to_string())
            {
                continue;
            }
            let item_id = format!("otp:{code}");
            if notif.read {
                self.notification_center.read_ids.insert(item_id.clone());
            }
            let ts_ms = if notif.post_time_ms > 0 {
                notif.post_time_ms
            } else {
                now_ms
            };
            let unread = !self.notification_center.read_ids.contains(&item_id);
            let source_app = if !notif.app_name.trim().is_empty() {
                short_app_display_name(&notif.app_name)
            } else if !notif.title.trim().is_empty() {
                short_app_display_name(&notif.title)
            } else {
                "手机验证".into()
            };
            let full_body = if !notif.text.trim().is_empty() {
                notif.text.trim().to_string()
            } else {
                format!("{source_app} · 验证码 {code}")
            };
            let banner_body = format!("{source_app} · 收到一条新验证码提醒");
            let pkg = if !notif.package_name.trim().is_empty() {
                notif.package_name.clone()
            } else {
                "com.android.mms".into()
            };
            if seen_ids.insert(item_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: item_id,
                    category: NotificationCategory::PhoneComm,
                    visual: NotificationVisualKind::SmsOtp,
                    priority: NotificationPriority::P0Immediate,
                    popup_widget: NotificationPopupWidget::OtpCode {
                        code: code.to_string(),
                        source_app: source_app.clone(),
                    },
                    package_name: pkg,
                    app_name: source_app.clone(),
                    sender: source_app,
                    full_body,
                    banner_body,
                    source_badge: "验证码".into(),
                    title: truncate_concise(&format!("验证码 {code}"), 24),
                    summary: truncate_concise(&notif.text, 64),
                    time_label: format_concise_time(ts_ms, now_ms),
                    sort_ts_ms: ts_ms,
                    unread,
                    status_tag: Some(if unread {
                        "待复制".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "复制".into(),
                        NotificationPrimaryAction::CopyText {
                            label: format!("验证码 {code}"),
                            text: code.to_string(),
                        },
                    )),
                });
            }
        }

        // Extract OTP codes from incoming SMS messages + build Unread SMS items
        for msg in &self.sms {
            if msg.outgoing {
                continue;
            }
            let sender = if msg.address.trim().is_empty() {
                "未知号码"
            } else {
                msg.address.trim()
            };
            let extracted_sms_otp = extract_otp_code(&msg.body);
            if !msg.read
                && (msg.date_ms <= 0
                    || now_ms <= 1_700_000_000_000
                    || now_ms.saturating_sub(msg.date_ms) <= 900_000)
            {
                if let Some(ref code) = extracted_sms_otp {
                    let code = code.trim();
                    if !code.is_empty()
                        && !self
                            .push
                            .prefs
                            .dismissed_pickup_codes
                            .contains(&code.to_uppercase())
                        && seen_otp_codes.insert(code.to_string())
                    {
                        let otp_id = format!("otp:{code}");
                        let ts_ms = if msg.date_ms > 0 { msg.date_ms } else { now_ms };
                        let unread = !self.notification_center.read_ids.contains(&otp_id);
                        let src_app = short_app_display_name(sender);
                        if seen_ids.insert(otp_id.clone()) {
                            raw_items.push(UnifiedNotificationItem {
                                id: otp_id,
                                category: NotificationCategory::PhoneComm,
                                visual: NotificationVisualKind::SmsOtp,
                                priority: NotificationPriority::P0Immediate,
                                popup_widget: NotificationPopupWidget::OtpCode {
                                    code: code.to_string(),
                                    source_app: src_app.clone(),
                                },
                                package_name: "com.android.mms".into(),
                                app_name: "短信".into(),
                                sender: sender.to_string(),
                                full_body: msg.body.trim().to_string(),
                                banner_body: format!("短信 · 收到来自「{src_app}」的验证码提醒"),
                                source_badge: "验证码".into(),
                                title: truncate_concise(&format!("验证码 {code}"), 24),
                                summary: truncate_concise(&msg.body, 64),
                                time_label: format_concise_time(ts_ms, now_ms),
                                sort_ts_ms: ts_ms,
                                unread,
                                status_tag: Some(if unread {
                                    "待复制".into()
                                } else {
                                    "已读".into()
                                }),
                                primary_action: Some((
                                    "复制".into(),
                                    NotificationPrimaryAction::CopyText {
                                        label: format!("验证码 {code}"),
                                        text: code.to_string(),
                                    },
                                )),
                            });
                        }
                    }
                }
            }

            let sms_id = format!("sms:{}:{}", sender, msg.date_ms);
            let ts_ms = if msg.date_ms > 0 { msg.date_ms } else { now_ms };
            let sms_delivery = stepx_companion::home_hub::classify_delivery_or_mobility_signal(
                "", "", sender, &msg.body,
            );
            let is_recent_arrived_delivery = sms_delivery
                .as_ref()
                .is_some_and(|(_, _, _, is_arrived)| *is_arrived)
                && now_ms.saturating_sub(ts_ms) <= 21_600_000;
            if msg.read {
                self.notification_center.read_ids.insert(sms_id.clone());
            }
            let unread = !msg.read && !self.notification_center.read_ids.contains(&sms_id);
            let extracted_pickup = super::pages::sms::extract_pickup_code(&msg.body)
                .map(|c| c.trim().to_string())
                .filter(|c| {
                    !c.is_empty()
                        && !self
                            .push
                            .prefs
                            .dismissed_pickup_codes
                            .contains(&c.to_uppercase())
                });
            let (sms_visual, sms_widget) = if let Some(ref pickup_code) = extracted_pickup {
                let carrier = sms_delivery
                    .as_ref()
                    .map(|(b, _, _, _)| b.clone())
                    .or_else(|| extract_sms_brand_signature(&msg.body))
                    .unwrap_or_else(|| "驿站包裹".into());
                let station = sms_delivery
                    .as_ref()
                    .map(|(_, _, d, _)| d.clone())
                    .unwrap_or_else(|| "凭取件码前往驿站/快递柜领取".into());
                (
                    NotificationVisualKind::Express,
                    NotificationPopupWidget::ExpressPickup {
                        pickup_code: pickup_code.clone(),
                        carrier,
                        station,
                    },
                )
            } else if let Some((ref badge, ref _headline, ref detail, is_arrived)) = sms_delivery {
                (
                    NotificationVisualKind::Express,
                    NotificationPopupWidget::ExpressPickup {
                        pickup_code: if is_arrived {
                            "已送达家门口".into()
                        } else {
                            "派送中".into()
                        },
                        carrier: badge.clone(),
                        station: detail.clone(),
                    },
                )
            } else if let Some(code) = extracted_sms_otp
                .as_ref()
                .map(|c| c.trim())
                .filter(|c| !c.is_empty())
            {
                (
                    NotificationVisualKind::SmsOtp,
                    NotificationPopupWidget::OtpCode {
                        code: code.to_string(),
                        source_app: short_app_display_name(sender),
                    },
                )
            } else {
                (
                    NotificationVisualKind::SmsOtp,
                    NotificationPopupWidget::InlineImReply {
                        thread_id: sender.to_string(),
                        package_name: "com.android.mms".into(),
                        can_reply: true,
                        quick_phrases: vec![
                            "收到，谢谢".into(),
                            "好的，马上处理".into(),
                            "稍后回复".into(),
                        ],
                    },
                )
            };
            if (!msg.read || is_recent_arrived_delivery || raw_items.len() < 12)
                && seen_ids.insert(sms_id.clone())
            {
                let (source_badge, display_title, status_tag) =
                    if let Some((ref badge, ref headline, _, is_arrived)) = sms_delivery {
                        let tag = if is_arrived {
                            "AI 买菜送达"
                        } else if extracted_pickup.is_some() && unread {
                            "AI 快递提醒"
                        } else if unread {
                            "配送中"
                        } else {
                            "已读"
                        };
                        (
                            badge.clone(),
                            truncate_concise(headline, 24),
                            Some(tag.into()),
                        )
                    } else if let Some(ref pk) = extracted_pickup {
                        let brand = extract_sms_brand_signature(&msg.body)
                            .unwrap_or_else(|| "快递取件".into());
                        (
                            brand.clone(),
                            truncate_concise(&format!("{brand} · 取件码 {pk}"), 24),
                            Some(if unread {
                                "AI 快递提醒".into()
                            } else {
                                "已读".into()
                            }),
                        )
                    } else {
                        let title = if let Some(brand) = extract_sms_brand_signature(&msg.body) {
                            truncate_concise(&format!("{brand} · {sender}"), 24)
                        } else {
                            truncate_concise(sender, 24)
                        };
                        (
                            "短信".into(),
                            title,
                            Some(if unread {
                                "未读".into()
                            } else {
                                "已读".into()
                            }),
                        )
                    };
                let banner_body = format!("{} · 收到来自「{}」的新消息", source_badge, sender);
                raw_items.push(UnifiedNotificationItem {
                    id: sms_id,
                    category: NotificationCategory::PhoneComm,
                    visual: sms_visual,
                    priority: NotificationPriority::P0Immediate,
                    popup_widget: sms_widget,
                    package_name: "com.android.mms".into(),
                    app_name: "短信".into(),
                    sender: sender.to_string(),
                    full_body: msg.body.trim().to_string(),
                    banner_body,
                    source_badge,
                    title: display_title,
                    summary: truncate_concise(&msg.body, 64),
                    time_label: format_concise_time(ts_ms, now_ms),
                    sort_ts_ms: ts_ms,
                    unread,
                    status_tag,
                    primary_action: Some(("查看".into(), NotificationPrimaryAction::OpenSms)),
                });
            }
        }

        // Missed calls from `self.call_logs` (`call_type == 3`, within 24h)
        let live_call = &self.push.snapshot.live_call;
        if self.selected.is_some() && live_call.ongoing(now_ms) {
            let caller = self.live_caller_name(live_call);
            raw_items.push(UnifiedNotificationItem {
                id: format!("live-call:{}", live_call.id),
                category: NotificationCategory::PhoneComm,
                visual: NotificationVisualKind::IncomingCall,
                priority: NotificationPriority::P0Immediate,
                popup_widget: NotificationPopupWidget::LiveCall { id: live_call.id.clone(), ringing: live_call.state == "ringing" },
                package_name: "com.android.dialer".into(), app_name: "电话".into(),
                sender: caller.clone(), title: caller,
                full_body: format!("{}\n{}", if live_call.number.is_empty() { "手机未提供号码" } else { &live_call.number }, self.live_call_audio_label()),
                summary: live_call.number.clone(), source_badge: if live_call.state == "ringing" { "手机来电" } else { "当前通话" }.into(),
                banner_body: "手机来电".into(), time_label: "现在".into(),
                sort_ts_ms: live_call.started_ms, unread: true,
                status_tag: Some(if live_call.state == "ringing" { "响铃中" } else { "通话进行中" }.into()),
                primary_action: Some(("查看通话".into(), NotificationPrimaryAction::OpenPage(Page::Phone))),
            });
        }
        for call in self.call_logs.iter().filter(|c| {
            c.call_type == 3
                && c.date_ms > 0
                && (now_ms <= 1_700_000_000_000 || now_ms.saturating_sub(c.date_ms) <= 86_400_000)
        }) {
            let caller = if !call.name.trim().is_empty() {
                call.name.trim()
            } else if !call.number.trim().is_empty() {
                call.number.trim()
            } else {
                "未知来电"
            };
            let call_id = format!("missed-call:{}:{}", call.number.trim(), call.date_ms);
            let ts_ms = call.date_ms;
            let followed_up = self.is_missed_call_followed_up(call);
            if followed_up {
                self.notification_center.read_ids.insert(call_id.clone());
            }
            let unread = !followed_up && !self.notification_center.read_ids.contains(&call_id);
            if seen_ids.insert(call_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: call_id,
                    category: NotificationCategory::PhoneComm,
                    visual: NotificationVisualKind::MissedCall,
                    priority: NotificationPriority::P0Immediate,
                    popup_widget: NotificationPopupWidget::CallerBadge {
                        name: caller.to_string(),
                        number: call.number.trim().to_string(),
                    },
                    package_name: "com.android.dialer".into(),
                    app_name: "电话".into(),
                    sender: caller.to_string(),
                    full_body: format!(
                        "{}\n{}\n未接来电，可回拨或查看通话记录",
                        caller,
                        call.number.trim()
                    ),
                    banner_body: format!("未接来电 · 收到来自「{}」的来电提醒", caller),
                    source_badge: "未接来电".into(),
                    title: truncate_concise(caller, 24),
                    summary: truncate_concise(
                        &format!("{} · 未接来电", call.number.trim()),
                        34,
                    ),
                    time_label: format_concise_time(ts_ms, now_ms),
                    sort_ts_ms: ts_ms,
                    unread,
                    status_tag: Some(if unread {
                        "未接".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some(("查看通话".into(), NotificationPrimaryAction::OpenPage(Page::Phone))),
                });
            }
        }

        // Express parcels (`self.active_express_parcels()`)
        let active_parcels = self.active_express_parcels().to_vec();
        for (idx, parcel) in active_parcels.iter().enumerate() {
            let code = parcel.pickup_code.trim();
            let tracking = parcel.tracking_no.trim();
            if (!code.is_empty()
                && self
                    .push
                    .prefs
                    .dismissed_pickup_codes
                    .contains(&code.to_uppercase()))
                || (!tracking.is_empty()
                    && self
                        .push
                        .prefs
                        .dismissed_pickup_codes
                        .contains(&tracking.to_uppercase()))
            {
                continue;
            }
            let parcel_ts = self.express_parcel_timestamp_ms(parcel, now_ms);
            if parcel_ts <= 0
                || (now_ms > 1_700_000_000_000 && now_ms.saturating_sub(parcel_ts) > 86_400_000)
            {
                continue;
            }
            let express_id = format!(
                "express:{}:{}",
                if tracking.is_empty() {
                    idx.to_string()
                } else {
                    tracking.to_string()
                },
                code
            );
            let unread =
                !code.is_empty() && !self.notification_center.read_ids.contains(&express_id);
            let title = if !code.is_empty() {
                truncate_concise(&format!("取件码 {code}"), 24)
            } else {
                truncate_concise(parcel.carrier.trim(), 24)
            };
            let full_body = if !code.is_empty() {
                format!(
                    "{} · {} · {}（取件码 {}）",
                    parcel.carrier.trim(),
                    parcel.station.trim(),
                    parcel.status.trim(),
                    code
                )
            } else {
                format!("{} · {}", parcel.station.trim(), parcel.status.trim())
            };
            let summary = truncate_concise(
                &format!("{} · {}", parcel.station.trim(), parcel.status.trim()),
                64,
            );
            let carrier_label = if !parcel.carrier.trim().is_empty() {
                parcel.carrier.trim().to_string()
            } else {
                "驿站包裹".to_string()
            };
            let primary_action = if !code.is_empty() {
                Some((
                    "复制".into(),
                    NotificationPrimaryAction::CopyText {
                        label: format!("取件码 {code}"),
                        text: code.to_string(),
                    },
                ))
            } else {
                Some((
                    "查看".into(),
                    NotificationPrimaryAction::OpenPage(Page::Home),
                ))
            };
            if seen_ids.insert(express_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: express_id,
                    category: NotificationCategory::PhoneComm,
                    visual: NotificationVisualKind::Express,
                    priority: NotificationPriority::P0Immediate,
                    popup_widget: NotificationPopupWidget::ExpressPickup {
                        pickup_code: code.to_string(),
                        carrier: parcel.carrier.trim().to_string(),
                        station: parcel.station.trim().to_string(),
                    },
                    package_name: "com.cainiao.wireless".into(),
                    app_name: carrier_label.clone(),
                    sender: carrier_label.clone(),
                    full_body,
                    banner_body: format!("快递取件 · 收到来自「{carrier_label}」的包裹提醒"),
                    source_badge: "快递取件".into(),
                    title,
                    summary,
                    time_label: if parcel.updated_at.trim().is_empty() {
                        "今日".into()
                    } else {
                        truncate_concise(parcel.updated_at.trim(), 8)
                    },
                    sort_ts_ms: parcel_ts,
                    unread,
                    status_tag: Some(if unread {
                        "待取件".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action,
                });
            }
        }

        // Upcoming calendar meetings (within -15m..=+65m) & enabled alarms (within 30m or ringing -10m..=0m)
        for ev in &self.back_screen.stepos_suite.calendar_events {
            if ev.all_day || ev.dtstart <= 0 {
                continue;
            }
            if self.back_screen.completed_calendar_events.contains(&ev.id) {
                continue;
            }
            if self.reminder_governance.is_any_key_suppressed(
                &[
                    format!("cal:{}", ev.id),
                    format!("schedule:{}", ev.id),
                    format!("calendar:{}", ev.id),
                ],
                None,
                None,
                Some("pkg:com.android.calendar"),
                now_ms,
            ) || self
                .reminder_governance
                .muted_sources
                .contains_key("com.android.calendar")
            {
                continue;
            }
            let effective_end_ms = if ev.dtend > ev.dtstart {
                ev.dtend
            } else {
                ev.dtstart + 3_600_000
            };
            if effective_end_ms <= now_ms {
                continue;
            }
            let diff_min = (ev.dtstart - now_ms) / 60_000;
            if (-15..=65).contains(&diff_min) {
                let cal_id = format!("calendar:{}", ev.id);
                let unread = !self.notification_center.read_ids.contains(&cal_id);
                let loc = if ev.location.trim().is_empty() {
                    "即将开始"
                } else {
                    ev.location.trim()
                };
                if seen_ids.insert(cal_id.clone()) {
                    raw_items.push(UnifiedNotificationItem {
                        id: cal_id,
                        category: NotificationCategory::PhoneComm,
                        visual: NotificationVisualKind::CalendarAlarm,
                        priority: NotificationPriority::P0Immediate,
                        popup_widget: NotificationPopupWidget::None,
                        package_name: "com.android.calendar".into(),
                        app_name: "日程提醒".into(),
                        sender: ev.title.trim().to_string(),
                        full_body: format!("{} · {} · {loc}", ev.hm_label(), ev.title.trim()),
                        banner_body: "日程提醒 · 收到一条即将开始的日程提醒".into(),
                        source_badge: "日程提醒".into(),
                        title: truncate_concise(ev.title.trim(), 24),
                        summary: truncate_concise(&format!("{} · {loc}", ev.hm_label()), 34),
                        time_label: ev.hm_label(),
                        sort_ts_ms: ev.dtstart,
                        unread,
                        status_tag: Some(if unread {
                            "即将开始".into()
                        } else {
                            "已读".into()
                        }),
                        primary_action: Some((
                            "查看日程".into(),
                            NotificationPrimaryAction::OpenPage(Page::Calendar),
                        )),
                    });
                }
            }
        }

        use chrono::Datelike;
        let now_local = Local::now();
        let now_min_of_day = (now_local.hour() * 60 + now_local.minute()) as i32;
        let weekday_idx = now_local.weekday().num_days_from_monday();
        for alarm in &self.back_screen.stepos_suite.alarms {
            if !alarm.enabled {
                continue;
            }
            if self.reminder_governance.is_any_key_suppressed(
                &[
                    format!("alarm:{}", alarm.id),
                    format!("capsule:alarm-{}", alarm.id),
                ],
                None,
                None,
                Some("pkg:com.android.deskclock"),
                now_ms,
            ) || self
                .reminder_governance
                .muted_sources
                .contains_key("com.android.deskclock")
            {
                continue;
            }
            let matches_today =
                alarm.days_of_week == 0 || (alarm.days_of_week & (1u32 << weekday_idx.min(6))) != 0;
            if !matches_today {
                continue;
            }
            let alarm_min = alarm.hour as i32 * 60 + alarm.minutes as i32;
            let raw_diff = alarm_min - now_min_of_day;
            if raw_diff < -10 && alarm.days_of_week == 0 {
                continue;
            }
            let (diff, ringing_now) = if (-10..=0).contains(&raw_diff) {
                (0, true)
            } else {
                (raw_diff.rem_euclid(1440), false)
            };
            if diff <= 30 {
                let alarm_id = format!("alarm:{}", alarm.id);
                let unread = !self.notification_center.read_ids.contains(&alarm_id);
                let stable_target_ts_ms = (now_ms / 60_000) * 60_000
                    + (if ringing_now {
                        raw_diff as i64
                    } else {
                        diff as i64
                    } * 60_000);
                let state_phrase = if ringing_now {
                    "正在响铃"
                } else {
                    "即将响铃"
                };
                if seen_ids.insert(alarm_id.clone()) {
                    raw_items.push(UnifiedNotificationItem {
                        id: alarm_id,
                        category: NotificationCategory::PhoneComm,
                        visual: NotificationVisualKind::CalendarAlarm,
                        priority: NotificationPriority::P0Immediate,
                        popup_widget: NotificationPopupWidget::None,
                        package_name: "com.android.deskclock".into(),
                        app_name: "闹钟提醒".into(),
                        sender: alarm.display_title().to_string(),
                        full_body: format!(
                            "{} {} · {} · {state_phrase}",
                            alarm.time_label(),
                            alarm.display_title(),
                            alarm.repeat_label()
                        ),
                        banner_body: "闹钟提醒 · 收到一条即将响铃提醒".into(),
                        source_badge: "闹钟提醒".into(),
                        title: truncate_concise(
                            &format!("{} {}", alarm.time_label(), alarm.display_title()),
                            24,
                        ),
                        summary: truncate_concise(
                            &format!("{} · {state_phrase}", alarm.repeat_label()),
                            34,
                        ),
                        time_label: alarm.time_label(),
                        sort_ts_ms: stable_target_ts_ms,
                        unread,
                        status_tag: Some(if unread {
                            state_phrase.into()
                        } else {
                            "已读".into()
                        }),
                        primary_action: Some((
                            "管理闹钟".into(),
                            NotificationPrimaryAction::OpenPage(Page::Alarms),
                        )),
                    });
                }
            }
        }

        // =====================================================================
        // 3. AppUpdates: Store tasks & updatable apps from catalog vs installed
        // =====================================================================
        let mut task_pkgs: Vec<&String> = self.app_store.tasks.keys().collect();
        task_pkgs.sort();
        for pkg in task_pkgs {
            let Some(task) = self.app_store.tasks.get(pkg) else {
                continue;
            };
            let app_short = short_app_display_name(&task.app_name);
            let is_completed = matches!(task.stage, StoreTaskStage::Completed(_));
            let (task_id, title, summary, tag, action) = match &task.stage {
                StoreTaskStage::Queued => (
                    format!("store-task:{pkg}:{}", task.target_version),
                    format!("准备更新 {app_short}"),
                    format!("目标版本 v{} · 排队中", task.target_version),
                    "排队中".to_string(),
                    NotificationPrimaryAction::OpenAppUpdates,
                ),
                StoreTaskStage::Downloading { percent, .. } => (
                    format!("store-task:{pkg}:{}", task.target_version),
                    format!("正在下载 {app_short} {percent}%"),
                    format!("v{} · 高速下载中", task.target_version),
                    format!("{percent}%"),
                    NotificationPrimaryAction::OpenAppUpdates,
                ),
                StoreTaskStage::Verifying | StoreTaskStage::Installing => (
                    format!("store-task:{pkg}:{}", task.target_version),
                    format!("正在安装 {app_short}"),
                    format!("v{} · 正在同步安装至手机", task.target_version),
                    "安装中".to_string(),
                    NotificationPrimaryAction::OpenAppUpdates,
                ),
                StoreTaskStage::Uninstalling => (
                    format!("store-task:{pkg}:uninstall"),
                    format!("正在卸载 {app_short}"),
                    "正在从手机卸载应用".to_string(),
                    "卸载中".to_string(),
                    NotificationPrimaryAction::OpenAppUpdates,
                ),
                StoreTaskStage::Completed(msg) => {
                    self.notification_center
                        .read_ids
                        .insert(format!("app-update:{pkg}"));
                    (
                        format!("store-task-done:{pkg}:{}", task.target_version),
                        format!("{app_short} 已安装完成"),
                        if msg.trim().is_empty() {
                            format!("已更新至 v{}", task.target_version)
                        } else {
                            truncate_concise(msg.trim(), 34)
                        },
                        "已安装".to_string(),
                        NotificationPrimaryAction::OpenAppUpdates,
                    )
                }
                StoreTaskStage::Failed(err) => (
                    format!("store-task-fail:{pkg}:{}", task.target_version),
                    format!("{app_short} 安装失败"),
                    truncate_concise(err.trim(), 34),
                    "失败".to_string(),
                    NotificationPrimaryAction::UpdateSingleApp {
                        package_name: pkg.clone(),
                    },
                ),
            };
            let unread = !self.notification_center.read_ids.contains(&task_id);
            let btn_label = if matches!(task.stage, StoreTaskStage::Failed(_)) {
                "重试"
            } else {
                "查看"
            };
            let task_priority = if is_completed {
                NotificationPriority::P1Digest
            } else {
                NotificationPriority::P2Silent
            };
            let task_widget = if is_completed {
                NotificationPopupWidget::AppUpdateStrip {
                    count: 1,
                    app_names: vec![app_short.clone()],
                    package_name: Some(pkg.clone()),
                }
            } else {
                NotificationPopupWidget::None
            };
            if seen_ids.insert(task_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: task_id,
                    category: NotificationCategory::AppUpdates,
                    visual: NotificationVisualKind::AppUpdate,
                    priority: task_priority,
                    popup_widget: task_widget,
                    package_name: pkg.clone(),
                    app_name: app_short.clone(),
                    sender: app_short.clone(),
                    full_body: format!("{title} · {summary}"),
                    banner_body: format!("应用更新 · 「{app_short}」有新的状态提醒"),
                    source_badge: "应用更新".into(),
                    title: truncate_concise(&title, 24),
                    summary: truncate_concise(&summary, 34),
                    time_label: "刚刚".into(),
                    sort_ts_ms: now_ms - 5_000,
                    unread,
                    status_tag: Some(if unread { tag } else { "已读".into() }),
                    primary_action: Some((btn_label.into(), action)),
                });
            }
        }

        let installed_map: HashMap<&str, &stepx_companion::apps::AppInfo> =
            self.apps.iter().map(|a| (a.package.as_str(), a)).collect();
        let mut updatable_apps: Vec<(String, String, String, String)> = Vec::new();
        let mut currently_updatable_pkgs: HashSet<String> = HashSet::new();

        for store_app in &self.app_store.catalog {
            if let Some(&installed_entry) = installed_map.get(store_app.package.as_str()) {
                if let StoreInstallState::Updatable {
                    installed_version,
                    latest_version,
                    ..
                } = check_store_install_state(std::slice::from_ref(installed_entry), store_app)
                {
                    currently_updatable_pkgs.insert(store_app.package.clone());
                    let short_name = short_app_display_name(&store_app.name);
                    let is_new_or_newer_update = self
                        .notification_center
                        .seen_updatable_apps
                        .get(&store_app.package)
                        .is_none_or(|(_, prev_ver)| prev_ver != &latest_version);
                    if is_new_or_newer_update {
                        self.notification_center
                            .read_ids
                            .remove("app-updates-summary");
                        self.notification_center
                            .dismissed_ids
                            .remove("app-updates-summary");
                    }
                    self.notification_center.seen_updatable_apps.insert(
                        store_app.package.clone(),
                        (short_name.clone(), latest_version.clone()),
                    );
                    updatable_apps.push((
                        store_app.package.clone(),
                        short_name,
                        installed_version,
                        latest_version,
                    ));
                }
            }
        }

        // Automatically mark any previously-updatable app that is now updated (`!is_updatable`) as read/resolved!
        let mut resolved_pkgs: Vec<(String, String, String)> = Vec::new();
        if !self.apps.is_empty() {
            for (pkg, (short_name, target_ver)) in &self.notification_center.seen_updatable_apps {
                if !currently_updatable_pkgs.contains(pkg) {
                    let item_id = format!("app-update:{pkg}");
                    self.notification_center.read_ids.insert(item_id);
                    resolved_pkgs.push((pkg.clone(), short_name.clone(), target_ver.clone()));
                }
            }
        }

        if updatable_apps.is_empty() {
            if !self.apps.is_empty() {
                self.notification_center
                    .read_ids
                    .insert("app-updates-summary".into());
            }
        } else {
            let count = updatable_apps.len();
            let summary_id = "app-updates-summary".to_string();
            let all_individual_read = updatable_apps.iter().all(|(pkg, _, _, _)| {
                self.notification_center
                    .read_ids
                    .contains(&format!("app-update:{pkg}"))
            });
            if all_individual_read {
                self.notification_center.read_ids.insert(summary_id.clone());
            }
            let top_names: Vec<&str> = updatable_apps
                .iter()
                .take(2)
                .map(|(_, name, _, _)| name.as_str())
                .collect();
            let strip_app_names: Vec<String> = updatable_apps
                .iter()
                .take(4)
                .map(|(_, name, _, _)| name.clone())
                .collect();
            let single_pkg = (count == 1)
                .then(|| updatable_apps.first().map(|(pkg, _, _, _)| pkg.clone()))
                .flatten();
            let names_joined = top_names.join("、");
            let summary_text = if count > 2 {
                format!("{names_joined} 等 {count} 款应用可一键升级")
            } else {
                format!("{names_joined} 可一键升级至最新版")
            };
            let unread = !self.notification_center.read_ids.contains(&summary_id);
            if seen_ids.insert(summary_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: summary_id,
                    category: NotificationCategory::AppUpdates,
                    visual: NotificationVisualKind::AppUpdate,
                    priority: NotificationPriority::P1Digest,
                    popup_widget: NotificationPopupWidget::AppUpdateStrip {
                        count,
                        app_names: strip_app_names,
                        package_name: single_pkg,
                    },
                    package_name: "com.stepx.appstore".into(),
                    app_name: "应用更新".into(),
                    sender: "应用商店".into(),
                    full_body: summary_text.clone(),
                    banner_body: format!("应用更新 · 有 {count} 款手机应用可更新"),
                    source_badge: "应用更新".into(),
                    title: format!("{count} 款手机应用可更新"),
                    summary: truncate_concise(&summary_text, 34),
                    time_label: "今日".into(),
                    sort_ts_ms: now_ms - 45_000,
                    unread,
                    status_tag: Some(if unread {
                        "可更新".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "去更新".into(),
                        NotificationPrimaryAction::OpenAppUpdates,
                    )),
                });
            }

            for (idx, (pkg, short_name, installed_ver, latest_ver)) in
                updatable_apps.into_iter().take(6).enumerate()
            {
                let item_id = format!("app-update:{pkg}");
                let unread = !self.notification_center.read_ids.contains(&item_id);
                if seen_ids.insert(item_id.clone()) {
                    raw_items.push(UnifiedNotificationItem {
                        id: item_id,
                        category: NotificationCategory::AppUpdates,
                        visual: NotificationVisualKind::AppUpdate,
                        priority: NotificationPriority::P2Silent,
                        popup_widget: NotificationPopupWidget::AppUpdateStrip {
                            count: 1,
                            app_names: vec![short_name.clone()],
                            package_name: Some(pkg.clone()),
                        },
                        package_name: pkg.clone(),
                        app_name: short_name.clone(),
                        sender: short_name.clone(),
                        full_body: format!(
                            "{short_name} 可从 v{installed_ver} 升级至 v{latest_ver}"
                        ),
                        banner_body: format!("应用更新 · 「{short_name}」有新版本可升级"),
                        source_badge: "应用更新".into(),
                        title: truncate_concise(&format!("{short_name} 可更新"), 24),
                        summary: truncate_concise(&format!("v{installed_ver} → v{latest_ver}"), 34),
                        time_label: "今日".into(),
                        sort_ts_ms: now_ms - 50_000 - (idx as i64 * 1_000),
                        unread,
                        status_tag: Some(if unread {
                            "可更新".into()
                        } else {
                            "已读".into()
                        }),
                        primary_action: Some((
                            "更新".into(),
                            NotificationPrimaryAction::UpdateSingleApp { package_name: pkg },
                        )),
                    });
                }
            }
        }

        for (pkg, short_name, target_ver) in resolved_pkgs {
            let item_id = format!("app-update:{pkg}");
            if !self.notification_center.dismissed_ids.contains(&item_id)
                && seen_ids.insert(item_id.clone())
            {
                raw_items.push(UnifiedNotificationItem {
                    id: item_id,
                    category: NotificationCategory::AppUpdates,
                    visual: NotificationVisualKind::AppUpdate,
                    priority: NotificationPriority::P2Silent,
                    popup_widget: NotificationPopupWidget::None,
                    package_name: pkg,
                    app_name: short_name.clone(),
                    sender: short_name.clone(),
                    full_body: format!("{short_name} 已更新完成，当前版本 v{target_ver}"),
                    banner_body: format!("应用更新 · 「{short_name}」已完成更新"),
                    source_badge: "应用更新".into(),
                    title: truncate_concise(&format!("{short_name} 已更新完成"), 24),
                    summary: truncate_concise(&format!("已升级至 v{target_ver}"), 34),
                    time_label: "刚刚".into(),
                    sort_ts_ms: now_ms - 55_000,
                    unread: false,
                    status_tag: Some("已安装".into()),
                    primary_action: Some((
                        "查看".into(),
                        NotificationPrimaryAction::OpenAppUpdates,
                    )),
                });
            }
        }

        // =====================================================================
        // 4. CompanionAndTasks: phone-work-* tasks, GUI safety, clipboard, software status
        // =====================================================================
        if self.amoo_ui.gui_unconfirmed {
            let safety_id = "gui-safety-approval".to_string();
            let unread = !self.notification_center.read_ids.contains(&safety_id);
            let detail = if self.amoo_ui.gui_message.trim().is_empty() {
                "手机看屏敏感操作等待您的确认"
            } else {
                self.amoo_ui.gui_message.trim()
            };
            if seen_ids.insert(safety_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: safety_id,
                    category: NotificationCategory::CompanionAndTasks,
                    visual: NotificationVisualKind::SafetyApproval,
                    priority: NotificationPriority::P0Immediate,
                    popup_widget: NotificationPopupWidget::SafetyConfirm {
                        prompt: detail.to_string(),
                    },
                    package_name: "com.stepx.amoo".into(),
                    app_name: "安全确认".into(),
                    sender: "Amoo 安全助手".into(),
                    full_body: detail.to_string(),
                    banner_body: "安全确认 · 有一项手机操作等待您的批准".into(),
                    source_badge: "安全确认".into(),
                    title: "手机操作待您批准".into(),
                    summary: truncate_concise(detail, 34),
                    time_label: "刚刚".into(),
                    sort_ts_ms: now_ms + 1_000,
                    unread,
                    status_tag: Some(if unread {
                        "待批准".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "批准".into(),
                        NotificationPrimaryAction::ApproveGuiSafety(true),
                    )),
                });
            }
        } else {
            self.notification_center
                .read_ids
                .insert("gui-safety-approval".into());
        }

        for task in &self.workspace.journal.tasks {
            let is_phone_work = task.id.starts_with("phone-work-");
            if !is_phone_work && task.outputs.is_empty() && task.note.trim().is_empty() {
                continue;
            }
            let item_id = format!("task:{}", task.id);
            let awaiting_back = is_phone_work
                && (task.route_note.contains("待回传")
                    || task.note.contains("待回传")
                    || task.outputs.iter().any(|o| o.kind.contains("待回传")));
            let is_completed_or_ready =
                awaiting_back || !task.outputs.is_empty() || !task.receipts.is_empty();
            let title = if awaiting_back {
                "手机委派任务待回传".to_string()
            } else if is_phone_work && (!task.outputs.is_empty() || !task.receipts.is_empty()) {
                "手机协同任务已完成".to_string()
            } else if is_phone_work {
                truncate_concise(&format!("手机委派 · {}", task.title.trim()), 24)
            } else {
                truncate_concise(&format!("协同任务 · {}", task.title.trim()), 24)
            };
            let full_body = if !task.note.trim().is_empty() {
                format!("{} · {}", task.title.trim(), task.note.trim())
            } else {
                task.title.trim().to_string()
            };
            let summary = truncate_concise(&full_body, 34);
            let ts_ms = if task.updated > 1_000_000_000_000 {
                task.updated
            } else if task.updated > 0 {
                task.updated * 1_000
            } else {
                now_ms - 30_000
            };
            let unread = !self.notification_center.read_ids.contains(&item_id);
            let tag = if awaiting_back {
                "待回传"
            } else if !task.outputs.is_empty() {
                "已完成"
            } else {
                "待处理"
            };
            if seen_ids.insert(item_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: item_id,
                    category: NotificationCategory::CompanionAndTasks,
                    visual: NotificationVisualKind::TaskReady,
                    priority: if is_completed_or_ready {
                        NotificationPriority::P1Digest
                    } else {
                        NotificationPriority::P2Silent
                    },
                    popup_widget: NotificationPopupWidget::None,
                    package_name: "com.stepx.companion".into(),
                    app_name: "跨端协同".into(),
                    sender: "协同任务".into(),
                    full_body,
                    banner_body: "跨端协同 · 收到一条协同任务状态提醒".into(),
                    source_badge: "跨端协同".into(),
                    title,
                    summary,
                    time_label: format_concise_time(ts_ms, now_ms),
                    sort_ts_ms: ts_ms,
                    unread,
                    status_tag: Some(if unread { tag.into() } else { "已读".into() }),
                    primary_action: Some((
                        "继续".into(),
                        NotificationPrimaryAction::ResumeTask {
                            task_id: task.id.clone(),
                        },
                    )),
                });
            }
        }

        let phone_clip = self
            .back_screen
            .stepos_suite
            .phone_clipboard
            .trim()
            .to_string();
        if !phone_clip.is_empty()
            && self
                .notification_center
                .recent_clipboards
                .front()
                .map(|(t, _)| t.as_str())
                != Some(phone_clip.as_str())
        {
            self.notification_center
                .recent_clipboards
                .retain(|(t, _)| t != &phone_clip);
            self.notification_center
                .recent_clipboards
                .push_front((phone_clip, now_ms - 6_000));
            while self.notification_center.recent_clipboards.len() > 10 {
                self.notification_center.recent_clipboards.pop_back();
            }
        }
        let recent_clips: Vec<(String, i64)> = self
            .notification_center
            .recent_clipboards
            .iter()
            .cloned()
            .collect();
        for (idx, (clip_text, clip_ts)) in recent_clips.into_iter().enumerate() {
            let clip_id = format!("phone-clipboard:{:016x}", fnv1a64(clip_text.as_bytes()));
            let unread = !self.notification_center.read_ids.contains(&clip_id);
            if seen_ids.insert(clip_id.clone()) {
                let sort_ts_ms = clip_ts.min(now_ms - 6_000 - (idx as i64 * 1_000));
                raw_items.push(UnifiedNotificationItem {
                    id: clip_id,
                    category: NotificationCategory::CompanionAndTasks,
                    visual: NotificationVisualKind::CompanionInfo,
                    priority: if idx == 0 {
                        NotificationPriority::P0Immediate
                    } else {
                        NotificationPriority::P1Digest
                    },
                    popup_widget: NotificationPopupWidget::ClipboardPreview {
                        text: clip_text.clone(),
                    },
                    package_name: "com.stepx.clipboard".into(),
                    app_name: "跨端剪贴".into(),
                    sender: "手机剪贴板".into(),
                    full_body: clip_text.clone(),
                    banner_body: "跨端剪贴 · 手机剪贴板已同步一条新内容".into(),
                    source_badge: "跨端剪贴".into(),
                    title: "手机剪贴板已同步".into(),
                    summary: truncate_concise(&clip_text, 34),
                    time_label: if idx == 0 {
                        "刚刚".into()
                    } else {
                        format_concise_time(sort_ts_ms, now_ms)
                    },
                    sort_ts_ms,
                    unread,
                    status_tag: Some(if unread {
                        "待复制".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "复制".into(),
                        NotificationPrimaryAction::CopyText {
                            label: "手机剪贴板".into(),
                            text: clip_text,
                        },
                    )),
                });
            }
        }

        if !self.expansion.feedback.trim().is_empty() {
            let fb = self.expansion.feedback.trim();
            let exp_id = format!("expansion-note:{:016x}", fnv1a64(fb.as_bytes()));
            let unread = !self.notification_center.read_ids.contains(&exp_id);
            if seen_ids.insert(exp_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: exp_id,
                    category: NotificationCategory::CompanionAndTasks,
                    visual: NotificationVisualKind::TaskReady,
                    priority: NotificationPriority::P2Silent,
                    popup_widget: NotificationPopupWidget::None,
                    package_name: "com.stepx.expansion".into(),
                    app_name: "存储扩容".into(),
                    sender: "存储扩容".into(),
                    full_body: fb.to_string(),
                    banner_body: "存储扩容 · 收到一条存储同步状态提醒".into(),
                    source_badge: "存储扩容".into(),
                    title: "电脑存储扩容同步".into(),
                    summary: truncate_concise(fb, 34),
                    time_label: "刚刚".into(),
                    sort_ts_ms: now_ms - 85_000,
                    unread,
                    status_tag: Some(if unread {
                        "已更新".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "查看".into(),
                        NotificationPrimaryAction::OpenPage(Page::Expansion),
                    )),
                });
            }
        }

        if let Some((ref link_msg, is_err)) = self.phone_link_notice {
            if !link_msg.trim().is_empty() {
                let link_id = format!("phone-link-note:{:016x}", fnv1a64(link_msg.as_bytes()));
                let unread = !self.notification_center.read_ids.contains(&link_id);
                if seen_ids.insert(link_id.clone()) {
                    raw_items.push(UnifiedNotificationItem {
                        id: link_id,
                        category: if is_err {
                            NotificationCategory::System
                        } else {
                            NotificationCategory::CompanionAndTasks
                        },
                        visual: if is_err {
                            NotificationVisualKind::SystemAlert
                        } else {
                            NotificationVisualKind::TaskReady
                        },
                        priority: NotificationPriority::P2Silent,
                        popup_widget: NotificationPopupWidget::None,
                        package_name: "com.stepx.companion".into(),
                        app_name: "跨端协同".into(),
                        sender: "手机伴侣".into(),
                        full_body: link_msg.trim().to_string(),
                        banner_body: "跨端协同 · 收到一条手机协同通道提醒".into(),
                        source_badge: "跨端协同".into(),
                        title: if is_err {
                            "手机协同通道提醒".into()
                        } else {
                            "手机伴侣协同消息".into()
                        },
                        summary: truncate_concise(link_msg.trim(), 34),
                        time_label: "刚刚".into(),
                        sort_ts_ms: now_ms - 25_000,
                        unread,
                        status_tag: Some(if unread {
                            "新动态".into()
                        } else {
                            "已读".into()
                        }),
                        primary_action: Some((
                            "查看".into(),
                            NotificationPrimaryAction::OpenPage(Page::Home),
                        )),
                    });
                }
            }
        }

        for (idx, (time_str, title, detail)) in self.interaction.notices.iter().take(15).enumerate()
        {
            let notice_id = format!(
                "interaction-notice:{:016x}",
                fnv1a64(format!("{time_str}:{title}:{detail}").as_bytes())
            );
            self.notification_center.read_ids.insert(notice_id.clone());
            let unread = false;
            if seen_ids.insert(notice_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: notice_id,
                    category: NotificationCategory::CompanionAndTasks,
                    visual: NotificationVisualKind::CompanionInfo,
                    priority: NotificationPriority::P2Silent,
                    popup_widget: NotificationPopupWidget::None,
                    package_name: "com.stepx.companion".into(),
                    app_name: "操作提醒".into(),
                    sender: title.clone(),
                    full_body: detail.clone(),
                    banner_body: format!("操作提醒 · {}", truncate_concise(title, 20)),
                    source_badge: "操作提醒".into(),
                    title: truncate_concise(title, 24),
                    summary: truncate_concise(detail, 34),
                    time_label: truncate_concise(time_str, 8),
                    sort_ts_ms: now_ms - 35_000 - (idx as i64 * 1_000),
                    unread,
                    status_tag: Some(if unread {
                        "新记录".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "设置".into(),
                        NotificationPrimaryAction::OpenPage(Page::Settings),
                    )),
                });
            }
        }

        let companion_id = format!("companion-status:v{}", stepx_companion::build_info::VERSION);
        self.notification_center
            .read_ids
            .insert(companion_id.clone());
        let companion_unread = false;
        let companion_summary = if !self.desktop_update.notice.trim().is_empty() {
            truncate_concise(self.desktop_update.notice.trim(), 34)
        } else if self.desktop_update.shown() {
            truncate_concise(&self.desktop_update.label(), 34)
        } else {
            "桌面与手机跨端协同服务运行正常".into()
        };
        if seen_ids.insert(companion_id.clone()) {
            raw_items.push(UnifiedNotificationItem {
                id: companion_id,
                category: NotificationCategory::CompanionAndTasks,
                visual: NotificationVisualKind::CompanionInfo,
                priority: NotificationPriority::P2Silent,
                popup_widget: NotificationPopupWidget::None,
                package_name: "com.stepx.companion".into(),
                app_name: "本机软件".into(),
                sender: "StepX Companion".into(),
                full_body: companion_summary.clone(),
                banner_body: "本机软件 · StepX Companion 已就绪".into(),
                source_badge: "本机软件".into(),
                title: format!(
                    "StepX Companion v{} 已就绪",
                    stepx_companion::build_info::VERSION
                ),
                summary: companion_summary,
                time_label: "今日".into(),
                sort_ts_ms: now_ms - 300_000,
                unread: companion_unread,
                status_tag: Some("已就绪".into()),
                primary_action: Some((
                    "设置".into(),
                    NotificationPrimaryAction::OpenPage(Page::Settings),
                )),
            });
        }

        // =====================================================================
        // 5. System: Device link, low battery (<20%), high storage (>85%), errors
        // =====================================================================
        let metrics = self.cached_device_metrics();
        if let Some(ref dev) = self.selected {
            self.notification_center
                .read_ids
                .insert("sys-device-disconnected".into());

            if let Some(lvl) = metrics.battery_level {
                let bat_id = format!("sys-battery-low:{}", dev.serial);
                if lvl < 20 {
                    let unread = !self.notification_center.read_ids.contains(&bat_id);
                    if seen_ids.insert(bat_id.clone()) {
                        raw_items.push(UnifiedNotificationItem {
                            id: bat_id,
                            category: NotificationCategory::System,
                            visual: NotificationVisualKind::BatteryWarn,
                            priority: NotificationPriority::P1Digest,
                            popup_widget: NotificationPopupWidget::None,
                            package_name: "com.android.systemui".into(),
                            app_name: "电量告警".into(),
                            sender: "手机系统".into(),
                            full_body: format!(
                                "手机当前电量 {lvl}%（低于 20%），建议及时连接电源充电"
                            ),
                            banner_body: "电量告警 · 手机电量偏低提醒".into(),
                            source_badge: "电量告警".into(),
                            title: format!("手机电量低 ({lvl}%)"),
                            summary: "电量低于 20%，建议及时连接电源充电".into(),
                            time_label: "刚刚".into(),
                            sort_ts_ms: now_ms - 10_000,
                            unread,
                            status_tag: Some(if unread {
                                "低电量".into()
                            } else {
                                "已读".into()
                            }),
                            primary_action: Some((
                                "查看终端".into(),
                                NotificationPrimaryAction::OpenPage(Page::Overview),
                            )),
                        });
                    }
                } else {
                    self.notification_center.read_ids.insert(bat_id);
                }
            }

            let storage_pct = metrics.storage_percent.or_else(|| {
                self.storage_report.as_ref().and_then(|r| {
                    (r.total_bytes > 0).then(|| {
                        ((r.total_bytes.saturating_sub(r.available_bytes)) * 100 / r.total_bytes)
                            as u8
                    })
                })
            });
            if let Some(pct) = storage_pct {
                let stor_id = format!("sys-storage-warn:{}", dev.serial);
                if pct > 85 {
                    let unread = !self.notification_center.read_ids.contains(&stor_id);
                    if seen_ids.insert(stor_id.clone()) {
                        raw_items.push(UnifiedNotificationItem {
                            id: stor_id,
                            category: NotificationCategory::System,
                            visual: NotificationVisualKind::SystemAlert,
                            priority: NotificationPriority::P1Digest,
                            popup_widget: NotificationPopupWidget::None,
                            package_name: "com.android.systemui".into(),
                            app_name: "存储空间".into(),
                            sender: "手机系统".into(),
                            full_body: format!(
                                "手机存储空间已使用 {pct}%，建议开启电脑存储扩容释放手机空间"
                            ),
                            banner_body: "存储空间 · 手机存储空间紧张提醒".into(),
                            source_badge: "存储空间".into(),
                            title: format!("手机存储空间紧张 ({pct}%)"),
                            summary: "建议开启电脑存储扩容释放手机空间".into(),
                            time_label: "刚刚".into(),
                            sort_ts_ms: now_ms - 15_000,
                            unread,
                            status_tag: Some(if unread {
                                "待扩容".into()
                            } else {
                                "已读".into()
                            }),
                            primary_action: Some((
                                "去扩容".into(),
                                NotificationPrimaryAction::OpenPage(Page::Expansion),
                            )),
                        });
                    }
                } else {
                    self.notification_center.read_ids.insert(stor_id);
                }
            }

            if self
                .updates
                .report
                .as_ref()
                .is_some_and(|r| r.channels.iter().any(|c| c.status == OtaStatus::Available))
            {
                let ota_id = format!("sys-ota:{}", dev.serial);
                let unread = !self.notification_center.read_ids.contains(&ota_id);
                if seen_ids.insert(ota_id.clone()) {
                    raw_items.push(UnifiedNotificationItem {
                        id: ota_id,
                        category: NotificationCategory::System,
                        visual: NotificationVisualKind::SystemAlert,
                        priority: NotificationPriority::P1Digest,
                        popup_widget: NotificationPopupWidget::None,
                        package_name: "com.android.systemui".into(),
                        app_name: "系统更新".into(),
                        sender: "手机系统".into(),
                        full_body: "检测到可用固件升级包，点击查看详情".into(),
                        banner_body: "系统更新 · 发现手机系统 OTA 更新提醒".into(),
                        source_badge: "系统更新".into(),
                        title: "发现手机系统 OTA 更新".into(),
                        summary: "检测到可用固件升级包，点击查看详情".into(),
                        time_label: "今日".into(),
                        sort_ts_ms: now_ms - 40_000,
                        unread,
                        status_tag: Some(if unread {
                            "可升级".into()
                        } else {
                            "已读".into()
                        }),
                        primary_action: Some((
                            "查看更新".into(),
                            NotificationPrimaryAction::OpenPage(Page::Tools),
                        )),
                    });
                }
            }

            let link_id = format!("sys-device-link:{}", dev.serial);
            self.notification_center.read_ids.insert(link_id.clone());
            let link_unread = false;
            let model_name = if dev.model.trim().is_empty() {
                "StepX PZ261A"
            } else {
                dev.model.trim()
            };
            let transport_label = if is_wireless(&dev.transport) {
                "无线 ADB 在线"
            } else {
                "USB 高速连接"
            };
            let bat_suffix = metrics
                .battery_level
                .map(|b| format!(" · 电量 {b}%"))
                .unwrap_or_default();
            if seen_ids.insert(link_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: link_id,
                    category: NotificationCategory::System,
                    visual: NotificationVisualKind::DeviceLink,
                    priority: NotificationPriority::P2Silent,
                    popup_widget: NotificationPopupWidget::None,
                    package_name: "com.android.systemui".into(),
                    app_name: "手机系统".into(),
                    sender: model_name.to_string(),
                    full_body: format!("{transport_label}{bat_suffix}"),
                    banner_body: format!("手机系统 · 已连接 {model_name}"),
                    source_badge: "手机系统".into(),
                    title: truncate_concise(&format!("已连接 {model_name}"), 24),
                    summary: truncate_concise(&format!("{transport_label}{bat_suffix}"), 34),
                    time_label: "在线".into(),
                    sort_ts_ms: now_ms - 250_000,
                    unread: link_unread,
                    status_tag: Some("在线".into()),
                    primary_action: Some((
                        "设备详情".into(),
                        NotificationPrimaryAction::OpenPage(Page::Overview),
                    )),
                });
            }
        } else {
            let disc_id = "sys-device-disconnected".to_string();
            self.notification_center.read_ids.insert(disc_id.clone());
            let unread = false;
            if seen_ids.insert(disc_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: disc_id,
                    category: NotificationCategory::System,
                    visual: NotificationVisualKind::DeviceLink,
                    priority: NotificationPriority::P2Silent,
                    popup_widget: NotificationPopupWidget::None,
                    package_name: "com.android.systemui".into(),
                    app_name: "手机系统".into(),
                    sender: "手机系统".into(),
                    full_body: "支持 USB 或同 Wi-Fi 扫码无线连接".into(),
                    banner_body: "手机系统 · 尚未连接 StepX 手机".into(),
                    source_badge: "手机系统".into(),
                    title: "尚未连接 StepX 手机".into(),
                    summary: "支持 USB 或同 Wi-Fi 扫码无线连接".into(),
                    time_label: "刚刚".into(),
                    sort_ts_ms: now_ms - 200_000,
                    unread,
                    status_tag: Some("未连接".into()),
                    primary_action: Some((
                        "去连接".into(),
                        NotificationPrimaryAction::OpenPage(Page::Connect),
                    )),
                });
            }
        }

        if let Some(ref push_err) = self.push.error {
            if !push_err.trim().is_empty() {
                let err_id = format!("sys-push-err:{:016x}", fnv1a64(push_err.as_bytes()));
                let unread = !self.notification_center.read_ids.contains(&err_id);
                if seen_ids.insert(err_id.clone()) {
                    raw_items.push(UnifiedNotificationItem {
                        id: err_id,
                        category: NotificationCategory::System,
                        visual: NotificationVisualKind::SystemAlert,
                        priority: NotificationPriority::P2Silent,
                        popup_widget: NotificationPopupWidget::None,
                        package_name: "com.android.systemui".into(),
                        app_name: "系统状态".into(),
                        sender: "系统状态".into(),
                        full_body: push_err.trim().to_string(),
                        banner_body: "系统状态 · 手机推送同步异常提醒".into(),
                        source_badge: "系统状态".into(),
                        title: "手机推送同步异常".into(),
                        summary: truncate_concise(push_err.trim(), 34),
                        time_label: "刚刚".into(),
                        sort_ts_ms: now_ms - 20_000,
                        unread,
                        status_tag: Some(if unread {
                            "待处理".into()
                        } else {
                            "已读".into()
                        }),
                        primary_action: Some((
                            "查看".into(),
                            NotificationPrimaryAction::OpenPage(Page::Notifications),
                        )),
                    });
                }
            }
        } else if self.error && !self.status.trim().is_empty() {
            let err_id = format!(
                "sys-status-err:{:016x}",
                fnv1a64(self.status.trim().as_bytes())
            );
            let unread = !self.notification_center.read_ids.contains(&err_id);
            if seen_ids.insert(err_id.clone()) {
                raw_items.push(UnifiedNotificationItem {
                    id: err_id,
                    category: NotificationCategory::System,
                    visual: NotificationVisualKind::SystemAlert,
                    priority: NotificationPriority::P2Silent,
                    popup_widget: NotificationPopupWidget::None,
                    package_name: "com.android.systemui".into(),
                    app_name: "系统状态".into(),
                    sender: "系统状态".into(),
                    full_body: self.status.trim().to_string(),
                    banner_body: "系统状态 · 收到一条操作状态提醒".into(),
                    source_badge: "系统状态".into(),
                    title: "操作未能完成".into(),
                    summary: truncate_concise(self.status.trim(), 34),
                    time_label: "刚刚".into(),
                    sort_ts_ms: now_ms - 18_000,
                    unread,
                    status_tag: Some(if unread {
                        "异常".into()
                    } else {
                        "已读".into()
                    }),
                    primary_action: Some((
                        "连接设置".into(),
                        NotificationPrimaryAction::OpenPage(Page::Connect),
                    )),
                });
            }
        }

        // Merge persisted/cached notification history so historical items survive device
        // disconnection or app restarts until explicitly dismissed or cleared.
        for mut hist_item in self.notification_center.cached_history.clone() {
            if hist_item.id == "sys-device-disconnected"
                || hist_item.id.starts_with("sys-device-link:")
                || hist_item.id.starts_with("companion-status:")
                || hist_item.id == "gui-safety-approval"
                || self
                    .notification_center
                    .dismissed_ids
                    .contains(&hist_item.id)
            {
                continue;
            }
            if seen_ids.insert(hist_item.id.clone()) {
                hist_item.unread = !self.notification_center.read_ids.contains(&hist_item.id);
                if hist_item.full_body.trim().is_empty() {
                    hist_item.full_body = hist_item.summary.clone();
                }
                if hist_item.banner_body.trim().is_empty() {
                    let sender_or_title = if !hist_item.sender.trim().is_empty() {
                        hist_item.sender.trim()
                    } else {
                        hist_item.title.trim()
                    };
                    hist_item.banner_body = format!(
                        "{} · 收到来自「{}」的新消息",
                        hist_item.source_badge, sender_or_title
                    );
                }
                if !hist_item.unread
                    && hist_item
                        .status_tag
                        .as_deref()
                        .is_some_and(|t| t != "已读" && !t.starts_with("AI ") && t != "已安装")
                {
                    hist_item.status_tag = Some("已读".into());
                }
                raw_items.push(hist_item);
            }
        }

        // Filter out dismissed items and sort unread first, priority first, then newest timestamp
        let prev_queue_set: HashSet<String> = self
            .notification_center
            .popup_queue
            .iter()
            .cloned()
            .collect();
        for item in &mut raw_items {
            if prev_queue_set.contains(&item.id)
                && !self.notification_center.read_ids.contains(&item.id)
                && !self.notification_center.dismissed_ids.contains(&item.id)
            {
                item.unread = true;
            }
        }
        raw_items.retain(|item| !self.notification_center.dismissed_ids.contains(&item.id));
        raw_items.sort_by(|a, b| {
            b.unread
                .cmp(&a.unread)
                .then_with(|| a.priority.cmp(&b.priority))
                .then_with(|| b.sort_ts_ms.cmp(&a.sort_ts_ms))
                .then_with(|| a.id.cmp(&b.id))
        });

        self.notification_center.cached_history = raw_items
            .iter()
            .filter(|i| {
                i.id != "sys-device-disconnected"
                    && !i.id.starts_with("live-call:")
                    && !i.id.starts_with("sys-device-link:")
                    && !i.id.starts_with("companion-status:")
                    && i.id != "gui-safety-approval"
            })
            .take(MAX_CACHED_HISTORY_LEN)
            .cloned()
            .collect();

        let previous_events: HashMap<_, _> = self.notification_center.items.iter()
            .filter(|item| prev_queue_set.contains(&item.id))
            .map(|item| (item.id.clone(), self.popup_delivery_key(item))).collect();
        self.notification_center.items = raw_items;

        if self
            .notification_center
            .popup_feedback_text
            .as_ref()
            .is_some_and(|(_, exp)| *exp <= now_ms)
        {
            self.notification_center.popup_feedback_text = None;
        }
        if self
            .notification_center
            .snooze_until_ms
            .is_some_and(|until| until <= now_ms)
        {
            self.notification_center.snooze_until_ms = None;
        }

        let globally_silent = self.notification_center.alert_mode == NotificationAlertMode::Muted
            || (self.push.prefs.popup_mode == stepx_companion::desktop_notify::PopupMode::Silent
                && self
                    .push
                    .prefs
                    .app_popup_modes
                    .values()
                    .all(|m| *m == stepx_companion::desktop_notify::PopupMode::Silent));

        if globally_silent {
            self.notification_center.popup_queue.clear();
            self.sync_popup_reply_draft(None);
            self.notification_center.active_popup_id = None;
            self.sync_smart_backscreen_display();
            return;
        }

        // Connection baselines suppress history. Once connected, every new source
        // can join the queue; a burst must not silently discard its later items.
        self.notification_center.initial_sync_seeded = true;

        let prev_active_id = self
            .notification_center
            .popup_queue
            .get(self.notification_center.popup_cursor)
            .cloned()
            .or_else(|| self.notification_center.active_popup_id.clone());
        let prev_queue_set: HashSet<String> = self
            .notification_center
            .popup_queue
            .iter()
            .cloned()
            .collect();

        let mut eligible_items: Vec<&UnifiedNotificationItem> = self
            .notification_center
            .items
            .iter()
            .filter(|item| {
                if !Self::is_item_popup_eligible(item, &self.push.prefs, &self.reminder_governance)
                {
                    return false;
                }
                if self.notification_center.popup_shown_ids.contains(&item.id) {
                    return false;
                }
                if !prev_queue_set.contains(&item.id) && self.notification_center.delivered_events.contains(&self.popup_delivery_key(item)) {
                    return false;
                }
                let is_current = self.demo_mode
                    || matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. })
                    || prev_queue_set.contains(&item.id)
                    || self
                        .notification_center
                        .is_current_popup_event(item, now_ms);
                is_current
            })
            .collect();
        // The inbox's digest priority is not arrival order. Calls keep their
        // controls first; all other cards (including OTPs) are newest first.
        eligible_items.sort_by(|a, b| {
            b.id.starts_with("live-call:").cmp(&a.id.starts_with("live-call:"))
                .then_with(|| b.sort_ts_ms.cmp(&a.sort_ts_ms))
                .then_with(|| a.priority.cmp(&b.priority))
                .then_with(|| a.id.cmp(&b.id))
        });
        let newest_arrival = eligible_items.iter().find(|item| {
            !prev_queue_set.contains(&item.id)
                || previous_events.get(&item.id).is_some_and(|key| key != &self.popup_delivery_key(item))
        }).map(|item| item.id.clone());
        let live_call = eligible_items.iter().find(|item| item.id.starts_with("live-call:")).map(|item| item.id.clone());
        self.notification_center.popup_queue = eligible_items.into_iter().map(|item| item.id.clone()).collect();
        if live_call.is_some() || newest_arrival.is_some() {
            // A delayed batch from another source cannot displace a newer card.
            self.notification_center.popup_pending_front = self.notification_center.popup_queue.front().cloned();
        }
        self.notification_center.popup_cursor = prev_active_id
            .as_ref()
            .and_then(|id| {
                self.notification_center
                    .popup_queue
                    .iter()
                    .position(|queued| queued == id)
            })
            .unwrap_or(0);
        self.select_pending_popup();
        self.sync_active_popup_from_queue(true);
        self.sync_smart_backscreen_display();
    }

    fn mark_underlying_source_read(&mut self, id: &str) {
        self.notification_center.read_ids.insert(id.to_string());
        self.notification_center
            .popup_shown_ids
            .insert(id.to_string());
        self.back_screen
            .smart_display_cleared_ids
            .insert(id.to_string());
        if id == "app-updates-summary" {
            let pkgs: Vec<String> = self
                .notification_center
                .seen_updatable_apps
                .keys()
                .cloned()
                .collect();
            for pkg in pkgs {
                self.notification_center
                    .read_ids
                    .insert(format!("app-update:{pkg}"));
            }
            let item_ids: Vec<String> = self
                .notification_center
                .items
                .iter()
                .filter(|i| i.id.starts_with("app-update:"))
                .map(|i| i.id.clone())
                .collect();
            for item_id in item_ids {
                self.notification_center.read_ids.insert(item_id);
            }
        } else if id.starts_with("app-update:") {
            let all_updates_read = self
                .notification_center
                .items
                .iter()
                .filter(|i| i.id.starts_with("app-update:"))
                .all(|i| self.notification_center.read_ids.contains(&i.id))
                && self
                    .notification_center
                    .seen_updatable_apps
                    .keys()
                    .all(|pkg| {
                        self.notification_center
                            .read_ids
                            .contains(&format!("app-update:{pkg}"))
                    });
            if all_updates_read {
                self.notification_center
                    .read_ids
                    .insert("app-updates-summary".to_string());
            }
        } else if let Some(tid) = id.strip_prefix("im-thread:") {
            for thread in &mut self.push.snapshot.threads {
                if thread.thread_id == tid || thread.title.trim() == tid {
                    thread.unread_count = 0;
                }
            }
            for notif in &mut self.push.snapshot.notifications {
                if notif.thread_id == tid {
                    notif.read = true;
                }
            }
        } else if let Some(key) = id
            .strip_prefix("push-notif:")
            .or_else(|| id.strip_prefix("push:"))
        {
            for notif in &mut self.push.snapshot.notifications {
                if notif.key == key || format!("seq-{}", notif.seq_id) == key {
                    notif.read = true;
                }
            }
        } else if let Some(rest) = id.strip_prefix("sms:") {
            for msg in &mut self.sms {
                let candidate = format!("{}:{}", msg.address.trim(), msg.date_ms);
                if candidate == rest || msg.address.trim() == rest {
                    msg.read = true;
                }
            }
        } else if let Some(code) = id.strip_prefix("otp:") {
            for notif in &mut self.push.snapshot.notifications {
                let extracted = extract_otp_code(&format!("{} {}", notif.title, notif.text));
                if notif.otp_code.as_deref() == Some(code) || extracted.as_deref() == Some(code) {
                    notif.read = true;
                }
            }
            for msg in &mut self.sms {
                if extract_otp_code(&msg.body).as_deref() == Some(code) {
                    msg.read = true;
                }
            }
            self.push
                .active_toasts
                .retain(|t| t.otp_code.as_deref() != Some(code));
        } else if id.starts_with("missed-call:")
            || id.starts_with("calendar:")
            || id.starts_with("alarm:")
        {
            self.notification_center.read_ids.insert(id.to_string());
            self.notification_center
                .popup_shown_ids
                .insert(id.to_string());
        }
    }

    pub(crate) fn submit_popup_quick_reply(&mut self, item_id: &str, text: &str) {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            return;
        }
        let now_ms = Local::now().timestamp_millis();
        if !self.demo_mode && self.selected.is_none() {
            self.status = "手机已断开，回复未发送；请连接原手机后重试".into();
            self.error = true;
            return;
        }
        let Some(matched_item) = self
            .notification_center
            .items
            .iter()
            .find(|item| item.id == item_id)
            .cloned()
        else {
            self.status = "这条通知已失效，回复未发送；请重新打开会话".into();
            self.error = true;
            return;
        };

        let is_sms_or_call = matched_item.package_name == "com.android.mms"
            || item_id.starts_with("sms:")
            || item_id.starts_with("missed-call:");

        if !self.demo_mode
            && !is_sms_or_call
            && !matches!(
                matched_item.popup_widget,
                NotificationPopupWidget::InlineImReply {
                    can_reply: true,
                    ..
                }
            )
        {
            self.status = "当前通知未提供后台回复入口，消息未发送；请打开会话查看".into();
            self.error = true;
            self.notification_center.popup_feedback_text =
                Some((self.status.clone(), now_ms + 5_000));
            return;
        }

        if is_sms_or_call {
            let target_addr = item_id
                .strip_prefix("sms:")
                .or_else(|| item_id.strip_prefix("missed-call:"))
                .and_then(|rest| rest.rsplit_once(':').map(|(addr, _)| addr).or(Some(rest)))
                .map(str::trim)
                .filter(|address| !address.is_empty())
                .map(ToString::to_string)
                .or_else(|| {
                    (!matched_item.sender.trim().is_empty())
                        .then(|| matched_item.sender.trim().to_string())
                })
                .or_else(|| match &matched_item.popup_widget {
                    NotificationPopupWidget::CallerBadge { number, name } => {
                        if !number.trim().is_empty() {
                            Some(number.trim().to_string())
                        } else if !name.trim().is_empty() {
                            Some(name.trim().to_string())
                        } else {
                            None
                        }
                    }
                    NotificationPopupWidget::InlineImReply { thread_id, .. }
                        if !thread_id.trim().is_empty() =>
                    {
                        Some(thread_id.trim().to_string())
                    }
                    _ => None,
                });
            let Some(target_addr) = target_addr else {
                self.status = "无法确认短信收件人，回复未发送".into();
                self.error = true;
                return;
            };
            let optimistic_msg = stepx_companion::sms::Message {
                provider_id: Some(now_ms),
                address: target_addr.clone(),
                body: trimmed.to_string(),
                date_ms: now_ms,
                outgoing: true,
                kind: 2,
                read: true,
                subscription: self.selected_sim,
            };
            if self.demo_mode {
                self.sms.insert(0, optimistic_msg.clone());
            }
            if !self.demo_mode {
                if let Some(mut device) = self.selected.clone() {
                    let sim = self.selected_sim;
                    let to = target_addr.clone();
                    let body = trimmed.to_string();
                    let (tx, rx) = std::sync::mpsc::channel();
                    self.bg_receivers.push((Page::Sms, None, rx));
                    std::thread::spawn(move || {
                        let outcome = stepx_companion::adb::Adb::discover().and_then(|adb| {
                            device.transport = device.resolve_active_transport(&adb)?;
                            let plan = stepx_companion::bridge::sms_prepare(
                                &adb, &device, &to, &body, sim,
                            )?;
                            let status = stepx_companion::bridge::sms_send(&adb, &device, &plan)?;
                            let mut messages = super::read_result(device.messages(&adb));
                            super::device_actions::merge_confirmed_sms(
                                &mut messages,
                                &status,
                                optimistic_msg,
                            );
                            Ok(super::Outcome::SmsSent(status, messages))
                        });
                        let _ = tx.send(outcome);
                    });
                }
            }
            self.status = if self.demo_mode {
                format!("演示：已向 {target_addr} 模拟发送短信回复")
            } else {
                format!("正在通过手机向 {target_addr} 发送短信，等待确认…")
            };
            self.error = false;
        } else if item_id.starts_with("im-thread:")
            || item_id.starts_with("push-notif:")
            || !matched_item.package_name.trim().is_empty()
        {
            let resolved = if let Some(tid) = item_id.strip_prefix("im-thread:") {
                self.push
                    .snapshot
                    .threads
                    .iter()
                    .find(|t| t.thread_id == tid)
                    .map(|t| {
                        (
                            t.thread_id.clone(),
                            t.package_name.clone(),
                            t.title.clone(),
                            (!t.notification_key.is_empty()).then(|| t.notification_key.clone()),
                        )
                    })
                    .or_else(|| match &matched_item.popup_widget {
                        NotificationPopupWidget::InlineImReply {
                            thread_id,
                            package_name,
                            ..
                        } => Some((
                            thread_id.clone(),
                            package_name.clone(),
                            matched_item.title.clone(),
                            None,
                        )),
                        _ => (!matched_item.package_name.trim().is_empty()).then(|| {
                            (
                                tid.to_string(),
                                matched_item.package_name.clone(),
                                matched_item.title.clone(),
                                None,
                            )
                        }),
                    })
            } else if let Some(key) = item_id.strip_prefix("push-notif:") {
                self.push
                    .snapshot
                    .notifications
                    .iter()
                    .find(|n| n.key == key || format!("seq-{}", n.seq_id) == key)
                    .map(|n| {
                        let tid = if !n.thread_id.trim().is_empty() {
                            n.thread_id.trim().to_string()
                        } else {
                            format!("{}::{}", n.package_name.trim(), n.title.trim())
                        };
                        (
                            tid,
                            n.package_name.clone(),
                            n.title.clone(),
                            Some(n.key.clone()),
                        )
                    })
                    .or_else(|| {
                        (!matched_item.package_name.trim().is_empty()).then(|| {
                            let tid = match &matched_item.popup_widget {
                                NotificationPopupWidget::InlineImReply { thread_id, .. }
                                    if !thread_id.trim().is_empty() =>
                                {
                                    thread_id.trim().to_string()
                                }
                                _ => format!(
                                    "{}::{}",
                                    matched_item.package_name.trim(),
                                    matched_item.title.trim()
                                ),
                            };
                            (
                                tid,
                                matched_item.package_name.clone(),
                                matched_item.title.clone(),
                                Some(key.to_string()),
                            )
                        })
                    })
            } else {
                let pkg = matched_item.package_name.trim().to_string();
                let title = if !matched_item.sender.trim().is_empty() {
                    matched_item.sender.trim().to_string()
                } else {
                    matched_item.title.clone()
                };
                let tid = match &matched_item.popup_widget {
                    NotificationPopupWidget::InlineImReply { thread_id, .. }
                        if !thread_id.trim().is_empty() =>
                    {
                        thread_id.trim().to_string()
                    }
                    _ => format!("{pkg}::{title}"),
                };
                Some((tid, pkg, title, None))
            };

            let Some((thread_id, package_name, title, notif_key)) = resolved else {
                self.status = "无法确认回复会话，消息未发送；请重新打开会话".into();
                self.error = true;
                return;
            };
            if !self.demo_mode && self.selected.is_some() {
                self.dispatch_im_reply(
                    &thread_id,
                    &package_name,
                    &title,
                    trimmed,
                    notif_key.as_deref(),
                );
            } else {
                let client_msg_id = format!("stepx-{now_ms}-{}", self.push.snapshot.revision);
                let (effective_tid, effective_title) =
                    super::push_state::record_local_outgoing_attempt(
                        &mut self.push.snapshot,
                        &thread_id,
                        &package_name,
                        &title,
                        trimmed,
                        notif_key.as_deref(),
                        &client_msg_id,
                        now_ms,
                        true,
                    );
                self.push.active_toasts.retain(|t| {
                    t.thread_id != effective_tid
                        && t.thread_id != thread_id
                        && notif_key.as_deref().is_none_or(|k| t.notification_key != k)
                });
                let status_msg =
                    format!("已通过同 Wi-Fi RemoteInput 秒回「{effective_title}」：{trimmed}");
                self.push.last_reply_status = Some(status_msg.clone());
                self.status = status_msg;
                self.error = false;
            }
        } else {
            self.status = "这条通知不支持快捷回复，未发送消息".into();
            self.error = true;
            return;
        }

        self.mark_underlying_source_read(item_id);
        self.notification_center
            .popup_shown_ids
            .insert(item_id.to_string());
        self.notification_center
            .popup_queue
            .retain(|id| id != item_id);
        self.notification_center.popup_feedback_text = Some((
            if self.demo_mode {
                "演示：已模拟回复"
            } else {
                "正在发送，等待手机确认…"
            }
            .into(),
            now_ms + 1_200,
        ));
        self.notification_center.save();
        self.sync_notification_center();
    }

    pub(crate) fn apply_notification_center_action(&mut self, action: NotificationCenterAction) {
        if !matches!(
            action,
            NotificationCenterAction::PopupMenu(_) | NotificationCenterAction::PopupHoverChanged(_)
        ) {
            self.notification_center.popup_menu_item_id = None;
        }
        match action {
            NotificationCenterAction::PopupMenu(id) => {
                if id.is_some() && self.notification_center.popup_menu_item_id != id {
                    self.notification_center.popup_menu_scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
                }
                self.notification_center.popup_menu_item_id = id.filter(|id| {
                    self.notification_active_popup()
                        .is_some_and(|item| &item.id == id)
                });
            }
            NotificationCenterAction::SnoozeToday => {
                let now = Local::now();
                // Calendar midnight, not a fixed 24-hour interval (DST safe).
                let until = now
                    .date_naive()
                    .succ_opt()
                    .and_then(|day| day.and_hms_opt(0, 0, 0))
                    .and_then(|time| time.and_local_timezone(Local).earliest())
                    .map(|time| time.timestamp_millis())
                    .unwrap_or_else(|| now.timestamp_millis() + 24 * 60 * 60 * 1000);
                self.apply_notification_center_action(NotificationCenterAction::SnoozePopups {
                    duration_ms: until - now.timestamp_millis(),
                });
            }
            NotificationCenterAction::MutePopupApp(id) => {
                let Some(item) = self
                    .notification_center
                    .items
                    .iter()
                    .find(|item| item.id == id)
                    .cloned()
                else {
                    return;
                };
                let package = stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(
                    &item.package_name,
                );
                if package.is_empty() {
                    return;
                }
                let mut next = self.push.prefs.clone();
                next.set_app_popup_mode(
                    &package,
                    Some(stepx_companion::desktop_notify::PopupMode::Silent),
                );
                if !self.demo_mode {
                    if let Err(error) = next.save() {
                        self.status = format!("提醒设置未保存：{error}");
                        self.error = true;
                        return;
                    }
                }
                self.push.prefs = next;
                // Keep unrelated settings edits; update only this application's rule.
                self.settings.notify_draft.set_app_popup_mode(
                    &package,
                    Some(stepx_companion::desktop_notify::PopupMode::Silent),
                );
                let ids: HashSet<String> = self
                    .notification_center
                    .items
                    .iter()
                    .filter(|item| {
                        stepx_companion::desktop_notify::NotifyPreferences::normalize_app_key(
                            &item.package_name,
                        ) == package
                    })
                    .map(|item| item.id.clone())
                    .collect();
                self.notification_center
                    .popup_queue
                    .retain(|id| !ids.contains(id));
                self.notification_center.popup_shown_ids.extend(ids);
                self.notification_center.active_popup_id = None;
                self.notification_center.popup_cursor = 0;
                self.push.active_toasts.clear();
                self.sync_notification_center();
                self.status = "此应用不再弹窗，可在设置中恢复".into();
                self.error = false;
            }
            NotificationCenterAction::OpenPopupDetails(id) => {
                let Some(item) = self
                    .notification_center
                    .items
                    .iter()
                    .find(|item| item.id == id)
                    .cloned()
                else {
                    return;
                };
                match item.primary_action.as_ref().map(|(_, action)| action) {
                    Some(
                        NotificationPrimaryAction::OpenImThread { .. }
                        | NotificationPrimaryAction::OpenSms
                        | NotificationPrimaryAction::OpenAppUpdates
                        | NotificationPrimaryAction::UpdateSingleApp { .. }
                        | NotificationPrimaryAction::ResumeTask { .. }
                        | NotificationPrimaryAction::OpenPage(_),
                    ) => {
                        self.apply_notification_center_action(
                            NotificationCenterAction::TriggerItem(id),
                        );
                    }
                    _ => {
                        // Card clicks navigate; they never copy codes or approve phone actions.
                        self.page = if matches!(
                            item.popup_widget,
                            NotificationPopupWidget::SafetyConfirm { .. }
                        ) {
                            Page::Amoo
                        } else {
                            Page::Notifications
                        };
                        self.notification_center.drawer_open = false;
                        self.notification_center
                            .popup_queue
                            .retain(|queued| queued != &id);
                        self.notification_center.popup_shown_ids.insert(id.clone());
                        self.notification_center.active_popup_id = None;
                        if let Some(key) = id
                            .strip_prefix("push-notif:")
                            .or_else(|| id.strip_prefix("push:"))
                        {
                            self.push.selected_key = Some(key.into());
                        }
                        self.status = "已打开提醒详情".into();
                        self.error = false;
                    }
                }
            }
            NotificationCenterAction::ToggleDrawer => {
                self.notification_center.drawer_open = !self.notification_center.drawer_open;
                if self.notification_center.drawer_open {
                    self.notification_center.drawer_list_started = Some(Instant::now());
                    self.dismiss_search_for_header();
                    self.back_screen.live_capsule_popover_open = false;
                    self.header_actions_open = false;
                    for id in self.notification_center.popup_queue.drain(..) {
                        self.notification_center.popup_shown_ids.insert(id);
                    }
                    self.notification_center.popup_cursor = 0;
                    self.notification_center.active_popup_id = None;
                    self.notification_center.popup_remaining_ms = 0;
                    self.notification_center.popup_total_ms = 0;
                    self.sync_notification_center();
                }
            }
            NotificationCenterAction::CloseDrawer => {
                self.notification_center.drawer_open = false;
            }
            NotificationCenterAction::SetAlertMode(mode) => {
                self.notification_center.alert_mode = mode;
                if mode != NotificationAlertMode::PopupAndBadge {
                    self.notification_center.active_popup_id = None;
                    self.push.active_toasts.clear();
                    self.push.prefs.popup_mode = stepx_companion::desktop_notify::PopupMode::Silent;
                    self.settings.notify_draft.popup_mode =
                        stepx_companion::desktop_notify::PopupMode::Silent;
                } else if self.push.prefs.popup_mode
                    == stepx_companion::desktop_notify::PopupMode::Silent
                {
                    let restored = if self.push.prefs.privacy_mask {
                        stepx_companion::desktop_notify::PopupMode::Banner
                    } else {
                        stepx_companion::desktop_notify::PopupMode::FullReply
                    };
                    self.push.prefs.popup_mode = restored;
                    self.settings.notify_draft.popup_mode = restored;
                }
                if mode == NotificationAlertMode::Muted {
                    self.push.prefs.scope = stepx_companion::desktop_notify::AlertScope::Muted;
                    self.settings.notify_draft.scope =
                        stepx_companion::desktop_notify::AlertScope::Muted;
                } else if self.push.prefs.scope
                    == stepx_companion::desktop_notify::AlertScope::Muted
                {
                    self.push.prefs.scope =
                        stepx_companion::desktop_notify::AlertScope::ImAndOtpOnly;
                    self.settings.notify_draft.scope =
                        stepx_companion::desktop_notify::AlertScope::ImAndOtpOnly;
                }
                let _ = self.push.prefs.save();
                self.notification_center.save();
                self.sync_notification_center();
            }
            NotificationCenterAction::SelectCategory(cat) => {
                if self.notification_center.selected_category != cat {
                    self.notification_center.selected_category = cat;
                    self.notification_center.list_scroll = Default::default();
                    self.notification_center.drawer_list_started = Some(Instant::now());
                }
            }
            NotificationCenterAction::TriggerItem(id) => {
                if self.notification_center.items.is_empty() {
                    self.sync_notification_center();
                }
                let matched_item = self
                    .notification_center
                    .items
                    .iter()
                    .find(|i| i.id == id)
                    .cloned();
                let primary = matched_item
                    .as_ref()
                    .and_then(|i| i.primary_action.as_ref().map(|(_, act)| act.clone()));

                let now_ms = Local::now().timestamp_millis();
                if let Some(ref item) = matched_item {
                    if let NotificationPopupWidget::ExpressPickup {
                        ref pickup_code, ..
                    } = item.popup_widget
                    {
                        if is_real_pickup_code_label(pickup_code) {
                            let code = pickup_code.trim().to_string();
                            self.notification_center.last_copied_text = Some(code.clone());
                            self.back_screen.record_clipboard_history(&code);
                            self.back_screen.last_synced_desktop_clip = code.clone();
                            self.push.clipboard_preview = code.chars().take(240).collect();
                            self.notification_center.popup_feedback_text =
                                Some((format!("✓ 已复制取件码 {code}"), now_ms + 1_200));
                        }
                    }
                }
                self.mark_underlying_source_read(&id);
                for qid in self.notification_center.popup_queue.drain(..) {
                    self.notification_center.popup_shown_ids.insert(qid);
                }
                self.notification_center.popup_shown_ids.insert(id.clone());
                self.notification_center.popup_cursor = 0;
                self.notification_center.active_popup_id = None;
                if let Some(key) = id
                    .strip_prefix("push-notif:")
                    .or_else(|| id.strip_prefix("push:"))
                {
                    self.push.selected_key = Some(key.to_string());
                }

                if let Some(act) = primary {
                    match act {
                        NotificationPrimaryAction::CopyText { label, text } => {
                            self.notification_center.last_copied_text = Some(text.clone());
                            self.back_screen.record_clipboard_history(&text);
                            self.back_screen.last_synced_desktop_clip = text.clone();
                            self.push.clipboard_preview = text.chars().take(240).collect();
                            self.push
                                .active_toasts
                                .retain(|t| t.otp_code.as_deref() != Some(text.as_str()));
                            let fb = if label.contains("验证码") {
                                "✓ 已复制验证码".to_string()
                            } else {
                                format!("✓ 已复制{label}")
                            };
                            self.notification_center.popup_feedback_text =
                                Some((fb, now_ms + 1_200));
                            self.status = format!("已复制{label}：{text}");
                            self.error = false;
                        }
                        NotificationPrimaryAction::OpenImThread { thread_id } => {
                            self.page = Page::Sms;
                            self.selected_thread = None;
                            self.pending_sms_plan = None;
                            if !thread_id.is_empty() {
                                self.push.selected_im_thread = Some(thread_id.clone());
                                if let Some(thread) = self
                                    .push
                                    .snapshot
                                    .threads
                                    .iter_mut()
                                    .find(|t| t.thread_id == thread_id)
                                {
                                    self.push.im_channel = thread.channel();
                                    thread.unread_count = 0;
                                }
                                for notif in &mut self.push.snapshot.notifications {
                                    if notif.thread_id == thread_id {
                                        notif.read = true;
                                    }
                                }
                            }
                            Self::scroll_to_end(&self.sms_messages_scroll);
                            Self::scroll_to_end(&self.scroll_aux);
                            self.notification_center.drawer_open = false;
                            self.status = "已打开聊天会话".into();
                            self.error = false;
                        }
                        NotificationPrimaryAction::OpenSms => {
                            self.page = Page::Sms;
                            self.push.im_channel = ImChannel::Sms;
                            self.push.selected_im_thread = Some(String::new());
                            self.sms_filter = SmsFilter::All;
                            self.pending_sms_plan = None;
                            let target_addr = id
                                .strip_prefix("sms:")
                                .or_else(|| id.strip_prefix("missed-call:"))
                                .and_then(|rest| {
                                    rest.rsplit_once(':').map(|(addr, _)| addr).or(Some(rest))
                                })
                                .map(str::trim)
                                .filter(|s| !s.is_empty())
                                .map(ToOwned::to_owned)
                                .or_else(|| {
                                    self.sms
                                        .iter()
                                        .find(|m| {
                                            !m.outgoing && !m.read && !m.address.trim().is_empty()
                                        })
                                        .or_else(|| {
                                            self.sms.iter().find(|m| !m.address.trim().is_empty())
                                        })
                                        .map(|m| m.address.trim().to_string())
                                });
                            if let Some(addr) = target_addr {
                                self.selected_thread = Some(addr.clone());
                                self.fields[F_SMS_TO].set(&addr);
                            }
                            Self::scroll_to_end(&self.scroll_aux);
                            Self::scroll_to_end(&self.sms_messages_scroll);
                            self.notification_center.drawer_open = false;
                            self.status = "已打开消息与短信".into();
                            self.error = false;
                        }
                        NotificationPrimaryAction::OpenAppUpdates => {
                            self.page = Page::AppStore;
                            self.app_store.category = StoreCategory::Updatable;
                            self.app_store.selected_package = None;
                            self.notification_center.drawer_open = false;
                            self.status = "已打开手机应用更新".into();
                            self.error = false;
                        }
                        NotificationPrimaryAction::UpdateSingleApp { package_name } => {
                            self.page = Page::AppStore;
                            self.app_store.category = StoreCategory::Updatable;
                            self.app_store.selected_package = Some(package_name.clone());
                            self.notification_center.drawer_open = false;
                            self.status = format!("已定位待更新应用：{package_name}");
                            self.error = false;
                        }
                        NotificationPrimaryAction::ApproveGuiSafety(approved) => {
                            if approved {
                                self.handle_amoo_ui_action(&AmooUiAction::GuiAcknowledgeFinished);
                                self.amoo_ui.gui_paused = false;
                                self.amoo_ui.gui_message =
                                    "已批准当前手机操作，可继续执行。".into();
                                self.status = "已批准执行当前手机操作".into();
                            } else {
                                self.amoo_ui.gui_running = false;
                                self.amoo_ui.gui_paused = false;
                                self.amoo_ui.gui_message =
                                    "已拒绝当前手机敏感操作并停止等待。".into();
                                self.status = "已拒绝当前手机敏感操作".into();
                            }
                            self.amoo_ui.gui_unconfirmed = false;
                            self.amoo_safety_decision = if approved {
                                SafetyDecision::Approved
                            } else {
                                SafetyDecision::Rejected
                            };
                            self.notification_center.popup_feedback_text = Some((
                                if approved {
                                    "✓ 已批准操作".into()
                                } else {
                                    "已拒绝操作".into()
                                },
                                now_ms + 1_200,
                            ));
                            self.page = Page::Amoo;
                            self.notification_center.drawer_open = false;
                            self.error = false;
                        }
                        NotificationPrimaryAction::ResumeTask { task_id } => {
                            self.page = Page::Amoo;
                            self.workspace.panel = 0;
                            self.workspace.history = false;
                            if self.workspace.journal.tasks.iter().any(|t| t.id == task_id) {
                                self.workspace.active = Some(task_id);
                                self.workspace_restore();
                            }
                            self.reveal_latest_chat();
                            self.notification_center.drawer_open = false;
                            self.status = "已打开跨端协同任务".into();
                            self.error = false;
                        }
                        NotificationPrimaryAction::OpenPage(target_page) => {
                            if let Some(key) = id
                                .strip_prefix("push-notif:")
                                .or_else(|| id.strip_prefix("push:"))
                            {
                                self.push.selected_key = Some(key.to_string());
                            }
                            self.page = target_page;
                            self.notification_center.drawer_open = false;
                            self.error = false;
                        }
                    }
                }

                self.sync_notification_center();
                self.notification_center.save();
            }
            NotificationCenterAction::MarkRead(id) => {
                self.mark_underlying_source_read(&id);
                self.notification_center.popup_queue.retain(|q| q != &id);
                if self.notification_center.active_popup_id.as_deref() == Some(id.as_str()) {
                    self.notification_center.active_popup_id = None;
                }
                self.sync_notification_center();
                self.notification_center.save();
            }
            NotificationCenterAction::MarkAllRead => {
                let all_ids: Vec<String> = self
                    .notification_center
                    .items
                    .iter()
                    .map(|i| i.id.clone())
                    .collect();
                for id in all_ids {
                    self.mark_underlying_source_read(&id);
                }
                for notif in &self.push.snapshot.notifications {
                    self.back_screen
                        .smart_display_cleared_ids
                        .insert(format!("raw-notif:{}", notif.key));
                }
                self.notification_center.popup_queue.clear();
                self.notification_center.popup_cursor = 0;
                self.notification_center.active_popup_id = None;
                self.sync_notification_center();
                self.notification_center.save();
            }
            NotificationCenterAction::DismissItem(id) => {
                self.mark_underlying_source_read(&id);
                self.notification_center.dismissed_ids.insert(id.clone());
                self.notification_center.popup_queue.retain(|q| q != &id);
                if self.notification_center.active_popup_id.as_deref() == Some(id.as_str()) {
                    self.notification_center.active_popup_id = None;
                }
                self.sync_notification_center();
                self.notification_center.save();
            }
            NotificationCenterAction::ClearReadAndDismissed => {
                let read_item_ids: Vec<String> = self
                    .notification_center
                    .items
                    .iter()
                    .filter(|i| !i.unread)
                    .map(|i| i.id.clone())
                    .collect();
                for id in read_item_ids {
                    self.notification_center.dismissed_ids.insert(id);
                }
                for id in self.notification_center.read_ids.iter() {
                    self.notification_center.dismissed_ids.insert(id.clone());
                }
                self.notification_center.cached_history.retain(|i| {
                    i.unread && !self.notification_center.dismissed_ids.contains(&i.id)
                });
                self.sync_notification_center();
                self.notification_center.save();
            }
            NotificationCenterAction::DismissPopup => {
                if !self.notification_center.popup_queue.is_empty() {
                    let idx = self
                        .notification_center
                        .popup_cursor
                        .min(self.notification_center.popup_queue.len().saturating_sub(1));
                    if let Some(removed_id) = self.notification_center.popup_queue.remove(idx) {
                        self.notification_center.popup_shown_ids.insert(removed_id);
                    }
                    self.sync_active_popup_from_queue(true);
                } else if let Some(active_id) = self.notification_center.active_popup_id.take() {
                    self.notification_center.popup_shown_ids.insert(active_id);
                }
            }
            NotificationCenterAction::ClearPopupBatch { receipts } => {
                self.clear_popup_batch(receipts);
            }
            NotificationCenterAction::SwitchToBadgeOnlyFromPopup => {
                self.notification_center.alert_mode = NotificationAlertMode::BadgeOnly;
                self.notification_center.active_popup_id = None;
                self.push.active_toasts.clear();
                self.notification_center.save();
                self.sync_notification_center();
            }
            NotificationCenterAction::PopupPrev => {
                self.notification_center.popup_pending_front = None;
                let len = self.notification_center.popup_queue.len();
                if len > 0 {
                    if self.notification_center.popup_cursor == 0 {
                        self.notification_center.popup_cursor = len - 1;
                    } else {
                        self.notification_center.popup_cursor -= 1;
                    }
                    self.sync_active_popup_from_queue(true);
                }
            }
            NotificationCenterAction::PopupNext => {
                self.notification_center.popup_pending_front = None;
                let len = self.notification_center.popup_queue.len();
                if len > 0 {
                    self.notification_center.popup_cursor =
                        (self.notification_center.popup_cursor + 1) % len;
                    self.sync_active_popup_from_queue(true);
                }
            }
            NotificationCenterAction::PopupHoverChanged(hovered) => {
                self.notification_center.popup_hovered = hovered;
                if hovered
                    && self.notification_center.popup_total_ms > 0
                    && self.notification_center.popup_remaining_ms == 0
                {
                    self.notification_center.popup_remaining_ms =
                        self.notification_center.popup_total_ms;
                }
            }
            NotificationCenterAction::SnoozePopups { duration_ms } => {
                let now_ms = Local::now().timestamp_millis();
                self.notification_center.snooze_until_ms = Some(now_ms + duration_ms.max(0));
                self.notification_center.popup_queue.clear();
                self.notification_center.popup_cursor = 0;
                self.notification_center.active_popup_id = None;
                self.notification_center.popup_remaining_ms = 0;
                self.notification_center.popup_total_ms = 0;
                self.notification_center.popup_hovered = false;
                self.push.active_toasts.clear();
                self.status = format!(
                    "弹窗已暂停至 {}",
                    Local::now()
                        .checked_add_signed(chrono::Duration::milliseconds(duration_ms.max(0)))
                        .unwrap_or_else(Local::now)
                        .format("%H:%M")
                );
                if !self.demo_mode {
                    self.notification_center.save();
                }
                self.error = false;
            }
            NotificationCenterAction::ClearSnooze => {
                self.notification_center.snooze_until_ms = None;
                if !self.demo_mode {
                    self.notification_center.save();
                }
                self.sync_notification_center();
            }
            NotificationCenterAction::PopupQuickReply { item_id, text }
            | NotificationCenterAction::SendQuickReply(item_id, text) => {
                self.submit_popup_quick_reply(&item_id, &text);
            }
            NotificationCenterAction::SubmitPopupReply(item_id) => {
                // An event painted for a previous card must never send the next
                // conversation's draft to its captured recipient.
                if self.notification_center.popup_reply_item_id.as_deref() != Some(item_id.as_str())
                    || self.notification_center.active_popup_id.as_deref() != Some(item_id.as_str())
                {
                    return;
                }
                let text = self.fields[super::F_TOAST_REPLY].text.trim().to_string();
                if !text.is_empty() {
                    self.submit_popup_quick_reply(&item_id, &text);
                    if !self.error {
                        self.notification_center.popup_reply_drafts.remove(&item_id);
                        if self.notification_center.popup_reply_item_id.as_deref()
                            == Some(item_id.as_str())
                        {
                            self.fields[super::F_TOAST_REPLY].clear();
                        }
                    }
                }
            }
            NotificationCenterAction::ReplayLatestPopups => {
                self.notification_center.alert_mode = NotificationAlertMode::PopupAndBadge;
                self.notification_center.snooze_until_ms = None;
                self.notification_center.drawer_open = false;
                self.sync_notification_center();

                let mut selected_ids: Vec<String> = Vec::with_capacity(MAX_POPUP_QUEUE_LEN);
                for item in &self.notification_center.items {
                    if selected_ids.len() >= MAX_POPUP_QUEUE_LEN {
                        break;
                    }
                    if item.unread
                        && matches!(
                            item.priority,
                            NotificationPriority::P0Immediate | NotificationPriority::P1Digest
                        )
                        && (matches!(
                            item.category,
                            NotificationCategory::PhoneApps | NotificationCategory::PhoneComm
                        ) || matches!(
                            item.popup_widget,
                            NotificationPopupWidget::ClipboardPreview { .. }
                        ))
                    {
                        selected_ids.push(item.id.clone());
                    }
                }
                for item in &self.notification_center.items {
                    if selected_ids.len() >= MAX_POPUP_QUEUE_LEN {
                        break;
                    }
                    if item.unread
                        && matches!(
                            item.priority,
                            NotificationPriority::P0Immediate | NotificationPriority::P1Digest
                        )
                        && !selected_ids.contains(&item.id)
                    {
                        selected_ids.push(item.id.clone());
                    }
                }
                if selected_ids.is_empty() {
                    for item in &self.notification_center.items {
                        if selected_ids.len() >= MAX_POPUP_QUEUE_LEN {
                            break;
                        }
                        if item.unread && !selected_ids.contains(&item.id) {
                            selected_ids.push(item.id.clone());
                        }
                    }
                }
                if selected_ids.is_empty() {
                    for item in self
                        .notification_center
                        .items
                        .iter()
                        .filter(|i| {
                            matches!(
                                i.category,
                                NotificationCategory::PhoneApps | NotificationCategory::PhoneComm
                            )
                        })
                        .take(MAX_POPUP_QUEUE_LEN)
                    {
                        selected_ids.push(item.id.clone());
                    }
                }
                if selected_ids.is_empty() {
                    for item in self
                        .notification_center
                        .items
                        .iter()
                        .take(MAX_POPUP_QUEUE_LEN)
                    {
                        selected_ids.push(item.id.clone());
                    }
                }

                for id in &selected_ids {
                    self.notification_center.popup_shown_ids.remove(id);
                    self.notification_center.read_ids.remove(id);
                    self.notification_center.dismissed_ids.remove(id);
                    if let Some(item) = self
                        .notification_center
                        .items
                        .iter_mut()
                        .find(|i| &i.id == id)
                    {
                        item.unread = true;
                    }
                }
                self.notification_center.popup_queue = VecDeque::from(selected_ids);
                self.notification_center.popup_cursor = 0;
                self.sync_active_popup_from_queue(true);
                self.notification_center.save();
                self.status = format!(
                    "已弹出 {} 条高价值消息提醒卡片",
                    self.notification_center.popup_queue.len()
                );
                self.error = false;
            }
            NotificationCenterAction::PreviewItemPopup(target) => {
                self.notification_center.alert_mode = NotificationAlertMode::PopupAndBadge;
                self.notification_center.snooze_until_ms = None;
                self.notification_center.drawer_open = false;
                self.sync_notification_center();

                let trimmed = target.trim();
                let resolved_id = self
                    .notification_center
                    .items
                    .iter()
                    .find(|i| i.id == trimmed)
                    .or_else(|| {
                        self.notification_center.items.iter().find(|i| {
                            i.id == format!("push-notif:{trimmed}")
                                || i.id == format!("im-thread:{trimmed}")
                                || i.id == format!("otp:{trimmed}")
                        })
                    })
                    .or_else(|| {
                        self.notification_center.items.iter().find(|i| {
                            !trimmed.is_empty()
                                && (i.id.starts_with(trimmed)
                                    || i.id.starts_with(&format!("sms:{trimmed}:"))
                                    || i.id.starts_with(&format!("missed-call:{trimmed}:"))
                                    || i.id.starts_with(&format!("express:{trimmed}:"))
                                    || i.id.contains(trimmed)
                                    || i.title.contains(trimmed)
                                    || i.source_badge.contains(trimmed))
                        })
                    })
                    .or_else(|| self.notification_center.items.iter().find(|i| i.unread))
                    .or_else(|| self.notification_center.items.first())
                    .map(|i| i.id.clone());

                if let Some(rid) = resolved_id {
                    self.notification_center.popup_shown_ids.remove(&rid);
                    self.notification_center.read_ids.remove(&rid);
                    self.notification_center.dismissed_ids.remove(&rid);
                    if let Some(item) = self
                        .notification_center
                        .items
                        .iter_mut()
                        .find(|i| i.id == rid)
                    {
                        item.unread = true;
                    }
                    self.notification_center.popup_queue.retain(|q| q != &rid);
                    let was_empty = self.notification_center.popup_queue.is_empty();
                    self.notification_center.popup_queue.push_front(rid.clone());
                    if was_empty {
                        let extras: Vec<String> = self
                            .notification_center
                            .items
                            .iter()
                            .filter(|i| {
                                i.id != rid
                                    && i.unread
                                    && matches!(
                                        i.priority,
                                        NotificationPriority::P0Immediate
                                            | NotificationPriority::P1Digest
                                    )
                            })
                            .take(4)
                            .map(|i| i.id.clone())
                            .collect();
                        for eid in extras {
                            self.notification_center.popup_shown_ids.remove(&eid);
                            self.notification_center.popup_queue.push_back(eid);
                        }
                    }
                    while self.notification_center.popup_queue.len() > MAX_POPUP_QUEUE_LEN {
                        self.notification_center.popup_queue.pop_back();
                    }
                    self.notification_center.popup_cursor = 0;
                    self.sync_active_popup_from_queue(true);
                    self.notification_center.save();
                    self.status = "已弹出消息提醒卡片预览".into();
                    self.error = false;
                }
            }
            NotificationCenterAction::MovePopupBy { dx, dy } => {
                let (ox, oy) = self.notification_center.popup_drag_offset;
                self.notification_center.popup_drag_offset = (
                    (ox + dx as f32).clamp(-1200.0, 400.0),
                    (oy + dy as f32).clamp(-100.0, 900.0),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::{TestUiConfigDirGuard, F_GLOBAL_SEARCH};
    use stepx_companion::{
        amoo_workspace::Task,
        apps::AppInfo,
        device::{CallRecord, Device, ExpressParcelInfo},
        push::{ImThread, OtpBanner},
        sms::Message,
    };

    fn setup_test_companion() -> (tempfile::TempDir, TestUiConfigDirGuard, Companion) {
        let dir = tempfile::tempdir().expect("create temp config dir");
        let guard = TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.notification_center = NotificationCenterState::new_empty();
        (dir, guard, app)
    }

    #[test]
    fn displayed_clipboard_is_not_replayed_on_restart_but_new_content_is() {
        let (dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = true;
        app.selected = Some(Device { serial:"popup-receipt-test".into(), ..Default::default() });
        app.push.prefs.popup_mode = stepx_companion::desktop_notify::PopupMode::FullReply;
        app.back_screen.stepos_suite.phone_clipboard = "第一份资料".into();
        app.sync_notification_center();
        assert!(app.notification_center.popup_queue.iter().any(|id| id.starts_with("phone-clipboard:")));
        let path = dir.path().join("restart.json");
        app.notification_center.save_to_path(&path).unwrap();
        app.notification_center = NotificationCenterState::load_from_path(&path);
        // Receipt persistence must work even after history eviction and without marking read.
        app.notification_center.cached_history.clear();
        app.notification_center.popup_shown_ids.clear();
        app.notification_center.read_ids.clear();
        app.sync_notification_center();
        assert!(app.notification_center.popup_queue.is_empty());
        app.back_screen.stepos_suite.phone_clipboard = "第二份新资料".into();
        app.sync_notification_center();
        assert_eq!(app.notification_center.popup_queue.len(), 1);
    }

    #[test]
    fn live_call_alert_disappears_on_idle_and_never_becomes_cached_history() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = true;
        app.selected = Some(Device { serial:"call-test".into(), ..Default::default() });
        app.push.prefs.popup_mode = stepx_companion::desktop_notify::PopupMode::FullReply;
        let now = Local::now().timestamp_millis();
        app.push.snapshot.live_call = stepx_companion::phone::LiveCall {
            available:true, id:"test-call".into(), state:"ringing".into(), number:"+15550163".into(),
            started_ms:now, observed_ms:now, ..Default::default()
        };
        app.sync_notification_center();
        assert_eq!(app.notification_center.popup_queue.front().unwrap(), "live-call:test-call");
        assert!(!app.notification_center.cached_history.iter().any(|i| i.id.starts_with("live-call:")));
        app.push.snapshot.live_call.state = "idle".into();
        app.sync_notification_center();
        assert!(!app.notification_center.items.iter().any(|i| i.id.starts_with("live-call:")));
        assert!(app.notification_center.popup_queue.is_empty());
    }

    #[test]
    fn opening_notification_center_replaces_search_without_losing_query() {
        let (_dir, _guard, mut app) = setup_test_companion();
        for closing in [false, true] {
            app.notification_center.drawer_open = false;
            app.search.open = !closing;
            app.search.closing = closing.then(std::time::Instant::now);
            app.fields[F_GLOBAL_SEARCH].set("待查的消息");
            app.focus = Some(F_GLOBAL_SEARCH);
            app.back_screen.live_capsule_popover_open = true;
            app.header_actions_open = true;

            app.apply_notification_center_action(NotificationCenterAction::ToggleDrawer);

            assert!(app.notification_center.drawer_open);
            assert!(!app.search.open);
            assert!(app.search.closing.is_none());
            assert!(!app.back_screen.live_capsule_popover_open);
            assert!(!app.header_actions_open);
            assert_eq!(app.focus, None);
            assert_eq!(app.fields[F_GLOBAL_SEARCH].text, "待查的消息");
            app.apply_notification_center_action(NotificationCenterAction::ToggleDrawer);
            assert!(!app.notification_center.drawer_open);
            assert!(!app.search.open);
        }
    }

    fn popup_test_threads() -> Vec<ImThread> {
        [
            ("wx:one", "com.tencent.mm"),
            ("wx:two", "com.tencent.mm"),
            ("wa:one", "com.whatsapp"),
        ]
        .into_iter()
        .map(|(id, package)| ImThread {
            thread_id: id.into(),
            package_name: package.into(),
            title: id.into(),
            last_message: "未读消息".into(),
            unread_count: 1,
            last_timestamp_ms: Local::now().timestamp_millis(),
            ..Default::default()
        })
        .collect()
    }

    #[test]
    fn clear_popup_batch_acknowledges_waiting_events_without_changing_history_or_preferences() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.push.snapshot.threads = popup_test_threads();
        app.sync_notification_center();
        assert_eq!(app.notification_center.popup_queue.len(), 3);
        let active = app.notification_active_popup().unwrap().id.clone();
        let history = serde_json::to_value(&app.notification_center.cached_history).unwrap();
        let unread = app.notification_raw_unread_count();
        let prefs = serde_json::to_value(&app.push.prefs).unwrap();
        let read_ids = app.notification_center.read_ids.clone();
        let dismissed_ids = app.notification_center.dismissed_ids.clone();
        app.fields[super::super::F_TOAST_REPLY].set("还没有发送的草稿");
        app.notification_center.popup_menu_item_id = Some(active.clone());
        app.notification_center.popup_pending_front = app.notification_center.popup_queue.back().cloned();
        let receipts = app.popup_batch_receipts();

        app.apply_notification_center_action(NotificationCenterAction::ClearPopupBatch { receipts });

        assert!(app.notification_center.popup_queue.is_empty());
        assert!(app.notification_center.active_popup_id.is_none());
        assert!(app.notification_center.popup_pending_front.is_none());
        assert!(app.notification_center.popup_menu_item_id.is_none());
        assert_eq!(app.notification_raw_unread_count(), unread);
        assert_eq!(serde_json::to_value(&app.notification_center.cached_history).unwrap(), history);
        assert_eq!(serde_json::to_value(&app.push.prefs).unwrap(), prefs);
        assert_eq!(app.notification_center.popup_reply_drafts.get(&active).unwrap(), "还没有发送的草稿");
        assert_eq!(app.notification_center.read_ids, read_ids);
        assert_eq!(app.notification_center.dismissed_ids, dismissed_ids);
        assert!(app.notification_center.snooze_until_ms.is_none());
        assert_eq!(app.notification_center.alert_mode, NotificationAlertMode::PopupAndBadge);
        app.sync_notification_center();
        assert!(app.notification_center.popup_queue.is_empty(), "polling cannot replay cleared waiting items");
    }

    #[test]
    fn cleared_popup_batch_survives_restart_and_new_message_in_same_thread_still_alerts() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.push.snapshot.threads = popup_test_threads();
        app.sync_notification_center();
        app.apply_notification_center_action(NotificationCenterAction::ClearPopupBatch {
            receipts: app.popup_batch_receipts(),
        });
        // Use the real action's persisted file, not a separate test save.
        app.notification_center = NotificationCenterState::load_from_path(&persistence_path());
        assert_eq!(app.notification_center.items.len(), 3);
        app.sync_notification_center();
        assert!(app.notification_center.popup_queue.is_empty());

        app.push.snapshot.threads[0].last_message = "清理后发来的新消息".into();
        app.push.snapshot.threads[0].last_timestamp_ms += 1;
        // Simulate new content arriving before the first post-restart read too.
        app.notification_center = NotificationCenterState::load_from_path(&persistence_path());
        app.sync_notification_center();
        assert_eq!(app.notification_center.popup_queue.len(), 1);
        assert_eq!(app.notification_active_popup().unwrap().id, "im-thread:wx:one");
    }

    #[test]
    fn clear_popup_batch_does_not_acknowledge_new_arrivals_after_menu_snapshot() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.push.snapshot.threads = popup_test_threads();
        app.sync_notification_center();
        let receipts = app.popup_batch_receipts();
        app.push.snapshot.threads[0].last_message = "菜单打开后同一会话的新消息".into();
        app.push.snapshot.threads[0].last_timestamp_ms += 10;
        app.push.snapshot.threads.push(ImThread {
            thread_id: "new-arrival".into(), package_name: "com.tencent.mm".into(),
            title: "新会话".into(), last_message: "新内容".into(), unread_count: 1,
            last_timestamp_ms: Local::now().timestamp_millis() + 20, ..Default::default()
        });
        app.sync_notification_center();
        app.apply_notification_center_action(NotificationCenterAction::ClearPopupBatch { receipts });
        let queue: HashSet<_> = app.notification_center.popup_queue.iter().map(String::as_str).collect();
        assert_eq!(queue, HashSet::from(["im-thread:wx:one", "im-thread:new-arrival"]));
        app.sync_notification_center();
        assert_eq!(app.notification_center.popup_queue.len(), 2);
    }

    #[test]
    fn clear_popup_batch_preserves_ringing_and_connected_calls() {
        for state in ["ringing", "active"] {
            let (_dir, _guard, mut app) = setup_test_companion();
            app.selected = Some(Device { serial: "popup-batch-call-test".into(), ..Default::default() });
            app.push.snapshot.threads = popup_test_threads();
            let now = Local::now().timestamp_millis();
            app.push.snapshot.live_call = stepx_companion::phone::LiveCall {
                available: true, id: "protected-call".into(), state: state.into(), number: "+15550163".into(),
                started_ms: now, observed_ms: now, ..Default::default()
            };
            app.sync_notification_center();
            let receipts = app.popup_batch_receipts();
            assert_eq!(receipts.len(), 3);
            assert!(receipts.iter().all(|(id, _)| !id.starts_with("live-call:")));
            app.apply_notification_center_action(NotificationCenterAction::ClearPopupBatch { receipts });
            assert_eq!(app.notification_center.popup_queue.len(), 1);
            assert_eq!(app.notification_active_popup().unwrap().id, "live-call:protected-call");
            assert_eq!(app.push.snapshot.live_call.state, state);
            assert_eq!(app.push.snapshot.live_call.number, "+15550163");
            app.notification_center.popup_queue.clear();
            app.apply_notification_center_action(NotificationCenterAction::ClearPopupBatch { receipts: Vec::new() });
            assert_eq!(app.notification_active_popup().unwrap().id, "live-call:protected-call",
                "a visible call remains actionable while the queue rebuilds");
            app.sync_notification_center();
            assert_eq!(app.notification_center.popup_queue.len(), 1);
        }
    }

    #[test]
    fn latest_arrivals_and_updated_conversations_precede_old_popups() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now = Local::now().timestamp_millis();
        app.push.snapshot.threads = vec![popup_test_threads().remove(0)];
        app.push.snapshot.threads[0].last_timestamp_ms = now - 1_000;
        app.sync_notification_center();
        let old = app.notification_active_popup().unwrap().id.clone();
        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "735291".into(), source_app: "短信".into(), title: "登录验证码".into(),
            raw_text: "验证码735291".into(), timestamp_ms: now, ..Default::default()
        });
        app.sync_notification_center();
        assert_eq!(app.notification_center.popup_queue.front().unwrap(), "otp:735291");
        assert_eq!(app.notification_active_popup().unwrap().id, "otp:735291");
        app.apply_notification_center_action(NotificationCenterAction::PopupNext);
        app.sync_notification_center();
        assert_eq!(app.notification_active_popup().unwrap().id, old, "unchanged polling must not reset explicit navigation");
        app.push.snapshot.threads.push(ImThread {
            thread_id: "latest".into(), package_name: "com.tencent.mm".into(), title: "新的消息".into(),
            last_message: "新内容".into(), unread_count: 1, last_timestamp_ms: now + 2, ..Default::default()
        });
        app.sync_notification_center();
        assert_eq!(app.notification_active_popup().unwrap().id, "im-thread:latest");
        app.push.snapshot.threads[0].last_timestamp_ms = now + 3;
        app.push.snapshot.threads[0].last_message = "原会话里的新消息".into();
        app.sync_notification_center();
        assert_eq!(app.notification_center.popup_queue.front(), Some(&old));
        assert_eq!(app.notification_active_popup().unwrap().id, old);
        assert_eq!(app.notification_center.popup_queue_len(), 3, "one card per conversation");
        app.push.snapshot.threads.push(ImThread {
            thread_id: "late-batch".into(), package_name: "com.tencent.mm".into(), title: "延迟读取".into(),
            last_message: "较早消息".into(), unread_count: 1, last_timestamp_ms: now - 500, ..Default::default()
        });
        app.sync_notification_center();
        assert_eq!(app.notification_active_popup().unwrap().id, old, "late batch cannot replace a newer message");
    }

    #[test]
    fn newest_popup_waits_for_reply_ime_pointer_and_menu_ownership() {
        for protection in 0..5 {
            let (_dir, _guard, mut app) = setup_test_companion();
            app.push.snapshot.threads = vec![popup_test_threads().remove(0)];
            app.sync_notification_center();
            let old = app.notification_active_popup().unwrap().id.clone();
            match protection {
                0 => app.desktop_popup.reply_focused = true,
                1 => app.fields[super::super::F_TOAST_REPLY].set("原会话草稿"),
                2 => app.fields[super::super::F_TOAST_REPLY].preedit = "拼音".into(),
                3 => app.notification_center.popup_hovered = true,
                _ => app.notification_center.popup_menu_item_id = Some(old.clone()),
            }
            app.push.snapshot.otp_banners.push(OtpBanner {
                code: "735291".into(), raw_text: "验证码735291".into(),
                timestamp_ms: Local::now().timestamp_millis() + 1, ..Default::default()
            });
            app.sync_notification_center();
            assert_eq!(app.notification_active_popup().unwrap().id, old, "protection {protection}");
            assert_eq!(app.notification_center.popup_queue.front().unwrap(), "otp:735291");
            assert_eq!(app.notification_center.popup_reply_item_id.as_ref(), Some(&old));
            assert!(!app.tick_notification_popup_timer(1));
            app.desktop_popup.reply_focused = false;
            app.fields[super::super::F_TOAST_REPLY].set("");
            app.fields[super::super::F_TOAST_REPLY].preedit.clear();
            app.notification_center.popup_hovered = false;
            app.notification_center.popup_menu_item_id = None;
            assert!(app.tick_notification_popup_timer(1));
            assert_eq!(app.notification_active_popup().unwrap().id, "otp:735291");
        }
    }

    #[test]
    fn active_call_retains_controls_when_new_otp_arrives() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.selected = Some(Device { serial: "popup-call-test".into(), ..Default::default() });
        let now = Local::now().timestamp_millis();
        app.push.snapshot.live_call = stepx_companion::phone::LiveCall {
            available: true, id: "popup-call".into(), state: "ringing".into(), number: "+15550163".into(),
            started_ms: now, observed_ms: now, ..Default::default()
        };
        app.sync_notification_center();
        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "735291".into(), raw_text: "验证码735291".into(), timestamp_ms: now + 1, ..Default::default()
        });
        app.sync_notification_center();
        assert_eq!(app.notification_active_popup().unwrap().id, "live-call:popup-call");
        app.push.snapshot.live_call.state = "idle".into();
        app.sync_notification_center();
        assert_eq!(app.notification_active_popup().unwrap().id, "otp:735291");
    }

    #[test]
    fn red_packet_style_requires_original_structured_payload() {
        for (package, body, expected) in [
            ("com.tencent.mm", "[微信红包]恭喜发财，大吉大利", true),
            ("com.tencent.mm", "[3条]小林：[红包]午饭红包", true),
            ("com.tencent.mobileqq", "[QQ红包]生日快乐", true),
            ("com.eg.android.AlipayGphone", "收到一个红包", true),
            ("com.tencent.mm", "给你发了一个红包", true),
            ("com.tencent.mm", "今晚有红包吗？", false),
            ("com.tencent.mm", "请发送 [红包] 试试看", false),
            ("com.tencent.mm", "[微信红包]已领取", false),
            ("com.tencent.mm", "红包到账提醒：已退还", false),
            ("com.shopping.app", "领红包，点我抽奖", false),
            ("com.shopping.app", "[红包]", false),
        ] {
            let item = UnifiedNotificationItem {
                package_name: package.into(), full_body: body.into(), title: "红包群".into(), ..Default::default()
            };
            assert_eq!(item.is_red_packet(), expected, "{package}: {body}");
        }
    }

    #[test]
    fn popup_menu_snooze_persists_and_keeps_messages_readable() {
        let (dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = false;
        app.push.snapshot.threads = popup_test_threads();
        app.sync_notification_center();
        for minutes in [30, 60, 180, 360] {
            let before = Local::now().timestamp_millis();
            app.apply_notification_center_action(NotificationCenterAction::SnoozePopups {
                duration_ms: minutes * 60_000,
            });
            let until = app.notification_center.snooze_until_ms.unwrap();
            assert!((before + minutes * 60_000
                ..=Local::now().timestamp_millis() + minutes * 60_000)
                .contains(&until));
            assert_eq!(NotificationCenterState::load().snooze_until_ms, Some(until));
            app.sync_notification_center();
            assert!(app.notification_active_popup().is_none());
            assert_eq!(app.push.snapshot.threads.len(), 3);
            assert!(app
                .push
                .snapshot
                .threads
                .iter()
                .all(|t| t.unread_count == 1));
        }
        app.apply_notification_center_action(NotificationCenterAction::ClearSnooze);
        assert_eq!(NotificationCenterState::load().snooze_until_ms, None);
        assert!(app.notification_active_popup().is_some());

        app.apply_notification_center_action(NotificationCenterAction::SnoozeToday);
        let now = Local::now();
        let tomorrow = now
            .date_naive()
            .succ_opt()
            .unwrap()
            .and_hms_opt(0, 0, 0)
            .unwrap()
            .and_local_timezone(Local)
            .earliest()
            .unwrap()
            .timestamp_millis();
        assert!((app.notification_center.snooze_until_ms.unwrap() - tomorrow).abs() < 100);
        assert_eq!(
            NotificationCenterState::load().snooze_until_ms,
            app.notification_center.snooze_until_ms
        );

        app.notification_center.snooze_until_ms = Some(now.timestamp_millis() - 1);
        let expired = dir.path().join("expired-snooze.json");
        app.notification_center.save_to_path(&expired).unwrap();
        assert_eq!(
            NotificationCenterState::load_from_path(&expired).snooze_until_ms,
            None
        );
    }

    #[test]
    fn popup_menu_mutes_only_its_app_and_preserves_other_settings_edits() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = false;
        app.push.prefs.scope = stepx_companion::desktop_notify::AlertScope::All;
        app.push.snapshot.threads = popup_test_threads();
        app.sync_notification_center();
        let privacy = app.push.prefs.privacy_mask;
        app.settings.notify_draft.privacy_mask = !privacy;
        app.apply_notification_center_action(NotificationCenterAction::MutePopupApp(
            "im-thread:wx:one".into(),
        ));
        let persisted = stepx_companion::desktop_notify::load_preferences();
        assert_eq!(
            persisted.app_popup_mode_override("com.tencent.mm"),
            Some(stepx_companion::desktop_notify::PopupMode::Silent)
        );
        assert!(persisted
            .effective_popup_mode("com.whatsapp")
            .allows_popup());
        assert_eq!(persisted.privacy_mask, privacy);
        assert_eq!(app.settings.notify_draft.privacy_mask, !privacy);
        assert_eq!(
            app.settings
                .notify_draft
                .app_popup_mode_override("com.tencent.mm"),
            Some(stepx_companion::desktop_notify::PopupMode::Silent)
        );
        assert!(app
            .notification_center
            .popup_queue
            .iter()
            .all(|id| !id.starts_with("im-thread:wx:")));
        assert!(app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id == "im-thread:wa:one"));
        assert!(app
            .push
            .snapshot
            .threads
            .iter()
            .all(|t| t.unread_count == 1));
    }

    #[test]
    fn popup_menu_stays_bound_to_the_card_and_card_click_only_navigates() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.push.snapshot.threads = popup_test_threads();
        app.sync_notification_center();
        let id = app.notification_active_popup().unwrap().id.clone();
        app.apply_notification_center_action(NotificationCenterAction::PopupMenu(Some(id.clone())));
        assert_eq!(
            app.notification_center.popup_menu_item_id.as_ref(),
            Some(&id)
        );
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:new".into(),
            package_name: "com.tencent.mm".into(),
            title: "新消息".into(),
            last_message: "刚收到的消息".into(),
            unread_count: 1,
            last_timestamp_ms: Local::now().timestamp_millis() + 1,
            ..Default::default()
        });
        app.sync_notification_center();
        assert_eq!(app.notification_active_popup().unwrap().id, id);
        assert_eq!(
            app.notification_center.popup_menu_item_id.as_ref(),
            Some(&id)
        );
        app.apply_notification_center_action(NotificationCenterAction::OpenPopupDetails(
            id.clone(),
        ));
        assert_eq!(app.page, Page::Sms);
        assert_eq!(
            app.push.selected_im_thread.as_deref(),
            id.strip_prefix("im-thread:")
        );
        assert!(app.notification_center.popup_menu_item_id.is_none());

        let mut item = app.notification_center.items[0].clone();
        item.id = "copy-test".into();
        item.primary_action = Some((
            "复制".into(),
            NotificationPrimaryAction::CopyText {
                text: "654321".into(),
                label: "验证码".into(),
            },
        ));
        app.notification_center.items.push(item);
        app.apply_notification_center_action(NotificationCenterAction::OpenPopupDetails(
            "copy-test".into(),
        ));
        assert_eq!(app.page, Page::Notifications);
        assert!(app.notification_center.last_copied_text.is_none());
    }

    #[test]
    fn periodic_aggregation_does_not_delay_real_message_receipts() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.sync_notification_center();
        let previous = app.notification_center.last_materialized_at;
        for _ in 0..20 {
            app.tick_notification_center();
        }
        assert_eq!(app.notification_center.last_materialized_at, previous);

        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:realtime".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "项目群".into(),
            unread_count: 1,
            last_message: "新的讨论".into(),
            last_timestamp_ms: Local::now().timestamp_millis(),
            ..Default::default()
        });
        // This is the same immediate path used when a phone snapshot arrives.
        app.sync_notification_center();
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|i| i.id == "im-thread:wx:realtime"));
        assert_eq!(
            app.notification_center.active_popup_id.as_deref(),
            Some("im-thread:wx:realtime")
        );

        app.notification_center.last_materialized_at =
            Some(std::time::Instant::now() - std::time::Duration::from_secs(2));
        let stale = app.notification_center.last_materialized_at;
        app.tick_notification_center();
        assert!(app.notification_center.last_materialized_at > stale);
    }

    #[test]
    fn notification_ui_tests_save_library_preferences_only_in_the_test_directory() {
        let (dir, _guard, mut app) = setup_test_companion();
        assert_eq!(stepx_companion::resources::user_config(), dir.path());
        app.apply_notification_center_action(NotificationCenterAction::SetAlertMode(
            NotificationAlertMode::Muted,
        ));
        let saved = std::fs::read(dir.path().join("stepx-companion/notify_prefs.json")).unwrap();
        let prefs: stepx_companion::desktop_notify::NotifyPreferences =
            serde_json::from_slice(&saved).unwrap();
        assert_eq!(
            prefs.scope,
            stepx_companion::desktop_notify::AlertScope::Muted
        );
    }

    #[test]
    fn aggregates_all_five_categories_from_six_sources() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();

        // 1. PhoneApps: WeChat unread IM thread
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:product-group".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "产品研发群".into(),
            is_group: true,
            unread_count: 3,
            last_sender: "张晓峰".into(),
            last_message: "今晚 8 点联调新版通知中心".into(),
            last_timestamp_ms: now_ms - 10_000,
            can_remote_reply: true,
            ..Default::default()
        });

        // 2. PhoneComm: OTP banner + missed call + unread SMS + express parcel
        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "839201".into(),
            source_app: "招商银行".into(),
            title: "招商银行".into(),
            raw_text: "您的验证码为 839201，5 分钟内有效".into(),
            timestamp_ms: now_ms - 5_000,
            notification_key: "otp-1".into(),
        });
        app.call_logs.push(CallRecord {
            number: "139 1024 6688".into(),
            name: "林婉清".into(),
            call_type: 3,
            date_ms: now_ms - 60_000,
            duration_sec: 0,
        });
        app.sms.push(Message {
            provider_id: Some(10),
            address: "10086".into(),
            body: "您本月套餐流量剩余 4.2GB".into(),
            date_ms: now_ms - 120_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        app.express_parcels.push(ExpressParcelInfo {
            carrier: "顺丰速运".into(),
            tracking_no: "SF1001".into(),
            pickup_code: "6-1-2048".into(),
            station: "科技园丰巢柜".into(),
            status: "待取件".into(),
            updated_at: "刚刚".into(),
        });

        // 3. AppUpdates: installed older WeChat + active store task
        app.apps.push(AppInfo {
            package: "com.tencent.mm".into(),
            label: "微信".into(),
            version_name: "1.0.0".into(),
            version_code: 1,
            enabled: true,
            launchable: true,
            ..Default::default()
        });

        // 4. CompanionAndTasks: phone-work-* task + GUI safety approval
        let mut phone_task = Task::new(Some("STEPX_TEST".into()), None);
        phone_task.id = "phone-work-42".into();
        phone_task.title = "整理周报并发回手机".into();
        phone_task.note = "待回传".into();
        app.workspace.journal.tasks.push(phone_task);
        app.amoo_ui.gui_unconfirmed = true;
        app.amoo_ui.gui_message = "需要确认提交订单操作".into();

        // 5. System: connected phone with low battery (18%) and high storage (89%)
        app.selected = Some(Device {
            transport: "192.168.1.8:5555".into(),
            serial: "STEPX_TEST_SERIAL".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        app.summary =
            "电量  18%    ·    未充电\n手机存储  256G\n已用  228G    ·    可用  28G    ·    使用率  89%"
                .into();

        app.sync_notification_center();

        assert!(app.notification_category_unread(NotificationCategory::PhoneApps) >= 1);
        assert!(app.notification_category_unread(NotificationCategory::PhoneComm) >= 4);
        assert!(app.notification_category_unread(NotificationCategory::AppUpdates) >= 1);
        assert!(app.notification_category_unread(NotificationCategory::CompanionAndTasks) >= 2);
        assert!(app.notification_category_unread(NotificationCategory::System) >= 2);
        assert!(app.notification_unread_count() >= 10);

        // Verify concise titles and expected IDs across sources
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|i| i.id == "otp:839201" && i.title.contains("839201")));
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|i| i.id.starts_with("missed-call:") && i.title.contains("林婉清")));
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|i| i.id == "gui-safety-approval" && i.title == "手机操作待您批准"));
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|i| i.id == "task:phone-work-42" && i.title == "手机委派任务待回传"));
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|i| i.id.starts_with("sys-battery-low:") && i.title == "手机电量低 (18%)"));
    }

    #[test]
    fn alert_mode_switching_controls_popup_and_badge_and_persists_to_disk() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "123456".into(),
            source_app: "短信".into(),
            title: "验证码".into(),
            raw_text: "验证码 123456".into(),
            timestamp_ms: Local::now().timestamp_millis(),
            notification_key: "k1".into(),
        });

        app.sync_notification_center();
        assert_eq!(
            app.notification_center.alert_mode,
            NotificationAlertMode::PopupAndBadge
        );
        assert!(app.notification_active_popup().is_some());
        assert!(app.notification_unread_count() > 0);

        // Switch to BadgeOnly -> popup closes immediately, red dot badge count remains > 0
        app.apply_notification_center_action(NotificationCenterAction::SetAlertMode(
            NotificationAlertMode::BadgeOnly,
        ));
        assert_eq!(
            app.notification_center.alert_mode,
            NotificationAlertMode::BadgeOnly
        );
        assert!(app.notification_center.active_popup_id.is_none());
        assert!(app.notification_active_popup().is_none());
        assert!(app.notification_unread_count() > 0);

        // Switch to Muted -> no popup AND no red dot badge count (0)
        app.apply_notification_center_action(NotificationCenterAction::SetAlertMode(
            NotificationAlertMode::Muted,
        ));
        assert_eq!(
            app.notification_center.alert_mode,
            NotificationAlertMode::Muted
        );
        assert!(app.notification_active_popup().is_none());
        assert_eq!(app.notification_unread_count(), 0);
        // Raw category count inside drawer still tracks unread items
        assert!(app.notification_category_unread(NotificationCategory::PhoneComm) > 0);

        // Mark OTP read and verify persistence roundtrip from disk
        app.apply_notification_center_action(NotificationCenterAction::MarkRead(
            "otp:123456".into(),
        ));
        let reloaded = NotificationCenterState::load();
        assert_eq!(reloaded.alert_mode, NotificationAlertMode::Muted);
        assert!(reloaded.read_ids.contains("otp:123456"));
    }

    #[test]
    fn state_sync_and_trigger_item_update_read_and_execute_actions() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();

        app.push.snapshot.threads.push(ImThread {
            thread_id: "feishu:design".into(),
            package_name: "com.ss.android.lark".into(),
            app_name: "飞书".into(),
            title: "设计评审组".into(),
            unread_count: 2,
            last_message: "请确认弹窗与红点交互稿".into(),
            last_timestamp_ms: now_ms,
            ..Default::default()
        });
        app.apps.push(AppInfo {
            package: "com.tencent.mm".into(),
            label: "微信".into(),
            version_name: "1.0.0".into(),
            version_code: 1,
            enabled: true,
            launchable: true,
            ..Default::default()
        });
        app.sync_notification_center();

        assert!(app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "im-thread:feishu:design")
            .is_some_and(|i| i.unread));
        assert!(app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "app-update:com.tencent.mm")
            .is_some_and(|i| i.unread));

        // 1. Reading IM thread externally (`unread_count = 0`) automatically marks notification read
        app.push.snapshot.threads[0].unread_count = 0;
        app.sync_notification_center();
        assert!(app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "im-thread:feishu:design")
            .is_some_and(|i| !i.unread));

        // 2. Finishing app update (`!is_updatable`) automatically marks its notification read/resolved
        app.apps[0].version_name = "99.9.9".into();
        app.apps[0].version_code = u64::MAX;
        app.sync_notification_center();
        assert!(app
            .notification_center
            .read_ids
            .contains("app-update:com.tencent.mm"));
        assert!(app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "app-update:com.tencent.mm")
            .is_some_and(|i| !i.unread && i.status_tag.as_deref() == Some("已安装")));

        // 3. Clicking `TriggerItem` executes primary action, marks read, and closes popup
        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "998877".into(),
            source_app: "微信".into(),
            title: "登录验证码".into(),
            raw_text: "验证码 998877".into(),
            timestamp_ms: now_ms + 1_000,
            notification_key: "otp-998877".into(),
        });
        app.sync_notification_center();
        assert_eq!(
            app.notification_center.active_popup_id.as_deref(),
            Some("otp:998877")
        );

        app.apply_notification_center_action(NotificationCenterAction::TriggerItem(
            "otp:998877".into(),
        ));
        assert_eq!(
            app.notification_center.last_copied_text.as_deref(),
            Some("998877")
        );
        assert!(app.notification_center.active_popup_id.is_none());
        assert!(app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "otp:998877")
            .is_some_and(|i| !i.unread));

        // 4. `MarkAllRead` and `ClearReadAndDismissed`
        app.apply_notification_center_action(NotificationCenterAction::MarkAllRead);
        assert_eq!(app.notification_unread_count(), 0);
        app.apply_notification_center_action(NotificationCenterAction::ClearReadAndDismissed);
        assert!(app.notification_visible_items().is_empty());
    }

    #[test]
    fn edge_cases_rearriving_thread_corrupted_json_and_category_filtering() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();

        // 1. Empty inputs baseline (only the disconnected device notice is present)
        app.sync_notification_center();
        assert_eq!(
            app.notification_category_unread(NotificationCategory::PhoneApps),
            0
        );
        assert_eq!(
            app.notification_category_unread(NotificationCategory::PhoneComm),
            0
        );
        assert_eq!(
            app.notification_category_unread(NotificationCategory::AppUpdates),
            0
        );

        // 2. Re-arriving unread message on a previously read/dismissed thread re-opens it & triggers popup
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:team".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "项目群".into(),
            unread_count: 1,
            last_message: "第一条消息".into(),
            last_timestamp_ms: now_ms,
            ..Default::default()
        });
        app.sync_notification_center();
        assert_eq!(
            app.notification_center.active_popup_id.as_deref(),
            Some("im-thread:wx:team")
        );

        // Dismiss the item
        app.apply_notification_center_action(NotificationCenterAction::DismissItem(
            "im-thread:wx:team".into(),
        ));
        assert!(app.notification_center.active_popup_id.is_none());
        assert!(!app
            .notification_visible_items()
            .iter()
            .any(|i| i.id == "im-thread:wx:team"));

        // Now a newer message arrives in the same thread (`unread_count` increases or `last_timestamp_ms` advances)
        app.push.snapshot.threads[0].unread_count = 2;
        app.push.snapshot.threads[0].last_message = "第二条新消息".into();
        app.push.snapshot.threads[0].last_timestamp_ms = now_ms + 5_000;
        app.sync_notification_center();

        assert!(app
            .notification_visible_items()
            .iter()
            .any(|i| i.id == "im-thread:wx:team" && i.unread));
        assert_eq!(
            app.notification_center.active_popup_id.as_deref(),
            Some("im-thread:wx:team")
        );

        // 3. Category filtering via `SelectCategory`
        app.apply_notification_center_action(NotificationCenterAction::SelectCategory(
            NotificationCategory::PhoneApps,
        ));
        assert!(app
            .notification_visible_items()
            .iter()
            .all(|i| i.category == NotificationCategory::PhoneApps));

        // 4. Corrupted JSON on disk falls back gracefully without panicking
        let path = persistence_path();
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&path, b"{ corrupted json !!!").unwrap();
        let fallback = NotificationCenterState::load();
        assert_eq!(fallback.alert_mode, NotificationAlertMode::PopupAndBadge);
        assert!(fallback.read_ids.is_empty());
    }

    #[test]
    fn app_updates_summary_read_and_dismiss_syncs_with_underlying_app_update_items() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.selected = Some(Device {
            transport: "192.168.1.8:5555".into(),
            serial: "STEPX_TEST_SERIAL".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        app.apps.push(AppInfo {
            package: "com.tencent.mm".into(),
            label: "微信".into(),
            version_name: "1.0.0".into(),
            version_code: 1,
            enabled: true,
            launchable: true,
            ..Default::default()
        });
        app.apps.push(AppInfo {
            package: "com.ss.android.lark".into(),
            label: "飞书".into(),
            version_name: "1.0.0".into(),
            version_code: 1,
            enabled: true,
            launchable: true,
            ..Default::default()
        });

        app.sync_notification_center();
        let non_app_unread = app
            .notification_center
            .items
            .iter()
            .filter(|i| i.unread && i.category != NotificationCategory::AppUpdates)
            .count();
        assert!(
            app.notification_category_unread(NotificationCategory::AppUpdates) >= 2,
            "both app-updates-summary and individual app-update:* items should start unread"
        );

        // Marking `app-updates-summary` read must also mark all underlying `app-update:*` items read
        // so `AppUpdates` unread count drops to 0 without ghost unread items hidden under `All`.
        app.apply_notification_center_action(NotificationCenterAction::MarkRead(
            "app-updates-summary".into(),
        ));
        assert_eq!(
            app.notification_category_unread(NotificationCategory::AppUpdates),
            0
        );
        assert_eq!(app.notification_unread_count(), non_app_unread);
        assert!(app
            .notification_center
            .read_ids
            .contains("app-update:com.tencent.mm"));
        assert!(app
            .notification_center
            .read_ids
            .contains("app-update:com.ss.android.lark"));

        // Reset read_ids and verify `DismissItem("app-updates-summary")` also clears all underlying app-update unreads
        app.notification_center.read_ids.clear();
        app.notification_center.dismissed_ids.clear();
        app.sync_notification_center();
        assert!(app.notification_unread_count() > non_app_unread);

        app.apply_notification_center_action(NotificationCenterAction::DismissItem(
            "app-updates-summary".into(),
        ));
        assert_eq!(
            app.notification_category_unread(NotificationCategory::AppUpdates),
            0
        );
        assert_eq!(app.notification_unread_count(), non_app_unread);

        // Reset again and verify marking all individual `app-update:*` items read automatically marks `app-updates-summary` read
        app.notification_center.read_ids.clear();
        app.notification_center.dismissed_ids.clear();
        app.sync_notification_center();
        let update_ids: Vec<String> = app
            .notification_center
            .items
            .iter()
            .filter(|i| i.id.starts_with("app-update:"))
            .map(|i| i.id.clone())
            .collect();
        for uid in update_ids {
            app.apply_notification_center_action(NotificationCenterAction::MarkRead(uid));
        }
        assert!(app
            .notification_center
            .read_ids
            .contains("app-updates-summary"));
        assert_eq!(
            app.notification_category_unread(NotificationCategory::AppUpdates),
            0
        );
    }

    #[test]
    fn primary_actions_copy_text_update_app_open_im_sms_and_approve_gui_safety_transitions() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();

        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "654321".into(),
            source_app: "支付宝".into(),
            title: "安全校验码".into(),
            raw_text: "校验码 654321".into(),
            timestamp_ms: now_ms,
            notification_key: "otp-654321".into(),
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:arch".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "架构设计组".into(),
            unread_count: 2,
            last_message: "请看最新方案".into(),
            last_timestamp_ms: now_ms,
            ..Default::default()
        });
        app.sms.push(Message {
            provider_id: Some(77),
            address: "10000".into(),
            body: "电信宽带账单提醒".into(),
            date_ms: now_ms - 5_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        app.apps.push(AppInfo {
            package: "com.tencent.mm".into(),
            label: "微信".into(),
            version_name: "1.0.0".into(),
            version_code: 1,
            enabled: true,
            launchable: true,
            ..Default::default()
        });
        app.amoo_ui.gui_unconfirmed = true;
        app.amoo_ui.gui_paused = true;
        app.amoo_ui.gui_message = "待确认支付操作".into();

        app.sync_notification_center();

        // 1. CopyText updates clipboard history, desktop clip state, preview, and marks OTP read
        app.apply_notification_center_action(NotificationCenterAction::TriggerItem(
            "otp:654321".into(),
        ));
        assert_eq!(
            app.notification_center.last_copied_text.as_deref(),
            Some("654321")
        );
        assert_eq!(app.back_screen.last_synced_desktop_clip, "654321");
        assert_eq!(
            app.back_screen
                .clipboard_history
                .first()
                .map(String::as_str),
            Some("654321")
        );
        assert_eq!(app.push.clipboard_preview, "654321");

        // 2. OpenImThread selects thread, clears thread unread count, and navigates to Page::Sms
        app.selected_thread = Some("__new__".into());
        app.apply_notification_center_action(NotificationCenterAction::TriggerItem(
            "im-thread:wx:arch".into(),
        ));
        assert_eq!(app.page, Page::Sms);
        assert!(app.selected_thread.is_none());
        assert_eq!(app.push.selected_im_thread.as_deref(), Some("wx:arch"));
        assert_eq!(app.push.im_channel, ImChannel::WeChat);
        assert_eq!(app.push.snapshot.threads[0].unread_count, 0);

        // 3. OpenSms from SMS notification selects the sender thread and switches to ImChannel::Sms
        let sms_id = format!("sms:10000:{}", now_ms - 5_000);
        app.apply_notification_center_action(NotificationCenterAction::TriggerItem(sms_id));
        assert_eq!(app.page, Page::Sms);
        assert_eq!(app.push.im_channel, ImChannel::Sms);
        assert_eq!(app.selected_thread.as_deref(), Some("10000"));
        assert_eq!(app.fields[F_SMS_TO].text, "10000");
        assert!(app.sms[0].read);

        // 4. UpdateSingleApp navigates to AppStore Updatable category and selects the target package
        app.apply_notification_center_action(NotificationCenterAction::TriggerItem(
            "app-update:com.tencent.mm".into(),
        ));
        assert_eq!(app.page, Page::AppStore);
        assert_eq!(app.app_store.category, StoreCategory::Updatable);
        assert_eq!(
            app.app_store.selected_package.as_deref(),
            Some("com.tencent.mm")
        );

        // 5. ApproveGuiSafety(true) approves safety decision, unpauses GUI, and navigates to Page::Amoo
        app.apply_notification_center_action(NotificationCenterAction::TriggerItem(
            "gui-safety-approval".into(),
        ));
        assert_eq!(app.page, Page::Amoo);
        assert!(!app.amoo_ui.gui_unconfirmed);
        assert!(!app.amoo_ui.gui_paused);
        assert_eq!(app.amoo_safety_decision, SafetyDecision::Approved);
        assert!(app.status.contains("已批准"));
    }

    #[test]
    fn popup_queue_navigation_hover_timer_snooze_and_smart_suppression() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();

        // All eligible sources are queued; the single visible card prevents a burst of windows.
        app.amoo_ui.gui_unconfirmed = true;
        app.amoo_ui.gui_message = "确认支付".into();
        app.push.clipboard_preview = "https://stepx.ai/invite".into();
        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "889900".into(),
            source_app: "短信".into(),
            title: "10690000".into(),
            raw_text: "验证码 889900，5分钟内有效".into(),
            timestamp_ms: now_ms - 1_000,
            notification_key: "n_otp".into(),
        });
        app.express_parcels.push(ExpressParcelInfo {
            carrier: "菜鸟驿站".into(),
            tracking_no: "CN1088".into(),
            pickup_code: "9-2-1088".into(),
            station: "园区1号柜".into(),
            status: "待取件".into(),
            updated_at: "刚刚".into(),
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:team".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "产品研发群".into(),
            last_message: "请确认发版时间".into(),
            last_timestamp_ms: now_ms - 3_000,
            unread_count: 2,
            notification_key: "notif_wx_team".into(),
            can_remote_reply: true,
            ..Default::default()
        });
        app.call_logs.push(CallRecord {
            number: "13800138000".into(),
            name: "王总".into(),
            call_type: 3,
            date_ms: now_ms - 4_000,
            duration_sec: 0,
        });

        app.sync_notification_center();
        assert!(app.notification_center.initial_sync_seeded);
        assert_eq!(app.notification_center.popup_queue_len(), 5);
        assert_eq!(app.notification_center.popup_position_label(), "1 / 5");

        // Verify widget kinds and priorities across items
        let otp_item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "otp:889900")
            .expect("otp item");
        assert_eq!(otp_item.priority, NotificationPriority::P0Immediate);
        assert_eq!(otp_item.popup_widget.kind_name(), "otp_code");

        let express_item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id.starts_with("express:"))
            .expect("express item");
        assert_eq!(express_item.popup_widget.kind_name(), "express_pickup");

        let wx_item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "im-thread:wx:team")
            .expect("wx item");
        assert_eq!(wx_item.popup_widget.kind_name(), "inline_im_reply");

        // Verify explicit carousel navigation
        let first_id = app.notification_active_popup().unwrap().id.clone();
        app.apply_notification_center_action(NotificationCenterAction::PopupNext);
        assert_eq!(app.notification_center.popup_position_label(), "2 / 5");
        let second_id = app.notification_active_popup().unwrap().id.clone();
        assert_ne!(first_id, second_id);

        app.apply_notification_center_action(NotificationCenterAction::PopupPrev);
        assert_eq!(app.notification_center.popup_position_label(), "1 / 5");
        assert_eq!(app.notification_active_popup().unwrap().id, first_id);

        // Hover and elapsed time never dismiss the card
        let rem_before = app.notification_center.popup_remaining_ms;
        app.apply_notification_center_action(NotificationCenterAction::PopupHoverChanged(true));
        assert!(!app.tick_notification_popup_timer(2_000));
        assert_eq!(app.notification_center.popup_remaining_ms, rem_before);

        // Time passing never dismisses a notification or moves the user to another chat.
        app.apply_notification_center_action(NotificationCenterAction::PopupHoverChanged(false));
        let total = app.notification_center.popup_remaining_ms;
        assert!(!app.tick_notification_popup_timer(total + 100));
        assert_eq!(app.notification_center.popup_queue_len(), 5);
        assert_eq!(app.notification_active_popup().unwrap().id, first_id);
        // The persistent card remains unread until explicitly handled.
        let timed_out_item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == first_id)
            .unwrap();
        assert!(timed_out_item.unread);

        // Verify SnoozePopups suppresses active_popup and popup_queue
        app.apply_notification_center_action(NotificationCenterAction::SnoozePopups {
            duration_ms: 3_600_000,
        });
        assert!(app.notification_active_popup().is_none());
        assert_eq!(app.notification_center.popup_queue_len(), 0);

        app.apply_notification_center_action(NotificationCenterAction::ClearSnooze);
        assert!(app.notification_active_popup().is_some());

        // Verify per-app PopupMode::Silent suppression while other apps still pop up,
        // and that phone_in_hand no longer suppresses IM popups when mode is FullReply/Banner.
        app.push.snapshot.screen_interactive = true;
        app.push.snapshot.keyguard_locked = false;
        app.push.snapshot.human_touch_recent = true;
        app.notification_center
            .popup_shown_ids
            .remove("im-thread:wx:team");
        app.notification_center.popup_shown_ids.remove("otp:889900");
        // This part exercises explicit re-preview under per-app preferences.
        // Automatic delivery receipts must continue suppressing old alerts.
        app.notification_center.delivered_events.retain(|key| !key.contains("|otp:889900|"));
        app.sync_notification_center();
        assert!(app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id == "im-thread:wx:team"));

        app.push.prefs.set_app_popup_mode(
            "com.tencent.mm",
            Some(stepx_companion::desktop_notify::PopupMode::Silent),
        );
        app.sync_notification_center();
        assert!(app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id == "otp:889900"));
        assert!(!app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id == "im-thread:wx:team"));
    }

    #[test]
    fn offline_popup_replies_never_create_sent_messages_or_dismiss_sources() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = false;
        app.selected = None;
        app.push.snapshot.threads = vec![ImThread {
            thread_id: "wx:offline".into(),
            package_name: "com.tencent.mm".into(),
            title: "offline thread".into(),
            last_message: "hello".into(),
            unread_count: 1,
            last_timestamp_ms: Local::now().timestamp_millis(),
            can_remote_reply: true,
            ..Default::default()
        }];
        app.sync_notification_center();
        let before_sms = app.sms.len();
        let before_reads = app.notification_center.read_ids.clone();
        for id in [
            "im-thread:wx:offline",
            "sms:10086:1",
            "push-notif:expired",
            "unsupported",
        ] {
            app.submit_popup_quick_reply(id, "must stay unsent");
            assert!(app.error && app.status.contains("未发送"));
            assert_eq!(app.sms.len(), before_sms);
            assert_eq!(app.notification_center.read_ids, before_reads);
            assert!(app
                .push
                .snapshot
                .threads
                .iter()
                .flat_map(|t| &t.messages)
                .all(|m| !m.outgoing));
            assert!(app.push.active_reply_job.is_none() && app.push.reply_queue.is_empty());
        }
    }

    #[test]
    fn popup_without_background_handle_preserves_draft_and_unread_message() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = false;
        app.selected = Some(Device {
            serial: "not-a-real-device".into(),
            ..Default::default()
        });
        app.push.snapshot.threads = vec![ImThread {
            thread_id: "wx:no-handle".into(),
            package_name: "com.tencent.mm".into(),
            title: "某个会话".into(),
            last_message: "hello".into(),
            unread_count: 1,
            last_timestamp_ms: Local::now().timestamp_millis(),
            can_ui_reply: true,
            ..Default::default()
        }];
        app.sync_notification_center();
        let id = "im-thread:wx:no-handle";
        assert!(matches!(
            app.notification_center
                .items
                .iter()
                .find(|i| i.id == id)
                .unwrap()
                .popup_widget,
            NotificationPopupWidget::InlineImReply {
                can_reply: false,
                ..
            }
        ));
        let reads = app.notification_center.read_ids.clone();
        app.fields[super::super::F_TOAST_REPLY].set("保留这份回复草稿");
        app.apply_notification_center_action(NotificationCenterAction::SubmitPopupReply(id.into()));
        assert!(app.error && app.status.contains("未发送"));
        assert_eq!(
            app.fields[super::super::F_TOAST_REPLY].text,
            "保留这份回复草稿"
        );
        assert_eq!(app.notification_center.read_ids, reads);
        assert!(app.push.active_reply_job.is_none() && app.push.reply_queue.is_empty());
    }

    #[test]
    fn expired_popup_target_does_not_guess_a_wechat_recipient() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = false;
        app.selected = Some(Device {
            serial: "not-a-real-device".into(),
            ..Default::default()
        });
        app.notification_center.items.clear();
        for id in ["im-thread:unknown", "push-notif:gone", "sms:10086:1"] {
            app.submit_popup_quick_reply(id, "must stay unsent");
            assert!(app.error && app.status.contains("失效"));
            assert!(app.push.active_reply_job.is_none() && app.push.reply_queue.is_empty());
        }
    }

    #[test]
    fn startup_history_and_late_sync_waves_keep_badges_without_replaying_popups() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = false;
        let now = Local::now().timestamp_millis();
        app.notification_center.popup_session_started_ms = now - 1_000;
        app.push.snapshot.threads = vec![ImThread {
            thread_id: "wx:old".into(),
            package_name: "com.tencent.mm".into(),
            title: "old conversation".into(),
            last_message: "yesterday".into(),
            unread_count: 1,
            last_timestamp_ms: now - 86_400_000,
            ..Default::default()
        }];
        app.sms = vec![Message {
            provider_id: Some(1),
            address: "10086".into(),
            body: "older message".into(),
            date_ms: now - 86_400_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        }];
        app.sync_notification_center();
        assert!(!app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id == "im-thread:wx:old" || id.starts_with("sms:10086:")));
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|item| item.id == "im-thread:wx:old" && item.unread));
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|item| item.id.starts_with("sms:10086:") && item.unread));
        app.call_logs.push(CallRecord {
            number: "10010".into(),
            name: "old missed call".into(),
            call_type: 3,
            date_ms: now - 86_400_000,
            duration_sec: 0,
        });
        app.sync_notification_center();
        assert!(!app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id.starts_with("missed-call:")));
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:new".into(),
            package_name: "com.tencent.mm".into(),
            title: "new conversation".into(),
            last_message: "just arrived".into(),
            unread_count: 1,
            last_timestamp_ms: now,
            ..Default::default()
        });
        app.sync_notification_center();
        assert!(app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id == "im-thread:wx:new"));
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:future".into(),
            package_name: "com.tencent.mm".into(),
            title: "future timestamp".into(),
            last_message: "unverified device clock".into(),
            unread_count: 1,
            last_timestamp_ms: now + 3_600_000,
            ..Default::default()
        });
        app.sync_notification_center();
        assert!(!app
            .notification_center
            .popup_queue
            .iter()
            .any(|id| id == "im-thread:wx:future"));
        assert!(app
            .notification_center
            .items
            .iter()
            .any(|item| item.id == "im-thread:wx:future" && item.unread));
        assert!(
            app.notification_center
                .is_current_popup_time(now + 7_000, now),
            "Allow the small clock offset measured on the connected phone"
        );
        assert!(!app
            .notification_center
            .read_ids
            .contains("im-thread:wx:old"));
        assert!(!app
            .notification_center
            .read_ids
            .iter()
            .any(|id| id.starts_with("sms:10086:")));
    }

    #[test]
    fn popup_quick_reply_and_focus_main_window_policy() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = true; // The success path is an explicit demo; no real phone is used.
        let now_ms = Local::now().timestamp_millis();

        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "112233".into(),
            source_app: "短信".into(),
            title: "10690000".into(),
            raw_text: "验证码 112233".into(),
            timestamp_ms: now_ms - 1_000,
            notification_key: "n_otp_focus".into(),
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:quick".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "李四".into(),
            last_message: "在吗？".into(),
            last_timestamp_ms: now_ms - 500,
            unread_count: 1,
            notification_key: "notif_wx_quick".into(),
            can_remote_reply: true,
            ..Default::default()
        });
        app.sync_notification_center();

        // CopyText primary action should NOT focus the main window
        assert!(!app.item_action_focuses_main_window("otp:112233"));
        // OpenImThread primary action SHOULD focus the main window
        assert!(app.item_action_focuses_main_window("im-thread:wx:quick"));

        // Quick reply from popup records outgoing message, marks item read, and sets feedback
        app.apply_notification_center_action(NotificationCenterAction::PopupQuickReply {
            item_id: "im-thread:wx:quick".into(),
            text: "收到，马上处理".into(),
        });
        assert!(app
            .notification_center
            .read_ids
            .contains("im-thread:wx:quick"));
        assert!(app
            .notification_center
            .active_feedback(Local::now().timestamp_millis())
            .is_some());
        assert_eq!(
            app.push.snapshot.threads[0]
                .messages
                .last()
                .map(|m| m.text.as_str()),
            Some("收到，马上处理")
        );
    }

    #[test]
    fn surfaces_dingdong_grocery_boss_zhipin_and_direct_at_mentions_with_ai_tags() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();

        app.sms.push(Message {
            provider_id: Some(88),
            address: "10690011".into(),
            body: "【叮咚买菜】您在叮咚买菜购买的商品已代收（家门口），请及时取走。".into(),
            date_ms: now_ms - 30_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:at-me".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "核心研发群".into(),
            unread_count: 1,
            last_sender: "王工".into(),
            last_message: "[有人@我] 王工: 请确认一下今天下午的评审时间".into(),
            last_timestamp_ms: now_ms - 15_000,
            ..Default::default()
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "boss:hr1".into(),
            package_name: "com.hpbr.bosszhipin".into(),
            app_name: "BOSS直聘".into(),
            title: "李HR · 某AI公司".into(),
            unread_count: 1,
            last_sender: "李HR".into(),
            last_message: "您好，对您的简历很感兴趣，方便沟通吗？".into(),
            last_timestamp_ms: now_ms - 20_000,
            ..Default::default()
        });

        app.sync_notification_center();

        let sms_item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id.starts_with("sms:10690011:"))
            .expect("Dingdong SMS item");
        assert!(sms_item.unread);
        assert_eq!(sms_item.source_badge, "叮咚买菜");
        assert_eq!(sms_item.status_tag.as_deref(), Some("AI 买菜送达"));

        let at_item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "im-thread:wx:at-me")
            .expect("@mention thread item");
        assert!(at_item.unread);
        assert_eq!(at_item.status_tag.as_deref(), Some("AI @提及我"));

        for ordinary in ["@所有人 今天开会", "@李四 请看一下"] {
            app.push.snapshot.threads[0].last_message = ordinary.into();
            app.sync_notification_center();
            let item = app
                .notification_center
                .items
                .iter()
                .find(|i| i.id == "im-thread:wx:at-me")
                .unwrap();
            assert_ne!(item.status_tag.as_deref(), Some("AI @提及我"));
        }

        let boss_item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "im-thread:boss:hr1")
            .expect("BOSS Zhipin thread item");
        assert!(boss_item.unread);
        assert_eq!(boss_item.source_badge, "BOSS直聘");
        assert_eq!(boss_item.status_tag.as_deref(), Some("AI 招聘消息"));

        // Verify that once read on phone (`read = true`, `unread_count = 0`), items are NOT forced back to unread!
        app.sms[0].read = true;
        app.push.snapshot.threads[0].unread_count = 0;
        app.push.snapshot.threads[1].unread_count = 0;
        app.sync_notification_center();
        assert_eq!(app.notification_unread_count(), 0);
    }

    #[test]
    fn bursts_keep_current_conversation_and_isolate_reply_drafts() {
        let (_dir, _guard, mut app) = setup_test_companion();
        app.demo_mode = false;
        app.notification_center.connect_baseline_enabled = false;
        let now = Local::now().timestamp_millis();
        let thread = |i: usize| ImThread {
            thread_id: format!("burst:{i}"),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: format!("联系人{i}"),
            unread_count: 1,
            last_sender: format!("联系人{i}"),
            last_message: "新消息".into(),
            last_timestamp_ms: now + i as i64,
            can_remote_reply: true,
            ..Default::default()
        };
        app.push.snapshot.threads.push(thread(0));
        app.sync_notification_center();
        let original = app.notification_active_popup().unwrap().id.clone();
        app.fields[super::super::F_TOAST_REPLY].set("只发给第一位的草稿");
        for i in 1..13 {
            app.push.snapshot.threads.push(thread(i));
        }
        app.sync_notification_center();
        assert_eq!(app.notification_active_popup().unwrap().id, original);
        assert_eq!(
            app.notification_center.popup_queue_len(),
            13,
            "burst items remain queued"
        );
        assert_eq!(
            app.fields[super::super::F_TOAST_REPLY].text,
            "只发给第一位的草稿"
        );
        app.push.snapshot.threads[0].last_message = "同一会话第二条".into();
        app.push.snapshot.threads[0].unread_count = 2;
        app.push.snapshot.threads[0].last_timestamp_ms = now + 20;
        app.sync_notification_center();
        assert_eq!(
            app.notification_center.popup_queue_len(),
            13,
            "same conversation updates one card"
        );
        assert_eq!(app.notification_active_popup().unwrap().id, original);
        app.apply_notification_center_action(NotificationCenterAction::PopupNext);
        assert!(app.fields[super::super::F_TOAST_REPLY].text.is_empty());
        app.fields[super::super::F_TOAST_REPLY].set("第二位的草稿");
        app.apply_notification_center_action(NotificationCenterAction::SubmitPopupReply(
            original.clone(),
        ));
        assert_eq!(app.fields[super::super::F_TOAST_REPLY].text, "第二位的草稿");
        assert!(app.push.reply_queue.is_empty() && app.push.active_reply_job.is_none());
        app.apply_notification_center_action(NotificationCenterAction::PopupPrev);
        assert_eq!(
            app.fields[super::super::F_TOAST_REPLY].text,
            "只发给第一位的草稿"
        );
        assert!(!app.tick_notification_popup_timer(86_400_000));
        assert_eq!(app.notification_active_popup().unwrap().id, original);
        app.apply_notification_center_action(NotificationCenterAction::DismissPopup);
        assert!(!app.notification_center.read_ids.contains(&original));
        assert_eq!(app.fields[super::super::F_TOAST_REPLY].text, "第二位的草稿");
        app.push.prefs.set_app_popup_mode(
            "com.tencent.mm",
            Some(stepx_companion::desktop_notify::PopupMode::Silent),
        );
        app.sync_notification_center();
        assert!(app.notification_active_popup().is_none());
    }

    #[test]
    fn repeated_sync_keeps_new_phone_popup_unread_until_read_on_phone() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now = Local::now().timestamp_millis();
        app.demo_mode = false;
        app.push.loaded = true;
        app.push.prefs.scope = stepx_companion::desktop_notify::AlertScope::All;
        app.notification_center.connect_baseline_enabled = true;
        app.selected = Some(Device {
            transport: "192.168.1.8:5555".into(),
            serial: "POPUP_PHONE".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        let make_notification = |key: &str, ts| stepx_companion::push::PushNotification {
            key: key.into(),
            package_name: "com.example.messages".into(),
            app_name: "消息".into(),
            title: "新讨论".into(),
            text: "这是一条新消息".into(),
            post_time_ms: ts,
            active: true,
            is_clearable: true,
            ..Default::default()
        };
        app.push
            .snapshot
            .notifications
            .push(make_notification("old", now - 20_000));
        app.sync_notification_center();
        assert!(
            app.notification_active_popup().is_none(),
            "initial history stays quiet"
        );
        app.push
            .snapshot
            .notifications
            .push(make_notification("new", now));
        for _ in 0..3 {
            app.sync_notification_center();
            assert_eq!(
                app.notification_active_popup().map(|i| i.id.as_str()),
                Some("push-notif:new")
            );
            assert!(
                !app.notification_center.read_ids.contains("push-notif:new"),
                "background refresh must not mark a new notification as read"
            );
        }
        app.push.snapshot.notifications[1].read = true;
        app.sync_notification_center();
        assert!(app.notification_active_popup().is_none());
    }

    #[test]
    fn disconnect_reconnect_seeds_baseline_and_suppresses_false_unreads() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();
        app.demo_mode = false;
        app.notification_center.connect_baseline_enabled = true;

        // Connect phone and load historical snapshot across all domains
        app.selected = Some(Device {
            transport: "192.168.1.8:5555".into(),
            serial: "STEPX_RECONNECT_SERIAL".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:dingdong".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "叮咚买菜".into(),
            unread_count: 0,
            last_message: "您的订单正在配送中".into(),
            last_timestamp_ms: 0,
            ..Default::default()
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:hist".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "历史群聊".into(),
            unread_count: 2,
            last_message: "连接前的旧消息".into(),
            last_timestamp_ms: now_ms - 600_000,
            ..Default::default()
        });
        app.sms.push(Message {
            provider_id: Some(1),
            address: "10086".into(),
            body: "历史未读短信".into(),
            date_ms: now_ms - 500_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        app.call_logs.push(CallRecord {
            number: "13800138000".into(),
            name: "张三".into(),
            call_type: 3,
            date_ms: now_ms - 400_000,
            duration_sec: 0,
        });
        app.apps.push(AppInfo {
            package: "com.tencent.mm".into(),
            label: "微信".into(),
            version_name: "1.0.0".into(),
            version_code: 1,
            enabled: true,
            launchable: true,
            ..Default::default()
        });

        app.sync_notification_center();
        assert_eq!(
            app.notification_unread_count(),
            0,
            "initial connection snapshot must be baseline-seeded with 0 unread notifications"
        );

        // Simulate disconnect (apps/selected cleared temporarily) then reconnect
        app.selected = None;
        app.apps.clear();
        app.sync_notification_center();
        assert_eq!(app.notification_unread_count(), 0);

        app.selected = Some(Device {
            transport: "192.168.1.8:5555".into(),
            serial: "STEPX_RECONNECT_SERIAL".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        app.apps.push(AppInfo {
            package: "com.tencent.mm".into(),
            label: "微信".into(),
            version_name: "1.0.0".into(),
            version_code: 1,
            enabled: true,
            launchable: true,
            ..Default::default()
        });
        app.sync_notification_center();
        assert_eq!(
            app.notification_unread_count(),
            0,
            "reconnecting phone with same historical snapshot must have 0 unread notifications"
        );

        // Now a genuinely NEW message arrives after connection -> prompts as unread!
        app.push.snapshot.threads[1].unread_count = 3;
        app.push.snapshot.threads[1].last_message = "连接后的新消息".into();
        app.push.snapshot.threads[1].last_timestamp_ms = now_ms + 5_000;
        app.sync_notification_center();
        assert_eq!(app.notification_unread_count(), 1);
    }

    #[test]
    fn non_otp_sms_and_express_notifications_use_accurate_scene_widgets_and_replay_works() {
        let (_dir, _guard, mut app) = setup_test_companion();
        let now_ms = Local::now().timestamp_millis();

        // 1. Dingdong delivery SMS -> ExpressPickup (not empty OtpCode)
        app.sms.push(Message {
            provider_id: Some(101),
            address: "1068393923451722026".into(),
            body: "【叮咚买菜】您在叮咚买菜购买的商品已代收（家门口），请及时取走。".into(),
            date_ms: now_ms - 40_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        // 2. Express pickup SMS -> ExpressPickup with real pickup code
        app.sms.push(Message {
            provider_id: Some(102),
            address: "106881517503453".into(),
            body: "【丰巢】凭取件码79983898至金隅万科城丰巢快递柜取包裹。".into(),
            date_ms: now_ms - 30_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        // 3. Regular carrier SMS -> InlineImReply (not empty OtpCode)
        app.sms.push(Message {
            provider_id: Some(103),
            address: "10086".into(),
            body: "【中国移动】尊敬的客户，您本月套餐流量余量充足。".into(),
            date_ms: now_ms - 20_000,
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        // 4. Express push notification -> ExpressPickup
        app.push
            .snapshot
            .notifications
            .push(stepx_companion::push::PushNotification {
                key: "notif-cainiao-1".into(),
                package_name: "com.cainiao.wireless".into(),
                app_name: "菜鸟".into(),
                title: "菜鸟驿站取件通知".into(),
                text: "您的顺丰包裹已到站，请凭取件码 4-2-1086 前往领取。".into(),
                post_time_ms: now_ms - 10_000,
                active: true,
                is_clearable: true,
                ..Default::default()
            });

        app.sync_notification_center();

        for item in &app.notification_center.items {
            if let NotificationPopupWidget::OtpCode { code, .. } = &item.popup_widget {
                assert!(
                    !code.trim().is_empty(),
                    "Item {} must never have an empty OtpCode widget",
                    item.id
                );
            }
        }

        let dingdong = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id.starts_with("sms:1068393923451722026:"))
            .expect("Dingdong item");
        assert!(matches!(
            dingdong.popup_widget,
            NotificationPopupWidget::ExpressPickup { .. }
        ));
        assert!(dingdong.title.contains("叮咚买菜"));

        let fengchao = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id.starts_with("sms:106881517503453:"))
            .expect("Fengchao item");
        assert!(matches!(
            &fengchao.popup_widget,
            NotificationPopupWidget::ExpressPickup { pickup_code, .. } if pickup_code == "79983898"
        ));

        let regular_sms = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id.starts_with("sms:10086:"))
            .expect("10086 item");
        assert!(matches!(
            regular_sms.popup_widget,
            NotificationPopupWidget::InlineImReply { .. }
        ));

        let cainiao_notif = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "push-notif:notif-cainiao-1")
            .expect("Cainiao notification item");
        assert!(matches!(
            &cainiao_notif.popup_widget,
            NotificationPopupWidget::ExpressPickup { pickup_code, .. } if pickup_code == "4-2-1086"
        ));

        // Clear popup queue and test PreviewItemPopup & ReplayLatestPopups
        let target_id = dingdong.id.clone();
        app.notification_center.popup_queue.clear();
        app.notification_center.active_popup_id = None;
        assert!(app.notification_active_popup().is_none());

        app.apply_notification_center_action(NotificationCenterAction::PreviewItemPopup(
            target_id.clone(),
        ));
        assert_eq!(
            app.notification_active_popup().map(|i| i.id.as_str()),
            Some(target_id.as_str())
        );

        app.apply_notification_center_action(NotificationCenterAction::ReplayLatestPopups);
        assert!(app.notification_active_popup().is_some());
        assert!(app.notification_center.popup_queue.len() >= 3);
    }

    #[test]
    fn multi_notices_rolling_clipboard_and_cached_history_persist_across_restarts() {
        let (_dir, _guard, mut app) = setup_test_companion();

        // 1. Multiple interaction notices are surfaced instead of only `.first()`
        app.record_notice("安装包已同步", "app-v1.apk 安装完成");
        app.record_notice("截图已保存", "screenshot_01.png 已导出到电脑");
        app.record_notice("文件已移入回收站", "2 项手机文件已移入回收站");

        // 2. Rolling clipboard cache retains consecutive phone clipboard updates
        app.back_screen.stepos_suite.phone_clipboard = "https://stepx.ai/doc-1".into();
        app.sync_notification_center();
        app.back_screen.stepos_suite.phone_clipboard = "https://stepx.ai/doc-2".into();
        app.sync_notification_center();

        let notice_count = app
            .notification_center
            .items
            .iter()
            .filter(|i| i.id.starts_with("interaction-notice:"))
            .count();
        assert_eq!(notice_count, 3);

        let clip_count = app
            .notification_center
            .items
            .iter()
            .filter(|i| i.id.starts_with("phone-clipboard:"))
            .count();
        assert_eq!(clip_count, 2);

        // 3. Save and reload from disk preserves cached_history without re-triggering popups
        app.notification_center.save();
        let reloaded = NotificationCenterState::load();
        assert!(
            reloaded
                .cached_history
                .iter()
                .filter(|i| i.id.starts_with("interaction-notice:"))
                .count()
                == 3
        );
        assert!(
            reloaded
                .cached_history
                .iter()
                .filter(|i| i.id.starts_with("phone-clipboard:"))
                .count()
                == 2
        );
        for item in &reloaded.cached_history {
            // New versions acknowledge an exact event rather than permanently
            // suppressing the whole conversation ID. Legacy IDs remain valid.
            assert!(reloaded.popup_shown_ids.contains(&item.id)
                || reloaded.delivered_events.contains(&app.popup_delivery_key(item)));
        }
        app.notification_center = reloaded;
        app.sync_notification_center();
        assert!(app.notification_center.popup_queue.is_empty(),
            "reloading cached history must not replay delivered events");
    }
}
