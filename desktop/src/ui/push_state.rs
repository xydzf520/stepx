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
    SendImReply,
    SetReplyMode(ReplyMode),
    SyncActivePhoneChatWindow,
    SetAlertScope(AlertScope),
    SetPopupMode(PopupMode),
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
    TriggerTestToast,
    LoadDemoPush,
    SetCallFilter(u8),
    SetNotifCategory(u8),
    FillSmsTemplate(String),
    SelectCalendarQuickTime(String),
    OpenNewAmooChat,
}

pub(super) enum PushOutcome {
    Snapshot(String, Result<PushSnapshot, String>),
    ReplyFinished(String, Result<ReplyReceipt, String>),
    ActiveWindowSynced(String, Result<Option<ImThread>, String>),
    NotificationDismissed(String, String, Result<(), String>),
    AllDismissed(String, Result<usize, String>),
    PermissionEnsured(String, Result<bool, String>),
}

#[allow(dead_code)]
pub(super) type PushUi = PushUiState;

pub(super) struct PushUiState {
    pub snapshot: PushSnapshot,
    pub loaded: bool,
    pub syncing: bool,
    pub sync_started: Instant,
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
    pub im_drafts: HashMap<String, String>,
    pub reply_mode: ReplyMode,
    pub sending_thread: Option<String>,
    pub last_reply_status: Option<String>,
    pub prefs: NotifyPreferences,
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
            loaded: false,
            syncing: false,
            sync_started: Instant::now(),
            error: None,
            category: PushCategory::All,
            notif_category: 0,
            call_filter: 0,
            app_filter: None,
            collapse_ongoing: true,
            selected_key: None,
            replying_key: None,
            im_channel: ImChannel::All,
            selected_im_thread: None,
            im_drafts: Default::default(),
            reply_mode: ReplyMode::Auto,
            sending_thread: None,
            last_reply_status: None,
            prefs: desktop_notify::load_preferences(),
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
                    | PushAction::DismissAll
                    | PushAction::DismissNotification(_)
                    | PushAction::OpenOnPhone(_)
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
                if let Some(device) = self.selected.clone() {
                    self.push.syncing = true;
                    self.push.sync_started = Instant::now();
                    self.push.last_sync = Instant::now();
                    self.status = "正在通过同 Wi-Fi / ADB 通道同步手机推送与聊天记录…".into();
                    let tx = self.push.outcome_tx.clone();
                    let serial = device.serial.clone();
                    let transport = device.transport.clone();
                    std::thread::spawn(move || {
                        let res = Adb::discover()
                            .and_then(|adb| push::fetch_snapshot(&adb, &transport, 0))
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(PushOutcome::Snapshot(serial, res));
                    });
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
                            .and_then(|adb| push::ensure_notification_listener(&adb, &transport))
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(PushOutcome::PermissionEnsured(serial, res));
                    });
                }
            }
            PushAction::SelectCategory(cat) => {
                if self.push.category != *cat {
                    self.push.category = *cat;
                    self.trigger_subtab_switch();
                }
                if let Some(first) = self
                    .push
                    .snapshot
                    .notifications
                    .iter()
                    .find(|n| !n.dismissed && cat.matches(&n.category))
                {
                    self.push.selected_key = Some(first.key.clone());
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
                self.push.selected_key = Some(key.clone());
                self.push.replying_key = Some(key.clone());
                if let Some(n) = self
                    .push
                    .snapshot
                    .notifications
                    .iter_mut()
                    .find(|n| n.key == *key)
                {
                    n.read = true;
                }
            }
            PushAction::DismissNotification(key) => {
                if let Some(n) = self
                    .push
                    .snapshot
                    .notifications
                    .iter_mut()
                    .find(|n| n.key == *key || n.thread_id == *key)
                {
                    n.dismissed = true;
                    n.read = true;
                }
                self.push
                    .active_toasts
                    .retain(|t| t.notification_key != *key && t.id != *key && t.thread_id != *key);
                if self.push.selected_key.as_deref() == Some(key.as_str()) {
                    self.push.selected_key = self
                        .push
                        .snapshot
                        .notifications
                        .iter()
                        .find(|n| !n.dismissed)
                        .map(|n| n.key.clone());
                }
                self.status = "已同步清除该条手机通知".into();
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let tx = self.push.outcome_tx.clone();
                        let serial = device.serial.clone();
                        let transport = device.transport.clone();
                        let k = key.clone();
                        std::thread::spawn(move || {
                            let res = Adb::discover()
                                .and_then(|adb| push::dismiss_notification(&adb, &transport, &k))
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
                self.push.active_toasts.clear();
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
                let notif_key = self
                    .push
                    .snapshot
                    .threads
                    .iter()
                    .find(|t| t.thread_id == *key_or_thread)
                    .map(|t| t.notification_key.clone())
                    .filter(|k| !k.is_empty())
                    .unwrap_or_else(|| key_or_thread.clone());
                self.status = "已在手机上触发打开该通知对应页面".into();
                if !self.demo_mode {
                    if let Some(device) = self.selected.clone() {
                        let transport = device.transport.clone();
                        std::thread::spawn(move || {
                            if let Ok(adb) = Adb::discover() {
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
                self.push
                    .active_toasts
                    .retain(|t| t.otp_code.as_deref() != Some(code.as_str()));
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
                let draft = if let Some(thread) = self
                    .push
                    .snapshot
                    .threads
                    .iter()
                    .find(|t| t.thread_id == *thread_id)
                {
                    push::generate_smart_reply_draft(thread)
                } else {
                    let temp = ImThread {
                        thread_id: thread_id.clone(),
                        package_name: package_name.clone(),
                        app_name: String::new(),
                        title: title.clone(),
                        ..Default::default()
                    };
                    push::generate_smart_reply_draft(&temp)
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
                self.status = format!("已为「{title}」准备常用回复建议，请检查后发送");
            }
            PushAction::SelectImChannel(ch) => {
                if self.push.im_channel != *ch {
                    self.push.im_channel = *ch;
                    self.trigger_subtab_switch();
                }
                if *ch != ImChannel::Sms {
                    let current_matches = self
                        .push
                        .selected_im_thread
                        .as_ref()
                        .and_then(|id| {
                            self.push
                                .snapshot
                                .threads
                                .iter()
                                .find(|t| &t.thread_id == id)
                        })
                        .is_some_and(|t| ch.matches_package(&t.package_name));
                    if !current_matches {
                        self.push.selected_im_thread = self
                            .push
                            .snapshot
                            .threads
                            .iter()
                            .find(|t| ch.matches_package(&t.package_name))
                            .map(|t| t.thread_id.clone());
                    }
                }
            }
            PushAction::SelectImThread(thread_id) => {
                if self.push.shell_mode == ShellMode::MiniNotify {
                    self.compact.notification_detail = true;
                }
                if self.push.selected_im_thread.as_ref() != Some(thread_id) {
                    if let Some(previous) = &self.push.selected_im_thread {
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
                    self.fields[F_IM_REPLY].set(
                        self.push
                            .im_drafts
                            .get(thread_id)
                            .map(String::as_str)
                            .unwrap_or(""),
                    );
                }
                self.push.selected_im_thread = Some(thread_id.clone());
                if let Some(t) = self
                    .push
                    .snapshot
                    .threads
                    .iter_mut()
                    .find(|t| t.thread_id == *thread_id)
                {
                    t.unread_count = 0;
                }
                for n in &mut self.push.snapshot.notifications {
                    if n.thread_id == *thread_id {
                        n.read = true;
                    }
                }
                Self::scroll_to_end(&self.sms_messages_scroll);
            }
            PushAction::SendImReply => {
                if self.push.sending_thread.is_some() {
                    self.status = "正在发送，请稍候".into();
                    return;
                }
                let text = self.fields[F_IM_REPLY].text.trim().to_string();
                if text.is_empty() {
                    self.status = "请输入要回复的聊天内容".into();
                    self.focus_field(F_IM_REPLY, ctx);
                    return;
                }
                let target = self
                    .push
                    .selected_im_thread
                    .as_ref()
                    .and_then(|id| {
                        self.push
                            .snapshot
                            .threads
                            .iter()
                            .find(|t| &t.thread_id == id)
                    })
                    .map(|t| (t.thread_id.clone(), t.package_name.clone(), t.title.clone()));
                let Some((thread_id, package_name, title)) = target else {
                    self.status = "请先选择一个聊天会话".into();
                    return;
                };
                self.fields[F_IM_REPLY].clear();
                self.push.im_drafts.remove(&thread_id);
                self.dispatch_im_reply(&thread_id, &package_name, &title, &text, None);
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
                    std::thread::spawn(move || {
                        let res = Adb::discover()
                            .and_then(|adb| push::sync_active_chat_window(&adb, &transport))
                            .map_err(|e| format!("{e:#}"));
                        let _ = tx.send(PushOutcome::ActiveWindowSynced(serial, res));
                    });
                }
            }
            PushAction::SetAlertScope(scope) => {
                self.push.prefs.scope = *scope;
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                self.status = format!("桌面弹窗提醒范围：{}", scope.label());
            }
            PushAction::SetPopupMode(mode) => {
                self.push.prefs.popup_mode = *mode;
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                self.status = format!("桌面弹窗呈现方式：{}", mode.label());
            }
            PushAction::TogglePrivacyMask => {
                self.push.prefs.privacy_mask = !self.push.prefs.privacy_mask;
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                self.status = if self.push.prefs.privacy_mask {
                    "已开启防窥脱敏模式：通知正文将自动隐藏".into()
                } else {
                    "已关闭防窥脱敏模式：直接显示通知完整正文".into()
                };
            }
            PushAction::ToggleAutoCopyOtp => {
                self.push.prefs.auto_copy_otp = !self.push.prefs.auto_copy_otp;
                let _ = desktop_notify::save_preferences(&self.push.prefs);
                self.status = if self.push.prefs.auto_copy_otp {
                    "已开启：收到验证码推送时自动复制到电脑剪贴板".into()
                } else {
                    "已关闭验证码自动复制".into()
                };
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
            PushAction::TriggerTestToast => {
                if self.push.snapshot.threads.is_empty()
                    && self.push.snapshot.notifications.is_empty()
                {
                    self.init_demo_push();
                }
                let now_ms = chrono::Utc::now().timestamp_millis();
                if let Some(first_im) = self.push.snapshot.threads.first().cloned() {
                    let item = DesktopToastItem {
                        id: format!("toast:test:{now_ms}"),
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
                        timestamp_ms: now_ms,
                    };
                    let _ = desktop_notify::send_system_notification(
                        &item,
                        self.push.prefs.privacy_mask,
                    );
                    self.push.active_toasts.insert(0, item);
                    self.push.active_toasts.truncate(3);
                } else {
                    let item = DesktopToastItem {
                        id: format!("toast:test:{now_ms}"),
                        notification_key: "test_key".into(),
                        thread_id: "com.tencent.mm::产品研发群".into(),
                        package_name: "com.tencent.mm".into(),
                        app_name: "微信".into(),
                        category: "im".into(),
                        title: "产品研发群".into(),
                        sender: "王敏".into(),
                        body: "王敏：@全体 今晚 8 点发版前大家把推送中心与小窗秒回验收一下，有问题吗？".into(),
                        masked_body: "收到来自「产品研发群」的新消息（防窥模式已隐藏正文）".into(),
                        otp_code: None,
                        url_hint: None,
                        can_reply: true,
                        timestamp_ms: now_ms,
                    };
                    let _ = desktop_notify::send_system_notification(
                        &item,
                        self.push.prefs.privacy_mask,
                    );
                    self.push.active_toasts.insert(0, item);
                    self.push.active_toasts.truncate(3);
                }
                self.focus_field(F_TOAST_REPLY, ctx);
                self.status = "已在右下角弹出交互式快回卡片（支持直接输入回车秒发）".into();
            }
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
        }
    }

    fn dispatch_im_reply(
        &mut self,
        thread_id: &str,
        package_name: &str,
        title: &str,
        text: &str,
        notif_key: Option<&str>,
    ) {
        let now_ms = chrono::Utc::now().timestamp_millis();
        let client_msg_id = format!("stepx-{now_ms}-{}", self.push.snapshot.revision);
        if !self.demo_mode && self.selected.is_none() {
            self.status = "手机已断开，消息未发送".into();
            self.error = true;
            return;
        }
        let immediate_sent = self.demo_mode;
        let new_msg = ImMessage {
            id: client_msg_id.clone(),
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
            client_msg_id: client_msg_id.clone(),
        };

        if let Some(thread) = self.push.snapshot.threads.iter_mut().find(|t| {
            t.thread_id == thread_id || (t.package_name == package_name && t.title == title)
        }) {
            thread.messages.push(new_msg);
            thread.last_sender = "我".into();
            thread.last_message = text.to_string();
            thread.last_timestamp_ms = now_ms;
            thread.unread_count = 0;
        } else {
            let ch = ImChannel::from_package(package_name);
            self.push.snapshot.threads.insert(
                0,
                ImThread {
                    thread_id: thread_id.to_string(),
                    package_name: package_name.to_string(),
                    app_name: ch.label().into(),
                    title: title.to_string(),
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
                },
            );
        }

        for n in &mut self.push.snapshot.notifications {
            if n.thread_id == thread_id
                || notif_key.is_some_and(|k| n.key == k)
                || (n.package_name == package_name && n.title == title)
            {
                n.read = true;
            }
        }
        self.push.active_toasts.retain(|t| {
            t.thread_id != thread_id && notif_key.is_none_or(|k| t.notification_key != k)
        });
        Self::scroll_to_end(&self.sms_messages_scroll);

        if immediate_sent {
            self.push.sending_thread = None;
            let status_msg = format!("已通过同 Wi-Fi RemoteInput 秒回「{title}」：{text}");
            self.push.last_reply_status = Some(status_msg.clone());
            self.status = status_msg;
            return;
        }

        if let Some(device) = self.selected.clone() {
            self.push.sending_thread = Some(thread_id.to_string());
            let waiting = format!("正在向「{title}」发送回复…");
            self.push.last_reply_status = Some(waiting.clone());
            self.status = waiting;
            let tx = self.push.outcome_tx.clone();
            let serial = device.serial.clone();
            let transport = device.transport.clone();
            let tid = thread_id.to_string();
            let pkg = package_name.to_string();
            let ttl = title.to_string();
            let body = text.to_string();
            let mode = self.push.reply_mode;
            std::thread::spawn(move || {
                let res = Adb::discover()
                    .and_then(|adb| {
                        push::reply_im(
                            &adb,
                            &transport,
                            &tid,
                            &pkg,
                            &ttl,
                            &body,
                            mode,
                            &client_msg_id,
                        )
                    })
                    .map_err(|e| format!("{e:#}"));
                let _ = tx.send(PushOutcome::ReplyFinished(serial, res));
            });
        }
    }

    pub(super) fn handle_push_outcome(
        &mut self,
        outcome: PushOutcome,
        ctx: &mut ViewContext<Self>,
    ) {
        match outcome {
            PushOutcome::Snapshot(serial, res) => {
                self.push.syncing = false;
                if !self.demo_mode && self.selected.as_ref().is_none_or(|d| d.serial != serial) {
                    return;
                }
                match res {
                    Ok(mut next) => {
                        push::merge_local_replies(&mut next, &self.push.snapshot);
                        self.push.cap_seen_toast_keys(&next);
                        let active_thread_id =
                            if self.push.shell_mode == ShellMode::Full && self.page == Page::Sms {
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
                        for toast in new_toasts {
                            if self.push.prefs.auto_copy_otp {
                                if let Some(code) = &toast.otp_code {
                                    ctx.clipboard()
                                        .write(ClipboardContent::plain_text(code.clone()));
                                    self.status = format!("已自动复制验证码 {code} 到剪贴板");
                                }
                            }
                            if matches!(
                                self.push.prefs.popup_mode,
                                PopupMode::InteractiveAndSystem | PopupMode::SystemOnly
                            ) {
                                let _ = desktop_notify::send_system_notification(
                                    &toast,
                                    self.push.prefs.privacy_mask,
                                );
                            }
                            if matches!(
                                self.push.prefs.popup_mode,
                                PopupMode::InteractiveAndSystem | PopupMode::InteractiveOnly
                            ) {
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

                        if self.push.selected_im_thread.is_none() {
                            self.push.selected_im_thread =
                                next.threads.first().map(|t| t.thread_id.clone());
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
                    }
                    Err(err) => {
                        self.push.error = Some(err);
                    }
                }
            }
            PushOutcome::ReplyFinished(serial, res) => {
                if !self.demo_mode && self.selected.as_ref().is_none_or(|d| d.serial != serial) {
                    return;
                }
                self.push.sending_thread = None;
                match res {
                    Ok(receipt) => {
                        if let Some(thread) = self
                            .push
                            .snapshot
                            .threads
                            .iter_mut()
                            .find(|t| t.thread_id == receipt.thread_id)
                        {
                            for m in thread.messages.iter_mut().rev() {
                                if m.outgoing
                                    && (m.id == receipt.client_msg_id
                                        || m.client_msg_id == receipt.client_msg_id)
                                {
                                    m.delivery_status = "sent".into();
                                    if !receipt.via.is_empty() {
                                        m.via = receipt.via.clone();
                                    }
                                    break;
                                }
                            }
                        }
                        let msg = if receipt.message.is_empty() {
                            "消息已通过同 Wi-Fi 通道直接发出".to_string()
                        } else {
                            receipt.message
                        };
                        self.push.last_reply_status = Some(msg.clone());
                        self.status = msg;
                    }
                    Err(err) => {
                        for thread in &mut self.push.snapshot.threads {
                            for m in thread.messages.iter_mut().rev() {
                                if m.outgoing && m.delivery_status == "sending" {
                                    m.delivery_status = if err.contains("未核实")
                                        || err.contains("超时")
                                        || err.contains("timed out")
                                    {
                                        "unknown"
                                    } else {
                                        "failed"
                                    }
                                    .into();
                                    if self.push.selected_im_thread.as_ref()
                                        == Some(&thread.thread_id)
                                        && self.fields[F_IM_REPLY].text.is_empty()
                                    {
                                        self.fields[F_IM_REPLY].set(&m.text);
                                    } else {
                                        self.push
                                            .im_drafts
                                            .entry(thread.thread_id.clone())
                                            .or_insert_with(|| m.text.clone());
                                    }
                                    break;
                                }
                            }
                        }
                        let msg = format!("回复未完成：{err}");
                        self.push.last_reply_status = Some(msg.clone());
                        self.status = msg;
                        self.error = true;
                    }
                }
            }
            PushOutcome::ActiveWindowSynced(serial, res) => {
                if !self.demo_mode && self.selected.as_ref().is_none_or(|d| d.serial != serial) {
                    return;
                }
                match res {
                    Ok(Some(thread)) => {
                        let tid = thread.thread_id.clone();
                        let title = thread.title.clone();
                        let count = thread.messages.len();
                        if let Some(existing) = self
                            .push
                            .snapshot
                            .threads
                            .iter_mut()
                            .find(|t| t.thread_id == tid)
                        {
                            *existing = thread;
                        } else {
                            self.push.snapshot.threads.insert(0, thread);
                        }
                        self.push.selected_im_thread = Some(tid);
                        self.status =
                            format!("已同步手机当前聊天窗口「{title}」（{count} 条消息）");
                        Self::scroll_to_end(&self.sms_messages_scroll);
                    }
                    Ok(None) => {
                        self.status = "手机当前前台未处于受支持的聊天窗口".into();
                    }
                    Err(err) => {
                        self.status = format!("抓取手机当前聊天窗口失败：{err}");
                    }
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
                    if let Some(device) = self.selected.clone().filter(|d| d.serial == serial) {
                        let tx = self.push.outcome_tx.clone();
                        let transport = device.transport.clone();
                        std::thread::spawn(move || {
                            let snap = Adb::discover()
                                .and_then(|adb| push::fetch_snapshot(&adb, &transport, 0))
                                .map_err(|e| format!("{e:#}"));
                            let _ = tx.send(PushOutcome::Snapshot(serial, snap));
                        });
                    }
                }
                Err(err) => {
                    self.status = format!("开启通知监听权限失败：{err}");
                }
            },
        }
    }

    pub(super) fn tick_push_sync(&mut self, ctx: &mut ViewContext<Self>) {
        let mut changed = false;
        while let Ok(mut outcome) = self.push.outcome_rx.try_recv() {
            if let PushOutcome::Snapshot(serial, Ok(next)) = &mut outcome {
                if self.push.loaded
                    && self.push.error.is_none()
                    && self.selected.as_ref().is_some_and(|d| d.serial == *serial)
                {
                    push::merge_local_replies(next, &self.push.snapshot);
                    if *next == self.push.snapshot {
                        self.push.syncing = false;
                        continue;
                    }
                }
            }
            self.handle_push_outcome(outcome, ctx);
            changed = true;
        }
        if !self.demo_mode
            && !self.push.syncing
            && self.push.last_sync.elapsed()
                >= Duration::from_millis(if self.push.error.is_some() {
                    10_000
                } else {
                    2500
                })
        {
            if let Some(device) = self.selected.clone() {
                self.push.syncing = true;
                self.push.sync_started = Instant::now();
                self.push.last_sync = Instant::now();
                let tx = self.push.outcome_tx.clone();
                let serial = device.serial.clone();
                let transport = device.transport.clone();
                let since_revision = if self.push.loaded {
                    self.push.snapshot.revision
                } else {
                    0
                };
                std::thread::spawn(move || {
                    let res = Adb::discover()
                        .and_then(|adb| push::fetch_snapshot(&adb, &transport, since_revision))
                        .map_err(|e| format!("{e:#}"));
                    let _ = tx.send(PushOutcome::Snapshot(serial, res));
                });
            }
        }
        if changed {
            ctx.notify();
        }
    }
}
