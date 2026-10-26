//! Desktop entry points for the fixed-firmware Google workflow. Inspection is
//! read-only; compatibility changes require an explicit installation or action.
use crate::{
    adb::Adb,
    device::{
        Device, GoogleSuiteStatus, PlayProtectBlocker, PlayProtectCheck, PlayProtectFirmware,
        PlayProtectLayer, PlayProtectLeverage, PlayProtectPatchFreshness, PlayProtectReport,
        PlayProtectVendorServices,
    },
};
use anyhow::{bail, Context, Result};
use std::path::PathBuf;

const PLAY_PROTECT_SCHEMA: &str = "stepx-play-protect-certification-v1";

pub fn workflow_root() -> Result<PathBuf> {
    let root = std::env::var_os("STEPX_GOOGLE_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| crate::resources::root().join("handoff/google-play-stepx"));
    if !root.join("tools/manage.py").is_file() {
        bail!("缺少 Google 安装组件，请配置 STEPX_GOOGLE_HOME 后重试");
    }
    Ok(root)
}

pub fn run_script(device: &Device, adb: &Adb, script: &str, args: &[&str]) -> Result<String> {
    let root = workflow_root()?;
    let output = crate::resources::python_command()
        .arg(root.join("tools").join(script))
        .args(args)
        .current_dir(&root)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE")
        .output()
        .context("无法启动 Google 安装组件，请检查 Python 3 环境")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        let reason = stderr
            .lines()
            .rev()
            .find(|s| !s.trim().is_empty())
            .unwrap_or("组件未完成操作");
        bail!("{} {} 失败：{}", script, args.join(" "), reason);
    }
    Ok(String::from_utf8(output.stdout)?.trim().into())
}

fn run_compatibility_script(device: &Device, adb: &Adb, action: &str) -> Result<serde_json::Value> {
    if !matches!(action, "status" | "apply" | "rollback") {
        bail!("不支持的 Google 兼容补丁操作");
    }
    let script = crate::resources::root().join("scripts/google-play-compat.py");
    if !script.is_file() {
        bail!("当前桌面安装缺少 Google Play 兼容组件，请更新 desktop 后重试");
    }
    let output = crate::resources::python_command()
        .arg(&script)
        .arg(action)
        .current_dir(script.parent().context("兼容组件目录无效")?)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE")
        .output()
        .context("无法启动 Google Play 兼容组件，请检查 Python 3 环境")?;
    // Only the component's bounded JSON summary may reach the UI. Worker
    // stderr and private rollback receipts are never displayed here.
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("Google Play 兼容组件未返回有效状态，请重新检测")?;
    if value["ok"].as_bool().is_none()
        || value["supported"].as_bool().is_none()
        || value["active"].as_bool().is_none()
        || value["can_rollback"].as_bool().is_none()
        || value["message"].as_str().is_none()
    {
        bail!("Google Play 兼容组件返回的状态不完整");
    }
    if !output.status.success() && value["ok"] == true {
        bail!("Google Play 兼容组件异常退出，请重新检测");
    }
    Ok(value)
}

/// The read-only certification model script sits next to the compatibility
/// component so a packaged desktop carries both.
fn play_protect_script() -> Result<PathBuf> {
    let script = crate::resources::root().join("scripts/play_protect_certification.py");
    if !script.is_file() {
        bail!("当前桌面安装缺少 Play 保护机制认证组件，请更新 desktop 后重试");
    }
    Ok(script)
}

/// Attestation evidence captured earlier. Generating a new attestation key
/// needs a diagnostic app on the phone, so only an existing capture is read.
fn captured_attestation_chain() -> Option<PathBuf> {
    let directory = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../backups/google-play-compat-20261023/key-attestation-validation");
    let complete = ["leaf.pem", "intermediates.pem", "chain-provided-root.pem"]
        .iter()
        .all(|name| directory.join(name).is_file());
    complete.then_some(directory)
}

fn play_protect_text(value: &serde_json::Value, key: &str) -> Result<String> {
    value
        .get(key)
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .with_context(|| format!("Play 保护机制认证结果缺少 {key}"))
}

/// Optional text: a missing key stays absent instead of becoming an empty string.
fn optional_text(value: &serde_json::Value, key: &str) -> Option<String> {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(text)) => Some(text.clone()),
        Some(other) => Some(other.to_string()),
    }
}

/// Optional integer; a missing key stays absent.
fn optional_number(value: &serde_json::Value, key: &str) -> Option<i64> {
    value.get(key).and_then(serde_json::Value::as_i64)
}

/// Optional booleans must survive as `None`; a missing key is not a failure.
fn tri_state_flag(value: &serde_json::Value, key: &str) -> Option<bool> {
    match value.get(key) {
        None | Some(serde_json::Value::Null) => None,
        Some(flag) => flag.as_bool(),
    }
}

/// `value` and `expected` carry numbers and booleans as well as strings.
fn play_protect_scalar(value: &serde_json::Value, key: &str) -> Result<String> {
    let field = value
        .get(key)
        .with_context(|| format!("Play 保护机制认证结果缺少 {key}"))?;
    match field {
        serde_json::Value::String(text) => Ok(text.clone()),
        serde_json::Value::Bool(flag) => Ok(flag.to_string()),
        serde_json::Value::Number(number) => Ok(number.to_string()),
        serde_json::Value::Null => Ok("空".into()),
        _ => bail!("Play 保护机制认证字段 {key} 类型无效"),
    }
}

/// Only the script's bounded JSON summary may reach the UI. Worker stderr and
/// private rollback receipts are never displayed here.
pub fn play_protect_from_value(value: &serde_json::Value) -> Result<PlayProtectReport> {
    if value.get("schema").and_then(serde_json::Value::as_str) != Some(PLAY_PROTECT_SCHEMA) {
        bail!("Play 保护机制认证组件版本不匹配");
    }
    let layers = value
        .get("layers")
        .and_then(serde_json::Value::as_array)
        .context("Play 保护机制认证结果缺少分层证据")?;
    let mut parsed = Vec::with_capacity(layers.len());
    for layer in layers {
        let checks = layer
            .get("checks")
            .and_then(serde_json::Value::as_array)
            .context("Play 保护机制认证分层缺少检查项")?;
        let mut entries = Vec::with_capacity(checks.len());
        for entry in checks {
            entries.push(PlayProtectCheck {
                check: play_protect_text(entry, "check")?,
                value: play_protect_scalar(entry, "value")?,
                expected: play_protect_scalar(entry, "expected")?,
                // A missing `ok` means the probe did not run; it is never a pass.
                ok: entry.get("ok").and_then(serde_json::Value::as_bool),
                detail: entry
                    .get("detail")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string),
            });
        }
        parsed.push(PlayProtectLayer {
            layer: play_protect_text(layer, "layer")?,
            title: play_protect_text(layer, "title")?,
            owner: play_protect_text(layer, "owner")?,
            state: play_protect_text(layer, "state")?,
            checks: entries,
            note: play_protect_text(layer, "note")?,
        });
    }
    let vendor_services = value.get("vendor_services").and_then(|item| {
        if item.is_null() {
            return None;
        }
        Some(PlayProtectVendorServices {
            updater_installed: tri_state_flag(item, "updater_installed"),
            updater_holds_dsid_permission: tri_state_flag(item, "updater_holds_dsid_permission"),
            dsid_readable_from_host: tri_state_flag(item, "dsid_readable_from_host"),
            dsid_host_error: play_protect_text(item, "dsid_host_error").unwrap_or_default(),
            gsf_android_id: play_protect_text(item, "gsf_android_id").unwrap_or_default(),
            note: play_protect_text(item, "note").unwrap_or_default(),
        })
    });
    let leverage = value
        .get("leverage")
        .and_then(serde_json::Value::as_array)
        .context("Play 保护机制认证结果缺少厂商动作排序")?
        .iter()
        .map(|item| {
            Ok(PlayProtectLeverage {
                action: play_protect_text(item, "action")?,
                clears: item
                    .get("clears")
                    .and_then(serde_json::Value::as_array)
                    .context("Play 保护机制认证动作缺少可解除的层")?
                    .iter()
                    .map(|layer| {
                        layer
                            .as_str()
                            .map(str::to_string)
                            .context("Play 保护机制认证动作的层列表无效")
                    })
                    .collect::<Result<Vec<_>>>()?,
                owner: play_protect_text(item, "owner")?,
                reason: play_protect_text(item, "reason")?,
            })
        })
        .collect::<Result<Vec<_>>>()?;
    let patch_freshness = value.get("patch_freshness").and_then(|item| {
        if item.is_null() {
            return None;
        }
        Some(PlayProtectPatchFreshness {
            security_patch: optional_text(item, "security_patch"),
            vendor_security_patch: optional_text(item, "vendor_security_patch"),
            age_days: optional_number(item, "age_days"),
            stale: tri_state_flag(item, "stale"),
            note: play_protect_text(item, "note").unwrap_or_default(),
        })
    });
    let firmware = value.get("firmware").and_then(|item| {
        if item.is_null() {
            return None;
        }
        Some(PlayProtectFirmware {
            available: item
                .get("available")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            http_code: optional_number(item, "http_code"),
            new_version: optional_text(item, "new_version"),
            latest_version: optional_text(item, "latest_version"),
            current_build: optional_text(item, "current_build"),
            current_incremental: optional_text(item, "current_incremental"),
            same_as_current: item
                .get("same_as_current")
                .and_then(serde_json::Value::as_bool)
                .unwrap_or(false),
            need_update: tri_state_flag(item, "need_update"),
            release_date: optional_text(item, "release_date"),
            note: play_protect_text(item, "note").unwrap_or_default(),
        })
    });
    let blockers = value
        .get("blockers")
        .and_then(serde_json::Value::as_array)
        .context("Play 保护机制认证结果缺少阻塞项")?;
    let mut parsed_blockers = Vec::with_capacity(blockers.len());
    for blocker in blockers {
        parsed_blockers.push(PlayProtectBlocker {
            layer: play_protect_text(blocker, "layer")?,
            title: play_protect_text(blocker, "title")?,
            owner: play_protect_text(blocker, "owner")?,
            detail: play_protect_text(blocker, "detail")?,
            required_action: play_protect_text(blocker, "required_action")?,
        });
    }
    Ok(PlayProtectReport {
        checked: true,
        certified: value
            .get("certified")
            .and_then(serde_json::Value::as_bool)
            .context("Play 保护机制认证结果缺少认证判定")?,
        verdict: play_protect_text(value, "verdict")?,
        layers: parsed,
        blockers: parsed_blockers,
        vendor_services,
        patch_freshness,
        firmware,
        leverage,
        note: String::new(),
    })
}

/// Runs the read-only four-layer certification model. Nothing here changes the
/// device, requests a certificate from Google, or reports a certification the
/// phone does not actually hold.
pub fn inspect_play_protect(device: &Device, adb: &Adb) -> Result<PlayProtectReport> {
    device.verify(adb)?;
    let script = play_protect_script()?;
    let mut command = crate::resources::python_command();
    command
        .arg(&script)
        .arg("status")
        .arg("--serial")
        .arg(&device.transport)
        .current_dir(script.parent().context("认证组件目录无效")?)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE");
    if let Some(directory) = captured_attestation_chain() {
        command.arg("--attestation-chain").arg(directory);
    }
    let output = command
        .output()
        .context("无法启动 Play 保护机制认证组件，请检查 Python 3 环境")?;
    let value: serde_json::Value = serde_json::from_slice(&output.stdout)
        .context("Play 保护机制认证组件未返回有效状态，请重新检测")?;
    if !output.status.success() || value["ok"] == false {
        bail!(
            "{}",
            value["message"]
                .as_str()
                .unwrap_or("Play 保护机制认证检测未完成，请重新检测")
        );
    }
    let report = play_protect_from_value(&value)?;
    if report.layers.len() != 4 {
        bail!("Play 保护机制认证分层证据不完整");
    }
    Ok(report)
}

fn apply_compatibility_status(status: &mut GoogleSuiteStatus, value: &serde_json::Value) {
    status.compatibility_checked = true;
    status.compatibility_supported =
        status.installed && value["ok"] == true && value["supported"] == true;
    status.compatibility_active = status.compatibility_supported && value["active"] == true;
    // A valid backup can remain available after a partial application failure.
    status.compatibility_can_rollback = status.installed && value["can_rollback"] == true;
    status.compatibility_note = value["message"]
        .as_str()
        .unwrap_or("兼容补丁状态未确认，请重新检测")
        .into();
}

fn should_apply_during_install(plan: GoogleInstallPlan, status: &GoogleSuiteStatus) -> bool {
    plan == GoogleInstallPlan::Existing
        && status.installed
        && status.compatibility_checked
        && status.compatibility_supported
        && !status.compatibility_active
}

/// Called only after the existing utility confirmation panel or an explicitly
/// confirmed combined installation. The CLI rechecks exact device/APK guards.
pub fn change_compatibility(
    device: &Device,
    adb: &Adb,
    rollback: bool,
    progress: impl Fn(&str),
) -> Result<(GoogleSuiteStatus, String)> {
    device.verify(adb)?;
    progress(if rollback {
        "核对备份并撤销 Google Play 兼容补丁…"
    } else {
        "核对版本、保存备份并应用 Google Play 兼容补丁…"
    });
    let value = run_compatibility_script(device, adb, if rollback { "rollback" } else { "apply" })?;
    let message = value["message"]
        .as_str()
        .unwrap_or("兼容操作未完成")
        .to_string();
    if value["ok"] != true {
        bail!("{message}");
    }
    if !rollback {
        // A missing/old boot receipt is itself a reason to repair. The repair
        // command independently verifies the firmware, signatures and boot hook.
        let runtime_ready = run_script(device, adb, "manage.py", &["status"])
            .ok()
            .and_then(|raw| serde_json::from_str::<serde_json::Value>(&raw).ok())
            .is_some_and(|value| apply_adapter_status(&mut GoogleSuiteStatus::default(), &value));
        if !runtime_ready {
            progress("Google 组件更新后权限发生变化，正在恢复并重新校验…");
            run_script(device, adb, "manage.py", &["repair-runtime"])?;
        }
        progress("检查 Google 通用登录与组件更新恢复…");
        run_script(device, adb, "health.py", &["repair"])?;
    }
    progress("重新读取 Google 服务与兼容补丁状态…");
    let status =
        inspect(device, adb).with_context(|| format!("{message} 状态回读未完成，请重新检测"))?;
    if !status.compatibility_checked || status.compatibility_active == rollback {
        bail!(
            "兼容补丁状态回读与操作结果不一致，请点击检测状态：{}",
            status.compatibility_note
        );
    }
    if !rollback && !status.boot_module_ready {
        bail!("Google 服务修复后权限回读未通过，请查看检测详情");
    }
    if !rollback && status.health_needs_repair {
        bail!("Google 通用登录或应用链接验证尚未恢复，请查看分项检测结果；账号和数据已保留");
    }
    Ok((status, message))
}

pub fn apply_adapter_status(status: &mut GoogleSuiteStatus, value: &serde_json::Value) -> bool {
    let count = value["permission_checks"].as_u64().unwrap_or(0) as u32;
    status.total_permissions = count;
    status.system_server_pid = value["target_pid"].as_u64().unwrap_or(0) as u32;
    status.pid_matches = value["current_pid_matches"].as_bool() == Some(true)
        && value["current_boot_matches"].as_bool() == Some(true);
    let ready = value["status"].as_str() == Some("success")
        && value["detached"].as_bool() == Some(true)
        && status.pid_matches
        && status.system_server_pid > 0
        && count >= 120
        && value["all_checked_granted"].as_bool() == Some(true)
        && value["current_permissions_granted"].as_bool() == Some(true);
    status.granted_count = if ready { count } else { 0 };
    status.boot_module_ready = ready;
    ready
}

fn package_flags_clear(flags: &serde_json::Value) -> Result<bool> {
    // Android writes signed 32-bit flags to packages.xml; negative values are valid.
    let parse = |key: &str| -> Result<i64> {
        flags[key]
            .as_str()
            .context("未读取到持久化标记")?
            .parse()
            .context("持久化标记无效")
    };
    Ok(parse("publicFlags")? & 1 == 0 && parse("privateFlags")? & 8 == 0)
}

pub fn inspect(device: &Device, adb: &Adb) -> Result<GoogleSuiteStatus> {
    device.verify(adb)?;
    let pkg_versions = adb.script(
        &device.transport,
        "pkgs=$(pm list packages --user 0 | tr -d '\\r'); \
         for p in com.google.android.gsf com.google.android.gms com.android.vending; do \
           if printf '%s\\n' \"$pkgs\" | grep -Fxq \"package:$p\"; then \
             v=$(dumpsys package \"$p\" | tr -d '\\r' | sed -n 's/^[[:space:]]*versionName=//p' | head -n 1); \
             printf '%s\\t%s\\n' \"$p\" \"${v:-版本未知}\"; \
           fi; \
         done",
    )?;
    let mut status = GoogleSuiteStatus {
        checked: true,
        ..Default::default()
    };
    match run_script(device, adb, "certification.py", &[])
        .and_then(|text| Ok(serde_json::from_str::<serde_json::Value>(&text)?))
    {
        Ok(value) => {
            status.certification_note = if value["certification_block_visible"] == true {
                "已在手机发现“设备未认证”拦截页；Google 服务当前不可按正常可用验收。"
            } else {
                "未读取到 Google 认证结果；请在手机 Play → 设置 → 关于核对。"
            }
            .into();
            status.certification_catalog = value["catalog_note"]
                .as_str()
                .unwrap_or("机型列表未确认")
                .into();
            status.boot_security_note = format!(
                "Bootloader：{} · Verified Boot：{}",
                match value["bootloader_locked"].as_str() {
                    Some("1") => "已锁定",
                    Some("0") => "已解锁",
                    _ => "未知",
                },
                value["verified_boot"].as_str().unwrap_or("未知")
            );
            if value["disabled_components"]
                .as_array()
                .is_some_and(|a| !a.is_empty())
            {
                status
                    .certification_note
                    .push_str(" 检测到 Google 组件被停用，请在手机应用设置中启用后重试。");
            }
        }
        Err(_) => {
            status.certification_note =
                "认证诊断暂不可用；权限校验成功不代表 Google 认证通过。".into()
        }
    }
    for line in pkg_versions.lines() {
        let Some((pkg, ver)) = line.trim().split_once('\t') else {
            continue;
        };
        let ver = ver.trim().to_string();
        match pkg {
            "com.google.android.gsf" => status.gsf_version = ver,
            "com.google.android.gms" => status.gms_version = ver,
            "com.android.vending" => status.play_version = ver,
            _ => {}
        }
    }
    status.installed = [
        &status.gsf_version,
        &status.gms_version,
        &status.play_version,
    ]
    .iter()
    .all(|v| !v.is_empty());
    if !status.installed {
        status.compatibility_note = "完整安装 Google 服务后，可检查兼容补丁适配情况。".into();
        status.verification_note = if [
            &status.gsf_version,
            &status.gms_version,
            &status.play_version,
        ]
        .iter()
        .all(|v| v.is_empty())
        {
            "未检测到 Google 服务，可继续安装条件检查。"
        } else {
            "检测到部分 Google 组件。为保留现有数据，已停用重复安装，请先检查已有安装。"
        }
        .into();
        return Ok(status);
    }
    match run_compatibility_script(device, adb, "status") {
        Ok(value) => apply_compatibility_status(&mut status, &value),
        Err(error) => status.compatibility_note = format!("兼容补丁暂未完成检测：{error}"),
    }
    // Certification is probed on its own: store usability never implies it.
    match inspect_play_protect(device, adb) {
        Ok(report) => {
            status.certification_note = report.summary();
            status.play_protect = report;
        }
        Err(error) => {
            status.play_protect.note = format!("Play 保护机制认证检测未完成：{error}");
        }
    }
    let verification = (|| -> Result<()> {
        run_script(device, adb, "manage.py", &["preflight"])?;
        let value: serde_json::Value =
            serde_json::from_str(&run_script(device, adb, "manage.py", &["status"])?)?;
        if !apply_adapter_status(&mut status, &value) {
            bail!("当前 Google 权限已失效或未恢复；组件更新后请点击「修复 Google 服务」");
        }
        let package_state: serde_json::Value =
            serde_json::from_str(&run_script(device, adb, "package_state.py", &[])?)?;
        status.persistent_flags_clear = package_flags_clear(&package_state["persistent_package"])?;
        if !status.persistent_flags_clear {
            bail!("检测到异常的持久化标记，请停止安装操作并检查设备");
        }
        Ok(())
    })();
    status.verification_note = match verification {
        Ok(()) => format!(
            "三件套已安装；本次启动权限检测 {}/{} 通过。请在手机打开 Play 验证实际使用。",
            status.granted_count, status.total_permissions
        ),
        Err(e) => format!(
            "三件套已安装，适配状态未确认。请解锁手机并确认 StepClaw 可用，再检测。原因：{e}"
        ),
    };
    match run_script(device, adb, "health.py", &["status"])
        .and_then(|raw| Ok(serde_json::from_str::<serde_json::Value>(&raw)?))
    {
        Ok(report) => {
            status.health_needs_repair = report["needs_repair"].as_bool().unwrap_or(true);
            status.google_accounts = report["account_count"].as_u64().unwrap_or(0) as u32;
            if let Some(checks) = report["checks"].as_array() {
                status.health_checks = checks
                    .iter()
                    .filter_map(|c| {
                        Some(crate::device::GoogleHealthCheck {
                            title: c["title"].as_str()?.to_string(),
                            state: c["state"].as_str()?.to_string(),
                            detail: c["detail"].as_str()?.to_string(),
                        })
                    })
                    .collect();
            }
        }
        Err(_) => {
            status.health_needs_repair = true;
            status.health_checks.push(crate::device::GoogleHealthCheck {
                title: "通用登录检测".into(),
                state: "unknown".into(),
                detail: "读取未完成，请重新检测；不会据此清除账号或重新安装".into(),
            });
        }
    }
    Ok(status)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum GoogleInstallPlan {
    Missing,
    Existing,
    Partial,
}

fn install_plan(preflight: &serde_json::Value) -> Result<GoogleInstallPlan> {
    let packages = preflight["google_packages"]
        .as_object()
        .context("未读取到 Google 包状态")?;
    let mut installed = 0;
    for package in [
        "com.google.android.gsf",
        "com.google.android.gms",
        "com.android.vending",
    ] {
        installed += usize::from(
            packages
                .get(package)
                .and_then(serde_json::Value::as_bool)
                .context("Google 包检测结果不完整")?,
        );
    }
    Ok(match installed {
        0 => GoogleInstallPlan::Missing,
        3 => GoogleInstallPlan::Existing,
        _ => GoogleInstallPlan::Partial,
    })
}

pub struct GoogleInstallResult {
    pub google: GoogleSuiteStatus,
    pub companion: crate::companion_app::CompanionStatus,
    pub message: String,
}

/// A single user-facing installation action always provisions the companion.
/// Existing/partial Google APKs are preserved. A verified existing bundle can
/// receive the compatibility patch as part of this explicit installation.
pub fn install(device: &Device, adb: &Adb, progress: impl Fn(&str)) -> Result<GoogleInstallResult> {
    device.verify(adb)?;
    progress("检查 Google Play 与 StepX伴侣安装条件…");
    let preflight: serde_json::Value =
        serde_json::from_str(&run_script(device, adb, "manage.py", &["preflight"])?)?;
    if preflight["user_unlocked"].as_bool() != Some(true) {
        bail!("请先解锁手机，再安装 Google Play 与 StepX伴侣");
    }
    let plan = install_plan(&preflight)?;
    let companion = crate::companion_app::install(adb, device, |message| {
        progress(&format!("StepX伴侣 · {message}"));
    })
    .context("StepX伴侣未完成安装或权限校验；尚未改动 Google 安装")?;
    let google_message = match plan {
        GoogleInstallPlan::Missing => install_fresh(device, adb, |label| {
            progress(&format!("Google Play · {label}"))
        })
        .with_context(|| format!("{} Google Play 安装尚未完成", companion.message))?,
        GoogleInstallPlan::Existing => "已有 Google 组件已保留，未重复安装。".into(),
        GoogleInstallPlan::Partial => {
            "Google 组件不完整，现有组件已保留；请按检测结果处理 Google 安装。".into()
        }
    };
    // Preserve a partial companion service failure in this combined entry point.
    let mut message = format!("{} {google_message}", companion.message);
    progress("正在回读 Google Play 与 StepX伴侣安装状态…");
    let mut google = inspect(device, adb)
        .with_context(|| format!("{} Google Play 状态回读未完成", companion.message))?;
    if should_apply_during_install(plan, &google) {
        let (updated, patch_message) = change_compatibility(device, adb, false, &progress)
            .with_context(|| format!("{} Google 组件已保留，兼容补丁未完成", companion.message))?;
        google = updated;
        message.push(' ');
        message.push_str(&patch_message);
    } else if plan == GoogleInstallPlan::Existing && !google.compatibility_active {
        message.push(' ');
        message.push_str(&google.compatibility_note);
    }
    if plan == GoogleInstallPlan::Existing
        && google.compatibility_supported
        && !google.boot_module_ready
    {
        progress("恢复 Google 组件更新后的运行权限…");
        run_script(device, adb, "manage.py", &["repair-runtime"])?;
        google = inspect(device, adb)?;
        if !google.boot_module_ready {
            bail!("Google 服务恢复后校验未通过，请查看检测详情");
        }
    }
    if google.compatibility_supported && google.installed && google.health_needs_repair {
        progress("完善通用 Google 登录与组件更新恢复…");
        run_script(device, adb, "health.py", &["repair"])?;
        google = inspect(device, adb)?;
    }
    Ok(GoogleInstallResult {
        google,
        companion,
        message,
    })
}

fn install_fresh(device: &Device, adb: &Adb, progress: impl Fn(&str)) -> Result<String> {
    device.verify(adb)?;
    progress("1 / 6 · 检查设备与安装条件");
    // Preflight validates the exact firmware before downloading or changing phone data.
    let preflight: serde_json::Value =
        serde_json::from_str(&run_script(device, adb, "manage.py", &["preflight"])?)?;
    if preflight["user_unlocked"].as_bool() != Some(true) {
        bail!("请先在手机上解锁，再安装 Google 服务");
    }
    let packages = preflight["google_packages"]
        .as_object()
        .context("未读取到 Google 包状态")?;
    if packages.len() != 3 || packages.values().any(|v| v.as_bool() != Some(false)) {
        bail!("手机已有 Google 组件，已停止重复安装；请使用检测状态");
    }
    // These scripts recheck serial, firmware, APK signatures and baseline before each mutation.
    for (stage, label) in [
        ("fetch", "2 / 6 · 获取并核验安装包"),
        ("prepare", "3 / 6 · 保存安装基线"),
        ("install-apks", "4 / 6 · 安装 Google 三件套"),
    ] {
        progress(label);
        run_script(device, adb, "manage.py", &[stage])
            .with_context(|| format!("Google 安装停在 {stage} 阶段；请检查原因，勿重复覆盖安装"))?;
    }
    progress("5 / 6 · 保存首次启动前的包状态");
    run_script(device, adb, "package_state.py", &[])?;
    let state = crate::resources::user_state().join("google-play/state");
    let before = state.join("package-before-first-reboot.json");
    // Never overwrite evidence from a prior attempt.
    let mut target = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&before)?;
    use std::io::Write;
    target.write_all(&std::fs::read(state.join("package-state-current.json"))?)?;
    progress("6 / 6 · 安装开机适配，完成后需手动重启");
    run_script(device, adb, "install_adapter.py", &[])?;
    Ok("安装步骤已完成。请手动重启手机并解锁，再点击「检测状态」；确认适配通过前暂不要登录 Google。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compatibility_status_keeps_certification_separate_and_requires_guards() {
        let mut status = GoogleSuiteStatus {
            installed: true,
            certification_note: "认证未核验".into(),
            ..Default::default()
        };
        let valid = serde_json::json!({
            "ok": true, "supported": true, "active": true,
            "can_rollback": true, "message": "已应用兼容补丁"
        });
        apply_compatibility_status(&mut status, &valid);
        assert!(status.compatibility_active);
        assert!(status.compatibility_can_rollback);
        assert_eq!(status.certification_note, "认证未核验");
        for key in ["ok", "supported", "active"] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(key);
            apply_compatibility_status(&mut status, &missing);
            assert!(!status.compatibility_active, "{key}");
        }
        status.installed = false;
        apply_compatibility_status(&mut status, &valid);
        assert!(!status.compatibility_supported);
        assert!(!status.compatibility_active);
        assert!(!status.compatibility_can_rollback);
    }

    #[test]
    fn failed_patch_preserves_the_rollback_entry() {
        let mut status = GoogleSuiteStatus {
            installed: true,
            ..Default::default()
        };
        apply_compatibility_status(
            &mut status,
            &serde_json::json!({
                "ok": false, "supported": false, "active": false,
                "can_rollback": true, "message": "本次应用未完成，可撤销已修改的部分"
            }),
        );
        assert!(status.compatibility_checked);
        assert!(!status.compatibility_active);
        assert!(status.compatibility_can_rollback);
        assert!(status.compatibility_note.contains("未完成"));
    }

    #[test]
    fn combined_install_only_patches_a_verified_existing_bundle() {
        let valid = GoogleSuiteStatus {
            installed: true,
            compatibility_checked: true,
            compatibility_supported: true,
            ..Default::default()
        };
        assert!(should_apply_during_install(
            GoogleInstallPlan::Existing,
            &valid
        ));
        for plan in [GoogleInstallPlan::Missing, GoogleInstallPlan::Partial] {
            assert!(!should_apply_during_install(plan, &valid));
        }
        let mut status = valid.clone();
        status.compatibility_active = true;
        assert!(!should_apply_during_install(
            GoogleInstallPlan::Existing,
            &status
        ));
        status = valid.clone();
        status.compatibility_checked = false;
        assert!(!should_apply_during_install(
            GoogleInstallPlan::Existing,
            &status
        ));
        status = valid.clone();
        status.compatibility_supported = false;
        assert!(!should_apply_during_install(
            GoogleInstallPlan::Existing,
            &status
        ));
        status = valid;
        status.installed = false;
        assert!(!should_apply_during_install(
            GoogleInstallPlan::Existing,
            &status
        ));
    }

    #[test]
    fn installation_plan_preserves_existing_and_partial_google_packages() {
        let report = |gsf, gms, play| serde_json::json!({"google_packages":{"com.google.android.gsf":gsf,"com.google.android.gms":gms,"com.android.vending":play}});
        assert_eq!(
            install_plan(&report(false, false, false)).unwrap(),
            GoogleInstallPlan::Missing
        );
        assert_eq!(
            install_plan(&report(true, true, true)).unwrap(),
            GoogleInstallPlan::Existing
        );
        assert_eq!(
            install_plan(&report(false, true, true)).unwrap(),
            GoogleInstallPlan::Partial
        );
        assert_eq!(
            install_plan(&report(true, false, false)).unwrap(),
            GoogleInstallPlan::Partial
        );
        assert!(install_plan(&serde_json::json!({})).is_err());
        assert!(install_plan(&serde_json::json!({"google_packages":{"com.google.android.gsf":true,"com.google.android.gms":true}})).is_err());
    }
    #[test]
    fn signed_android_flags_are_checked_without_assuming_clear() {
        assert!(package_flags_clear(
            &serde_json::json!({"publicFlags":"-1605714364", "privateFlags":"-1476357872"})
        )
        .unwrap());
        assert!(
            !package_flags_clear(&serde_json::json!({"publicFlags":"1", "privateFlags":"0"}))
                .unwrap()
        );
        assert!(
            !package_flags_clear(&serde_json::json!({"publicFlags":"0", "privateFlags":"8"}))
                .unwrap()
        );
        assert!(package_flags_clear(&serde_json::json!({"publicFlags":"0"})).is_err());
    }
    #[test]
    fn missing_or_stale_adapter_evidence_never_passes() {
        let valid = serde_json::json!({"status":"success", "detached":true, "current_pid_matches":true,
            "current_boot_matches":true, "permission_checks":153, "all_checked_granted":true,
            "current_permissions_granted":true, "target_pid":123});
        assert!(apply_adapter_status(
            &mut GoogleSuiteStatus::default(),
            &valid
        ));
        for key in [
            "status",
            "detached",
            "current_pid_matches",
            "current_boot_matches",
            "permission_checks",
            "all_checked_granted",
            "current_permissions_granted",
            "target_pid",
        ] {
            let mut missing = valid.clone();
            missing.as_object_mut().unwrap().remove(key);
            assert!(
                !apply_adapter_status(&mut GoogleSuiteStatus::default(), &missing),
                "{key}"
            );
        }
        assert!(!apply_adapter_status(
            &mut GoogleSuiteStatus::default(),
            &serde_json::json!({})
        ));
    }

    fn certification_payload() -> serde_json::Value {
        serde_json::json!({
            "schema": "stepx-play-protect-certification-v1",
            "format": 1,
            "checked_at": "2026-10-23T10:46:53+00:00",
            "serial": "ZYPZ261A01M692000LP",
            "identity": {"brand": "StepX", "manufacturer": "StepX", "model": "PZ261A",
                         "device": "pandora", "name": "pandora"},
            "fingerprint": "StepX/pandora/pandora:16/BQ2A.260122.002-BP2A.250705.008/172:user/release-keys",
            "certified": false,
            "verdict": "not_certified",
            "store_access": {"usable": true, "note": "商店可用性与设备认证是两个独立结论"},
            "layers": [
                {"layer": "system_boot", "title": "第 2 层 · 系统与启动层", "owner": "oem",
                 "state": "pass", "note": "启动链正常",
                 "checks": [{"check": "ro.build.tags", "value": "release-keys",
                             "expected": "release-keys", "ok": true}]},
                {"layer": "client_admission", "title": "第 1 层 · 客户端准入层", "owner": "user",
                 "state": "pass", "note": "商店可用", "store_usable": true,
                 "checks": [{"check": "Play 兼容补丁", "value": true, "expected": true, "ok": true}]},
                {"layer": "key_attestation", "title": "第 3 层 · 硬件密钥证明层", "owner": "oem",
                 "state": "fail", "note": "根证书不匹配",
                 "checks": [{"check": "链内签名与有效期", "value": true,
                             "expected": true, "ok": true},
                            {"check": "根证书匹配 Google 官方硬件根", "value": false,
                             "expected": true, "ok": false,
                             "detail": "verification failed"}]},
                {"layer": "rkp_and_catalog", "title": "第 4 层 · 远程密钥配置与设备目录层",
                 "owner": "oem", "state": "fail", "note": "后端未登记",
                 "rkp_components": {"default": {"hwVersion": "3"}},
                 "checks": [
                     {"check": "RKP signCertificates", "value": 444, "expected": "200", "ok": false},
                     {"check": "防火墙放行 IRemoteProvisioning", "value": false, "expected": true,
                      "ok": null, "detail": "未取证"},
                     {"check": "Google 认证设备目录收录", "value": false, "expected": true,
                      "ok": false}
                 ]}
            ],
            "blockers": [
                {"layer": "key_attestation", "title": "第 3 层 · 硬件密钥证明层", "owner": "oem",
                 "detail": "根证书匹配 Google 官方硬件根=False（期望 True）",
                 "required_action": "只能由厂商在产线烧录 Google 签发的生产 Keybox"},
                {"layer": "rkp_and_catalog", "title": "第 4 层 · 远程密钥配置与设备目录层",
                 "owner": "oem", "detail": "RKP signCertificates=444",
                 "required_action": "需要 SoC 厂商向 Google RKP 后台登记硬件公钥"}
            ],
            "sources": ["https://android.googleapis.com/attestation/root"],
            "limitations": ["本工具只读取证"],
            "vendor_services": {
                "updater_installed": true,
                "updater_holds_dsid_permission": true,
                "dsid_readable_from_host": false,
                "dsid_host_error": "需要 com.stepos.secureservice.permission.OPEN_DATA_PROVIDER",
                "gsf_android_id": "23392eef6ce29cb0",
                "note": "dsid 受 signature|privileged 权限保护。",
            },
            "patch_freshness": {
                "security_patch": "2025-12-05", "vendor_security_patch": "2025-12-05",
                "age_days": 322, "stale": true,
                "note": "安全补丁 2025-12-05，距今 322 天。",
            },
            "firmware": {
                "available": true, "http_code": 200, "new_version": null,
                "latest_version": "Step AOS 1.0.0.2.26001_stable_user_20261004_131555",
                "current_build": "Step AOS 1.0.0.2.26001_stable_user_20261004_131555",
                "current_incremental": "172", "same_as_current": true,
                "need_update": false, "release_date": "2026-10-16 17:37:46",
                "note": "官方通道没有比设备当前版本更新的构建。",
            },
            "leverage": [
                {"action": "向 Google RKP 后台登记硬件公钥",
                 "clears": ["key_attestation", "rkp_and_catalog"],
                 "owner": "soc_vendor_or_oem", "reason": "一步解两层"},
                {"action": "移除防火墙拦截并配置 hostname", "clears": ["rkp_and_catalog"],
                 "owner": "oem", "reason": "让系统能真正发起轮换"},
                {"action": "CTS/VTS/GTS 备案", "clears": ["rkp_and_catalog"],
                 "owner": "oem", "reason": "进入认证设备目录"},
                {"action": "烧录生产 Keybox", "clears": ["key_attestation"],
                 "owner": "oem", "reason": "传统工厂预置路径"},
            ],
        })
    }

    fn certification_payload_with_extras() -> serde_json::Value {
        let mut payload = certification_payload();
        // Layer 3 now carries a real revocation check that may be unprobed.
        let revocation = serde_json::json!({
            "check": "链上证书不在 Google 吊销列表", "value": null, "expected": false,
            "ok": null, "detail": "离线模式未读取 Google 吊销列表",
        });
        payload["layers"][2]["checks"]
            .as_array_mut()
            .unwrap()
            .push(revocation);
        payload
    }

    #[test]
    fn certification_report_carries_vendor_services_and_leverage() {
        let report = play_protect_from_value(&certification_payload_with_extras()).unwrap();
        let vendor = report.vendor_services.as_ref().expect("vendor services");
        assert_eq!(vendor.updater_installed, Some(true));
        assert_eq!(vendor.updater_holds_dsid_permission, Some(true));
        assert_eq!(vendor.dsid_readable_from_host, Some(false));
        assert_eq!(vendor.gsf_android_id, "23392eef6ce29cb0");
        assert_eq!(report.leverage.len(), 4);
        assert_eq!(
            report.leverage[0].clears,
            ["key_attestation", "rkp_and_catalog"]
        );
        assert_eq!(report.leverage[0].owner, "soc_vendor_or_oem");
        assert!(report.leverage[1].clears.len() < report.leverage[0].clears.len());
    }

    #[test]
    fn certification_unprobed_vendor_flag_stays_none() {
        let mut payload = certification_payload_with_extras();
        payload["vendor_services"]["dsid_readable_from_host"] = serde_json::json!(null);
        let vendor = play_protect_from_value(&payload)
            .unwrap()
            .vendor_services
            .unwrap();
        // A probe that could not run stays None rather than becoming a failure.
        assert_eq!(vendor.dsid_readable_from_host, None);
    }

    #[test]
    fn certification_null_vendor_services_stay_absent() {
        let mut payload = certification_payload_with_extras();
        payload["vendor_services"] = serde_json::json!(null);
        assert!(play_protect_from_value(&payload)
            .unwrap()
            .vendor_services
            .is_none());
    }

    #[test]
    fn certification_leverage_must_be_complete() {
        for key in ["action", "clears", "owner", "reason"] {
            let mut payload = certification_payload_with_extras();
            payload["leverage"][0].as_object_mut().unwrap().remove(key);
            assert!(play_protect_from_value(&payload).is_err(), "{key}");
        }
        let mut payload = certification_payload_with_extras();
        payload["leverage"][0]["clears"] = serde_json::json!(["not_a_layer"]);
        // Unknown layer ids are still parsed; the UI shows them verbatim.
        assert_eq!(
            play_protect_from_value(&payload).unwrap().leverage[0].clears,
            vec!["not_a_layer".to_string()]
        );
    }

    #[test]
    fn certification_revocation_is_a_distinct_third_check() {
        let report = play_protect_from_value(&certification_payload_with_extras()).unwrap();
        let attestation = report
            .layers
            .iter()
            .find(|l| l.layer == "key_attestation")
            .unwrap();
        assert_eq!(attestation.checks.len(), 3);
        assert_eq!(attestation.checks[2].check, "链上证书不在 Google 吊销列表");
        // An unprobed revocation check must stay None, never a silent failure.
        assert_eq!(attestation.checks[2].ok, None);
    }

    #[test]
    fn certification_report_parses_all_four_layers_and_blockers() {
        let report = play_protect_from_value(&certification_payload()).unwrap();
        assert!(report.checked);
        assert!(!report.certified);
        assert_eq!(report.verdict, "not_certified");
        assert_eq!(report.layers.len(), 4);
        assert_eq!(report.blockers.len(), 2);
        assert_eq!(report.blockers[0].owner, "oem");
        assert_eq!(report.layers[0].state, "pass");
        assert_eq!(report.layers[2].state, "fail");
        assert_eq!(report.state_label(), "未通过认证");
        // Numbers and booleans must reach the UI as text, never be dropped.
        let rkp = &report.layers[3];
        assert_eq!(rkp.checks[0].value, "444");
        assert_eq!(rkp.checks[1].value, "false");
        assert_eq!(rkp.checks[1].expected, "true");
    }

    #[test]
    fn certification_unprobed_check_stays_unknown_and_never_passes() {
        let report = play_protect_from_value(&certification_payload()).unwrap();
        let entry = &report.layers[3].checks[1];
        assert_eq!(entry.ok, None);
        assert_eq!(entry.detail.as_deref(), Some("未取证"));
    }

    #[test]
    fn certification_summary_separates_store_access_from_the_verdict() {
        let mut status = GoogleSuiteStatus::default();
        status.play_protect = play_protect_from_value(&certification_payload()).unwrap();
        let summary = status.play_protect.summary();
        assert!(summary.contains("2 层通过"), "{summary}");
        assert!(summary.contains("未通过"), "{summary}");
        assert!(summary.contains("厂商侧"), "{summary}");
        assert!(!summary.contains("已通过认证"));
    }

    #[test]
    fn certification_schema_mismatch_is_rejected() {
        let mut payload = certification_payload();
        payload["schema"] = serde_json::json!("some-other-schema");
        assert!(play_protect_from_value(&payload).is_err());
    }

    #[test]
    fn certification_missing_fields_are_rejected() {
        for key in ["schema", "certified", "verdict", "layers", "blockers"] {
            let mut payload = certification_payload();
            payload.as_object_mut().unwrap().remove(key);
            assert!(play_protect_from_value(&payload).is_err(), "{key}");
        }
        for key in ["layer", "title", "owner", "state", "note", "checks"] {
            let mut payload = certification_payload();
            payload["layers"].as_array_mut().unwrap()[0]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                play_protect_from_value(&payload).is_err(),
                "layers[0].{key}"
            );
        }
        for key in ["check", "value", "expected"] {
            let mut payload = certification_payload();
            payload["layers"][0]["checks"].as_array_mut().unwrap()[0]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                play_protect_from_value(&payload).is_err(),
                "checks[0].{key}"
            );
        }
        for key in ["layer", "title", "owner", "detail", "required_action"] {
            let mut payload = certification_payload();
            payload["blockers"].as_array_mut().unwrap()[0]
                .as_object_mut()
                .unwrap()
                .remove(key);
            assert!(
                play_protect_from_value(&payload).is_err(),
                "blockers[0].{key}"
            );
        }
    }

    #[test]
    fn certification_non_scalar_check_value_is_rejected() {
        let mut payload = certification_payload();
        payload["layers"][0]["checks"][0]["value"] = serde_json::json!({"nested": true});
        assert!(play_protect_from_value(&payload).is_err());
    }

    #[test]
    fn certification_report_carries_patch_freshness_and_firmware() {
        let report = play_protect_from_value(&certification_payload_with_extras()).unwrap();
        let patch = report.patch_freshness.as_ref().expect("patch freshness");
        assert_eq!(patch.security_patch.as_deref(), Some("2025-12-05"));
        assert_eq!(patch.age_days, Some(322));
        assert_eq!(patch.stale, Some(true));
        assert!(patch.note.contains("322"));
        let firmware = report.firmware.as_ref().expect("firmware");
        assert!(firmware.available);
        assert_eq!(firmware.http_code, Some(200));
        assert!(firmware.new_version.is_none());
        assert!(firmware.same_as_current);
        assert_eq!(firmware.need_update, Some(false));
        assert_eq!(firmware.current_incremental.as_deref(), Some("172"));
    }

    #[test]
    fn certification_null_patch_and_firmware_stay_absent() {
        let mut payload = certification_payload_with_extras();
        payload["patch_freshness"] = serde_json::json!(null);
        payload["firmware"] = serde_json::json!(null);
        let report = play_protect_from_value(&payload).unwrap();
        assert!(report.patch_freshness.is_none());
        assert!(report.firmware.is_none());
    }

    #[test]
    fn certification_unprobed_patch_fields_stay_none() {
        let mut payload = certification_payload_with_extras();
        payload["patch_freshness"] = serde_json::json!({
            "security_patch": null, "vendor_security_patch": null,
            "age_days": null, "stale": null, "note": "未能解析安全补丁日期",
        });
        let patch = play_protect_from_value(&payload)
            .unwrap()
            .patch_freshness
            .unwrap();
        // An unmeasured age must never be rendered as zero or as fresh.
        assert_eq!(patch.age_days, None);
        assert_eq!(patch.stale, None);
        assert!(patch.security_patch.is_none());
    }

    #[test]
    fn certification_firmware_defaults_to_not_available() {
        let mut payload = certification_payload_with_extras();
        payload["firmware"] = serde_json::json!({"note": "没有可用的固件元数据探针"});
        let firmware = play_protect_from_value(&payload).unwrap().firmware.unwrap();
        assert!(!firmware.available);
        assert!(!firmware.same_as_current);
        assert!(firmware.new_version.is_none());
    }

    #[test]
    fn certification_firmware_a_newer_build_is_not_latest() {
        let mut payload = certification_payload_with_extras();
        payload["firmware"]["new_version"] = serde_json::json!("Step AOS 1.0.0.3.99999");
        payload["firmware"]["same_as_current"] = serde_json::json!(false);
        let firmware = play_protect_from_value(&payload).unwrap().firmware.unwrap();
        assert_eq!(
            firmware.new_version.as_deref(),
            Some("Step AOS 1.0.0.3.99999")
        );
        assert!(!firmware.same_as_current);
    }

    #[test]
    fn certification_layer_count_is_validated() {
        let mut payload = certification_payload();
        payload["layers"].as_array_mut().unwrap().pop();
        let report = play_protect_from_value(&payload).unwrap();
        assert_eq!(report.layers.len(), 3);
    }
}
