//! Native phone GUI tasks. The desktop checks real screen/keyguard state and
//! invokes only gui_agent; it never dismisses keyguard or synthesizes input.
use crate::{
    adb::Adb,
    amoo::{AgentBus, LocalInvokeEnd},
    device::Device,
};
use anyhow::{ensure, Result};
use serde::Serialize;
use serde_json::Value;
use std::{
    sync::atomic::{AtomicBool, Ordering},
    time::Instant,
};

#[derive(Clone, Debug, Serialize)]
pub struct GuiStatus {
    pub supported: bool,
    pub locked: Option<bool>,
    pub interactive: Option<bool>,
    pub message: String,
    pub native_cancel_supported: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize)]
pub enum GuiRunState {
    Completed,
    Failed,
    Submitted,
    WaitingForUnlock,
    ScreenOff,
    Unavailable,
    CancelledBeforeStart,
    WaitingCancelled,
    TimedOut,
    Disconnected,
    Unknown,
}

#[derive(Clone, Debug, Serialize)]
pub struct GuiRunResult {
    pub state: GuiRunState,
    pub message: String,
    pub invocation_id: String,
    pub raw_result: Option<Value>,
    pub elapsed_ms: u64,
}

pub(crate) fn validate_request(app_name: &str, task: &str) -> Result<()> {
    ensure!(
        !app_name.trim().is_empty()
            && app_name.chars().count() <= 120
            && !app_name.chars().any(char::is_control),
        "请选择要操作的手机应用"
    );
    ensure!(
        !task.trim().is_empty()
            && task.chars().count() <= 12000
            && !task
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t'),
        "请填写操作目标，最多 12000 字"
    );
    Ok(())
}

fn bool_field(dump: &str, names: &[&str]) -> Option<bool> {
    let mut found = Vec::new();
    for token in dump.split(|c: char| c.is_whitespace() || matches!(c, ',' | '{' | '}' | ';')) {
        if let Some((key, value)) = token.split_once('=') {
            if names.contains(&key) {
                match value {
                    "true" => found.push(true),
                    "false" => found.push(false),
                    _ => {}
                }
            }
        }
    }
    // Conflicting display/user states cannot establish that the device is unlocked.
    if found.contains(&true) {
        Some(true)
    } else if found.contains(&false) {
        Some(false)
    } else {
        None
    }
}

fn screen_state(policy: &str, power: &str) -> (Option<bool>, Option<bool>) {
    let explicit = bool_field(
        policy,
        &["mKeyguardShowing", "keyguardShowing", "isKeyguardShowing"],
    );
    let locked = explicit.or_else(|| {
        // Generic `showing` occurs elsewhere in dumpsys: only accept it inside
        // KeyguardServiceDelegate's own indented section.
        let mut section = String::new();
        let mut indentation = None;
        for line in policy.lines() {
            if line.contains("KeyguardServiceDelegate") {
                indentation = Some(line.len() - line.trim_start().len());
                continue;
            }
            if let Some(start) = indentation {
                if !line.trim().is_empty() && line.len() - line.trim_start().len() <= start {
                    break;
                }
                section.push_str(line);
                section.push('\n');
            }
        }
        bool_field(&section, &["showing"])
    });
    let interactive = power
        .lines()
        .find_map(
            |line| match line.trim().strip_prefix("mWakefulness=")?.trim() {
                "Awake" | "Dreaming" => Some(true),
                "Asleep" | "Dozing" => Some(false),
                _ => None,
            },
        )
        .or_else(|| bool_field(power, &["mInteractive"]));
    (locked, interactive)
}

pub(crate) fn read_screen(adb: &Adb, device: &Device) -> Result<(Option<bool>, Option<bool>)> {
    let combined = adb.script(
        &device.transport,
        "dumpsys window policy; printf '\\n===STEPX_POWER===\\n'; dumpsys power --brief",
    )?;
    let (policy, power) = combined
        .split_once("\n===STEPX_POWER===\n")
        .or_else(|| combined.split_once("===STEPX_POWER==="))
        .unwrap_or((&combined, ""));
    Ok(screen_state(policy, power))
}

fn catalog_has_gui(value: &Value, depth: usize) -> bool {
    if depth > 12 {
        return false;
    }
    match value {
        Value::Object(map) => {
            map.get("gui_agent").is_some_and(|v| v.is_object())
                || ["name", "method", "toolName"]
                    .iter()
                    .any(|key| map.get(*key).and_then(Value::as_str) == Some("gui_agent"))
                || map.iter().any(|(key, value)| {
                    !["description", "message", "error"].contains(&key.as_str())
                        && catalog_has_gui(value, depth + 1)
                })
        }
        Value::Array(items) => items
            .iter()
            .any(|item| item.as_str() == Some("gui_agent") || catalog_has_gui(item, depth + 1)),
        Value::String(raw) => serde_json::from_str::<Value>(raw)
            .ok()
            .is_some_and(|value| catalog_has_gui(&value, depth + 1)),
        _ => false,
    }
}

/// Safe to call while idle: does not wake the screen, acquire a task lease,
/// launch an Activity, grant a permission, or modify a phone setting.
pub fn inspect(adb: &Adb, device: &Device) -> Result<GuiStatus> {
    device.verify(adb)?;
    let (locked, interactive) = read_screen(adb, device)?;
    let probe = AgentBus::observe(adb, device).and_then(|mut bus| bus.local_gui_catalog());
    let supported = probe.as_ref().is_ok_and(|reply| {
        reply["ok"] == true && reply["payload"]["ok"] != false && catalog_has_gui(reply, 0)
    });
    let message = if !supported {
        "手机尚未确认提供屏幕操作能力；请刷新能力状态"
    } else if locked != Some(false) {
        "请先在手机上解锁，再开始屏幕操作"
    } else if interactive != Some(true) {
        "请点亮手机屏幕，再开始屏幕操作"
    } else {
        "手机已解锁，已发现屏幕操作接口；是否可执行以手机实际返回为准"
    };
    Ok(GuiStatus {
        supported,
        locked,
        interactive,
        message: message.into(),
        native_cancel_supported: false,
    })
}

fn result_state(reply: &Value) -> GuiRunState {
    if reply["ok"] == false || reply["payload"]["ok"] == false {
        let error = native_error(reply);
        if error.contains("GUI 自动化功能暂未开放")
            || [
                "ABILITY_NOT_FOUND",
                "FEATURE_DISABLED",
                "NOT_IMPLEMENTED",
                "METHOD_UNAVAILABLE",
            ]
            .iter()
            .any(|code| {
                reply["error"]["code"] == *code || reply["payload"]["error"]["code"] == *code
            })
        {
            return GuiRunState::Unavailable;
        }
        return GuiRunState::Failed;
    }
    let payload = &reply["payload"];
    let content = &payload["content"];
    for body in [content, payload, reply] {
        for key in ["status", "state", "taskStatus"] {
            match body[key]
                .as_str()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str()
            {
                "completed" | "complete" | "succeeded" | "success" | "finished" | "done" => {
                    return GuiRunState::Completed
                }
                "failed" | "failure" | "error" | "cancelled" | "canceled" => {
                    return GuiRunState::Failed
                }
                "running" | "pending" | "queued" | "accepted" | "submitted" | "in_progress" => {
                    return GuiRunState::Submitted
                }
                "locked" | "waiting_for_unlock" | "waiting_unlock" => {
                    return GuiRunState::WaitingForUnlock
                }
                _ => {}
            }
        }
        if body["success"] == false || body["isError"] == true {
            return GuiRunState::Failed;
        }
    }
    // A task ID can be only an acknowledgement. A transport `ok` is never
    // enough to call the user task complete.
    if !content["taskId"].is_null() || !payload["taskId"].is_null() {
        return GuiRunState::Submitted;
    }
    if content["success"] == true || content["done"] == true {
        return GuiRunState::Completed;
    }
    GuiRunState::Unknown
}

fn native_error(reply: &Value) -> String {
    [
        &reply["error"]["message"],
        &reply["payload"]["error"]["message"],
    ]
    .into_iter()
    .find_map(Value::as_str)
    .unwrap_or("")
    .chars()
    .filter(|c| !c.is_control() || *c == '\n' || *c == '\t')
    .take(1000)
    .collect()
}

fn outcome_message(state: GuiRunState) -> &'static str {
    match state {
        GuiRunState::Completed => "手机原生执行器已报告完成，请查看执行结果",
        GuiRunState::Failed => "手机原生执行器报告失败，请查看执行记录；未自动重试",
        GuiRunState::Submitted => "手机已接受任务，尚未确认执行完成；请在手机上查看进度",
        GuiRunState::WaitingForUnlock => "手机已锁定或锁屏状态未确认，请手动解锁后再开始",
        GuiRunState::ScreenOff => "手机屏幕未点亮，尚未发送操作；点亮并解锁后再开始",
        GuiRunState::Unavailable => "尚未确认手机提供 gui_agent 能力，没有发送操作",
        GuiRunState::CancelledBeforeStart => "已取消，本次操作尚未发送到手机",
        GuiRunState::WaitingCancelled => {
            "已停止桌面等待；手机可能仍在执行，请在手机上核对，勿直接重复提交"
        }
        GuiRunState::TimedOut => {
            "等待超过 10 分钟，未确认任务完成或停止；请在手机上核对，勿直接重复提交"
        }
        GuiRunState::Disconnected => {
            "连接中断，无法确认任务是否已执行；请在手机上核对，勿直接重复提交"
        }
        GuiRunState::Unknown => "手机已返回结果，但未提供明确终态；请查看原始结果和手机界面",
    }
}

/// The caller owns the worker thread and cancellation token. Cancellation only
/// stops desktop waiting: this firmware exposes no verified native abort route.
pub fn run(
    adb: &Adb,
    device: &Device,
    app_name: &str,
    task: &str,
    cancel: &AtomicBool,
) -> Result<GuiRunResult> {
    validate_request(app_name, task)?;
    let started = Instant::now();
    let finish = |state, invocation_id, raw_result| GuiRunResult {
        state,
        message: outcome_message(state).into(),
        invocation_id,
        raw_result,
        elapsed_ms: started.elapsed().as_millis().min(u64::MAX as u128) as u64,
    };
    if cancel.load(Ordering::Relaxed) {
        return Ok(finish(
            GuiRunState::CancelledBeforeStart,
            String::new(),
            None,
        ));
    }
    let status = inspect(adb, device)?;
    let blocked = if !status.supported {
        Some(GuiRunState::Unavailable)
    } else if status.locked != Some(false) {
        Some(GuiRunState::WaitingForUnlock)
    } else if status.interactive != Some(true) {
        Some(GuiRunState::ScreenOff)
    } else {
        None
    };
    if let Some(state) = blocked {
        return Ok(finish(state, String::new(), None));
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(finish(
            GuiRunState::CancelledBeforeStart,
            String::new(),
            None,
        ));
    }
    let mut bus = AgentBus::connect(adb, device)?;
    // A capability probe/connect may take time: refresh the lock state directly
    // before dispatch. Android's native executor also enforces its own policy.
    let (locked, interactive) = read_screen(adb, device)?;
    if locked != Some(false) {
        return Ok(finish(GuiRunState::WaitingForUnlock, String::new(), None));
    }
    if interactive != Some(true) {
        return Ok(finish(GuiRunState::ScreenOff, String::new(), None));
    }
    if cancel.load(Ordering::Relaxed) {
        return Ok(finish(
            GuiRunState::CancelledBeforeStart,
            String::new(),
            None,
        ));
    }
    let receipt = bus.invoke_local_gui(app_name, task, cancel)?;
    let (state, raw) = match receipt.end {
        LocalInvokeEnd::Reply(reply) => (result_state(&reply), Some(reply)),
        LocalInvokeEnd::WaitCancelled => (GuiRunState::WaitingCancelled, None),
        LocalInvokeEnd::TimedOut => (GuiRunState::TimedOut, None),
        LocalInvokeEnd::Disconnected => (GuiRunState::Disconnected, None),
    };
    let mut result = finish(state, receipt.id, raw);
    if matches!(state, GuiRunState::Unavailable | GuiRunState::Failed) {
        if let Some(raw) = result.raw_result.as_ref() {
            let reason = native_error(raw);
            if !reason.is_empty() {
                result.message = format!("手机返回：{reason}。本次未执行成功，未自动重试");
            }
        }
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn real_keyguard_state_never_assumes_unlocked_from_missing_or_occluded() {
        assert_eq!(
            screen_state(
                "KeyguardServiceDelegate\n  showing=true\n  occluded=true\nOther\n  showing=false",
                "mWakefulness=Awake"
            ),
            (Some(true), Some(true))
        );
        assert_eq!(
            screen_state(
                "KeyguardServiceDelegate\n  showing=false\n  secure=true",
                "mWakefulness=Dozing"
            ),
            (Some(false), Some(false))
        );
        assert_eq!(
            screen_state("Other\n  showing=false", "random=true"),
            (None, None)
        );
        assert_eq!(
            screen_state(
                "mKeyguardShowing=false\nmKeyguardShowing=true",
                "mWakefulness=Awake"
            ),
            (Some(true), Some(true))
        );
    }

    #[test]
    fn capability_requires_actual_method_not_prose() {
        assert!(catalog_has_gui(
            &json!({"payload":{"tools":[{"name":"gui_agent"}]}}),
            0
        ));
        assert!(catalog_has_gui(
            &json!({"content":"{\"abilities\":[{\"method\":\"gui_agent\"}]}"}),
            0
        ));
        assert!(!catalog_has_gui(
            &json!({"message":"gui_agent", "description":"gui_agent", "tools":[{"name":"exec"}]}),
            0
        ));
    }

    #[test]
    fn transport_acknowledgement_is_not_task_completion() {
        assert_eq!(
            result_state(&json!({"ok":true,"payload":{"ok":true}})),
            GuiRunState::Unknown
        );
        assert_eq!(
            result_state(&json!({"ok":true,"payload":{"content":{"taskId":"x"}}})),
            GuiRunState::Submitted
        );
        assert_eq!(
            result_state(&json!({"ok":true,"payload":{"content":{"status":"running"}}})),
            GuiRunState::Submitted
        );
        assert_eq!(
            result_state(&json!({"ok":true,"payload":{"content":{"status":"completed"}}})),
            GuiRunState::Completed
        );
        assert_eq!(
            result_state(
                &json!({"ok":true,"payload":{"ok":false,"content":{"status":"completed"}}})
            ),
            GuiRunState::Failed
        );
        assert_eq!(
            result_state(&json!({"ok":false,"error":{"code":"ABILITY_NOT_FOUND"}})),
            GuiRunState::Unavailable
        );
        assert_eq!(
            result_state(
                &json!({"ok":false,"error":{"code":"UNKNOWN","message":"GUI 自动化功能暂未开放"}})
            ),
            GuiRunState::Unavailable
        );
    }
}
