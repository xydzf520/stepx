//! File handoff over the selected, already-authorized ADB transport.
use crate::{
    adb::{shell_quote, Adb},
    device::Device,
};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
const BASE: &str = "/sdcard/Android/data/com.stepx.companion.app/files";
const LIMIT: u64 = 256 * 1024 * 1024;
const CHECKED_BASE_TTL: Duration = Duration::from_secs(30);
static RECEIVE_LOCK: Mutex<()> = Mutex::new(());
static ID_LOCK: Mutex<()> = Mutex::new(());
static CHECKED_BASE_CACHE: OnceLock<Mutex<HashMap<(String, String), (Instant, String)>>> =
    OnceLock::new();
static DESKTOP_ID_CACHE: OnceLock<Mutex<Option<(PathBuf, String)>>> = OnceLock::new();
static RECEIVED_OUTBOX_CACHE: OnceLock<Mutex<HashSet<(PathBuf, String, String)>>> = OnceLock::new();

fn checked_base_cache() -> &'static Mutex<HashMap<(String, String), (Instant, String)>> {
    CHECKED_BASE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn desktop_id_cache() -> &'static Mutex<Option<(PathBuf, String)>> {
    DESKTOP_ID_CACHE.get_or_init(|| Mutex::new(None))
}

fn received_outbox_cache() -> &'static Mutex<HashSet<(PathBuf, String, String)>> {
    RECEIVED_OUTBOX_CACHE.get_or_init(|| Mutex::new(HashSet::new()))
}

fn remember_received_outbox(key: (PathBuf, String, String)) {
    if let Ok(mut guard) = received_outbox_cache().lock() {
        if guard.len() >= 2048 {
            guard.clear();
        }
        guard.insert(key);
    }
}

pub(crate) fn uuid() -> String {
    let h = format!("{:032x}", rand::random::<u128>());
    format!(
        "{}-{}-{}-{}-{}",
        &h[..8],
        &h[8..12],
        &h[12..16],
        &h[16..20],
        &h[20..]
    )
}
fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.trim() == name
        && !name.starts_with('.')
        && name.len() <= 220
        && !name
            .chars()
            .any(|c| c.is_control() || c == '/' || c == '\\')
        && !["manifest.json", "receipt.json"].contains(&name.to_ascii_lowercase().as_str())
        && !name.to_ascii_lowercase().ends_with(".json.tmp")
}
fn home() -> Result<PathBuf> {
    std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from)
        .context("找不到用户文件夹")
}
fn private_dir(path: &Path) -> Result<()> {
    if path.exists() {
        ensure!(
            !fs::symlink_metadata(path)?.file_type().is_symlink() && path.is_dir(),
            "传输目录无效"
        );
    }
    fs::create_dir_all(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
pub fn desktop_id() -> Result<String> {
    let _guard = ID_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .unwrap_or(home()?.join(".config"))
        .join("stepx-companion");
    let path = base.join("desktop-id");
    if let Ok(guard) = desktop_id_cache().lock() {
        if let Some((cached_base, cached_id)) = guard.as_ref() {
            if cached_base == &base
                && fs::symlink_metadata(&path)
                    .is_ok_and(|m| m.is_file() && !m.file_type().is_symlink())
            {
                return Ok(cached_id.clone());
            }
        }
    }
    private_dir(&base)?;
    match fs::symlink_metadata(&path) {
        Ok(metadata) => {
            ensure!(
                metadata.is_file() && !metadata.file_type().is_symlink(),
                "电脑身份文件无效"
            );
            let id = fs::read_to_string(&path).context("无法读取电脑身份文件")?;
            let trimmed = id.trim().to_string();
            ensure!(valid_id(&trimmed), "电脑身份文件无效");
            if let Ok(mut guard) = desktop_id_cache().lock() {
                *guard = Some((base, trimmed.clone()));
            }
            return Ok(trimmed);
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error).context("无法检测电脑身份文件"),
    }
    let id = uuid();
    let mut options = fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    match options.open(&path) {
        Ok(mut file) => {
            file.write_all(id.as_bytes())?;
            file.sync_all()?;
            if let Ok(mut guard) = desktop_id_cache().lock() {
                *guard = Some((base, id.clone()));
            }
            Ok(id)
        }
        Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => {
            ensure!(fs::symlink_metadata(&path)?.is_file(), "电脑身份文件无效");
            let id = fs::read_to_string(&path).context("无法读取并发创建的电脑身份文件")?;
            let trimmed = id.trim().to_string();
            ensure!(valid_id(&trimmed), "电脑身份文件无效");
            if let Ok(mut guard) = desktop_id_cache().lock() {
                *guard = Some((base, trimmed.clone()));
            }
            Ok(trimmed)
        }
        Err(e) => Err(e.into()),
    }
}
pub fn receive_directory() -> Result<PathBuf> {
    let path = crate::settings::downloads();
    if crate::settings::load().download_directory.is_some() {
        ensure!(
            path.is_dir(),
            "接收目录不可用，请在设置中重新选择；未改存到其他位置"
        );
    }
    if crate::settings::load().download_directory.is_none() {
        private_dir(&path)?;
    }
    Ok(path)
}
pub(crate) fn digest(path: &Path) -> Result<String> {
    let hash = crate::device::sha256_file(path).context("无法校验文件")?;
    ensure!(
        hash.len() == 64 && hash.bytes().all(|c| c.is_ascii_hexdigit()),
        "文件校验结果无效"
    );
    Ok(hash)
}
fn phone_now(adb: &Adb, device: &Device) -> Result<i64> {
    Ok(adb
        .shell(&device.transport, &["date", "+%s"])?
        .trim()
        .parse::<i64>()?
        * 1000)
}
fn json_write(adb: &Adb, device: &Device, path: &str, value: &Value) -> Result<()> {
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(&serde_json::to_vec(value)?)?;
    let staging = format!("{path}.{}.tmp", uuid());
    adb.transfer(&device.transport, "push", file.path(), Path::new(&staging))?;
    adb.shell(&device.transport, &["mv", "-f", &staging, path])?;
    Ok(())
}
fn checked_base(adb: &Adb, device: &Device) -> Result<()> {
    device.verify(adb)?;
    let cache_key = (device.transport.clone(), device.serial.clone());
    let expected_base = format!("{}:{BASE}", adb.path.display());
    if let Ok(guard) = checked_base_cache().lock() {
        if let Some((at, cached_base)) = guard.get(&cache_key) {
            if at.elapsed() < CHECKED_BASE_TTL && cached_base == &expected_base {
                return Ok(());
            }
        }
    }
    ensure!(
        adb.shell(
            &device.transport,
            &["pm", "path", "com.stepx.companion.app"]
        )?
        .contains("package:"),
        "请先安装 StepX伴侣"
    );
    let root = shell_quote(BASE);
    let out = adb.script(&device.transport, &format!("test ! -L {root} && test ! -L {root}/transfers && test ! -L {root}/transfers/inbox && test ! -L {root}/transfers/outbox && mkdir -p {root}/transfers/inbox {root}/transfers/outbox && echo ready"))?;
    ensure!(out.trim() == "ready", "手机传输目录无效");
    if let Ok(mut guard) = checked_base_cache().lock() {
        guard.insert(cache_key, (Instant::now(), expected_base));
    }
    Ok(())
}
pub fn heartbeat(adb: &Adb, device: &Device) -> Result<()> {
    checked_base(adb, device)?;
    let now = phone_now(adb, device)?;
    let name = std::env::var("HOSTNAME")
        .or_else(|_| std::env::var("COMPUTERNAME"))
        .unwrap_or_else(|_| "StepX Desktop".into());
    json_write(
        adb,
        device,
        &format!("{BASE}/desktop-link.json"),
        &json!({"protocol":1,
        "desktopId":desktop_id()?,"computerName":name,"updatedAtMs":now,"expiresAtMs":now+30000,"connected":true}),
    )
}
pub fn send_files(adb: &Adb, device: &Device, paths: &[PathBuf]) -> Result<usize> {
    ensure!(
        !paths.is_empty() && paths.len() <= 10,
        "每次请选择 1–10 个文件"
    );
    checked_base(adb, device)?;
    let desktop = desktop_id()?;
    // Validate the entire selection before starting transfers.
    for path in paths {
        let m = fs::symlink_metadata(path)?;
        ensure!(
            m.is_file() && m.len() <= LIMIT,
            "请选择 256 MB 以内的普通文件"
        );
        ensure!(
            path.file_name()
                .and_then(|n| n.to_str())
                .is_some_and(valid_name),
            "文件名无效或与传输记录重名，请重命名后发送"
        );
    }
    let mut count = 0;
    for path in paths {
        if count > 0 {
            device.verify(adb)?;
        }
        let id = uuid();
        let name = path.file_name().unwrap().to_str().unwrap();
        let size = fs::metadata(path)?.len();
        let hash = digest(path)?;
        let dir = format!("{BASE}/transfers/inbox/{id}");
        adb.shell(&device.transport, &["mkdir", &dir])?;
        let target = format!("{dir}/{name}");
        let operation = (|| -> Result<()> {
            adb.transfer(&device.transport, "push", path, Path::new(&target))?;
            let remote = adb.shell(&device.transport, &["sha256sum", &target])?;
            ensure!(
                remote.split_whitespace().next() == Some(&hash),
                "手机接收的文件校验失败"
            );
            ensure!(
                adb.shell(&device.transport, &["stat", "-c", "%s", &target])?
                    .trim()
                    .parse::<u64>()?
                    == size,
                "手机文件大小不匹配"
            );
            json_write(
                adb,
                device,
                &format!("{dir}/manifest.json"),
                &json!({"protocol":1,"id":id,"name":name,
                "size":size,"sha256":hash,"status":"ready","createdAt":phone_now(adb,device)?,
                "sourceDesktopId":desktop,"computerName":"StepX Desktop"}),
            )?;
            Ok(())
        })();
        if let Err(e) = operation {
            let _ = adb.shell(&device.transport, &["rm", "-rf", &dir]);
            bail!("已发送 {count} 个文件，下一项失败：{e}");
        }
        count += 1;
    }
    Ok(count)
}
/// Stable transfer ID permits retry after a lost acknowledgement without duplicates.
pub fn send_staged(
    adb: &Adb,
    device: &Device,
    path: &Path,
    id: &str,
    hash: &str,
    size: u64,
) -> Result<()> {
    ensure!(valid_id(id), "无效传输标识");
    checked_base(adb, device)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("无效文件名")?;
    ensure!(
        valid_name(name) && size <= LIMIT && fs::symlink_metadata(path)?.is_file(),
        "无效交付文件"
    );
    ensure!(
        fs::metadata(path)?.len() == size && digest(path)? == hash,
        "本地交付缓存已变化，停止发送"
    );
    let desktop = desktop_id()?;
    let dir = format!("{BASE}/transfers/inbox/{id}");
    let target = format!("{dir}/{name}");
    let manifest = format!("{dir}/manifest.json");
    if let Ok(m) = read_json(adb, device, &manifest) {
        ensure!(
            m["sourceDesktopId"].as_str() == Some(&desktop)
                && m["sha256"].as_str() == Some(hash)
                && m["name"].as_str() == Some(name),
            "传输标识已被其他文件占用"
        );
        if m["status"].as_str() == Some("ready")
            && adb
                .shell(&device.transport, &["sha256sum", &target])?
                .split_whitespace()
                .next()
                == Some(hash)
        {
            return Ok(());
        }
    }
    adb.script(
        &device.transport,
        &format!(
            "test ! -L {} && mkdir -p {} && test ! -L {}",
            shell_quote(&dir),
            shell_quote(&dir),
            shell_quote(&target)
        ),
    )?;
    adb.transfer(&device.transport, "push", path, Path::new(&target))?;
    ensure!(
        adb.shell(&device.transport, &["sha256sum", &target])?
            .split_whitespace()
            .next()
            == Some(hash),
        "手机交付文件校验失败"
    );
    json_write(
        adb,
        device,
        &manifest,
        &json!({"protocol":1,"id":id,"name":name,"size":size,"sha256":hash,"status":"ready","createdAt":phone_now(adb,device)?,"sourceDesktopId":desktop,"computerName":"StepX Companion · Amoo Work"}),
    )?;
    Ok(())
}

fn read_json(adb: &Adb, device: &Device, path: &str) -> Result<Value> {
    let p = shell_quote(path);
    let s = adb.script(
        &device.transport,
        &format!("test -f {p} && test ! -L {p} && head -c 32769 {p}"),
    )?;
    ensure!(s.len() <= 32768, "传输记录过大");
    serde_json::from_str(&s).context("传输记录无效")
}
pub fn receive_pending(adb: &Adb, device: &Device) -> Result<usize> {
    let _guard = RECEIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    checked_base(adb, device)?;
    let desktop = desktop_id()?;
    let root_dir = crate::settings::downloads();
    let use_received_cache = adb
        .path
        .file_name()
        .and_then(|n| n.to_str())
        .is_some_and(|n| n == "adb" || n.eq_ignore_ascii_case("adb.exe"));
    let mut count = 0;
    let mut errors = Vec::new();
    let listing = adb.shell(
        &device.transport,
        &["ls", "-1", &format!("{BASE}/transfers/outbox")],
    )?;
    for id in listing.lines().filter(|s| valid_id(s)).take(300) {
        if count >= 10 {
            break;
        }
        let cache_key = (root_dir.clone(), device.serial.clone(), id.to_string());
        if use_received_cache
            && received_outbox_cache()
                .lock()
                .is_ok_and(|guard| guard.contains(&cache_key))
        {
            continue;
        }
        let dir = format!("{BASE}/transfers/outbox/{id}");
        let qdir = shell_quote(&dir);
        if adb
            .script(
                &device.transport,
                &format!("test ! -L {qdir} && echo valid"),
            )?
            .trim()
            != "valid"
        {
            continue;
        }
        let manifest = match read_json(adb, device, &format!("{dir}/manifest.json")) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if manifest["targetDesktopId"] != desktop {
            continue;
        }
        let name = manifest["name"].as_str().unwrap_or("");
        let size = manifest["size"].as_u64().unwrap_or(u64::MAX);
        let hash = manifest["sha256"].as_str().unwrap_or("");
        if manifest["protocol"] != 1
            || manifest["id"] != id
            || manifest["status"] != "queued"
            || !valid_name(name)
            || size > LIMIT
            || hash.len() != 64
            || !hash.bytes().all(|c| c.is_ascii_hexdigit())
        {
            continue;
        }
        if let Ok(receipt) = read_json(adb, device, &format!("{dir}/receipt.json")) {
            if receipt["id"] == id
                && (receipt["status"] == "received" || receipt["status"] == "error")
            {
                if use_received_cache && receipt["status"] == "received" {
                    remember_received_outbox(cache_key);
                }
                continue;
            }
        }
        let received = (|| -> Result<()> {
            let source = format!("{dir}/{name}");
            let q = shell_quote(&source);
            ensure!(
                adb.script(
                    &device.transport,
                    &format!("test -f {q} && test ! -L {q} && stat -c %s {q}")
                )?
                .trim()
                .parse::<u64>()?
                    == size,
                "手机文件大小不匹配"
            );
            let directory = receive_directory()?.join(id);
            private_dir(&directory)?;
            let destination = directory.join(name);
            if destination.exists() {
                ensure!(
                    fs::symlink_metadata(&destination)?.is_file()
                        && digest(&destination)? == hash
                        && fs::metadata(&destination)?.len() == size,
                    "接收目录存在同名不同内容的文件"
                );
                return Ok(());
            }
            let temp = tempfile::NamedTempFile::new_in(&directory)?;
            adb.transfer(&device.transport, "pull", Path::new(&source), temp.path())?;
            ensure!(
                fs::metadata(temp.path())?.len() == size && digest(temp.path())? == hash,
                "文件校验失败，请重新发送"
            );
            temp.persist_noclobber(destination).map_err(|e| e.error)?;
            Ok(())
        })();
        let mut receipt = json!({"protocol":1,"id":id,"size":size,"sha256":hash,"receivedAt":phone_now(adb,device)?,"status":if received.is_ok(){"received"}else{"error"}});
        if let Err(e) = received {
            errors.push(format!("{name}：{e}"));
            receipt["error"] = Value::String(e.to_string());
        }
        json_write(adb, device, &format!("{dir}/receipt.json"), &receipt)?;
        if receipt["status"] == "received" {
            if use_received_cache {
                remember_received_outbox(cache_key);
            }
            count += 1;
        }
    }
    if !errors.is_empty() {
        bail!(
            "已接收 {count} 个文件，{} 个文件失败：{}；手机端可查看失败记录并重新发送。",
            errors.len(),
            errors[0]
        );
    }
    Ok(count)
}

#[derive(Clone, Debug)]
pub struct LinkEvent {
    pub message: String,
    pub error: bool,
}

pub struct Link {
    pub transport: String,
    pub serial: String,
    stop: Arc<AtomicBool>,
    events: mpsc::Receiver<LinkEvent>,
}
impl Link {
    pub fn start(adb: Adb, device: Device) -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, events) = mpsc::channel();
        for heartbeat_only in [true, false] {
            let (stop, adb, device, tx) = (stop.clone(), adb.clone(), device.clone(), tx.clone());
            thread::spawn(move || {
                let mut previous_error: Option<String> = None;
                while !stop.load(Ordering::Relaxed) {
                    let result = if heartbeat_only {
                        heartbeat(&adb, &device).map(|_| 0)
                    } else {
                        let result = receive_pending(&adb, &device);
                        match crate::work_sync::process(&adb, &device) {
                            Ok(n) if n > 0 => {
                                let _ = tx.send(LinkEvent {
                                    message: format!("Amoo Work 的 {n} 个成果已送达手机伴侣"),
                                    error: false,
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(LinkEvent {
                                    message: format!("成果等待重传：{e}"),
                                    error: true,
                                });
                            }
                            _ => {}
                        }
                        result
                    };
                    match result {
                        Ok(n) => {
                            let recovered = previous_error.take().is_some();
                            if n > 0 {
                                let _ = tx.send(LinkEvent {
                                    message: format!(
                                        "已收到手机发送的 {n} 个文件，保存在设置中的接收文件夹"
                                    ),
                                    error: false,
                                });
                            } else if recovered {
                                let _ = tx.send(LinkEvent {
                                    message: if heartbeat_only {
                                        "手机与电脑的文件连接已恢复"
                                    } else {
                                        "手机文件接收检测已恢复"
                                    }
                                    .into(),
                                    error: false,
                                });
                            }
                        }
                        Err(error) => {
                            let message = format!(
                                "{}：{error:#}",
                                if heartbeat_only {
                                    "伴侣连接检测失败"
                                } else {
                                    "手机文件接收失败"
                                }
                            );
                            if previous_error.as_ref() != Some(&message) {
                                let _ = tx.send(LinkEvent {
                                    message: message.clone(),
                                    error: true,
                                });
                                previous_error = Some(message);
                            }
                        }
                    }
                    for _ in 0..if heartbeat_only { 8 } else { 5 } {
                        if stop.load(Ordering::Relaxed) {
                            return;
                        }
                        thread::sleep(Duration::from_secs(1));
                    }
                }
            });
        }
        Self {
            transport: device.transport,
            serial: device.serial,
            stop,
            events,
        }
    }
    pub fn event(&self) -> Option<LinkEvent> {
        self.events.try_recv().ok()
    }
}
impl Drop for Link {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reject_unsafe_transfer_paths() {
        for s in [
            "",
            "../x",
            "/x",
            "x/y",
            "x\\y",
            "x\ny",
            ".hidden",
            "manifest.json",
            "receipt.json",
        ] {
            assert!(!valid_name(s), "{s}");
        }
        assert!(valid_name("工作截图 1.png"));
        assert!(!valid_name(&format!("{}.png", "图".repeat(73))));
        assert!(valid_name(&format!("{}.png", "图".repeat(72))));
        for s in [
            "Manifest.json",
            "x.json.tmp",
            "x.JSON.TMP",
            " file.txt",
            "file.txt ",
        ] {
            assert!(!valid_name(s));
        }
    }
    #[test]
    fn queue_ids_are_fixed_safe_names() {
        assert!(valid_id(&uuid()));
        for id in ["../x", "z2345678-1234-1234-1234-123456789012", "1234"] {
            assert!(!valid_id(id));
        }
    }
    #[test]
    fn streaming_sha256_matches_in_memory_sha256_across_chunk_boundaries() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sample.bin");
        let payload: Vec<u8> = (0..150_000).map(|i| (i % 251) as u8).collect();
        fs::write(&path, &payload).unwrap();
        let expected: String = crate::device::sha256(&payload)
            .iter()
            .map(|b| format!("{b:02x}"))
            .collect();
        assert_eq!(digest(&path).unwrap(), expected);
    }
}
