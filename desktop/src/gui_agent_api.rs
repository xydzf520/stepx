//! Local authenticated tool endpoint shared with Amoo's phone skill.
use crate::{
    adb::Adb,
    amoo_screen::ScreenEvent,
    device::Device,
    gui_agent::{self, Control},
    openclaw::GatewayWs,
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};

const MAX_CONCURRENT_CONTROL_HANDLERS: usize = 8;

struct ControlConnPermit(Arc<AtomicUsize>);
impl ControlConnPermit {
    fn try_acquire(counter: &Arc<AtomicUsize>) -> Option<Self> {
        loop {
            let cur = counter.load(Ordering::Acquire);
            if cur >= MAX_CONCURRENT_CONTROL_HANDLERS {
                return None;
            }
            if counter
                .compare_exchange_weak(cur, cur + 1, Ordering::AcqRel, Ordering::Acquire)
                .is_ok()
            {
                return Some(Self(counter.clone()));
            }
        }
    }
}
impl Drop for ControlConnPermit {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

fn has_valid_bearer_authorization(headers: &str, expected_token: &str) -> bool {
    headers.lines().skip(1).any(|line| {
        let Some((k, v)) = line.split_once(':') else {
            return false;
        };
        if !k.trim().eq_ignore_ascii_case("authorization") {
            return false;
        }
        let mut parts = v.trim().splitn(2, char::is_whitespace);
        let Some(scheme) = parts.next() else {
            return false;
        };
        let Some(tok) = parts.next().map(str::trim) else {
            return false;
        };
        scheme.eq_ignore_ascii_case("bearer") && tok == expected_token
    })
}

#[derive(Default)]
struct Task {
    active: bool,
    id: String,
    goal: String,
    status: String,
    result: Value,
    control: Control,
}
fn request(
    mut stream: TcpStream,
    token: &str,
    state: &Arc<Mutex<Task>>,
    adb: &Adb,
    device: &Arc<Mutex<Device>>,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
    stream.set_write_timeout(Some(Duration::from_secs(3)))?;
    let mut bytes = vec![];
    let mut byte = [0u8; 1];
    while bytes.len() < 8192 && !bytes.ends_with(b"\r\n\r\n") {
        stream.read_exact(&mut byte)?;
        bytes.push(byte[0]);
    }
    ensure!(bytes.ends_with(b"\r\n\r\n"), "headers too large");
    let headers = String::from_utf8(bytes)?;
    ensure!(
        headers.starts_with("POST /tool HTTP/1.1\r\n")
            && has_valid_bearer_authorization(&headers, token),
        "unauthorized"
    );
    let size: usize = headers
        .lines()
        .find_map(|s| {
            s.split_once(':')
                .filter(|(k, _)| k.trim().eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim())
        })
        .context("missing length")?
        .parse()?;
    ensure!(size <= 20000, "request too large");
    let mut body = vec![0; size];
    stream.read_exact(&mut body)?;
    let request: Value = serde_json::from_slice(&body)?;
    let response = (|| -> Result<Value> {
        let mut task = state.lock().unwrap_or_else(|p| p.into_inner());
        match request["op"].as_str().unwrap_or("") {
            "start" => {
                ensure!(!task.active, "已有 GUI 任务，请先查询状态或停止");
                let goal = request["goal"].as_str().context("缺少目标")?.to_string();
                let app = request["app"].as_str().context("缺少应用名称")?.to_string();
                crate::amoo_gui::validate_request(&app, &goal)?;
                let mut run_device = device.lock().unwrap_or_else(|p| p.into_inner()).clone();
                run_device.transport = run_device.resolve_active_transport(adb)?;
                *device.lock().unwrap_or_else(|p| p.into_inner()) = run_device.clone();
                let control = Control::default();
                let id = format!("{:032x}", rand::random::<u128>());
                *task = Task {
                    active: true,
                    id: id.clone(),
                    goal: goal.clone(),
                    status: "正在准备手机操作".into(),
                    control: control.clone(),
                    ..Default::default()
                };
                let state = state.clone();
                let adb = adb.clone();
                std::thread::spawn(move || {
                    let result = gui_agent::run(&adb, &run_device, &app, &goal, &control, |event| {
                        let message = match event {
                            ScreenEvent::Status(s) => Some(s),
                            ScreenEvent::Step {
                                number,
                                description,
                            } => Some(format!("第 {number} 步：{description}")),
                            _ => None,
                        };
                        if let Some(message) = message {
                            state.lock().unwrap_or_else(|p| p.into_inner()).status = message;
                        }
                    });
                    let mut task = state.lock().unwrap_or_else(|p| p.into_inner());
                    task.active = false;
                    task.result = match result {
                        Ok(v) => json!(v),
                        Err(e) => json!({"state":"Failed","message":e.to_string()}),
                    };
                });
                Ok(json!({"ok":true,"task":id,"state":"running"}))
            }
            "status" => Ok(json!({
                "ok": true,
                "task": task.id,
                "active": task.active,
                "goal": task.goal,
                "progress": task.status,
                "result": task.result
            })),
            "pause" | "resume" | "stop" | "takeover" => {
                ensure!(
                    task.active && request["task"] == task.id,
                    "任务已结束或标识不匹配"
                );
                if request["op"] == "stop" {
                    task.control.stop.store(true, Ordering::SeqCst);
                    task.control.pause.store(false, Ordering::SeqCst);
                } else {
                    task.control.pause.store(
                        request["op"] == "pause" || request["op"] == "takeover",
                        Ordering::SeqCst,
                    );
                }
                Ok(json!({"ok":true,"task":task.id,"requested":request["op"]}))
            }
            _ => anyhow::bail!("未知工具操作"),
        }
    })()
    .unwrap_or_else(|e| json!({"ok":false,"error":e.to_string()}));
    let body = response.to_string();
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        body.len(),
        body
    )?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiAgentApiHandle {
    pub port: u16,
    pub token: String,
    pub serial: String,
}

pub(crate) struct ServerEntry {
    pub(crate) handle: GuiAgentApiHandle,
    pub(crate) device: Arc<Mutex<Device>>,
    pub(crate) adb: Option<Adb>,
    pub(crate) stop_flag: Arc<AtomicBool>,
}

impl Drop for ServerEntry {
    fn drop(&mut self) {
        self.stop_flag.store(true, Ordering::SeqCst);
        let _ = TcpStream::connect(("127.0.0.1", self.handle.port));
        if let Some(ref adb) = self.adb {
            let dev = self.device.lock().unwrap_or_else(|p| p.into_inner()).clone();
            let transport = if !dev.transport.trim().is_empty() {
                dev.transport
            } else {
                dev.serial
            };
            if !transport.trim().is_empty() {
                let _ = adb.run_timeout(
                    &["-s", &transport, "reverse", "--remove", "tcp:18796"],
                    Duration::from_secs(2),
                );
            }
        }
    }
}

static SERVERS: std::sync::OnceLock<Mutex<std::collections::BTreeMap<String, ServerEntry>>> =
    std::sync::OnceLock::new();

pub(crate) fn ensure_started_in_registry<F>(
    registry: &Mutex<std::collections::BTreeMap<String, ServerEntry>>,
    device: &Device,
    adb: Option<Adb>,
    starter: F,
) -> Result<GuiAgentApiHandle>
where
    F: FnOnce(Arc<Mutex<Device>>, Arc<AtomicBool>) -> Result<(u16, String)>,
{
    let mut servers = registry.lock().unwrap_or_else(|p| p.into_inner());
    if let Some(existing) = servers.get_mut(&device.serial) {
        *existing.device.lock().unwrap_or_else(|p| p.into_inner()) = device.clone();
        if adb.is_some() {
            existing.adb = adb;
        }
        return Ok(existing.handle.clone());
    }
    let shared_device = Arc::new(Mutex::new(device.clone()));
    let stop_flag = Arc::new(AtomicBool::new(false));
    let (port, token) = starter(shared_device.clone(), stop_flag.clone())?;
    let handle = GuiAgentApiHandle {
        port,
        token,
        serial: device.serial.clone(),
    };
    servers.insert(
        device.serial.clone(),
        ServerEntry {
            handle: handle.clone(),
            device: shared_device,
            adb,
            stop_flag,
        },
    );
    Ok(handle)
}

/// Stops the local GUI Agent HTTP endpoint for `serial` and removes `adb reverse tcp:18796`.
pub fn stop_for_device(adb: &Adb, device: &Device) {
    if let Some(registry) = SERVERS.get() {
        let mut servers = registry.lock().unwrap_or_else(|p| p.into_inner());
        if let Some(mut entry) = servers.remove(&device.serial) {
            entry.adb = Some(adb.clone());
            drop(entry);
            return;
        }
    }
    let transport = if !device.transport.trim().is_empty() {
        device.transport.clone()
    } else {
        device.serial.clone()
    };
    if !transport.trim().is_empty() {
        let _ = adb.run_timeout(
            &["-s", &transport, "reverse", "--remove", "tcp:18796"],
            Duration::from_secs(2),
        );
    }
}

/// Shuts down all active GUI Agent HTTP endpoints and cleans up their `adb reverse` forwards.
pub fn shutdown_all() {
    if let Some(registry) = SERVERS.get() {
        registry.lock().unwrap_or_else(|p| p.into_inner()).clear();
    }
}

/// Ensures the local authenticated GUI Agent HTTP endpoint is started for `device`
/// and `adb reverse tcp:18796 tcp:{port}` is configured idempotently.
pub fn ensure_started(adb: &Adb, device: &Device) -> Result<GuiAgentApiHandle> {
    let active_transport = device.resolve_active_transport(adb)?;
    let mut resolved_device = device.clone();
    resolved_device.transport = active_transport.clone();
    let registry = SERVERS.get_or_init(Default::default);
    let handle = ensure_started_in_registry(
        registry,
        &resolved_device,
        Some(adb.clone()),
        |device_worker, stop_flag| {
            let listener = TcpListener::bind("127.0.0.1:0")?;
            let port = listener.local_addr()?.port();
            let token = format!(
                "{:032x}{:032x}",
                rand::random::<u128>(),
                rand::random::<u128>()
            );
            let state = Arc::new(Mutex::new(Task::default()));
            let adb_worker = adb.clone();
            let secret = token.clone();
            let active_conns = Arc::new(AtomicUsize::new(0));
            std::thread::spawn(move || {
                for stream in listener.incoming().flatten() {
                    if stop_flag.load(Ordering::Acquire) {
                        break;
                    }
                    let Some(permit) = ControlConnPermit::try_acquire(&active_conns) else {
                        let mut busy = stream;
                        let _ = busy.set_write_timeout(Some(Duration::from_millis(500)));
                        let _ = busy.write_all(
                            b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n",
                        );
                        continue;
                    };
                    let secret_c = secret.clone();
                    let state_c = state.clone();
                    let adb_c = adb_worker.clone();
                    let dev_c = device_worker.clone();
                    std::thread::spawn(move || {
                        let _permit = permit;
                        let _ = request(stream, &secret_c, &state_c, &adb_c, &dev_c);
                    });
                }
            });
            Ok((port, token))
        },
    )?;
    adb.run(&[
        "-s",
        &active_transport,
        "reverse",
        "tcp:18796",
        &format!("tcp:{}", handle.port),
    ])?;
    Ok(handle)
}

/// Installs a private skill with an ephemeral endpoint capability, never account credentials.
pub fn start(adb: &Adb, device: &Device) -> Result<()> {
    let handle = ensure_started(adb, device)?;
    let mut ws = GatewayWs::connect(adb, device)?;
    let status = ws.call("skills.status", json!({}))?;
    let workspace = status["workspaceDir"]
        .as_str()
        .context("手机未返回技能工作目录")?;
    let script = include_str!("gui_agent_skill_install.py");
    let result = ws.call(
        "stepclaw.system.run",
        json!({"command":["python3","-c",script,workspace,handle.token],"timeoutMs":15000}),
    )?;
    ensure!(result["exitCode"] == 0, "手机技能安装未完成");
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};

    #[test]
    fn ensure_started_in_registry_is_idempotent_per_device_serial() {
        let registry = Mutex::new(std::collections::BTreeMap::new());
        let calls = AtomicUsize::new(0);
        let captured = Mutex::new(None);

        let dev1_usb = Device {
            serial: "DEV001".into(),
            transport: "DEV001".into(),
            ..Default::default()
        };
        let first = ensure_started_in_registry(&registry, &dev1_usb, None, |shared, _stop| {
            calls.fetch_add(1, Ordering::SeqCst);
            *captured.lock().unwrap_or_else(|p| p.into_inner()) = Some(shared);
            Ok((19001, "tok-abc".into()))
        })
        .unwrap();
        assert_eq!(first.port, 19001);
        assert_eq!(first.token, "tok-abc");
        assert_eq!(first.serial, "DEV001");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let dev1_wifi = Device {
            serial: "DEV001".into(),
            transport: "192.0.2.44:40555".into(),
            ..Default::default()
        };
        let second = ensure_started_in_registry(&registry, &dev1_wifi, None, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((19002, "tok-other".into()))
        })
        .unwrap();
        assert_eq!(second, first);
        assert_eq!(calls.load(Ordering::SeqCst), 1);
        let updated_transport = captured
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .as_ref()
            .unwrap()
            .lock()
            .unwrap_or_else(|p| p.into_inner())
            .transport
            .clone();
        assert_eq!(updated_transport, "192.0.2.44:40555");

        let dev2 = Device {
            serial: "DEV002".into(),
            transport: "DEV002".into(),
            ..Default::default()
        };
        let other_dev = ensure_started_in_registry(&registry, &dev2, None, |_, _| {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((19003, "tok-dev2".into()))
        })
        .unwrap();
        assert_eq!(other_dev.port, 19003);
        assert_eq!(other_dev.serial, "DEV002");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }

    #[test]
    fn lowercase_authorization_header_and_nonblocking_concurrent_stop_work() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let state = Arc::new(Mutex::new(Task {
            id: "task-42".into(),
            active: true,
            goal: "查看设置".into(),
            status: "running".into(),
            result: Value::Null,
            control: Control::default(),
        }));
        let state_for_server = state.clone();
        let active_conns = Arc::new(AtomicUsize::new(0));
        let stop_server = Arc::new(AtomicBool::new(false));
        let stop_worker = stop_server.clone();
        std::thread::spawn(move || {
            let adb = Adb {
                path: std::path::PathBuf::from("adb"),
            };
            let dev = Arc::new(Mutex::new(Device::default()));
            for stream in listener.incoming().flatten() {
                if stop_worker.load(Ordering::Acquire) {
                    break;
                }
                let Some(permit) = ControlConnPermit::try_acquire(&active_conns) else {
                    continue;
                };
                let st = state_for_server.clone();
                let adb_c = adb.clone();
                let dev_c = dev.clone();
                std::thread::spawn(move || {
                    let _permit = permit;
                    let _ = request(stream, "sec-token", &st, &adb_c, &dev_c);
                });
            }
        });

        // Hold one slow socket open without sending headers.
        let _slow_socket = TcpStream::connect(("127.0.0.1", port)).unwrap();

        let send_ctrl = |body: &str| -> Value {
            let mut client = TcpStream::connect(("127.0.0.1", port)).unwrap();
            client
                .set_read_timeout(Some(Duration::from_secs(2)))
                .unwrap();
            let req = format!(
                "POST /tool HTTP/1.1\r\nHost: 127.0.0.1\r\nauthorization: Bearer sec-token\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                body.len(),
                body
            );
            client.write_all(req.as_bytes()).unwrap();
            let mut resp = String::new();
            client.read_to_string(&mut resp).unwrap();
            let (_, json_body) = resp.split_once("\r\n\r\n").unwrap();
            serde_json::from_str(json_body).unwrap()
        };

        let r1 = send_ctrl(r#"{"op":"takeover","task":"task-42"}"#);
        assert_eq!(r1["ok"], true);
        assert!(
            state
                .lock()
                .unwrap()
                .control
                .pause
                .load(Ordering::SeqCst)
        );

        let r2 = send_ctrl(r#"{"op":"stop","task":"task-42"}"#);
        assert_eq!(r2["ok"], true);
        let guard = state.lock().unwrap();
        assert!(guard.control.stop.load(Ordering::SeqCst));
        assert!(!guard.control.pause.load(Ordering::SeqCst));
        drop(guard);

        stop_server.store(true, Ordering::Release);
        let _ = TcpStream::connect(("127.0.0.1", port));
    }
}
