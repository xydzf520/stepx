//! Install the integrated GUI runtime inside a same-signer StepX Companion APK.
use crate::{
    adb::{run_process, Adb},
    companion_gui,
    device::Device,
};
use anyhow::{bail, ensure, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{path::Path, time::Duration};

pub const PACKAGE: &str = "com.stepx.companion.app";

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct NeoStatus {
    pub checked: bool,
    pub installed: bool,
    pub companion_installed: bool,
    pub version: String,
    pub signature_verified: bool,
    pub accessibility_enabled: bool,
    pub accessibility_bound: bool,
    pub ime_enabled: bool,
    pub doze_exempt: bool,
    pub recovery_available: bool,
    pub selected: bool,
    pub model_configured: bool,
    pub message: String,
}

fn helper(
    adb: &Adb,
    device: &Device,
    operation: &str,
    apk: Option<&Path>,
    ime: bool,
) -> Result<Value> {
    device.verify(adb)?;
    let script = crate::resources::root().join("scripts/neo-agent-install.py");
    ensure!(script.is_file(), "缺少 Neo 安装组件，请使用完整安装包");
    let mut command = crate::resources::python_command();
    command
        .arg(script)
        .arg(operation)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE");
    if let Some(apk) = apk {
        command.arg("--apk").arg(apk);
    }
    if ime {
        command.arg("--enable-ime");
    }
    let output = run_process(command, None, Duration::from_secs(300))?;
    let value: Value =
        serde_json::from_slice(&output.stdout).context("Neo 安装组件未返回有效结果")?;
    ensure!(
        output.status.success() && value["ok"] == true,
        "{}",
        value["error"]
            .as_str()
            .unwrap_or("Neo 配置未完成，请检查手机连接")
    );
    Ok(value)
}

fn with_route(mut status: NeoStatus, route: Result<Value>) -> NeoStatus {
    status.message = if !status.installed {
        "当前伴侣未内置 GUI Agent；请选择同签名的完整伴侣 APK 安装或更新。".into()
    } else if !status.signature_verified {
        "签名未通过核验，未启用执行路由。".into()
    } else {
        match route {
            Ok(value) => {
                status.selected = value["selected"].as_bool().unwrap_or(false);
                status.model_configured = value["modelConfigured"].as_bool().unwrap_or(false);
                if value["accessibilityConnected"] == true
                    || value["executorConnected"] == true
                    || value["connected"] == true
                {
                    status.accessibility_bound = true;
                }
                if status.selected {
                    "伴侣内置 GUI Agent 已接入任务路由；请在手机确认模型及允许操作的应用。".into()
                } else {
                    "伴侣已内置 GUI Agent；打开手机配置后可启用任务路由。".into()
                }
            }
            Err(_) => "伴侣内置 GUI Agent 的路由尚未就绪，请打开手机配置检查。".into(),
        }
    };
    status
}

pub fn inspect(adb: &Adb, device: &Device) -> Result<NeoStatus> {
    let status: NeoStatus = serde_json::from_value(helper(adb, device, "inspect", None, false)?)?;
    let route = if status.installed && status.signature_verified {
        companion_gui::call(adb, device, json!({"op":"agent","agentOp":"neo_status"}))
    } else {
        Ok(json!({}))
    };
    Ok(with_route(status, route))
}

pub fn install(adb: &Adb, device: &Device, apk: &Path, enable_ime: bool) -> Result<NeoStatus> {
    let status: NeoStatus =
        serde_json::from_value(helper(adb, device, "install", Some(apk), enable_ime)?)?;
    match companion_gui::call(adb, device, json!({"op":"agent","agentOp":"neo_enable"})) {
        Ok(route) if route["selected"] == true => Ok(with_route(status, Ok(route))),
        result => {
            let cause = result
                .err()
                .map(|e| e.to_string())
                .unwrap_or_else(|| "手机尚未确认 Neo 路由启用".into());
            let recovery = helper(adb, device, "restore", None, false);
            bail!(
                "伴侣已更新，但 GUI Agent 激活未完成：{cause}；{}",
                match recovery {
                    Ok(_) => "本次权限配置已恢复；APK 与应用数据保留".to_string(),
                    Err(error) => format!("权限恢复待处理：{error}"),
                }
            );
        }
    }
}

pub fn activate(adb: &Adb, device: &Device) -> Result<NeoStatus> {
    let status: NeoStatus = serde_json::from_value(helper(adb, device, "inspect", None, false)?)?;
    ensure!(
        status.installed && status.signature_verified && status.accessibility_enabled,
        "请先安装并完成 Neo 无障碍配置"
    );
    let route = companion_gui::call(adb, device, json!({"op":"agent","agentOp":"neo_enable"}))?;
    ensure!(route["selected"] == true, "手机尚未确认 Neo 路由启用");
    Ok(with_route(status, Ok(route)))
}

pub fn restore(adb: &Adb, device: &Device) -> Result<NeoStatus> {
    // Stop/finish on the phone first; the phone refuses this operation while a task owns the runtime.
    let route = companion_gui::call(adb, device, json!({"op":"agent","agentOp":"neo_disable"}))?;
    ensure!(
        route["selected"] == false,
        "请先结束 Neo 当前任务再撤销配置"
    );
    let mut status: NeoStatus =
        serde_json::from_value(helper(adb, device, "restore", None, false)?)?;
    status.message =
        "已撤销内置 GUI Agent 的组件配置；保留伴侣其他服务、后台策略与应用数据。".into();
    Ok(status)
}

pub fn open_phone(adb: &Adb, device: &Device) -> Result<()> {
    device.verify(adb)?;
    let route = companion_gui::call(adb, device, json!({"op":"agent","agentOp":"neo_status"}))
        .or_else(|_| companion_gui::call(adb, device, json!({"op":"neo_status"})))
        .context("请先安装通过签名核验的 Neo")?;
    ensure!(
        route["supported"] != false && route["installed"] != false,
        "请先安装通过签名核验的 Neo"
    );
    let response = adb.shell(
        &device.transport,
        &[
            "am",
            "start",
            "--user",
            "0",
            "-n",
            "com.stepx.companion.app/.NeoSettingsActivity",
        ],
    )?;
    ensure!(
        !response.contains("Error:") && !response.contains("Exception"),
        "伴侣 GUI Agent 配置页未打开，请解锁手机后重试"
    );
    Ok(())
}
