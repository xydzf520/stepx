//! Verified phone offload, on the existing USB / wireless ADB connection.
//! The phone alone removes originals after a durable, matching desktop receipt.
use crate::{
    adb::{run_process, shell_quote, Adb},
    device::Device,
    resources,
};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    fs,
    io::{BufRead, BufReader, Write},
    net::{SocketAddr, TcpStream},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex, OnceLock,
    },
    thread,
    time::{Duration, Instant},
};
static TRANSFER_LOCK: Mutex<()> = Mutex::new(());
const RESERVE: u64 = 128 * 1024 * 1024;
const EXPANSION_SOCKET_NAME: &str = "localabstract:stepx_push_v1";
const SOCKET_MISS_TTL: Duration = Duration::from_secs(15);
const VERIFY_CACHE_TTL: Duration = Duration::from_secs(5);

struct ExpansionForwardEntry {
    port: Option<u16>,
    stream: Option<BufReader<TcpStream>>,
    updated_at: Instant,
}

fn expansion_forward_cache() -> &'static Mutex<HashMap<String, ExpansionForwardEntry>> {
    static CACHE: OnceLock<Mutex<HashMap<String, ExpansionForwardEntry>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn device_verify_cache() -> &'static Mutex<HashMap<(PathBuf, String, String), Instant>> {
    static CACHE: OnceLock<Mutex<HashMap<(PathBuf, String, String), Instant>>> = OnceLock::new();
    CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn verify_device_cached(adb: &Adb, d: &Device) -> Result<()> {
    let key = (adb.path.clone(), d.transport.clone(), d.serial.clone());
    if let Ok(guard) = device_verify_cache().lock() {
        if let Some(ts) = guard.get(&key) {
            if ts.elapsed() < VERIFY_CACHE_TTL {
                return Ok(());
            }
        }
    }
    d.verify(adb)?;
    if let Ok(mut guard) = device_verify_cache().lock() {
        guard.insert(key, Instant::now());
    }
    Ok(())
}

fn obtain_expansion_port(adb: &Adb, serial: &str, force_refresh: bool) -> Result<(u16, bool)> {
    if !force_refresh {
        if let Ok(guard) = expansion_forward_cache().lock() {
            if let Some(entry) = guard.get(serial) {
                if let Some(port) = entry.port {
                    if entry.updated_at.elapsed() <= Duration::from_secs(300) {
                        return Ok((port, true));
                    }
                } else if entry.updated_at.elapsed() < SOCKET_MISS_TTL {
                    anyhow::bail!("本地推送流连接不可用（冷却期内直接回退）");
                }
            }
        }
    }
    let out = adb.run_timeout(
        &["-s", serial, "forward", "tcp:0", EXPANSION_SOCKET_NAME],
        Duration::from_millis(2500),
    )?;
    let port: u16 = out
        .lines()
        .find_map(|line| line.trim().parse::<u16>().ok())
        .context("无法解析 adb forward 分配的本地端口")?;
    if let Ok(mut guard) = expansion_forward_cache().lock() {
        guard.insert(
            serial.to_string(),
            ExpansionForwardEntry {
                port: Some(port),
                stream: None,
                updated_at: Instant::now(),
            },
        );
    }
    Ok((port, false))
}

fn mark_expansion_socket_unavailable(serial: &str) {
    if let Ok(mut guard) = expansion_forward_cache().lock() {
        guard.insert(
            serial.to_string(),
            ExpansionForwardEntry {
                port: None,
                stream: None,
                updated_at: Instant::now(),
            },
        );
    }
}

fn invalidate_expansion_port(serial: &str) {
    if let Ok(mut guard) = expansion_forward_cache().lock() {
        guard.remove(serial);
    }
}

fn take_cached_expansion_stream(serial: &str, expected_port: u16) -> Option<BufReader<TcpStream>> {
    if let Ok(mut guard) = expansion_forward_cache().lock() {
        if let Some(entry) = guard.get_mut(serial) {
            if entry.port == Some(expected_port) {
                return entry.stream.take();
            }
        }
    }
    None
}

fn return_cached_expansion_stream(serial: &str, port: u16, reader: BufReader<TcpStream>) {
    if let Ok(mut guard) = expansion_forward_cache().lock() {
        if let Some(entry) = guard.get_mut(serial) {
            entry.port = Some(port);
            entry.stream = Some(reader);
            entry.updated_at = Instant::now();
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default)]
pub struct Config {
    pub folder: Option<PathBuf>,
    pub kind: String,
    pub sort: String,
    pub descending: bool,
    pub filter: String,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            folder: None,
            kind: "all".into(),
            sort: "date".into(),
            descending: true,
            filter: "all".into(),
        }
    }
}
fn config_path() -> PathBuf {
    resources::user_config().join("stepx-companion/storage-expansion.json")
}
pub fn config() -> Config {
    fs::read(config_path())
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default()
}
pub fn save_config(c: &Config) -> Result<()> {
    let p = config_path();
    fs::create_dir_all(p.parent().unwrap())?;
    atomic_json(&p, &serde_json::to_value(c)?)
}
fn safe_dir(p: &Path) -> Result<()> {
    if p.exists() {
        ensure!(
            !fs::symlink_metadata(p)?.file_type().is_symlink() && p.is_dir(),
            "保存目录不可为链接或文件"
        );
    }
    fs::create_dir_all(p)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(p, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}
fn atomic_json(p: &Path, j: &Value) -> Result<()> {
    let parent = p.parent().context("记录路径无效")?;
    let mut f = tempfile::NamedTempFile::new_in(parent)?;
    serde_json::to_writer(&mut f, j)?;
    f.as_file().sync_all()?;
    f.persist(p)?;
    sync_dir(parent)?;
    Ok(())
}
fn sync_dir(p: &Path) -> Result<()> {
    #[cfg(unix)]
    fs::File::open(p)?.sync_all()?;
    #[cfg(not(unix))]
    let _ = p;
    Ok(())
}
pub fn valid_id(s: &str) -> bool {
    s.len() == 32
        && s.bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}
fn strval<'a>(v: &'a Value, k: &str) -> &'a str {
    v[k].as_str().unwrap_or("")
}
pub fn check_record(j: &Value) -> Result<()> {
    ensure!(
        valid_id(strval(j, "id")) && valid_id(strval(j, "token")),
        "文件回执编号无效"
    );
    let h = strval(j, "sha256");
    ensure!(
        h.len() == 64 && h.bytes().all(|b| b.is_ascii_hexdigit()),
        "文件校验值无效"
    );
    ensure!(
        j["size"]
            .as_u64()
            .is_some_and(|v| v > 0 && v <= 32 * 1024 * 1024 * 1024),
        "文件大小超出范围"
    );
    let p = strval(j, "path");
    ensure!(
        !p.is_empty()
            && !p.starts_with('/')
            && !p.contains('\\')
            && !p.chars().any(char::is_control)
            && p.split('/').all(|p| !p.starts_with('.') && !p.is_empty()),
        "手机文件路径无效"
    );
    ensure!(
        (p.starts_with("DCIM/") && !p[5..].contains('/'))
            || ["DCIM/Camera/", "Documents/", "Download/"]
                .iter()
                .any(|x| p.starts_with(x)),
        "文件不在支持的迁移目录"
    );
    Ok(())
}
fn vault(c: &Config) -> Result<PathBuf> {
    let root = c.folder.as_ref().context("请先选择电脑保存文件夹")?;
    ensure!(root.is_dir(), "电脑保存目录已离线");
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink(),
        "保存目录已变化"
    );
    let root = root.join("StepX Storage");
    safe_dir(&root)?;
    Ok(root)
}
fn file_dir(c: &Config, id: &str) -> Result<PathBuf> {
    ensure!(valid_id(id), "文件编号无效");
    let p = vault(c)?.join(id);
    safe_dir(&p)?;
    Ok(p)
}
fn data_name(j: &Value) -> String {
    let ext = Path::new(strval(j, "name"))
        .extension()
        .and_then(|s| s.to_str())
        .filter(|s| s.len() <= 12 && s.bytes().all(|b| b.is_ascii_alphanumeric()))
        .unwrap_or("bin");
    format!("original.{ext}")
}
pub fn local_file(c: &Config, j: &Value) -> Result<PathBuf> {
    check_record(j)?;
    let p = file_dir(c, strval(j, "id"))?.join(data_name(j));
    ensure!(
        p.is_file() && !fs::symlink_metadata(&p)?.file_type().is_symlink(),
        "电脑原件不存在，请检查保存文件夹"
    );
    Ok(p)
}
pub fn digest(p: &Path) -> Result<String> {
    let h = crate::device::sha256_file(p).context("无法校验文件，请检查运行依赖")?;
    ensure!(h.len() == 64, "文件校验回执无效");
    Ok(h)
}
fn verify(p: &Path, j: &Value) -> Result<()> {
    ensure!(
        fs::metadata(p)?.len() == j["size"].as_u64().unwrap_or(0)
            && digest(p)? == strval(j, "sha256"),
        "文件完整性校验失败，保留手机原件"
    );
    Ok(())
}
fn free_space(p: &Path) -> Result<u64> {
    #[cfg(unix)]
    {
        use std::{ffi::CString, os::unix::ffi::OsStrExt};
        if let Ok(c_path) = CString::new(p.as_os_str().as_bytes()) {
            unsafe {
                let mut stat: libc::statvfs = std::mem::zeroed();
                if libc::statvfs(c_path.as_ptr(), &mut stat) == 0 {
                    let block_size = if stat.f_frsize > 0 {
                        stat.f_frsize as u64
                    } else {
                        stat.f_bsize as u64
                    };
                    return Ok((stat.f_bavail as u64).saturating_mul(block_size));
                }
            }
        }
        let mut cmd = std::process::Command::new("df");
        cmd.arg("-Pk").arg(p);
        let out = run_process(cmd, None, Duration::from_secs(10))?;
        ensure!(out.status.success(), "无法读取电脑可用空间");
        let text = String::from_utf8(out.stdout)?;
        let avail_kb: u64 = text
            .lines()
            .nth(1)
            .and_then(|line| line.split_whitespace().nth(3))
            .context("无法读取电脑可用空间")?
            .parse()?;
        return Ok(avail_kb.saturating_mul(1024));
    }
    #[cfg(not(unix))]
    {
        let mut cmd = resources::python_command();
        cmd.arg("-c")
            .arg("import shutil,sys;print(shutil.disk_usage(sys.argv[1]).free)")
            .arg(p);
        let out = run_process(cmd, None, Duration::from_secs(10))?;
        ensure!(out.status.success(), "无法读取电脑可用空间");
        Ok(String::from_utf8(out.stdout)?.trim().parse()?)
    }
}
fn try_call_expansion_socket(
    adb: &Adb,
    serial: &str,
    method: &str,
    arg_json: &str,
    read_timeout: Duration,
) -> Result<Value> {
    let mut payload = serde_json::to_vec(&json!({
        "op": "expansion_call",
        "method": method,
        "arg": arg_json,
    }))?;
    ensure!(payload.len() <= 32768, "扩容请求过长");
    payload.push(b'\n');

    for attempt in 0..2 {
        let force_refresh = attempt > 0;
        let (port, from_cache) = match obtain_expansion_port(adb, serial, force_refresh) {
            Ok(res) => res,
            Err(e) => {
                mark_expansion_socket_unavailable(serial);
                return Err(e);
            }
        };

        let addr = SocketAddr::from(([127, 0, 0, 1], port));
        let io_result = (|| -> Result<Value> {
            let mut reader = match take_cached_expansion_stream(serial, port) {
                Some(r) => r,
                None => {
                    let s = TcpStream::connect_timeout(&addr, Duration::from_millis(1500))
                        .context("连接本地扩容套接字超时")?;
                    let _ = s.set_nodelay(true);
                    BufReader::new(s)
                }
            };
            reader
                .get_mut()
                .set_write_timeout(Some(Duration::from_millis(1500)))?;
            reader.get_mut().set_read_timeout(Some(read_timeout))?;
            reader.get_mut().write_all(&payload)?;
            reader.get_mut().flush()?;

            let mut line = String::new();
            let n = reader.read_line(&mut line)?;
            ensure!(n > 0, "本地扩容套接字已断开 (EOF)");
            let value: Value =
                serde_json::from_str(line.trim()).context("解析扩容套接字 JSON 失败")?;
            return_cached_expansion_stream(serial, port, reader);
            Ok(value)
        })();

        match io_result {
            Ok(val) => {
                return Ok(val);
            }
            Err(err) => {
                if !from_cache || attempt == 1 {
                    mark_expansion_socket_unavailable(serial);
                    return Err(err);
                }
                invalidate_expansion_port(serial);
            }
        }
    }
    anyhow::bail!("本地扩容套接字连接不可用")
}

pub fn call_provider(
    adb: &Adb,
    serial: &str,
    method: &str,
    arg_json: &str,
    timeout: Duration,
) -> Result<Value> {
    let socket_read_timeout = timeout
        .min(Duration::from_secs(600))
        .max(Duration::from_millis(1500));
    if let Ok(r) = try_call_expansion_socket(adb, serial, method, arg_json, socket_read_timeout) {
        if r.get("error")
            .and_then(Value::as_str)
            .is_none_or(|e| !e.contains("未知推送操作"))
        {
            ensure!(
                r["ok"] == true,
                "{}",
                r["error"].as_str().unwrap_or("手机操作未完成")
            );
            return Ok(r);
        }
    }

    let encoded = if arg_json.trim_start().starts_with('{') {
        STANDARD.encode(arg_json.as_bytes())
    } else {
        arg_json.to_string()
    };
    ensure!(encoded.len() <= 32768, "扩容请求过长");
    let args = [
        "content",
        "call",
        "--user",
        "0",
        "--uri",
        "content://com.stepx.companion.app.expansion",
        "--method",
        method,
        "--arg",
        &encoded,
    ];
    let command = args
        .iter()
        .map(|s| shell_quote(s))
        .collect::<Vec<_>>()
        .join(" ");
    let out = adb.run_timeout(&["-s", serial, "shell", &command], timeout)?;
    let raw = out
        .split("result=")
        .nth(1)
        .context("请更新 StepX伴侣以启用存储扩容")?
        .split(|c: char| !c.is_ascii_alphanumeric() && !"+/=".contains(c))
        .next()
        .unwrap_or("");
    let r: Value = serde_json::from_slice(&STANDARD.decode(raw)?)?;
    ensure!(
        r["ok"] == true,
        "{}",
        r["error"].as_str().unwrap_or("手机操作未完成")
    );
    Ok(r)
}

pub fn phone_call(adb: &Adb, d: &Device, q: Value) -> Result<Value> {
    verify_device_cached(adb, d)?;
    let timeout = if matches!(
        q["op"].as_str(),
        Some(
            "status" | "offer" | "configure" | "requests" | "request" | "inspect" | "scan" | "list"
        )
    ) {
        Duration::from_millis(2500)
    } else {
        Duration::from_secs(600)
    };
    let arg_json = serde_json::to_string(&q)?;
    call_provider(adb, &d.transport, "request", &arg_json, timeout)
}
pub fn offer(adb: &Adb, d: &Device, c: &Config) -> Result<Value> {
    let ready = c.folder.as_ref().is_some_and(|f| f.is_dir());
    phone_call(
        adb,
        d,
        json!({"op":"offer","desktopId":crate::companion_link::desktop_id()?,"folder":c.folder.as_ref().map(|p|p.display().to_string()).unwrap_or_default(),"ready":ready}),
    )
}
pub fn records(c: &Config) -> Result<Vec<Value>> {
    let Some(root) = &c.folder else {
        return Ok(vec![]);
    };
    let root = root.join("StepX Storage");
    if !root.is_dir() {
        return Ok(vec![]);
    }
    ensure!(
        !fs::symlink_metadata(&root)?.file_type().is_symlink(),
        "扩容目录已变化"
    );
    let mut rows = vec![];
    for e in fs::read_dir(root)? {
        let e = e?;
        if e.file_type()?.is_dir() && valid_id(&e.file_name().to_string_lossy()) {
            let p = e.path().join("receipt.json");
            if let Ok(b) = fs::read(p) {
                if let Ok(j) = serde_json::from_slice::<Value>(&b) {
                    if check_record(&j).is_ok() {
                        if local_file(c, &j).is_err() && j["state"] != "archived" {
                            let _ = fs::remove_dir_all(e.path());
                        } else {
                            rows.push(j);
                        }
                    }
                }
            }
        }
    }
    Ok(rows)
}
/// Snapshot notifications carry genuine phases, not synthetic transfer percentages.
pub fn migrate(
    adb: &Adb,
    d: &Device,
    c: &Config,
    candidate: Value,
    progress: impl FnMut(&str, &Value),
) -> Result<Value> {
    migrate_cancellable(adb, d, c, candidate, progress, &AtomicBool::new(false))
}
fn migrate_cancellable(
    adb: &Adb,
    d: &Device,
    c: &Config,
    candidate: Value,
    mut progress: impl FnMut(&str, &Value),
    stop: &AtomicBool,
) -> Result<Value> {
    let desktop = crate::companion_link::desktop_id()?;
    let mut j = if candidate["resume"] == true {
        candidate
    } else {
        progress("正在校验手机原件", &candidate);
        phone_call(
            adb,
            d,
            json!({"op":"prepare","desktopId":desktop,"path":candidate["path"]}),
        )?["file"]
            .clone()
    };
    check_record(&j)?;
    j["phoneSerial"] = json!(d.serial);
    ensure!(j["desktop"] == desktop, "文件属于另一台电脑");
    ensure!(!stop.load(Ordering::Acquire), "设备已切换，手机原件保留");
    let dir = file_dir(c, strval(&j, "id"))?;
    let target = dir.join(data_name(&j));
    if target.exists() {
        ensure!(
            !fs::symlink_metadata(&target)?.file_type().is_symlink(),
            "保存文件不可为链接"
        );
        verify(&target, &j)?;
    } else {
        for entry in fs::read_dir(&dir)? {
            let entry = entry?;
            if entry
                .file_name()
                .to_string_lossy()
                .starts_with(".stepx-transfer-")
                && entry.file_type()?.is_file()
            {
                fs::remove_file(entry.path())?;
            }
        }
        ensure!(
            free_space(&dir)? > j["size"].as_u64().unwrap() + RESERVE,
            "电脑空间不足，保留手机原件"
        );
        let tmp = tempfile::Builder::new()
            .prefix(".stepx-transfer-")
            .tempfile_in(&dir)?;
        progress("正在传输到电脑", &j);
        adb.transfer(
            &d.transport,
            "pull",
            Path::new(&format!("/sdcard/{}", strval(&j, "path"))),
            tmp.path(),
        )?;
        progress("正在校验电脑副本", &j);
        verify(tmp.path(), &j)?;
        tmp.as_file().sync_all()?;
        tmp.persist_noclobber(&target)?;
        sync_dir(&dir)?;
    }
    let mut receipt = j.clone();
    receipt["state"] = json!("verified");
    atomic_json(&dir.join("receipt.json"), &receipt)?;
    progress("正在保留入口并释放空间", &j);
    // A phone receipt is requested only after both the original and manifest are durable.
    ensure!(!stop.load(Ordering::Acquire), "设备已切换，手机原件保留");
    let staged=phone_call(adb,d,json!({"op":"commit","id":j["id"],"desktopId":desktop,"token":j["token"],"sha256":j["sha256"],"size":j["size"]}))?["file"].clone();
    if staged["state"] == "releasing" {
        ensure!(!stop.load(Ordering::Acquire), "设备已切换，保留待清理原件");
        let parent = Path::new(strval(&j, "path"))
            .parent()
            .context("文件路径无效")?
            .to_str()
            .context("文件路径编码无效")?;
        let cleanup = format!(
            "/storage/emulated/0/{parent}/.stepx-migrate-{}",
            strval(&j, "id")
        );
        ensure!(staged["cleanupPath"] == cleanup, "清理路径不匹配");
        // Revalidate on the phone immediately before unlink; never delete by a display name.
        let q = shell_quote(&cleanup);
        let hash = shell_quote(strval(&j, "sha256"));
        let command=format!("test ! -L {q} && test -f {q} && h=$(sha256sum {q}) && test \"${{h%% *}}\" = {hash} && rm -- {q} && test ! -e {q}");
        adb.run_timeout(
            &["-s", &d.transport, "shell", &command],
            Duration::from_secs(600),
        )?;
    }
    let mut result = phone_call(
        adb,
        d,
        json!({"op":"finalize","id":j["id"],"desktopId":desktop,"token":j["token"]}),
    )?["file"]
        .clone();
    result["phoneSerial"] = json!(d.serial);
    atomic_json(&dir.join("receipt.json"), &result)?;
    progress("迁移完成", &result);
    Ok(result)
}
pub fn deliver(adb: &Adb, d: &Device, c: &Config, j: &Value) -> Result<Value> {
    check_record(j)?;
    let desktop = crate::companion_link::desktop_id()?;
    ensure!(j["desktop"] == desktop, "文件属于另一台电脑");
    let file = local_file(c, j)?;
    verify(&file, j)?;
    let id = strval(j, "id");
    let remote = strval(j, "incoming");
    ensure!(remote==format!("/storage/emulated/0/Android/data/com.stepx.companion.app/files/expansion-incoming/{id}/original.part"),"手机接收目录无效");
    adb.transfer(&d.transport, "push", &file, Path::new(remote))?;
    let mut r =
        phone_call(adb, d, json!({"op":"deliver","id":id,"desktopId":desktop}))?["file"].clone();
    r["phoneSerial"] = json!(d.serial);
    atomic_json(&file_dir(c, id)?.join("receipt.json"), &r)?;
    Ok(r)
}
/// Serialized by the transfer worker; a verified response, not a successful dispatch, commits UI state.
fn apply_policy(adb: &Adb, d: &Device, folder: Option<PathBuf>, mut q: Value) -> Result<Value> {
    let mut cfg = config();
    let before = phone_call(adb, d, json!({"op":"status"}))?;
    if let Some(epoch) = q["expectedEpoch"].as_str() {
        ensure!(
            before["policy"]["epoch"] == epoch,
            "手机迁移规则已改变，请刷新后重新确认"
        );
    }
    if q["enabled"] == true {
        ensure!(
            ["photos", "videos", "documents"]
                .iter()
                .any(|k| q[*k] == true),
            "请选择至少一种文件类型"
        );
        let folder =
            crate::settings::choose_directory(folder.as_deref().context("请选择电脑保存文件夹")?)?;
        if cfg.folder.as_ref() != Some(&folder) {
            ensure!(
                before["policy"]["enabled"] != true,
                "请先暂停自动迁移，再更换保存位置"
            );
            ensure!(
                records(&cfg)?.is_empty(),
                "原目录已有迁移记录，请保留原目录"
            );
            cfg.folder = Some(folder);
            vault(&cfg)?;
            save_config(&cfg)?;
        }
        offer(adb, d, &cfg)?;
    }
    q["op"] = json!("configure");
    q["desktopId"] = json!(crate::companion_link::desktop_id()?);
    let result = (|| -> Result<Value> {
        phone_call(adb, d, q.clone())?;
        let readback = phone_call(adb, d, json!({"op":"status"}))?;
        for key in [
            "enabled",
            "photos",
            "videos",
            "documents",
            "retentionDays",
            "desktopId",
        ] {
            ensure!(
                readback["policy"][key] == q[key],
                "手机设置回读不一致：{key}"
            );
        }
        Ok(readback)
    })();
    if let Err(error) = result {
        q["enabled"] = json!(false);
        let paused = phone_call(adb, d, q).is_ok();
        anyhow::bail!(
            "设置未完成：{error:#}。{}；编辑内容已保留。",
            if paused {
                "已暂停手机自动迁移"
            } else {
                "暂时无法确认手机状态，本机已停止自动迁移，请重新连接检查"
            }
        );
    }
    result
}
#[derive(Clone, Default)]
pub struct Snapshot {
    pub status: Value,
    pub files: Vec<Value>,
    pub phase: String,
    pub active: Option<Value>,
    pub error: bool,
    pub online: bool,
    pub report: Value,
    pub report_error: String,
}

/// Explain the first blocking condition without claiming that enabled means running.
pub fn waiting_reason(status: &Value, c: &Config, report: &Value, desktop: &str) -> String {
    if status["permission"] != true {
        return "手机缺少文件管理权限，请在伴侣中开启".into();
    }
    if c.folder.is_none() {
        return "还没有选择电脑保存位置".into();
    }
    if c.folder.as_ref().is_some_and(|p| !p.is_dir()) {
        return "电脑保存目录不可用，请连接对应磁盘".into();
    }
    if status["policy"]["enabled"] != true {
        return "自动迁移尚未开启，请设置规则并确认开启".into();
    }
    if status["policy"]["desktopId"] != desktop {
        return "迁移规则绑定的是另一台电脑，请重新确认本机规则".into();
    }
    if status["online"] != true || status["host"]["ready"] != true {
        return "正在等待电脑与手机确认连接".into();
    }
    if report["candidateCount"].as_u64().unwrap_or(0) > 0 {
        return format!(
            "发现 {} 个符合条件的文件，准备自动迁移",
            report["candidateCount"]
        );
    }
    let retained = report["retainedCount"].as_u64().unwrap_or(0);
    let writing = report["writingCount"].as_u64().unwrap_or(0);
    if retained + writing > 0 {
        return format!(
            "{} 个文件仍在保留期或写入等待期，尚未到迁移时间",
            retained + writing
        );
    }
    if report["selectedCount"] == 0 {
        return "所选类型暂时没有文件，请检查文件类型与扫描范围".into();
    }
    if report["excludedCount"].as_u64().unwrap_or(0) > 0 {
        return "文件已迁移、恢复或取消，不会重复自动迁移".into();
    }
    if report["invalidCount"].as_u64().unwrap_or(0) > 0 {
        return "文件为空或超过单文件 32 GB 上限，已跳过".into();
    }
    "正在检查手机文件…".into()
}
#[derive(Clone, Debug)]
pub enum Command {
    Refresh,
    Inspect {
        policy: Value,
        reply: mpsc::Sender<std::result::Result<Value, String>>,
    },
    Configure(Value),
    ApplyPolicy {
        folder: Option<PathBuf>,
        policy: Value,
        reply: mpsc::Sender<std::result::Result<Value, String>>,
    },
    Restore(String),
    Folder(PathBuf),
}
pub struct Worker {
    pub serial: String,
    pub transport: String,
    pub commands: mpsc::Sender<Command>,
    pub events: mpsc::Receiver<Snapshot>,
    stop: Arc<AtomicBool>,
}
impl Drop for Worker {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
}
impl Worker {
    pub fn start(adb: Adb, d: Device) -> Self {
        let (tx, rx) = mpsc::channel();
        let (out, events) = mpsc::channel();
        let stop = Arc::new(AtomicBool::new(false));
        let stopped = stop.clone();
        let serial = d.serial.clone();
        let transport = d.transport.clone();
        // A long video copy must not expire the connected host lease.
        let lease_stop = stop.clone();
        let lease_adb = adb.clone();
        let lease_device = d.clone();
        let migrating_active = Arc::new(AtomicBool::new(false));
        let lease_migrating = migrating_active.clone();
        thread::spawn(move || {
            while !lease_stop.load(Ordering::Acquire) {
                if lease_migrating.load(Ordering::Relaxed) {
                    let _ = offer(&lease_adb, &lease_device, &config());
                }
                for _ in 0..16 {
                    if lease_stop.load(Ordering::Acquire) {
                        return;
                    }
                    thread::sleep(Duration::from_millis(500));
                }
            }
        });
        thread::spawn(move || {
            let _ = adb.run_timeout(
                &[
                    "-s",
                    &d.transport,
                    "shell",
                    "pm grant com.stepx.companion.app android.permission.ACCESS_MEDIA_LOCATION >/dev/null 2>&1; pm grant com.stepx.companion.app android.permission.READ_MEDIA_IMAGES >/dev/null 2>&1; pm grant com.stepx.companion.app android.permission.READ_MEDIA_VIDEO >/dev/null 2>&1; pm grant com.stepx.companion.app android.permission.WRITE_SECURE_SETTINGS >/dev/null 2>&1; appops set com.stepx.companion.app ACCESS_MEDIA_LOCATION allow >/dev/null 2>&1; settings put secure com.stepx.companion.app_op_100 4 >/dev/null 2>&1; cmd notification allow_listener com.stepx.companion.app/com.stepx.companion.app.PushNotificationService >/dev/null 2>&1 || true; true",
                ],
                Duration::from_secs(5),
            );
            let mut snap = Snapshot::default();
            let mut configuration_blocked = false;
            let mut cooldown = std::collections::HashMap::<String, Instant>::new();
            let mut last = Instant::now() - Duration::from_secs(30);
            let mut inspected = Instant::now() - Duration::from_secs(60);
            let mut inspected_epoch = Value::Null;
            while !stopped.load(Ordering::Acquire) {
                let command = rx.recv_timeout(Duration::from_millis(500)).ok();
                if command.is_none() && last.elapsed() < Duration::from_secs(5) {
                    continue;
                }
                // A user command must never be dropped when another transfer holds the lock.
                let _guard = if command.is_some() {
                    match TRANSFER_LOCK.lock() {
                        Ok(g) => g,
                        Err(_) => break,
                    }
                } else {
                    match TRANSFER_LOCK.try_lock() {
                        Ok(g) => g,
                        Err(_) => continue,
                    }
                };
                let mut cfg = config();
                let result = (|| -> Result<()> {
                    if let Some(command) = command {
                        match command {
                            Command::Inspect { policy, reply } => {
                                let result =
                                    phone_call(&adb, &d, json!({"op":"inspect","policy":policy}))
                                        .map(|r| r["report"].clone())
                                        .map_err(|e| format!("{e:#}"));
                                let _ = reply.send(result);
                                return Ok(());
                            }
                            Command::ApplyPolicy {
                                folder,
                                policy,
                                reply,
                            } => {
                                let r = apply_policy(&adb, &d, folder, policy);
                                configuration_blocked = r.is_err();
                                let result =
                                    r.as_ref().map(|v| v.clone()).map_err(|e| format!("{e:#}"));
                                let _ = reply.send(result);
                                r?;
                                cfg = config();
                            }
                            Command::Folder(folder) => {
                                let folder = folder.canonicalize()?;
                                if cfg.folder.as_ref() != Some(&folder) {
                                    ensure!(
                                        records(&cfg)?.is_empty(),
                                        "当前文件夹已有迁移记录，不能直接更换保存位置"
                                    );
                                    cfg.folder = Some(folder);
                                    vault(&cfg)?;
                                    save_config(&cfg)?;
                                }
                            }
                            Command::Configure(mut q) => {
                                q["op"] = json!("configure");
                                q["desktopId"] = json!(crate::companion_link::desktop_id()?);
                                phone_call(&adb, &d, q)?;
                            }
                            Command::Restore(id) => {
                                phone_call(
                                    &adb,
                                    &d,
                                    json!({"op":"request","id":id,"mode":"restore"}),
                                )?;
                            }
                            Command::Refresh => {
                                cooldown.clear();
                                inspected = Instant::now() - Duration::from_secs(60);
                            }
                        }
                    }
                    let mut did_work = false;
                    let desktop = crate::companion_link::desktop_id()?;
                    snap.status = offer(&adb, &d, &cfg)?;
                    snap.online = true;
                    snap.error = false;
                    snap.active = None;
                    snap.files = records(&cfg)?
                        .into_iter()
                        .filter(|j| {
                            j["desktop"] == desktop
                                && j["phoneSerial"].as_str().is_none_or(|s| s == d.serial)
                        })
                        .collect();
                    if snap.status["permission"] == true
                        && (inspected.elapsed() >= Duration::from_secs(30)
                            || inspected_epoch != snap.status["policy"]["epoch"])
                    {
                        match phone_call(&adb, &d, json!({"op":"inspect"})) {
                            Ok(r) => {
                                snap.report = r["report"].clone();
                                snap.report_error.clear();
                            }
                            Err(e) => {
                                snap.report = Value::Null;
                                snap.report_error =
                                    format!("无法检查文件：{e:#}。可在小工具更新手机伴侣后重试。");
                            }
                        }
                        inspected = Instant::now();
                        inspected_epoch = snap.status["policy"]["epoch"].clone();
                    }
                    snap.phase = waiting_reason(&snap.status, &cfg, &snap.report, &desktop);
                    let _ = out.send(snap.clone());
                    if configuration_blocked {
                        snap.error = true;
                        snap.phase = "设置未确认，自动迁移已在本机阻止。请检查后重新保存。".into();
                        let _ = out.send(snap.clone());
                        return Ok(());
                    }
                    let requests =
                        phone_call(&adb, &d, json!({"op":"requests","desktopId":desktop}))?;
                    if let Some(items) = requests["items"].as_array() {
                        for j in items {
                            if stopped.load(Ordering::Acquire) {
                                return Ok(());
                            }
                            did_work = true;
                            migrating_active.store(true, Ordering::Relaxed);
                            snap.phase = "正在取回手机文件".into();
                            snap.active = Some(j.clone());
                            let _ = out.send(snap.clone());
                            if let Err(e) = deliver(&adb, &d, &cfg, j) {
                                let _ = phone_call(
                                    &adb,
                                    &d,
                                    json!({"op":"request-error","id":j["id"],"error":format!("{e:#}")}),
                                );
                                return Err(e);
                            }
                        }
                        migrating_active.store(false, Ordering::Relaxed);
                    }
                    if snap.status["policy"]["enabled"] == true
                        && cfg.folder.is_some()
                        && !stopped.load(Ordering::Acquire)
                    {
                        let rows = phone_call(&adb, &d, json!({"op":"scan","desktopId":desktop}))?;
                        if let Some(j) = rows["items"].as_array().and_then(|a| {
                            a.iter().find(|j| {
                                cooldown
                                    .get(strval(j, "path"))
                                    .is_none_or(|t| t.elapsed() > Duration::from_secs(300))
                            })
                        }) {
                            did_work = true;
                            migrating_active.store(true, Ordering::Relaxed);
                            if let Err(e) = migrate_cancellable(
                                &adb,
                                &d,
                                &cfg,
                                j.clone(),
                                |phase, j| {
                                    snap.phase = phase.into();
                                    snap.active = Some(j.clone());
                                    let _ = out.send(snap.clone());
                                },
                                &stopped,
                            ) {
                                migrating_active.store(false, Ordering::Relaxed);
                                cooldown.insert(strval(j, "path").into(), Instant::now());
                                if let Some(file) =
                                    snap.active.as_ref().filter(|f| valid_id(strval(f, "id")))
                                {
                                    let _ = phone_call(
                                        &adb,
                                        &d,
                                        json!({"op":"fail","id":file["id"],"error":format!("{e:#}")}),
                                    );
                                    if let Ok(dir) = file_dir(&cfg, strval(file, "id")) {
                                        if local_file(&cfg, file).is_ok() {
                                            let mut failed = file.clone();
                                            failed["state"] = json!("failed");
                                            failed["error"] = json!(format!("{e:#}"));
                                            let _ = atomic_json(&dir.join("receipt.json"), &failed);
                                        } else {
                                            let _ = fs::remove_dir_all(dir);
                                        }
                                    }
                                }
                                return Err(e);
                            }
                            migrating_active.store(false, Ordering::Relaxed);
                            inspected = Instant::now() - Duration::from_secs(60);
                        } else if !cooldown.is_empty() {
                            snap.phase =
                                "部分文件迁移失败，5 分钟后重试；也可点击“检查迁移”立即重试".into();
                        }
                    }
                    if did_work {
                        snap.status = offer(&adb, &d, &cfg)?;
                        snap.files = records(&cfg)?
                            .into_iter()
                            .filter(|j| {
                                j["desktop"] == desktop
                                    && j["phoneSerial"].as_str().is_none_or(|s| s == d.serial)
                            })
                            .collect();
                    }
                    snap.active = None;
                    Ok(())
                })();
                migrating_active.store(false, Ordering::Relaxed);
                if let Err(e) = result {
                    let current_desktop_id =
                        crate::companion_link::desktop_id().unwrap_or_default();
                    snap.files = records(&cfg)
                        .unwrap_or_else(|_| snap.files.clone())
                        .into_iter()
                        .filter(|j| {
                            j["desktop"] == current_desktop_id
                                && j["phoneSerial"].as_str().is_none_or(|s| s == d.serial)
                        })
                        .collect();
                    snap.error = true;
                    snap.phase = format!("{e:#}");
                    snap.active = None;
                }
                let _ = out.send(snap.clone());
                last = Instant::now();
            }
        });
        Self {
            serial,
            transport,
            commands: tx,
            events,
            stop,
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    fn record() -> Value {
        json!({"id":"a".repeat(32),"token":"b".repeat(32),"path":"Documents/报告.pdf","name":"报告.pdf","sha256":"f".repeat(64),"size":42})
    }
    #[test]
    fn rejects_unsafe_phone_paths() {
        for p in [
            "/etc/passwd",
            "Documents/../secret.pdf",
            "Documents/.hidden/x.txt",
            "Documents/a\\b.pdf",
            "Android/data/a.txt",
            "Documents/a\n.pdf",
        ] {
            let mut j = record();
            j["path"] = json!(p);
            assert!(check_record(&j).is_err(), "{p}");
        }
        assert!(check_record(&record()).is_ok());
    }
    #[test]
    fn rejects_bad_receipt_and_huge_file() {
        let mut j = record();
        j["id"] = json!("../oops");
        assert!(check_record(&j).is_err());
        let mut j = record();
        j["size"] = json!(u64::MAX);
        assert!(check_record(&j).is_err());
    }
    #[test]
    fn supports_camera_root_but_not_screenshots_or_hidden_paths() {
        let mut j = record();
        j["path"] = json!("DCIM/20261024.jpg");
        assert!(check_record(&j).is_ok());
        for path in [
            "DCIM/Screenshots/a.png",
            "DCIM/.cache/a.jpg",
            "Pictures/a.jpg",
        ] {
            j["path"] = json!(path);
            assert!(check_record(&j).is_err());
        }
    }
    #[test]
    fn enabled_is_not_the_same_as_ready_to_migrate() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config {
            folder: Some(dir.path().into()),
            ..Default::default()
        };
        let mut status = json!({"permission":true,"online":true,"host":{"ready":true},"policy":{"enabled":true,"desktopId":"this"}});
        assert!(waiting_reason(
            &status,
            &c,
            &json!({"candidateCount":0,"retainedCount":17}),
            "this"
        )
        .contains("17 个文件仍在"));
        assert!(waiting_reason(
            &status,
            &c,
            &json!({"candidateCount":0,"selectedCount":0}),
            "this"
        )
        .contains("暂时没有文件"));
        assert!(waiting_reason(&status, &c, &json!({}), "other").contains("另一台电脑"));
        status["permission"] = json!(false);
        assert!(waiting_reason(&status, &c, &json!({}), "this").contains("权限"));
    }
    #[test]
    fn mismatched_copy_is_never_verified() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("f");
        fs::write(&p, b"abc").unwrap();
        assert!(verify(&p, &record()).is_err());
    }
    #[test]
    fn durable_receipt_roundtrip() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("receipt.json");
        atomic_json(&p, &record()).unwrap();
        assert_eq!(
            serde_json::from_slice::<Value>(&fs::read(p).unwrap()).unwrap(),
            record()
        );
    }
    #[test]
    fn managed_file_names_are_portable() {
        let mut j = record();
        j["name"] = json!("CON:bad/name?.pdf");
        assert_eq!(data_name(&j), "original.pdf");
    }
    #[cfg(unix)]
    #[test]
    fn phone_call_prefers_local_socket_and_falls_back_to_content_call() {
        use std::net::TcpListener;
        use std::os::unix::fs::PermissionsExt;

        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut line = String::new();
            BufReader::new(&mut stream).read_line(&mut line).unwrap();
            let req: Value = serde_json::from_str(&line).unwrap();
            assert_eq!(req["op"], "expansion_call");
            assert_eq!(req["method"], "request");
            let inner: Value = serde_json::from_str(req["arg"].as_str().unwrap()).unwrap();
            assert_eq!(inner["op"], "status");
            stream
                .write_all(b"{\"ok\":true,\"online\":true,\"via\":\"socket\"}\n")
                .unwrap();
        });

        let d = tempfile::tempdir().unwrap();
        let adb_path = d.path().join("adb");
        let log_path = d.path().join("adb.log");
        fs::write(
            &adb_path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\ncase \"$*\" in\n*ro.serialno*) printf 'sock-phone\\n';;\n*forward*) printf '{port}\\n';;\n*content\\ call*) exit 99;;\nesac\n",
                shell_quote(log_path.to_str().unwrap())
            ),
        )
        .unwrap();
        fs::set_permissions(&adb_path, fs::Permissions::from_mode(0o700)).unwrap();

        let adb = Adb { path: adb_path };
        let dev = Device {
            transport: "sock-phone".into(),
            serial: "sock-phone".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        let res = phone_call(&adb, &dev, json!({"op": "status"})).unwrap();
        assert_eq!(res["via"], "socket");
        server.join().unwrap();
        let log = fs::read_to_string(&log_path).unwrap();
        assert!(!log.contains("content call"));
    }
}
