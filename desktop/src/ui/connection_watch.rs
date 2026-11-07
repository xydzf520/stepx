//! Connection discovery runs independently of page errors and foreground work.
use super::*;
use stepx_companion::{pairing::WirelessIdentity, reconnect};
struct Snapshot {
    devices: Vec<Transport>,
    online_transports: Option<Vec<String>>,
    services: Vec<Service>,
    identity: Option<WirelessIdentity>,
    started_at: Instant,
}
#[derive(Default)]
pub(super) struct ConnectionWatch {
    job: Option<(Option<String>, mpsc::Receiver<Result<Snapshot>>)>,
    pending: Option<(Option<String>, Snapshot)>,
    pub(super) misses: u8,
    pub(super) user_paused: bool,
    last_backups: Vec<String>,
    pub detached_serial: Option<String>,
    pub disconnected_at: Option<Instant>,
    pub identity_at: Option<Instant>,
    online_transports: Option<Vec<String>>,
    observed_serial: Option<String>,
    reported_failure: Option<(String, Instant)>,
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
        let origin = current.as_ref().map(|d| d.transport.clone());
        let prefs = self.preferences.clone();
        let hosts = self.pairing_hosts.clone();
        let needs_wireless_standby = self
            .selected
            .as_ref()
            .is_some_and(|d| !is_wireless(&d.transport) && d.backup_transports().is_empty());
        let identity_due = self.connection_watch.identity_at.is_none_or(|t| {
            t.elapsed() > Duration::from_secs(if needs_wireless_standby { 8 } else { 120 })
        });
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
                        if is_wireless(&d.transport) {
                            let probe_ok = adb.probe_device_identity(
                                &d.transport,
                                &d.serial,
                                Duration::from_millis(1800),
                            );
                            if !probe_ok {
                                let recovered = stepx_companion::device::recover_transport(
                                    &adb,
                                    &d.transport,
                                    Duration::from_secs(4),
                                );
                                if let Ok(raw) =
                                    adb.run_timeout(&["devices", "-l"], Duration::from_secs(2))
                                {
                                    online_transports = Some(online_transport_ids(&raw));
                                    devices = parse_devices(&raw);
                                }
                                online = recovered.as_deref() == Some(d.transport.as_str());
                                if !online {
                                    if let Some(ids) = online_transports.as_mut() {
                                        ids.retain(|id| id != &d.transport);
                                    }
                                    stepx_companion::device::invalidate_verified_transport(
                                        &d.transport,
                                    );
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
                    let identity = if identity_due {
                        current
                            .as_ref()
                            .and_then(|d| reconnect::ensure_usb_wireless_standby(&adb, d))
                    } else {
                        None
                    };
                    // Warm up wireless standby while on USB so unplugging fails over seamlessly.
                    if let Some(d) = current.as_ref() {
                        if !is_wireless(&d.transport) {
                            let standby_ep = identity
                                .as_ref()
                                .map(|id| id.endpoint.as_str())
                                .filter(|ep| !ep.is_empty())
                                .or_else(|| {
                                    prefs
                                        .wireless
                                        .get(&d.serial)
                                        .map(|id| id.endpoint.as_str())
                                        .filter(|ep| !ep.is_empty())
                                });
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
                        })
                    } else {
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
        if let (Some(d), Some(mut identity)) = (&self.selected, snapshot.identity.clone()) {
            // Never overwrite remembered endpoint, guid, wlan_mac, or wifi_ip with empty values.
            if let Some(prior) = self.preferences.wireless.get(&d.serial) {
                if identity.endpoint.is_empty() && !prior.endpoint.is_empty() {
                    identity.endpoint = prior.endpoint.clone();
                }
                if identity.guid.is_empty() && !prior.guid.is_empty() {
                    identity.guid = prior.guid.clone();
                }
                if identity.wlan_mac.is_empty() && !prior.wlan_mac.is_empty() {
                    identity.wlan_mac = prior.wlan_mac.clone();
                }
                if identity.wifi_ip.is_empty() && !prior.wifi_ip.is_empty() {
                    identity.wifi_ip = prior.wifi_ip.clone();
                }
            }
            if self.preferences.wireless.get(&d.serial) != Some(&identity) {
                self.preferences.wireless.insert(d.serial.clone(), identity);
                if let Err(e) = self.preferences.save() {
                    eprintln!("Unable to remember wireless connection: {e}");
                }
            }
        }
        let online = origin.as_ref().is_some_and(|id| {
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
}
