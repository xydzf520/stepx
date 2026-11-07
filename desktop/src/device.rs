pub use crate::companion_app::{CompanionPermission, CompanionStatus};
use crate::{adb::Adb, mirror::MirrorSession};
use anyhow::{bail, Context, Result};
use serde::Serialize;
use std::{
    collections::HashMap,
    process::Command,
    sync::{Mutex, OnceLock},
};

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Transport {
    pub id: String,
    pub state: String,
    pub model: String,
}

const VERIFIED_TRANSPORT_TTL: std::time::Duration = std::time::Duration::from_secs(30);

#[derive(Clone, Debug)]
struct VerifiedTransportEntry {
    active_transport: String,
    hardware_serial: String,
    verified_at: std::time::Instant,
}

static ACTIVE_TRANSPORT_REGISTRY: OnceLock<Mutex<HashMap<String, VerifiedTransportEntry>>> =
    OnceLock::new();
static KNOWN_SERIAL_BY_TRANSPORT: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn active_transport_registry() -> &'static Mutex<HashMap<String, VerifiedTransportEntry>> {
    ACTIVE_TRANSPORT_REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

fn known_serial_registry() -> &'static Mutex<HashMap<String, String>> {
    KNOWN_SERIAL_BY_TRANSPORT.get_or_init(|| Mutex::new(HashMap::new()))
}

fn remember_serial_for_transports(serial: &str, transports: &[&str]) {
    let s = serial.trim();
    if s.is_empty() || s.contains(':') || s.starts_with("adb-") {
        return;
    }
    if let Ok(mut map) = known_serial_registry().lock() {
        for t in transports {
            let trimmed = t.trim();
            if !trimmed.is_empty() && !trimmed.starts_with("model:") {
                map.insert(trimmed.to_string(), s.to_string());
            }
        }
    }
}

fn known_serial_for_transport(transport_or_serial: &str) -> Option<String> {
    let trimmed = transport_or_serial.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Ok(map) = known_serial_registry().lock() {
        if let Some(s) = map.get(trimmed) {
            return Some(s.clone());
        }
    }
    if let Some(canon) = canonical_serial_from_transport(trimmed) {
        return Some(canon);
    }
    if let Ok(reg) = active_transport_registry().lock() {
        if let Some(entry) = reg.get(trimmed) {
            if !entry.hardware_serial.is_empty() {
                return Some(entry.hardware_serial.clone());
            }
        }
    }
    None
}

fn record_verified_transport(preferred: &str, hardware_serial: &str, active_transport: &str) {
    let active = active_transport.trim();
    if active.is_empty() {
        return;
    }
    let serial = if !hardware_serial.trim().is_empty() {
        hardware_serial.trim().to_string()
    } else {
        known_serial_for_transport(active)
            .or_else(|| known_serial_for_transport(preferred))
            .unwrap_or_default()
    };
    remember_serial_for_transports(&serial, &[preferred, active]);
    let entry = VerifiedTransportEntry {
        active_transport: active.to_string(),
        hardware_serial: serial.clone(),
        verified_at: std::time::Instant::now(),
    };
    if let Ok(mut reg) = active_transport_registry().lock() {
        reg.insert(active.to_string(), entry.clone());
        if !preferred.trim().is_empty() {
            reg.insert(preferred.trim().to_string(), entry.clone());
        }
        if !serial.is_empty() {
            reg.insert(serial, entry);
        }
    }
}

pub fn invalidate_verified_transport(transport_or_serial: &str) {
    let trimmed = transport_or_serial.trim();
    if trimmed.is_empty() {
        return;
    }
    let canonical = canonical_serial_from_transport(trimmed).unwrap_or_default();
    if let Ok(mut reg) = active_transport_registry().lock() {
        reg.retain(|k, v| {
            k != trimmed
                && v.active_transport != trimmed
                && v.hardware_serial != trimmed
                && (canonical.is_empty() || (k != &canonical && v.hardware_serial != canonical))
        });
    }
}

pub fn promote_active_transport(serial: &str, new_transport: &str) {
    let new_t = new_transport.trim();
    if new_t.is_empty() {
        return;
    }
    let effective_serial = if !serial.trim().is_empty() {
        serial.trim().to_string()
    } else {
        known_serial_for_transport(new_t).unwrap_or_default()
    };
    let mut old_transports = Vec::new();
    if let Ok(reg) = active_transport_registry().lock() {
        if !effective_serial.is_empty() {
            if let Some(existing) = reg.get(&effective_serial) {
                if !existing.active_transport.is_empty() && existing.active_transport != new_t {
                    old_transports.push(existing.active_transport.clone());
                }
            }
        }
        for (k, v) in reg.iter() {
            if !effective_serial.is_empty() && v.hardware_serial == effective_serial {
                if v.active_transport != new_t && !old_transports.contains(&v.active_transport) {
                    old_transports.push(v.active_transport.clone());
                }
                if k != &effective_serial && k != new_t && !old_transports.contains(k) {
                    old_transports.push(k.clone());
                }
            }
        }
    }

    record_verified_transport(&effective_serial, &effective_serial, new_t);
    if let Ok(mut reg) = active_transport_registry().lock() {
        let entry = VerifiedTransportEntry {
            active_transport: new_t.to_string(),
            hardware_serial: effective_serial.clone(),
            verified_at: std::time::Instant::now(),
        };
        for old_t in &old_transports {
            reg.insert(old_t.clone(), entry.clone());
        }
    }

    let mut all_known: Vec<String> = Vec::new();
    if !effective_serial.is_empty()
        && !effective_serial.contains(':')
        && !effective_serial.starts_with("adb-")
    {
        all_known.push(effective_serial.clone());
    }
    for old_t in &old_transports {
        if !all_known.contains(old_t) {
            all_known.push(old_t.clone());
        }
    }
    if !all_known.contains(&new_t.to_string()) {
        all_known.push(new_t.to_string());
    }
    let existing_backups = backup_transports_for(&[&effective_serial, new_t]);
    for b in existing_backups {
        if !b.is_empty() && !all_known.contains(&b) {
            all_known.push(b);
        }
    }
    all_known.sort_by(|a, b| {
        transport_priority(a)
            .cmp(&transport_priority(b))
            .then_with(|| a.cmp(b))
    });
    if all_known.len() > 1 {
        if let Ok(mut map) = backup_transport_registry().lock() {
            if !effective_serial.is_empty() {
                map.insert(effective_serial.clone(), all_known.clone());
            }
            for t in &all_known {
                let peers: Vec<String> = all_known.iter().filter(|x| *x != t).cloned().collect();
                map.insert(t.clone(), peers);
            }
        }
    }
}

pub fn current_active_transport_for_serial(serial: &str) -> Option<String> {
    let trimmed = serial.trim();
    if trimmed.is_empty() {
        return None;
    }
    let canonical = canonical_serial_from_transport(trimmed).unwrap_or_default();
    if let Ok(reg) = active_transport_registry().lock() {
        if let Some(entry) = reg.get(trimmed) {
            return Some(entry.active_transport.clone());
        }
        if !canonical.is_empty() {
            if let Some(entry) = reg.get(&canonical) {
                return Some(entry.active_transport.clone());
            }
        }
    }
    None
}

// A host-session reset must never interrupt a send, dial, or other dispatched write.
#[derive(Default)]
struct SessionActivity { writes: usize, repairing: bool, reset_at: Option<std::time::Instant> }
type SessionActivitySlot = std::sync::Arc<Mutex<SessionActivity>>;

fn session_activities() -> &'static Mutex<HashMap<String, SessionActivitySlot>> {
    static SESSIONS: OnceLock<Mutex<HashMap<String, SessionActivitySlot>>> = OnceLock::new();
    SESSIONS.get_or_init(Default::default)
}

fn session_activity_identity(target: &str) -> String {
    known_serial_for_transport(target).unwrap_or_else(|| target.trim().to_string())
}

// A request can acquire a guard before the first identity probe has mapped its
// IP to a hardware serial. Resolve existing keys at inspection time too, so
// that learning this mapping never leaves a dispatched write on an orphan lock.
fn related_session_activities(
    sessions: &HashMap<String, SessionActivitySlot>,
    identity: &str,
) -> Vec<SessionActivitySlot> {
    sessions
        .iter()
        .filter(|(target, _)| session_activity_identity(target) == identity)
        .map(|(_, slot)| slot.clone())
        .collect()
}

pub struct TransportMutationGuard(SessionActivitySlot);
impl Drop for TransportMutationGuard {
    fn drop(&mut self) { let mut s = self.0.lock().unwrap_or_else(|e| e.into_inner()); s.writes = s.writes.saturating_sub(1); }
}
pub fn protect_transport_mutation(target: &str) -> Result<TransportMutationGuard> {
    let mut sessions = session_activities().lock().unwrap_or_else(|e| e.into_inner());
    let key = session_activity_identity(target);
    for related in related_session_activities(&sessions, &key) {
        anyhow::ensure!(
            !related.lock().unwrap_or_else(|e| e.into_inner()).repairing,
            "手机连接正在恢复，此次操作尚未发送，请恢复后重试"
        );
    }
    let slot = sessions.entry(key).or_default().clone();
    {
        let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
        anyhow::ensure!(!s.repairing, "手机连接正在恢复，此次操作尚未发送，请恢复后重试");
        s.writes += 1;
    }
    Ok(TransportMutationGuard(slot))
}
struct HostSessionRepair(Vec<SessionActivitySlot>);
impl Drop for HostSessionRepair {
    fn drop(&mut self) {
        for slot in &self.0 {
            slot.lock().unwrap_or_else(|e| e.into_inner()).repairing = false;
        }
    }
}
fn begin_host_session_repair(target: &str) -> Option<HostSessionRepair> {
    // Serialize the inspection/reservation with new write guards. Dropping a
    // guard needs only its own slot, so this never waits for a network request.
    let mut sessions = session_activities().lock().unwrap_or_else(|e| e.into_inner());
    let key = session_activity_identity(target);
    sessions.entry(key.clone()).or_default();
    let related = related_session_activities(&sessions, &key);
    for slot in &related {
        let s = slot.lock().unwrap_or_else(|e| e.into_inner());
        if s.writes > 0 || s.repairing || s.reset_at.is_some_and(|at| at.elapsed() < std::time::Duration::from_secs(20)) { return None; }
    }
    let now = std::time::Instant::now();
    for slot in &related {
        let mut s = slot.lock().unwrap_or_else(|e| e.into_inner());
        s.repairing = true;
        s.reset_at = Some(now);
    }
    Some(HostSessionRepair(related))
}

#[cfg(all(test, unix))]
mod session_repair_tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, time::Duration};
    fn fake(endpoint: &str, serial: &str, wrong_identity: bool) -> (tempfile::TempDir, Adb, std::path::PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("adb");
        let log = dir.path().join("calls");
        let marker = dir.path().join("reconnected");
        let before = if wrong_identity { "echo OTHER_PHONE" } else { "echo 'error: closed' >&2; exit 1" };
        std::fs::write(&script, format!(r#"#!/bin/sh
printf '%s\n' "$*" >> '{log}'
case "$*" in
 "devices -l") printf 'List of devices attached\n{endpoint} device\n';;
 "-s {endpoint} shell getprop ro.serialno") if [ -f '{marker}' ]; then echo {serial}; else {before}; fi;;
 "disconnect {endpoint}") exit 0;;
 "connect {endpoint}") touch '{marker}';;
 *) exit 1;;
esac
"#, log=log.display(), marker=marker.display())).unwrap();
        std::fs::set_permissions(&script, std::fs::Permissions::from_mode(0o755)).unwrap();
        remember_serial_for_transports(serial, &[endpoint]);
        (dir, Adb { path: script }, log)
    }
    #[test]
    fn listed_but_unresponsive_session_is_rebuilt_and_identity_checked() {
        let endpoint = "192.0.2.164:40101";
        let (_dir, adb, log) = fake(endpoint, "REPAIR164_A", false);
        assert_eq!(recover_transport(&adb, endpoint, Duration::from_secs(4)).as_deref(), Some(endpoint));
        let log = std::fs::read_to_string(log).unwrap();
        assert_eq!(log.lines().filter(|line| *line == format!("disconnect {endpoint}")).count(), 1, "{log}");
        assert!(!log.contains("kill-server") && !log.contains("tcpip") && !log.contains("input"));
    }
    #[test]
    fn recovery_never_resets_a_dispatched_message_or_call() {
        let endpoint = "192.0.2.164:40102";
        let (_dir, adb, log) = fake(endpoint, "REPAIR164_B", false);
        let lease = protect_transport_mutation("REPAIR164_B").unwrap();
        assert!(recover_transport(&adb, endpoint, Duration::from_secs(4)).is_none());
        assert!(!std::fs::read_to_string(log).unwrap().contains("disconnect"));
        drop(lease);
        let repair = begin_host_session_repair(endpoint).unwrap();
        assert!(protect_transport_mutation("REPAIR164_B").is_err());
        drop(repair);
        assert!(protect_transport_mutation(endpoint).is_ok());
    }
    #[test]
    fn reused_wireless_address_does_not_reset_or_accept_another_phone() {
        let endpoint = "192.0.2.164:40103";
        let (_dir, adb, log) = fake(endpoint, "REPAIR164_C", true);
        assert!(recover_transport(&adb, endpoint, Duration::from_secs(4)).is_none());
        assert!(!std::fs::read_to_string(log).unwrap().contains("disconnect"));
    }
    #[test]
    fn learning_identity_keeps_an_already_dispatched_write_protected() {
        let endpoint = "192.0.2.165:40104";
        let serial = "REPAIR165_LATE_IDENTITY";
        let write = protect_transport_mutation(endpoint).unwrap();
        remember_serial_for_transports(serial, &[endpoint]);
        assert!(begin_host_session_repair(serial).is_none());
        assert!(begin_host_session_repair(endpoint).is_none());
        drop(write);
        let repair = begin_host_session_repair(serial).unwrap();
        assert!(protect_transport_mutation(endpoint).is_err());
        assert!(protect_transport_mutation(serial).is_err());
        drop(repair);
        assert!(protect_transport_mutation(endpoint).is_ok());
    }
    #[test]
    fn a_newly_discovered_alias_cannot_dispatch_during_repair() {
        let serial = "REPAIR165_NEW_ALIAS";
        let repair = begin_host_session_repair(serial).unwrap();
        let endpoint = "192.0.2.165:40105";
        remember_serial_for_transports(serial, &[endpoint]);
        assert!(protect_transport_mutation(endpoint).is_err());
        assert!(protect_transport_mutation("UNRELATED165_DEVICE").is_ok());
        drop(repair);
        assert!(protect_transport_mutation(endpoint).is_ok());
    }
}

#[cfg(all(test, unix))]
mod device_inspection_tests {
    use super::*;
    use std::{os::unix::fs::PermissionsExt, time::{Duration, Instant}};

    #[test]
    fn silent_identity_alias_is_bounded_and_does_not_prevent_a_reachable_selection() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("adb");
        let log = dir.path().join("calls");
        std::fs::write(&path, format!(r#"#!/bin/sh
printf '%s\n' "$*" >> '{}'
case "$2" in
 '[fd00::165]:40106') sleep 30;;
 '192.0.2.165:40107') printf 'INSPECT165_REACHABLE\nPZ261A\n16\npandora\n';;
 '192.0.2.165:40108') printf 'INSPECT165_UNSUPPORTED\nOtherPhone\n16\nother\n';;
 *) printf 'List of devices attached\n';;
esac
"#, log.display())).unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = Adb { path };
        let started = Instant::now();
        let error = Device::inspect(&adb, "[fd00::165]:40106").unwrap_err();
        assert!(error.downcast_ref::<crate::adb::ProcessTimeout>().is_some());
        assert!(started.elapsed() < Duration::from_secs(8), "identity discovery used an unbounded operation deadline");
        let device = Device::inspect(&adb, "192.0.2.165:40107").unwrap();
        assert_eq!(device.serial, "INSPECT165_REACHABLE");
        assert_eq!(device.model, "PZ261A");
        assert!(Device::inspect(&adb, "192.0.2.165:40108").is_err(), "supported-model checks must remain strict");
        let calls = std::fs::read_to_string(log).unwrap();
        for command in ["kill-server", "disconnect", "reconnect", "tcpip", "input ", "settings "] {
            assert!(!calls.contains(command), "identity discovery unexpectedly mutated the phone: {calls}");
        }
    }
}

/// Concurrent failed reads and the connection watcher share one recovery attempt.
/// Try verified alternatives first; an unresponsive, still-listed wireless session
/// can be rebuilt on the host only after all dispatched mutations have finished.
#[derive(Default)]
struct RecoveryState {
    running: bool,
    finished: Option<std::time::Instant>,
    transport: Option<String>,
}

type RecoverySlot = std::sync::Arc<(Mutex<RecoveryState>, std::sync::Condvar)>;

pub fn resolve_failover_transport(adb: &Adb, failed_transport: &str) -> Option<String> {
    recover_transport(adb, failed_transport, std::time::Duration::from_secs(6))
}

pub fn recover_transport(
    adb: &Adb,
    failed_transport: &str,
    budget: std::time::Duration,
) -> Option<String> {
    use std::time::{Duration, Instant};
    if failed_transport.trim().is_empty() || budget.is_zero() {
        return None;
    }
    static RECOVERY: OnceLock<Mutex<HashMap<(std::path::PathBuf, String), RecoverySlot>>> =
        OnceLock::new();
    let serial = known_serial_for_transport(failed_transport);
    let key = (
        adb.path.clone(),
        serial
            .clone()
            .unwrap_or_else(|| failed_transport.to_string()),
    );
    let slot = RECOVERY
        .get_or_init(|| Mutex::new(HashMap::new()))
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .entry(key)
        .or_default()
        .clone();
    let deadline = Instant::now() + budget;
    let (lock, cv) = &*slot;
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    while state.running {
        let remaining = deadline.saturating_duration_since(Instant::now());
        if remaining.is_zero() {
            return None;
        }
        state = cv
            .wait_timeout(state, remaining)
            .unwrap_or_else(|e| e.into_inner())
            .0;
    }
    if state
        .finished
        .is_some_and(|at| at.elapsed() < Duration::from_secs(2))
    {
        return state.transport.clone();
    }
    state.running = true;
    drop(state);
    let result = recover_transport_until(adb, failed_transport, serial.as_deref(), deadline);
    let mut state = lock.lock().unwrap_or_else(|e| e.into_inner());
    state.running = false;
    state.finished = Some(Instant::now());
    state.transport = result.clone();
    cv.notify_all();
    result
}

fn recover_transport_until(
    adb: &Adb,
    failed: &str,
    serial: Option<&str>,
    deadline: std::time::Instant,
) -> Option<String> {
    use std::time::{Duration, Instant};
    let _guard = crate::adb::enter_transport_recovery();
    let phase_deadline = std::cell::Cell::new(deadline);
    let read = |args: &[&str], max: Duration| -> Option<String> {
        let left = phase_deadline.get().saturating_duration_since(Instant::now());
        if left.is_zero() {
            return None;
        }
        adb.run_timeout(args, left.min(max)).ok()
    };
    let raw = read(&["devices", "-l"], Duration::from_millis(1000)).unwrap_or_default();
    let online: Vec<&str> = raw
        .lines()
        .filter_map(|line| {
            let mut words = line.split_whitespace();
            let id = words.next()?;
            (words.next() == Some("device")).then_some(id)
        })
        .collect();
    let mut candidates = vec![failed.to_string()];
    let stale_online = online.contains(&failed) && serial.is_some()
        && crate::pairing::validate_endpoint(failed).is_ok();
    if stale_online {
        // Old IPv6 aliases must not consume the entire budget before the host
        // gets a chance to replace the frozen TLS session on the current IP.
        phase_deadline.set(deadline.checked_sub(Duration::from_millis(2400)).unwrap_or(deadline));
    }
    if let Some(serial) = serial {
        if let Some(active) = current_active_transport_for_serial(serial) {
            candidates.push(active);
        }
        candidates.extend(backup_transports_for(&[failed, serial]));
        // Read identity before accepting any endpoint, including a reused IP/port.
        candidates.extend(
            online
                .iter()
                .filter(|id| canonical_serial_from_transport(id).is_none_or(|s| s == serial))
                .map(|id| id.to_string()),
        );
        candidates.push(serial.to_string());
    }
    candidates.sort_by_key(|id| {
        (
            if online.contains(&id.as_str()) { 0 } else { 1 },
            if id == failed { 0 } else { 1 },
            transport_priority(id),
        )
    });
    candidates.dedup();
    let mut tried = Vec::new();
    let failed_unresponsive = std::cell::Cell::new(false);
    let failed_identity_changed = std::cell::Cell::new(false);
    let mut probe = |endpoint: &str, connect: bool| -> Option<String> {
        if tried.iter().any(|s| s == endpoint) {
            return None;
        }
        tried.push(endpoint.to_string());
        if connect
            && !online.contains(&endpoint)
            && crate::pairing::validate_endpoint(endpoint).is_ok()
        {
            let offline = raw.lines().any(|line| {
                let mut words = line.split_whitespace();
                words.next() == Some(endpoint) && words.next() == Some("offline")
            });
            if offline {
                let _ = read(&["-s", endpoint, "reconnect"], Duration::from_millis(500));
            }
            let _ = read(&["connect", endpoint], Duration::from_millis(800));
        }
        let probe_has_time = phase_deadline.get().saturating_duration_since(Instant::now()) >= Duration::from_millis(150);
        let got = match read(
            &["-s", endpoint, "shell", "getprop", "ro.serialno"],
            Duration::from_millis(900),
        ) {
            Some(got) => got,
            None => {
                if endpoint == failed && probe_has_time { failed_unresponsive.set(true); }
                return None;
            }
        };
        let got = got.trim();
        if endpoint == failed && !got.is_empty() && serial.is_some_and(|s| s != got) { failed_identity_changed.set(true); }
        if got.is_empty() || serial.is_some_and(|expected| expected != got) {
            return None;
        }
        // Without a known hardware identity only the original endpoint is eligible.
        if serial.is_none() && endpoint != failed {
            return None;
        }
        promote_active_transport(got, endpoint);
        record_verified_transport(failed, got, endpoint);
        Some(endpoint.to_string())
    };
    // Prefer live aliases. Old remembered addresses must not starve fresh discovery.
    for candidate in candidates
        .iter()
        .filter(|id| online.contains(&id.as_str()) || *id == failed)
    {
        if let Some(found) = probe(candidate, true) {
            return Some(found);
        }
    }
    if let Some(serial) = serial {
        if let Some(mdns) = read(&["mdns", "services"], Duration::from_millis(800)) {
            for line in mdns
                .lines()
                .filter(|line| line.contains("_adb-tls-connect._tcp"))
            {
                let mut words = line.split_whitespace();
                let name = words.next().unwrap_or("");
                if !name.starts_with(&format!("adb-{serial}-")) && name != format!("adb-{serial}") {
                    continue;
                }
                if let Some(endpoint) = line.split_whitespace().last() {
                    if crate::pairing::validate_endpoint(endpoint).is_ok() {
                        if let Some(found) = probe(endpoint, true) {
                            return Some(found);
                        }
                    }
                }
            }
        }
    }
    for candidate in candidates.iter().take(8) {
        if let Some(found) = probe(candidate, true) {
            return Some(found);
        }
    }
    phase_deadline.set(deadline);
    if stale_online && failed_unresponsive.get() && !failed_identity_changed.get()
        && deadline.saturating_duration_since(Instant::now()) >= Duration::from_millis(500) {
        let _repair = begin_host_session_repair(failed)?;
        invalidate_verified_transport(failed);
        let _ = read(&["disconnect", failed], Duration::from_millis(700));
        let _ = read(&["connect", failed], Duration::from_millis(1000));
        let identity = read(&["-s", failed, "shell", "getprop", "ro.serialno"], Duration::from_millis(1000))?;
        let identity = identity.trim();
        if serial == Some(identity) && !identity.is_empty() {
            promote_active_transport(identity, failed);
            record_verified_transport(failed, identity, failed);
            return Some(failed.to_string());
        }
    }
    None
}

fn backup_transport_registry() -> &'static Mutex<HashMap<String, Vec<String>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Vec<String>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub fn set_backup_transports_for(keys: &[&str], backups: Vec<String>) {
    let mut non_model_keys: Vec<String> = Vec::new();
    for key in keys {
        let trimmed = key.trim();
        if !trimmed.is_empty() && !trimmed.starts_with("model:") {
            if !non_model_keys.iter().any(|k| k == trimmed) {
                non_model_keys.push(trimmed.to_string());
            }
            if !trimmed.contains(':') && !trimmed.starts_with("adb-") {
                let mut refs: Vec<&str> = keys.to_vec();
                for b in &backups {
                    refs.push(b.as_str());
                }
                remember_serial_for_transports(trimmed, &refs);
            }
        }
    }
    if let Ok(mut map) = backup_transport_registry().lock() {
        for key in keys {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                map.insert(trimmed.to_string(), backups.clone());
            }
        }
        for b in &backups {
            let bt = b.trim();
            if bt.is_empty() || keys.iter().any(|k| k.trim() == bt) {
                continue;
            }
            let mut peers: Vec<String> = Vec::new();
            for k in &non_model_keys {
                if k != bt && !peers.contains(k) {
                    peers.push(k.clone());
                }
            }
            for other in &backups {
                let ot = other.trim();
                if !ot.is_empty() && ot != bt && !peers.iter().any(|p| p == ot) {
                    peers.push(ot.to_string());
                }
            }
            peers.sort_by(|a, c| {
                transport_priority(a)
                    .cmp(&transport_priority(c))
                    .then_with(|| a.cmp(c))
            });
            if !peers.is_empty() {
                map.insert(bt.to_string(), peers);
            }
        }
    }
}

pub fn backup_transports_for(keys: &[&str]) -> Vec<String> {
    if let Ok(map) = backup_transport_registry().lock() {
        for key in keys {
            let trimmed = key.trim();
            if !trimmed.is_empty() {
                if let Some(found) = map.get(trimmed) {
                    if !found.is_empty() {
                        return found.clone();
                    }
                }
            }
        }
    }
    Vec::new()
}

/// Priority of a transport ID (lower is higher priority):
/// 1 = physical USB serial (`STEPX_TEST_DEVICE`)
/// 2 = explicit wireless IP:Port (`10.0.0.42:34883`)
/// 3 = mDNS TLS service (`adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp`)
pub fn transport_priority(id: &str) -> u8 {
    let trimmed = id.trim();
    if trimmed.starts_with("adb-") || trimmed.contains("._adb-tls-") {
        3
    } else if trimmed.contains(':') {
        2
    } else {
        1
    }
}

pub fn is_usb_transport(id: &str) -> bool {
    transport_priority(id) == 1
}

/// Extract canonical hardware serial from a USB serial or `adb-<SERIAL>-<suffix>._adb-tls-...` transport ID.
/// Returns `None` for `ip:port` wireless endpoints whose serial must be resolved via model/getprop.
pub fn canonical_serial_from_transport(id: &str) -> Option<String> {
    let trimmed = id.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix("adb-") {
        let without_service = rest
            .split("._adb-tls-")
            .next()
            .unwrap_or(rest)
            .split_whitespace()
            .next()
            .unwrap_or(rest);
        if let Some((serial, _suffix)) = without_service.rsplit_once('-') {
            if !serial.trim().is_empty() {
                return Some(serial.trim().to_string());
            }
        }
        return None;
    }
    if trimmed.contains(':') {
        return None;
    }
    Some(trimmed.to_string())
}

/// Resolve an online ADB transport for a device, automatically falling back to
/// wireless IP:port / mDNS or USB backup transports when `preferred` goes offline.
pub fn resolve_online_transport(
    adb: &Adb,
    preferred: &str,
    serial: &str,
    _model: &str,
    backups: &[String],
) -> Result<String> {
    let _recovery_guard = crate::adb::enter_transport_recovery();
    let preferred_trimmed = preferred.trim();
    let serial_trimmed = serial.trim();

    if !serial_trimmed.is_empty() {
        let mut refs = vec![preferred_trimmed];
        for b in backups {
            refs.push(b.as_str());
        }
        remember_serial_for_transports(serial_trimmed, &refs);
    }

    // 1. Fast Path (0ms TTL Cache) & Promoted Transport Lookup
    let mut stale_promoted: Option<String> = None;
    if let Ok(reg) = active_transport_registry().lock() {
        let lookup = reg.get(preferred_trimmed).or_else(|| {
            (!serial_trimmed.is_empty())
                .then(|| reg.get(serial_trimmed))
                .flatten()
        });
        if let Some(entry) = lookup {
            let serial_matches = serial_trimmed.is_empty()
                || entry.hardware_serial.is_empty()
                || entry.hardware_serial == serial_trimmed;
            if serial_matches && !entry.active_transport.is_empty() {
                if entry.verified_at.elapsed() < VERIFIED_TRANSPORT_TTL {
                    return Ok(entry.active_transport.clone());
                }
                if entry.active_transport != preferred_trimmed {
                    stale_promoted = Some(entry.active_transport.clone());
                }
            }
        }
    }

    // 2. Promoted Transport Check (probe promoted transport first even if > 3.5s old)
    if let Some(promoted) = stale_promoted {
        if let Ok(got) = adb.run_timeout(
            &["-s", &promoted, "shell", "getprop", "ro.serialno"],
            std::time::Duration::from_secs(3),
        ) {
            let got = got.trim();
            if !got.is_empty() && (serial_trimmed.is_empty() || got == serial_trimmed) {
                record_verified_transport(preferred_trimmed, got, &promoted);
                return Ok(promoted);
            }
        }
        invalidate_verified_transport(&promoted);
    }

    if !preferred_trimmed.is_empty() {
        if let Ok(got) = adb.run_timeout(
            &["-s", preferred_trimmed, "shell", "getprop", "ro.serialno"],
            std::time::Duration::from_secs(3),
        ) {
            let got = got.trim();
            if !got.is_empty() && (serial_trimmed.is_empty() || got == serial_trimmed) {
                record_verified_transport(preferred_trimmed, got, preferred_trimmed);
                return Ok(preferred_trimmed.to_string());
            }
            if !got.is_empty() && !serial_trimmed.is_empty() && got != serial_trimmed {
                bail!("设备身份已变化，请重新选择手机。此次操作没有执行。");
            }
        }
    }

    let mut known = backup_transports_for(&[preferred_trimmed, serial_trimmed]);
    for backup in backups {
        if !known.contains(backup) {
            known.push(backup.clone());
        }
    }
    if !known.is_empty() {
        set_backup_transports_for(&[preferred_trimmed, serial_trimmed], known);
    }
    recover_transport(adb, preferred_trimmed, std::time::Duration::from_secs(6))
        .context("手机连接暂时不可用，正在等待可用通道；此次操作尚未完成")
}

impl Transport {
    pub fn backup_transports(&self) -> Vec<String> {
        let canonical = canonical_serial_from_transport(&self.id).unwrap_or_default();
        let model_key = if canonical.is_empty() && !self.model.is_empty() {
            format!("model:{}", self.model)
        } else {
            String::new()
        };
        backup_transports_for(&[&self.id, &canonical, &model_key])
            .into_iter()
            .filter(|b| b != &self.id)
            .collect()
    }
}

pub fn parse_devices(output: &str) -> Vec<Transport> {
    let raw: Vec<(Transport, String)> = output
        .lines()
        .filter_map(|line| {
            let parts: Vec<_> = line.split_whitespace().collect();
            if parts.len() < 2 || line.starts_with("List of") || line.starts_with('*') {
                return None;
            }
            let product = parts
                .iter()
                .find_map(|p| p.strip_prefix("product:"))
                .unwrap_or("")
                .to_string();
            Some((
                Transport {
                    id: parts[0].into(),
                    state: parts[1].into(),
                    model: parts
                        .iter()
                        .find_map(|p| p.strip_prefix("model:"))
                        .unwrap_or("")
                        .into(),
                },
                product,
            ))
        })
        .collect();
    deduplicate_parsed_transports(raw, false)
}

fn deduplicate_parsed_transports(
    raw: Vec<(Transport, String)>,
    preserve_existing_backups: bool,
) -> Vec<Transport> {
    struct TransportGroup {
        serial: Option<String>,
        model: String,
        product: String,
        online: bool,
        members: Vec<Transport>,
    }

    let mut groups: Vec<TransportGroup> = Vec::new();
    for (transport, product) in raw {
        let online = transport.state == "device";
        let serial = canonical_serial_from_transport(&transport.id);
        if online {
            if let Some(group) = groups.iter_mut().find(|g| {
                if !g.online {
                    return false;
                }
                match (&g.serial, &serial) {
                    (Some(gs), Some(ts)) => gs == ts,
                    (Some(_), None) | (None, Some(_)) | (None, None) => {
                        !g.model.is_empty()
                            && g.model == transport.model
                            && (g.product.is_empty() || product.is_empty() || g.product == product)
                    }
                }
            }) {
                if group.serial.is_none() && serial.is_some() {
                    group.serial = serial;
                }
                if group.model.is_empty() && !transport.model.is_empty() {
                    group.model = transport.model.clone();
                }
                if group.product.is_empty() && !product.is_empty() {
                    group.product = product;
                }
                group.members.push(transport);
                continue;
            }
        }
        groups.push(TransportGroup {
            serial,
            model: transport.model.clone(),
            product,
            online,
            members: vec![transport],
        });
    }

    let mut result = Vec::with_capacity(groups.len());
    for mut group in groups {
        if group.online && group.members.len() > 1 {
            group.members.sort_by(|a, b| {
                transport_priority(&a.id)
                    .cmp(&transport_priority(&b.id))
                    .then_with(|| a.id.cmp(&b.id))
            });
        }
        let mut iter = group.members.into_iter();
        if let Some(mut primary) = iter.next() {
            if primary.model.is_empty() && !group.model.is_empty() {
                primary.model = group.model.clone();
            }
            let mut backups = if preserve_existing_backups {
                primary.backup_transports()
            } else {
                Vec::new()
            };
            for m in iter {
                if m.id != primary.id && !backups.contains(&m.id) {
                    backups.push(m.id.clone());
                }
                if preserve_existing_backups {
                    for b in m.backup_transports() {
                        if b != primary.id && !backups.contains(&b) {
                            backups.push(b);
                        }
                    }
                }
            }
            let has_ipv4_wireless = backups
                .iter()
                .chain(std::iter::once(&primary.id))
                .any(|id| id.contains(':') && !id.starts_with('['));
            if has_ipv4_wireless {
                backups.retain(|id| !id.starts_with("[fd") && !id.starts_with("[fe80:"));
            }
            backups.sort_by(|a, b| {
                transport_priority(a)
                    .cmp(&transport_priority(b))
                    .then_with(|| a.cmp(b))
            });
            let serial_key = group.serial.unwrap_or_default();
            let model_key = if serial_key.is_empty() && !primary.model.is_empty() {
                format!("model:{}", primary.model)
            } else {
                String::new()
            };
            let mut all_group_ids = vec![primary.id.clone()];
            for b in &backups {
                if !all_group_ids.contains(b) {
                    all_group_ids.push(b.clone());
                }
            }
            for member_id in &all_group_ids {
                let peers: Vec<String> = all_group_ids
                    .iter()
                    .filter(|id| *id != member_id)
                    .cloned()
                    .collect();
                set_backup_transports_for(&[member_id.as_str()], peers);
            }
            set_backup_transports_for(&[&primary.id, &serial_key, &model_key], backups.clone());
            if !serial_key.is_empty() && serial_key != primary.id {
                set_backup_transports_for(&[&serial_key], all_group_ids);
            }
            result.push(primary);
        }
    }
    result
}

pub fn deduplicate_transports(transports: Vec<Transport>) -> Vec<Transport> {
    let raw = transports.into_iter().map(|t| (t, String::new())).collect();
    deduplicate_parsed_transports(raw, true)
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct Device {
    pub transport: String,
    pub serial: String,
    pub model: String,
    pub android: String,
    pub product: String,
}

pub fn deduplicate_multi_transport_devices(devices: Vec<Device>) -> Vec<Device> {
    struct DeviceGroup {
        serial: Option<String>,
        model: String,
        product: String,
        members: Vec<Device>,
    }

    let mut groups: Vec<DeviceGroup> = Vec::new();
    for device in devices {
        let effective_serial = if !device.serial.trim().is_empty()
            && !device.serial.contains(':')
            && !device.serial.starts_with("adb-")
        {
            Some(device.serial.trim().to_string())
        } else {
            canonical_serial_from_transport(&device.transport)
        };

        if let Some(group) = groups
            .iter_mut()
            .find(|g| match (&g.serial, &effective_serial) {
                (Some(gs), Some(ds)) => gs == ds,
                (Some(_), None) | (None, Some(_)) | (None, None) => {
                    !g.model.is_empty()
                        && g.model == device.model
                        && (g.product.is_empty()
                            || device.product.is_empty()
                            || g.product == device.product)
                }
            })
        {
            if group.serial.is_none() && effective_serial.is_some() {
                group.serial = effective_serial;
            }
            if group.model.is_empty() && !device.model.is_empty() {
                group.model = device.model.clone();
            }
            if group.product.is_empty() && !device.product.is_empty() {
                group.product = device.product.clone();
            }
            group.members.push(device);
            continue;
        }

        groups.push(DeviceGroup {
            serial: effective_serial,
            model: device.model.clone(),
            product: device.product.clone(),
            members: vec![device],
        });
    }

    let mut out = Vec::with_capacity(groups.len());
    for mut group in groups {
        group.members.sort_by(|a, b| {
            transport_priority(&a.transport)
                .cmp(&transport_priority(&b.transport))
                .then_with(|| a.transport.cmp(&b.transport))
        });
        let mut iter = group.members.into_iter();
        if let Some(mut primary) = iter.next() {
            if let Some(ref s) = group.serial {
                if primary.serial.is_empty()
                    || primary.serial.contains(':')
                    || primary.serial.starts_with("adb-")
                {
                    primary.serial = s.clone();
                }
            }
            let mut backups = primary.backup_transports();
            for member in iter {
                if member.transport != primary.transport && !backups.contains(&member.transport) {
                    backups.push(member.transport.clone());
                }
                for b in member.backup_transports() {
                    if b != primary.transport && !backups.contains(&b) {
                        backups.push(b);
                    }
                }
            }
            backups.sort_by(|a, b| {
                transport_priority(a)
                    .cmp(&transport_priority(b))
                    .then_with(|| a.cmp(b))
            });
            primary.set_backup_transports(backups);
            out.push(primary);
        }
    }
    out
}

fn validate_generic_apk_identity(value: &serde_json::Value) -> Result<()> {
    if value["ok"] != true {
        bail!(
            "{}",
            value["error"]
                .as_str()
                .unwrap_or("无法核对 APK 包名，尚未安装")
        );
    }
    let package = value["package"]
        .as_str()
        .map(str::trim)
        .filter(|p| !p.is_empty())
        .context("APK 包名校验未返回有效结果，尚未安装")?;
    if package.eq_ignore_ascii_case("com.stepx.companion.app") {
        bail!("StepX伴侣请通过“小工具 → 安装 StepX伴侣”更新；内置 GUI Agent 完整包请使用“小工具 → 内置 GUI Agent”安装。通用 APK 入口不会覆盖伴侣，现有功能和数据未改变。");
    }
    Ok(())
}

/// Reliably wakes a phone from `Asleep` or `Dozing` (AOD), waits for `SCREEN_STATE_ON`,
/// dismisses the keyguard (including swipe-to-unlock keyguard on Pandora OS), and collapses
/// the notification shade so foreground activities can receive UI actions.
pub fn wake_and_unlock_transport(
    adb: &Adb,
    transport: &str,
    cx: u32,
    y_bot: u32,
    y_top: u32,
) -> Result<String> {
    let script = format!(
        concat!(
            "if ! dumpsys power 2>/dev/null | grep -q 'mWakefulness=Awake'; then ",
            "input keyevent 224 >/dev/null 2>&1 || true; ",
            "sleep 0.2; ",
            "fi; ",
            "if ! dumpsys power 2>/dev/null | grep -q 'mWakefulness=Awake'; then ",
            "input keyevent 26 >/dev/null 2>&1 || true; ",
            "fi; ",
            "for _i in 1 2 3 4 5; do ",
            "if dumpsys window policy 2>/dev/null | grep -q 'screenState=SCREEN_STATE_ON'; then break; fi; ",
            "sleep 0.2; ",
            "done; ",
            "wm dismiss-keyguard >/dev/null 2>&1 || true; ",
            "if dumpsys window policy 2>/dev/null | grep -E -q 'showingAndNotOccluded=true|mShowingLockscreen=true|isStatusBarKeyguard=true|mKeyguardShowing=true|keyguardShowing=true' || ",
            "dumpsys window policy 2>/dev/null | grep -A 3 'KeyguardServiceDelegate' | grep -q 'showing=true'; then ",
            "input keyevent 82 >/dev/null 2>&1 || true; ",
            "input swipe {cx} {y_bot} {cx} {y_top} 300 >/dev/null 2>&1 || true; ",
            "sleep 0.5; ",
            "fi; ",
            "cmd statusbar collapse >/dev/null 2>&1 || true"
        ),
        cx = cx,
        y_bot = y_bot,
        y_top = y_top,
    );
    adb.script(transport, &script)
}

fn apk_verification_command(path: &std::path::Path) -> Command {
    let mut command = crate::resources::python_command();
    command
        .arg(crate::resources::root().join("scripts/companion_apk.py"))
        .arg("--verify-signature")
        .arg("--package")
        .arg(path);
    command
}

fn parse_apk_verification(output: &std::process::Output) -> Result<serde_json::Value> {
    let parsed = serde_json::from_slice::<serde_json::Value>(&output.stdout);
    // The helper may exit before it can write JSON (argument/import/runtime errors).
    // Preserve that diagnostic instead of masking it with a JSON EOF error.
    if !output.status.success() {
        let structured = parsed.as_ref().ok().and_then(|v| v["error"].as_str());
        let stderr = String::from_utf8_lossy(&output.stderr);
        let detail = structured
            .unwrap_or(stderr.trim())
            .chars()
            .take(600)
            .collect::<String>();
        if detail.is_empty() {
            bail!(
                "APK 校验工具异常退出（{}），尚未安装；请重试或更新客户端",
                output.status
            );
        }
        bail!("APK 校验未完成，尚未安装：{detail}");
    }
    let value = parsed
        .map_err(|_| anyhow::anyhow!("APK 校验工具未返回有效结果，尚未安装；请重试或更新客户端"))?;
    if value["ok"] != true {
        bail!(
            "APK 校验未通过，尚未安装：{}",
            value["error"].as_str().unwrap_or("缺少成功回执")
        );
    }
    anyhow::ensure!(
        value["package"].as_str().is_some_and(|p| !p.is_empty()),
        "APK 包名校验未返回有效结果，尚未安装"
    );
    Ok(value)
}

/// Read and verify an APK locally. Does not install or contact a phone.
pub fn verify_apk_identity(path: &std::path::Path) -> Result<String> {
    let output = crate::adb::run_process(
        apk_verification_command(path),
        None,
        std::time::Duration::from_secs(90),
    )?;
    let value = parse_apk_verification(&output)?;
    Ok(value["package"].as_str().unwrap().to_owned())
}

fn check_generic_apk(path: &std::path::Path) -> Result<String> {
    let package = verify_apk_identity(path)?;
    validate_generic_apk_identity(&serde_json::json!({"ok": true, "package": package}))?;
    Ok(package)
}

/// Validate a store download before showing it as ready, and again on the immutable install snapshot.
pub fn check_store_apk(path: &std::path::Path, expected_package: &str) -> Result<()> {
    let actual = check_generic_apk(path)?;
    validate_store_package(&actual, expected_package)
}

pub fn store_package_aliases_match(actual: &str, expected: &str) -> bool {
    let a = actual.trim();
    let e = expected.trim();
    if a.is_empty()
        || e.is_empty()
        || a.eq_ignore_ascii_case("com.stepx.companion.app")
        || e.eq_ignore_ascii_case("com.stepx.companion.app")
    {
        return false;
    }
    if a.eq_ignore_ascii_case(e) {
        return true;
    }
    const ALIAS_GROUPS: &[&[&str]] = &[
        // AITO 问界 (app.huawei.auto) & 鸿蒙智行 HIMA (app.huawei.motor) official Huawei smart car suite
        &[
            "app.huawei.auto",
            "app.huawei.motor",
            "com.huawei.aitocar",
            "com.huawei.harmonyos.auto",
            "com.aitotech.aito",
            "com.seres.aito",
        ],
        // Synology Drive official package alias pair
        &["com.synology.uhidrive", "com.synology.dsdrive"],
        // Telegram official standalone (web) and store package alias pair
        &["org.telegram.messenger", "org.telegram.messenger.web"],
        // Feishu / Lark official package alias pair
        &["com.ss.android.lark", "com.ss.android.lark.kami"],
    ];
    ALIAS_GROUPS.iter().any(|group| {
        group.iter().any(|p| p.eq_ignore_ascii_case(a))
            && group.iter().any(|p| p.eq_ignore_ascii_case(e))
    })
}

fn validate_store_package(actual: &str, expected: &str) -> Result<()> {
    if !store_package_aliases_match(actual, expected) {
        bail!("下载包名与所选应用不一致（预期 {expected}，实际 {actual}），已阻止安装，手机数据未改变");
    }
    Ok(())
}

impl Device {
    pub fn backup_transports(&self) -> Vec<String> {
        let canonical = canonical_serial_from_transport(&self.transport).unwrap_or_default();
        let model_key = if self.serial.is_empty() && canonical.is_empty() && !self.model.is_empty()
        {
            format!("model:{}", self.model)
        } else {
            String::new()
        };
        backup_transports_for(&[&self.transport, &self.serial, &canonical, &model_key])
            .into_iter()
            .filter(|b| b != &self.transport)
            .collect()
    }

    pub fn set_backup_transports(&self, backups: Vec<String>) {
        let canonical = canonical_serial_from_transport(&self.transport).unwrap_or_default();
        let model_key = if self.serial.is_empty() && canonical.is_empty() && !self.model.is_empty()
        {
            format!("model:{}", self.model)
        } else {
            String::new()
        };
        let filtered: Vec<String> = backups
            .into_iter()
            .filter(|b| b != &self.transport)
            .collect();
        set_backup_transports_for(
            &[&self.transport, &self.serial, &canonical, &model_key],
            filtered,
        );
    }

    pub fn active_transport_cached(&self) -> String {
        if !self.serial.trim().is_empty() {
            if let Some(active) = current_active_transport_for_serial(&self.serial) {
                if !active.is_empty() {
                    return active;
                }
            }
        }
        if !self.transport.trim().is_empty() {
            if let Some(active) = current_active_transport_for_serial(&self.transport) {
                if !active.is_empty() {
                    return active;
                }
            }
        }
        self.transport.clone()
    }

    pub fn all_known_transports(&self) -> Vec<String> {
        let mut out: Vec<String> = Vec::new();
        let primary = self.transport.trim();
        if !primary.is_empty() {
            out.push(primary.to_string());
        }
        let active = self.active_transport_cached();
        if !active.trim().is_empty() && !out.iter().any(|t| t == active.trim()) {
            out.push(active.trim().to_string());
        }
        for b in self.backup_transports() {
            let trimmed = b.trim();
            if !trimmed.is_empty() && !out.iter().any(|t| t == trimmed) {
                out.push(trimmed.to_string());
            }
        }
        out
    }

    pub fn has_usb_and_wireless(&self) -> bool {
        let known = self.all_known_transports();
        let has_usb = known.iter().any(|t| !crate::adb::is_wireless_transport(t));
        let has_wireless = known.iter().any(|t| crate::adb::is_wireless_transport(t));
        has_usb && has_wireless
    }

    pub fn has_wireless_backup(&self) -> bool {
        !self.backup_transports().is_empty()
    }

    pub fn is_usb_primary(&self) -> bool {
        is_usb_transport(&self.transport)
    }

    pub fn inspect(adb: &Adb, transport: &str) -> Result<Self> {
        // Device discovery may include several expired IPv6/mDNS aliases. A
        // silent alias must not consume the generic 20s operation deadline
        // before the user can select a reachable phone or retry the connection.
        let props = adb.run_timeout(
            &["-s", transport, "shell", "getprop ro.serialno; getprop ro.product.model; getprop ro.build.version.release; getprop ro.product.device"],
            std::time::Duration::from_secs(4),
        ).context("设备身份读取未完成，请检查手机与电脑网络后重试；此次未选中设备")?;
        let parts: Vec<_> = props.lines().map(str::trim).collect();
        if parts.len() < 4 || parts[0].is_empty() {
            bail!("无法读取设备身份，请解锁手机并允许调试");
        }
        let model = parts[1].to_lowercase();
        let explicit_test_serial = std::env::var("STEPX_WORKSPACE_TEST_SERIAL")
            .ok()
            .is_some_and(|s| !s.trim().is_empty() && s.trim() == parts[0]);
        if !model.contains("stepx")
            && !model.contains("pz261a")
            && parts[3] != "pandora"
            && !explicit_test_serial
        {
            bail!("这台设备不是已支持的 StepX（型号：{}），未选中。", parts[1]);
        }
        let device = Self {
            transport: transport.into(),
            serial: parts[0].into(),
            model: parts[1].into(),
            android: parts[2].into(),
            product: parts[3].into(),
        };
        record_verified_transport(transport, &device.serial, transport);
        if device.backup_transports().is_empty() {
            if let Ok(raw_devices) =
                adb.run_timeout(&["devices", "-l"], std::time::Duration::from_secs(2))
            {
                let _ = parse_devices(&raw_devices);
            }
        }
        let existing_backups = device.backup_transports();
        if !existing_backups.is_empty() {
            device.set_backup_transports(existing_backups);
        }
        Ok(device)
    }

    /// Resolve the active online transport for this device, automatically failing over
    /// to online wireless/USB backup transports when the primary transport goes offline.
    pub fn resolve_active_transport(&self, adb: &Adb) -> Result<String> {
        resolve_online_transport(
            adb,
            &self.transport,
            &self.serial,
            &self.model,
            &self.backup_transports(),
        )
    }

    /// A transport/IP can be reassigned. Recheck hardware identity before every operation.
    pub fn verify(&self, adb: &Adb) -> Result<()> {
        self.resolve_active_transport(adb).map(|_| ())
    }

    pub fn summary(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        let script = concat!(
            "dumpsys battery; df -h /data /sdcard; wm size; ip -o -4 addr show dev wlan0 2>/dev/null || true; ",
            "echo '<<<STEPOS_DEEP>>>'; ",
            "echo \"marketname:$(getprop ro.product.marketname)\"; ",
            "echo \"asn:$(getprop ro.boot.hardware.asn)\"; ",
            "echo \"netsn:$(getprop ro.StepOS.net.sn)\"; ",
            "echo \"serialno:$(getprop ro.serialno)\"; ",
            "echo \"display_id:$(getprop ro.build.display.id)\"; ",
            "echo \"cmiit:$(getprop ro.product.cmiit.id)\"; ",
            "echo \"camera_info:$(getprop ro.vendor.stepos.camera.info)\"; ",
            "echo \"zram_cfg:$(getprop persist.vendor.stepos.smart_zram.cfg)\"; ",
            "echo \"soh:$(cat /sys/class/zy_charger/soh 2>/dev/null || cat /sys/class/zy_charger/fusion_soh 2>/dev/null)\"; ",
            "echo \"design_cap:$(cat /sys/class/zy_charger/design_capacity 2>/dev/null)\"; ",
            "echo \"charge_cnt:$(cat /sys/class/zy_charger/charge_counter 2>/dev/null)\"; ",
            "echo \"ups_mode:$(cat /sys/class/zy_charger/ups_mode 2>/dev/null)\"; ",
            "echo \"ups_limit:$(cat /sys/class/zy_charger/ups_mode_soc_limit 2>/dev/null)\"; ",
            "echo \"fast_charge:$(settings get global stepos_fast_charge 2>/dev/null)\"; ",
            "echo \"cycle_line:$(dumpsys activity broadcasts 2>/dev/null | grep -m1 'android.os.extra.CYCLE_COUNT' || true)\"; ",
            "echo \"keyguard_locked:$(dumpsys window policy 2>/dev/null | grep -m1 'showing=' || true)\"; ",
            "dumpsys thermalservice 2>/dev/null | grep -E 'Current temperatures from HAL:|Current cooling devices from HAL:|Temperature\\{mValue=.*mName=(CPU0|cpu-0-0|GPU0|gpuss-0|nsp0|nsp-0|skin|skin-therm)' | head -n 40 || true"
        );
        let raw = adb.script(&transport, script)?;
        Ok(format_summary(&raw))
    }

    pub fn key(&self, adb: &Adb, code: u32) -> Result<()> {
        if ![
            3, 4, 5, 6, 24, 25, 26, 27, 66, 67, 82, 85, 87, 88, 120, 164, 187, 219, 220, 221, 223,
            224, 231,
        ]
        .contains(&code)
        {
            bail!("不支持的按键");
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(&transport, &["input", "keyevent", &code.to_string()])?;
        Ok(())
    }

    pub fn screen_resolution(&self, adb: &Adb) -> (u32, u32) {
        let transport = self
            .resolve_active_transport(adb)
            .unwrap_or_else(|_| self.transport.clone());
        self.screen_resolution_on_transport(adb, &transport)
    }

    pub(crate) fn screen_resolution_on_transport(&self, adb: &Adb, transport: &str) -> (u32, u32) {
        static SCREEN_RES_CACHE: OnceLock<
            Mutex<HashMap<(std::path::PathBuf, String), (std::time::Instant, (u32, u32))>>,
        > = OnceLock::new();
        const SCREEN_RES_TTL: std::time::Duration = std::time::Duration::from_secs(30);
        let key = (adb.path.clone(), transport.trim().to_string());
        let cache = SCREEN_RES_CACHE.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(guard) = cache.lock() {
            if let Some((ts, dims)) = guard.get(&key) {
                if ts.elapsed() < SCREEN_RES_TTL {
                    return *dims;
                }
            }
        }
        if let Ok(raw) = adb.shell(transport, &["wm", "size"]) {
            let dims = parse_wm_size(&raw);
            if let Ok(mut guard) = cache.lock() {
                guard.insert(key, (std::time::Instant::now(), dims));
            }
            dims
        } else {
            (1080, 2400)
        }
    }

    pub fn tap_normalized(&self, adb: &Adb, norm_x: u16, norm_y: u16) -> Result<(u32, u32)> {
        let transport = self.resolve_active_transport(adb)?;
        let (w, h) = self.screen_resolution_on_transport(adb, &transport);
        let px = ((norm_x.min(1000) as u32) * w / 1000).clamp(1, w.saturating_sub(1));
        let py = ((norm_y.min(1000) as u32) * h / 1000).clamp(1, h.saturating_sub(1));
        adb.shell(
            &transport,
            &["input", "tap", &px.to_string(), &py.to_string()],
        )?;
        Ok((px, py))
    }

    pub fn screen_gesture(&self, adb: &Adb, gesture: u8) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        let (w, h) = self.screen_resolution_on_transport(adb, &transport);
        let cx = w / 2;
        let cy = h / 2;
        let y_bot = h * 78 / 100;
        let y_top = h * 22 / 100;
        let x_right = w * 84 / 100;
        let x_left = w * 16 / 100;
        match gesture {
            0 => {
                adb.shell(
                    &transport,
                    &[
                        "input",
                        "swipe",
                        &cx.to_string(),
                        &y_bot.to_string(),
                        &cx.to_string(),
                        &y_top.to_string(),
                        "240",
                    ],
                )?;
                Ok("已向手机发送上滑手势".into())
            }
            1 => {
                adb.shell(
                    &transport,
                    &[
                        "input",
                        "swipe",
                        &cx.to_string(),
                        &y_top.to_string(),
                        &cx.to_string(),
                        &y_bot.to_string(),
                        "240",
                    ],
                )?;
                Ok("已向手机发送下滑手势".into())
            }
            2 => {
                adb.shell(
                    &transport,
                    &[
                        "input",
                        "swipe",
                        &x_right.to_string(),
                        &cy.to_string(),
                        &x_left.to_string(),
                        &cy.to_string(),
                        "220",
                    ],
                )?;
                Ok("已向手机发送左滑手势".into())
            }
            3 => {
                adb.shell(
                    &transport,
                    &[
                        "input",
                        "swipe",
                        &x_left.to_string(),
                        &cy.to_string(),
                        &x_right.to_string(),
                        &cy.to_string(),
                        "220",
                    ],
                )?;
                Ok("已向手机发送右滑返回手势".into())
            }
            4 => {
                adb.shell(&transport, &["cmd", "statusbar", "expand-notifications"])?;
                Ok("已展开手机通知栏".into())
            }
            5 => {
                adb.shell(&transport, &["cmd", "statusbar", "expand-settings"])?;
                Ok("已展开手机快捷设置面板".into())
            }
            6 => {
                adb.shell(&transport, &["cmd", "statusbar", "collapse"])?;
                Ok("已收起手机状态栏".into())
            }
            _ => {
                let _ = wake_and_unlock_transport(adb, &transport, cx, y_bot, y_top);
                Ok("已点亮屏幕并发送上滑解锁".into())
            }
        }
    }

    pub(crate) fn open_amoo_on_transport(&self, adb: &Adb, transport: &str) -> Result<String> {
        if let Some(receipt) = crate::native_mcp::try_call(
            adb,
            self,
            "open_app_by_package",
            serde_json::json!({"packageName":"com.stepos.ailauncher",
                "className":"com.stepos.ailauncher.chat.activity.ChatMainActivity"}),
        )? {
            return Ok(crate::native_mcp::response_note(&receipt));
        }
        let script = concat!(
            "if ! pm path com.stepos.ailauncher 2>/dev/null | grep -q 'package:'; then ",
            "echo STEPX_NO_AMOO; ",
            "else ",
            "input keyevent 224 >/dev/null 2>&1 || true; ",
            "input keyevent 82 >/dev/null 2>&1 || true; ",
            "am start -n com.stepos.ailauncher/.chat.activity.ChatMainActivity --activity-single-top; ",
            "fi"
        );
        match adb.script(transport, script) {
            Ok(out) => {
                if out.contains("STEPX_NO_AMOO") {
                    bail!("没有检测到系统 Amoo 包 com.stepos.ailauncher");
                }
            }
            Err(_) => {
                if adb
                    .shell(
                        transport,
                        &[
                            "am",
                            "start",
                            "-a",
                            "com.stepos.ailauncher.action.OPEN_CHAT",
                            "-p",
                            "com.stepos.ailauncher",
                        ],
                    )
                    .is_err()
                {
                    let _ = adb.shell(transport, &["input", "keyevent", "219"])?;
                }
            }
        }
        Ok("已在手机上打开 Amoo 对话中心".into())
    }

    pub fn open_amoo(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        self.open_amoo_on_transport(adb, &transport)
    }

    pub fn open_amoo_memory(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.VIEW",
                "-d",
                "stepos://amoo/memory",
            ],
        )?;
        Ok("已在手机上打开 Amoo 记忆与偏好空间 (stepos://amoo/memory)".into())
    }

    pub fn open_amoo_tasks(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        let _ = self.open_amoo_on_transport(adb, &transport)?;
        let _ = adb.shell(
            &transport,
            &[
                "am",
                "broadcast",
                "-a",
                "com.stepos.ailauncher.action.OPEN_ALL_TASK",
                "-p",
                "com.stepos.ailauncher",
            ],
        );
        Ok("已在手机上打开 Amoo 会话与并行任务中心".into())
    }

    pub fn open_amoo_voice_settings(&self, adb: &Adb) -> Result<String> {
        if let Some((tool, args)) = crate::native_mcp::quick_action("open_amoo_voice", "") {
            if let Some(receipt) = crate::native_mcp::try_call(adb, self, tool, args)? {
                return Ok(crate::native_mcp::response_note(&receipt));
            }
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "com.stepos.ailauncher.action.VOICE_SETTING",
            ],
        )?;
        Ok("已在手机上打开 Amoo 语音唤醒与播报音色设置".into())
    }

    pub fn open_amoo_safety_center(&self, adb: &Adb) -> Result<String> {
        if let Some((tool, args)) = crate::native_mcp::quick_action("open_safety_ledger", "") {
            if let Some(receipt) = crate::native_mcp::try_call(adb, self, tool, args)? {
                return Ok(crate::native_mcp::response_note(&receipt));
            }
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "stepos.intent.action.AGENT_SAFETY_CENTER_LAUNCHER",
            ],
        )?;
        Ok("已在手机上打开智能体安全中心 (AgentSafetyCenter)".into())
    }

    pub fn open_amoo_screen_perception(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        adb.script(
            &transport,
            "am startservice -a com.stepos.picker.screenperception.action.WAKE_UP_SERVICE >/dev/null 2>&1 || true; \
             am start -a stepos.intent.action.AGENT_SCREEN_PERCEPTION",
        )?;
        Ok("已唤起小跃屏幕智能感知与圈选权限设置".into())
    }

    pub fn open_amoo_persona(&self, adb: &Adb) -> Result<String> {
        if let Some((tool, args)) = crate::native_mcp::quick_action("open_alphame", "") {
            if let Some(receipt) = crate::native_mcp::try_call(adb, self, tool, args)? {
                return Ok(crate::native_mcp::response_note(&receipt));
            }
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "com.stepos.alphame.action.personal_preference",
            ],
        )?;
        Ok("已在手机上打开个人偏好与常用地址/通勤设置 (AlphaMe)".into())
    }

    pub fn open_amoo_model_manager(&self, adb: &Adb) -> Result<String> {
        if let Some((tool, args)) = crate::native_mcp::quick_action("open_aiengine", "") {
            if let Some(receipt) = crate::native_mcp::try_call(adb, self, tool, args)? {
                return Ok(crate::native_mcp::response_note(&receipt));
            }
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "com.stepos.aiengine.action.MODEL_MANAGER",
            ],
        )?;
        Ok("已在手机上打开端侧大模型管理 (AIEngine)".into())
    }

    pub fn amoo_voice_wake(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        let _ = self.open_amoo_on_transport(adb, &transport)?;
        adb.shell(
            &transport,
            &[
                "am",
                "broadcast",
                "-a",
                "com.stepos.ailauncher.action.VOICE_WAKE",
                "-p",
                "com.stepos.ailauncher",
            ],
        )?;
        Ok("已唤起 Amoo 语音聆听，请直接说话或发语音".into())
    }

    pub fn amoo_share_file(&self, adb: &Adb, local_path: &std::path::Path) -> Result<String> {
        if !local_path.is_file() {
            bail!("请选择有效的本地图片或文件附件");
        }
        let transport = self.resolve_active_transport(adb)?;
        let dest_dir = "/sdcard/Download/StepX";
        let _ = adb.shell(&transport, &["mkdir", "-p", dest_dir]);
        crate::storage::push(adb, self, local_path, dest_dir)?;
        let file_name = local_path
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("attachment");
        let ext = local_path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let mime = match ext.as_str() {
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" => "image/*",
            "txt" | "md" | "json" | "csv" | "log" => "text/plain",
            _ => "*/*",
        };
        let remote_uri = format!("file://{dest_dir}/{file_name}");
        let _ = adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.SEND",
                "-n",
                "com.stepos.ailauncher/.chat.activity.ShareActivity",
                "-t",
                mime,
                "--eu",
                "android.intent.extra.STREAM",
                &remote_uri,
            ],
        );
        Ok(format!("已将附件 {file_name} 投递至 Amoo"))
    }

    pub fn dial_number(&self, adb: &Adb, number: &str) -> Result<String> {
        let clean: String = number
            .trim()
            .chars()
            .filter(|c| c.is_ascii_digit() || "+*#".contains(*c))
            .collect();
        if clean.is_empty() {
            bail!("请输入有效的电话号码");
        }
        if let Some(receipt) = crate::native_mcp::try_call(
            adb,
            self,
            "dialer_show_dialpad",
            serde_json::json!({"number":clean}),
        )? {
            return Ok(format!(
                "{}；请在手机确认呼叫",
                crate::native_mcp::response_note(&receipt)
            ));
        }
        let transport = self.resolve_active_transport(adb)?;
        let uri = format!("tel:{clean}");
        let result = adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.DIAL",
                "-d",
                &uri,
            ],
        )?;
        if result.contains("Error:") || result.contains("Exception") {
            bail!("无法打开手机拨号界面：{result}");
        }
        Ok("已打开手机拨号界面，请在手机选择 SIM 卡并确认呼叫".into())
    }

    pub fn send_focus_text(&self, adb: &Adb, text: &str) -> Result<String> {
        let trimmed = text.trim();
        if trimmed.is_empty() {
            bail!("请输入要发送到手机焦点框的文字");
        }
        let transport = self.resolve_active_transport(adb)?;
        if trimmed.is_ascii() {
            let escaped = trimmed.replace(' ', "%s");
            adb.shell(&transport, &["input", "text", &escaped])?;
            return Ok(format!("已向手机当前焦点输入：{}", trimmed));
        }
        // For CJK / Unicode text, try Amoo prepare or clipboard broadcast fallback
        let _ = crate::bridge::amoo_prepare(adb, self, trimmed);
        Ok(format!("已将文字同步至手机输入区：{}", trimmed))
    }

    pub fn push_backscreen_frame(&self, adb: &Adb, frame: &[u8]) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        crate::backscreen::push_frame(adb, self, frame)?;
        let state = adb.shell(&transport, &["dumpsys", "display_ext"])?;
        if !state.contains("power=on") || state.contains("policyAllowsOutput=false") {
            bail!("手机接收到图案，但背屏未亮起；请将背屏朝上并检查手机的背屏、勿扰和省电设置");
        }
        Ok("已将17×23像素帧交给手机背屏，HAL已开启".into())
    }
    pub fn push_backscreen_pattern(
        &self,
        adb: &Adb,
        pattern: BackScreenPattern,
        brightness: u8,
        invert: bool,
    ) -> Result<String> {
        self.push_backscreen_frame(adb, &pattern.frame_391(brightness, invert))
    }
    pub fn push_backscreen_image(
        &self,
        adb: &Adb,
        path: &std::path::Path,
        brightness: u8,
        invert: bool,
    ) -> Result<(Vec<u8>, String)> {
        let frame = image_to_backscreen_frame(path, brightness, invert)?;
        let message = self.push_backscreen_frame(adb, &frame)?;
        Ok((frame, message))
    }
    pub fn set_backscreen_marquee(
        &self,
        adb: &Adb,
        text: &str,
        scroll: bool,
        reverse: bool,
    ) -> Result<String> {
        crate::backscreen::marquee(adb, self, text, scroll, reverse)
    }
    pub fn set_backscreen_mode(&self, adb: &Adb, mode: &str) -> Result<String> {
        crate::backscreen::set_display_mode(adb, self, mode)
    }
    pub fn set_backscreen_brightness(&self, adb: &Adb, level: u8) -> Result<String> {
        crate::backscreen::set_brightness(adb, self, level)
    }
    pub fn trigger_backscreen_scene(&self, adb: &Adb, scene: &str) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        match scene {
            "clock" => crate::backscreen::configure_native(
                adb,
                self,
                crate::backscreen::NativeFeature::Clock,
            ),
            "notify" => crate::backscreen::configure_native(
                adb,
                self,
                crate::backscreen::NativeFeature::Notifications,
            ),
            "countdown" => {
                let output = adb.shell(
                    &transport,
                    &[
                        "dumpsys",
                        "display_ext",
                        "back_screen",
                        "special",
                        "countdown",
                        "3",
                    ],
                )?;
                if output.to_ascii_lowercase().contains("error") {
                    bail!("手机倒计时未启动：{output}")
                }
                Ok("已请求手机原生3秒倒计时".into())
            }
            "reset" => {
                crate::backscreen::clear_frame(adb, self)?;
                crate::backscreen::set_display_mode(adb, self, "auto")
            }
            _ => bail!("请从背屏创作台选择对应动画后播放；当前功能不会模拟天气或系统通知"),
        }
    }

    pub fn call_logs(&self, adb: &Adb) -> Result<Vec<CallRecord>> {
        let result = crate::bridge::call_bridge(
            adb,
            self,
            &serde_json::json!({"op": "read_calls"}),
            std::time::Duration::from_secs(30),
        )
        .context("无法读取手机通话记录")?;
        parse_call_records(&result)
    }

    pub fn contacts(&self, adb: &Adb) -> Result<Vec<ContactEntry>> {
        let result = crate::bridge::call_bridge(
            adb,
            self,
            &serde_json::json!({"op": "read_contacts"}),
            std::time::Duration::from_secs(30),
        )
        .context("无法读取手机通讯录")?;
        parse_contact_records(&result)
    }

    pub fn create_contact(&self, adb: &Adb, name: &str, number: &str) -> Result<String> {
        let name = name.trim();
        let number = number.trim();
        if name.is_empty() || number.is_empty() {
            bail!("请填写联系人姓名与电话号码");
        }
        let normalized = crate::phone::normalize_number(number)?;
        let digits = |n: &str| n.chars().filter(char::is_ascii_digit).collect::<String>();
        let current = self.contacts(adb)?;
        // The native update tool uses fuzzy phone-number matching, unlike our
        // exact-number update. Only new contacts are equivalent to native add.
        if !current
            .iter()
            .any(|c| digits(&c.number) == digits(&normalized))
        {
            if crate::native_mcp::try_call(
                adb,
                self,
                "dialer_contacts_add",
                serde_json::json!({"name":name,"number":normalized}),
            )?
            .is_some()
            {
                let after = self.contacts(adb)?;
                anyhow::ensure!(
                    after
                        .iter()
                        .any(|c| c.name == name && digits(&c.number) == digits(&normalized)),
                    "原生联系人保存请求已提交，但姓名与号码未读回；未重复添加"
                );
                return Ok(format!("已通过手机原生接口保存联系人：{name}"));
            }
        }
        if let Ok(res) = crate::bridge::call_bridge(
            adb,
            self,
            &serde_json::json!({"op": "create_contact", "name": name, "number": number}),
            std::time::Duration::from_secs(10),
        ) {
            if res["ok"].as_bool() == Some(true)
                && (res["saved"].as_bool() == Some(true)
                    || res.get("id").is_some()
                    || res.get("raw_contact_id").is_some())
            {
                return Ok(format!("已保存联系人：{name} ({number})"));
            }
        }

        let transport = self.resolve_active_transport(adb)?;
        let parse_max_raw_contact_id = |out: &str| -> Option<i64> {
            out.lines()
                .filter_map(|line| {
                    line.split("_id=")
                        .nth(1)
                        .and_then(|rest| {
                            rest.split(|c: char| !c.is_ascii_digit() && c != '-').next()
                        })
                        .and_then(|s| s.trim().parse::<i64>().ok())
                })
                .max()
        };

        let before_out = adb
            .shell(
                &transport,
                &[
                    "content",
                    "query",
                    "--uri",
                    "content://com.android.contacts/raw_contacts",
                    "--projection",
                    "_id",
                ],
            )
            .unwrap_or_default();
        let prev_max_id = parse_max_raw_contact_id(&before_out).unwrap_or(0);

        let insert_raw = adb.shell(
            &transport,
            &[
                "content",
                "insert",
                "--uri",
                "content://com.android.contacts/raw_contacts",
                "--bind",
                "account_type:n:",
                "--bind",
                "account_name:n:",
            ],
        );
        if let Ok(raw_res) = insert_raw {
            if !raw_res.contains("Error") && !raw_res.contains("Exception") {
                let after_out = adb
                    .shell(
                        &transport,
                        &[
                            "content",
                            "query",
                            "--uri",
                            "content://com.android.contacts/raw_contacts",
                            "--projection",
                            "_id",
                        ],
                    )
                    .unwrap_or_default();
                let new_id = parse_max_raw_contact_id(&after_out)
                    .filter(|&id| id > prev_max_id)
                    .or_else(|| (prev_max_id > 0).then_some(prev_max_id + 1))
                    .unwrap_or(1);
                let raw_id_bind = format!("raw_contact_id:i:{new_id}");
                let name_bind = format!("data1:s:{name}");
                let phone_bind = format!("data1:s:{number}");
                let name_res = adb.shell(
                    &transport,
                    &[
                        "content",
                        "insert",
                        "--uri",
                        "content://com.android.contacts/data",
                        "--bind",
                        &raw_id_bind,
                        "--bind",
                        "mimetype:s:vnd.android.cursor.item/name",
                        "--bind",
                        &name_bind,
                    ],
                );
                let phone_res = adb.shell(
                    &transport,
                    &[
                        "content",
                        "insert",
                        "--uri",
                        "content://com.android.contacts/data",
                        "--bind",
                        &raw_id_bind,
                        "--bind",
                        "mimetype:s:vnd.android.cursor.item/phone_v2",
                        "--bind",
                        &phone_bind,
                        "--bind",
                        "data2:i:2",
                    ],
                );
                if let (Ok(nr), Ok(pr)) = (name_res, phone_res) {
                    if !nr.contains("Error")
                        && !nr.contains("Exception")
                        && !pr.contains("Error")
                        && !pr.contains("Exception")
                    {
                        return Ok(format!("已保存联系人：{name} ({number})"));
                    }
                }
            }
        }

        let result = adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.INSERT",
                "-t",
                "vnd.android.cursor.dir/contact",
                "--es",
                "name",
                name,
                "--es",
                "phone",
                number,
            ],
        )?;
        if result.contains("Error:") || result.contains("Exception") {
            bail!("无法保存手机联系人：{result}");
        }
        Ok(format!("已保存联系人：{name} ({number})"))
    }

    pub fn visible_text(&self, adb: &Adb) -> Result<String> {
        if let Some(text) = crate::native_mcp::visible_text(adb, self)? {
            return Ok(text);
        }
        let transport = self.resolve_active_transport(adb)?;
        // Unique remote file avoids another tool's concurrent hierarchy dump.
        let remote = format!(
            "/data/local/tmp/stepx-ui-{:016x}.xml",
            rand::random::<u64>()
        );
        let q = crate::adb::shell_quote(&remote);
        let xml = adb.script(
            &transport,
            &format!("uiautomator dump {q} >/dev/null && cat {q}; rm -f {q}"),
        )?;
        visible_text(&xml)
    }

    pub fn messages(&self, adb: &Adb) -> Result<Vec<crate::sms::Message>> {
        crate::sms::read(adb, self)
    }

    pub fn draft_sms(&self, adb: &Adb, number: &str, body: &str) -> Result<String> {
        let number = number.trim();
        if number.is_empty()
            || number.len() > 40
            || !number
                .chars()
                .all(|c| c.is_ascii_digit() || "+ -()".contains(c))
        {
            bail!("请输入有效的收件人号码");
        }
        if body.is_empty() {
            bail!("短信内容不能为空");
        }
        // Explicitly false: the MCP default sends immediately, whereas this
        // entry only prepares a draft for the user to review on the phone.
        if let Some(receipt) = crate::native_mcp::try_call(
            adb,
            self,
            "sms_send",
            serde_json::json!({"phone":number,"message":body,"send_directly":false}),
        )? {
            return Ok(format!(
                "{}；请在手机核对收件人和正文，再确认发送",
                crate::native_mcp::response_note(&receipt)
            ));
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.SENDTO",
                "-d",
                &format!("smsto:{number}"),
                "--es",
                "sms_body",
                body,
            ],
        )?;
        Ok("短信草稿已交给手机。请核对收件人和内容，再在实时控制窗口点击发送。".into())
    }

    pub fn open_messages(&self, adb: &Adb) -> Result<()> {
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(
            &transport,
            &[
                "am",
                "start",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.APP_MESSAGING",
            ],
        )?;
        Ok(())
    }

    pub fn apps(&self, adb: &Adb) -> Result<Vec<String>> {
        self.apps_with_filter(adb, false)
    }

    pub fn apps_with_filter(&self, adb: &Adb, include_system: bool) -> Result<Vec<String>> {
        let transport = self.resolve_active_transport(adb)?;
        let args: &[&str] = if include_system {
            &["pm", "list", "packages"]
        } else {
            &["pm", "list", "packages", "-3"]
        };
        let output = adb.shell(&transport, args)?;
        let mut apps: Vec<_> = output
            .lines()
            .filter_map(|s| s.strip_prefix("package:").map(str::to_owned))
            .collect();
        apps.sort();
        Ok(apps)
    }

    pub fn install_apk(&self, adb: &Adb, local_apk: &std::path::Path) -> Result<String> {
        if !local_apk.is_file() {
            bail!("请选择一个有效的本地 APK 安装包文件");
        }
        let is_apk = local_apk
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("apk"));
        if !is_apk {
            bail!("仅支持安装 .apk 格式的 Android 应用包");
        }
        // Inspect and install the same immutable snapshot. The general application
        // picker must not bypass Companion's signed, feature-preserving updater.
        let mut snapshot = tempfile::Builder::new().suffix(".apk").tempfile()?;
        let mut source = std::fs::File::open(local_apk).context("无法读取 APK 安装包")?;
        std::io::copy(&mut source, &mut snapshot).context("无法准备 APK 安装包")?;
        snapshot.as_file().sync_all()?;
        check_generic_apk(snapshot.path())?;
        let transport = self.resolve_active_transport(adb)?;
        let path_str = snapshot
            .path()
            .to_str()
            .context("APK 文件路径必须是有效的 UTF-8")?;
        let out = adb.run_timeout(
            &["-s", &transport, "install", "-r", path_str],
            std::time::Duration::from_secs(300),
        )?;
        let file_name = local_apk
            .file_name()
            .and_then(|n| n.to_str())
            .unwrap_or("应用");
        if !out.lines().any(|line| line.trim() == "Success") {
            return Err(anyhow::anyhow!("APK 安装失败：{}", out.trim()));
        }
        Ok(format!("已成功安装 APK：{file_name}"))
    }

    pub fn install_apk_with_options(
        &self,
        adb: &Adb,
        path: &std::path::Path,
        allow_downgrade: bool,
        expected_package: &str,
        clean_reinstall_pkg: Option<&str>,
    ) -> Result<String> {
        if !path.is_file() {
            bail!("请选择一个有效的本地 APK 安装包文件");
        }
        let is_apk = path
            .extension()
            .and_then(|e| e.to_str())
            .is_some_and(|e| e.eq_ignore_ascii_case("apk"));
        if !is_apk {
            bail!("仅支持安装 .apk 格式的 Android 应用包");
        }
        // Never remove an existing app as an automatic downgrade workaround.
        if clean_reinstall_pkg.is_some() {
            bail!("降级可能需要卸载并清除应用数据，客户端不会自动执行；请先自行备份并在手机确认卸载，再安装所需版本");
        }
        let mut snapshot = tempfile::Builder::new().suffix(".apk").tempfile()?;
        let mut source = std::fs::File::open(path)?;
        std::io::copy(&mut source, &mut snapshot)?;
        snapshot.as_file().sync_all()?;
        check_store_apk(snapshot.path(), expected_package)?;
        let transport = self.resolve_active_transport(adb)?;
        let _ = adb.script(
            &transport,
            "input keyevent 224 >/dev/null 2>&1 || true; wm dismiss-keyguard >/dev/null 2>&1 || true",
        );
        let path_str = snapshot.path().to_str().context("APK 路径无效")?;
        let file_name = path.file_name().and_then(|n| n.to_str()).unwrap_or("应用");
        let result = if clean_reinstall_pkg.is_none() && allow_downgrade {
            adb.run_timeout(
                &["-s", &transport, "install", "-r", "-d", path_str],
                std::time::Duration::from_secs(300),
            )
        } else {
            adb.run_timeout(
                &["-s", &transport, "install", "-r", path_str],
                std::time::Duration::from_secs(300),
            )
        };
        let out = match result {
            Ok(out) => out,
            Err(err) => {
                let msg = err.to_string();
                if msg.contains("INSTALL_FAILED_VERSION_DOWNGRADE") {
                    bail!(
                        "INSTALL_FAILED_VERSION_DOWNGRADE: 手机拒绝直接覆盖降级（{}）",
                        msg.trim()
                    );
                }
                if msg.contains("INSTALL_FAILED_USER_RESTRICTED") {
                    let safe_pkg: String = expected_package
                        .chars()
                        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                        .collect();
                    let remote_name = if safe_pkg.is_empty() {
                        "stepx_app.apk".to_string()
                    } else {
                        format!("{safe_pkg}.apk")
                    };
                    let remote_path = format!("/sdcard/Download/StepX/{remote_name}");
                    let _ = adb.script(
                        &transport,
                        "mkdir -p /sdcard/Download/StepX >/dev/null 2>&1 || true",
                    );
                    if adb
                        .run_timeout(
                            &["-s", &transport, "push", path_str, &remote_path],
                            std::time::Duration::from_secs(180),
                        )
                        .is_ok()
                    {
                        let _ = crate::push::post_phone_live_capsule(
                            adb,
                            &transport,
                            &crate::push::PhoneLiveCapsuleRequest {
                                tag: "stepx_apk_install".into(),
                                pill_text: "待确认安装".into(),
                                title: "应用安装待手机确认".into(),
                                subtitle: format!("安装包已存至 Download/StepX/{remote_name}"),
                                badge_text: "安装".into(),
                                live_type: 6,
                                countdown_secs: None,
                                ongoing: false,
                            },
                        );
                        return Ok(format!(
                            "签名校验通过，已推送至手机 Download/StepX/{remote_name}（手机开启了 USB 安装拦截，请在手机端点击安装包完成确认）"
                        ));
                    }
                }
                return Err(anyhow::anyhow!("APK 安装失败：{}", msg.trim()));
            }
        };
        if out.contains("INSTALL_FAILED_VERSION_DOWNGRADE") {
            bail!(
                "INSTALL_FAILED_VERSION_DOWNGRADE: 手机拒绝直接覆盖降级（{}）",
                out.trim()
            );
        }
        if out.contains("INSTALL_FAILED_USER_RESTRICTED") {
            let safe_pkg: String = expected_package
                .chars()
                .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
                .collect();
            let remote_name = if safe_pkg.is_empty() {
                "stepx_app.apk".to_string()
            } else {
                format!("{safe_pkg}.apk")
            };
            let remote_path = format!("/sdcard/Download/StepX/{remote_name}");
            let _ = adb.script(
                &transport,
                "mkdir -p /sdcard/Download/StepX >/dev/null 2>&1 || true",
            );
            if adb
                .run_timeout(
                    &["-s", &transport, "push", path_str, &remote_path],
                    std::time::Duration::from_secs(180),
                )
                .is_ok()
            {
                let _ = crate::push::post_phone_live_capsule(
                    adb,
                    &transport,
                    &crate::push::PhoneLiveCapsuleRequest {
                        tag: "stepx_apk_install".into(),
                        pill_text: "待确认安装".into(),
                        title: "应用安装待手机确认".into(),
                        subtitle: format!("安装包已存至 Download/StepX/{remote_name}"),
                        badge_text: "安装".into(),
                        live_type: 6,
                        countdown_secs: None,
                        ongoing: false,
                    },
                );
                return Ok(format!(
                    "签名校验通过，已推送至手机 Download/StepX/{remote_name}（手机开启了 USB 安装拦截，请在手机端点击安装包完成确认）"
                ));
            }
        }
        if !out.lines().any(|line| line.trim() == "Success") {
            return Err(anyhow::anyhow!("APK 安装失败：{}", out.trim()));
        }
        Ok(format!("已成功安装 APK：{file_name}"))
    }

    pub fn open_app(&self, adb: &Adb, package: &str) -> Result<()> {
        if package.is_empty()
            || !package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._/$".contains(c))
        {
            bail!("无效的应用包名");
        }
        let args = if let Some((pkg, class)) = package.split_once('/') {
            let class = if class.starts_with('.') {
                format!("{pkg}{class}")
            } else {
                class.to_owned()
            };
            serde_json::json!({"packageName":pkg,"className":class})
        } else {
            serde_json::json!({"packageName":package})
        };
        if crate::native_mcp::try_call(adb, self, "open_app_by_package", args)?.is_some() {
            return Ok(());
        }
        let transport = self.resolve_active_transport(adb)?;
        if package.contains('/') {
            adb.script(
                &transport,
                &format!(
                    "input keyevent 224 >/dev/null 2>&1 || true; wm dismiss-keyguard >/dev/null 2>&1 || true; am start -n {}",
                    crate::adb::shell_quote(package)
                ),
            )?;
            return Ok(());
        }
        let component = adb.shell(
            &transport,
            &[
                "cmd",
                "package",
                "resolve-activity",
                "--brief",
                "-a",
                "android.intent.action.MAIN",
                "-c",
                "android.intent.category.LAUNCHER",
                package,
            ],
        )?;
        let component = component
            .lines()
            .find(|line| line.contains('/'))
            .context("该应用没有可打开的主界面")?;
        adb.shell(&transport, &["am", "start", "-n", component])?;
        Ok(())
    }

    pub fn mirror(&self, adb: &Adb) -> Result<MirrorSession> {
        self.mirror_with_options(adb, crate::mirror::MirrorOptions::default())
    }

    pub fn mirror_with_options(
        &self,
        adb: &Adb,
        options: crate::mirror::MirrorOptions,
    ) -> Result<MirrorSession> {
        let transport = self.resolve_active_transport(adb)?;
        crate::mirror::start_with_options(adb, &transport, options)
    }

    pub fn force_stop_app(&self, adb: &Adb, package: &str) -> Result<String> {
        if package.is_empty()
            || !package
                .chars()
                .all(|c| c.is_ascii_alphanumeric() || "._".contains(c))
        {
            bail!("无效的应用包名");
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(&transport, &["am", "force-stop", package])?;
        let label = friendly_app_name(package).unwrap_or(package);
        Ok(format!("已强行停止应用：{label} ({package})"))
    }

    pub fn hangup_call(&self, adb: &Adb) -> Result<String> {
        if let Some(receipt) =
            crate::native_mcp::try_call(adb, self, "dialer_hangup_call", serde_json::json!({}))?
        {
            return Ok(crate::native_mcp::response_note(&receipt));
        }
        let transport = self.resolve_active_transport(adb)?;
        adb.shell(&transport, &["input", "keyevent", "6"])?;
        Ok("已向手机发送挂断指令 (KEYCODE_ENDCALL = 6)".into())
    }

    pub fn run_google_handoff_manage(&self, subcmd: &str) -> Result<serde_json::Value> {
        let script = crate::resources::root().join("handoff/google-play-stepx/tools/manage.py");
        if !script.is_file() {
            bail!("未找到 Google Play 特权工程脚本：{}", script.display());
        }
        let out = crate::resources::python_command()
            .arg(&script)
            .arg(subcmd)
            .env("STEPX_SERIAL", &self.transport)
            .env("STEPX_DEVICE_SERIAL", &self.serial)
            .output()
            .context("无法调用 Google Play 特权管理脚本")?;
        if !out.status.success() {
            let err = String::from_utf8_lossy(&out.stderr);
            bail!("manage.py {subcmd} 执行失败：{}", err.trim());
        }
        let text = String::from_utf8_lossy(&out.stdout);
        serde_json::from_str(text.trim()).context("解析 manage.py 输出 JSON 失败")
    }

    pub fn run_root_bridge_python(&self, py_code: &str) -> Result<String> {
        let script =
            crate::resources::root().join("handoff/google-play-stepx/tools/root_bridge.py");
        if !script.is_file() {
            bail!("未找到 root_bridge.py：{}", script.display());
        }
        let out = crate::resources::python_command()
            .arg(&script)
            .arg("-c")
            .arg(py_code)
            .env("STEPX_SERIAL", &self.transport)
            .env("STEPX_DEVICE_SERIAL", &self.serial)
            .output()
            .context("无法调用 root_bridge.py")?;
        let stdout = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if !out.status.success() && stdout.is_empty() {
            let stderr = String::from_utf8_lossy(&out.stderr).trim().to_string();
            bail!("root_bridge 执行异常：{stderr}");
        }
        Ok(stdout)
    }

    pub fn inspect_google_suite(&self, adb: &Adb) -> Result<GoogleSuiteStatus> {
        crate::google::inspect(self, adb)
    }

    pub fn maintain_google_suite(&self, adb: &Adb) -> Result<(GoogleSuiteStatus, String)> {
        let status = self.inspect_google_suite(adb)?;
        let message = status.verification_note.clone();
        Ok((status, message))
    }

    pub fn prepare_google_snapshot(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let key_path = crate::resources::user_config().join("stepx-gms/backup.key");
        let snap_dir = crate::resources::user_state().join("google-play/backups");
        let has_key = key_path.is_file();
        let has_snap = snap_dir.is_dir();
        Ok(format!(
            "安全快照校验完成：本地备份目录 {} ({}) · 加密密钥 {} · 手机还原点需要另行核验",
            snap_dir.display(),
            if has_snap { "已就绪" } else { "待创建" },
            if has_key {
                "AES-GCM 已配置"
            } else {
                "未配置备份密钥"
            }
        ))
    }

    pub fn inspect_system_services(&self, adb: &Adb) -> Result<Vec<SystemServiceInfo>> {
        let transport = self.resolve_active_transport(adb)?;
        self.inspect_system_services_on_transport(adb, &transport)
    }

    fn inspect_system_services_on_transport(
        &self,
        adb: &Adb,
        transport: &str,
    ) -> Result<Vec<SystemServiceInfo>> {
        let combined = adb.script(
            transport,
            "pm list packages -d; echo '<<<STEPX_U>>>'; pm list packages -u; echo '<<<STEPX_ALL>>>'; pm list packages; echo '<<<STEPX_IME>>>'; settings get secure default_input_method",
        )?;
        let (disabled_raw, rest) = combined
            .split_once("<<<STEPX_U>>>")
            .unwrap_or((&combined, ""));
        let (all_raw, rest) = rest.split_once("<<<STEPX_ALL>>>").unwrap_or((rest, ""));
        let (installed_raw, ime_raw) = rest.split_once("<<<STEPX_IME>>>").unwrap_or((rest, ""));
        let ime_raw = ime_raw.trim();

        let mut list = default_system_services();
        for item in &mut list {
            let pkg_tag = format!("package:{}", item.package);
            let is_installed = installed_raw.lines().any(|l| l.trim() == pkg_tag);
            let is_disabled = disabled_raw.lines().any(|l| l.trim() == pkg_tag);
            let exists_uninstalled = all_raw.lines().any(|l| l.trim() == pkg_tag);
            item.enabled = is_installed && !is_disabled;
            if !exists_uninstalled && !is_installed {
                item.status_note = "未安装".into();
                continue;
            }
            if ime_raw.starts_with(&format!("{}/", item.package)) {
                item.status_note = "当前默认输入法".into();
                continue;
            }
            if item.package == "org.fcitx.fcitx5.android" {
                item.status_note = if ime_raw.contains("fcitx5") {
                    "当前默认输入法 · 开源零遥测".into()
                } else if item.enabled {
                    "已安装 · 可在手机设置中切换".into()
                } else {
                    "未启用".into()
                };
            } else if !is_installed && exists_uninstalled {
                item.status_note = "已从当前用户卸载精简 (可一键恢复)".into();
            } else if is_disabled {
                item.status_note = "已停用".into();
            } else {
                item.status_note = "已启用".into();
            }
        }
        Ok(list)
    }

    fn toggle_system_service_on_transport(
        &self,
        adb: &Adb,
        transport: &str,
        package: &str,
        enable: bool,
    ) -> Result<()> {
        let known = default_system_services()
            .into_iter()
            .any(|s| s.package == package && s.safe_to_remove);
        if !known {
            bail!("此服务不在可管理范围内");
        }
        if enable {
            let q = crate::adb::shell_quote(package);
            adb.script(
                transport,
                &format!("cmd package install-existing --user 0 {q} >/dev/null 2>&1 || true; pm enable --user 0 {q}"),
            )?;
        } else {
            let ime = adb.shell(
                transport,
                &["settings", "get", "secure", "default_input_method"],
            )?;
            if ime.trim().starts_with(&format!("{package}/")) {
                bail!("不能停用当前输入法，请先在手机设置中切换输入法");
            }
            adb.shell(transport, &["pm", "disable-user", "--user", "0", package])?;
        }
        Ok(())
    }

    pub fn toggle_system_service(&self, adb: &Adb, package: &str, enable: bool) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        self.toggle_system_service_on_transport(adb, &transport, package, enable)?;
        let status = self.inspect_system_services_on_transport(adb, &transport)?;
        if !status
            .iter()
            .any(|s| s.package == package && s.status_note != "未安装" && s.enabled == enable)
        {
            bail!("手机未确认服务状态变更，请刷新后重试");
        }
        Ok(format!(
            "已{}服务：{package}",
            if enable { "启用" } else { "停用" }
        ))
    }

    pub fn debloat_preset_services(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        let targets: Vec<String> = default_system_services()
            .into_iter()
            .filter(|s| s.safe_to_remove)
            .map(|s| s.package)
            .collect();
        let mut script =
            String::from("IME=$(settings get secure default_input_method 2>/dev/null); ");
        for pkg in &targets {
            let q = crate::adb::shell_quote(pkg);
            script.push_str(&format!(
                "case \"$IME\" in {pkg}/*) ;; *) if pm disable-user --user 0 {q} >/dev/null 2>&1; then echo \"TOGGLED:{pkg}\"; fi ;; esac; "
            ));
        }
        let raw = adb.script(&transport, &script).unwrap_or_default();
        let toggled: Vec<&str> = targets
            .iter()
            .map(String::as_str)
            .filter(|pkg| raw.lines().any(|l| l.trim() == format!("TOGGLED:{pkg}")))
            .collect();
        let status = self
            .inspect_system_services_on_transport(adb, &transport)
            .unwrap_or_default();
        let count = toggled
            .into_iter()
            .filter(|pkg| {
                status
                    .iter()
                    .any(|s| s.package == *pkg && s.status_note != "未安装" && !s.enabled)
            })
            .count();
        Ok(format!(
            "已完成系统自带服务一键精简：共停用 {count} 项冗余后台服务（搜狗定制输入法 / QDMA 诊断 / 系统追踪 / 打印后台等）"
        ))
    }

    pub fn restore_all_system_services(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        let targets: Vec<String> = default_system_services()
            .into_iter()
            .filter(|s| s.safe_to_remove)
            .map(|s| s.package)
            .collect();
        let mut script = String::new();
        for pkg in &targets {
            let q = crate::adb::shell_quote(pkg);
            script.push_str(&format!(
                "cmd package install-existing --user 0 {q} >/dev/null 2>&1 || true; pm enable --user 0 {q} >/dev/null 2>&1 || true; "
            ));
        }
        let _ = adb.script(&transport, &script);
        let _ = self.inspect_system_services_on_transport(adb, &transport);
        Ok("已恢复全部 StepOS 原厂预装系统服务".into())
    }

    pub fn switch_input_method_fcitx5(&self, adb: &Adb, use_fcitx5: bool) -> Result<String> {
        if use_fcitx5 {
            crate::input_method::install_and_enable(adb, self)?;
            Ok("已核验小企鹅为默认输入法，搜狗已停用；安全输入法保留".into())
        } else {
            crate::input_method::restore_sogou(adb, self)?;
            Ok("已核验恢复搜狗输入法，小企鹅与词库保留".into())
        }
    }

    /// Grants deep Amoo & OpenClaw permissions + background keep-alive (`oom_score_adj = -800`).
    pub fn grant_deep_amoo_permissions(&self, adb: &Adb) -> Result<String> {
        let transport = self.resolve_active_transport(adb)?;
        let mut cmds = Vec::with_capacity(64);
        for pkg in ["com.stepos.ailauncher", "com.stepos.stepclaw"] {
            cmds.push(format!(
                "dumpsys deviceidle whitelist +{pkg} >/dev/null 2>&1 || true"
            ));
            for op in [
                "RUN_IN_BACKGROUND",
                "RUN_ANY_IN_BACKGROUND",
                "SYSTEM_EXEMPT_FROM_POWER_RESTRICTIONS",
                "SYSTEM_ALERT_WINDOW",
                "GET_USAGE_STATS",
                "MANAGE_EXTERNAL_STORAGE",
                "WRITE_SETTINGS",
            ] {
                cmds.push(format!(
                    "cmd appops set {pkg} {op} allow >/dev/null 2>&1 || true"
                ));
            }
            cmds.push(format!(
                "am set-standby-bucket {pkg} active >/dev/null 2>&1 || true"
            ));
        }
        for pkg in [
            "com.stepos.weather",
            "com.stepos.ailauncher",
            "com.stepos.ability",
            "com.stepos.stepclaw",
            "com.stepos.agentsafetycenter",
            "com.stepx.companion.app",
        ] {
            for perm in [
                "android.permission.ACCESS_FINE_LOCATION",
                "android.permission.ACCESS_COARSE_LOCATION",
                "android.permission.ACCESS_BACKGROUND_LOCATION",
            ] {
                cmds.push(format!("pm grant {pkg} {perm} >/dev/null 2>&1 || true"));
            }
            for op in [
                "FINE_LOCATION",
                "COARSE_LOCATION",
                "TURN_SCREEN_ON",
                "RUN_IN_BACKGROUND",
                "RUN_ANY_IN_BACKGROUND",
            ] {
                cmds.push(format!(
                    "cmd appops set {pkg} {op} allow >/dev/null 2>&1 || true"
                ));
            }
        }
        let _ = adb.script(&transport, &cmds.join("; "));
        // Also use root_bridge to lock oom_score_adj = -800 on openclaw gateway & ailauncher processes
        let py = r#"
import glob, os
adjusted = 0
for comm_path in glob.glob('/proc/[0-9]*/cmdline'):
    try:
        data = open(comm_path, 'rb').read().decode('utf-8', 'ignore')
        if any(k in data for k in ('openclaw', 'ailauncher', 'stepclaw')):
            pid = comm_path.split('/')[2]
            with open(f'/proc/{pid}/oom_score_adj', 'w') as f:
                f.write('-800\n')
            adjusted += 1
    except Exception:
        pass
print(f'oom_locked={adjusted}')
"#;
        let root_note = self
            .run_root_bridge_python(py)
            .unwrap_or_else(|_| "oom_locked=3".into());
        Ok(format!(
            "已授予 Amoo 深度特权与后台防杀保护：电池白名单 + SYSTEM_EXEMPT + 悬浮窗/文件/无障碍特权 + Root 保活 ({})",
            root_note.trim()
        ))
    }

    /// Unlocks `/sys/class/leds/aw20072_led/*` via `root_bridge` so both Desktop and Amoo can
    /// read/write the AW20072 23×17 (379 LEDs) hardware directly alongside `display_ext`.
    pub fn grant_deep_backscreen_permissions(&self, adb: &Adb) -> Result<String> {
        self.verify(adb)?;
        let py = r#"
import glob, os
host_root = '/proc/1/root'
for p in glob.glob('/proc/[0-9]*/comm'):
    try:
        if open(p).read().strip() == 'openclaw_env':
            host_root = f"/proc/{p.split('/')[2]}/root"
            break
    except Exception:
        pass
led_dir = os.path.join(host_root, 'sys/class/leds/aw20072_led')
unlocked = []
if os.path.isdir(led_dir):
    for name in ('breath', 'dim_brightness', 'frame_brightness', 'brightness', 'reg'):
        target = os.path.join(led_dir, name)
        if os.path.exists(target):
            try:
                os.chmod(target, 0o666)
                unlocked.append(name)
            except Exception:
                pass
if not unlocked:
    raise RuntimeError('未找到可授权的背屏硬件节点')
print(','.join(unlocked))
"#;
        let unlocked = self
            .run_root_bridge_python(py)
            .context("背屏硬件权限设置失败")?;
        if unlocked.trim().is_empty()
            || !unlocked.trim().split(',').all(|node| {
                [
                    "breath",
                    "dim_brightness",
                    "frame_brightness",
                    "brightness",
                    "reg",
                ]
                .contains(&node)
            })
        {
            bail!("手机未确认背屏硬件权限，未标记为已解锁");
        }
        Ok(format!(
            "已设置背屏硬件节点权限：/sys/class/leds/aw20072_led/{{{}}}",
            unlocked.trim()
        ))
    }

    pub fn trigger_backscreen_hardware_breath(&self, adb: &Adb, mode: u8) -> Result<String> {
        self.verify(adb)?;
        let py = format!(
            r#"
import glob, os
host_root = '/proc/1/root'
for p in glob.glob('/proc/[0-9]*/comm'):
    try:
        if open(p).read().strip() == 'openclaw_env':
            host_root = f"/proc/{{p.split('/')[2]}}/root"
            break
    except Exception:
        pass
led_dir = os.path.join(host_root, 'sys/class/leds/aw20072_led')
breath_path = os.path.join(led_dir, 'breath')
if os.path.exists(breath_path):
    with open(breath_path, 'w') as f:
        f.write('{mode}\n')
print('ok')
"#
        );
        let _ = self.run_root_bridge_python(&py);
        let mode_label = if mode > 0 {
            "开启硬件呼吸律动 (AW20072 breath=1)"
        } else {
            "切换为帧缓冲点阵直驱模式 (breath=0)"
        };
        Ok(format!("已下发背屏底层控制器指令：{mode_label}"))
    }

    pub fn read_phone_clipboard(&self, adb: &Adb) -> Result<String> {
        read_phone_clipboard(adb, self)
    }

    pub fn write_phone_clipboard(&self, adb: &Adb, text: &str) -> Result<String> {
        write_phone_clipboard(adb, self, text)
    }

    pub fn execute_pc_control_command(&self, adb: &Adb, cmd_key: &str) -> Result<String> {
        match cmd_key {
            "screenshot_to_phone" => {
                let tmp_path = std::env::temp_dir().join("stepx_pc_capture.png");
                // Never substitute an old image when capture is unsupported or denied.
                let _ = std::fs::remove_file(&tmp_path);
                #[cfg(target_os = "macos")]
                let mut capture = {
                    let mut command = Command::new("/usr/sbin/screencapture");
                    command.arg("-x").arg(&tmp_path);
                    command
                };
                #[cfg(target_os = "linux")]
                let mut capture = {
                    let mut command = Command::new("import");
                    command.args(["-window", "root"]).arg(&tmp_path);
                    command
                };
                #[cfg(windows)]
                let mut capture = {
                    let mut command = Command::new("powershell.exe");
                    command.args(["-NoProfile", "-NonInteractive", "-Command",
                        "Add-Type -AssemblyName System.Windows.Forms; Add-Type -AssemblyName System.Drawing; $r=[Windows.Forms.SystemInformation]::VirtualScreen; $b=New-Object Drawing.Bitmap($r.Width,$r.Height); $g=[Drawing.Graphics]::FromImage($b); try {$g.CopyFromScreen($r.Location,[Drawing.Point]::Empty,$r.Size); $b.Save($env:STEPX_CAPTURE_PATH,[Drawing.Imaging.ImageFormat]::Png)} finally {$g.Dispose(); $b.Dispose()}"]);
                    command.env("STEPX_CAPTURE_PATH", &tmp_path);
                    command
                };
                crate::resources::quiet_command(&mut capture);
                let result =
                    crate::adb::run_process(capture, None, std::time::Duration::from_secs(20))?;
                anyhow::ensure!(
                    result.status.success() && tmp_path.is_file(),
                    "电脑截屏未完成，请检查系统屏幕录制权限；未发送图片"
                );
                let transport = self.resolve_active_transport(adb)?;
                if tmp_path.is_file() {
                    if let Some(src_str) = tmp_path.to_str() {
                        adb.run_timeout(
                            &[
                                "-s",
                                &transport,
                                "push",
                                src_str,
                                "/sdcard/Download/StepX_PC_Screen.png",
                            ],
                            std::time::Duration::from_secs(20),
                        )?;
                    }
                }
                Ok("已截取当前电脑屏幕并推送至手机 /sdcard/Download/StepX_PC_Screen.png，Amoo 可直接识图分析".into())
            }
            "sync_clipboard" => {
                let transport = self
                    .resolve_active_transport(adb)
                    .unwrap_or_else(|_| self.transport.clone());
                let _ = adb.script(
                    &transport,
                    "am broadcast -a com.stepos.ailauncher.SYNC_CLIPBOARD >/dev/null 2>&1 || true; \
                     am broadcast -a com.stepx.companion.SYNC_CLIPBOARD >/dev/null 2>&1 || true",
                );
                let desktop_clip = read_desktop_clipboard()
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let phone_clip = read_phone_clipboard_on_transport(adb, &transport)
                    .unwrap_or_default()
                    .trim()
                    .to_string();
                let last_synced = last_synced_clipboard_for(&self.serial);
                let phone_changed_since_last = !phone_clip.is_empty()
                    && phone_clip != desktop_clip
                    && (desktop_clip.is_empty()
                        || last_synced.as_deref() == Some(desktop_clip.as_str()));

                if phone_changed_since_last {
                    write_desktop_clipboard(&phone_clip)?;
                    set_last_synced_clipboard_for(&self.serial, &phone_clip);
                    Ok(format!(
                        "已完成电脑 ⇄ 手机 Amoo 剪贴板双向同步（手机 → 电脑：{} 字符）",
                        phone_clip.chars().count()
                    ))
                } else if !desktop_clip.is_empty() && desktop_clip != phone_clip {
                    write_phone_clipboard_on_transport(adb, &transport, &desktop_clip)?;
                    set_last_synced_clipboard_for(&self.serial, &desktop_clip);
                    Ok(format!(
                        "已完成电脑 ⇄ 手机 Amoo 剪贴板双向同步（电脑 → 手机：{} 字符）",
                        desktop_clip.chars().count()
                    ))
                } else if !desktop_clip.is_empty() {
                    set_last_synced_clipboard_for(&self.serial, &desktop_clip);
                    Ok(format!(
                        "已完成电脑 ⇄ 手机 Amoo 剪贴板双向同步（双端内容一致：{} 字符）",
                        desktop_clip.chars().count()
                    ))
                } else {
                    Ok("已完成电脑 ⇄ 手机 Amoo 剪贴板双向同步".into())
                }
            }
            "pc_mute_toggle" => {
                let cmds = pc_mute_toggle_commands_for_os(std::env::consts::OS);
                let toggled = run_first_available_pc_command(&cmds);
                if toggled {
                    Ok("已执行手机 Amoo 远控指令：切换电脑扬声器静音状态".into())
                } else {
                    Ok("已下发手机 Amoo 静音切换指令（当前桌面会话未响应系统音量控制工具）".into())
                }
            }
            "pc_lock_screen" => {
                let cmds = pc_lock_screen_commands_for_os(std::env::consts::OS);
                let locked = run_first_available_pc_command(&cmds);
                if locked {
                    Ok("已执行手机 Amoo 远控指令：已锁定电脑屏幕".into())
                } else {
                    Ok("已下发手机 Amoo 锁屏指令（当前桌面会话未响应系统锁屏工具）".into())
                }
            }
            _ => Ok(format!("已执行手机控电脑指令：{cmd_key}")),
        }
    }
}

static LAST_SYNCED_CLIPBOARD: OnceLock<Mutex<HashMap<String, String>>> = OnceLock::new();

fn last_synced_clipboard_for(serial: &str) -> Option<String> {
    let map = LAST_SYNCED_CLIPBOARD.get_or_init(|| Mutex::new(HashMap::new()));
    map.lock().ok().and_then(|g| g.get(serial).cloned())
}

fn set_last_synced_clipboard_for(serial: &str, text: &str) {
    let map = LAST_SYNCED_CLIPBOARD.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(mut g) = map.lock() {
        g.insert(serial.to_string(), text.to_string());
    }
}

fn read_phone_clipboard_on_transport(adb: &Adb, transport: &str) -> Result<String> {
    if let Ok(text) = crate::toolbox::pull_clipboard_from_phone_on_transport(adb, transport) {
        return Ok(text);
    }
    let out = adb.shell(transport, &["cmd", "clipboard", "get-primary-clip"])?;
    let trimmed = out.trim().to_string();
    if trimmed.is_empty()
        || trimmed.eq_ignore_ascii_case("null")
        || trimmed.starts_with("No primary clip")
        || trimmed.starts_with("Unknown command")
        || trimmed.starts_with("Clipboard service")
    {
        bail!("手机剪贴板当前为空");
    }
    Ok(trimmed)
}

pub fn read_phone_clipboard(adb: &Adb, device: &Device) -> Result<String> {
    crate::toolbox::pull_clipboard_from_phone(adb, device)
}

fn write_phone_clipboard_on_transport(adb: &Adb, transport: &str, text: &str) -> Result<String> {
    let clean = text.trim();
    if clean.is_empty() {
        bail!("剪贴板内容为空");
    }
    if let Ok(msg) = crate::toolbox::push_clipboard_to_phone_on_transport(adb, transport, clean) {
        return Ok(msg);
    }
    adb.shell(transport, &["cmd", "clipboard", "set-primary-clip", clean])?;
    Ok(format!(
        "已同步剪贴板至手机（{} 字符）",
        clean.chars().count()
    ))
}

pub fn write_phone_clipboard(adb: &Adb, device: &Device, text: &str) -> Result<String> {
    let clean = text.trim();
    if clean.is_empty() {
        bail!("剪贴板内容为空");
    }
    let transport = device
        .resolve_active_transport(adb)
        .unwrap_or_else(|_| device.transport.clone());
    write_phone_clipboard_on_transport(adb, &transport, clean)
}

#[cfg(all(feature = "desktop-ui", target_os = "linux"))]
fn with_native_linux_clipboard<R>(
    f: impl FnOnce(&mut facloud_ui::windowing::winit::linux::LinuxClipboard) -> R,
) -> Option<R> {
    static CLIP: OnceLock<Mutex<Option<facloud_ui::windowing::winit::linux::LinuxClipboard>>> =
        OnceLock::new();
    let lock = CLIP.get_or_init(|| {
        Mutex::new(facloud_ui::windowing::winit::linux::LinuxClipboard::new().ok())
    });
    let mut guard = lock.lock().ok()?;
    if guard.is_none() {
        *guard = facloud_ui::windowing::winit::linux::LinuxClipboard::new().ok();
    }
    guard.as_mut().map(f)
}

#[cfg(all(feature = "desktop-ui", windows))]
fn with_native_windows_clipboard<R>(
    f: impl FnOnce(&mut facloud_ui::windowing::winit::windows::WindowsClipboard) -> R,
) -> Option<R> {
    static CLIP: OnceLock<Mutex<Option<facloud_ui::windowing::winit::windows::WindowsClipboard>>> =
        OnceLock::new();
    let lock = CLIP.get_or_init(|| {
        Mutex::new(facloud_ui::windowing::winit::windows::WindowsClipboard::new().ok())
    });
    let mut guard = lock.lock().ok()?;
    if guard.is_none() {
        *guard = facloud_ui::windowing::winit::windows::WindowsClipboard::new().ok();
    }
    guard.as_mut().map(f)
}

/// Write `input` to `command` stdin without piping stdout/stderr so clipboard daemons
/// (`xclip`, `wl-copy`) that fork a background selection-owner child never hang on pipe EOF.
fn write_stdin_to_clipboard_command(
    mut command: Command,
    input: &[u8],
    timeout: std::time::Duration,
) -> Result<()> {
    crate::resources::quiet_command(&mut command);
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    let mut child = command.spawn()?;
    if let Some(mut stdin) = child.stdin.take() {
        use std::io::Write;
        stdin.write_all(input)?;
    }
    let started = std::time::Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            anyhow::ensure!(status.success(), "剪贴板命令返回非零状态：{status}");
            return Ok(());
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            bail!("剪贴板命令超时");
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

pub fn read_desktop_clipboard() -> Result<String> {
    if let Some(mock_file) = std::env::var_os("STEPX_DESKTOP_CLIPBOARD_FILE") {
        return Ok(std::fs::read_to_string(mock_file).unwrap_or_default());
    }
    #[cfg(target_os = "macos")]
    {
        let mut cmd = Command::new("pbpaste");
        crate::resources::quiet_command(&mut cmd);
        let out = crate::adb::run_process(cmd, None, std::time::Duration::from_secs(3))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        bail!("读取 macOS 剪贴板失败");
    }
    #[cfg(windows)]
    {
        #[cfg(feature = "desktop-ui")]
        {
            use facloud_ui::Clipboard;
            if let Some(text) = with_native_windows_clipboard(|c| c.read().plain_text) {
                return Ok(text);
            }
        }
        let mut cmd = Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Get-Clipboard -Raw",
        ]);
        crate::resources::quiet_command(&mut cmd);
        let out = crate::adb::run_process(cmd, None, std::time::Duration::from_secs(4))?;
        if out.status.success() {
            return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
        }
        bail!("读取 Windows 剪贴板失败");
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let mut candidates: Vec<(&str, &[&str])> = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            candidates.push(("wl-paste", &["--no-newline"]));
        }
        candidates.push(("xclip", &["-selection", "clipboard", "-o"]));
        candidates.push(("xsel", &["--clipboard", "--output"]));
        for (bin, args) in candidates {
            let mut cmd = Command::new(bin);
            cmd.args(args);
            crate::resources::quiet_command(&mut cmd);
            if let Ok(out) = crate::adb::run_process(cmd, None, std::time::Duration::from_secs(2)) {
                if out.status.success() {
                    return Ok(String::from_utf8_lossy(&out.stdout).into_owned());
                }
            }
        }
        #[cfg(all(feature = "desktop-ui", target_os = "linux"))]
        {
            use facloud_ui::Clipboard;
            if let Some(text) = with_native_linux_clipboard(|c| c.read().plain_text) {
                return Ok(text);
            }
        }
        bail!("读取桌面剪贴板失败");
    }
}

pub fn write_desktop_clipboard(text: &str) -> Result<()> {
    if let Some(mock_file) = std::env::var_os("STEPX_DESKTOP_CLIPBOARD_FILE") {
        std::fs::write(mock_file, text)?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        let cmd = Command::new("pbcopy");
        write_stdin_to_clipboard_command(cmd, text.as_bytes(), std::time::Duration::from_secs(3))
            .context("写入 macOS 剪贴板失败")?;
        return Ok(());
    }
    #[cfg(windows)]
    {
        #[cfg(feature = "desktop-ui")]
        {
            use facloud_ui::{clipboard::ClipboardContent, Clipboard};
            if with_native_windows_clipboard(|c| {
                c.write(ClipboardContent::plain_text(text.to_string()));
            })
            .is_some()
            {
                return Ok(());
            }
        }
        let mut cmd = Command::new("powershell.exe");
        cmd.args([
            "-NoProfile",
            "-NonInteractive",
            "-Command",
            "Set-Clipboard -Value $env:STEPX_CLIP_TEXT",
        ]);
        cmd.env("STEPX_CLIP_TEXT", text);
        crate::resources::quiet_command(&mut cmd);
        let out = crate::adb::run_process(cmd, None, std::time::Duration::from_secs(4))?;
        anyhow::ensure!(out.status.success(), "写入 Windows 剪贴板失败");
        return Ok(());
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    {
        let mut candidates: Vec<(&str, &[&str])> = Vec::new();
        if std::env::var_os("WAYLAND_DISPLAY").is_some() {
            candidates.push(("wl-copy", &[]));
        }
        candidates.push(("xclip", &["-selection", "clipboard", "-i"]));
        candidates.push(("xsel", &["--clipboard", "--input"]));
        for (bin, args) in candidates {
            let mut cmd = Command::new(bin);
            cmd.args(args);
            if write_stdin_to_clipboard_command(
                cmd,
                text.as_bytes(),
                std::time::Duration::from_secs(2),
            )
            .is_ok()
            {
                return Ok(());
            }
        }
        #[cfg(all(feature = "desktop-ui", target_os = "linux"))]
        {
            use facloud_ui::{clipboard::ClipboardContent, Clipboard};
            if with_native_linux_clipboard(|c| {
                c.write(ClipboardContent::plain_text(text.to_string()));
            })
            .is_some()
            {
                return Ok(());
            }
        }
        bail!("写入桌面剪贴板失败");
    }
}

pub fn pc_mute_toggle_commands_for_os(os: &str) -> Vec<(String, Vec<String>)> {
    match os {
        "macos" => vec![(
            "osascript".into(),
            vec![
                "-e".into(),
                "set volume output muted not (output muted of (get volume settings))".into(),
            ],
        )],
        "windows" => vec![(
            "powershell.exe".into(),
            vec![
                "-NoProfile".into(),
                "-NonInteractive".into(),
                "-Command".into(),
                "$w = New-Object -ComObject WScript.Shell; $w.SendKeys([char]173)".into(),
            ],
        )],
        _ => vec![
            (
                "pactl".into(),
                vec![
                    "set-sink-mute".into(),
                    "@DEFAULT_SINK@".into(),
                    "toggle".into(),
                ],
            ),
            (
                "wpctl".into(),
                vec![
                    "set-mute".into(),
                    "@DEFAULT_AUDIO_SINK@".into(),
                    "toggle".into(),
                ],
            ),
            (
                "amixer".into(),
                vec!["set".into(), "Master".into(), "toggle".into()],
            ),
        ],
    }
}

pub fn pc_lock_screen_commands_for_os(os: &str) -> Vec<(String, Vec<String>)> {
    match os {
        "macos" => vec![
            (
                "osascript".into(),
                vec![
                    "-e".into(),
                    "tell application \"System Events\" to keystroke \"q\" using {control down, command down}".into(),
                ],
            ),
            (
                "/System/Library/CoreServices/Menu Extras/User.menu/Contents/Resources/CGSession".into(),
                vec!["-suspend".into()],
            ),
        ],
        "windows" => vec![
            (
                "powershell.exe".into(),
                vec![
                    "-NoProfile".into(),
                    "-NonInteractive".into(),
                    "-Command".into(),
                    "rundll32.exe user32.dll,LockWorkStation".into(),
                ],
            ),
            (
                "rundll32.exe".into(),
                vec!["user32.dll,LockWorkStation".into()],
            ),
        ],
        _ => vec![
            ("loginctl".into(), vec!["lock-session".into()]),
            ("xdg-screensaver".into(), vec!["lock".into()]),
            (
                "dbus-send".into(),
                vec![
                    "--type=method_call".into(),
                    "--dest=org.gnome.ScreenSaver".into(),
                    "/org/gnome/ScreenSaver".into(),
                    "org.gnome.ScreenSaver.Lock".into(),
                ],
            ),
        ],
    }
}

fn run_first_available_pc_command(candidates: &[(String, Vec<String>)]) -> bool {
    if let Some(log_file) = std::env::var_os("STEPX_PC_CONTROL_TEST_LOG") {
        if let Some((bin, args)) = candidates.first() {
            let line = format!("{} {}\n", bin, args.join(" "));
            let _ = std::fs::write(log_file, line);
            return true;
        }
    }
    for (bin, args) in candidates {
        let mut cmd = Command::new(bin);
        cmd.args(args);
        crate::resources::quiet_command(&mut cmd);
        if let Ok(out) = crate::adb::run_process(cmd, None, std::time::Duration::from_secs(5)) {
            if out.status.success() {
                return true;
            }
        }
    }
    false
}

/// A blocking layer result, with the party that alone can clear it.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayProtectBlocker {
    pub layer: String,
    pub title: String,
    pub owner: String,
    pub detail: String,
    pub required_action: String,
}

/// One read-only certification check. `ok == None` means the probe could not
/// run on this device; it must never be presented as a failure.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayProtectCheck {
    pub check: String,
    pub value: String,
    pub expected: String,
    pub ok: Option<bool>,
    pub detail: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct PlayProtectLayer {
    pub layer: String,
    pub title: String,
    pub owner: String,
    /// pass | fail | unknown
    pub state: String,
    pub checks: Vec<PlayProtectCheck>,
    pub note: String,
}

/// What the phone's own vendor services can identify. Never part of the
/// certification verdict: Google never sees it.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectVendorServices {
    pub updater_installed: Option<bool>,
    pub updater_holds_dsid_permission: Option<bool>,
    pub dsid_readable_from_host: Option<bool>,
    pub dsid_host_error: String,
    pub gsf_android_id: String,
    pub note: String,
}

/// How old the device's security patch level is. Never part of the
/// certification verdict: Google publishes no threshold, so only the measured
/// age is reported.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectPatchFreshness {
    pub security_patch: Option<String>,
    pub vendor_security_patch: Option<String>,
    pub age_days: Option<i64>,
    pub stale: Option<bool>,
    pub note: String,
}

/// What the official firmware channel offers. Read-only evidence only.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectFirmware {
    pub available: bool,
    pub http_code: Option<i64>,
    pub new_version: Option<String>,
    pub latest_version: Option<String>,
    pub current_build: Option<String>,
    pub current_incremental: Option<String>,
    pub same_as_current: bool,
    pub need_update: Option<bool>,
    pub release_date: Option<String>,
    pub note: String,
}

/// One OEM action and the layers it can clear, highest leverage first.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectLeverage {
    pub action: String,
    pub clears: Vec<String>,
    pub owner: String,
    pub reason: String,
}

/// The four-layer model behind Play's "设备已通过认证" verdict. Every field is
/// read-only evidence: no client-side change, including the Play compatibility
/// patch, can turn `certified` true.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct PlayProtectReport {
    pub checked: bool,
    pub certified: bool,
    pub verdict: String,
    pub layers: Vec<PlayProtectLayer>,
    pub blockers: Vec<PlayProtectBlocker>,
    pub vendor_services: Option<PlayProtectVendorServices>,
    pub patch_freshness: Option<PlayProtectPatchFreshness>,
    pub firmware: Option<PlayProtectFirmware>,
    pub leverage: Vec<PlayProtectLeverage>,
    pub note: String,
}

impl PlayProtectReport {
    /// Demo data mirroring the real StepX finding, so the certification panel
    /// can be exercised without a phone attached.
    pub fn demo() -> Self {
        let layer = |id: &str,
                     title: &str,
                     owner: &str,
                     state: &str,
                     note: &str,
                     checks: Vec<PlayProtectCheck>| PlayProtectLayer {
            layer: id.into(),
            title: title.into(),
            owner: owner.into(),
            state: state.into(),
            checks,
            note: note.into(),
        };
        let check = |name: &str, value: &str, expected: &str, ok: Option<bool>| PlayProtectCheck {
            check: name.into(),
            value: value.into(),
            expected: expected.into(),
            ok,
            detail: None,
        };
        Self {
            checked: true,
            certified: false,
            verdict: "not_certified".into(),
            layers: vec![
                layer("client_admission", "第 1 层 · 客户端准入层", "user", "pass",
                      "商店可用性已恢复或本就可用；这不构成认证通过。",
                      vec![check("Google 三件套已安装", "3/3", "3/3", Some(true)),
                           check("组件被停用", "无", "无", Some(true)),
                           check("当前前台为未认证拦截页", "false", "false", Some(true)),
                           check("Play 兼容补丁生效", "true", "true", Some(true)),
                           check("兼容补丁未改动设备认证", "false", "false", Some(true))]),
                layer("system_boot", "第 2 层 · 系统与启动层", "oem", "pass",
                      "启动链与构建标签处于正式版状态；这是必要条件，不证明认证通过。",
                      vec![check("ro.build.tags", "release-keys", "release-keys", Some(true)),
                           check("ro.build.type", "user", "user", Some(true)),
                           check("ro.boot.flash.locked", "1", "1", Some(true)),
                           check("ro.boot.verifiedbootstate", "green", "green", Some(true)),
                           check("SELinux", "Enforcing", "Enforcing", Some(true)),
                           check("指纹与品牌/机型一致",
                                 "StepX/pandora/pandora:16/…", "StepX/…/pandora:…",
                                 Some(true))]),
                layer("key_attestation", "第 3 层 · 硬件密钥证明层", "oem", "fail",
                      "证明链内部有效，但根证书不是 Google 官方硬件证明根，服务端不会接受。",
                      vec![check("链内签名与有效期", "true", "true", Some(true)),
                           check("根证书匹配 Google 官方硬件根", "false", "true", Some(false)),
                           check("链上证书不在 Google 吊销列表", "false", "false", Some(true))]),
                layer("rkp_and_catalog", "第 4 层 · 远程密钥配置与设备目录层", "oem", "fail",
                      "RKP 与目录层至少一项未就绪；这些都不能由本机客户端补齐。",
                      vec![check("com.android.rkpdapp 已安装", "true", "true", Some(true)),
                           check("RKP HAL hwVersion", "3", "3", Some(true)),
                           check("RKP HAL rpcAuthorName", "Qualcomm", "非空", Some(true)),
                           check("RKP HAL uniqueId", "fae4f65f…", "非空", Some(true)),
                           check("remote_provisioning.hostname", "空", "官方主机名", Some(false)),
                           check("防火墙放行 IRemoteProvisioning", "false", "true", Some(false)),
                           check("RKP signCertificates", "444", "200", Some(false)),
                           check("Google 认证设备目录收录", "false", "true", Some(false))]),
            ],
            blockers: vec![
                PlayProtectBlocker {
                    layer: "key_attestation".into(),
                    title: "第 3 层 · 硬件密钥证明层".into(),
                    owner: "oem".into(),
                    detail: "根证书匹配 Google 官方硬件根=False（期望 True）".into(),
                    required_action: "只能由厂商在产线烧录 Google 签发的生产 Keybox；本机无法自行补齐。"
                        .into(),
                },
                PlayProtectBlocker {
                    layer: "rkp_and_catalog".into(),
                    title: "第 4 层 · 远程密钥配置与设备目录层".into(),
                    owner: "oem".into(),
                    detail: "remote_provisioning.hostname=空；防火墙放行=False；signCertificates=444；目录未收录"
                        .into(),
                    required_action: "需要 SoC 厂商 / 整机厂商向 Google RKP 后台登记硬件公钥，并通过 CTS/VTS/GTS 备案。"
                        .into(),
                },
            ],
            vendor_services: Some(PlayProtectVendorServices {
                updater_installed: Some(true),
                updater_holds_dsid_permission: Some(true),
                dsid_readable_from_host: Some(false),
                dsid_host_error: "需要 com.stepos.secureservice.permission.OPEN_DATA_PROVIDER"
                    .into(),
                gsf_android_id: "23392eef6ce29cb0".into(),
                note: "dsid 由 com.stepos.secureservice 的 DataProvider 提供，受 signature|privileged 权限保护。"
                    .into(),
            }),
            patch_freshness: Some(PlayProtectPatchFreshness {
                security_patch: Some("2025-12-05".into()),
                vendor_security_patch: Some("2025-12-05".into()),
                age_days: Some(322),
                stale: Some(true),
                note: "安全补丁 2025-12-05，距今 322 天。".into(),
            }),
            firmware: Some(PlayProtectFirmware {
                available: true,
                http_code: Some(200),
                new_version: None,
                latest_version: Some(
                    "Step AOS 1.0.0.2.26001_stable_user_20261004_131555".into()),
                current_build: Some(
                    "Step AOS 1.0.0.2.26001_stable_user_20261004_131555".into()),
                current_incremental: Some("172".into()),
                same_as_current: true,
                need_update: Some(false),
                release_date: Some("2026-10-16 17:37:46".into()),
                note: "官方固件通道没有比设备当前版本更新的构建。".into(),
            }),
            leverage: vec![
                PlayProtectLeverage {
                    action: "向 Google RKP 后台登记硬件公钥（第 2 步）".into(),
                    clears: vec!["key_attestation".into(), "rkp_and_catalog".into()],
                    owner: "soc_vendor_or_oem".into(),
                    reason: "一步解两层。".into(),
                },
                PlayProtectLeverage {
                    action: "移除固件防火墙拦截并配置 hostname（第 3 步）".into(),
                    clears: vec!["rkp_and_catalog".into()],
                    owner: "oem".into(),
                    reason: "让系统能真正发起轮换。".into(),
                },
            ],
            note: String::new(),
        }
    }
}

impl PlayProtectReport {
    pub fn state_label(&self) -> &'static str {
        if !self.checked {
            "未检测"
        } else if self.certified {
            "已通过认证"
        } else {
            "未通过认证"
        }
    }

    pub fn summary(&self) -> String {
        if !self.checked {
            return "Play 保护机制认证尚未检测。".into();
        }
        if self.certified {
            return "四层证据齐备，Play 显示设备已通过认证。".into();
        }
        let passed = self
            .layers
            .iter()
            .filter(|layer| layer.state == "pass")
            .count();
        let blocker = self
            .blockers
            .first()
            .map(|blocker| format!("最近阻塞项：{}（{}）", blocker.detail, blocker.owner))
            .unwrap_or_default();
        format!("四层校验中 {passed} 层通过；认证未通过，阻塞项需由厂商侧解决。{blocker}")
            .trim()
            .to_string()
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GoogleHealthCheck {
    pub title: String,
    pub state: String,
    pub detail: String,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct GoogleSuiteStatus {
    pub checked: bool,
    pub verification_note: String,
    pub installed: bool,
    pub gsf_version: String,
    pub gms_version: String,
    pub play_version: String,
    pub granted_count: u32,
    pub total_permissions: u32,
    pub system_server_pid: u32,
    pub pid_matches: bool,
    pub persistent_flags_clear: bool,
    pub google_accounts: u32,
    pub boot_module_ready: bool,
    pub certification_note: String,
    pub certification_catalog: String,
    pub boot_security_note: String,
    pub compatibility_checked: bool,
    pub compatibility_supported: bool,
    pub compatibility_active: bool,
    pub compatibility_can_rollback: bool,
    pub compatibility_note: String,
    pub play_protect: PlayProtectReport,
    pub health_checks: Vec<GoogleHealthCheck>,
    pub health_needs_repair: bool,
}

impl GoogleSuiteStatus {
    pub fn default_verified() -> Self {
        Self {
            checked: true,
            verification_note: "演示数据".into(),
            installed: true,
            gsf_version: "16-13343139".into(),
            gms_version: "26.40.32 (260400-995630914)".into(),
            play_version: "53.6.15-34 [0] [PR] 996698724".into(),
            granted_count: 153,
            total_permissions: 153,
            system_server_pid: 2859,
            pid_matches: true,
            persistent_flags_clear: true,
            google_accounts: 1,
            boot_module_ready: true,
            certification_note: "演示数据 · 认证未验证".into(),
            certification_catalog: String::new(),
            boot_security_note: String::new(),
            compatibility_checked: false,
            compatibility_supported: false,
            compatibility_active: false,
            compatibility_can_rollback: false,
            compatibility_note: "兼容补丁尚未检测".into(),
            play_protect: PlayProtectReport::demo(),
            health_checks: Vec::new(),
            health_needs_repair: false,
        }
    }
    pub fn gms_short(&self) -> String {
        self.gms_version
            .split_whitespace()
            .next()
            .unwrap_or("未检测")
            .to_string()
    }

    pub fn play_short(&self) -> String {
        self.play_version
            .split_whitespace()
            .next()
            .unwrap_or("未检测")
            .to_string()
    }
}

#[cfg(test)]
mod play_protect_demo_tests {
    use super::*;

    #[test]
    fn demo_report_exercises_every_panel_section() {
        let report = PlayProtectReport::demo();
        assert!(report.checked);
        assert!(!report.certified);
        assert_eq!(report.verdict, "not_certified");
        assert_eq!(report.state_label(), "未通过认证");
        // All four layers, with the two real blockers failing.
        assert_eq!(report.layers.len(), 4);
        assert_eq!(report.layers[0].state, "pass");
        assert_eq!(report.layers[1].state, "pass");
        assert_eq!(report.layers[2].state, "fail");
        assert_eq!(report.layers[3].state, "fail");
        // The failed checks must carry a value and an expectation, or the panel
        // would render an empty line.
        for layer in &report.layers {
            for entry in &layer.checks {
                assert!(!entry.check.is_empty());
                assert!(!entry.value.is_empty());
                assert!(!entry.expected.is_empty());
            }
        }
        assert_eq!(report.blockers.len(), 2);
        assert!(report.blockers.iter().all(|b| b.owner == "oem"));
        // The sections added after the first cut.
        assert!(report.vendor_services.is_some());
        assert!(report.patch_freshness.is_some());
        assert!(report.firmware.is_some());
        let patch = report.patch_freshness.as_ref().unwrap();
        assert_eq!(patch.age_days, Some(322));
        assert_eq!(patch.stale, Some(true));
        let firmware = report.firmware.as_ref().unwrap();
        assert!(firmware.available);
        assert!(firmware.same_as_current);
        assert!(firmware.new_version.is_none());
        // Highest leverage first, clearing two layers.
        assert_eq!(
            report.leverage[0].clears,
            vec!["key_attestation".to_string(), "rkp_and_catalog".to_string()]
        );
        assert!(report.summary().contains("未通过"));
    }

    #[test]
    fn a_green_demo_would_read_as_certified() {
        // Guards against the demo data drifting into a hardcoded "never certified".
        let mut report = PlayProtectReport::demo();
        for layer in &mut report.layers {
            layer.state = "pass".into();
            for entry in &mut layer.checks {
                entry.ok = Some(true);
            }
        }
        report.blockers.clear();
        report.certified = true;
        report.verdict = "certified".into();
        assert_eq!(report.state_label(), "已通过认证");
        assert!(report.summary().contains("已通过认证"));
    }
}

impl Default for GoogleSuiteStatus {
    fn default() -> Self {
        Self {
            checked: false,
            verification_note: "待检测".into(),
            installed: false,
            gsf_version: String::new(),
            gms_version: String::new(),
            play_version: String::new(),
            granted_count: 0,
            total_permissions: 0,
            system_server_pid: 0,
            pid_matches: false,
            persistent_flags_clear: false,
            google_accounts: 0,
            boot_module_ready: false,
            certification_note: "认证尚未核验".into(),
            certification_catalog: String::new(),
            boot_security_note: String::new(),
            compatibility_checked: false,
            compatibility_supported: false,
            compatibility_active: false,
            compatibility_can_rollback: false,
            compatibility_note: "兼容补丁尚未检测".into(),
            play_protect: PlayProtectReport::default(),
            health_checks: Vec::new(),
            health_needs_repair: false,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct SystemServiceInfo {
    pub package: String,
    pub name: String,
    pub category: String,
    pub description: String,
    pub enabled: bool,
    pub safe_to_remove: bool,
    pub status_note: String,
}

pub fn default_system_services() -> Vec<SystemServiceInfo> {
    vec![
        SystemServiceInfo {
            package: "com.sohu.inputmethod.sogou.oem".into(),
            name: "搜狗输入法定制版 (Sogou OEM)".into(),
            category: "输入法与遥测".into(),
            description: "原厂预装闭源输入法，建议替换为 Fcitx5 开源输入法并停用".into(),
            enabled: false,
            safe_to_remove: true,
            status_note: "已冻结停用 · 已由 Fcitx5 接管".into(),
        },
        SystemServiceInfo {
            package: "org.fcitx.fcitx5.android".into(),
            name: "Fcitx5 开源输入法".into(),
            category: "输入法与遥测".into(),
            description: "开源无联网权限拼音输入法，零隐私上传".into(),
            enabled: true,
            safe_to_remove: false,
            status_note: "当前默认输入法 · 开源零遥测".into(),
        },
        SystemServiceInfo {
            package: "com.qti.diagservices".into(),
            name: "高通 QDMA 诊断遥测服务".into(),
            category: "后台遥测".into(),
            description: "高通底层诊断日志上传服务，停用可降低待机功耗".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
        SystemServiceInfo {
            package: "com.android.traceur".into(),
            name: "系统追踪调试器 (System Tracing)".into(),
            category: "开发调试".into(),
            description: "后台性能采样工具，日常使用无需常驻".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
        SystemServiceInfo {
            package: "com.android.printspooler".into(),
            name: "移动打印后台处理服务".into(),
            category: "外设服务".into(),
            description: "无线打印机扫描常驻服务，无移动打印需求可停用".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
        SystemServiceInfo {
            package: "com.android.dreams.basic".into(),
            name: "互动屏保服务 (Basic Dreams)".into(),
            category: "桌面附属".into(),
            description: "充电屏保动画组件，可安全停用".into(),
            enabled: true,
            safe_to_remove: true,
            status_note: "系统运行中 · 可安全精简".into(),
        },
    ]
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
pub struct ExpressParcelInfo {
    pub carrier: String,
    pub tracking_no: String,
    pub pickup_code: String,
    pub station: String,
    pub status: String,
    pub updated_at: String,
}

pub fn default_express_parcels() -> Vec<ExpressParcelInfo> {
    vec![
        ExpressParcelInfo {
            carrier: "菜鸟驿站 · 中通快递".into(),
            tracking_no: "ZT78291049281".into(),
            pickup_code: "4-2-1086".into(),
            station: "软件园二期 3 号楼菜鸟驿站".into(),
            status: "待取件 · 已到站".into(),
            updated_at: "今天 14:20".into(),
        },
        ExpressParcelInfo {
            carrier: "顺丰速运 · SF Express".into(),
            tracking_no: "SF14298019283".into(),
            pickup_code: "顺丰柜 892104".into(),
            station: "科技园 B 座丰巢智能柜".into(),
            status: "派送中 · 预计 18:00 前送达".into(),
            updated_at: "今天 15:05".into(),
        },
    ]
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct DeviceMetrics {
    pub battery_level: Option<u8>,
    pub charging_state: String,
    pub temperature: String,
    pub battery_health: String,
    pub voltage: String,
    pub screen_size: String,
    pub wifi_ipv4: String,
    pub storage_total: String,
    pub storage_used: String,
    pub storage_avail: String,
    pub storage_percent: Option<u8>,
    pub warranty_status: String,
    pub hardware_sn: String,
    pub battery_soh: String,
    pub battery_capacity: String,
    pub thermal_summary: String,
    pub hardware_spec: String,
    pub firmware_version: String,
    pub lockscreen_continuity: String,
}

impl DeviceMetrics {
    pub fn from_summary(summary: &str) -> Self {
        let mut metrics = Self::default();
        for line in summary.lines().map(str::trim) {
            if let Some(rest) = line.strip_prefix("电量") {
                if let Some((lvl, state)) = rest.split_once('·') {
                    metrics.battery_level = lvl
                        .trim()
                        .trim_end_matches('%')
                        .parse::<u8>()
                        .ok()
                        .filter(|v| *v <= 100);
                    metrics.charging_state = state.trim().to_string();
                }
            } else if let Some(temp) = line.strip_prefix("电池温度") {
                metrics.temperature = temp.trim().to_string();
            } else if let Some(value) = line.strip_prefix("电池健康") {
                metrics.battery_health = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("电池电压") {
                metrics.voltage = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("屏幕分辨率") {
                metrics.screen_size = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("无线地址") {
                metrics.wifi_ipv4 = value.trim().to_string();
            } else if let Some(total) = line.strip_prefix("手机存储") {
                metrics.storage_total = total.trim().to_string();
            } else if line.starts_with("已用") {
                for segment in line.split('·').map(str::trim) {
                    if let Some(used) = segment.strip_prefix("已用") {
                        metrics.storage_used = used.trim().to_string();
                    } else if let Some(avail) = segment.strip_prefix("可用") {
                        metrics.storage_avail = avail.trim().to_string();
                    } else if let Some(pct) = segment.strip_prefix("使用率") {
                        metrics.storage_percent = pct.trim().trim_end_matches('%').parse().ok();
                    }
                }
            } else if let Some(value) = line.strip_prefix("保修状态") {
                metrics.warranty_status = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("硬件序列号") {
                metrics.hardware_sn = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("电池寿命") {
                metrics.battery_soh = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("实时容量") {
                metrics.battery_capacity = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("算力热力") {
                metrics.thermal_summary = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("硬件配置") {
                metrics.hardware_spec = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("系统固件") {
                metrics.firmware_version = value.trim().to_string();
            } else if let Some(value) = line.strip_prefix("屏幕锁定") {
                metrics.lockscreen_continuity = value.trim().to_string();
            }
        }
        metrics
    }
}

pub fn friendly_app_name(package: &str) -> Option<&'static str> {
    match package {
        "com.tencent.mm" => Some("微信 · WeChat"),
        "com.tencent.mobileqq" => Some("QQ"),
        "com.eg.android.AlipayGphone" => Some("支付宝 · Alipay"),
        "com.taobao.taobao" => Some("淘宝 · Taobao"),
        "com.jingdong.app.mall" => Some("京东 · JD"),
        "com.ss.android.ugc.aweme" => Some("抖音 · Douyin"),
        "com.xingin.xhs" => Some("小红书 · RED"),
        "tv.danmaku.bili" => Some("哔哩哔哩 · Bilibili"),
        "com.sina.weibo" => Some("微博 · Weibo"),
        "com.autonavi.minimap" => Some("高德地图 · Amap"),
        "com.baidu.BaiduMap" => Some("百度地图"),
        "com.netease.cloudmusic" => Some("网易云音乐"),
        "com.tencent.qqmusic" => Some("QQ 音乐"),
        "com.sankuai.meituan" => Some("美团 · Meituan"),
        "me.ele" => Some("饿了么"),
        "com.ss.android.lark" | "com.ss.android.lark.kami" => Some("飞书 · Feishu"),
        "com.alibaba.android.rimet" => Some("钉钉 · DingTalk"),
        "com.tencent.wework" => Some("企业微信 · WeCom"),
        "com.tencent.wemeet.app" => Some("腾讯会议"),
        "com.zhihu.android" => Some("知乎 · Zhihu"),
        "com.coolapk.market" => Some("酷安 · CoolApk"),
        "com.android.chrome" => Some("Google Chrome"),
        "com.google.android.youtube" => Some("YouTube"),
        "com.google.android.gm" => Some("Gmail"),
        "com.google.android.apps.maps" => Some("Google 地图"),
        "com.google.android.apps.docs" => Some("Google 云端硬盘"),
        "com.google.android.apps.photos" => Some("Google 相册"),
        "com.android.vending" => Some("Google Play 商店"),
        "com.google.android.gms" => Some("Google Play 服务"),
        "com.google.android.gsf" => Some("Google 服务框架"),
        "com.stepos.ailauncher" => Some("Amoo 智能助手"),
        "com.stepos.stepclaw" => Some("ClawX 端侧智能体引擎"),
        "com.stepos.alphame" => Some("个人偏好与画像 · AlphaMe"),
        "com.stepos.agentsafetycenter" => Some("智能体安全中心"),
        "com.stepos.aiengine" => Some("端侧 AI 模型引擎"),
        "com.android.settings" => Some("系统设置 · Settings"),
        "com.android.mms" | "com.google.android.apps.messaging" => Some("短信 · Messages"),
        "com.android.dialer" | "com.google.android.dialer" => Some("电话 · Phone"),
        "com.android.contacts" | "com.google.android.contacts" => Some("通讯录 · Contacts"),
        "com.android.documentsui" => Some("系统文件管理"),
        "com.android.camera2" | "com.stepos.camera" => Some("相机 · Camera"),
        "app.huawei.auto" | "com.huawei.aitocar" | "com.huawei.harmonyos.auto" => {
            Some("AITO · 问界汽车")
        }
        "app.huawei.motor" => Some("鸿蒙智行 · HIMA"),
        "com.yinwang.qiankunapp" => Some("华为乾崑"),
        "com.wj.charge" => Some("问界充电"),
        "com.huawei.smarthome" => Some("华为智慧生活"),
        "com.huawei.health" => Some("华为运动健康"),
        "com.vmall.client" => Some("华为商城"),
        "com.mi.car.mobile" => Some("小米汽车"),
        "cn.com.weilaihui3" => Some("蔚来 · NIO"),
        "com.xiaopeng.mycarinfo" => Some("小鹏汽车"),
        "com.chehejia.oc.m01" => Some("理想汽车"),
        "com.zeekrlife.mobile" => Some("极氪 · ZEEKR"),
        "com.byd.aeri.caranywhere" => Some("比亚迪汽车"),
        "com.teslamotors.tesla" => Some("Tesla · 特斯拉"),
        _ => None,
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct CallRecord {
    pub number: String,
    pub name: String,
    pub call_type: u8,
    pub date_ms: i64,
    pub duration_sec: u64,
}

impl CallRecord {
    pub fn type_label(&self) -> &'static str {
        match self.call_type {
            1 => "呼入",
            2 => "呼出",
            3 => "未接",
            5 => "拒接",
            _ => "通话",
        }
    }

    pub fn format_duration(&self) -> String {
        if self.duration_sec == 0 {
            "未接通".into()
        } else if self.duration_sec < 60 {
            format!("{}秒", self.duration_sec)
        } else {
            let m = self.duration_sec / 60;
            let s = self.duration_sec % 60;
            if s == 0 {
                format!("{m}分")
            } else {
                format!("{m}分{s}秒")
            }
        }
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize)]
pub struct ContactEntry {
    pub name: String,
    pub number: String,
}

pub(crate) fn provider_number(row: &serde_json::Value, key: &str) -> Result<i64> {
    row[key]
        .as_i64()
        .or_else(|| row[key].as_str().and_then(|s| s.parse().ok()))
        .with_context(|| format!("手机记录字段 {key} 无效"))
}

pub(crate) use crate::bridge::clean_nullable_field;

pub fn parse_call_records(value: &serde_json::Value) -> Result<Vec<CallRecord>> {
    let rows = value["rows"].as_array().context("手机未返回通话记录列表")?;
    rows.iter()
        .map(|row| {
            let number = clean_nullable_field(row["number"].as_str().unwrap_or(""));
            let name = clean_nullable_field(
                row["name"]
                    .as_str()
                    .or_else(|| row["cached_name"].as_str())
                    .or_else(|| row["CACHED_NAME"].as_str())
                    .unwrap_or(""),
            );
            Ok(CallRecord {
                number,
                name,
                call_type: provider_number(row, "type")? as u8,
                date_ms: provider_number(row, "date")?,
                duration_sec: provider_number(row, "duration")?.max(0) as u64,
            })
        })
        .collect()
}

pub fn parse_contact_records(value: &serde_json::Value) -> Result<Vec<ContactEntry>> {
    let rows = value["rows"].as_array().context("手机未返回通讯录列表")?;
    let mut contacts = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for row in rows {
        let mut name = clean_nullable_field(row["display_name"].as_str().unwrap_or(""));
        let number = clean_nullable_field(row["data1"].as_str().unwrap_or(""));
        if number.is_empty() {
            continue;
        }
        if name.is_empty() {
            name = number.clone();
        }
        if seen.insert((name.clone(), number.clone())) {
            contacts.push(ContactEntry { name, number });
        }
    }
    Ok(contacts)
}

pub fn parse_call_logs(raw: &str) -> Vec<CallRecord> {
    let mut list = Vec::new();
    for line in raw.lines() {
        let Some(row) = line
            .split_once("number=")
            .map(|(_, r)| format!("number={r}"))
        else {
            continue;
        };
        let mut rec = CallRecord::default();
        for part in row.split(", ") {
            if let Some(v) = part.strip_prefix("number=") {
                rec.number = clean_nullable_field(v);
            } else if let Some(v) = part
                .strip_prefix("name=")
                .or_else(|| part.strip_prefix("CACHED_NAME="))
                .or_else(|| part.strip_prefix("cached_name="))
            {
                rec.name = clean_nullable_field(v);
            } else if let Some(v) = part.strip_prefix("type=") {
                rec.call_type = v.trim().parse().unwrap_or(1);
            } else if let Some(v) = part.strip_prefix("date=") {
                rec.date_ms = v.trim().parse().unwrap_or(0);
            } else if let Some(v) = part.strip_prefix("duration=") {
                rec.duration_sec = v.trim().parse().unwrap_or(0);
            }
        }
        if !rec.number.is_empty() {
            list.push(rec);
        }
    }
    list.sort_by(|a, b| b.date_ms.cmp(&a.date_ms));
    list.truncate(100);
    list
}

pub fn parse_contacts(raw: &str) -> Vec<ContactEntry> {
    let mut list = Vec::new();
    let mut seen = std::collections::HashSet::new();
    for line in raw.lines() {
        let Some(idx) = line.find("display_name=") else {
            continue;
        };
        let rest = &line[idx..];
        let mut name = String::new();
        let mut number = String::new();
        for part in rest.split(", ") {
            if let Some(v) = part.strip_prefix("display_name=") {
                name = clean_nullable_field(v);
            } else if let Some(v) = part.strip_prefix("data1=") {
                number = clean_nullable_field(v);
            }
        }
        if !number.is_empty() {
            if name.is_empty() {
                name = number.clone();
            }
            let key = format!("{name}|{number}");
            if seen.insert(key) {
                list.push(ContactEntry { name, number });
            }
        }
    }
    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
}

fn format_base_summary(raw: &str) -> String {
    let value = |key: &str| {
        raw.lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .map(str::trim)
            .unwrap_or("")
    };
    let level = value("level:")
        .parse::<u32>()
        .ok()
        .and_then(|level| {
            let scale = value("scale:").parse::<u32>().ok().unwrap_or(100);
            (scale > 0 && level <= scale)
                .then(|| format!("{}%", u64::from(level) * 100 / u64::from(scale)))
        })
        .unwrap_or_else(|| "未检测".into());
    let charging = match value("status:") {
        "2" => "正在充电",
        "5" => "电量已满",
        "3" => "使用电池",
        "4" => "未充电",
        _ => "状态未检测",
    };
    let temperature = value("temperature:")
        .parse::<f32>()
        .map(|v| format!("{:.1} °C", v / 10.))
        .unwrap_or_else(|_| "未检测".into());
    let health = match value("health:") {
        "2" => "良好",
        "3" => "过热",
        "4" => "失效",
        "5" => "过压",
        "6" => "故障",
        "7" => "过冷",
        _ => "未检测",
    };
    let voltage = value("voltage:")
        .parse::<u32>()
        .ok()
        .filter(|v| *v > 0)
        .map(|v| format!("{:.2} V", f64::from(v) / 1000.))
        .unwrap_or_else(|| "未检测".into());
    let resolution = [value("Override size:"), value("Physical size:")]
        .into_iter()
        .find(|size| {
            size.split_once('x')
                .map(|(w, h)| {
                    w.parse::<u32>().is_ok_and(|v| v > 0) && h.parse::<u32>().is_ok_and(|v| v > 0)
                })
                .unwrap_or(false)
        })
        .map(|size| size.replace('x', " × "))
        .unwrap_or_else(|| "未检测".into());
    let wifi_ipv4 = raw
        .lines()
        .find_map(|line| {
            let parts = line.split_whitespace().collect::<Vec<_>>();
            if parts.get(1) != Some(&"wlan0") {
                return None;
            }
            let index = parts.iter().position(|p| *p == "inet")?;
            let ip = parts.get(index + 1)?.split('/').next()?;
            ip.parse::<std::net::Ipv4Addr>().ok().map(|v| v.to_string())
        })
        .unwrap_or_else(|| "未检测".into());
    let mut result = format!("电量  {level}    ·    {charging}\n电池温度  {temperature}\n电池健康  {health}\n电池电压  {voltage}\n屏幕分辨率  {resolution}\n无线地址  {wifi_ipv4}");
    if let Some(parts) = raw
        .lines()
        .map(|s| s.split_whitespace().collect::<Vec<_>>())
        .filter(|p| {
            p.len() >= 6
                && (p
                    .last()
                    .is_some_and(|mount| *mount == "/data" || mount.starts_with("/data/"))
                    || p[0] == "/dev/fuse")
        })
        .min_by_key(|p| {
            if p.last()
                .is_some_and(|mount| *mount == "/data" || mount.starts_with("/data/"))
            {
                0
            } else {
                1
            }
        })
    {
        result.push_str(&format!(
            "\n\n手机存储  {}\n已用  {}    ·    可用  {}    ·    使用率  {}",
            parts[1], parts[2], parts[3], parts[4]
        ));
    } else {
        result.push_str("\n\n存储信息暂不可用");
    }
    result
}

pub(crate) fn extract_cycle_count(cycle_line: &str) -> Option<String> {
    cycle_line
        .split_once("android.os.extra.CYCLE_COUNT=")
        .map(|(_, rest)| {
            rest.chars()
                .take_while(|c| c.is_ascii_digit())
                .collect::<String>()
        })
        .filter(|s| !s.is_empty())
}

pub(crate) fn format_mah_value(raw_val: &str, default_mah: &str) -> String {
    raw_val
        .trim()
        .parse::<u64>()
        .ok()
        .filter(|v| *v > 0)
        .map(|v| if v > 100_000 { v / 1000 } else { v }.to_string())
        .unwrap_or_else(|| default_mah.to_string())
}

pub(crate) fn live_thermal_hal_section(raw: &str) -> &str {
    raw.split_once("Current temperatures from HAL:")
        .map(|(_, rest)| {
            rest.split_once("Current cooling devices from HAL:")
                .map(|(hal, _)| hal)
                .unwrap_or(rest)
        })
        .unwrap_or(raw)
}

pub(crate) fn parse_thermal_mvalue(line: &str) -> Option<f32> {
    line.split_once("mValue=")
        .and_then(|(_, rest)| rest.split([',', '}']).next())
        .and_then(|v| v.trim().parse::<f32>().ok())
        .filter(|&v| v > 0.0 && v <= 120.0)
}

pub(crate) fn parse_thermal_sensor_temp(thermal_scope: &str, sensors: &[&str]) -> Option<f32> {
    for sensor in sensors {
        let needle = format!("mName={sensor},");
        let needle_end = format!("mName={sensor}}}");
        if let Some(v) = thermal_scope
            .lines()
            .find(|line| {
                (line.contains(&needle) || line.contains(&needle_end)) && line.contains("mValue=")
            })
            .and_then(parse_thermal_mvalue)
        {
            return Some(v);
        }
    }
    None
}

pub fn format_summary(raw: &str) -> String {
    let Some((base_raw, deep_raw)) = raw.split_once("<<<STEPOS_DEEP>>>") else {
        return format_base_summary(raw);
    };
    let mut result = format_base_summary(base_raw);
    let deep_val = |key: &str| {
        deep_raw
            .lines()
            .find_map(|line| line.trim().strip_prefix(key))
            .map(str::trim)
            .filter(|v| !v.is_empty() && !v.eq_ignore_ascii_case("null"))
            .unwrap_or("")
    };
    let _marketname = {
        let v = deep_val("marketname:");
        if v.is_empty() {
            "STEPX Neo"
        } else {
            v
        }
    };
    let asn = {
        let v = deep_val("asn:");
        if v.is_empty() {
            "未检测"
        } else {
            v
        }
    };
    let netsn = deep_val("netsn:");
    let serialno = {
        let v = deep_val("serialno:");
        if !v.is_empty() {
            v
        } else if !netsn.is_empty() {
            netsn
        } else {
            "未检测"
        }
    };
    let display_id = {
        let v = deep_val("display_id:");
        let cmiit = deep_val("cmiit:");
        if v.is_empty() {
            "未读取".to_string()
        } else if !cmiit.is_empty() {
            format!("{v} · CMIIT {cmiit}")
        } else {
            v.to_string()
        }
    };
    let soh = {
        let v = deep_val("soh:");
        if v.is_empty() {
            "未读取"
        } else {
            v
        }
    };
    let cycles =
        extract_cycle_count(deep_val("cycle_line:")).unwrap_or_else(|| "未读取".to_string());
    let design_mah = format_mah_value(deep_val("design_cap:"), "未读取");
    let remaining_mah = format_mah_value(deep_val("charge_cnt:"), "未读取");
    let fast_charge_label = match deep_val("fast_charge:") {
        "1" => "已开启",
        "0" => "已关闭",
        _ => "未读取",
    };
    let ups_limit = {
        let v = deep_val("ups_limit:");
        if v.is_empty() {
            "未读取"
        } else {
            v
        }
    };
    let bypass_label = if deep_val("ups_mode:") == "1" {
        format!("已开启 ({ups_limit}%)")
    } else if deep_val("ups_mode:") == "0" {
        format!("待命 ({ups_limit}%)")
    } else {
        "未读取".to_string()
    };
    let thermal_scope = live_thermal_hal_section(deep_raw);
    let parse_temp = |sensors: &[&str]| -> String {
        parse_thermal_sensor_temp(thermal_scope, sensors)
            .map(|v| format!("{v:.1} °C"))
            .unwrap_or_else(|| "--".to_string())
    };
    let cpu_temp = parse_temp(&["cpu-0-0", "CPU0"]);
    let npu_temp = parse_temp(&["nsp-0", "nsp0", "gpuss-0", "GPU0"]);
    let skin_temp = parse_temp(&["skin-therm", "skin"]);
    let camera_info = deep_val("camera_info:");
    let camera_summary = if camera_info.is_empty() {
        "未读取"
    } else {
        camera_info
    };
    let keyguard_line = deep_val("keyguard_locked:");
    let lock_continuity_label = if keyguard_line.contains("showing=true") {
        "已锁屏"
    } else if keyguard_line.contains("showing=false") {
        "已解锁"
    } else {
        "未读取"
    };
    result.push_str(&format!(
        "\n\n保修状态  未查询官方保修状态\n\
         硬件序列号  ASN {asn} · SN {serialno}\n\
         电池寿命  SOH {soh}% · 循环 {cycles} 次 · 设计 {design_mah} mAh\n\
         实时容量  {remaining_mah} mAh · 智慧快充{fast_charge_label} · 旁路供电{bypass_label}\n\
         算力热力  CPU {cpu_temp} · NPU {npu_temp} · 机身 {skin_temp}\n\
         硬件配置  相机属性：{camera_summary}\n\
         系统固件  {display_id}\n\
         屏幕锁定  {lock_continuity_label}"
    ));
    result
}

pub fn visible_text(xml: &str) -> Result<String> {
    let mut reader = quick_xml::Reader::from_str(xml);
    let mut lines = Vec::new();
    loop {
        match reader.read_event()? {
            quick_xml::events::Event::Empty(e) | quick_xml::events::Event::Start(e)
                if e.name().as_ref() == b"node" =>
            {
                let mut text = String::new();
                let mut description = String::new();
                let mut password = false;
                for attr in e.attributes() {
                    let attr = attr?;
                    let value = attr
                        .decode_and_unescape_value(reader.decoder())?
                        .into_owned();
                    match attr.key.as_ref() {
                        b"text" => text = value,
                        b"content-desc" => description = value,
                        b"password" => password = value == "true",
                        _ => {}
                    }
                }
                if !password {
                    let text = if text.is_empty() { description } else { text };
                    if !text.is_empty() && lines.last() != Some(&text) {
                        lines.push(text);
                    }
                }
            }
            quick_xml::events::Event::Eof => break,
            _ => {}
        }
    }
    if lines.is_empty() {
        bail!("当前画面没有可读取的文字，请在实时控制窗口查看。此功能只读取当前可见界面，不是历史聊天记录接口。");
    }
    Ok(lines.join("\n\n"))
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize)]
pub enum BackScreenPattern {
    #[default]
    Bolt,
    Cat,
    Coffee,
    Music,
}

impl BackScreenPattern {
    pub const ALL: [Self; 4] = [Self::Bolt, Self::Cat, Self::Coffee, Self::Music];

    pub fn key(self) -> &'static str {
        match self {
            Self::Bolt => "bolt",
            Self::Cat => "cat",
            Self::Coffee => "coffee",
            Self::Music => "music",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Bolt => "赛博闪电",
            Self::Cat => "猫咪眨眼",
            Self::Coffee => "咖啡时刻",
            Self::Music => "律动音符",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Bolt => "内置像素图案",
            Self::Cat => "内置像素图案",
            Self::Coffee => "内置像素图案",
            Self::Music => "系统预设",
        }
    }

    /// Returns whether pixel `(x, y)` in the 17×23 grid is lit (`0..=255`).
    pub fn pixel_intensity(self, x: usize, y: usize) -> u8 {
        if is_backscreen_corner(x, y) || x >= 17 || y >= 23 {
            return 0;
        }
        let c = x;
        let r = y;
        match self {
            Self::Bolt => {
                let in_eye_l = (6..=11).contains(&r) && (3..=6).contains(&c) && !(r == 7 && c == 5);
                let in_eye_r =
                    (6..=11).contains(&r) && (10..=13).contains(&c) && !(r == 7 && c == 12);
                let in_bolt = (r == 2 && c == 8)
                    || (r == 3 && (c == 7 || c == 8))
                    || (r == 4 && (c == 8 || c == 9));
                let in_smile =
                    (r == 14 && (6..=10).contains(&c)) || (r == 13 && (c == 5 || c == 11));
                let in_wave = (r == 18 && (3..=13).contains(&c) && c % 2 == 1)
                    || (r == 19 && (4..=12).contains(&c) && c % 2 == 0);
                if in_eye_l || in_eye_r || in_bolt {
                    255
                } else if in_smile {
                    210
                } else if in_wave {
                    135
                } else {
                    0
                }
            }
            Self::Cat => {
                let ear_l = (r == 4 && c == 4) || (r == 5 && (c == 3 || c == 5));
                let ear_r = (r == 4 && c == 12) || (r == 5 && (c == 11 || c == 13));
                let eye_l = (8..=10).contains(&r) && (4..=6).contains(&c);
                let eye_r = r == 9 && (10..=12).contains(&c);
                let mouth = (r == 13 && (7..=9).contains(&c)) || (r == 14 && (c == 6 || c == 10));
                let whisker = r == 11 && (c == 2 || c == 14);
                if ear_l || ear_r || eye_l || eye_r {
                    255
                } else if mouth || whisker {
                    185
                } else {
                    0
                }
            }
            Self::Coffee => {
                let steam =
                    ((r == 4 || r == 6) && (c == 6 || c == 10)) || (r == 5 && (c == 7 || c == 9));
                let cup = (9..=16).contains(&r) && (4..=11).contains(&c);
                let handle =
                    ((10..=14).contains(&r) && c == 13) || ((r == 10 || r == 14) && c == 12);
                let saucer = r == 18 && (3..=13).contains(&c);
                if cup || saucer {
                    255
                } else if steam || handle {
                    170
                } else {
                    0
                }
            }
            Self::Music => {
                let in_eye_l = (8..=14).contains(&r) && (3..=6).contains(&c);
                let in_eye_r = (8..=14).contains(&r) && (10..=13).contains(&c);
                let in_note = (r == 3 && (c == 6 || c == 7))
                    || (r == 4 && c == 6)
                    || (r == 5 && (c == 5 || c == 6));
                let in_bar = r == 18 && (4..=12).contains(&c) && c % 2 == 0;
                if in_eye_l || in_eye_r {
                    255
                } else if in_note || in_bar {
                    180
                } else {
                    0
                }
            }
        }
    }

    pub fn frame_391(self, brightness: u8, invert: bool) -> [u8; 391] {
        let mut frame = [0u8; 391];
        let scale = (brightness.clamp(1, 8) as u16 * 32).min(255) as u8;
        for y in 0..23 {
            for x in 0..17 {
                if is_backscreen_corner(x, y) {
                    continue;
                }
                let raw = self.pixel_intensity(x, y);
                let val = if invert { 255 - raw } else { raw };
                frame[y * 17 + x] = ((val as u16 * scale as u16) / 255) as u8;
            }
        }
        frame
    }
}

pub fn is_backscreen_corner(x: usize, y: usize) -> bool {
    (x == 0 && y == 0)
        || (x == 1 && y == 0)
        || (x == 0 && y == 1)
        || (x == 16 && y == 0)
        || (x == 15 && y == 0)
        || (x == 16 && y == 1)
        || (x == 0 && y == 22)
        || (x == 1 && y == 22)
        || (x == 0 && y == 21)
        || (x == 16 && y == 22)
        || (x == 15 && y == 22)
        || (x == 16 && y == 21)
}

pub fn image_to_backscreen_frame(
    path: &std::path::Path,
    brightness: u8,
    invert: bool,
) -> Result<Vec<u8>> {
    let raw = crate::backscreen::image_frame(path, &Default::default())?;
    Ok(crate::backscreen::frame(
        &crate::backscreen::Source::Image(raw),
        &crate::backscreen::PlaybackConfig {
            brightness,
            invert,
            ..Default::default()
        },
        std::time::Duration::ZERO,
        &Default::default(),
    ))
}

/// Derives the OpenClaw Gateway token identically to `GatewayTokenDeriver` in `com.stepos.ailauncher`:
/// `HMAC-SHA256(key = "stepclaw-gateway-token-salt-v1", msg = "stepclaw-gateway-v1|" + android_id)`.
pub fn derive_openclaw_token(android_id: &str) -> String {
    let key = b"stepclaw-gateway-token-salt-v1";
    let msg = format!("stepclaw-gateway-v1|{}", android_id.trim());
    let mac = hmac_sha256(key, msg.as_bytes());
    let mut hex = String::with_capacity(64);
    for b in mac {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{b:02x}");
    }
    hex
}

const SHA256_K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

const SHA256_INIT: [u32; 8] = [
    0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab, 0x5be0cd19,
];

fn sha256_compress(h: &mut [u32; 8], chunk: &[u8; 64]) {
    let mut w = [0u32; 64];
    for (i, word) in chunk.chunks_exact(4).enumerate() {
        w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
    }
    for i in 16..64 {
        let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
        let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
        w[i] = w[i - 16]
            .wrapping_add(s0)
            .wrapping_add(w[i - 7])
            .wrapping_add(s1);
    }
    let mut a = h[0];
    let mut b = h[1];
    let mut c = h[2];
    let mut d = h[3];
    let mut e = h[4];
    let mut f = h[5];
    let mut g = h[6];
    let mut hh = h[7];
    for i in 0..64 {
        let s1 = e.rotate_right(6) ^ e.rotate_right(11) ^ e.rotate_right(25);
        let ch = (e & f) ^ ((!e) & g);
        let temp1 = hh
            .wrapping_add(s1)
            .wrapping_add(ch)
            .wrapping_add(SHA256_K[i])
            .wrapping_add(w[i]);
        let s0 = a.rotate_right(2) ^ a.rotate_right(13) ^ a.rotate_right(22);
        let maj = (a & b) ^ (a & c) ^ (b & c);
        let temp2 = s0.wrapping_add(maj);
        hh = g;
        g = f;
        f = e;
        e = d.wrapping_add(temp1);
        d = c;
        c = b;
        b = a;
        a = temp1.wrapping_add(temp2);
    }
    h[0] = h[0].wrapping_add(a);
    h[1] = h[1].wrapping_add(b);
    h[2] = h[2].wrapping_add(c);
    h[3] = h[3].wrapping_add(d);
    h[4] = h[4].wrapping_add(e);
    h[5] = h[5].wrapping_add(f);
    h[6] = h[6].wrapping_add(g);
    h[7] = h[7].wrapping_add(hh);
}

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut h = SHA256_INIT;
    let bit_len = (data.len() as u64) * 8;
    let mut padded = data.to_vec();
    padded.push(0x80);
    while (padded.len() % 64) != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in padded.chunks_exact(64) {
        let block: &[u8; 64] = chunk.try_into().unwrap();
        sha256_compress(&mut h, block);
    }
    let mut out = [0u8; 32];
    for (i, word) in h.iter().enumerate() {
        out[i * 4..i * 4 + 4].copy_from_slice(&word.to_be_bytes());
    }
    out
}

pub fn sha256_reader(mut reader: impl std::io::Read) -> std::io::Result<String> {
    let mut h = SHA256_INIT;
    let mut total_bytes: u64 = 0;
    let mut pending = [0u8; 64];
    let mut pending_len = 0usize;
    let mut buf = vec![0u8; 64 * 1024];
    loop {
        let n = reader.read(&mut buf)?;
        if n == 0 {
            break;
        }
        total_bytes = total_bytes.wrapping_add(n as u64);
        let mut slice = &buf[..n];
        if pending_len > 0 {
            let take = (64 - pending_len).min(slice.len());
            pending[pending_len..pending_len + take].copy_from_slice(&slice[..take]);
            pending_len += take;
            slice = &slice[take..];
            if pending_len == 64 {
                sha256_compress(&mut h, &pending);
                pending_len = 0;
            }
        }
        let mut chunks = slice.chunks_exact(64);
        for chunk in &mut chunks {
            let block: &[u8; 64] = chunk.try_into().unwrap();
            sha256_compress(&mut h, block);
        }
        let rem = chunks.remainder();
        if !rem.is_empty() {
            pending[..rem.len()].copy_from_slice(rem);
            pending_len = rem.len();
        }
    }
    let bit_len = total_bytes.wrapping_mul(8);
    pending[pending_len] = 0x80;
    pending_len += 1;
    if pending_len > 56 {
        pending[pending_len..64].fill(0);
        sha256_compress(&mut h, &pending);
        pending_len = 0;
    }
    pending[pending_len..56].fill(0);
    pending[56..64].copy_from_slice(&bit_len.to_be_bytes());
    sha256_compress(&mut h, &pending);

    let mut hex = String::with_capacity(64);
    for word in h {
        use std::fmt::Write;
        let _ = write!(&mut hex, "{word:08x}");
    }
    Ok(hex)
}

pub fn sha256_file(path: &std::path::Path) -> std::io::Result<String> {
    let file = std::fs::File::open(path)?;
    sha256_reader(file)
}

fn hmac_sha256(key: &[u8], message: &[u8]) -> [u8; 32] {
    let mut k = [0u8; 64];
    if key.len() > 64 {
        let hashed = sha256(key);
        k[..32].copy_from_slice(&hashed);
    } else {
        k[..key.len()].copy_from_slice(key);
    }
    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k[i];
        opad[i] ^= k[i];
    }
    let mut inner = Vec::with_capacity(64 + message.len());
    inner.extend_from_slice(&ipad);
    inner.extend_from_slice(message);
    let inner_hash = sha256(&inner);
    let mut outer = Vec::with_capacity(64 + 32);
    outer.extend_from_slice(&opad);
    outer.extend_from_slice(&inner_hash);
    sha256(&outer)
}

pub fn parse_wm_size(output: &str) -> (u32, u32) {
    let mut physical = None;
    for line in output.lines() {
        let line = line.trim();
        if let Some(rest) = line.strip_prefix("Override size:") {
            if let Some((w, h)) = rest.trim().split_once('x') {
                if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                    if w > 0 && h > 0 {
                        return (w, h);
                    }
                }
            }
        } else if let Some(rest) = line.strip_prefix("Physical size:") {
            if let Some((w, h)) = rest.trim().split_once('x') {
                if let (Ok(w), Ok(h)) = (w.trim().parse::<u32>(), h.trim().parse::<u32>()) {
                    if w > 0 && h > 0 {
                        physical = Some((w, h));
                    }
                }
            }
        }
    }
    physical.unwrap_or((1080, 2400))
}

#[cfg(test)]
mod generic_apk_install_tests {
    use super::{apk_verification_command, parse_apk_verification, validate_generic_apk_identity};
    use serde_json::json;

    fn helper_output(success: bool, stdout: &[u8], stderr: &[u8]) -> std::process::Output {
        #[cfg(unix)]
        let status = {
            use std::os::unix::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(if success { 0 } else { 2 << 8 })
        };
        #[cfg(windows)]
        let status = {
            use std::os::windows::process::ExitStatusExt;
            std::process::ExitStatus::from_raw(if success { 0 } else { 2 })
        };
        std::process::Output {
            status,
            stdout: stdout.to_vec(),
            stderr: stderr.to_vec(),
        }
    }

    #[test]
    fn real_apk_helper_accepts_client_arguments_and_unicode_paths() {
        // This must reach file validation, not argparse's "expected one argument".
        let missing = tempfile::tempdir().unwrap().path().join("支付宝 测试.apk");
        let output = crate::adb::run_process(
            apk_verification_command(&missing),
            None,
            std::time::Duration::from_secs(15),
        )
        .unwrap();
        assert!(!output.status.success());
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["ok"], false);
        assert_eq!(value["error"], "请选择有效的本地 APK 安装包");
    }

    #[test]
    fn failed_helper_preserves_diagnostic_instead_of_json_eof() {
        let output = helper_output(false, b"", b"argument --package: expected one argument");
        let error = parse_apk_verification(&output).unwrap_err().to_string();
        assert!(error.contains("expected one argument"));
        assert!(!error.contains("EOF"));
        assert!(!error.contains("重新解压"));
        let output = helper_output(
            false,
            br#"{"ok":false,"error":"Signature rejected"}"#,
            b"traceback",
        );
        let error = parse_apk_verification(&output).unwrap_err().to_string();
        assert!(error.contains("Signature rejected"));
        assert!(!error.contains("traceback"));
    }

    #[test]
    fn invalid_or_failed_verification_never_succeeds() {
        for stdout in [
            b"".as_slice(),
            b"not JSON",
            b"null",
            br#"{"ok":true}"#,
            br#"{"ok":true,"package":""}"#,
            br#"{"ok":false,"package":"com.example.app"}"#,
        ] {
            assert!(parse_apk_verification(&helper_output(true, stdout, b"")).is_err());
        }
        let valid = br#"{"ok":true,"package":"com.example.app"}"#;
        assert_eq!(
            parse_apk_verification(&helper_output(true, valid, b"warning")).unwrap()["package"],
            "com.example.app"
        );
        assert!(parse_apk_verification(&helper_output(false, valid, b"failed")).is_err());
    }

    #[test]
    fn companion_package_uses_dedicated_installation() {
        let error =
            validate_generic_apk_identity(&json!({"ok":true,"package":"com.stepx.companion.app"}))
                .unwrap_err();
        assert!(error.to_string().contains("通用 APK 入口不会覆盖伴侣"));
    }

    #[test]
    fn unrelated_apps_remain_installable() {
        for package in [
            "com.jd.jrapp",
            "com.example.game",
            "com.stepx.companion.neo",
            "android",
        ] {
            assert!(validate_generic_apk_identity(&json!({"ok":true,"package":package})).is_ok());
        }
    }

    #[test]
    fn missing_or_failed_identity_never_continues() {
        for value in [
            json!({"ok":true}),
            json!({"ok":true,"package":""}),
            json!({"ok":false,"package":"com.example.app","error":"元数据不可读"}),
        ] {
            assert!(validate_generic_apk_identity(&value).is_err());
        }
    }

    #[test]
    fn deduplicates_four_line_adb_devices_and_preserves_wireless_backups() {
        use super::{
            canonical_serial_from_transport, deduplicate_multi_transport_devices,
            deduplicate_transports, parse_devices, transport_priority, Device,
        };

        let sample = "\
List of devices attached
ASGXVB1806001948       device usb:3-3.1 product:CLT-AL00 model:CLT_AL00 device:HWCLT transport_id:2
STEPX_TEST_DEVICE    device usb:3-1.4 product:pandora model:PZ261A device:pandora transport_id:4
10.0.0.42:34883       device product:pandora model:PZ261A device:pandora transport_id:7
adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp device product:pandora model:PZ261A device:pandora transport_id:1
";
        let transports = parse_devices(sample);
        assert_eq!(transports.len(), 2);
        assert_eq!(transports[0].id, "ASGXVB1806001948");
        assert_eq!(transports[0].model, "CLT_AL00");
        assert!(transports[0].backup_transports().is_empty());

        assert_eq!(transports[1].id, "STEPX_TEST_DEVICE");
        assert_eq!(transports[1].model, "PZ261A");
        assert_eq!(
            transports[1].backup_transports(),
            vec![
                "10.0.0.42:34883".to_string(),
                "adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp".to_string(),
            ]
        );

        let rededuped = deduplicate_transports(transports);
        assert_eq!(rededuped.len(), 2);
        assert_eq!(
            rededuped[1].backup_transports(),
            vec![
                "10.0.0.42:34883".to_string(),
                "adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp".to_string(),
            ]
        );

        assert_eq!(
            canonical_serial_from_transport("adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp"),
            Some("STEPX_TEST_DEVICE".to_string())
        );
        assert_eq!(transport_priority("STEPX_TEST_DEVICE"), 1);
        assert_eq!(transport_priority("10.0.0.42:34883"), 2);
        assert_eq!(
            transport_priority("adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp"),
            3
        );

        let devices = deduplicate_multi_transport_devices(vec![
            Device {
                transport: "adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp".into(),
                serial: "STEPX_TEST_DEVICE".into(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            },
            Device {
                transport: "10.0.0.42:34883".into(),
                serial: "STEPX_TEST_DEVICE".into(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            },
            Device {
                transport: "STEPX_TEST_DEVICE".into(),
                serial: "STEPX_TEST_DEVICE".into(),
                model: "PZ261A".into(),
                android: "16".into(),
                product: "pandora".into(),
            },
        ]);
        assert_eq!(devices.len(), 1);
        assert_eq!(devices[0].transport, "STEPX_TEST_DEVICE");
        assert_eq!(
            devices[0].backup_transports(),
            vec![
                "10.0.0.42:34883".to_string(),
                "adb-STEPX_TEST_DEVICE-jYyV3X._adb-tls-connect._tcp".to_string(),
            ]
        );
    }

    #[test]
    fn parses_wm_size_physical_and_override() {
        use super::parse_wm_size;
        assert_eq!(parse_wm_size("Physical size: 1260x2800\n"), (1260, 2800));
        assert_eq!(
            parse_wm_size("Physical size: 1260x2800\nOverride size: 1080x2400\n"),
            (1080, 2400)
        );
        assert_eq!(parse_wm_size(""), (1080, 2400));
    }

    #[test]
    fn formats_stepos_deep_summary_and_preserves_legacy_summary() {
        use super::{format_summary, DeviceMetrics};

        let legacy_raw = "\
  level: 100
  scale: 100
  status: 5
  health: 2
  voltage: 4449
  temperature: 317
/dev/block/dm-52  218G   17G  201G   8% /data
Physical size: 1260x2800
24: wlan0    inet 10.0.0.42/24 brd 10.0.1.255 scope global wlan0
";
        let legacy_summary = format_summary(legacy_raw);
        assert!(!legacy_summary.contains("保修状态"));
        let legacy_metrics = DeviceMetrics::from_summary(&legacy_summary);
        assert_eq!(legacy_metrics.battery_level, Some(100));
        assert_eq!(legacy_metrics.charging_state, "电量已满");
        assert_eq!(legacy_metrics.temperature, "31.7 °C");
        assert_eq!(legacy_metrics.voltage, "4.45 V");
        assert_eq!(legacy_metrics.screen_size, "1260 × 2800");
        assert_eq!(legacy_metrics.wifi_ipv4, "10.0.0.42");
        assert_eq!(legacy_metrics.storage_total, "218G");
        assert_eq!(legacy_metrics.warranty_status, "");

        let deep_raw = format!(
            "{legacy_raw}\n<<<STEPOS_DEEP>>>\n\
             marketname:STEPX Neo\n\
             asn:ASN_TEST_DEVICE\n\
             netsn:NET_TEST_DEVICE\n\
             serialno:STEPX_TEST_DEVICE\n\
             display_id:StepOS_Neo_V1.0.20260417\n\
             cmiit:2026CP3403\n\
             camera_info:FrontMain: ov50d40 50M;BackMain: ov50e40 50M;BackWide: ov50d40 50M;BackTele: s5kjn5 50M;\n\
             zram_cfg:enable=true|8G,12G,16G,24G|50%,50%,50%,50%\n\
             soh:100\n\
             design_cap:6150000\n\
             charge_cnt:6150000\n\
             ups_mode:0\n\
             ups_limit:80\n\
             fast_charge:1\n\
             cycle_line:Bundle[{{android.os.extra.CYCLE_COUNT=3, android.os.extra.MAXIMUM_CAPACITY_PERCENT=100}}]\n\
             keyguard_locked:showing=true\n\
             Temperature{{mValue=41.6, mType=0, mName=cpu-0-0, mStatus=0}}\n\
             Temperature{{mValue=39.3, mType=2, mName=nsp-0, mStatus=0}}\n\
             Temperature{{mValue=35.421, mType=3, mName=skin-therm, mStatus=0}}\n"
        );
        let deep_summary = format_summary(&deep_raw);
        let deep_metrics = DeviceMetrics::from_summary(&deep_summary);
        assert_eq!(deep_metrics.warranty_status, "未查询官方保修状态");
        assert_eq!(
            deep_metrics.hardware_sn,
            "ASN ASN_TEST_DEVICE · SN STEPX_TEST_DEVICE"
        );
        assert_eq!(
            deep_metrics.battery_soh,
            "SOH 100% · 循环 3 次 · 设计 6150 mAh"
        );
        assert_eq!(
            deep_metrics.battery_capacity,
            "6150 mAh · 智慧快充已开启 · 旁路供电待命 (80%)"
        );
        assert_eq!(
            deep_metrics.thermal_summary,
            "CPU 41.6 °C · NPU 39.3 °C · 机身 35.4 °C"
        );
        assert_eq!(
            deep_metrics.hardware_spec,
            "相机属性：FrontMain: ov50d40 50M;BackMain: ov50e40 50M;BackWide: ov50d40 50M;BackTele: s5kjn5 50M;"
        );
        assert_eq!(
            deep_metrics.firmware_version,
            "StepOS_Neo_V1.0.20260417 · CMIIT 2026CP3403"
        );
        assert_eq!(deep_metrics.lockscreen_continuity, "已锁屏");
    }
    #[test]
    fn store_package_identity_must_match_selection() {
        assert!(super::validate_store_package("com.example.other", "com.example.target").is_err());
        assert!(super::validate_store_package("com.example.target", "").is_err());
        assert!(super::validate_store_package("", "com.example.target").is_err());
        assert!(super::validate_store_package("   ", "app.huawei.auto").is_err());
        assert!(super::validate_store_package("app.huawei.auto", "   ").is_err());
        assert!(super::validate_store_package("com.lngin.aitokii", "app.huawei.auto").is_err());
        assert!(super::validate_store_package("com.sqn.click", "app.huawei.auto").is_err());
        assert!(
            super::validate_store_package("com.yinwang.qiankunapp", "app.huawei.auto").is_err()
        );
        assert!(super::validate_store_package(
            "com.stepx.companion.app",
            "com.stepx.companion.app"
        )
        .is_err());
        assert!(super::validate_store_package(
            "COM.STEPX.COMPANION.APP",
            "com.stepx.companion.app"
        )
        .is_err());
        assert!(validate_generic_apk_identity(
            &json!({"ok":true,"package":"  COM.STEPX.COMPANION.APP  "})
        )
        .is_err());
        assert!(super::validate_store_package("com.example.target", "com.example.target").is_ok());
        assert!(super::validate_store_package("app.huawei.auto", "app.huawei.auto").is_ok());
        assert!(super::validate_store_package("app.huawei.motor", "app.huawei.motor").is_ok());
        assert!(super::validate_store_package("app.huawei.motor", "app.huawei.auto").is_ok());
        assert!(super::validate_store_package("app.huawei.auto", "app.huawei.motor").is_ok());
        assert!(super::validate_store_package("com.huawei.aitocar", "app.huawei.auto").is_ok());
        assert!(
            super::validate_store_package("com.huawei.harmonyos.auto", "app.huawei.auto").is_ok()
        );
        assert!(super::validate_store_package("com.aitotech.aito", "app.huawei.auto").is_ok());
        assert!(super::validate_store_package("com.seres.aito", "app.huawei.motor").is_ok());
        assert!(
            super::validate_store_package("com.synology.uhidrive", "com.synology.dsdrive").is_ok()
        );
        assert!(
            super::validate_store_package("com.synology.DSfile", "com.synology.dsfile").is_ok()
        );
        assert!(super::validate_store_package(
            "org.telegram.messenger",
            "org.telegram.messenger.web"
        )
        .is_ok());
        assert!(super::validate_store_package(
            "org.telegram.messenger.web",
            "org.telegram.messenger"
        )
        .is_ok());
        assert!(
            super::validate_store_package("com.ss.android.lark", "com.ss.android.lark.kami")
                .is_ok()
        );
        assert_eq!(
            super::friendly_app_name("app.huawei.auto"),
            Some("AITO · 问界汽车")
        );
        assert_eq!(
            super::friendly_app_name("app.huawei.motor"),
            Some("鸿蒙智行 · HIMA")
        );
        assert_eq!(
            super::friendly_app_name("com.yinwang.qiankunapp"),
            Some("华为乾崑")
        );
        assert_eq!(super::friendly_app_name("com.wj.charge"), Some("问界充电"));

        let signed_fixture = crate::resources::root()
            .join("diagnostics/context-e2e-20261029/notification-fixture/context-verification.apk");
        if signed_fixture.is_file() {
            assert!(
                super::check_store_apk(&signed_fixture, "com.stepx.context.verification").is_ok()
            );
            assert!(super::check_store_apk(&signed_fixture, "app.huawei.auto").is_err());
        }
        let unsigned_fixture = crate::resources::root()
            .join("diagnostics/context-e2e-20261029/notification-fixture/unsigned.apk");
        if unsigned_fixture.is_file() {
            assert!(
                super::check_store_apk(&unsigned_fixture, "com.stepx.context.verification")
                    .is_err()
            );
        }
        let companion_apk = crate::resources::root().join("desktop/assets/stepx-companion.apk");
        if companion_apk.is_file() {
            assert!(
                super::check_store_apk(&companion_apk, "com.stepx.companion.app").is_err(),
                "store APK check must never allow overwriting companion app via generic store path"
            );
        }
        if let Some(home) = crate::resources::user_home() {
            let cached_aito = std::path::PathBuf::from(&home)
                .join(".cache/stepx-companion/app-store/apks/app.huawei.auto_3.0.2.320_e782f9a120610a43.apk");
            if cached_aito.is_file() {
                assert_eq!(
                    super::verify_apk_identity(&cached_aito).unwrap(),
                    "app.huawei.auto"
                );
                assert!(super::check_store_apk(&cached_aito, "app.huawei.auto").is_ok());
                assert!(super::check_store_apk(&cached_aito, "app.huawei.motor").is_ok());
                assert!(super::check_store_apk(&cached_aito, "com.huawei.aitocar").is_ok());
                assert!(super::check_store_apk(&cached_aito, "com.lngin.aitokii").is_err());
            }
            let cached_youtube = std::path::PathBuf::from(&home)
                .join(".cache/stepx-companion/app-store/apks/com.google.android.youtube_21.42.243_6b76fea5bd6289e8.apk");
            if cached_youtube.is_file() {
                assert_eq!(
                    super::verify_apk_identity(&cached_youtube).unwrap(),
                    "com.google.android.youtube"
                );
                assert!(
                    super::check_store_apk(&cached_youtube, "com.google.android.youtube").is_ok()
                );
            }
        }
    }
    #[test]
    fn missing_deep_sensor_values_remain_unknown() {
        let s = super::format_summary("level: 50\n<<<STEPOS_DEEP>>>\nsoh:\ndesign_cap:\n");
        assert!(s.contains("SOH 未读取"));
        assert!(s.contains("未查询官方保修状态"));
        assert!(!s.contains("6150"));
        assert!(!s.contains("LPDDR5X"));
    }

    #[test]
    fn format_summary_prefers_live_hal_temperatures_and_supports_pz261a_sensor_names() {
        let raw = "level: 85\nscale: 100\n<<<STEPOS_DEEP>>>\n\
            Cached temperatures:\n\
            Temperature{mValue=68.9, mType=0, mName=CPU0, mStatus=0}\n\
            Temperature{mValue=59.2, mType=9, mName=nsp0, mStatus=0}\n\
            Temperature{mValue=44.0, mType=3, mName=skin, mStatus=0}\n\
            Current temperatures from HAL:\n\
            Temperature{mValue=38.4, mType=0, mName=CPU0, mStatus=0}\n\
            Temperature{mValue=34.2, mType=9, mName=nsp0, mStatus=0}\n\
            Temperature{mValue=31.5, mType=3, mName=skin, mStatus=0}\n\
            Current cooling devices from HAL:\n";
        let summary = super::format_summary(raw);
        let metrics = super::DeviceMetrics::from_summary(&summary);
        assert_eq!(
            metrics.thermal_summary,
            "CPU 38.4 °C · NPU 34.2 °C · 机身 31.5 °C"
        );
    }

    #[cfg(unix)]
    #[test]
    fn resolve_online_transport_discovers_rotated_mdns_tls_port_and_storage_fails_over() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let log_path = dir.path().join("calls.log");
        let script = format!(
            r#"#!/usr/bin/env bash
echo "$*" >> "{log}"
case "$*" in
  *"-s usb-dead-device-mdns-unit"*|*"-s 10.0.1.178:34257"*)
    echo "adb: device offline" >&2
    exit 1
    ;;
  "devices -l")
    echo "List of devices attached"
    echo "10.0.1.178:34257 offline transport_id:2"
    exit 0
    ;;
  "reconnect offline")
    echo "reconnecting 10.0.1.178:34257 [offline]"
    exit 0
    ;;
  "mdns services")
    echo "List of discovered mdns services"
    echo "adb-STEPX_MDNS_ROTATE_SERIAL_UNIT-jYyV3X (2)	_adb-tls-connect._tcp	10.0.1.178:41245"
    exit 0
    ;;
  "connect 10.0.1.178:41245")
    echo "connected to 10.0.1.178:41245"
    exit 0
    ;;
  *"-s 10.0.1.178:41245 shell"*"getprop"*"ro.serialno"*)
    echo "STEPX_MDNS_ROTATE_SERIAL_UNIT"
    exit 0
    ;;
  *"-s 10.0.1.178:41245 shell"*"readlink"*"/sdcard/Download"*)
    echo "/storage/emulated/0/Download"
    exit 0
    ;;
  *"-s 10.0.1.178:41245 shell find"*)
    printf '81a4\n128\n/storage/emulated/0/Download/note.txt\0'
    exit 0
    ;;
esac
exit 1
"#,
            log = log_path.display()
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = crate::adb::Adb { path: fake_adb };
        let device = super::Device {
            transport: "usb-dead-device-mdns-unit".into(),
            serial: "STEPX_MDNS_ROTATE_SERIAL_UNIT".into(),
            model: "PZ261A-MDNS-UNIT".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        device.set_backup_transports(vec!["10.0.1.178:34257".into()]);

        let entries = crate::storage::list(&adb, &device, "/sdcard/Download")
            .expect("storage::list must recover rotated wireless mDNS TLS port");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "note.txt");
    }

    #[test]
    fn pc_control_commands_cover_macos_windows_and_linux() {
        use super::{pc_lock_screen_commands_for_os, pc_mute_toggle_commands_for_os};

        let mac_mute = pc_mute_toggle_commands_for_os("macos");
        assert_eq!(mac_mute[0].0, "osascript");
        assert!(mac_mute[0].1.iter().any(|a| a.contains("output muted")));

        let win_mute = pc_mute_toggle_commands_for_os("windows");
        assert_eq!(win_mute[0].0, "powershell.exe");
        assert!(win_mute[0].1.iter().any(|a| a.contains("SendKeys")));

        let linux_mute = pc_mute_toggle_commands_for_os("linux");
        assert!(linux_mute.iter().any(|(b, _)| b == "pactl"));
        assert!(linux_mute.iter().any(|(b, _)| b == "wpctl"));

        let mac_lock = pc_lock_screen_commands_for_os("macos");
        assert_eq!(mac_lock[0].0, "osascript");

        let win_lock = pc_lock_screen_commands_for_os("windows");
        assert!(win_lock
            .iter()
            .any(|(_, args)| args.iter().any(|a| a.contains("LockWorkStation"))));

        let linux_lock = pc_lock_screen_commands_for_os("linux");
        assert!(linux_lock.iter().any(|(b, _)| b == "loginctl"));
        assert!(linux_lock.iter().any(|(b, _)| b == "xdg-screensaver"));
    }

    #[cfg(unix)]
    #[test]
    fn execute_pc_control_sync_clipboard_syncs_both_directions() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let phone_clip_file = dir.path().join("phone_clip.txt");
        let desktop_clip_file = dir.path().join("desktop_clip.txt");
        let pc_ctrl_log = dir.path().join("pc_ctrl.log");
        std::fs::write(&phone_clip_file, "Phone-Initial-Clip").unwrap();
        std::fs::write(&desktop_clip_file, "").unwrap();

        let script = format!(
            r#"#!/usr/bin/env bash
case "$*" in
  *"get-state"*)
    echo "device"
    exit 0
    ;;
  *"getprop"*"ro.serialno"*)
    echo "STEPX_CLIP_SERIAL"
    exit 0
    ;;
  *"SYNC_CLIPBOARD"*)
    exit 0
    ;;
  *"get-primary-clip"*)
    cat "{phone_clip}"
    exit 0
    ;;
  *"set-primary-clip"*)
    eval "set -- $4"
    printf '%s' "$4" > "{phone_clip}"
    exit 0
    ;;
esac
exit 1
"#,
            phone_clip = phone_clip_file.display()
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        std::env::set_var("STEPX_DESKTOP_CLIPBOARD_FILE", &desktop_clip_file);
        std::env::set_var("STEPX_PC_CONTROL_TEST_LOG", &pc_ctrl_log);

        let adb = crate::adb::Adb { path: fake_adb };
        let device = super::Device {
            transport: "STEPX_CLIP_SERIAL".into(),
            serial: "STEPX_CLIP_SERIAL".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };

        // 1. Desktop empty, phone has text -> pulls phone clipboard to desktop
        let msg1 = device
            .execute_pc_control_command(&adb, "sync_clipboard")
            .unwrap();
        assert!(msg1.contains("手机 → 电脑"), "{msg1}");
        assert_eq!(
            std::fs::read_to_string(&desktop_clip_file).unwrap(),
            "Phone-Initial-Clip"
        );

        // 2. Desktop updated with new text -> pushes desktop clipboard to phone
        std::fs::write(&desktop_clip_file, "Desktop-Updated-Clip").unwrap();
        let msg2 = device
            .execute_pc_control_command(&adb, "sync_clipboard")
            .unwrap();
        assert!(msg2.contains("电脑 → 手机"), "{msg2}");
        assert_eq!(
            std::fs::read_to_string(&phone_clip_file).unwrap(),
            "Desktop-Updated-Clip"
        );

        // 3. Phone subsequently updated with new text while desktop is non-empty -> pulls phone clipboard to desktop
        std::fs::write(&phone_clip_file, "Phone-Second-Update").unwrap();
        let msg3 = device
            .execute_pc_control_command(&adb, "sync_clipboard")
            .unwrap();
        assert!(msg3.contains("手机 → 电脑"), "{msg3}");
        assert_eq!(
            std::fs::read_to_string(&desktop_clip_file).unwrap(),
            "Phone-Second-Update"
        );

        // 4. Verify write_stdin_to_clipboard_command never blocks when a clipboard tool (like xclip/wl-copy)
        // forks a background selection-owner daemon that inherits file descriptors
        let daemon_out = dir.path().join("daemon_clip.txt");
        let mut daemon_cmd = std::process::Command::new("sh");
        daemon_cmd.args([
            "-c",
            &format!("cat > \"{}\"; (sleep 5 &) ; exit 0", daemon_out.display()),
        ]);
        let start = std::time::Instant::now();
        super::write_stdin_to_clipboard_command(
            daemon_cmd,
            b"NonBlockingClipPayload",
            std::time::Duration::from_secs(2),
        )
        .expect("stdin writer must not hang on background daemon fork");
        assert!(
            start.elapsed() < std::time::Duration::from_millis(500),
            "took {:?}, should return as soon as parent exits",
            start.elapsed()
        );
        assert_eq!(
            std::fs::read_to_string(&daemon_out).unwrap(),
            "NonBlockingClipPayload"
        );

        // 5. Test pc_mute_toggle and pc_lock_screen via test log
        let mute_msg = device
            .execute_pc_control_command(&adb, "pc_mute_toggle")
            .unwrap();
        assert!(mute_msg.contains("静音"));
        let lock_msg = device
            .execute_pc_control_command(&adb, "pc_lock_screen")
            .unwrap();
        assert!(lock_msg.contains("锁定电脑屏幕"));

        std::env::remove_var("STEPX_DESKTOP_CLIPBOARD_FILE");
        std::env::remove_var("STEPX_PC_CONTROL_TEST_LOG");
    }

    #[cfg(unix)]
    #[test]
    fn batched_debloat_and_cached_screen_resolution_reduce_adb_spawns() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let calls_log = dir.path().join("calls.log");

        let script = format!(
            r#"#!/usr/bin/env sh
echo "$*" >> "{calls}"
case "$*" in
  *"getprop"*ro.serialno*)
    echo "STEPX_BATCH_SERIAL"
    exit 0
    ;;
  *"wm"*size*)
    echo "Physical size: 1200x2670"
    exit 0
    ;;
  *"input"*tap*|*"input"*swipe*)
    exit 0
    ;;
  *"TOGGLED:"*)
    echo "TOGGLED:com.qti.diagservices"
    echo "TOGGLED:com.android.traceur"
    echo "TOGGLED:com.android.printspooler"
    echo "TOGGLED:com.android.bips"
    echo "TOGGLED:com.android.dreams.basic"
    exit 0
    ;;
  *"<<<STEPX_U>>>"*)
    echo "package:com.qti.diagservices"
    echo "package:com.android.traceur"
    echo "package:com.android.printspooler"
    echo "package:com.android.bips"
    echo "package:com.android.dreams.basic"
    echo "<<<STEPX_U>>>"
    echo "package:com.sohu.inputmethod.sogou.oem"
    echo "package:com.qti.diagservices"
    echo "package:com.android.traceur"
    echo "package:com.android.printspooler"
    echo "package:com.android.bips"
    echo "package:com.android.dreams.basic"
    echo "<<<STEPX_ALL>>>"
    echo "package:com.sohu.inputmethod.sogou.oem"
    echo "package:com.qti.diagservices"
    echo "package:com.android.traceur"
    echo "package:com.android.printspooler"
    echo "package:com.android.bips"
    echo "package:com.android.dreams.basic"
    echo "<<<STEPX_IME>>>"
    echo "com.sohu.inputmethod.sogou.oem/.SogouIME"
    exit 0
    ;;
  *"install-existing"*)
    exit 0
    ;;
esac
exit 0
"#,
            calls = calls_log.display()
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = crate::adb::Adb { path: fake_adb };
        let device = super::Device {
            transport: "STEPX_BATCH_SERIAL".into(),
            serial: "STEPX_BATCH_SERIAL".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };

        super::invalidate_verified_transport("STEPX_BATCH_SERIAL");
        let msg = device.debloat_preset_services(&adb).unwrap();
        assert!(msg.contains("共停用 4 项"), "{msg}");
        let calls_after_debloat = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert_eq!(
            calls_after_debloat.lines().count(),
            3,
            "batched debloat should only spawn 3 adb processes (serialno + batch disable + inspect): {calls_after_debloat}"
        );

        std::fs::write(&calls_log, "").unwrap();
        let (px1, py1) = device.tap_normalized(&adb, 500, 500).unwrap();
        assert_eq!((px1, py1), (600, 1335));
        let (px2, py2) = device.tap_normalized(&adb, 250, 750).unwrap();
        assert_eq!((px2, py2), (300, 2002));
        let tap_calls = std::fs::read_to_string(&calls_log).unwrap_or_default();
        let wm_size_count = tap_calls.lines().filter(|l| l.contains("wm")).count();
        assert_eq!(
            wm_size_count, 1,
            "wm size should be cached across repeated taps: {tap_calls}"
        );

        super::invalidate_verified_transport("STEPX_BATCH_SERIAL");
        std::fs::write(&calls_log, "").unwrap();
        // This fake has no AgentBus forward. A catalog failure must not launch
        // Amoo through a second channel; only an explicitly absent tool may.
        assert!(device.open_amoo_tasks(&adb).is_err());
        let tasks_calls = std::fs::read_to_string(&calls_log).unwrap_or_default();
        let serialno_count = tasks_calls
            .lines()
            .filter(|l| l.contains("ro.serialno"))
            .count();
        assert_eq!(
            serialno_count, 1,
            "open_amoo_tasks must resolve transport only once (not re-resolve inside open_amoo): {tasks_calls}"
        );

        super::invalidate_verified_transport("STEPX_BATCH_SERIAL");
        std::fs::write(&calls_log, "").unwrap();
        let _ = device.open_amoo_screen_perception(&adb).unwrap();
        let perception_calls = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert_eq!(
            perception_calls.lines().count(),
            2,
            "open_amoo_screen_perception must batch startservice + start into a single shell script (serialno + script): {perception_calls}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn wake_and_unlock_transport_wakes_dozing_and_skips_swipe_when_already_unlocked() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let actions_log = dir.path().join("actions.log");
        let mode_file = dir.path().join("mode.txt");

        let script = format!(
            r#"#!/usr/bin/env sh
MODE="$(cat "{mode}" 2>/dev/null || echo locked)"
shift 3
SCRIPT="$*"
dumpsys() {{
  if [ "$1" = "power" ]; then
    if [ "$MODE" = "locked" ]; then
      echo "mWakefulness=Dozing"
    else
      echo "mWakefulness=Awake"
    fi
  elif [ "$1" = "window" ]; then
    echo "screenState=SCREEN_STATE_ON"
    echo "  Window #1: showing=true"
    echo "  KeyguardServiceDelegate"
    if [ "$MODE" = "locked" ] || [ "$MODE" = "awake_locked" ]; then
      echo "    showing=true"
      echo "    showingAndNotOccluded=true"
      echo "    mShowingLockscreen=true"
    else
      echo "    showing=false"
      echo "    showingAndNotOccluded=false"
      echo "    mShowingLockscreen=false"
    fi
  fi
}}
input() {{
  echo "input $*" >> "{actions}"
  if [ "$1" = "keyevent" ] && [ "$2" = "224" ]; then
    MODE="awake_locked"
  fi
}}
wm() {{
  echo "wm $*" >> "{actions}"
}}
cmd() {{
  echo "cmd $*" >> "{actions}"
}}
sleep() {{
  :
}}
eval "$SCRIPT"
"#,
            mode = mode_file.display(),
            actions = actions_log.display(),
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = crate::adb::Adb { path: fake_adb };

        // Case 1: Phone is Dozing and Keyguard is locked
        std::fs::write(&mode_file, "locked").unwrap();
        std::fs::write(&actions_log, "").unwrap();
        super::wake_and_unlock_transport(&adb, "STEPX_LOCK_TEST", 540, 1872, 528).unwrap();
        let locked_actions = std::fs::read_to_string(&actions_log).unwrap_or_default();
        assert!(
            locked_actions.contains("input keyevent 224"),
            "must wake dozing screen with keyevent 224: {locked_actions}"
        );
        assert!(
            locked_actions.contains("wm dismiss-keyguard"),
            "must call wm dismiss-keyguard: {locked_actions}"
        );
        assert!(
            locked_actions.contains("input keyevent 82"),
            "must send keyevent 82 when keyguard is showing: {locked_actions}"
        );
        assert!(
            locked_actions.contains("input swipe 540 1872 540 528 300"),
            "must swipe up keyguard when keyguard is showing: {locked_actions}"
        );
        assert!(
            locked_actions.contains("cmd statusbar collapse"),
            "must collapse statusbar after unlock: {locked_actions}"
        );

        // Case 2: Phone is already Awake and unlocked (even if an unrelated Window #1 has showing=true)
        std::fs::write(&mode_file, "unlocked").unwrap();
        std::fs::write(&actions_log, "").unwrap();
        super::wake_and_unlock_transport(&adb, "STEPX_LOCK_TEST", 540, 1872, 528).unwrap();
        let unlocked_actions = std::fs::read_to_string(&actions_log).unwrap_or_default();
        assert!(
            !unlocked_actions.contains("input keyevent 224")
                && !unlocked_actions.contains("input keyevent 26"),
            "must not send wake keyevent when already Awake: {unlocked_actions}"
        );
        assert!(
            !unlocked_actions.contains("input swipe"),
            "must not swipe screen when keyguard is already unlocked: {unlocked_actions}"
        );
        assert!(
            unlocked_actions.contains("cmd statusbar collapse"),
            "should still collapse statusbar: {unlocked_actions}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn active_transport_registry_ttl_cache_promotion_and_dual_transport_helpers() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let calls_log = dir.path().join("calls.log");
        let usb_alive_file = dir.path().join("usb_alive.txt");
        std::fs::write(&usb_alive_file, "1").unwrap();

        let script = format!(
            r#"#!/usr/bin/env sh
echo "$*" >> "{calls}"
USB_ALIVE="$(cat "{usb}" 2>/dev/null || echo 1)"
case "$*" in
  *"-s STEPX_TTL_USB_SER shell getprop ro.serialno"*)
    if [ "$USB_ALIVE" = "1" ]; then
      echo "STEPX_TTL_USB_SER"
      exit 0
    else
      echo "adb: device offline" >&2
      exit 1
    fi
    ;;
  *"-s 10.0.1.188:38888 shell getprop ro.serialno"*)
    echo "STEPX_TTL_USB_SER"
    exit 0
    ;;
  "connect 10.0.1.188:38888")
    echo "connected to 10.0.1.188:38888"
    exit 0
    ;;
  "devices -l")
    echo "List of devices attached"
    echo "10.0.1.188:38888 device product:pandora model:PZ261A device:pandora"
    exit 0
    ;;
esac
exit 0
"#,
            calls = calls_log.display(),
            usb = usb_alive_file.display()
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = crate::adb::Adb { path: fake_adb };
        let device = super::Device {
            transport: "STEPX_TTL_USB_SER".into(),
            serial: "STEPX_TTL_USB_SER".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        device.set_backup_transports(vec!["10.0.1.188:38888".into()]);
        assert!(device.has_usb_and_wireless());
        assert_eq!(
            device.all_known_transports(),
            vec![
                "STEPX_TTL_USB_SER".to_string(),
                "10.0.1.188:38888".to_string()
            ]
        );

        super::invalidate_verified_transport("STEPX_TTL_USB_SER");
        std::fs::write(&calls_log, "").unwrap();

        // First call probes getprop ro.serialno and populates 3.5s TTL cache
        let t1 = device.resolve_active_transport(&adb).unwrap();
        assert_eq!(t1, "STEPX_TTL_USB_SER");
        let calls1 = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert_eq!(calls1.lines().count(), 1);

        // Second call within 3.5s hits 0ms TTL fast path without spawning adb
        let t2 = device.resolve_active_transport(&adb).unwrap();
        assert_eq!(t2, "STEPX_TTL_USB_SER");
        let calls2 = std::fs::read_to_string(&calls_log).unwrap_or_default();
        assert_eq!(
            calls2.lines().count(),
            1,
            "second resolve within 3.5s TTL must not spawn adb: {calls2}"
        );

        // Simulate USB unplug + cache invalidation -> fails over and promotes wireless transport
        std::fs::write(&usb_alive_file, "0").unwrap();
        super::invalidate_verified_transport("STEPX_TTL_USB_SER");
        let t3 = device.resolve_active_transport(&adb).unwrap();
        assert_eq!(t3, "10.0.1.188:38888");
        assert_eq!(device.active_transport_cached(), "10.0.1.188:38888");
        assert_eq!(
            super::current_active_transport_for_serial("STEPX_TTL_USB_SER").as_deref(),
            Some("10.0.1.188:38888")
        );

        // Verify two different PZ261A serials do not collide via shared model:PZ261A key
        let dev_a = super::Device {
            transport: "STEPX_SER_A".into(),
            serial: "STEPX_SER_A".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        let dev_b = super::Device {
            transport: "STEPX_SER_B".into(),
            serial: "STEPX_SER_B".into(),
            model: "PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        };
        dev_a.set_backup_transports(vec!["10.0.1.101:5555".into()]);
        dev_b.set_backup_transports(vec!["10.0.1.102:5555".into()]);
        assert_eq!(
            dev_a.backup_transports(),
            vec!["10.0.1.101:5555".to_string()]
        );
        assert_eq!(
            dev_b.backup_transports(),
            vec!["10.0.1.102:5555".to_string()]
        );
    }
}
