//! Short task leases use the phone's existing protected CloudWake receiver.
//! No screen state, credentials, native executable or global power policy is changed.
use crate::{adb::Adb, device::Device};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    collections::HashMap,
    sync::{mpsc, Arc, Mutex, OnceLock, Weak},
    thread,
    time::{Duration, Instant},
};

const SERVICE: &str = "com.stepx.companion.app/.AmooConnectionService";
const STATUS_PATH: &str = "/sdcard/Android/data/com.stepx.companion.app/files/amoo-connection.json";
pub const FP_172: &str =
    "StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys";
pub const FP_12: &str =
    "StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/12:user/release-keys";
pub const EXPECTED_FINGERPRINTS: [&str; 2] = [FP_172, FP_12];
const NATIVE_APK: &str = "/system/priv-app/AgentBus/AgentBus.apk";
pub const SHA_172: &str = "50a1bbd6af20d1cceff335e2336cff04e08d3718cf4cad581a86954c354f2a48";
pub const SHA_12: &str = "e89a65e1a153e87fdec850caffe9a19db4ed70244754e847e0c512c540a902f8";
pub const EXPECTED_AGENTBUS_SHAS: [&str; 2] = [SHA_172, SHA_12];
pub const EXPECTED_SDK_VERSION: &str = "36";
const UNSUPPORTED: &str = "当前手机尚未通过 Amoo 熄屏适配核验。请在小工具安装或修复 StepX伴侣，或点亮手机后重试；无需打开 Amoo 界面";
// Only permissions used by the CPU lease/CloudWake service gate task
// connectivity. Pairing, camera and back-screen features are independent.
const BACKGROUND_PERMISSIONS: &[&str] = &[
    "com.stepos.cloud.push.permission.MESSAGE",
    "android.permission.WAKE_LOCK",
    "android.permission.FOREGROUND_SERVICE",
    "android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE",
    "android.permission.INTERNET",
    "android.permission.ACCESS_NETWORK_STATE",
    "android.permission.ACCESS_WIFI_STATE",
    "android.permission.CHANGE_NETWORK_STATE",
];
const PREPARE_CACHE_TTL: Duration = Duration::from_secs(180);
const PREPARE_NEGATIVE_CACHE_TTL: Duration = Duration::from_secs(30);
// The phone returns acknowledgements in one atomic status file. Serialize only
// commands/acknowledgements per phone, never the task lifetime, model traffic,
// or an unrelated phone. Weak entries keep disconnected devices out of the map.
static COMMAND_LOCKS: OnceLock<Mutex<HashMap<String, Weak<Mutex<()>>>>> = OnceLock::new();
static PREPARE_CACHE: OnceLock<Mutex<HashMap<String, (Instant, Value)>>> = OnceLock::new();

#[inline]
pub fn is_supported_firmware_fingerprint(fingerprint: &str) -> bool {
    let fp = fingerprint.trim();
    EXPECTED_FINGERPRINTS.contains(&fp)
}

#[inline]
pub fn is_supported_agentbus_sha256(sha_output: &str) -> bool {
    sha_output
        .split_whitespace()
        .next()
        .is_some_and(|digest| EXPECTED_AGENTBUS_SHAS.contains(&digest))
}

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
    let result = if !is_supported_firmware_fingerprint(fingerprint) || sdk.trim() != EXPECTED_SDK_VERSION {
        json!({"status":"unsupported","supported":false,"installed":false,
            "background_ready":false,"reason":"firmware_mismatch","message":UNSUPPORTED})
    } else if !is_supported_agentbus_sha256(sha) {
        json!({"status":"unsupported","supported":false,"installed":false,
            "background_ready":false,"reason":"native_version_mismatch","message":UNSUPPORTED})
    } else {
        let permissions: Vec<_> = crate::companion_app::permission_status(dump).into_iter()
            .filter(|permission| BACKGROUND_PERMISSIONS.contains(&permission.name)).collect();
        let missing: Vec<_> = permissions
            .iter()
            .filter(|p| !p.granted)
            .map(|p| p.name)
            .collect();
        let installed = permissions.len() == BACKGROUND_PERMISSIONS.len() && missing.is_empty();
        json!({"status":if installed {"verified"} else {"permissions_required"},
            "supported":true,"installed":installed,"background_ready":false,
            "permissionCount":permissions.len(),"missingPermissions":missing,
            "reason":if installed {"task_lease_required"} else {"permissions_required"},
            "message":if installed {"已核验受保护后台入口，任务连接时建立短租约；以真实任务回执为准"}
                else {"伴侣缺少后台连接权限，请在小工具修复 StepX伴侣；无需解锁，资料同步继续运行"}})
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
    let mut device = device.clone();
    device.transport = device.resolve_active_transport(adb)?;
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

fn command_lock(device: &Device, transport: &str) -> Arc<Mutex<()>> {
    let serial = crate::device::canonical_serial_from_transport(&device.serial)
        .or_else(|| crate::device::canonical_serial_from_transport(transport));
    let key = serial.map(|serial| format!("serial:{serial}"))
        .unwrap_or_else(|| format!("transport:{}", transport.trim()));
    let mut locks = COMMAND_LOCKS.get_or_init(|| Mutex::new(HashMap::new()))
        .lock().unwrap_or_else(|e| e.into_inner());
    locks.retain(|_, lock| lock.strong_count() > 0);
    if let Some(lock) = locks.get(&key).and_then(Weak::upgrade) {
        return lock;
    }
    let lock = Arc::new(Mutex::new(()));
    locks.insert(key, Arc::downgrade(&lock));
    lock
}

fn command(adb: &Adb, device: &Device, op: &str, lease: Option<&str>) -> Result<Value> {
    let transport = device.resolve_active_transport(adb)?;
    let command_lock = command_lock(device, &transport);
    // The phone exposes one atomic acknowledgement file. Keep this guard until
    // our requestId is verified, including errors/timeouts, so a concurrent
    // lease/status/renew cannot replace a receipt before its caller reads it.
    let _guard = command_lock.lock().unwrap_or_else(|e| e.into_inner());
    let id = format!("desktop-{:032x}", rand::random::<u128>());
    let transport = {
        // A command waiting behind another receipt may now have a new transport.
        let transport = device.resolve_active_transport(adb)?;
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
        let output = adb.shell(&transport, &args)?;
        ensure!(
            !output.contains("Error:") && !output.contains("Exception"),
            "无法启动 StepX伴侣后台服务"
        );
        transport
    };
    let deadline = Instant::now() + Duration::from_secs(15);
    const POLL_BACKOFF_MS: [u64; 4] = [40, 80, 140, 200];
    let mut poll_step = 0usize;
    loop {
        let active = device
            .resolve_active_transport(adb)
            .unwrap_or_else(|_| transport.clone());
        if let Ok(raw) = adb.shell(&active, &["cat", STATUS_PATH]) {
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

/// Refresh a repaired installation's capability cache and phone diagnostic
/// receipt without changing the user's enabled preference or acquiring a lease.
pub(crate) fn refresh_status(adb: &Adb, device: &Device) -> Result<Value> {
    prepare(adb, device)?;
    command(adb, device, "status", None)
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
    let transport = device
        .resolve_active_transport(adb)
        .unwrap_or_else(|_| device.transport.clone());
    match adb.shell(&transport, &["dumpsys", "power", "--brief"]).ok()
        .and_then(|dump| interactive_from_power_dump(&dump).ok()) {
        Some(false) => "Amoo 熄屏连接尚未收到真实回执。请检查手机网络及 StepX伴侣的后台协作开关；未确认发送的草稿与附件已保留",
        _ => "Amoo 连接中断，请检查手机网络后重试；未确认发送的草稿与附件已保留",
    }
}

/// Explain an observed task failure using verified capability state. This is
/// separate from basic device/data connectivity and never enables a service.
pub fn task_failure_hint(adb: &Adb, device: &Device) -> String {
    match cached_prepare(adb, device) {
        Ok(capability) if capability["reason"] == "permissions_required" =>
            "Amoo 未确认发送，草稿已保留。手机伴侣缺少后台连接权限；请到小工具 → StepX伴侣执行修复后重试，资料同步不受影响".into(),
        Ok(capability) if capability["supported"] == false =>
            "Amoo 未确认发送，草稿已保留。当前手机固件的后台连接尚未通过适配核验；请查看小工具 → StepX伴侣中的诊断".into(),
        _ => "Amoo 未确认发送，草稿已保留。请检查手机网络与伴侣后台协作状态，刷新会话核对后重试".into(),
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
        let mut device = device.clone();
        device.transport = device.resolve_active_transport(adb)?;
        let device = &device;
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
                let active = device
                    .resolve_active_transport(adb)
                    .unwrap_or_else(|_| device.transport.clone());
                let Ok(raw) = adb.shell(&active, &["cat", STATUS_PATH]) else {
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
    fn command_receipts_share_a_lock_across_normalized_device_transports() {
        use super::{command_lock, Arc, Device};
        let serial = format!("ack-test-{:016x}", rand::random::<u64>());
        let usb = Device { serial: format!(" {serial} "), ..Default::default() };
        let wireless = Device { serial: serial.clone(), ..Default::default() };
        let mdns = Device::default();
        let first = command_lock(&usb, &serial);
        assert!(Arc::ptr_eq(&first, &command_lock(&wireless, "10.0.0.8:1234")));
        assert!(Arc::ptr_eq(&first, &command_lock(&mdns, &format!("adb-{serial}-nonce._adb-tls-connect._tcp"))));
        let unknown = command_lock(&Device::default(), " 10.0.0.9:4321 ");
        assert!(Arc::ptr_eq(&unknown, &command_lock(&Device::default(), "10.0.0.9:4321")));
        assert!(!Arc::ptr_eq(&first, &unknown));
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_commands_preserve_each_ack_without_blocking_another_phone() {
        use super::{command, Adb, Device, Duration, Instant};
        use std::{os::unix::fs::PermissionsExt, sync::mpsc, thread};
        let dir = tempfile::tempdir().unwrap();
        let first_serial = format!("ack-a-{:016x}", rand::random::<u64>());
        let other_serial = format!("ack-b-{:016x}", rand::random::<u64>());
        let path = dir.path().join("adb");
        let script = format!("#!/usr/bin/env python3\nROOT = {:?}\nFIRST = {:?}\n", dir.path().to_str().unwrap(), first_serial) + r#"
import json, pathlib, shlex, sys, time
root = pathlib.Path(ROOT)
args = sys.argv[1:]
assert args[0] == '-s' and args[2] == 'shell', args
serial = args[1]
tokens = shlex.split(' '.join(args[3:]))
if tokens == ['getprop', 'ro.serialno']:
    print(serial)
elif tokens[:2] == ['am', 'start-foreground-service']:
    request = tokens[tokens.index('request_id') + 1]
    op = tokens[tokens.index('op') + 1]
    state = {'requestId': request, 'lastError': '', 'operation': op}
    (root / (serial + '.json')).write_text(json.dumps(state))
    with (root / 'commands').open('a') as log:
        log.write(serial + ' ' + op + '\n')
elif tokens[0] == 'cat':
    if serial == FIRST and not (root / 'release').exists():
        (root / 'reading').touch()
        deadline = time.monotonic() + 8
        while not (root / 'release').exists() and time.monotonic() < deadline:
            time.sleep(0.005)
    print((root / (serial + '.json')).read_text())
else:
    raise AssertionError(tokens)
"#;
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let adb = Adb { path };
        let first = Device { serial: first_serial.clone(), transport: first_serial.clone(), ..Default::default() };
        let other = Device { serial: other_serial.clone(), transport: other_serial.clone(), ..Default::default() };
        let first_adb = adb.clone();
        let first_device = first.clone();
        let lease = thread::spawn(move || command(&first_adb, &first_device, "lease", Some("task-fixture-0001")));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dir.path().join("reading").exists() {
            assert!(Instant::now() < deadline, "first command did not reach its acknowledgement read");
            thread::sleep(Duration::from_millis(5));
        }
        let (status_tx, status_rx) = mpsc::channel();
        let status_adb = adb.clone();
        let status = thread::spawn(move || {
            let result = command(&status_adb, &first, "status", None);
            status_tx.send(result).unwrap();
        });
        let (other_tx, other_rx) = mpsc::channel();
        let other_status = thread::spawn(move || {
            other_tx.send(command(&adb, &other, "status", None)).unwrap();
        });
        // A different phone must finish while the first phone is still waiting.
        let other_result = other_rx.recv_timeout(Duration::from_secs(3));
        thread::sleep(Duration::from_millis(100));
        let before_release = std::fs::read_to_string(dir.path().join("commands")).unwrap();
        std::fs::write(dir.path().join("release"), b"ready").unwrap();
        let first_result = lease.join().unwrap();
        let status_result = status_rx.recv_timeout(Duration::from_secs(5));
        status.join().unwrap();
        other_status.join().unwrap();
        assert_eq!(other_result.unwrap().unwrap()["operation"], "status");
        assert!(!before_release.contains(&format!("{first_serial} status")),
            "a second command replaced the first phone's unread receipt: {before_release}");
        let lease_ack = first_result.unwrap();
        let status_ack = status_result.unwrap().unwrap();
        assert_eq!(lease_ack["operation"], "lease");
        assert_eq!(status_ack["operation"], "status");
        assert_ne!(lease_ack["requestId"], status_ack["requestId"]);
    }

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
    #[test]
    fn desktop_and_phone_require_the_same_background_permissions() {
        let java = include_str!("../companion-app/src/com/stepx/companion/app/AmooConnectionService.java");
        let array = java.split("String[] PERMISSIONS={").nth(1).unwrap().split("};").next().unwrap();
        let permissions: std::collections::BTreeSet<_> = array.split(',').map(|s| s.trim().trim_matches('"')).collect();
        assert_eq!(permissions, super::BACKGROUND_PERMISSIONS.iter().copied().collect());
    }
    #[test]
    fn background_readiness_does_not_require_pairing_camera_or_backscreen() {
        let dump = super::BACKGROUND_PERMISSIONS.iter()
            .map(|name| format!("  {name}: granted=true\n")).collect::<String>();
        let ready = super::prepare_from_parts("wireless", "", super::FP_12, "36", super::SHA_12, &dump).unwrap();
        assert_eq!(ready["installed"], true);
        assert_eq!(ready["permissionCount"], super::BACKGROUND_PERMISSIONS.len());
        let missing_wake = dump.replace("com.stepos.cloud.push.permission.MESSAGE: granted=true", "com.stepos.cloud.push.permission.MESSAGE: granted=false");
        let unavailable = super::prepare_from_parts("wireless", "", super::FP_12, "36", super::SHA_12, &missing_wake).unwrap();
        assert_eq!(unavailable["installed"], false);
        assert_eq!(unavailable["missingPermissions"], serde_json::json!(["com.stepos.cloud.push.permission.MESSAGE"]));
    }
    #[test]
    fn prepare_from_parts_supports_both_pandora_172_and_pandora_12() {
        use super::{prepare_from_parts, FP_12, FP_172, SHA_12, SHA_172};
        let all_granted_dump = concat!(
            "    install permissions:\n",
            "      com.stepos.permission.MANAGE_BACK_SCREEN: granted=true\n",
            "      com.stepos.cloud.push.permission.MESSAGE: granted=true\n",
            "      android.permission.MANAGE_DEBUGGING: granted=true\n",
            "      android.permission.WRITE_SECURE_SETTINGS: granted=true\n",
            "      android.permission.WAKE_LOCK: granted=true\n",
            "      android.permission.FOREGROUND_SERVICE: granted=true\n",
            "      android.permission.FOREGROUND_SERVICE_SPECIAL_USE: granted=true\n",
            "      android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE: granted=true\n",
            "      android.permission.INTERNET: granted=true\n",
            "      android.permission.ACCESS_NETWORK_STATE: granted=true\n",
            "      android.permission.ACCESS_WIFI_STATE: granted=true\n",
            "      android.permission.CHANGE_NETWORK_STATE: granted=true\n",
            "    User 0: installed=true\n",
            "      runtime permissions:\n",
            "        android.permission.CAMERA: granted=true\n",
            "        android.permission.POST_NOTIFICATIONS: granted=true\n",
            "        android.permission.READ_PHONE_STATE: granted=true\n",
        );

        let res_172 = prepare_from_parts(
            "usb-172",
            "SER172",
            FP_172,
            "36",
            &format!("{SHA_172}  /system/priv-app/AgentBus/AgentBus.apk"),
            all_granted_dump,
        )
        .unwrap();
        assert_eq!(res_172["supported"], true);
        assert_eq!(res_172["installed"], true);
        assert_eq!(res_172["status"], "verified");

        let res_12 = prepare_from_parts(
            "usb-12",
            "SER12",
            FP_12,
            "36",
            &format!("{SHA_12}  /system/priv-app/AgentBus/AgentBus.apk"),
            all_granted_dump,
        )
        .unwrap();
        assert_eq!(res_12["supported"], true);
        assert_eq!(res_12["installed"], true);
        assert_eq!(res_12["status"], "verified");

        let bad_fp = prepare_from_parts(
            "usb",
            "SER_BAD",
            "StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/999:user/release-keys",
            "36",
            SHA_12,
            all_granted_dump,
        )
        .unwrap();
        assert_eq!(bad_fp["supported"], false);
        assert_eq!(bad_fp["reason"], "firmware_mismatch");

        let bad_sha = prepare_from_parts(
            "usb",
            "SER_BAD_SHA",
            FP_12,
            "36",
            "0000000000000000000000000000000000000000000000000000000000000000",
            all_granted_dump,
        )
        .unwrap();
        assert_eq!(bad_sha["supported"], false);
        assert_eq!(bad_sha["reason"], "native_version_mismatch");
    }

    #[test]
    fn unified_firmware_and_agentbus_sha_helpers_handle_whitespace_and_edge_cases() {
        use super::{
            is_supported_agentbus_sha256, is_supported_firmware_fingerprint,
            EXPECTED_AGENTBUS_SHAS, EXPECTED_FINGERPRINTS, FP_12, FP_172, SHA_12, SHA_172,
        };
        assert_eq!(EXPECTED_FINGERPRINTS.len(), 2);
        assert_eq!(EXPECTED_AGENTBUS_SHAS.len(), 2);
        assert!(is_supported_firmware_fingerprint(FP_172));
        assert!(is_supported_firmware_fingerprint(&format!("  {FP_12}\r\n")));
        assert!(!is_supported_firmware_fingerprint(""));
        assert!(!is_supported_firmware_fingerprint("   "));
        assert!(!is_supported_firmware_fingerprint("StepX/pandora/unknown"));

        assert!(is_supported_agentbus_sha256(SHA_172));
        assert!(is_supported_agentbus_sha256(&format!(
            "  {SHA_12}  /system/priv-app/AgentBus/AgentBus.apk\n"
        )));
        assert!(!is_supported_agentbus_sha256(""));
        assert!(!is_supported_agentbus_sha256("   "));
        assert!(!is_supported_agentbus_sha256("deadbeef"));
    }
}
