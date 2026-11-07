//! Amoo's own AgentBus channel, over an explicitly selected ADB transport.
//! Screen automation is limited to the native gui_agent capability, with its own
//! lock-screen checks in amoo_gui; no shell/input fallback is provided here.
use crate::{
    adb::Adb,
    device::Device,
    openclaw::{RealAmooData, RealAttachment, RealChatMessage, RealCronInfo, RealSessionInfo},
};
use anyhow::{bail, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashSet,
    io::{ErrorKind, Read, Write},
    net::TcpStream,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const MAX_FRAME: usize = 16 * 1024 * 1024;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(45);
const AMOO_CONTEXT_TTL: Duration = Duration::from_secs(60);
const LIVE_DEVICE_CONTEXT_MARKER: &str = "\n\n[StepX 真机实时硬件、电子三包保修与电池健康档案";

static AMOO_CONTEXT_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (Instant, Value)>>,
> = std::sync::OnceLock::new();

fn should_inject_device_telemetry(text: &str) -> bool {
    let lower = text.to_lowercase();
    [
        "保修",
        "三包",
        "激活",
        "序列号",
        "sn",
        "asn",
        "关于本机",
        "本机信息",
        "手机配置",
        "硬件",
        "型号",
        "电池健康",
        "电池寿命",
        "soh",
        "循环次数",
        "充电循环",
        "旁路",
        "快充",
        "电池容量",
        "温度",
        "发热",
        "发烫",
        "热力",
        "npu",
        "算力",
        "手机状态",
    ]
    .iter()
    .any(|kw| lower.contains(kw))
}

fn format_live_device_telemetry_block(raw: &str, device: &Device) -> Option<String> {
    if raw.trim().is_empty() {
        return None;
    }
    let kv = |key: &str| -> String {
        let prefix = format!("{key}=");
        raw.lines()
            .find_map(|line| line.trim().strip_prefix(&prefix))
            .map(crate::bridge::clean_nullable_field)
            .unwrap_or_default()
    };
    let non_empty = |primary: String, fallback: &str| -> String {
        if primary.is_empty() {
            fallback.to_string()
        } else {
            primary
        }
    };

    let marketname = non_empty(kv("MARKETNAME"), "未读取");
    let model = non_empty(
        kv("MODEL"),
        if device.model.is_empty() {
            "未读取"
        } else {
            &device.model
        },
    );
    let device_code = non_empty(
        kv("DEVICE_CODE"),
        if device.product.is_empty() {
            "未读取"
        } else {
            &device.product
        },
    );
    let serialno = non_empty(
        kv("SERIALNO"),
        if device.serial.is_empty() {
            "未读取"
        } else {
            &device.serial
        },
    );
    let asn = non_empty(kv("ASN"), "未读取");
    let net_sn = non_empty(kv("NET_SN"), "未读取");
    let cmiit = non_empty(kv("CMIIT"), "未读取");
    let display_id = non_empty(kv("DISPLAY_ID"), "未读取");
    let release = non_empty(
        kv("RELEASE"),
        if device.android.is_empty() {
            "未读取"
        } else {
            &device.android
        },
    );

    let soh = non_empty(kv("SOH"), "未读取");
    let design_cap = crate::device::format_mah_value(&kv("DESIGN_CAP"), "未读取");
    let charge_cnt = crate::device::format_mah_value(&kv("CHARGE_CNT"), "未读取");
    let cycle_count = crate::device::extract_cycle_count(&kv("CYCLE_LINE"))
        .unwrap_or_else(|| "未读取".to_string());

    let batt_val = |key: &str| -> String {
        raw.lines()
            .find_map(|l| l.trim().strip_prefix(key))
            .map(str::trim)
            .unwrap_or("")
            .to_string()
    };
    let level = non_empty(batt_val("level:"), "未知");
    let voltage = batt_val("voltage:")
        .parse::<u32>()
        .ok()
        .filter(|v| *v > 0)
        .map(|v| format!("{:.2}", f64::from(v) / 1000.0))
        .unwrap_or_else(|| "未读取".to_string());
    let temp = batt_val("temperature:")
        .parse::<i32>()
        .ok()
        .map(|t| format!("{:.1}", f64::from(t) / 10.0))
        .unwrap_or_else(|| "未读取".to_string());

    let fast_charge_raw = kv("FAST_CHARGE");
    let fast_charge = match fast_charge_raw.as_str() {
        "1" | "true" => "已开启".to_string(),
        "0" | "false" => "已关闭".to_string(),
        other if !other.is_empty() => other.to_string(),
        _ => "未读取".to_string(),
    };

    let ups_mode = kv("UPS_MODE");
    let ups_limit = non_empty(kv("UPS_LIMIT"), "未读取");
    let ups_desc = if ups_mode == "1" {
        format!("已开启（阈值 {ups_limit}%）")
    } else if ups_mode == "0" {
        format!("未开启（设定阈值 {ups_limit}%）")
    } else {
        "未读取".to_string()
    };

    let zram_raw = kv("ZRAM");
    let zram_desc = non_empty(zram_raw, "未读取");
    let camera_raw = non_empty(kv("CAMERA"), "未读取");

    let mut thermal_parts = Vec::new();
    if let Some((_, thermal_sec)) = raw.split_once("---THERMAL---") {
        for line in thermal_sec.lines() {
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let name = line
                .split_once("mName=")
                .map(|(_, r)| {
                    r.split(|c: char| c == ',' || c == '}' || c.is_whitespace())
                        .next()
                        .unwrap_or("")
                })
                .unwrap_or("");
            if !name.is_empty() {
                if let Some(v) = crate::device::parse_thermal_mvalue(line) {
                    thermal_parts.push(format!("{name}: {v:.1} °C"));
                }
            }
        }
    }
    let thermal_line = if thermal_parts.is_empty() {
        String::new()
    } else {
        format!("\n- 核心与壳温热力传感器：{}", thermal_parts.join(" · "))
    };

    Some(format!(
        "{LIVE_DEVICE_CONTEXT_MARKER}（设备只读快照；仅引用已读取的数据。未读取项请明确说明未知，不推断保修或认证状态）]\n\
- 机型与代号：{marketname} ({model} / {device_code})\n\
- 设备序列号 (SN)：{serialno}\n\
- 硬件三包码 (ASN)：{asn}\n\
- 入网序列号 (Net SN)：{net_sn} · CMIIT ID：{cmiit}\n\
- 电子三包保修状态：未查询官方保修服务，无法确认是否在保\n\
- 系统固件版本：Android {release} · {display_id}\n\
- 电池寿命与循环：健康度 (SOH) {soh}% · 充电循环 {cycle_count} 次 · 设计容量 {design_cap} mAh · 当前剩余容量 {charge_cnt} mAh\n\
- 实时电源与快充：电量 {level}% · 电压 {voltage} V · 电池温度 {temp} °C · 智慧快充 {fast_charge} · 旁路供电 {ups_desc}\n\
- ZRAM 配置与相机属性：{zram_desc} · {camera_raw}{thermal_line}"
    ))
}

fn collect_live_device_telemetry_block(adb: &Adb, device: &Device) -> Option<String> {
    if device.transport.is_empty() {
        return None;
    }
    let script = r#"echo "MARKETNAME=$(getprop ro.product.marketname 2>/dev/null)"
echo "MODEL=$(getprop ro.product.model 2>/dev/null)"
echo "DEVICE_CODE=$(getprop ro.product.device 2>/dev/null)"
echo "SERIALNO=$(getprop ro.serialno 2>/dev/null)"
echo "ASN=$(getprop ro.boot.hardware.asn 2>/dev/null)"
echo "NET_SN=$(getprop ro.StepOS.net.sn 2>/dev/null)"
echo "DISPLAY_ID=$(getprop ro.build.display.id 2>/dev/null)"
echo "RELEASE=$(getprop ro.build.version.release 2>/dev/null)"
echo "CMIIT=$(getprop ro.product.cmiit.id 2>/dev/null)"
echo "CAMERA=$(getprop ro.vendor.stepos.camera.info 2>/dev/null)"
echo "ZRAM=$(getprop persist.vendor.stepos.smart_zram.cfg 2>/dev/null)"
echo "SOH=$(cat /sys/class/zy_charger/soh 2>/dev/null || true)"
echo "DESIGN_CAP=$(cat /sys/class/zy_charger/design_capacity 2>/dev/null || true)"
echo "CHARGE_CNT=$(cat /sys/class/zy_charger/charge_counter 2>/dev/null || true)"
echo "UPS_MODE=$(cat /sys/class/zy_charger/ups_mode 2>/dev/null || true)"
echo "UPS_LIMIT=$(cat /sys/class/zy_charger/ups_mode_soc_limit 2>/dev/null || true)"
echo "FAST_CHARGE=$(settings get global stepos_fast_charge 2>/dev/null || true)"
echo "CYCLE_LINE=$(dumpsys activity broadcasts 2>/dev/null | grep -m1 'android.os.extra.CYCLE_COUNT' || true)"
echo "---BATTERY---"
dumpsys battery 2>/dev/null || true
echo "---THERMAL---"
dumpsys thermalservice 2>/dev/null | grep -E 'mName=(cpu-0-0|gpuss-0|nsp-0|skin-therm)' | head -n 6 || true"#;
    let raw = adb
        .script(&device.transport, script)
        .or_else(|_| {
            let active = device.resolve_active_transport(adb)?;
            adb.script(&active, script)
        })
        .ok()?;
    format_live_device_telemetry_block(&raw, device)
}

fn enrich_prompt_with_live_device_telemetry(adb: &Adb, device: &Device, text: &str) -> String {
    if !should_inject_device_telemetry(text) || text.contains(LIVE_DEVICE_CONTEXT_MARKER) {
        return text.to_string();
    }
    if let Some(block) = collect_live_device_telemetry_block(adb, device) {
        format!("{text}{block}")
    } else {
        text.to_string()
    }
}

fn amoo_context_cache(
) -> &'static std::sync::Mutex<std::collections::HashMap<String, (Instant, Value)>> {
    AMOO_CONTEXT_CACHE.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

fn amoo_context_cache_key_for(serial: &str, transport: &str) -> String {
    if serial.is_empty() {
        transport.to_string()
    } else {
        serial.to_string()
    }
}

fn amoo_context_cache_key(device: &Device) -> String {
    amoo_context_cache_key_for(&device.serial, &device.transport)
}

pub fn invalidate_amoo_context_cache(device: &Device) {
    let key = amoo_context_cache_key(device);
    if let Ok(mut guard) = amoo_context_cache().lock() {
        guard.remove(&key);
    }
}

fn get_cached_amoo_context_at(key: &str, now: Instant) -> Option<Value> {
    let mut guard = amoo_context_cache().lock().ok()?;
    if let Some((cached_at, val)) = guard.get(key) {
        if now.saturating_duration_since(*cached_at) < AMOO_CONTEXT_TTL {
            return Some(val.clone());
        }
    }
    guard.remove(key);
    None
}

fn put_cached_amoo_context_at(key: String, now: Instant, val: Value) {
    if let Ok(mut guard) = amoo_context_cache().lock() {
        guard.insert(key, (now, val));
    }
}

pub fn cached_amoo_context(adb: &Adb, device: &Device) -> Result<Value> {
    let key = amoo_context_cache_key(device);
    let now = Instant::now();
    if let Some(cached) = get_cached_amoo_context_at(&key, now) {
        return Ok(cached);
    }
    let req = json!({"op":"amoo_context"});
    let val = crate::bridge::call_bridge_on_transport(
        adb,
        &device.transport,
        &req,
        Duration::from_secs(10),
    )
    .or_else(|e| {
        let active = device.resolve_active_transport(adb)?;
        if active == device.transport {
            Err(e)
        } else {
            crate::bridge::call_bridge_on_transport(adb, &active, &req, Duration::from_secs(10))
        }
    })?;
    put_cached_amoo_context_at(key, Instant::now(), val.clone());
    Ok(val)
}

pub(crate) enum LocalInvokeEnd {
    Reply(Value),
    WaitCancelled,
    TimedOut,
    Disconnected,
}
pub(crate) struct LocalInvokeResult {
    pub id: String,
    pub end: LocalInvokeEnd,
}

fn local_gui_frame(id: &str, app_name: &str, task: &str) -> Result<Value> {
    crate::amoo_gui::validate_request(app_name, task)?;
    Ok(
        json!({"type":"event","event":"node.invoke.request","payload":{
            "id":id,"command":"ability.call","idempotencyKey":id,"timeoutMs":600000,
            "_meta":{"dataClass":"private"},
            "params":{"method":"gui_agent","timeoutMs":600000,
                "invokeParamsJson":serde_json::to_string(&json!({"method":"gui_agent",
                    "params":{"gui_agent_param":{"app_name":app_name.trim(),"task":task.trim()}}}))?}
        }}),
    )
}

/// Unlike restarting read_frame every poll, this retains a partially received
/// header/body while checking cancellation and the original absolute deadline.
fn read_local_frame(
    stream: &mut TcpStream,
    deadline: Instant,
    cancel: &AtomicBool,
) -> LocalInvokeEnd {
    fn exact(
        stream: &mut TcpStream,
        mut bytes: &mut [u8],
        deadline: Instant,
        cancel: &AtomicBool,
    ) -> Result<(), LocalInvokeEnd> {
        while !bytes.is_empty() {
            if cancel.load(Ordering::Relaxed) {
                return Err(LocalInvokeEnd::WaitCancelled);
            }
            let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
                return Err(LocalInvokeEnd::TimedOut);
            };
            if stream
                .set_read_timeout(Some(
                    remaining
                        .min(Duration::from_millis(500))
                        .max(Duration::from_millis(1)),
                ))
                .is_err()
            {
                return Err(LocalInvokeEnd::Disconnected);
            }
            match stream.read(bytes) {
                Ok(0) => return Err(LocalInvokeEnd::Disconnected),
                Ok(count) => bytes = &mut bytes[count..],
                Err(error)
                    if matches!(
                        error.kind(),
                        ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                    ) => {}
                Err(_) => return Err(LocalInvokeEnd::Disconnected),
            }
        }
        Ok(())
    }
    let mut header = [0; 4];
    if let Err(end) = exact(stream, &mut header, deadline, cancel) {
        return end;
    }
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > MAX_FRAME {
        return LocalInvokeEnd::Disconnected;
    }
    let mut body = vec![0; size];
    if let Err(end) = exact(stream, &mut body, deadline, cancel) {
        return end;
    }
    match serde_json::from_slice(&body) {
        Ok(value) => LocalInvokeEnd::Reply(value),
        Err(_) => LocalInvokeEnd::Disconnected,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct StreamDelta {
    pub session_key: String,
    pub run_id: String,
    pub delta_text: String,
    pub full_text: String,
    pub thinking_text: String,
    pub tool_status: Option<String>,
    pub finished: bool,
}

fn extract_content(value: &Value) -> Option<String> {
    if let Some(s) = value.as_str() {
        if !s.is_empty() {
            return Some(crate::amoo_gui_chat::display_text(s));
        }
    }
    if let Some(parts) = value.as_array() {
        let joined = parts
            .iter()
            .filter(|p| p["type"] == "text" || p.get("type").is_none())
            .filter_map(|p| p["text"].as_str().or_else(|| p["content"].as_str()))
            .collect::<Vec<_>>()
            .join("\n");
        if !joined.is_empty() {
            return Some(crate::amoo_gui_chat::display_text(&joined));
        }
    }
    None
}

pub fn parse_stream_event(frame: &Value) -> Option<StreamDelta> {
    let (event, payload) = match frame.get("type").and_then(Value::as_str) {
        Some("event") => (
            string(frame, &["event"]),
            frame.get("payload").unwrap_or(frame),
        ),
        Some("req") if frame.get("method").and_then(Value::as_str) == Some("node.event") => {
            let params = frame.get("params")?;
            (
                string(params, &["event"]),
                params.get("payload").unwrap_or(params),
            )
        }
        _ => return None,
    };
    let supported = matches!(
        event,
        "chat"
            | "chat.delta"
            | "chat.message"
            | "session.message"
            | "agent"
            | "agent.event"
            | "tool"
            | "tool.call"
            | "tool.status"
            | "run"
            | "run.delta"
            | "message"
            | "message.delta"
    ) || event.starts_with("chat.")
        || event.starts_with("session.message")
        || event.starts_with("agent.")
        || event.starts_with("tool.")
        || event.starts_with("run.")
        || event.starts_with("message.");
    if !supported {
        return None;
    }
    let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
    if !role.is_empty() && !matches!(role, "assistant" | "tool") {
        return None;
    }
    let is_tool_role = role == "tool";

    let data = payload.get("data").filter(|v| v.is_object());
    let metadata = payload.get("metadata").filter(|v| v.is_object());

    let raw_session_key = string(
        payload,
        &["sessionKey", "session_key", "sessionId", "session_id"],
    );
    let raw_session_key = if raw_session_key.is_empty() {
        metadata
            .map(|m| string(m, &["sessionKey", "session_key", "sessionId", "session_id"]))
            .unwrap_or("")
    } else {
        raw_session_key
    };
    let raw_session_key = if raw_session_key.is_empty() {
        string(
            frame,
            &["sessionKey", "session_key", "sessionId", "session_id"],
        )
    } else {
        raw_session_key
    };
    if raw_session_key.is_empty() {
        return None;
    }
    let session_key = raw_session_key.to_string();

    let run_id = {
        let r = string(payload, &["runId", "run_id", "id"]);
        let r = if r.is_empty() {
            metadata
                .map(|m| string(m, &["runId", "run_id", "id"]))
                .unwrap_or("")
        } else {
            r
        };
        if r.is_empty() {
            string(frame, &["runId", "run_id"]).to_string()
        } else {
            r.to_string()
        }
    };

    let mut delta_text = if is_tool_role {
        String::new()
    } else {
        string(
            payload,
            &["delta", "textDelta", "contentDelta", "deltaText"],
        )
        .to_string()
    };
    if !is_tool_role && delta_text.is_empty() {
        if let Some(delta_obj) = payload.get("delta").filter(|v| v.is_object()) {
            delta_text =
                string(delta_obj, &["text", "content", "textDelta", "contentDelta"]).to_string();
            if delta_text.is_empty() {
                if let Some(full) = extract_content(&delta_obj["content"]) {
                    delta_text = full;
                }
            }
        }
    }
    if !is_tool_role && delta_text.is_empty() {
        if let Some(d) = data {
            delta_text = string(d, &["delta", "textDelta", "contentDelta", "text"]).to_string();
        }
    }

    let full_text = if is_tool_role {
        String::new()
    } else {
        extract_content(&payload["text"])
            .or_else(|| extract_content(&payload["content"]))
            .or_else(|| {
                payload.get("message").and_then(|m| {
                    extract_content(&m["text"]).or_else(|| extract_content(&m["content"]))
                })
            })
            .unwrap_or_default()
    };

    let mut thinking_text = if is_tool_role {
        String::new()
    } else {
        string(payload, &["thinking", "reasoning", "thinkingDelta"]).to_string()
    };
    if !is_tool_role && thinking_text.is_empty() {
        if let Some(delta_obj) = payload.get("delta").filter(|v| v.is_object()) {
            thinking_text = string(delta_obj, &["thinking", "reasoning"]).to_string();
        }
    }
    if !is_tool_role && thinking_text.is_empty() {
        if let Some(msg_obj) = payload.get("message").filter(|v| v.is_object()) {
            thinking_text = string(msg_obj, &["thinking", "reasoning"]).to_string();
        }
    }
    if !is_tool_role && thinking_text.is_empty() {
        if let Some(d) = data {
            thinking_text = string(d, &["thinking", "reasoning"]).to_string();
        }
    }
    if !is_tool_role && thinking_text.is_empty() {
        if let Some(m) = metadata {
            thinking_text = string(m, &["thinking", "reasoning"]).to_string();
        }
    }

    let state = string(payload, &["state", "status", "stopReason", "stop_reason"]);
    let state = if state.is_empty() {
        payload
            .get("message")
            .map(|m| string(m, &["stopReason", "stop_reason", "state", "status"]))
            .unwrap_or("")
    } else {
        state
    };
    let state = if state.is_empty() {
        metadata
            .map(|m| {
                string(
                    m,
                    &[
                        "lifecyclePhase",
                        "stopReason",
                        "stop_reason",
                        "state",
                        "status",
                    ],
                )
            })
            .unwrap_or("")
    } else {
        state
    };
    let state_lower = state.to_ascii_lowercase();
    let has_tool_calls = payload
        .get("toolCalls")
        .and_then(Value::as_array)
        .is_some_and(|a| !a.is_empty());
    let is_tool_step_state = matches!(
        state_lower.as_str(),
        "tool_use" | "tool_calls" | "tool" | "calling_tool" | "running" | "streaming"
    );
    let streaming_ended_turn = metadata.and_then(|m| m["streaming"].as_bool()) == Some(false)
        && !is_tool_step_state
        && !(has_tool_calls
            && delta_text.is_empty()
            && full_text.is_empty()
            && state_lower != "turn_done");
    let finished = !is_tool_role
        && (payload["done"].as_bool() == Some(true)
            || payload["finished"].as_bool() == Some(true)
            || payload["final"].as_bool() == Some(true)
            || streaming_ended_turn
            || matches!(
                state_lower.as_str(),
                "done"
                    | "turn_done"
                    | "completed"
                    | "finished"
                    | "final"
                    | "error"
                    | "aborted"
                    | "stop"
                    | "end_turn"
                    | "cancelled"
            ));

    let is_tool_event = is_tool_role
        || event == "tool"
        || event.contains("tool")
        || matches!(
            state_lower.as_str(),
            "tool_use" | "tool_calls" | "tool" | "calling_tool"
        )
        || payload["stream"].as_str() == Some("tool")
        || payload.get("tool").is_some()
        || payload.get("toolName").is_some()
        || payload.get("tool_name").is_some()
        || has_tool_calls;

    let mut tool_name = string(payload, &["tool", "toolName", "tool_name"]).to_string();
    if tool_name.is_empty() {
        if let Some(tool_obj) = payload.get("tool").filter(|v| v.is_object()) {
            tool_name = string(tool_obj, &["name", "toolName", "id"]).to_string();
        }
    }
    if tool_name.is_empty() && is_tool_event {
        tool_name = string(payload, &["name"]).to_string();
    }
    if tool_name.is_empty() {
        if let Some(d) = data {
            tool_name = string(d, &["tool", "toolName", "tool_name", "name"]).to_string();
        }
    }
    if tool_name.is_empty() {
        if let Some(m) = metadata {
            tool_name = string(m, &["tool", "toolName", "tool_name", "name"]).to_string();
        }
    }
    if tool_name.is_empty() {
        if let Some(tools) = payload.get("toolCalls").and_then(Value::as_array) {
            tool_name = tools
                .iter()
                .filter_map(|t| t["name"].as_str().filter(|s| !s.is_empty()))
                .map(friendly_tool_label)
                .collect::<Vec<_>>()
                .join(" · ");
        }
    }

    let tool_status = if !tool_name.is_empty() {
        let friendly_name = tool_name
            .split(" · ")
            .map(friendly_tool_label)
            .collect::<Vec<_>>()
            .join(" · ");
        Some(format!("正在调用工具：{friendly_name}"))
    } else if is_tool_event && !finished {
        let status_label = string(payload, &["status", "phase"]);
        if !status_label.is_empty()
            && !matches!(
                status_label.to_ascii_lowercase().as_str(),
                "tool_use" | "tool" | "running" | "started"
            )
        {
            Some(format!(
                "正在调用工具：{}",
                friendly_tool_label(status_label)
            ))
        } else {
            Some("正在调用工具…".to_string())
        }
    } else {
        None
    };

    if delta_text.is_empty()
        && full_text.is_empty()
        && thinking_text.is_empty()
        && tool_status.is_none()
        && !finished
    {
        return None;
    }

    Some(StreamDelta {
        session_key,
        run_id,
        delta_text,
        full_text,
        thinking_text,
        tool_status,
        finished,
    })
}

const CLOUD_TIMEOUT_MESSAGE: &str =
    "Amoo 云连接已中断或超时；请检查手机网络及小工具中的伴侣后台协作状态，未确认发送的草稿与附件已保留";

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct StepclawStatus {
    pub chat_status: String,
    pub chat_reason: String,
    pub upstream_status: String,
    pub upstream_reason: String,
}

impl StepclawStatus {
    pub(crate) fn is_upstream_screen_off(&self) -> bool {
        self.upstream_status == "unavailable" && self.upstream_reason == "screen_off"
    }

    pub(crate) fn is_chat_unavailable(&self) -> bool {
        self.chat_status == "unavailable" || self.upstream_status == "unavailable"
    }

    pub(crate) fn is_chat_available(&self) -> bool {
        self.chat_status == "available" && self.upstream_status != "unavailable"
    }
}

pub(crate) fn parse_stepclaw_status_change(frame: &Value) -> Option<StepclawStatus> {
    let payload = match frame.get("type").and_then(Value::as_str) {
        Some("req")
            if frame.get("method").and_then(Value::as_str) == Some("node.event")
                && frame
                    .get("params")
                    .and_then(|p| p.get("event"))
                    .and_then(Value::as_str)
                    == Some("stepclaw.status.change") =>
        {
            frame.get("params")?.get("payload")?
        }
        Some("event")
            if frame.get("event").and_then(Value::as_str) == Some("stepclaw.status.change") =>
        {
            frame.get("payload")?
        }
        _ => return None,
    };
    let resources = payload.get("resources")?;
    Some(StepclawStatus {
        chat_status: string(&resources["chat"], &["status"]).to_string(),
        chat_reason: string(&resources["chat"], &["reason"]).to_string(),
        upstream_status: string(&resources["upstream"], &["status"]).to_string(),
        upstream_reason: string(&resources["upstream"], &["reason"]).to_string(),
    })
}

pub struct AgentBus {
    _lease: Option<crate::amoo_background::Lease>,
    stream: TcpStream,
    adb: Adb,
    device: Device,
    port: u16,
    task: bool,
    stream_tainted: bool,
    has_completed_rpc: bool,
    last_status: Option<StepclawStatus>,
    pub version: String,
    pending_events: Vec<StreamDelta>,
}

fn write_all_clean(stream: &mut TcpStream, mut bytes: &[u8], err_msg: &str) -> Result<()> {
    let deadline = Instant::now() + Duration::from_secs(5);
    while !bytes.is_empty() {
        match stream.write(bytes) {
            Ok(0) => bail!("{err_msg}"),
            Ok(n) => bytes = &bytes[n..],
            Err(err) if err.kind() == ErrorKind::Interrupted => continue,
            Err(err) if matches!(err.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                if Instant::now() >= deadline {
                    bail!("{err_msg}");
                }
                std::thread::sleep(Duration::from_millis(5));
            }
            Err(_) => bail!("{err_msg}"),
        }
    }
    Ok(())
}

fn write_frame(stream: &mut TcpStream, body: &[u8], err_msg: &str) -> Result<()> {
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(body);
    write_all_clean(stream, &frame, err_msg)
}

fn read_exact_until(stream: &mut TcpStream, bytes: &mut [u8], deadline: Instant) -> Result<()> {
    read_exact_until_with_progress(stream, bytes, deadline, &mut false)
}

fn read_exact_until_with_progress(
    stream: &mut TcpStream,
    mut bytes: &mut [u8],
    deadline: Instant,
    received_bytes: &mut bool,
) -> Result<()> {
    while !bytes.is_empty() {
        let Some(remaining) = deadline.checked_duration_since(Instant::now()) else {
            bail!("{CLOUD_TIMEOUT_MESSAGE}");
        };
        if stream
            .set_read_timeout(Some(
                remaining
                    .min(Duration::from_millis(250))
                    .max(Duration::from_millis(1)),
            ))
            .is_err()
        {
            bail!("{CLOUD_TIMEOUT_MESSAGE}");
        }
        match stream.read(bytes) {
            Ok(0) => bail!("Amoo 后台连接已关闭"),
            Ok(count) => {
                *received_bytes = true;
                bytes = &mut bytes[count..];
            }
            Err(err)
                if matches!(
                    err.kind(),
                    ErrorKind::WouldBlock | ErrorKind::TimedOut | ErrorKind::Interrupted
                ) =>
            {
                if Instant::now() >= deadline {
                    bail!("{CLOUD_TIMEOUT_MESSAGE}");
                }
            }
            Err(_) => bail!("{CLOUD_TIMEOUT_MESSAGE}"),
        }
    }
    Ok(())
}

fn read_frame(stream: &mut TcpStream, deadline: Instant) -> Result<Value> {
    read_frame_with_progress(stream, deadline, &mut false)
}

fn read_frame_with_progress(
    stream: &mut TcpStream,
    deadline: Instant,
    received_bytes: &mut bool,
) -> Result<Value> {
    let mut header = [0; 4];
    read_exact_until_with_progress(stream, &mut header, deadline, received_bytes)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > MAX_FRAME {
        bail!("Amoo 后台返回了无效的数据帧长度");
    }
    let mut body = vec![0; size];
    read_exact_until_with_progress(stream, &mut body, deadline, received_bytes)?;
    serde_json::from_slice(&body).context("Amoo 后台返回了无效的 JSON 数据")
}

fn local_image_upload_frame(id: &str, paths: &[String]) -> Result<Value> {
    if paths.is_empty() || paths.len() > 8 {
        bail!("每次图片上传需要 1 至 8 张图片");
    }
    let mut unique = HashSet::new();
    for path in paths {
        let relative = path
            .strip_prefix("/sdcard/Download/Amoo Work/")
            .context("图片必须位于本次 StepX 附件目录")?;
        let (directory, name) = relative.split_once('/').context("图片路径无效")?;
        if directory.len() != 32
            || !directory.bytes().all(|b| b.is_ascii_hexdigit())
            || name.is_empty()
            || name == "."
            || name == ".."
            || name.contains('/')
            || path.chars().any(char::is_control)
            || !crate::amoo_media::mime_type(std::path::Path::new(name)).starts_with("image/")
            || !unique.insert(path)
        {
            bail!("图片路径不在已选择的 StepX 附件范围内");
        }
    }
    let method = "batch_file_upload_return_all";
    // Matches the phone's r4.a ability adapter: params travel in invokeParamsJson.
    // The local registry normally owns routing. Private data classification is
    // enforced by the native GatewaySelector too: no CLOUD fallback if the local
    // capability disappears or a static route takes precedence.
    Ok(
        json!({"type":"event","event":"node.invoke.request","payload":{
            "id":id,"command":"ability.call","idempotencyKey":id,"timeoutMs":110000,
            "_meta":{"dataClass":"private"},
            "params":{"method":method,"timeoutMs":110000,
                "invokeParamsJson":serde_json::to_string(&json!({"method":method,
                    "params":{"files":serde_json::to_string(paths)?}}))?}
        }}),
    )
}

fn update_deadline_for_status(
    task: bool,
    has_lease: bool,
    timeout: Duration,
    call_started: Instant,
    status: &StepclawStatus,
    deadline: &mut Instant,
    extended_for_wake: &mut bool,
    extended_on_available: &mut bool,
) {
    if !task {
        if status.is_upstream_screen_off() {
            *deadline = (*deadline).min(Instant::now() + Duration::from_millis(1200));
        } else {
            *deadline = (*deadline).max(call_started + timeout);
        }
        return;
    }
    if status.is_upstream_screen_off() && !has_lease {
        *deadline = (*deadline).min(Instant::now() + Duration::from_secs(3));
        return;
    }
    *deadline = (*deadline).max(call_started + timeout);
    if status.is_chat_unavailable() {
        if timeout >= REQUEST_TIMEOUT {
            let hard_cap = call_started + Duration::from_secs(90);
            if status.is_upstream_screen_off() {
                if !*extended_for_wake {
                    *extended_for_wake = true;
                    *deadline = (*deadline)
                        .max(Instant::now() + Duration::from_secs(35))
                        .min(hard_cap);
                }
            } else {
                // Companion wake has transitioned upstream out of screen_off (e.g.
                // into "cloud claw offline" while openclaw-cloud mTLS reconnects).
                // Refresh the wake window even if an earlier screen_off event
                // already set extended_for_wake.
                *extended_for_wake = true;
                *deadline = (*deadline)
                    .max(Instant::now() + Duration::from_secs(45))
                    .min(hard_cap);
            }
        }
    } else if status.is_chat_available() {
        if *extended_for_wake && !*extended_on_available {
            *extended_on_available = true;
            *deadline = (*deadline).max(Instant::now() + timeout);
        }
    }
}

fn hello_request_params() -> Value {
    json!({"minProtocol":3,"maxProtocol":3,"role":"channel",
    "client":{"id":format!("stepx-desktop-{:016x}",rand::random::<u64>()),
        "displayName":"StepX Desktop","version":"1.0.0","platform":"linux"}})
}

impl AgentBus {
    pub fn connect(adb: &Adb, device: &Device) -> Result<Self> {
        Self::connect_mode(adb, device, true)
    }

    /// Background observation must not wake or extend the phone's task window.
    pub fn observe(adb: &Adb, device: &Device) -> Result<Self> {
        Self::connect_mode(adb, device, false)
    }

    fn connect_mode(adb: &Adb, device: &Device, task: bool) -> Result<Self> {
        let mut device = device.clone();
        device.transport = device.resolve_active_transport(adb)?;
        let device = &device;
        let lease = if task {
            crate::amoo_background::Lease::acquire(adb, device)?
        } else {
            None
        };
        let (stream, port, was_cached) = adb
            .connect_forwarded_tcp(
                &device.transport,
                "localabstract:agentbus",
                Duration::from_secs(3),
            )
            .context("无法连接 Amoo 后台服务，请检查手机连接")?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut bus = Self {
            _lease: lease,
            stream,
            adb: adb.clone(),
            device: device.clone(),
            port,
            task,
            stream_tainted: false,
            has_completed_rpc: false,
            last_status: None,
            version: String::new(),
            pending_events: Vec::new(),
        };
        bus.perform_handshake(was_cached)?;
        if let Some(lease) = bus._lease.as_ref() {
            // This acknowledges only the real protocol handshake. Task completion
            // continues to require its matching native reply and final run state.
            let _ = lease.mark_connected();
        }
        Ok(bus)
    }

    fn perform_handshake(&mut self, was_cached: bool) -> Result<()> {
        let hello_timeout = if self.task {
            REQUEST_TIMEOUT
        } else {
            Duration::from_secs(3)
        };
        let first_hello_timeout = if was_cached {
            hello_timeout.min(Duration::from_secs(3))
        } else {
            hello_timeout
        };
        let hello =
            match self.call_with_timeout("connect", hello_request_params(), first_hello_timeout) {
                Ok(h) => h,
                Err(_) if was_cached => {
                    self.adb.invalidate_forwarded_tcp(
                        &self.device.transport,
                        "localabstract:agentbus",
                        Some(self.port),
                    );
                    if let Ok(active) = self.device.resolve_active_transport(&self.adb) {
                        if active != self.device.transport {
                            self.adb
                                .invalidate_forwards_for_transport(&self.device.transport);
                            self.device.transport = active;
                        }
                    }
                    let (fresh_stream, fresh_port, _) = self
                        .adb
                        .connect_forwarded_tcp(
                            &self.device.transport,
                            "localabstract:agentbus",
                            Duration::from_secs(3),
                        )
                        .context("无法连接 Amoo 后台服务，请检查手机连接")?;
                    fresh_stream.set_write_timeout(Some(Duration::from_secs(5)))?;
                    self.port = fresh_port;
                    self.stream = fresh_stream;
                    self.stream_tainted = false;
                    self.has_completed_rpc = false;
                    self.last_status = None;
                    self.pending_events.clear();
                    self.call_with_timeout("connect", hello_request_params(), hello_timeout)
                        .map_err(|e| {
                            self.adb.invalidate_forwarded_tcp(
                                &self.device.transport,
                                "localabstract:agentbus",
                                Some(self.port),
                            );
                            e
                        })
                        .with_context(|| {
                            crate::amoo_background::connection_failure_message(
                                &self.adb,
                                &self.device,
                            )
                        })?
                }
                Err(e) => {
                    self.adb.invalidate_forwarded_tcp(
                        &self.device.transport,
                        "localabstract:agentbus",
                        Some(self.port),
                    );
                    return Err(e).with_context(|| {
                        crate::amoo_background::connection_failure_message(&self.adb, &self.device)
                    });
                }
            };
        if hello["protocol"] != 3 {
            self.adb.invalidate_forwarded_tcp(
                &self.device.transport,
                "localabstract:agentbus",
                Some(self.port),
            );
            bail!("当前 Amoo 后台协议不受支持");
        }
        self.version = hello["server"]["version"].as_str().unwrap_or("").into();
        self.stream_tainted = false;
        self.has_completed_rpc = false;
        Ok(())
    }

    fn reconnect_stream(&mut self) -> Result<()> {
        if let Ok(active) = self.device.resolve_active_transport(&self.adb) {
            if active != self.device.transport {
                self.adb
                    .invalidate_forwards_for_transport(&self.device.transport);
                self.device.transport = active;
            }
        }
        let (stream, port, was_cached) = self
            .adb
            .connect_forwarded_tcp(
                &self.device.transport,
                "localabstract:agentbus",
                Duration::from_secs(3),
            )
            .context("无法连接 Amoo 后台服务，请检查手机连接")?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        self.stream = stream;
        self.port = port;
        self.stream_tainted = false;
        self.has_completed_rpc = false;
        self.last_status = None;
        self.pending_events.clear();
        self.perform_handshake(was_cached)
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.call_with_timeout(method, params, REQUEST_TIMEOUT)
    }

    /// The native bus routes this event to its own registered ability node.
    /// Keep the API limited to selected images already staged by StepX; it is not
    /// a general command transport or a cloud-account node selector.
    pub fn upload_local_images(&mut self, paths: &[String]) -> Result<Value> {
        if self.stream_tainted {
            self.reconnect_stream()?;
        }
        let id = format!("stepx-upload-{:032x}", rand::random::<u128>());
        let body = serde_json::to_vec(&local_image_upload_frame(&id, paths)?)?;
        if body.len() > MAX_FRAME {
            bail!("图片上传请求过大");
        }
        if let Err(err) = write_frame(
            &mut self.stream,
            &body,
            "手机本机图片上传未确认完成；附件已保留，请勿重复提交",
        ) {
            self.stream_tainted = true;
            return Err(err);
        }
        let deadline = Instant::now() + Duration::from_secs(115);
        loop {
            let response = match read_frame(&mut self.stream, deadline) {
                Ok(frame) => frame,
                Err(err) => {
                    self.stream_tainted = true;
                    return Err(err)
                        .context("手机本机图片上传未确认完成；附件已保留，请勿重复提交");
                }
            };
            if let Some(status) = parse_stepclaw_status_change(&response) {
                self.last_status = Some(status);
            }
            if let Some(delta) = parse_stream_event(&response) {
                self.pending_events.push(delta);
            }
            if response["type"] != "req"
                || response["method"] != "node.invoke.result"
                || response["params"]["id"].as_str() != Some(&id)
            {
                continue;
            }
            let result = response
                .get("params")
                .filter(|value| value.is_object())
                .context("手机图片上传结果格式无效")?;
            if result["ok"] != true {
                let code = result["error"]["code"]
                    .as_str()
                    .filter(|code| {
                        code.len() <= 80
                            && code.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
                    })
                    .unwrap_or("UNKNOWN");
                bail!("手机本机图片上传失败（{code}），图片尚未交给 Amoo");
            }
            self.has_completed_rpc = true;
            return Ok(result.clone());
        }
    }

    /// Read-only discovery uses the same native registry as local image upload.
    /// This does not expose a general-purpose ability call to the desktop UI.
    pub(crate) fn local_gui_catalog(&mut self) -> Result<Value> {
        let id = format!("stepx-gui-probe-{:032x}", rand::random::<u128>());
        let frame = json!({"type":"event","event":"node.invoke.request","payload":{
            "id":id,"command":"ability.list","idempotencyKey":id,"timeoutMs":8000,
            "_meta":{"dataClass":"private"},"params":{}
        }});
        let response =
            self.local_gui_exchange(id, frame, Duration::from_secs(8), &AtomicBool::new(false))?;
        match response.end {
            LocalInvokeEnd::Reply(value) => Ok(value),
            _ => bail!("手机屏幕操作能力暂未返回，尚未执行任何操作"),
        }
    }

    /// Direct native ability invocation. Unlike cloud RPC this must never replay
    /// after a partial write, lost reply or timeout: the phone may have executed it.
    pub(crate) fn invoke_local_ability(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<LocalInvokeResult> {
        let id = format!("stepx-native-{:032x}", rand::random::<u128>());
        let timeout_ms = timeout.as_millis().min(600_000) as u64;
        let frame = json!({"type":"event","event":"node.invoke.request","payload":{
            "id":id,"command":"ability.call","idempotencyKey":id,"timeoutMs":timeout_ms,
            "_meta":{"dataClass":"private"},"params":{"method":method,"timeoutMs":timeout_ms,
            "invokeParamsJson":serde_json::to_string(&json!({"method":method,"params":params}))?}
        }});
        self.local_gui_exchange(id, frame, timeout, cancel)
    }

    pub(crate) fn invoke_local_gui(
        &mut self,
        app_name: &str,
        task: &str,
        cancel: &AtomicBool,
    ) -> Result<LocalInvokeResult> {
        let id = format!("stepx-gui-{:032x}", rand::random::<u128>());
        let frame = local_gui_frame(&id, app_name, task)?;
        self.local_gui_exchange(id, frame, Duration::from_secs(600), cancel)
    }

    fn local_gui_exchange(
        &mut self,
        id: String,
        frame: Value,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<LocalInvokeResult> {
        let body = serde_json::to_vec(&frame)?;
        if body.len() > MAX_FRAME {
            bail!("手机屏幕操作请求过大");
        }
        if cancel.load(Ordering::Relaxed) {
            return Ok(LocalInvokeResult {
                id,
                end: LocalInvokeEnd::WaitCancelled,
            });
        }
        if self.stream_tainted && self.reconnect_stream().is_err() {
            return Ok(LocalInvokeResult {
                id,
                end: LocalInvokeEnd::Disconnected,
            });
        }
        let deadline = Instant::now() + timeout;
        // A partial write is ambiguous: never classify it as safe to resubmit.
        if write_frame(&mut self.stream, &body, "手机屏幕操作连接已断开").is_err() {
            self.stream_tainted = true;
            return Ok(LocalInvokeResult {
                id,
                end: LocalInvokeEnd::Disconnected,
            });
        }
        loop {
            match read_local_frame(&mut self.stream, deadline, cancel) {
                LocalInvokeEnd::Reply(response) => {
                    if let Some(status) = parse_stepclaw_status_change(&response) {
                        self.last_status = Some(status);
                    }
                    if let Some(delta) = parse_stream_event(&response) {
                        self.pending_events.push(delta);
                    }
                    if response["type"] == "req"
                        && response["method"] == "node.invoke.result"
                        && response["params"]["id"].as_str() == Some(&id)
                    {
                        self.has_completed_rpc = true;
                        return Ok(LocalInvokeResult {
                            id,
                            end: LocalInvokeEnd::Reply(response["params"].clone()),
                        });
                    }
                }
                end => {
                    self.stream_tainted = true;
                    return Ok(LocalInvokeResult { id, end });
                }
            }
        }
    }

    /// Capability probes use a short absolute deadline even when events arrive.
    pub fn call_with_timeout(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
    ) -> Result<Value> {
        if method != "connect" && self.stream_tainted {
            self.reconnect_stream()?;
        }
        let can_retry_stale_warm_stream = method != "connect" && self.has_completed_rpc;
        let mut retried_warm_stream = false;
        loop {
            // AgentBus multiplexes clients upstream; IDs must remain unique across reconnects.
            let id = format!("stepx-{:032x}", rand::random::<u128>());
            let body =
                serde_json::to_vec(&json!({"type":"req","id":id,"method":method,"params":params}))?;
            if body.len() > MAX_FRAME {
                bail!("Amoo 请求过大");
            }
            if let Err(err) = write_frame(&mut self.stream, &body, CLOUD_TIMEOUT_MESSAGE) {
                self.stream_tainted = true;
                if method != "connect" && self.port != 0 {
                    self.adb.invalidate_forwarded_tcp(
                        &self.device.transport,
                        "localabstract:agentbus",
                        Some(self.port),
                    );
                }
                if can_retry_stale_warm_stream && !retried_warm_stream {
                    retried_warm_stream = true;
                    if self.reconnect_stream().is_ok() {
                        continue;
                    }
                }
                return Err(err);
            }
            let call_started = Instant::now();
            let mut deadline = call_started + timeout;
            let has_lease = self._lease.is_some();
            let mut extended_for_wake = false;
            let mut extended_on_available = false;
            if method != "connect" {
                if let Some(ref status) = self.last_status {
                    update_deadline_for_status(
                        self.task,
                        has_lease,
                        timeout,
                        call_started,
                        status,
                        &mut deadline,
                        &mut extended_for_wake,
                        &mut extended_on_available,
                    );
                }
            }
            let mut received_response_bytes = false;
            let result = loop {
                let response = match read_frame_with_progress(
                    &mut self.stream,
                    deadline,
                    &mut received_response_bytes,
                ) {
                    Ok(frame) => frame,
                    Err(err) => {
                        self.stream_tainted = true;
                        let is_eof = err.to_string().contains("连接已关闭");
                        if method != "connect" && self.port != 0 && is_eof {
                            self.adb.invalidate_forwarded_tcp(
                                &self.device.transport,
                                "localabstract:agentbus",
                                Some(self.port),
                            );
                        }
                        // A partial header or body is already a response. Its lost
                        // remainder does not prove that replaying the RPC is safe.
                        break Err((err, is_eof && !received_response_bytes));
                    }
                };
                if let Some(status) = parse_stepclaw_status_change(&response) {
                    if method != "connect" {
                        update_deadline_for_status(
                            self.task,
                            has_lease,
                            timeout,
                            call_started,
                            &status,
                            &mut deadline,
                            &mut extended_for_wake,
                            &mut extended_on_available,
                        );
                    }
                    self.last_status = Some(status);
                }
                if let Some(delta) = parse_stream_event(&response) {
                    self.pending_events.push(delta);
                }
                if response["id"].as_str() != Some(&id) {
                    continue;
                }
                if response["ok"].as_bool() != Some(true) {
                    let code = response["error"]["code"].as_str().unwrap_or("UNKNOWN");
                    let message = response["error"]["message"]
                        .as_str()
                        .unwrap_or("请求未被接受");
                    break Err((
                        anyhow::anyhow!("Amoo 后台请求失败（{code}）：{message}"),
                        false,
                    ));
                }
                if response["payload"]["ok"].as_bool() == Some(false) {
                    break Err((anyhow::anyhow!("Amoo 后台未完成请求"), false));
                }
                let payload = response
                    .get("payload")
                    .cloned()
                    .context("Amoo 后台响应缺少 payload");
                break match payload {
                    Ok(val) => Ok(val),
                    Err(err) => Err((err, false)),
                };
            };
            match result {
                Ok(val) => {
                    if method != "connect" {
                        self.has_completed_rpc = true;
                    }
                    return Ok(val);
                }
                Err((err, immediate_eof)) => {
                    if immediate_eof && can_retry_stale_warm_stream && !retried_warm_stream {
                        retried_warm_stream = true;
                        if self.reconnect_stream().is_ok() {
                            continue;
                        }
                    }
                    return Err(err);
                }
            }
        }
    }

    pub fn poll_stream_events(&mut self, wait: Duration) -> Result<Vec<StreamDelta>> {
        let mut out = std::mem::take(&mut self.pending_events);
        match poll_stream_events_and_status_from_stream(&mut self.stream, wait, !out.is_empty()) {
            Ok((polled, latest_status, saw_eof)) => {
                if let Some(status) = latest_status {
                    self.last_status = Some(status);
                }
                if saw_eof {
                    self.stream_tainted = true;
                }
                out.extend(polled);
                Ok(out)
            }
            Err(err) => {
                self.stream_tainted = true;
                Err(err)
            }
        }
    }

    /// Read-only background collection uses bounded, interruptible waits. Drop
    /// this connection after an error: cancellation may interrupt a frame.
    pub(crate) fn call_cancellable(
        &mut self,
        method: &str,
        params: Value,
        timeout: Duration,
        cancel: &AtomicBool,
    ) -> Result<Value> {
        if cancel.load(Ordering::Relaxed) {
            bail!("已停止读取手机上下文");
        }
        if self.stream_tainted {
            self.reconnect_stream()?;
        }
        let id = format!("stepx-context-{:032x}", rand::random::<u128>());
        let body =
            serde_json::to_vec(&json!({"type":"req","id":id,"method":method,"params":params}))?;
        if body.len() > MAX_FRAME {
            bail!("手机上下文读取请求过大");
        }
        let call_started = Instant::now();
        let mut deadline = call_started + timeout;
        if !self.task
            && self
                .last_status
                .as_ref()
                .is_some_and(StepclawStatus::is_upstream_screen_off)
        {
            deadline = deadline.min(Instant::now() + Duration::from_millis(800));
        }
        self.stream.set_write_timeout(Some(timeout))?;
        if let Err(err) = write_frame(&mut self.stream, &body, "手机上下文连接已断开") {
            self.stream_tainted = true;
            return Err(err);
        }
        loop {
            let response = match read_local_frame(&mut self.stream, deadline, cancel) {
                LocalInvokeEnd::Reply(value) => value,
                LocalInvokeEnd::WaitCancelled => {
                    self.stream_tainted = true;
                    bail!("已停止读取手机上下文");
                }
                LocalInvokeEnd::TimedOut => {
                    self.stream_tainted = true;
                    bail!("读取手机上下文超时");
                }
                LocalInvokeEnd::Disconnected => {
                    self.stream_tainted = true;
                    bail!("手机上下文连接已断开");
                }
            };
            if let Some(status) = parse_stepclaw_status_change(&response) {
                if !self.task {
                    if status.is_upstream_screen_off() {
                        deadline = deadline.min(Instant::now() + Duration::from_millis(800));
                    } else {
                        deadline = deadline.max(call_started + timeout);
                    }
                }
                self.last_status = Some(status);
            }
            if response["id"].as_str() != Some(&id) {
                // This dedicated read connection does not need chat/tool events.
                continue;
            }
            if response["ok"].as_bool() != Some(true)
                || response["payload"]["ok"].as_bool() == Some(false)
            {
                // Do not expose server error bodies that may contain source text.
                bail!("手机暂未提供这部分上下文");
            }
            self.has_completed_rpc = true;
            return response
                .get("payload")
                .cloned()
                .context("手机上下文响应不完整");
        }
    }

    pub fn snapshot(&mut self, selected: Option<&str>) -> Result<RealAmooData> {
        self.snapshot_with_pending(selected, &[])
    }

    pub fn snapshot_with_pending(
        &mut self,
        selected: Option<&str>,
        pending: &[String],
    ) -> Result<RealAmooData> {
        let snapshot_started = Instant::now();
        let scope_before_res = crate::amoo_usage::account_scope(&self.adb, &self.device);
        let scope_before = scope_before_res.as_ref().ok().cloned().flatten();
        let payload = self.call("sessions.list", json!({"limit":100}))?;
        let rows = payload["sessions"]
            .as_array()
            .context("Amoo 未返回会话列表")?;
        let mut data = parse_sessions(rows)?;
        data.account_scope = scope_before;
        data.gateway_version = self.version.clone();
        data.gateway_online = true;
        let index = match selected {
            Some(key) => data.sessions.iter().position(|s| s.key == key),
            None => {
                if data.sessions.is_empty() {
                    None
                } else {
                    Some(0)
                }
            }
        };
        data.selected_session = selected
            .map(str::to_string)
            .or_else(|| index.map(|i| data.sessions[i].key.clone()));
        for (i, session) in data.sessions.iter_mut().enumerate() {
            if Some(i) != index && !pending.contains(&session.key) {
                continue;
            }
            let history =
                self.call("chat.history", json!({"sessionId":session.key,"limit":100}))?;
            session.messages = parse_messages(
                history["messages"]
                    .as_array()
                    .context("Amoo 未返回会话历史")?,
            )?;
            for message in &mut session.messages {
                message.usage_session_key = session.key.clone();
            }
            let rows = history["messages"]
                .as_array()
                .context("Amoo 未返回会话历史")?;
            let mut seen = HashSet::from([session.key.clone()]);
            let mut budget = 12usize;
            self.merge_child_tasks(
                rows,
                None,
                &mut session.messages,
                &mut seen,
                &mut budget,
                0,
                Instant::now() + Duration::from_secs(12),
            );
            session.messages.sort_by_key(|message| message.timestamp_ms);
            let now_ms = chrono::Utc::now().timestamp_millis();
            let (status, status_kind) = status_from_session_and_history(session, now_ms);
            session.status = status;
            session.status_kind = status_kind;
            // Report the last real parent-session inference, not a catalog default
            // or the model used by an unrelated child task.
            session.model = recent_session_model(session)
                .unwrap_or_default()
                .to_string();
            let more = history["page"]["hasMore"].as_bool().unwrap_or(false);
            session.meta = format!(
                "后台同步 · {} 条消息{}",
                session.messages.len(),
                if more { " · 部分历史" } else { "" }
            );
            session.history_loaded = true;
        }
        let scope_after =
            if scope_before_res.is_ok() && snapshot_started.elapsed() < Duration::from_secs(5) {
                data.account_scope.clone()
            } else {
                crate::amoo_usage::account_scope(&self.adb, &self.device)
                    .ok()
                    .flatten()
            };
        if data.account_scope != scope_after {
            bail!("同步期间手机 Amoo 账号已改变，请重新同步；未合并不同账号的记录");
        }
        Ok(data)
    }

    /// A task-create acknowledgement is not a completed deliverable. Follow the
    /// exact child keys returned by this conversation, never unrelated sessions.
    fn merge_child_tasks(
        &mut self,
        rows: &[Value],
        parent_run: Option<&str>,
        messages: &mut Vec<RealChatMessage>,
        seen: &mut HashSet<String>,
        budget: &mut usize,
        depth: usize,
        deadline: Instant,
    ) {
        for task in child_tasks(rows) {
            let run = parent_run.unwrap_or(&task.run_id).to_string();
            if seen.contains(&task.key) {
                continue;
            }
            if depth >= 3 || *budget == 0 || Instant::now() >= deadline {
                messages.push(RealChatMessage {
                    message_id: format!("child-limited:{}",task.key), run_id:run,
                    role:"assistant".into(), stop_reason:"tracking_limit".into(), is_error:true,
                    error_message:Some("部分手机子任务尚未确认结果：本次后台同步已达到跟踪上限。已返回的文件已保留，可再次同步或在手机查看该任务。".into()),
                    timestamp_ms:task.timestamp, ..Default::default()
                });
                continue;
            }
            let mut waiting = true;
            let mut read_confirmed = false;
            if depth < 3 && *budget > 0 {
                *budget -= 1;
                seen.insert(task.key.clone());
                if let Ok(history) = self.call_with_timeout(
                    "chat.history",
                    json!({"sessionId":task.key,"limit":100}),
                    deadline
                        .saturating_duration_since(Instant::now())
                        .min(Duration::from_secs(8)),
                ) {
                    if let Some(child_rows) = history["messages"].as_array() {
                        if let Ok(child_messages) = parse_messages(child_rows) {
                            read_confirmed = true;
                            // A verified image/file result is already deliverable even
                            // while the phone finishes its post-processing narration.
                            waiting = !child_messages.iter().any(|message| {
                                message.role == "assistant"
                                    && (is_terminal(&message.stop_reason)
                                        || !message.attachments.is_empty())
                            });
                            for mut message in
                                child_messages.into_iter().filter(|m| m.role == "assistant")
                            {
                                message.usage_session_key = task.key.clone();
                                message.message_id =
                                    format!("child:{}:{}", task.key, message.message_id);
                                message.run_id = run.clone();
                                messages.push(message);
                            }
                            self.merge_child_tasks(
                                child_rows,
                                Some(&run),
                                messages,
                                seen,
                                budget,
                                depth + 1,
                                deadline,
                            );
                        }
                    }
                }
            }
            if !read_confirmed {
                messages.push(RealChatMessage {
                    message_id: format!("child-unavailable:{}", task.key), run_id: run,
                    role: "assistant".into(), stop_reason: "tracking_limit".into(), is_error: true,
                    error_message: Some("暂时未能读取手机子任务的结果，无法确认是否完成。已保留会话；可再次同步查看，避免重复提交同一任务。".into()),
                    timestamp_ms: task.timestamp, ..Default::default()
                });
            } else if waiting {
                messages.push(RealChatMessage {
                    message_id: format!("child-pending:{}", task.key),
                    run_id: run,
                    role: "assistant".into(),
                    stop_reason: "child_running".into(),
                    tools_summary: "手机子任务仍在处理，正在同步实际结果".into(),
                    timestamp_ms: task.timestamp,
                    ..Default::default()
                });
            }
        }
    }

    pub fn create_session(&mut self) -> Result<String> {
        // The same protocol as Amoo's createOpenClawSession; never reset an existing key.
        let key = format!("agent:main:desktop-{:032x}", rand::random::<u128>());
        let payload = self.call("sessions.reset", json!({"key":key,"reason":"new"}))?;
        Ok(payload
            .get("key")
            .and_then(Value::as_str)
            .unwrap_or(&key)
            .to_string())
    }

    /// Cloud Amoo and the phone OpenClaw gateway have different session stores.
    /// Never silently patch a gateway session with an unrelated cloud key.
    /// AgentBus 1.0.0 also ACKs sessions.patch for a deliberately nonexistent
    /// model while inference still returns the default model. An ACK is not
    /// evidence of an applied override (verify_amoo_model_selection reproduces it).
    pub fn set_session_model(&mut self, _key: &str, _model: &str) -> Result<AppliedModel> {
        bail!("当前 Amoo 云会话未开放可验证的模型切换接口；请使用手机自动模型。手机 OpenClaw 模型目录与云会话独立")
    }

    pub fn send(&mut self, key: &str, text: &str) -> Result<PendingReply> {
        self.send_with_attachments(key, text, &[])
    }

    /// Native image attachments reference uploaded HTTPS images in `blob`.
    /// Inline base64 `content` is not consumed by this phone's AgentBus protocol.
    pub fn send_with_attachments(
        &mut self,
        key: &str,
        text: &str,
        attachments: &[Value],
    ) -> Result<PendingReply> {
        self.send_inner(key, text, attachments, true)
    }

    /// GUI observations already contain their bounded context. Hardware words
    /// inside a screenshot or goal must not add unrelated device identifiers.
    pub(crate) fn send_observation(
        &mut self,
        key: &str,
        text: &str,
        attachments: &[Value],
    ) -> Result<PendingReply> {
        self.send_inner(key, text, attachments, false)
    }

    /// Sends desktop-generated text verbatim. Unlike [`Self::send`], this never appends
    /// device identifiers (serial / ASN / CMIIT) even if the text happens to contain
    /// hardware keywords, so aggregated notification context cannot trigger telemetry.
    pub fn send_verbatim(&mut self, key: &str, text: &str) -> Result<PendingReply> {
        self.send_inner(key, text, &[], false)
    }

    fn send_inner(
        &mut self,
        key: &str,
        text: &str,
        attachments: &[Value],
        allow_telemetry: bool,
    ) -> Result<PendingReply> {
        if key.is_empty() || text.trim().is_empty() {
            bail!("请先创建会话并输入消息");
        }
        let mut context = cached_amoo_context(&self.adb, &self.device)?;
        let run_id = format!("stepx-{:032x}", rand::random::<u128>());
        context
            .as_object_mut()
            .context("Amoo 设备信息无效")?
            .remove("ok");
        context["session_id"] = json!(key);
        context["msg_id"] = json!(&run_id);
        context["channelMessageId"] = json!(&run_id);
        context["trace_id"] = json!(format!("tr-{run_id}"));
        let outbound_text = if allow_telemetry && should_inject_device_telemetry(text) {
            enrich_prompt_with_live_device_telemetry(&self.adb, &self.device, text)
        } else {
            text.to_string()
        };
        let payload = self
            .call(
                "chat.send",
                json!({"sessionId":key,"text":outbound_text,"stepclaw":context,
                    "attachments":attachments,"idempotencyKey":run_id}),
            )
            .context("发送结果未能确认，请先刷新会话再决定是否重试；输入内容已保留")?;
        if payload["accepted"].as_bool() == Some(false) {
            bail!("Amoo 未接受此消息，输入内容已保留");
        }
        let accepted_run = string(&payload, &["runId"]);
        if accepted_run.is_empty()
            && payload["accepted"] != true
            && !matches!(
                payload["status"].as_str(),
                Some("started" | "accepted" | "queued")
            )
        {
            bail!("Amoo 未返回有效的发送回执，请先同步会话确认；输入内容已保留");
        }
        Ok(PendingReply {
            key: key.into(),
            text: text.into(),
            run_id: if accepted_run.is_empty() {
                run_id
            } else {
                accepted_run.into()
            },
        })
    }

    pub fn abort(&mut self, session_key: &str, run_id: Option<&str>) -> Result<()> {
        let mut params = json!({
            "sessionKey": session_key,
            "sessionId": session_key,
        });
        if let Some(id) = run_id.filter(|s| !s.is_empty()) {
            params["runId"] = json!(id);
        }
        let _ = self.call_with_timeout("chat.abort", params, Duration::from_secs(3));
        Ok(())
    }
}

#[derive(Clone, Debug)]
pub struct AppliedModel {
    pub session_key: String,
    pub model: String,
    pub provider: String,
    pub verified: bool,
}

#[derive(Clone, Debug)]
pub struct PendingReply {
    pub key: String,
    pub text: String,
    pub run_id: String,
}

impl PendingReply {
    pub fn finished(&self, session: &RealSessionInfo) -> bool {
        if session.key != self.key {
            return false;
        }
        if session
            .messages
            .iter()
            .any(|m| m.run_id == self.run_id && m.stop_reason == "tracking_limit")
        {
            return true;
        }
        if session
            .messages
            .iter()
            .any(|m| m.run_id == self.run_id && m.stop_reason == "child_running")
        {
            return false;
        }
        if session.messages.iter().any(|m| {
            m.role == "assistant" && m.run_id == self.run_id && is_terminal(&m.stop_reason)
        }) {
            return true;
        }
        // Fallback when the service omits runId on the final assistant message:
        // check assistant messages strictly after our user turn.
        let user_pos = session
            .messages
            .iter()
            .rposition(|m| m.role == "user" && m.run_id == self.run_id);
        if let Some(idx) = user_pos {
            return session.messages[idx + 1..].iter().any(|m| {
                m.role == "assistant" && m.run_id.is_empty() && is_terminal(&m.stop_reason)
            });
        }
        false
    }
}

#[cfg(test)]
pub(crate) fn poll_stream_events_from_stream(
    stream: &mut TcpStream,
    wait: Duration,
    already_have_events: bool,
) -> Result<Vec<StreamDelta>> {
    let (deltas, _, _) =
        poll_stream_events_and_status_from_stream(stream, wait, already_have_events)?;
    Ok(deltas)
}

fn poll_stream_events_and_status_from_stream(
    stream: &mut TcpStream,
    wait: Duration,
    already_have_events: bool,
) -> Result<(Vec<StreamDelta>, Option<StepclawStatus>, bool)> {
    let mut out = Vec::new();
    let mut latest_status = None;
    let mut saw_eof = false;
    for _ in 0..16 {
        let first_wait = if out.is_empty() && !already_have_events {
            wait.max(Duration::from_millis(5))
        } else {
            Duration::from_millis(5)
        };
        if stream.set_read_timeout(Some(first_wait)).is_err() {
            bail!("{CLOUD_TIMEOUT_MESSAGE}");
        }
        let mut header = [0u8; 4];
        match stream.read(&mut header[..1]) {
            Ok(0) => {
                if out.is_empty() && !already_have_events {
                    bail!("Amoo 后台连接已关闭");
                }
                saw_eof = true;
                break;
            }
            Ok(1) => {}
            Ok(_) => unreachable!(),
            Err(err)
                if matches!(
                    err.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                break;
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => bail!("{CLOUD_TIMEOUT_MESSAGE}"),
        }
        let frame_deadline = Instant::now() + Duration::from_secs(5);
        read_exact_until(stream, &mut header[1..], frame_deadline)?;
        let size = u32::from_be_bytes(header) as usize;
        if size == 0 || size > MAX_FRAME {
            bail!("Amoo 响应帧长度异常：{size}");
        }
        let mut body = vec![0u8; size];
        read_exact_until(stream, &mut body, frame_deadline)?;
        if let Ok(frame) = serde_json::from_slice::<Value>(&body) {
            if let Some(status) = parse_stepclaw_status_change(&frame) {
                latest_status = Some(status);
            }
            if let Some(delta) = parse_stream_event(&frame) {
                out.push(delta);
            }
        }
    }
    Ok((out, latest_status, saw_eof))
}

pub fn is_terminal(reason: &str) -> bool {
    matches!(
        reason.to_ascii_lowercase().as_str(),
        "stop"
            | "end_turn"
            | "error"
            | "aborted"
            | "cancelled"
            | "length"
            | "tracking_limit"
            | "task_delivered"
            | "completed"
            | "done"
            | "finished"
            | "stop_sequence"
    )
}

pub(crate) fn status_from_session_and_history(
    session: &RealSessionInfo,
    now_ms: i64,
) -> (String, u8) {
    if let Some(last) = session.messages.last() {
        if last.role == "user" || (!last.stop_reason.is_empty() && !is_terminal(&last.stop_reason))
        {
            return ("进行中".into(), 1);
        }
        if last.role != "user" {
            if is_terminal(&last.stop_reason) {
                return ("已完成".into(), 2);
            }
            if last.stop_reason.is_empty()
                && !last.text.trim().is_empty()
                && (last.timestamp_ms <= 0 || now_ms.saturating_sub(last.timestamp_ms) > 15_000)
            {
                return ("已完成".into(), 2);
            }
        }
        if session.status_kind != 1 {
            return ("已完成".into(), 2);
        }
    }
    (session.status.clone(), session.status_kind)
}

struct ChildTask {
    key: String,
    run_id: String,
    timestamp: i64,
}
fn tool_result(tool: &Value) -> Option<Value> {
    let value = tool.get("result")?;
    let inner = value.get("result").unwrap_or(value);
    if let Some(text) = inner.as_str() {
        serde_json::from_str(text).ok()
    } else if inner.is_object() {
        Some(inner.clone())
    } else {
        None
    }
}
/// A successful tool acknowledgement binds inline task output to the exact
/// submitted artifact. Tool arguments alone are never treated as delivered work.
fn submitted_text_results(row: &Value) -> Vec<RealChatMessage> {
    let mut delivered = Vec::new();
    for tool in row["toolCalls"].as_array().into_iter().flatten() {
        if tool["name"] != "task_result_submit" || tool["status"] != "OK" {
            continue;
        }
        let Some(result) = tool_result(tool) else {
            continue;
        };
        let Some(receipts) = result["artifacts"].as_array() else {
            continue;
        };
        let Some(artifacts) = tool["arguments"]["artifacts"].as_array() else {
            continue;
        };
        // Positional correspondence is additionally checked by type, content type
        // and title; reordered or mismatched receipts cannot acknowledge prose.
        let matched = |index: usize, artifact: &Value| {
            receipts.get(index).is_some_and(|receipt| {
                receipt["ok"] == true
                    && receipt["type"] == artifact["type"]
                    && receipt["contentType"] == artifact["contentType"]
                    && receipt["title"] == artifact["title"]
                    && receipt["artifactId"]
                        .as_str()
                        .is_some_and(|id| !id.is_empty())
            })
        };
        let complete = !artifacts.is_empty()
            && artifacts.len() == receipts.len()
            && result["subTaskId"].as_u64() == Some(0)
            && artifacts.iter().enumerate().all(|(i, a)| {
                matched(i, a)
                    && a["type"] == "text"
                    && a["contentType"] == "inline"
                    && a["content"].as_str().is_some_and(|s| !s.trim().is_empty())
            });
        for (i, artifact) in artifacts.iter().enumerate() {
            if !matched(i, artifact)
                || artifact["type"] != "text"
                || artifact["contentType"] != "inline"
            {
                continue;
            }
            let Some(text) = artifact["content"]
                .as_str()
                .filter(|s| !s.trim().is_empty())
            else {
                continue;
            };
            let timestamp = row["timestamp"].as_i64().unwrap_or(0);
            delivered.push(RealChatMessage {
                message_id: format!(
                    "task-result:{}:{}",
                    string(tool, &["id"]),
                    string(&receipts[i], &["artifactId"])
                ),
                run_id: string(row, &["runId"]).into(),
                role: "assistant".into(),
                text: text.into(),
                stop_reason: if complete { "task_delivered" } else { "" }.into(),
                timestamp_ms: timestamp,
                time_label: chrono::DateTime::from_timestamp_millis(timestamp)
                    .map(|d| {
                        d.with_timezone(&chrono::Local)
                            .format("%m-%d %H:%M")
                            .to_string()
                    })
                    .unwrap_or_default(),
                // Token usage belongs to the original tool message, not this view.
                ..Default::default()
            });
        }
    }
    delivered
}

fn child_tasks(rows: &[Value]) -> Vec<ChildTask> {
    let mut tasks = Vec::new();
    let mut seen = HashSet::new();
    for row in rows {
        for tool in row["toolCalls"].as_array().into_iter().flatten() {
            if !matches!(
                tool["name"].as_str(),
                Some("task_create" | "sessions_spawn")
            ) {
                continue;
            }
            let Some(result) = tool_result(tool) else {
                continue;
            };
            let key = string(&result, &["childSessionKey"]);
            if key.is_empty() || !seen.insert(key.to_string()) {
                continue;
            }
            tasks.push(ChildTask {
                key: key.into(),
                run_id: string(row, &["runId"]).into(),
                timestamp: row["timestamp"].as_i64().unwrap_or(0),
            });
        }
    }
    tasks.reverse();
    tasks
}

pub fn fetch(adb: &Adb, device: &Device, selected: Option<&str>) -> Result<RealAmooData> {
    AgentBus::connect(adb, device)?.snapshot(selected)
}

pub fn friendly_tool_label(raw: &str) -> &str {
    match raw.trim() {
        "batch_file_upload_return_all" => "批量上传文件",
        "task_result_submit" => "提交任务成果",
        "sessions_spawn" => "启动协同子智能体",
        "task_create" => "创建拆解任务",
        "text_to_image" => "AI 图像生成",
        "web_search" => "联网搜索",
        "web_fetch" | "fetch_page" => "读取网页",
        "bash" | "exec" | "run_command" => "执行终端命令",
        "read" | "read_file" => "读取文件",
        "write" | "write_file" => "写入文件",
        "edit" | "edit_file" => "编辑文件",
        "browser" => "浏览器操作",
        other => other,
    }
}

fn string<'a>(value: &'a Value, names: &[&str]) -> &'a str {
    names
        .iter()
        .find_map(|name| value[*name].as_str().filter(|s| !s.is_empty()))
        .unwrap_or("")
}

fn parse_sessions(rows: &[Value]) -> Result<RealAmooData> {
    let mut data = RealAmooData {
        default_agent_id: "amoo".into(),
        ..Default::default()
    };
    for row in rows {
        let key = string(row, &["key"]);
        if key.is_empty() {
            bail!("Amoo 会话缺少真实标识，已停止同步");
        }
        let raw_title = string(row, &["title", "displayName", "name"]).trim();
        let title = if raw_title.is_empty() {
            "未命名会话"
        } else if raw_title == "main" || raw_title == "default" || raw_title.starts_with("agent:") {
            "主对话"
        } else {
            raw_title
        };
        let status = string(row, &["status"]);
        let (label, kind) = match status {
            "done" => ("已完成", 2),
            "killed" => ("已停止", 0),
            "idle" => ("空闲", 0),
            "running" | "pending" | "streaming" => ("进行中", 1),
            "error" | "failed" => ("失败", 1),
            _ => (status, 0),
        };
        data.sessions.push(RealSessionInfo {
            key: key.into(),
            session_id: string(row, &["id"]).into(),
            title: title.into(),
            agent: "Amoo 助手".into(),
            status: label.into(),
            status_kind: kind,
            updated_at_ms: row["updatedAtMs"].as_i64().unwrap_or(0),
            meta: "后台会话 · 点击读取历史".into(),
            provider: "Amoo AgentBus".into(),
            ..Default::default()
        });
        if let Some(cron) = row.get("cron").filter(|c| c.is_object()) {
            data.cron_jobs.push(RealCronInfo {
                session_key: key.into(),
                status: string(cron, &["status"]).into(),
                id: string(cron, &["id"]).into(),
                name: string(cron, &["name"]).into(),
                schedule: string(cron, &["scheduleText"]).into(),
                enabled: cron["status"] == "active",
            });
        }
    }
    Ok(data)
}

pub(crate) fn parse_messages(rows: &[Value]) -> Result<Vec<RealChatMessage>> {
    let mut messages = Vec::new();
    for row in rows {
        let role = string(row, &["role"]);
        if !matches!(role, "user" | "assistant") {
            continue;
        }
        let mut text = string(row, &["text", "content"]).to_string();
        if text.is_empty() {
            if let Some(parts) = row["content"].as_array() {
                text = parts
                    .iter()
                    .filter(|p| p["type"] == "text")
                    .filter_map(|p| p["text"].as_str())
                    .collect::<Vec<_>>()
                    .join("\n");
            }
        }
        text = crate::amoo_gui_chat::display_text(&text);
        let error = string(row, &["errorMessage"]);
        let mut attachments = row["attachments"]
            .as_array()
            .map(|items| {
                items
                    .iter()
                    .filter_map(parse_attachment)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        if role == "user" {
            attachments.extend(take_native_attachments(&mut text));
            if let Some((clean, _)) = text.split_once(LIVE_DEVICE_CONTEXT_MARKER) {
                text = clean.trim_end().to_string();
            } else if let Some((clean, _)) =
                text.split_once(LIVE_DEVICE_CONTEXT_MARKER.trim_start())
            {
                text = clean.trim_end().to_string();
            }
        }
        for tool in row["toolCalls"].as_array().into_iter().flatten() {
            if !matches!(
                tool["name"].as_str(),
                Some("text_to_image" | "image_to_image")
            ) {
                continue;
            }
            let Some(result) = tool_result(tool) else {
                continue;
            };
            if result["ok"] != true {
                continue;
            }
            let names = result["artifacts"].as_array().into_iter().flatten();
            let name = names
                .filter(|artifact| artifact["ok"] == true && artifact["type"] == "image")
                .find_map(|artifact| artifact["title"].as_str())
                .unwrap_or("生成图片.png");
            let url = string(&result, &["url", "fileUrl"]).replace("\\u0026", "&");
            if let Some(attachment) =
                parse_attachment(&json!({"type":"image","mime":"image/png","filename":name,
                "url":url,"path":result["path"],"size":result["bytes"],"blob":result["fileId"]}))
            {
                if !attachment.url.is_empty() || !attachment.path.is_empty() {
                    attachments.push(attachment);
                }
            }
        }
        let mut attachment_keys = HashSet::new();
        attachments.retain(|a| {
            let key = if !a.url.is_empty() {
                &a.url
            } else if !a.path.is_empty() {
                &a.path
            } else {
                &a.blob
            };
            key.is_empty() || attachment_keys.insert(key.clone())
        });
        let tool_names = row["toolCalls"]
            .as_array()
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|t| t["name"].as_str())
                    .collect::<Vec<_>>()
                    .join(" · ")
            })
            .unwrap_or_default();
        if text.is_empty()
            && error.is_empty()
            && tool_names.is_empty()
            && attachments.is_empty()
            && string(row, &["stopReason"]).is_empty()
        {
            continue;
        }
        let timestamp = row["timestamp"].as_i64().unwrap_or(0);
        messages.push(RealChatMessage {
            usage: crate::amoo_usage::parse_message(row),
            usage_message_id: string(row, &["id", "responseId"]).into(),
            message_id: string(row, &["id"]).into(),
            run_id: string(row, &["runId"]).into(),
            stop_reason: string(row, &["stopReason"]).into(),
            role: role.into(),
            text,
            attachments,
            tools_summary: tool_names,
            tool_authorization_failures: row["toolCalls"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|tool| {
                    // Inspect the result only; a user quoting an error in tool arguments
                    // is not evidence of an authorization failure.
                    let result = tool["result"].to_string();
                    result.contains("[Security Guard]") && result.contains("[ask_timeout]")
                })
                .filter_map(|tool| tool["name"].as_str().map(str::to_owned))
                .collect(),
            timestamp_ms: timestamp,
            time_label: chrono::DateTime::from_timestamp_millis(timestamp)
                .map(|d| {
                    d.with_timezone(&chrono::Local)
                        .format("%m-%d %H:%M")
                        .to_string()
                })
                .unwrap_or_default(),
            is_error: !error.is_empty()
                || string(row, &["stopReason"]).eq_ignore_ascii_case("error"),
            error_message: if error.is_empty() {
                None
            } else {
                Some(error.into())
            },
            model: string(row, &["responseModel", "model"]).into(),
            provider: string(row, &["provider"]).into(),
            ..Default::default()
        });
        if role == "assistant" {
            messages.extend(submitted_text_results(row));
        }
    }
    Ok(messages)
}

fn recent_session_model(session: &RealSessionInfo) -> Option<&str> {
    session
        .messages
        .iter()
        .rev()
        .find(|message| {
            message.role == "assistant"
                && !message.is_error
                && !message.model.is_empty()
                && message.usage.has_tokens()
                && message.usage_session_key == session.key
        })
        .map(|message| message.model.as_str())
}

fn parse_attachment(value: &Value) -> Option<RealAttachment> {
    if !value.is_object() {
        return None;
    }
    let blob = string(value, &["blob"]);
    let explicit_url = string(value, &["url", "fileUrl"]);
    let candidate = if explicit_url.is_empty() {
        blob
    } else {
        explicit_url
    };
    let url = if (candidate.starts_with("https://") || candidate.starts_with("http://"))
        && !candidate.chars().any(char::is_control)
    {
        candidate
    } else {
        ""
    };
    let path = string(value, &["devicePath", "path"]);
    let name = string(value, &["filename", "fileName", "name"]);
    let kind = string(value, &["type", "kind"]);
    let mime = string(value, &["mime", "mimeType"]);
    if blob.is_empty() && url.is_empty() && path.is_empty() && name.is_empty() {
        return None;
    }
    Some(RealAttachment {
        kind: kind.into(),
        mime: mime.into(),
        name: if name.is_empty() {
            if kind == "image" || mime.starts_with("image/") {
                "图片".into()
            } else {
                "附件".into()
            }
        } else {
            name.chars().filter(|c| !c.is_control()).take(160).collect()
        },
        url: url.into(),
        blob: blob.into(),
        path: path.into(),
        size_bytes: value["size"].as_u64(),
    })
}

fn take_native_attachments(text: &mut String) -> Vec<RealAttachment> {
    let Some(start) = text.find("<extra>") else {
        return vec![];
    };
    let body_start = start + "<extra>".len();
    let Some(end) = text[body_start..]
        .find("</extra>")
        .map(|index| index + body_start)
    else {
        return vec![];
    };
    let Ok(value) = serde_json::from_str::<Value>(&text[body_start..end]) else {
        return vec![];
    };
    let Some(rows) = value["attachments"].as_array() else {
        return vec![];
    };
    let attachments = rows.iter().filter_map(parse_attachment).collect::<Vec<_>>();
    if !attachments.is_empty() {
        text.replace_range(start..end + "</extra>".len(), "");
        *text = text.trim().to_string();
    }
    attachments
}

#[cfg(test)]
mod tests {
    #[test]
    fn actual_response_model_excludes_child_tasks_and_failures() {
        let mut messages = super::parse_messages(&[
            serde_json::json!({"role":"assistant","text":"OK","model":"requested-model","responseModel":"actual-model","usage":{"output_tokens":2}}),
            serde_json::json!({"role":"assistant","text":"child","model":"child-model","usage":{"output_tokens":2}}),
            serde_json::json!({"role":"assistant","text":"error","model":"failed-model","stopReason":"error","usage":{"output_tokens":2}})
        ]).unwrap();
        messages[0].usage_session_key = "parent".into();
        messages[1].usage_session_key = "child".into();
        messages[2].usage_session_key = "parent".into();
        assert_eq!(messages[0].model, "actual-model");
        let session = super::RealSessionInfo {
            key: "parent".into(),
            messages,
            ..Default::default()
        };
        assert_eq!(super::recent_session_model(&session), Some("actual-model"));
    }
    #[test]
    fn authorization_timeout_comes_from_tool_result_not_prompt_or_arguments() {
        let error = "[Security Guard] 用户在 180 秒内未确认授权 [ask_timeout]";
        let messages = super::parse_messages(&[
            serde_json::json!({"role":"assistant","toolCalls":[{"name":"image","result":{"error":error}}]}),
            serde_json::json!({"role":"assistant","text":error,"toolCalls":[{"name":"image","arguments":{"prompt":error},"result":{"ok":true}}]})
        ]).unwrap();
        assert_eq!(messages[0].tool_authorization_failures, vec!["image"]);
        assert!(messages[1].tool_authorization_failures.is_empty());
    }
    use super::*;

    #[test]
    fn gui_invocation_is_fixed_to_native_method_and_private_local_routing() {
        let frame =
            local_gui_frame("gui-test", "设置", "查看 Android 版本，不修改任何设置").unwrap();
        let payload = &frame["payload"];
        assert_eq!(payload["command"], "ability.call");
        assert_eq!(payload["params"]["method"], "gui_agent");
        assert_eq!(payload["_meta"]["dataClass"], "private");
        assert_eq!(payload["timeoutMs"], 600000);
        let args: Value =
            serde_json::from_str(payload["params"]["invokeParamsJson"].as_str().unwrap()).unwrap();
        assert_eq!(args["params"]["gui_agent_param"]["app_name"], "设置");
        assert_eq!(args["params"].as_object().unwrap().len(), 1);
        assert!(local_gui_frame("id", "", "查看版本").is_err());
        assert!(local_gui_frame("id", "设置\u{0}", "查看版本").is_err());
        assert!(local_gui_frame("id", "设置", &"a".repeat(12001)).is_err());
    }

    #[test]
    fn gui_poll_preserves_partial_frames_and_absolute_deadline() {
        use std::{net::TcpListener, thread};
        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(server.local_addr().unwrap()).unwrap();
        let worker = thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            let body = br#"{"type":"req","params":{"ok":true}}"#;
            let header = (body.len() as u32).to_be_bytes();
            stream.write_all(&header[..2]).unwrap();
            thread::sleep(Duration::from_millis(550));
            stream.write_all(&header[2..]).unwrap();
            stream.write_all(body).unwrap();
        });
        let end = read_local_frame(
            &mut client,
            Instant::now() + Duration::from_secs(2),
            &AtomicBool::new(false),
        );
        assert!(matches!(end, LocalInvokeEnd::Reply(value) if value["params"]["ok"] == true));
        worker.join().unwrap();

        let server = TcpListener::bind("127.0.0.1:0").unwrap();
        let mut client = TcpStream::connect(server.local_addr().unwrap()).unwrap();
        let (_stream, _) = server.accept().unwrap();
        assert!(matches!(
            read_local_frame(
                &mut client,
                Instant::now() + Duration::from_millis(30),
                &AtomicBool::new(false)
            ),
            LocalInvokeEnd::TimedOut
        ));
        assert!(matches!(
            read_local_frame(
                &mut client,
                Instant::now() + Duration::from_secs(600),
                &AtomicBool::new(true)
            ),
            LocalInvokeEnd::WaitCancelled
        ));
    }

    #[test]
    fn local_image_upload_is_fixed_to_selected_image_paths() {
        let path = "/sdcard/Download/Amoo Work/0123456789abcdef0123456789abcdef/00-图片 #1%.png"
            .to_string();
        let frame = local_image_upload_frame("test-id", &[path.clone()]).unwrap();
        assert_eq!(frame["type"], "event");
        assert_eq!(frame["event"], "node.invoke.request");
        assert_eq!(frame["payload"]["command"], "ability.call");
        assert_eq!(frame["payload"]["_meta"]["dataClass"], "private");
        assert!(frame["payload"].get("nodeId").is_none());
        let invoke: Value = serde_json::from_str(
            frame["payload"]["params"]["invokeParamsJson"]
                .as_str()
                .unwrap(),
        )
        .unwrap();
        assert_eq!(invoke["method"], "batch_file_upload_return_all");
        assert_eq!(
            serde_json::from_str::<Vec<String>>(invoke["params"]["files"].as_str().unwrap())
                .unwrap(),
            vec![path.clone()]
        );
        assert!(local_image_upload_frame("id", &[]).is_err());
        assert!(local_image_upload_frame("id", &[path.clone(), path.clone()]).is_err());
        for invalid in [
            "/sdcard/private.png".to_string(),
            path.replace("00-图片 #1%.png", "../private.png"),
            path.replace(".png", ".txt"),
            path.replace("0123456789abcdef0123456789abcdef", "invalid"),
        ] {
            assert!(local_image_upload_frame("id", &[invalid]).is_err());
        }
    }
    #[test]
    fn native_uploaded_attachment_becomes_a_card_without_protocol_text() {
        let rows = vec![
            json!({"role":"user","text":"分析这个文件\n<extra>{\"attachments\":[{\"id\":\"\",\"devicePath\":\"/sdcard/Download/Amoo Work/test/sales.csv\",\"filename\":\"sales.csv\",\"mimeType\":\"text/csv\",\"size\":43}]}</extra>"}),
        ];
        let messages = parse_messages(&rows).unwrap();
        assert_eq!(messages[0].text, "分析这个文件");
        assert_eq!(messages[0].attachments[0].name, "sales.csv");
        assert_eq!(messages[0].attachments[0].size_bytes, Some(43));
        assert!(messages[0].attachments[0].url.is_empty());
    }
    #[test]
    fn image_wire_and_history_metadata_render_one_preview() {
        let image = json!({"type":"image","blob":"https://example.test/photo.png","mime":"image/png","filename":"照片.png"});
        let extra = json!({"attachments":[{"id":"","url":"https://example.test/photo.png","devicePath":"/sdcard/test.png","filename":"照片.png","mimeType":"image/png"}]});
        let rows = [
            json!({"role":"user","text":format!("看图\n<extra>{extra}</extra>"),"attachments":[image]}),
        ];
        let messages = parse_messages(&rows).unwrap();
        assert_eq!(messages[0].text, "看图");
        assert_eq!(messages[0].attachments.len(), 1);
        assert_eq!(
            messages[0].attachments[0].url,
            "https://example.test/photo.png"
        );
        assert_eq!(messages[0].attachments[0].name, "照片.png");
    }
    #[test]
    fn child_task_ack_is_not_completion_and_successful_image_result_is_kept() {
        let tasks = child_tasks(&[
            json!({"runId":"run","toolCalls":[{"name":"task_create","result":{"result":"{\"childSessionKey\":\"child\",\"taskId\":\"1\"}"}}]}),
        ]);
        assert_eq!(tasks[0].key, "child");
        assert_eq!(tasks[0].run_id, "run");
        let pending = PendingReply {
            key: "parent".into(),
            run_id: "run".into(),
            text: "生成图片".into(),
        };
        let mut session = RealSessionInfo {
            key: "parent".into(),
            messages: vec![
                RealChatMessage {
                    role: "assistant".into(),
                    run_id: "run".into(),
                    stop_reason: "STOP".into(),
                    ..Default::default()
                },
                RealChatMessage {
                    role: "assistant".into(),
                    run_id: "run".into(),
                    stop_reason: "child_running".into(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert!(!pending.finished(&session));
        session.messages.pop();
        assert!(pending.finished(&session));
        session.messages.push(RealChatMessage {
            role: "assistant".into(),
            run_id: "run".into(),
            stop_reason: "child_running".into(),
            ..Default::default()
        });
        session.messages.push(RealChatMessage {
            role: "assistant".into(),
            run_id: "run".into(),
            stop_reason: "tracking_limit".into(),
            is_error: true,
            ..Default::default()
        });
        assert!(
            pending.finished(&session),
            "tracking limit must end waiting with a visible error, not lock sending forever"
        );
        let result = json!({"ok":true,"url":"https://example.test/boat.png?a=1\\u0026b=2","bytes":678741,"path":"/generated/boat.png","artifacts":[{"ok":true,"type":"image","title":"boat.png"}]}).to_string();
        let image = parse_messages(&[json!({"role":"assistant","runId":"child-run","stopReason":"TOOL_CALLS","toolCalls":[{"name":"text_to_image","result":{"result":result}}]})]).unwrap();
        assert_eq!(
            image[0].attachments[0].url,
            "https://example.test/boat.png?a=1&b=2"
        );
        assert_eq!(image[0].attachments[0].size_bytes, Some(678741));
        assert_eq!(image[0].attachments[0].name, "boat.png");
    }
    fn submitted_fixture() -> Value {
        json!({"id":"m","role":"assistant","runId":"child-run","timestamp":1792808400521i64,
            "stopReason":"TOOL_CALLS","toolCalls":[{"id":"t","name":"task_result_submit","status":"OK",
            "arguments":{"artifacts":[{"type":"text","contentType":"inline","title":"推荐","content":"完整的推荐正文"}]},
            "result":{"result":json!({"taskId":"task1","subTaskId":0,"artifacts":[{"artifactId":"a1","ok":true,"type":"text","contentType":"inline","title":"推荐"}]}).to_string()}}]})
    }
    #[test]
    fn successful_inline_task_result_is_visible_and_finishes_parent() {
        let messages = parse_messages(&[submitted_fixture()]).unwrap();
        assert_eq!(messages.len(), 2);
        assert_eq!(messages[1].text, "完整的推荐正文");
        assert_eq!(messages[1].message_id, "task-result:t:a1");
        assert!(!messages[1].usage.has_tokens());
        assert!(is_terminal(&messages[1].stop_reason));
        let session = RealSessionInfo {
            key: "parent".into(),
            messages: messages
                .into_iter()
                .map(|mut m| {
                    m.run_id = "parent-run".into();
                    m
                })
                .collect(),
            ..Default::default()
        };
        assert!(PendingReply {
            key: "parent".into(),
            text: "".into(),
            run_id: "parent-run".into()
        }
        .finished(&session));
    }
    #[test]
    fn unacknowledged_or_mismatched_submission_does_not_show_arguments_as_result() {
        for mode in [
            "error",
            "pending",
            "mismatch",
            "missing-id",
            "failed-receipt",
        ] {
            let mut row = submitted_fixture();
            let tool = &mut row["toolCalls"][0];
            let mut result = tool_result(tool).unwrap();
            match mode {
                "error" => tool["status"] = json!("ERROR"),
                "pending" => tool["result"] = Value::Null,
                "mismatch" => result["artifacts"][0]["title"] = json!("别的结果"),
                "missing-id" => result["artifacts"][0]["artifactId"] = Value::Null,
                _ => result["artifacts"][0]["ok"] = json!(false),
            }
            if !matches!(mode, "error" | "pending") {
                tool["result"]["result"] = json!(result.to_string());
            }
            assert!(submitted_text_results(&row).is_empty(), "{mode}");
        }
    }
    #[test]
    fn partial_submission_and_non_root_subtask_do_not_finish_whole_task() {
        let mut row = submitted_fixture();
        row["toolCalls"][0]["arguments"]["artifacts"]
            .as_array_mut()
            .unwrap()
            .push(
                json!({"type":"text","contentType":"inline","title":"未提交","content":"不能展示"}),
            );
        let output = submitted_text_results(&row);
        assert_eq!(output.len(), 1);
        assert!(!is_terminal(&output[0].stop_reason));
        let mut row = submitted_fixture();
        let mut result = tool_result(&row["toolCalls"][0]).unwrap();
        result["subTaskId"] = json!(3);
        row["toolCalls"][0]["result"]["result"] = json!(result.to_string());
        assert!(!is_terminal(&submitted_text_results(&row)[0].stop_reason));
    }
    #[test]
    fn inline_results_are_not_taken_from_user_rows() {
        let mut row = submitted_fixture();
        row["role"] = json!("user");
        assert_eq!(parse_messages(&[row]).unwrap().len(), 1);
    }
    #[test]
    fn child_tracking_prioritizes_latest_run_in_long_conversations() {
        let rows = (0..13).map(|i| json!({"runId":format!("run-{i}"),"toolCalls":[{"name":"task_create","result":{"result":json!({"childSessionKey":format!("child-{i}")}).to_string()}}]})).collect::<Vec<_>>();
        let tasks = child_tasks(&rows);
        assert_eq!(tasks[0].run_id, "run-12");
        assert_eq!(tasks[11].run_id, "run-1");
        assert_eq!(tasks[12].run_id, "run-0");
    }
    #[test]
    fn attachment_only_reply_keeps_the_service_reference() {
        let messages=parse_messages(&[json!({"id":"image-reply","role":"assistant","attachments":[{"type":"image","blob":"https://example.test/artifact.png","mime":"image/png","filename":"生成图.png","size":123}],"stopReason":"STOP"})]).unwrap();
        assert_eq!(messages.len(), 1);
        assert_eq!(messages[0].attachments[0].name, "生成图.png");
        assert_eq!(
            messages[0].attachments[0].url,
            "https://example.test/artifact.png"
        );
        assert_eq!(messages[0].attachments[0].size_bytes, Some(123));
    }
    #[test]
    fn opaque_attachment_blob_is_not_assumed_to_be_a_url_or_base64() {
        let attachment = parse_attachment(
            &json!({"type":"file","blob":"opaque-service-blob","mime":"application/pdf"}),
        )
        .unwrap();
        assert!(attachment.url.is_empty());
        assert!(attachment.path.is_empty());
        assert_eq!(attachment.blob, "opaque-service-blob");
        let attachment = parse_attachment(
            &json!({"type":"image","url":"javascript:alert(1)","filename":"photo"}),
        )
        .unwrap();
        assert!(attachment.url.is_empty());
    }
    #[test]
    fn completion_requires_this_run_and_final_marker_not_intermediate_text() {
        let pending = PendingReply {
            key: "session".into(),
            text: "重复提问".into(),
            run_id: "new-run".into(),
        };
        let mut session = RealSessionInfo {
            key: "session".into(),
            status: "已完成".into(),
            ..Default::default()
        };
        session.messages = parse_messages(&[
            json!({"role":"user","text":"重复提问","runId":"old-run"}),
            json!({"role":"assistant","text":"旧结果","runId":"old-run","stopReason":"STOP"}),
            json!({"role":"user","text":"重复提问","runId":"new-run"}),
            json!({"role":"assistant","text":"我用美团来处理","runId":"new-run"}),
            json!({"role":"assistant","runId":"new-run","stopReason":"TOOL_CALLS","toolCalls":[{"name":"ability"}]}),
        ]).unwrap();
        assert!(!pending.finished(&session));
        session.messages.extend(parse_messages(&[json!({"role":"assistant","id":"final-id","text":"完整结果","runId":"new-run","stopReason":"STOP"})]).unwrap());
        assert!(pending.finished(&session));
        assert_eq!(session.messages.last().unwrap().message_id, "final-id");
        session.key = "other-session".into();
        assert!(!pending.finished(&session));
    }

    #[test]
    fn empty_terminal_error_is_not_dropped() {
        let pending = PendingReply {
            key: "session".into(),
            text: "test".into(),
            run_id: "run".into(),
        };
        let session = RealSessionInfo {
            key: "session".into(),
            messages: parse_messages(&[
                json!({"role":"assistant","runId":"run","stopReason":"ERROR"}),
            ])
            .unwrap(),
            ..Default::default()
        };
        assert!(session.messages[0].is_error);
        assert!(pending.finished(&session));
    }
    #[test]
    fn actual_bus_text_and_errors_are_preserved_without_inventing_messages() {
        let rows = vec![
            json!({"role":"user","text":"中文输入","timestamp":1792602316149_i64}),
            json!({"role":"assistant","text":"实际回复"}),
            json!({"role":"assistant","errorMessage":"offline","stopReason":"error"}),
            json!({"role":"assistant","text":""}),
        ];
        let parsed = parse_messages(&rows).unwrap();
        assert_eq!(parsed.len(), 3);
        assert_eq!(parsed[0].text, "中文输入");
        assert_eq!(parsed[1].text, "实际回复");
        assert!(parsed[2].is_error);
        assert!(parsed[1].thinking.is_empty());
        assert!(parse_messages(&[]).unwrap().is_empty());
    }
    #[test]
    fn session_keys_and_state_come_from_service_not_titles() {
        let data = parse_sessions(&[
            json!({"key":"id-a","title":"相同标题","status":"done"}),
            json!({"key":"id-b","title":"相同标题","status":"killed"}),
        ])
        .unwrap();
        assert_ne!(data.sessions[0].key, data.sessions[1].key);
        assert_eq!(data.sessions[1].status, "已停止");
        assert!(data.memory_entries.is_empty());
        assert!(data.skills.is_empty());
        assert!(!data.memory_initialized);
        assert!(parse_sessions(&[json!({"title":"缺少标识"})]).is_err());
        assert!(parse_sessions(&[]).unwrap().sessions.is_empty());
    }
    #[test]
    fn split_frames_read_exactly_and_oversize_is_rejected() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let sender = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            for b in b"\0\0\0\x0b{\"ok\":true}" {
                s.write_all(&[*b]).unwrap();
            }
            s.write_all(&((MAX_FRAME + 1) as u32).to_be_bytes())
                .unwrap();
        });
        let mut stream = TcpStream::connect(address).unwrap();
        assert_eq!(
            read_frame(&mut stream, Instant::now() + Duration::from_secs(2)).unwrap()["ok"],
            true
        );
        assert!(read_frame(&mut stream, Instant::now() + Duration::from_secs(2)).is_err());
        sender.join().unwrap();
    }

    #[test]
    fn parse_stream_event_extracts_delta_tool_and_completion() {
        let delta_frame = json!({
            "type": "event",
            "event": "chat.delta",
            "payload": {
                "sessionId": "agent:main:s1",
                "runId": "run-42",
                "delta": "增量内容",
                "thinking": "推理步骤"
            }
        });
        let delta = parse_stream_event(&delta_frame).unwrap();
        assert_eq!(delta.session_key, "agent:main:s1");
        assert_eq!(delta.run_id, "run-42");
        assert_eq!(delta.delta_text, "增量内容");
        assert_eq!(delta.thinking_text, "推理步骤");
        assert!(!delta.finished);

        let tool_frame = json!({
            "type": "event",
            "event": "agent.tool.call",
            "payload": {
                "sessionKey": "agent:main:s1",
                "runId": "run-42",
                "toolName": "web_search"
            }
        });
        let tool_delta = parse_stream_event(&tool_frame).unwrap();
        assert_eq!(tool_delta.session_key, "agent:main:s1");
        assert_eq!(
            tool_delta.tool_status.as_deref(),
            Some("正在调用工具：联网搜索")
        );
        assert!(!tool_delta.finished);

        let upload_frame = json!({
            "type": "event",
            "event": "agent.tool.call",
            "payload": {
                "sessionKey": "agent:main:s1",
                "runId": "run-42",
                "toolName": "batch_file_upload_return_all"
            }
        });
        let upload_delta = parse_stream_event(&upload_frame).unwrap();
        assert_eq!(
            upload_delta.tool_status.as_deref(),
            Some("正在调用工具：批量上传文件")
        );

        let done_frame = json!({
            "type": "event",
            "event": "chat.message.completed",
            "payload": {
                "sessionId": "agent:main:s1",
                "runId": "run-42",
                "text": "完整回复内容",
                "stopReason": "STOP"
            }
        });
        let done_delta = parse_stream_event(&done_frame).unwrap();
        assert_eq!(done_delta.full_text, "完整回复内容");
        assert!(done_delta.finished);
    }

    #[test]
    fn poll_stream_events_reads_pushed_frames_and_returns_cleanly_on_timeout() {
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let sender = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let f1 = serde_json::to_vec(&json!({
                "type": "event",
                "event": "chat.delta",
                "payload": {
                    "sessionId": "s1",
                    "runId": "r1",
                    "delta": "你好"
                }
            }))
            .unwrap();
            s.write_all(&(f1.len() as u32).to_be_bytes()).unwrap();
            s.write_all(&f1).unwrap();
            let f2 = serde_json::to_vec(&json!({
                "type": "event",
                "event": "chat.delta",
                "payload": {
                    "sessionId": "s1",
                    "runId": "r1",
                    "delta": "，世界",
                    "stopReason": "STOP"
                }
            }))
            .unwrap();
            s.write_all(&(f2.len() as u32).to_be_bytes()).unwrap();
            s.write_all(&f2).unwrap();
            std::thread::sleep(Duration::from_millis(120));
        });
        let mut client = TcpStream::connect(addr).unwrap();
        let events =
            poll_stream_events_from_stream(&mut client, Duration::from_millis(50), false).unwrap();
        assert_eq!(events.len(), 2);
        assert_eq!(events[0].delta_text, "你好");
        assert!(!events[0].finished);
        assert_eq!(events[1].delta_text, "，世界");
        assert!(events[1].finished);

        let empty =
            poll_stream_events_from_stream(&mut client, Duration::from_millis(15), false).unwrap();
        assert!(empty.is_empty());
        sender.join().unwrap();
    }

    #[test]
    fn amoo_context_cache_ttl_hit_and_expiration() {
        let now = Instant::now();
        let key = amoo_context_cache_key_for("SERIAL-CACHE-TEST-01", "192.168.1.99:5555");
        assert_eq!(key, "SERIAL-CACHE-TEST-01");
        assert_eq!(
            amoo_context_cache_key_for("", "192.168.1.99:5555"),
            "192.168.1.99:5555"
        );

        let ctx_val = json!({"ok": true, "brand": "stepos", "model": "AI Phone"});
        put_cached_amoo_context_at(key.clone(), now, ctx_val.clone());

        // Hit within 60-second TTL
        let hit = get_cached_amoo_context_at(&key, now + Duration::from_secs(30));
        assert_eq!(hit, Some(ctx_val));

        // Expired at 60 seconds
        let expired = get_cached_amoo_context_at(&key, now + AMOO_CONTEXT_TTL);
        assert!(expired.is_none());
        // Entry was evicted upon expiration check
        assert!(get_cached_amoo_context_at(&key, now + Duration::from_secs(10)).is_none());
    }

    #[test]
    fn pending_reply_finishes_on_terminal_assistant_message_without_run_id_after_user_turn() {
        let pending = PendingReply {
            key: "session".into(),
            text: "测试无 runId 终态回复".into(),
            run_id: "run-target".into(),
        };
        let mut session = RealSessionInfo {
            key: "session".into(),
            messages: parse_messages(&[
                json!({"role":"user","text":"上一轮","runId":"run-prev"}),
                json!({"role":"assistant","text":"上一轮无 runId 回复","stopReason":"completed"}),
                json!({"role":"user","text":"测试无 runId 终态回复","runId":"run-target"}),
                json!({"role":"assistant","text":"流式中间态","stopReason":"streaming"}),
            ])
            .unwrap(),
            ..Default::default()
        };
        // Earlier assistant message without run_id and non-terminal assistant message after user turn do not finish
        assert!(!pending.finished(&session));

        // Terminal assistant message with stop_reason "completed" and empty run_id immediately after user turn
        session.messages.pop();
        session.messages.extend(
            parse_messages(&[
                json!({"role":"assistant","text":"已完成回复","stopReason":"completed"}),
            ])
            .unwrap(),
        );
        assert!(session.messages.last().unwrap().run_id.is_empty());
        assert!(pending.finished(&session));

        // Also verify with stop_reason "stop" and empty run_id immediately following the user's message
        session.messages.pop();
        session.messages.extend(
            parse_messages(&[json!({"role":"assistant","text":"已停止回复","stopReason":"stop"})])
                .unwrap(),
        );
        assert!(session.messages.last().unwrap().run_id.is_empty());
        assert!(pending.finished(&session));
    }

    #[test]
    fn parse_stream_event_rejects_missing_session_key_to_prevent_cross_session_hijack() {
        let missing_session = json!({
            "type": "event",
            "event": "agent",
            "payload": {
                "stream": "assistant",
                "runId": "run-123",
                "data": { "text": "没有 sessionKey 的碎片" }
            }
        });
        assert!(parse_stream_event(&missing_session).is_none());

        let with_session = json!({
            "type": "event",
            "event": "agent",
            "payload": {
                "sessionKey": "agent:main:subagent:abc",
                "stream": "assistant",
                "runId": "run-123",
                "data": { "text": "带 sessionKey 的碎片" }
            }
        });
        let delta = parse_stream_event(&with_session).expect("delta with sessionKey");
        assert_eq!(delta.session_key, "agent:main:subagent:abc");
        assert_eq!(delta.delta_text.as_str(), "带 sessionKey 的碎片");
    }

    #[test]
    fn status_from_session_and_history_clears_stale_running_status_when_reply_is_done() {
        let mut session = RealSessionInfo {
            key: "agent:main:main".into(),
            status: "进行中".into(),
            status_kind: 1,
            messages: parse_messages(&[
                json!({"role":"user","text":"写一篇长文","timestamp": 1_000}),
                json!({"role":"assistant","text":"这是完整的长文内容。","stopReason":"end_turn","timestamp": 2_000}),
            ])
            .unwrap(),
            ..Default::default()
        };
        let (status, kind) = status_from_session_and_history(&session, 3_000);
        assert_eq!(status, "已完成");
        assert_eq!(kind, 2);

        // Assistant message without explicit stopReason that is older than 15s also clears stale running status
        session.messages = parse_messages(&[
            json!({"role":"user","text":"写一篇长文","timestamp": 1_000}),
            json!({"role":"assistant","text":"已输出完毕但未带 stopReason","timestamp": 2_000}),
        ])
        .unwrap();
        let (status_aged, kind_aged) = status_from_session_and_history(&session, 20_000);
        assert_eq!(status_aged, "已完成");
        assert_eq!(kind_aged, 2);

        // Fresh streaming assistant message (<15s) without stopReason keeps running status if gateway says running
        let (status_fresh, kind_fresh) = status_from_session_and_history(&session, 5_000);
        assert_eq!(status_fresh, "进行中");
        assert_eq!(kind_fresh, 1);
    }

    #[test]
    fn live_device_telemetry_matches_hardware_queries_and_strips_marker_from_user_history() {
        assert!(should_inject_device_telemetry("看下手机的保修情况"));
        assert!(should_inject_device_telemetry("电池健康度和循环次数"));
        assert!(!should_inject_device_telemetry("上海的天气"));

        let device = Device {
            transport: "STEPX_TEST_DEVICE".into(),
            serial: "STEPX_TEST_DEVICE".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        let raw = "\
MARKETNAME=STEPX Neo
MODEL=PZ261A
DEVICE_CODE=pandora
SERIALNO=STEPX_TEST_DEVICE
ASN=ASN123456789
NET_SN=NET987654
DISPLAY_ID=Step AOS 1.0.0
RELEASE=16
CMIIT=2025CP1234
SOH=100
DESIGN_CAP=6150000
CHARGE_CNT=6020000
UPS_MODE=0
UPS_LIMIT=80
FAST_CHARGE=1
CYCLE_LINE=Bundle[{android.os.extra.CYCLE_COUNT=15}]
---BATTERY---
  level: 88
  voltage: 4180
  temperature: 312
---THERMAL---
  Temperature{mValue=34.5, mType=0, mName=skin-therm, mStatus=0}
";
        let block = format_live_device_telemetry_block(raw, &device).expect("telemetry block");
        assert!(block.starts_with(LIVE_DEVICE_CONTEXT_MARKER));
        assert!(block.contains("ASN123456789"));
        assert!(block.contains("健康度 (SOH) 100% · 充电循环 15 次 · 设计容量 6150 mAh"));

        let enriched = format!("看下手机的保修情况{block}");
        let parsed = parse_messages(&[
            json!({"role": "user", "text": enriched}),
            json!({"role": "assistant", "text": "尚未查询官方保修状态。"}),
        ])
        .unwrap();
        assert_eq!(parsed.len(), 2);
        assert_eq!(parsed[0].text, "看下手机的保修情况");
    }
    #[test]
    fn missing_telemetry_never_implies_warranty_or_sensor_values() {
        let device = Device {
            transport: "test".into(),
            serial: String::new(),
            model: String::new(),
            android: String::new(),
            product: String::new(),
        };
        let block = format_live_device_telemetry_block("SOH=\nDESIGN_CAP=\n", &device).unwrap();
        assert!(block.contains("健康度 (SOH) 未读取"));
        assert!(block.contains("无法确认是否在保"));
        assert!(!block.contains("6150"));
        assert!(!block.contains("已认证"));
        assert!(!block.contains("16 GB"));
    }

    #[cfg(unix)]
    #[test]
    fn agentbus_observe_recovers_from_stale_cached_port_on_handshake_failure() {
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let port_file = dir.path().join("port.txt");
        let calls_log = dir.path().join("calls.log");

        let stale_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let stale_port = stale_listener.local_addr().unwrap().port();
        let fresh_listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let fresh_port = fresh_listener.local_addr().unwrap().port();

        std::fs::write(&port_file, format!("{stale_port}\n")).unwrap();

        let script = format!(
            r#"#!/usr/bin/env sh
echo "$*" >> "{calls}"
case "$*" in
  *"getprop"*ro.serialno*)
    echo "AGENTBUS_STALE_SERIAL_AMOO_UNIT"
    exit 0
    ;;
  *"forward tcp:0 localabstract:agentbus"*)
    cat "{port}"
    exit 0
    ;;
  *"forward --remove"*)
    exit 0
    ;;
esac
exit 0
"#,
            calls = calls_log.display(),
            port = port_file.display()
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = Adb { path: fake_adb };
        let device = Device {
            transport: "AGENTBUS_STALE_SERIAL_AMOO_UNIT".into(),
            serial: "AGENTBUS_STALE_SERIAL_AMOO_UNIT".into(),
            model: "PZ261A-AMOO-UNIT".into(),
            android: "16".into(),
            product: "pandora".into(),
        };

        // Prime FORWARD_POOL with stale_port, then switch port_file to fresh_port.
        let (primed_stream, primed_port, was_cached) = adb
            .connect_forwarded_tcp(
                "AGENTBUS_STALE_SERIAL_AMOO_UNIT",
                "localabstract:agentbus",
                Duration::from_secs(1),
            )
            .unwrap();
        assert_eq!(primed_port, stale_port);
        assert!(!was_cached);
        drop(primed_stream);
        let _ = stale_listener.accept();
        std::fs::write(&port_file, format!("{fresh_port}\n")).unwrap();

        // Stale listener accepts cached TCP stream and closes immediately (simulating dead remote daemon/reused port).
        let stale_handle = std::thread::spawn(move || {
            if let Ok((stream, _)) = stale_listener.accept() {
                drop(stream);
            }
        });

        // Fresh listener reads connect request frame and replies with protocol 3 hello.
        let fresh_handle = std::thread::spawn(move || {
            let (mut stream, _) = fresh_listener.accept().unwrap();
            let req = read_frame(&mut stream, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(req["method"], "connect");
            let id = req["id"].as_str().unwrap().to_string();
            let resp = serde_json::to_vec(&json!({
                "type": "res",
                "id": id,
                "ok": true,
                "payload": {
                    "protocol": 3,
                    "server": { "version": "stepclaw-stale-recovered" }
                }
            }))
            .unwrap();
            stream
                .write_all(&(resp.len() as u32).to_be_bytes())
                .unwrap();
            stream.write_all(&resp).unwrap();
        });

        let bus = AgentBus::observe(&adb, &device)
            .expect("AgentBus::observe must evict stale cached port and reconnect on fresh port");
        assert_eq!(bus.version, "stepclaw-stale-recovered");
        stale_handle.join().unwrap();
        fresh_handle.join().unwrap();

        let calls = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert!(
            calls.contains(&format!("forward --remove tcp:{stale_port}")),
            "stale port must be removed via forward --remove: {calls}"
        );
        adb.invalidate_forwarded_tcp(
            "AGENTBUS_STALE_SERIAL_AMOO_UNIT",
            "localabstract:agentbus",
            None,
        );
    }

    #[test]
    fn read_frame_timeout_never_leaks_os_error_11_and_survives_chunked_delivery() {
        use std::net::TcpListener;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (timed_out, wait_for_timeout) = std::sync::mpsc::channel();

        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            // First test: write a frame in two halves separated by 320ms (across a 250ms read slice)
            let payload = serde_json::to_vec(&json!({"ok": true, "step": 1})).unwrap();
            let mut framed = Vec::new();
            framed.extend_from_slice(&(payload.len() as u32).to_be_bytes());
            framed.extend_from_slice(&payload);
            s.write_all(&framed[..3]).unwrap();
            std::thread::sleep(Duration::from_millis(320));
            s.write_all(&framed[3..]).unwrap();

            // Keep the connection open until the client actually observed its
            // timeout. A fixed sleep races slow CI schedulers and can become EOF.
            wait_for_timeout
                .recv_timeout(Duration::from_secs(10))
                .unwrap();
        });

        let mut client = TcpStream::connect(addr).unwrap();
        let val = read_frame(&mut client, Instant::now() + Duration::from_secs(2))
            .expect("chunked frame across 250ms read slices must succeed");
        assert_eq!(val["step"], 1);

        let err = read_frame(&mut client, Instant::now() + Duration::from_millis(80))
            .expect_err("silent stream must time out at deadline");
        let rendered = format!("{err:#}");
        assert_eq!(rendered, CLOUD_TIMEOUT_MESSAGE);
        assert!(
            !rendered.contains("os error 11")
                && !rendered.contains("Resource temporarily unavailable"),
            "rendered error must not leak raw EAGAIN/os error 11: {rendered}"
        );
        timed_out.send(()).unwrap();
        server.join().unwrap();
    }

    #[test]
    fn parse_stream_event_supports_node_event_session_message_and_turn_done() {
        let streaming_frame = json!({
            "type": "req",
            "method": "node.event",
            "params": {
                "event": "session.message",
                "payload": {
                    "sessionId": "agent:main:desktop-s1",
                    "text": "上海今天晴",
                    "role": "assistant",
                    "replyTo": "c1",
                    "toolCalls": [{"name": "web_search"}],
                    "delta": "上海今天晴",
                    "metadata": {
                        "runId": "stepx-run-123",
                        "streaming": true
                    }
                }
            }
        });
        let delta =
            parse_stream_event(&streaming_frame).expect("must parse node.event session.message");
        assert_eq!(delta.session_key, "agent:main:desktop-s1");
        assert_eq!(delta.run_id, "stepx-run-123");
        assert_eq!(delta.delta_text, "上海今天晴");
        assert_eq!(delta.full_text, "上海今天晴");
        assert_eq!(delta.tool_status.as_deref(), Some("正在调用工具：联网搜索"));
        assert!(!delta.finished);

        let done_frame = json!({
            "type": "req",
            "method": "node.event",
            "params": {
                "event": "session.message",
                "payload": {
                    "sessionId": "agent:main:desktop-s1",
                    "text": "上海今天晴，气温 22°C。",
                    "role": "assistant",
                    "replyTo": "c1",
                    "toolCalls": [],
                    "metadata": {
                        "runId": "stepx-run-123",
                        "streaming": false,
                        "lifecyclePhase": "turn_done"
                    }
                }
            }
        });
        let done = parse_stream_event(&done_frame).expect("must parse turn_done frame");
        assert_eq!(done.session_key, "agent:main:desktop-s1");
        assert_eq!(done.run_id, "stepx-run-123");
        assert_eq!(done.full_text, "上海今天晴，气温 22°C。");
        assert!(done.finished);

        let user_echo = json!({
            "type": "req",
            "method": "node.event",
            "params": {
                "event": "session.message",
                "payload": {
                    "sessionId": "agent:main:desktop-s1",
                    "text": "上海天气",
                    "role": "user"
                }
            }
        });
        assert!(parse_stream_event(&user_echo).is_none());
    }

    #[test]
    fn stepclaw_status_change_parsing_and_deadline_transitions() {
        let screen_off_frame = json!({
            "type": "req",
            "id": "evt-1",
            "method": "node.event",
            "params": {
                "event": "stepclaw.status.change",
                "payload": {
                    "version": 2,
                    "resources": {
                        "service": {"status": "available"},
                        "chat": {"status": "unavailable", "reason": "no cloud gateway online"},
                        "upstream": {"status": "unavailable", "reason": "screen_off"}
                    }
                }
            }
        });
        let st_off = parse_stepclaw_status_change(&screen_off_frame).unwrap();
        assert!(st_off.is_upstream_screen_off());
        assert!(st_off.is_chat_unavailable());
        assert!(!st_off.is_chat_available());

        let waking_frame = json!({
            "type": "req",
            "id": "evt-2",
            "method": "node.event",
            "params": {
                "event": "stepclaw.status.change",
                "payload": {
                    "version": 2,
                    "resources": {
                        "chat": {"status": "unavailable", "reason": "no cloud gateway online"},
                        "upstream": {"status": "unavailable", "reason": "cloud claw offline"}
                    }
                }
            }
        });
        let st_waking = parse_stepclaw_status_change(&waking_frame).unwrap();
        assert!(!st_waking.is_upstream_screen_off());
        assert!(st_waking.is_chat_unavailable());

        let online_frame = json!({
            "type": "req",
            "id": "evt-3",
            "method": "node.event",
            "params": {
                "event": "stepclaw.status.change",
                "payload": {
                    "version": 2,
                    "resources": {
                        "chat": {"status": "available"},
                        "upstream": {"status": "available", "reason": "screen_on"}
                    }
                }
            }
        });
        let st_online = parse_stepclaw_status_change(&online_frame).unwrap();
        assert!(st_online.is_chat_available());
        assert!(!st_online.is_chat_unavailable());

        // 1. Observe mode caps deadline to <= ~1.2s on screen_off, and restores on waking
        let start = Instant::now();
        let mut deadline = start + REQUEST_TIMEOUT;
        let mut ext_wake = false;
        let mut ext_avail = false;
        update_deadline_for_status(
            false,
            false,
            REQUEST_TIMEOUT,
            start,
            &st_off,
            &mut deadline,
            &mut ext_wake,
            &mut ext_avail,
        );
        assert!(deadline.duration_since(start) <= Duration::from_millis(1500));

        update_deadline_for_status(
            false,
            false,
            REQUEST_TIMEOUT,
            start,
            &st_waking,
            &mut deadline,
            &mut ext_wake,
            &mut ext_avail,
        );
        assert!(deadline >= start + REQUEST_TIMEOUT);

        // 2. Task mode without lease caps screen_off to ~3s, then extends to 30s on waking, then resets on available
        let mut task_deadline = start + REQUEST_TIMEOUT;
        let mut task_ext_wake = false;
        let mut task_ext_avail = false;
        update_deadline_for_status(
            true,
            false,
            REQUEST_TIMEOUT,
            start,
            &st_off,
            &mut task_deadline,
            &mut task_ext_wake,
            &mut task_ext_avail,
        );
        assert!(task_deadline.duration_since(start) <= Duration::from_millis(3300));
        assert!(!task_ext_wake);

        update_deadline_for_status(
            true,
            false,
            REQUEST_TIMEOUT,
            start,
            &st_waking,
            &mut task_deadline,
            &mut task_ext_wake,
            &mut task_ext_avail,
        );
        assert!(task_ext_wake);
        assert!(task_deadline >= start + Duration::from_secs(29));

        update_deadline_for_status(
            true,
            false,
            REQUEST_TIMEOUT,
            start,
            &st_online,
            &mut task_deadline,
            &mut task_ext_wake,
            &mut task_ext_avail,
        );
        assert!(task_ext_avail);

        // 3. Task mode with lease refreshes wake deadline when transitioning from screen_off to cloud claw offline
        let mut leased_deadline = start + REQUEST_TIMEOUT;
        let mut leased_ext_wake = false;
        let mut leased_ext_avail = false;
        update_deadline_for_status(
            true,
            true,
            REQUEST_TIMEOUT,
            start,
            &st_off,
            &mut leased_deadline,
            &mut leased_ext_wake,
            &mut leased_ext_avail,
        );
        assert!(leased_ext_wake);
        // Simulate 15s elapsed during OFF_SETTLE_MS before CloudWake triggers cloud claw offline
        leased_deadline = Instant::now() + Duration::from_secs(5);
        update_deadline_for_status(
            true,
            true,
            REQUEST_TIMEOUT,
            start,
            &st_waking,
            &mut leased_deadline,
            &mut leased_ext_wake,
            &mut leased_ext_avail,
        );
        assert!(leased_deadline >= Instant::now() + Duration::from_secs(44));

        // 4. Short-timeout task call (< REQUEST_TIMEOUT) restores deadline once no longer screen_off
        let short_timeout = Duration::from_secs(6);
        let mut short_deadline = start + short_timeout;
        let mut short_ext_wake = false;
        let mut short_ext_avail = false;
        update_deadline_for_status(
            true,
            false,
            short_timeout,
            start,
            &st_off,
            &mut short_deadline,
            &mut short_ext_wake,
            &mut short_ext_avail,
        );
        assert!(short_deadline.duration_since(start) <= Duration::from_millis(3300));
        update_deadline_for_status(
            true,
            false,
            short_timeout,
            start,
            &st_waking,
            &mut short_deadline,
            &mut short_ext_wake,
            &mut short_ext_avail,
        );
        assert!(short_deadline >= start + short_timeout);
    }

    #[test]
    fn parse_stream_event_filters_tool_result_text_and_keeps_intermediate_tool_calls_unfinished() {
        // 1. Intermediate assistant tool-calling frame (streaming=false, stopReason=TOOL_CALLS) must NOT be marked finished
        let assistant_tool_call = json!({
            "type": "req",
            "id": "evt-tc-1",
            "method": "node.event",
            "params": {
                "event": "session.message",
                "payload": {
                    "sessionId": "agent:main:stepx-1",
                    "role": "assistant",
                    "text": "",
                    "stopReason": "TOOL_CALLS",
                    "toolCalls": [{"name": "web_search"}],
                    "metadata": {"runId": "run-1", "streaming": false}
                }
            }
        });
        let d1 = parse_stream_event(&assistant_tool_call).expect("assistant tool call event");
        assert!(d1.tool_status.as_deref().unwrap_or("").contains("联网搜索"));
        assert_eq!(d1.full_text, "");
        assert!(
            !d1.finished,
            "intermediate TOOL_CALLS step must not mark stream finished"
        );

        // 2. Tool result frame (role="tool", streaming=false) must NOT overwrite assistant text or mark finished
        let tool_result = json!({
            "type": "req",
            "id": "evt-tc-2",
            "method": "node.event",
            "params": {
                "event": "session.message",
                "payload": {
                    "sessionId": "agent:main:stepx-1",
                    "role": "tool",
                    "text": "{\"raw_tool_json\": 12345}",
                    "delta": "{\"raw_tool_json\": 12345}",
                    "metadata": {"runId": "run-1", "streaming": false, "toolName": "web_search"}
                }
            }
        });
        let d2 = parse_stream_event(&tool_result).expect("tool result event");
        assert!(d2.tool_status.as_deref().unwrap_or("").contains("联网搜索"));
        assert_eq!(
            d2.full_text, "",
            "tool role output must not leak into assistant full_text"
        );
        assert_eq!(
            d2.delta_text, "",
            "tool role output must not leak into assistant delta_text"
        );
        assert!(
            !d2.finished,
            "tool role frame must not mark assistant reply finished"
        );

        // 3. System frame must be ignored
        let system_frame = json!({
            "type": "req",
            "method": "node.event",
            "params": {
                "event": "session.message",
                "payload": {
                    "sessionId": "agent:main:stepx-1",
                    "role": "system",
                    "text": "system prompt",
                    "metadata": {"runId": "run-1", "streaming": false}
                }
            }
        });
        assert!(parse_stream_event(&system_frame).is_none());
    }

    #[cfg(unix)]
    #[test]
    fn agentbus_reconnects_cleanly_after_mid_session_timeout_and_idle_eof() {
        check_agentbus_timeout_and_warm_eof(None);
    }

    #[cfg(unix)]
    #[test]
    fn agentbus_does_not_replay_mutation_after_partial_response_header_eof() {
        check_agentbus_timeout_and_warm_eof(Some(2));
    }

    #[cfg(unix)]
    #[test]
    fn agentbus_does_not_replay_mutation_after_partial_response_body_eof() {
        check_agentbus_timeout_and_warm_eof(Some(6));
    }

    #[cfg(unix)]
    fn check_agentbus_timeout_and_warm_eof(partial_reply_bytes: Option<usize>) {
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        // Each run has an independent cached forward/context, including when
        // these regression cases execute in parallel.
        let serial = format!("AGENTBUS_RECONNECT_SERIAL_{port}");
        let (call_finished, wait_for_call) = std::sync::mpsc::channel();

        let script = format!(
            r#"#!/usr/bin/env sh
case "$*" in
  *"getprop"*ro.serialno*)
    echo "{serial}"
    exit 0
    ;;
  *"forward tcp:0 localabstract:agentbus"*)
    echo "{port}"
    exit 0
    ;;
  *"forward --remove"*)
    exit 0
    ;;
esac
exit 0
"#
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = Adb { path: fake_adb };
        let device = Device {
            transport: serial.clone(),
            serial: serial.clone(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        put_cached_amoo_context_at(
            serial.clone(),
            Instant::now(),
            json!({"ok": true, "brand": "stepos", "model": "PZ261A"}),
        );

        let server = std::thread::spawn(move || {
            let send = |stream: &mut TcpStream, val: Value| {
                let bytes = serde_json::to_vec(&val).unwrap();
                stream
                    .write_all(&(bytes.len() as u32).to_be_bytes())
                    .unwrap();
                stream.write_all(&bytes).unwrap();
            };

            // Connection 1: handshake succeeds, then first RPC ("sessions.list") times out without reply
            let (mut s1, _) = listener.accept().unwrap();
            let c1 = read_frame(&mut s1, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(c1["method"], "connect");
            send(
                &mut s1,
                json!({
                    "type": "hello-ok",
                    "id": c1["id"],
                    "ok": true,
                    "payload": {"protocol": 3, "server": {"id": "agentbus", "version": "1.0.0"}}
                }),
            );
            let timed_out_req =
                read_frame(&mut s1, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(timed_out_req["method"], "sessions.list");
            // Do not reply on s1 so client times out after 100ms

            // Connection 2: client automatically reconnects for the next RPC.
            let (mut s2, _) = listener.accept().unwrap();
            let c2 = read_frame(&mut s2, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(c2["method"], "connect");
            send(
                &mut s2,
                json!({
                    "type": "hello-ok",
                    "id": c2["id"],
                    "ok": true,
                    "payload": {"protocol": 3, "server": {"id": "agentbus", "version": "1.0.0"}}
                }),
            );
            let send_req = read_frame(&mut s2, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(send_req["method"], "chat.send");
            send(
                &mut s2,
                json!({
                    "type": "res",
                    "id": send_req["id"],
                    "ok": true,
                    "payload": {"runId": "run-after-timeout", "status": "started"}
                }),
            );
            if let Some(prefix_len) = partial_reply_bytes {
                let request = read_frame(&mut s2, Instant::now() + Duration::from_secs(2)).unwrap();
                assert_eq!(request["method"], "sessions.delete");
                let payload = serde_json::to_vec(&json!({
                    "type": "res", "id": request["id"], "ok": true,
                    "payload": {"ok": true}
                }))
                .unwrap();
                let mut frame = (payload.len() as u32).to_be_bytes().to_vec();
                frame.extend_from_slice(&payload);
                s2.write_all(&frame[..prefix_len]).unwrap();
                s2.shutdown(std::net::Shutdown::Write).unwrap();
                wait_for_call.recv_timeout(Duration::from_secs(10)).unwrap();
                listener.set_nonblocking(true).unwrap();
                assert_eq!(
                    listener.accept().unwrap_err().kind(),
                    ErrorKind::WouldBlock,
                    "partial responses must not trigger a reconnect or replay"
                );
                return;
            }

            // Explicit FIN with the receive half retained models an idle EOF.
            // drop(s2) could race the client's next write, discard unread bytes,
            // and become RST on BSD/macOS instead of the intended orderly EOF.
            s2.shutdown(std::net::Shutdown::Write).unwrap();

            // Connection 3: client sees EOF on warm s2 during "sessions.delete" and transparently reconnects
            let (mut s3, _) = listener.accept().unwrap();
            let c3 = read_frame(&mut s3, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(c3["method"], "connect");
            send(
                &mut s3,
                json!({
                    "type": "hello-ok",
                    "id": c3["id"],
                    "ok": true,
                    "payload": {"protocol": 3, "server": {"id": "agentbus", "version": "1.0.0"}}
                }),
            );
            drop(s2);
            let del_req = read_frame(&mut s3, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(del_req["method"], "sessions.delete");
            send(
                &mut s3,
                json!({
                    "type": "res",
                    "id": del_req["id"],
                    "ok": true,
                    "payload": {"ok": true}
                }),
            );
        });

        let mut bus = AgentBus::connect(&adb, &device).expect("AgentBus::connect");
        // 1. First call times out
        let err = bus
            .call_with_timeout(
                "sessions.list",
                json!({"limit": 10}),
                Duration::from_millis(100),
            )
            .expect_err("sessions.list should time out");
        assert!(err.to_string().contains("超时"));

        // 2. Subsequent call on the same AgentBus automatically reconnects to a fresh stream and succeeds
        let receipt = bus
            .send("agent:main:stepx-1", "继续发送")
            .expect("send after timeout should auto-reconnect and succeed");
        assert_eq!(receipt.run_id, "run-after-timeout");

        // 3. Subsequent call on warm bus whose socket closed with EOF transparently reconnects and succeeds
        let deletion = bus.call_with_timeout(
            "sessions.delete",
            json!({"key": "agent:main:stepx-1"}),
            Duration::from_secs(2),
        );
        if partial_reply_bytes.is_some() {
            let error = deletion.expect_err("an incomplete response cannot confirm deletion");
            assert!(error.to_string().contains("连接已关闭"));
            assert!(
                bus.stream_tainted,
                "the next operation must use a fresh stream"
            );
            call_finished.send(()).unwrap();
        } else {
            deletion.expect("delete after warm socket EOF should auto-reconnect and succeed");
        }

        server.join().unwrap();
        adb.invalidate_forwarded_tcp(&serial, "localabstract:agentbus", None);
    }

    #[cfg(unix)]
    #[test]
    fn agentbus_end_to_end_status_change_and_node_event_streaming() {
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();

        let script = format!(
            r#"#!/usr/bin/env sh
case "$*" in
  *"getprop"*ro.serialno*)
    echo "AGENTBUS_E2E_SERIAL"
    exit 0
    ;;
  *"forward tcp:0 localabstract:agentbus"*)
    echo "{port}"
    exit 0
    ;;
  *"forward --remove"*)
    exit 0
    ;;
esac
exit 0
"#
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = Adb { path: fake_adb };
        let device = Device {
            transport: "AGENTBUS_E2E_SERIAL".into(),
            serial: "AGENTBUS_E2E_SERIAL".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        put_cached_amoo_context_at(
            "AGENTBUS_E2E_SERIAL".into(),
            Instant::now(),
            json!({"ok": true, "brand": "stepos", "model": "PZ261A"}),
        );

        let server = std::thread::spawn(move || {
            let (mut s, _) = listener.accept().unwrap();
            let send = |stream: &mut TcpStream, val: Value| {
                let bytes = serde_json::to_vec(&val).unwrap();
                stream
                    .write_all(&(bytes.len() as u32).to_be_bytes())
                    .unwrap();
                stream.write_all(&bytes).unwrap();
            };

            // 1. Handshake
            let connect_req = read_frame(&mut s, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(connect_req["method"], "connect");
            send(
                &mut s,
                json!({
                    "type": "hello-ok",
                    "id": connect_req["id"],
                    "ok": true,
                    "payload": {"protocol": 3, "server": {"id": "agentbus", "version": "1.0.0"}}
                }),
            );

            // Push initial screen_off status, then waking, then available before replying to sessions.reset
            send(
                &mut s,
                json!({
                    "type": "req",
                    "id": "evt-1",
                    "method": "node.event",
                    "params": {
                        "event": "stepclaw.status.change",
                        "payload": {
                            "version": 2,
                            "resources": {
                                "chat": {"status": "unavailable", "reason": "no cloud gateway online"},
                                "upstream": {"status": "unavailable", "reason": "screen_off"}
                            }
                        }
                    }
                }),
            );

            let reset_req = read_frame(&mut s, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(reset_req["method"], "sessions.reset");
            let session_key = reset_req["params"]["key"].as_str().unwrap().to_string();

            send(
                &mut s,
                json!({
                    "type": "req",
                    "id": "evt-2",
                    "method": "node.event",
                    "params": {
                        "event": "stepclaw.status.change",
                        "payload": {
                            "version": 2,
                            "resources": {
                                "chat": {"status": "unavailable", "reason": "no cloud gateway online"},
                                "upstream": {"status": "unavailable", "reason": "cloud claw offline"}
                            }
                        }
                    }
                }),
            );
            send(
                &mut s,
                json!({
                    "type": "req",
                    "id": "evt-3",
                    "method": "node.event",
                    "params": {
                        "event": "stepclaw.status.change",
                        "payload": {
                            "version": 2,
                            "resources": {
                                "chat": {"status": "available"},
                                "upstream": {"status": "available", "reason": "screen_on"}
                            }
                        }
                    }
                }),
            );
            send(
                &mut s,
                json!({
                    "type": "res",
                    "id": reset_req["id"],
                    "ok": true,
                    "payload": {"ok": true, "key": session_key}
                }),
            );

            // 2. chat.send + node.event session.message streaming
            let send_req = read_frame(&mut s, Instant::now() + Duration::from_secs(2)).unwrap();
            assert_eq!(send_req["method"], "chat.send");
            send(
                &mut s,
                json!({
                    "type": "res",
                    "id": send_req["id"],
                    "ok": true,
                    "payload": {"runId": "stepx-run-e2e", "status": "started"}
                }),
            );
            send(
                &mut s,
                json!({
                    "type": "req",
                    "method": "node.event",
                    "params": {
                        "event": "session.message",
                        "payload": {
                            "sessionId": session_key,
                            "text": "后台连接成功",
                            "role": "assistant",
                            "delta": "后台连接成功",
                            "metadata": {"runId": "stepx-run-e2e", "streaming": true}
                        }
                    }
                }),
            );
            send(
                &mut s,
                json!({
                    "type": "req",
                    "method": "node.event",
                    "params": {
                        "event": "session.message",
                        "payload": {
                            "sessionId": session_key,
                            "text": "后台连接成功",
                            "role": "assistant",
                            "metadata": {"runId": "stepx-run-e2e", "streaming": false, "lifecyclePhase": "turn_done"}
                        }
                    }
                }),
            );
        });

        let mut bus = AgentBus::connect(&adb, &device).expect("AgentBus::connect");
        let key = bus
            .create_session()
            .expect("create_session across status change");
        let receipt = bus.send(&key, "上海天气").expect("bus.send");
        assert_eq!(receipt.run_id, "stepx-run-e2e");

        let deltas = bus
            .poll_stream_events(Duration::from_millis(100))
            .expect("poll_stream_events");
        assert_eq!(deltas.len(), 2);
        assert_eq!(deltas[0].delta_text, "后台连接成功");
        assert!(!deltas[0].finished);
        assert_eq!(deltas[1].full_text, "后台连接成功");
        assert!(deltas[1].finished);

        server.join().unwrap();
        adb.invalidate_forwarded_tcp("AGENTBUS_E2E_SERIAL", "localabstract:agentbus", None);
    }
}
