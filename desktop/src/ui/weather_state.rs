//! Device-scoped weather scheduling. A bounded background request must never
//! block synchronization or leave startup weather waiting for a manual refresh.

use super::{Companion, Device, ViewContext};
use stepx_companion::weather;

#[derive(Clone, Debug, PartialEq, Eq)]
struct WeatherFetchRequest {
    // Empty means restore the saved city in the worker (no UI-thread disk I/O).
    city: String,
    report_status: bool,
}

#[derive(Default)]
pub(super) struct WeatherFetchState {
    observed_device: Option<Device>,
    observed_initialized: bool,
    in_flight: bool,
    pending: Option<WeatherFetchRequest>,
}

impl WeatherFetchState {
    fn observe_device(&mut self, current: Option<&Device>) -> bool {
        let changed = !self.observed_initialized
            || (self.observed_device.as_ref() != current
                && !weather::weather_request_matches_device(
                    self.observed_device.as_ref(),
                    current,
                ));
        if changed {
            self.observed_initialized = true;
            self.observed_device = current.cloned();
            // Preserve a newer explicit city selection, otherwise restore the
            // saved selection once for this phone. Heartbeats do not enqueue.
            self.pending.get_or_insert(WeatherFetchRequest {
                city: String::new(),
                report_status: false,
            });
        }
        changed
    }

    fn queue(&mut self, city: &str, report_status: bool) {
        // Rapid refreshes/city changes coalesce to the latest request, never
        // create a second simultaneous phone or HTTP fetch.
        self.pending = Some(WeatherFetchRequest {
            city: city.trim().to_string(),
            report_status,
        });
    }

    fn begin(&mut self) -> Option<WeatherFetchRequest> {
        if self.in_flight {
            return None;
        }
        let request = self.pending.take()?;
        self.in_flight = true;
        Some(request)
    }

    fn finish(&mut self) {
        self.in_flight = false;
    }
}

impl Companion {
    pub(in crate::ui) fn spawn_live_weather_fetch(
        &mut self,
        city: &str,
        ctx: &mut ViewContext<Self>,
    ) {
        self.queue_live_weather_fetch(city, false, ctx);
    }

    pub(in crate::ui) fn queue_live_weather_fetch(
        &mut self,
        city: &str,
        report_status: bool,
        ctx: &mut ViewContext<Self>,
    ) {
        if cfg!(test) || self.demo_mode || self.is_demo_device() {
            return;
        }
        self.weather_fetch.queue(city, report_status);
        self.tick_weather_fetch(ctx);
    }

    pub(super) fn tick_weather_fetch(&mut self, ctx: &mut ViewContext<Self>) {
        if cfg!(test) || self.demo_mode || self.is_demo_device() {
            return;
        }
        if self.weather_fetch.observe_device(self.selected.as_ref()) {
            weather::discard_weather_snapshot_for_other_device(self.selected.as_ref());
            if self.weather_snapshot.region_label.contains("系统天气 MCP") {
                // A different phone must not temporarily show the previous
                // phone's location while the replacement fetch is pending.
                self.weather_snapshot = weather::WeatherSnapshot::default_for_city("手机定位");
                self.weather_selected_day = 0;
                ctx.notify();
            }
        }
        let Some(request) = self.weather_fetch.begin() else {
            return;
        };
        let selection_revision = weather::weather_selection_revision();
        let requested_device = self.selected.clone();
        let device_for_fetch = requested_device.clone();
        let city_for_fetch = request.city;
        let report_status = request.report_status;
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let city = if city_for_fetch.is_empty() {
                weather::load_preferred_city()
            } else {
                city_for_fetch
            };
            let result = weather::fetch_live_weather_snapshot_for_device_selection(
                &city,
                selection_revision,
                device_for_fetch.as_ref(),
            );
            let _ = tx.send_blocking(result);
        });
        ctx.spawn(async move { rx.recv().await }, move |me, result, ctx| {
            me.weather_fetch.finish();
            let current_request = weather::weather_selection_revision() == selection_revision
                && weather::weather_request_matches_device(
                    requested_device.as_ref(),
                    me.selected.as_ref(),
                )
                && me.weather_fetch.pending.is_none();
            if current_request {
                match result {
                    Ok(Ok(snapshot)) => {
                        if weather::publish_weather_snapshot_for_device_selection(
                            &snapshot,
                            selection_revision,
                            requested_device.as_ref(),
                        )
                        .is_ok()
                        {
                            me.weather_snapshot = snapshot;
                            if report_status {
                                me.error = false;
                                me.status = format!(
                                    "已更新「{}」天气：{} {}°C（{}；{}）",
                                    me.weather_snapshot.city,
                                    me.weather_snapshot.condition,
                                    me.weather_snapshot.temp_c,
                                    me.weather_snapshot.range_label(),
                                    me.weather_snapshot.source_label()
                                );
                            }
                            ctx.notify();
                        }
                    }
                    Ok(Err(error)) if report_status => {
                        me.error = true;
                        me.status = format!("天气更新失败：{error}。已保留现有显示，可再次刷新。");
                        ctx.notify();
                    }
                    Err(_) if report_status => {
                        me.error = true;
                        me.status = "天气更新未完成，已保留现有显示，可再次刷新。".into();
                        ctx.notify();
                    }
                    _ => {}
                }
            }
            // Auto-selection may have completed after the startup request, or
            // the user may have switched phones/cities while it was running.
            // Drain the latest request immediately, even for a stale response.
            me.tick_weather_fetch(ctx);
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phone(serial: &str, transport: &str) -> Device {
        Device {
            serial: serial.into(),
            transport: transport.into(),
            ..Default::default()
        }
    }

    #[test]
    fn weather_async_startup_selection_queues_one_replacement_without_parallel_fetch() {
        let mut state = WeatherFetchState::default();
        assert!(state.observe_device(None));
        assert!(state.begin().is_some());
        let connected = phone("phone-a", "10.0.1.2:5555");
        assert!(state.observe_device(Some(&connected)));
        for _ in 0..100 {
            assert!(!state.observe_device(Some(&connected)));
            assert!(state.begin().is_none());
        }
        state.finish();
        assert!(state.begin().is_some());
        state.finish();
        assert!(state.begin().is_none());
    }

    #[test]
    fn weather_route_changes_keep_snapshot_but_replacement_and_disconnect_refresh() {
        let mut state = WeatherFetchState::default();
        assert!(state.observe_device(Some(&phone("phone-a", "usb-a"))));
        state.begin();
        state.finish();
        assert!(!state.observe_device(Some(&phone("phone-a", "10.0.1.2:5555"))));
        assert!(state.begin().is_none());
        assert!(state.observe_device(Some(&phone("phone-b", "10.0.1.2:5555"))));
        assert!(state.begin().is_some());
        state.finish();
        assert!(state.observe_device(None));
        assert!(state.begin().is_some());
        state.finish();
        assert!(!state.observe_device(None));
        assert!(state.begin().is_none());
    }

    #[test]
    fn weather_refresh_coalesces_and_device_change_preserves_latest_city_choice() {
        let mut state = WeatherFetchState::default();
        state.observe_device(None);
        state.begin();
        state.queue("南京", true);
        state.queue("苏州", true);
        state.queue(" 北京 ", true);
        state.observe_device(Some(&phone("phone-a", "usb-a")));
        assert!(state.begin().is_none());
        state.finish();
        assert_eq!(
            state.begin(),
            Some(WeatherFetchRequest {
                city: "北京".into(),
                report_status: true,
            })
        );
        state.finish();
        assert!(state.begin().is_none());
    }
}
