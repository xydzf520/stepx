//! Low-frequency Google maintenance, isolated from message/call synchronization.
//!
//! Python owns the durable repair budget and verifies current device/install
//! identity. This coordinator owns scheduling only: no device IO on the UI thread.
use crate::{adb::Adb, device::Device};
use serde_json::Value;
use std::{
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum RecoveryPhase {
    #[default]
    Idle,
    Checking,
    Waiting,
    Repairing,
    Healthy,
    Retrying,
    NeedsAttention,
    Cancelled,
    Blocked,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RecoverySnapshot {
    pub phase: RecoveryPhase,
    pub message: String,
    pub retry_after_seconds: u64,
    pub in_flight: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Identity {
    serial: String,
    transport: String,
}

struct State {
    identity: Option<Identity>,
    epoch: u64,
    flight: Option<u64>,
    authorized: bool,
    busy: bool,
    next_check: Instant,
    failures: u32,
    snapshot: RecoverySnapshot,
}
impl Default for State {
    fn default() -> Self {
        Self {
            identity: None,
            epoch: 0,
            flight: None,
            authorized: false,
            busy: false,
            next_check: Instant::now(),
            failures: 0,
            snapshot: RecoverySnapshot::default(),
        }
    }
}

#[derive(Clone, Default)]
pub struct RecoveryCoordinator {
    inner: Arc<Mutex<State>>,
}

impl RecoveryCoordinator {
    pub fn tick(
        &self,
        device: &Device,
        adb: &Adb,
        authorized: bool,
        communication_busy: bool,
    ) -> RecoverySnapshot {
        let identity = Identity {
            serial: device.serial.clone(),
            transport: device.transport.clone(),
        };
        let ticket = {
            let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if state.identity.as_ref() != Some(&identity) || state.authorized != authorized {
                state.epoch = state.epoch.wrapping_add(1);
                state.identity = Some(identity);
                state.authorized = authorized;
                state.failures = 0;
                state.next_check = Instant::now();
                state.snapshot = RecoverySnapshot::default();
            }
            state.busy = communication_busy;
            if !authorized {
                state.snapshot.phase = RecoveryPhase::Cancelled;
                state.snapshot.message = "Google 自动维护已暂停".into();
                None
            } else if state.flight.is_some() {
                None
            } else if communication_busy {
                if Instant::now() >= state.next_check {
                    state.snapshot.phase = RecoveryPhase::Waiting;
                    state.snapshot.message = "等待通话或当前操作结束后检查 Google 服务".into();
                }
                None
            } else if device.serial.trim().is_empty() || device.transport.trim().is_empty() {
                state.snapshot.phase = RecoveryPhase::Blocked;
                state.snapshot.message = "等待已授权的手机连接".into();
                None
            } else if Instant::now() >= state.next_check {
                let ticket = state.epoch;
                state.flight = Some(ticket);
                state.snapshot.phase = RecoveryPhase::Checking;
                state.snapshot.message = "正在后台检查 Google 服务".into();
                Some(ticket)
            } else {
                None
            }
        };
        if let Some(ticket) = ticket {
            let worker = self.clone();
            let device = device.clone();
            let adb = adb.clone();
            let spawn = std::thread::Builder::new()
                .name("google-maintenance".into())
                .spawn(move || {
                    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                        worker.run(ticket, &device, &adb)
                    }))
                    .unwrap_or_else(|_| Err(anyhow::anyhow!("Google maintenance worker stopped")));
                    worker.finish(ticket, result);
                });
            if let Err(error) = spawn {
                self.finish(ticket, Err(error.into()));
            }
        }
        self.snapshot(&device.serial)
    }

    /// A check can be requested immediately, but it never resets the durable
    /// mutation budget or bypasses phone identity, active calls, or singleflight.
    pub fn request_now(&self, serial: &str) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state
            .identity
            .as_ref()
            .is_some_and(|id| id.serial == serial)
        {
            state.next_check = Instant::now();
        }
    }
    pub fn retry_now(&self, serial: &str) {
        self.request_now(serial);
    }

    pub fn cancel(&self) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.epoch = state.epoch.wrapping_add(1);
        state.authorized = false;
        state.snapshot.phase = RecoveryPhase::Cancelled;
        state.snapshot.message = "Google 自动维护已暂停".into();
        // A step already running is bounded and may have a durable reservation.
        // Do not kill it half-way through device mutation or overlap a new step.
    }
    pub fn disconnect(&self) {
        self.cancel();
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.identity = None;
    }
    pub fn snapshot(&self, serial: &str) -> RecoverySnapshot {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if !state
            .identity
            .as_ref()
            .is_some_and(|id| id.serial == serial)
        {
            return RecoverySnapshot::default();
        }
        let mut snapshot = state.snapshot.clone();
        snapshot.in_flight = state.flight.is_some();
        snapshot.retry_after_seconds = state
            .next_check
            .saturating_duration_since(Instant::now())
            .as_secs();
        snapshot
    }
    fn can_continue(&self, ticket: u64) -> bool {
        let state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        state.epoch == ticket && state.authorized && !state.busy
    }
    fn run(&self, ticket: u64, device: &Device, adb: &Adb) -> anyhow::Result<Value> {
        if !self.can_continue(ticket) {
            return Ok(deferred());
        }
        let result = run_action(device, adb, "auto-status")?;
        if result["repair_allowed"].as_bool() != Some(true) {
            return Ok(result);
        }
        {
            let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
            if state.epoch != ticket || !state.authorized || state.busy {
                return Ok(deferred());
            }
            state.snapshot.phase = RecoveryPhase::Repairing;
            state.snapshot.message = "Google 更新后正在恢复，随后重新校验".into();
        }
        // The helper rechecks ownership, signatures, active calls and the
        // persistent budget under the same nonblocking lock as manual actions.
        if !self.can_continue(ticket) {
            return Ok(deferred());
        }
        run_action(device, adb, "repair-autonomous")
    }
    fn finish(&self, ticket: u64, result: anyhow::Result<Value>) {
        let mut state = self.inner.lock().unwrap_or_else(|e| e.into_inner());
        if state.flight == Some(ticket) {
            state.flight = None;
        }
        if state.epoch != ticket {
            return;
        }
        let (snapshot, delay) = match result {
            Ok(value) => {
                state.failures = 0;
                let phase = match value["state"].as_str() {
                    Some("healthy") => RecoveryPhase::Healthy,
                    Some("waiting" | "repair_needed") => RecoveryPhase::Waiting,
                    Some("retrying") => RecoveryPhase::Retrying,
                    Some("needs_attention") => RecoveryPhase::NeedsAttention,
                    Some("blocked") => RecoveryPhase::Blocked,
                    _ => RecoveryPhase::NeedsAttention,
                };
                let delay = value["retry_after_seconds"]
                    .as_u64()
                    .unwrap_or(300)
                    .clamp(5, 1800);
                let message = value["message"]
                    .as_str()
                    .unwrap_or("Google 状态暂不完整，请查看检测详情")
                    .chars()
                    .take(240)
                    .collect();
                (
                    RecoverySnapshot {
                        phase,
                        message,
                        retry_after_seconds: delay,
                        in_flight: false,
                    },
                    delay,
                )
            }
            Err(_) => {
                state.failures = state.failures.saturating_add(1);
                let delay = (30u64.saturating_mul(1u64 << state.failures.saturating_sub(1).min(6)))
                    .min(1800);
                // Raw bridge stderr can contain host paths or device identifiers.
                (
                    RecoverySnapshot {
                        phase: RecoveryPhase::Retrying,
                        message: "Google 检测未完成，连接恢复后自动重试；未因读取失败修改手机"
                            .into(),
                        retry_after_seconds: delay,
                        in_flight: false,
                    },
                    delay,
                )
            }
        };
        state.snapshot = snapshot;
        state.next_check = Instant::now() + Duration::from_secs(delay);
    }
}
fn deferred() -> Value {
    serde_json::json!({"state":"waiting", "message":"等待通话或当前操作结束后继续维护", "retry_after_seconds":30})
}
fn run_action(device: &Device, adb: &Adb, action: &str) -> anyhow::Result<Value> {
    let raw = super::run_script(device, adb, "manage.py", &[action])?;
    let value: Value = serde_json::from_str(&raw)?;
    anyhow::ensure!(
        value["state"].is_string()
            && value["repair_allowed"].is_boolean()
            && value["retry_after_seconds"].is_u64(),
        "Google maintenance response incomplete"
    );
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn coordinator() -> RecoveryCoordinator {
        let worker = RecoveryCoordinator::default();
        {
            let mut s = worker.inner.lock().unwrap();
            s.identity = Some(Identity {
                serial: "phone".into(),
                transport: "wifi".into(),
            });
            s.authorized = true;
            s.epoch = 7;
            s.flight = Some(7);
        }
        worker
    }
    #[test]
    fn cancellation_and_transport_change_discard_stale_completion() {
        let c = coordinator();
        c.cancel();
        c.finish(
            7,
            Ok(serde_json::json!({"state":"healthy","retry_after_seconds":300})),
        );
        assert_eq!(c.snapshot("phone").phase, RecoveryPhase::Cancelled);
        assert!(!c.snapshot("phone").in_flight);
        let c = coordinator();
        c.inner.lock().unwrap().epoch += 1;
        c.finish(
            7,
            Ok(serde_json::json!({"state":"healthy","retry_after_seconds":300})),
        );
        assert_ne!(c.snapshot("phone").phase, RecoveryPhase::Healthy);
    }
    #[test]
    fn active_call_prevents_next_mutating_step() {
        let c = coordinator();
        assert!(c.can_continue(7));
        c.inner.lock().unwrap().busy = true;
        assert!(!c.can_continue(7));
        c.inner.lock().unwrap().busy = false;
        c.disconnect();
        assert!(!c.can_continue(7));
    }
    #[test]
    fn brief_busy_period_preserves_healthy_result_until_next_check() {
        let c = coordinator();
        c.finish(
            7,
            Ok(serde_json::json!({"state":"healthy", "retry_after_seconds":300})),
        );
        let phone = Device {
            serial: "phone".into(),
            transport: "wifi".into(),
            ..Default::default()
        };
        let adb = Adb {
            path: "must-not-run".into(),
        };
        assert_eq!(
            c.tick(&phone, &adb, true, true).phase,
            RecoveryPhase::Healthy
        );
        assert_eq!(
            c.tick(&phone, &adb, true, false).phase,
            RecoveryPhase::Healthy
        );
    }
    #[test]
    fn repeated_ticks_keep_existing_singleflight_without_io() {
        let c = coordinator();
        let phone = Device {
            serial: "phone".into(),
            transport: "wifi".into(),
            ..Default::default()
        };
        let adb = Adb {
            path: "must-not-run".into(),
        };
        for _ in 0..30 {
            assert!(c.tick(&phone, &adb, true, false).in_flight);
        }
        assert_eq!(c.inner.lock().unwrap().flight, Some(7));
        c.request_now("phone");
        assert_eq!(c.inner.lock().unwrap().flight, Some(7));
    }
    #[test]
    fn busy_new_device_cancels_old_epoch_without_spawning() {
        let c = coordinator();
        let phone = Device {
            serial: "phone".into(),
            transport: "new-wifi".into(),
            ..Default::default()
        };
        c.tick(
            &phone,
            &Adb {
                path: "must-not-run".into(),
            },
            true,
            true,
        );
        assert!(!c.can_continue(7));
        c.finish(7, Ok(serde_json::json!({"state":"healthy"})));
        assert_ne!(c.snapshot("phone").phase, RecoveryPhase::Healthy);
    }
    #[test]
    fn errors_back_off_without_private_exception_details() {
        let c = coordinator();
        for expected in [30, 60, 120, 240, 480, 960, 1800, 1800] {
            c.finish(7, Err(anyhow::anyhow!("secret phone id and path")));
            let s = c.inner.lock().unwrap();
            assert_eq!(s.snapshot.retry_after_seconds, expected);
            assert!(!s.snapshot.message.contains("secret"));
        }
    }
    #[test]
    fn explicit_check_does_not_change_authorization_or_retry_budget() {
        let c = coordinator();
        c.finish(
            7,
            Ok(serde_json::json!({"state":"needs_attention","retry_after_seconds":300})),
        );
        c.retry_now("different-phone");
        assert!(c.snapshot("phone").retry_after_seconds > 290);
        c.cancel();
        c.retry_now("phone");
        assert!(!c.can_continue(7));
        assert_eq!(c.snapshot("phone").phase, RecoveryPhase::Cancelled);
    }
}
