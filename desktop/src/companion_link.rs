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
pub(crate) fn valid_id(id: &str) -> bool {
    id.len() == 36
        && id.bytes().enumerate().all(|(i, c)| {
            if [8, 13, 18, 23].contains(&i) {
                c == b'-'
            } else {
                c.is_ascii_hexdigit()
            }
        })
}
pub(crate) fn valid_name(name: &str) -> bool {
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
    let transport = device.resolve_active_transport(adb)?;
    Ok(adb
        .shell(&transport, &["date", "+%s"])?
        .trim()
        .parse::<i64>()?
        * 1000)
}
pub(crate) fn json_write(adb: &Adb, device: &Device, path: &str, value: &Value) -> Result<()> {
    let transport = device.resolve_active_transport(adb)?;
    let payload = serde_json::to_string(value)?;
    let staging = format!("{path}.{}.tmp", uuid());
    let q_staging = shell_quote(&staging);
    let q_path = shell_quote(path);
    let q_payload = shell_quote(&payload);
    adb.script(
        &transport,
        &format!("rm -f {q_path}.*.tmp && printf '%s' {q_payload} > {q_staging} && mv -f {q_staging} {q_path}"),
    )?;
    Ok(())
}
fn checked_base(adb: &Adb, device: &Device) -> Result<String> {
    let transport = device.resolve_active_transport(adb)?;
    let cache_key = (transport.clone(), device.serial.clone());
    let expected_base = format!("{}:{BASE}", adb.path.display());
    if let Ok(guard) = checked_base_cache().lock() {
        if let Some((at, cached_base)) = guard.get(&cache_key) {
            if at.elapsed() < CHECKED_BASE_TTL && cached_base == &expected_base {
                return Ok(transport);
            }
        }
    }
    let check_res = (|| -> Result<()> {
        ensure!(
            adb.shell(
                &transport,
                &["pm", "path", "com.stepx.companion.app"]
            )?
            .contains("package:"),
            "请先安装 StepX伴侣"
        );
        let root = shell_quote(BASE);
        let out = adb.script(&transport, &format!("test ! -L {root} && test ! -L {root}/transfers && test ! -L {root}/transfers/inbox && test ! -L {root}/transfers/outbox && mkdir -p {root}/transfers/inbox {root}/transfers/outbox && echo ready"))?;
        ensure!(out.trim() == "ready", "手机传输目录无效");
        Ok(())
    })();
    if let Err(err) = check_res {
        if crate::adb::is_data_transport_failure(&err) {
            crate::device::invalidate_verified_transport(&transport);
        }
        return Err(err);
    }
    if let Ok(mut guard) = checked_base_cache().lock() {
        guard.insert(cache_key, (Instant::now(), expected_base));
    }
    Ok(transport)
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
        "desktopId":desktop_id()?,"computerName":name,"updatedAtMs":now,"expiresAtMs":now+90000,"connected":true}),
    )
}
pub fn send_files(adb: &Adb, device: &Device, paths: &[PathBuf]) -> Result<usize> {
    ensure!(
        !paths.is_empty() && paths.len() <= 10,
        "每次请选择 1–10 个文件"
    );
    let mut transport = checked_base(adb, device)?;
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
            transport = device.resolve_active_transport(adb)?;
        }
        let id = uuid();
        let name = path
            .file_name()
            .and_then(|s| s.to_str())
            .context("文件名包含非 UTF-8 字符")?;
        let size = fs::metadata(path)?.len();
        let hash = digest(path)?;
        let dir = format!("{BASE}/transfers/inbox/{id}");
        adb.shell(&transport, &["mkdir", &dir])?;
        let target = format!("{dir}/{name}");
        let operation = (|| -> Result<()> {
            adb.transfer(&transport, "push", path, Path::new(&target))?;
            let remote = adb.shell(&transport, &["sha256sum", &target])?;
            ensure!(
                remote.split_whitespace().next() == Some(&hash),
                "手机接收的文件校验失败"
            );
            ensure!(
                adb.shell(&transport, &["stat", "-c", "%s", &target])?
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
            let _ = adb.shell(&transport, &["rm", "-rf", &dir]);
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
    send_staged_guarded(
        adb,
        device,
        path,
        id,
        hash,
        size,
        &AtomicBool::new(false),
        None,
    )
}

fn check_send_guard(
    cancel: &AtomicBool,
    expected_account: Option<&str>,
    read_account: impl FnOnce() -> Result<Option<String>>,
) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "已停止继续回传，已有文件与接收记录保留"
    );
    if let Some(expected) = expected_account {
        ensure!(!expected.trim().is_empty(), "账号尚未确认，未发布交付");
        ensure!(
            read_account()?.as_deref() == Some(expected),
            "手机账号已变化，未继续发布本次交付；已有记录保留"
        );
    }
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "已停止继续回传，已有文件与接收记录保留"
    );
    Ok(())
}

/// Guard every publication boundary. A push already in flight cannot be
/// revoked, so it targets an unadvertised pending file. Only a final guarded
/// commit makes the payload visible to the phone's normal inbox protocol.
pub fn send_staged_guarded(
    adb: &Adb,
    device: &Device,
    path: &Path,
    id: &str,
    hash: &str,
    size: u64,
    cancel: &AtomicBool,
    expected_account: Option<&str>,
) -> Result<()> {
    ensure!(
        !cancel.load(Ordering::Relaxed),
        "已停止继续回传，已有文件与接收记录保留"
    );
    let desktop = desktop_id()?;
    send_staged_with(adb, device, path, id, hash, size, &desktop, || {
        check_send_guard(cancel, expected_account, || {
            crate::amoo_usage::account_scope_fresh(adb, device)
        })
    })
}

fn send_staged_with(
    adb: &Adb,
    device: &Device,
    path: &Path,
    id: &str,
    hash: &str,
    size: u64,
    desktop: &str,
    mut guard: impl FnMut() -> Result<()>,
) -> Result<()> {
    guard()?;
    ensure!(valid_id(id), "无效传输标识");
    let transport = checked_base(adb, device)?;
    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .context("无效文件名")?;
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        valid_name(name) && size <= LIMIT && meta.is_file() && !meta.file_type().is_symlink(),
        "无效交付文件"
    );
    ensure!(
        meta.len() == size && digest(path)? == hash,
        "本地交付缓存已变化，停止发送"
    );
    let dir = format!("{BASE}/transfers/inbox/{id}");
    let target = format!("{dir}/{name}");
    let manifest = format!("{dir}/manifest.json");
    if let Ok(m) = read_json(adb, device, &manifest) {
        ensure!(
            m["sourceDesktopId"].as_str() == Some(desktop)
                && m["sha256"].as_str() == Some(hash)
                && m["name"].as_str() == Some(name),
            "传输标识已被其他文件占用"
        );
        if m["status"].as_str() == Some("ready")
            && adb
                .shell(&transport, &["sha256sum", &target])?
                .split_whitespace()
                .next()
                == Some(hash)
        {
            guard()?;
            return Ok(());
        }
    }
    adb.script(
        &transport,
        &format!(
            "test ! -L {} && mkdir -p {} && test ! -L {}",
            shell_quote(&dir),
            shell_quote(&dir),
            shell_quote(&target)
        ),
    )?;
    // The phone's TransferStore treats a valid inbox manifest as ready even if
    // its status says pending. Never advertise one until the final commit.
    let payload_pending = format!("{dir}/payload.{}.pending", uuid());
    let manifest_pending = format!("{dir}/manifest.{}.pending", uuid());
    guard()?;
    adb.transfer(&transport, "push", path, Path::new(&payload_pending))?;
    guard()?;
    ensure!(
        adb.shell(&transport, &["sha256sum", &payload_pending])?
            .split_whitespace()
            .next()
            == Some(hash),
        "手机交付文件校验失败，尚未发布"
    );
    let value = json!({"protocol":1,"id":id,"name":name,"size":size,"sha256":hash,"status":"ready",
        "createdAt":phone_now(adb,device)?,"sourceDesktopId":desktop,"computerName":"StepX Companion · Amoo Work"});
    let mut local_manifest = tempfile::NamedTempFile::new()?;
    local_manifest.write_all(&serde_json::to_vec(&value)?)?;
    guard()?;
    adb.transfer(
        &transport,
        "push",
        local_manifest.path(),
        Path::new(&manifest_pending),
    )?;
    guard()?;
    // No cancellation check after this publication: a late stop must not turn
    // a committed delivery into a false 'not sent' result. Both final names and
    // the ready manifest remain compatible with older phone clients.
    adb.script(&transport, &format!("set -e; test ! -L {dir}; test ! -L {target}; test ! -L {manifest}; mv -f {pending} {target}; mv -f {staged_manifest} {manifest}",
        dir=shell_quote(&dir),target=shell_quote(&target),manifest=shell_quote(&manifest),
        pending=shell_quote(&payload_pending),staged_manifest=shell_quote(&manifest_pending)))?;
    Ok(())
}

pub(crate) fn read_json(adb: &Adb, device: &Device, path: &str) -> Result<Value> {
    let transport = device.resolve_active_transport(adb)?;
    let p = shell_quote(path);
    let s = adb.script(
        &transport,
        &format!("test -f {p} && test ! -L {p} && head -c 32769 {p}"),
    )?;
    ensure!(s.len() <= 32768, "传输记录过大");
    serde_json::from_str(&s).context("传输记录无效")
}
fn unique_destination(
    directory: &Path,
    name: &str,
    expected_size: u64,
    expected_hash: &str,
) -> Result<Option<PathBuf>> {
    let primary = directory.join(name);
    match fs::symlink_metadata(&primary) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Some(primary)),
        Ok(meta) => {
            if meta.is_file()
                && fs::metadata(&primary).is_ok_and(|m| m.len() == expected_size)
                && digest(&primary).is_ok_and(|h| h == expected_hash)
            {
                return Ok(None);
            }
        }
        Err(e) => return Err(e.into()),
    }
    let path_obj = Path::new(name);
    let stem = path_obj
        .file_stem()
        .and_then(|s| s.to_str())
        .filter(|s| !s.is_empty())
        .unwrap_or(name);
    let ext = path_obj.extension().and_then(|e| e.to_str()).unwrap_or("");
    for idx in 1..=1000u32 {
        let candidate_name = if ext.is_empty() {
            format!("{stem} ({idx})")
        } else {
            format!("{stem} ({idx}).{ext}")
        };
        let candidate = directory.join(candidate_name);
        match fs::symlink_metadata(&candidate) {
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Some(candidate)),
            Ok(meta)
                if meta.is_file()
                    && fs::metadata(&candidate).is_ok_and(|m| m.len() == expected_size)
                    && digest(&candidate).is_ok_and(|h| h == expected_hash) =>
            {
                return Ok(None);
            }
            Ok(_) => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("接收目录同名文件过多，请清理后重试")
}

pub fn receive_pending(adb: &Adb, device: &Device) -> Result<usize> {
    receive_pending_cancelable(adb, device, &AtomicBool::new(false))
}

pub fn receive_pending_cancelable(
    adb: &Adb,
    device: &Device,
    stop: &AtomicBool,
) -> Result<usize> {
    if stop.load(Ordering::Acquire) {
        return Ok(0);
    }
    let mut transport = checked_base(adb, device)?;
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
        &transport,
        &["ls", "-1", &format!("{BASE}/transfers/outbox")],
    )?;
    for id in listing.lines().filter(|s| valid_id(s)).take(300) {
        if count >= 10 || stop.load(Ordering::Acquire) {
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
        transport = device.resolve_active_transport(adb)?;
        let dir = format!("{BASE}/transfers/outbox/{id}");
        let qdir = shell_quote(&dir);
        if adb
            .script(
                &transport,
                &format!("test ! -L {qdir} && echo valid"),
            )?
            .trim()
            != "valid"
        {
            if use_received_cache {
                remember_received_outbox(cache_key);
            }
            continue;
        }
        let manifest = match read_json(adb, device, &format!("{dir}/manifest.json")) {
            Ok(v) => v,
            Err(_) => {
                if use_received_cache {
                    remember_received_outbox(cache_key);
                }
                continue;
            }
        };
        if manifest["targetDesktopId"] != desktop {
            if use_received_cache {
                remember_received_outbox(cache_key);
            }
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
            if use_received_cache {
                remember_received_outbox(cache_key);
            }
            continue;
        }
        if let Ok(receipt) = read_json(adb, device, &format!("{dir}/receipt.json")) {
            if receipt["id"] == id
                && receipt["sha256"].as_str() == Some(hash)
                && receipt["size"].as_u64() == Some(size)
                && (receipt["status"] == "received"
                    || (receipt["status"] == "error" && receipt["retryAfterMs"].as_i64().unwrap_or(0) > phone_now(adb,device)?))
            {
                if use_received_cache && receipt["status"] == "received" {
                    remember_received_outbox(cache_key);
                }
                continue;
            }
        }
        if stop.load(Ordering::Acquire) {
            break;
        }
        let received = (|| -> Result<bool> {
            let source = format!("{dir}/{name}");
            let q = shell_quote(&source);
            ensure!(
                adb.script(
                    &transport,
                    &format!("test -f {q} && test ! -L {q} && stat -c %s {q}")
                )?
                .trim()
                .parse::<u64>()?
                    == size,
                "手机文件大小不匹配"
            );
            let directory = receive_directory()?.join(id);
            private_dir(&directory)?;
            let existing = directory.join(name);
            if fs::symlink_metadata(&existing).is_ok_and(|m| m.is_file())
                && fs::metadata(&existing).is_ok_and(|m| m.len() == size)
                && digest(&existing).is_ok_and(|h| h == hash)
            {
                return Ok(true);
            }
            if stop.load(Ordering::Acquire) {
                return Ok(false);
            }
            // Pull into a unique temp file WITHOUT holding `RECEIVE_LOCK` across
            // slow ADB network I/O; acquire `RECEIVE_LOCK` only briefly to pick
            // `unique_destination` and atomically rename into place.
            let temp = tempfile::Builder::new()
                .prefix(".stepx-recv-")
                .suffix(".part")
                .tempfile_in(&directory)?;
            adb.transfer(&transport, "pull", Path::new(&source), temp.path())?;
            ensure!(
                fs::metadata(temp.path())?.len() == size && digest(temp.path())? == hash,
                "文件校验失败，请重新发送"
            );
            let _guard = RECEIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(destination) = unique_destination(&directory, name, size, hash)? {
                temp.persist_noclobber(destination).map_err(|e| e.error)?;
            }
            Ok(true)
        })();
        match received {
            Ok(false) => break,
            Ok(true) => {
                let receipt = json!({
                    "protocol": 1,
                    "id": id,
                    "size": size,
                    "sha256": hash,
                    "receivedAt": phone_now(adb, device)?,
                    "status": "received",
                });
                json_write(adb, device, &format!("{dir}/receipt.json"), &receipt)?;
                if use_received_cache {
                    remember_received_outbox(cache_key);
                }
                count += 1;
            }
            Err(e) => {
                errors.push(format!("{name}：{e}"));
                let receipt = json!({
                    "protocol": 1,
                    "id": id,
                    "size": size,
                    "sha256": hash,
                    "receivedAt": phone_now(adb, device)?,
                    "status": "error",
                    "error": e.to_string(),
                    "retryAfterMs": phone_now(adb, device)? + 60_000,
                });
                json_write(adb, device, &format!("{dir}/receipt.json"), &receipt)?;
            }
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
    pub phone_work: Vec<crate::amoo_workspace::Task>,
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
                let mut previous_work_error: Option<String> = None;
                let mut previous_work=String::new();
                while !stop.load(Ordering::Acquire) {
                    let result = if heartbeat_only {
                        heartbeat(&adb, &device).map(|_| 0)
                    } else {
                        let result = receive_pending_cancelable(&adb, &device, &stop);
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        match crate::desktop_work::poll(&adb, &device) {
                            Err(e) => {
                                let message = format!("手机任务同步暂未完成：{e}");
                                if previous_work_error.as_ref() != Some(&message) {
                                    let _ = tx.send(LinkEvent { message: message.clone(), error: true, phone_work: vec![] });
                                    previous_work_error = Some(message);
                                }
                            }
                            Ok(()) => { previous_work_error = None; }
                        }
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        match crate::work_sync::process(&adb, &device) {
                            Ok(n) if n > 0 => {
                                let _ = tx.send(LinkEvent {
                                    message: format!("Amoo Work 的 {n} 个成果已送达手机伴侣"),
                                    error: false,
                                    phone_work: vec![],
                                });
                            }
                            Err(e) => {
                                let _ = tx.send(LinkEvent {
                                    message: format!("成果等待重传：{e}"),
                                    error: true,
                                    phone_work: vec![],
                                });
                            }
                            _ => {}
                        }
                        if stop.load(Ordering::Acquire) {
                            return;
                        }
                        let tasks=crate::desktop_work::snapshot(&device.serial);
                        let fingerprint=serde_json::to_string(&tasks).unwrap_or_default();
                        if fingerprint!=previous_work {previous_work=fingerprint;if !tasks.is_empty(){let _=tx.send(LinkEvent{message:String::new(),error:false,phone_work:tasks});}}
                        result
                    };
                    if stop.load(Ordering::Acquire) {
                        return;
                    }
                    match result {
                        Ok(n) => {
                            let recovered = previous_error.take().is_some();
                            if n > 0 {
                                let _ = tx.send(LinkEvent {
                                    message: format!(
                                        "已收到手机发送的 {n} 个文件，保存在设置中的接收文件夹"
                                    ),
                                    error: false,
                                    phone_work: vec![],
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
                                    phone_work: vec![],
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
                                    phone_work: vec![],
                                });
                                previous_error = Some(message);
                            }
                        }
                    }
                    for _ in 0..if heartbeat_only { 25 } else { 12 } {
                        if stop.load(Ordering::Acquire) {
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
        self.stop.store(true, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delivery_guard_rejects_cancel_and_account_change_before_publication() {
        let cancel = AtomicBool::new(true);
        assert!(check_send_guard(&cancel, Some("one"), || panic!(
            "cancelled transfer must not read account"
        ))
        .is_err());
        cancel.store(false, Ordering::Relaxed);
        assert!(check_send_guard(&cancel, Some("one"), || Ok(Some("two".into()))).is_err());
        assert!(check_send_guard(&cancel, Some("one"), || Ok(None)).is_err());
        assert!(check_send_guard(&cancel, None, || panic!(
            "legacy transfers need no account query"
        ))
        .is_ok());
        assert!(check_send_guard(&cancel, Some("one"), || {
            cancel.store(true, Ordering::Relaxed);
            Ok(Some("one".into()))
        })
        .is_err());
    }

    #[cfg(unix)]
    fn staged_test_adb() -> (tempfile::TempDir, Adb, Device) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("fake-adb");
        let script = format!(
            r#"#!/bin/sh
cd {work}
printf '%s\n' "$*" >> calls
if [ "$3" = "push" ]; then
  dest=$(printf '%s' "$5" | sed 's|{base}|phone|g')
  mkdir -p "$(dirname "$dest")"
  cp "$4" "$dest"
  case "$5" in *payload.*.pending) touch payload-pushed;; esac
  exit 0
fi
case "$*" in
*ro.serialno*) echo guarded-phone;;
*pm*path*) echo package:/data/app/companion.apk;;
*) script=$(printf '%s' "$4" | sed 's|{base}|phone|g'); sh -c "$script";;
esac
"#,
            work = shell_quote(dir.path().to_str().unwrap()),
            base = BASE
        );
        fs::write(&path, script).unwrap();
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).unwrap();
        (
            dir,
            Adb { path },
            Device {
                serial: "guarded-phone".into(),
                transport: "guarded-phone".into(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            },
        )
    }

    #[cfg(unix)]
    #[test]
    fn cancelled_payload_stays_pending_without_a_phone_ready_manifest() {
        let (dir, adb, device) = staged_test_adb();
        let file = dir.path().join("result.md");
        fs::write(&file, b"verified report").unwrap();
        let hash = digest(&file).unwrap();
        let id = uuid();
        let error = send_staged_with(&adb, &device, &file, &id, &hash, 15, "desktop", || {
            ensure!(
                !dir.path().join("payload-pushed").exists(),
                "cancelled after payload push"
            );
            Ok(())
        })
        .unwrap_err();
        assert!(error.to_string().contains("cancelled"));
        let remote = dir.path().join("phone/transfers/inbox").join(&id);
        assert!(!remote.join("manifest.json").exists());
        assert!(!remote.join("result.md").exists());
        assert!(fs::read_dir(remote)
            .unwrap()
            .flatten()
            .any(|e| e.file_name().to_string_lossy().ends_with(".pending")));
    }

    #[cfg(unix)]
    #[test]
    fn idempotent_ready_shortcut_still_checks_guard_before_reporting_delivery() {
        let (dir, adb, device) = staged_test_adb();
        let file = dir.path().join("result.md");
        fs::write(&file, b"report").unwrap();
        let hash = digest(&file).unwrap();
        let id = uuid();
        let remote = dir.path().join("phone/transfers/inbox").join(&id);
        fs::create_dir_all(&remote).unwrap();
        fs::write(remote.join("result.md"), b"report").unwrap();
        fs::write(remote.join("manifest.json"),serde_json::to_vec(&json!({"protocol":1,"status":"ready","sourceDesktopId":"desktop","sha256":hash,"name":"result.md"})).unwrap()).unwrap();
        let mut calls = 0;
        assert!(
            send_staged_with(&adb, &device, &file, &id, &hash, 6, "desktop", || {
                calls += 1;
                ensure!(calls == 1, "account changed before ready acknowledgement");
                Ok(())
            })
            .is_err()
        );
        assert_eq!(calls, 2);
        assert!(!fs::read_to_string(dir.path().join("calls"))
            .unwrap()
            .contains(" push "));
        assert_eq!(fs::read(remote.join("result.md")).unwrap(), b"report");
    }
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

    #[cfg(unix)]
    #[test]
    fn receive_pending_releases_receive_lock_during_pull_and_stops_promptly_on_cancel() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("fake-recv-adb");
        let pull_started = dir.path().join("pull-started");
        let pull_continue = dir.path().join("pull-continue");
        let id1 = "11111111-1111-1111-1111-111111111111";
        let id2 = "22222222-2222-2222-2222-222222222222";
        let payload1 = b"first incoming file";
        let payload2 = b"second incoming file";
        let sample1 = dir.path().join("sample1.bin");
        let sample2 = dir.path().join("sample2.bin");
        fs::write(&sample1, payload1).unwrap();
        fs::write(&sample2, payload2).unwrap();
        let hash1 = digest(&sample1).unwrap();
        let hash2 = digest(&sample2).unwrap();
        let desktop = desktop_id().unwrap();

        let outbox1 = dir.path().join(format!("phone/transfers/outbox/{id1}"));
        let outbox2 = dir.path().join(format!("phone/transfers/outbox/{id2}"));
        fs::create_dir_all(&outbox1).unwrap();
        fs::create_dir_all(&outbox2).unwrap();
        fs::write(outbox1.join("doc.txt"), payload1).unwrap();
        fs::write(
            outbox1.join("manifest.json"),
            serde_json::to_vec(&json!({
                "protocol": 1,
                "id": id1,
                "name": "doc.txt",
                "size": payload1.len(),
                "sha256": hash1,
                "status": "queued",
                "targetDesktopId": desktop,
            }))
            .unwrap(),
        )
        .unwrap();
        fs::write(outbox2.join("doc2.txt"), payload2).unwrap();
        fs::write(
            outbox2.join("manifest.json"),
            serde_json::to_vec(&json!({
                "protocol": 1,
                "id": id2,
                "name": "doc2.txt",
                "size": payload2.len(),
                "sha256": hash2,
                "status": "queued",
                "targetDesktopId": desktop,
            }))
            .unwrap(),
        )
        .unwrap();

        let script = format!(
            r#"#!/bin/sh
cd {work}
if [ "$3" = "pull" ]; then
  src=$(printf '%s' "$4" | sed 's|{base}|phone|g')
  touch '{started}'
  for _ in 1 2 3 4 5 6 7 8 9 10 11 12 13 14 15 16 17 18 19 20; do
    [ -f '{cont}' ] && break
    sleep 0.02
  done
  cp "$src" "$5"
  exit 0
fi
case "$*" in
  *ro.serialno*) echo recv-lock-phone;;
  *pm*path*) echo package:/data/app/companion.apk;;
  *date*+%s*) echo 1700000000;;
  *) script=$(printf '%s' "$4" | sed 's|{base}|phone|g'); sh -c "$script";;
esac
"#,
            work = shell_quote(dir.path().to_str().unwrap()),
            base = BASE,
            started = pull_started.display(),
            cont = pull_continue.display(),
        );
        fs::write(&fake_adb, script).unwrap();
        fs::set_permissions(&fake_adb, fs::Permissions::from_mode(0o700)).unwrap();

        let adb = Adb { path: fake_adb };
        let device = Device {
            serial: "recv-lock-phone".into(),
            transport: "recv-lock-phone".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        let stop = Arc::new(AtomicBool::new(false));
        let stop_bg = stop.clone();
        let adb_bg = adb.clone();
        let dev_bg = device.clone();

        // Pre-populate a colliding file with DIFFERENT bytes at `downloads/<id1>/doc.txt`
        // to also verify `unique_destination` picks `doc (1).txt` under `RECEIVE_LOCK`.
        let dest_dir1 = receive_directory().unwrap().join(id1);
        let _ = fs::remove_dir_all(&dest_dir1);
        private_dir(&dest_dir1).unwrap();
        fs::write(dest_dir1.join("doc.txt"), b"different existing content").unwrap();
        let dest_dir2 = receive_directory().unwrap().join(id2);
        let _ = fs::remove_dir_all(&dest_dir2);

        let handle = thread::spawn(move || receive_pending_cancelable(&adb_bg, &dev_bg, &stop_bg));
        for _ in 0..100 {
            if pull_started.exists() {
                break;
            }
            thread::sleep(Duration::from_millis(10));
        }
        assert!(pull_started.exists(), "first pull should have started");

        // While `adb pull` is actively blocked in network I/O, `RECEIVE_LOCK` MUST be unlocked!
        let lock_free_during_io = RECEIVE_LOCK.try_lock().is_ok();
        assert!(
            lock_free_during_io,
            "RECEIVE_LOCK must NOT be held across slow pull I/O"
        );

        // Request cancellation before the second file starts pulling
        stop.store(true, Ordering::Release);
        fs::write(&pull_continue, b"go").unwrap();

        let count = handle.join().unwrap().unwrap();
        assert_eq!(
            count, 1,
            "stop flag must cancel before pulling the second file"
        );
        assert_eq!(
            fs::read(dest_dir1.join("doc (1).txt")).unwrap(),
            payload1,
            "unique_destination should save colliding file as `doc (1).txt`"
        );
        assert!(
            !dest_dir2.join("doc2.txt").exists(),
            "second file must not be pulled after stop=true"
        );
        let _ = fs::remove_dir_all(&dest_dir1);
        let _ = fs::remove_dir_all(&dest_dir2);
    }
}
