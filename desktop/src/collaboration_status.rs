//! Passive, bounded observation of the phone-owned GUI queue.
//! No task starts, screenshots, foreground changes, lease heartbeats or repair.
use crate::{
    adb::{self, Adb},
    device::Device,
};
use anyhow::{ensure, Context, Result};
use base64::{engine::general_purpose::STANDARD, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    process::Command,
    time::{Duration, Instant},
};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Scope {
    pub device: String,
    /// Observation partition only. Independent phone tasks are device-owned;
    /// this must never be filled with a guessed or remembered Amoo account.
    pub account: Option<String>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneGuiPhase {
    Unknown,
    Idle,
    Queued,
    Running,
    Paused,
    NeedsInput,
    Reported,
    Completed,
    Failed,
    Cancelled,
    Cancelling,
    Interrupted,
}
impl PhoneGuiPhase {
    pub fn label(self) -> &'static str {
        match self {
            Self::Unknown => "手机任务状态待确认",
            Self::Idle => "手机暂无执行任务",
            Self::Queued => "手机任务排队中",
            Self::Running => "手机正在处理",
            Self::Paused => "手机任务已暂停",
            Self::NeedsInput => "需要你在手机处理",
            Self::Reported => "手机结果需要核对",
            Self::Completed => "手机任务已核验完成",
            Self::Failed => "手机任务未完成",
            Self::Cancelled => "手机任务已停止",
            Self::Cancelling => "手机任务正在停止",
            Self::Interrupted => "手机任务已中断",
        }
    }
    pub fn active(self) -> bool {
        matches!(
            self,
            Self::Queued | Self::Running | Self::Paused | Self::NeedsInput | Self::Cancelling
        )
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneGuiNextAction {
    OpenPhone,
    Review,
    Resume,
    ConfigureModel,
    RetryRead,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PhoneGuiExecutor {
    Neo,
    Legacy,
    Unknown,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PhoneGuiControl {
    Pause,
    Resume,
    Stop,
}
impl PhoneGuiNextAction {
    pub fn label(self) -> &'static str {
        match self {
            Self::OpenPhone => "去手机处理",
            Self::Review => "查看并核对",
            Self::Resume => "确认后继续",
            Self::ConfigureModel => "配置手机执行器",
            Self::RetryRead => "重试读取",
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct PhoneGuiSnapshot {
    pub scope: Scope,
    pub task: Option<String>,
    pub executor: PhoneGuiExecutor,
    pub phase: PhoneGuiPhase,
    pub ready: Option<bool>,
    pub model_configured: Option<bool>,
    pub verified: bool,
    pub phone_stopped: bool,
    pub next_action: Option<PhoneGuiNextAction>,
    pub observed_ms: i64,
    pub stale: bool,
    pub error: Option<String>,
}
impl PhoneGuiSnapshot {
    /// Builds only an explicit user's command; it never executes it. The UI
    /// must recheck the current selected scope before sending this request.
    pub fn control_request(&self, control: PhoneGuiControl) -> Option<Value> {
        if self.stale || self.error.is_some() {
            return None;
        }
        let task = self.task.as_deref()?;
        let allowed = match control {
            PhoneGuiControl::Pause => {
                matches!(self.phase, PhoneGuiPhase::Queued | PhoneGuiPhase::Running)
            }
            PhoneGuiControl::Resume => self.phase == PhoneGuiPhase::Paused,
            PhoneGuiControl::Stop => self.phase.active() && self.phase != PhoneGuiPhase::Cancelling,
        };
        if !allowed {
            return None;
        }
        Some(match self.executor {
            PhoneGuiExecutor::Neo => json!({"op":"agent","agentOp":match control {
                PhoneGuiControl::Pause=>"pause",PhoneGuiControl::Resume=>"resume",PhoneGuiControl::Stop=>"stop"
            },"task":task}),
            PhoneGuiExecutor::Legacy => json!({"op":match control {
                PhoneGuiControl::Pause=>"user_pause",PhoneGuiControl::Resume=>"user_resume",PhoneGuiControl::Stop=>"user_stop"
            },"task":task}),
            PhoneGuiExecutor::Unknown => return None,
        })
    }
}

/// Contains only public state. Goals, screen nodes, model configuration and
/// result bodies stay on the phone and are never copied into this cache.
pub fn parse_snapshot(value: &Value, scope: Scope, observed_ms: i64) -> Result<PhoneGuiSnapshot> {
    ensure!(value["ok"] == true, "手机只读任务状态未返回成功");
    let task = value["task"]
        .as_str()
        .filter(|v| {
            !v.is_empty()
                && v.len() <= 128
                && v.bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
        })
        .map(str::to_owned);
    let executor = match value["framework"].as_str() {
        Some("Neo Agent Touch") => PhoneGuiExecutor::Neo,
        Some("GELab-Zero Android adapter") => PhoneGuiExecutor::Legacy,
        _ => PhoneGuiExecutor::Unknown,
    };
    let verified = value["result"]["verified"].as_bool() == Some(true);
    let phone_stopped = value["result"]["phoneStopped"].as_bool() == Some(true);
    let raw = value["phase"]
        .as_str()
        .or_else(|| value["state"].as_str())
        .unwrap_or("");
    let phase = match raw {
        "idle" | "" if task.is_none() => PhoneGuiPhase::Idle,
        "queued" | "planning" => PhoneGuiPhase::Queued,
        "running" | "executing" => PhoneGuiPhase::Running,
        "paused" | "paused_planning" => PhoneGuiPhase::Paused,
        "needs_input" | "awaiting_user" | "awaiting_user_authorize" => PhoneGuiPhase::NeedsInput,
        "completed" if verified && phone_stopped => PhoneGuiPhase::Completed,
        "completed" | "reported" => PhoneGuiPhase::Reported,
        "failed" | "error" => PhoneGuiPhase::Failed,
        "cancelled" | "stopped" => PhoneGuiPhase::Cancelled,
        "cancelling" => PhoneGuiPhase::Cancelling,
        "interrupted" => PhoneGuiPhase::Interrupted,
        _ => PhoneGuiPhase::Unknown,
    };
    let phase = if phase != PhoneGuiPhase::Idle && task.is_none() {
        PhoneGuiPhase::Unknown
    } else {
        phase
    };
    let ready = if value["readinessKnown"] == false {
        None
    } else {
        value["ready"].as_bool()
    };
    let model_configured = value["modelConfigured"].as_bool();
    let next_action = match phase {
        PhoneGuiPhase::NeedsInput => Some(PhoneGuiNextAction::OpenPhone),
        PhoneGuiPhase::Paused => Some(PhoneGuiNextAction::Resume),
        PhoneGuiPhase::Reported | PhoneGuiPhase::Failed | PhoneGuiPhase::Interrupted => {
            Some(PhoneGuiNextAction::Review)
        }
        PhoneGuiPhase::Unknown => Some(PhoneGuiNextAction::RetryRead),
        PhoneGuiPhase::Idle if model_configured == Some(false) => {
            Some(PhoneGuiNextAction::ConfigureModel)
        }
        _ => None,
    };
    Ok(PhoneGuiSnapshot {
        scope,
        task,
        executor,
        phase,
        ready,
        model_configured,
        verified,
        phone_stopped,
        next_action,
        observed_ms,
        stale: false,
        error: None,
    })
}

fn bounded_read(adb: &Adb, args: &[&str], budget: Duration) -> Result<String> {
    ensure!(!budget.is_zero(), "手机状态查询已达到时间预算");
    // Deliberately do not enter the generic ADB retry/failover pipeline: a
    // passive Home poll has a hard deadline and cannot reconnect transports.
    let mut command = Command::new(&adb.path);
    command.args(args).env("ADB_MDNS_OPENSCREEN", "1");
    let out = adb::run_process(command, None, budget)?;
    ensure!(
        out.status.success() && out.stdout.len() <= 65536,
        "手机只读状态查询失败"
    );
    String::from_utf8(out.stdout).context("手机状态响应无效")
}

fn query(adb: &Adb, device: &Device, scope: Scope) -> Result<PhoneGuiSnapshot> {
    ensure!(
        !device.serial.is_empty() && device.serial == scope.device,
        "手机任务设备不匹配"
    );
    let deadline = Instant::now() + Duration::from_secs(6);
    let serial = bounded_read(
        adb,
        &["-s", &device.transport, "shell", "getprop", "ro.serialno"],
        Duration::from_secs(2),
    )?;
    ensure!(serial.trim() == scope.device, "手机连接的设备身份已变化");
    let encoded = STANDARD.encode(serde_json::to_vec(
        &json!({"op":"agent","agentOp":"status_readonly"}),
    )?);
    let remaining = deadline.saturating_duration_since(Instant::now());
    let result = bounded_read(
        adb,
        &[
            "-s",
            &device.transport,
            "shell",
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
        remaining,
    )?;
    let encoded = result
        .split("result=")
        .nth(1)
        .context("手机伴侣尚不支持只读任务状态，请更新组件")?
        .split(|c: char| !c.is_ascii_alphanumeric() && !"+/=".contains(c))
        .next()
        .unwrap_or("");
    let value: Value = serde_json::from_slice(&STANDARD.decode(encoded)?)?;
    parse_snapshot(&value, scope, chrono::Utc::now().timestamp_millis())
}

#[derive(Default)]
pub struct StatusCache {
    entry: Option<PhoneGuiSnapshot>,
    observed_at: Option<Instant>,
    attempted_at: Option<Instant>,
}
impl StatusCache {
    pub fn invalidate(&mut self) {
        *self = Self::default();
    }
    fn interval(&self) -> Duration {
        Duration::from_secs(if self.entry.as_ref().is_some_and(|s| s.error.is_some()) {
            10
        } else if self.entry.as_ref().is_some_and(|s| s.phase.active()) {
            5
        } else {
            30
        })
    }
    pub fn snapshot(&self, scope: &Scope) -> Option<PhoneGuiSnapshot> {
        let mut value = self.entry.as_ref().filter(|s| &s.scope == scope)?.clone();
        value.stale |= self
            .observed_at
            .is_none_or(|at| at.elapsed() > self.interval());
        Some(value)
    }
    /// Invoke on a background worker. The account must come from the caller's
    /// freshly confirmed account state; None remains a distinct device partition.
    pub fn refresh(
        &mut self,
        adb: &Adb,
        device: &Device,
        account: Option<&str>,
    ) -> PhoneGuiSnapshot {
        let scope = Scope {
            device: device.serial.clone(),
            account: account.filter(|v| !v.trim().is_empty()).map(str::to_owned),
        };
        if self.entry.as_ref().is_some_and(|v| v.scope != scope) {
            self.invalidate();
        }
        if self
            .attempted_at
            .is_some_and(|at| at.elapsed() < self.interval())
        {
            if let Some(snapshot) = self.snapshot(&scope) {
                return snapshot;
            }
        }
        self.attempted_at = Some(Instant::now());
        match query(adb, device, scope.clone()) {
            Ok(snapshot) => {
                self.observed_at = Some(Instant::now());
                self.entry = Some(snapshot.clone());
                snapshot
            }
            Err(_) => {
                let mut value = self.snapshot(&scope).unwrap_or(PhoneGuiSnapshot {
                    scope,
                    task: None,
                    executor: PhoneGuiExecutor::Unknown,
                    phase: PhoneGuiPhase::Unknown,
                    ready: None,
                    model_configured: None,
                    verified: false,
                    phone_stopped: false,
                    next_action: Some(PhoneGuiNextAction::RetryRead),
                    observed_ms: 0,
                    stale: true,
                    error: None,
                });
                value.stale = true;
                value.error = Some("手机任务状态未更新，未启动或恢复任何任务".into());
                value.next_action = Some(PhoneGuiNextAction::RetryRead);
                self.entry = Some(value.clone());
                value
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn scope() -> Scope {
        Scope {
            device: "phone".into(),
            account: Some("account".into()),
        }
    }
    #[test]
    fn completed_requires_both_screen_verification_and_stopped_phone() {
        for (verified, stopped, expected) in [
            (false, true, PhoneGuiPhase::Reported),
            (true, false, PhoneGuiPhase::Reported),
            (true, true, PhoneGuiPhase::Completed),
        ] {
            let state=parse_snapshot(&json!({"ok":true,"task":"task-1","phase":"completed","goal":"private","result":{"verified":verified,"phoneStopped":stopped}}),scope(),123).unwrap();
            assert_eq!(state.phase, expected);
            assert!(!serde_json::to_string(&state).unwrap().contains("private"));
        }
    }
    #[test]
    fn paused_and_input_needed_preserve_task_and_unknown_readiness() {
        let value=parse_snapshot(&json!({"ok":true,"task":"task-1","phase":"paused","ready":true,"readinessKnown":false}),scope(),123).unwrap();
        assert_eq!(value.next_action, Some(PhoneGuiNextAction::Resume));
        assert_eq!(value.task.as_deref(), Some("task-1"));
        assert_eq!(value.ready, None);
        let needs = parse_snapshot(
            &json!({"ok":true,"task":"task-1","phase":"needs_input"}),
            scope(),
            123,
        )
        .unwrap();
        assert_eq!(needs.next_action, Some(PhoneGuiNextAction::OpenPhone));
    }
    #[test]
    fn cache_never_shares_between_device_or_account_partitions() {
        let value = parse_snapshot(&json!({"ok":true,"phase":"idle"}), scope(), 123).unwrap();
        let cache = StatusCache {
            entry: Some(value),
            observed_at: Some(Instant::now()),
            attempted_at: None,
        };
        assert!(cache
            .snapshot(&Scope {
                device: "other".into(),
                account: scope().account
            })
            .is_none());
        assert!(cache
            .snapshot(&Scope {
                device: "phone".into(),
                account: None
            })
            .is_none());
    }

    #[test]
    fn explicit_resume_preserves_task_and_respects_legacy_human_control() {
        for (framework, expected) in [
            (
                "Neo Agent Touch",
                json!({"op":"agent","agentOp":"resume","task":"task-1"}),
            ),
            (
                "GELab-Zero Android adapter",
                json!({"op":"user_resume","task":"task-1"}),
            ),
        ] {
            let mut value = parse_snapshot(
                &json!({"ok":true,"task":"task-1","phase":"paused","framework":framework}),
                scope(),
                123,
            )
            .unwrap();
            assert_eq!(
                value.control_request(PhoneGuiControl::Resume),
                Some(expected)
            );
            value.stale = true;
            assert_eq!(value.control_request(PhoneGuiControl::Resume), None);
        }
        let unknown = parse_snapshot(
            &json!({"ok":true,"task":"task-1","phase":"paused"}),
            scope(),
            123,
        )
        .unwrap();
        assert_eq!(unknown.control_request(PhoneGuiControl::Resume), None);
    }

    #[cfg(unix)]
    #[test]
    fn passive_poll_is_bounded_cached_and_sends_only_readonly_status() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adb");
        let log = dir.path().join("calls");
        let encoded = STANDARD
            .encode(br#"{"ok":true,"task":"task-1","phase":"needs_input","goal":"private"}"#);
        std::fs::write(&path, format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> {}\ncase \"$*\" in\n*ro.serialno*) echo phone;;\n*content*call*) echo 'Bundle[{{result={encoded}}}]';;\n*) exit 19;;\nesac\n", adb::shell_quote(log.to_str().unwrap()))).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let adb = Adb { path };
        let device = Device {
            serial: "phone".into(),
            transport: "phone".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        let mut cache = StatusCache::default();
        let first = cache.refresh(&adb, &device, Some("account"));
        assert_eq!(first.phase, PhoneGuiPhase::NeedsInput);
        assert_eq!(first.next_action, Some(PhoneGuiNextAction::OpenPhone));
        let again = cache.refresh(&adb, &device, Some("account"));
        assert_eq!(again.observed_ms, first.observed_ms);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert_eq!(
            calls.lines().count(),
            2,
            "a cached Home render must not poll ADB"
        );
        let request = calls
            .lines()
            .find(|line| line.contains("content call"))
            .unwrap()
            .split("--arg ")
            .nth(1)
            .unwrap();
        let request: Value = serde_json::from_slice(&STANDARD.decode(request).unwrap()).unwrap();
        assert_eq!(request, json!({"op":"agent","agentOp":"status_readonly"}));
        assert!(!serde_json::to_string(&first).unwrap().contains("private"));
        let next = cache.refresh(&adb, &device, None);
        assert_eq!(next.scope.account, None);
        assert_eq!(std::fs::read_to_string(log).unwrap().lines().count(), 4);
    }
}
