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
use std::{
    sync::atomic::Ordering,
    time::{Duration, Instant},
};
pub fn run(
    adb: &Adb,
    device: &Device,
    goal: &str,
    _id: Option<&str>,
    control: &Control,
    mut emit: impl FnMut(ScreenEvent),
) -> Result<GuiRunResult> {
    let start = companion_gui::call(
        adb,
        device,
        json!({"op":"agent","agentOp":"start","goal":goal}),
    )?;
    let id = start["task"].as_str().unwrap_or("").to_owned();
    ensure!(!id.is_empty(), "手机没有返回任务标识");
    let started = Instant::now();
    let mut paused = false;
    let mut progress = String::new();
    let mut step_number = 0usize;
    loop {
        if control.stop.load(Ordering::Relaxed) {
            let _ = companion_gui::call(
                adb,
                device,
                json!({"op":"agent","agentOp":"stop","task":id}),
            );
            return Ok(GuiRunResult {
                state: GuiRunState::WaitingCancelled,
                message: "已停止手机操作。".into(),
                invocation_id: id,
                raw_result: None,
                elapsed_ms: started.elapsed().as_millis() as u64,
            });
        }
        if control.pause.load(Ordering::Relaxed) != paused {
            paused = control.pause.load(Ordering::Relaxed);
            let _ = companion_gui::call(
                adb,
                device,
                json!({"op":"agent","agentOp":if paused{"pause"}else{"resume"},"task":id}),
            );
        }
        let value = match companion_gui::call(
            adb,
            device,
            json!({"op":"agent","agentOp":"status","task":id}),
        ) {
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
                if let Ok(png) = crate::amoo_screen::capture_png(adb, device) {
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
            if let Ok(png) = crate::amoo_screen::capture_png(adb, device) {
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
            if control.stop.load(Ordering::Relaxed)
                || control.pause.load(Ordering::Relaxed) != paused
            {
                break;
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }
}
