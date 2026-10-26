use crate::{
    adb::{run_process, shell_quote, Adb},
    device::Device,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    ffi::OsString,
    io::Write,
    path::Path,
    process::Command,
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
    if is_bridge_jar_cached(&device.transport) {
        return Ok(());
    }
    let expected_len = BRIDGE_JAR.len();
    let check_cmd = format!(
        "mkdir -p {dir} && chmod 700 {dir} && [ -f {jar} ] && [ \"$(stat -c %s {jar} 2>/dev/null)\" = \"{expected_len}\" ] && printf READY",
        dir = shell_quote(REMOTE_DIR),
        jar = shell_quote(REMOTE_JAR),
    );
    if let Ok(out) = adb.script(&device.transport, &check_cmd) {
        if out.trim() == "READY" {
            mark_bridge_jar_cached(&device.transport);
            return Ok(());
        }
    }
    let mut temp = tempfile::NamedTempFile::new().context("无法创建临时桥接包")?;
    temp.write_all(BRIDGE_JAR)?;
    temp.as_file().sync_all()?;
    adb.transfer(
        &device.transport,
        "push",
        temp.path(),
        Path::new(REMOTE_JAR),
    )
    .context("无法推送 Android Bridge 到手机")?;
    adb.script(
        &device.transport,
        &format!("chmod 600 {}", shell_quote(REMOTE_JAR)),
    )?;
    mark_bridge_jar_cached(&device.transport);
    Ok(())
}

fn run_bridge_once(
    adb: &Adb,
    device: &Device,
    request: &Value,
    timeout: Duration,
) -> Result<Value> {
    let shell_cmd = format!(
        "CLASSPATH={} app_process {} com.stepx.companion.Bridge",
        shell_quote(REMOTE_JAR),
        shell_quote(REMOTE_DIR)
    );
    let args: Vec<OsString> = vec![
        "-s".into(),
        device.transport.clone().into(),
        "shell".into(),
        shell_cmd.into(),
    ];
    let mut command = Command::new(&adb.path);
    command.args(&args).env("ADB_MDNS_OPENSCREEN", "1");
    let payload = format!("{}\n", serde_json::to_string(request)?);
    let output = run_process(command, Some(payload.into_bytes()), timeout)?;
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

pub fn call_bridge(
    adb: &Adb,
    device: &Device,
    request: &Value,
    timeout: Duration,
) -> Result<Value> {
    device.verify(adb)?;
    let was_cached = is_bridge_jar_cached(&device.transport);
    ensure_bridge_jar(adb, device)?;
    match run_bridge_once(adb, device, request, timeout) {
        Ok(val) => Ok(val),
        Err(err) if was_cached && should_retry_cached_bridge(&err) => {
            invalidate_bridge_cache(&device.transport);
            ensure_bridge_jar(adb, device)?;
            run_bridge_once(adb, device, request, timeout)
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
    if open_if_needed {
        let _ = device.open_amoo(adb);
        std::thread::sleep(Duration::from_millis(300));
    }
    let val = call_bridge(
        adb,
        device,
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
    let _ = device.open_amoo(adb);
    std::thread::sleep(Duration::from_millis(250));
    let val = call_bridge(
        adb,
        device,
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
    let _ = device.open_amoo(adb);
    std::thread::sleep(Duration::from_millis(250));
    let val = call_bridge(
        adb,
        device,
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
    let _ = device.open_amoo(adb);
    std::thread::sleep(Duration::from_millis(350));
    let val = call_bridge(
        adb,
        device,
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
    let _ = device.open_amoo(adb);
    std::thread::sleep(Duration::from_millis(350));
    if call_bridge(
        adb,
        device,
        &json!({ "op": "amoo_send", "text": text }),
        Duration::from_secs(25),
    )
    .is_err()
    {
        let _ = adb.shell(
            &device.transport,
            &[
                "am",
                "broadcast",
                "-a",
                "com.stepos.ailauncher.action.CARD_SEND_TEXT",
                "-p",
                "com.stepos.ailauncher",
                "--es",
                "text",
                text,
            ],
        );
        let _ = adb.shell(
            &device.transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.SEND",
                "-n",
                "com.stepos.ailauncher/.chat.activity.ShareActivity",
                "-t",
                "text/plain",
                "--es",
                "android.intent.extra.TEXT",
                text,
            ],
        );
    }
    std::thread::sleep(Duration::from_millis(600));
    amoo_snapshot(adb, device, false).or_else(|_| {
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
}
