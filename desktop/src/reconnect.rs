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
        let cleaned = token
            .trim_matches(|c: char| matches!(c, '(' | ')' | '[' | ']' | ',' | ';'))
            .to_ascii_lowercase();
        let sep = if cleaned.contains(':') {
            ':'
        } else if cleaned.contains('-') {
            '-'
        } else {
            continue;
        };
        let parts: Vec<&str> = cleaned.split(sep).collect();
        if parts.len() != 6
            || !parts
                .iter()
                .all(|p| (1..=2).contains(&p.len()) && p.bytes().all(|b| b.is_ascii_hexdigit()))
        {
            continue;
        }
        let mut bytes = [0u8; 6];
        let mut ok = true;
        for (i, part) in parts.iter().enumerate() {
            match u8::from_str_radix(part, 16) {
                Ok(v) => bytes[i] = v,
                Err(_) => {
                    ok = false;
                    break;
                }
            }
        }
        if !ok {
            continue;
        }
        let normalized = format!(
            "{:02x}:{:02x}:{:02x}:{:02x}:{:02x}:{:02x}",
            bytes[0], bytes[1], bytes[2], bytes[3], bytes[4], bytes[5]
        );
        // Reject Android dummy MAC (02:00:00:00:00:00), all-zero, broadcast, and multicast MACs
        if normalized != "02:00:00:00:00:00"
            && normalized != "00:00:00:00:00:00"
            && normalized != "ff:ff:ff:ff:ff:ff"
            && (bytes[0] & 0x01) == 0
        {
            return Some(normalized);
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
    // Only trust `transport` itself (proven live by the shell query above); never
    // copy unverified `device.transport` or `device.backup_transports()` when live
    // USB `getprop` reported both TLS and TCP ports disabled (`<= 0` or unset).
    if id.endpoint.is_empty() {
        if let Ok(address) = validate_endpoint(&transport) {
            if ip.is_none() || Some(address.ip()) == ip {
                id.endpoint = transport.clone();
                if id.wifi_ip.is_empty() {
                    id.wifi_ip = address.ip().to_string();
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

fn usable_arp_ip(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(v4) => {
            !v4.is_unspecified()
                && !v4.is_loopback()
                && !v4.is_multicast()
                && !v4.is_broadcast()
        }
        IpAddr::V6(v6) => {
            !v6.is_unspecified()
                && !v6.is_loopback()
                && !v6.is_multicast()
                && (v6.segments()[0] & 0xffc0) != 0xfe80
        }
    }
}

pub fn parse_arp_output_for_mac(arp_text: &str, mac: &str) -> Option<IpAddr> {
    let target_mac = normalize_mac(mac)?;
    for line in arp_text.lines() {
        let tokens: Vec<&str> = line.split_whitespace().collect();
        if tokens.len() < 3 {
            continue;
        }

        // macOS / BSD `arp -an` format:
        // `? (192.168.1.55) at a:b:cc:dd:ee:f on en0 ifscope [ethernet]`
        if tokens.len() >= 4 && tokens[2].eq_ignore_ascii_case("at") {
            if tokens[3].eq_ignore_ascii_case("(incomplete)")
                || line.to_ascii_lowercase().contains("(incomplete)")
            {
                continue;
            }
            if let Some(ip_str) = tokens[1].strip_prefix('(').and_then(|s| s.strip_suffix(')')) {
                if let Ok(ip) = ip_str.parse::<IpAddr>() {
                    if usable_arp_ip(ip)
                        && normalize_mac(tokens[3]).as_deref() == Some(target_mac.as_str())
                    {
                        return Some(ip);
                    }
                }
            }
            continue;
        }

        let Ok(ip) = tokens[0].parse::<IpAddr>() else {
            continue;
        };
        if !usable_arp_ip(ip) {
            continue;
        }

        // Linux `/proc/net/arp` format: IP HW_type Flags HW_address Mask Device
        if tokens.len() >= 4 && tokens[1].starts_with("0x") && tokens[2].starts_with("0x") {
            if tokens[2] == "0x0" {
                continue;
            }
            if normalize_mac(tokens[3]).as_deref() == Some(target_mac.as_str()) {
                return Some(ip);
            }
            continue;
        }

        // Windows `arp -a` format:
        // `  192.168.1.88          aa-bb-cc-dd-ee-ff     dynamic` (or localized `动态` / `无效`)
        if tokens.len() == 3 {
            let entry_type = tokens[2];
            if entry_type.eq_ignore_ascii_case("invalid")
                || entry_type.eq_ignore_ascii_case("incomplete")
                || entry_type == "无效"
            {
                continue;
            }
            if normalize_mac(tokens[1]).as_deref() == Some(target_mac.as_str()) {
                return Some(ip);
            }
            continue;
        }

        // Linux `ip -4 neigh show` format: `10.0.1.28 dev wlan0 lladdr aa:bb:cc:dd:ee:ff REACHABLE`
        if let Some(last) = tokens.last() {
            if last.eq_ignore_ascii_case("FAILED")
                || last.eq_ignore_ascii_case("INCOMPLETE")
                || last.eq_ignore_ascii_case("invalid")
                || *last == "无效"
            {
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

    #[cfg(target_os = "linux")]
    {
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
    }

    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("arp");
        cmd.arg("-an");
        crate::resources::quiet_command(&mut cmd);
        if let Ok(out) = crate::adb::run_process(cmd, None, Duration::from_millis(1500)) {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Some(ip) = parse_arp_output_for_mac(&text, &target_mac) {
                    return Some(ip);
                }
            }
        }
    }

    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mut cmd = Command::new("arp");
        cmd.arg("-a").creation_flags(CREATE_NO_WINDOW);
        crate::resources::quiet_command(&mut cmd);
        if let Ok(out) = crate::adb::run_process(cmd, None, Duration::from_millis(1500)) {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Some(ip) = parse_arp_output_for_mac(&text, &target_mac) {
                    return Some(ip);
                }
            }
        }
    }

    #[cfg(all(unix, not(target_os = "linux"), not(target_os = "macos")))]
    {
        let mut cmd = Command::new("arp");
        cmd.arg("-an");
        crate::resources::quiet_command(&mut cmd);
        if let Ok(out) = crate::adb::run_process(cmd, None, Duration::from_millis(1500)) {
            if out.status.success() {
                let text = String::from_utf8_lossy(&out.stdout);
                if let Some(ip) = parse_arp_output_for_mac(&text, &target_mac) {
                    return Some(ip);
                }
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
            crate::device::record_verified_serial_for_transport(&device.serial, endpoint);
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
    let backups = device.backup_transports();
    let mut usb_candidates: Vec<String> = Vec::new();
    for cand in [device.transport.as_str(), device.serial.as_str()]
        .into_iter()
        .chain(backups.iter().map(String::as_str))
    {
        let trimmed = cand.trim();
        if !trimmed.is_empty()
            && !trimmed.starts_with("model:")
            && !crate::adb::is_wireless_transport(trimmed)
            && !usb_candidates.iter().any(|c| c == trimmed)
        {
            usb_candidates.push(trimmed.to_string());
        }
    }
    let mut usb_transport: Option<String> = None;
    let mut identity_mismatch = false;
    let mut last_usb_err: Option<anyhow::Error> = None;
    for cand in &usb_candidates {
        match adb.run_timeout(
            &["-s", cand, "shell", "getprop", "ro.serialno"],
            Duration::from_secs(3),
        ) {
            Ok(serial) if serial.trim() == device.serial => {
                usb_transport = Some(cand.clone());
                break;
            }
            Ok(_) => {
                identity_mismatch = true;
            }
            Err(err) => {
                last_usb_err = Some(err);
            }
        }
    }
    let usb_transport = match usb_transport {
        Some(t) => t,
        None if identity_mismatch && !crate::adb::is_wireless_transport(&device.transport) => {
            anyhow::bail!("USB 手机身份发生变化，请重新选择设备");
        }
        None if crate::adb::is_wireless_transport(&device.transport) || usb_candidates.is_empty() => {
            anyhow::bail!(
                "当前无线连接尚未验证，请刷新发现或使用手机系统配对码；USB 设置需要数据线连接"
            );
        }
        None if identity_mismatch => {
            anyhow::bail!("USB 手机身份发生变化，请重新选择设备");
        }
        None => {
            return Err(last_usb_err.unwrap_or_else(|| {
                anyhow::anyhow!(
                    "当前无线连接尚未验证，请刷新发现或使用手机系统配对码；USB 设置需要数据线连接"
                )
            }));
        }
    };
    if usb_transport != device.transport {
        crate::device::invalidate_verified_transport(&device.transport);
        crate::device::promote_active_transport(&device.serial, &usb_transport);
    }
    let mut usb_device = device.clone();
    usb_device.transport = usb_transport.clone();
    let remember_verified_on_device = |verified: &NetworkIdentity| {
        if device.transport != usb_device.transport && !verified.endpoint.is_empty() {
            let mut updated = device.backup_transports();
            if verified.endpoint != device.transport && !updated.contains(&verified.endpoint) {
                updated.push(verified.endpoint.clone());
                device.set_backup_transports(updated);
            }
        }
    };
    let verify_usb = || -> Result<()> {
        let serial = adb.run_timeout(
            &["-s", &usb_transport, "shell", "getprop", "ro.serialno"],
            Duration::from_secs(3),
        )?;
        anyhow::ensure!(
            serial.trim() == device.serial,
            "USB 手机身份发生变化，请重新选择设备"
        );
        Ok(())
    };
    let mut id = network_identity(adb, &usb_device)
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
            &usb_transport,
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
                &usb_transport,
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
        if let Some(verified) = ensure_usb_wireless_standby(adb, &usb_device) {
            remember_verified_on_device(&verified);
            return Ok(verified);
        }
        if let Some(fresh) = network_identity(adb, &usb_device) {
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
                &usb_transport,
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
        &["-s", &usb_transport, "tcpip", "5555"],
        Duration::from_secs(5),
    )?;
    // adbd restarted: discard all pre-restart addresses, and wait for fresh identity.
    for _ in 0..10 {
        std::thread::sleep(Duration::from_millis(300));
        if let Some(verified) = ensure_usb_wireless_standby(adb, &usb_device) {
            remember_verified_on_device(&verified);
            return Ok(verified);
        }
    }
    anyhow::bail!("无线服务已请求启动，但连接尚未通过手机身份核验；请保持 USB 连接后重新扫描")
}

pub fn targets(prefs: &Preferences, services: &[Service], paired_hosts: &[IpAddr]) -> Vec<String> {
    let prioritized = crate::discovery::prioritize_and_dedup_services(services.to_vec());
    let mut ipv4_targets = Vec::new();
    let mut ipv6_targets = Vec::new();
    let mut push_unique = |endpoint: String, is_ipv4: bool| {
        if ipv4_targets.contains(&endpoint) || ipv6_targets.contains(&endpoint) {
            return;
        }
        if is_ipv4 {
            ipv4_targets.push(endpoint);
        } else {
            ipv6_targets.push(endpoint);
        }
    };

    for service in prioritized
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
        if remembered || paired_hosts.contains(&address.ip()) {
            push_unique(service.endpoint.clone(), address.is_ipv4());
        }
    }
    // A remembered address helps networks which suppress multicast. Never reuse a pairing port.
    // Place IPv4 remembered & ARP endpoints ahead of secondary IPv6 aliases so `truncate(6)`
    // never starves reachable IPv4 endpoints when dual-stack / temporary IPv6 is active.
    for d in &prefs.devices {
        if let Some(i) = prefs.wireless.get(&d.serial) {
            let old_addr = validate_endpoint(&i.endpoint).ok();
            if let Some(addr) = old_addr {
                push_unique(i.endpoint.clone(), addr.is_ipv4());
            }
            if !i.wlan_mac.is_empty() {
                if let Some(arp_ip) = resolve_ip_by_mac(&i.wlan_mac) {
                    if old_addr.map(|a| a.ip()) != Some(arp_ip) {
                        let arp_5555 = SocketAddr::new(arp_ip, 5555);
                        push_unique(arp_5555.to_string(), arp_5555.is_ipv4());
                        if let Some(addr) = old_addr {
                            let arp_old_port = SocketAddr::new(arp_ip, addr.port());
                            push_unique(arp_old_port.to_string(), arp_old_port.is_ipv4());
                        }
                    }
                }
            }
        }
    }
    let mut targets = ipv4_targets;
    targets.extend(ipv6_targets);
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
            let trimmed_serial = serial.trim();
            let paired = validate_endpoint(&endpoint).is_ok_and(|a| paired_hosts.contains(&a.ip()));
            if prefs.devices.iter().any(|d| d.serial == trimmed_serial) || paired {
                crate::device::record_verified_serial_for_transport(trimmed_serial, &endpoint);
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

        // macOS `arp -an` (including single-digit hex octets and `(incomplete)` skip)
        let macos_arp = "\
? (192.168.1.1) at 0:11:22:33:44:55 on en0 ifscope [ethernet]
? (192.168.1.54) at (incomplete) on en0 ifscope [ethernet]
? (192.168.1.55) at a:b:cc:d:ee:f on en0 ifscope [ethernet]
? (192.168.1.255) at ff:ff:ff:ff:ff:ff on en0 ifscope [ethernet]
";
        assert_eq!(
            parse_arp_output_for_mac(macos_arp, "0a:0b:cc:0d:ee:0f"),
            Some("192.168.1.55".parse().unwrap())
        );

        // Windows `arp -a` (both English and Chinese locales, skipping invalid/multicast/broadcast)
        let windows_arp_en = "\
Interface: 192.168.1.10 --- 0xc
  Internet Address      Physical Address      Type
  192.168.1.1           00-11-22-33-44-55     dynamic
  192.168.1.87          aa-bb-cc-dd-ee-ff     invalid
  192.168.1.88          aa-bb-cc-dd-ee-ff     dynamic
  224.0.0.22            01-00-5e-00-00-16     static
  255.255.255.255       ff-ff-ff-ff-ff-ff     static
";
        assert_eq!(
            parse_arp_output_for_mac(windows_arp_en, "aa:bb:cc:dd:ee:ff"),
            Some("192.168.1.88".parse().unwrap())
        );

        let windows_arp_zh = "\
接口: 192.168.31.12 --- 0x11
  Internet 地址         物理地址              类型
  192.168.31.65         aa-bb-cc-dd-ee-ff     无效
  192.168.31.66         AA-BB-CC-DD-EE-FF     动态
";
        assert_eq!(
            parse_arp_output_for_mac(windows_arp_zh, "aa:bb:cc:dd:ee:ff"),
            Some("192.168.31.66".parse().unwrap())
        );
    }

    #[test]
    fn targets_prioritizes_ipv4_and_deduplicates_temp_ipv6_without_starving_remembered_ipv4() {
        let mut prefs = Preferences::default();
        prefs.remember("serial-dual".into(), "PZ261A".into());
        prefs.wireless.insert(
            "serial-dual".into(),
            WirelessIdentity {
                guid: "adb-serial-dual-guid".into(),
                endpoint: "192.168.1.42:38111".into(),
                ..Default::default()
            },
        );
        let svc = |name: &str, ep: &str| Service {
            name: name.into(),
            kind: "_adb-tls-connect._tcp".into(),
            endpoint: ep.into(),
        };
        let services = vec![
            svc("adb-serial-dual-guid", "[fe80::1234]:41000"),
            svc("adb-serial-dual-guid", "[2408:8207:1::a1]:41000"),
            svc("adb-serial-dual-guid", "[2408:8207:1::a2]:41000"),
            svc("adb-serial-dual-guid", "[2408:8207:1::a3]:41000"),
            svc("adb-serial-dual-guid", "[2408:8207:1::a4]:41000"),
            svc("adb-serial-dual-guid", "[2408:8207:1::a5]:41000"),
            svc("adb-serial-dual-guid", "[2408:8207:1::a6]:41000"),
            svc("adb-serial-dual-guid", "192.168.1.90:41000"),
        ];
        let list = targets(&prefs, &services, &[]);
        assert_eq!(
            list,
            vec![
                "192.168.1.90:41000".to_string(),
                "192.168.1.42:38111".to_string(),
                "[2408:8207:1::a1]:41000".to_string(),
            ],
            "IPv4 mDNS and remembered IPv4 must precede collapsed temporary IPv6: {list:?}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn usb_disabled_ports_do_not_copy_stale_backup_transports_and_wireless_handle_uses_plugged_usb() {
        let (_dir, adb, usb_device) = wireless_fixture("bootstrap");
        usb_device.set_backup_transports(vec!["192.0.2.44:41001".into()]);

        let id = network_identity(&adb, &usb_device).expect("usb network identity");
        assert!(
            id.endpoint.is_empty(),
            "stale backup wireless transport must not be copied when USB getprop ports are disabled"
        );

        let wireless_handle = Device {
            serial: usb_device.serial.clone(),
            transport: "192.0.2.44:41001".into(),
            model: usb_device.model.clone(),
            android: usb_device.android.clone(),
            product: usb_device.product.clone(),
        };
        let bridged = enable_usb_wireless_bridge(&adb, &wireless_handle)
            .expect("plugged-in USB transport should bootstrap wireless even when device.transport is wireless");
        assert_eq!(bridged.endpoint, "192.0.2.99:5555");
    }
}
