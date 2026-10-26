//! Phone-to-desktop IM Chat Hub & full phone push notification center client.
//!
//! Supports dual-stack transport over same-Wi-Fi or USB:
//! 1. Fast local TCP stream via `adb forward tcp:0 localabstract:stepx_push_v1`
//!    (zero JVM fork overhead, < 15ms round-trip latency).
//! 2. Seamless fallback to `content call --uri content://com.stepx.companion.app.push`
//!    with Base64-encoded JSON payloads.

use crate::adb::Adb;
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const PUSH_SOCKET_NAME: &str = "localabstract:stepx_push_v1";
const PUSH_PROVIDER_URI: &str = "content://com.stepx.companion.app.push";
const LISTENER_COMPONENT: &str =
    "com.stepx.companion.app/com.stepx.companion.app.PushNotificationService";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ImChannel {
    #[default]
    All,
    WeChat,
    WeCom,
    DingTalk,
    Feishu,
    QqTim,
    GlobalIm,
    SocialDm,
    Sms,
}

impl ImChannel {
    pub const ALL: [ImChannel; 8] = [
        Self::All,
        Self::WeChat,
        Self::WeCom,
        Self::DingTalk,
        Self::Feishu,
        Self::QqTim,
        Self::GlobalIm,
        Self::Sms,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部",
            Self::WeChat => "微信",
            Self::WeCom => "企微",
            Self::DingTalk => "钉钉",
            Self::Feishu => "飞书",
            Self::QqTim => "QQ/TIM",
            Self::GlobalIm => "国际IM",
            Self::SocialDm => "社媒私信",
            Self::Sms => "短信",
        }
    }

    pub fn badge(self) -> &'static str {
        match self {
            Self::All => "全",
            Self::WeChat => "微",
            Self::WeCom => "企",
            Self::DingTalk => "钉",
            Self::Feishu => "飞",
            Self::QqTim => "Q",
            Self::GlobalIm => "IM",
            Self::SocialDm => "私",
            Self::Sms => "短",
        }
    }

    pub fn from_package(pkg: &str) -> Self {
        let trimmed = pkg.trim();
        if trimmed.is_empty() {
            return Self::All;
        }
        match trimmed {
            "com.tencent.mm" => Self::WeChat,
            "com.tencent.wework" => Self::WeCom,
            "com.alibaba.android.rimet" => Self::DingTalk,
            "com.ss.android.lark" | "com.larksuite.suite" => Self::Feishu,
            "com.tencent.mobileqq" | "com.tencent.tim" | "com.tencent.qqlite" => Self::QqTim,
            "org.telegram.messenger"
            | "org.telegram.messenger.web"
            | "org.thunderdog.challegram"
            | "tw.nekomimi.nekogram"
            | "com.whatsapp"
            | "com.whatsapp.w4b"
            | "com.Slack"
            | "com.slack"
            | "com.discord"
            | "com.microsoft.teams"
            | "org.thoughtcrime.securesms"
            | "com.facebook.orca"
            | "jp.naver.line.android" => Self::GlobalIm,
            "com.xingin.xhs"
            | "com.ss.android.ugc.aweme"
            | "com.taobao.idlefish"
            | "com.sina.weibo"
            | "tv.danmaku.bili"
            | "com.zhihu.android" => Self::SocialDm,
            "com.android.mms"
            | "com.google.android.apps.messaging"
            | "com.samsung.android.messaging"
            | "com.miui.mms"
            | "sms"
            | "telephony" => Self::Sms,
            _ => {
                let lower = trimmed.to_ascii_lowercase();
                if lower.contains("tencent.mm") {
                    Self::WeChat
                } else if lower.contains("wework") {
                    Self::WeCom
                } else if lower.contains("rimet") || lower.contains("dingtalk") {
                    Self::DingTalk
                } else if lower.contains("lark") || lower.contains("feishu") {
                    Self::Feishu
                } else if lower.contains("mobileqq") || lower.contains("tencent.tim") {
                    Self::QqTim
                } else if lower.contains("mms")
                    || lower.contains("messaging")
                    || lower == "sms"
                    || lower.contains("telephony")
                {
                    Self::Sms
                } else if lower.contains("xhs")
                    || lower.contains("aweme")
                    || lower.contains("idlefish")
                    || lower.contains("weibo")
                {
                    Self::SocialDm
                } else {
                    Self::GlobalIm
                }
            }
        }
    }

    pub fn matches_package(self, pkg: &str) -> bool {
        if is_excluded_im_package(pkg) {
            return false;
        }
        if self == Self::All {
            return true;
        }
        let detected = Self::from_package(pkg);
        if detected == self {
            return true;
        }
        // When filtering via the 8-tab bar (`ImChannel::ALL`), include `SocialDm` under `GlobalIm`
        // unless the caller specifically selected `SocialDm`.
        self == Self::GlobalIm && detected == Self::SocialDm
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum PushCategory {
    #[default]
    All,
    Im,
    Social,
    Finance,
    DeliveryTravel,
    SystemSecurity,
    Other,
}

impl PushCategory {
    pub const ALL: [PushCategory; 7] = [
        Self::All,
        Self::Im,
        Self::Social,
        Self::Finance,
        Self::DeliveryTravel,
        Self::SystemSecurity,
        Self::Other,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::All => "全部推送",
            Self::Im => "即时通讯",
            Self::Social => "社交互动",
            Self::Finance => "交易账单",
            Self::DeliveryTravel => "物流出行",
            Self::SystemSecurity => "系统安全",
            Self::Other => "其他资讯",
        }
    }

    pub fn from_str_code(code: &str) -> Self {
        match code.trim().to_ascii_lowercase().as_str() {
            "" | "all" => Self::All,
            "im" | "chat" | "message" | "msg" => Self::Im,
            "social" => Self::Social,
            "finance" | "bill" | "payment" => Self::Finance,
            "delivery_travel" | "delivery" | "travel" | "logistics" => Self::DeliveryTravel,
            "system" | "system_security" | "security" | "otp" => Self::SystemSecurity,
            _ => Self::Other,
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::All => "all",
            Self::Im => "im",
            Self::Social => "social",
            Self::Finance => "finance",
            Self::DeliveryTravel => "delivery_travel",
            Self::SystemSecurity => "system",
            Self::Other => "other",
        }
    }

    pub fn matches(self, cat: &str) -> bool {
        self == Self::All || Self::from_str_code(cat) == self
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
pub enum ReplyMode {
    #[default]
    Auto,
    RemoteInputOnly,
    UiBridgeFallback,
}

impl ReplyMode {
    pub const ALL: [ReplyMode; 3] = [Self::Auto, Self::RemoteInputOnly, Self::UiBridgeFallback];

    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动择优（静默秒回 + 看屏兜底）",
            Self::RemoteInputOnly => "仅通知栏静默秒回",
            Self::UiBridgeFallback => "无障碍看屏发送",
        }
    }

    pub fn code(self) -> &'static str {
        match self {
            Self::Auto => "auto",
            Self::RemoteInputOnly => "remote_only",
            Self::UiBridgeFallback => "ui_bridge",
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ImMessage {
    pub id: String,
    pub sender: String,
    pub text: String,
    #[serde(alias = "timestampMs", alias = "timestamp")]
    pub timestamp_ms: i64,
    pub outgoing: bool,
    #[serde(alias = "deliveryStatus", alias = "status")]
    pub delivery_status: String,
    pub via: String,
    #[serde(alias = "mediaHint")]
    pub media_hint: String,
    #[serde(alias = "clientMsgId")]
    pub client_msg_id: String,
}

impl ImMessage {
    pub fn same_delivery(&self, other: &Self) -> bool {
        (!self.id.is_empty() && self.id == other.id)
            || (!self.client_msg_id.is_empty()
                && (self.client_msg_id == other.client_msg_id || self.client_msg_id == other.id))
            || (!other.client_msg_id.is_empty() && other.client_msg_id == self.id)
    }
}

/// Preserve local sends across snapshots, replacing them by correlated phone records.
/// When the phone reports `unchanged == true` for a delta poll (`since_revision > 0`),
/// preserve the previous threads, notifications, and OTP banners while keeping updated device state.
pub fn merge_local_replies(next: &mut PushSnapshot, previous: &PushSnapshot) {
    if next.unchanged {
        if next.revision == 0 {
            next.revision = previous.revision;
        }
        if next.threads.is_empty() {
            next.threads = previous.threads.clone();
        }
        if next.notifications.is_empty() {
            next.notifications = previous.notifications.clone();
        }
        if next.otp_banners.is_empty() {
            next.otp_banners = previous.otp_banners.clone();
        }
        next.unchanged = false;
    }
    for old in &previous.threads {
        let pending: Vec<_> = old
            .messages
            .iter()
            .filter(|m| m.outgoing && !m.client_msg_id.is_empty() && m.id == m.client_msg_id)
            .collect();
        if pending.is_empty() {
            continue;
        }
        if let Some(thread) = next
            .threads
            .iter_mut()
            .find(|t| t.thread_id == old.thread_id)
        {
            for message in pending {
                if !thread.messages.iter().any(|m| m.same_delivery(message)) {
                    thread.messages.push(message.clone());
                }
            }
        } else {
            next.threads.push(old.clone());
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ImThread {
    #[serde(alias = "threadId", alias = "id")]
    pub thread_id: String,
    #[serde(alias = "packageName")]
    pub package_name: String,
    #[serde(alias = "appName")]
    pub app_name: String,
    pub title: String,
    #[serde(alias = "isGroup", alias = "group")]
    pub is_group: bool,
    #[serde(alias = "unreadCount")]
    pub unread_count: usize,
    #[serde(alias = "lastSender")]
    pub last_sender: String,
    #[serde(alias = "lastMessage", alias = "lastText")]
    pub last_message: String,
    #[serde(alias = "lastTimestampMs", alias = "updatedAtMs")]
    pub last_timestamp_ms: i64,
    #[serde(alias = "canRemoteReply", alias = "canReply")]
    pub can_remote_reply: bool,
    #[serde(alias = "canUiReply")]
    pub can_ui_reply: bool,
    #[serde(alias = "hasContentIntent")]
    pub has_content_intent: bool,
    #[serde(alias = "notificationKey", alias = "lastKey")]
    pub notification_key: String,
    pub messages: Vec<ImMessage>,
}

impl ImThread {
    pub fn channel(&self) -> ImChannel {
        ImChannel::from_package(&self.package_name)
    }

    pub fn can_reply(&self) -> bool {
        self.can_remote_reply || self.can_ui_reply
    }
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PushNotification {
    pub key: String,
    #[serde(alias = "seqId", alias = "id")]
    pub seq_id: u64,
    #[serde(alias = "packageName")]
    pub package_name: String,
    #[serde(alias = "appName")]
    pub app_name: String,
    pub category: String,
    pub title: String,
    pub sender: String,
    pub text: String,
    #[serde(alias = "subText")]
    pub sub_text: String,
    #[serde(alias = "postTimeMs", alias = "timestampMs")]
    pub post_time_ms: i64,
    #[serde(alias = "isOngoing", alias = "ongoing")]
    pub is_ongoing: bool,
    #[serde(default, alias = "clearable", alias = "isClearable")]
    pub is_clearable: bool,
    #[serde(default = "default_true")]
    pub active: bool,
    #[serde(default, alias = "removedTimeMs")]
    pub removed_time_ms: i64,
    #[serde(alias = "isGroup", alias = "group")]
    pub is_group: bool,
    pub dismissed: bool,
    pub read: bool,
    #[serde(alias = "canReply")]
    pub can_reply: bool,
    #[serde(alias = "threadId")]
    pub thread_id: String,
    #[serde(alias = "otpCode")]
    pub otp_code: Option<String>,
    #[serde(alias = "actionLabels", alias = "actions")]
    pub action_labels: Vec<String>,
    #[serde(alias = "urlHint")]
    pub url_hint: Option<String>,
}

impl Default for PushNotification {
    fn default() -> Self {
        Self {
            key: String::new(),
            seq_id: 0,
            package_name: String::new(),
            app_name: String::new(),
            category: String::new(),
            title: String::new(),
            sender: String::new(),
            text: String::new(),
            sub_text: String::new(),
            post_time_ms: 0,
            is_ongoing: false,
            is_clearable: false,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: String::new(),
            otp_code: None,
            action_labels: Vec::new(),
            url_hint: None,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct OtpBanner {
    pub code: String,
    #[serde(alias = "sourceApp", alias = "appName", alias = "app_name")]
    pub source_app: String,
    pub title: String,
    #[serde(alias = "rawText", alias = "text")]
    pub raw_text: String,
    #[serde(alias = "timestampMs", alias = "postTimeMs", alias = "post_time_ms")]
    pub timestamp_ms: i64,
    #[serde(alias = "notificationKey", alias = "key")]
    pub notification_key: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct PushSnapshot {
    pub revision: u64,
    pub unchanged: bool,
    #[serde(alias = "listenerConnected")]
    pub listener_connected: bool,
    #[serde(alias = "accessibilityConnected")]
    pub accessibility_connected: bool,
    #[serde(alias = "wifiAdbEnabled")]
    pub wifi_adb_enabled: bool,
    #[serde(alias = "screenInteractive")]
    pub screen_interactive: bool,
    #[serde(alias = "keyguardLocked")]
    pub keyguard_locked: bool,
    #[serde(alias = "humanTouchRecent")]
    pub human_touch_recent: bool,
    #[serde(alias = "transportMode")]
    pub transport_mode: String,
    pub threads: Vec<ImThread>,
    pub notifications: Vec<PushNotification>,
    #[serde(alias = "otpBanners")]
    pub otp_banners: Vec<OtpBanner>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct ReplyReceipt {
    pub ok: bool,
    #[serde(alias = "threadId")]
    pub thread_id: String,
    #[serde(alias = "clientMsgId")]
    pub client_msg_id: String,
    pub via: String,
    #[serde(alias = "error")]
    pub message: String,
    #[serde(alias = "timestampMs")]
    pub timestamp_ms: i64,
}

const SOCKET_MISS_TTL: Duration = Duration::from_secs(15);

#[derive(Clone, Copy, Debug)]
struct ForwardCacheEntry {
    port: Option<u16>,
    updated_at: Instant,
}

fn forward_cache() -> &'static Mutex<HashMap<String, ForwardCacheEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ForwardCacheEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn invalidate_cached_port(serial: &str) {
    if let Ok(mut guard) = forward_cache().lock() {
        guard.remove(serial);
    }
}

fn mark_socket_unavailable(serial: &str) {
    if let Ok(mut guard) = forward_cache().lock() {
        guard.insert(
            serial.to_string(),
            ForwardCacheEntry {
                port: None,
                updated_at: Instant::now(),
            },
        );
    }
}

fn is_socket_recently_unavailable(serial: &str, now: Instant) -> bool {
    if let Ok(guard) = forward_cache().lock() {
        if let Some(entry) = guard.get(serial) {
            if entry.port.is_none()
                && now.saturating_duration_since(entry.updated_at) < SOCKET_MISS_TTL
            {
                return true;
            }
        }
    }
    false
}

fn obtain_forwarded_port(adb: &Adb, serial: &str, force_refresh: bool) -> Result<(u16, bool)> {
    if !force_refresh {
        if let Ok(guard) = forward_cache().lock() {
            if let Some(entry) = guard.get(serial) {
                if let Some(port) = entry.port {
                    // Re-establish forward if the laptop likely slept or the port is older than 5 minutes.
                    if entry.updated_at.elapsed() <= Duration::from_secs(300) {
                        return Ok((port, true));
                    }
                }
            }
        }
    }

    let out = adb.run_timeout(
        &["-s", serial, "forward", "tcp:0", PUSH_SOCKET_NAME],
        Duration::from_millis(2500),
    )?;
    let port: u16 = out
        .lines()
        .find_map(|line| line.trim().parse::<u16>().ok())
        .context("无法解析 adb forward 分配的本地端口")?;

    if let Ok(mut guard) = forward_cache().lock() {
        guard.insert(
            serial.to_string(),
            ForwardCacheEntry {
                port: Some(port),
                updated_at: Instant::now(),
            },
        );
    }
    Ok((port, false))
}

fn try_call_local_socket(adb: &Adb, serial: &str, request: &Value) -> Result<Value> {
    if is_socket_recently_unavailable(serial, Instant::now()) {
        anyhow::bail!("本地推送流连接不可用（冷却期内直接回退）");
    }
    let mut payload = serde_json::to_vec(request)?;
    ensure!(payload.len() <= 32768, "推送请求过长");
    payload.push(b'\n');

    for attempt in 0..2 {
        let force_refresh = attempt > 0;
        let (port, from_cache) = match obtain_forwarded_port(adb, serial, force_refresh) {
            Ok(res) => res,
            Err(e) => {
                mark_socket_unavailable(serial);
                return Err(e);
            }
        };

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let io_result = (|| -> Result<Value> {
            let mut stream = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
                .context("连接本地推送流端口超时")?;
            let _ = stream.set_nodelay(true);
            stream.set_write_timeout(Some(Duration::from_millis(1500)))?;
            stream.set_read_timeout(Some(Duration::from_millis(1500)))?;
            stream.write_all(&payload)?;
            stream.flush()?;

            let mut reader = BufReader::new(stream);
            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            ensure!(n > 0, "本地推送流已断开 (EOF)");
            let mut value: Value =
                serde_json::from_str(line.trim()).context("解析推送流 JSON 失败")?;
            if let Some(obj) = value.as_object_mut() {
                obj.entry("transport_mode".to_string())
                    .or_insert_with(|| Value::String("local_socket".to_string()));
            }
            Ok(value)
        })();

        match io_result {
            Ok(val) => {
                if let Ok(mut guard) = forward_cache().lock() {
                    if let Some(entry) = guard.get_mut(serial) {
                        entry.port = Some(port);
                        entry.updated_at = Instant::now();
                    }
                }
                ensure!(
                    val.get("ok").and_then(Value::as_bool).unwrap_or(true),
                    "{}",
                    val["error"].as_str().unwrap_or("手机推送服务返回错误")
                );
                return Ok(val);
            }
            Err(err) => {
                if !from_cache || attempt == 1 {
                    mark_socket_unavailable(serial);
                    return Err(err);
                }
                invalidate_cached_port(serial);
            }
        }
    }
    anyhow::bail!("本地推送流连接不可用")
}

/// Dual-stack RPC transport to the phone's `PushNotificationService` / `PushControlProvider`.
///
/// 1. First attempts fast local TCP stream via cached `adb forward tcp:0 localabstract:stepx_push_v1`
///    (1.5s read/write timeout, automatic port cache per serial, auto-reconnect on broken pipe).
/// 2. Falls back seamlessly to `content call --uri content://com.stepx.companion.app.push --method request --arg <base64>`.
fn replay_safe(request: &Value) -> bool {
    matches!(
        request["op"].as_str(),
        Some("snapshot" | "status" | "ping" | "thread_detail" | "sync_window")
    )
}

pub fn call_push_provider(adb: &Adb, serial: &str, request: Value) -> Result<Value> {
    // Read requests may retry across transports. A send may already have reached
    // the phone when a socket times out: never replay it through another channel.
    if replay_safe(&request) {
        if let Ok(response) = try_call_local_socket(adb, serial, &request) {
            return Ok(response);
        }
    }

    let encoded = STANDARD.encode(serde_json::to_vec(&request)?);
    ensure!(encoded.len() <= 32768, "推送请求过长");
    let result = adb.shell(
        serial,
        &[
            "content",
            "call",
            "--user",
            "0",
            "--uri",
            PUSH_PROVIDER_URI,
            "--method",
            "request",
            "--arg",
            &encoded,
        ],
    )?;
    let raw = result
        .split("result=")
        .nth(1)
        .context("手机没有返回推送服务响应，请安装或更新新版 StepX伴侣")?
        .split(|c: char| !c.is_ascii_alphanumeric() && !"+/=".contains(c))
        .next()
        .unwrap_or("");
    let mut value: Value =
        serde_json::from_slice(&STANDARD.decode(raw).context("推送响应 Base64 解码失败")?)
            .context("推送响应 JSON 解析失败")?;
    ensure!(
        value.get("ok").and_then(Value::as_bool).unwrap_or(true),
        "{}",
        value["error"].as_str().unwrap_or("手机推送操作未完成")
    );
    if let Some(obj) = value.as_object_mut() {
        obj.entry("transport_mode".to_string())
            .or_insert_with(|| Value::String("content_provider".to_string()));
    }
    Ok(value)
}

/// Ensures `PushNotificationService` is granted notification listener access on the phone.
pub fn ensure_notification_listener(adb: &Adb, serial: &str) -> Result<bool> {
    let mut enabled = adb
        .script(
            serial,
            &format!(
                "cmd notification allow_listener {LISTENER_COMPONENT} >/dev/null 2>&1 || true; settings --user 0 get secure enabled_notification_listeners"
            ),
        )
        .unwrap_or_default();
    let mut listeners: Vec<&str> = enabled
        .trim()
        .split(':')
        .filter(|s| !s.is_empty() && *s != "null")
        .collect();
    if !listeners.contains(&LISTENER_COMPONENT) {
        listeners.push(LISTENER_COMPONENT);
        let joined = listeners.join(":");
        if adb
            .shell(
                serial,
                &[
                    "settings",
                    "--user",
                    "0",
                    "put",
                    "secure",
                    "enabled_notification_listeners",
                    &joined,
                ],
            )
            .is_ok()
        {
            enabled = joined;
        }
    }

    if let Ok(status) = call_push_provider(adb, serial, json!({ "op": "status" })) {
        if let Some(connected) = status
            .get("listener_connected")
            .or_else(|| status.get("listenerConnected"))
            .and_then(Value::as_bool)
        {
            return Ok(connected);
        }
        return Ok(true);
    }

    Ok(enabled.contains(LISTENER_COMPONENT))
}

pub fn is_excluded_im_package(pkg: &str) -> bool {
    let trimmed = pkg.trim();
    matches!(
        trimmed,
        "com.android.shell"
            | "android"
            | "com.android.systemui"
            | "com.android.settings"
            | "com.android.vending"
            | "com.android.launcher3"
            | "com.stepos.ailauncher"
            | "com.stepx.companion"
            | "com.stepx.companion.app"
            | "com.jingdong.app.mall"
            | "com.taobao.taobao"
            | "com.xunmeng.pinduoduo"
            | "com.sankuai.meituan"
    ) || trimmed.to_ascii_lowercase().contains("launcher")
}

fn is_non_im_system_package(package_name: &str) -> bool {
    is_excluded_im_package(package_name)
}

pub fn should_hide_notification(n: &PushNotification) -> bool {
    if !n.active || n.removed_time_ms > 0 {
        return true;
    }
    let pkg = n.package_name.trim();
    if pkg == "com.android.shell"
        || n.key.contains("stepx_e2e_test")
        || n.key.contains("stepx_test_tag")
    {
        return true;
    }
    let title = n.title.trim();
    let text = n.text.trim();
    let sub_text = n.sub_text.trim();
    if title.is_empty() && text.is_empty() && sub_text.is_empty() {
        return true;
    }
    if text.is_empty() && sub_text.is_empty() && pkg == "android" {
        return true;
    }
    if pkg == "android" || pkg == "com.android.systemui" {
        const LOW_VALUE_BANNERS: [&str; 6] = [
            "USB 文件传输",
            "正在通过 USB",
            "USB 调试",
            "无线调试",
            "选择输入法",
            "更改键盘",
        ];
        if LOW_VALUE_BANNERS
            .iter()
            .any(|kw| title.contains(kw) || text.contains(kw))
        {
            return true;
        }
    }
    false
}

fn enrich_snapshot(mut snapshot: PushSnapshot, default_transport: &str) -> PushSnapshot {
    if snapshot.transport_mode.is_empty() {
        snapshot.transport_mode = default_transport.to_string();
    }
    snapshot.threads.retain(|t| {
        !is_excluded_im_package(&t.package_name)
            && !(t.last_message.trim().is_empty()
                && t.messages.iter().all(|m| m.text.trim().is_empty()))
            && !t.thread_id.contains("stepx_e2e_test")
            && !t.thread_id.contains("stepx_test_tag")
    });
    snapshot
        .notifications
        .retain(|n| !should_hide_notification(n));
    snapshot
        .notifications
        .sort_by(|a, b| b.post_time_ms.cmp(&a.post_time_ms));
    let mut seen_notifs = std::collections::HashSet::new();
    snapshot.notifications.retain(|n| {
        seen_notifs.insert((
            n.package_name.clone(),
            n.title.trim().to_string(),
            n.text.trim().to_string(),
        ))
    });
    snapshot.otp_banners.retain(|b| {
        !b.notification_key.contains("stepx_e2e_test")
            && !b.notification_key.contains("stepx_test_tag")
    });
    for notif in &mut snapshot.notifications {
        if notif.otp_code.is_none() {
            let combined = format!("{} {}", notif.title, notif.text);
            notif.otp_code = extract_otp_code(&combined);
        }
        if notif.url_hint.is_none() {
            notif.url_hint =
                extract_first_url(&notif.text).or_else(|| extract_first_url(&notif.sub_text));
        }
    }
    if snapshot.otp_banners.is_empty() {
        for notif in &snapshot.notifications {
            if notif.dismissed {
                continue;
            }
            if let Some(code) = &notif.otp_code {
                snapshot.otp_banners.push(OtpBanner {
                    code: code.clone(),
                    source_app: if notif.app_name.is_empty() {
                        notif.package_name.clone()
                    } else {
                        notif.app_name.clone()
                    },
                    title: notif.title.clone(),
                    raw_text: notif.text.clone(),
                    timestamp_ms: notif.post_time_ms,
                    notification_key: notif.key.clone(),
                });
            }
        }
    }
    snapshot
}

pub fn fetch_snapshot(adb: &Adb, serial: &str, since_revision: u64) -> Result<PushSnapshot> {
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "snapshot",
            "since_revision": since_revision,
            "sinceRevision": since_revision,
        }),
    )?;
    let transport = value
        .get("transport_mode")
        .and_then(Value::as_str)
        .unwrap_or("adb");
    let target = value.get("snapshot").unwrap_or(&value);
    let mut snapshot: PushSnapshot =
        serde_json::from_value(target.clone()).context("无法解析手机推送快照数据")?;
    if value.get("unchanged").and_then(Value::as_bool) == Some(true)
        || target.get("unchanged").and_then(Value::as_bool) == Some(true)
    {
        snapshot.unchanged = true;
    }
    Ok(enrich_snapshot(snapshot, transport))
}

pub fn fetch_thread_detail(adb: &Adb, serial: &str, thread_id: &str) -> Result<ImThread> {
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "thread_detail",
            "thread_id": thread_id,
            "threadId": thread_id,
        }),
    )?;
    let target = value.get("thread").unwrap_or(&value);
    serde_json::from_value(target.clone()).context("无法解析会话详情")
}

#[allow(clippy::too_many_arguments)]
pub fn reply_im(
    adb: &Adb,
    serial: &str,
    thread_id: &str,
    package_name: &str,
    title: &str,
    text: &str,
    mode: ReplyMode,
    client_msg_id: &str,
) -> Result<ReplyReceipt> {
    let trimmed = text.trim();
    ensure!(!trimmed.is_empty(), "回复内容不能为空");
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "reply_im",
            "thread_id": thread_id,
            "threadId": thread_id,
            "package_name": package_name,
            "packageName": package_name,
            "title": title,
            "text": trimmed,
            "mode": mode.code(),
            "client_msg_id": client_msg_id,
            "clientMsgId": client_msg_id,
        }),
    )?;
    let target = value.get("receipt").unwrap_or(&value);
    let mut receipt: ReplyReceipt =
        serde_json::from_value(target.clone()).context("无法解析回复回执")?;
    ensure!(
        receipt.ok,
        "{}",
        if receipt.message.is_empty() {
            "手机未确认回复成功"
        } else {
            &receipt.message
        }
    );
    if receipt.thread_id.is_empty() {
        receipt.thread_id = thread_id.to_string();
    }
    if receipt.client_msg_id.is_empty() {
        receipt.client_msg_id = client_msg_id.to_string();
    }
    if receipt.via.is_empty() {
        receipt.via = mode.code().to_string();
    }
    Ok(receipt)
}

pub fn sync_active_chat_window(adb: &Adb, serial: &str) -> Result<Option<ImThread>> {
    let value = call_push_provider(adb, serial, json!({ "op": "sync_window" }))?;
    if let Some(thread_val) = value.get("thread") {
        if thread_val.is_null() {
            return Ok(None);
        }
        let thread: ImThread =
            serde_json::from_value(thread_val.clone()).context("无法解析当前聊天窗口记录")?;
        if (thread.thread_id.is_empty() && thread.title.is_empty())
            || is_non_im_system_package(&thread.package_name)
        {
            return Ok(None);
        }
        return Ok(Some(thread));
    }
    if value.get("thread_id").is_some() || value.get("threadId").is_some() {
        let thread: ImThread = serde_json::from_value(value).context("无法解析当前聊天窗口记录")?;
        if (!thread.thread_id.is_empty() || !thread.title.is_empty())
            && !is_non_im_system_package(&thread.package_name)
        {
            return Ok(Some(thread));
        }
    }
    Ok(None)
}

pub fn dismiss_notification(adb: &Adb, serial: &str, key: &str) -> Result<()> {
    ensure!(!key.trim().is_empty(), "通知标识不能为空");
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "dismiss_push",
            "key": key,
        }),
    )?;
    Ok(())
}

pub fn dismiss_all_notifications(adb: &Adb, serial: &str, category: Option<&str>) -> Result<usize> {
    let value = call_push_provider(
        adb,
        serial,
        json!({
            "op": "dismiss_all_push",
            "category": category.unwrap_or("all"),
        }),
    )?;
    let count = value
        .get("cleared")
        .or_else(|| value.get("count"))
        .or_else(|| value.get("dismissed"))
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    Ok(count)
}

pub fn open_notification_on_phone(adb: &Adb, serial: &str, key: &str) -> Result<()> {
    ensure!(!key.trim().is_empty(), "通知标识不能为空");
    let _ = call_push_provider(
        adb,
        serial,
        json!({
            "op": "open_push",
            "key": key,
        }),
    )?;
    Ok(())
}

/// Extracts a 4 to 8 digit verification code (OTP) when verification keywords are present.
pub fn extract_otp_code(text: &str) -> Option<String> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }
    let lower = trimmed.to_ascii_lowercase();
    let keywords = [
        "验证码",
        "校验码",
        "动态码",
        "确认码",
        "安全码",
        "授权码",
        "认证码",
        "登录码",
        "提取码",
        "verification",
        "passcode",
        "security code",
        "auth code",
        "one-time",
        "otp",
        "code is",
        "code:",
    ];

    let mut keyword_end_byte: Option<usize> = None;
    for kw in keywords {
        let mut offset = 0usize;
        while let Some(rel) = lower[offset..].find(kw) {
            let idx = offset + rel;
            let end = idx + kw.len();
            let boundary_ok = if kw.is_ascii() {
                let prev_ok = lower[..idx]
                    .chars()
                    .next_back()
                    .is_none_or(|c| !c.is_ascii_alphabetic() && c != '_');
                let next_ok = !kw.ends_with(|c: char| c.is_ascii_alphabetic())
                    || lower[end..]
                        .chars()
                        .next()
                        .is_none_or(|c| !c.is_ascii_alphabetic() && c != '_');
                prev_ok && next_ok
            } else {
                true
            };
            if boundary_ok {
                keyword_end_byte = Some(match keyword_end_byte {
                    Some(prev) => prev.min(end),
                    None => end,
                });
                break;
            }
            offset = end;
        }
    }
    let kw_end = keyword_end_byte?;

    let candidates = collect_digit_candidates(trimmed);
    if candidates.is_empty() {
        return None;
    }

    // Prefer the first valid candidate appearing after the keyword, otherwise the closest one before it.
    if let Some((_, code)) = candidates.iter().find(|(start, _)| *start >= kw_end) {
        return Some(code.clone());
    }
    candidates.last().map(|(_, code)| code.clone())
}

fn collect_digit_candidates(text: &str) -> Vec<(usize, String)> {
    let bytes = text.as_bytes();
    let mut results = Vec::new();
    let mut i = 0;

    while i < bytes.len() {
        if bytes[i].is_ascii_digit() {
            let start = i;
            while i < bytes.len() && bytes[i].is_ascii_digit() {
                i += 1;
            }
            let len = i - start;
            if (4..=8).contains(&len) {
                let prev_char = text[..start].chars().next_back();
                let next_char = text[i..].chars().next();
                let after_str = text[i..].trim_start();

                let prev_is_decimal = prev_char == Some('.')
                    && text[..start.saturating_sub(1)]
                        .chars()
                        .next_back()
                        .is_some_and(|c| c.is_ascii_digit());
                let next_is_decimal = next_char == Some('.')
                    && text[i + 1..]
                        .chars()
                        .next()
                        .is_some_and(|c| c.is_ascii_digit());
                let boundary_invalid = matches!(prev_char, Some('-' | '/' | ':'))
                    || matches!(next_char, Some('-' | '/' | ':'))
                    || prev_is_decimal
                    || next_is_decimal;
                let unit_invalid = [
                    "元", "分钟", "秒", "小时", "年", "月", "日", "条", "人", "个", "%", "MB", "GB",
                ]
                .iter()
                .any(|unit| after_str.starts_with(unit));
                let tail_number_context = text[..start].trim_end().ends_with("尾号")
                    || text[..start].trim_end().ends_with("卡号")
                    || text[..start].trim_end().ends_with("账户");

                if !boundary_invalid && !unit_invalid && !tail_number_context {
                    results.push((start, text[start..i].to_string()));
                }
            }
        } else {
            i += 1;
        }
    }
    results
}

/// Extracts the first `https://` or `http://` URL from notification or chat text.
pub fn extract_first_url(text: &str) -> Option<String> {
    for scheme in ["https://", "http://"] {
        if let Some(start) = text.find(scheme) {
            let rest = &text[start..];
            let end = rest
                .find(|c: char| {
                    c.is_whitespace()
                        || matches!(
                            c,
                            ')' | ']'
                                | '>'
                                | '"'
                                | '\''
                                | '，'
                                | '。'
                                | '；'
                                | '！'
                                | '？'
                                | '、'
                                | '”'
                                | '’'
                                | '】'
                                | '》'
                                | '（'
                                | '）'
                        )
                })
                .unwrap_or(rest.len());
            let candidate = rest[..end].trim_end_matches(['.', ',', ';', ':', '!']);
            if candidate.len() > scheme.len() + 3 && candidate[scheme.len()..].contains('.') {
                return Some(candidate.to_string());
            }
        }
    }
    None
}

/// Generates a clean, structured summary of unread IM conversations, OTP codes, and important
/// notifications for AI 简洁模式.
pub fn summarize_unread_for_ai(snapshot: &PushSnapshot) -> String {
    let unread_threads: Vec<&ImThread> = snapshot
        .threads
        .iter()
        .filter(|t| t.unread_count > 0)
        .collect();
    let total_unread_msgs: usize = unread_threads.iter().map(|t| t.unread_count).sum();

    let active_important_notifs: Vec<&PushNotification> = snapshot
        .notifications
        .iter()
        .filter(|n| !n.dismissed && !n.is_ongoing && n.category != "im")
        .collect();

    if unread_threads.is_empty()
        && snapshot.otp_banners.is_empty()
        && active_important_notifs.is_empty()
    {
        return "目前手机没有未读聊天消息或待处理的验证码/重要推送。".to_string();
    }

    let mut lines = Vec::new();
    lines.push(format!(
        "手机实时消息汇总（共 {} 个未读会话 / {} 条未读消息，{} 条验证码/取件码）：",
        unread_threads.len(),
        total_unread_msgs,
        snapshot.otp_banners.len()
    ));

    if !unread_threads.is_empty() {
        lines.push("\n【未读聊天会话】".to_string());
        for (idx, thread) in unread_threads.iter().take(6).enumerate() {
            let ch = thread.channel().label();
            let reply_tag = if thread.can_remote_reply {
                "可秒回"
            } else if thread.can_ui_reply {
                "可看屏回"
            } else {
                "只读"
            };
            let preview = if !thread.last_sender.is_empty()
                && thread.last_sender != thread.title
                && !thread.last_message.starts_with(&thread.last_sender)
            {
                format!("{}: {}", thread.last_sender, thread.last_message)
            } else {
                thread.last_message.clone()
            };
            lines.push(format!(
                "{}. [{}] {}（{}条未读 · {}）：{}",
                idx + 1,
                ch,
                thread.title,
                thread.unread_count,
                reply_tag,
                preview
            ));
        }
    }

    if !snapshot.otp_banners.is_empty() {
        lines.push("\n【最新验证码与取件码】".to_string());
        for banner in snapshot.otp_banners.iter().take(3) {
            lines.push(format!(
                "- [验证码] `{}`（来自 {} · {}）",
                banner.code, banner.source_app, banner.title
            ));
        }
    }

    if !active_important_notifs.is_empty() {
        lines.push("\n【重要应用推送】".to_string());
        for notif in active_important_notifs.iter().take(4) {
            let cat = PushCategory::from_str_code(&notif.category).label();
            let app = if notif.app_name.is_empty() {
                &notif.package_name
            } else {
                &notif.app_name
            };
            lines.push(format!(
                "- [{} · {}] {}：{}",
                app, cat, notif.title, notif.text
            ));
        }
    }

    lines.join("\n")
}

/// Generates a contextual quick reply draft based on the latest incoming message in `thread`.
pub fn generate_smart_reply_draft(thread: &ImThread) -> String {
    let latest_incoming = thread
        .messages
        .iter()
        .rev()
        .find(|m| !m.outgoing && !m.text.trim().is_empty())
        .map(|m| m.text.as_str())
        .unwrap_or(thread.last_message.as_str());

    let text = latest_incoming.trim();
    if text.is_empty() {
        return "收到，稍后联系。".to_string();
    }

    let lower = text.to_ascii_lowercase();
    if text.contains("发版")
        || text.contains("上线")
        || text.contains("部署")
        || text.contains("验收")
    {
        "收到，我这边已经验收就绪，可以按计划推进发版。".to_string()
    } else if lower.contains("pr")
        || text.contains("联调")
        || text.contains("接口")
        || text.contains("代码")
        || text.contains("文档")
    {
        "收到，我马上看下详细内容，确认后第一时间回复你。".to_string()
    } else if text.contains("会议")
        || text.contains("评审")
        || text.contains("开会")
        || text.contains("会议室")
    {
        "好的，已记下时间和地点，我准时参加。".to_string()
    } else if text.contains("告警")
        || text.contains("故障")
        || text.contains("异常")
        || text.contains("延迟")
        || text.contains("报错")
    {
        "收到告警，我立即排查一下当前节点指标，稍后同步进展。".to_string()
    } else if text.contains("试用")
        || text.contains("客户")
        || text.contains("方案")
        || text.contains("报价")
        || text.contains("合同")
    {
        "好的王总，我今天整理好最新安装包与使用说明发给您体验。".to_string()
    } else if text.contains('?')
        || text.contains('？')
        || text.contains("吗")
        || text.contains("哪些")
    {
        "收到，目前进展顺利，稍后把详细说明发给你。".to_string()
    } else {
        "收到，好的，我马上处理。".to_string()
    }
}

/// Rich, realistic Chinese demo data for `--capture-ui`, previews, and offline testing.
pub fn demo_push_snapshot() -> PushSnapshot {
    let now_ms: i64 = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(1_761_238_800_000);

    let threads = vec![
        ImThread {
            thread_id: "com.tencent.mm::产品研发群".to_string(),
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            title: "产品研发群".to_string(),
            is_group: true,
            unread_count: 3,
            last_sender: "王敏".to_string(),
            last_message: "@全体 今晚 8 点发版前大家把推送中心与小窗秒回验收一下，有问题吗？"
                .to_string(),
            last_timestamp_ms: now_ms - 45_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.tencent.mm|4097|prod_group|10231".to_string(),
            messages: vec![
                ImMessage {
                    id: "msg_wx_1".to_string(),
                    sender: "陈亮".to_string(),
                    text: "同 Wi-Fi LocalSocket 长连接实测往返延迟只有 9ms，熄屏锁屏下很稳。"
                        .to_string(),
                    timestamp_ms: now_ms - 240_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
                ImMessage {
                    id: "msg_wx_2".to_string(),
                    sender: "我".to_string(),
                    text: "桌面三端原生通知和置顶悬浮小窗也已经联调通过。".to_string(),
                    timestamp_ms: now_ms - 180_000,
                    outgoing: true,
                    delivery_status: "sent".to_string(),
                    via: "remote_input".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
                ImMessage {
                    id: "msg_wx_3".to_string(),
                    sender: "林工".to_string(),
                    text: "安卓端 RemoteInput 句柄缓存池已加上，支持连续追发多条回复。".to_string(),
                    timestamp_ms: now_ms - 95_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
                ImMessage {
                    id: "msg_wx_4".to_string(),
                    sender: "王敏".to_string(),
                    text: "@全体 今晚 8 点发版前大家把推送中心与小窗秒回验收一下，有问题吗？"
                        .to_string(),
                    timestamp_ms: now_ms - 45_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
            ],
        },
        ImThread {
            thread_id: "com.tencent.mm::张三".to_string(),
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            title: "张三".to_string(),
            is_group: false,
            unread_count: 1,
            last_sender: "张三".to_string(),
            last_message: "下午 3 点的架构评审会改到 B302 会议室了，记得带测试机过来".to_string(),
            last_timestamp_ms: now_ms - 120_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.tencent.mm|4098|zhangsan|10231".to_string(),
            messages: vec![
                ImMessage {
                    id: "msg_zs_1".to_string(),
                    sender: "张三".to_string(),
                    text: "无线调试自愈重连那块今天下午一起过一下？".to_string(),
                    timestamp_ms: now_ms - 600_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
                ImMessage {
                    id: "msg_zs_2".to_string(),
                    sender: "我".to_string(),
                    text: "好，文档我已经整理好了。".to_string(),
                    timestamp_ms: now_ms - 520_000,
                    outgoing: true,
                    delivery_status: "sent".to_string(),
                    via: "remote_input".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
                ImMessage {
                    id: "msg_zs_3".to_string(),
                    sender: "张三".to_string(),
                    text: "下午 3 点的架构评审会改到 B302 会议室了，记得带测试机过来".to_string(),
                    timestamp_ms: now_ms - 120_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
            ],
        },
        ImThread {
            thread_id: "com.ss.android.lark::李四".to_string(),
            package_name: "com.ss.android.lark".to_string(),
            app_name: "飞书".to_string(),
            title: "李四".to_string(),
            is_group: false,
            unread_count: 2,
            last_sender: "李四".to_string(),
            last_message:
                "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42"
                    .to_string(),
            last_timestamp_ms: now_ms - 195_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.ss.android.lark|2001|lisi|10245".to_string(),
            messages: vec![
                ImMessage {
                    id: "msg_ls_1".to_string(),
                    sender: "李四".to_string(),
                    text: "我把全量推送的 6 大智能分类和 OTP 提取器合进主分支了".to_string(),
                    timestamp_ms: now_ms - 260_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
                ImMessage {
                    id: "msg_ls_2".to_string(),
                    sender: "李四".to_string(),
                    text:
                        "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42"
                            .to_string(),
                    timestamp_ms: now_ms - 195_000,
                    outgoing: false,
                    delivery_status: "received".to_string(),
                    via: "notification".to_string(),
                    media_hint: String::new(),
                    client_msg_id: String::new(),
                },
            ],
        },
        ImThread {
            thread_id: "com.alibaba.android.rimet::运维告警群".to_string(),
            package_name: "com.alibaba.android.rimet".to_string(),
            app_name: "钉钉".to_string(),
            title: "运维告警群".to_string(),
            is_group: true,
            unread_count: 1,
            last_sender: "告警机器人".to_string(),
            last_message:
                "[P2 紧急] 华东节点推送网关 P99 延迟升至 380ms，请值班同学确认".to_string(),
            last_timestamp_ms: now_ms - 310_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.alibaba.android.rimet|3011|ops_alert|10250".to_string(),
            messages: vec![ImMessage {
                id: "msg_dd_1".to_string(),
                sender: "告警机器人".to_string(),
                text: "[P2 紧急] 华东节点推送网关 P99 延迟升至 380ms，请值班同学确认".to_string(),
                timestamp_ms: now_ms - 310_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
            }],
        },
        ImThread {
            thread_id: "com.tencent.wework::客户王总".to_string(),
            package_name: "com.tencent.wework".to_string(),
            app_name: "企业微信".to_string(),
            title: "客户王总".to_string(),
            is_group: false,
            unread_count: 1,
            last_sender: "客户王总".to_string(),
            last_message: "这周能把新版桌面伴侣的试用包发我们团队体验一下吗？".to_string(),
            last_timestamp_ms: now_ms - 540_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.tencent.wework|5001|wangzong|10260".to_string(),
            messages: vec![ImMessage {
                id: "msg_wc_1".to_string(),
                sender: "客户王总".to_string(),
                text: "这周能把新版桌面伴侣的试用包发我们团队体验一下吗？".to_string(),
                timestamp_ms: now_ms - 540_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
            }],
        },
        ImThread {
            thread_id: "org.telegram.messenger::StepX OpenSource".to_string(),
            package_name: "org.telegram.messenger".to_string(),
            app_name: "Telegram".to_string(),
            title: "StepX OpenSource".to_string(),
            is_group: true,
            unread_count: 0,
            last_sender: "Alex".to_string(),
            last_message: "v0.2.3 cross-platform desktop notification center works great on Linux!"
                .to_string(),
            last_timestamp_ms: now_ms - 900_000,
            can_remote_reply: true,
            can_ui_reply: false,
            has_content_intent: true,
            notification_key: "0|org.telegram.messenger|6001|stepx_tg|10270".to_string(),
            messages: vec![ImMessage {
                id: "msg_tg_1".to_string(),
                sender: "Alex".to_string(),
                text: "v0.2.3 cross-platform desktop notification center works great on Linux!"
                    .to_string(),
                timestamp_ms: now_ms - 900_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
            }],
        },
        ImThread {
            thread_id: "com.xingin.xhs::数码测评君".to_string(),
            package_name: "com.xingin.xhs".to_string(),
            app_name: "小红书".to_string(),
            title: "数码测评君".to_string(),
            is_group: false,
            unread_count: 1,
            last_sender: "数码测评君".to_string(),
            last_message: "你好，看到你们做的同 Wi-Fi 锁屏秒回微信功能很酷，支持哪些安卓机型呀？"
                .to_string(),
            last_timestamp_ms: now_ms - 1_200_000,
            can_remote_reply: false,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.xingin.xhs|7001|xhs_dm|10280".to_string(),
            messages: vec![ImMessage {
                id: "msg_xhs_1".to_string(),
                sender: "数码测评君".to_string(),
                text: "你好，看到你们做的同 Wi-Fi 锁屏秒回微信功能很酷，支持哪些安卓机型呀？"
                    .to_string(),
                timestamp_ms: now_ms - 1_200_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "notification".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
            }],
        },
        ImThread {
            thread_id: "com.google.android.apps.messaging::1069800095555".to_string(),
            package_name: "com.google.android.apps.messaging".to_string(),
            app_name: "短信".to_string(),
            title: "1069800095555".to_string(),
            is_group: false,
            unread_count: 1,
            last_sender: "招商银行".to_string(),
            last_message: "【招商银行】验证码 839201，5分钟内有效，用于登录掌上生活，请勿泄露。"
                .to_string(),
            last_timestamp_ms: now_ms - 60_000,
            can_remote_reply: true,
            can_ui_reply: true,
            has_content_intent: true,
            notification_key: "0|com.google.android.apps.messaging|8001|cmb_otp|10090".to_string(),
            messages: vec![ImMessage {
                id: "msg_sms_1".to_string(),
                sender: "招商银行".to_string(),
                text: "【招商银行】验证码 839201，5分钟内有效，用于登录掌上生活，请勿泄露。"
                    .to_string(),
                timestamp_ms: now_ms - 60_000,
                outgoing: false,
                delivery_status: "received".to_string(),
                via: "sms".to_string(),
                media_hint: String::new(),
                    client_msg_id: String::new(),
            }],
        },
    ];

    let notifications = vec![
        PushNotification {
            key: "0|com.tencent.mm|4097|prod_group|10231".to_string(),
            seq_id: 108,
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            category: "im".to_string(),
            title: "产品研发群".to_string(),
            sender: "王敏".to_string(),
            text: "@全体 今晚 8 点发版前大家把推送中心与小窗秒回验收一下，有问题吗？".to_string(),
            sub_text: "3条未读消息".to_string(),
            post_time_ms: now_ms - 45_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: true,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.tencent.mm::产品研发群".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string(), "标为已读".to_string()],
            url_hint: None,
        },
        PushNotification {
            key: "0|com.google.android.apps.messaging|8001|cmb_otp|10090".to_string(),
            seq_id: 107,
            package_name: "com.google.android.apps.messaging".to_string(),
            app_name: "短信".to_string(),
            category: "system".to_string(),
            title: "招商银行 (1069800095555)".to_string(),
            sender: "1069800095555".to_string(),
            text: "【招商银行】验证码 839201，5分钟内有效，用于登录掌上生活，请勿泄露。"
                .to_string(),
            sub_text: "刚刚 · 短信验证码".to_string(),
            post_time_ms: now_ms - 60_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.google.android.apps.messaging::1069800095555".to_string(),
            otp_code: Some("839201".to_string()),
            action_labels: vec!["复制验证码".to_string(), "回复".to_string()],
            url_hint: None,
        },
        PushNotification {
            key: "0|com.tencent.mm|4098|zhangsan|10231".to_string(),
            seq_id: 106,
            package_name: "com.tencent.mm".to_string(),
            app_name: "微信".to_string(),
            category: "im".to_string(),
            title: "张三".to_string(),
            sender: "张三".to_string(),
            text: "下午 3 点的架构评审会改到 B302 会议室了，记得带测试机过来".to_string(),
            sub_text: String::new(),
            post_time_ms: now_ms - 120_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.tencent.mm::张三".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: None,
        },
        PushNotification {
            key: "0|com.ss.android.lark|2001|lisi|10245".to_string(),
            seq_id: 105,
            package_name: "com.ss.android.lark".to_string(),
            app_name: "飞书".to_string(),
            category: "im".to_string(),
            title: "李四".to_string(),
            sender: "李四".to_string(),
            text: "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42"
                .to_string(),
            sub_text: "研发协作".to_string(),
            post_time_ms: now_ms - 195_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.ss.android.lark::李四".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: Some("https://github.com/stepx/companion/pull/42".to_string()),
        },
        PushNotification {
            key: "0|com.alibaba.android.rimet|3011|ops_alert|10250".to_string(),
            seq_id: 104,
            package_name: "com.alibaba.android.rimet".to_string(),
            app_name: "钉钉".to_string(),
            category: "im".to_string(),
            title: "运维告警群".to_string(),
            sender: "告警机器人".to_string(),
            text: "[P2 紧急] 华东节点推送网关 P99 延迟升至 380ms，请值班同学确认".to_string(),
            sub_text: "生产环境监控".to_string(),
            post_time_ms: now_ms - 310_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: true,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.alibaba.android.rimet::运维告警群".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: None,
        },
        PushNotification {
            key: "0|com.cainiao.wireless|9001|pickup|10290".to_string(),
            seq_id: 103,
            package_name: "com.cainiao.wireless".to_string(),
            app_name: "菜鸟".to_string(),
            category: "delivery_travel".to_string(),
            title: "包裹到站待取提醒".to_string(),
            sender: "菜鸟驿站".to_string(),
            text: "您的顺丰包裹已到达博彦科技园菜鸟驿站，凭取件码 4-2-1088 及时领取。".to_string(),
            sub_text: "物流出行".to_string(),
            post_time_ms: now_ms - 420_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: String::new(),
            otp_code: Some("4-2-1088".to_string()),
            action_labels: vec!["查看取件码".to_string()],
            url_hint: None,
        },
        PushNotification {
            key: "0|com.tencent.wework|5001|wangzong|10260".to_string(),
            seq_id: 102,
            package_name: "com.tencent.wework".to_string(),
            app_name: "企业微信".to_string(),
            category: "im".to_string(),
            title: "客户王总".to_string(),
            sender: "客户王总".to_string(),
            text: "这周能把新版桌面伴侣的试用包发我们团队体验一下吗？".to_string(),
            sub_text: "外部联系人".to_string(),
            post_time_ms: now_ms - 540_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: true,
            thread_id: "com.tencent.wework::客户王总".to_string(),
            otp_code: None,
            action_labels: vec!["回复".to_string()],
            url_hint: None,
        },
        PushNotification {
            key: "0|cmb.pb|9201|bill|10300".to_string(),
            seq_id: 101,
            package_name: "cmb.pb".to_string(),
            app_name: "招商银行".to_string(),
            category: "finance".to_string(),
            title: "信用卡消费提醒".to_string(),
            sender: "招商银行".to_string(),
            text: "您尾号 6219 的招行信用卡于 12:35 在【瑞幸咖啡】消费人民币 16.50 元。"
                .to_string(),
            sub_text: "交易与账单".to_string(),
            post_time_ms: now_ms - 780_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: String::new(),
            otp_code: None,
            action_labels: vec!["查看账单".to_string()],
            url_hint: None,
        },
        PushNotification {
            key: "0|com.xingin.xhs|7001|xhs_dm|10280".to_string(),
            seq_id: 100,
            package_name: "com.xingin.xhs".to_string(),
            app_name: "小红书".to_string(),
            category: "social".to_string(),
            title: "数码测评君 私信了你".to_string(),
            sender: "数码测评君".to_string(),
            text: "你好，看到你们做的同 Wi-Fi 锁屏秒回微信功能很酷，支持哪些安卓机型呀？"
                .to_string(),
            sub_text: "社交互动".to_string(),
            post_time_ms: now_ms - 1_200_000,
            is_ongoing: false,
            is_clearable: true,
            active: true,
            removed_time_ms: 0,
            is_group: false,
            dismissed: false,
            read: false,
            can_reply: false,
            thread_id: "com.xingin.xhs::数码测评君".to_string(),
            otp_code: None,
            action_labels: vec!["打开详情".to_string()],
            url_hint: None,
        },
    ];

    let otp_banners = vec![
        OtpBanner {
            code: "839201".to_string(),
            source_app: "招商银行 · 短信".to_string(),
            title: "掌上生活登录验证码".to_string(),
            raw_text: "【招商银行】验证码 839201，5分钟内有效，用于登录掌上生活，请勿泄露。"
                .to_string(),
            timestamp_ms: now_ms - 60_000,
            notification_key: "0|com.google.android.apps.messaging|8001|cmb_otp|10090".to_string(),
        },
        OtpBanner {
            code: "4-2-1088".to_string(),
            source_app: "菜鸟驿站".to_string(),
            title: "博彦科技园驿站取件码".to_string(),
            raw_text: "您的顺丰包裹已到达博彦科技园菜鸟驿站，凭取件码 4-2-1088 及时领取。"
                .to_string(),
            timestamp_ms: now_ms - 420_000,
            notification_key: "0|com.cainiao.wireless|9001|pickup|10290".to_string(),
        },
    ];

    PushSnapshot {
        revision: 42,
        unchanged: false,
        listener_connected: true,
        accessibility_connected: true,
        wifi_adb_enabled: true,
        screen_interactive: false,
        keyguard_locked: true,
        human_touch_recent: false,
        transport_mode: "同 Wi-Fi 极速流 (LocalSocket < 15ms)".to_string(),
        threads,
        notifications,
        otp_banners,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extracts_chinese_and_english_otp_codes_accurately() {
        assert_eq!(
            extract_otp_code("【招商银行】验证码 839201，5分钟内有效，请勿泄露。"),
            Some("839201".to_string())
        );
        assert_eq!(
            extract_otp_code(
                "【招商银行】您尾号 6219 的储蓄卡正在登录，动态码：490218，请在 3 分钟内输入。"
            ),
            Some("490218".to_string())
        );
        assert_eq!(
            extract_otp_code("739104 is your GitHub verification code."),
            Some("739104".to_string())
        );
        assert_eq!(
            extract_otp_code("Your StepX OTP: 9812. Do not share it."),
            Some("9812".to_string())
        );
        // Normal bill or chat messages without OTP keywords must not false-trigger.
        assert_eq!(
            extract_otp_code("您尾号 6219 的信用卡消费人民币 1650 元"),
            None
        );
        assert_eq!(
            extract_otp_code("今晚 2026 年发版会议改到 14:30 开始"),
            None
        );
    }

    #[test]
    fn extracts_first_url_without_trailing_punctuation() {
        assert_eq!(
            extract_first_url(
                "接口联调通过了，辛苦看下 PR #42：https://github.com/stepx/companion/pull/42，谢谢！"
            ),
            Some("https://github.com/stepx/companion/pull/42".to_string())
        );
        assert_eq!(
            extract_first_url("点击链接入会 (https://meeting.feishu.cn/j/938271625)."),
            Some("https://meeting.feishu.cn/j/938271625".to_string())
        );
        assert_eq!(extract_first_url("普通聊天文本没有网址"), None);
    }

    #[test]
    fn classifies_im_channels_and_categories() {
        assert_eq!(ImChannel::from_package("com.tencent.mm"), ImChannel::WeChat);
        assert_eq!(
            ImChannel::from_package("com.tencent.wework"),
            ImChannel::WeCom
        );
        assert_eq!(
            ImChannel::from_package("com.alibaba.android.rimet"),
            ImChannel::DingTalk
        );
        assert_eq!(
            ImChannel::from_package("com.ss.android.lark"),
            ImChannel::Feishu
        );
        assert_eq!(
            ImChannel::from_package("com.tencent.mobileqq"),
            ImChannel::QqTim
        );
        assert_eq!(
            ImChannel::from_package("org.telegram.messenger"),
            ImChannel::GlobalIm
        );
        assert_eq!(
            ImChannel::from_package("com.xingin.xhs"),
            ImChannel::SocialDm
        );
        assert_eq!(
            ImChannel::from_package("com.google.android.apps.messaging"),
            ImChannel::Sms
        );

        assert!(ImChannel::All.matches_package("com.tencent.mm"));
        assert!(ImChannel::WeChat.matches_package("com.tencent.mm"));
        assert!(!ImChannel::WeChat.matches_package("com.ss.android.lark"));

        assert_eq!(PushCategory::from_str_code("im"), PushCategory::Im);
        assert_eq!(
            PushCategory::from_str_code("finance"),
            PushCategory::Finance
        );
        assert_eq!(
            PushCategory::from_str_code("delivery_travel"),
            PushCategory::DeliveryTravel
        );
        assert_eq!(
            PushCategory::from_str_code("system"),
            PushCategory::SystemSecurity
        );
        assert!(PushCategory::All.matches("finance"));
        assert!(PushCategory::Finance.matches("finance"));
        assert!(!PushCategory::Im.matches("finance"));
    }

    #[test]
    fn parses_camel_case_and_snake_case_push_snapshot_json() {
        let raw_json = json!({
            "revision": 99,
            "listenerConnected": true,
            "accessibilityConnected": false,
            "wifiAdbEnabled": true,
            "screenInteractive": false,
            "keyguardLocked": true,
            "humanTouchRecent": false,
            "transportMode": "local_socket",
            "threads": [{
                "threadId": "com.tencent.mm::测试群",
                "packageName": "com.tencent.mm",
                "appName": "微信",
                "title": "测试群",
                "isGroup": true,
                "unreadCount": 2,
                "lastSender": "小王",
                "lastMessage": "今晚发版准备好了吗？",
                "lastTimestampMs": 1700000000000i64,
                "canRemoteReply": true,
                "canUiReply": false,
                "hasContentIntent": true,
                "notificationKey": "k1",
                "messages": [{
                    "id": "m1",
                    "sender": "小王",
                    "text": "今晚发版准备好了吗？",
                    "timestampMs": 1700000000000i64,
                    "outgoing": false,
                    "deliveryStatus": "received",
                    "via": "notification"
                }]
            }],
            "notifications": [{
                "key": "k_otp",
                "seqId": 12,
                "packageName": "com.android.mms",
                "appName": "短信",
                "category": "system",
                "title": "招商银行",
                "sender": "95555",
                "text": "【招商银行】您的验证码 654321，请勿泄露。详情 https://cmbchina.com/info",
                "postTimeMs": 1700000001000i64,
                "isOngoing": false,
                "isClearable": true,
                "canReply": false
            }]
        });

        let parsed: PushSnapshot = serde_json::from_value(raw_json).expect("valid snapshot JSON");
        let enriched = enrich_snapshot(parsed, "local_socket");
        assert_eq!(enriched.revision, 99);
        assert!(enriched.listener_connected);
        assert_eq!(enriched.threads.len(), 1);
        assert_eq!(enriched.threads[0].title, "测试群");
        assert!(enriched.threads[0].can_remote_reply);
        assert_eq!(
            enriched.notifications[0].otp_code,
            Some("654321".to_string())
        );
        assert_eq!(
            enriched.notifications[0].url_hint,
            Some("https://cmbchina.com/info".to_string())
        );
        assert_eq!(enriched.otp_banners.len(), 1);
        assert_eq!(enriched.otp_banners[0].code, "654321");
    }

    #[test]
    fn generates_ai_summary_and_smart_reply_draft() {
        let demo = demo_push_snapshot();
        let summary = summarize_unread_for_ai(&demo);
        assert!(summary.contains("产品研发群"));
        assert!(summary.contains("张三"));
        assert!(summary.contains("839201"));
        assert!(summary.contains("4-2-1088"));

        let draft_release = generate_smart_reply_draft(&demo.threads[0]);
        assert!(draft_release.contains("发版") || draft_release.contains("验收"));

        let draft_pr = generate_smart_reply_draft(&demo.threads[2]);
        assert!(draft_pr.contains("看下") || draft_pr.contains("回复"));
    }

    #[test]
    fn filters_and_deduplicates_notifications_and_non_im_threads() {
        let raw_json = json!({
            "revision": 100,
            "listenerConnected": true,
            "threads": [
                {
                    "threadId": "com.jingdong.app.mall::促销",
                    "packageName": "com.jingdong.app.mall",
                    "title": "京东促销",
                    "lastMessage": "限时满减"
                },
                {
                    "threadId": "com.tencent.mm::empty",
                    "packageName": "com.tencent.mm",
                    "title": "空会话",
                    "lastMessage": "   ",
                    "messages": []
                },
                {
                    "threadId": "com.tencent.mm::stepx_e2e_test",
                    "packageName": "com.tencent.mm",
                    "title": "E2E",
                    "lastMessage": "test"
                },
                {
                    "threadId": "com.tencent.mm::real",
                    "packageName": "com.tencent.mm",
                    "title": "真实会话",
                    "lastMessage": "你好"
                }
            ],
            "notifications": [
                {
                    "key": "0|com.stepx.companion.app|1|a",
                    "packageName": "com.stepx.companion.app",
                    "title": "StepX 伴侣权限已就绪",
                    "text": "通知监听与无障碍已开启",
                    "postTimeMs": 1000,
                    "clearable": false,
                    "active": true
                },
                {
                    "key": "0|com.stepx.companion.app|2|b",
                    "packageName": "com.stepx.companion.app",
                    "title": "StepX 伴侣权限已就绪",
                    "text": "通知监听与无障碍已开启",
                    "postTimeMs": 3000,
                    "clearable": false,
                    "active": true
                },
                {
                    "key": "0|com.android.shell|99|stepx_e2e_test",
                    "packageName": "com.android.shell",
                    "title": "测试通知",
                    "text": "验证码 123456",
                    "postTimeMs": 4000,
                    "clearable": true,
                    "active": true
                },
                {
                    "key": "0|android|10|usb",
                    "packageName": "android",
                    "title": "USB 调试已连接",
                    "text": "轻触可停用 USB 调试",
                    "postTimeMs": 5000,
                    "clearable": false,
                    "active": true
                },
                {
                    "key": "0|com.tencent.mm|20|removed",
                    "packageName": "com.tencent.mm",
                    "title": "已移除",
                    "text": "旧消息",
                    "postTimeMs": 6000,
                    "clearable": true,
                    "active": false,
                    "removedTimeMs": 6001
                }
            ]
        });

        let parsed: PushSnapshot =
            serde_json::from_value(raw_json).expect("clearable + active must not collide");
        let enriched = enrich_snapshot(parsed, "local_socket");
        assert_eq!(enriched.threads.len(), 1);
        assert_eq!(enriched.threads[0].thread_id, "com.tencent.mm::real");
        assert_eq!(enriched.notifications.len(), 1);
        assert_eq!(enriched.notifications[0].post_time_ms, 3000);
        assert_eq!(enriched.notifications[0].title, "StepX 伴侣权限已就绪");
    }
}

#[cfg(test)]
mod reply_transport_tests {
    use super::*;
    #[test]
    fn sending_and_unknown_operations_never_retry_over_another_transport() {
        for op in [
            "reply_im",
            "open_notification",
            "dismiss_all",
            "future_write",
        ] {
            assert!(!replay_safe(&json!({"op":op})));
        }
        for op in ["snapshot", "status", "ping", "thread_detail", "sync_window"] {
            assert!(replay_safe(&json!({"op":op})));
        }
    }
}

#[cfg(test)]
mod reply_merge_tests {
    use super::*;
    #[test]
    fn phone_id_and_client_id_correlate_without_collapsing_repeated_text() {
        let local = ImMessage {
            id: "stepx-a".into(),
            client_msg_id: "stepx-a".into(),
            outgoing: true,
            text: "same body".into(),
            delivery_status: "sending".into(),
            ..Default::default()
        };
        let remote: ImMessage = serde_json::from_value(
            json!({"id":"phone-a","clientMsgId":"stepx-a","outgoing":true,"text":"same body"}),
        )
        .unwrap();
        let old = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "thread".into(),
                messages: vec![local.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        let mut next = PushSnapshot {
            threads: vec![ImThread {
                thread_id: "thread".into(),
                messages: vec![remote.clone()],
                ..Default::default()
            }],
            ..Default::default()
        };
        merge_local_replies(&mut next, &old);
        assert_eq!(next.threads[0].messages.len(), 1);
        let mut distinct = local.clone();
        distinct.id = "stepx-b".into();
        distinct.client_msg_id = "stepx-b".into();
        assert!(!distinct.same_delivery(&remote));
        next.threads[0].messages.clear();
        merge_local_replies(&mut next, &old);
        assert_eq!(next.threads[0].messages, vec![local]);
        let mut empty = PushSnapshot::default();
        merge_local_replies(&mut empty, &old);
        assert_eq!(empty.threads.len(), 1);
    }

    #[test]
    fn unchanged_delta_snapshot_preserves_existing_threads_and_notifications() {
        let prev = demo_push_snapshot();
        let mut delta = PushSnapshot {
            revision: prev.revision,
            unchanged: true,
            listener_connected: true,
            screen_interactive: true,
            transport_mode: "local_socket".into(),
            ..Default::default()
        };
        merge_local_replies(&mut delta, &prev);
        assert!(!delta.unchanged);
        assert_eq!(delta.revision, prev.revision);
        assert_eq!(delta.threads, prev.threads);
        assert_eq!(delta.notifications, prev.notifications);
        assert_eq!(delta.otp_banners, prev.otp_banners);
        assert!(delta.screen_interactive);
    }

    #[test]
    fn socket_miss_cache_blocks_repeated_forward_within_ttl() {
        let serial = "TEST-SOCKET-MISS-SERIAL";
        let now = Instant::now();
        invalidate_cached_port(serial);
        assert!(!is_socket_recently_unavailable(serial, now));
        mark_socket_unavailable(serial);
        assert!(is_socket_recently_unavailable(
            serial,
            now + Duration::from_secs(5)
        ));
        assert!(!is_socket_recently_unavailable(
            serial,
            now + SOCKET_MISS_TTL + Duration::from_secs(1)
        ));
        invalidate_cached_port(serial);
    }
}
