//! Authenticated, bounded LAN reconnect. No subnet scan, pairing secret, or global ADB reset.
use crate::{
    adb::Adb,
    device::{parse_devices, Device, Transport},
    pairing::{validate_endpoint, Preferences, Service, WirelessIdentity},
};
use anyhow::Result;
use std::{net::IpAddr, time::Duration};

pub fn network_identity(adb: &Adb, device: &Device) -> Option<WirelessIdentity> {
    let raw=adb.run_timeout(&["-s", &device.transport, "shell", "getprop ro.serialno; getprop persist.adb.wifi.guid; getprop service.adb.tls.port; ip -o -4 addr show dev wlan0"],Duration::from_secs(3)).ok()?;
    let mut lines = raw.lines();
    if lines.next()?.trim() != device.serial {
        return None;
    }
    let guid = lines.next().unwrap_or("").trim();
    let port = lines
        .next()
        .unwrap_or("")
        .trim()
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0);
    let ip = lines.flat_map(str::split_whitespace).find_map(|s| {
        s.split_once('/')
            .and_then(|(ip, _)| ip.parse::<IpAddr>().ok())
    });
    let mut endpoint = match (ip, port) {
        (Some(ip), Some(port)) => std::net::SocketAddr::new(ip, port).to_string(),
        _ => String::new(),
    };
    let mut guid = guid.to_string();
    // Some production firmware does not expose ADB's TLS properties to shell.
    if endpoint.is_empty() {
        if let Ok(address) = validate_endpoint(&device.transport) {
            if Some(address.ip()) == ip {
                endpoint = device.transport.clone();
            }
        }
        if let Ok(services) = crate::discovery::discover_services(adb) {
            if let Some(service) = services.iter().find(|s| {
                s.kind == "_adb-tls-connect._tcp"
                    && validate_endpoint(&s.endpoint).is_ok_and(|a| Some(a.ip()) == ip)
                    && (s.name.starts_with(&format!("adb-{}-", device.serial))
                        || (!guid.is_empty() && s.name.starts_with(&guid)))
            }) {
                endpoint = service.endpoint.clone();
                if guid.is_empty() {
                    guid = service.name.clone();
                }
            }
        }
    }
    if guid.is_empty() && endpoint.is_empty() {
        return None;
    }
    Some(WirelessIdentity { guid, endpoint })
}

pub fn targets(prefs: &Preferences, services: &[Service], paired_hosts: &[IpAddr]) -> Vec<String> {
    let mut targets = Vec::new();
    for service in services
        .iter()
        .filter(|s| s.kind == "_adb-tls-connect._tcp")
    {
        let Ok(address) = validate_endpoint(&service.endpoint) else {
            continue;
        };
        let remembered = prefs.devices.iter().any(|d| {
            // ADB's GUID is stable while its TLS port and LAN address can change.
            let identity = prefs.wireless.get(&d.serial);
            let matches = |id: &str| {
                !id.is_empty()
                    && (service.name == id
                        || service.name == format!("adb-{id}")
                        || service.name.starts_with(&format!("adb-{id}-")))
            };
            matches(&d.serial) || identity.is_some_and(|i| matches(&i.guid))
        });
        if (remembered || paired_hosts.contains(&address.ip()))
            && !targets.contains(&service.endpoint)
        {
            targets.push(service.endpoint.clone());
        }
    }
    // A remembered address helps networks which suppress multicast. Never reuse a pairing port.
    for d in &prefs.devices {
        if let Some(i) = prefs.wireless.get(&d.serial) {
            if validate_endpoint(&i.endpoint).is_ok() && !targets.contains(&i.endpoint) {
                targets.push(i.endpoint.clone());
            }
        }
    }
    targets.truncate(4);
    targets
}

pub fn scan(
    adb: &Adb,
    prefs: &Preferences,
    paired_hosts: &[IpAddr],
) -> Result<(Vec<Transport>, Vec<Service>)> {
    let before = parse_devices(&adb.run_timeout(&["devices", "-l"], Duration::from_secs(3))?);
    let services = crate::discovery::discover_services(adb).unwrap_or_default();
    if !connect_remembered(adb, prefs, &services, paired_hosts, &before) {
        return Ok((before, services));
    }
    Ok((
        parse_devices(&adb.run_timeout(&["devices", "-l"], Duration::from_secs(3))?),
        services,
    ))
}

fn connect_remembered(
    adb: &Adb,
    prefs: &Preferences,
    services: &[Service],
    paired_hosts: &[IpAddr],
    before: &[Transport],
) -> bool {
    let mut attempted = false;
    for endpoint in targets(prefs, services, paired_hosts) {
        if before
            .iter()
            .any(|t| t.id == endpoint && t.state == "device")
        {
            continue;
        }
        attempted = true;
        if adb
            .run_timeout(&["connect", &endpoint], Duration::from_secs(2))
            .is_err()
        {
            continue;
        }
        if let Ok(serial) = adb.run_timeout(
            &["-s", &endpoint, "shell", "getprop ro.serialno"],
            Duration::from_secs(3),
        ) {
            let paired = validate_endpoint(&endpoint).is_ok_and(|a| paired_hosts.contains(&a.ip()));
            if prefs.devices.iter().any(|d| d.serial == serial.trim()) || paired {
                continue;
            }
        }
        // Only release this newly attempted endpoint; never touch other connected phones.
        if !before
            .iter()
            .any(|t| t.id == endpoint && t.state == "device")
        {
            let _ = adb.run_timeout(&["disconnect", &endpoint], Duration::from_secs(2));
        }
    }
    attempted
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn guid_reconnects_after_address_and_port_change_without_matching_another_phone() {
        let mut p = Preferences::default();
        p.remember("serial-1".into(), "PZ261A".into());
        p.wireless.insert(
            "serial-1".into(),
            WirelessIdentity {
                guid: "adb-guid17".into(),
                endpoint: "10.0.1.8:38101".into(),
            },
        );
        let service = |name: &str, kind: &str, endpoint: &str| Service {
            name: name.into(),
            kind: kind.into(),
            endpoint: endpoint.into(),
        };
        let list = vec![
            service("adb-guid17", "_adb-tls-connect._tcp", "10.0.1.9:40201"),
            service("adb-guid17", "_adb-tls-pairing._tcp", "10.0.1.9:39999"),
            service("adb-guid170", "_adb-tls-connect._tcp", "10.0.1.10:40201"),
        ];
        assert_eq!(
            targets(&p, &list, &[]),
            vec!["10.0.1.9:40201", "10.0.1.8:38101"]
        );
        p.devices.clear();
        assert!(targets(&p, &list, &[]).is_empty());
    }
    #[test]
    fn legacy_preferences_and_incomplete_wireless_state_are_supported() {
        let p: Preferences =
            serde_json::from_str(r#"{"devices":[],"selected_serial":null}"#).unwrap();
        assert!(p.wireless.is_empty());
        let services = vec![Service {
            name: "unrelated".into(),
            kind: "_adb-tls-connect._tcp".into(),
            endpoint: "10.0.1.3:40000".into(),
        }];
        assert!(targets(&p, &services, &[]).is_empty());
        assert_eq!(
            targets(&p, &services, &["10.0.1.3".parse().unwrap()]).len(),
            1
        );
    }
    #[cfg(unix)]
    #[test]
    fn reconnect_checks_serial_and_does_not_touch_existing_or_unremembered_phones() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adb");
        let log = dir.path().join("calls");
        let script=format!("#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nif [ \"$1\" = '-s' ]; then printf 'different-phone\\n'; fi\n",log.display());
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let adb = Adb { path };
        let mut prefs = Preferences::default();
        prefs.remember("expected".into(), "PZ261A".into());
        prefs.wireless.insert(
            "expected".into(),
            WirelessIdentity {
                guid: "stable".into(),
                endpoint: "10.0.1.5:33333".into(),
            },
        );
        let unknown = Service {
            name: "somebody-else".into(),
            kind: "_adb-tls-connect._tcp".into(),
            endpoint: "10.0.1.77:33333".into(),
        };
        connect_remembered(&adb, &prefs, &[unknown], &[], &[]);
        let calls = std::fs::read_to_string(&log).unwrap();
        assert!(calls.contains("connect 10.0.1.5:33333"));
        assert!(calls.contains("shell getprop ro.serialno"));
        assert!(calls.contains("disconnect 10.0.1.5:33333"));
        assert!(!calls.contains("10.0.1.77"));
        std::fs::write(&log, "").unwrap();
        let existing=parse_devices("List of devices attached\n10.0.1.5:33333 device product:pandora model:PZ261A device:pandora\n");
        connect_remembered(&adb, &prefs, &[], &[], &existing);
        assert!(std::fs::read_to_string(&log).unwrap().is_empty());
    }
    #[test]
    fn wireless_identity_survives_restart_alongside_legacy_device_preferences() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("devices.json");
        let mut prefs = Preferences::default();
        prefs.remember("serial".into(), "phone".into());
        prefs.wireless.insert(
            "serial".into(),
            WirelessIdentity {
                guid: "wireless-guid".into(),
                endpoint: "10.0.1.5:33333".into(),
            },
        );
        prefs.save_to(&path).unwrap();
        let restored: Preferences = serde_json::from_slice(&std::fs::read(&path).unwrap()).unwrap();
        assert_eq!(restored.wireless, prefs.wireless);
        assert_eq!(targets(&restored, &[], &[]), vec!["10.0.1.5:33333"]);
    }
    #[test]
    fn adb_mdns_conflict_suffix_does_not_hide_a_remembered_phone() {
        let services=crate::pairing::parse_services("List of discovered mdns services\nadb-serial-one-abc (2)\t_adb-tls-connect._tcp\t10.0.1.3:45000\n");
        let mut prefs = Preferences::default();
        prefs.remember("serial-one".into(), "PZ261A".into());
        assert_eq!(services.len(), 1);
        assert_eq!(targets(&prefs, &services, &[]), vec!["10.0.1.3:45000"]);
    }
}
