use crate::{
    adb::{shell_quote, Adb},
    device::Device,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::Write,
    path::Path,
    sync::{Mutex, OnceLock},
    time::Duration,
};

const BRIDGE_JAR: &[u8] = include_bytes!("../assets/stepx-bridge.jar");
const REMOTE_DIR: &str = "/data/local/tmp/stepx-companion";
const REMOTE_JAR: &str = "/data/local/tmp/stepx-companion/stepx-bridge.jar";

static VERIFIED_BRIDGE_CACHE: OnceLock<Mutex<HashSet<(String, usize)>>> = OnceLock::new();

fn bridge_cache() -> &'static Mutex<HashSet<(String, usize)>> {
    VERIFIED_BRIDGE_CACHE.get_or_init(|| Mutex::new(HashSet::new()))
}

pub fn is_bridge_jar_cached(transport: &str) -> bool {
    let key = transport.trim();
    if key.is_empty() {
        return false;
    }
    bridge_cache()
        .lock()
        .map(|guard| guard.contains(&(key.to_string(), BRIDGE_JAR.len())))
        .unwrap_or(false)
}

pub fn mark_bridge_jar_cached(transport: &str) {
    let key = transport.trim();
    if key.is_empty() {
        return;
    }
    if let Ok(mut guard) = bridge_cache().lock() {
        guard.insert((key.to_string(), BRIDGE_JAR.len()));
    }
}

pub fn invalidate_bridge_cache(transport: &str) {
    let key = transport.trim();
    if let Ok(mut guard) = bridge_cache().lock() {
        guard.retain(|(t, _)| t != key);
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct SimSubscription {
    pub id: i32,
    pub slot: i32,
    pub label: String,
}

pub fn friendly_carrier_name(raw: &str) -> String {
    let trimmed = raw.trim();
    match trimmed.to_ascii_uppercase().as_str() {
        "CT" | "CHN-CT" | "CHINA TELECOM" | "CHN-TELECOM" => "中国电信".to_string(),
        "CMCC" | "CM" | "CHINA MOBILE" | "CHN-CMCC" => "中国移动".to_string(),
        "CUCC" | "CU" | "CHN-UNICOM" | "CHINA UNICOM" | "CHN-CU" => "中国联通".to_string(),
        "CBN" | "CHINA BROADNET" | "CHN-CBN" => "中国广电".to_string(),
        _ => trimmed.to_string(),
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Capabilities {
    pub send_sms: bool,
    pub locked: bool,
    pub default_subscription: i32,
    pub subscriptions: Vec<SimSubscription>,
    pub protocol: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeNode {
    pub id: String,
    pub text: String,
    pub description: String,
    #[serde(rename = "class")]
    pub class_name: String,
    pub clickable: bool,
    pub enabled: bool,
    pub depth: u32,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmooSessionEntry {
    pub title: String,
    pub group: String,
    pub has_status_icon: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmooTurn {
    pub speaker: String,
    pub is_user: bool,
    pub text: String,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AmooSnapshot {
    pub turns: Vec<AmooTurn>,
    pub sessions: Vec<AmooSessionEntry>,
    pub selected_title: String,
    pub prepared: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsPlan {
    pub request_id: String,
    pub number: String,
    pub body: String,
    pub subscription: i32,
    pub subscription_label: String,
    pub parts: usize,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SmsSendStatus {
    pub id: String,
    pub state: String,
    pub parts: usize,
    pub completed: usize,
    pub error_code: Option<i32>,
    pub error: Option<String>,
    pub updated_ms: Option<i64>,
}

impl SmsSendStatus {
    pub fn summary_label(&self) -> String {
        match self.state.as_str() {
            "sent" => format!("已发送成功（{}/{} 段）", self.completed, self.parts.max(1)),
            "sending" => format!("正在发送中（{}/{} 段）…", self.completed, self.parts.max(1)),
            "failed" => {
                if let Some(code) = self.error_code {
                    format!(
                        "发送失败（错误码 {code}，已完成 {}/{} 段）",
                        self.completed,
                        self.parts.max(1)
                    )
                } else if let Some(err) = &self.error {
                    format!("发送失败：{err}")
                } else {
                    "短信发送失败".into()
                }
            }
            "unknown" => "发送状态未确认（为防重复扣费未自动重发，可点击查询状态）".into(),
            "not_found" => "未找到该发送请求记录".into(),
            other => format!("状态：{other}"),
        }
    }

    pub fn is_pending_or_unknown(&self) -> bool {
        matches!(self.state.as_str(), "sending" | "unknown")
    }
}

pub fn ensure_bridge_jar(adb: &Adb, device: &Device) -> Result<()> {
    let active_transport = device.resolve_active_transport(adb)?;
    if is_bridge_jar_cached(&active_transport) || is_bridge_jar_cached(&device.serial) {
        mark_bridge_jar_cached(&active_transport);
        mark_bridge_jar_cached(&device.serial);
        return Ok(());
    }
    ensure_bridge_jar_on_transport(adb, &active_transport)?;
    mark_bridge_jar_cached(&device.serial);
    Ok(())
}

fn ensure_bridge_jar_on_transport(adb: &Adb, transport: &str) -> Result<()> {
    if is_bridge_jar_cached(transport) {
        return Ok(());
    }
    let expected_len = BRIDGE_JAR.len();
    let check_cmd = format!(
        "mkdir -p {dir} && chmod 700 {dir} && [ -f {jar} ] && [ \"$(stat -c %s {jar} 2>/dev/null)\" = \"{expected_len}\" ] && printf READY",
        dir = shell_quote(REMOTE_DIR),
        jar = shell_quote(REMOTE_JAR),
    );
    if let Ok(out) = adb.script(transport, &check_cmd) {
        if out.trim() == "READY" {
            mark_bridge_jar_cached(transport);
            return Ok(());
        }
    }
    let mut temp = tempfile::NamedTempFile::new().context("无法创建临时桥接包")?;
    temp.write_all(BRIDGE_JAR)?;
    temp.as_file().sync_all()?;
    adb.transfer(
        transport,
        "push",
        temp.path(),
        Path::new(REMOTE_JAR),
    )
    .context("无法推送 Android Bridge 到手机")?;
    adb.script(
        transport,
        &format!("chmod 600 {}", shell_quote(REMOTE_JAR)),
    )?;
    mark_bridge_jar_cached(transport);
    Ok(())
}

fn run_bridge_once_on_transport(
    adb: &Adb,
    transport: &str,
    request: &Value,
    timeout: Duration,
) -> Result<Value> {
    let shell_cmd = format!(
        "CLASSPATH={} app_process {} com.stepx.companion.Bridge",
        shell_quote(REMOTE_JAR),
        shell_quote(REMOTE_DIR)
    );
    let payload = format!("{}\n", serde_json::to_string(request)?);
    let output = adb.run_output_with_input(
        &["-s", transport, "shell", &shell_cmd],
        Some(payload.into_bytes()),
        timeout,
    )?;
    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);
    parse_bridge_output(&stdout, &stderr)
}

fn should_retry_cached_bridge(err: &anyhow::Error) -> bool {
    let msg = format!("{err:#}");
    msg.contains("ClassNotFoundException")
        || msg.contains("NoSuchFileException")
        || msg.contains("aborted")
        || msg.contains("未返回有效结果")
}

pub fn clean_nullable_field(raw: &str) -> String {
    let trimmed = raw.trim();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed.eq_ignore_ascii_case("none")
        || trimmed.eq_ignore_ascii_case("undefined")
    {
        String::new()
    } else {
        trimmed.to_string()
    }
}

pub fn call_bridge_on_transport(
    adb: &Adb,
    transport: &str,
    request: &Value,
    timeout: Duration,
) -> Result<Value> {
    let was_cached = is_bridge_jar_cached(transport);
    ensure_bridge_jar_on_transport(adb, transport)?;
    match run_bridge_once_on_transport(adb, transport, request, timeout) {
        Ok(val) => Ok(val),
        Err(err) if was_cached && should_retry_cached_bridge(&err) => {
            invalidate_bridge_cache(transport);
            ensure_bridge_jar_on_transport(adb, transport)?;
            run_bridge_once_on_transport(adb, transport, request, timeout)
        }
        Err(err) => Err(err),
    }
}

pub fn call_bridge(
    adb: &Adb,
    device: &Device,
    request: &Value,
    timeout: Duration,
) -> Result<Value> {
    let active_transport = device.resolve_active_transport(adb)?;
    let was_cached =
        is_bridge_jar_cached(&active_transport) || is_bridge_jar_cached(&device.serial);
    if was_cached {
        mark_bridge_jar_cached(&active_transport);
        mark_bridge_jar_cached(&device.serial);
    } else {
        ensure_bridge_jar_on_transport(adb, &active_transport)?;
        mark_bridge_jar_cached(&device.serial);
    }
    match run_bridge_once_on_transport(adb, &active_transport, request, timeout) {
        Ok(val) => Ok(val),
        Err(err) if was_cached && should_retry_cached_bridge(&err) => {
            invalidate_bridge_cache(&active_transport);
            invalidate_bridge_cache(&device.serial);
            ensure_bridge_jar_on_transport(adb, &active_transport)?;
            mark_bridge_jar_cached(&device.serial);
            run_bridge_once_on_transport(adb, &active_transport, request, timeout)
        }
        Err(err) => Err(err),
    }
}

pub fn parse_bridge_output(stdout: &str, stderr: &str) -> Result<Value> {
    let line = stdout
        .lines()
        .rev()
        .find_map(|l| l.trim().strip_prefix("STEPX_RESULT:"))
        .with_context(|| {
            let detail = stderr.trim();
            if detail.is_empty() {
                "手机桥接服务未返回有效结果，请确认手机已解锁并允许调试".to_string()
            } else {
                format!("手机桥接服务执行异常：{detail}")
            }
        })?;
    let val: Value = serde_json::from_str(line).context("无法解析手机桥接返回的 JSON")?;
    if !val.get("ok").and_then(Value::as_bool).unwrap_or(false) {
        let err = val
            .get("error")
            .and_then(Value::as_str)
            .unwrap_or("手机端拒绝或无法完成该操作");
        bail!("{err}");
    }
    Ok(val)
}

pub fn capabilities(adb: &Adb, device: &Device) -> Result<Capabilities> {
    let val = call_bridge(
        adb,
        device,
        &json!({ "op": "capabilities" }),
        Duration::from_secs(10),
    )?;
    parse_capabilities(&val)
}

pub fn parse_capabilities(val: &Value) -> Result<Capabilities> {
    let send_sms = val
        .get("send_sms")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let locked = val.get("locked").and_then(Value::as_bool).unwrap_or(false);
    let default_subscription = val
        .get("default_subscription")
        .and_then(Value::as_i64)
        .unwrap_or(-1) as i32;
    let protocol = val.get("protocol").and_then(Value::as_u64).unwrap_or(1) as u32;
    let mut subscriptions = Vec::new();
    if let Some(arr) = val.get("subscriptions").and_then(Value::as_array) {
        for item in arr {
            let id = item.get("id").and_then(Value::as_i64).unwrap_or(-1) as i32;
            let slot = (item.get("slot").and_then(Value::as_i64).unwrap_or(1) as i32).max(1);
            let label =
                friendly_carrier_name(item.get("label").and_then(Value::as_str).unwrap_or("SIM"));
            if id >= 0 {
                subscriptions.push(SimSubscription { id, slot, label });
            }
        }
    }
    Ok(Capabilities {
        send_sms,
        locked,
        default_subscription,
        subscriptions,
        protocol,
    })
}

pub fn parse_amoo_snapshot(val: &Value) -> AmooSnapshot {
    let prepared = val
        .get("prepared")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let selected_title = val
        .get("selected_title")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    let mut sessions = Vec::new();
    if let Some(sess_arr) = val.get("sessions").and_then(Value::as_array) {
        for s in sess_arr {
            let title = s
                .get("title")
                .and_then(Value::as_str)
                .unwrap_or("")
                .trim()
                .to_string();
            if title.is_empty() {
                continue;
            }
            let group = s
                .get("group")
                .and_then(Value::as_str)
                .unwrap_or("今天")
                .trim()
                .to_string();
            let has_status_icon = s
                .get("has_status_icon")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            sessions.push(AmooSessionEntry {
                title,
                group,
                has_status_icon,
            });
        }
    }

    let mut turns = Vec::new();
    if let Some(rows) = val.get("rows").and_then(Value::as_array) {
        for row in rows {
            if let Some(nodes) = row.as_array() {
                let is_action_row = nodes.iter().any(|n| {
                    let id = n.get("id").and_then(Value::as_str).unwrap_or("");
                    id.ends_with("action_layout") || id.ends_with("iv_copy")
                });
                if is_action_row {
                    continue;
                }
                let mut texts = Vec::new();
                let mut is_user = false;
                let mut is_ai = false;
                let mut has_tti_image = false;
                for node in nodes {
                    let raw_id = node
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or("")
                        .to_lowercase();
                    let id = raw_id
                        .rsplit_once('/')
                        .map(|(_, name)| name)
                        .unwrap_or(raw_id.as_str());
                    if id.contains("user")
                        || id.contains("right")
                        || id.contains("send")
                        || id.contains("sent")
                        || id.contains("question")
                        || id.contains("mine")
                        || id.contains("prompt")
                    {
                        is_user = true;
                    }
                    if id.contains("ai")
                        || id.contains("amoo")
                        || id.contains("bot")
                        || id.contains("reply")
                        || id.contains("receive")
                        || id.contains("message_container")
                        || id.contains("deep")
                        || id.contains("tti_card")
                        || id.contains("answer")
                        || id.contains("assistant")
                        || id.contains("markdown")
                    {
                        is_ai = true;
                    }
                    if id.contains("iv_tti_card_result_image")
                        || id.contains("layout_tti_card_visual")
                    {
                        has_tti_image = true;
                    }
                    let text = node
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::trim)
                        .unwrap_or("");
                    let chosen = if !text.is_empty() {
                        text
                    } else if id.contains("iv_refresh")
                        || id.contains("iv_copy")
                        || id.contains("iv_like")
                        || id.contains("iv_dislike")
                    {
                        ""
                    } else {
                        node.get("description")
                            .and_then(Value::as_str)
                            .map(str::trim)
                            .unwrap_or("")
                    };
                    if !chosen.is_empty()
                        && chosen != "发消息或按住说话"
                        && chosen != "换一个"
                        && texts.last().map(String::as_str) != Some(chosen)
                    {
                        texts.push(chosen.to_string());
                    }
                }
                if has_tti_image && !texts.iter().any(|t| t.contains("[AI 生成图片卡片]")) {
                    texts.insert(
                        texts.len().min(1),
                        "[AI 生成图片卡片 · 手机端已渲染高清写真]".to_string(),
                    );
                }
                if !texts.is_empty() {
                    let combined = texts.join("\n\n");
                    let user_turn = is_user && !is_ai;
                    turns.push(AmooTurn {
                        speaker: if user_turn {
                            "我".into()
                        } else if is_ai {
                            "Amoo".into()
                        } else {
                            "对话内容".into()
                        },
                        is_user: user_turn,
                        text: combined,
                    });
                }
            }
        }
    }
    AmooSnapshot {
        turns,
        sessions,
        selected_title,
        prepared,
    }
}

pub fn amoo_snapshot(adb: &Adb, device: &Device, open_if_needed: bool) -> Result<AmooSnapshot> {
    let transport = device.resolve_active_transport(adb)?;
    if open_if_needed {
        let _ = device.open_amoo_on_transport(adb, &transport);
        std::thread::sleep(Duration::from_millis(300));
    }
    let val = call_bridge_on_transport(
        adb,
        &transport,
        &json!({ "op": "amoo_snapshot", "debug": false }),
        Duration::from_secs(20),
    )?;
    Ok(parse_amoo_snapshot(&val))
}

pub fn amoo_sync(
    adb: &Adb,
    device: &Device,
    select_title: Option<&str>,
    select_index: Option<i32>,
) -> Result<AmooSnapshot> {
    let transport = device.resolve_active_transport(adb)?;
    let _ = device.open_amoo_on_transport(adb, &transport);
    std::thread::sleep(Duration::from_millis(250));
    let val = call_bridge_on_transport(
        adb,
        &transport,
        &json!({
            "op": "amoo_sync",
            "select_title": select_title.unwrap_or(""),
            "select_index": select_index.unwrap_or(-1),
        }),
        Duration::from_secs(25),
    )?;
    Ok(parse_amoo_snapshot(&val))
}

pub fn amoo_new_chat(adb: &Adb, device: &Device) -> Result<AmooSnapshot> {
    let transport = device.resolve_active_transport(adb)?;
    let _ = device.open_amoo_on_transport(adb, &transport);
    std::thread::sleep(Duration::from_millis(250));
    let val = call_bridge_on_transport(
        adb,
        &transport,
        &json!({ "op": "amoo_new_chat" }),
        Duration::from_secs(20),
    )?;
    Ok(parse_amoo_snapshot(&val))
}

pub fn amoo_prepare(adb: &Adb, device: &Device, text: &str) -> Result<AmooSnapshot> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 8000 {
        bail!("问题不能为空，且最多 8000 字");
    }
    let transport = device.resolve_active_transport(adb)?;
    let _ = device.open_amoo_on_transport(adb, &transport);
    std::thread::sleep(Duration::from_millis(350));
    let val = call_bridge_on_transport(
        adb,
        &transport,
        &json!({ "op": "amoo_prepare", "text": text }),
        Duration::from_secs(20),
    )?;
    Ok(parse_amoo_snapshot(&val))
}

pub fn amoo_send(adb: &Adb, device: &Device, text: &str) -> Result<AmooSnapshot> {
    let text = text.trim();
    if text.is_empty() || text.chars().count() > 8000 {
        bail!("问题不能为空，且最多 8000 字");
    }
    let transport = device.resolve_active_transport(adb)?;
    let _ = device.open_amoo_on_transport(adb, &transport);
    std::thread::sleep(Duration::from_millis(350));
    if call_bridge_on_transport(
        adb,
        &transport,
        &json!({ "op": "amoo_send", "text": text }),
        Duration::from_secs(25),
    )
    .is_err()
    {
        let quoted = shell_quote(text);
        let fallback_script = format!(
            "am broadcast -a com.stepos.ailauncher.action.CARD_SEND_TEXT -p com.stepos.ailauncher --es text {q} >/dev/null 2>&1 || true; \
             am start -a android.intent.action.SEND -n com.stepos.ailauncher/.chat.activity.ShareActivity -t text/plain --es android.intent.extra.TEXT {q} >/dev/null 2>&1 || true",
            q = quoted
        );
        let _ = adb.script(&transport, &fallback_script);
    }
    std::thread::sleep(Duration::from_millis(600));
    call_bridge_on_transport(
        adb,
        &transport,
        &json!({ "op": "amoo_snapshot", "debug": false }),
        Duration::from_secs(20),
    )
    .map(|val| parse_amoo_snapshot(&val))
    .or_else(|_| {
        Ok(AmooSnapshot {
            turns: vec![AmooTurn {
                speaker: "我".into(),
                is_user: true,
                text: text.to_string(),
            }],
            ..Default::default()
        })
    })
}

pub fn sms_prepare(
    adb: &Adb,
    device: &Device,
    number: &str,
    body: &str,
    subscription: Option<i32>,
) -> Result<SmsPlan> {
    let number = number.trim();
    if number.is_empty()
        || number.len() > 40
        || !number
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "+ -()".contains(c))
    {
        bail!("请输入有效的收件人号码");
    }
    if body.trim().is_empty() || body.chars().count() > 2000 {
        bail!("短信正文不能为空，最多 2000 字");
    }
    let req = json!({
        "op": "sms_prepare",
        "number": number,
        "body": body,
        "subscription": subscription.unwrap_or(-1),
    });
    let val = call_bridge(adb, device, &req, Duration::from_secs(12))?;
    let sub_id = val
        .get("subscription")
        .and_then(Value::as_i64)
        .context("手机未返回有效的发信 SIM")? as i32;
    let parts = val
        .get("parts")
        .and_then(Value::as_u64)
        .context("手机未返回短信分段数")? as usize;
    let sub_label = val
        .get("subscription_label")
        .and_then(Value::as_str)
        .map(friendly_carrier_name)
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| format!("SIM {sub_id}"));
    let request_id = format!(
        "{:016x}{:016x}",
        rand::random::<u64>(),
        rand::random::<u64>()
    );
    Ok(SmsPlan {
        request_id,
        number: number.to_string(),
        body: body.to_string(),
        subscription: sub_id,
        subscription_label: sub_label,
        parts,
    })
}

pub fn parse_sms_status(val: &Value) -> SmsSendStatus {
    SmsSendStatus {
        id: val
            .get("id")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string(),
        state: val
            .get("state")
            .and_then(Value::as_str)
            .unwrap_or("unknown")
            .to_string(),
        parts: val.get("parts").and_then(Value::as_u64).unwrap_or(1) as usize,
        completed: val.get("completed").and_then(Value::as_u64).unwrap_or(0) as usize,
        error_code: val
            .get("error_code")
            .and_then(Value::as_i64)
            .map(|c| c as i32),
        error: val.get("error").and_then(Value::as_str).map(str::to_owned),
        updated_ms: val.get("updated_ms").and_then(Value::as_i64),
    }
}

pub fn sms_send(adb: &Adb, device: &Device, plan: &SmsPlan) -> Result<SmsSendStatus> {
    let req = json!({
        "op": "sms_send",
        "id": plan.request_id,
        "number": plan.number,
        "body": plan.body,
        "subscription": plan.subscription,
    });
    match call_bridge(adb, device, &req, Duration::from_secs(20)) {
        Ok(val) => Ok(parse_sms_status(&val)),
        Err(error) => Ok(SmsSendStatus {
            id: plan.request_id.clone(),
            state: "unknown".into(),
            parts: plan.parts,
            completed: 0,
            error_code: None,
            error: Some(format!("{error:#}")),
            updated_ms: None,
        }),
    }
}

pub fn sms_status(adb: &Adb, device: &Device, request_id: &str) -> Result<SmsSendStatus> {
    let req = json!({
        "op": "sms_status",
        "id": request_id,
    });
    let val = call_bridge(adb, device, &req, Duration::from_secs(12))?;
    Ok(parse_sms_status(&val))
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageSummary {
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub free_bytes: u64,
    pub used_bytes: u64,
    pub filesystem: String,
    pub scanned_files: usize,
    pub scanned_bytes: u64,
    pub duplicate_bytes: u64,
    pub scanned_at_ms: i64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageCategory {
    pub name: String,
    pub files: usize,
    pub bytes: u64,
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageFileOpResult {
    pub completed: bool,
    pub available_before: u64,
    pub available_after: u64,
    pub logical_saved: i64,
    pub result_path: String,
}

pub fn parse_storage_summary(val: &Value) -> Result<StorageSummary> {
    let total_bytes = val
        .get("total_bytes")
        .and_then(Value::as_u64)
        .context("手机未返回总存储容量 (total_bytes)")?;
    let available_bytes = val
        .get("available_bytes")
        .and_then(Value::as_u64)
        .context("手机未返回可用存储容量 (available_bytes)")?;
    let free_bytes = val
        .get("free_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(available_bytes);
    if total_bytes == 0 || free_bytes > total_bytes || available_bytes > free_bytes {
        bail!("手机返回的存储容量数据无效");
    }
    let used_bytes = total_bytes.saturating_sub(available_bytes);
    let filesystem = val
        .get("filesystem")
        .and_then(Value::as_str)
        .unwrap_or("未知")
        .to_string();
    let scanned_files = val
        .get("scanned_files")
        .and_then(Value::as_u64)
        .unwrap_or(0) as usize;
    let scanned_bytes = val
        .get("scanned_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let duplicate_bytes = val
        .get("duplicate_bytes")
        .and_then(Value::as_u64)
        .unwrap_or(0);
    let scanned_at_ms = val
        .get("scanned_at_ms")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    Ok(StorageSummary {
        total_bytes,
        available_bytes,
        free_bytes,
        used_bytes,
        filesystem,
        scanned_files,
        scanned_bytes,
        duplicate_bytes,
        scanned_at_ms,
    })
}

pub fn parse_storage_categories(val: &Value) -> Result<Vec<StorageCategory>> {
    let folders = val
        .get("folders")
        .or_else(|| val.get("categories"))
        .and_then(Value::as_array)
        .context("手机未返回存储分类列表 (folders)")?;
    let mut out = Vec::with_capacity(folders.len());
    for item in folders {
        let name = item
            .get("name")
            .and_then(Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string();
        if name.is_empty() || name.starts_with('.') {
            continue;
        }
        let files = item.get("files").and_then(Value::as_u64).unwrap_or(0) as usize;
        let bytes = item.get("bytes").and_then(Value::as_u64).unwrap_or(0);
        out.push(StorageCategory { name, files, bytes });
    }
    Ok(out)
}

pub fn parse_storage_file_op_result(val: &Value) -> Result<StorageFileOpResult> {
    let completed = val
        .get("completed")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let available_before = val
        .get("available_before")
        .and_then(Value::as_u64)
        .context("缺少操作前可用空间 (available_before)")?;
    let available_after = val
        .get("available_after")
        .and_then(Value::as_u64)
        .context("缺少操作后可用空间 (available_after)")?;
    let logical_saved = val
        .get("logical_saved")
        .and_then(Value::as_i64)
        .unwrap_or(0);
    let result_path = val
        .get("result_path")
        .and_then(Value::as_str)
        .unwrap_or("")
        .to_string();
    Ok(StorageFileOpResult {
        completed,
        available_before,
        available_after,
        logical_saved,
        result_path,
    })
}

/// Queries `StorageTools.scan()` via `Bridge.java` (`op: "storage_scan"`) and returns structured storage summary metrics.
pub fn storage_summary(adb: &Adb, device: &Device) -> Result<StorageSummary> {
    let val = call_bridge(
        adb,
        device,
        &json!({ "op": "storage_scan" }),
        Duration::from_secs(45),
    )?;
    parse_storage_summary(&val)
}

/// Queries `StorageTools.scan()` via `Bridge.java` (`op: "storage_scan"`) and returns per-folder category statistics.
pub fn storage_categories(adb: &Adb, device: &Device) -> Result<Vec<StorageCategory>> {
    let val = call_bridge(
        adb,
        device,
        &json!({ "op": "storage_scan" }),
        Duration::from_secs(45),
    )?;
    parse_storage_categories(&val)
}

/// Executes `FileTools.clearCache()` via `Bridge.java` (`op: "storage_clear_cache"`).
pub fn storage_clean(adb: &Adb, device: &Device) -> Result<StorageFileOpResult> {
    let val = call_bridge(
        adb,
        device,
        &json!({ "op": "storage_clear_cache" }),
        Duration::from_secs(60),
    )?;
    parse_storage_file_op_result(&val)
}

/// Executes a verified file operation (`delete_file`, `delete_duplicate`, `compress`, `restore`)
/// through `FileTools.run()` in `Bridge.java` (`op: "storage_file_operation"`).
pub fn storage_file_operation(
    adb: &Adb,
    device: &Device,
    action: &str,
    path: &str,
    bytes: u64,
    modified_ms: u64,
    keep: Option<&str>,
) -> Result<StorageFileOpResult> {
    crate::storage::validate_path(path)?;
    if let Some(k) = keep.filter(|s| !s.is_empty()) {
        crate::storage::validate_path(k)?;
    }
    let req = json!({
        "op": "storage_file_operation",
        "action": action,
        "path": path,
        "bytes": bytes,
        "modified_ms": modified_ms,
        "keep": keep.unwrap_or(""),
    });
    let val = call_bridge(adb, device, &req, Duration::from_secs(60))?;
    parse_storage_file_op_result(&val)
}

/// Lists entries in a public `/sdcard` directory.
pub fn list_files(adb: &Adb, device: &Device, path: &str) -> Result<Vec<crate::storage::Entry>> {
    crate::storage::list(adb, device, path)
}

/// Creates a new directory under `directory` on `/sdcard`.
pub fn create_dir(adb: &Adb, device: &Device, directory: &str, name: &str) -> Result<String> {
    crate::storage::mkdir(adb, device, directory, name)
}

/// Renames a file or directory on `/sdcard` within its parent directory.
pub fn rename_path(adb: &Adb, device: &Device, path: &str, new_name: &str) -> Result<String> {
    crate::storage::rename(adb, device, path, new_name)
}

fn is_bridge_public_folder_file(path: &str) -> bool {
    let Some(rel) = path.strip_prefix("/sdcard/") else {
        return false;
    };
    let mut parts = rel.split('/');
    let Some(top) = parts.next() else {
        return false;
    };
    let Some(rest) = parts.next() else {
        return false;
    };
    !rest.is_empty()
        && matches!(
            top,
            "Download" | "Documents" | "DCIM" | "Pictures" | "Movies" | "Music"
        )
}

/// Deletes a file or directory on `/sdcard`. When the target is a regular file inside one of the
/// public folders managed by `FileTools.java` (`Download`, `Documents`, `DCIM`, `Pictures`,
/// `Movies`, `Music`), invokes `FileTools.run(action="delete_file")` via `Bridge.java` and
/// falls back to `storage::delete` only for directories or paths outside those 6 folders.
pub fn delete_path(adb: &Adb, device: &Device, path: &str) -> Result<StorageFileOpResult> {
    crate::storage::validate_path(path)?;
    if path.trim_end_matches('/') == "/sdcard" {
        bail!("不能删除共享存储根目录");
    }
    let transport = device.resolve_active_transport(adb)?;
    let quoted = shell_quote(path);
    let check_cmd = format!(
        "if [ -L {p} ]; then echo symlink; elif [ -f {p} ]; then stat -c 'file %s %Y' {p}; elif [ -d {p} ]; then echo dir; else echo missing; fi",
        p = quoted
    );
    let check_out = adb.script(&transport, &check_cmd)?;
    let check_trimmed = check_out.trim();
    if check_trimmed == "missing" {
        bail!("目标文件或文件夹不存在");
    }
    if check_trimmed == "symlink" {
        bail!("不支持删除符号链接");
    }
    if is_bridge_public_folder_file(path) && check_trimmed.starts_with("file ") {
        let mut toks = check_trimmed.split_whitespace();
        let _kind = toks.next();
        if let (Some(b_str), Some(sec_str)) = (toks.next(), toks.next()) {
            if let (Ok(bytes), Ok(secs)) = (b_str.parse::<u64>(), sec_str.parse::<u64>()) {
                return storage_file_operation(
                    adb,
                    device,
                    "delete_file",
                    path,
                    bytes,
                    secs.saturating_mul(1000),
                    None,
                );
            }
        }
    }
    crate::storage::delete(adb, device, path)?;
    Ok(StorageFileOpResult {
        completed: true,
        available_before: 0,
        available_after: 0,
        logical_saved: 0,
        result_path: path.to_string(),
    })
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCall {
    pub number: String,
    pub name: String,
    pub call_type: i32,
    pub date_ms: i64,
    pub duration_secs: i64,
}

pub type CallRecord = BridgeCall;

impl BridgeCall {
    pub fn is_missed(&self) -> bool {
        matches!(self.call_type, 3 | 5) || (self.call_type == 1 && self.duration_secs == 0)
    }

    pub fn is_long_business_call(&self) -> bool {
        self.duration_secs >= 60
    }

    pub fn display_peer(&self) -> String {
        let name = self.name.trim();
        let num = self.number.trim();
        if !name.is_empty() && name != "null" && name != num {
            if num.is_empty() {
                name.to_string()
            } else {
                format!("{name} ({num})")
            }
        } else if !num.is_empty() {
            num.to_string()
        } else {
            "未知来电".to_string()
        }
    }

    pub fn type_label(&self) -> &'static str {
        match self.call_type {
            1 => {
                if self.duration_secs == 0 {
                    "未接来电"
                } else {
                    "呼入通话"
                }
            }
            2 => "呼出通话",
            3 => "未接来电",
            5 => "拒接来电",
            _ => "通话记录",
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeContact {
    pub display_name: String,
    pub number: String,
}

pub type ContactRecord = BridgeContact;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeCalendarEvent {
    pub event_id: i64,
    pub title: String,
    pub description: String,
    pub location: String,
    pub begin_ms: i64,
    pub end_ms: i64,
    pub all_day: bool,
}

pub type CalendarRecord = BridgeCalendarEvent;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeMediaItem {
    pub path: String,
    pub name: String,
    pub kind: String,
    pub bytes: u64,
    pub modified_ms: i64,
}

pub type MediaRecord = BridgeMediaItem;

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct BridgeStatus {
    pub send_sms: bool,
    pub locked: bool,
    pub default_subscription: i32,
    pub subscriptions: Vec<SimSubscription>,
    pub screen_time_minutes: u32,
    pub unlock_count: u32,
    pub clipboard: String,
    pub dnd_on: bool,
    pub wifi_on: bool,
    pub bluetooth_on: bool,
    pub brightness_percent: u8,
}

fn value_as_i64_flexible(v: Option<&Value>) -> i64 {
    match v {
        Some(Value::Number(n)) => n.as_i64().unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse::<i64>().unwrap_or(0),
        _ => 0,
    }
}

fn value_as_string_clean(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => {
            let trimmed = s.trim();
            if trimmed.eq_ignore_ascii_case("null") {
                String::new()
            } else {
                trimmed.to_string()
            }
        }
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

pub fn parse_calls(val: &Value) -> Result<Vec<BridgeCall>> {
    let rows = val
        .get("rows")
        .or_else(|| val.get("calls"))
        .and_then(Value::as_array)
        .context("手机未返回通话记录列表 (rows)")?;
    let mut out = Vec::with_capacity(rows.len());
    for item in rows {
        let number = value_as_string_clean(item.get("number"));
        let name = value_as_string_clean(item.get("name").or_else(|| item.get("cached_name")));
        let call_type = value_as_i64_flexible(item.get("type").or_else(|| item.get("call_type"))) as i32;
        let date_ms = value_as_i64_flexible(item.get("date").or_else(|| item.get("date_ms")));
        let duration_secs =
            value_as_i64_flexible(item.get("duration").or_else(|| item.get("duration_secs")))
                .max(0);
        if number.is_empty() && name.is_empty() && date_ms <= 0 {
            continue;
        }
        out.push(BridgeCall {
            number,
            name,
            call_type: if call_type <= 0 { 1 } else { call_type },
            date_ms,
            duration_secs,
        });
    }
    out.sort_by(|a, b| b.date_ms.cmp(&a.date_ms));
    Ok(out)
}

pub fn parse_calls_content_output(raw: &str, limit: usize) -> Vec<BridgeCall> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("Row:") {
            continue;
        }
        let mut number = String::new();
        let mut name = String::new();
        let mut call_type = 1i32;
        let mut date_ms = 0i64;
        let mut duration_secs = 0i64;
        for part in trimmed.split(", ") {
            if let Some(v) = part.split("number=").nth(1) {
                let v = v.trim();
                if v != "NULL" && v != "null" {
                    number = v.to_string();
                }
            } else if let Some(v) = part.strip_prefix("name=") {
                let v = v.trim();
                if v != "NULL" && v != "null" {
                    name = v.to_string();
                }
            } else if let Some(v) = part.strip_prefix("type=") {
                call_type = v.trim().parse::<i32>().unwrap_or(1);
            } else if let Some(v) = part.strip_prefix("date=") {
                date_ms = v.trim().parse::<i64>().unwrap_or(0);
            } else if let Some(v) = part.strip_prefix("duration=") {
                duration_secs = v.trim().parse::<i64>().unwrap_or(0).max(0);
            }
        }
        if !number.is_empty() || !name.is_empty() || date_ms > 0 {
            out.push(BridgeCall {
                number,
                name,
                call_type,
                date_ms,
                duration_secs,
            });
            if out.len() >= limit.max(1) {
                break;
            }
        }
    }
    out.sort_by(|a, b| b.date_ms.cmp(&a.date_ms));
    out
}

/// Reads recent call logs from `Bridge.java` (`op: "read_calls"`) with fallback to `content query`.
pub fn calls(adb: &Adb, serial: &str, limit: usize) -> Result<Vec<BridgeCall>> {
    let transport = serial.trim();
    if transport.is_empty() {
        bail!("未指定手机连接标识");
    }
    let capped = limit.clamp(1, 200);
    if let Ok(val) = call_bridge_on_transport(
        adb,
        transport,
        &json!({ "op": "read_calls", "limit": capped }),
        Duration::from_secs(12),
    ) {
        if let Ok(mut list) = parse_calls(&val) {
            list.truncate(capped);
            return Ok(list);
        }
    }
    let raw = adb.script(
        transport,
        &format!(
            "content query --uri content://call_log/calls --projection number:name:type:date:duration --sort 'date DESC' 2>/dev/null | head -n {capped}"
        ),
    )?;
    Ok(parse_calls_content_output(&raw, capped))
}

pub fn parse_contacts(val: &Value) -> Result<Vec<BridgeContact>> {
    let rows = val
        .get("rows")
        .or_else(|| val.get("contacts"))
        .and_then(Value::as_array)
        .context("手机未返回联系人列表 (rows)")?;
    let mut out = Vec::with_capacity(rows.len());
    for item in rows {
        let display_name =
            value_as_string_clean(item.get("display_name").or_else(|| item.get("name")));
        let number = value_as_string_clean(item.get("data1").or_else(|| item.get("number")));
        if display_name.is_empty() && number.is_empty() {
            continue;
        }
        out.push(BridgeContact {
            display_name,
            number,
        });
    }
    Ok(out)
}

/// Reads phone contacts from `Bridge.java` (`op: "read_contacts"`) with fallback to `content query`.
pub fn contacts(adb: &Adb, serial: &str, limit: usize) -> Result<Vec<BridgeContact>> {
    let transport = serial.trim();
    if transport.is_empty() {
        bail!("未指定手机连接标识");
    }
    let capped = limit.clamp(1, 500);
    if let Ok(val) = call_bridge_on_transport(
        adb,
        transport,
        &json!({ "op": "read_contacts", "limit": capped }),
        Duration::from_secs(12),
    ) {
        if let Ok(mut list) = parse_contacts(&val) {
            list.truncate(capped);
            return Ok(list);
        }
    }
    let raw = adb.script(
        transport,
        &format!(
            "content query --uri content://com.android.contacts/data/phones --projection display_name:data1 2>/dev/null | head -n {capped}"
        ),
    )?;
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("Row:") {
            continue;
        }
        let mut display_name = String::new();
        let mut number = String::new();
        for part in trimmed.split(", ") {
            if let Some(v) = part.split("display_name=").nth(1) {
                let v = v.trim();
                if v != "NULL" && v != "null" {
                    display_name = v.to_string();
                }
            } else if let Some(v) = part.strip_prefix("data1=") {
                let v = v.trim();
                if v != "NULL" && v != "null" {
                    number = v.to_string();
                }
            }
        }
        if !display_name.is_empty() || !number.is_empty() {
            out.push(BridgeContact {
                display_name,
                number,
            });
            if out.len() >= capped {
                break;
            }
        }
    }
    Ok(out)
}

pub fn parse_calendar(val: &Value) -> Result<Vec<BridgeCalendarEvent>> {
    let rows = val
        .get("rows")
        .or_else(|| val.get("calendar_events"))
        .and_then(Value::as_array)
        .context("手机未返回日程列表 (rows)")?;
    let mut out = Vec::with_capacity(rows.len());
    for item in rows {
        let event_id = value_as_i64_flexible(
            item.get("event_id")
                .or_else(|| item.get("_id"))
                .or_else(|| item.get("id")),
        );
        let title = value_as_string_clean(item.get("title"));
        let description = value_as_string_clean(item.get("description"));
        let location = value_as_string_clean(
            item.get("eventLocation").or_else(|| item.get("location")),
        );
        let begin_ms = value_as_i64_flexible(
            item.get("begin")
                .or_else(|| item.get("dtstart"))
                .or_else(|| item.get("start_ms")),
        );
        let end_ms = value_as_i64_flexible(
            item.get("end")
                .or_else(|| item.get("dtend"))
                .or_else(|| item.get("end_ms")),
        );
        let all_day = match item.get("allDay").or_else(|| item.get("all_day")) {
            Some(Value::Bool(b)) => *b,
            Some(v) => value_as_i64_flexible(Some(v)) != 0,
            None => false,
        };
        if title.is_empty() && begin_ms <= 0 {
            continue;
        }
        out.push(BridgeCalendarEvent {
            event_id,
            title: if title.is_empty() {
                "未命名日程".to_string()
            } else {
                title
            },
            description,
            location,
            begin_ms,
            end_ms: if end_ms > 0 { end_ms } else { begin_ms },
            all_day,
        });
    }
    out.sort_by(|a, b| a.begin_ms.cmp(&b.begin_ms));
    Ok(out)
}

pub fn parse_calendar_content_output(raw: &str, limit: usize) -> Vec<BridgeCalendarEvent> {
    let mut out = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if !trimmed.starts_with("Row:") {
            continue;
        }
        let mut event_id = 0i64;
        let mut title = String::new();
        let mut description = String::new();
        let mut location = String::new();
        let mut begin_ms = 0i64;
        let mut end_ms = 0i64;
        let mut all_day = false;
        for part in trimmed.split(", ") {
            if let Some(v) = part.split("_id=").nth(1) {
                event_id = v.trim().parse::<i64>().unwrap_or(0);
            } else if let Some(v) = part.strip_prefix("title=") {
                let v = v.trim();
                if v != "NULL" && v != "null" {
                    title = v.to_string();
                }
            } else if let Some(v) = part.strip_prefix("description=") {
                let v = v.trim();
                if v != "NULL" && v != "null" {
                    description = v.to_string();
                }
            } else if let Some(v) = part.strip_prefix("eventLocation=") {
                let v = v.trim();
                if v != "NULL" && v != "null" {
                    location = v.to_string();
                }
            } else if let Some(v) = part.strip_prefix("dtstart=") {
                begin_ms = v.trim().parse::<i64>().unwrap_or(0);
            } else if let Some(v) = part.strip_prefix("dtend=") {
                end_ms = v.trim().parse::<i64>().unwrap_or(0);
            } else if let Some(v) = part.strip_prefix("allDay=") {
                let v = v.trim();
                all_day = v == "1" || v.eq_ignore_ascii_case("true");
            }
        }
        if title.is_empty() && begin_ms <= 0 {
            continue;
        }
        out.push(BridgeCalendarEvent {
            event_id,
            title: if title.is_empty() {
                "未命名日程".to_string()
            } else {
                title
            },
            description,
            location,
            begin_ms,
            end_ms: if end_ms > 0 { end_ms } else { begin_ms },
            all_day,
        });
        if out.len() >= limit.max(1) {
            break;
        }
    }
    out.sort_by(|a, b| a.begin_ms.cmp(&b.begin_ms));
    out
}

/// Reads upcoming/recent calendar instances from `Bridge.java` (`op: "read_calendar"`)
/// with a `content query` fallback when `Bridge.java` is unavailable.
pub fn calendar(adb: &Adb, serial: &str, limit: usize) -> Result<Vec<BridgeCalendarEvent>> {
    let transport = serial.trim();
    if transport.is_empty() {
        bail!("未指定手机连接标识");
    }
    let capped = limit.clamp(1, 200);
    if let Ok(val) = call_bridge_on_transport(
        adb,
        transport,
        &json!({ "op": "read_calendar", "limit": capped }),
        Duration::from_secs(15),
    ) {
        if let Ok(mut list) = parse_calendar(&val) {
            list.truncate(capped);
            return Ok(list);
        }
    }
    let raw = adb.script(
        transport,
        &format!(
            "content query --uri content://com.android.calendar/events --projection _id:title:description:eventLocation:dtstart:dtend:allDay --sort 'dtstart DESC' 2>/dev/null | head -n {capped}"
        ),
    )?;
    Ok(parse_calendar_content_output(&raw, capped))
}

pub fn classify_media_kind(path: &str) -> &'static str {
    let lower = path.to_ascii_lowercase();
    if lower.contains("screenshot") || lower.contains("截屏") || lower.contains("截图") {
        "screenshot"
    } else if lower.contains("/recordings/")
        || lower.contains("sound_recorder")
        || lower.contains("call_rec")
        || lower.contains("/sounds/")
        || lower.ends_with(".m4a")
        || lower.ends_with(".aac")
        || lower.ends_with(".amr")
        || lower.ends_with(".wav")
        || lower.ends_with(".mp3")
        || lower.ends_with(".ogg")
    {
        "recording"
    } else if lower.ends_with(".mp4") || lower.ends_with(".mov") || lower.ends_with(".mkv") {
        "video"
    } else if lower.ends_with(".jpg")
        || lower.ends_with(".jpeg")
        || lower.ends_with(".png")
        || lower.ends_with(".webp")
        || lower.ends_with(".heic")
    {
        "photo"
    } else {
        "document"
    }
}

pub fn parse_media(val: &Value, limit: usize) -> Result<Vec<BridgeMediaItem>> {
    let mut items = Vec::new();
    for key in ["media", "recordings", "rows", "largest", "documents"] {
        if let Some(arr) = val.get(key).and_then(Value::as_array) {
            for entry in arr {
                let path = value_as_string_clean(entry.get("path"));
                if path.is_empty() {
                    continue;
                }
                let name = value_as_string_clean(entry.get("name")).chars().collect::<String>();
                let resolved_name = if name.is_empty() {
                    path.rsplit('/').next().unwrap_or(&path).to_string()
                } else {
                    name
                };
                let bytes = entry
                    .get("bytes")
                    .or_else(|| entry.get("size_bytes"))
                    .and_then(Value::as_u64)
                    .unwrap_or(0);
                let modified_ms = value_as_i64_flexible(
                    entry.get("modified_ms").or_else(|| entry.get("date_ms")),
                );
                let kind = value_as_string_clean(entry.get("kind"));
                let resolved_kind = if !kind.is_empty() {
                    kind
                } else if key == "recordings" {
                    "recording".to_string()
                } else {
                    classify_media_kind(&path).to_string()
                };
                if items.iter().any(|existing: &BridgeMediaItem| existing.path == path) {
                    continue;
                }
                items.push(BridgeMediaItem {
                    path,
                    name: resolved_name,
                    kind: resolved_kind,
                    bytes,
                    modified_ms,
                });
            }
        }
    }
    items.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    items.truncate(limit.max(1));
    Ok(items)
}

pub fn parse_media_find_output(raw: &str, limit: usize) -> Vec<BridgeMediaItem> {
    let mut items = Vec::new();
    for line in raw.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let mut parts = trimmed.splitn(3, '|');
        let Some(ts_str) = parts.next() else {
            continue;
        };
        let Some(bytes_str) = parts.next() else {
            continue;
        };
        let Some(path_str) = parts.next() else {
            continue;
        };
        let path = path_str.trim().to_string();
        if path.is_empty() || !path.starts_with("/sdcard/") {
            continue;
        }
        let secs = ts_str
            .trim()
            .split('.')
            .next()
            .and_then(|s| s.parse::<i64>().ok())
            .unwrap_or(0);
        let modified_ms = if secs > 1_000_000_000_000 {
            secs
        } else {
            secs.saturating_mul(1000)
        };
        let bytes = bytes_str.trim().parse::<u64>().unwrap_or(0);
        let name = path.rsplit('/').next().unwrap_or(&path).to_string();
        if name.starts_with('.') {
            continue;
        }
        let kind = classify_media_kind(&path).to_string();
        if items.iter().any(|existing: &BridgeMediaItem| existing.path == path) {
            continue;
        }
        items.push(BridgeMediaItem {
            path,
            name,
            kind,
            bytes,
            modified_ms,
        });
    }
    items.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
    items.truncate(limit.max(1));
    items
}

/// Scans recent phone media items (voice recordings, screenshots, photos, videos) with `limit`.
pub fn media(adb: &Adb, serial: &str, limit: usize) -> Result<Vec<BridgeMediaItem>> {
    let transport = serial.trim();
    if transport.is_empty() {
        bail!("未指定手机连接标识");
    }
    let capped = limit.clamp(1, 200);
    let mut combined = Vec::new();
    if let Ok(rec_val) = call_bridge_on_transport(
        adb,
        transport,
        &json!({ "op": "read_recordings" }),
        Duration::from_secs(10),
    ) {
        if let Ok(rec_items) = parse_media(&rec_val, capped) {
            combined.extend(rec_items);
        }
    }
    let find_script = format!(
        "find /sdcard/Recordings /sdcard/Music/Recordings /sdcard/Sounds /sdcard/MIUI/sound_recorder /sdcard/Pictures/Screenshots /sdcard/DCIM/Screenshots /sdcard/DCIM/Camera -maxdepth 3 -type f -printf '%T@|%s|%p\\n' 2>/dev/null | sort -rn | head -n {capped}"
    );
    if let Ok(raw) = adb.script(transport, &find_script) {
        for item in parse_media_find_output(&raw, capped) {
            if !combined.iter().any(|existing: &BridgeMediaItem| existing.path == item.path) {
                combined.push(item);
            }
        }
    }
    if !combined.is_empty() {
        combined.sort_by(|a, b| b.modified_ms.cmp(&a.modified_ms));
        combined.truncate(capped);
        return Ok(combined);
    }
    let val = call_bridge_on_transport(
        adb,
        transport,
        &json!({ "op": "storage_scan" }),
        Duration::from_secs(30),
    )?;
    parse_media(&val, capped)
}

pub fn parse_status(caps_val: &Value, usage_val: Option<&Value>) -> Result<BridgeStatus> {
    let caps = parse_capabilities(caps_val)?;
    let mut status = BridgeStatus {
        send_sms: caps.send_sms,
        locked: caps.locked,
        default_subscription: caps.default_subscription,
        subscriptions: caps.subscriptions,
        ..BridgeStatus::default()
    };
    if let Some(usage) = usage_val {
        status.screen_time_minutes = usage
            .get("screen_time_minutes")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        status.unlock_count = usage
            .get("unlock_count")
            .and_then(Value::as_u64)
            .unwrap_or(0) as u32;
        status.clipboard = value_as_string_clean(usage.get("clipboard"));
        if let Some(ctrl) = usage.get("controls") {
            status.dnd_on = ctrl.get("dnd_on").and_then(Value::as_bool).unwrap_or(false);
            status.wifi_on = ctrl.get("wifi_on").and_then(Value::as_bool).unwrap_or(false);
            status.bluetooth_on = ctrl
                .get("bluetooth_on")
                .and_then(Value::as_bool)
                .unwrap_or(false);
            status.brightness_percent = ctrl
                .get("brightness_percent")
                .and_then(Value::as_u64)
                .unwrap_or(0)
                .min(100) as u8;
        }
    }
    Ok(status)
}

/// Queries live phone status (`capabilities` + `stepos_usage`) via `Bridge.java`.
pub fn status(adb: &Adb, serial: &str) -> Result<BridgeStatus> {
    let transport = serial.trim();
    if transport.is_empty() {
        bail!("未指定手机连接标识");
    }
    let caps_val = call_bridge_on_transport(
        adb,
        transport,
        &json!({ "op": "capabilities" }),
        Duration::from_secs(10),
    )?;
    let usage_val = call_bridge_on_transport(
        adb,
        transport,
        &json!({ "op": "stepos_usage" }),
        Duration::from_secs(12),
    )
    .ok();
    parse_status(&caps_val, usage_val.as_ref())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn verified_bridge_cache_lifecycle_and_invalidation() {
        let transport_a = "test-transport-cache-a";
        let transport_b = "test-transport-cache-b";
        invalidate_bridge_cache(transport_a);
        invalidate_bridge_cache(transport_b);

        assert!(!is_bridge_jar_cached(transport_a));
        assert!(!is_bridge_jar_cached(""));

        mark_bridge_jar_cached(transport_a);
        mark_bridge_jar_cached(transport_b);
        assert!(is_bridge_jar_cached(transport_a));
        assert!(is_bridge_jar_cached(transport_b));

        invalidate_bridge_cache(transport_a);
        assert!(!is_bridge_jar_cached(transport_a));
        assert!(is_bridge_jar_cached(transport_b));

        invalidate_bridge_cache(transport_b);
        assert!(!is_bridge_jar_cached(transport_b));
    }

    #[test]
    fn retry_detector_matches_missing_jar_errors_only() {
        let missing_err = parse_bridge_output(
            "",
            "java.lang.ClassNotFoundException: Didn't find class \"com.stepx.companion.Bridge\"",
        )
        .unwrap_err();
        assert!(should_retry_cached_bridge(&missing_err));

        let empty_err = parse_bridge_output("", "").unwrap_err();
        assert!(should_retry_cached_bridge(&empty_err));

        let business_err =
            parse_bridge_output("STEPX_RESULT:{\"ok\":false,\"error\":\"屏幕已锁定\"}", "")
                .unwrap_err();
        assert!(!should_retry_cached_bridge(&business_err));
    }

    #[test]
    fn parse_bridge_output_and_capabilities_work() {
        let out = "noise\nSTEPX_RESULT:{\"ok\":true,\"send_sms\":true,\"locked\":false,\"default_subscription\":2,\"protocol\":1,\"subscriptions\":[{\"id\":2,\"slot\":1,\"label\":\"CMCC\"}]}\n";
        let val = parse_bridge_output(out, "").unwrap();
        let caps = parse_capabilities(&val).unwrap();
        assert!(caps.send_sms);
        assert!(!caps.locked);
        assert_eq!(caps.default_subscription, 2);
        assert_eq!(caps.subscriptions.len(), 1);
        assert_eq!(caps.subscriptions[0].slot, 1);
        assert_eq!(caps.subscriptions[0].label, "中国移动");
        assert_eq!(friendly_carrier_name("CT"), "中国电信");
        assert_eq!(friendly_carrier_name("CHN-CT"), "中国电信");
        assert_eq!(friendly_carrier_name("CUCC"), "中国联通");
        assert_eq!(friendly_carrier_name("CBN"), "中国广电");
    }

    #[test]
    fn parse_storage_summary_categories_and_file_ops_work() {
        let scan_json = json!({
            "ok": true,
            "total_bytes": 512_000_000_000u64,
            "available_bytes": 312_000_000_000u64,
            "free_bytes": 320_000_000_000u64,
            "filesystem": "f2fs",
            "scanned_files": 420,
            "scanned_bytes": 8_500_000_000u64,
            "duplicate_bytes": 120_000_000u64,
            "scanned_at_ms": 1761840000000i64,
            "folders": [
                {"name": "Download", "files": 35, "bytes": 1_200_000_000u64},
                {"name": "DCIM", "files": 300, "bytes": 6_800_000_000u64},
                {"name": ".hidden", "files": 2, "bytes": 1024u64}
            ]
        });
        let summary = parse_storage_summary(&scan_json).unwrap();
        assert_eq!(summary.total_bytes, 512_000_000_000);
        assert_eq!(summary.available_bytes, 312_000_000_000);
        assert_eq!(summary.used_bytes, 200_000_000_000);
        assert_eq!(summary.filesystem, "f2fs");
        assert_eq!(summary.scanned_files, 420);
        assert_eq!(summary.duplicate_bytes, 120_000_000);

        let categories = parse_storage_categories(&scan_json).unwrap();
        assert_eq!(categories.len(), 2);
        assert_eq!(
            categories[0],
            StorageCategory {
                name: "Download".into(),
                files: 35,
                bytes: 1_200_000_000,
            }
        );
        assert_eq!(categories[1].name, "DCIM");

        let invalid_scan = json!({
            "total_bytes": 100u64,
            "available_bytes": 200u64,
            "free_bytes": 200u64
        });
        assert!(parse_storage_summary(&invalid_scan).is_err());

        let clean_json = json!({
            "ok": true,
            "completed": true,
            "available_before": 100_000_000u64,
            "available_after": 150_000_000u64,
            "logical_saved": 50_000_000i64,
            "result_path": "/sdcard/Download/report.txt.gz"
        });
        let op_res = parse_storage_file_op_result(&clean_json).unwrap();
        assert!(op_res.completed);
        assert_eq!(op_res.available_before, 100_000_000);
        assert_eq!(op_res.available_after, 150_000_000);
        assert_eq!(op_res.logical_saved, 50_000_000);
        assert_eq!(op_res.result_path, "/sdcard/Download/report.txt.gz");

        assert!(is_bridge_public_folder_file("/sdcard/Download/a.txt"));
        assert!(is_bridge_public_folder_file("/sdcard/Documents/sub/b.md"));
        assert!(!is_bridge_public_folder_file("/sdcard/Download"));
        assert!(!is_bridge_public_folder_file("/sdcard/CustomDir/a.txt"));
        if let Ok(adb) = Adb::discover() {
            assert!(delete_path(&adb, &Device::default(), "/sdcard").is_err());
            assert!(delete_path(&adb, &Device::default(), "/data/local/tmp").is_err());
        }
    }

    #[test]
    #[ignore = "writes to a physical phone; use verify_native_mcp with an explicit transport"]
    fn live_phone_storage_and_file_tools_roundtrip() {
        let Ok(adb) = Adb::discover() else {
            return;
        };
        let serial = std::env::var("STEPX_VERIFY_SERIAL")
            .expect("Set STEPX_VERIFY_SERIAL to the explicitly authorized test transport");
        assert!(!serial.trim().is_empty(), "STEPX_VERIFY_SERIAL must not be empty");
        let dev = Device::inspect(&adb, &serial).expect("Selected test phone must be reachable");

        let summary = storage_summary(&adb, &dev).expect("storage_summary on live phone");
        assert!(summary.total_bytes > 0);
        assert!(summary.available_bytes <= summary.total_bytes);

        let categories = storage_categories(&adb, &dev).expect("storage_categories on live phone");
        assert!(categories.iter().any(|c| c.name == "Download"));

        let dir_name = format!("stepx_it_{}", rand::random::<u32>());
        let created = create_dir(&adb, &dev, "/sdcard/Download", &dir_name)
            .expect("create_dir on live phone");
        assert_eq!(created, format!("/sdcard/Download/{dir_name}"));

        let renamed_name = format!("{dir_name}_renamed");
        let renamed = rename_path(&adb, &dev, &created, &renamed_name)
            .expect("rename_path on live phone");
        assert_eq!(renamed, format!("/sdcard/Download/{renamed_name}"));

        let entries = list_files(&adb, &dev, "/sdcard/Download").expect("list_files on live phone");
        assert!(entries.iter().any(|e| e.name == renamed_name && e.directory));

        let transport = dev.resolve_active_transport(&adb).expect("resolve transport");
        let regular_file = format!("{renamed}/stepx_probe.txt");
        adb.script(
            &transport,
            &format!("printf 'stepx-bridge-test' > {}", shell_quote(&regular_file)),
        )
        .expect("create regular file in Download subdirectory");

        let deleted_file =
            delete_path(&adb, &dev, &regular_file).expect("delete_path regular file via FileTools");
        assert!(deleted_file.completed);
        assert!(deleted_file.available_before > 0);
        assert_eq!(deleted_file.logical_saved, 17);

        assert!(
            delete_path(&adb, &dev, &regular_file).is_err(),
            "deleting already-deleted file must fail"
        );

        let deleted = delete_path(&adb, &dev, &renamed).expect("delete_path directory on live phone");
        assert!(deleted.completed);
    }

    #[test]
    fn parse_calls_contacts_calendar_media_and_status_work() {
        let calls_val = json!({
            "ok": true,
            "rows": [
                {"number": "13800138000", "name": "王总", "type": "3", "date": "1761840000000", "duration": "0"},
                {"number": "13900139000", "name": "李工", "type": "1", "date": "1761840600000", "duration": "185"}
            ]
        });
        let parsed_calls = parse_calls(&calls_val).unwrap();
        assert_eq!(parsed_calls.len(), 2);
        assert_eq!(parsed_calls[0].name, "李工");
        assert!(parsed_calls[0].is_long_business_call());
        assert!(!parsed_calls[0].is_missed());
        assert_eq!(parsed_calls[1].name, "王总");
        assert!(parsed_calls[1].is_missed());
        assert_eq!(parsed_calls[1].type_label(), "未接来电");
        assert_eq!(parsed_calls[1].display_peer(), "王总 (13800138000)");

        let content_raw = "Row: 0 number=13700001111, name=张经理, type=3, date=1761850000000, duration=0\nRow: 1 number=13600002222, name=NULL, type=2, date=1761845000000, duration=95\n";
        let fallback_calls = parse_calls_content_output(content_raw, 10);
        assert_eq!(fallback_calls.len(), 2);
        assert!(fallback_calls[0].is_missed());
        assert_eq!(fallback_calls[1].display_peer(), "13600002222");

        let contacts_val = json!({
            "ok": true,
            "rows": [
                {"display_name": "张三", "data1": "13800138000"},
                {"display_name": "李四", "data1": "13900139000"}
            ]
        });
        let parsed_contacts = parse_contacts(&contacts_val).unwrap();
        assert_eq!(parsed_contacts.len(), 2);
        assert_eq!(parsed_contacts[0].display_name, "张三");

        let cal_val = json!({
            "ok": true,
            "rows": [
                {
                    "event_id": "42",
                    "title": "Q4 预算评审",
                    "description": "讨论研发预算",
                    "eventLocation": "3号楼A会议室",
                    "begin": "1761840000000",
                    "end": "1761843600000",
                    "allDay": "0"
                },
                {
                    "id": 99,
                    "title": "商务合同复核",
                    "description": "线上腾讯会议",
                    "location": "总部2F",
                    "start_ms": 1761850000000i64,
                    "end_ms": 1761853600000i64,
                    "all_day": false
                }
            ]
        });
        let parsed_cal = parse_calendar(&cal_val).unwrap();
        assert_eq!(parsed_cal.len(), 2);
        assert_eq!(parsed_cal[0].event_id, 42);
        assert_eq!(parsed_cal[0].title, "Q4 预算评审");
        assert_eq!(parsed_cal[0].location, "3号楼A会议室");
        assert_eq!(parsed_cal[1].event_id, 99);
        assert_eq!(parsed_cal[1].begin_ms, 1761850000000);

        let cal_content_raw = "Row: 0 _id=88, title=季度战略会, description=携带Demo, eventLocation=B座402, dtstart=1761860000000, dtend=1761863600000, allDay=0\n";
        let fallback_cal = parse_calendar_content_output(cal_content_raw, 10);
        assert_eq!(fallback_cal.len(), 1);
        assert_eq!(fallback_cal[0].event_id, 88);
        assert_eq!(fallback_cal[0].location, "B座402");

        let find_raw = "1761841200.0|409600|/sdcard/Recordings/20261031_客户会议录音.m4a\n1761841800.0|819200|/sdcard/Pictures/Screenshots/Screenshot_20261031_报价单.png\n";
        let media_items = parse_media_find_output(find_raw, 10);
        assert_eq!(media_items.len(), 2);
        assert_eq!(media_items[0].kind, "screenshot");
        assert_eq!(media_items[1].kind, "recording");

        let rec_json = json!({
            "ok": true,
            "recordings": [
                {
                    "path": "/sdcard/Recordings/20261031_电话纪要.m4a",
                    "name": "20261031_电话纪要.m4a",
                    "size_bytes": 128000u64,
                    "modified_ms": 1761842400000i64
                }
            ]
        });
        let rec_parsed = parse_media(&rec_json, 10).unwrap();
        assert_eq!(rec_parsed.len(), 1);
        assert_eq!(rec_parsed[0].kind, "recording");
        assert_eq!(rec_parsed[0].bytes, 128000);

        let caps_val = json!({
            "ok": true,
            "send_sms": true,
            "locked": false,
            "default_subscription": 1,
            "protocol": 2,
            "subscriptions": [{"id": 1, "slot": 0, "label": "CT"}]
        });
        let usage_val = json!({
            "ok": true,
            "screen_time_minutes": 95,
            "unlock_count": 14,
            "clipboard": "SF1234567890123",
            "controls": {
                "dnd_on": true,
                "wifi_on": true,
                "bluetooth_on": false,
                "brightness_percent": 65
            }
        });
        let st = parse_status(&caps_val, Some(&usage_val)).unwrap();
        assert!(st.send_sms);
        assert_eq!(st.screen_time_minutes, 95);
        assert_eq!(st.clipboard, "SF1234567890123");
        assert!(st.dnd_on);
        assert_eq!(st.brightness_percent, 65);
    }
}
