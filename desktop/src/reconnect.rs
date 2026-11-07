//! Authenticated, bounded LAN reconnect. No subnet scan, pairing secret, or global ADB reset.
use crate::{
    adb::Adb,
    device::{parse_devices, Device, Transport},
    pairing::{validate_endpoint, Preferences, Service, WirelessIdentity},
};
use anyhow::Result;
use std::{
    net::{IpAddr, SocketAddr},
    process::Command,
    sync::{Mutex, OnceLock},
    time::Duration,
};

pub type NetworkIdentity = WirelessIdentity;

pub fn normalize_mac(raw: &str) -> Option<String> {
    for token in raw.split_whitespace() {
        let trimmed = token.trim().to_ascii_lowercase();
        let parts: Vec<&str> = trimmed.split(':').collect();
        if parts.len() == 6
            && parts
                .iter()
                .all(|p| p.len() == 2 && p.bytes().all(|b| b.is_ascii_hexdigit()))
            && trimmed != "02:00:00:00:00:00"
            && trimmed != "00:00:00:00:00:00"
            && trimmed != "ff:ff:ff:ff:ff:ff"
        {
            return Some(trimmed);
        }
    }
    None
}

fn parse_port(raw: &str) -> Option<u16> {
    raw.trim().parse::<u16>().ok().filter(|p| *p > 0)
}

fn extract_ipv4_from_addr_show(raw: &str) -> Option<IpAddr> {
    raw.split_whitespace().find_map(|s| {
        s.split_once('/')
            .and_then(|(ip, _)| ip.parse::<IpAddr>().ok())
            .filter(|ip| !ip.is_unspecified() && !ip.is_loopback())
    })
}

pub fn parse_network_identity_output(raw: &str, expected_serial: &str) -> Option<WirelessIdentity> {
    if raw.contains("---") {
        let sections: Vec<&str> = raw.split("---").map(str::trim).collect();
        let serial = sections.first().copied().unwrap_or("");
        if serial != expected_serial {
            return None;
        }
        let guid = sections.get(1).copied().unwrap_or("").trim().to_string();
        let tls_port = sections.get(2).copied().and_then(parse_port);
        let tcp_port = sections
            .get(3)
            .copied()
            .and_then(parse_port)
            .or_else(|| sections.get(4).copied().and_then(parse_port));
        let ip = sections
            .get(5)
            .copied()
            .and_then(extract_ipv4_from_addr_show);
        let wlan_mac = sections
            .get(6)
            .copied()
            .and_then(|s| s.lines().find_map(normalize_mac))
            .unwrap_or_default();
        let chosen_port = tls_port.or(tcp_port);
        let endpoint = match (ip, chosen_port) {
            (Some(ip), Some(port)) => SocketAddr::new(ip, port).to_string(),
            _ => String::new(),
        };
        let wifi_ip = ip.map(|a| a.to_string()).unwrap_or_default();
        Some(WirelessIdentity {
            guid,
            endpoint,
            wlan_mac,
            wifi_ip,
        })
    } else {
        let mut lines = raw.lines();
        if lines.next()?.trim() != expected_serial {
            return None;
        }
        let guid = lines.next().unwrap_or("").trim().to_string();
        let port = lines.next().and_then(parse_port);
        let rest: Vec<&str> = lines.collect();
        let rest_joined = rest.join("\n");
        let ip = extract_ipv4_from_addr_show(&rest_joined);
        let wlan_mac = rest
            .iter()
            .copied()
            .find_map(normalize_mac)
            .unwrap_or_default();
        let endpoint = match (ip, port) {
            (Some(ip), Some(port)) => SocketAddr::new(ip, port).to_string(),
            _ => String::new(),
        };
        let wifi_ip = ip.map(|a| a.to_string()).unwrap_or_default();
        Some(WirelessIdentity {
            guid,
            endpoint,
            wlan_mac,
            wifi_ip,
        })
    }
}

pub fn network_identity(adb: &Adb, device: &Device) -> Option<WirelessIdentity> {
    let transport = device
        .resolve_active_transport(adb)
        .unwrap_or_else(|_| device.transport.clone());
    let raw = adb
        .run_timeout(
            &[
                "-s",
                &transport,
                "shell",
                "getprop ro.serialno; echo ---; getprop persist.adb.wifi.guid; echo ---; getprop service.adb.tls.port; echo ---; getprop service.adb.tcp.port; echo ---; getprop persist.adb.tcp.port; echo ---; ip -o -4 addr show dev wlan0 2>/dev/null || ip -o -4 addr show dev wlan1 2>/dev/null || true; echo ---; cat /sys/class/net/wlan0/address 2>/dev/null || ip -o link show dev wlan0 2>/dev/null || true",
            ],
            Duration::from_secs(3),
        )
        .ok()?;
    let mut id = parse_network_identity_output(&raw, &device.serial)?;
    let ip: Option<IpAddr> = id.wifi_ip.parse().ok();

    // Some production firmware does not expose ADB's TLS properties to shell.
    if id.endpoint.is_empty() {
        let backups = device.backup_transports();
        for cand_transport in std::iter::once(&transport)
            .chain(std::iter::once(&device.transport))
            .chain(backups.iter())
        {
            if let Ok(address) = validate_endpoint(cand_transport) {
                if ip.is_none() || Some(address.ip()) == ip {
                    id.endpoint = cand_transport.clone();
                    if id.wifi_ip.is_empty() {
                        id.wifi_ip = address.ip().to_string();
                    }
                    break;
                }
            }
        }
        if id.endpoint.is_empty() {
            if let Ok(services) = crate::discovery::discover_services(adb) {
                if let Some(service) = services.iter().find(|s| {
                    if s.kind != "_adb-tls-connect._tcp" {
                        return false;
                    }
                    let Ok(addr) = validate_endpoint(&s.endpoint) else {
                        return false;
                    };
                    let ip_matches = ip.is_none() || Some(addr.ip()) == ip;
                    let name_matches = s.name.starts_with(&format!("adb-{}-", device.serial))
                        || s.name == format!("adb-{}", device.serial)
                        || (!id.guid.is_empty()
                            && (s.name == id.guid
                                || s.name.starts_with(&id.guid)
                                || s.name.starts_with(&format!("adb-{}", id.guid))));
                    ip_matches && name_matches
                }) {
                    id.endpoint = service.endpoint.clone();
                    if id.guid.is_empty() {
                        id.guid = service.name.clone();
                    }
                    if id.wifi_ip.is_empty() {
                        if let Ok(addr) = validate_endpoint(&id.endpoint) {
                            id.wifi_ip = addr.ip().to_string();
                        }
                    }
                }
            }
        }
    }
    if id.guid.is_empty()
        && id.endpoint.is_empty()
        && id.wlan_mac.is_empty()
        && id.wifi_ip.is_empty()
    {
        return None;
    }
    Some(id)
}

pub fn parse_arp_output_for_mac(arp_text: &str, mac: &str) -> Option<IpAddr> {
    let target_mac = normalize_mac(mac)?;
    for line in arp_text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 4 {
            continue;
        }
        let Ok(ip) = tokens[0].parse::<IpAddr>() else {
            continue;
        };
        if ip.is_unspecified() || ip.is_loopback() {
            continue;
        }
        // /proc/net/arp format: IP HW_type Flags HW_address Mask Device
        if tokens[1].starts_with("0x") && tokens[2].starts_with("0x") {
            if tokens[2] == "0x0" {
                continue;
            }
            if normalize_mac(tokens[3]).as_deref() == Some(target_mac.as_str()) {
                return Some(ip);
            }
            continue;
        }
        // `ip -4 neigh show` format: 10.0.1.28 dev wlan0 lladdr aa:bb:cc:dd:ee:ff REACHABLE
        if let Some(last) = tokens.last() {
            if last.eq_ignore_ascii_case("FAILED") || last.eq_ignore_ascii_case("INCOMPLETE") {
                continue;
            }
        }
        if tokens[1..]
            .iter()
            .any(|t| normalize_mac(t).as_deref() == Some(target_mac.as_str()))
        {
            return Some(ip);
        }
    }
    None
}

pub fn resolve_ip_by_mac(mac: &str) -> Option<IpAddr> {
    let target_mac = normalize_mac(mac)?;
    if let Ok(arp_table) = std::fs::read_to_string("/proc/net/arp") {
        if let Some(ip) = parse_arp_output_for_mac(&arp_table, &target_mac) {
            return Some(ip);
        }
    }
    let mut cmd = Command::new("ip");
    cmd.args(["-4", "neigh", "show"]);
    crate::resources::quiet_command(&mut cmd);
    if let Ok(out) = crate::adb::run_process(cmd, None, Duration::from_millis(1500)) {
        if out.status.success() {
            let text = String::from_utf8_lossy(&out.stdout);
            if let Some(ip) = parse_arp_output_for_mac(&text, &target_mac) {
                return Some(ip);
            }
        }
    }
    None
}

/// Read/connect only: background checks must never restart adbd or change Wi-Fi settings.
pub fn ensure_usb_wireless_standby(adb: &Adb, device: &Device) -> Option<NetworkIdentity> {
    let mut id = network_identity(adb, device)?;
    let mut candidates = vec![id.endpoint.clone(), device.transport.clone()];
    candidates.extend(device.backup_transports());
    if let Some(endpoint) = verified_wireless_endpoint(adb, device, &candidates) {
        id.wifi_ip = validate_endpoint(&endpoint).ok()?.ip().to_string();
        id.endpoint = endpoint;
        return Some(id);
    }
    // A remembered/property endpoint can expire when Android rotates its TLS port.
    // Rediscover even when the old property was nonempty; verify hardware before use.
    let services = crate::discovery::discover_services(adb).ok()?;
    let mut prefs = Preferences::default();
    prefs.remember(device.serial.clone(), device.model.clone());
    prefs.wireless.insert(
        device.serial.clone(),
        WirelessIdentity {
            guid: id.guid.clone(),
            ..Default::default()
        },
    );
    let endpoint = verified_wireless_endpoint(adb, device, &targets(&prefs, &services, &[]))?;
    id.wifi_ip = validate_endpoint(&endpoint).ok()?.ip().to_string();
    id.endpoint = endpoint;
    Some(id)
}

fn verified_wireless_endpoint(adb: &Adb, device: &Device, candidates: &[String]) -> Option<String> {
    let mut seen = Vec::new();
    for endpoint in candidates {
        if validate_endpoint(endpoint).is_err() || seen.contains(endpoint) {
            continue;
        }
        seen.push(endpoint.clone());
        let _ = adb.run_timeout(&["connect", endpoint], Duration::from_secs(2));
        // adb connect can exit zero with "failed to connect". Only a serial read is proof.
        if adb
            .run_timeout(
                &["-s", endpoint, "shell", "getprop", "ro.serialno"],
                Duration::from_secs(2),
            )
            .is_ok_and(|s| s.trim() == device.serial)
        {
            let mut backups = device.backup_transports();
            if endpoint != &device.transport && !backups.contains(endpoint) {
                backups.push(endpoint.clone());
                device.set_backup_transports(backups);
            }
            return Some(endpoint.clone());
        }
    }
    None
}

/// Explicit USB setup only. Existing wireless listeners are reused, never reset.
pub fn enable_usb_wireless_bridge(adb: &Adb, device: &Device) -> Result<NetworkIdentity> {
    static SETUP: OnceLock<Mutex<()>> = OnceLock::new();
    let _lock = SETUP
        .get_or_init(|| Mutex::new(()))
        .lock()
        .map_err(|_| anyhow::anyhow!("无线连接设置正在恢复，请稍后重试"))?;
    if let Some(id) = ensure_usb_wireless_standby(adb, device) {
        return Ok(id);
    }
    anyhow::ensure!(
        !crate::adb::is_wireless_transport(&device.transport),
        "当前无线连接尚未验证，请刷新发现或使用手机系统配对码；USB 设置需要数据线连接"
    );
    let verify_usb = || -> Result<()> {
        let serial = adb.run_timeout(
            &["-s", &device.transport, "shell", "getprop", "ro.serialno"],
            Duration::from_secs(3),
        )?;
        anyhow::ensure!(
            serial.trim() == device.serial,
            "USB 手机身份发生变化，请重新选择设备"
        );
        Ok(())
    };
    verify_usb()?;
    let mut id = network_identity(adb, device)
        .ok_or_else(|| anyhow::anyhow!("未能读取手机无线地址，请确认手机已连接 Wi-Fi"))?;
    anyhow::ensure!(!id.wifi_ip.is_empty(), "请先将手机连接至与电脑互通的 Wi-Fi");
    // Never restart a running listener because authorization/discovery is still pending.
    anyhow::ensure!(
        id.endpoint.is_empty(),
        "手机已开启无线通道，但尚未验证连接。请在手机允许调试或使用无线调试配对码，再重新扫描"
    );
    let enabled = adb.run_timeout(
        &[
            "-s",
            &device.transport,
            "shell",
            "settings",
            "get",
            "global",
            "adb_wifi_enabled",
        ],
        Duration::from_secs(3),
    )?;
    if enabled.trim() != "1" {
        verify_usb()?;
        adb.run_timeout(
            &[
                "-s",
                &device.transport,
                "shell",
                "settings",
                "put",
                "global",
                "adb_wifi_enabled",
                "1",
            ],
            Duration::from_secs(3),
        )?;
    }
    for _ in 0..6 {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(verified) = ensure_usb_wireless_standby(adb, device) {
            return Ok(verified);
        }
        if let Some(fresh) = network_identity(adb, device) {
            id = fresh;
        }
        if !id.endpoint.is_empty() {
            break;
        }
    }
    anyhow::ensure!(
        id.endpoint.is_empty(),
        "手机无线调试已开启，请完成系统配对码授权后重新扫描；已保留当前无线端口"
    );
    // TLS may be waiting for the phone's network approval dialog. Do not reset it.
    anyhow::ensure!(
        adb.run_timeout(
            &[
                "-s",
                &device.transport,
                "shell",
                "settings",
                "get",
                "global",
                "adb_wifi_enabled"
            ],
            Duration::from_secs(3)
        )?
        .trim()
            != "1",
        "无线调试正在等待手机确认，请允许当前 Wi-Fi 并使用系统配对码"
    );
    verify_usb()?;
    adb.run_timeout(
        &["-s", &device.transport, "tcpip", "5555"],
        Duration::from_secs(5),
    )?;
    // adbd restarted: discard all pre-restart addresses, and wait for fresh identity.
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(verified) = ensure_usb_wireless_standby(adb, device) {
            return Ok(verified);
        }
    }
    anyhow::bail!("无线服务已请求启动，但连接尚未通过手机身份核验；请保持 USB 连接后重新扫描")
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
            let old_addr = validate_endpoint(&i.endpoint).ok();
            if old_addr.is_some() && !targets.contains(&i.endpoint) {
                targets.push(i.endpoint.clone());
            }
            if !i.wlan_mac.is_empty() {
                if let Some(arp_ip) = resolve_ip_by_mac(&i.wlan_mac) {
                    if old_addr.map(|a| a.ip()) != Some(arp_ip) {
                        let arp_5555 = SocketAddr::new(arp_ip, 5555).to_string();
                        if !targets.contains(&arp_5555) {
                            targets.push(arp_5555);
                        }
                        if let Some(addr) = old_addr {
                            let arp_old_port = SocketAddr::new(arp_ip, addr.port()).to_string();
                            if !targets.contains(&arp_old_port) {
                                targets.push(arp_old_port);
                            }
                        }
                    }
                }
            }
        }
    }
    targets.truncate(6);
    targets
}

/// Check the current phone before attempting recovery. A healthy connection is
/// never disconnected, reset, or assigned a guessed wireless port.
pub fn check_and_recover(
    adb: &Adb,
    device: Option<&Device>,
    prefs: &Preferences,
    paired_hosts: &[IpAddr],
) -> Result<(Vec<Transport>, Vec<Service>)> {
    if let Some(device) = device {
        let endpoint = device.active_transport_cached();
        if adb.probe_device_identity(&endpoint, &device.serial, Duration::from_secs(3)) {
            let raw = adb.run_timeout(&["devices", "-l"], Duration::from_secs(3))?;
            return Ok((parse_devices(&raw), vec![]));
        }
    }
    // Only remembered identities and fresh discovery are eligible for recovery.
    scan(adb, prefs, paired_hosts)
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
        if before
            .iter()
            .any(|t| t.id == endpoint && t.state == "unauthorized")
        {
            continue;
        }
        if before
            .iter()
            .any(|t| t.id == endpoint && t.state == "offline")
        {
            // Only this already-offline endpoint; never a server-wide reconnect.
            let _ = adb.run_timeout(&["-s", &endpoint, "reconnect"], Duration::from_millis(1200));
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
    #[cfg(unix)]
    fn wireless_fixture(mode: &str) -> (tempfile::TempDir, Adb, Device) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adb");
        let serial = format!("WIRELESS_{mode}");
        let script = r#"#!/usr/bin/env python3
import sys, pathlib, json
root=pathlib.Path(__file__).parent
args=sys.argv[1:]
with (root/'calls').open('a') as f: f.write(json.dumps(args)+'\n')
mode='__MODE__'; serial='__SERIAL__'
started=(root/'started').exists()
ip='192.0.2.99' if started else '192.0.2.44'
port='5555' if started else ('41001' if mode!='bootstrap' else '')
endpoint=ip+':'+port
if args==['devices','-l']:
 print('List of devices attached\n'+serial+' device model:PZ261A product:pandora')
elif args==['mdns','services']:
 print('List of discovered mdns services')
 if mode=='rotated': print('adb-'+serial+'-guid _adb-tls-connect._tcp 192.0.2.77:42002')
elif args and args[0]=='connect': print('connected to '+args[1])
elif len(args)>2 and args[0]=='-s':
 target=args[1]; command=' '.join(args[2:])
 if command=='tcpip 5555': (root/'started').touch(); print('restarting in TCP mode port: 5555')
 elif 'ro.serialno' in command and '---' in command:
  print(serial+'\n---\nadb-'+serial+'-guid\n---\n'+port+'\n---\n\n---\n\n---\ninet '+ip+'/24\n---\n')
 elif command=='shell getprop ro.serialno':
  valid = target==serial or (mode=='existing' and target==endpoint) or (mode=='rotated' and target=='192.0.2.77:42002') or (mode=='bootstrap' and started and target==endpoint)
  print(serial if valid else 'DIFFERENT_PHONE')
 elif command=='shell settings get global adb_wifi_enabled': print('0' if mode=='bootstrap' else '1')
 elif command=='shell settings put global adb_wifi_enabled 1': pass
 else: print(serial)
"#.replace("__MODE__", mode).replace("__SERIAL__", &serial);
        std::fs::write(&path, script).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
        let device = Device {
            serial: serial.clone(),
            transport: serial,
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        (dir, Adb { path }, device)
    }

    #[cfg(unix)]
    #[test]
    fn checking_a_healthy_connection_never_resets_or_disconnects_it() {
        let (dir, adb, device) = wireless_fixture("existing");
        for _ in 0..3 {
            check_and_recover(&adb, Some(&device), &Preferences::default(), &[]).unwrap();
        }
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        for command in ["disconnect", "reconnect", "tcpip", "settings", "mdns"] {
            assert!(!calls.contains(command), "healthy check ran {command}");
        }
    }

    #[cfg(unix)]
    #[test]
    fn repeated_wireless_setup_reuses_listener_without_settings_or_restart() {
        let (dir, adb, device) = wireless_fixture("existing");
        for _ in 0..3 {
            assert_eq!(
                enable_usb_wireless_bridge(&adb, &device).unwrap().endpoint,
                "192.0.2.44:41001"
            );
        }
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert!(!calls.contains("tcpip"));
        assert!(!calls.contains("settings"));
    }

    #[cfg(unix)]
    #[test]
    fn stale_tls_address_is_replaced_with_fresh_discovery_verified_on_phone() {
        let (dir, adb, device) = wireless_fixture("rotated");
        let id = enable_usb_wireless_bridge(&adb, &device).unwrap();
        assert_eq!(id.endpoint, "192.0.2.77:42002");
        assert_eq!(id.wifi_ip, "192.0.2.77");
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert!(!calls.contains("tcpip"));
        assert!(!calls.contains("settings"));
    }

    #[cfg(unix)]
    #[test]
    fn unreachable_existing_listener_is_not_reset_or_reported_as_success() {
        let (dir, adb, device) = wireless_fixture("unreachable");
        assert!(enable_usb_wireless_bridge(&adb, &device).is_err());
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert!(!calls.contains("tcpip"));
        assert!(!calls.contains("settings"));
    }

    #[cfg(unix)]
    #[test]
    fn background_check_does_not_enable_or_restart_missing_wireless_listener() {
        let (dir, adb, device) = wireless_fixture("bootstrap");
        assert!(ensure_usb_wireless_standby(&adb, &device).is_none());
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert!(!calls.contains("tcpip"));
        assert!(!calls.contains("settings"));
    }

    #[cfg(unix)]
    #[test]
    fn usb_bootstrap_uses_post_restart_ip_and_repeated_click_does_not_restart_again() {
        let (dir, adb, device) = wireless_fixture("bootstrap");
        for _ in 0..2 {
            assert_eq!(
                enable_usb_wireless_bridge(&adb, &device).unwrap().endpoint,
                "192.0.2.99:5555"
            );
        }
        let calls = std::fs::read_to_string(dir.path().join("calls")).unwrap();
        assert_eq!(calls.matches("tcpip").count(), 1);
        assert!(!calls.contains("192.0.2.44:5555"));
    }

    #[test]
    fn guid_reconnects_after_address_and_port_change_without_matching_another_phone() {
        let mut p = Preferences::default();
        p.remember("serial-1".into(), "PZ261A".into());
        p.wireless.insert(
            "serial-1".into(),
            WirelessIdentity {
                guid: "adb-guid17".into(),
                endpoint: "10.0.1.8:38101".into(),
                ..Default::default()
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
                ..Default::default()
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
                ..Default::default()
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
    #[test]
    fn parse_network_identity_extracts_tls_or_tcp_port_and_mac() {
        let raw_tls = "ZYPZ261A01\n---\nadb-ZYPZ261A01-xyz\n---\n39123\n---\n5555\n---\n\n---\n12: wlan0    inet 10.0.1.88/24 brd 10.0.1.255 scope global wlan0\n---\nAA:BB:CC:11:22:33\n";
        let id_tls = parse_network_identity_output(raw_tls, "ZYPZ261A01").expect("tls identity");
        assert_eq!(id_tls.guid, "adb-ZYPZ261A01-xyz");
        assert_eq!(id_tls.endpoint, "10.0.1.88:39123");
        assert_eq!(id_tls.wifi_ip, "10.0.1.88");
        assert_eq!(id_tls.wlan_mac, "aa:bb:cc:11:22:33");

        let raw_tcp = "ZYPZ261A01\n---\n\n---\n-1\n---\n5555\n---\n\n---\n12: wlan0    inet 192.168.1.42/24 brd 192.168.1.255 scope global wlan0\n---\n02:00:00:00:00:00\n";
        let id_tcp = parse_network_identity_output(raw_tcp, "ZYPZ261A01").expect("tcp identity");
        assert_eq!(id_tcp.endpoint, "192.168.1.42:5555");
        assert_eq!(id_tcp.wifi_ip, "192.168.1.42");
        assert!(id_tcp.wlan_mac.is_empty());
    }
    #[test]
    fn parse_arp_output_resolves_mac_across_proc_arp_and_ip_neigh() {
        let proc_arp = "\
IP address       HW type     Flags       HW address            Mask     Device
10.0.1.1         0x1         0x2         00:11:22:33:44:55     *        wlan0
10.0.1.76        0x1         0x0         aa:bb:cc:dd:ee:ff     *        wlan0
10.0.1.77        0x1         0x2         AA:BB:CC:DD:EE:FF     *        wlan0
";
        assert_eq!(
            parse_arp_output_for_mac(proc_arp, "aa:bb:cc:dd:ee:ff"),
            Some("10.0.1.77".parse().unwrap())
        );

        let ip_neigh = "\
10.0.1.1 dev wlan0 lladdr 00:11:22:33:44:55 REACHABLE
10.0.1.91 dev wlan0 lladdr aa:bb:cc:dd:ee:ff STALE
";
        assert_eq!(
            parse_arp_output_for_mac(ip_neigh, "AA:BB:CC:DD:EE:FF"),
            Some("10.0.1.91".parse().unwrap())
        );
    }
}
