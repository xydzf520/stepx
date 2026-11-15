//! Phone-owned GUI execution endpoint. Only the paired ADB shell can call it.
use crate::{adb::Adb, device::Device};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::Value;

pub fn call(adb: &Adb, device: &Device, request: Value) -> Result<Value> {
    let transport = device.resolve_active_transport(adb)?;
    let encoded = STANDARD.encode(serde_json::to_vec(&request)?);
    ensure!(encoded.len() <= 32768, "GUI 请求过长");
    let result = adb.shell(
        &transport,
        &[
            "content",
            "call",
            "--user",
            "0",
            "--uri",
            "content://com.stepx.companion.app.gui",
            "--method",
            "request",
            "--arg",
            &encoded,
        ],
    )?;
    let raw = result
        .split("result=")
        .nth(1)
        .context("手机没有返回 GUI 服务响应，请安装新版 StepX伴侣")?
        .split(|c: char| !c.is_ascii_alphanumeric() && !"+/=".contains(c))
        .next()
        .unwrap_or("");
    let result: Value = serde_json::from_slice(&STANDARD.decode(raw)?)?;
    ensure!(
        result["ok"] == true,
        "{}",
        result["error"].as_str().unwrap_or("手机操作未完成")
    );
    Ok(result)
}

pub fn enable(adb: &Adb, device: &Device) -> Result<()> {
    let transport = device.resolve_active_transport(adb)?;
    let component = "com.stepx.companion.app/com.stepx.companion.app.GuiAccessibilityService";
    let old = adb.shell(
        &transport,
        &[
            "settings",
            "--user",
            "0",
            "get",
            "secure",
            "enabled_accessibility_services",
        ],
    )?;
    let mut services: Vec<&str> = old
        .trim()
        .split(':')
        .filter(|s| !s.is_empty() && *s != "null")
        .collect();
    if !services.contains(&component) {
        services.push(component);
        adb.shell(
            &transport,
            &[
                "settings",
                "--user",
                "0",
                "put",
                "secure",
                "enabled_accessibility_services",
                &services.join(":"),
            ],
        )?;
    }
    adb.shell(
        &transport,
        &[
            "settings",
            "--user",
            "0",
            "put",
            "secure",
            "accessibility_enabled",
            "1",
        ],
    )?;
    for _ in 0..20 {
        if call(adb, device, serde_json::json!({"op":"status"})).is_ok() {
            return Ok(());
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    anyhow::bail!("看屏服务尚未连接，请在手机的无障碍设置启用 StepX伴侣")
}

pub fn observe(adb: &Adb, device: &Device, task: &str) -> Result<(Value, Vec<u8>)> {
    let state = call(adb, device, serde_json::json!({"op":"observe","task":task}))?;
    let remote = state["screenshot"].as_str().context("没有截图路径")?;
    ensure!(
        remote == "/storage/emulated/0/Android/data/com.stepx.companion.app/files/gui-screen.png"
            || remote == "/sdcard/Android/data/com.stepx.companion.app/files/gui-screen.png",
        "截图路径异常"
    );
    let transport = device.resolve_active_transport(adb)?;
    let folder = tempfile::tempdir()?;
    let path = folder.path().join("screen.png");
    adb.run(&[
        "-s",
        &transport,
        "pull",
        remote,
        path.to_str().context("本地路径无效")?,
    ])?;
    let png = std::fs::read(path)?;
    ensure!(
        png.len() <= 16 * 1024 * 1024 && png.starts_with(b"\x89PNG"),
        "截图无效"
    );
    Ok((state, png))
}
