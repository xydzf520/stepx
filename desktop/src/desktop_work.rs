//! Phone-selected sources → official Amoo account → bounded desktop tools → verified phone files.
//! Requests and receipts are durable; interrupted work is never silently submitted twice.
use crate::{adb::Adb, companion_link as link, device::Device, work};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};

const BASE: &str = "/sdcard/Android/data/com.stepx.companion.app/files/desktop-work";
pub use crate::amoo_background::{
    is_supported_firmware_fingerprint, EXPECTED_FINGERPRINTS, FP_12, FP_172,
};
#[allow(dead_code)]
const FIRMWARE: &str = FP_172;
static CLOSED: std::sync::Mutex<Option<std::collections::HashSet<String>>> =
    std::sync::Mutex::new(None);
static REQUEST_RETRIES: std::sync::OnceLock<
    std::sync::Mutex<std::collections::HashMap<String, (u32, Instant)>>,
> = std::sync::OnceLock::new();
static ACTIVE: AtomicBool = AtomicBool::new(false);

fn request_retries() -> &'static std::sync::Mutex<std::collections::HashMap<String, (u32, Instant)>>
{
    REQUEST_RETRIES.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

pub fn clear_device_closed_cache(serial: &str) {
    let prefix = format!("{serial}:");
    if let Ok(mut guard) = CLOSED.lock() {
        if let Some(set) = guard.as_mut() {
            set.retain(|key| !key.starts_with(&prefix));
        }
    }
    if let Ok(mut guard) = request_retries().lock() {
        guard.retain(|key, _| !key.starts_with(&prefix));
    }
}

fn should_skip_transient_retry(cache_key: &str, now: Instant) -> bool {
    request_retries()
        .lock()
        .ok()
        .and_then(|guard| guard.get(cache_key).map(|(_, not_before)| now < *not_before))
        .unwrap_or(false)
}

fn note_transient_read_error(cache_key: &str, now: Instant) {
    if let Ok(mut guard) = request_retries().lock() {
        if guard.len() >= 2048 {
            guard.retain(|_, (_, not_before)| now < *not_before);
        }
        let next_attempts = guard.get(cache_key).map(|(n, _)| n + 1).unwrap_or(1);
        if next_attempts >= 5 {
            guard.insert(cache_key.to_owned(), (0, now + Duration::from_secs(60)));
        } else {
            let backoff = Duration::from_secs(1u64 << (next_attempts - 1).min(3));
            guard.insert(cache_key.to_owned(), (next_attempts, now + backoff));
        }
    }
}

fn clear_transient_retry(cache_key: &str) {
    if let Ok(mut guard) = request_retries().lock() {
        guard.remove(cache_key);
    }
}

fn remote_receipt_exists(adb: &Adb, transport: &str, remote: &str) -> bool {
    let receipt = crate::adb::shell_quote(&format!("{remote}/receipt.json"));
    adb.script(
        transport,
        &format!("test -f {receipt} && test ! -L {receipt} && echo yes"),
    )
    .is_ok_and(|out| out.trim() == "yes")
}

/// Used by the existing updater so a phone-delegated job cannot be interrupted
/// merely because its conversation is not the selected desktop view.
pub fn is_running() -> bool {
    ACTIVE.load(Ordering::SeqCst)
}
struct Lease;
impl Drop for Lease {
    fn drop(&mut self) {
        ACTIVE.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Deserialize, Serialize)]
struct Request {
    protocol: u32,
    id: String,
    #[serde(rename = "desktopId")]
    desktop: String,
    goal: String,
    files: Vec<String>,
    firmware: String,
    #[serde(default, alias = "accountScope", skip_serializing_if = "Option::is_none")]
    account: Option<String>,
}
#[derive(Clone, Serialize, Deserialize)]
struct Output {
    id: String,
    name: String,
    path: PathBuf,
    sha256: String,
    size: u64,
    delivered: bool,
}
#[derive(Clone, Serialize, Deserialize)]
struct State {
    #[serde(default)]
    created_ms: Option<i64>,
    #[serde(default)]
    updated_ms: Option<i64>,
    task: String,
    #[serde(default)]
    device: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    account: Option<String>,
    #[serde(default)]
    workspace: Option<PathBuf>,
    #[serde(rename = "desktopId")]
    desktop: String,
    phase: String,
    detail: String,
    sequence: u64,
    #[serde(default)]
    session: Option<String>,
    #[serde(default)]
    outputs: Vec<Output>,
}
fn save(dir: &Path, state: &mut State) -> Result<()> {
    state.updated_ms = Some(chrono::Utc::now().timestamp_millis());
    state.sequence += 1;
    let tmp = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(tmp.as_file(), state)?;
    tmp.as_file().sync_all()?;
    tmp.persist(dir.join("state.json"))?;
    Ok(())
}
fn valid(r: &Request, desktop: &str) -> bool {
    r.protocol == 1
        && link::valid_id(&r.id)
        && r.desktop == desktop
        && is_supported_firmware_fingerprint(&r.firmware)
        && !r.goal.trim().is_empty()
        && r.goal.chars().count() <= 4000
        && !r.files.is_empty()
        && r.files.len() <= 10
        && r.files.iter().all(|f| link::valid_id(f))
        && r.files
            .iter()
            .collect::<std::collections::HashSet<_>>()
            .len()
            == r.files.len()
}
fn folder(id: &str) -> Result<PathBuf> {
    let path = crate::resources::user_state()
        .join("stepx-companion/phone-work")
        .join(id);
    fs::create_dir_all(&path)?;
    ensure!(
        !fs::symlink_metadata(&path)?.file_type().is_symlink(),
        "任务缓存不能是链接"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
fn public_receipt(state: &State) -> Value {
    json!({"protocol":1,"task":state.task,"desktopId":state.desktop,"phase":state.phase,
        "detail":state.detail,"sequence":state.sequence,"session":state.session,
        "files":state.outputs.iter().map(|o|json!({"id":o.id,"name":o.name,"size":o.size,"sha256":o.sha256,"delivered":o.delivered})).collect::<Vec<_>>()})
}
pub fn poll(adb: &Adb, device: &Device) -> Result<()> {
    let desktop = link::desktop_id()?;
    let transport = device.resolve_active_transport(adb)?;
    let listing = adb.script(
        &transport,
        &format!(
            "if test -d {BASE} && test ! -L {BASE}; then \
               now=$(date +%s 2>/dev/null || echo 0); pruned=0; \
               if [ \"$now\" -gt 604800 ]; then \
                 cutoff=$((now - 604800)); \
                 for r in {BASE}/*/receipt.json; do \
                   [ -f \"$r\" ] && [ ! -L \"$r\" ] || continue; \
                   d=${{r%/receipt.json}}; id=${{d##*/}}; \
                   [ -d \"$d\" ] && [ ! -L \"$d\" ] && [ \"${{#id}}\" -eq 36 ] || continue; \
                   ts=$(stat -c %Y \"$r\" 2>/dev/null || echo \"$now\"); \
                   if [ \"$ts\" -gt 0 ] && [ \"$ts\" -le \"$cutoff\" ]; then \
                     rm -rf -- \"$d\" && echo \"PRUNED:$id\"; \
                     pruned=$((pruned + 1)); \
                     [ \"$pruned\" -ge 5 ] && break; \
                   fi; \
                 done; \
               fi; \
               ls -1 {BASE}; \
             fi"
        ),
    )?;
    for pruned in listing.lines().filter_map(|line| line.strip_prefix("PRUNED:")) {
        let key = format!("{}:{pruned}", device.serial);
        if let Ok(mut guard) = CLOSED.lock() {
            if let Some(set) = guard.as_mut() {
                set.remove(&key);
            }
        }
        clear_transient_retry(&key);
    }
    for id in listing
        .lines()
        .filter(|s| !s.starts_with("PRUNED:") && link::valid_id(s))
        .take(100)
    {
        let cache_key = format!("{}:{id}", device.serial);
        if CLOSED
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|set| set.contains(&cache_key))
        {
            continue;
        }
        let now = Instant::now();
        if should_skip_transient_retry(&cache_key, now) {
            continue;
        }
        let remote = format!("{BASE}/{id}");
        let raw = match link::read_json(adb, device, &format!("{remote}/request.json")) {
            Ok(v) => {
                clear_transient_retry(&cache_key);
                v
            }
            Err(_) => {
                if remote_receipt_exists(adb, &transport, &remote) {
                    clear_transient_retry(&cache_key);
                    CLOSED
                        .lock()
                        .unwrap()
                        .get_or_insert_with(Default::default)
                        .insert(cache_key);
                } else {
                    note_transient_read_error(&cache_key, now);
                }
                continue;
            }
        };
        let request: Request = match serde_json::from_value(raw) {
            Ok(v) => v,
            Err(_) => {
                if remote_receipt_exists(adb, &transport, &remote) {
                    CLOSED
                        .lock()
                        .unwrap()
                        .get_or_insert_with(Default::default)
                        .insert(cache_key);
                } else {
                    note_transient_read_error(&cache_key, now);
                }
                continue;
            }
        };
        if request.id != id || !valid(&request, &desktop) {
            if remote_receipt_exists(adb, &transport, &remote) {
                CLOSED
                    .lock()
                    .unwrap()
                    .get_or_insert_with(Default::default)
                    .insert(cache_key);
            } else {
                note_transient_read_error(&cache_key, now);
            }
            continue;
        }
        let dir = folder(id)?;
        let cancelled = link::read_json(adb, device, &format!("{remote}/cancel.json"))
            .ok()
            .is_some_and(|v| v["task"] == id);
        if cancelled {
            fs::write(dir.join("cancel"), b"cancel")?;
        }
        let state_path = dir.join("state.json");
        let resolved_account = || {
            request
                .account
                .clone()
                .filter(|s| !s.trim().is_empty())
                .or_else(|| crate::amoo_usage::account_scope(adb, device).ok().flatten())
        };
        let mut state: State = if state_path.exists() {
            let mut loaded: State = serde_json::from_slice(&fs::read(&state_path)?)?;
            if loaded.account.is_none() {
                loaded.account = resolved_account();
            }
            loaded
        } else {
            let file = tempfile::NamedTempFile::new_in(&dir)?;
            serde_json::to_writer(file.as_file(), &request)?;
            file.as_file().sync_all()?;
            file.persist(dir.join("request.json"))?;
            State {
                created_ms: Some(chrono::Utc::now().timestamp_millis()),
                updated_ms: None,
                task: id.into(),
                device: device.serial.clone(),
                account: resolved_account(),
                workspace: None,
                desktop: desktop.clone(),
                phase: "queued".into(),
                detail: "等待电脑处理".into(),
                sequence: 0,
                session: None,
                outputs: vec![],
            }
        };
        ensure!(
            state.task == id && state.desktop == desktop,
            "任务归属已变化"
        );
        if !state_path.exists() { save(&dir, &mut state)?; }
        if matches!(state.phase.as_str(), "running" | "cancelling") && !ACTIVE.load(Ordering::SeqCst) {
            state = serde_json::from_slice(&fs::read(&state_path)?)?;
            if matches!(state.phase.as_str(), "running" | "cancelling") {
                state.phase = "interrupted".into();
                state.detail =
                    "电脑重启中断了处理；已保存资料与成果，请在 Amoo 中继续原会话或重新提交".into();
                save(&dir, &mut state)?;
            }
        }
        if cancelled && matches!(state.phase.as_str(), "queued" | "awaiting_delivery") {
            state.phase = "cancelled".into();
            state.detail = "任务已取消，原资料及已经送达的成果保留".into();
            save(&dir, &mut state)?;
        }
        if state.phase == "awaiting_delivery" {
            for i in 0..state.outputs.len() {
                let o = &state.outputs[i];
                if o.delivered {
                    continue;
                }
                // Stable ID plus size/hash checks make a lost receipt safe to retry.
                if let Err(error) = link::send_staged(adb, device, &o.path, &o.id, &o.sha256, o.size) {
                    let detail = format!("成果保留在电脑，等待重传：{error}");
                    if state.detail != detail {
                        state.detail = detail;
                        save(&dir, &mut state)?;
                    }
                    break;
                }
                state.outputs[i].delivered = true;
                save(&dir, &mut state)?;
            }
            if !state.outputs.is_empty() && state.outputs.iter().all(|o| o.delivered) {
                state.phase = "completed".into();
                state.detail = "处理完成，成果已校验并送达手机。请在伴侣“文件”中查看".into();
                save(&dir, &mut state)?;
            }
        }
        let receipt_written = link::json_write(
            adb,
            device,
            &format!("{remote}/receipt.json"),
            &public_receipt(&state),
        )
        .is_ok();
        if ["completed", "cancelled", "interrupted", "failed"].contains(&state.phase.as_str()) {
            if receipt_written || remote_receipt_exists(adb, &transport, &remote) {
                CLOSED
                    .lock()
                    .unwrap()
                    .get_or_insert_with(Default::default)
                    .insert(cache_key);
            }
            continue;
        }
        if state.phase != "queued"
            || ACTIVE
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            continue;
        }
        if request
            .files
            .iter()
            .any(|file| !link::receive_directory().is_ok_and(|root| root.join(file).is_dir()))
        {
            ACTIVE.store(false, Ordering::SeqCst);
            for file in &request.files {
                let receipt = link::read_json(adb, device, &format!("/sdcard/Android/data/com.stepx.companion.app/files/transfers/outbox/{file}/receipt.json")).ok();
                if let Some(receipt) = receipt.filter(|r| r["status"] == "error") {
                    state.phase = "failed".into();
                    state.detail = format!("资料传输失败：{}。修复连接或资料后，可在手机点击重新尝试。", receipt["error"].as_str().unwrap_or("校验未通过"));
                    save(&dir, &mut state)?;
                    break;
                }
            }
            continue;
        }
        let setup = prepare(adb, device, &request, &dir);
        match setup {
            Ok((workspace, attachments)) => {
                state.workspace = Some(workspace.clone());
                state.phase = "running".into();
                state.detail = "Amoo 正在读取资料并安排电脑处理".into();
                if let Err(e) = save(&dir, &mut state) {
                    ACTIVE.store(false, Ordering::SeqCst);
                    return Err(e);
                }
                let (adb, device) = (adb.clone(), device.clone());
                std::thread::spawn(move || {
                    let _lease = Lease;
                    run(adb, device, request, dir, workspace, attachments, state);
                });
            }
            Err(error) => {
                ACTIVE.store(false, Ordering::SeqCst);
                state.phase = "failed".into();
                state.detail = format!("资料准备失败：{error}");
                save(&dir, &mut state)?;
            }
        }
    }
    Ok(())
}
fn prepare(adb: &Adb, device: &Device, r: &Request, dir: &Path) -> Result<(PathBuf, Vec<PathBuf>)> {
    let transport = device.resolve_active_transport(adb)?;
    ensure!(
        is_supported_firmware_fingerprint(
            &adb.shell(&transport, &["getprop", "ro.build.fingerprint"])?
        ),
        "固件已更新，请重新验证 MCP 后再处理"
    );
    let root = crate::settings::load()
        .work_directory
        .unwrap_or_else(|| crate::settings::downloads().join("Amoo 工作"));
    fs::create_dir_all(&root)?;
    let root = root.canonicalize()?;
    let workspace = root.join("手机任务").join(&r.id);
    fs::create_dir_all(&workspace)?;
    ensure!(workspace.canonicalize()?.starts_with(&root), "工作目录越界");
    let mut attachments = Vec::new();
    for file in &r.files {
        let remote=format!("/sdcard/Android/data/com.stepx.companion.app/files/transfers/outbox/{file}/manifest.json");
        let m = link::read_json(adb, device, &remote)?;
        ensure!(
            m["id"] == *file && m["workAllowed"] == true && m["targetDesktopId"] == r.desktop,
            "资料没有授权给此电脑"
        );
        let name = m["name"].as_str().context("资料没有文件名")?;
        ensure!(link::valid_name(name), "资料文件名无效");
        let source = link::receive_directory()?.join(file).join(name);
        ensure!(
            fs::symlink_metadata(&source)?.is_file()
                && fs::metadata(&source)?.len() == m["size"].as_u64().unwrap_or(u64::MAX),
            "资料尚未接收完整"
        );
        ensure!(
            link::digest(&source)? == m["sha256"].as_str().unwrap_or(""),
            "资料校验失败"
        );
        // Separate folders avoid overwriting two equally named phone documents.
        let to = workspace.join("资料").join(file);
        fs::create_dir_all(&to)?;
        let target = to.join(name);
        ensure!(
            to.canonicalize()?.starts_with(workspace.canonicalize()?) && !target.exists(),
            "任务资料目录已经存在，请重新提交新任务"
        );
        fs::copy(&source, &target)?;
        ensure!(
            link::digest(&target)? == link::digest(&source)?,
            "资料复制后校验失败"
        );
        attachments.push(target);
    }
    let request_file = tempfile::NamedTempFile::new_in(dir)?;
    serde_json::to_writer(request_file.as_file(), r)?;
    request_file.as_file().sync_all()?;
    request_file.persist(dir.join("request.json"))?;
    Ok((workspace, attachments))
}
fn run(
    adb: Adb,
    device: Device,
    r: Request,
    dir: PathBuf,
    workspace: PathBuf,
    attachments: Vec<PathBuf>,
    mut state: State,
) {
    let result = (|| -> Result<()> {
        let task=work::start(adb,device,work::WorkRequest{prompt:format!("{}\n\n本任务来自手机明确选择的资料。请用桌面工具读取所附资料，实际生成可打开的交付文件；不要只口头给出结论，不改写原资料。交付完成后系统负责校验并送回手机，不要另外上传或发给联系人。",r.goal),workspace:workspace.clone(),session_key:None,model:None,attachments});
        let deadline = Instant::now() + Duration::from_secs(900);
        let mut stopping = false;
        let mut artifacts = std::collections::BTreeMap::new();
        loop {
            if !stopping && dir.join("cancel").exists() {
                task.cancel.store(true, Ordering::SeqCst);
                stopping = true;
                state.phase = "cancelling".into();
                state.detail = "正在停止电脑任务，等待执行器确认；已生成文件保留".into();
                save(&dir, &mut state)?;
            }
            ensure!(
                Instant::now() < deadline,
                "处理超过 15 分钟，已停止；资料和已生成文件保留"
            );
            let event = match task.events.recv_timeout(Duration::from_secs(2)) {
                Ok(v) => v,
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => continue,
                Err(e) => return Err(e.into()),
            };
            match event {
                work::WorkEvent::SessionReady(key) => state.session = Some(key),
                work::WorkEvent::Status(value) => state.detail = value.chars().take(300).collect(),
                work::WorkEvent::Tool(log) => {
                    state.detail = format!(
                        "{} · {}",
                        if log.success {
                            "已完成"
                        } else {
                            "正在调整"
                        },
                        log.tool
                    )
                }
                work::WorkEvent::Artifact(artifact) => remember_artifact(&mut artifacts, artifact),
                work::WorkEvent::Failed(_) if stopping => {
                    state.phase = "cancelled".into();
                    state.detail = "电脑执行器已停止，原始资料和已生成文件保留".into();
                    return Ok(());
                }
                work::WorkEvent::Failed(e) => anyhow::bail!("{e}"),
                work::WorkEvent::Cancelled => {
                    state.phase = "cancelled".into();
                    state.detail = "任务已停止".into();
                    return Ok(());
                }
                work::WorkEvent::Complete(_) if stopping => continue,
                work::WorkEvent::Complete(_) => break,
                _ => continue,
            }
            save(&dir, &mut state)?;
        }
        if dir.join("cancel").exists() {
            state.phase = "cancelled".into();
            state.detail = "电脑处理已结束，已按取消请求停止回传；成果保留在电脑".into();
            return Ok(());
        }
        // Tools may correct a generated document into a new output directory.
        // Deliver the last journaled version of each filename, never a superseded draft.
        let artifacts: Vec<_> = artifacts.into_values().collect();
        ensure!(
            !artifacts.is_empty() && artifacts.len() <= 10,
            "任务未生成可交付文件，或超过 10 个成果；请在电脑继续查看"
        );
        for a in artifacts {
            ensure!(
                fs::symlink_metadata(&a.path)?.is_file()
                    && a.path
                        .canonicalize()?
                        .starts_with(workspace.canonicalize()?),
                "成果路径无效"
            );
            let name = a
                .path
                .file_name()
                .and_then(|n| n.to_str())
                .context("成果文件名无效")?
                .to_string();
            ensure!(
                link::valid_name(&name) && fs::metadata(&a.path)?.len() <= 256 * 1024 * 1024,
                "成果文件过大或名称无效"
            );
            let id = link::uuid();
            let cache = dir.join(&id);
            fs::create_dir(&cache)?;
            let path = cache.join(&name);
            let sha256 = link::digest(&a.path)?;
            fs::copy(&a.path, &path)?;
            ensure!(
                link::digest(&path)? == sha256,
                "成果发生变化，请在电脑重新发送"
            );
            let size = fs::metadata(&path)?.len();
            state.outputs.push(Output {
                id,
                name,
                path,
                sha256,
                size,
                delivered: false,
            });
        }
        state.phase = "awaiting_delivery".into();
        state.detail = "电脑已完成处理，正在校验并送回手机；断线后会继续传输".into();
        Ok(())
    })();
    if let Err(e) = result {
        state.phase = "failed".into();
        state.detail = format!("{e:#}").chars().take(500).collect();
    }
    let _ = save(&dir, &mut state);
}
fn remember_artifact(files: &mut std::collections::BTreeMap<String, work::WorkArtifact>, artifact: work::WorkArtifact) {
    if let Some(name) = artifact.path.file_name().and_then(|s| s.to_str()) {
        files.insert(name.to_owned(), artifact);
    }
}
fn build_task_from_state(
    serial: &str,
    state: State,
    request: Request,
    updated: i64,
) -> crate::amoo_workspace::Task {
    use crate::amoo_workspace::{Executor, Intent, Output, Phase, Step, Task, Turn};
    let mut task = Task::new(Some(serial.into()), state.account.clone());
    task.id = format!("phone-work-{}", state.task);
    task.title = crate::amoo_workspace::clean_task_title(&request.goal);
    task.intent = Intent::Files;
    task.route = Some(Intent::Files);
    let total_outputs = state.outputs.len();
    let delivered_outputs = state.outputs.iter().filter(|o| o.delivered).count();
    task.route_note = match state.phase.as_str() {
        "awaiting_delivery" => "手机资料 · 待回传手机".into(),
        "queued" => "手机资料 · 排队等待处理".into(),
        "completed" if total_outputs > 0 && delivered_outputs == total_outputs => {
            "手机资料 · 已回传手机".into()
        }
        _ => "手机资料 · 电脑处理".into(),
    };
    let delivery_badge = if total_outputs > 0 {
        if delivered_outputs < total_outputs || state.phase == "awaiting_delivery" {
            Some(format!(
                "待回传手机 · 已回传 {delivered_outputs}/{total_outputs} 项成果"
            ))
        } else {
            Some(format!(
                "已回传 {delivered_outputs}/{total_outputs} 项成果"
            ))
        }
    } else if state.phase == "awaiting_delivery" {
        Some("待回传手机".to_string())
    } else if state.phase == "queued" {
        Some("排队等待处理".to_string())
    } else {
        None
    };
    let status_note = match delivery_badge {
        Some(badge) if state.detail.trim().is_empty() => badge,
        Some(badge) if state.detail.contains(&badge) => state.detail.clone(),
        Some(badge) => format!("{badge} · {}", state.detail),
        None => state.detail.clone(),
    };
    task.workspace = state.workspace;
    task.session = state.session;
    task.note = status_note.clone();
    task.turns = vec![
        Turn {
            at_ms: state.created_ms,
            user: true,
            text: request.goal,
        },
        Turn {
            at_ms: state.updated_ms,
            user: false,
            text: status_note.clone(),
        },
    ];
    task.updated = updated;
    let phase = match state.phase.as_str() {
        "completed" => Phase::Done,
        "failed" => Phase::Failed,
        "cancelled" => Phase::Stopped,
        "interrupted" => Phase::Unconfirmed,
        _ => Phase::Running,
    };
    task.steps = vec![Step {
        executor: Executor::Files,
        phase,
        note: status_note,
    }];
    task.outputs = state
        .outputs
        .into_iter()
        .map(|o| Output {
            path: o.path,
            name: o.name,
            kind: if o.delivered {
                "手机任务成果 · 已回传".into()
            } else {
                "手机任务成果 · 待回传".into()
            },
            lab_id: None,
        })
        .collect();
    task
}

/// Read by the background link thread, merged into the existing Amoo task UI.
pub fn snapshot(serial: &str) -> Vec<crate::amoo_workspace::Task> {
    snapshot_for_account(serial, None)
}

pub fn snapshot_for_account(
    serial: &str,
    account: Option<&str>,
) -> Vec<crate::amoo_workspace::Task> {
    let root = crate::resources::user_state().join("stepx-companion/phone-work");
    snapshot_from_root(&root, serial, account)
}

fn snapshot_from_root(
    root: &Path,
    serial: &str,
    account: Option<&str>,
) -> Vec<crate::amoo_workspace::Task> {
    use crate::amoo_workspace::Task;
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut ranked: Vec<(i64, Task)> = Vec::new();
    for entry in entries.flatten().take(4096) {
        let load = || -> Result<(i64, Task)> {
            let state_path = entry.path().join("state.json");
            let state: State = serde_json::from_slice(&fs::read(&state_path)?)?;
            ensure!(state.device == serial, "different phone");
            if let Some(expected) = account {
                ensure!(
                    state.account.as_deref().is_none_or(|a| a == expected),
                    "different account"
                );
            }
            let request: Request =
                serde_json::from_slice(&fs::read(entry.path().join("request.json"))?)?;
            let mtime_ms = fs::metadata(&state_path)?
                .modified()?
                .duration_since(std::time::UNIX_EPOCH)?
                .as_millis()
                .min(i64::MAX as u128) as i64;
            let sort_ms = state.updated_ms.or(state.created_ms).unwrap_or(mtime_ms);
            let updated = sort_ms.div_euclid(1000);
            Ok((sort_ms, build_task_from_state(serial, state, request, updated)))
        };
        if let Ok(item) = load() {
            ranked.push(item);
        }
    }
    ranked.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| b.1.id.cmp(&a.1.id)));
    ranked.truncate(100);
    ranked.into_iter().map(|(_, task)| task).collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    fn request() -> Request {
        Request {
            protocol: 1,
            id: link::uuid(),
            desktop: "computer".into(),
            goal: "分析所选表格".into(),
            files: vec![link::uuid()],
            firmware: FIRMWARE.into(),
            account: None,
        }
    }
    #[test]
    fn refuses_wrong_desktop_new_firmware_and_duplicate_sources() {
        let mut r = request();
        assert!(valid(&r, "computer"));
        r.firmware = FP_12.into();
        assert!(valid(&r, "computer"));
        assert!(!valid(&r, "other"));
        r.firmware = "new firmware".into();
        assert!(!valid(&r, "computer"));
        r.firmware = FP_172.into();
        r.files.push(r.files[0].clone());
        assert!(!valid(&r, "computer"));
    }
    #[test]
    fn valid_accepts_both_pandora_172_and_pandora_12() {
        assert_eq!(EXPECTED_FINGERPRINTS, [FP_172, FP_12]);
        assert_eq!(FIRMWARE, FP_172);
        let mut r = request();
        r.firmware = FP_172.into();
        assert!(valid(&r, "computer"));
        r.firmware = FP_12.into();
        assert!(valid(&r, "computer"));
        r.firmware = format!("  {FP_12}\n");
        assert!(valid(&r, "computer"));
    }
    #[test]
    fn public_receipt_does_not_leak_local_paths() {
        let s = State {
            created_ms: None,
            updated_ms: None,
            task: link::uuid(),
            device: "phone".into(),
            account: None,
            workspace: None,
            desktop: "computer".into(),
            phase: "completed".into(),
            detail: "完成".into(),
            sequence: 2,
            session: None,
            outputs: vec![Output {
                id: link::uuid(),
                name: "报告.txt".into(),
                path: "/private/report.txt".into(),
                sha256: "a".repeat(64),
                size: 3,
                delivered: true,
            }],
        };
        let v = public_receipt(&s);
        assert!(!v.to_string().contains("/private"));
        assert_eq!(v["files"][0]["delivered"], true);
    }
    #[test]
    fn delivers_corrected_document_not_superseded_draft() {
        let mut outputs = std::collections::BTreeMap::new();
        for path in ["draft/summary.xlsx", "report.txt", "final/summary.xlsx"] {
            remember_artifact(&mut outputs, work::WorkArtifact { path: path.into(), relative_path: path.into(), bytes: 10, kind: "file".into() });
        }
        assert_eq!(outputs.len(), 2);
        assert_eq!(outputs["summary.xlsx"].path, PathBuf::from("final/summary.xlsx"));
        assert!(outputs.contains_key("report.txt"));
    }
    #[test]
    fn build_task_from_state_surfaces_awaiting_delivery_and_output_progress() {
        let s = State {
            created_ms: Some(1000),
            updated_ms: Some(2000),
            task: "t1".into(),
            device: "phone".into(),
            account: Some("acct-a".into()),
            workspace: None,
            desktop: "computer".into(),
            phase: "awaiting_delivery".into(),
            detail: "已生成 2 项成果，等待手机连接回传".into(),
            sequence: 3,
            session: Some("sess-1".into()),
            outputs: vec![
                Output {
                    id: "o1".into(),
                    name: "报告.md".into(),
                    path: "/tmp/report.md".into(),
                    sha256: "a".repeat(64),
                    size: 12,
                    delivered: true,
                },
                Output {
                    id: "o2".into(),
                    name: "数据.xlsx".into(),
                    path: "/tmp/data.xlsx".into(),
                    sha256: "b".repeat(64),
                    size: 24,
                    delivered: false,
                },
            ],
        };
        let task = build_task_from_state("phone", s, request(), 2000);
        assert_eq!(task.id, "phone-work-t1");
        assert_eq!(task.route_note, "手机资料 · 待回传手机");
        assert!(task.note.contains("待回传手机 · 已回传 1/2 项成果"));
        assert_eq!(task.outputs[0].kind, "手机任务成果 · 已回传");
        assert_eq!(task.outputs[1].kind, "手机任务成果 · 待回传");
        assert!(task.visible(Some("phone"), Some("acct-a")));
        assert!(!task.visible(Some("phone"), Some("acct-b")));
    }
    #[test]
    fn snapshot_filters_device_and_account_and_sorts_descending_before_cap() {
        let dir = tempfile::tempdir().unwrap();
        for idx in 0..115u32 {
            let id = format!("00000000-0000-0000-0000-{idx:012}");
            let sub = dir.path().join(&id);
            fs::create_dir_all(&sub).unwrap();
            let mut r = request();
            r.id = id.clone();
            fs::write(sub.join("request.json"), serde_json::to_vec(&r).unwrap()).unwrap();
            let s = State {
                created_ms: Some(i64::from(idx) * 1000),
                updated_ms: Some(i64::from(idx) * 1000),
                task: id,
                device: if idx < 5 { "other-phone".into() } else { "phone".into() },
                account: Some(if idx == 114 { "acct-b".into() } else { "acct-a".into() }),
                workspace: None,
                desktop: "computer".into(),
                phase: "completed".into(),
                detail: "done".into(),
                sequence: 1,
                session: None,
                outputs: vec![],
            };
            fs::write(sub.join("state.json"), serde_json::to_vec(&s).unwrap()).unwrap();
        }
        let all_for_phone = snapshot_from_root(dir.path(), "phone", None);
        assert_eq!(all_for_phone.len(), 100);
        assert_eq!(
            all_for_phone[0].id,
            "phone-work-00000000-0000-0000-0000-000000000114"
        );
        assert_eq!(
            all_for_phone[99].id,
            "phone-work-00000000-0000-0000-0000-000000000015"
        );
        let acct_a = snapshot_from_root(dir.path(), "phone", Some("acct-a"));
        assert_eq!(acct_a.len(), 100);
        assert_eq!(
            acct_a[0].id,
            "phone-work-00000000-0000-0000-0000-000000000113"
        );
    }
    #[test]
    fn transient_read_errors_backoff_and_clear_device_closed_cache_resets() {
        let key = "serial-test:00000000-0000-0000-0000-000000000001";
        let now = Instant::now();
        clear_device_closed_cache("serial-test");
        for _ in 0..5 {
            note_transient_read_error(key, now);
        }
        assert!(should_skip_transient_retry(key, now + Duration::from_secs(30)));
        assert!(!should_skip_transient_retry(key, now + Duration::from_secs(61)));
        CLOSED
            .lock()
            .unwrap()
            .get_or_insert_with(Default::default)
            .insert(key.into());
        clear_device_closed_cache("serial-test");
        assert!(!should_skip_transient_retry(key, now));
        assert!(!CLOSED
            .lock()
            .unwrap()
            .as_ref()
            .is_some_and(|set| set.contains(key)));
    }
}
