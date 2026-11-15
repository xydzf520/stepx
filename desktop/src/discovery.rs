//! Native mDNS discovery for ADB builds that omit their own mDNS backend.
use crate::{
    adb::Adb,
    pairing::{parse_services, validate_endpoint, Service},
};
use anyhow::{Context, Result};
use mdns_sd::{Receiver, ServiceDaemon, ServiceEvent};
use std::{
    collections::HashMap,
    sync::{Mutex, OnceLock},
    time::{Duration, Instant},
};

const TYPES: [&str; 2] = [
    "_adb-tls-pairing._tcp.local.",
    "_adb-tls-connect._tcp.local.",
];

struct Discovery {
    daemon: ServiceDaemon,
    receivers: Vec<Receiver<ServiceEvent>>,
    services: HashMap<String, Vec<Service>>,
    primed: bool,
}

impl Discovery {
    fn new() -> Result<Self> {
        let daemon = ServiceDaemon::new()?;
        let mut receivers = Vec::new();
        for kind in TYPES {
            match daemon.browse(kind) {
                Ok(receiver) => receivers.push(receiver),
                Err(error) => {
                    let _ = daemon.shutdown();
                    return Err(error.into());
                }
            }
        }
        Ok(Self {
            daemon,
            receivers,
            services: HashMap::new(),
            primed: false,
        })
    }

    fn drain(&mut self) {
        for receiver in &self.receivers {
            while let Ok(event) = receiver.try_recv() {
                match event {
                    ServiceEvent::ServiceResolved(info) => {
                        let raw_entries = info
                            .get_addresses()
                            .iter()
                            .map(|scoped| scoped.to_ip_addr())
                            .filter(|ip| {
                                !matches!(ip, std::net::IpAddr::V6(v6) if v6.is_unicast_link_local())
                            })
                            .filter_map(|ip| {
                                service_from_dns(
                                    &info.fullname,
                                    &info.ty_domain,
                                    &std::net::SocketAddr::new(ip, info.port).to_string(),
                                )
                            })
                            .collect();
                        self.services.insert(
                            info.fullname.to_lowercase(),
                            prioritize_and_dedup_services(raw_entries),
                        );
                    }
                    ServiceEvent::ServiceRemoved(ty, fullname) => {
                        let full_lower = fullname.to_lowercase();
                        let trimmed_name = fullname
                            .trim_end_matches(&format!(".{ty}"))
                            .trim_end_matches('.');
                        self.services.retain(|key, list| {
                            !full_lower.starts_with(key)
                                && !key.starts_with(&full_lower)
                                && !list.iter().any(|s| s.name == trimmed_name)
                        });
                    }
                    _ => {}
                }
            }
        }
    }
}

impl Drop for Discovery {
    fn drop(&mut self) {
        let _ = self.daemon.shutdown();
    }
}

fn service_from_dns(fullname: &str, domain: &str, endpoint: &str) -> Option<Service> {
    let domain = domain.to_lowercase();
    if !TYPES.contains(&domain.as_str()) || validate_endpoint(endpoint).is_err() {
        return None;
    }
    let suffix = format!(".{domain}");
    if !fullname.to_lowercase().ends_with(&suffix) {
        return None;
    }
    let name = &fullname[..fullname.len() - suffix.len()];
    if name.is_empty() {
        return None;
    }
    Some(Service {
        name: name.into(),
        kind: domain.trim_end_matches(".local.").into(),
        endpoint: endpoint.into(),
    })
}

pub fn prioritize_and_dedup_services(services: Vec<Service>) -> Vec<Service> {
    let mut parsed: Vec<(Service, std::net::SocketAddr)> = services
        .into_iter()
        .filter_map(|s| {
            let addr = validate_endpoint(&s.endpoint).ok()?;
            if let std::net::IpAddr::V6(v6) = addr.ip() {
                if v6.is_unicast_link_local() {
                    return None;
                }
            }
            Some((s, addr))
        })
        .collect();
    let addr_rank = |addr: &std::net::SocketAddr| -> u8 {
        match addr.ip() {
            std::net::IpAddr::V4(_) => 0,
            std::net::IpAddr::V6(v6) => {
                // Prefer ULA (`fc00::/7`) slightly ahead of rotating temporary global SLAAC,
                // while keeping all IPv6 strictly behind IPv4.
                if (v6.segments()[0] & 0xfe00) == 0xfc00 {
                    1
                } else {
                    2
                }
            }
        }
    };
    parsed.sort_by(|(a, addr_a), (b, addr_b)| {
        (&a.kind, &a.name, addr_a.port(), addr_rank(addr_a), &a.endpoint).cmp(&(
            &b.kind,
            &b.name,
            addr_b.port(),
            addr_rank(addr_b),
            &b.endpoint,
        ))
    });
    parsed.dedup_by(|(a, _), (b, _)| a == b);

    // Deduplicate temporary IPv6 addresses so a single `(kind, name, port)` service
    // never floods candidate lists with multiple rotating SLAAC addresses.
    let mut seen_v6_service: std::collections::HashSet<(String, String, u16)> =
        std::collections::HashSet::new();
    let mut out = Vec::with_capacity(parsed.len());
    for (service, addr) in parsed {
        if addr.is_ipv6() {
            let key = (service.kind.clone(), service.name.clone(), addr.port());
            if !seen_v6_service.insert(key) {
                continue;
            }
        }
        out.push(service);
    }
    out
}

/// Called on the ADB worker; the first query waits briefly for multicast replies.
pub fn discover_services(adb: &Adb) -> Result<Vec<Service>> {
    static NATIVE: OnceLock<Mutex<Option<Discovery>>> = OnceLock::new();
    let mut native = NATIVE
        .get_or_init(|| Mutex::new(None))
        .lock()
        .map_err(|_| anyhow::anyhow!("设备发现服务不可用"))?;
    let first = native.is_none();
    let start = if first {
        Discovery::new().map(|d| {
            *native = Some(d);
        })
    } else {
        Ok(())
    };
    let adb_result = adb.run_timeout(&["mdns", "services"], Duration::from_secs(3));
    let mut result = adb_result
        .as_ref()
        .map(|s| parse_services(s))
        .unwrap_or_default();
    if let Some(discovery) = native.as_mut() {
        let wait = if !discovery.primed {
            discovery.primed = true;
            Duration::from_millis(2000)
        } else {
            Duration::from_millis(80)
        };
        let deadline = Instant::now() + wait;
        loop {
            discovery.drain();
            if Instant::now() >= deadline {
                break;
            }
            std::thread::sleep(Duration::from_millis(25));
        }
        result.extend(discovery.services.values().flatten().cloned());
    } else if result.is_empty() {
        start.context("无法启动自动发现；仍可使用 USB 或手工输入连接地址")?;
    }
    Ok(prioritize_and_dedup_services(result))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn dns_names_preserve_qr_identity_and_separate_ports() {
        let qr = service_from_dns(
            "studio-StepX-abc._adb-tls-pairing._tcp.local.",
            TYPES[0],
            "192.168.1.2:33333",
        )
        .unwrap();
        assert_eq!(qr.name, "studio-StepX-abc");
        assert_eq!(qr.kind, "_adb-tls-pairing._tcp");
        let connect = service_from_dns(
            "adb-phone._adb-tls-connect._tcp.local.",
            TYPES[1],
            "192.168.1.2:44444",
        )
        .unwrap();
        assert_eq!(connect.endpoint, "192.168.1.2:44444");
        assert!(
            service_from_dns("x._http._tcp.local.", "_http._tcp.local.", "192.168.1.2:80")
                .is_none()
        );
        assert!(service_from_dns("wrong.local.", TYPES[0], "192.168.1.2:80").is_none());
    }

    #[test]
    fn prioritize_and_dedup_orders_ipv4_before_ipv6_and_collapses_temp_ipv6() {
        let svc = |ep: &str| Service {
            name: "adb-SERIAL1-guid".into(),
            kind: "_adb-tls-connect._tcp".into(),
            endpoint: ep.into(),
        };
        let raw = vec![
            svc("[240e:390:100::abcd]:41555"),
            svc("[240e:390:100::1111]:41555"),
            svc("[fe80::1234]:41555"),
            svc("192.168.1.25:41555"),
            svc("[fd00::99]:41555"),
            svc("192.168.1.25:41555"),
        ];
        let deduped = prioritize_and_dedup_services(raw);
        assert_eq!(
            deduped,
            vec![
                svc("192.168.1.25:41555"),
                svc("[fd00::99]:41555"),
            ]
        );
    }
}
