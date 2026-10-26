//! Install and control the phone-local StepX伴侣 runtime. No game frames depend on ADB.
use crate::{
    adb::{run_process, Adb},
    device::{is_backscreen_corner, Device},
};
use anyhow::{bail, ensure, Context, Result};
use serde_json::{json, Value};
use std::{
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    thread,
    time::{Duration, Instant},
};

pub const PACKAGE: &str = "com.stepx.companion.app";
const SERVICE: &str = "com.stepx.companion.app/.CompanionService";
const FILES: &str = "/sdcard/Android/data/com.stepx.companion.app/files";
const STATUS: &str = "/sdcard/Android/data/com.stepx.companion.app/files/status.json";
const PERMISSION: &str = "com.stepos.permission.MANAGE_BACK_SCREEN";
const RUNTIME_PERMISSIONS: [&str; 3] = [
    "android.permission.CAMERA",
    "android.permission.POST_NOTIFICATIONS",
    "android.permission.READ_PHONE_STATE",
];
const APK: &[u8] = include_bytes!("../assets/stepx-companion.apk");
static REQUEST: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Default)]
pub struct CompanionStatus {
    pub checked: bool,
    pub installed: bool,
    pub version: String,
    pub message: String,
    pub permission_granted: bool,
    pub running: bool,
    pub mode: String,
    pub applied_title: String,
    pub permissions: Vec<CompanionPermission>,
    pub pairing_permission_granted: bool,
    pub pairing_ready: bool,
    pub pairing_message: String,
    pub background_enabled: Option<bool>,
    pub background_ready: Option<bool>,
    pub background_supported: Option<bool>,
    pub wireless_enabled: Option<bool>,
    pub background_message: String,
    pub doze_exempt: Option<bool>,
    pub oem_freezer_exempt: Option<bool>,
    pub power_message: String,
}

#[derive(Clone, Debug)]
pub struct CompanionPermission {
    pub name: &'static str,
    pub label: &'static str,
    pub purpose: &'static str,
    pub granted: bool,
}

fn granted(dump: &str, permission: &str) -> bool {
    // Runtime permissions are per Android user. Another profile's grant cannot
    // satisfy installation for user 0; normal/signature permissions are global.
    let runtime = RUNTIME_PERMISSIONS.contains(&permission);
    let mut user_zero_indent = None;
    for line in dump.lines() {
        let trimmed = line.trim();
        let indent = line.len() - line.trim_start().len();
        if trimmed.starts_with("User ") && trimmed.contains(':') {
            user_zero_indent = trimmed.starts_with("User 0:").then_some(indent);
            continue;
        }
        if user_zero_indent.is_some_and(|level| !trimmed.is_empty() && indent <= level) {
            user_zero_indent = None;
        }
        if runtime && user_zero_indent.is_none() {
            continue;
        }
        if trimmed.strip_prefix(permission).is_some_and(|tail| {
            tail.strip_prefix(": granted=true").is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(',') || rest.starts_with(' ')
            })
        }) {
            return true;
        }
    }
    false
}

pub(crate) fn permission_status(dump: &str) -> Vec<CompanionPermission> {
    [
        (PERMISSION, "背屏控制", "应用图案、动画和重力游戏"),
        (
            "com.stepos.cloud.push.permission.MESSAGE",
            "原生任务唤醒",
            "仅在用户任务期间请求原生短时连接，实际能力单独检测",
        ),
        (
            "android.permission.MANAGE_DEBUGGING",
            "无线配对",
            "通过 Android 配对服务建立受信任连接",
        ),
        (
            "android.permission.WRITE_SECURE_SETTINGS",
            "无线调试设置",
            "在手机明确开启后设置无线调试",
        ),
        (
            "android.permission.CAMERA",
            "扫码相机",
            "安装时授权，用于扫描电脑连接二维码",
        ),
        (
            "android.permission.POST_NOTIFICATIONS",
            "服务通知",
            "展示后台服务运行状态和停止入口",
        ),
        (
            "android.permission.READ_PHONE_STATE",
            "通话状态",
            "来电时暂停背屏游戏，不读取通话记录",
        ),
        (
            "android.permission.WAKE_LOCK",
            "后台 CPU",
            "任务租约有效时保持 CPU 工作，不点亮屏幕",
        ),
        (
            "android.permission.FOREGROUND_SERVICE",
            "前台服务",
            "持续任务的服务生命周期",
        ),
        (
            "android.permission.FOREGROUND_SERVICE_SPECIAL_USE",
            "背屏前台服务",
            "用户启用的背屏动画与传感器游戏",
        ),
        (
            "android.permission.FOREGROUND_SERVICE_CONNECTED_DEVICE",
            "设备连接服务",
            "与电脑协作的手机服务",
        ),
        (
            "android.permission.INTERNET",
            "网络通信",
            "连接电脑及后台服务",
        ),
        (
            "android.permission.ACCESS_NETWORK_STATE",
            "网络状态",
            "检测当前连接是否可用",
        ),
        (
            "android.permission.ACCESS_WIFI_STATE",
            "Wi-Fi 状态",
            "查看扫码连接使用的无线网络",
        ),
        (
            "android.permission.CHANGE_NETWORK_STATE",
            "设备网络协作",
            "满足 Android 设备连接服务的运行条件",
        ),
    ]
    .into_iter()
    .map(|(name, label, purpose)| CompanionPermission {
        name,
        label,
        purpose,
        granted: granted(dump, name),
    })
    .collect()
}

fn doze_exempt(text: &str) -> Result<bool> {
    let mut exempt = false;
    for line in text.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let fields: Vec<_> = line.split(',').collect();
        ensure!(
            fields.len() == 3
                && ["system", "system-excidle", "user"].contains(&fields[0])
                && fields[2].parse::<u32>().is_ok(),
            "无法识别 Android Doze 检测结果"
        );
        if fields[1] == PACKAGE && ["system", "user"].contains(&fields[0]) {
            exempt = true;
        }
    }
    Ok(exempt)
}

fn oem_exempt(text: &str) -> Result<bool> {
    let mut sections = std::collections::HashSet::new();
    let mut exempt = false;
    for line in text.lines().map(str::trim) {
        if let Some(section) = line.strip_suffix(" freeze white apps:") {
            ensure!(
                ["cloud", "user", "local"].contains(&section),
                "无法识别厂商冻结白名单"
            );
            sections.insert(section);
        } else if let Some(package) = line.strip_prefix("freeze white app:") {
            ensure!(!sections.is_empty(), "厂商冻结白名单缺少分类");
            exempt |= package.trim() == PACKAGE;
        }
    }
    ensure!(sections.len() == 3, "尚未读到完整的厂商冻结白名单");
    Ok(exempt)
}

/// These three runtime permissions are explicitly authorized as part of install.
/// Keep the list fixed to the app's actual camera, notification and call-state features.
pub fn grant_runtime_permissions(adb: &Adb, device: &Device) -> Result<()> {
    device.verify(adb)?;
    let mut script = String::new();
    for permission in RUNTIME_PERMISSIONS {
        script.push_str(&format!(
            "pm grant --user 0 {PACKAGE} {permission} || {{ printf 'STEPX_GRANT_FAILED:{permission}\\n'; exit 0; }}; "
        ));
    }
    script.push_str(&format!("dumpsys package {PACKAGE}"));
    let dump = adb
        .script(&device.transport, &script)
        .context("StepX伴侣已安装，但授予运行权限失败")?;
    for permission in RUNTIME_PERMISSIONS {
        if dump.contains(&format!("STEPX_GRANT_FAILED:{permission}")) {
            bail!("StepX伴侣已安装，但授予 {permission} 失败");
        }
        ensure!(
            granted(&dump, permission),
            "StepX伴侣已安装，但用户 0 的 {permission} 未通过授权回读"
        );
    }
    Ok(())
}

fn apply_power_exemption(adb: &Adb, device: &Device, apk: &Path) -> Result<()> {
    let script = crate::resources::root().join("scripts/companion-power.py");
    ensure!(script.is_file(), "缺少 StepX伴侣电池优化适配组件");
    let mut command = crate::resources::python_command();
    command
        .arg(script)
        .arg("apply")
        .arg("--apk")
        .arg(apk)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE");
    let output = run_process(command, None, Duration::from_secs(120))?;
    let value =
        serde_json::from_slice::<Value>(&output.stdout).context("电池优化适配未返回有效状态")?;
    ensure!(
        output.status.success() && value["doze_exempt"] == true && value["package"] == PACKAGE,
        "StepX伴侣电池优化豁免未通过核验：{}",
        value["error"].as_str().unwrap_or("手机未确认豁免生效")
    );
    Ok(())
}

fn background_waiting_for_task(state: &Value, uptime_ms: Option<u64>) -> bool {
    state["enabled"].as_bool() == Some(true)
        && state["active"].as_bool() == Some(false)
        && state["lastError"].as_str() == Some("")
        && uptime_ms
            .zip(state["leaseExpiresElapsedMs"].as_u64())
            .is_some_and(|(now, expires)| expires <= now)
}

// A saved "active" flag is insufficient: require a live process and current lease.
fn background_status(
    state: &Value,
    alive: bool,
    uptime_ms: Option<u64>,
) -> (Option<bool>, Option<bool>, String) {
    let Some(enabled) = state["enabled"].as_bool() else {
        return (
            None,
            None,
            "尚无后台服务状态，无法确认熄屏任务能力。".into(),
        );
    };
    if !enabled {
        return (
            Some(false),
            Some(false),
            "后台连接已停止；发送新任务不会自动重新开启。".into(),
        );
    }
    if let Some(error) = state["lastError"].as_str().filter(|s| !s.is_empty()) {
        return (Some(true), Some(false), format!("后台连接未就绪：{error}"));
    }
    // An idle service intentionally exits after writing its final inactive
    // state. It need not keep a fresh heartbeat when there is no task lease.
    if background_waiting_for_task(state, uptime_ms) {
        return (
            Some(true),
            Some(false),
            "已配置，任务时连接。当前没有后台任务租约；发送任务后再确认实际连接和回复。".into(),
        );
    }
    let fresh = uptime_ms
        .zip(state["uptimeMs"].as_u64())
        .is_some_and(|(now, updated)| {
            updated <= now.saturating_add(2_000) && now.saturating_sub(updated) <= 30_000
        });
    let leased = uptime_ms
        .zip(state["leaseExpiresElapsedMs"].as_u64())
        .is_some_and(|(now, expires)| expires > now);
    let ready = alive
        && fresh
        && leased
        && state["active"].as_bool() == Some(true)
        && state["wakeLockHeld"].as_bool() == Some(true)
        && state["observedConnected"].as_bool() == Some(true)
        && state["observedConnectionSource"].as_str() == Some("agentbus_handshake")
        && uptime_ms
            .zip(state["observedConnectedElapsedMs"].as_u64())
            .is_some_and(|(now, observed)| {
                observed <= now.saturating_add(2_000) && now.saturating_sub(observed) <= 60_000
            });
    let message = if ready {
        "后台通道已确认可达，任务是否完成以实际回执为准。"
    } else if !alive || !fresh {
        "已开启，但服务状态未更新；请检测状态，当前不能确认熄屏任务能力。"
    } else if !leased {
        "已开启，当前无有效任务租约；任务连接时会续期，尚不能确认后台服务正在运行。"
    } else if let Some(message) = state["message"].as_str().filter(|s| !s.is_empty()) {
        message
    } else if state["wakeRequested"].as_bool() == Some(true) {
        "已请求手机原生短时连接，正在等待 Amoo 通道的实际连接回执。"
    } else {
        "已开启，正在等待手机确认后台服务与保持唤醒状态。"
    };
    (Some(true), Some(ready), message.into())
}

fn pairing_status(
    permission_granted: bool,
    state: &Value,
    alive: bool,
    phone_time_ms: Option<u64>,
) -> (bool, String) {
    if !permission_granted {
        return (
            false,
            "扫码所需权限尚未全部授权，请安装或修复伴侣权限。".into(),
        );
    }
    let fresh = phone_time_ms
        .zip(state["updatedAtMs"].as_u64())
        .is_some_and(|(now, updated)| {
            updated <= now.saturating_add(2_000) && now.saturating_sub(updated) <= 30_000
        });
    if !alive || !fresh {
        return (false, "扫码服务正在恢复。手机刚重启时，请先完成首次解锁并等待片刻；若仍未就绪，可在连接管理查看修复提示或使用系统配对码。".into());
    }
    if state["pairingBridgeReady"].as_bool() != Some(true) {
        return (false, "扫码权限已授权，但伴侣系统配对服务未连接。请保持已授权的 USB 连接，点击更新／修复伴侣启动服务；也可使用 Android 系统无线调试配对码。".into());
    }
    if state["pairingSupported"].as_bool() != Some(true) {
        return (
            false,
            "系统服务尚未确认支持伴侣扫码配对。当前请使用 Android 系统无线调试配对码。".into(),
        );
    }
    (true, "扫码服务已就绪。首次扫码需确认当前 Wi-Fi；已配对电脑在同一局域网自动连接。手机重启后先完成系统首次解锁，服务会自动恢复。".into())
}

fn read_pairing_readiness(adb: &Adb, device: &Device) -> (bool, String) {
    let cmd = format!(
        "cat {FILES}/pairing.json 2>/dev/null || true; \
         printf '\\n<<<STEPX_PAIRING_META>>>\\n'; \
         pidof {PACKAGE} 2>/dev/null || true; \
         printf '\\n<<<STEPX_PAIRING_TIME>>>\\n'; \
         date +%s 2>/dev/null || true"
    );
    let Ok(raw) = adb.script(&device.transport, &cmd) else {
        return pairing_status(true, &Value::Null, false, None);
    };
    let (pairing_raw, rest) = raw
        .split_once("\n<<<STEPX_PAIRING_META>>>\n")
        .unwrap_or(("", ""));
    let (pid_raw, time_raw) = rest
        .split_once("\n<<<STEPX_PAIRING_TIME>>>\n")
        .unwrap_or((rest, ""));
    let state = Some(pairing_raw.trim())
        .filter(|s| !s.is_empty() && s.len() <= 64 * 1024)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    let alive = !pid_raw.trim().is_empty();
    let phone_time = time_raw
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000));
    pairing_status(true, &state, alive, phone_time)
}

fn background_capability(
    state: &Value,
    alive: bool,
    uptime_ms: Option<u64>,
    capability: &Value,
) -> (Option<bool>, Option<bool>, String) {
    if capability["supported"].as_bool() == Some(false) {
        return (
            state["enabled"].as_bool(),
            Some(false),
            "当前固件不支持此熄屏适配。Amoo 会在熄屏后主动断开云连接，伴侣权限和电池豁免尚不能接管该连接；重新安装不能解决。请保持手机亮屏处理 Amoo / Amoo Work 任务，无需打开 Amoo 界面。".into(),
        );
    }
    if capability["supported"].as_bool() != Some(true) {
        return (
            state["enabled"].as_bool(),
            None,
            "尚未确认当前固件的熄屏服务能力，不能据已授权权限认定后台任务可用。".into(),
        );
    }
    if capability["installed"].as_bool() == Some(false) {
        return (
            state["enabled"].as_bool(),
            Some(false),
            capability["message"]
                .as_str()
                .unwrap_or("后台协作权限不完整，请安装／修复 StepX伴侣。")
                .into(),
        );
    }
    background_status(state, alive, uptime_ms)
}

fn read_status(adb: &Adb, device: &Device) -> Result<Value> {
    let text = adb.shell(&device.transport, &["cat", STATUS])?;
    ensure!(text.len() <= 64 * 1024, "StepX伴侣状态数据过大");
    serde_json::from_str(&text).context("尚未收到 StepX伴侣的有效状态")
}

pub fn inspect(adb: &Adb, device: &Device) -> Result<CompanionStatus> {
    device.verify(adb)?;
    let script = format!(
        "pkg=$(pm path {PACKAGE}); \
         printf '%s\\n' \"$pkg\"; \
         case \"$pkg\" in *package:*) ;; *) exit 0 ;; esac; \
         printf '\\n<<<STEPX_DUMP>>>\\n'; \
         dumpsys package {PACKAGE}; \
         printf '\\n<<<STEPX_PID>>>\\n'; \
         pidof {PACKAGE} 2>/dev/null || true; \
         printf '\\n<<<STEPX_STATUS>>>\\n'; \
         cat {STATUS} 2>/dev/null || true; \
         printf '\\n<<<STEPX_PAIRING>>>\\n'; \
         cat {FILES}/pairing.json 2>/dev/null || true; \
         printf '\\n<<<STEPX_TIME>>>\\n'; \
         date +%s 2>/dev/null || true; \
         printf '\\n<<<STEPX_UPTIME>>>\\n'; \
         cat /proc/uptime 2>/dev/null || true; \
         printf '\\n<<<STEPX_BG>>>\\n'; \
         cat {FILES}/amoo-connection.json 2>/dev/null || true; \
         printf '\\n<<<STEPX_AMOO_META>>>\\n'; \
         getprop ro.build.fingerprint; \
         getprop ro.build.version.sdk; \
         sha256sum /system/priv-app/AgentBus/AgentBus.apk 2>/dev/null || printf '\\n'; \
         printf '\\n<<<STEPX_WIFI>>>\\n'; \
         settings get global adb_wifi_enabled 2>/dev/null || true; \
         printf '\\n<<<STEPX_DOZE>>>\\n'; \
         cmd deviceidle whitelist 2>/dev/null || printf 'STEPX_DOZE_ERR\\n'; \
         printf '\\n<<<STEPX_OEM>>>\\n'; \
         dumpsys activity freeze whitelist 2>/dev/null || printf 'STEPX_OEM_ERR\\n'"
    );
    let raw = adb.script(&device.transport, &script)?;
    let (package, rest) = raw.split_once("\n<<<STEPX_DUMP>>>\n").unwrap_or((&raw, ""));
    if !package.lines().any(|x| x.starts_with("package:")) {
        return Ok(CompanionStatus {
            checked: true,
            message: "安装 StepX伴侣以使用背屏与文件收发；扫码和熄屏服务能力将单独检测。".into(),
            ..Default::default()
        });
    }
    let (dump, rest) = rest.split_once("\n<<<STEPX_PID>>>\n").unwrap_or((rest, ""));
    let (pid_raw, rest) = rest
        .split_once("\n<<<STEPX_STATUS>>>\n")
        .unwrap_or((rest, ""));
    let (status_raw, rest) = rest
        .split_once("\n<<<STEPX_PAIRING>>>\n")
        .unwrap_or((rest, ""));
    let (pairing_raw, rest) = rest
        .split_once("\n<<<STEPX_TIME>>>\n")
        .unwrap_or((rest, ""));
    let (time_raw, rest) = rest
        .split_once("\n<<<STEPX_UPTIME>>>\n")
        .unwrap_or((rest, ""));
    let (uptime_raw, rest) = rest.split_once("\n<<<STEPX_BG>>>\n").unwrap_or((rest, ""));
    let (bg_raw, rest) = rest
        .split_once("\n<<<STEPX_AMOO_META>>>\n")
        .unwrap_or((rest, ""));
    let (amoo_meta_raw, rest) = rest
        .split_once("\n<<<STEPX_WIFI>>>\n")
        .unwrap_or((rest, ""));
    let (wifi_raw, rest) = rest
        .split_once("\n<<<STEPX_DOZE>>>\n")
        .unwrap_or((rest, ""));
    let (doze_raw, oem_raw) = rest.split_once("\n<<<STEPX_OEM>>>\n").unwrap_or((rest, ""));

    let version = dump
        .lines()
        .find_map(|l| l.trim().strip_prefix("versionName="))
        .unwrap_or("未知")
        .to_string();
    let permission_granted = granted(dump, PERMISSION);
    let permissions = permission_status(dump);
    let pairing_permission_granted = granted(dump, "android.permission.MANAGE_DEBUGGING")
        && granted(dump, "android.permission.WRITE_SECURE_SETTINGS")
        && granted(dump, "android.permission.CAMERA");
    let alive = !pid_raw.trim().is_empty();
    let state = Some(status_raw.trim())
        .filter(|s| !s.is_empty() && s.len() <= 64 * 1024)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    let pairing = Some(pairing_raw.trim())
        .filter(|s| !s.is_empty() && s.len() <= 64 * 1024)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    let phone_time_ms = time_raw
        .trim()
        .parse::<u64>()
        .ok()
        .and_then(|seconds| seconds.checked_mul(1000));
    let (pairing_ready, pairing_message) =
        pairing_status(pairing_permission_granted, &pairing, alive, phone_time_ms);
    let uptime_secs = uptime_raw
        .split_whitespace()
        .next()
        .and_then(|s| s.parse::<f64>().ok());
    let uptime_ms = uptime_secs.map(|s| (s * 1000.) as u64);
    let background = Some(bg_raw.trim())
        .filter(|s| !s.is_empty() && s.len() <= 64 * 1024)
        .and_then(|s| serde_json::from_str::<Value>(s).ok())
        .unwrap_or(Value::Null);
    // This read-only capability gate does not start services. A saved active
    // flag cannot replace a fresh connection observation.
    let mut amoo_lines = amoo_meta_raw.lines();
    let amoo_fp = amoo_lines.next().unwrap_or("");
    let amoo_sdk = amoo_lines.next().unwrap_or("");
    let amoo_sha = amoo_lines.next().unwrap_or("");
    let capability = crate::amoo_background::prepare_from_parts(
        &device.transport,
        &device.serial,
        amoo_fp,
        amoo_sdk,
        amoo_sha,
        dump,
    )
    .unwrap_or(Value::Null);
    let background_supported = capability["supported"].as_bool();
    let (background_enabled, background_ready, background_message) =
        background_capability(&background, alive, uptime_ms, &capability);
    let wireless_enabled = match wifi_raw.trim() {
        "1" => Some(true),
        "0" => Some(false),
        _ => None,
    };
    let doze_state = doze_exempt(doze_raw);
    let doze_exempt = doze_state.as_ref().ok().copied();
    let oem_state = oem_exempt(oem_raw);
    let oem_freezer_exempt = oem_state.as_ref().ok().copied();
    let power_message = [
        doze_state
            .err()
            .map(|error| format!("Android 电池优化检测：{error}")),
        oem_state
            .err()
            .map(|error| format!("厂商冻结策略检测：{error}")),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join("；");
    let running = alive && state["running"].as_bool() == Some(true);
    let mode = if running {
        state["mode"].as_str().unwrap_or("").into()
    } else {
        String::new()
    };
    let applied_title = if running {
        state["appliedTitle"].as_str().unwrap_or("").into()
    } else {
        String::new()
    };
    let last_error = state["lastError"].as_str().unwrap_or("");
    let paused = state["pausedReason"].as_str().unwrap_or("");
    let stalled = running
        && uptime_secs
            .zip(state["uptimeMs"].as_u64())
            .is_some_and(|(uptime, updated)| uptime * 1000. - updated as f64 > 5_000.);
    let backscreen_message: String = if !permission_granted {
        "权限未授权，请安装／修复后重新检测。".into()
    } else if alive && !last_error.is_empty() {
        format!("运行异常：{last_error}")
    } else if stalled {
        "背屏状态已停止更新；若持续暂停，请点击更新／修复伴侣。".into()
    } else if running && !paused.is_empty() {
        format!("已保存手机配置，当前暂停：{paused}")
    } else if running {
        match mode.as_str() {
            "standby" => format!(
                "三敲已开启 · 上一局 {} 分 · 最高 {} 分。深度休眠时请先拿起手机。",
                state["lastGameScore"].as_u64().unwrap_or(0),
                state["bestScore"].as_u64().unwrap_or(0),
            ),
            "game" => format!(
                "第 {} 轮 · {} 格 · 累计 {} 分 · 最高 {} 分。左右倾斜移动，翻到正面结束。",
                state["round"].as_u64().unwrap_or(1),
                state["barWidth"].as_u64().unwrap_or(5),
                state["score"].as_u64().unwrap_or(0),
                state["bestScore"].as_u64().unwrap_or(0),
            ),
            _ => format!(
                "手机正在显示：{}",
                if applied_title.is_empty() {
                    "已应用效果"
                } else {
                    &applied_title
                }
            ),
        }
    } else {
        "权限已授权，当前未播放效果。".into()
    };
    let message = format!(
        "手机界面 APK 已安装。背屏：{backscreen_message} 扫码权限：{}；扫码服务：{}；熄屏任务：{}。",
        if pairing_permission_granted {
            "已授权"
        } else {
            "未就绪，请安装／修复"
        },
        if pairing_ready { "已核验" } else { "未通过核验，可用系统配对码连接" },
        match (background_supported, background_enabled, background_ready) {
            (Some(false), _, _) => "当前固件不支持此适配",
            (_, _, Some(true)) => "近期已确认后台通道可达",
            (_, Some(false), _) => "已停止",
            (Some(true), Some(true), _) if capability["installed"] == true
                && background_waiting_for_task(&background, uptime_ms) => "已配置，任务时连接",
            (_, _, Some(false)) => "尚未就绪",
            _ => "尚未检测到服务状态",
        }
    );
    Ok(CompanionStatus {
        checked: true,
        installed: true,
        version,
        message,
        permission_granted,
        running,
        mode,
        applied_title,
        permissions,
        pairing_permission_granted,
        pairing_ready,
        pairing_message,
        background_enabled,
        background_ready,
        background_supported,
        background_message,
        wireless_enabled,
        doze_exempt,
        oem_freezer_exempt,
        power_message,
    })
}

fn permission_script() -> Result<PathBuf> {
    let path = std::env::var_os("STEPX_COMPANION_PERMISSIONS")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::resources::root().join("scripts/companion-permissions.py"));
    ensure!(path.is_file(), "缺少 StepX伴侣权限适配组件");
    Ok(path)
}

// Keep a single immutable snapshot through preflight, install and permission setup.
// Installed resource bundles may carry a locally merged runtime; the public
// compile-time fallback intentionally contains no private Neo implementation.
fn compiled_apk_snapshot() -> Result<tempfile::NamedTempFile> {
    // Trust is anchored in this executable, never in the replaceable resource directory.
    let mut snapshot = tempfile::Builder::new().suffix(".apk").tempfile()?;
    snapshot.write_all(APK)?;
    snapshot.as_file().sync_all()?;
    Ok(snapshot)
}

fn installation_apk(resource_root: &Path) -> Result<tempfile::NamedTempFile> {
    let candidate = resource_root.join("desktop/assets/stepx-companion.apk");
    let mut snapshot = tempfile::Builder::new().suffix(".apk").tempfile()?;
    match std::fs::File::open(&candidate) {
        Ok(mut source) => {
            let size = source.metadata()?.len();
            ensure!(
                size > 0 && size <= 200 * 1024 * 1024,
                "伴侣资源 APK 大小异常，请使用完整安装包"
            );
            std::io::copy(&mut source, &mut snapshot).context("无法读取伴侣资源 APK")?;
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => snapshot.write_all(APK)?,
        Err(error) => return Err(error).context("无法读取伴侣资源 APK，请检查安装目录权限"),
    }
    snapshot.as_file().sync_all()?;
    Ok(snapshot)
}

pub fn install(
    adb: &Adb,
    device: &Device,
    mut progress: impl FnMut(&str),
) -> Result<CompanionStatus> {
    device.verify(adb)?;
    let script = permission_script()?;
    let apk = installation_apk(&crate::resources::root())?;
    let path = apk.path().to_str().context("安装包路径无效")?;
    progress("正在检查设备、应用签名与内置功能保留条件…");
    run_adapter(adb, device, &script, apk.path(), "preflight")?;
    // Only this package's USB-install gate is temporarily opened. Restore it on
    // every normal error path, including a partially completed prepare step.
    let installation = (|| -> Result<()> {
        run_adapter(adb, device, &script, apk.path(), "prepare-install")?;
        progress("正在安装 StepX伴侣…");
        let result = adb.run_timeout(
            &[
                "-s",
                &device.transport,
                "install",
                "-r",
                "--user",
                "0",
                path,
            ],
            Duration::from_secs(120),
        )?;
        ensure!(
            result.lines().any(|l| l.trim() == "Success"),
            "手机未确认 StepX伴侣安装成功：{}",
            result.trim()
        );
        Ok(())
    })();
    let restored = run_adapter(adb, device, &script, apk.path(), "finish-install");
    if let Err(error) = restored {
        bail!(
            "安装流程结束，但恢复 USB 安装许可失败：{error}；安装结果：{}",
            installation
                .as_ref()
                .err()
                .map(ToString::to_string)
                .unwrap_or_else(|| "APK 已安装".into())
        );
    }
    installation?;
    let mut warnings = Vec::new();
    progress("正在授予相机、服务通知与通话状态权限，并逐项回读…");
    grant_runtime_permissions(adb, device)?;
    // Runtime grants go first: PackageManager can recalculate nonstandard permission flags.
    progress("正在适配背屏、扫码连接权限，并核验应用签名…");
    for (operation, label) in [("grant", "背屏权限"), ("grant-connection", "无线配对权限")]
    {
        if let Err(error) = run_adapter(adb, device, &script, apk.path(), operation) {
            warnings.push(format!("{label}未完成：{error}"));
        }
    }
    progress("正在配置 StepX伴侣电池优化豁免并保存原状态…");
    if let Err(error) = apply_power_exemption(adb, device, apk.path()) {
        warnings.push(format!("后台电量设置未完成：{error}"));
    }
    progress("正在启用并验证伴侣看屏操作服务…");
    if let Err(error) = crate::companion_gui::enable(adb, device) {
        warnings.push(format!("看屏操作服务未完成：{error}"));
    }
    progress("正在回读权限、电池豁免与各项实际能力…");
    let checked = inspect(adb, device)?;
    ensure!(checked.installed, "手机未确认 StepX伴侣已安装");
    progress("正在安装并验证当前手机会话的系统扫码服务…");
    let service = start_pairing_service(adb, device, apk.path());
    // A CloudWake adapter failure must not prevent installing the independent scanner.
    progress("正在核验熄屏任务唤醒能力…");
    if let Err(error) = run_adapter(adb, device, &script, apk.path(), "grant-wake") {
        warnings.push(format!("熄屏任务唤醒权限未完成：{error}"));
    }
    progress("正在安装开机恢复并保存原启动文件与权限记录…");
    if let Err(error) = install_boot_adapter(adb, device, apk.path()) {
        warnings.push(format!("开机自动恢复未完成：{error}"));
    }
    let mut result = inspect(adb, device).unwrap_or(checked);
    finish_installation_status(&mut result, service);
    if let Err(error) = verify_installation(&result) {
        warnings.push(error.to_string());
    }
    if !warnings.is_empty() {
        result.message = format!(
            "伴侣已安装。扫码服务：{}。部分功能需要修复：{}",
            if result.pairing_ready {
                "已就绪"
            } else {
                "未就绪"
            },
            warnings.join("；")
        );
    }
    Ok(result)
}

fn finish_installation_status(result: &mut CompanionStatus, service: Result<()>) {
    if let Err(error) = service {
        result.pairing_ready = false;
        result.pairing_message = format!("扫码系统服务配置未完成：{error}。手机界面与权限已安装；请保持已授权的 USB 连接后更新／修复伴侣。当前也可使用系统配对码。");
        result.message = format!(
            "手机界面、权限和电池豁免配置完成，扫码服务部分失败。{} 熄屏任务：{}",
            result.pairing_message, result.background_message
        );
    } else if !result.pairing_ready {
        result.message = format!(
            "手机界面、权限和电池豁免配置完成。扫码系统服务已启动，但手机尚未确认就绪：{}",
            result.pairing_message
        );
    } else {
        result.message = format!("手机界面、权限和扫码服务配置完成。{}", result.message);
    }
}

fn run_pairing_adapter(adb: &Adb, device: &Device, apk: &Path, operation: &str) -> Result<Value> {
    ensure!(
        ["status", "start", "stop"].contains(&operation),
        "不支持的扫码服务操作"
    );
    let script = crate::resources::root().join("scripts/companion-system.py");
    ensure!(script.is_file(), "缺少 StepX伴侣扫码系统服务组件");
    let mut command = crate::resources::python_command();
    command
        .arg(script)
        .arg(operation)
        .arg("--apk")
        .arg(apk)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE");
    let output = run_process(command, None, Duration::from_secs(90))?;
    let result =
        serde_json::from_slice::<Value>(&output.stdout).context("扫码系统服务未返回有效状态")?;
    ensure!(
        output.status.success() && result["status"] != "failed",
        "扫码系统服务操作失败：{}",
        result["error"].as_str().unwrap_or("手机未确认服务状态")
    );
    ensure!(
        result["service"] == "stepx-pairing-v1"
            && result["package"] == PACKAGE
            && result["scope"] == "current_device_session"
            && result["reboot_verified"] == false
            && result["native_injection"] == false
            && result["google_launcher_changed"] == false,
        "扫码系统服务返回的身份或运行范围不符"
    );
    Ok(result)
}

fn configure_pairing_service(
    mut run: impl FnMut(&str) -> Result<Value>,
    confirm_phone: impl FnOnce() -> Result<()>,
) -> Result<()> {
    let previous = run("status")?;
    let running = previous["running"]
        .as_bool()
        .context("无法确认扫码服务是否运行")?;
    if running {
        let current = previous["artifact_current"]
            .as_bool()
            .context("无法确认扫码服务版本")?;
        if !current {
            let stopped = run("stop")?;
            ensure!(
                stopped["running"] == false
                    && ["stopped", "not_running"]
                        .contains(&stopped["status"].as_str().unwrap_or("")),
                "旧扫码服务尚未确认停止，未覆盖运行中的服务"
            );
        }
    }
    let started = run("start")?;
    ensure!(
        started["running"] == true
            && ["ready", "already_running"].contains(&started["status"].as_str().unwrap_or("")),
        "扫码系统服务未确认启动"
    );
    let verified = run("status")?;
    ensure!(
        verified["running"] == true && verified["artifact_current"] == true,
        "扫码系统服务运行记录或版本未通过回读"
    );
    // In particular, already_running alone says nothing about the app's Binder.
    confirm_phone()
}

fn install_boot_adapter(adb: &Adb, device: &Device, apk: &Path) -> Result<()> {
    let script = crate::resources::root().join("scripts/companion-boot.py");
    ensure!(script.is_file(), "缺少伴侣开机恢复组件");
    let mut command = crate::resources::python_command();
    command
        .arg(script)
        .arg("install")
        .arg("--apk")
        .arg(apk)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE");
    let output = run_process(command, None, Duration::from_secs(100))?;
    let result: Value = serde_json::from_slice(&output.stdout).context("开机恢复未返回有效回执")?;
    ensure!(
        output.status.success()
            && result["status"] == "installed"
            && result["config"]["package"] == PACKAGE
            && result["config"]["version"] == 1,
        "开机恢复未完成：{}",
        result["error"].as_str().unwrap_or("回读校验失败")
    );
    Ok(())
}

fn start_pairing_service(adb: &Adb, device: &Device, apk: &Path) -> Result<()> {
    // The boot adapter runs a scoped Binder in system_server, not a disposable shell process.
    // A fresh successful roundtrip to the signed app is sufficient; don't spawn a duplicate.
    if let Ok(raw) = adb.shell(
        &device.transport,
        &["cat", &format!("{FILES}/pairing.json")],
    ) {
        if let Ok(state) = serde_json::from_str::<Value>(&raw) {
            let system_pid = adb
                .shell(&device.transport, &["pidof", "system_server"])
                .ok()
                .and_then(|pid| pid.trim().parse::<u64>().ok());
            if state["pairingProcessUid"] == 1000
                && system_pid.is_some()
                && state["pairingProcessPid"].as_u64() == system_pid
                && read_pairing_readiness(adb, device).0
            {
                return Ok(());
            }
        }
    }

    configure_pairing_service(
        |operation| run_pairing_adapter(adb, device, apk, operation),
        || {
            let deadline = Instant::now() + Duration::from_secs(12);
            loop {
                let (ready, message) = read_pairing_readiness(adb, device);
                if ready {
                    return Ok(());
                }
                if Instant::now() >= deadline {
                    bail!("{message}");
                }
                thread::sleep(Duration::from_millis(500));
            }
        },
    )
}

fn verify_installation(result: &CompanionStatus) -> Result<()> {
    ensure!(result.installed, "手机未确认 StepX伴侣已安装");
    ensure!(
        result.permission_granted,
        "StepX伴侣已安装，但手机未确认背屏权限"
    );
    ensure!(
        result.pairing_permission_granted,
        "StepX伴侣已安装，但手机未确认扫码连接权限，请重新检测或修复"
    );
    let missing: Vec<_> = result
        .permissions
        .iter()
        .filter(|permission| !permission.granted)
        .map(|permission| permission.label)
        .collect();
    ensure!(
        !result.permissions.is_empty() && missing.is_empty(),
        "StepX伴侣安装授权未完成：{}",
        missing.join("、")
    );
    ensure!(
        result.doze_exempt == Some(true),
        "StepX伴侣已安装，但 Android 电池优化豁免未生效"
    );
    ensure!(
        result.oem_freezer_exempt == Some(true),
        "StepX伴侣已安装，但厂商后台冻结豁免未生效"
    );
    Ok(())
}

fn run_adapter(
    adb: &Adb,
    device: &Device,
    script: &Path,
    apk: &Path,
    operation: &str,
) -> Result<()> {
    let trusted = if matches!(operation, "preflight" | "prepare-install") {
        Some(compiled_apk_snapshot()?)
    } else {
        None
    };
    let mut command = crate::resources::python_command();
    command
        .arg(script)
        .arg(operation)
        .arg("--apk")
        .arg(apk)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE");
    if let Some(anchor) = &trusted {
        command.arg("--trusted-apk").arg(anchor.path());
    }
    let output = run_process(command, None, Duration::from_secs(180))?;
    if !output.status.success() {
        let reason = serde_json::from_slice::<Value>(&output.stdout)
            .ok()
            .and_then(|v| v["error"].as_str().map(String::from))
            .unwrap_or_else(|| {
                String::from_utf8_lossy(&output.stderr)
                    .lines()
                    .last()
                    .unwrap_or("请检查 StepClaw 是否可用")
                    .to_string()
            });
        bail!("StepX伴侣权限适配未完成：{reason}");
    }
    Ok(())
}

fn command_ack(adb: &Adb, device: &Device, op: &str, extras: &[&str]) -> Result<Value> {
    device.verify(adb)?;
    let id = format!(
        "desktop-{}-{}",
        chrono::Utc::now().timestamp_millis(),
        REQUEST.fetch_add(1, Ordering::Relaxed)
    );
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
    args.extend_from_slice(extras);
    adb.shell(&device.transport, &args)?;
    let deadline = Instant::now() + Duration::from_secs(12);
    while Instant::now() < deadline {
        if let Ok(state) = read_status(adb, device) {
            if state["requestId"].as_str() == Some(&id) {
                if let Some(error) = state["lastError"].as_str().filter(|e| !e.is_empty()) {
                    bail!("StepX伴侣：{error}");
                }
                if state["commandReady"].as_bool() == Some(true) {
                    return Ok(state);
                }
            }
        }
        thread::sleep(Duration::from_millis(200));
    }
    bail!("尚未收到 StepX伴侣的执行确认，请检查手机通知和背屏权限后重试")
}

fn require_ready(adb: &Adb, device: &Device) -> Result<()> {
    let state = inspect(adb, device)?;
    ensure!(state.installed, "请先安装 StepX伴侣");
    ensure!(
        state.permission_granted,
        "StepX伴侣背屏权限尚未就绪，请点击安装／修复"
    );
    // This firmware freezes even a valid foreground service unless its package
    // is explicitly allowed. Recheck on each user action because that list lives
    // in system_server memory and may have reset after a phone restart.
    let apk = installation_apk(&crate::resources::root())?;
    run_adapter(adb, device, &permission_script()?, apk.path(), "runtime")?;
    Ok(())
}

pub fn arm_game(adb: &Adb, device: &Device) -> Result<String> {
    require_ready(adb, device)?;
    let reply = command_ack(adb, device, "standby", &[])?;
    ensure!(
        reply["running"].as_bool() == Some(true),
        "手机未确认启动三敲监听"
    );
    Ok("已启用三敲游戏。拿起手机，背屏朝向自己，连续轻敲三次开始；深度休眠时需先唤醒手机。".into())
}

pub fn start_game(adb: &Adb, device: &Device) -> Result<String> {
    require_ready(adb, device)?;
    let reply = command_ack(adb, device, "game", &[])?;
    ensure!(
        reply["running"].as_bool() == Some(true),
        "手机未确认游戏启动"
    );
    let output = confirm_output(adb, device, &reply)?;
    Ok(format!(
        "游戏已在手机启动，{output}。保持握持姿势完成校准，再左右倾斜对齐长条。"
    ))
}

pub fn stop(adb: &Adb, device: &Device) -> Result<String> {
    let reply = command_ack(adb, device, "stop", &[])?;
    ensure!(
        reply["running"].as_bool() != Some(true),
        "手机仍报告背屏服务运行中"
    );
    ensure!(
        reply["settingsRestored"].as_bool() == Some(true),
        "手机尚未确认恢复原背屏设置"
    );
    Ok("StepX伴侣已停止，原背屏设置已恢复。".into())
}

fn validate_frames(frames: &[Vec<u8>], fps: u8, brightness: u8) -> Result<()> {
    ensure!((1..=120).contains(&frames.len()), "背屏动画需 1–120 帧");
    ensure!(
        (1..=12).contains(&fps) && (1..=8).contains(&brightness),
        "帧率需1–12，亮度需1–8"
    );
    for f in frames {
        ensure!(f.len() == 391, "背屏帧必须包含391个位置");
        ensure!(
            !f.iter()
                .enumerate()
                .any(|(i, v)| *v != 0 && is_backscreen_corner(i % 17, i / 17)),
            "不存在的灯珠必须为0"
        );
    }
    Ok(())
}

fn apply_config(adb: &Adb, device: &Device, config: Value) -> Result<String> {
    require_ready(adb, device)?;
    command_ack(adb, device, "status", &[])?;
    let data = serde_json::to_vec(&config)?;
    ensure!(data.len() <= 256 * 1024, "动画配置超过256KiB");
    let mut file = tempfile::NamedTempFile::new()?;
    file.write_all(&data)?;
    file.as_file().sync_all()?;
    let part = format!("{FILES}/incoming.json.part");
    let remote = format!("{FILES}/incoming.json");
    adb.transfer(&device.transport, "push", file.path(), Path::new(&part))?;
    adb.shell(&device.transport, &["mv", &part, &remote])?;
    let reply = command_ack(
        adb,
        device,
        "apply",
        &["--es", "config_name", "incoming.json"],
    )?;
    ensure!(
        reply["running"].as_bool() == Some(true),
        "手机未确认应用效果"
    );
    let output = confirm_output(adb, device, &reply)?;
    Ok(format!(
        "已应用：{}，{output}。效果在手机本地运行。",
        config["title"].as_str().unwrap_or("背屏效果"),
    ))
}

/// Keep a configuration acknowledgement distinct from verified physical output.
fn confirm_output(adb: &Adb, device: &Device, acknowledged: &Value) -> Result<String> {
    let request = acknowledged["requestId"]
        .as_str()
        .context("手机未返回请求标识")?;
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        thread::sleep(Duration::from_millis(250));
        let state = read_status(adb, device)?;
        ensure!(
            state["requestId"].as_str() == Some(request),
            "手机端操作已改变，请刷新状态"
        );
        if let Some(error) = state["lastError"].as_str().filter(|s| !s.is_empty()) {
            bail!("手机背屏执行失败：{error}");
        }
        if let Some(reason) = state["pausedReason"]
            .as_str()
            .filter(|s| !s.is_empty() && *s != "等待姿态传感器")
        {
            return Ok(format!("当前暂停：{reason}"));
        }
        if state["outputAccepted"].as_bool() == Some(true) {
            let dump = adb.shell(&device.transport, &["dumpsys", "display_ext"])?;
            let owner = state["uid"]
                .as_u64()
                .map(|uid| format!("appFrameOwner=uid={uid}"));
            let owned = owner.is_some_and(|prefix| dump.lines().any(|line| line.trim() == prefix));
            if owned
                && dump.contains("appUsesCustomFrame=true")
                && dump.contains("appliedDriverFlat=391")
                && dump.lines().any(|l| l.trim() == "power=on")
                && dump.contains("policyAllowsOutput=true")
            {
                return Ok("已确认背屏输出".into());
            }
            if !dump.contains("policyAllowsOutput=true") {
                return Ok("手机系统暂未允许亮屏，翻起背屏后显示".into());
            }
        }
        if Instant::now() >= deadline {
            return Ok("手机已保存配置，灯珠输出尚待确认".into());
        }
    }
}

pub fn apply_animation(
    adb: &Adb,
    device: &Device,
    frames: &[Vec<u8>],
    fps: u8,
    brightness: u8,
    title: &str,
) -> Result<String> {
    validate_frames(frames, fps, brightness)?;
    ensure!(title.chars().count() <= 80, "效果名称过长");
    apply_config(
        adb,
        device,
        json!({"kind":"frames","title":title,"frames":frames,"fps":fps,"brightness":brightness,"durationSecs":0}),
    )
}

pub fn apply_preset(
    adb: &Adb,
    device: &Device,
    key: &str,
    brightness: u8,
    duration_secs: u32,
) -> Result<String> {
    ensure!(
        ["clock", "battery", "countdown"].contains(&key),
        "不支持的手机动态效果"
    );
    ensure!(
        (1..=8).contains(&brightness) && (1..=3600).contains(&duration_secs),
        "亮度或时长不合法"
    );
    let title = match key {
        "clock" => "像素时钟",
        "battery" => "手机电量",
        _ => "专注倒计时",
    };
    apply_config(
        adb,
        device,
        json!({"kind":"preset","key":key,"title":title,"brightness":brightness,"fps":1,"durationSecs":if key=="countdown" {duration_secs} else {0}}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installation_snapshot_prefers_resource_package_and_is_immutable() {
        let root = tempfile::tempdir().unwrap();
        let assets = root.path().join("desktop/assets");
        std::fs::create_dir_all(&assets).unwrap();
        let candidate = assets.join("stepx-companion.apk");
        std::fs::write(&candidate, b"local-complete-apk").unwrap();
        let snapshot = installation_apk(root.path()).unwrap();
        std::fs::write(&candidate, b"changed-after-snapshot").unwrap();
        assert_eq!(
            std::fs::read(snapshot.path()).unwrap(),
            b"local-complete-apk"
        );
    }

    #[test]
    fn compiled_signature_anchor_does_not_read_replaceable_resources() {
        let root = tempfile::tempdir().unwrap();
        let candidate = root.path().join("desktop/assets/stepx-companion.apk");
        std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
        std::fs::write(&candidate, b"replacement-with-another-certificate").unwrap();
        let selected = installation_apk(root.path()).unwrap();
        let anchor = compiled_apk_snapshot().unwrap();
        assert_ne!(std::fs::read(selected.path()).unwrap(), APK);
        assert_eq!(std::fs::read(anchor.path()).unwrap(), APK);
    }

    #[test]
    fn installation_snapshot_falls_back_only_when_resource_is_missing() {
        let root = tempfile::tempdir().unwrap();
        let snapshot = installation_apk(root.path()).unwrap();
        assert_eq!(std::fs::read(snapshot.path()).unwrap(), APK);
        let candidate = root.path().join("desktop/assets/stepx-companion.apk");
        std::fs::create_dir_all(candidate.parent().unwrap()).unwrap();
        std::fs::write(&candidate, b"").unwrap();
        assert!(installation_apk(root.path()).is_err());
    }

    #[test]
    fn permission_status_requires_exact_granted_permission() {
        let dump = "android.permission.CAMERA
android.permission.MANAGE_DEBUGGING: granted=false
android.permission.WRITE_SECURE_SETTINGS: granted=true, flags=[ USER_SET ]
android.permission.CAMERA_EXTRA: granted=true";
        assert!(!granted(dump, "android.permission.CAMERA"));
        assert!(!granted(dump, "android.permission.MANAGE_DEBUGGING"));
        assert!(granted(dump, "android.permission.WRITE_SECURE_SETTINGS"));
        let permissions = permission_status(dump);
        assert_eq!(permissions.len(), 15);
        assert!(
            !permissions
                .iter()
                .find(|p| p.name == "com.stepos.cloud.push.permission.MESSAGE")
                .unwrap()
                .granted
        );
    }
    #[test]
    fn runtime_grants_are_scoped_to_user_zero() {
        let permission = "android.permission.CAMERA";
        let other_user = "    User 0: installed=true
      runtime permissions:
        android.permission.CAMERA: granted=false
    User 10: installed=true
      runtime permissions:
        android.permission.CAMERA: granted=true, flags=[ USER_SET ]";
        assert!(!granted(other_user, permission));
        assert!(granted(
            &other_user.replace("CAMERA: granted=false", "CAMERA: granted=true"),
            permission
        ));
        assert!(!granted(
            "android.permission.CAMERA: granted=true",
            permission
        ));
        assert!(!granted(
            "    User 0: installed=true
      android.permission.CAMERA: granted=truefalse",
            permission
        ));
    }
    #[test]
    fn android_doze_and_oem_freezer_are_distinct_checks() {
        assert!(!doze_exempt(&format!("system-excidle,{PACKAGE},10215")).unwrap());
        assert!(doze_exempt(&format!("user,{PACKAGE},10215")).unwrap());
        assert!(doze_exempt("permission denied").is_err());
        let oem = format!(
            "cloud freeze white apps:
freeze white app: {PACKAGE}
user freeze white apps:
local freeze white apps:
"
        );
        assert!(oem_exempt(&oem).unwrap());
        assert!(oem_exempt(
            "user freeze white apps:
"
        )
        .is_err());
        assert!(!oem_exempt(&oem.replace(PACKAGE, "another.package")).unwrap());
    }
    #[test]
    fn install_accepts_partial_service_capability_but_requires_real_permission_readback() {
        let mut status = CompanionStatus {
            installed: true,
            permission_granted: true,
            pairing_permission_granted: true,
            pairing_ready: false,
            background_supported: Some(false),
            background_ready: Some(false),
            doze_exempt: Some(true),
            oem_freezer_exempt: Some(true),
            permissions: permission_status(""),
            ..Default::default()
        };
        for permission in &mut status.permissions {
            permission.granted = true;
        }
        assert!(verify_installation(&status).is_ok());
        let wake_index = status
            .permissions
            .iter()
            .position(|p| p.name == "com.stepos.cloud.push.permission.MESSAGE")
            .unwrap();
        status.permissions[wake_index].granted = false;
        assert!(verify_installation(&status)
            .unwrap_err()
            .to_string()
            .contains("原生任务唤醒"));
        status.permissions[wake_index].granted = true;
        status.permissions[3].granted = false;
        assert!(verify_installation(&status).is_err());
        status.permissions[3].granted = true;
        status.doze_exempt = None;
        assert!(verify_installation(&status).is_err());
        status.doze_exempt = Some(true);
        status.oem_freezer_exempt = Some(false);
        assert!(verify_installation(&status).is_err());
    }
    #[test]
    fn pairing_permissions_do_not_imply_service_support_or_freshness() {
        let active =
            json!({"pairingBridgeReady":true,"pairingSupported":true,"updatedAtMs":100000});
        assert!(pairing_status(true, &active, true, Some(101000)).0);
        assert!(!pairing_status(false, &active, true, Some(101000)).0);
        assert!(!pairing_status(true, &active, false, Some(101000)).0);
        assert!(!pairing_status(true, &active, true, Some(140000)).0);
        assert!(!pairing_status(true, &active, true, Some(100)).0);
        assert!(!pairing_status(true, &active, true, None).0);
        assert!(!pairing_status(true, &Value::Null, true, Some(101000)).0);
        let mut unsupported = active.clone();
        unsupported["pairingSupported"] = json!(false);
        assert!(!pairing_status(true, &unsupported, true, Some(101000)).0);
        unsupported["pairingSupported"] = json!(true);
        unsupported["pairingBridgeReady"] = json!(false);
        assert!(!pairing_status(true, &unsupported, true, Some(101000)).0);
    }
    #[test]
    fn pairing_service_replaces_only_outdated_record_and_checks_phone_even_if_running() {
        for outdated in [false, true] {
            let mut operations = Vec::new();
            let mut confirmed = false;
            let result = configure_pairing_service(
                |op| {
                    operations.push(op.to_string());
                    Ok(match op {
                        "status" => {
                            json!({"running":true,"artifact_current":!outdated || operations.len()>1})
                        }
                        "stop" => json!({"status":"stopped","running":false}),
                        "start" => json!({"status":"already_running","running":true}),
                        _ => unreachable!(),
                    })
                },
                || {
                    confirmed = true;
                    Ok(())
                },
            );
            assert!(result.is_ok());
            assert!(confirmed);
            assert_eq!(
                operations,
                if outdated {
                    vec!["status", "stop", "start", "status"]
                } else {
                    vec!["status", "start", "status"]
                }
            );
        }
        let result = configure_pairing_service(
            |op| {
                Ok(if op == "start" {
                    json!({"status":"already_running","running":true})
                } else {
                    json!({"running":true,"artifact_current":true})
                })
            },
            || bail!("phone Binder has no fresh acknowledgement"),
        );
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("fresh acknowledgement"));
    }
    #[test]
    fn pairing_service_failed_stop_never_launches_replacement() {
        let mut operations = Vec::new();
        let result = configure_pairing_service(
            |op| {
                operations.push(op.to_string());
                Ok(if op == "status" {
                    json!({"running":true,"artifact_current":false})
                } else {
                    json!({"status":"failed","running":true})
                })
            },
            || panic!("must not confirm an unstarted replacement"),
        );
        assert!(result.is_err());
        assert_eq!(operations, vec!["status", "stop"]);
    }
    #[test]
    fn pairing_service_partial_failure_cannot_report_old_ready_status() {
        let mut status = CompanionStatus {
            installed: true,
            pairing_ready: true,
            background_message: "尚未确认任务连接".into(),
            ..Default::default()
        };
        finish_installation_status(&mut status, Err(anyhow::anyhow!("PID changed")));
        assert!(status.installed);
        assert!(!status.pairing_ready);
        assert!(status.message.contains("部分失败"));
        assert!(status.pairing_message.contains("PID changed"));
    }
    #[test]
    fn unsupported_background_capability_overrides_saved_active_state() {
        let active = json!({"enabled":true,"active":true,"bridgeReady":true,"wakeLockHeld":true,"uptimeMs":100000,"leaseExpiresElapsedMs":160000,"bridge":{"operator":"ONLINE"}});
        let unsupported = json!({"supported":false,"status":"unsupported"});
        let result = background_capability(&active, true, Some(101000), &unsupported);
        assert_eq!(result.1, Some(false));
        assert!(result.2.contains("当前固件不支持"));
        assert!(!result.2.contains("修复"));
        assert_eq!(
            background_capability(&active, true, Some(101000), &Value::Null).1,
            None
        );
        let missing = json!({"supported":true,"installed":false,"message":"后台协作权限不完整"});
        let incomplete = background_capability(&active, true, Some(101000), &missing);
        assert_eq!(incomplete.1, Some(false));
        assert!(incomplete.2.contains("权限不完整"));
    }
    #[test]
    fn background_state_never_trusts_stale_or_stopped_service() {
        let active = json!({"enabled":true,"active":true,"wakeLockHeld":true,"uptimeMs":100000,"leaseExpiresElapsedMs":160000,"observedConnected":true,"observedConnectedElapsedMs":100000,"observedConnectionSource":"agentbus_handshake"});
        assert_eq!(background_status(&active, true, Some(101000)).1, Some(true));
        assert_eq!(
            background_status(&active, true, Some(140000)).1,
            Some(false)
        );
        assert_eq!(
            background_status(&active, false, Some(101000)).1,
            Some(false)
        );
        assert_eq!(background_status(&active, true, None).1, Some(false));
        assert_eq!(background_status(&active, true, Some(100)).1, Some(false));
        let mut disabled = active.clone();
        disabled["enabled"] = json!(false);
        assert_eq!(
            background_status(&disabled, true, Some(101000)).0,
            Some(false)
        );
        assert_eq!(
            background_status(&disabled, true, Some(101000)).1,
            Some(false)
        );
        let mut expired = active.clone();
        expired["leaseExpiresElapsedMs"] = json!(100500);
        assert_eq!(
            background_status(&expired, true, Some(101000)).1,
            Some(false)
        );
        let mut denied = active.clone();
        denied["observedConnected"] = json!(false);
        denied["wakeRequested"] = json!(true);
        denied["bridge"] = json!({"operator":"ONLINE"});
        assert_eq!(
            background_status(&denied, true, Some(101000)).1,
            Some(false)
        );
        assert!(background_status(&denied, true, Some(101000))
            .2
            .contains("等待"));
        denied = active.clone();
        denied["observedConnectedElapsedMs"] = json!(1000);
        assert_eq!(
            background_status(&denied, true, Some(101000)).1,
            Some(false)
        );
        denied = active.clone();
        denied["observedConnectionSource"] = json!("wakeRequested");
        assert_eq!(
            background_status(&denied, true, Some(101000)).1,
            Some(false)
        );
        assert_eq!(background_status(&Value::Null, true, Some(101000)).1, None);
    }
    #[test]
    fn idle_background_does_not_require_a_running_service_or_claim_online() {
        let idle = json!({"enabled":true,"active":false,"lastError":"","uptimeMs":1000,"leaseExpiresElapsedMs":0});
        let status = background_status(&idle, false, Some(100000));
        assert_eq!(status.0, Some(true));
        assert_eq!(status.1, Some(false));
        assert!(status.2.starts_with("已配置，任务时连接"));
        let mut failure = idle.clone();
        failure["lastError"] = json!("原生权限已撤销");
        assert!(!background_waiting_for_task(&failure, Some(100000)));
        assert!(background_status(&failure, false, Some(100000))
            .2
            .contains("权限已撤销"));
        let mut stale_active = idle.clone();
        stale_active["active"] = json!(true);
        stale_active["leaseExpiresElapsedMs"] = json!(150000);
        assert!(!background_waiting_for_task(&stale_active, Some(100000)));
        assert!(background_status(&stale_active, false, Some(100000))
            .2
            .contains("服务状态未更新"));
        assert!(!background_waiting_for_task(&idle, None));
    }
    #[test]
    fn physical_matrix_validation_rejects_absent_leds_and_unbounded_animation() {
        let mut frame = vec![0; 391];
        frame[3] = 200;
        assert!(validate_frames(&[frame.clone()], 12, 8).is_ok());
        frame[0] = 1;
        assert!(validate_frames(&[frame], 12, 8).is_err());
        assert!(validate_frames(&[vec![0; 390]], 6, 5).is_err());
        assert!(validate_frames(&vec![vec![0; 391]; 121], 6, 5).is_err());
        assert!(validate_frames(&[vec![0; 391]], 13, 5).is_err());
    }
}
