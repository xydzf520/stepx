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
    pub amoo_skill_installed: bool,
    pub execution_mode: String,
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
                status.execution_mode = value["preferredExecutionMode"]
                    .as_str()
                    .unwrap_or("foreground")
                    .into();
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
    let mut ready = with_route(status, route);
    if ready.installed && ready.signature_verified && ready.selected {
        let _ = crate::gui_agent_api::ensure_started(adb, device);
    }
    if !ready.amoo_skill_installed {
        if let Ok(official) = crate::studio::gui_skill_status(adb, device) {
            if official.installed {
                ready.amoo_skill_installed = true;
            }
        }
    }
    Ok(ready)
}

pub fn amoo_skill_install_prompt() -> String {
    format!("请安装或更新我账号的 StepX伴侣跨端协作技能（手机资料交给电脑处理，以及手机界面操作）。使用官方技能文件机制，仅写入 /root/.openclaw/workspace/skills/stepx-background-gui/SKILL.md；若你的实际技能工作目录不同，请使用当前技能目录。已有同名文件先备份。不要操作手机、不要修改其他技能、不要创建测试任务。写入后读回核对，并确认新会话能发现此技能。需要授权时请显示确认。文件完整内容：\n{}", include_str!("../assets/stepx-background-gui-skill.md"))
}

pub fn resolve_gui_skill_install_with<F>(try_official: F) -> Option<String>
where
    F: FnOnce() -> Result<crate::studio::OfficialGuiSkillStatus>,
{
    match try_official() {
        Ok(status) if status.installed => None,
        _ => Some(amoo_skill_install_prompt()),
    }
}

/// Attempts to install the official GUI skill via `StudioProvider` (`gui-skill-status` /
/// `gui-skill-install`) and starts the local `gui_agent_api` server, falling back to
/// `amoo_skill_install_prompt()` only when the official skill market installation is unavailable.
pub fn ensure_official_gui_skill_or_prompt(adb: &Adb, device: &Device) -> Option<String> {
    let _ = crate::gui_agent_api::ensure_started(adb, device);
    resolve_gui_skill_install_with(|| crate::studio::gui_skill_install(adb, device))
}

/// Installs the official GUI skill via `StudioProvider` first, or falls back to sending
/// `amoo_skill_install_prompt()` over `AgentBus` when the official market is unavailable.
pub fn connect_amoo_skill(
    adb: &Adb,
    device: &Device,
) -> Result<Option<crate::amoo::PendingReply>> {
    if let Some(prompt) = ensure_official_gui_skill_or_prompt(adb, device) {
        let mut bus = crate::amoo::AgentBus::connect(adb, device)?;
        let key = bus.create_session()?;
        let receipt = bus.send(&key, &prompt)?;
        Ok(Some(receipt))
    } else {
        Ok(None)
    }
}

fn install_system_service(adb: &Adb, device: &Device, apk: &Path) -> Result<()> {
    let script = crate::resources::root().join("scripts/companion-boot.py");
    ensure!(script.is_file(), "缺少伴侣系统服务安装组件");
    let mut command = crate::resources::python_command();
    command
        .arg(script)
        .arg("activate")
        .arg("--apk")
        .arg(apk)
        .env("STEPX_SERIAL", &device.transport)
        .env("STEPX_DEVICE_SERIAL", &device.serial)
        .env("STEPX_ADB", &adb.path)
        .env_remove("PYTHONOPTIMIZE");
    let output = run_process(command, None, Duration::from_secs(180))?;
    let value: Value = serde_json::from_slice(&output.stdout).context("系统服务未返回安装回执")?;
    ensure!(
        output.status.success() && value["status"] == "activated" && value["service_uid"] == 1000,
        "{}",
        value["error"].as_str().unwrap_or("系统服务未通过回读")
    );
    Ok(())
}

pub fn install(adb: &Adb, device: &Device, apk: &Path, enable_ime: bool) -> Result<NeoStatus> {
    let status: NeoStatus =
        serde_json::from_value(helper(adb, device, "install", Some(apk), enable_ime)?)?;
    match companion_gui::call(adb, device, json!({"op":"agent","agentOp":"neo_enable"})) {
        Ok(route) if route["selected"] == true => {
            let mut ready = with_route(status, Ok(route));
            let _ = crate::gui_agent_api::ensure_started(adb, device);
            if let Ok(official) = crate::studio::gui_skill_install(adb, device) {
                if official.installed {
                    ready.amoo_skill_installed = true;
                }
            }
            match install_system_service(adb, device, apk) {
                Ok(()) => ready
                    .message
                    .push_str(" 系统级服务已安装，支持后台虚拟屏的固件可切换执行位置。"),
                Err(error) => ready
                    .message
                    .push_str(&format!(" 前台执行已配置；系统级后台服务未完成：{error}")),
            }
            Ok(ready)
        }
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
    let mut ready = with_route(status, Ok(route));
    let _ = crate::gui_agent_api::ensure_started(adb, device);
    if let Ok(official) = crate::studio::gui_skill_install(adb, device) {
        if official.installed {
            ready.amoo_skill_installed = true;
        }
    }
    Ok(ready)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::OfficialGuiSkillStatus;

    #[test]
    fn resolve_gui_skill_install_with_prefers_official_market_and_falls_back_to_prompt() {
        let no_prompt = resolve_gui_skill_install_with(|| {
            Ok(OfficialGuiSkillStatus {
                scope: "a".repeat(64),
                candidate_id: "skill_gui_1".into(),
                candidate_name: "操作手机屏幕".into(),
                candidate_version: "1.0.0".into(),
                installed: true,
            })
        });
        assert!(no_prompt.is_none());

        let fallback_uninstalled = resolve_gui_skill_install_with(|| {
            Ok(OfficialGuiSkillStatus {
                scope: "a".repeat(64),
                candidate_id: "skill_gui_1".into(),
                candidate_name: "操作手机屏幕".into(),
                candidate_version: "1.0.0".into(),
                installed: false,
            })
        });
        assert!(fallback_uninstalled
            .as_deref()
            .unwrap()
            .contains("stepx-background-gui/SKILL.md"));

        let fallback_err =
            resolve_gui_skill_install_with(|| anyhow::bail!("studio provider unavailable"));
        assert!(fallback_err
            .as_deref()
            .unwrap()
            .contains("stepx-background-gui/SKILL.md"));
    }
}
