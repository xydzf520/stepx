//! Desktop observer/controller. Model requests and execution live entirely on phone.
use crate::{
    adb::Adb,
    amoo_gui::{GuiRunResult, GuiRunState},
    amoo_screen::ScreenEvent,
    companion_gui,
    device::Device,
    gui_agent::Control,
};
use anyhow::{ensure, Result};
use serde_json::json;
use std::{io::Cursor, process::Command};
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
fn preview(adb: &Adb, device: &Device, value: &serde_json::Value) -> Result<Vec<u8>> {
    if value["executionMode"] != "background" {
        return crate::amoo_screen::capture_png(adb, device);
    }
    ensure!(
        value["previewTask"]
            .as_str()
            .is_some_and(|id| !id.is_empty() && value["task"].as_str() == Some(id)),
        "虚拟屏尚无预览"
    );
    let mut command = Command::new(&adb.path);
    command.args([
        "-s",
        &device.transport,
        "exec-out",
        "cat",
        "/sdcard/Android/data/com.stepx.companion.app/files/mdi-preview.jpg",
    ]);
    let output = crate::adb::run_process(command, None, Duration::from_secs(6))?;
    ensure!(
        output.status.success() && output.stdout.len() <= 320000,
        "虚拟屏预览读取失败"
    );
    let current = companion_gui::call(
        adb,
        device,
        json!({"op":"agent","agentOp":"status","task":value["task"]}),
    )?;
    ensure!(current["task"] == value["task"], "虚拟屏任务已变化");
    let mut png = Cursor::new(Vec::new());
    image::load_from_memory(&output.stdout)?.write_to(&mut png, image::ImageFormat::Png)?;
    Ok(png.into_inner())
}

#[cfg(all(test, unix))]
mod native_readiness_tests {
    use super::*;

    #[test]
    fn unconfigured_phone_executor_never_starts_a_task() {
        let fixture = crate::native_mcp::test_support::Fixture::new(
            vec![json!({"name":"stepx_gui_agent","callingMethod":"LOCAL","callType":"MCP",
                "inputSchema":{"required":["op"],"properties":{"op":{"type":"string"}}}})],
            Some(json!({"ok":true,"ready":false,"modelConfigured":false})),
        );
        let result = run(&fixture.adb("exit 19"), &fixture.device(), "只查看设置", None,
            &Default::default(), |_| {}).unwrap();
        assert_eq!(result.state, GuiRunState::Unavailable);
        let calls = fixture.calls.lock().unwrap();
        assert_eq!(calls.len(), 1);
        let args: serde_json::Value = serde_json::from_str(
            calls[0]["payload"]["params"]["invokeParamsJson"].as_str().unwrap()).unwrap();
        assert_eq!(args["params"]["op"], "status");
    }
}

pub fn run(
    adb: &Adb,
    device: &Device,
    goal: &str,
    _id: Option<&str>,
    control: &Control,
    mut emit: impl FnMut(ScreenEvent),
) -> Result<GuiRunResult> {
    let mut native = crate::native_mcp::Client::connect(adb, device)?;
    let readiness = native.call_cancellable("stepx_gui_agent", json!({"op":"status"}), &control.stop)?;
    if readiness["ready"] == false || readiness["modelConfigured"] == false {
        return Ok(GuiRunResult {
            state: GuiRunState::Unavailable,
            message: "伴侣 MCP 已连接，但手机执行器尚未就绪；请在手机伴侣检查所选执行器与模型配置。尚未发起操作。".into(),
            invocation_id: String::new(), raw_result: Some(readiness), elapsed_ms: 0,
        });
    }
    let start = native.call_cancellable("stepx_gui_agent", json!({"op":"start","goal":goal}), &control.stop)?;
    let id = start["task"].as_str().unwrap_or("").to_owned();
    ensure!(!id.is_empty(), "手机没有返回任务标识");
    let started = Instant::now();
    let mut paused = false;
    let mut stopping = false;
    let mut progress = String::new();
    let mut step_number = 0usize;
    loop {
        if control.stop.load(Ordering::Relaxed) && !stopping {
            companion_gui::call(
                adb,
                device,
                json!({"op":"agent","agentOp":"stop","task":id}),
            )?;
            stopping = true;
            emit(ScreenEvent::Status("正在停止手机任务并释放资源…".into()));
        }
        if !stopping && control.pause.load(Ordering::Relaxed) != paused {
            paused = control.pause.load(Ordering::Relaxed);
            companion_gui::call(
                adb,
                device,
                json!({"op":"agent","agentOp":if paused{"pause"}else{"resume"},"task":id}),
            )?;
        }
        let value = match native.call("stepx_gui_agent", json!({"op":"status","task":id})) {
            Ok(v) => v,
            Err(_) => {
                return Ok(GuiRunResult {
                    state: GuiRunState::Disconnected,
                    message: "desktop 连接中断；任务继续由手机执行，可在伴侣查看或停止。".into(),
                    invocation_id: id,
                    raw_result: None,
                    elapsed_ms: started.elapsed().as_millis() as u64,
                })
            }
        };
        ensure!(
            value["task"].as_str() == Some(id.as_str()),
            "手机任务已经改变，请重新查看当前任务"
        );
        let next = value["progress"].as_str().unwrap_or("");
        if next != progress {
            progress = next.into();
            emit(ScreenEvent::Status(progress.clone()));
            if !progress.trim().is_empty() {
                step_number += 1;
                emit(ScreenEvent::Step {
                    number: step_number,
                    description: progress.clone(),
                });
                if let Ok(png) = preview(adb, device, &value) {
                    emit(ScreenEvent::Screenshot {
                        number: step_number,
                        png,
                    });
                }
            }
        }
        if !matches!(
            value["phase"].as_str(),
            Some("running" | "queued" | "paused" | "cancelling")
        ) {
            if let Ok(png) = preview(adb, device, &value) {
                step_number = step_number.max(1);
                emit(ScreenEvent::Screenshot {
                    number: step_number,
                    png,
                });
            }
            let completed = value["phase"] == "completed"
                && value["result"]["verified"] == true
                && value["result"]["phoneStopped"] == true;
            let state = if completed {
                GuiRunState::Completed
            } else if value["phase"] == "cancelled" {
                GuiRunState::WaitingCancelled
            } else if value["result"]["phoneStopped"] != true {
                GuiRunState::Unknown
            } else {
                GuiRunState::Failed
            };
            return Ok(GuiRunResult {
                state,
                message: progress,
                invocation_id: id,
                elapsed_ms: started.elapsed().as_millis() as u64,
                raw_result: Some(value),
            });
        }
        if started.elapsed() > Duration::from_secs(1250) {
            return Ok(GuiRunResult {
                state: GuiRunState::TimedOut,
                message: "desktop 等待超时，请在手机查看任务状态。".into(),
                invocation_id: id,
                elapsed_ms: started.elapsed().as_millis() as u64,
                raw_result: Some(value),
            });
        }
        for _ in 0..20 {
            if (!stopping && control.stop.load(Ordering::Relaxed))
                || (!stopping && control.pause.load(Ordering::Relaxed) != paused)
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
