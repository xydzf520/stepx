//! Connection discovery runs independently of page errors and foreground work.
use super::*;
use stepx_companion::{pairing::WirelessIdentity, reconnect};
struct Snapshot {
    devices: Vec<Transport>,
    online_transports: Option<Vec<String>>,
    services: Vec<Service>,
    identity: Option<WirelessIdentity>,
    standby_probed: bool,
    started_at: Instant,
}
pub(super) const MAX_WIRELESS_RECONNECT_ATTEMPTS: u8 = 6;

pub(super) fn wireless_reconnect_delay(attempt: u8) -> Duration {
    let shift = attempt.min(5);
    Duration::from_secs((2u64 << shift).min(60))
}

#[derive(Default)]
pub(super) struct ConnectionWatch {
    job: Option<(Option<String>, mpsc::Receiver<Result<Snapshot>>)>,
    pending: Option<(Option<String>, Snapshot)>,
    pub(super) misses: u8,
    pub(super) standby_misses: u8,
    standby_serial: Option<String>,
    pub(super) user_paused: bool,
    last_backups: Vec<String>,
    pub detached_serial: Option<String>,
    pub disconnected_at: Option<Instant>,
    pub identity_at: Option<Instant>,
    online_transports: Option<Vec<String>>,
    observed_serial: Option<String>,
    reported_failure: Option<(String, Instant)>,
    pub(super) reconnect_attempts: u8,
    pub(super) next_reconnect_at: Option<Instant>,
}

impl ConnectionWatch {
    pub(super) fn reset_wireless_backoff(&mut self) {
        self.reconnect_attempts = 0;
        self.next_reconnect_at = None;
    }

    pub(super) fn take_wireless_reconnect_budget(&mut self, now: Instant) -> bool {
        if self.reconnect_attempts >= MAX_WIRELESS_RECONNECT_ATTEMPTS {
            return false;
        }
        if self.next_reconnect_at.is_some_and(|due| now < due) {
            return false;
        }
        let delay = wireless_reconnect_delay(self.reconnect_attempts);
        self.reconnect_attempts = self.reconnect_attempts.saturating_add(1);
        self.next_reconnect_at = Some(now + delay);
        true
    }
}

fn wireless_standby_poll_interval(needs_wireless_standby: bool, standby_misses: u8) -> Duration {
    if !needs_wireless_standby {
        return Duration::from_secs(120);
    }
    let shift = standby_misses.min(4);
    Duration::from_secs((8u64 << shift).min(120))
}

fn online_transport_ids(raw: &str) -> Vec<String> {
    raw.lines()
        .filter_map(|line| {
            let mut fields = line.split_whitespace();
            let id = fields.next()?;
            (fields.next() == Some("device")).then(|| id.to_string())
        })
        .collect()
}

impl Companion {
    pub(super) fn connection_failure_status(&mut self, raw: &str) -> String {
        if !stepx_companion::adb::is_transient_transport_error("", "", raw)
            && !raw.contains("操作超时；请检查手机连接") {
            return raw.to_string();
        }
        let operation = raw
            .split(": ")
            .next()
            .filter(|s| *s != "error" && *s != "adb" && *s != raw)
            .unwrap_or("本次操作")
            .trim_start_matches("正在")
            .trim_end_matches(['…', '.', ' ']);
        let text = format!("{operation}未完成：手机通信中断，正在检查连接");
        self.connection_watch.reported_failure = Some((text.clone(), Instant::now()));
        self.begin_connection_watch();
        text
    }

    fn acknowledge_connection_recovery(&mut self, started_at: Instant) -> bool {
        let Some((message, failed_at)) = self.connection_watch.reported_failure.as_ref() else {
            return false;
        };
        if started_at < *failed_at {
            return false;
        }
        if self.selected.as_ref().is_some_and(stepx_companion::push::message_data_stalled) {
            return false; // A getprop response alone cannot acknowledge message-data recovery.
        }
        let still_shown = self.status == *message;
        self.connection_watch.reported_failure = None;
        if still_shown {
            self.error = false;
            self.status = "手机连接已恢复；上次操作未完成，可重试".into();
        }
        still_shown
    }

    /// Backup endpoints are remembered for reconnecting, not evidence that a
    /// second cable or network connection is currently online.
    pub(super) fn online_device_transports(&self, device: &Device) -> Vec<String> {
        let Some(observed) = self
            .connection_watch
            .online_transports
            .as_ref()
            .filter(|_| {
                self.connection_watch.observed_serial.as_deref() == Some(device.serial.as_str())
            })
        else {
            return vec![device.transport.clone()];
        };
        let known = device.all_known_transports();
        observed
            .iter()
            .filter(
                |id| match stepx_companion::device::canonical_serial_from_transport(id) {
                    Some(serial) => serial == device.serial,
                    None => known.contains(id),
                },
            )
            .cloned()
            .collect()
    }

    pub(super) fn online_device_active_transport(&self, device: &Device) -> String {
        let online = self.online_device_transports(device);
        if online.contains(&device.transport) {
            return device.transport.clone();
        }
        let cached = device.active_transport_cached();
        if online.contains(&cached) {
            return cached;
        }
        online
            .first()
            .cloned()
            .unwrap_or_else(|| device.transport.clone())
    }

    pub(super) fn online_device_backups(&self, device: &Device) -> Vec<String> {
        let active = self.online_device_active_transport(device);
        self.online_device_transports(device)
            .into_iter()
            .filter(|id| id != &active)
            .collect()
    }

    pub(super) fn device_has_online_usb_and_wireless(&self, device: &Device) -> bool {
        let online = self.online_device_transports(device);
        online.iter().any(|id| !is_wireless(id)) && online.iter().any(|id| is_wireless(id))
    }

    pub(super) fn device_connection_label(&self, device: &Device) -> &'static str {
        let wireless = is_wireless(&self.online_device_active_transport(device));
        if self.device_has_online_usb_and_wireless(device) {
            if wireless {
                "无线+USB"
            } else {
                "USB+无线"
            }
        } else if wireless {
            "无线直连"
        } else {
            "USB 连接"
        }
    }

    pub(super) fn begin_connection_watch(&mut self) {
        if self.demo_mode
            || self.connection_watch.user_paused
            || self.connection_watch.job.is_some()
        {
            return;
        }
        let current = self.selected.clone();
        let has_remembered_wireless = self
            .preferences
            .wireless
            .values()
            .any(|w| !w.endpoint.trim().is_empty())
            || !self.pairing_hosts.is_empty();
        let allow_wireless_reconnect = if current.is_some() {
            true
        } else if has_remembered_wireless {
            self.connection_watch
                .take_wireless_reconnect_budget(Instant::now())
        } else {
            false
        };
        let current_serial = current.as_ref().map(|d| d.serial.clone());
        if self.connection_watch.standby_serial != current_serial {
            self.connection_watch.standby_serial = current_serial;
            self.connection_watch.standby_misses = 0;
        }
        let origin = current.as_ref().map(|d| d.transport.clone());
        let prefs = self.preferences.clone();
        let hosts = self.pairing_hosts.clone();
        let needs_wireless_standby = self
            .selected
            .as_ref()
            .is_some_and(|d| !is_wireless(&d.transport) && d.backup_transports().is_empty());
        if !needs_wireless_standby {
            self.connection_watch.standby_misses = 0;
        }
        let poll_interval = wireless_standby_poll_interval(
            needs_wireless_standby,
            self.connection_watch.standby_misses,
        );
        let identity_due = self
            .connection_watch
            .identity_at
            .is_none_or(|t| t.elapsed() > poll_interval);
        if identity_due {
            self.connection_watch.identity_at = Some(Instant::now());
        }
        let (tx, rx) = mpsc::channel();
        self.connection_watch.job = Some((origin, rx));
        std::thread::spawn(move || {
            let result = (|| -> Result<Snapshot> {
                let started_at = Instant::now();
                let adb = Adb::discover()?;
                let raw = adb.run_timeout(&["devices", "-l"], Duration::from_secs(3))?;
                let mut online_transports = Some(online_transport_ids(&raw));
                let mut devices = parse_devices(&raw);
                let mut online = current.as_ref().is_some_and(|d| {
                    devices.iter().any(|t| {
                        t.state == "device"
                            && (t.id == d.transport
                                || t.backup_transports().iter().any(|b| b == &d.transport))
                    })
                });
                if online {
                    if let Some(d) = current.as_ref() {
                        if stepx_companion::adb::is_wireless_transport(&d.transport) {
                            let probe_ok = adb.probe_device_identity(
                                &d.transport,
                                &d.serial,
                                Duration::from_millis(1800),
                            );
                            if !probe_ok {
                                let _ = adb.run_timeout(
                                    &["disconnect", &d.transport],
                                    Duration::from_millis(800),
                                );
                                stepx_companion::device::invalidate_verified_transport(
                                    &d.transport,
                                );
                                let recovered = stepx_companion::device::recover_transport(
                                    &adb,
                                    &d.transport,
                                    Duration::from_secs(4),
                                );
                                online = recovered.as_deref() == Some(d.transport.as_str());
                                if !online {
                                    let _ = adb.run_timeout(
                                        &["disconnect", &d.transport],
                                        Duration::from_millis(800),
                                    );
                                    stepx_companion::device::invalidate_verified_transport(
                                        &d.transport,
                                    );
                                    if let Some(ref next) = recovered {
                                        stepx_companion::device::promote_active_transport(
                                            &d.serial,
                                            next,
                                        );
                                    }
                                }
                                if let Ok(raw) =
                                    adb.run_timeout(&["devices", "-l"], Duration::from_secs(2))
                                {
                                    online_transports = Some(online_transport_ids(&raw));
                                    devices = parse_devices(&raw);
                                }
                                if !online {
                                    if let Some(ids) = online_transports.as_mut() {
                                        ids.retain(|id| id != &d.transport);
                                    }
                                    for t in &mut devices {
                                        if t.id == d.transport {
                                            t.state = "offline".into();
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                if online {
                    let standby_probed = identity_due
                        && current.as_ref().is_some_and(|d| !is_wireless(&d.transport));
                    let identity = if identity_due {
                        current.as_ref().and_then(|d| {
                            reconnect::ensure_usb_wireless_standby(&adb, d).or_else(|| {
                                if !is_wireless(&d.transport) {
                                    let mut id =
                                        reconnect::network_identity(&adb, d).unwrap_or_default();
                                    id.endpoint.clear();
                                    Some(id)
                                } else {
                                    None
                                }
                            })
                        })
                    } else {
                        None
                    };
                    // Warm up wireless standby while on USB so unplugging fails over seamlessly.
                    if let Some(d) = current.as_ref() {
                        if !is_wireless(&d.transport) {
                            let standby_ep = if identity_due {
                                identity
                                    .as_ref()
                                    .map(|id| id.endpoint.as_str())
                                    .filter(|ep| !ep.is_empty())
                            } else {
                                prefs
                                    .wireless
                                    .get(&d.serial)
                                    .map(|id| id.endpoint.as_str())
                                    .filter(|ep| !ep.is_empty())
                            };
                            if identity_due && standby_ep.is_none() {
                                if let Some(stale_ep) = prefs
                                    .wireless
                                    .get(&d.serial)
                                    .map(|id| id.endpoint.as_str())
                                    .filter(|ep| !ep.is_empty())
                                {
                                    if devices.iter().any(|t| t.id == stale_ep && t.state != "device") {
                                        let _ = adb.run_timeout(
                                            &["disconnect", stale_ep],
                                            Duration::from_millis(1200),
                                        );
                                        devices.retain(|t| t.id != stale_ep);
                                    }
                                }
                            }
                            if let Some(ep) = standby_ep {
                                let is_offline =
                                    devices.iter().any(|t| t.id == ep && t.state != "device");
                                let already_connected = devices.iter().any(|t| {
                                    t.state == "device"
                                        && (t.id == ep
                                            || t.backup_transports().iter().any(|b| b == ep))
                                });
                                if (!already_connected && identity_due) || is_offline {
                                    let _ = adb
                                        .run_timeout(&["connect", ep], Duration::from_millis(1800));
                                    if let Ok(raw) =
                                        adb.run_timeout(&["devices", "-l"], Duration::from_secs(2))
                                    {
                                        online_transports = Some(online_transport_ids(&raw));
                                        devices = parse_devices(&raw);
                                    }
                                }
                                if devices.iter().any(|t| t.id == ep && t.state == "device")
                                    && adb
                                        .run_timeout(
                                            &["-s", ep, "shell", "getprop", "ro.serialno"],
                                            Duration::from_secs(2),
                                        )
                                        .is_ok_and(|serial| serial.trim() == d.serial)
                                {
                                    devices.retain(|t| t.id != ep);
                                    let mut backups = d.backup_transports();
                                    if !backups.iter().any(|b| b == ep) {
                                        backups.push(ep.to_string());
                                        backups.sort_by(|a, b| {
                                            stepx_companion::device::transport_priority(a)
                                                .cmp(&stepx_companion::device::transport_priority(
                                                    b,
                                                ))
                                                .then_with(|| a.cmp(b))
                                        });
                                        d.set_backup_transports(backups);
                                    }
                                }
                            }
                            if let Some(active) = devices
                                .iter()
                                .find(|t| t.id == d.transport && t.state == "device")
                            {
                                let active_backups = active.backup_transports();
                                if !active_backups.is_empty() {
                                    d.set_backup_transports(active_backups);
                                }
                            }
                        }
                    }
                    Ok(Snapshot {
                        started_at,
                        devices,
                        online_transports,
                        services: vec![],
                        identity,
                        standby_probed,
                    })
                } else {
                    if let Some(d) = current.as_ref() {
                        let known_backups = d.backup_transports();
                        let mut healed_offline = false;
                        for t in &devices {
                            if t.state != "device"
                                && is_wireless(&t.id)
                                && (t.id == d.transport || known_backups.contains(&t.id))
                                && !t.id.starts_with("adb-")
                            {
                                let _ = stepx_companion::device::recover_transport(
                                    &adb,
                                    &t.id,
                                    Duration::from_secs(3),
                                );
                                healed_offline = true;
                            }
                        }
                        if healed_offline {
                            if let Ok(raw) =
                                adb.run_timeout(&["devices", "-l"], Duration::from_secs(2))
                            {
                                online_transports = Some(online_transport_ids(&raw));
                                devices = parse_devices(&raw);
                            }
                        }
                    }
                    let has_online_failover = current.as_ref().is_some_and(|d| {
                        let known_backups = d.backup_transports();
                        devices.iter().any(|t| {
                            t.state == "device"
                                && (known_backups.contains(&t.id)
                                    || stepx_companion::device::canonical_serial_from_transport(
                                        &t.id,
                                    )
                                    .as_deref()
                                        == Some(d.serial.as_str()))
                        })
                    });
                    if has_online_failover {
                        Ok(Snapshot {
                            started_at,
                            devices,
                            online_transports,
                            services: vec![],
                            identity: None,
                            standby_probed: false,
                        })
                    } else if allow_wireless_reconnect {
                        let (devices, services) = reconnect::scan(&adb, &prefs, &hosts)?;
                        let online_transports = adb
                            .run_timeout(&["devices", "-l"], Duration::from_secs(2))
                            .ok()
                            .map(|raw| online_transport_ids(&raw));
                        Ok(Snapshot {
                            started_at,
                            devices,
                            online_transports,
                            services,
                            identity: None,
                            standby_probed: false,
                        })
                    } else {
                        let services = stepx_companion::discovery::discover_services(&adb)
                            .unwrap_or_default();
                        Ok(Snapshot {
                            started_at,
                            devices,
                            online_transports,
                            services,
                            identity: None,
                            standby_probed: false,
                        })
                    }
                }
            })();
            let _ = tx.send(result);
        });
    }
    pub(super) fn tick_connection_watch(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.busy {
            if let Some((origin, snapshot)) = self.connection_watch.pending.take() {
                if self.selected.as_ref().map(|d| d.transport.clone()) == origin {
                    self.connection_watch.observed_serial =
                        self.selected.as_ref().map(|device| device.serial.clone());
                    self.connection_watch.online_transports = snapshot.online_transports;
                    self.apply(Outcome::Transports(snapshot.devices, snapshot.services));
                    ctx.notify();
                    return;
                }
            }
        }
        let Some((origin, rx)) = &self.connection_watch.job else {
            if self.selected.is_none()
                && !self.demo_mode
                && !self.connection_watch.user_paused
                && self.connection_watch.reconnect_attempts < MAX_WIRELESS_RECONNECT_ATTEMPTS
                && self
                    .preferences
                    .wireless
                    .values()
                    .any(|w| !w.endpoint.trim().is_empty())
                && self
                    .connection_watch
                    .next_reconnect_at
                    .is_none_or(|due| Instant::now() >= due)
            {
                self.last_scan = Instant::now();
                self.begin_connection_watch();
            }
            return;
        };
        let result = match rx.try_recv() {
            Ok(r) => r,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(_) => Err(anyhow::anyhow!("连接检测未完成")),
        };
        let origin = origin.clone();
        self.connection_watch.job = None;
        if self.selected.as_ref().map(|d| d.transport.clone()) != origin {
            return;
        }
        let Ok(snapshot) = result else {
            if self.connection_watch.online_transports.take().is_some() {
                ctx.notify();
            }
            self.connection_watch.observed_serial = None;
            return;
        };
        let online_changed = self.connection_watch.online_transports != snapshot.online_transports;
        self.connection_watch.observed_serial =
            self.selected.as_ref().map(|device| device.serial.clone());
        self.connection_watch.online_transports = snapshot.online_transports.clone();
        if online_changed {
            ctx.notify();
        }
        if snapshot.standby_probed {
            let has_standby_ep = snapshot
                .identity
                .as_ref()
                .is_some_and(|id| !id.endpoint.is_empty());
            if has_standby_ep {
                self.connection_watch.standby_misses = 0;
            } else {
                self.connection_watch.standby_misses =
                    self.connection_watch.standby_misses.saturating_add(1);
            }
        }
        if let (Some(d), Some(identity)) = (&self.selected, snapshot.identity.clone()) {
            if self.apply_wireless_identity_update(&d.serial.clone(), identity) {
                if let Err(e) = self.preferences.save() {
                    use std::io::Write as _;
                    let _ = writeln!(
                        std::io::stderr(),
                        "Unable to remember wireless connection: {e}"
                    );
                }
            }
        }
        let promoted_away = origin.as_ref().zip(self.selected.as_ref()).is_some_and(
            |(id, selected)| {
                stepx_companion::adb::is_wireless_transport(id)
                    && super::active_transport_for(&selected.serial, id) != *id
            },
        );
        let online = !promoted_away
            && origin.as_ref().is_some_and(|id| {
                snapshot.devices.iter().any(|d| {
                    d.state == "device"
                        && (&d.id == id || d.backup_transports().iter().any(|b| b == id))
                })
            });
        if online {
            if self.acknowledge_connection_recovery(snapshot.started_at) {
                ctx.notify();
            }
            self.connection_watch.misses = 0;
            self.connection_watch.disconnected_at = None;
            self.connection_watch.reset_wireless_backoff();
        }
        let has_online_failover = self.selected.as_ref().is_some_and(|selected| {
            let known_backups = selected.backup_transports();
            snapshot.devices.iter().any(|t| {
                t.state == "device"
                    && (known_backups.contains(&t.id)
                        || stepx_companion::device::canonical_serial_from_transport(&t.id)
                            .as_deref()
                            == Some(selected.serial.as_str()))
            })
        });
        if !online {
            if self.busy && !has_online_failover {
                self.connection_watch.pending = Some((origin, snapshot));
                return;
            } // Apply after the foreground result; discovery itself is never blocked by it.
            self.apply(Outcome::Transports(snapshot.devices, snapshot.services));
            ctx.notify();
        } else {
            let new_devices =
                stepx_companion::device::deduplicate_transports(snapshot.devices.clone());
            let mut backup_changed = false;
            if let Some(selected) = self.selected.as_mut() {
                let before = selected.backup_transports();
                if let Some(active) = new_devices.iter().find(|t| {
                    t.state == "device"
                        && (t.id == selected.transport
                            || t.backup_transports()
                                .iter()
                                .any(|b| b == &selected.transport))
                }) {
                    let mut active_backups: Vec<String> = std::iter::once(active.id.clone())
                        .chain(active.backup_transports())
                        .filter(|b| b != &selected.transport)
                        .collect();
                    active_backups.dedup();
                    selected.set_backup_transports(active_backups);
                }
                let after = selected.backup_transports();
                if after != before || after != self.connection_watch.last_backups {
                    self.connection_watch.last_backups = after;
                    backup_changed = true;
                }
            }
            let services_changed =
                !snapshot.services.is_empty() && self.mdns_services != snapshot.services;
            if services_changed {
                self.mdns_services = snapshot.services;
            }
            let devices_changed = self.devices != new_devices;
            self.devices = new_devices;
            if devices_changed || services_changed || backup_changed || self.selected.is_none() {
                ctx.notify();
            }
        }
    }

    pub(super) fn apply_wireless_identity_update(
        &mut self,
        serial: &str,
        mut identity: WirelessIdentity,
    ) -> bool {
        // Preserve remembered guid, wlan_mac, and wifi_ip, while allowing an empty
        // live endpoint from USB inspection to clear a stale ephemeral port.
        if let Some(prior) = self.preferences.wireless.get(serial) {
            if identity.guid.is_empty() && !prior.guid.is_empty() {
                identity.guid = prior.guid.clone();
            }
            if identity.wlan_mac.is_empty() && !prior.wlan_mac.is_empty() {
                identity.wlan_mac = prior.wlan_mac.clone();
            }
            if identity.wifi_ip.is_empty() && !prior.wifi_ip.is_empty() {
                identity.wifi_ip = prior.wifi_ip.clone();
            }
        } else if identity.endpoint.is_empty()
            && identity.guid.is_empty()
            && identity.wlan_mac.is_empty()
            && identity.wifi_ip.is_empty()
        {
            return false;
        }
        if self.preferences.wireless.get(serial) != Some(&identity) {
            self.preferences.wireless.insert(serial.to_string(), identity);
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_fresh_connection_receipt_clears_the_matching_error() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        let stale = Instant::now();
        let status = app.connection_failure_status("读取应用列表: error: closed");
        assert_eq!(status, "读取应用列表未完成：手机通信中断，正在检查连接");
        app.status = status;
        app.error = true;
        assert!(!app.acknowledge_connection_recovery(stale));
        assert!(app.error);
        assert!(app.acknowledge_connection_recovery(Instant::now()));
        assert!(!app.error);
        assert!(app.status.contains("上次操作未完成"));
        app.connection_failure_status("读取文件: error: closed");
        app.status = "安装被手机拒绝".into();
        app.error = true;
        assert!(!app.acknowledge_connection_recovery(Instant::now()));
        assert_eq!(app.status, "安装被手机拒绝");
        assert!(app.error);
    }

    #[test]
    fn outcome_transports_requires_two_misses_before_disconnect_and_switches_channel() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true; // Avoid spawning real adb threads during unit test
        let dev = Device {
            serial: "STEPX-SWITCH-TEST".into(),
            transport: "STEPX-SWITCH-TEST".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        dev.set_backup_transports(vec!["192.168.1.88:5555".into()]);
        app.selected = Some(dev);

        // Saved backup addresses alone must not be selectable until observed online.
        app.handle_device_action(&Action::SwitchPreferredTransport(
            "192.168.1.88:5555".into(),
        ));
        assert_eq!(
            app.selected.as_ref().unwrap().transport,
            "STEPX-SWITCH-TEST"
        );
        app.connection_watch.observed_serial = Some("STEPX-SWITCH-TEST".into());
        app.connection_watch.online_transports =
            Some(vec!["STEPX-SWITCH-TEST".into(), "192.168.1.88:5555".into()]);
        // 1. Switch preferred transport to verified online wireless
        app.handle_device_action(&Action::SwitchPreferredTransport(
            "192.168.1.88:5555".into(),
        ));
        assert_eq!(
            app.selected.as_ref().map(|d| d.transport.as_str()),
            Some("192.168.1.88:5555")
        );
        assert!(app
            .selected
            .as_ref()
            .unwrap()
            .backup_transports()
            .contains(&"STEPX-SWITCH-TEST".to_string()));

        // 2. Clear backups to test 2-miss hysteresis
        app.selected.as_ref().unwrap().set_backup_transports(vec![]);

        // First empty transport snapshot -> misses == 1, still connected!
        app.apply(Outcome::Transports(vec![], vec![]));
        assert_eq!(app.connection_watch.misses, 1);
        assert!(app.selected.is_some());

        // Second empty transport snapshot -> misses == 2, transitions to disconnected
        app.apply(Outcome::Transports(vec![], vec![]));
        assert_eq!(app.connection_watch.misses, 2);
        assert!(app.selected.is_none());
        assert_eq!(
            app.connection_watch.detached_serial.as_deref(),
            Some("STEPX-SWITCH-TEST")
        );
    }

    #[test]
    fn wireless_backups_and_historical_usb_do_not_claim_live_dual_connection() {
        let mut app = Companion::new_for_test();
        let device = Device {
            serial: "ONLINE_LINK_UI_ONE".into(),
            transport: "192.0.2.240:43001".into(),
            ..Default::default()
        };
        device.set_backup_transports(vec![device.serial.clone(), "192.0.2.240:43002".into()]);
        // Before a current observation, remembered peers cannot establish dual connectivity.
        assert_eq!(app.device_connection_label(&device), "无线直连");
        assert!(!app.device_has_online_usb_and_wireless(&device));
        app.connection_watch.observed_serial = Some(device.serial.clone());
        app.connection_watch.online_transports=Some(online_transport_ids(
            "List of devices attached\n192.0.2.240:43001 device\n192.0.2.240:43002 device\nONLINE_LINK_UI_ONE offline\n"));
        assert_eq!(
            app.online_device_backups(&device),
            vec!["192.0.2.240:43002".to_string()]
        );
        assert_eq!(app.device_connection_label(&device), "无线直连");
        assert!(!app.device_has_online_usb_and_wireless(&device));
        // The same USB identity must actually reappear online before it is displayed.
        app.connection_watch
            .online_transports
            .as_mut()
            .unwrap()
            .push(device.serial.clone());
        assert!(app.device_has_online_usb_and_wireless(&device));
        assert_eq!(app.device_connection_label(&device), "无线+USB");
    }

    #[test]
    fn another_devices_online_usb_and_unknown_wireless_do_not_become_this_phones_backup() {
        let mut app = Companion::new_for_test();
        let device = Device {
            serial: "ONLINE_LINK_UI_TWO".into(),
            transport: "192.0.2.241:43001".into(),
            ..Default::default()
        };
        device.set_backup_transports(vec!["ONLINE_LINK_UI_OTHER".into()]);
        app.connection_watch.observed_serial = Some(device.serial.clone());
        app.connection_watch.online_transports=Some(online_transport_ids(
            "List of devices attached\n192.0.2.241:43001 device\nONLINE_LINK_UI_OTHER device\n192.0.2.242:43001 device\nONLINE_LINK_UI_TWO unauthorized\n"));
        assert_eq!(
            app.online_device_transports(&device),
            vec![device.transport.clone()]
        );
        assert!(app.online_device_backups(&device).is_empty());
        assert!(!app.device_has_online_usb_and_wireless(&device));
        assert_eq!(app.device_connection_label(&device), "无线直连");
    }

    #[test]
    fn promoted_usb_flips_half_open_wireless_immediately_and_clears_stale_endpoint() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        let serial = "STEPX-HALF-OPEN-USB";
        let stale_wireless = "192.168.43.22:42959";
        let dev = Device {
            serial: serial.into(),
            transport: stale_wireless.into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        dev.set_backup_transports(vec![serial.into()]);
        app.selected = Some(dev);
        app.preferences.wireless.insert(
            serial.into(),
            WirelessIdentity {
                endpoint: stale_wireless.into(),
                guid: "adb-STEPX-HALF-OPEN-USB-abcd".into(),
                wlan_mac: "aa:bb:cc:dd:ee:ff".into(),
                wifi_ip: "192.168.43.22".into(),
            },
        );

        // Simulate recover_transport having promoted USB while adb devices still listed wireless.
        stepx_companion::device::promote_active_transport(serial, serial);
        let raw_devices = parse_devices(
            "List of devices attached\n192.168.43.22:42959 device model:PZ261A\nSTEPX-HALF-OPEN-USB device model:PZ261A\n",
        );
        app.apply(Outcome::Transports(raw_devices, vec![]));
        assert_eq!(
            app.selected.as_ref().map(|d| d.transport.as_str()),
            Some(serial)
        );

        // Live USB inspection with Wireless Debugging off clears stale ephemeral port.
        assert!(app.apply_wireless_identity_update(
            serial,
            WirelessIdentity {
                endpoint: String::new(),
                guid: String::new(),
                wlan_mac: String::new(),
                wifi_ip: "192.168.43.22".into(),
            }
        ));
        let updated = app.preferences.wireless.get(serial).unwrap();
        assert_eq!(updated.endpoint, "");
        assert_eq!(updated.guid, "adb-STEPX-HALF-OPEN-USB-abcd");
        assert_eq!(updated.wlan_mac, "aa:bb:cc:dd:ee:ff");
        assert_eq!(updated.wifi_ip, "192.168.43.22");

        // Standby polling interval backs off exponentially on consecutive misses up to 120s.
        assert_eq!(wireless_standby_poll_interval(true, 0), Duration::from_secs(8));
        assert_eq!(wireless_standby_poll_interval(true, 1), Duration::from_secs(16));
        assert_eq!(wireless_standby_poll_interval(true, 2), Duration::from_secs(32));
        assert_eq!(wireless_standby_poll_interval(true, 4), Duration::from_secs(120));
        assert_eq!(wireless_standby_poll_interval(false, 0), Duration::from_secs(120));
    }

    #[test]
    fn wireless_reconnect_backoff_is_exponential_and_bounded_at_max_attempts() {
        assert_eq!(wireless_reconnect_delay(0), Duration::from_secs(2));
        assert_eq!(wireless_reconnect_delay(1), Duration::from_secs(4));
        assert_eq!(wireless_reconnect_delay(2), Duration::from_secs(8));
        assert_eq!(wireless_reconnect_delay(3), Duration::from_secs(16));
        assert_eq!(wireless_reconnect_delay(4), Duration::from_secs(32));
        assert_eq!(wireless_reconnect_delay(5), Duration::from_secs(60));

        let mut watch = ConnectionWatch::default();
        let mut t = Instant::now();
        for i in 0..MAX_WIRELESS_RECONNECT_ATTEMPTS {
            assert!(watch.take_wireless_reconnect_budget(t));
            // Immediate retry before backoff window is rejected
            assert!(!watch.take_wireless_reconnect_budget(t));
            t += wireless_reconnect_delay(i) + Duration::from_millis(10);
        }
        assert!(!watch.take_wireless_reconnect_budget(t));
        watch.reset_wireless_backoff();
        assert!(watch.take_wireless_reconnect_budget(t));
    }
}
