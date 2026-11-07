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
    sync::{Arc, Mutex},
    time::Duration,
};

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
    device: &Device,
) -> Result<()> {
    stream.set_read_timeout(Some(Duration::from_secs(3)))?;
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
            && headers
                .lines()
                .any(|s| s == format!("Authorization: Bearer {token}")),
        "unauthorized"
    );
    let size: usize = headers
        .lines()
        .find_map(|s| {
            s.split_once(':')
                .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                .map(|(_, v)| v.trim())
        })
        .context("missing length")?
        .parse()?;
    ensure!(size <= 20000, "request too large");
    let mut body = vec![0; size];
    stream.read_exact(&mut body)?;
    let request: Value = serde_json::from_slice(&body)?;
    let response=(||->Result<Value>{
        let mut task=state.lock().unwrap();
        match request["op"].as_str().unwrap_or("") {
            "start"=>{
                ensure!(!task.active,"已有 GUI 任务，请先查询状态或停止");
                let goal=request["goal"].as_str().context("缺少目标")?.to_string();let app=request["app"].as_str().context("缺少应用名称")?.to_string();
                crate::amoo_gui::validate_request(&app,&goal)?;
                let control=Control::default();let id=format!("{:032x}",rand::random::<u128>());
                *task=Task{active:true,id:id.clone(),goal:goal.clone(),status:"正在准备手机操作".into(),control:control.clone(),..Default::default()};
                let state=state.clone();let adb=adb.clone();let device=device.clone();
                std::thread::spawn(move||{
                    let result=gui_agent::run(&adb,&device,&app,&goal,&control,|event|{
                        let message=match event{ScreenEvent::Status(s)=>Some(s),ScreenEvent::Step{number,description}=>Some(format!("第 {number} 步：{description}")),_=>None};
                        if let Some(message)=message{state.lock().unwrap().status=message;}
                    });
                    let mut task=state.lock().unwrap();task.active=false;task.result=match result{Ok(v)=>json!(v),Err(e)=>json!({"state":"Failed","message":e.to_string()})};
                });
                Ok(json!({"ok":true,"task":id,"state":"running"}))
            },
            "status"=>Ok(json!({"ok":true,"task":task.id,"active":task.active,"goal":task.goal,"progress":task.status,"result":task.result})),
            "pause"|"resume"|"stop"=>{
                ensure!(task.active&&request["task"]==task.id,"任务已结束或标识不匹配");
                use std::sync::atomic::Ordering;
                if request["op"]=="stop"{task.control.stop.store(true,Ordering::Relaxed);}else{task.control.pause.store(request["op"]=="pause",Ordering::Relaxed);}
                Ok(json!({"ok":true,"task":task.id,"requested":request["op"]}))
            },_=>anyhow::bail!("未知工具操作")
        }
    })().unwrap_or_else(|e|json!({"ok":false,"error":e.to_string()}));
    let body = response.to_string();
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body)?;
    Ok(())
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GuiAgentApiHandle {
    pub port: u16,
    pub token: String,
    pub serial: String,
}

static SERVERS: std::sync::OnceLock<Mutex<std::collections::BTreeMap<String, GuiAgentApiHandle>>> =
    std::sync::OnceLock::new();

pub(crate) fn ensure_started_in_registry<F>(
    registry: &Mutex<std::collections::BTreeMap<String, GuiAgentApiHandle>>,
    serial: &str,
    starter: F,
) -> Result<GuiAgentApiHandle>
where
    F: FnOnce() -> Result<(u16, String)>,
{
    let mut servers = registry.lock().unwrap();
    if let Some(existing) = servers.get(serial) {
        return Ok(existing.clone());
    }
    let (port, token) = starter()?;
    let handle = GuiAgentApiHandle {
        port,
        token,
        serial: serial.to_string(),
    };
    servers.insert(serial.to_string(), handle.clone());
    Ok(handle)
}

/// Ensures the local authenticated GUI Agent HTTP endpoint is started for `device`
/// and `adb reverse tcp:18796 tcp:{port}` is configured idempotently.
pub fn ensure_started(adb: &Adb, device: &Device) -> Result<GuiAgentApiHandle> {
    let registry = SERVERS.get_or_init(Default::default);
    let handle = ensure_started_in_registry(registry, &device.serial, || {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let port = listener.local_addr()?.port();
        let token = format!(
            "{:032x}{:032x}",
            rand::random::<u128>(),
            rand::random::<u128>()
        );
        let state = Arc::new(Mutex::new(Task::default()));
        let adb_worker = adb.clone();
        let device_worker = device.clone();
        let secret = token.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming().flatten() {
                let _ = request(stream, &secret, &state, &adb_worker, &device_worker);
            }
        });
        Ok((port, token))
    })?;
    adb.run(&[
        "-s",
        &device.transport,
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

        let first = ensure_started_in_registry(&registry, "DEV001", || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((19001, "tok-abc".into()))
        })
        .unwrap();
        assert_eq!(first.port, 19001);
        assert_eq!(first.token, "tok-abc");
        assert_eq!(first.serial, "DEV001");
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let second = ensure_started_in_registry(&registry, "DEV001", || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((19002, "tok-other".into()))
        })
        .unwrap();
        assert_eq!(second, first);
        assert_eq!(calls.load(Ordering::SeqCst), 1);

        let other_dev = ensure_started_in_registry(&registry, "DEV002", || {
            calls.fetch_add(1, Ordering::SeqCst);
            Ok((19003, "tok-dev2".into()))
        })
        .unwrap();
        assert_eq!(other_dev.port, 19003);
        assert_eq!(other_dev.serial, "DEV002");
        assert_eq!(calls.load(Ordering::SeqCst), 2);
    }
}
