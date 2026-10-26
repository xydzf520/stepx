//! Bounded official OTA download, integrity checking and phone staging.
//! System installation is delegated to the phone's own policy-protected updater.
use crate::{
    adb::Adb,
    device::Device,
    ota::{Firmware, OtaReport, VersionRelation},
};
use anyhow::{ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    io::{BufRead, BufReader, Write},
    path::PathBuf,
    process::Stdio,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver},
        Arc,
    },
    thread,
    time::{Duration, Instant},
};
const HELPER: &str = include_str!("ota_transfer.py");
#[derive(Clone, Debug)]
pub struct VerifiedPackage {
    pub path: PathBuf,
    pub version: String,
    pub size_bytes: u64,
    pub metadata: BTreeMap<String, String>,
    pub(crate) verified_stat: Option<[u64; 3]>,
    sha256: String,
    firmware: Firmware,
}
impl VerifiedPackage {
    pub fn firmware(&self) -> &Firmware {
        &self.firmware
    }
}
#[derive(Clone, Debug)]
pub struct StagedPackage {
    pub package: VerifiedPackage,
    pub remote_path: String,
    pub serial: String,
}
#[derive(Debug)]
pub enum Event {
    Progress {
        phase: String,
        done: u64,
        total: u64,
    },
    Downloaded(VerifiedPackage),
    Staged(StagedPackage),
    Failed(String),
    Cancelled,
}
pub struct Job {
    pub events: Receiver<Event>,
    cancel: Arc<AtomicBool>,
}
impl Job {
    pub fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }
}
impl Drop for Job {
    fn drop(&mut self) {
        self.cancel();
    }
}
fn cache_dir() -> Result<PathBuf> {
    let home = std::env::var_os("XDG_CACHE_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|v| PathBuf::from(v).join(".cache")))
        .context("无法定位固件缓存目录")?;
    Ok(home.join("stepx-companion/phone-ota"))
}
fn request(firmware: &Firmware) -> Result<Value> {
    ensure!(
        firmware.size_bytes > 0 && firmware.size_bytes <= 12 * 1024 * 1024 * 1024,
        "官方固件大小缺失或超过 12 GiB 限制"
    );
    ensure!(
        firmware.sha256.is_some() || firmware.md5.is_some(),
        "官方未提供固件摘要，无法验证下载"
    );
    Ok(
        json!({"size":firmware.size_bytes,"sha256":firmware.sha256,"md5":firmware.md5,"version":firmware.version}),
    )
}
pub fn start_download(firmware: Firmware) -> Result<Job> {
    let mut req = request(&firmware)?;
    let url = firmware
        .download_url
        .as_ref()
        .context("官方未提供可用固件下载地址")?;
    req["op"] = json!("download");
    req["url"] = json!(url);
    req["cache"] = json!(cache_dir()?);
    spawn(req, firmware, None)
}
/// Verify an existing official package without downloading or touching the phone.
pub fn start_verify(firmware: Firmware, path: PathBuf) -> Result<Job> {
    let mut req = request(&firmware)?;
    req["op"] = json!("verify");
    req["path"] = json!(path);
    spawn(req, firmware, None)
}
pub fn start_stage(adb: Adb, device: Device, package: VerifiedPackage) -> Result<Job> {
    // The worker rechecks the serial before any phone operation; keep UI startup nonblocking.
    let mut req = request(&package.firmware)?;
    req["sha256"] = json!(package.sha256);
    req["verified_stat"] = json!(package.verified_stat);
    req["op"] = json!("stage");
    req["path"] = json!(package.path);
    req["adb"] = json!(adb.path);
    req["transport"] = json!(device.transport);
    req["serial"] = json!(device.serial);
    req["remote"] = json!(format!(
        "/sdcard/Download/StepX-OTA/{:032x}/update.zip",
        rand::random::<u128>()
    ));
    spawn(req, package.firmware, Some(device.serial))
}
fn spawn(mut req: Value, firmware: Firmware, serial: Option<String>) -> Result<Job> {
    let work = tempfile::tempdir()?;
    let cancel_path = work.path().join("cancel");
    req["cancel"] = json!(cancel_path);
    req["parent_pid"] = json!(std::process::id());
    let mut command = crate::resources::python_command();
    command
        .args(["-u", "-c", HELPER])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    let mut child = command
        .spawn()
        .context("无法启动固件传输工具（需要 Python 3）")?;
    let mut input = child.stdin.take().context("固件传输输入不可用")?;
    serde_json::to_writer(&mut input, &req)?;
    input.flush()?;
    drop(input);
    let stdout = child.stdout.take().context("固件传输出不可用")?;
    let (tx, rx) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let flag = cancel.clone();
    thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel();
        thread::spawn(move || {
            for line in BufReader::new(stdout)
                .lines()
                .map_while(std::result::Result::ok)
            {
                if line.len() < 128 * 1024 {
                    if line_tx.send(line).is_err() {
                        break;
                    }
                }
            }
        });
        let mut terminal = false;
        let start = Instant::now();
        let mut cancelled_at = None;
        loop {
            if flag.load(Ordering::Acquire) && cancelled_at.is_none() {
                let _ = std::fs::write(&cancel_path, b"cancel");
                cancelled_at = Some(Instant::now());
            }
            if cancelled_at.is_some_and(|t| t.elapsed() > Duration::from_secs(20))
                || start.elapsed() > Duration::from_secs(7200)
            {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                let _ = tx.send(if cancelled_at.is_some() {
                    Event::Cancelled
                } else {
                    Event::Failed("固件操作超时，已停止；可重试".into())
                });
                terminal = true;
                break;
            }
            match line_rx.recv_timeout(Duration::from_millis(150)) {
                Ok(line) => {
                    let Ok(value) = serde_json::from_str::<Value>(&line) else {
                        continue;
                    };
                    match value["event"].as_str().unwrap_or("") {
                        "progress" => {
                            let _ = tx.send(Event::Progress {
                                phase: value["phase"].as_str().unwrap_or("").into(),
                                done: value["done"].as_u64().unwrap_or(0),
                                total: value["total"].as_u64().unwrap_or(0),
                            });
                        }
                        "failed" => {
                            let _ = tx.send(Event::Failed(
                                value["message"].as_str().unwrap_or("固件处理失败").into(),
                            ));
                            terminal = true;
                        }
                        "cancelled" => {
                            let _ = tx.send(Event::Cancelled);
                            terminal = true;
                        }
                        "complete" => {
                            let result = (|| -> Result<Event> {
                                ensure!(!flag.load(Ordering::Acquire), "操作已取消");
                                let package = VerifiedPackage {
                                    path: value["path"].as_str().context("缺少固件路径")?.into(),
                                    version: firmware.version.clone(),
                                    size_bytes: firmware.size_bytes,
                                    metadata: serde_json::from_value(value["metadata"].clone())?,
                                    verified_stat: serde_json::from_value(
                                        value["verified_stat"].clone(),
                                    )
                                    .ok(),
                                    sha256: value["sha256"]
                                        .as_str()
                                        .context("缺少固件校验摘要")?
                                        .into(),
                                    firmware: firmware.clone(),
                                };
                                Ok(if let Some(serial) = &serial {
                                    Event::Staged(StagedPackage {
                                        package,
                                        remote_path: value["remote"]
                                            .as_str()
                                            .context("缺少手机固件路径")?
                                            .into(),
                                        serial: serial.clone(),
                                    })
                                } else {
                                    Event::Downloaded(package)
                                })
                            })();
                            let _ =
                                tx.send(result.unwrap_or_else(|e| Event::Failed(e.to_string())));
                            terminal = true;
                        }
                        _ => {}
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
                Err(mpsc::RecvTimeoutError::Timeout) => {}
            }
            if terminal {
                break;
            }
        }
        if !terminal {
            let _ = tx.send(Event::Failed("固件处理进程提前退出，未完成操作".into()));
        }
        let _ = child.wait();
        drop(work);
    });
    Ok(Job { events: rx, cancel })
}
/// Removes only the exact temporary directory created for this staged package.
pub fn remove_staged(adb: &Adb, device: &Device, staged: &StagedPackage) -> Result<()> {
    device.verify(adb)?;
    ensure!(device.serial == staged.serial, "手机身份不匹配");
    let folder = staged
        .remote_path
        .strip_prefix("/sdcard/Download/StepX-OTA/")
        .and_then(|s| s.strip_suffix("/update.zip"))
        .context("固件暂存路径无效")?;
    ensure!(
        folder.len() == 32 && folder.bytes().all(|b| b.is_ascii_hexdigit()),
        "固件暂存路径无效"
    );
    adb.shell(
        &device.transport,
        &["rm", "-rf", &format!("/sdcard/Download/StepX-OTA/{folder}")],
    )?;
    Ok(())
}
#[derive(Clone, Debug)]
pub struct InstallSubmission {
    pub message: String,
    pub accepted: bool,
}
/// The UI must obtain explicit install confirmation before invoking this function.
pub fn submit_install(
    adb: &Adb,
    device: &Device,
    staged: &StagedPackage,
    report: &OtaReport,
    firmware: &Firmware,
) -> Result<InstallSubmission> {
    device.verify(adb)?;
    ensure!(
        report.serial == device.serial && staged.serial == device.serial,
        "固件与当前手机不匹配"
    );
    ensure!(
        // An official-process response can establish the server candidate without
        // exposing protected identifiers to the desktop. Background probes never set this flag.
        report.eligibility_verified,
        "官方尚未确认这台手机的升级资格，请先在手机系统更新中检查"
    );
    ensure!(
        firmware.relation == VersionRelation::Newer
            && firmware.need_update
            && firmware.exists_update,
        "只能安装官方确认可升级的新版本，不能重刷当前版本或降级"
    );
    ensure!(
        firmware.version == staged.package.version
            && firmware.size_bytes == staged.package.size_bytes
            && firmware.sha256 == staged.package.firmware.sha256
            && firmware.md5 == staged.package.firmware.md5,
        "候选版本发生变化，请重新下载校验"
    );
    ensure!(
        !firmware.mandatory_clear,
        "官方要求此升级清除数据，请在手机官方更新器中确认"
    );
    ensure!(
        report
            .channels
            .iter()
            .filter(|c| c.channel == "current")
            .flat_map(|c| &c.releases)
            .any(|r| r.version == firmware.version
                && r.slot == "new"
                && r.relation == VersionRelation::Newer
                && r.size_bytes == firmware.size_bytes
                && r.sha256 == firmware.sha256
                && r.md5 == firmware.md5
                && r.need_update
                && r.exists_update
                && !r.mandatory_clear),
        "当前官方报告已不包含该可升级候选，请重新检查"
    );
    ensure!(
        staged.package.metadata.get("post-aios-version") == Some(&firmware.version),
        "固件元数据版本与官方候选不匹配"
    );
    let checked = chrono::NaiveDateTime::parse_from_str(&report.checked_at, "%Y-%m-%d %H:%M:%S")
        .context("无法确认官方报告时间，请重新检查")?;
    let age = chrono::Local::now().naive_local() - checked;
    ensure!(
        (0..=600).contains(&age.num_seconds()),
        "官方报告已过期，请重新检查更新"
    );
    let raw_props = adb.script(
        &device.transport,
        "v=$(getprop ro.build.mask.id); if [ -z \"$v\" ]; then v=$(getprop ro.build.aios_version); fi; printf '%s\\n' \"$v\"; getprop ro.product.device; getprop ro.build.version.incremental; getprop ro.build.fingerprint; getprop ro.build.date.utc; getprop ro.boot.shopdemo; getprop ro.sys.shopdemo; getprop ro.demo_mode",
    )?;
    let (current, props) = raw_props.split_once('\n').unwrap_or((&raw_props, ""));
    ensure!(
        current.trim() == report.current_version,
        "手机系统已变化，请重新检查更新"
    );
    validate_device(&staged.package.metadata, props)?;
    let actual = adb.run_timeout(
        &[
            "-s",
            &device.transport,
            "shell",
            &format!("sha256sum {}", crate::adb::shell_quote(&staged.remote_path)),
        ],
        Duration::from_secs(240),
    )?;
    ensure!(
        actual.split_whitespace().next() == Some(staged.package.sha256.as_str()),
        "手机固件文件已变化，未提交安装"
    );
    // Signature-protected official components are not bypassed with root or raw update_engine calls.
    let permission = crate::ota::install_status(adb, device)?;
    if permission.internal_permission != Some(true) {
        open_official(adb, device)?;
        return Ok(InstallSubmission{message:"固件已传到手机并校验完成；系统未授权桌面后台安装，已打开手机官方更新页面，请由官方更新器确认安装".into(),accepted:false});
    }
    ensure!(
        permission.shop_demo == Some(false),
        "无法确认手机不是演示机；此模式可能强制清除数据，请在官方更新器中处理"
    );
    let relative = staged
        .remote_path
        .strip_prefix("/sdcard/")
        .context("固件路径无效")?;
    let output = adb.shell(
        &device.transport,
        &[
            "am",
            "broadcast",
            "-n",
            "com.stepos.update/.receiver.ABSystemUpdateReceiver",
            "-a",
            "com.stepos.update.action.AB_SYSTEM_INSTALL_FROM_APP",
            "--es",
            "upgrade_locate_filepath",
            relative,
            "--ez",
            "wipe_userdata",
            "false",
            "--ez",
            "auto_upgrade",
            "false",
        ],
    )?;
    ensure!(
        !output.contains("Permission Denial") && !output.contains("SecurityException"),
        "手机拒绝后台安装权限，请使用官方系统更新页面"
    );
    open_official(adb, device)?;
    Ok(InstallSubmission{message:"已向官方更新器提交校验请求；尚未确认安装受理，请在手机官方更新页面查看结果。不会自动重启".into(),accepted:false})
}
fn open_official(adb: &Adb, device: &Device) -> Result<()> {
    adb.shell(
        &device.transport,
        &[
            "am",
            "start",
            "-a",
            "stepos.settings.SYSTEM_UPDATE_SETTINGS",
            "-p",
            "com.stepos.update",
        ],
    )?;
    Ok(())
}
fn validate_device(meta: &BTreeMap<String, String>, props: &str) -> Result<()> {
    let p: Vec<_> = props.lines().map(str::trim).collect();
    ensure!(p.len() >= 4, "无法读取完整的手机固件身份");
    ensure!(
        meta.get("pre-device")
            .is_some_and(|s| s.split('|').any(|v| v == p[0])),
        "固件不适用于当前手机型号"
    );
    if let Some(pre) = meta.get("pre-build") {
        ensure!(
            pre.split('|').any(|s| s == p[2]),
            "增量固件的前置系统版本不匹配"
        );
    }
    if let Some(pre) = meta.get("pre-build-incremental") {
        ensure!(
            pre.split('|').any(|s| s == p[1]),
            "增量固件的前置版本号不匹配"
        );
    }
    let current = p[1]
        .parse::<u64>()
        .context("无法确认本机内部版本号，不能安装")?;
    let next = meta
        .get("post-build-incremental")
        .and_then(|s| s.parse::<u64>().ok())
        .context("固件内部版本号无效")?;
    ensure!(next > current, "固件不是更高内部版本，已阻止重复安装或降级");
    let now = p[3].parse::<u64>().context("无法确认本机编译时间")?;
    let then = meta
        .get("post-timestamp")
        .and_then(|s| s.parse::<u64>().ok())
        .context("固件编译时间无效")?;
    ensure!(then > now, "固件编译时间不比本机新，已阻止安装");
    ensure!(
        p.iter()
            .skip(4)
            .all(|v| !matches!(*v, "1" | "true" | "yes")),
        "演示机安装可能清除数据，桌面不支持此安装模式"
    );
    ensure!(
        !meta
            .get("ota-wipe")
            .is_some_and(|s| matches!(s.as_str(), "yes" | "1" | "true")),
        "此固件要求清除数据，桌面不支持此安装模式"
    );
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    fn meta() -> BTreeMap<String, String> {
        [
            ("pre-device", "pandora"),
            ("post-build-incremental", "173"),
            ("post-timestamp", "200"),
        ]
        .into_iter()
        .map(|(k, v)| (k.into(), v.into()))
        .collect()
    }
    #[test]
    fn rejects_wrong_device_same_version_and_wipe() {
        let m = meta();
        assert!(validate_device(&m, "pandora\n172\nfingerprint\n100\n0\n0\n0").is_ok());
        assert!(validate_device(&m, "other\n172\nfingerprint\n100").is_err());
        assert!(validate_device(&m, "pandora\n173\nfingerprint\n100").is_err());
        let mut m = m;
        m.insert("ota-wipe".into(), "yes".into());
        assert!(validate_device(&m, "pandora\n172\nfingerprint\n100").is_err());
    }
    #[test]
    fn rejects_incremental_base_mismatch() {
        let mut m = meta();
        m.insert("pre-build".into(), "other-fingerprint".into());
        assert!(validate_device(&m, "pandora\n172\nfingerprint\n100").is_err());
    }
}

#[derive(Clone, Debug, Default)]
pub struct StagingUsage {
    pub count: u64,
    pub bytes: u64,
}
fn staged_directories(adb: &Adb, device: &Device) -> Result<Vec<(String, u64)>> {
    device.verify(adb)?;
    let root = "/sdcard/Download/StepX-OTA";
    let output = adb.script(
        &device.transport,
        "if [ -d /sdcard/Download/StepX-OTA ] && [ ! -L /sdcard/Download/StepX-OTA ]; then \
           for d in /sdcard/Download/StepX-OTA/*; do \
             if [ -d \"$d\" ] && [ ! -L \"$d\" ]; then \
               name=${d##*/}; \
               for f in \"$d/update.zip\" \"$d/update.zip.part\"; do \
                 if [ -f \"$f\" ] && [ ! -L \"$f\" ]; then \
                   s=$(stat -c %s \"$f\" 2>/dev/null); \
                   if [ -n \"$s\" ]; then printf '%s\\t%s\\n' \"$name\" \"$s\"; fi; \
                 fi; \
               done; \
             fi; \
           done; \
         fi",
    )?;
    let mut totals: BTreeMap<String, u64> = BTreeMap::new();
    let mut order: Vec<String> = Vec::new();
    for line in output.lines() {
        let mut parts = line.trim().split('\t');
        let (Some(name), Some(size_str)) = (parts.next(), parts.next()) else {
            continue;
        };
        if name.len() != 32 || !name.bytes().all(|c| c.is_ascii_hexdigit()) {
            continue;
        }
        let Ok(bytes) = size_str.trim().parse::<u64>() else {
            continue;
        };
        if !totals.contains_key(name) {
            if order.len() >= 512 {
                continue;
            }
            order.push(name.to_owned());
            totals.insert(name.to_owned(), 0);
        }
        if let Some(total) = totals.get_mut(name) {
            *total = total.saturating_add(bytes);
        }
    }
    Ok(order
        .into_iter()
        .filter_map(|name| {
            let bytes = totals.remove(&name).unwrap_or(0);
            (bytes > 0).then(|| (format!("{root}/{name}"), bytes))
        })
        .collect())
}
/// Only recognizes application-owned 32-hex directories; never scans unrelated downloads.
pub fn inspect_staged(adb: &Adb, device: &Device) -> Result<StagingUsage> {
    let dirs = staged_directories(adb, device)?;
    Ok(StagingUsage {
        count: dirs.len() as u64,
        bytes: dirs.iter().map(|(_, bytes)| bytes).sum(),
    })
}
/// User-triggered cleanup. Do not call while a transfer or system installation is active.
pub fn cleanup_staged(adb: &Adb, device: &Device) -> Result<StagingUsage> {
    let dirs = staged_directories(adb, device)?;
    if dirs.is_empty() {
        return Ok(StagingUsage::default());
    }
    device.verify(adb)?;
    let mut script = String::new();
    let mut removed = StagingUsage::default();
    for (dir, bytes) in dirs {
        let zip = crate::adb::shell_quote(&format!("{dir}/update.zip"));
        let part = crate::adb::shell_quote(&format!("{dir}/update.zip.part"));
        let dir_q = crate::adb::shell_quote(&dir);
        // Preserve any unexpected files instead of recursively removing the folder.
        script.push_str(&format!(
            "rm -f {zip} {part}; rmdir {dir_q} 2>/dev/null || true; "
        ));
        removed.count += 1;
        removed.bytes += bytes;
    }
    adb.script(&device.transport, &script)?;
    Ok(removed)
}
