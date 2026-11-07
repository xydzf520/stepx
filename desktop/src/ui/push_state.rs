//! State, actions, and non-blocking fast-lane orchestration for:
//! 1. Phone Push Notification Center (`Page::Notifications`)
//! 2. Mainstream IM Chat Hub & Direct Reply (`Page::Sms`)
//! 3. Compact Modes (`ShellMode::MiniAi` & `ShellMode::MiniNotify`)
//! 4. Cross-platform Interactive Desktop Toast Popups (`push_toast.rs`)

use super::*;
use facloud_ui::clipboard::ClipboardContent;
use std::collections::HashSet;
use stepx_companion::{
    desktop_notify::{self, AlertScope, DesktopToastItem, NotifyPreferences, PopupMode},
    push::{
        self, ImChannel, ImMessage, ImThread, PushCategory, PushSnapshot, ReplyMode, ReplyReceipt,
    },
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum ShellMode {
    #[default]
    Full,
    MiniAi,
    MiniNotify,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum PushCenterTab {
    #[default]
    ImChat,
    Notifications,
    Settings,
}

impl ShellMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Full => "完整工作台",
            Self::MiniAi => "Amoo 对话",
            Self::MiniNotify => "手机通知",
        }
    }
}

#[derive(Clone, Debug)]
pub(super) enum PushAction {
    Refresh,
    EnsurePermission,
    SelectCategory(PushCategory),
    SelectAppFilter(Option<String>),
    ToggleCollapseOngoing,
    SelectNotification(String),
    DismissNotification(String),
    DismissAll,
    OpenOnPhone(String),
    CopyOtp(String),
    OpenUrl(String),
    ReplyInline {
        key: String,
        thread_id: String,
        package_name: String,
        title: String,
    },
    QuickReplyPhrase {
        thread_id: String,
        package_name: String,
        title: String,
        text: String,
    },
    AiDraftReply {
        thread_id: String,
        package_name: String,
        title: String,
    },
    SelectImChannel(ImChannel),
    SelectImThread(String),
    ImHistoryPage {
        thread_id: String,
        first: Option<String>,
    },
    SendImReply,
    SetReplyMode(ReplyMode),
    SyncActivePhoneChatWindow,
    SetAlertScope(AlertScope),
    SetPopupMode(PopupMode),
    SetAppPopupMode {
        package_name: String,
        mode: Option<PopupMode>,
    },
    TogglePrivacyMask,
    ToggleAutoCopyOtp,
    DismissToast(String),
    DismissAllToasts,
    ToastSendReply(String),
    ToastOpenInMain(String),
    ToastOpenInMini(String),
    SetShellMode(ShellMode),
    CompactHistory,
    CompactInbox,
    CompactSystemNotices,
    TogglePinWindow,
    AskClipboard(u8),
    CaptureClipboardPreview,
    OpenMiniAiWindow,
    OpenMiniNotifyWindow,
    LoadDemoPush,
    SetCallFilter(u8),
    SetNotifCategory(u8),
    FillSmsTemplate(String),
    SelectCalendarQuickTime(String),
    OpenNewAmooChat,
    MarkAllNotificationsRead,
    ResetNotificationFilters,
    MarkAllMessagesRead,
    QuoteReply {
        is_im: bool,
        sender: String,
        text: String,
    },
    ClearQuoteReply,
    DeleteImThread {
        thread_id: String,
        package_name: String,
        title: String,
    },
    DeleteSmsThread {
        address: String,
        name: String,
    },
    TogglePinTarget(String),
    ToggleThreadRead {
        is_im: bool,
        target_id: String,
    },
    ClearThreadMessages {
        is_im: bool,
        target_id: String,
        package_name: String,
        title: String,
    },
    DeleteMessage {
        thread_id: String,
        package_name: String,
        msg_id: String,
        text: String,
    },
    DeleteSmsMessage {
        address: String,
        date_ms: i64,
        body: String,
    },
    DismissPickupCode(String),
}

#[derive(Clone, Debug)]
pub(super) struct QueuedReplyJob {
    pub client_msg_id: String,
    pub device_serial: String,
    pub thread_id: String,
    pub package_name: String,
    pub title: String,
    pub text: String,
    pub mode: stepx_companion::push::ReplyMode,
}

pub(super) enum PushOutcome {
    Snapshot(String, Result<PushSnapshot, String>),
    ReplyFinished(String, String, String, Result<ReplyReceipt, String>),
    ActiveWindowSynced(String, Result<Option<ImThread>, String>),
    ThreadDetailFetched(String, ImThread),
    NotificationDismissed(String, String, Result<(), String>),
    AllDismissed(String, Result<usize, String>),
    PermissionEnsured(String, Result<bool, String>),
}

#[allow(dead_code)]
pub(super) type SmsImUserPrefs = NotifyPreferences;
#[allow(dead_code)]
pub(super) type PushUi = PushUiState;
#[allow(dead_code)]
pub(super) type PushCenterState = PushUiState;

pub(super) struct PushUiState {
    pub snapshot: PushSnapshot,
    pub loaded: bool,
    pub syncing: bool,
    pub sync_started: Instant,
    snapshot_in_flight_full: bool,
    full_refresh_requested: bool,
    pub error: Option<String>,
    pub category: PushCategory,
    pub notif_category: u8,
    pub call_filter: u8,
    pub app_filter: Option<String>,
    pub collapse_ongoing: bool,
    pub selected_key: Option<String>,
    pub replying_key: Option<String>,
    pub im_channel: ImChannel,
    pub selected_im_thread: Option<String>,
    pub message_window: std::cell::RefCell<super::message_window::MessageWindow>,
    pub im_drafts: HashMap<String, String>,
    pub reply_quote: Option<(String, String, bool)>,
    pub reply_mode: ReplyMode,
    pub sending_thread: Option<String>,
    pub last_reply_status: Option<String>,
    pub reply_error: Option<String>,
    pub reply_in_flight: bool,
    pub reply_queue: std::collections::VecDeque<QueuedReplyJob>,
    pub active_reply_job: Option<QueuedReplyJob>,
    pub next_client_msg_seq: u64,
    pub tab: PushCenterTab,
    pub prefs: NotifyPreferences,
    pub parcel_receipts: stepx_companion::parcel_state::CompletionIndex,
    pub active_toasts: Vec<DesktopToastItem>,
    pub seen_toast_keys: HashSet<String>,
    pub shell_mode: ShellMode,
    pub pin_window: bool,
    pub clipboard_preview: String,
    pub expanded_mini_card: Option<String>,
    pub last_sync: Instant,
    pub outcome_tx: mpsc::Sender<PushOutcome>,
    pub outcome_rx: mpsc::Receiver<PushOutcome>,
}

pub(super) const MAX_SEEN_TOAST_KEYS: usize = 2048;
const MAX_IM_DRAFTS: usize = 128;

impl PushUiState {
    fn begin_snapshot_read(&mut self, full: bool) -> bool {
        if self.syncing {
            // Repeated clicks join a full read. If an incremental read is in
            // flight, remember one explicit full read to run after its receipt.
            self.full_refresh_requested |= full && !self.snapshot_in_flight_full;
            return false;
        }
        self.syncing = true;
        self.snapshot_in_flight_full = full;
        self.full_refresh_requested = false;
        self.sync_started = Instant::now();
        self.last_sync = self.sync_started;
        true
    }

    fn snapshot_active_toast_keys(next: &PushSnapshot) -> HashSet<String> {
        let mut active_keys = HashSet::with_capacity(
            next.notifications.len() * 2 + next.threads.len() * 2 + next.otp_banners.len(),
        );
        for n in &next.notifications {
            if !n.key.is_empty() {
                active_keys.insert(n.key.clone());
            }
            active_keys.insert(format!("seq:{}", n.seq_id));
        }
        for t in &next.threads {
            if !t.thread_id.is_empty() {
                active_keys.insert(t.thread_id.clone());
            }
            if !t.notification_key.is_empty() {
                active_keys.insert(t.notification_key.clone());
            }
        }
        for b in &next.otp_banners {
            if !b.code.is_empty() {
                active_keys.insert(b.code.clone());
            }
        }
        active_keys
    }

    /// Bound `seen_toast_keys` at `MAX_SEEN_TOAST_KEYS` (2048) entries by retaining
    /// keys still present in the current `PushSnapshot` and evicting stale keys when full.
    pub(super) fn cap_seen_toast_keys(&mut self, next: &PushSnapshot) {
        if self.seen_toast_keys.len() < MAX_SEEN_TOAST_KEYS {
            return;
        }
        let active_keys = Self::snapshot_active_toast_keys(next);
        if !active_keys.is_empty() {
            self.seen_toast_keys.retain(|k| active_keys.contains(k));
        }
        while self.seen_toast_keys.len() > MAX_SEEN_TOAST_KEYS {
            let victim = self
                .seen_toast_keys
                .iter()
                .find(|k| !active_keys.contains(*k))
                .or_else(|| self.seen_toast_keys.iter().next())
                .cloned();
            let Some(victim) = victim else {
                break;
            };
            self.seen_toast_keys.remove(&victim);
        }
    }

    pub(super) fn enforce_seen_toast_keys_bound(&mut self, next: &PushSnapshot) {
        if self.seen_toast_keys.len() > MAX_SEEN_TOAST_KEYS {
            self.cap_seen_toast_keys(next);
        }
    }

    pub(super) fn insert_seen_toast_key(&mut self, key: String, next: &PushSnapshot) {
        if !self.seen_toast_keys.contains(&key) && self.seen_toast_keys.len() >= MAX_SEEN_TOAST_KEYS
        {
            let active_keys = Self::snapshot_active_toast_keys(next);
            while self.seen_toast_keys.len() >= MAX_SEEN_TOAST_KEYS {
                let victim = self
                    .seen_toast_keys
                    .iter()
                    .find(|k| !active_keys.contains(*k))
                    .or_else(|| self.seen_toast_keys.iter().next())
                    .cloned();
                let Some(victim) = victim else {
                    break;
                };
                self.seen_toast_keys.remove(&victim);
            }
        }
        self.seen_toast_keys.insert(key);
    }
}

impl Default for PushUiState {
    fn default() -> Self {
        let (outcome_tx, outcome_rx) = mpsc::channel();
        Self {
            snapshot: PushSnapshot::default(),
            parcel_receipts: Default::default(),
            loaded: false,
            syncing: false,
            sync_started: Instant::now(),
            snapshot_in_flight_full: false,
            full_refresh_requested: false,
            error: None,
            category: PushCategory::All,
            notif_category: 0,
            call_filter: 0,
            app_filter: None,
            collapse_ongoing: false,
            selected_key: None,
            replying_key: None,
            im_channel: ImChannel::All,
            selected_im_thread: None,
            message_window: Default::default(),
            im_drafts: Default::default(),
            reply_quote: None,
            reply_mode: ReplyMode::Auto,
            sending_thread: None,
            last_reply_status: None,
            reply_error: None,
            reply_in_flight: false,
            reply_queue: std::collections::VecDeque::new(),
            active_reply_job: None,
            next_client_msg_seq: 0,
            tab: PushCenterTab::default(),
            prefs: if cfg!(test) {
                NotifyPreferences::default()
            } else {
                desktop_notify::load_preferences()
            },
            active_toasts: Vec::new(),
            seen_toast_keys: HashSet::new(),
            shell_mode: ShellMode::Full,
            pin_window: false,
            clipboard_preview: String::new(),
            expanded_mini_card: None,
            last_sync: Instant::now()
                .checked_sub(Duration::from_secs(10))
                .unwrap_or_else(Instant::now),
            outcome_tx,
            outcome_rx,
        }
    }
}

impl Companion {
    pub(super) fn effective_push_notifications(
        &self,
    ) -> Vec<stepx_companion::push::PushNotification> {
        let demo_fallback: stepx_companion::push::PushSnapshot;
        let snapshot: &stepx_companion::push::PushSnapshot = if self.demo_mode
            && self.push.snapshot.notifications.is_empty()
            && self.push.snapshot.threads.is_empty()
        {
            demo_fallback = stepx_companion::push::demo_push_snapshot();
            &demo_fallback
        } else {
            &self.push.snapshot
        };

        let mut list: Vec<stepx_companion::push::PushNotification> = snapshot
            .notifications
            .iter()
            .filter(|n| {
                (n.active || n.removed_time_ms == 0)
                    && !n.dismissed
                    && !stepx_companion::push::should_hide_notification(n)
                    && !self.notification_center.dismissed_ids.contains(&n.key)
                    && !self
                        .notification_center
                        .dismissed_ids
                        .contains(&format!("push-notif:{}", n.key))
                    && (!self.push.collapse_ongoing
                        || !n.is_ongoing
                        || stepx_companion::home_hub::classify_delivery_or_mobility_signal(
                            &n.package_name,
                            &n.app_name,
                            &n.title,
                            &n.text,
                        )
                        .is_some())
            })
            .cloned()
            .map(|mut n| {
                if self.notification_center.read_ids.contains(&n.key)
                    || self
                        .notification_center
                        .read_ids
                        .contains(&format!("push-notif:{}", n.key))
                {
                    n.read = true;
                }
                n
            })
            .collect();

        // Synthesize notifications for unread IM threads not already covered
        for t in &snapshot.threads {
            let last_msg_ts = t
                .messages
                .iter()
                .map(|m| m.timestamp_ms.max(m.observed_at_ms))
                .max()
                .unwrap_or(0);
            let post_time_ms = t.last_timestamp_ms.max(last_msg_ts);
            if t.unread_count == 0
                || post_time_ms <= 0
                || stepx_companion::push::is_excluded_im_package(&t.package_name)
                || t.title.trim_start().starts_with('{')
                || stepx_companion::push::is_timer_or_countdown_notification(
                    &t.package_name,
                    &t.app_name,
                    &t.title,
                    &t.last_message,
                )
            {
                continue;
            }
            let syn_key = if !t.notification_key.trim().is_empty() {
                t.notification_key.clone()
            } else {
                format!("im-thread:{}", t.thread_id)
            };
            if self.notification_center.dismissed_ids.contains(&syn_key)
                || self
                    .notification_center
                    .dismissed_ids
                    .contains(&format!("im-thread:{}", t.thread_id))
            {
                continue;
            }
            let norm_t_title = stepx_companion::push::normalize_conversation_title(&t.title);
            let already_covered = list.iter().any(|n| {
                (!t.thread_id.trim().is_empty() && n.thread_id == t.thread_id)
                    || (!n.key.is_empty() && n.key == syn_key)
                    || (n.package_name == t.package_name
                        && (n.title == t.title
                            || (!norm_t_title.is_empty()
                                && stepx_companion::push::normalize_conversation_title(&n.title)
                                    == norm_t_title)))
            });
            if already_covered {
                continue;
            }
            let is_career = stepx_companion::push::is_career_or_recruitment_signal(
                &t.package_name,
                &t.app_name,
                &t.title,
                &t.last_message,
            );
            let app_name = if !t.app_name.trim().is_empty() {
                t.app_name.clone()
            } else if is_career && t.package_name.contains("bosszhipin") {
                "BOSS直聘".to_string()
            } else {
                t.channel().label().to_string()
            };
            let sub_text = if push::thread_mentions_current_user(t, &snapshot.notifications) {
                "有人@我 · 重要消息推送".into()
            } else if is_career {
                "BOSS直聘 / 招聘沟通推送".into()
            } else {
                format!("{} 条未读会话消息", t.unread_count)
            };
            let is_read = t.unread_count == 0
                || self.notification_center.read_ids.contains(&syn_key)
                || self
                    .notification_center
                    .read_ids
                    .contains(&format!("im-thread:{}", t.thread_id));
            list.push(stepx_companion::push::PushNotification {
                key: syn_key,
                package_name: t.package_name.clone(),
                app_name,
                category: "im".to_string(),
                title: t.title.clone(),
                sender: t.last_sender.clone(),
                text: t.last_message.clone(),
                sub_text,
                post_time_ms,
                active: t.unread_count > 0 && !is_read,
                is_clearable: true,
                read: is_read,
                can_reply: true,
                thread_id: t.thread_id.clone(),
                ..Default::default()
            });
        }

        // Synthesize notifications for unread incoming SMS not already covered
        let now_ms = chrono::Local::now().timestamp_millis().max(0) as u64;
        for m in &self.sms {
            let norm_addr = stepx_companion::sms::normalize_address(&m.address);
            let addr_key = if norm_addr.is_empty() {
                m.address.trim().to_string()
            } else {
                norm_addr.clone()
            };
            if m.outgoing
                || m.read
                || m.kind != 1
                || self.read_sms_addresses.contains(&m.address)
                || (!addr_key.is_empty() && self.read_sms_addresses.contains(&addr_key))
            {
                continue;
            }
            if m.date_ms >= 1_700_000_000_000
                && now_ms > 1_700_000_000_000
                && now_ms.saturating_sub(m.date_ms as u64) > 24 * 3600 * 1000
            {
                continue;
            }
            let syn_key = format!("sms:{}:{}", m.address.trim(), m.date_ms);
            if self.notification_center.dismissed_ids.contains(&syn_key) {
                continue;
            }
            let otp = super::pages::sms::extract_verification_code(&m.body);
            if let Some(ref c) = otp {
                let up = c.trim().to_uppercase();
                if !up.is_empty() && self.push.prefs.dismissed_pickup_codes.contains(&up) {
                    continue;
                }
            }
            let already_covered = list.iter().any(|n| {
                n.key == syn_key
                    || (!m.body.trim().is_empty() && n.text.trim() == m.body.trim())
                    || (otp.is_some()
                        && (n.otp_code == otp
                            || super::pages::sms::extract_verification_code(&n.text) == otp))
            });
            if already_covered {
                continue;
            }
            let delivery = stepx_companion::home_hub::classify_delivery_or_mobility_signal(
                "", "", &m.address, &m.body,
            );
            let brand = super::pages::sms::extract_sms_bracket_brand(&m.body);
            let brand_or_address = brand
                .clone()
                .unwrap_or_else(|| m.address.trim().to_string());
            let app_name = delivery
                .as_ref()
                .map(|(b, _, _, _)| b.clone())
                .or(brand)
                .unwrap_or_else(|| "短信".into());
            let category = if delivery.is_some() {
                "delivery_travel".into()
            } else if otp.is_some() {
                "system".into()
            } else {
                "im".into()
            };
            let title = delivery
                .as_ref()
                .map(|(_, h, _, _)| h.clone())
                .unwrap_or(brand_or_address);
            let sub_text = if let Some((_, _, ref detail, is_arrived)) = delivery {
                if is_arrived {
                    format!("已送达 · {detail}")
                } else {
                    format!("实时动态 · {detail}")
                }
            } else {
                "手机短信推送".into()
            };
            let is_read = self.notification_center.read_ids.contains(&syn_key);
            list.push(stepx_companion::push::PushNotification {
                key: syn_key,
                package_name: "com.android.mms".to_string(),
                app_name,
                category,
                title,
                sender: m.address.clone(),
                text: m.body.clone(),
                sub_text,
                post_time_ms: m.date_ms,
                active: true,
                is_clearable: true,
                read: is_read,
                can_reply: true,
                thread_id: format!("sms:{}", m.address.trim()),
                otp_code: otp,
                ..Default::default()
            });
        }

        // Also surface cross-device clipboard sync (`手机剪贴板已同步`) and unified notification center items
        // inside `effective_push_notifications()` so every popup notification is visible in the Desktop `消息` stream.
        let phone_clip = self.back_screen.stepos_suite.phone_clipboard.trim();
        if !phone_clip.is_empty() {
            let mut h: u64 = 0xcbf2_9ce4_8422_2325;
            for b in phone_clip.as_bytes() {
                h ^= u64::from(*b);
                h = h.wrapping_mul(0x0100_0000_01b3);
            }
            let clip_key = format!("phone-clipboard:{h:016x}");
            if !self.notification_center.dismissed_ids.contains(&clip_key)
                && !list
                    .iter()
                    .any(|n| n.key == clip_key || n.text.trim() == phone_clip)
            {
                let is_read = self.notification_center.read_ids.contains(&clip_key);
                list.push(stepx_companion::push::PushNotification {
                    key: clip_key,
                    package_name: "com.stepx.companion.clipboard".to_string(),
                    app_name: "跨端剪贴".to_string(),
                    category: "system".to_string(),
                    title: "手机剪贴板已同步".to_string(),
                    sender: "StepX 跨端互通".to_string(),
                    text: phone_clip.to_string(),
                    sub_text: "手机剪贴板实时同步到桌面 · 点击可一键复制".to_string(),
                    post_time_ms: (now_ms as i64).saturating_sub(30_000),
                    active: true,
                    is_clearable: true,
                    read: is_read,
                    can_reply: false,
                    ..Default::default()
                });
            }
        }

        for item in &self.notification_center.items {
            if self.notification_center.dismissed_ids.contains(&item.id)
                || item.id.starts_with("companion-status:")
                || item.id.starts_with("sys-device-")
            {
                continue;
            }
            let already = list.iter().any(|n| {
                if n.key == item.id
                    || format!("push-notif:{}", n.key) == item.id
                    || (!item.summary.trim().is_empty() && n.text.trim() == item.summary.trim())
                {
                    return true;
                }
                if let Some(tid) = item.id.strip_prefix("im-thread:") {
                    if n.thread_id == tid
                        || n.key == tid
                        || (!n.title.trim().is_empty() && n.title.trim() == item.title.trim())
                    {
                        return true;
                    }
                }
                if let Some(raw_k) = item.id.strip_prefix("push-notif:") {
                    if n.key == raw_k || format!("seq-{}", n.seq_id) == raw_k {
                        return true;
                    }
                }
                if let Some(rest) = item.id.strip_prefix("sms:") {
                    let addr = rest.split(':').next().unwrap_or(rest).trim();
                    let summary_prefix = item.summary.trim().trim_end_matches('…');
                    if n.thread_id == format!("sms:{addr}")
                        || (n.package_name == "com.android.mms"
                            && !summary_prefix.is_empty()
                            && n.text.contains(summary_prefix))
                    {
                        return true;
                    }
                }
                if let Some(code) = item.id.strip_prefix("otp:") {
                    let code = code.trim();
                    if !code.is_empty()
                        && (n.otp_code.as_deref() == Some(code)
                            || super::pages::sms::extract_verification_code(&n.text).as_deref()
                                == Some(code))
                    {
                        return true;
                    }
                }
                false
            });
            if already {
                continue;
            }
            let (pkg, cat) = match item.visual {
                super::notification_center_state::NotificationVisualKind::Express => {
                    ("com.cainiao.wireless", "delivery_travel")
                }
                super::notification_center_state::NotificationVisualKind::SmsOtp => {
                    ("com.android.mms", "verification")
                }
                super::notification_center_state::NotificationVisualKind::MissedCall => {
                    ("com.android.dialer", "im")
                }
                super::notification_center_state::NotificationVisualKind::CalendarAlarm => {
                    ("com.android.calendar", "system")
                }
                super::notification_center_state::NotificationVisualKind::AppUpdate => {
                    ("com.stepx.companion.store", "system")
                }
                super::notification_center_state::NotificationVisualKind::WeChat
                | super::notification_center_state::NotificationVisualKind::Feishu
                | super::notification_center_state::NotificationVisualKind::DingTalk
                | super::notification_center_state::NotificationVisualKind::ImChat => {
                    ("com.stepx.companion.app", "im")
                }
                _ => ("com.stepx.companion.app", "system"),
            };
            let mut full_text = match &item.popup_widget {
                super::notification_center_state::NotificationPopupWidget::ClipboardPreview {
                    text,
                } => {
                    let full_clip = self.back_screen.stepos_suite.phone_clipboard.trim();
                    if !full_clip.is_empty() {
                        full_clip.to_string()
                    } else {
                        text.clone()
                    }
                }
                _ => item.summary.clone(),
            };
            if item.id.starts_with("phone-clipboard:") {
                let full_clip = self.back_screen.stepos_suite.phone_clipboard.trim();
                if !full_clip.is_empty() {
                    full_text = full_clip.to_string();
                }
            }
            let mut syn_thread_id = String::new();
            let mut syn_can_reply = false;
            if let Some(tid) = item.id.strip_prefix("im-thread:") {
                syn_thread_id = tid.to_string();
                syn_can_reply = true;
            } else if let Some(rest) = item.id.strip_prefix("sms:") {
                let addr = rest.split(':').next().unwrap_or(rest).trim();
                if !addr.is_empty() {
                    syn_thread_id = format!("sms:{addr}");
                    syn_can_reply = true;
                }
                if let Some(m) = self
                    .sms
                    .iter()
                    .find(|m| format!("sms:{}:{}", m.address.trim(), m.date_ms) == item.id)
                    .or_else(|| self.sms.iter().find(|m| m.address.trim() == addr))
                {
                    if !m.body.trim().is_empty() {
                        full_text = m.body.clone();
                    }
                }
            }
            let otp_code = match &item.popup_widget {
                super::notification_center_state::NotificationPopupWidget::OtpCode {
                    code, ..
                } => Some(code.clone()),
                super::notification_center_state::NotificationPopupWidget::ExpressPickup {
                    pickup_code,
                    ..
                } if super::notification_center_state::is_real_pickup_code_label(pickup_code) => {
                    Some(pickup_code.clone())
                }
                _ => item
                    .id
                    .strip_prefix("otp:")
                    .map(str::trim)
                    .filter(|c| !c.is_empty())
                    .map(ToOwned::to_owned)
                    .or_else(|| super::pages::sms::extract_verification_code(&full_text)),
            };
            list.push(stepx_companion::push::PushNotification {
                key: item.id.clone(),
                package_name: pkg.to_string(),
                app_name: item.source_badge.clone(),
                category: cat.to_string(),
                title: item.title.clone(),
                sender: item.source_badge.clone(),
                text: full_text,
                sub_text: item
                    .status_tag
                    .clone()
                    .unwrap_or_else(|| "StepX 桌面通知中心已同步".into()),
                post_time_ms: item.sort_ts_ms,
                active: item.unread,
                is_clearable: true,
                read: !item.unread,
                can_reply: syn_can_reply,
                thread_id: syn_thread_id,
                otp_code,
                ..Default::default()
            });
        }

        list.sort_by(|a, b| b.post_time_ms.cmp(&a.post_time_ms));
        list
    }

    fn mark_push_underlying_source_read(&mut self, id: &str) {
        self.notification_center.read_ids.insert(id.to_string());
        let raw_key = id.strip_prefix("push-notif:").unwrap_or(id);
        self.notification_center
            .read_ids
            .insert(raw_key.to_string());
        self.notification_center
            .read_ids
            .insert(format!("push-notif:{raw_key}"));

        if id == "app-updates-summary" || id.starts_with("app-update:") {
            self.notification_center
                .read_ids
                .insert("app-updates-summary".to_string());
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
        }

        let tid = id.strip_prefix("im-thread:").unwrap_or(raw_key);
        for t in &mut self.push.snapshot.threads {
            if t.thread_id == tid
                || (!t.notification_key.is_empty()
                    && (t.notification_key == raw_key || t.notification_key == id))
            {
                t.unread_count = 0;
                let k1 = desktop_notify::NotifyPreferences::im_target_key(&t.thread_id);
                let clean_title =
                    push::clean_message_display_text(&push::normalize_conversation_title(&t.title));
                let k2 = format!("im:{}::{}", t.package_name.trim(), clean_title);
                self.push.prefs.manual_unread_targets.remove(&k1);
                self.push.prefs.manual_unread_targets.remove(&k2);
            }
        }
        for n in &mut self.push.snapshot.notifications {
            if n.key == raw_key
                || n.key == id
                || format!("seq-{}", n.seq_id) == raw_key
                || (!n.thread_id.is_empty() && n.thread_id == tid)
            {
                n.read = true;
            }
        }
        if let Some(rest) = id.strip_prefix("sms:") {
            let sender = rest.split(':').next().unwrap_or(rest).trim();
            let norm = stepx_companion::sms::normalize_address(sender);
            if !norm.is_empty() {
                self.read_sms_addresses.insert(norm.clone());
                let sk = desktop_notify::NotifyPreferences::sms_target_key(&norm);
                self.push.prefs.manual_unread_targets.remove(&sk);
            }
            if !sender.is_empty() {
                self.read_sms_addresses.insert(sender.to_string());
                let sk = desktop_notify::NotifyPreferences::sms_target_key(sender);
                self.push.prefs.manual_unread_targets.remove(&sk);
            }
            for m in &mut self.sms {
                let m_norm = stepx_companion::sms::normalize_address(&m.address);
                if format!("sms:{}:{}", m.address.trim(), m.date_ms) == id
                    || m.address.trim() == sender
                    || (!norm.is_empty() && m_norm == norm)
                {
                    m.read = true;
                }
            }
        }
        if let Some(code) = id.strip_prefix("otp:") {
            let code = code.trim();
            if !code.is_empty() {
                for n in &mut self.push.snapshot.notifications {
                    if n.otp_code.as_deref() == Some(code)
                        || n.text.contains(code)
                        || n.title.contains(code)
                    {
                        n.read = true;
                    }
                }
                for m in &mut self.sms {
                    if m.body.contains(code) {
                        m.read = true;
                        let norm = stepx_companion::sms::normalize_address(&m.address);
                        if !norm.is_empty() {
                            self.read_sms_addresses.insert(norm);
                        }
                        self.read_sms_addresses.insert(m.address.trim().to_string());
                    }
                }
                self.push
                    .active_toasts
                    .retain(|t| t.otp_code.as_deref() != Some(code));
            }
        }
    }

    pub(super) fn maybe_auto_copy_sms_otp(&mut self) -> Option<String> {
        if !self.push.prefs.auto_copy_otp {
            return None;
        }
        let now_ms = chrono::Local::now().timestamp_millis();
        for m in &self.sms {
            if m.read || m.outgoing || m.kind != 1 {
                continue;
            }
            if m.date_ms > 0
                && now_ms > 1_700_000_000_000
                && now_ms.saturating_sub(m.date_ms) > 900_000
            {
                continue;
            }
            if let Some(code) = super::pages::sms::extract_verification_code(&m.body) {
                if super::pages::sms::is_pickup_code_text(&code, &m.body) {
                    continue;
                }
                let seen_key = format!(
                    "auto_copy_sms_otp:{}:{}:{}",
                    m.address.trim(),
                    m.date_ms,
                    code
                );
                if self.push.seen_toast_keys.insert(seen_key) {
                    return Some(code);
                }
            }
        }
        None
    }

    pub(super) fn init_demo_push(&mut self) {
        if !self.demo_mode {
            return;
        }
        let snap = push::demo_push_snapshot();
        for n in &snap.notifications {
            self.push.insert_seen_toast_key(n.key.clone(), &snap);
        }
        self.push.selected_im_thread = snap.threads.first().map(|t| t.thread_id.clone());
        self.push.selected_key = snap.notifications.first().map(|n| n.key.clone());
        self.push.replying_key = self.push.selected_key.clone();
        self.push.expanded_mini_card = snap
            .threads
            .iter()
            .find(|t| t.unread_count > 0)
            .or_else(|| snap.threads.first())
            .map(|t| t.thread_id.clone());
        if std::env::args().any(|a| a == "--push-toast" || a == "--demo-toast")
            || std::env::var_os("STEPX_PUSH_TOAST").is_some()
        {
            if let Some(first_im) = snap.threads.first() {
                self.push.active_toasts.push(DesktopToastItem {
                    id: format!("toast:{}", first_im.thread_id),
                    notification_key: first_im.notification_key.clone(),
                    thread_id: first_im.thread_id.clone(),
                    package_name: first_im.package_name.clone(),
                    app_name: first_im.app_name.clone(),
                    category: "im".into(),
                    title: first_im.title.clone(),
                    sender: first_im.last_sender.clone(),
                    body: if first_im.last_sender.is_empty() {
                        first_im.last_message.clone()
                    } else {
                        format!("{}：{}", first_im.last_sender, first_im.last_message)
                    },
                    masked_body: format!(
                        "收到来自「{}」的新消息（防窥模式已隐藏正文）",
                        first_im.title
                    ),
                    otp_code: None,
                    url_hint: None,
                    can_reply: true,
                    timestamp_ms: first_im.last_timestamp_ms,
                });
            }
            if let Some(first_otp) = snap.otp_banners.first() {
                self.push.active_toasts.push(DesktopToastItem {
                    id: format!("toast:otp:{}", first_otp.code),
                    notification_key: first_otp.notification_key.clone(),
                    thread_id: String::new(),
                    package_name: "com.cmbchina.ccd.pluto.cmbActivity".into(),
                    app_name: first_otp.source_app.clone(),
                    category: "finance".into(),
                    title: first_otp.title.clone(),
                    sender: first_otp.source_app.clone(),
                    body: first_otp.raw_text.clone(),
                    masked_body: format!("收到来自「{}」的验证码提醒", first_otp.source_app),
                    otp_code: Some(first_otp.code.clone()),
                    url_hint: None,
                    can_reply: false,
                    timestamp_ms: first_otp.timestamp_ms,
                });
            }
        }
        self.push.snapshot = snap;
        self.push.loaded = true;
        self.push.error = None;
    }

    pub(super) fn handle_push_action(&mut self, action: &PushAction, ctx: &mut ViewContext<Self>) {
        if !self.demo_mode
            && self.selected.is_none()
            && matches!(
                action,
                PushAction::Refresh
                    | PushAction::EnsurePermission
                    | PushAction::SendImReply
                    | PushAction::SyncActivePhoneChatWindow
                    | PushAction::ReplyInline { .. }
                    | PushAction::QuickReplyPhrase { .. }
                    | PushAction::ToastSendReply(_)
            )
        {
            self.status = "请先连接手机，操作尚未执行".into();
            self.error = true;
            return;
        }
        match action {
            PushAction::Refresh => {
                if self.demo_mode || self.selected.is_none() {
                    if !self.push.loaded {
                        self.init_demo_push();
                    }
                    self.push.error = None;
                    self.status = "已同步手机全部推送与主流 IM 聊天会话".into();
                    return;
                }
                if self.selected.is_some() {
                    self.request_push_snapshot(true);
                    self.status =
                        "正在通过同 Wi-Fi / ADB 通道同步手机推送、聊天记录与联系人…".into();
                    if matches!(self.page, Page::Sms | Page::Notifications) {
                        self.spawn_message_sync();
                    }
                }
            }
            PushAction::EnsurePermission => {
                if self.demo_mode || self.selected.is_none() {
                    self.push.snapshot.listener_connected = true;
                    self.push.snapshot.accessibility_connected = true;
                    self.status = "已授权 StepX伴侣通知监听与同 Wi-Fi 原厂秒回权限".into();
                    return;
                }
                if let Some(device) = self.selected.clone() {
                    self.status = "正在为手机 StepX伴侣开启通知监听与秒回权限…".into();
                    let tx = self.push.outcome_tx.clone();
                    let serial = device.serial.clone();
                    let transport = device.transport.clone();
                    std::thread::spawn(move || {
                        let res = Adb::discover()
                            .and_then(|adb| {
                                let transport =
                                    device.resolve_active_transport(&adb).unwrap_or(transport);
                                push::ensure_notification_listener(&adb, &transport)
                            })
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(PushOutcome::PermissionEnsured(serial, res));
                    });
                }
            }
            PushAction::SelectCategory(cat) => {
                if self.push.category != *cat || self.push.notif_category != 0 {
                    self.push.category = *cat;
                    self.push.notif_category = 0;
                    self.trigger_subtab_switch();
                }
                if let Some(first) = self
                    .effective_push_notifications()
                    .into_iter()
                    .find(|n| !n.dismissed && cat.matches(&n.category))
                {
                    self.push.selected_key = Some(first.key);
                }
            }
            PushAction::SelectAppFilter(app) => {
                if self.push.app_filter != *app {
                    self.push.app_filter = app.clone();
                    self.trigger_subtab_switch();
                }
            }
            PushAction::ToggleCollapseOngoing => {
                self.push.collapse_ongoing = !self.push.collapse_ongoing;
            }
            PushAction::SelectNotification(key) => {
                if key.starts_with("msg_detail::") {
                    if self.push.selected_key.as_deref() == Some(key.as_str()) {
                        self.push.selected_key = None;
                    } else {
                        self.push.selected_key = Some(key.clone());
                    }
                } else {
                    self.push.selected_key = Some(key.clone());
                    self.push.replying_key = Some(key.clone());
                    self.mark_push_underlying_source_read(key);
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                    self.sync_notification_center();
                }
            }
            PushAction::DismissNotification(key) => {
                let raw_key = key.strip_prefix("push-notif:").unwrap_or(key.as_str());
                if let Some(n) = self.push.snapshot.notifications.iter_mut().find(|n| {
                    n.key == *key
                        || n.key == raw_key
                        || format!("seq-{}", n.seq_id) == raw_key
                        || n.thread_id == *key
                        || n.thread_id == raw_key
                }) {
                    n.dismissed = true;
                    n.read = true;
                }
                self.notification_center.dismissed_ids.insert(key.clone());
                self.notification_center
                    .dismissed_ids
                    .insert(raw_key.to_string());
                self.notification_center
                    .dismissed_ids
                    .insert(format!("push-notif:{raw_key}"));
                self.mark_push_underlying_source_read(key);
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                self.push.active_toasts.retain(|t| {
                    t.notification_key != *key
                        && t.notification_key != raw_key
                        && t.id != *key
                        && t.thread_id != *key
                        && t.thread_id != raw_key
                });
                if self.push.selected_key.as_deref() == Some(key.as_str())
                    || self.push.selected_key.as_deref() == Some(raw_key)
                {
                    self.push.selected_key = self
                        .effective_push_notifications()
                        .first()
                        .map(|n| n.key.clone());
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
                self.status = "已同步清除该条手机通知".into();
                let is_local_only_key = key.starts_with("im-thread:")
                    || key.starts_with("sms:")
                    || key.starts_with("otp:")
                    || key.starts_with("express:")
                    || key.starts_with("missed-call:")
                    || key.starts_with("phone-clipboard:")
                    || key.starts_with("calendar:")
                    || key.starts_with("alarm:")
                    || key == "app-updates-summary"
                    || key.starts_with("app-update:")
                    || key.starts_with("store-task:")
                    || key == "gui-safety-approval"
                    || key.starts_with("task:")
                    || key.starts_with("expansion-note:")
                    || key.starts_with("phone-link-note:")
                    || key.starts_with("notice:")
                    || key.starts_with("sys-");
                if !self.demo_mode && !is_local_only_key {
                    if let Some(device) = self.selected.clone() {
                        let tx = self.push.outcome_tx.clone();
                        let serial = device.serial.clone();
                        let transport = device.transport.clone();
                        let k = raw_key.to_string();
                        std::thread::spawn(move || {
                            let res = Adb::discover()
                                .and_then(|adb| {
                                    let transport =
                                        device.resolve_active_transport(&adb).unwrap_or(transport);
                                    push::dismiss_notification(&adb, &transport, &k)
                                })
                                .map_err(|e| format!("{e:#}"));
                            let _ = tx.send(PushOutcome::NotificationDismissed(serial, k, res));
                        });
                    }
                }
            }
            PushAction::DismissAll => {
                let cat = self.push.category;
                let mut cleared = 0usize;
                for n in &mut self.push.snapshot.notifications {
                    if !n.dismissed && !n.is_ongoing && cat.matches(&n.category) {
                        n.dismissed = true;
                        n.read = true;
                        cleared += 1;
                    }
                }
                let syn_list = self.effective_push_notifications();
                for syn in syn_list {
                    if !syn.is_ongoing && cat.matches(&syn.category) {
                        let raw_k = syn
                            .key
                            .strip_prefix("push-notif:")
                            .unwrap_or(syn.key.as_str())
                            .to_string();
                        self.notification_center
                            .dismissed_ids
                            .insert(syn.key.clone());
                        self.notification_center.dismissed_ids.insert(raw_k.clone());
                        self.notification_center
                            .dismissed_ids
                            .insert(format!("push-notif:{raw_k}"));
                        self.mark_push_underlying_source_read(&syn.key);
                        cleared = cleared.max(1);
                    }
                }
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                self.push.active_toasts.clear();
                self.sync_live_express_parcels();
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
                self.status = format!("已清理 {cleared} 条可清除通知");
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let tx = self.push.outcome_tx.clone();
                        let serial = device.serial.clone();
                        let transport = device.transport.clone();
                        let cat_code = (cat != PushCategory::All).then(|| cat.code().to_string());
                        std::thread::spawn(move || {
                            let res = Adb::discover()
                                .and_then(|adb| {
                                    let transport =
                                        device.resolve_active_transport(&adb).unwrap_or(transport);
                                    push::dismiss_all_notifications(
                                        &adb,
                                        &transport,
                                        cat_code.as_deref(),
                                    )
                                })
                                .map_err(|e| format!("{e:#}"));
                            let _ = tx.send(PushOutcome::AllDismissed(serial, res));
                        });
                    }
                }
            }
            PushAction::OpenOnPhone(key_or_thread) => {
                let is_center_item = self
                    .notification_center
                    .items
                    .iter()
                    .any(|item| item.id == *key_or_thread)
                    || key_or_thread.starts_with("otp:")
                    || key_or_thread.starts_with("express:")
                    || key_or_thread.starts_with("missed-call:")
                    || key_or_thread.starts_with("phone-clipboard:")
                    || key_or_thread.starts_with("calendar:")
                    || key_or_thread.starts_with("alarm:")
                    || key_or_thread == "app-updates-summary"
                    || key_or_thread.starts_with("app-update:")
                    || key_or_thread.starts_with("store-task:")
                    || key_or_thread == "gui-safety-approval"
                    || key_or_thread.starts_with("task:")
                    || key_or_thread.starts_with("expansion-note:")
                    || key_or_thread.starts_with("phone-link-note:")
                    || key_or_thread.starts_with("notice:")
                    || key_or_thread.starts_with("sys-")
                    || key_or_thread.starts_with("im-thread:")
                    || key_or_thread.starts_with("sms:");
                if is_center_item {
                    facloud_ui::TypedActionView::handle_action(
                        self,
                        &Action::NotificationCenter(
                            crate::ui::notification_center_state::NotificationCenterAction::TriggerItem(
                                key_or_thread.clone(),
                            ),
                        ),
                        ctx,
                    );
                    return;
                }
                if !self.demo_mode && self.selected.is_none() {
                    self.status = "请先连接手机，操作尚未执行".into();
                    self.error = true;
                    return;
                }
                let raw_target = key_or_thread
                    .strip_prefix("push-notif:")
                    .unwrap_or(key_or_thread.as_str());
                let notif_key = self
                    .push
                    .snapshot
                    .threads
                    .iter()
                    .find(|t| t.thread_id == raw_target || t.thread_id == *key_or_thread)
                    .map(|t| t.notification_key.clone())
                    .filter(|k| !k.is_empty())
                    .unwrap_or_else(|| raw_target.to_string());
                self.status = "已在手机上触发打开该通知对应页面".into();
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let transport = device.transport.clone();
                        std::thread::spawn(move || {
                            if let Ok(adb) = Adb::discover() {
                                let transport =
                                    device.resolve_active_transport(&adb).unwrap_or(transport);
                                let _ =
                                    push::open_notification_on_phone(&adb, &transport, &notif_key);
                            }
                        });
                    }
                }
            }
            PushAction::CopyOtp(code) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(code.clone()));
                for n in &mut self.push.snapshot.notifications {
                    if n.text.contains(code.as_str())
                        || n.title.contains(code.as_str())
                        || n.otp_code.as_deref() == Some(code.as_str())
                    {
                        n.read = true;
                    }
                }
                self.push
                    .active_toasts
                    .retain(|t| t.otp_code.as_deref() != Some(code.as_str()));
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
                self.status = format!("已复制验证码 {code}");
            }
            PushAction::OpenUrl(url) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(url.clone()));
                if url.starts_with("https://") || url.starts_with("http://") {
                    let url = url.clone();
                    std::thread::spawn(move || {
                        #[cfg(target_os = "linux")]
                        let child = std::process::Command::new("xdg-open").arg(&url).spawn();
                        #[cfg(target_os = "macos")]
                        let child = std::process::Command::new("open").arg(&url).spawn();
                        #[cfg(target_os = "windows")]
                        let child = std::process::Command::new("explorer.exe").arg(&url).spawn();
                        if let Ok(mut c) = child {
                            let _ = c.wait();
                        }
                    });
                }
                self.status = format!("已在浏览器打开并复制链接：{url}");
            }
            PushAction::ReplyInline {
                key,
                thread_id,
                package_name,
                title,
            } => {
                let text = self.fields[F_PUSH_REPLY].text.trim().to_string();
                if text.is_empty() {
                    self.status = "请输入回复内容后再发送".into();
                    self.focus_field(F_PUSH_REPLY, ctx);
                    return;
                }
                self.fields[F_PUSH_REPLY].clear();
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
                    self.apply_home_action(
                        &Action::HomeQuickSmsReply { number, body: text },
                        Some(ctx),
                    );
                    return;
                }
                self.dispatch_im_reply(thread_id, package_name, title, &text, Some(key));
            }
            PushAction::QuickReplyPhrase {
                thread_id,
                package_name,
                title,
                text,
            } => {
                self.dispatch_im_reply(thread_id, package_name, title, text, None);
            }
            PushAction::AiDraftReply {
                thread_id,
                package_name,
                title,
            } => {
                let now_ms = chrono::Local::now().timestamp_millis();
                let active_event = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|ev| {
                        !ev.all_day
                            && ev.dtstart <= now_ms + 45 * 60_000
                            && ev.dtend >= now_ms
                            && !ev.title.trim().is_empty()
                    })
                    .map(|ev| ev.title.as_str());
                let pomo_mins = self.back_screen.pomodoro_active.then(|| {
                    (self
                        .back_screen
                        .pomodoro_remaining_secs()
                        .saturating_add(59)
                        / 60)
                        .max(1)
                });
                let dnd_on = self.back_screen.stepos_suite.controls.dnd_on;
                let draft = if let Some(thread) = self
                    .push
                    .snapshot
                    .threads
                    .iter()
                    .find(|t| t.thread_id == *thread_id)
                {
                    push::generate_contextual_reply_draft(thread, active_event, pomo_mins, dnd_on)
                } else {
                    let temp = ImThread {
                        thread_id: thread_id.clone(),
                        package_name: package_name.clone(),
                        app_name: String::new(),
                        title: title.clone(),
                        ..Default::default()
                    };
                    push::generate_contextual_reply_draft(&temp, active_event, pomo_mins, dnd_on)
                };
                self.fields[F_PUSH_REPLY].set(&draft);
                self.fields[F_IM_REPLY].set(&draft);
                let target_field =
                    if self.push.shell_mode == ShellMode::Full && self.page == Page::Sms {
                        F_IM_REPLY
                    } else {
                        F_PUSH_REPLY
                    };
                self.focus_field(target_field, ctx);
                self.status =
                    format!("已结合当前日程与专注状态为「{title}」生成回复草稿，请确认后发送");
            }
            PushAction::SelectImChannel(ch) => {
                self.apply_select_im_channel(*ch);
            }
            PushAction::SelectImThread(thread_id) => {
                self.apply_select_im_thread(thread_id);
            }
            PushAction::ImHistoryPage { thread_id, first } => {
                self.push
                    .message_window
                    .borrow_mut()
                    .open(thread_id, first.clone());
                // Reset only on explicit page navigation; updates and hover retain position.
                super::pages::sms::reset_message_scroll();
            }
            PushAction::SendImReply => {
                let raw_input = self.fields[F_IM_REPLY].text.trim().to_string();
                if raw_input.is_empty() {
                    self.status = "请输入要回复的聊天内容".into();
                    self.focus_field(F_IM_REPLY, ctx);
                    return;
                }
                if self.demo_mode && self.push.snapshot.threads.is_empty() {
                    self.init_demo_push();
                }
                let mut target = self
                    .push
                    .selected_im_thread
                    .as_ref()
                    .filter(|id| !id.is_empty())
                    .and_then(|id| {
                        self.push
                            .snapshot
                            .threads
                            .iter()
                            .find(|t| &t.thread_id == id)
                    })
                    .map(|t| {
                        (
                            t.thread_id.clone(),
                            t.package_name.clone(),
                            t.title.clone(),
                            t.notification_key.clone(),
                        )
                    });
                if target.is_none() {
                    if let Some(t) = self.push.snapshot.threads.iter().find(|t| {
                        !push::is_excluded_im_package(&t.package_name)
                            && self.push.im_channel.matches_package(&t.package_name)
                    }) {
                        self.push.selected_im_thread = Some(t.thread_id.clone());
                        target = Some((
                            t.thread_id.clone(),
                            t.package_name.clone(),
                            t.title.clone(),
                            t.notification_key.clone(),
                        ));
                    }
                }
                let Some((thread_id, package_name, title, notif_key)) = target else {
                    self.status = "请先选择一个聊天会话".into();
                    return;
                };
                let clean_title = {
                    let norm = push::normalize_conversation_title(&title);
                    if norm.is_empty() {
                        title.trim().to_string()
                    } else {
                        norm
                    }
                };
                let text = if let Some((q_sender, q_text, false)) = self.push.reply_quote.take() {
                    if q_sender.trim().is_empty() {
                        format!("「{q_text}」\n{raw_input}")
                    } else {
                        format!("「{}: {q_text}」\n{raw_input}", q_sender.trim())
                    }
                } else {
                    raw_input
                };
                self.fields[F_IM_REPLY].clear();
                self.push.im_drafts.remove(&thread_id);
                self.dispatch_im_reply(
                    &thread_id,
                    &package_name,
                    &clean_title,
                    &text,
                    (!notif_key.trim().is_empty()).then_some(notif_key.trim()),
                );
            }
            PushAction::SetReplyMode(mode) => {
                self.push.reply_mode = *mode;
                self.status = format!("已切换直接回复策略：{}", mode.label());
            }
            PushAction::SyncActivePhoneChatWindow => {
                if self.demo_mode || self.selected.is_none() {
                    if !self.push.loaded {
                        self.init_demo_push();
                    }
                    let now_ms = chrono::Utc::now().timestamp_millis();
                    if let Some(t) = self.push.snapshot.threads.first_mut() {
                        let sync_id = format!("ui-sync-{now_ms}");
                        if !t.messages.iter().any(|m| m.via == "ui_bridge_capture") {
                            t.messages.push(ImMessage {
                                id: sync_id,
                                sender: t.last_sender.clone(),
                                text: "（已从手机前台屏幕实时抓取最新完整气泡上下文）".into(),
                                timestamp_ms: now_ms,
                                outgoing: false,
                                delivery_status: "received".into(),
                                via: "ui_bridge_capture".into(),
                                media_hint: String::new(),
                                client_msg_id: String::new(),
                                observed_at_ms: now_ms,
                                time_known: Some(false),
                                recalled: false,
                                ..Default::default()
                            });
                        }
                        self.status = format!(
                            "已同步手机前台「{}」聊天窗口（共 {} 条气泡）",
                            t.title,
                            t.messages.len()
                        );
                    }
                    return;
                }
                if let Some(device) = self.selected.clone() {
                    self.status = "正在抓取手机当前打开的聊天窗口气泡…".into();
                    let tx = self.push.outcome_tx.clone();
                    let serial = device.serial.clone();
                    let transport = device.transport.clone();
                    let selected_meta = self
                        .push
                        .selected_im_thread
                        .as_ref()
                        .and_then(|sel| {
                            self.push
                                .snapshot
                                .threads
                                .iter()
                                .find(|t| &t.thread_id == sel)
                        })
                        .map(|t| (t.package_name.clone(), t.title.clone()));
                    std::thread::spawn(move || {
                        let res = Adb::discover()
                            .and_then(|adb| {
                                let transport =
                                    device.resolve_active_transport(&adb).unwrap_or(transport);
                                let pkg = selected_meta.as_ref().map(|(p, _)| p.as_str());
                                let ttl = selected_meta.as_ref().map(|(_, t)| t.as_str());
                                push::sync_active_chat_window_for(&adb, &transport, pkg, ttl)
                            })
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(PushOutcome::ActiveWindowSynced(serial, res));
                    });
                }
            }
            PushAction::SetAlertScope(scope) => {
                self.push.prefs.scope = *scope;
                self.settings.notify_draft = self.push.prefs.clone();
                let _ = self.push.prefs.save();
                self.status = format!("桌面弹窗提醒范围：{}", scope.label());
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
            }
            PushAction::SetPopupMode(mode) => {
                self.push.prefs.set_popup_mode(*mode);
                self.settings.notify_draft = self.push.prefs.clone();
                self.notification_center.alert_mode = if *mode == PopupMode::Silent
                    && self
                        .push
                        .prefs
                        .app_popup_modes
                        .values()
                        .all(|m| *m == PopupMode::Silent)
                {
                    crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly
                } else {
                    crate::ui::notification_center_state::NotificationAlertMode::PopupAndBadge
                };
                if !self.demo_mode {
                    let _ = self.push.prefs.save();
                    self.notification_center.save();
                }
                self.status = format!("桌面弹窗模式：{}", mode.label());
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
            }
            PushAction::SetAppPopupMode { package_name, mode } => {
                self.push.prefs.set_app_popup_mode(package_name, *mode);
                self.settings.notify_draft = self.push.prefs.clone();
                let any_popup_enabled = self.push.prefs.popup_mode != PopupMode::Silent
                    || self
                        .push
                        .prefs
                        .app_popup_modes
                        .values()
                        .any(|m| *m != PopupMode::Silent);
                self.notification_center.alert_mode = if any_popup_enabled {
                    crate::ui::notification_center_state::NotificationAlertMode::PopupAndBadge
                } else {
                    crate::ui::notification_center_state::NotificationAlertMode::BadgeOnly
                };
                if !self.demo_mode {
                    let _ = self.push.prefs.save();
                    self.notification_center.save();
                }
                let mode_desc = mode.map(|m| m.short_label()).unwrap_or("跟随全局");
                self.status = format!("已更新应用 ({package_name}) 弹窗模式：{mode_desc}");
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
            }
            PushAction::TogglePrivacyMask => {
                self.push.prefs.privacy_mask = !self.push.prefs.privacy_mask;
                self.settings.notify_draft = self.push.prefs.clone();
                let _ = self.push.prefs.save();
                self.status = if self.push.prefs.privacy_mask {
                    "已开启弹窗防窥模式：桌面弹窗仅提醒有新消息，隐藏通知正文".into()
                } else {
                    "已关闭弹窗防窥模式：直接显示通知完整正文".into()
                };
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
            }
            PushAction::ToggleAutoCopyOtp => {
                self.push.prefs.auto_copy_otp = !self.push.prefs.auto_copy_otp;
                self.settings.notify_draft = self.push.prefs.clone();
                let _ = self.push.prefs.save();
                self.status = if self.push.prefs.auto_copy_otp {
                    "已开启：收到验证码推送时自动复制到电脑剪贴板".into()
                } else {
                    "已关闭验证码自动复制".into()
                };
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
            }
            PushAction::DismissToast(id) => {
                self.push.active_toasts.retain(|t| t.id != *id);
            }
            PushAction::DismissAllToasts => {
                self.push.active_toasts.clear();
            }
            PushAction::ToastSendReply(toast_id) => {
                let text = self.fields[F_TOAST_REPLY].text.trim().to_string();
                if text.is_empty() {
                    self.status = "请在弹窗输入框内输入回复内容".into();
                    self.focus_field(F_TOAST_REPLY, ctx);
                    return;
                }
                if let Some(popup) = self.notification_active_popup().cloned() {
                    self.fields[F_TOAST_REPLY].clear();
                    self.push.active_toasts.clear();
                    self.submit_popup_quick_reply(&popup.id, &text);
                    self.sync_desktop_popup_window(ctx);
                    return;
                }
                let toast = self
                    .push
                    .active_toasts
                    .iter()
                    .find(|t| t.id == *toast_id)
                    .or_else(|| self.push.active_toasts.first())
                    .cloned();
                self.fields[F_TOAST_REPLY].clear();
                if let Some(item) = toast {
                    self.push.active_toasts.retain(|t| t.id != item.id);
                    let tid = if item.thread_id.is_empty() {
                        format!("{}::{}", item.package_name, item.title)
                    } else {
                        item.thread_id.clone()
                    };
                    self.dispatch_im_reply(
                        &tid,
                        &item.package_name,
                        &item.title,
                        &text,
                        Some(&item.notification_key),
                    );
                }
            }
            PushAction::ToastOpenInMain(toast_id) => {
                let toast = self
                    .push
                    .active_toasts
                    .iter()
                    .find(|t| t.id == *toast_id)
                    .or_else(|| self.push.active_toasts.first())
                    .cloned();
                if let Some(item) = toast {
                    self.push.active_toasts.retain(|t| t.id != item.id);
                    self.set_compact_mode(ShellMode::Full, ctx);
                    if !item.thread_id.is_empty() || item.can_reply {
                        self.page = Page::Sms;
                        self.push.im_channel = ImChannel::All;
                        if !item.thread_id.is_empty() {
                            self.push.selected_im_thread = Some(item.thread_id);
                        }
                        self.focus_field(F_IM_REPLY, ctx);
                    } else {
                        self.page = Page::Notifications;
                        self.push.selected_key = Some(item.notification_key);
                    }
                }
            }
            PushAction::ToastOpenInMini(toast_id) => {
                let toast = self
                    .push
                    .active_toasts
                    .iter()
                    .find(|t| t.id == *toast_id)
                    .cloned();
                if let Some(item) = toast {
                    self.push.active_toasts.retain(|t| t.id != item.id);
                    self.push.selected_key = Some(item.notification_key);
                    self.set_compact_mode(ShellMode::MiniNotify, ctx);
                    if !item.thread_id.is_empty() {
                        self.handle_push_action(&PushAction::SelectImThread(item.thread_id), ctx);
                        self.focus_field(F_IM_REPLY, ctx);
                    } else {
                        self.compact.notification_detail = false;
                    }
                }
            }
            PushAction::SetShellMode(mode) => {
                self.set_compact_mode(*mode, ctx);
                if *mode == ShellMode::Full {
                    self.auto_load_for_page();
                }
            }
            PushAction::CompactInbox => {
                self.compact.notification_detail = false;
                self.focus = None;
            }
            PushAction::CompactSystemNotices => {
                self.compact.system_notifications = !self.compact.system_notifications;
            }
            PushAction::CompactHistory => {
                self.compact.history = !self.compact.history;
                self.scroll_aux = Default::default();
                if !self.compact.history {
                    self.reveal_latest_chat();
                }
            }
            PushAction::TogglePinWindow => {
                let next = !self.push.pin_window;
                self.set_compact_pin(next, ctx);
                self.status = if self.push.pin_window {
                    "已开启小窗置顶（Always on Top）".into()
                } else {
                    "已取消小窗置顶".into()
                };
                self.error = false;
            }
            PushAction::CaptureClipboardPreview => {
                let text = ctx.clipboard().read().plain_text.trim().to_string();
                let effective = if !text.is_empty() {
                    text
                } else if !self
                    .back_screen
                    .stepos_suite
                    .phone_clipboard
                    .trim()
                    .is_empty()
                {
                    self.back_screen
                        .stepos_suite
                        .phone_clipboard
                        .trim()
                        .to_string()
                } else {
                    String::new()
                };
                self.push.clipboard_preview = Self::truncate_chars(&effective, 240);
                self.status = if self.push.clipboard_preview.is_empty() {
                    "剪贴板暂无文本内容".into()
                } else {
                    "已读取系统剪贴板内容".into()
                };
                self.error = false;
            }
            PushAction::AskClipboard(kind) => {
                let raw_clip = ctx.clipboard().read().plain_text.trim().to_string();
                let clip = if !raw_clip.is_empty() {
                    raw_clip
                } else if !self.push.clipboard_preview.trim().is_empty() {
                    self.push.clipboard_preview.trim().to_string()
                } else if !self
                    .back_screen
                    .stepos_suite
                    .phone_clipboard
                    .trim()
                    .is_empty()
                {
                    self.back_screen
                        .stepos_suite
                        .phone_clipboard
                        .trim()
                        .to_string()
                } else if self.demo_mode {
                    "StepX Companion Desktop AI Workspace & Mobile Bridge".to_string()
                } else {
                    String::new()
                };
                if !clip.is_empty() {
                    self.push.clipboard_preview = Self::truncate_chars(&clip, 240);
                }
                let prompt = match *kind {
                    0 => format!("请帮我详细解释以下剪贴板内容的核心含义：\n\n{clip}"),
                    1 => format!(
                        "请将以下剪贴板内容准确翻译为自然流畅的中文（若已是中文则翻译为英文）：\n\n{clip}"
                    ),
                    2 => format!("请帮我提炼以下剪贴板内容的关键要点与行动项：\n\n{clip}"),
                    _ => format!("请帮我润色或优化以下剪贴板内容：\n\n{clip}"),
                };
                if self.push.shell_mode == ShellMode::MiniNotify {
                    self.set_compact_mode(ShellMode::MiniAi, ctx);
                }
                self.compact.history = false;
                self.workspace.sub_view = 0;
                self.workspace.history = false;
                self.workspace.panel = 0;
                self.fields[F_AMOO].set(&prompt);
                self.focus_field(F_AMOO, ctx);
                self.status = "已将剪贴板提问填入输入框，按 Enter 即可发送".into();
                self.error = false;
                ctx.notify();
            }
            PushAction::OpenMiniAiWindow => self.set_compact_mode(ShellMode::MiniAi, ctx),
            PushAction::OpenMiniNotifyWindow => self.set_compact_mode(ShellMode::MiniNotify, ctx),
            PushAction::LoadDemoPush => {
                if !self.demo_mode {
                    return;
                }
                self.init_demo_push();
                self.status = "已载入微信/飞书/钉钉/企微演示会话与全量手机推送数据".into();
            }
            PushAction::SetCallFilter(flt) => {
                self.push.call_filter = *flt;
                self.call_filter_missed = *flt == 1;
                self.trigger_subtab_switch();
                let label = match *flt {
                    1 => "未接来电",
                    2 => "呼入通话",
                    3 => "呼出通话",
                    _ => "全部通话记录",
                };
                self.status = format!("已筛选通话记录：{label}");
                self.error = false;
            }
            PushAction::SetNotifCategory(cat) => {
                self.push.notif_category = *cat;
                self.trigger_subtab_switch();
                let label = match *cat {
                    1 => "验证码/取件码",
                    2 => "社交通讯",
                    3 => "系统与服务",
                    4 => "未读通知",
                    _ => "全部通知",
                };
                self.status = format!("已切换通知快捷分组：{label}");
                self.error = false;
            }
            PushAction::FillSmsTemplate(tpl) => {
                self.fields[F_SMS_BODY].set(tpl);
                self.pending_sms_plan = None;
                self.focus_field(F_SMS_BODY, ctx);
                self.status = format!("已填入短信快捷模板：「{tpl}」");
                self.error = false;
            }
            PushAction::SelectCalendarQuickTime(time_str) => {
                self.fields[F_CALENDAR_TIME].set(time_str);
                self.focus_field(F_PHONE_SCHEDULE_TITLE, ctx);
                self.status = format!("已选择日程时间：{time_str}");
                self.error = false;
            }
            PushAction::OpenNewAmooChat => {
                self.set_compact_mode(ShellMode::Full, ctx);
                self.page = Page::Amoo;
                self.handle_workspace(&WorkspaceAction::New, ctx);
                self.focus_field(F_AMOO, ctx);
                self.status = "已为您新建 Amoo 对话，请直接输入问题或指令".into();
                self.error = false;
            }
            PushAction::MarkAllNotificationsRead => {
                for n in &mut self.push.snapshot.notifications {
                    n.read = true;
                }
                for t in &mut self.push.snapshot.threads {
                    t.unread_count = 0;
                }
                for m in &mut self.sms {
                    m.read = true;
                    let norm = stepx_companion::sms::normalize_address(&m.address);
                    if !norm.is_empty() {
                        self.read_sms_addresses.insert(norm);
                    }
                    self.read_sms_addresses.insert(m.address.trim().to_string());
                }
                self.push.prefs.manual_unread_targets.clear();
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                for item in &self.notification_center.items {
                    self.notification_center.read_ids.insert(item.id.clone());
                }
                for syn in self.effective_push_notifications() {
                    self.notification_center.read_ids.insert(syn.key);
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
                self.status = "已将全部通知标为已读".into();
                self.error = false;
            }
            PushAction::ResetNotificationFilters => {
                self.push.category = PushCategory::All;
                self.push.notif_category = 0;
                self.push.app_filter = None;
                self.fields[F_PUSH_SEARCH].clear();
                self.status = "已重置通知筛选条件".into();
                self.error = false;
            }
            PushAction::MarkAllMessagesRead => {
                let mut unread_im_targets: Vec<(String, String, String)> = Vec::new();
                for t in &mut self.push.snapshot.threads {
                    if t.unread_count > 0 {
                        unread_im_targets.push((
                            t.thread_id.clone(),
                            t.package_name.clone(),
                            t.title.clone(),
                        ));
                    }
                    t.unread_count = 0;
                }
                for n in &mut self.push.snapshot.notifications {
                    n.read = true;
                }
                for m in &mut self.sms {
                    m.read = true;
                    let norm = stepx_companion::sms::normalize_address(&m.address);
                    if !norm.is_empty() {
                        self.read_sms_addresses.insert(norm);
                    }
                    self.read_sms_addresses.insert(m.address.trim().to_string());
                }
                for item in &self.notification_center.items {
                    self.notification_center.read_ids.insert(item.id.clone());
                }
                for syn in self.effective_push_notifications() {
                    self.notification_center.read_ids.insert(syn.key);
                }
                self.push.prefs.manual_unread_targets.clear();
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                if !self.demo_mode && !unread_im_targets.is_empty() {
                    if let Some(device) = self.selected.clone() {
                        let transport = device.transport.clone();
                        std::thread::spawn(move || {
                            if let Ok(adb) = Adb::discover() {
                                let transport =
                                    device.resolve_active_transport(&adb).unwrap_or(transport);
                                for (tid, pkg, ttl) in unread_im_targets {
                                    let _ = push::mark_im_thread_read(
                                        &adb, &transport, &tid, &pkg, &ttl, true,
                                    );
                                }
                            }
                        });
                    }
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
                self.sync_desktop_popup_window(ctx);
                self.status = "已将全部消息标为已读".into();
                self.error = false;
            }
            PushAction::QuoteReply {
                is_im,
                sender,
                text,
            } => {
                let single_line = push::clean_message_display_text(text).replace(['\r', '\n'], " ");
                let snippet: String = single_line.trim().chars().take(48).collect();
                self.push.reply_quote = Some((sender.trim().to_string(), snippet, !*is_im));
                if *is_im {
                    self.focus_field(F_IM_REPLY, ctx);
                } else {
                    self.pending_sms_plan = None;
                    self.focus_field(F_SMS_BODY, ctx);
                }
                self.status = "已引用消息内容，请继续输入回复".into();
                self.error = false;
            }
            PushAction::ClearQuoteReply => {
                self.push.reply_quote = None;
                self.status = "已取消引用消息".into();
                self.error = false;
            }
            PushAction::DeleteImThread { .. }
            | PushAction::DeleteSmsThread { .. }
            | PushAction::TogglePinTarget(_)
            | PushAction::ToggleThreadRead { .. }
            | PushAction::ClearThreadMessages { .. }
            | PushAction::DeleteMessage { .. }
            | PushAction::DeleteSmsMessage { .. }
            | PushAction::DismissPickupCode(_) => {
                self.apply_common_push_action(action);
            }
        }
    }

    pub(super) fn apply_common_push_action(&mut self, action: &PushAction) -> bool {
        match action {
            PushAction::DismissPickupCode(code) => {
                let clean = code.trim().to_uppercase();
                if !clean.is_empty() {
                    self.push.prefs.dismissed_pickup_codes.insert(clean.clone());
                    for p in &self.express_parcels {
                        if p.pickup_code.trim().eq_ignore_ascii_case(&clean)
                            || p.tracking_no.trim().eq_ignore_ascii_case(&clean)
                        {
                            let pc = p.pickup_code.trim().to_uppercase();
                            let tn = p.tracking_no.trim().to_uppercase();
                            if !pc.is_empty() {
                                self.push.prefs.dismissed_pickup_codes.insert(pc);
                            }
                            if !tn.is_empty() {
                                self.push.prefs.dismissed_pickup_codes.insert(tn);
                            }
                        }
                    }
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                    self.express_parcels.retain(|p| {
                        let pc = p.pickup_code.trim().to_uppercase();
                        let tn = p.tracking_no.trim().to_uppercase();
                        !pc.eq_ignore_ascii_case(&clean)
                            && !tn.eq_ignore_ascii_case(&clean)
                            && (pc.is_empty()
                                || !self.push.prefs.dismissed_pickup_codes.contains(&pc))
                            && (tn.is_empty()
                                || !self.push.prefs.dismissed_pickup_codes.contains(&tn))
                    });
                    for n in &mut self.push.snapshot.notifications {
                        if n.otp_code
                            .as_deref()
                            .is_some_and(|c| c.trim().eq_ignore_ascii_case(&clean))
                            || n.title.to_uppercase().contains(&clean)
                            || n.text.to_uppercase().contains(&clean)
                        {
                            n.read = true;
                            n.dismissed = true;
                        }
                    }
                    self.push
                        .snapshot
                        .otp_banners
                        .retain(|b| !b.code.trim().eq_ignore_ascii_case(&clean));
                    self.push.active_toasts.retain(|t| {
                        !t.otp_code
                            .as_deref()
                            .is_some_and(|c| c.trim().eq_ignore_ascii_case(&clean))
                            && !t.body.to_uppercase().contains(&clean)
                    });
                    for m in &mut self.sms {
                        if m.body.to_uppercase().contains(&clean) {
                            m.read = true;
                            let norm = stepx_companion::sms::normalize_address(&m.address);
                            if !norm.is_empty() {
                                self.read_sms_addresses.insert(norm);
                            }
                        }
                    }
                    for t in &mut self.push.snapshot.threads {
                        if t.last_message.to_uppercase().contains(&clean) {
                            t.unread_count = 0;
                        }
                    }
                    self.sync_live_express_parcels();
                    self.sync_notification_center();
                    self.status = format!("已将「{}」标记为已取件", code.trim());
                    self.error = false;
                }
                true
            }
            PushAction::DeleteImThread {
                thread_id,
                package_name,
                title,
            } => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let clean_title =
                    push::clean_message_display_text(&push::normalize_conversation_title(title));
                let key1 = desktop_notify::NotifyPreferences::im_target_key(thread_id);
                let key2 = format!("im:{}::{}", package_name.trim(), clean_title);
                self.push.prefs.deleted_threads.insert(key1.clone(), now_ms);
                if !clean_title.is_empty() {
                    self.push.prefs.deleted_threads.insert(key2.clone(), now_ms);
                }
                self.push.prefs.manual_unread_targets.remove(&key1);
                self.push.prefs.manual_unread_targets.remove(&key2);
                self.push.im_drafts.remove(thread_id);
                let _ = desktop_notify::save_preferences(&self.push.prefs);

                self.push.snapshot.threads.retain(|t| {
                    let t_clean = push::clean_message_display_text(
                        &push::normalize_conversation_title(&t.title),
                    );
                    t.thread_id != *thread_id
                        && !(t.package_name == *package_name
                            && !clean_title.is_empty()
                            && t_clean == clean_title)
                });
                for n in &mut self.push.snapshot.notifications {
                    let n_clean = push::clean_message_display_text(
                        &push::normalize_conversation_title(&n.title),
                    );
                    if n.thread_id == *thread_id
                        || (n.package_name == *package_name
                            && !clean_title.is_empty()
                            && n_clean == clean_title)
                    {
                        n.dismissed = true;
                        n.read = true;
                    }
                }
                self.push
                    .active_toasts
                    .retain(|t| t.thread_id != *thread_id);
                if self.push.selected_im_thread.as_deref() == Some(thread_id.as_str()) {
                    let next_im = self
                        .push
                        .snapshot
                        .threads
                        .iter()
                        .find(|t| self.push.im_channel.matches_package(&t.package_name))
                        .or_else(|| self.push.snapshot.threads.first())
                        .map(|t| t.thread_id.clone());
                    self.push.selected_im_thread = next_im;
                    if self.push.selected_im_thread.is_none()
                        && self.push.im_channel == ImChannel::All
                    {
                        self.selected_thread =
                            self.sms_threads().first().map(|t| t.address.clone());
                    }
                }
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let transport = device.transport.clone();
                        let tid = thread_id.clone();
                        let pkg = package_name.clone();
                        let ttl = if clean_title.is_empty() {
                            title.clone()
                        } else {
                            clean_title.clone()
                        };
                        std::thread::spawn(move || {
                            if let Ok(adb) = Adb::discover() {
                                let transport =
                                    device.resolve_active_transport(&adb).unwrap_or(transport);
                                let _ = push::delete_im_thread(&adb, &transport, &tid, &pkg, &ttl);
                            }
                        });
                    }
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
                let label = if clean_title.is_empty() {
                    title.as_str()
                } else {
                    clean_title.as_str()
                };
                self.status = format!("已删除对话「{label}」并同步清理");
                self.error = false;
                true
            }
            PushAction::DeleteSmsThread { address, name } => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                let norm_addr = stepx_companion::sms::normalize_address(address);
                let key1 = desktop_notify::NotifyPreferences::sms_target_key(address);
                let key2 = desktop_notify::NotifyPreferences::sms_target_key(&norm_addr);
                self.push.prefs.deleted_threads.insert(key1.clone(), now_ms);
                self.push.prefs.deleted_threads.insert(key2.clone(), now_ms);
                for shadow_key in [
                    format!("com.android.mms::{norm_addr}"),
                    format!("com.google.android.apps.messaging::{norm_addr}"),
                    format!("im:com.android.mms::{norm_addr}"),
                    format!("im:com.google.android.apps.messaging::{norm_addr}"),
                    format!("com.android.mms::{}", address.trim()),
                    format!("im:com.android.mms::{}", address.trim()),
                ] {
                    self.push.prefs.deleted_threads.insert(shadow_key, now_ms);
                }
                self.push.prefs.manual_unread_targets.remove(&key1);
                self.push.prefs.manual_unread_targets.remove(&key2);
                let _ = desktop_notify::save_preferences(&self.push.prefs);

                self.sms.retain(|m| {
                    stepx_companion::sms::normalize_address(&m.address) != norm_addr
                        && m.address.trim() != address.trim()
                });
                self.local_sent_sms.retain(|m| {
                    stepx_companion::sms::normalize_address(&m.address) != norm_addr
                        && m.address.trim() != address.trim()
                });
                self.push.snapshot.threads.retain(|t| {
                    if t.channel() == ImChannel::Sms {
                        let t_norm = stepx_companion::sms::normalize_address(&t.title);
                        if (!t_norm.is_empty() && t_norm == norm_addr)
                            || t.title.trim() == address.trim()
                            || t.title.trim() == name.trim()
                        {
                            return false;
                        }
                    }
                    true
                });
                for n in &mut self.push.snapshot.notifications {
                    if ImChannel::from_package(&n.package_name) == ImChannel::Sms {
                        let n_norm = stepx_companion::sms::normalize_address(&n.title);
                        if (!n_norm.is_empty() && n_norm == norm_addr)
                            || n.title.trim() == address.trim()
                            || n.title.trim() == name.trim()
                        {
                            n.dismissed = true;
                            n.read = true;
                        }
                    }
                }
                if self.selected_thread.as_deref() == Some(address.as_str())
                    || self
                        .selected_thread
                        .as_deref()
                        .is_some_and(|a| stepx_companion::sms::normalize_address(a) == norm_addr)
                {
                    self.selected_thread = self.sms_threads().first().map(|t| t.address.clone());
                    if self.selected_thread.is_none() && self.push.im_channel == ImChannel::All {
                        self.push.selected_im_thread = self
                            .push
                            .snapshot
                            .threads
                            .first()
                            .map(|t| t.thread_id.clone());
                    }
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
                let label = if name.trim().is_empty() {
                    address.as_str()
                } else {
                    name.as_str()
                };
                self.status = format!("已删除短信对话「{label}」");
                self.error = false;
                true
            }
            PushAction::TogglePinTarget(target_key) => {
                let pinned = self.push.prefs.toggle_pin(target_key.clone());
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                self.status = if pinned {
                    "已将该对话置顶".into()
                } else {
                    "已取消置顶该对话".into()
                };
                self.error = false;
                true
            }
            PushAction::ToggleThreadRead { is_im, target_id } => {
                if *is_im {
                    let (package_name, title, mark_unread) = self
                        .push
                        .snapshot
                        .threads
                        .iter()
                        .find(|t| t.thread_id == *target_id)
                        .map(|t| (t.package_name.clone(), t.title.clone(), t.unread_count == 0))
                        .unwrap_or_else(|| {
                            let (pkg, ttl) = target_id
                                .split_once("::")
                                .unwrap_or(("com.tencent.mm", target_id.as_str()));
                            (pkg.to_string(), ttl.to_string(), false)
                        });
                    let clean_title = push::clean_message_display_text(
                        &push::normalize_conversation_title(&title),
                    );
                    let key1 = desktop_notify::NotifyPreferences::im_target_key(target_id);
                    let key2 = format!("im:{}::{}", package_name.trim(), clean_title);
                    if mark_unread {
                        self.push.prefs.manual_unread_targets.insert(key1);
                        if !clean_title.is_empty() {
                            self.push.prefs.manual_unread_targets.insert(key2);
                        }
                    } else {
                        self.push.prefs.manual_unread_targets.remove(&key1);
                        self.push.prefs.manual_unread_targets.remove(&key2);
                    }
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                    for t in &mut self.push.snapshot.threads {
                        if t.thread_id == *target_id {
                            t.unread_count = if mark_unread {
                                t.unread_count.max(1)
                            } else {
                                0
                            };
                        }
                    }
                    for n in &mut self.push.snapshot.notifications {
                        if n.thread_id == *target_id {
                            n.read = !mark_unread;
                        }
                    }
                    if !self.demo_mode {
                        if let Some(device) = self.selected.clone() {
                            let transport = device.transport.clone();
                            let tid = target_id.clone();
                            let pkg = package_name.clone();
                            let ttl = if clean_title.is_empty() {
                                title.clone()
                            } else {
                                clean_title.clone()
                            };
                            let read_flag = !mark_unread;
                            std::thread::spawn(move || {
                                if let Ok(adb) = Adb::discover() {
                                    let transport =
                                        device.resolve_active_transport(&adb).unwrap_or(transport);
                                    let _ = push::mark_im_thread_read(
                                        &adb, &transport, &tid, &pkg, &ttl, read_flag,
                                    );
                                }
                            });
                        }
                    }
                    self.sync_live_express_parcels();
                    self.sync_notification_center();
                    let label = if clean_title.is_empty() {
                        title.as_str()
                    } else {
                        clean_title.as_str()
                    };
                    self.status = if mark_unread {
                        format!("已将「{label}」标为未读")
                    } else {
                        format!("已将「{label}」标为已读")
                    };
                } else {
                    let norm_addr = stepx_companion::sms::normalize_address(target_id);
                    let key1 = desktop_notify::NotifyPreferences::sms_target_key(target_id);
                    let key2 = desktop_notify::NotifyPreferences::sms_target_key(&norm_addr);
                    let currently_unread = self.push.prefs.manual_unread_targets.contains(&key1)
                        || self.push.prefs.manual_unread_targets.contains(&key2)
                        || (!self.read_sms_addresses.contains(&norm_addr)
                            && self.sms.iter().any(|m| {
                                (stepx_companion::sms::normalize_address(&m.address) == norm_addr
                                    || m.address.trim() == target_id.trim())
                                    && m.kind == 1
                                    && !m.read
                            }));
                    let mark_unread = !currently_unread;
                    if mark_unread {
                        self.push.prefs.manual_unread_targets.insert(key1);
                        self.push.prefs.manual_unread_targets.insert(key2);
                        self.read_sms_addresses.remove(&norm_addr);
                        self.read_sms_addresses.remove(target_id.trim());
                        if let Some(m) = self.sms.iter_mut().find(|m| {
                            stepx_companion::sms::normalize_address(&m.address) == norm_addr
                                || m.address.trim() == target_id.trim()
                        }) {
                            m.read = false;
                        }
                    } else {
                        self.push.prefs.manual_unread_targets.remove(&key1);
                        self.push.prefs.manual_unread_targets.remove(&key2);
                        self.read_sms_addresses.insert(norm_addr.clone());
                        self.read_sms_addresses.insert(target_id.trim().to_string());
                        for m in &mut self.sms {
                            if stepx_companion::sms::normalize_address(&m.address) == norm_addr
                                || m.address.trim() == target_id.trim()
                            {
                                m.read = true;
                            }
                        }
                    }
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                    self.sync_live_express_parcels();
                    self.sync_notification_center();
                    self.status = if mark_unread {
                        format!("已将「{target_id}」标为未读")
                    } else {
                        format!("已将「{target_id}」标为已读")
                    };
                }
                self.error = false;
                true
            }
            PushAction::ClearThreadMessages {
                is_im,
                target_id,
                package_name,
                title,
            } => {
                let now_ms = chrono::Utc::now().timestamp_millis();
                if *is_im {
                    let clean_title = push::clean_message_display_text(
                        &push::normalize_conversation_title(title),
                    );
                    let key1 = desktop_notify::NotifyPreferences::im_target_key(target_id);
                    let key2 = format!("im:{}::{}", package_name.trim(), clean_title);
                    self.push.prefs.cleared_threads.insert(key1.clone(), now_ms);
                    if !clean_title.is_empty() {
                        self.push.prefs.cleared_threads.insert(key2.clone(), now_ms);
                    }
                    self.push.prefs.manual_unread_targets.remove(&key1);
                    self.push.prefs.manual_unread_targets.remove(&key2);
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                    for t in &mut self.push.snapshot.threads {
                        if t.thread_id == *target_id {
                            t.messages.clear();
                            t.last_message.clear();
                            t.last_sender.clear();
                            t.unread_count = 0;
                        }
                    }
                    if !self.demo_mode {
                        if let Some(device) = self.selected.clone() {
                            let transport = device.transport.clone();
                            let tid = target_id.clone();
                            let pkg = package_name.clone();
                            let ttl = if clean_title.is_empty() {
                                title.clone()
                            } else {
                                clean_title.clone()
                            };
                            std::thread::spawn(move || {
                                if let Ok(adb) = Adb::discover() {
                                    let transport =
                                        device.resolve_active_transport(&adb).unwrap_or(transport);
                                    let _ = push::clear_im_thread_messages(
                                        &adb, &transport, &tid, &pkg, &ttl,
                                    );
                                }
                            });
                        }
                    }
                    self.sync_live_express_parcels();
                    self.sync_notification_center();
                    self.status = format!("已清空「{title}」的聊天记录");
                } else {
                    let norm_addr = stepx_companion::sms::normalize_address(target_id);
                    let key1 = desktop_notify::NotifyPreferences::sms_target_key(target_id);
                    let key2 = desktop_notify::NotifyPreferences::sms_target_key(&norm_addr);
                    self.push.prefs.cleared_threads.insert(key1, now_ms);
                    self.push.prefs.cleared_threads.insert(key2, now_ms);
                    for shadow_key in [
                        format!("com.android.mms::{norm_addr}"),
                        format!("com.google.android.apps.messaging::{norm_addr}"),
                        format!("im:com.android.mms::{norm_addr}"),
                        format!("im:com.google.android.apps.messaging::{norm_addr}"),
                        format!("com.android.mms::{}", target_id.trim()),
                        format!("im:com.android.mms::{}", target_id.trim()),
                    ] {
                        self.push.prefs.cleared_threads.insert(shadow_key, now_ms);
                    }
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                    self.sms.retain(|m| {
                        stepx_companion::sms::normalize_address(&m.address) != norm_addr
                            && m.address.trim() != target_id.trim()
                    });
                    self.local_sent_sms.retain(|m| {
                        stepx_companion::sms::normalize_address(&m.address) != norm_addr
                            && m.address.trim() != target_id.trim()
                    });
                    for t in &mut self.push.snapshot.threads {
                        if t.channel() == ImChannel::Sms {
                            let t_norm = stepx_companion::sms::normalize_address(&t.title);
                            if (!t_norm.is_empty() && t_norm == norm_addr)
                                || t.title.trim() == target_id.trim()
                            {
                                t.messages.clear();
                                t.last_message.clear();
                                t.last_sender.clear();
                                t.unread_count = 0;
                            }
                        }
                    }
                    self.sync_live_express_parcels();
                    self.sync_notification_center();
                    self.status = format!("已清空「{title}」的短信记录");
                }
                self.error = false;
                true
            }
            PushAction::DeleteMessage {
                thread_id,
                package_name: _,
                msg_id,
                text,
            } => {
                if !msg_id.trim().is_empty() {
                    self.push.prefs.deleted_messages.insert(format!(
                        "im_msg:{}::{}",
                        thread_id.trim(),
                        msg_id.trim()
                    ));
                }
                if !text.trim().is_empty() {
                    self.push.prefs.deleted_messages.insert(format!(
                        "im_txt:{}::{}",
                        thread_id.trim(),
                        text.trim()
                    ));
                }
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                if let Some(t) = self
                    .push
                    .snapshot
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == *thread_id)
                {
                    let mut removed_one = false;
                    t.messages.retain(|m| {
                        if removed_one {
                            return true;
                        }
                        let match_id = !msg_id.trim().is_empty()
                            && (m.id == *msg_id || m.client_msg_id == *msg_id);
                        let match_txt = msg_id.trim().is_empty()
                            && !text.trim().is_empty()
                            && m.text.trim() == text.trim();
                        if match_id || match_txt {
                            removed_one = true;
                            false
                        } else {
                            true
                        }
                    });
                    if t.messages.is_empty() {
                        t.last_message.clear();
                        t.last_sender.clear();
                    } else {
                        push::recompute_thread_summary(t);
                    }
                }
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let transport = device.transport.clone();
                        let tid = thread_id.clone();
                        let mid = msg_id.clone();
                        let txt = text.clone();
                        std::thread::spawn(move || {
                            if let Ok(adb) = Adb::discover() {
                                let transport =
                                    device.resolve_active_transport(&adb).unwrap_or(transport);
                                let _ = push::delete_im_message(&adb, &transport, &tid, &mid, &txt);
                            }
                        });
                    }
                }
                self.sync_live_express_parcels();
                self.sync_notification_center();
                self.status = "已删除该条消息".into();
                self.error = false;
                true
            }
            PushAction::DeleteSmsMessage {
                address,
                date_ms,
                body,
            } => {
                let norm_addr = stepx_companion::sms::normalize_address(address);
                self.push
                    .prefs
                    .deleted_messages
                    .insert(format!("sms_msg:{norm_addr}::{date_ms}::{}", body.trim()));
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                let mut removed = false;
                self.sms.retain(|m| {
                    if removed {
                        return true;
                    }
                    let same_addr = stepx_companion::sms::normalize_address(&m.address)
                        == norm_addr
                        || m.address.trim() == address.trim();
                    if same_addr && m.date_ms == *date_ms && m.body.trim() == body.trim() {
                        removed = true;
                        false
                    } else {
                        true
                    }
                });
                self.local_sent_sms.retain(|m| {
                    let same_addr = stepx_companion::sms::normalize_address(&m.address)
                        == norm_addr
                        || m.address.trim() == address.trim();
                    !(same_addr && m.date_ms == *date_ms && m.body.trim() == body.trim())
                });
                self.sync_live_express_parcels();
                self.sync_notification_center();
                self.status = "已删除该条短信".into();
                self.error = false;
                true
            }
            _ => false,
        }
    }

    pub(super) fn apply_select_im_channel(&mut self, ch: ImChannel) {
        if self.push.im_channel != ch {
            self.push.reply_quote = None;
            self.trigger_subtab_switch();
        }
        let first_sms = self.sms_threads().first().map(|t| t.address.clone());
        self.pending_sms_plan = None;
        select_im_channel_state(
            &mut self.push.im_channel,
            &mut self.push.selected_im_thread,
            &mut self.selected_thread,
            &self.push.snapshot.threads,
            first_sms.as_deref(),
            ch,
        );
    }

    pub(super) fn apply_select_im_thread(&mut self, thread_id: &str) {
        self.push.reply_quote = None;
        if self.demo_mode && self.push.snapshot.threads.is_empty() {
            self.init_demo_push();
        }
        if self.push.shell_mode == ShellMode::MiniNotify && !thread_id.is_empty() {
            self.compact.notification_detail = true;
        } else if self.push.shell_mode == ShellMode::Full
            && self.page == Page::Notifications
            && !thread_id.is_empty()
        {
            self.page = Page::Sms;
        }
        if self.push.selected_im_thread.as_deref() != Some(thread_id) {
            if let Some(previous) = &self.push.selected_im_thread {
                if !previous.is_empty() {
                    let draft = self.fields[F_IM_REPLY].text.clone();
                    if draft.trim().is_empty() {
                        self.push.im_drafts.remove(previous);
                    } else {
                        if !self.push.im_drafts.contains_key(previous)
                            && self.push.im_drafts.len() >= MAX_IM_DRAFTS
                        {
                            if let Some(victim) = self.push.im_drafts.keys().next().cloned() {
                                self.push.im_drafts.remove(&victim);
                            }
                        }
                        self.push.im_drafts.insert(previous.clone(), draft);
                    }
                }
            }
            if !thread_id.is_empty() {
                self.fields[F_IM_REPLY].set(
                    self.push
                        .im_drafts
                        .get(thread_id)
                        .map(String::as_str)
                        .unwrap_or(""),
                );
            }
        }
        if !thread_id.is_empty() {
            self.pending_sms_plan = None;
        }
        select_im_thread_state(
            &mut self.push.im_channel,
            &mut self.push.selected_im_thread,
            &mut self.selected_thread,
            &self.push.snapshot.threads,
            thread_id,
        );
        let active_tid = self
            .push
            .selected_im_thread
            .clone()
            .unwrap_or_else(|| thread_id.to_string());
        if !active_tid.is_empty() {
            let mut selected_meta: Option<(String, String, String, bool)> = None;
            let mut target_pkg = String::new();
            let mut target_canon = String::new();
            let mut target_nkey = String::new();
            if let Some(t) = self
                .push
                .snapshot
                .threads
                .iter_mut()
                .find(|t| t.thread_id == active_tid)
            {
                let clean_title =
                    push::clean_message_display_text(&push::normalize_conversation_title(&t.title));
                target_pkg = t.package_name.clone();
                target_canon = push::canonical_conversation_title_key(&t.title);
                target_nkey = t.notification_key.clone();
                let key1 = desktop_notify::NotifyPreferences::im_target_key(&t.thread_id);
                let key2 = format!("im:{}::{}", t.package_name.trim(), clean_title);
                let was_manual_unread = self.push.prefs.manual_unread_targets.remove(&key1)
                    | self.push.prefs.manual_unread_targets.remove(&key2);
                if was_manual_unread {
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                }
                let should_mark_read = t.unread_count > 0 || was_manual_unread;
                selected_meta = Some((
                    t.thread_id.clone(),
                    t.package_name.clone(),
                    t.title.clone(),
                    should_mark_read,
                ));
                t.unread_count = 0;
            }
            if !target_pkg.is_empty() {
                let mut alias_prefs_dirty = false;
                for other in &mut self.push.snapshot.threads {
                    if other.package_name == target_pkg
                        && (other.thread_id == active_tid
                            || other.thread_id == thread_id
                            || (!target_canon.is_empty()
                                && push::canonical_conversation_title_key(&other.title)
                                    == target_canon))
                    {
                        other.unread_count = 0;
                        let other_clean = push::clean_message_display_text(
                            &push::normalize_conversation_title(&other.title),
                        );
                        let k1 = desktop_notify::NotifyPreferences::im_target_key(&other.thread_id);
                        let k2 = format!("im:{}::{}", other.package_name.trim(), other_clean);
                        alias_prefs_dirty |= self.push.prefs.manual_unread_targets.remove(&k1);
                        alias_prefs_dirty |= self.push.prefs.manual_unread_targets.remove(&k2);
                    }
                }
                if alias_prefs_dirty {
                    let _ = desktop_notify::save_preferences(&self.push.prefs);
                }
            }
            if let Some(nkey) = active_tid.strip_prefix("notif_detail::") {
                self.mark_push_underlying_source_read(nkey);
            }
            for n in &mut self.push.snapshot.notifications {
                if n.thread_id == active_tid
                    || n.thread_id == thread_id
                    || (!target_nkey.is_empty() && n.key == target_nkey)
                    || (n.package_name == target_pkg
                        && !target_canon.is_empty()
                        && push::canonical_conversation_title_key(&n.title) == target_canon)
                {
                    n.read = true;
                }
            }
            if !self.demo_mode {
                if let (Some(device), Some((tid, pkg, ttl, should_mark_read))) =
                    (self.selected.clone(), selected_meta)
                {
                    let transport = device.transport.clone();
                    let serial = device.serial.clone();
                    let tx = self.push.outcome_tx.clone();
                    std::thread::spawn(move || {
                        if let Ok(adb) = Adb::discover() {
                            let transport =
                                device.resolve_active_transport(&adb).unwrap_or(transport);
                            if should_mark_read {
                                let _ = push::mark_im_thread_read(
                                    &adb, &transport, &tid, &pkg, &ttl, true,
                                );
                            }
                            if !tid.starts_with("notif_detail::") {
                                if let Ok(detail_thread) =
                                    push::fetch_thread_detail(&adb, &transport, &tid)
                                {
                                    if !detail_thread.messages.is_empty() {
                                        let _ = tx.send(PushOutcome::ThreadDetailFetched(
                                            serial,
                                            detail_thread,
                                        ));
                                    }
                                }
                            }
                        }
                    });
                }
            }
            self.sync_notification_center();
            Self::scroll_to_end(&self.sms_messages_scroll);
        }
    }

    pub(super) fn dispatch_im_reply(
        &mut self,
        thread_id: &str,
        package_name: &str,
        title: &str,
        text: &str,
        notif_key: Option<&str>,
    ) {
        if text.trim().is_empty() {
            return;
        }
        if self
            .push
            .active_reply_job
            .iter()
            .chain(self.push.reply_queue.iter())
            .any(|job| {
                job.thread_id == thread_id
                    && job.package_name == package_name
                    && job.text.trim() == text.trim()
            })
        {
            self.status = "这条回复仍在发送队列中，请等待手机确认".into();
            return;
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        self.push.next_client_msg_seq = self.push.next_client_msg_seq.wrapping_add(1);
        let client_msg_id = format!("stepx-{now_ms}-{}", self.push.next_client_msg_seq);
        if !self.demo_mode && self.selected.is_none() {
            self.status = "手机已断开，消息未发送".into();
            self.error = true;
            return;
        }
        let immediate_sent = self.demo_mode || self.selected.is_none();
        let (effective_tid, effective_title) = record_local_outgoing_attempt(
            &mut self.push.snapshot,
            thread_id,
            package_name,
            title,
            text,
            notif_key,
            &client_msg_id,
            now_ms,
            immediate_sent,
        );

        for n in &mut self.push.snapshot.notifications {
            if n.thread_id == effective_tid
                || n.thread_id == thread_id
                || notif_key.is_some_and(|k| n.key == k)
                || (n.package_name == package_name
                    && (n.title == effective_title || n.title == title))
            {
                n.read = true;
            }
        }
        self.push.active_toasts.retain(|t| {
            t.thread_id != effective_tid
                && t.thread_id != thread_id
                && notif_key.is_none_or(|k| t.notification_key != k)
        });
        self.sync_notification_center();
        Self::scroll_to_end(&self.sms_messages_scroll);

        if self.selected.is_none() {
            self.push.sending_thread = None;
            self.push.reply_in_flight = false;
            self.push.reply_error = None;
            self.push.last_reply_status = Some("已通过后台发送回复".to_string());
            self.status = format!("已通过后台发送回复（{effective_title}）");
            return;
        }

        if immediate_sent {
            self.push.sending_thread = None;
            self.push.reply_in_flight = false;
            self.push.reply_error = None;
            let status_msg = format!("已通过后台发送回复「{effective_title}」：{text}");
            self.push.last_reply_status = Some(status_msg.clone());
            self.status = status_msg;
            return;
        }

        if self.selected.is_some() {
            let clean_ttl = {
                let norm = push::normalize_conversation_title(&effective_title);
                if norm.is_empty() {
                    effective_title.trim().to_string()
                } else {
                    norm
                }
            };
            self.push.reply_queue.push_back(QueuedReplyJob {
                device_serial: self.selected.as_ref().unwrap().serial.clone(),
                client_msg_id,
                thread_id: effective_tid,
                package_name: package_name.to_string(),
                title: clean_ttl,
                text: text.to_string(),
                mode: self.push.reply_mode,
            });
            self.pump_reply_queue();
        }
    }

    pub(super) fn pump_reply_queue(&mut self) {
        if self.push.active_reply_job.is_some() {
            let waiting = format!(
                "已加入后台发送队列（待发 {} 条）…",
                self.push.reply_queue.len()
            );
            self.push.last_reply_status = Some(waiting.clone());
            self.status = waiting;
            return;
        }
        let Some(job) = self.push.reply_queue.pop_front() else {
            self.push.sending_thread = None;
            self.push.reply_in_flight = false;
            return;
        };
        let Some(device) = self.selected.clone() else {
            self.push.active_reply_job = None;
            self.push.sending_thread = None;
            self.push.reply_in_flight = false;
            self.push.reply_queue.clear();
            return;
        };
        if device.serial != job.device_serial {
            apply_reply_failure_for_attempt(
                &mut self.push.snapshot,
                &job.client_msg_id,
                "手机已切换，回复未发送",
            );
            self.push.reply_error = Some("手机已切换，旧手机的排队回复已停止".into());
            self.status = self.push.reply_error.clone().unwrap();
            self.error = true;
            self.push.sending_thread = None;
            self.push.reply_in_flight = false;
            self.push
                .reply_queue
                .retain(|queued| queued.device_serial == device.serial);
            return;
        }
        self.push.active_reply_job = Some(job.clone());
        self.push.sending_thread = Some(job.thread_id.clone());
        self.push.reply_in_flight = true;
        self.push.reply_error = None;
        let waiting = if self.push.reply_queue.is_empty() {
            "正在后台发送微信/应用回复…".to_string()
        } else {
            format!(
                "正在后台发送回复（待发 {} 条）…",
                self.push.reply_queue.len()
            )
        };
        self.push.last_reply_status = Some(waiting.clone());
        self.status = waiting;
        self.error = false;

        let tx = self.push.outcome_tx.clone();
        let serial = device.serial.clone();
        std::thread::spawn(move || {
            let res = Adb::discover()
                .and_then(|adb| {
                    let transport = device.resolve_active_transport(&adb)?;
                    anyhow::ensure!(
                        adb.shell(&transport, &["getprop", "ro.serialno"])?.trim()
                            == job.device_serial,
                        "手机传输连接已变化，回复未发送"
                    );
                    // reply_im owns the silent RemoteInput attempt and any
                    // explicit UI fallback. Opening first would wake/steal focus
                    // even when background reply is sufficient.
                    push::reply_im(
                        &adb,
                        &transport,
                        &job.thread_id,
                        &job.package_name,
                        &job.title,
                        &job.text,
                        job.mode,
                        &job.client_msg_id,
                    )
                })
                .map_err(|e| format!("{e:#}"));
            let _ = tx.send(PushOutcome::ReplyFinished(
                serial,
                job.thread_id,
                job.client_msg_id,
                res,
            ));
        });
    }

    fn finish_im_reply(
        &mut self,
        serial: &str,
        thread_id: &str,
        client_msg_id: &str,
        result: Result<ReplyReceipt, String>,
    ) {
        if !self.demo_mode
            && (self.selected.as_ref().is_none_or(|d| d.serial != serial)
                || !self.push.active_reply_job.as_ref().is_some_and(|job| {
                    job.device_serial == serial
                        && job.thread_id == thread_id
                        && job.client_msg_id == client_msg_id
                }))
        {
            return;
        }
        let result = result.and_then(|mut receipt| {
            if !receipt.ok
                || (!receipt.client_msg_id.is_empty() && receipt.client_msg_id != client_msg_id)
            {
                return Err("手机回执与本次回复不一致，发送结果未核实；请先查看手机会话".into());
            }
            if receipt.client_msg_id.is_empty() {
                receipt.client_msg_id = client_msg_id.into();
            }
            Ok(receipt)
        });
        match result {
            Ok(receipt) => {
                apply_reply_success_to_snapshot(&mut self.push.snapshot, &receipt);
                let message = if receipt.message.is_empty() || receipt.message == "已发送" {
                    "手机已确认发送回复".to_string()
                } else {
                    receipt.message
                };
                self.push.last_reply_status = Some(message.clone());
                self.push.reply_error = None;
                self.status = message;
                self.error = false;
            }
            Err(error) => {
                let restored =
                    apply_reply_failure_for_attempt(&mut self.push.snapshot, client_msg_id, &error);
                for (tid, text) in restored {
                    if self.push.selected_im_thread.as_ref() == Some(&tid)
                        && self.fields[F_IM_REPLY].text.is_empty()
                    {
                        self.fields[F_IM_REPLY].set(&text);
                    } else {
                        self.push.im_drafts.entry(tid).or_insert(text);
                    }
                }
                self.status = if reply_failure_is_uncertain(&error) {
                    format!("回复状态未核实：{error}；请先查看手机会话，避免重复发送")
                } else {
                    format!("回复未完成：{error}")
                };
                self.push.last_reply_status = Some(self.status.clone());
                self.push.reply_error = Some(error);
                self.error = true;
            }
        }
        self.push.active_reply_job = None;
        if !self.push.reply_queue.is_empty() {
            self.pump_reply_queue();
        } else {
            self.push.sending_thread = None;
            self.push.reply_in_flight = false;
        }
    }

    pub(super) fn request_push_snapshot(&mut self, force_full: bool) -> bool {
        if self.is_demo_device() {
            return false;
        }
        let Some(device) = self.selected.clone() else {
            return false;
        };
        let full = force_full || !self.push.loaded;
        if !self.push.begin_snapshot_read(full) {
            return false;
        }
        let tx = self.push.outcome_tx.clone();
        let serial = device.serial.clone();
        let transport = device.transport.clone();
        let since_revision = if full { 0 } else { self.push.snapshot.revision };
        std::thread::spawn(move || {
            let res = Adb::discover()
                .and_then(|adb| {
                    let transport = device.resolve_active_transport(&adb).unwrap_or(transport);
                    push::fetch_snapshot(&adb, &transport, since_revision)
                })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(PushOutcome::Snapshot(serial, res));
        });
        true
    }

    fn acknowledge_push_snapshot(&mut self, serial: &str) -> bool {
        if !self.demo_mode && self.selected.as_ref().is_none_or(|device| device.serial != serial) {
            return false;
        }
        // A late reply from a previously selected phone does not finish the
        // current phone's in-flight read or permit another concurrent poll.
        self.push.syncing = false;
        self.push.snapshot_in_flight_full = false;
        true
    }

    pub(super) fn handle_push_outcome(
        &mut self,
        outcome: PushOutcome,
        ctx: &mut ViewContext<Self>,
    ) {
        match outcome {
            PushOutcome::Snapshot(serial, res) => {
                if !self.acknowledge_push_snapshot(&serial) {
                    return;
                }
                match res {
                    Ok(mut next) => {
                        if let Some(battery) = &next.battery_state {
                            self.observe_battery(&serial, battery);
                        }
                        push::merge_local_replies(&mut next, &self.push.snapshot);
                        push::consolidate_im_threads(&mut next.threads, &next.notifications);
                        apply_push_preferences_to_snapshot(&mut next, &mut self.push.prefs);
                        self.push.cap_seen_toast_keys(&next);
                        let active_thread_id = if self.push.shell_mode == ShellMode::Full
                            && self.page == Page::Sms
                            && !self.settings.hidden
                            && ctx.windows().active_window() == Some(ctx.window_id())
                        {
                            self.push.selected_im_thread.as_deref()
                        } else {
                            None
                        };
                        let new_toasts = desktop_notify::detect_new_toasts(
                            self.push.snapshot.revision,
                            &mut self.push.seen_toast_keys,
                            &next,
                            &self.push.prefs,
                            active_thread_id,
                        );
                        self.push.enforce_seen_toast_keys_bound(&next);
                        let now_ms = chrono::Local::now().timestamp_millis();
                        let allow_in_app_toasts = self.notification_center.alert_mode
                            == super::notification_center_state::NotificationAlertMode::PopupAndBadge
                            && !self.notification_center.is_snoozed(now_ms)
                            && (self.push.prefs.popup_mode.allows_popup()
                                || self
                                    .push
                                    .prefs
                                    .app_popup_modes
                                    .values()
                                    .any(|m| m.allows_popup()));
                        if !allow_in_app_toasts {
                            self.push.active_toasts.clear();
                        }
                        for toast in new_toasts {
                            if !self.toast_is_current(&toast, now_ms) {
                                continue;
                            }
                            if self.push.prefs.auto_copy_otp {
                                if let Some(code) = &toast.otp_code {
                                    ctx.clipboard()
                                        .write(ClipboardContent::plain_text(code.clone()));
                                    self.status = format!("已自动复制验证码 {code} 到剪贴板");
                                }
                            }
                            // Unified 360-style single-window popup replaces OS CLI toasts
                            // (`desktop_notify::send_system_notification`) during snapshot polling
                            // so users never get duplicate popups + dock icon bouncing.
                            if allow_in_app_toasts
                                && self
                                    .push
                                    .prefs
                                    .effective_popup_mode(&toast.package_name)
                                    .allows_popup()
                            {
                                self.push.active_toasts.retain(|existing| {
                                    (!toast.thread_id.is_empty()
                                        && existing.thread_id != toast.thread_id)
                                        || (toast.thread_id.is_empty()
                                            && existing.notification_key != toast.notification_key)
                                });
                                self.push.active_toasts.insert(0, toast);
                                if self.push.active_toasts.len() > 4 {
                                    self.push.active_toasts.truncate(4);
                                }
                            }
                        }

                        // Multi-device read/dismiss sync: clear desktop toasts whose notification was dismissed/read on phone.
                        self.push.active_toasts.retain(|t| {
                            let dismissed_on_phone = next
                                .notifications
                                .iter()
                                .find(|n| n.key == t.notification_key)
                                .is_some_and(|n| n.dismissed);
                            !dismissed_on_phone
                        });

                        let newest_im_ts = next
                            .threads
                            .iter()
                            .map(|t| t.last_timestamp_ms)
                            .max()
                            .unwrap_or(0);
                        let newest_sms_ts =
                            self.sms.iter().map(|m| m.date_ms as i64).max().unwrap_or(0);
                        let explicit_sms_selection =
                            self.push.selected_im_thread.as_deref() == Some("");
                        let viewing_sms = self.push.im_channel == ImChannel::Sms
                            || (self.selected_thread.is_some()
                                && (explicit_sms_selection || newest_sms_ts >= newest_im_ts));
                        let selected_valid = viewing_sms
                            || self.push.selected_im_thread.as_ref().is_some_and(|sel| {
                                !sel.is_empty()
                                    && (sel.starts_with("notif_detail::")
                                        || next.threads.iter().any(|t| &t.thread_id == sel))
                            });
                        if !selected_valid {
                            if !explicit_sms_selection && self.push.im_channel != ImChannel::Sms {
                                self.selected_thread = None;
                            }
                            let env_sel = std::env::var("STEPX_SELECT_IM_THREAD").ok();
                            let pkg_prefix = self
                                .push
                                .selected_im_thread
                                .as_deref()
                                .and_then(|s| s.split("::").next())
                                .unwrap_or("");
                            self.push.selected_im_thread = env_sel
                                .as_deref()
                                .filter(|q| !q.trim().is_empty())
                                .and_then(|q| {
                                    next.threads
                                        .iter()
                                        .find(|t| t.thread_id == q || t.title.contains(q))
                                })
                                .or_else(|| {
                                    next.threads.iter().find(|t| {
                                        !pkg_prefix.is_empty() && t.package_name == pkg_prefix
                                    })
                                })
                                .or_else(|| next.threads.first())
                                .map(|t| t.thread_id.clone());
                        }
                        if self.push.selected_key.is_none() {
                            self.push.selected_key =
                                next.notifications.first().map(|n| n.key.clone());
                        }
                        if self.push.expanded_mini_card.is_none() {
                            self.push.expanded_mini_card = next
                                .threads
                                .iter()
                                .find(|t| t.unread_count > 0)
                                .or_else(|| next.threads.first())
                                .map(|t| t.thread_id.clone());
                        }
                        self.push.snapshot = next;
                        self.push.loaded = true;
                        self.push.error = None;
                        self.sync_live_express_parcels();
                        if self.back_screen.gallery_items.is_empty() && self.selected.is_some() {
                            self.handle_backscreen_action(
                                &crate::ui::BackScreenAction::GalleryRefresh,
                                ctx,
                            );
                        }
                        self.reconcile_auto_sync_domains_from_state();
                        self.sync_notification_center();
                        if let Some(popup) = self.notification_active_popup().cloned() {
                            self.push.active_toasts.clear();
                            if self.focus == Some(F_TOAST_REPLY) {
                                self.push.active_toasts.push(DesktopToastItem {
                                    id: popup.id,
                                    timestamp_ms: now_ms,
                                    can_reply: true,
                                    ..Default::default()
                                });
                            }
                        }
                        self.sync_desktop_popup_window(ctx);
                    }
                    Err(err) => {
                        self.push.error = Some(err);
                        self.reconcile_auto_sync_domains_from_state();
                    }
                }
            }
            PushOutcome::ReplyFinished(serial, thread_id, client_msg_id, res) => {
                self.finish_im_reply(&serial, &thread_id, &client_msg_id, res);
            }
            PushOutcome::ActiveWindowSynced(serial, res) => {
                if !self.demo_mode && self.selected.as_ref().is_none_or(|d| d.serial != serial) {
                    return;
                }
                match res {
                    Ok(Some(mut thread)) => {
                        let now_ms = chrono::Local::now().timestamp_millis();
                        let norm_title = push::normalize_conversation_title(&thread.title);
                        let existing_msgs: std::collections::HashSet<(bool, String)> = self
                            .push
                            .snapshot
                            .threads
                            .iter()
                            .filter(|t| {
                                t.thread_id == thread.thread_id
                                    || (t.package_name == thread.package_name
                                        && !norm_title.is_empty()
                                        && push::normalize_conversation_title(&t.title)
                                            == norm_title)
                            })
                            .flat_map(|t| &t.messages)
                            .map(|m| (m.outgoing, m.text.trim().to_string()))
                            .collect();
                        for msg in &mut thread.messages {
                            let already_known = existing_msgs
                                .contains(&(msg.outgoing, msg.text.trim().to_string()));
                            if !already_known && !msg.outgoing && msg.timestamp_ms <= 0 {
                                msg.timestamp_ms = now_ms;
                            }
                        }
                        let merged = push::merge_synced_window_into_snapshot(
                            &mut self.push.snapshot,
                            thread,
                        );
                        apply_push_preferences_to_snapshot(
                            &mut self.push.snapshot,
                            &mut self.push.prefs,
                        );
                        let tid = merged.thread_id.clone();
                        let title = merged.title.clone();
                        let count = merged.messages.len();
                        self.push.selected_im_thread = Some(tid);
                        self.status =
                            format!("已同步手机当前聊天窗口「{title}」（{count} 条消息）");
                        self.error = false;
                        Self::scroll_to_end(&self.sms_messages_scroll);
                        self.sync_notification_center();
                        self.sync_desktop_popup_window(ctx);
                    }
                    Ok(None) => {
                        if let Some(first) = self.push.snapshot.threads.first() {
                            let tid = first.thread_id.clone();
                            let title = first.title.clone();
                            let count = first.messages.len();
                            self.push.selected_im_thread = Some(tid);
                            self.status = format!(
                                "手机当前处于锁屏或非聊天界面，已展示最新会话「{title}」（{count} 条消息）"
                            );
                            self.error = false;
                        } else {
                            self.status = "手机当前前台未处于受支持的聊天窗口".into();
                        }
                    }
                    Err(err) => {
                        self.status = format!("抓取手机当前聊天窗口失败：{err}");
                    }
                }
            }
            PushOutcome::ThreadDetailFetched(serial, mut detail_thread) => {
                if !self.demo_mode && self.selected.as_ref().is_none_or(|d| d.serial != serial) {
                    return;
                }
                if !detail_thread.messages.is_empty() {
                    detail_thread.unread_count = 0;
                    let _ = push::merge_synced_window_into_snapshot(
                        &mut self.push.snapshot,
                        detail_thread,
                    );
                    apply_push_preferences_to_snapshot(
                        &mut self.push.snapshot,
                        &mut self.push.prefs,
                    );
                    self.sync_notification_center();
                }
            }
            PushOutcome::NotificationDismissed(_serial, key, res) => {
                if let Err(err) = res {
                    self.status = format!("清除通知 {key} 未完成：{err}");
                }
            }
            PushOutcome::AllDismissed(_serial, res) => match res {
                Ok(count) => {
                    self.status = format!("已同步清除手机通知栏 {count} 条通知");
                }
                Err(err) => {
                    self.status = format!("批量清除通知失败：{err}");
                }
            },
            PushOutcome::PermissionEnsured(serial, res) => match res {
                Ok(connected) => {
                    self.push.snapshot.listener_connected = connected;
                    self.status = if connected {
                        "已开启手机通知监听与同 Wi-Fi 秒回权限".into()
                    } else {
                        "权限命令已执行，请在手机上确认开启 StepX伴侣通知监听".into()
                    };
                    if self.selected.as_ref().is_some_and(|device| device.serial == serial) {
                        self.request_push_snapshot(true);
                    }
                }
                Err(err) => {
                    self.status = format!("开启通知监听权限失败：{err}");
                }
            },
        }
    }

    pub(super) fn tick_push_sync(&mut self, ctx: &mut ViewContext<Self>) {
        // Prune independently of snapshot revisions: an unchanged or disconnected
        // phone must not leave an old in-app overlay covering the workspace.
        let mut changed = self.prune_expired_toasts(chrono::Local::now().timestamp_millis());
        if let Some(code) = self.maybe_auto_copy_sms_otp() {
            ctx.clipboard()
                .write(ClipboardContent::plain_text(code.clone()));
            self.status = format!("已自动复制验证码 {code} 到剪贴板");
            changed = true;
        }
        while let Ok(mut outcome) = self.push.outcome_rx.try_recv() {
            if let PushOutcome::Snapshot(serial, _) = &outcome {
                if !self.acknowledge_push_snapshot(serial) {
                    continue;
                }
            }
            if let PushOutcome::Snapshot(serial, Ok(next)) = &mut outcome {
                // Refresh receipt time even when data/revision is unchanged.
                if let Some(battery) = &next.battery_state {
                    changed |= self.observe_battery(serial, battery);
                }
                if self.push.loaded
                    && self.push.error.is_none()
                    && self.selected.as_ref().is_some_and(|d| d.serial == *serial)
                {
                    push::merge_local_replies(next, &self.push.snapshot);
                    push::consolidate_im_threads(&mut next.threads, &next.notifications);
                    apply_push_preferences_to_snapshot(next, &mut self.push.prefs);
                    if *next == self.push.snapshot {
                        self.push.syncing = false;
                        continue;
                    }
                }
            }
            self.handle_push_outcome(outcome, ctx);
            changed = true;
        }
        if !self.is_demo_device()
            && !self.push.syncing
            && (self.push.full_refresh_requested || self.push.last_sync.elapsed()
                >= Duration::from_millis(if self.push.error.is_some() {
                    10_000
                } else if matches!(self.page, Page::Notifications | Page::Sms | Page::Home) {
                    2500
                } else {
                    5000
                }))
        {
            self.request_push_snapshot(self.push.full_refresh_requested);
        }
        if changed {
            ctx.notify();
        }
    }

    fn toast_is_current(&self, toast: &DesktopToastItem, now_ms: i64) -> bool {
        self.demo_mode
            || self
                .notification_center
                .is_current_popup_time(toast.timestamp_ms, now_ms)
    }

    fn prune_expired_toasts(&mut self, now_ms: i64) -> bool {
        let previous = self.push.active_toasts.len();
        let center = &self.notification_center;
        let demo = self.demo_mode;
        self.push
            .active_toasts
            .retain(|toast| demo || center.is_current_popup_time(toast.timestamp_ms, now_ms));
        self.push.active_toasts.len() != previous
    }
}

#[allow(clippy::too_many_arguments)]
pub(crate) fn record_local_outgoing_attempt(
    snapshot: &mut PushSnapshot,
    thread_id: &str,
    package_name: &str,
    title: &str,
    text: &str,
    notif_key: Option<&str>,
    client_msg_id: &str,
    now_ms: i64,
    immediate_sent: bool,
) -> (String, String) {
    let new_msg = ImMessage {
        id: client_msg_id.to_string(),
        sender: "我".into(),
        text: text.to_string(),
        timestamp_ms: now_ms,
        outgoing: true,
        delivery_status: if immediate_sent {
            "sent".into()
        } else {
            "sending".into()
        },
        via: "remote_input".into(),
        media_hint: String::new(),
        client_msg_id: client_msg_id.to_string(),
        observed_at_ms: now_ms,
        time_known: Some(true),
        recalled: false,
        ..Default::default()
    };

    let mut effective_tid = thread_id.to_string();
    let mut effective_title = title.to_string();

    let norm_title = push::normalize_conversation_title(title);
    let target_idx = snapshot
        .threads
        .iter()
        .position(|t| t.thread_id == thread_id && t.package_name == package_name)
        .or_else(|| {
            snapshot.threads.iter().position(|t| {
                t.package_name == package_name
                    && !norm_title.is_empty()
                    && !push::is_generic_im_title(&norm_title)
                    && push::normalize_conversation_title(&t.title) == norm_title
            })
        });

    if let Some(idx) = target_idx {
        let thread = &mut snapshot.threads[idx];
        if !thread.thread_id.is_empty() {
            effective_tid = thread.thread_id.clone();
        }
        if push::title_specificity_rank(&thread.title)
            > push::title_specificity_rank(&effective_title)
        {
            effective_title = thread.title.clone();
        }
        // Only remove previous failed messages with identical text; keep other in-flight "sending" messages intact.
        thread.messages.retain(|m| {
            !(m.outgoing && m.delivery_status == "failed" && m.text.trim() == text.trim())
        });
        thread.messages.push(new_msg);
        push::deduplicate_thread_messages(&mut thread.messages, &thread.title);
        if immediate_sent {
            thread.last_sender = "我".into();
            thread.last_message = text.to_string();
            thread.last_timestamp_ms = now_ms;
        } else {
            push::recompute_thread_summary(thread);
        }
        thread.unread_count = 0;
    } else {
        let ch = ImChannel::from_package(package_name);
        snapshot.threads.insert(
            0,
            ImThread {
                thread_id: effective_tid.clone(),
                package_name: package_name.to_string(),
                app_name: ch.label().into(),
                title: effective_title.clone(),
                is_group: false,
                unread_count: 0,
                last_sender: "我".into(),
                last_message: text.to_string(),
                last_timestamp_ms: now_ms,
                can_remote_reply: true,
                can_ui_reply: true,
                has_content_intent: true,
                notification_key: notif_key.unwrap_or_default().to_string(),
                messages: vec![new_msg],
                ..Default::default()
            },
        );
    }

    (effective_tid, effective_title)
}

pub(crate) fn apply_push_preferences_to_snapshot(
    snapshot: &mut PushSnapshot,
    prefs: &mut NotifyPreferences,
) {
    let mut prefs_dirty = false;
    snapshot.threads.retain_mut(|thread| {
        let clean_title =
            push::clean_message_display_text(&push::normalize_conversation_title(&thread.title));
        let key1 = desktop_notify::NotifyPreferences::im_target_key(&thread.thread_id);
        let key2 = format!("im:{}::{}", thread.package_name.trim(), clean_title);
        let sms_key = (thread.channel() == ImChannel::Sms && !clean_title.is_empty())
            .then(|| desktop_notify::NotifyPreferences::sms_target_key(&clean_title));

        let deleted_at = prefs
            .deleted_threads
            .get(&key1)
            .copied()
            .or_else(|| {
                (!clean_title.is_empty())
                    .then(|| prefs.deleted_threads.get(&key2).copied())
                    .flatten()
            })
            .or_else(|| {
                sms_key
                    .as_ref()
                    .and_then(|sk| prefs.deleted_threads.get(sk).copied())
            });
        if let Some(del_ts) = deleted_at {
            let newest_ts = thread.last_timestamp_ms.max(
                thread
                    .messages
                    .iter()
                    .map(|m| m.timestamp_ms)
                    .max()
                    .unwrap_or(0),
            );
            if newest_ts > del_ts {
                prefs.deleted_threads.remove(&key1);
                prefs.deleted_threads.remove(&key2);
                if let Some(sk) = &sms_key {
                    prefs.deleted_threads.remove(sk);
                }
                prefs_dirty = true;
                thread
                    .messages
                    .retain(|m| m.timestamp_ms <= 0 || m.timestamp_ms > del_ts);
            } else {
                return false;
            }
        }

        let cleared_at = prefs
            .cleared_threads
            .get(&key1)
            .copied()
            .or_else(|| {
                (!clean_title.is_empty())
                    .then(|| prefs.cleared_threads.get(&key2).copied())
                    .flatten()
            })
            .or_else(|| {
                sms_key
                    .as_ref()
                    .and_then(|sk| prefs.cleared_threads.get(sk).copied())
            });
        if let Some(clr_ts) = cleared_at {
            thread.messages.retain(|m| m.timestamp_ms > clr_ts);
            if thread.messages.is_empty() && thread.last_timestamp_ms <= clr_ts {
                thread.last_message.clear();
                thread.last_sender.clear();
                thread.unread_count = 0;
            }
        }

        let tid_trim = thread.thread_id.trim().to_string();
        let before_len = thread.messages.len();
        thread.messages.retain(|m| {
            let id_key = format!("im_msg:{}::{}", tid_trim, m.id.trim());
            let cid_key = format!("im_msg:{}::{}", tid_trim, m.client_msg_id.trim());
            let txt_key = format!("im_txt:{}::{}", tid_trim, m.text.trim());
            !prefs.deleted_messages.contains(&id_key)
                && (m.client_msg_id.trim().is_empty() || !prefs.deleted_messages.contains(&cid_key))
                && !prefs.deleted_messages.contains(&txt_key)
        });
        if thread.messages.len() != before_len {
            if thread.messages.is_empty() {
                thread.last_message.clear();
                thread.last_sender.clear();
            } else {
                push::recompute_thread_summary(thread);
            }
        }

        if prefs.manual_unread_targets.contains(&key1)
            || (!clean_title.is_empty() && prefs.manual_unread_targets.contains(&key2))
        {
            thread.unread_count = thread.unread_count.max(1);
        }

        true
    });

    snapshot.otp_banners.retain(|b| {
        let code_up = b.code.trim().to_uppercase();
        code_up.is_empty() || !prefs.dismissed_pickup_codes.contains(&code_up)
    });

    for n in &mut snapshot.notifications {
        let clean_title =
            push::clean_message_display_text(&push::normalize_conversation_title(&n.title));
        let key1 = desktop_notify::NotifyPreferences::im_target_key(&n.thread_id);
        let key2 = format!("im:{}::{}", n.package_name.trim(), clean_title);
        let sms_key = (!clean_title.is_empty())
            .then(|| desktop_notify::NotifyPreferences::sms_target_key(&clean_title));
        let deleted_at = prefs
            .deleted_threads
            .get(&key1)
            .copied()
            .or_else(|| {
                (!clean_title.is_empty())
                    .then(|| prefs.deleted_threads.get(&key2).copied())
                    .flatten()
            })
            .or_else(|| {
                sms_key
                    .as_ref()
                    .and_then(|sk| prefs.deleted_threads.get(sk).copied())
            });
        if let Some(del_ts) = deleted_at {
            if n.post_time_ms <= del_ts {
                n.dismissed = true;
                n.read = true;
            }
        }
        let otp_up = n.otp_code.as_deref().unwrap_or("").trim().to_uppercase();
        if !otp_up.is_empty() && prefs.dismissed_pickup_codes.contains(&otp_up) {
            n.dismissed = true;
            n.read = true;
        }
    }

    if prefs_dirty {
        let _ = desktop_notify::save_preferences(prefs);
    }
}

pub(crate) fn apply_reply_success_to_snapshot(
    snapshot: &mut PushSnapshot,
    receipt: &push::ReplyReceipt,
) {
    let receipt_pkg_title = receipt
        .thread_id
        .split_once("::")
        .map(|(pkg, raw_title)| (pkg.trim(), push::normalize_conversation_title(raw_title)));
    if let Some(thread) = snapshot.threads.iter_mut().find(|t| {
        (!receipt.thread_id.is_empty() && t.thread_id == receipt.thread_id)
            || t.messages.iter().any(|m| {
                !receipt.client_msg_id.is_empty()
                    && (m.id == receipt.client_msg_id || m.client_msg_id == receipt.client_msg_id)
            })
            || receipt_pkg_title.as_ref().is_some_and(|(pkg, norm_title)| {
                !norm_title.is_empty()
                    && t.package_name == *pkg
                    && push::normalize_conversation_title(&t.title) == *norm_title
            })
    }) {
        let mut matched = false;
        for m in thread.messages.iter_mut().rev() {
            if m.outgoing
                && ((!receipt.client_msg_id.is_empty()
                    && (m.id == receipt.client_msg_id || m.client_msg_id == receipt.client_msg_id))
                    || (receipt.client_msg_id.is_empty() && m.delivery_status == "sending"))
            {
                m.delivery_status = match receipt.delivery_status.as_str() {
                    "sent" | "delivered" | "sending" | "unknown" => receipt.delivery_status.clone(),
                    _ => "sent".into(),
                };
                m.sender = "我".into();
                if !receipt.via.is_empty() {
                    m.via = receipt.via.clone();
                }
                if receipt.timestamp_ms > 0 {
                    m.timestamp_ms = m.timestamp_ms.max(receipt.timestamp_ms);
                    m.time_known = Some(true);
                }
                matched = true;
                break;
            }
        }
        if !matched {
            for m in thread.messages.iter_mut().rev() {
                if m.outgoing && m.delivery_status == "sending" {
                    m.delivery_status = match receipt.delivery_status.as_str() {
                        "sent" | "delivered" | "sending" | "unknown" => {
                            receipt.delivery_status.clone()
                        }
                        _ => "sent".into(),
                    };
                    m.sender = "我".into();
                    if !receipt.via.is_empty() {
                        m.via = receipt.via.clone();
                    }
                    if receipt.timestamp_ms > 0 {
                        m.timestamp_ms = m.timestamp_ms.max(receipt.timestamp_ms);
                        m.time_known = Some(true);
                    }
                    break;
                }
            }
        }
        thread.unread_count = 0;
        thread
            .messages
            .retain(|m| !(m.outgoing && m.delivery_status == "failed"));
        push::deduplicate_thread_messages(&mut thread.messages, &thread.title);
        push::recompute_thread_summary(thread);
    }
}

pub(crate) fn apply_reply_failure_to_snapshot(
    snapshot: &mut PushSnapshot,
    err: &str,
) -> Vec<(String, String)> {
    apply_reply_failure_matching(snapshot, None, err)
}

fn apply_reply_failure_for_attempt(
    snapshot: &mut PushSnapshot,
    client_msg_id: &str,
    err: &str,
) -> Vec<(String, String)> {
    apply_reply_failure_matching(snapshot, Some(client_msg_id), err)
}

fn apply_reply_failure_matching(
    snapshot: &mut PushSnapshot,
    client_msg_id: Option<&str>,
    err: &str,
) -> Vec<(String, String)> {
    let is_uncertain = reply_failure_is_uncertain(err);
    let mut restored = Vec::new();
    for thread in &mut snapshot.threads {
        let mut restored_text: Option<String> = None;
        for m in thread.messages.iter_mut().rev() {
            if m.outgoing
                && m.delivery_status == "sending"
                && client_msg_id.is_none_or(|id| m.id == id || m.client_msg_id == id)
            {
                restored_text = Some(m.text.clone());
                if is_uncertain {
                    m.delivery_status = "unknown".into();
                } else {
                    m.delivery_status = "failed".into();
                }
                break;
            }
        }
        if let Some(draft_text) = restored_text {
            push::deduplicate_thread_messages(&mut thread.messages, &thread.title);
            push::recompute_thread_summary(thread);
            restored.push((thread.thread_id.clone(), draft_text));
        }
    }
    restored
}

fn reply_failure_is_uncertain(error: &str) -> bool {
    if error.contains("未发送") {
        return false;
    }
    let lower = error.to_ascii_lowercase();
    [
        "未核实",
        "超时",
        "timed out",
        "timeout",
        "connection",
        "broken pipe",
        "socket",
        "通信",
        "网络",
    ]
    .iter()
    .any(|part| lower.contains(part))
}

pub(crate) fn select_im_channel_state(
    im_channel: &mut ImChannel,
    selected_im_thread: &mut Option<String>,
    selected_sms_thread: &mut Option<String>,
    threads: &[ImThread],
    first_sms_addr: Option<&str>,
    ch: ImChannel,
) {
    let is_visible_im_thread = |t: &ImThread| -> bool {
        !push::is_excluded_im_package(&t.package_name)
            && pages::sms::im_thread_has_visible_content(t)
            && !t.thread_id.contains("stepx_e2e_test")
            && !t.thread_id.contains("stepx_test_tag")
    };
    if ch == ImChannel::Sms {
        *im_channel = ImChannel::Sms;
        *selected_im_thread = Some(String::new());
        if selected_sms_thread.is_none() {
            *selected_sms_thread = first_sms_addr.map(ToString::to_string);
        }
    } else {
        *im_channel = ch;
        *selected_sms_thread = None;
        let current_matches = selected_im_thread
            .as_ref()
            .filter(|id| !id.is_empty())
            .and_then(|id| threads.iter().find(|t| &t.thread_id == id))
            .is_some_and(|t| is_visible_im_thread(t) && ch.matches_package(&t.package_name));
        if !current_matches {
            *selected_im_thread = threads
                .iter()
                .find(|t| is_visible_im_thread(t) && ch.matches_package(&t.package_name))
                .map(|t| t.thread_id.clone());
        }
        if selected_im_thread.is_none() && ch == ImChannel::All {
            *selected_sms_thread = first_sms_addr.map(ToString::to_string);
        }
    }
}

pub(crate) fn select_im_thread_state(
    im_channel: &mut ImChannel,
    selected_im_thread: &mut Option<String>,
    selected_sms_thread: &mut Option<String>,
    threads: &[ImThread],
    thread_id: &str,
) {
    let resolved_id = if thread_id.is_empty() || threads.iter().any(|t| t.thread_id == thread_id) {
        thread_id.to_string()
    } else if let Some((pkg, raw_title)) = thread_id.split_once("::") {
        let norm_title = push::normalize_conversation_title(raw_title);
        threads
            .iter()
            .find(|t| {
                t.package_name == pkg
                    && (t.title == raw_title
                        || (!norm_title.is_empty()
                            && push::normalize_conversation_title(&t.title) == norm_title))
            })
            .map(|t| t.thread_id.clone())
            .unwrap_or_else(|| thread_id.to_string())
    } else {
        thread_id.to_string()
    };

    if !resolved_id.is_empty() {
        *selected_sms_thread = None;
        let incompatible_channel = *im_channel == ImChannel::Sms
            || threads
                .iter()
                .find(|t| t.thread_id == resolved_id)
                .is_some_and(|t| !im_channel.matches_package(&t.package_name));
        if incompatible_channel {
            *im_channel = ImChannel::All;
        }
    }
    *selected_im_thread = Some(resolved_id);
}

pub(crate) fn select_sms_thread_state(
    im_channel: &mut ImChannel,
    selected_im_thread: &mut Option<String>,
    selected_sms_thread: &mut Option<String>,
    thread: Option<&str>,
) {
    match thread {
        Some(addr) => {
            if !matches!(*im_channel, ImChannel::All | ImChannel::Sms) {
                *im_channel = ImChannel::Sms;
            }
            *selected_im_thread = Some(String::new());
            *selected_sms_thread = Some(addr.to_string());
        }
        None => {
            *selected_sms_thread = None;
            if selected_im_thread.as_deref() == Some("") {
                *selected_im_thread = None;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stepx_companion::push::{ImMessage, ImThread, PushSnapshot, ReplyReceipt};

    #[test]
    fn another_phones_late_snapshot_cannot_release_the_current_read() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(Device { serial: "CURRENT_PUSH165_PHONE".into(), ..Default::default() });
        app.push.syncing = true;
        assert!(!app.acknowledge_push_snapshot("PREVIOUS_PUSH165_PHONE"));
        assert!(app.push.syncing);
        assert!(app.acknowledge_push_snapshot("CURRENT_PUSH165_PHONE"));
        assert!(!app.push.syncing);
    }

    #[test]
    fn a_disconnected_phones_late_snapshot_does_not_mutate_sync_state() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = None;
        app.push.syncing = true;
        assert!(!app.acknowledge_push_snapshot("DISCONNECTED_PUSH165_PHONE"));
        assert!(app.push.syncing);
    }

    #[test]
    fn manual_refresh_joins_full_reads_and_queues_one_full_read_after_an_incremental() {
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(Device { serial: "COALESCED_PUSH165_PHONE".into(), ..Default::default() });
        assert!(app.push.begin_snapshot_read(false));
        for _ in 0..3 {
            assert!(!app.push.begin_snapshot_read(true));
        }
        assert!(app.push.syncing && app.push.full_refresh_requested);
        assert!(app.acknowledge_push_snapshot("COALESCED_PUSH165_PHONE"));
        assert!(!app.push.syncing && app.push.full_refresh_requested);
        assert!(app.push.begin_snapshot_read(true));
        assert!(!app.push.full_refresh_requested);
        for _ in 0..3 {
            assert!(!app.push.begin_snapshot_read(true));
        }
        assert!(!app.push.full_refresh_requested, "in-flight full reads are already the requested refresh");
        assert!(app.acknowledge_push_snapshot("COALESCED_PUSH165_PHONE"));
        assert!(!app.push.syncing && !app.push.full_refresh_requested);
    }

    #[test]
    fn in_app_toasts_share_popup_freshness_and_expire_without_new_snapshots() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::ui::TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        let now = chrono::Local::now().timestamp_millis();
        app.notification_center.popup_session_started_ms = now - 1_000;
        let old = DesktopToastItem {
            id: "old".into(),
            timestamp_ms: now - 600_001,
            ..Default::default()
        };
        let fresh = DesktopToastItem {
            id: "fresh".into(),
            timestamp_ms: now,
            ..Default::default()
        };
        assert!(!app.toast_is_current(&old, now));
        assert!(app.toast_is_current(&fresh, now));
        app.push.active_toasts = vec![old, fresh];
        app.push.snapshot.threads = vec![ImThread {
            thread_id: "unread".into(),
            unread_count: 3,
            ..Default::default()
        }];
        let read_ids = app.notification_center.read_ids.clone();
        assert!(app.prune_expired_toasts(now));
        assert_eq!(app.push.active_toasts.len(), 1);
        assert_eq!(app.push.active_toasts[0].id, "fresh");
        assert!(app.prune_expired_toasts(now + 600_001));
        assert!(app.push.active_toasts.is_empty());
        assert_eq!(app.push.snapshot.threads[0].unread_count, 3);
        assert_eq!(app.notification_center.read_ids, read_ids);
    }

    fn pending_reply_app() -> (
        tempfile::TempDir,
        crate::ui::TestUiConfigDirGuard,
        Companion,
    ) {
        let dir = tempfile::tempdir().unwrap();
        let guard = crate::ui::TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.demo_mode = false;
        app.selected = Some(Device {
            serial: "phone-a".into(),
            transport: "phone-a".into(),
            ..Default::default()
        });
        app.push.snapshot = PushSnapshot::default();
        app.fields[F_IM_REPLY].clear();
        app.push.selected_im_thread = Some("thread-a".into());
        for (thread, id, text) in [
            ("thread-a", "client-a", "first reply"),
            ("thread-b", "client-b", "other pending reply"),
        ] {
            record_local_outgoing_attempt(
                &mut app.push.snapshot,
                thread,
                "com.tencent.mm",
                thread,
                text,
                None,
                id,
                1000,
                false,
            );
        }
        app.push.active_reply_job = Some(QueuedReplyJob {
            client_msg_id: "client-a".into(),
            device_serial: "phone-a".into(),
            thread_id: "thread-a".into(),
            package_name: "com.tencent.mm".into(),
            title: "thread-a".into(),
            text: "first reply".into(),
            mode: ReplyMode::Auto,
        });
        app.push.reply_in_flight = true;
        (dir, guard, app)
    }

    #[test]
    fn connected_send_failure_restores_only_its_own_draft_and_never_claims_delivery() {
        let (_dir, _guard, mut app) = pending_reply_app();
        app.finish_im_reply(
            "phone-a",
            "thread-a",
            "client-a",
            Err("未找到发送按钮".into()),
        );
        assert!(app.error && app.push.reply_error.is_some());
        assert_eq!(app.fields[F_IM_REPLY].text, "first reply");
        let state = |id: &str| {
            app.push
                .snapshot
                .threads
                .iter()
                .flat_map(|t| &t.messages)
                .find(|m| m.client_msg_id == id)
                .unwrap()
                .delivery_status
                .as_str()
        };
        assert_eq!(state("client-a"), "failed");
        assert_eq!(state("client-b"), "sending");
        assert!(!app.status.contains("已通过后台发送"));
    }

    #[test]
    fn late_or_mismatched_reply_receipts_do_not_acknowledge_another_message() {
        let (_dir, _guard, mut app) = pending_reply_app();
        let receipt = ReplyReceipt {
            ok: true,
            thread_id: "thread-a".into(),
            client_msg_id: "different-client".into(),
            ..Default::default()
        };
        app.finish_im_reply("phone-b", "thread-a", "client-a", Ok(receipt.clone()));
        assert!(app.push.active_reply_job.is_some());
        assert_eq!(
            app.push
                .snapshot
                .threads
                .iter()
                .find(|t| t.thread_id == "thread-a")
                .unwrap()
                .messages[0]
                .delivery_status,
            "sending"
        );
        app.finish_im_reply("phone-a", "thread-a", "client-a", Ok(receipt));
        assert!(app.error);
        assert_eq!(
            app.push
                .snapshot
                .threads
                .iter()
                .find(|t| t.thread_id == "thread-a")
                .unwrap()
                .messages[0]
                .delivery_status,
            "unknown"
        );
        assert!(app.status.contains("未核实"));
    }

    #[test]
    fn queued_replies_cannot_switch_devices_or_duplicate_an_inflight_click() {
        let (_dir, _guard, mut app) = pending_reply_app();
        let before = app
            .push
            .snapshot
            .threads
            .iter()
            .map(|t| t.messages.len())
            .sum::<usize>();
        app.dispatch_im_reply(
            "thread-a",
            "com.tencent.mm",
            "thread-a",
            "first reply",
            None,
        );
        assert_eq!(
            app.push
                .snapshot
                .threads
                .iter()
                .map(|t| t.messages.len())
                .sum::<usize>(),
            before
        );
        assert!(app.push.reply_queue.is_empty());
        let job = app.push.active_reply_job.take().unwrap();
        app.push.reply_queue.push_back(job);
        app.selected.as_mut().unwrap().serial = "phone-b".into();
        app.pump_reply_queue();
        assert!(app.push.active_reply_job.is_none() && app.push.reply_queue.is_empty());
        assert!(app.error && !app.push.reply_in_flight);
        assert_eq!(
            app.push
                .snapshot
                .threads
                .iter()
                .find(|t| t.thread_id == "thread-a")
                .unwrap()
                .messages[0]
                .delivery_status,
            "failed"
        );
    }

    #[test]
    fn phone_accepted_reply_preserves_sent_status_without_claiming_recipient_delivery() {
        let (_dir, _guard, mut app) = pending_reply_app();
        app.finish_im_reply(
            "phone-a",
            "thread-a",
            "client-a",
            Ok(ReplyReceipt {
                ok: true,
                thread_id: "thread-a".into(),
                client_msg_id: "client-a".into(),
                delivery_status: "sent".into(),
                ..Default::default()
            }),
        );
        assert!(!app.error);
        assert_eq!(
            app.push
                .snapshot
                .threads
                .iter()
                .find(|t| t.thread_id == "thread-a")
                .unwrap()
                .messages[0]
                .delivery_status,
            "sent"
        );
        assert!(!app.push.reply_in_flight);
    }

    #[test]
    fn lost_transport_after_dispatch_keeps_the_send_uncertain() {
        let (_dir, _guard, mut app) = pending_reply_app();
        app.finish_im_reply(
            "phone-a",
            "thread-a",
            "client-a",
            Err("Connection reset by peer".into()),
        );
        assert_eq!(
            app.push
                .snapshot
                .threads
                .iter()
                .find(|t| t.thread_id == "thread-a")
                .unwrap()
                .messages[0]
                .delivery_status,
            "unknown"
        );
        assert!(app.status.contains("未核实") && app.status.contains("避免重复发送"));
        assert!(app.error);
    }

    #[test]
    fn wechat_thread_selection_and_channel_filter_never_force_im_channel_to_sms() {
        let threads = vec![
            ImThread {
                thread_id: "com.tencent.mm::产品研发群".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "产品研发群".into(),
                is_group: true,
                last_sender: "王敏".into(),
                last_message: "今晚发版前大家验收一下".into(),
                ..Default::default()
            },
            ImThread {
                thread_id: "com.tencent.mm::张伟".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "张伟".into(),
                last_sender: "张伟".into(),
                last_message: "收到".into(),
                ..Default::default()
            },
        ];

        let mut im_channel = ImChannel::All;
        let mut selected_im_thread = Some("com.tencent.mm::产品研发群".to_string());
        let mut selected_sms_thread: Option<String> = None;

        // 1. Clicking a WeChat thread in "全部" (`ImChannel::All`) keeps `im_channel == ImChannel::All`
        //    and NEVER switches to `ImChannel::Sms`.
        select_im_thread_state(
            &mut im_channel,
            &mut selected_im_thread,
            &mut selected_sms_thread,
            &threads,
            "com.tencent.mm::张伟",
        );
        assert_eq!(im_channel, ImChannel::All);
        assert_eq!(selected_im_thread.as_deref(), Some("com.tencent.mm::张伟"));
        assert_eq!(selected_sms_thread, None);

        // 2. Clicking an SMS row while viewing "全部" (`ImChannel::All`) selects the SMS thread
        //    in the right pane WITHOUT hiding WeChat conversations on the left (`im_channel` stays `All`).
        select_sms_thread_state(
            &mut im_channel,
            &mut selected_im_thread,
            &mut selected_sms_thread,
            Some("10086"),
        );
        assert_eq!(im_channel, ImChannel::All);
        assert_eq!(selected_im_thread.as_deref(), Some(""));
        assert_eq!(selected_sms_thread.as_deref(), Some("10086"));

        // 3. Clicking the "微信" filter pill (`ImChannel::WeChat`) switches cleanly to `ImChannel::WeChat`,
        //    clears `selected_sms_thread`, and selects the first WeChat thread.
        select_im_channel_state(
            &mut im_channel,
            &mut selected_im_thread,
            &mut selected_sms_thread,
            &threads,
            Some("10086"),
            ImChannel::WeChat,
        );
        assert_eq!(im_channel, ImChannel::WeChat);
        assert_ne!(im_channel, ImChannel::Sms);
        assert_eq!(
            selected_im_thread.as_deref(),
            Some("com.tencent.mm::产品研发群")
        );
        assert_eq!(selected_sms_thread, None);

        // 4. Clearing SMS selection (`SelectSmsThread(None)`) never clobbers `im_channel` or `selected_im_thread`.
        select_sms_thread_state(
            &mut im_channel,
            &mut selected_im_thread,
            &mut selected_sms_thread,
            None,
        );
        assert_eq!(im_channel, ImChannel::WeChat);
        assert_eq!(
            selected_im_thread.as_deref(),
            Some("com.tencent.mm::产品研发群")
        );
    }

    #[test]
    fn repeated_failed_retries_replace_single_failed_bubble_and_preserve_last_message() {
        let mut snapshot = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::群聊".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "群聊".into(),
                is_group: true,
                last_sender: "卢".into(),
                last_message: "1".into(),
                last_timestamp_ms: 1775021100000,
                messages: vec![
                    ImMessage {
                        id: "m1".into(),
                        sender: "卢".into(),
                        text: "测试".into(),
                        timestamp_ms: 1775021000000,
                        outgoing: false,
                        delivery_status: "received".into(),
                        ..Default::default()
                    },
                    ImMessage {
                        id: "m2".into(),
                        sender: "卢".into(),
                        text: "1".into(),
                        timestamp_ms: 1775021100000,
                        outgoing: false,
                        delivery_status: "received".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        // Simulate user clicking "发送" ("hi") 4 times when every attempt fails
        for attempt in 1..=4 {
            let cid = format!("stepx-attempt-{attempt}");
            record_local_outgoing_attempt(
                &mut snapshot,
                "com.tencent.mm::群聊",
                "com.tencent.mm",
                "群聊",
                "hi",
                None,
                &cid,
                1775021100000 + attempt * 1000,
                false,
            );
            let restored = apply_reply_failure_to_snapshot(&mut snapshot, "未找到发送按钮");
            assert_eq!(
                restored,
                vec![("com.tencent.mm::群聊".to_string(), "hi".to_string())]
            );

            let thread = &snapshot.threads[0];
            let failed_bubbles: Vec<_> = thread
                .messages
                .iter()
                .filter(|m| m.outgoing && m.delivery_status == "failed")
                .collect();
            assert_eq!(
                failed_bubbles.len(),
                1,
                "attempt {attempt}: must have at most 1 failed bubble, never stack multiple failed bubbles"
            );
            assert_eq!(failed_bubbles[0].text, "hi");
            assert_eq!(
                thread.last_message, "1",
                "attempt {attempt}: thread preview must stay on last delivered message, not failed 'hi'"
            );
            assert_eq!(thread.last_sender, "卢");
        }

        // Now 5th attempt succeeds: replaces the failed bubble with a single "sent" bubble
        record_local_outgoing_attempt(
            &mut snapshot,
            "com.tencent.mm::群聊",
            "com.tencent.mm",
            "群聊",
            "hi",
            None,
            "stepx-attempt-5",
            1775021105000,
            false,
        );
        apply_reply_success_to_snapshot(
            &mut snapshot,
            &ReplyReceipt {
                ok: true,
                thread_id: "com.tencent.mm::群聊".into(),
                client_msg_id: "stepx-attempt-5".into(),
                via: "remote_input".into(),
                message: "已发送".into(),
                ..Default::default()
            },
        );
        let thread = &snapshot.threads[0];
        assert_eq!(thread.messages.len(), 3);
        assert_eq!(thread.messages[2].text, "hi");
        assert_eq!(thread.messages[2].delivery_status, "sent");
        assert_eq!(thread.last_sender, "我");
        assert_eq!(thread.last_message, "hi");
    }

    #[test]
    fn replying_to_generic_group_thread_never_targets_unrelated_specific_thread() {
        let mut snapshot = PushSnapshot {
            threads: vec![
                ImThread {
                    thread_id: "com.tencent.mm::微信支付".into(),
                    package_name: "com.tencent.mm".into(),
                    title: "微信支付".into(),
                    last_message: "支付成功".into(),
                    ..Default::default()
                },
                ImThread {
                    thread_id: "com.tencent.mm::群聊".into(),
                    package_name: "com.tencent.mm".into(),
                    title: "群聊".into(),
                    is_group: true,
                    last_message: "1".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };

        let (tid, ttl) = record_local_outgoing_attempt(
            &mut snapshot,
            "com.tencent.mm::群聊",
            "com.tencent.mm",
            "群聊",
            "hi",
            None,
            "cid-1",
            1775021200000,
            false,
        );
        assert_eq!(tid, "com.tencent.mm::群聊");
        assert_eq!(ttl, "群聊");
        assert!(
            snapshot.threads[0].messages.is_empty(),
            "must not attach reply to unrelated 微信支付 thread"
        );
        assert_eq!(snapshot.threads[1].messages.len(), 1);
        assert_eq!(snapshot.threads[1].messages[0].text, "hi");
    }

    #[test]
    fn consecutive_in_flight_replies_stay_queued_and_resolve_by_client_msg_id() {
        let mut snapshot = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::产品研发群".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "产品研发群".into(),
                is_group: true,
                ..Default::default()
            }],
            ..Default::default()
        };

        record_local_outgoing_attempt(
            &mut snapshot,
            "com.tencent.mm::产品研发群",
            "com.tencent.mm",
            "产品研发群",
            "第一条回复",
            None,
            "stepx-1000-1",
            1000,
            false,
        );
        record_local_outgoing_attempt(
            &mut snapshot,
            "com.tencent.mm::产品研发群",
            "com.tencent.mm",
            "产品研发群",
            "第二条回复",
            None,
            "stepx-1001-2",
            1001,
            false,
        );

        let msgs = &snapshot.threads[0].messages;
        assert_eq!(msgs.len(), 2, "both in-flight replies must remain visible");
        assert_eq!(msgs[0].client_msg_id, "stepx-1000-1");
        assert_eq!(msgs[0].delivery_status, "sending");
        assert_eq!(msgs[1].client_msg_id, "stepx-1001-2");
        assert_eq!(msgs[1].delivery_status, "sending");

        apply_reply_success_to_snapshot(
            &mut snapshot,
            &ReplyReceipt {
                ok: true,
                thread_id: "com.tencent.mm::产品研发群".into(),
                client_msg_id: "stepx-1000-1".into(),
                via: "remote_input".into(),
                message: "已发送".into(),
                ..Default::default()
            },
        );
        let msgs = &snapshot.threads[0].messages;
        assert_eq!(msgs[0].delivery_status, "sent");
        assert_eq!(msgs[1].delivery_status, "sending");
    }

    #[test]
    fn outgoing_replies_sort_chronologically_with_untimestamped_screen_messages_and_survive_merge()
    {
        let now_ms = chrono::Local::now().timestamp_millis();
        let mut incoming_snapshot = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::文件传输助手".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "文件传输助手".into(),
                last_timestamp_ms: now_ms - 1000,
                messages: vec![ImMessage {
                    id: "scr-1".into(),
                    sender: "文件传输助手".into(),
                    text: "旧屏幕消息".into(),
                    timestamp_ms: 0,
                    observed_at_ms: now_ms + 3000,
                    time_known: Some(false),
                    outgoing: false,
                    delivery_status: "received".into(),
                    via: "accessibility_chat_window".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let local_snapshot = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::文件传输助手".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "文件传输助手".into(),
                last_timestamp_ms: now_ms,
                messages: vec![ImMessage {
                    id: "local-1".into(),
                    sender: "我".into(),
                    text: "我刚发的桌面消息".into(),
                    timestamp_ms: now_ms,
                    observed_at_ms: now_ms,
                    time_known: Some(true),
                    outgoing: true,
                    delivery_status: "sent".into(),
                    via: "accessibility_ui".into(),
                    client_msg_id: "cid-2000".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        push::merge_local_replies(&mut incoming_snapshot, &local_snapshot);

        let msgs = &incoming_snapshot.threads[0].messages;
        assert_eq!(msgs.len(), 2);
        assert_eq!(msgs[0].text, "旧屏幕消息");
        assert_eq!(msgs[1].text, "我刚发的桌面消息");
        assert!(msgs[1].outgoing);
        assert_eq!(
            incoming_snapshot.threads[0].last_message,
            "我刚发的桌面消息"
        );
        assert_eq!(incoming_snapshot.threads[0].last_sender, "我");
    }

    #[test]
    fn deleted_thread_tombstone_prevents_resurrection_until_newer_message_arrives() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::ui::TestUiConfigDirGuard::set(dir.path());
        let mut prefs = desktop_notify::NotifyPreferences::default();
        let key = desktop_notify::NotifyPreferences::im_target_key("com.tencent.mm::已删除用户");
        prefs.deleted_threads.insert(key, 5000);

        let local_snapshot = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "com.tencent.mm::已删除用户".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "已删除用户".into(),
                last_timestamp_ms: 4000,
                messages: vec![ImMessage {
                    id: "local-old".into(),
                    sender: "我".into(),
                    text: "删之前的消息".into(),
                    timestamp_ms: 4000,
                    observed_at_ms: 4000,
                    outgoing: true,
                    delivery_status: "sent".into(),
                    ..Default::default()
                }],
                ..Default::default()
            }],
            ..Default::default()
        };

        let mut snapshot = PushSnapshot::default();
        push::merge_local_replies(&mut snapshot, &local_snapshot);
        apply_push_preferences_to_snapshot(&mut snapshot, &mut prefs);
        assert!(
            snapshot.threads.is_empty(),
            "deleted thread must not resurrect from older local replies"
        );
    }

    #[test]
    fn common_conversation_and_message_actions_update_state_and_tombstones() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::ui::TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.push.snapshot.threads = vec![
            ImThread {
                thread_id: "com.tencent.mm::文件传输助手".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "文件传输助手".into(),
                last_message: "第二条".into(),
                last_sender: "我".into(),
                last_timestamp_ms: 2000,
                unread_count: 0,
                messages: vec![
                    ImMessage {
                        id: "m1".into(),
                        sender: "文件传输助手".into(),
                        text: "第一条".into(),
                        timestamp_ms: 1000,
                        observed_at_ms: 1000,
                        outgoing: false,
                        ..Default::default()
                    },
                    ImMessage {
                        id: "m2".into(),
                        sender: "我".into(),
                        text: "第二条".into(),
                        timestamp_ms: 2000,
                        observed_at_ms: 2000,
                        outgoing: true,
                        ..Default::default()
                    },
                ],
                ..Default::default()
            },
            ImThread {
                thread_id: "com.tencent.mm::测试用户B".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "测试用户B".into(),
                last_message: "你好".into(),
                last_sender: "测试用户B".into(),
                last_timestamp_ms: 3000,
                unread_count: 2,
                messages: vec![ImMessage {
                    id: "b1".into(),
                    sender: "测试用户B".into(),
                    text: "你好".into(),
                    timestamp_ms: 3000,
                    observed_at_ms: 3000,
                    outgoing: false,
                    ..Default::default()
                }],
                ..Default::default()
            },
        ];

        // 1. Toggle Pin
        let pin_key =
            desktop_notify::NotifyPreferences::im_target_key("com.tencent.mm::文件传输助手");
        assert!(app.apply_common_push_action(&PushAction::TogglePinTarget(pin_key.clone())));
        assert!(app.push.prefs.is_pinned(&pin_key));

        // 2. Toggle Read / Unread
        assert!(app.apply_common_push_action(&PushAction::ToggleThreadRead {
            is_im: true,
            target_id: "com.tencent.mm::文件传输助手".into(),
        }));
        assert_eq!(app.push.snapshot.threads[0].unread_count, 1);
        assert!(app.apply_common_push_action(&PushAction::ToggleThreadRead {
            is_im: true,
            target_id: "com.tencent.mm::文件传输助手".into(),
        }));
        assert_eq!(app.push.snapshot.threads[0].unread_count, 0);

        // 3. Delete Single Message
        assert!(app.apply_common_push_action(&PushAction::DeleteMessage {
            thread_id: "com.tencent.mm::文件传输助手".into(),
            package_name: "com.tencent.mm".into(),
            msg_id: "m2".into(),
            text: "第二条".into(),
        }));
        assert_eq!(app.push.snapshot.threads[0].messages.len(), 1);
        assert_eq!(app.push.snapshot.threads[0].last_message, "第一条");

        // 4. Clear Chat History
        assert!(
            app.apply_common_push_action(&PushAction::ClearThreadMessages {
                is_im: true,
                target_id: "com.tencent.mm::文件传输助手".into(),
                package_name: "com.tencent.mm".into(),
                title: "文件传输助手".into(),
            })
        );
        assert!(app.push.snapshot.threads[0].messages.is_empty());
        assert!(app.push.snapshot.threads[0].last_message.is_empty());

        // 5. Delete Conversation
        assert!(app.apply_common_push_action(&PushAction::DeleteImThread {
            thread_id: "com.tencent.mm::测试用户B".into(),
            package_name: "com.tencent.mm".into(),
            title: "测试用户B".into(),
        }));
        assert_eq!(app.push.snapshot.threads.len(), 1);
        assert_eq!(
            app.push.snapshot.threads[0].thread_id,
            "com.tencent.mm::文件传输助手"
        );

        // 6. Dismiss Pickup Code across parcels, OTP banners, and notifications
        app.express_parcels
            .push(stepx_companion::device::ExpressParcelInfo {
                carrier: "顺丰速运".into(),
                tracking_no: "SF1234567890123".into(),
                status: "已到站待取".into(),
                pickup_code: "18-2-2009".into(),
                station: "菜鸟驿站".into(),
                updated_at: "刚刚".into(),
            });
        app.push
            .snapshot
            .otp_banners
            .push(stepx_companion::push::OtpBanner {
                code: "18-2-2009".into(),
                source_app: "短信".into(),
                title: "菜鸟驿站".into(),
                raw_text: "凭取件码 18-2-2009 取件".into(),
                timestamp_ms: 1_700_000_000_000,
                notification_key: "sms-pickup-1".into(),
            });
        assert!(app.apply_common_push_action(&PushAction::DismissPickupCode("18-2-2009".into())));
        assert!(app.push.prefs.dismissed_pickup_codes.contains("18-2-2009"));
        assert!(app.express_parcels.is_empty());
        assert!(app.push.snapshot.otp_banners.is_empty());
    }

    #[test]
    fn effective_push_notifications_unifies_ongoing_im_and_sms_signals() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = crate::ui::TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        assert!(
            !app.push.collapse_ongoing,
            "ongoing live delivery notifications should be expanded by default"
        );

        app.push.snapshot.notifications = vec![
            push::PushNotification {
                seq_id: 1,
                key: "0|com.sankuai.meituan|1001".into(),
                package_name: "com.sankuai.meituan".into(),
                app_name: "美团外卖".into(),
                title: "骑手距您 320m · 预计 3 分钟送达".into(),
                text: "轻食能量碗套餐正在火速配送中".into(),
                post_time_ms: 5_000,
                is_ongoing: true,
                read: false,
                ..Default::default()
            },
            push::PushNotification {
                seq_id: 2,
                key: "0|com.eg.android.AlipayGphone|1002".into(),
                package_name: "com.eg.android.AlipayGphone".into(),
                app_name: "支付宝".into(),
                title: "顺丰同城急送 · 派送中".into(),
                text: "骑士王师傅预计 14:25 送达".into(),
                post_time_ms: 4_500,
                is_ongoing: false,
                read: false,
                ..Default::default()
            },
        ];
        app.push.snapshot.threads = vec![
            ImThread {
                thread_id: "com.tencent.mm::产品架构评审群".into(),
                package_name: "com.tencent.mm".into(),
                app_name: "微信".into(),
                title: "产品架构评审群".into(),
                last_message: "[有人@我] 请确认 v0.2.101 通知弹窗方案".into(),
                last_sender: "张工".into(),
                last_timestamp_ms: 4_800,
                unread_count: 2,
                can_remote_reply: true,
                ..Default::default()
            },
            ImThread {
                thread_id: "com.hpbr.bosszhipin::王HR·AI架构师".into(),
                package_name: "com.hpbr.bosszhipin".into(),
                app_name: "BOSS直聘".into(),
                title: "王HR·AI架构师".into(),
                last_message: "您好，方便今天下午约个一面吗？".into(),
                last_sender: "王HR".into(),
                last_timestamp_ms: 4_600,
                unread_count: 1,
                can_remote_reply: true,
                ..Default::default()
            },
        ];
        app.sms = vec![
            stepx_companion::sms::Message {
                provider_id: Some(101),
                address: "106839128801".into(),
                body: "【叮咚买菜】您的生鲜订单已送达家门口，请及时查收保鲜。".into(),
                date_ms: 4_900,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
            stepx_companion::sms::Message {
                provider_id: Some(102),
                address: "106881920112".into(),
                body: "【菜鸟驿站】您的包裹已到站，请凭取件码 6-2-3018 前往自提柜取件。".into(),
                date_ms: 4_700,
                outgoing: false,
                kind: 1,
                read: false,
                subscription: None,
            },
        ];

        assert_eq!(
            crate::ui::pages::sms::extract_sms_bracket_brand(
                "【叮咚买菜】您的生鲜订单已送达家门口"
            )
            .as_deref(),
            Some("叮咚买菜")
        );

        let effective = app.effective_push_notifications();
        assert_eq!(effective.len(), 6);
        assert_eq!(app.unread_notifications(), 6);
        assert!(effective
            .iter()
            .any(|n| n.app_name == "美团外卖" && n.is_ongoing));
        assert!(effective.iter().any(|n| n.app_name == "叮咚买菜"));
        assert!(effective
            .iter()
            .any(|n| n.app_name == "菜鸟驿站" && n.otp_code.as_deref() == Some("6-2-3018")));
        assert!(effective.iter().any(|n| n.app_name == "微信"));
        assert!(effective.iter().any(|n| n.app_name == "BOSS直聘"));

        // Verify maybe_auto_copy_sms_otp extracts unread SMS verification code (and ignores pickup codes) cleanly once
        app.push.prefs.auto_copy_otp = true;
        app.sms.push(stepx_companion::sms::Message {
            provider_id: Some(103),
            address: "10690123456".into(),
            body: "【招商银行】您的登录验证码为 839201，5分钟内有效。".into(),
            date_ms: chrono::Local::now().timestamp_millis(),
            outgoing: false,
            kind: 1,
            read: false,
            subscription: None,
        });
        assert_eq!(app.maybe_auto_copy_sms_otp().as_deref(), Some("839201"));
        assert_eq!(app.maybe_auto_copy_sms_otp(), None);
    }
}
