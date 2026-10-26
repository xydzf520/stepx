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
const REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const AMOO_CONTEXT_TTL: Duration = Duration::from_secs(60);

static AMOO_CONTEXT_CACHE: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (Instant, Value)>>,
> = std::sync::OnceLock::new();

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

fn cached_amoo_context(adb: &Adb, device: &Device) -> Result<Value> {
    let key = amoo_context_cache_key(device);
    let now = Instant::now();
    if let Some(cached) = get_cached_amoo_context_at(&key, now) {
        return Ok(cached);
    }
    let val = crate::bridge::call_bridge(
        adb,
        device,
        &json!({"op":"amoo_context"}),
        Duration::from_secs(10),
    )?;
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
                    "params":{"app_name":app_name.trim(),"task":task.trim()}}))?}
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
    if frame.get("type").and_then(Value::as_str) != Some("event") {
        return None;
    }
    let event = string(frame, &["event"]);
    let supported = matches!(
        event,
        "chat"
            | "chat.delta"
            | "chat.message"
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
        || event.starts_with("agent.")
        || event.starts_with("tool.")
        || event.starts_with("run.")
        || event.starts_with("message.");
    if !supported {
        return None;
    }

    let payload = frame.get("payload").unwrap_or(frame);
    let data = payload.get("data").filter(|v| v.is_object());

    let raw_session_key = string(
        payload,
        &["sessionKey", "session_key", "sessionId", "session_id"],
    );
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
        if r.is_empty() {
            string(frame, &["runId", "run_id"]).to_string()
        } else {
            r.to_string()
        }
    };

    let mut delta_text = string(
        payload,
        &["delta", "textDelta", "contentDelta", "deltaText"],
    )
    .to_string();
    if delta_text.is_empty() {
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
    if delta_text.is_empty() {
        if let Some(d) = data {
            delta_text = string(d, &["delta", "textDelta", "contentDelta", "text"]).to_string();
        }
    }

    let full_text = extract_content(&payload["text"])
        .or_else(|| extract_content(&payload["content"]))
        .or_else(|| {
            payload.get("message").and_then(|m| {
                extract_content(&m["text"]).or_else(|| extract_content(&m["content"]))
            })
        })
        .unwrap_or_default();

    let mut thinking_text =
        string(payload, &["thinking", "reasoning", "thinkingDelta"]).to_string();
    if thinking_text.is_empty() {
        if let Some(delta_obj) = payload.get("delta").filter(|v| v.is_object()) {
            thinking_text = string(delta_obj, &["thinking", "reasoning"]).to_string();
        }
    }
    if thinking_text.is_empty() {
        if let Some(msg_obj) = payload.get("message").filter(|v| v.is_object()) {
            thinking_text = string(msg_obj, &["thinking", "reasoning"]).to_string();
        }
    }
    if thinking_text.is_empty() {
        if let Some(d) = data {
            thinking_text = string(d, &["thinking", "reasoning"]).to_string();
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
    let finished = payload["done"].as_bool() == Some(true)
        || payload["finished"].as_bool() == Some(true)
        || payload["final"].as_bool() == Some(true)
        || matches!(
            state.to_ascii_lowercase().as_str(),
            "done"
                | "completed"
                | "finished"
                | "final"
                | "error"
                | "aborted"
                | "stop"
                | "end_turn"
                | "cancelled"
        );

    let is_tool_event = event == "tool"
        || event.contains("tool")
        || matches!(
            state.to_ascii_lowercase().as_str(),
            "tool_use" | "tool_calls" | "tool" | "calling_tool"
        )
        || payload["stream"].as_str() == Some("tool")
        || payload.get("tool").is_some()
        || payload.get("toolName").is_some()
        || payload.get("tool_name").is_some()
        || payload
            .get("toolCalls")
            .and_then(Value::as_array)
            .is_some_and(|a| !a.is_empty());

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

pub struct AgentBus {
    _lease: Option<crate::amoo_background::Lease>,
    stream: TcpStream,
    _forward: Forward,
    device: Device,
    pub version: String,
    pending_events: Vec<StreamDelta>,
}

struct Forward {
    adb: Adb,
    serial: String,
    port: String,
}
impl Drop for Forward {
    fn drop(&mut self) {
        let _ = self.adb.run_timeout(
            &["-s", &self.serial, "forward", "--remove", &self.port],
            Duration::from_secs(3),
        );
    }
}

fn read_exact_until(stream: &mut TcpStream, mut bytes: &mut [u8], deadline: Instant) -> Result<()> {
    while !bytes.is_empty() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .context("Amoo 后台响应超时")?;
        stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
        let count = stream.read(bytes).context("Amoo 云连接已中断或超时；请检查手机网络及小工具中的伴侣后台协作状态，未确认发送的草稿与附件已保留")?;
        if count == 0 {
            bail!("Amoo 后台连接已关闭");
        }
        bytes = &mut bytes[count..];
    }
    Ok(())
}

fn read_frame(stream: &mut TcpStream, deadline: Instant) -> Result<Value> {
    let mut header = [0; 4];
    read_exact_until(stream, &mut header, deadline)?;
    let size = u32::from_be_bytes(header) as usize;
    if size == 0 || size > MAX_FRAME {
        bail!("Amoo 后台返回了无效的数据帧长度");
    }
    let mut body = vec![0; size];
    read_exact_until(stream, &mut body, deadline)?;
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

impl AgentBus {
    pub fn connect(adb: &Adb, device: &Device) -> Result<Self> {
        Self::connect_mode(adb, device, true)
    }

    /// Background observation must not wake or extend the phone's task window.
    pub fn observe(adb: &Adb, device: &Device) -> Result<Self> {
        Self::connect_mode(adb, device, false)
    }

    fn connect_mode(adb: &Adb, device: &Device, task: bool) -> Result<Self> {
        device.verify(adb)?;
        let lease = if task {
            crate::amoo_background::Lease::acquire(adb, device)?
        } else {
            None
        };
        let port: u16 = adb
            .run(&[
                "-s",
                &device.transport,
                "forward",
                "tcp:0",
                "localabstract:agentbus",
            ])?
            .trim()
            .parse()
            .context("无法建立 Amoo 后台连接")?;
        let forward = Forward {
            adb: adb.clone(),
            serial: device.transport.clone(),
            port: format!("tcp:{port}"),
        };
        let stream =
            TcpStream::connect_timeout(&([127, 0, 0, 1], port).into(), Duration::from_secs(3))
                .context("无法连接 Amoo 后台服务，请检查手机连接")?;
        stream.set_write_timeout(Some(Duration::from_secs(5)))?;
        let mut bus = Self {
            _lease: lease,
            stream,
            _forward: forward,
            device: device.clone(),
            version: String::new(),
            pending_events: Vec::new(),
        };
        let hello = bus
            .call_with_timeout(
                "connect",
                json!({"minProtocol":3,"maxProtocol":3,"role":"channel",
            "client":{"id":format!("stepx-desktop-{:016x}",rand::random::<u64>()),
                "displayName":"StepX Desktop","version":"1.0.0","platform":"linux"}}),
                if task {
                    REQUEST_TIMEOUT
                } else {
                    Duration::from_secs(3)
                },
            )
            .with_context(|| crate::amoo_background::connection_failure_message(adb, device))?;
        if hello["protocol"] != 3 {
            bail!("当前 Amoo 后台协议不受支持");
        }
        bus.version = hello["server"]["version"].as_str().unwrap_or("").into();
        if let Some(lease) = bus._lease.as_ref() {
            // This acknowledges only the real protocol handshake. Task completion
            // continues to require its matching native reply and final run state.
            let _ = lease.mark_connected();
        }
        Ok(bus)
    }

    pub fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        self.call_with_timeout(method, params, REQUEST_TIMEOUT)
    }

    /// The native bus routes this event to its own registered ability node.
    /// Keep the API limited to selected images already staged by StepX; it is not
    /// a general command transport or a cloud-account node selector.
    pub fn upload_local_images(&mut self, paths: &[String]) -> Result<Value> {
        let id = format!("stepx-upload-{:032x}", rand::random::<u128>());
        let body = serde_json::to_vec(&local_image_upload_frame(&id, paths)?)?;
        if body.len() > MAX_FRAME {
            bail!("图片上传请求过大");
        }
        self.stream.write_all(&(body.len() as u32).to_be_bytes())?;
        self.stream.write_all(&body)?;
        let deadline = Instant::now() + Duration::from_secs(115);
        loop {
            let response = read_frame(&mut self.stream, deadline)
                .context("手机本机图片上传未确认完成；附件已保留，请勿重复提交")?;
            if response.get("type").and_then(Value::as_str) == Some("event") {
                if let Some(delta) = parse_stream_event(&response) {
                    self.pending_events.push(delta);
                }
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
        let deadline = Instant::now() + timeout;
        // A partial write is ambiguous: never classify it as safe to resubmit.
        if self
            .stream
            .write_all(&(body.len() as u32).to_be_bytes())
            .is_err()
            || self.stream.write_all(&body).is_err()
        {
            return Ok(LocalInvokeResult {
                id,
                end: LocalInvokeEnd::Disconnected,
            });
        }
        loop {
            match read_local_frame(&mut self.stream, deadline, cancel) {
                LocalInvokeEnd::Reply(response) => {
                    if response.get("type").and_then(Value::as_str) == Some("event") {
                        if let Some(delta) = parse_stream_event(&response) {
                            self.pending_events.push(delta);
                        }
                    }
                    if response["type"] == "req"
                        && response["method"] == "node.invoke.result"
                        && response["params"]["id"].as_str() == Some(&id)
                    {
                        return Ok(LocalInvokeResult {
                            id,
                            end: LocalInvokeEnd::Reply(response["params"].clone()),
                        });
                    }
                }
                end => return Ok(LocalInvokeResult { id, end }),
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
        // AgentBus multiplexes clients upstream; IDs must remain unique across reconnects.
        let id = format!("stepx-{:032x}", rand::random::<u128>());
        let body =
            serde_json::to_vec(&json!({"type":"req","id":id,"method":method,"params":params}))?;
        if body.len() > MAX_FRAME {
            bail!("Amoo 请求过大");
        }
        self.stream.write_all(&(body.len() as u32).to_be_bytes())?;
        self.stream.write_all(&body)?;
        let deadline = Instant::now() + timeout;
        loop {
            let response = read_frame(&mut self.stream, deadline)?;
            if response.get("type").and_then(Value::as_str) == Some("event") {
                if let Some(delta) = parse_stream_event(&response) {
                    self.pending_events.push(delta);
                }
            }
            if response["id"].as_str() != Some(&id) {
                continue;
            }
            if response["ok"].as_bool() != Some(true) {
                let code = response["error"]["code"].as_str().unwrap_or("UNKNOWN");
                let message = response["error"]["message"]
                    .as_str()
                    .unwrap_or("请求未被接受");
                bail!("Amoo 后台请求失败（{code}）：{message}");
            }
            if response["payload"]["ok"].as_bool() == Some(false) {
                bail!("Amoo 后台未完成请求");
            }
            return response
                .get("payload")
                .cloned()
                .context("Amoo 后台响应缺少 payload");
        }
    }

    pub fn poll_stream_events(&mut self, wait: Duration) -> Result<Vec<StreamDelta>> {
        let mut out = std::mem::take(&mut self.pending_events);
        let polled = poll_stream_events_from_stream(&mut self.stream, wait, !out.is_empty())?;
        out.extend(polled);
        Ok(out)
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
        let scope_before_res = crate::amoo_usage::account_scope(&self._forward.adb, &self.device);
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
                crate::amoo_usage::account_scope(&self._forward.adb, &self.device)
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
        if key.is_empty() || text.trim().is_empty() {
            bail!("请先创建会话并输入消息");
        }
        let mut context = cached_amoo_context(&self._forward.adb, &self.device)?;
        let run_id = format!("stepx-{:032x}", rand::random::<u128>());
        context
            .as_object_mut()
            .context("Amoo 设备信息无效")?
            .remove("ok");
        context["session_id"] = json!(key);
        context["msg_id"] = json!(&run_id);
        context["channelMessageId"] = json!(&run_id);
        context["trace_id"] = json!(format!("tr-{run_id}"));
        let payload = self
            .call(
                "chat.send",
                json!({"sessionId":key,"text":text,"stepclaw":context,
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

pub(crate) fn poll_stream_events_from_stream(
    stream: &mut TcpStream,
    wait: Duration,
    already_have_events: bool,
) -> Result<Vec<StreamDelta>> {
    let mut out = Vec::new();
    for _ in 0..16 {
        let first_wait = if out.is_empty() && !already_have_events {
            wait.max(Duration::from_millis(5))
        } else {
            Duration::from_millis(5)
        };
        stream.set_read_timeout(Some(first_wait))?;
        let mut header = [0u8; 4];
        match stream.read(&mut header[..1]) {
            Ok(0) => bail!("Amoo 后台连接已关闭"),
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
            Err(err) => return Err(err.into()),
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
            if let Some(delta) = parse_stream_event(&frame) {
                out.push(delta);
            }
        }
    }
    Ok(out)
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
        assert_eq!(args["params"]["app_name"], "设置");
        assert_eq!(args["params"].as_object().unwrap().len(), 2);
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
}
