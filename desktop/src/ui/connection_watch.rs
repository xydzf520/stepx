//! Connection discovery runs independently of page errors and foreground work.
use super::*;
use stepx_companion::{pairing::WirelessIdentity, reconnect};
struct Snapshot {
    devices: Vec<Transport>,
    services: Vec<Service>,
    identity: Option<WirelessIdentity>,
}
#[derive(Default)]
pub(super) struct ConnectionWatch {
    job: Option<(Option<String>, mpsc::Receiver<Result<Snapshot>>)>,
    pending: Option<(Option<String>, Snapshot)>,
    misses: u8,
    last_backups: Vec<String>,
    pub detached_serial: Option<String>,
    pub disconnected_at: Option<Instant>,
    pub identity_at: Option<Instant>,
}
impl Companion {
    pub(super) fn begin_connection_watch(&mut self) {
        if self.demo_mode || self.connection_watch.job.is_some() {
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
            t.elapsed() > Duration::from_secs(if needs_wireless_standby { 8 } else { 20 })
        });
        if identity_due {
            self.connection_watch.identity_at = Some(Instant::now());
        }
        let (tx, rx) = mpsc::channel();
        self.connection_watch.job = Some((origin, rx));
        std::thread::spawn(move || {
            let result = (|| -> Result<Snapshot> {
                let adb = Adb::discover()?;
                let mut devices =
                    parse_devices(&adb.run_timeout(&["devices", "-l"], Duration::from_secs(3))?);
                let online = current.as_ref().is_some_and(|d| {
                    devices
                        .iter()
                        .any(|t| t.id == d.transport && t.state == "device")
                });
                if online {
                    let identity = if identity_due {
                        current
                            .as_ref()
                            .and_then(|d| reconnect::network_identity(&adb, d))
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
                                let already_connected = devices.iter().any(|t| {
                                    t.state == "device"
                                        && (t.id == ep
                                            || t.backup_transports().iter().any(|b| b == ep))
                                });
                                if !already_connected && identity_due {
                                    let _ = adb
                                        .run_timeout(&["connect", ep], Duration::from_millis(1800));
                                    if let Ok(raw) =
                                        adb.run_timeout(&["devices", "-l"], Duration::from_secs(2))
                                    {
                                        devices = parse_devices(&raw);
                                    }
                                }
                                if devices.iter().any(|t| t.id == ep && t.state == "device") {
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
                        devices,
                        services: vec![],
                        identity,
                    })
                } else {
                    let has_online_failover = current.as_ref().is_some_and(|d| {
                        let known_backups = d.backup_transports();
                        devices.iter().any(|t| {
                            t.state == "device"
                                && (known_backups.contains(&t.id)
                                    || stepx_companion::device::canonical_serial_from_transport(
                                        &t.id,
                                    )
                                    .as_deref()
                                        == Some(d.serial.as_str())
                                    || (!t.model.is_empty() && t.model == d.model))
                        })
                    });
                    if has_online_failover {
                        Ok(Snapshot {
                            devices,
                            services: vec![],
                            identity: None,
                        })
                    } else {
                        let (devices, services) = reconnect::scan(&adb, &prefs, &hosts)?;
                        Ok(Snapshot {
                            devices,
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
            return;
        };
        if let (Some(d), Some(mut identity)) = (&self.selected, snapshot.identity.clone()) {
            // A Wi-Fi transition may temporarily remove the port, not the remembered peer.
            if identity.endpoint.is_empty() {
                if let Some(prior) = self.preferences.wireless.get(&d.serial) {
                    if identity.guid == prior.guid {
                        identity.endpoint = prior.endpoint.clone();
                    }
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
            snapshot
                .devices
                .iter()
                .any(|d| &d.id == id && d.state == "device")
        });
        if online {
            self.connection_watch.misses = 0;
            self.connection_watch.disconnected_at = None;
        } else {
            self.connection_watch.misses = self.connection_watch.misses.saturating_add(1);
        }
        let has_online_failover = self.selected.as_ref().is_some_and(|selected| {
            let known_backups = selected.backup_transports();
            snapshot.devices.iter().any(|t| {
                t.state == "device"
                    && (known_backups.contains(&t.id)
                        || stepx_companion::device::canonical_serial_from_transport(&t.id)
                            .as_deref()
                            == Some(selected.serial.as_str())
                        || (!t.model.is_empty() && t.model == selected.model))
            })
        });
        // Two absent snapshots avoid clearing a connection during ADB discovery. Tasks and drafts survive.
        if origin.is_some() && !online && !has_online_failover && self.connection_watch.misses < 2 {
            return;
        }
        if !online {
            if self.busy {
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
                if let Some(active) = new_devices
                    .iter()
                    .find(|t| t.id == selected.transport && t.state == "device")
                {
                    let active_backups = active.backup_transports();
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
