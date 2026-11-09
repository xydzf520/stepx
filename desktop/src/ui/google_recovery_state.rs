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
    inspection: Option<(String, String, mpsc::Receiver<Result<GoogleSuiteStatus>>)>,
    pub inspection_error: String,
}

impl GoogleRecoveryUi {
    pub(super) fn is_inspecting(&self) -> bool { self.inspection.is_some() }
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
            self.google_recovery.coordinator.cancel();
        } else if let Some(device) = &self.selected {
            self.google_recovery.coordinator.request_now(&device.serial);
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
        // Connection warm-up and interactive operations take precedence. The
        // independent notification stream continues throughout this work.
        let communication_busy = self.initializing
            || self.busy
            || self.bulk_busy
            || self.work.running
            || self.workspace.running.is_some()
            || self.doctor_running
            || self.active_call_number.is_some()
            || self.push.snapshot.live_call.ongoing(chrono::Local::now().timestamp_millis())
            || self.connection_watch.user_paused
            || self.connection_watch.disconnected_at.is_some()
            || self.online_device_transports(&device).is_empty()
            || self.google_recovery.connected_at.is_some_and(|at| at.elapsed() < Duration::from_secs(20));
        let Ok(adb) = Adb::discover() else { return; };
        let snapshot = self.google_recovery.coordinator.tick(
            &device, &adb, self.settings.value.google_auto_repair, communication_busy);
        if self.google_recovery.snapshot.as_ref() != Some(&snapshot) {
            if snapshot.phase == RecoveryPhase::Healthy
                && self.google_recovery.snapshot.as_ref().is_none_or(|last| last.phase != RecoveryPhase::Healthy)
            {
                self.google_recovery.inspect_requested = true;
            }
            self.google_recovery.snapshot = Some(snapshot);
            if visible { ctx.notify(); }
        }
        if visible && self.google_recovery.inspect_requested
            && self.google_recovery.inspection.is_none()
            && !communication_busy
            && !self.google_recovery.snapshot.as_ref().is_some_and(|s| s.in_flight)
        {
            self.google_recovery.inspect_requested = false;
            let (tx, rx) = mpsc::channel();
            self.google_recovery.inspection = Some((scope.0, scope.1, rx));
            std::thread::spawn(move || {
                let _ = tx.send(stepx_companion::google::inspect(&device, &adb));
            });
        }
    }
}
