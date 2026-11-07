//! Cross-platform desktop notification policy, deduplication, and native OS notification bridge
//! (Windows WinRT Toast, macOS Notification Center, Linux Freedesktop `notify-send` / D-Bus).

use crate::push::{extract_first_url, extract_otp_code, PushNotification, PushSnapshot};
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::{HashMap, HashSet},
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    thread,
    time::Duration,
};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum AlertScope {
    All,
    #[default]
    ImAndOtpOnly,
    OtpOnly,
    FocusHostOnly,
    Muted,
}

impl AlertScope {
    pub const ALL: [AlertScope; 5] = [
        Self::All,
        Self::ImAndOtpOnly,
        Self::OtpOnly,
        Self::FocusHostOnly,
        Self::Muted,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部推送",
            Self::ImAndOtpOnly => "仅聊天与验证码",
            Self::OtpOnly => "仅验证码",
            Self::FocusHostOnly => "AI 专注托管",
            Self::Muted => "静音免打扰",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PopupMode {
    /// 设置内不弹：仅在消息页与通知中心内静默记录，不弹出桌面弹窗
    #[serde(
        alias = "SystemOnly",
        alias = "system_only",
        alias = "silent",
        alias = "off"
    )]
    Silent,
    /// 只提醒有消息：弹出精简横幅，仅提示某应用/联系人发来新消息，隐藏具体正文且不带回复框
    #[serde(
        alias = "InteractiveOnly",
        alias = "interactive_only",
        alias = "banner"
    )]
    Banner,
    /// 显示完整消息并可直接回复：弹出完整消息正文，支持全应用直接输入或快捷短语回复
    #[default]
    #[serde(
        alias = "InteractiveAndSystem",
        alias = "interactive_and_system",
        alias = "full_reply"
    )]
    FullReply,
}

#[allow(non_upper_case_globals)]
impl PopupMode {
    pub const InteractiveAndSystem: Self = Self::FullReply;
    pub const InteractiveOnly: Self = Self::Banner;
    pub const SystemOnly: Self = Self::Silent;

    pub const ALL: [PopupMode; 3] = [Self::Silent, Self::Banner, Self::FullReply];

    pub fn label(self) -> &'static str {
        match self {
            Self::Silent => "不弹窗",
            Self::Banner => "隐藏内容",
            Self::FullReply => "显示内容",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Silent => "不弹窗",
            Self::Banner => "隐藏内容",
            Self::FullReply => "显示内容",
        }
    }

    pub fn description(self) -> &'static str {
        match self {
            Self::Silent => "新消息仅在「消息」与通知中心内静默归档，桌面不弹出提醒窗口",
            Self::Banner => "在电脑右下角显示 StepX 弹窗，只显示来源，隐藏正文和图片",
            Self::FullReply => "在 StepX 弹窗中显示消息和对应图片，支持后台回复时提供输入框",
        }
    }

    pub fn uses_interactive_card(self) -> bool {
        !matches!(self, Self::Silent)
    }

    pub fn uses_system_notification(self) -> bool {
        false
    }

    pub fn allows_popup(self) -> bool {
        !matches!(self, Self::Silent)
    }

    pub fn shows_full_body_and_reply(self) -> bool {
        matches!(self, Self::FullReply)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct NotifyPreferences {
    pub scope: AlertScope,
    pub popup_mode: PopupMode,
    pub privacy_mask: bool,
    pub auto_copy_otp: bool,
    pub sound_enabled: bool,
    pub pinned_targets: HashSet<String>,
    pub deleted_threads: HashMap<String, i64>,
    pub cleared_threads: HashMap<String, i64>,
    pub deleted_messages: HashSet<String>,
    pub manual_unread_targets: HashSet<String>,
    #[serde(default)]
    pub dismissed_pickup_codes: std::collections::BTreeSet<String>,
    /// Terminal logistics receipts, scoped to hardware identity (never the Wi-Fi address).
    #[serde(default)]
    pub parcel_receipts: std::collections::BTreeMap<String, crate::parcel_state::CompletionIndex>,
    #[serde(default)]
    pub app_popup_modes: std::collections::BTreeMap<String, PopupMode>,
}

impl Default for NotifyPreferences {
    fn default() -> Self {
        Self {
            scope: AlertScope::ImAndOtpOnly,
            popup_mode: PopupMode::FullReply,
            privacy_mask: false,
            auto_copy_otp: false,
            sound_enabled: true,
            pinned_targets: HashSet::new(),
            deleted_threads: HashMap::new(),
            cleared_threads: HashMap::new(),
            deleted_messages: HashSet::new(),
            manual_unread_targets: HashSet::new(),
            dismissed_pickup_codes: std::collections::BTreeSet::new(),
            parcel_receipts: Default::default(),
            app_popup_modes: std::collections::BTreeMap::new(),
        }
    }
}

impl NotifyPreferences {
    /// Settings are distinct from saved conversation state (pins, read/deleted rows).
    pub fn settings_eq(&self, other: &Self) -> bool {
        self.scope == other.scope
            && self.popup_mode == other.popup_mode
            && self.privacy_mask == other.privacy_mask
            && self.auto_copy_otp == other.auto_copy_otp
            && self.sound_enabled == other.sound_enabled
            && self.app_popup_modes == other.app_popup_modes
    }

    /// Apply an editor draft without rolling back conversation changes made meanwhile.
    pub fn with_settings_from(&self, draft: &Self) -> Self {
        let mut next = self.clone();
        next.scope = draft.scope;
        next.popup_mode = draft.popup_mode;
        next.privacy_mask = draft.privacy_mask;
        next.auto_copy_otp = draft.auto_copy_otp;
        next.sound_enabled = draft.sound_enabled;
        next.app_popup_modes = draft.app_popup_modes.clone();
        next
    }
    /// An explicit choice to show desktop reminders also exits global mute.
    /// Keep narrower source filters and all per-app overrides intact.
    pub fn set_popup_mode(&mut self, mode: PopupMode) {
        self.popup_mode = mode;
        match mode {
            PopupMode::Banner => self.privacy_mask = true,
            PopupMode::FullReply => self.privacy_mask = false,
            PopupMode::Silent => return,
        }
        if self.scope == AlertScope::Muted {
            self.scope = AlertScope::ImAndOtpOnly;
        }
    }

    pub fn normalize_app_key(package_name: &str) -> String {
        let trimmed = package_name.trim();
        if trimmed.is_empty() {
            return String::new();
        }
        if matches!(
            trimmed,
            "sms" | "mms" | "com.android.mms" | "com.google.android.apps.messaging"
        ) {
            return "com.android.mms".to_string();
        }
        if matches!(
            trimmed,
            "com.android.server.telecom"
                | "com.android.dialer"
                | "com.android.incallui"
                | "com.google.android.dialer"
                | "com.android.phone"
        ) {
            return "com.android.dialer".to_string();
        }
        trimmed.to_string()
    }

    pub fn app_popup_mode_override(&self, package_name: &str) -> Option<PopupMode> {
        let key = Self::normalize_app_key(package_name);
        if key.is_empty() {
            return None;
        }
        if let Some(m) = self
            .app_popup_modes
            .get(&key)
            .copied()
            .or_else(|| self.app_popup_modes.get(package_name.trim()).copied())
        {
            return Some(m);
        }
        if key == "com.android.dialer" {
            for alias in [
                "com.android.server.telecom",
                "com.android.incallui",
                "com.google.android.dialer",
                "com.android.phone",
            ] {
                if let Some(m) = self.app_popup_modes.get(alias).copied() {
                    return Some(m);
                }
            }
        }
        None
    }

    pub fn effective_popup_mode(&self, package_name: &str) -> PopupMode {
        if self.scope == AlertScope::Muted {
            return PopupMode::Silent;
        }
        let resolved = self
            .app_popup_mode_override(package_name)
            .unwrap_or(self.popup_mode);
        if self.privacy_mask && resolved == PopupMode::FullReply {
            return PopupMode::Banner;
        }
        resolved
    }

    pub fn allows_channel_in_scope(
        &self,
        package_name: &str,
        is_im_or_comm: bool,
        is_otp: bool,
        window_focused: bool,
    ) -> bool {
        let _ = (package_name, window_focused);
        match self.scope {
            AlertScope::All => true,
            AlertScope::ImAndOtpOnly => is_im_or_comm || is_otp,
            AlertScope::OtpOnly => is_otp,
            AlertScope::FocusHostOnly => is_otp || is_im_or_comm,
            AlertScope::Muted => false,
        }
    }

    pub fn set_app_popup_mode(&mut self, package_name: &str, mode: Option<PopupMode>) {
        let key = Self::normalize_app_key(package_name);
        if key.is_empty() {
            return;
        }
        if key == "com.android.dialer" {
            for alias in [
                "com.android.server.telecom",
                "com.android.incallui",
                "com.google.android.dialer",
                "com.android.phone",
            ] {
                self.app_popup_modes.remove(alias);
            }
        }
        match mode {
            Some(m) => {
                self.app_popup_modes.insert(key, m);
            }
            None => {
                self.app_popup_modes.remove(&key);
                self.app_popup_modes.remove(package_name.trim());
            }
        }
    }
    pub fn im_target_key(title_or_thread_id: &str) -> String {
        let mut pkg = "";
        let mut raw_title = title_or_thread_id.trim();
        if let Some((p, t)) = raw_title.split_once("::") {
            pkg = p.trim();
            raw_title = t.trim();
        }
        if pkg.is_empty() {
            pkg = "com.tencent.mm";
        }
        let clean = crate::push::normalize_conversation_title(raw_title);
        let effective = if clean.is_empty() {
            raw_title
        } else {
            clean.as_str()
        };
        format!("im:{pkg}::{effective}")
    }

    pub fn sms_target_key(address: &str) -> String {
        let norm = crate::sms::normalize_address(address);
        let effective = if norm.is_empty() {
            address.trim()
        } else {
            norm.as_str()
        };
        format!("sms:{effective}")
    }

    pub fn is_pinned(&self, target_key: &str) -> bool {
        self.pinned_targets.contains(target_key)
    }

    pub fn toggle_pin(&mut self, target_key: String) -> bool {
        if self.pinned_targets.contains(&target_key) {
            self.pinned_targets.remove(&target_key);
            false
        } else {
            self.pinned_targets.insert(target_key);
            true
        }
    }

    pub fn save(&self) -> anyhow::Result<()> {
        save_preferences(self)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct DesktopToastItem {
    pub id: String,
    pub notification_key: String,
    pub thread_id: String,
    pub package_name: String,
    pub app_name: String,
    pub category: String,
    pub title: String,
    pub sender: String,
    pub body: String,
    pub masked_body: String,
    pub otp_code: Option<String>,
    pub url_hint: Option<String>,
    pub can_reply: bool,
    pub timestamp_ms: i64,
}

impl DesktopToastItem {
    pub fn display_body(&self, privacy_mask: bool) -> &str {
        if privacy_mask {
            &self.masked_body
        } else {
            &self.body
        }
    }
}

fn prefs_path() -> PathBuf {
    crate::resources::user_config().join("stepx-companion/notify_prefs.json")
}

pub fn load_preferences() -> NotifyPreferences {
    let path = prefs_path();
    let Ok(raw) = fs::read_to_string(path) else {
        return NotifyPreferences::default();
    };
    serde_json::from_str(&raw).unwrap_or_default()
}

pub fn save_preferences(prefs: &NotifyPreferences) -> Result<()> {
    if cfg!(test) {
        return Ok(());
    }
    let path = prefs_path();
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent).context("无法创建通知偏好配置目录")?;
    }
    let payload = serde_json::to_vec_pretty(prefs).context("无法序列化通知偏好")?;
    crate::local_store::write_if_changed(&path, &payload).context("无法保存通知偏好配置文件")?;
    Ok(())
}

fn is_urgent_or_mention(notif: &PushNotification, otp_code: Option<&str>) -> bool {
    if otp_code.is_some() {
        return true;
    }
    let combined = format!("{} {} {}", notif.title, notif.sender, notif.text);
    if crate::push::notification_mentions_current_user(notif) {
        return true;
    }
    const URGENT_KEYWORDS: &[&str] = &[
        "紧急", "故障", "报警", "告警", "发版", "马上", "急", "P0", "P1", "urgent", "URGENT",
    ];
    URGENT_KEYWORDS.iter().any(|kw| combined.contains(kw))
}

fn build_masked_body(notif: &PushNotification, otp_code: Option<&str>) -> String {
    let app = if notif.app_name.is_empty() {
        "手机应用"
    } else {
        notif.app_name.as_str()
    };
    if otp_code.is_some() {
        format!("{app} · 收到一条验证码消息（悬停或点击查看）")
    } else if !notif.title.trim().is_empty() {
        format!("{app} · 收到来自「{}」的新消息", notif.title.trim())
    } else {
        format!("{app} · 收到 1 条新推送通知")
    }
}

fn should_popup_for_scope(
    scope: AlertScope,
    notif: &PushNotification,
    otp_code: Option<&str>,
) -> bool {
    match scope {
        AlertScope::All => true,
        AlertScope::ImAndOtpOnly => {
            otp_code.is_some()
                || notif.category.eq_ignore_ascii_case("im")
                || notif.can_reply
                || !notif.thread_id.is_empty()
        }
        AlertScope::OtpOnly => otp_code.is_some(),
        AlertScope::FocusHostOnly => is_urgent_or_mention(notif, otp_code),
        AlertScope::Muted => false,
    }
}

/// Detects newly arrived notifications from `next` that should trigger a desktop toast.
///
/// - Deduplicates against `seen_keys`.
/// - Suppresses notifications belonging to `active_thread_id`.
/// - Filters according to `prefs.scope` (including `FocusHostOnly` AI 专注托管).
pub fn detect_new_toasts(
    prev_revision: u64,
    seen_keys: &mut HashSet<String>,
    next: &PushSnapshot,
    prefs: &NotifyPreferences,
    active_thread_id: Option<&str>,
) -> Vec<DesktopToastItem> {
    if prev_revision > 0 && next.revision <= prev_revision {
        return Vec::new();
    }

    let mut items = Vec::new();
    for notif in &next.notifications {
        if notif.dismissed || notif.is_ongoing || notif.read {
            continue;
        }

        let base_key = if !notif.key.is_empty() {
            notif.key.clone()
        } else {
            format!("seq:{}", notif.seq_id)
        };
        let dedup_key = format!(
            "{}::{}::{}",
            base_key,
            notif.post_time_ms,
            notif.text.trim()
        );
        if !seen_keys.insert(dedup_key.clone()) {
            continue;
        }

        let effective_mode = prefs.effective_popup_mode(&notif.package_name);
        if !effective_mode.allows_popup() {
            continue;
        }

        if prefs.app_popup_mode_override(&notif.package_name).is_none()
            && prefs.scope == AlertScope::Muted
        {
            continue;
        }

        if let Some(active) = active_thread_id {
            if !active.is_empty() && active == notif.thread_id.as_str() {
                continue;
            }
        }

        let otp_code = notif.otp_code.clone().or_else(|| {
            let combined = format!("{} {}", notif.title, notif.text);
            extract_otp_code(&combined)
        });

        if !should_popup_for_scope(prefs.scope, notif, otp_code.as_deref()) {
            continue;
        }

        let url_hint = notif
            .url_hint
            .clone()
            .or_else(|| extract_first_url(&notif.text));

        let body = if !notif.sender.is_empty()
            && notif.sender != notif.title
            && !notif.text.starts_with(&notif.sender)
        {
            format!("{}: {}", notif.sender, notif.text)
        } else {
            notif.text.clone()
        };
        let masked_body = build_masked_body(notif, otp_code.as_deref());

        let id = if notif.seq_id > 0 {
            format!("toast_{}_{}", notif.seq_id, notif.post_time_ms)
        } else {
            format!("toast_{}", dedup_key)
        };

        items.push(DesktopToastItem {
            id,
            notification_key: notif.key.clone(),
            thread_id: notif.thread_id.clone(),
            package_name: notif.package_name.clone(),
            app_name: if notif.app_name.is_empty() {
                notif.package_name.clone()
            } else {
                notif.app_name.clone()
            },
            category: notif.category.clone(),
            title: notif.title.clone(),
            sender: notif.sender.clone(),
            body,
            masked_body,
            otp_code,
            url_hint,
            can_reply: notif.can_reply,
            timestamp_ms: notif.post_time_ms,
        });
    }

    items
}

#[allow(dead_code)]
pub fn escape_applescript(input: &str) -> String {
    input
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('\n', " ")
        .replace('\r', " ")
}

#[allow(dead_code)]
pub fn escape_xml(input: &str) -> String {
    input
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[allow(dead_code)]
pub fn build_macos_osascript(summary: &str, subtitle: &str, body: &str) -> String {
    format!(
        "display notification \"{}\" with title \"{}\" subtitle \"{}\"",
        escape_applescript(body),
        escape_applescript(summary),
        escape_applescript(subtitle)
    )
}

#[allow(dead_code)]
pub fn build_windows_toast_ps_script(summary: &str, body: &str) -> String {
    let xml_title = escape_xml(summary).replace('\'', "''");
    let xml_body = escape_xml(body).replace('\'', "''");
    format!(
        "$ErrorActionPreference = 'SilentlyContinue'; \
         [Windows.UI.Notifications.ToastNotificationManager, Windows.UI.Notifications, ContentType = WindowsRuntime] > $null; \
         [Windows.Data.Xml.Dom.XmlDocument, Windows.Data.Xml.Dom.XmlDocument, ContentType = WindowsRuntime] > $null; \
         $xml = New-Object Windows.Data.Xml.Dom.XmlDocument; \
         $xml.LoadXml('<toast><visual><binding template=\"ToastGeneric\"><text>{xml_title}</text><text>{xml_body}</text></binding></visual></toast>'); \
         $toast = [Windows.UI.Notifications.ToastNotification]::new($xml); \
         [Windows.UI.Notifications.ToastNotificationManager]::CreateToastNotifier('StepX.Companion').Show($toast);"
    )
}

// Run on the notification worker, never the UI thread. A successful spawn is
// not a delivery receipt: notify-send may exit with a missing/failed D-Bus server.
fn run_notification_command(mut command: Command) -> Result<()> {
    command
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let mut child = command.spawn().context("无法启动系统通知进程")?;
    for _ in 0..30 {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(status.success(), "系统通知服务未接受提醒（{status}）");
            return Ok(());
        }
        thread::sleep(Duration::from_millis(100));
    }
    let _ = child.kill();
    let _ = child.wait();
    anyhow::bail!("系统通知服务响应超时")
}

/// Dispatches a native OS notification on Linux, macOS, or Windows.
pub fn send_system_notification(item: &DesktopToastItem, privacy_mask: bool) -> Result<()> {
    send_system_notification_with_sound(item, privacy_mask, true)
}

pub fn send_system_notification_with_sound(
    item: &DesktopToastItem,
    privacy_mask: bool,
    sound_enabled: bool,
) -> Result<()> {
    let app_label = if item.app_name.is_empty() {
        "StepX 伴侣"
    } else {
        item.app_name.as_str()
    };
    let summary = if item.title.is_empty() {
        format!("StepX · {app_label}")
    } else {
        format!("[{app_label}] {}", item.title)
    };
    let body = item.display_body(privacy_mask);

    #[cfg(target_os = "linux")]
    {
        let mut cmd = Command::new("notify-send");
        if !sound_enabled {
            cmd.args(["-h", "boolean:suppress-sound:true"]);
        }
        cmd.args([
            "-a",
            "StepX Companion",
            "-u",
            "normal",
            "--",
            summary.as_str(),
            &escape_xml(body),
        ]);
        if run_notification_command(cmd).is_err() {
            let mut gdbus = Command::new("gdbus");
            gdbus.args([
                "call",
                "--session",
                "--dest",
                "org.freedesktop.Notifications",
                "--object-path",
                "/org/freedesktop/Notifications",
                "--method",
                "org.freedesktop.Notifications.Notify",
                "StepX Companion",
                "0",
                "",
                summary.as_str(),
                &escape_xml(body),
                "[]",
                if sound_enabled {
                    "{}"
                } else {
                    "{'suppress-sound': <true>}"
                },
                "5000",
            ]);
            run_notification_command(gdbus)?;
        }
        Ok(())
    }

    #[cfg(target_os = "macos")]
    {
        let _ = sound_enabled; // No sound-name is requested from Notification Center.
        let script = build_macos_osascript("StepX Companion", &summary, body);
        let mut cmd = Command::new("osascript");
        cmd.args(["-e", &script]);
        run_notification_command(cmd)
    }

    #[cfg(target_os = "windows")]
    {
        let mut ps_script = build_windows_toast_ps_script(&summary, body);
        if !sound_enabled {
            ps_script = ps_script.replace("</visual>", "</visual><audio silent=\"true\"/>");
        }
        let mut cmd = Command::new("powershell");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-WindowStyle",
            "Hidden",
            "-Command",
            &ps_script,
        ]);
        run_notification_command(cmd)
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        let _ = (summary, body);
        anyhow::bail!("当前系统没有可用的原生通知通道")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::push::demo_push_snapshot;

    #[test]
    fn enabling_popup_exits_stale_mute_without_resetting_source_preferences() {
        let mut prefs = NotifyPreferences {
            scope: AlertScope::Muted,
            popup_mode: PopupMode::Silent,
            ..Default::default()
        };
        prefs.set_app_popup_mode("com.taobao.taobao", Some(PopupMode::Silent));
        prefs.set_popup_mode(PopupMode::Banner);
        assert_eq!(prefs.scope, AlertScope::ImAndOtpOnly);
        assert_eq!(
            prefs.effective_popup_mode("com.tencent.mm"),
            PopupMode::Banner
        );
        assert_eq!(
            prefs.effective_popup_mode("com.taobao.taobao"),
            PopupMode::Silent
        );
        prefs.scope = AlertScope::OtpOnly;
        prefs.set_popup_mode(PopupMode::FullReply);
        assert_eq!(prefs.scope, AlertScope::OtpOnly);
        assert!(!prefs.privacy_mask);
        prefs.set_popup_mode(PopupMode::Silent);
        assert_eq!(prefs.scope, AlertScope::OtpOnly);
    }

    #[test]
    fn conversation_changes_are_not_unsaved_settings_and_survive_apply() {
        let mut current = NotifyPreferences::default();
        let mut draft = current.clone();
        current.pinned_targets.insert("im:one".into());
        current.manual_unread_targets.insert("im:two".into());
        current.deleted_messages.insert("removed".into());
        current.deleted_threads.insert("old".into(), 123);
        current.cleared_threads.insert("cleared".into(), 456);
        current.dismissed_pickup_codes.insert("parcel".into());
        assert!(current.settings_eq(&draft));
        draft.set_popup_mode(PopupMode::Banner);
        assert!(!current.settings_eq(&draft));
        let applied = current.with_settings_from(&draft);
        assert!(applied.settings_eq(&draft));
        assert_eq!(applied.with_settings_from(&current), current);
    }

    #[test]
    fn all_enabled_popup_modes_use_stepx_owned_window() {
        assert!(!PopupMode::Banner.uses_system_notification());
        assert!(PopupMode::Banner.uses_interactive_card());
        assert!(PopupMode::FullReply.uses_interactive_card());
        assert!(!PopupMode::FullReply.uses_system_notification());
        assert!(!PopupMode::Silent.allows_popup());
    }

    #[cfg(unix)]
    #[test]
    fn notification_process_failure_is_not_a_delivery_receipt() {
        let mut command = Command::new("sh");
        command.args(["-c", "exit 17"]);
        assert!(run_notification_command(command).is_err());
        let mut command = Command::new("sh");
        command.args(["-c", "exit 0"]);
        assert!(run_notification_command(command).is_ok());
    }

    #[test]
    fn detects_new_toasts_and_deduplicates_seen_keys() {
        let snapshot = demo_push_snapshot();
        let prefs = NotifyPreferences::default(); // ImAndOtpOnly
        let mut seen = HashSet::new();

        let first_batch = detect_new_toasts(0, &mut seen, &snapshot, &prefs, None);
        // Should include IM notifications and OTP/pickup notifications, but skip pure finance/social non-reply notifications.
        assert!(!first_batch.is_empty());
        assert!(first_batch.iter().any(|t| t.title == "产品研发群"));
        assert!(first_batch
            .iter()
            .any(|t| t.otp_code.as_deref() == Some("839201")));
        assert!(!first_batch.iter().any(|t| t.category == "finance"));

        // Second call with same seen_keys should deduplicate everything.
        let second_batch = detect_new_toasts(42, &mut seen, &snapshot, &prefs, None);
        assert!(second_batch.is_empty());
    }

    #[test]
    fn suppresses_active_thread_and_filters_focus_host_only() {
        let snapshot = demo_push_snapshot();
        let mut prefs = NotifyPreferences {
            scope: AlertScope::FocusHostOnly,
            ..NotifyPreferences::default()
        };

        let mut seen = HashSet::new();
        let focus_toasts = detect_new_toasts(
            0,
            &mut seen,
            &snapshot,
            &prefs,
            Some("com.tencent.mm::产品研发群"),
        );

        // `产品研发群` has `@全体` and `发版`, so it would match FocusHostOnly,
        // EXCEPT it is suppressed because `active_thread_id` matches `com.tencent.mm::产品研发群`!
        assert!(!focus_toasts
            .iter()
            .any(|t| t.thread_id == "com.tencent.mm::产品研发群"));

        // `运维告警群` has `紧急` / `告警`, so it must pop up in FocusHostOnly mode.
        assert!(focus_toasts.iter().any(|t| t.title == "运维告警群"));

        // CMB OTP `839201` has an OTP code, so it must pop up in FocusHostOnly mode.
        assert!(focus_toasts
            .iter()
            .any(|t| t.otp_code.as_deref() == Some("839201")));

        // `李四` (normal PR link without urgent keyword or @) must be silenced in FocusHostOnly mode.
        assert!(!focus_toasts.iter().any(|t| t.title == "李四"));

        // Muted mode suppresses all toasts.
        prefs.scope = AlertScope::Muted;
        let mut seen_muted = HashSet::new();
        assert!(detect_new_toasts(0, &mut seen_muted, &snapshot, &prefs, None).is_empty());
    }

    #[test]
    fn important_notifications_do_not_promote_other_people_or_broadcast_mentions() {
        for body in [
            "@李四 请看一下",
            "@所有人 今天开会",
            "@all hello",
            "email@example.com",
        ] {
            let n = PushNotification {
                text: body.into(),
                ..Default::default()
            };
            assert!(!is_urgent_or_mention(&n, None), "{body}");
        }
        let n = PushNotification {
            text: "[有人@我] 张三: 请看一下".into(),
            ..Default::default()
        };
        assert!(is_urgent_or_mention(&n, None));
    }

    #[test]
    fn applies_privacy_masking_and_serializes_preferences() {
        let snapshot = demo_push_snapshot();
        let prefs = NotifyPreferences {
            scope: AlertScope::ImAndOtpOnly,
            popup_mode: PopupMode::InteractiveOnly,
            privacy_mask: true,
            auto_copy_otp: true,
            sound_enabled: false,
            ..NotifyPreferences::default()
        };
        let encoded = serde_json::to_string(&prefs).expect("serialize prefs");
        let decoded: NotifyPreferences = serde_json::from_str(&encoded).expect("deserialize prefs");
        assert_eq!(prefs, decoded);

        let mut seen = HashSet::new();
        let toasts = detect_new_toasts(0, &mut seen, &snapshot, &prefs, None);
        let wx_toast = toasts
            .iter()
            .find(|t| t.title == "产品研发群")
            .expect("wx toast");
        assert!(wx_toast.display_body(false).contains("发版"));
        assert!(!wx_toast.display_body(true).contains("发版"));
        assert!(wx_toast.display_body(true).contains("产品研发群"));

        // Also verify cross-platform script escaping helpers.
        let mac_script = build_macos_osascript("StepX", "微信 \"群\"", "你好\n世界");
        assert!(mac_script.contains("\\\"群\\\""));
        let win_script = build_windows_toast_ps_script("微信 <测试>", "A & B 'quote'");
        assert!(win_script.contains("&lt;测试&gt;"));
        assert!(win_script.contains("&amp;"));
    }

    #[test]
    fn three_popup_modes_and_per_app_overrides_work_across_all_apps() {
        let mut prefs = NotifyPreferences::default();
        assert_eq!(prefs.popup_mode, PopupMode::FullReply);
        assert_eq!(
            prefs.effective_popup_mode("com.tencent.mm"),
            PopupMode::FullReply
        );
        assert_eq!(
            prefs.effective_popup_mode("com.xingin.xhs"),
            PopupMode::FullReply
        );

        // Set global to Banner ("只提醒有消息"), override WeChat to FullReply and Taobao to Silent
        prefs.popup_mode = PopupMode::Banner;
        prefs.set_app_popup_mode("com.tencent.mm", Some(PopupMode::FullReply));
        prefs.set_app_popup_mode("com.taobao.taobao", Some(PopupMode::Silent));
        prefs.set_app_popup_mode("sms", Some(PopupMode::FullReply));

        assert_eq!(
            prefs.effective_popup_mode("com.tencent.mm"),
            PopupMode::FullReply
        );
        assert_eq!(
            prefs.effective_popup_mode("com.taobao.taobao"),
            PopupMode::Silent
        );
        assert_eq!(
            prefs.effective_popup_mode("com.xingin.xhs"),
            PopupMode::Banner
        );
        assert_eq!(
            prefs.effective_popup_mode("com.android.mms"),
            PopupMode::FullReply
        );

        // Clearing an app override reverts to global mode
        prefs.set_app_popup_mode("com.tencent.mm", None);
        assert_eq!(
            prefs.effective_popup_mode("com.tencent.mm"),
            PopupMode::Banner
        );
    }
}
