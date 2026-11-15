//! Google maintenance has its own worker; it never occupies message/call queues.
use super::*;
use stepx_companion::google::recovery::{RecoveryCoordinator, RecoveryPhase, RecoverySnapshot};

#[derive(Default)]
pub(super) struct GoogleRecoveryUi {
    pub coordinator: RecoveryCoordinator,
    pub snapshot: Option<RecoverySnapshot>,
    scope: Option<(String, String)>,
    connected_at: Option<Instant>,
    pub inspect_requested: bool,
    pub manual_inspect_requested: bool,
    pub manual_repair_active: bool,
    manual_repair_pending_kick: bool,
    inspection: Option<(String, String, mpsc::Receiver<Result<GoogleSuiteStatus>>)>,
    pub inspection_error: String,
}

impl GoogleRecoveryUi {
    pub(super) fn is_inspecting(&self) -> bool { self.inspection.is_some() }

    pub(super) fn request_manual_inspect(&mut self) {
        self.inspect_requested = true;
        self.manual_inspect_requested = true;
        self.inspection_error.clear();
    }

    pub(super) fn request_manual_repair(&mut self) {
        self.connected_at = None;
        self.manual_repair_active = true;
        self.manual_repair_pending_kick = true;
        self.inspect_requested = true;
        self.manual_inspect_requested = true;
        self.inspection_error.clear();
    }

    pub(super) fn clear_manual_cooldown(&mut self) {
        self.connected_at = None;
        self.manual_repair_active = false;
        self.manual_repair_pending_kick = false;
        self.inspect_requested = true;
        self.manual_inspect_requested = true;
        self.inspection_error.clear();
    }

    fn reset_manual_flags(&mut self) {
        self.manual_inspect_requested = false;
        self.manual_repair_active = false;
        self.manual_repair_pending_kick = false;
    }

    fn effective_coordinator_flags(
        &self,
        auto_repair_setting: bool,
        hard_busy: bool,
        auto_cooldown_or_paused: bool,
    ) -> (bool, bool) {
        let authorized = auto_repair_setting || self.manual_repair_active;
        let coordinator_busy =
            hard_busy || (!self.manual_repair_active && auto_cooldown_or_paused);
        (authorized, coordinator_busy)
    }

    fn can_start_inspection(
        &self,
        visible: bool,
        hard_busy: bool,
        auto_cooldown_or_paused: bool,
    ) -> bool {
        if !self.inspect_requested
            || self.inspection.is_some()
            || self.snapshot.as_ref().is_some_and(|s| s.in_flight)
        {
            return false;
        }
        if self.manual_inspect_requested {
            !hard_busy
        } else {
            visible && !hard_busy && !auto_cooldown_or_paused
        }
    }
}

impl Companion {
    pub(super) fn google_mutation_wait_reason(&self) -> Option<&'static str> {
        if self.google_recovery.is_inspecting() {
            Some("Google 状态正在检测，完成后可继续安装或修复")
        } else if self.google_recovery.snapshot.as_ref().is_some_and(|state| state.in_flight) {
            Some("自动维护正在完成当前步骤，请稍后重试")
        } else if self.active_call_number.is_some()
            || self.push.snapshot.live_call.ongoing(chrono::Local::now().timestamp_millis())
        {
            Some("通话中，结束后再修复 Google 服务")
        } else { None }
    }

    pub(super) fn set_google_auto_repair(&mut self, enabled: bool) -> Result<()> {
        let mut next = self.settings.value.clone();
        next.google_auto_repair = enabled;
        stepx_companion::settings::save(&next)?;
        self.settings.value = next;
        self.settings.draft.google_auto_repair = enabled;
        if !enabled {
            self.google_recovery.manual_repair_active = false;
            self.google_recovery.manual_repair_pending_kick = false;
            self.google_recovery.coordinator.cancel();
        } else if let Some(device) = &self.selected {
            self.google_recovery
                .coordinator
                .force_request_now(&device.serial);
        }
        Ok(())
    }

    pub(super) fn tick_google_recovery(&mut self, ctx: &mut ViewContext<Self>) {
        if self.is_demo_device() || self.selected.is_none() {
            if self.google_recovery.scope.take().is_some() {
                self.google_recovery.coordinator.disconnect();
                self.google_recovery.snapshot = None;
                self.google_recovery.inspection = None;
                self.google_recovery.connected_at = None;
                self.google_recovery.reset_manual_flags();
            }
            return;
        }
        let mut device = self.selected.clone().expect("selected above");
        device.transport = self.online_device_active_transport(&device);
        let scope = (device.serial.clone(), device.transport.clone());
        if self.google_recovery.scope.as_ref() != Some(&scope) {
            self.google_recovery.coordinator.disconnect();
            self.google_recovery.scope = Some(scope.clone());
            self.google_recovery.connected_at = Some(Instant::now());
            self.google_recovery.snapshot = None;
            self.google_recovery.inspection = None;
            self.google_recovery.inspection_error.clear();
            self.google_recovery.reset_manual_flags();
            self.google_suite = GoogleSuiteStatus::default();
        }
        let visible = self.page == Page::Tools && self.tool_page == Some(ToolPage::Google);
        if let Some((serial, transport, receiver)) = &self.google_recovery.inspection {
            let result = match receiver.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("Google 状态读取中断，请重新检查"))),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = result {
                if serial == &scope.0 && transport == &scope.1 {
                    match result {
                        Ok(suite) => { self.google_suite = suite; self.google_recovery.inspection_error.clear(); }
                        Err(error) => self.google_recovery.inspection_error = error.to_string(),
                    }
                }
                self.google_recovery.inspection = None;
                if visible { ctx.notify(); }
            }
        }
        // Connection warm-up and interactive operations take precedence for
        // background polling, while explicit manual checks bypass user_paused
        // and the 20s initial connect cooldown.
        let hard_busy = self.initializing
            || self.busy
            || self.bulk_busy
            || self.work.running
            || self.workspace.running.is_some()
            || self.doctor_running
            || self.active_call_number.is_some()
            || self.push.snapshot.live_call.ongoing(chrono::Local::now().timestamp_millis())
            || self.connection_watch.disconnected_at.is_some()
            || self.online_device_transports(&device).is_empty();
        let auto_cooldown_or_paused = self.connection_watch.user_paused
            || self
                .google_recovery
                .connected_at
                .is_some_and(|at| at.elapsed() < Duration::from_secs(20));
        if self.busy {
            // Foreground device job (such as GoogleSuiteMaintain) owns the phone.
            self.google_recovery.manual_repair_pending_kick = false;
            self.google_recovery.manual_repair_active = false;
        }
        let (authorized, coordinator_busy) = self.google_recovery.effective_coordinator_flags(
            self.settings.value.google_auto_repair,
            hard_busy,
            auto_cooldown_or_paused,
        );
        let Ok(adb) = Adb::discover() else { return; };
        if self.google_recovery.manual_repair_pending_kick && !coordinator_busy {
            self.google_recovery
                .coordinator
                .force_request_now(&device.serial);
        }
        let mut snapshot = self.google_recovery.coordinator.tick(
            &device,
            &adb,
            authorized,
            coordinator_busy,
        );
        if self.google_recovery.manual_repair_pending_kick && !coordinator_busy {
            self.google_recovery.manual_repair_pending_kick = false;
            if !snapshot.in_flight {
                self.google_recovery
                    .coordinator
                    .force_request_now(&device.serial);
                snapshot = self.google_recovery.coordinator.tick(
                    &device,
                    &adb,
                    authorized,
                    coordinator_busy,
                );
            }
        }
        if !snapshot.in_flight && !self.google_recovery.manual_repair_pending_kick {
            self.google_recovery.manual_repair_active = false;
        }
        if self.google_recovery.snapshot.as_ref() != Some(&snapshot) {
            if snapshot.phase == RecoveryPhase::Healthy
                && self.google_recovery.snapshot.as_ref().is_none_or(|last| last.phase != RecoveryPhase::Healthy)
            {
                self.google_recovery.inspect_requested = true;
            }
            self.google_recovery.snapshot = Some(snapshot);
            if visible { ctx.notify(); }
        }
        if self
            .google_recovery
            .can_start_inspection(visible, hard_busy, auto_cooldown_or_paused)
        {
            self.google_recovery.inspect_requested = false;
            self.google_recovery.manual_inspect_requested = false;
            let (tx, rx) = mpsc::channel();
            self.google_recovery.inspection = Some((scope.0, scope.1, rx));
            std::thread::spawn(move || {
                let _ = tx.send(stepx_companion::google::inspect(&device, &adb));
            });
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn manual_inspect_bypasses_cooldown_and_user_paused_but_respects_hard_busy() {
        let mut ui = GoogleRecoveryUi {
            connected_at: Some(Instant::now()),
            inspect_requested: true,
            ..Default::default()
        };
        // Auto-inspect is blocked during 20s cooldown or user_paused.
        assert!(!ui.can_start_inspection(true, false, true));

        // Manual inspect bypasses both cooldown and user_paused (and even page visibility).
        ui.request_manual_inspect();
        assert!(ui.can_start_inspection(true, false, true));
        assert!(ui.can_start_inspection(false, false, true));
        // Still respects hard_busy (active call / disconnected phone / foreground mutation).
        assert!(!ui.can_start_inspection(true, true, true));
    }

    #[test]
    fn manual_repair_clears_cooldown_and_authorizes_immediate_run_even_when_auto_paused() {
        let mut ui = GoogleRecoveryUi {
            connected_at: Some(Instant::now()),
            inspection_error: "旧错误".into(),
            ..Default::default()
        };
        assert_eq!(ui.effective_coordinator_flags(false, false, true), (false, true));

        ui.request_manual_repair();
        assert!(ui.connected_at.is_none());
        assert!(ui.manual_repair_active);
        assert!(ui.manual_inspect_requested);
        assert!(ui.inspection_error.is_empty());
        assert_eq!(ui.effective_coordinator_flags(false, false, true), (true, false));
        // Still waits if phone is offline or in an active call (hard_busy).
        assert_eq!(ui.effective_coordinator_flags(false, true, true), (true, true));
    }
}
