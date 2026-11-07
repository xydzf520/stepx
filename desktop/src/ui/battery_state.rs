use super::*;
use stepx_companion::battery::{BatteryCache, BatteryState};

pub(super) struct BatteryUi {
    scope: Option<String>,
    cache: BatteryCache,
    pending: Option<(Instant, mpsc::Receiver<Result<BatteryState>>)>,
    shown_stale: bool,
}
impl Default for BatteryUi {
    fn default() -> Self {
        Self {
            scope: None,
            cache: BatteryCache::new(Instant::now()),
            pending: None,
            shown_stale: false,
        }
    }
}
impl Companion {
    fn ensure_battery_scope(&mut self) {
        let scope = self.selected.as_ref().map(|d| d.serial.clone());
        if self.battery.scope != scope {
            self.battery = BatteryUi {
                scope,
                ..Default::default()
            };
        }
    }
    pub(super) fn observe_battery(&mut self, serial: &str, value: &BatteryState) -> bool {
        if self.selected.as_ref().is_none_or(|d| d.serial != serial) {
            return false;
        }
        self.ensure_battery_scope();
        self.battery.cache.observe(value, Instant::now())
    }
    pub(super) fn battery_reading(&self) -> Option<&BatteryState> {
        (self.selected.as_ref().map(|d| d.serial.as_str()) == self.battery.scope.as_deref())
            .then_some(self.battery.cache.reading.as_ref())
            .flatten()
    }
    pub(super) fn battery_hint(&self) -> String {
        let Some(reading) = self.battery_reading() else {
            return "手机电量 · 等待更新".into();
        };
        let status = reading.label();
        if self.battery.cache.stale(Instant::now()) {
            let minutes = self
                .battery
                .cache
                .observed_at
                .map(|at| at.elapsed().as_secs() / 60)
                .unwrap_or(0);
            format!("{status} · 上次读取于 {minutes} 分钟前，正在等待连接恢复")
        } else {
            status.into()
        }
    }
    pub(super) fn tick_battery(&mut self, ctx: &mut ViewContext<Self>) {
        self.ensure_battery_scope();
        let mut changed = false;
        if let Some((started, rx)) = &self.battery.pending {
            let outcome = match rx.try_recv() {
                Ok(result) => Some(result),
                Err(mpsc::TryRecvError::Disconnected) => Some(Err(anyhow::anyhow!("电量读取中断"))),
                Err(mpsc::TryRecvError::Empty) => None,
            };
            if let Some(result) = outcome {
                changed |= self
                    .battery
                    .cache
                    .fallback_result(*started, Instant::now(), result);
                self.battery.pending = None;
            }
        }
        let stale = self.battery.cache.stale(Instant::now());
        changed |= stale != self.battery.shown_stale;
        self.battery.shown_stale = stale;
        if changed {
            ctx.notify();
        }
        if self.demo_mode
            || self.battery.pending.is_some()
            || !self.battery.cache.due(Instant::now())
        {
            return;
        }
        let Some(device) = self.selected.as_ref() else {
            return;
        };
        // A single cheap read on older APKs; no device summary, screen input,
        // model task, provider installation, or separate polling timer.
        let transport = self.online_device_active_transport(device);
        let (tx, rx) = mpsc::channel();
        self.battery.pending = Some((Instant::now(), rx));
        std::thread::spawn(move || {
            let result = Adb::discover()
                .and_then(|adb| {
                    adb.run_timeout(
                        &["-s", &transport, "shell", "dumpsys", "battery"],
                        Duration::from_secs(4),
                    )
                })
                .and_then(|raw| BatteryState::parse_dumpsys(&raw));
            let _ = tx.send(result);
        });
    }
}
