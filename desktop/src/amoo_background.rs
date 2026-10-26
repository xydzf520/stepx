//! Short task leases use the phone's existing protected CloudWake receiver.
//! No screen state, credentials, native executable or global power policy is changed.
use crate::{adb::Adb, device::Device};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{mpsc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

const SERVICE: &str = "com.stepx.companion.app/.AmooConnectionService";
const STATUS_PATH: &str = "/sdcard/Android/data/com.stepx.companion.app/files/amoo-connection.json";
const FINGERPRINT: &str =
    "StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys";
const NATIVE_APK: &str = "/system/priv-app/AgentBus/AgentBus.apk";
const NATIVE_SHA: &str = "50a1bbd6af20d1cceff335e2336cff04e08d3718cf4cad581a86954c354f2a48";
const UNSUPPORTED: &str = "当前手机尚未通过 Amoo 熄屏适配核验。请在小工具安装或修复 StepX伴侣，或点亮手机后重试；无需打开 Amoo 界面";
const PREPARE_CACHE_TTL: Duration = Duration::from_secs(180);
const PREPARE_NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(30);
// The phone returns acknowledgements in one atomic status file. Serialize only
// commands/acknowledgements, never the task lifetime or model traffic.
static COMMAND_LOCK: Mutex<()> = Mutex::new(());
static PREPARE_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Value)>>> = OnceLock::new();

fn prepare_cache() -> &'static Mutex<HashMap<String, (Instant, Value)>> {
    PREPARE_CACHE.get_or_init(|| Mutex::new(HashMap::new()))
}

fn cached_prepare(adb: &Adb, device: &Device) -> Result<Value> {
    if !device.serial.is_empty() {
        if let Ok(mut guard) = prepare_cache().lock() {
            if let Some((at, value)) = guard.get(&device.serial) {
                let ttl = if value["installed"] == true {
                    PREPARE_CACHE_TTL
                } else {
                    PREPARE_NEGATIVE_CACHE_TTL
                };
                if at.elapsed() < ttl {
                    return Ok(value.clone());
                }
                guard.remove(&device.serial);
            }
        }
    }
    prepare(adb, device)
}

pub(crate) fn prepare_from_parts(
    _transport: &str,
    serial: &str,
    fingerprint: &str,
    sdk: &str,
    sha: &str,
    dump: &str,
) -> Result<Value> {
    let result = if fingerprint.trim() != FINGERPRINT || sdk.trim() != "36" {
        json!({"status":"unsupported","supported":false,"installed":false,
            "background_ready":false,"reason":"firmware_mismatch","message":UNSUPPORTED})
    } else if sha.split_whitespace().next() != Some(NATIVE_SHA) {
        json!({"status":"unsupported","supported":false,"installed":false,
            "background_ready":false,"reason":"native_version_mismatch","message":UNSUPPORTED})
    } else {
        let permissions = crate::companion_app::permission_status(dump);
        let missing: Vec<_> = permissions
            .iter()
            .filter(|p| !p.granted)
            .map(|p| p.name)
            .collect();
        let installed = permissions.len() == 15 && missing.is_empty();
        json!({"status":if installed {"verified"} else {"permissions_required"},
            "supported":true,"installed":installed,"background_ready":false,
            "permissionCount":permissions.len(),"missingPermissions":missing,
            "reason":if installed {"task_lease_required"} else {"permissions_required"},
            "message":if installed {"已核验受保护后台入口，任务连接时建立短租约；以真实任务回执为准"}
                else {"后台协作安装权限不完整，请在小工具安装或修复 StepX伴侣"}})
    };
    if !serial.is_empty() {
        if let Ok(mut guard) = prepare_cache().lock() {
            guard.insert(serial.to_string(), (Instant::now(), result.clone()));
        }
    }
    Ok(result)
}

/// Read-only compatibility and installation checks; never invokes the old loader.
pub fn prepare(adb: &Adb, device: &Device) -> Result<Value> {
    device.verify(adb)?;
    let raw = adb.script(
        &device.transport,
        &format!(
            "getprop ro.build.fingerprint; getprop ro.build.version.sdk; sha256sum {NATIVE_APK} 2>/dev/null || printf '\\n'; printf '<<<STEPX_AMOO_DUMP>>>\\n'; dumpsys package com.stepx.companion.app"
        ),
    )?;
    let (header, dump) = raw
        .split_once("<<<STEPX_AMOO_DUMP>>>\n")
        .unwrap_or((&raw, ""));
    let mut lines = header.lines();
    let fingerprint = lines.next().unwrap_or("");
    let sdk = lines.next().unwrap_or("");
    let sha = lines.next().unwrap_or("");
    prepare_from_parts(
        &device.transport,
        &device.serial,
        fingerprint,
        sdk,
        sha,
        dump,
    )
}

fn command(adb: &Adb, device: &Device, op: &str, lease: Option<&str>) -> Result<Value> {
    let _guard = COMMAND_LOCK.lock().unwrap_or_else(|e| e.into_inner());
    device.verify(adb)?;
    let id = format!("desktop-{:032x}", rand::random::<u128>());
    let mut args = vec![
        "am",
        "start-foreground-service",
        "--user",
        "0",
        "-n",
        SERVICE,
        "--es",
        "op",
        op,
        "--es",
        "request_id",
        &id,
    ];
    if let Some(token) = lease {
        args.extend(["--es", "lease_id", token]);
    }
    let output = adb.shell(&device.transport, &args)?;
    ensure!(
        !output.contains("Error:") && !output.contains("Exception"),
        "无法启动 StepX伴侣后台服务"
    );
    let deadline = Instant::now() + Duration::from_secs(15);
    const POLL_BACKOFF_MS: [u64; 4] = [40, 80, 140, 200];
    let mut poll_step = 0usize;
    loop {
        if let Ok(raw) = adb.shell(&device.transport, &["cat", STATUS_PATH]) {
            if raw.len() <= 64 * 1024 {
                if let Ok(value) = serde_json::from_str::<Value>(&raw) {
                    if value["requestId"] == id {
                        if let Some(error) = value["lastError"].as_str().filter(|x| !x.is_empty()) {
                            bail!("{error}");
                        }
                        return Ok(value);
                    }
                }
            }
        }
        ensure!(
            Instant::now() < deadline,
            "StepX伴侣尚未确认后台操作，请检查手机连接后重试"
        );
        let wait_ms = POLL_BACKOFF_MS[poll_step.min(POLL_BACKOFF_MS.len() - 1)];
        poll_step = poll_step.saturating_add(1);
        thread::sleep(Duration::from_millis(wait_ms));
    }
}

pub fn set_enabled(adb: &Adb, device: &Device, enabled: bool) -> Result<Value> {
    if enabled {
        let capability = prepare(adb, device)?;
        ensure!(
            capability["supported"] == true && capability["installed"] == true,
            "{}",
            capability["message"].as_str().unwrap_or(UNSUPPORTED)
        );
    }
    // Explicit disable always works, including after a native firmware update.
    command(
        adb,
        device,
        if enabled { "enable" } else { "disable" },
        None,
    )
}

fn interactive_from_power_dump(dump: &str) -> Result<bool> {
    let state = dump
        .lines()
        .find_map(|line| line.trim().strip_prefix("mWakefulness="))
        .context("无法确认手机当前屏幕状态，请检查手机连接后重试")?;
    match state.trim() {
        "Awake" | "Dreaming" => Ok(true),
        "Asleep" | "Dozing" => Ok(false),
        _ => bail!("手机返回了无法识别的屏幕状态，请检查手机连接后重试"),
    }
}

pub fn connection_failure_message(adb: &Adb, device: &Device) -> &'static str {
    match adb.shell(&device.transport, &["dumpsys", "power", "--brief"]).ok()
        .and_then(|dump| interactive_from_power_dump(&dump).ok()) {
        Some(false) => "Amoo 熄屏连接尚未收到真实回执。请检查手机网络及 StepX伴侣的后台协作开关；未确认发送的草稿与附件已保留",
        _ => "Amoo 连接中断，请检查手机网络后重试；未确认发送的草稿与附件已保留",
    }
}

pub struct Lease {
    adb: Adb,
    device: Device,
    id: String,
    stop: mpsc::Sender<()>,
}
impl Lease {
    pub fn acquire(adb: &Adb, device: &Device) -> Result<Option<Self>> {
        device.verify(adb)?;
        // Capability failure/explicit stop does not prohibit an already working
        // authentic native connection, including one with its own CloudWake.
        let capability = match cached_prepare(adb, device) {
            Ok(value) => value,
            Err(_) => return Ok(None),
        };
        if capability["supported"] != true || capability["installed"] != true {
            return Ok(None);
        }
        let id = format!("task-{:032x}", rand::random::<u128>());
        let state = match command(adb, device, "lease", Some(&id)) {
            Ok(value) => value,
            Err(_) => {
                if !device.serial.is_empty() {
                    if let Ok(mut guard) = prepare_cache().lock() {
                        guard.remove(&device.serial);
                    }
                }
                return Ok(None);
            }
        };
        if state["enabled"] != true || state["active"] != true {
            return Ok(None);
        }
        let (stop, receiver) = mpsc::channel();
        let task_adb = adb.clone();
        let task_device = device.clone();
        let task_id = id.clone();
        thread::Builder::new()
            .name("amoo-task-lease".into())
            .spawn(move || {
                loop {
                    match receiver.recv_timeout(Duration::from_secs(25)) {
                        Err(mpsc::RecvTimeoutError::Timeout) => {
                            if command(&task_adb, &task_device, "renew", Some(&task_id)).is_err() {
                                break;
                            }
                        }
                        _ => {
                            // Keep a brief overlap window so send -> SyncWorker handoff
                            // never drops the phone's last active lease in between.
                            thread::sleep(Duration::from_secs(4));
                            break;
                        }
                    }
                }
                // Release only this connection. On disconnection the phone's 90 s
                // monotonic expiry remains the fallback; there is no global release.
                let _ = command(&task_adb, &task_device, "release", Some(&task_id));
            })?;
        let lease = Self {
            adb: adb.clone(),
            device: device.clone(),
            id,
            stop,
        };
        // A newly started phone service intentionally waits for the native 10 s
        // screen-off debounce to finish. Respect the native request rate limit; the
        // subsequent original AgentBus handshake still determines success.
        if state["screenInteractive"] == false && state["wakeRequested"] != true {
            let wait_ms = state["nextWakeInMs"].as_u64().unwrap_or(15_000).min(60_000) + 5_000;
            let deadline = Instant::now() + Duration::from_millis(wait_ms);
            loop {
                if Instant::now() >= deadline {
                    break;
                }
                thread::sleep(Duration::from_millis(500));
                let Ok(raw) = adb.shell(&device.transport, &["cat", STATUS_PATH]) else {
                    break;
                };
                let Ok(current) = serde_json::from_str::<Value>(&raw) else {
                    break;
                };
                if current["active"] != true
                    || current["enabled"] != true
                    || current["screenInteractive"] == true
                    || current["wakeRequested"] == true
                {
                    break;
                }
            }
        }
        Ok(Some(lease))
    }

    /// Called only after the genuine protocol-3 AgentBus handshake succeeds.
    /// This is not evidence of cloud ONLINE or completion of a model task.
    pub fn mark_connected(&self) -> Result<()> {
        command(&self.adb, &self.device, "mark-connected", Some(&self.id))?;
        Ok(())
    }
}
impl Drop for Lease {
    fn drop(&mut self) {
        let _ = self.stop.send(());
    }
}

#[cfg(test)]
mod tests {
    use super::interactive_from_power_dump;
    #[test]
    fn screen_gate_uses_system_wakefulness_not_wakelock_or_uid_activity() {
        assert!(interactive_from_power_dump("mWakefulness=Awake\nmWakeLockSummary=0x0").unwrap());
        assert!(interactive_from_power_dump(" mWakefulness=Dreaming").unwrap());
        assert!(!interactive_from_power_dump(
            "mWakefulness=Asleep\nUID 1000: ACTIVE\nPARTIAL_WAKE_LOCK held"
        )
        .unwrap());
        assert!(!interactive_from_power_dump("mWakefulness=Dozing\nDisplay state=DOZE").unwrap());
    }
    #[test]
    fn missing_or_unknown_power_state_is_not_claimed_ready() {
        assert!(interactive_from_power_dump("mHalInteractiveModeEnabled=true").is_err());
        assert!(interactive_from_power_dump("mWakefulness=unknown").is_err());
    }
}
