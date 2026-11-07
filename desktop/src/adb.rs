use anyhow::{bail, Context, Result};
use std::{
    cell::Cell,
    collections::HashMap,
    ffi::{OsStr, OsString},
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, SocketAddrV4, TcpStream},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Arc, Condvar, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TransportKind {
    Usb,
    WirelessTcp,
    WirelessMdnsTls,
}

#[derive(Clone, Debug, Default)]
pub struct TransportTelemetry {
    pub last_rtt_ms: u64,
    pub avg_rtt_ms: u64,
    pub consecutive_failures: u32,
    pub total_commands: u64,
    pub total_retries: u64,
    pub total_failovers: u64,
}

pub fn classify_transport(serial_or_transport: &str) -> TransportKind {
    let trimmed = serial_or_transport.trim();
    if trimmed.starts_with("adb-") || trimmed.contains("._adb-tls-") {
        TransportKind::WirelessMdnsTls
    } else if trimmed.contains(':') {
        TransportKind::WirelessTcp
    } else {
        TransportKind::Usb
    }
}

pub fn is_wireless_transport(s: &str) -> bool {
    matches!(
        classify_transport(s),
        TransportKind::WirelessTcp | TransportKind::WirelessMdnsTls
    )
}

pub fn is_transient_transport_error(stderr: &str, stdout: &str, err_msg: &str) -> bool {
    let combined = format!("{stderr}\n{stdout}\n{err_msg}");
    let lower = combined.to_ascii_lowercase();
    lower.contains("device offline")
        || ((lower.contains("device") || lower.contains("emulator"))
            && lower.contains("not found")
            && !lower.contains("/system/bin/sh:"))
        || lower.contains("no devices/emulators found")
        || lower.contains("error: closed")
        || lower.contains("protocol fault")
        || lower.contains("connection reset by peer")
        || lower.contains("no route to host")
        || lower.contains("network is unreachable")
        || lower.contains("broken pipe")
        || lower.contains("cannot bind listener")
        || lower.contains("more than one device")
}

#[derive(Debug)]
pub struct ProcessTimeout;
impl std::fmt::Display for ProcessTimeout {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result { write!(f, "操作超时；请检查手机连接，然后重试。") }
}
impl std::error::Error for ProcessTimeout {}

pub fn is_data_transport_failure(error: &anyhow::Error) -> bool {
    error.downcast_ref::<ProcessTimeout>().is_some()
        || error.chain().any(|e| e.downcast_ref::<std::io::Error>().is_some_and(|e| matches!(e.kind(),
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::ConnectionReset | std::io::ErrorKind::ConnectionRefused
            | std::io::ErrorKind::ConnectionAborted | std::io::ErrorKind::NotConnected
            | std::io::ErrorKind::NetworkUnreachable | std::io::ErrorKind::HostUnreachable
            | std::io::ErrorKind::UnexpectedEof)))
        || is_transient_transport_error("", "", &format!("{error:#}"))
}

// A transport error does not prove that the phone did not execute a command.
// Only replay simple, known reads; arbitrary scripts, taps, sends and writes
// must return their uncertain result to the caller instead of executing twice.
fn can_replay_after_transport_error(args: &[&str]) -> bool {
    if args.len() < 3 || args[0] != "-s" {
        return false;
    }
    match args[2] {
        "pull" => true,
        "forward" => args.get(3).is_some_and(|arg| *arg != "tcp:0"),
        "exec-out" => args.get(3).is_some_and(|arg| *arg == "screencap"),
        "shell" => {
            let script = args[3..].join(" ");
            if script.chars().any(|c| {
                matches!(
                    c,
                    ';' | '|' | '&' | '>' | '<' | '$' | '`' | '\n' | '\r' | '\\'
                )
            }) {
                return false;
            }
            let words: Vec<_> = script
                .split_whitespace()
                .map(|word| word.trim_matches(['\'', '"']))
                .collect();
            match words.as_slice() {
                ["getprop", ..] | ["echo", ..] | ["screencap", "-p"] => true,
                ["settings", "get" | "list", ..] | ["content", "query", ..] => true,
                ["wm", "size" | "density"] => true,
                ["pm", "list", ..] | ["pm", "path", ..] => true,
                _ => false,
            }
        }
        _ => false,
    }
}

pub struct TransportCoordinator;

pub struct TransportSlotGuard {
    slot: Arc<(Mutex<usize>, Condvar)>,
}

impl Drop for TransportSlotGuard {
    fn drop(&mut self) {
        let (lock, cvar) = &*self.slot;
        let mut count = lock.lock().unwrap_or_else(|e| e.into_inner());
        *count = count.saturating_sub(1);
        cvar.notify_one();
    }
}

static TRANSPORT_SLOTS: OnceLock<Mutex<HashMap<String, Arc<(Mutex<usize>, Condvar)>>>> =
    OnceLock::new();
static TRANSPORT_TELEMETRY: OnceLock<Mutex<HashMap<String, TransportTelemetry>>> = OnceLock::new();

impl TransportCoordinator {
    pub const MAX_WIRELESS_CONCURRENCY: usize = 3;
    pub const MAX_USB_CONCURRENCY: usize = 6;
    const SLOT_WAIT_TIMEOUT: Duration = Duration::from_secs(12);

    pub fn max_concurrency_for(target: &str) -> usize {
        if is_wireless_transport(target) {
            Self::MAX_WIRELESS_CONCURRENCY
        } else {
            Self::MAX_USB_CONCURRENCY
        }
    }

    fn slot_for(target: &str) -> Arc<(Mutex<usize>, Condvar)> {
        let key = target.trim().to_string();
        let map = TRANSPORT_SLOTS.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        guard
            .entry(key)
            .or_insert_with(|| Arc::new((Mutex::new(0), Condvar::new())))
            .clone()
    }

    pub fn acquire_slot(target: &str) -> Result<TransportSlotGuard> {
        Self::acquire_slot_with_timeout(target, Self::SLOT_WAIT_TIMEOUT)
    }

    fn acquire_slot_with_timeout(target: &str, timeout: Duration) -> Result<TransportSlotGuard> {
        let limit = Self::max_concurrency_for(target);
        let slot = Self::slot_for(target);
        {
            let (lock, cvar) = &*slot;
            let mut in_flight = lock.lock().unwrap_or_else(|e| e.into_inner());
            let deadline = Instant::now() + timeout;
            while *in_flight >= limit {
                let now = Instant::now();
                if now >= deadline {
                    bail!("手机任务繁忙，等待连接通道超时；请稍后重试");
                }
                let remaining = deadline.saturating_duration_since(now);
                let (next_guard, _) = cvar
                    .wait_timeout(in_flight, remaining)
                    .unwrap_or_else(|e| e.into_inner());
                in_flight = next_guard;
            }
            *in_flight = in_flight.saturating_add(1);
        }
        Ok(TransportSlotGuard { slot })
    }

    pub fn active_in_flight(target: &str) -> usize {
        let slot = Self::slot_for(target);
        let val = *slot.0.lock().unwrap_or_else(|e| e.into_inner());
        val
    }

    pub fn record_command_success(target: &str, rtt_ms: u64) {
        let key = target.trim();
        if key.is_empty() {
            return;
        }
        let map = TRANSPORT_TELEMETRY.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        let entry = guard.entry(key.to_string()).or_default();
        entry.total_commands = entry.total_commands.saturating_add(1);
        entry.consecutive_failures = 0;
        entry.last_rtt_ms = rtt_ms;
        entry.avg_rtt_ms = if entry.total_commands == 1 || entry.avg_rtt_ms == 0 {
            rtt_ms
        } else {
            (entry.avg_rtt_ms.saturating_mul(7) + rtt_ms.saturating_mul(3)) / 10
        };
    }

    pub fn record_command_failure(target: &str) {
        let key = target.trim();
        if key.is_empty() {
            return;
        }
        let map = TRANSPORT_TELEMETRY.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        let entry = guard.entry(key.to_string()).or_default();
        entry.total_commands = entry.total_commands.saturating_add(1);
        entry.consecutive_failures = entry.consecutive_failures.saturating_add(1);
    }

    pub fn record_retry(target: &str) {
        let key = target.trim();
        if key.is_empty() {
            return;
        }
        let map = TRANSPORT_TELEMETRY.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        let entry = guard.entry(key.to_string()).or_default();
        entry.total_retries = entry.total_retries.saturating_add(1);
    }

    pub fn record_failover(target: &str) {
        let key = target.trim();
        if key.is_empty() {
            return;
        }
        let map = TRANSPORT_TELEMETRY.get_or_init(|| Mutex::new(HashMap::new()));
        let mut guard = map.lock().unwrap_or_else(|e| e.into_inner());
        let entry = guard.entry(key.to_string()).or_default();
        entry.total_failovers = entry.total_failovers.saturating_add(1);
    }

    pub fn telemetry(target: &str) -> TransportTelemetry {
        let key = target.trim();
        let map = TRANSPORT_TELEMETRY.get_or_init(|| Mutex::new(HashMap::new()));
        let guard = map.lock().unwrap_or_else(|e| e.into_inner());
        guard.get(key).cloned().unwrap_or_default()
    }
}

pub fn transport_telemetry(target: &str) -> TransportTelemetry {
    TransportCoordinator::telemetry(target)
}

pub fn record_transport_failover(target: &str) {
    TransportCoordinator::record_failover(target);
}

thread_local! {
    static IN_TRANSPORT_RECOVERY: Cell<bool> = const { Cell::new(false) };
}

pub(crate) struct TransportRecoveryGuard {
    prev: bool,
}

impl Drop for TransportRecoveryGuard {
    fn drop(&mut self) {
        IN_TRANSPORT_RECOVERY.with(|flag| flag.set(self.prev));
    }
}

pub(crate) fn enter_transport_recovery() -> TransportRecoveryGuard {
    let prev = IN_TRANSPORT_RECOVERY.with(|flag| {
        let old = flag.get();
        flag.set(true);
        old
    });
    TransportRecoveryGuard { prev }
}

fn in_transport_recovery() -> bool {
    IN_TRANSPORT_RECOVERY.with(|flag| flag.get())
}

#[derive(Clone, Debug)]
pub struct Adb {
    pub path: PathBuf,
}

static DISCOVER_CACHE: OnceLock<Mutex<Option<(Option<OsString>, PathBuf)>>> = OnceLock::new();
static ENSURED_ANDROID_HOME: OnceLock<Mutex<Option<OsString>>> = OnceLock::new();
static FORWARD_POOL: OnceLock<Mutex<HashMap<(PathBuf, String, String), u16>>> = OnceLock::new();

impl Adb {
    pub fn discover() -> Result<Self> {
        let stepx_adb = std::env::var_os("STEPX_ADB");
        let cache = DISCOVER_CACHE.get_or_init(|| Mutex::new(None));
        if let Ok(guard) = cache.lock() {
            if let Some((cached_env, cached_path)) = guard.as_ref() {
                if *cached_env == stepx_adb && cached_path.is_file() {
                    return Ok(Self {
                        path: cached_path.clone(),
                    });
                }
            }
        }
        let discovered = if let Some(path) = stepx_adb.clone() {
            Self { path: path.into() }
        } else if let Some(local) = bundled_tool("adb") {
            Self { path: local }
        } else {
            let mut sdk_path = None;
            for key in ["ANDROID_HOME", "ANDROID_SDK_ROOT"] {
                if Some(true) == std::env::var_os(key).as_ref().map(|d| !d.is_empty()) {
                    if let Some(dir) = std::env::var_os(key) {
                        let path = PathBuf::from(dir).join("platform-tools").join(exe("adb"));
                        if path.is_file() {
                            sdk_path = Some(path);
                            break;
                        }
                    }
                }
            }
            let path = sdk_path
                .or_else(|| find_executable("adb"))
                .or_else(|| platform_tool_candidate("adb"))
                .context(adb_missing_help())?;
            Self { path }
        };
        if let Ok(mut guard) = cache.lock() {
            *guard = Some((stepx_adb, discovered.path.clone()));
        }
        Ok(discovered)
    }

    fn command(&self, args: &[OsString]) -> Command {
        if let Some(home) = std::env::var_os("HOME").or_else(|| std::env::var_os("USERPROFILE")) {
            if !home.is_empty() {
                let cache = ENSURED_ANDROID_HOME.get_or_init(|| Mutex::new(None));
                let mut needs_create = true;
                if let Ok(guard) = cache.lock() {
                    if guard.as_ref() == Some(&home) {
                        needs_create = false;
                    }
                }
                if needs_create {
                    let _ = std::fs::create_dir_all(PathBuf::from(&home).join(".android"));
                    if let Ok(mut guard) = cache.lock() {
                        *guard = Some(home);
                    }
                }
            }
        }
        let mut command = Command::new(&self.path);
        command.args(args).env("ADB_MDNS_OPENSCREEN", "1");
        command
    }

    pub fn forward_tcp_cached(&self, serial: &str, remote_spec: &str) -> Result<(u16, bool)> {
        let key = (
            self.path.clone(),
            serial.trim().to_string(),
            remote_spec.trim().to_string(),
        );
        let pool = FORWARD_POOL.get_or_init(|| Mutex::new(HashMap::new()));
        if let Ok(guard) = pool.lock() {
            if let Some(&port) = guard.get(&key) {
                return Ok((port, true));
            }
        }
        let port_raw = self
            .run(&["-s", serial, "forward", "tcp:0", remote_spec])
            .with_context(|| format!("建立 ADB 端口转发失败 ({remote_spec})"))?;
        let port: u16 = port_raw
            .trim()
            .parse()
            .with_context(|| format!("解析 ADB 转发端口失败: {}", port_raw.trim()))?;
        if let Ok(mut guard) = pool.lock() {
            guard.insert(key, port);
        }
        Ok((port, false))
    }

    pub fn invalidate_forwarded_tcp(&self, serial: &str, remote_spec: &str, port: Option<u16>) {
        let key = (
            self.path.clone(),
            serial.trim().to_string(),
            remote_spec.trim().to_string(),
        );
        let pool = FORWARD_POOL.get_or_init(|| Mutex::new(HashMap::new()));
        let port_to_remove = match port {
            Some(target) => {
                if let Ok(mut guard) = pool.lock() {
                    if guard.get(&key) == Some(&target) {
                        guard.remove(&key);
                    }
                }
                Some(target)
            }
            None => pool.lock().ok().and_then(|mut guard| guard.remove(&key)),
        };
        if let Some(p) = port_to_remove {
            let _ = self.run_timeout(
                &["-s", serial, "forward", "--remove", &format!("tcp:{p}")],
                Duration::from_secs(2),
            );
        }
    }

    pub fn invalidate_forwards_for_transport(&self, serial: &str) {
        let trimmed = serial.trim();
        if trimmed.is_empty() {
            return;
        }
        let pool = FORWARD_POOL.get_or_init(|| Mutex::new(HashMap::new()));
        let ports = if let Ok(mut guard) = pool.lock() {
            let owned: Vec<_> = guard
                .iter()
                .filter(|((path, ser, _), _)| path == &self.path && ser == trimmed)
                .map(|(_, &port)| port)
                .collect();
            guard.retain(|(path, ser, _), _| !(path == &self.path && ser == trimmed));
            owned
        } else {
            vec![]
        };
        // ADB forwards are shared with debuggers and other applications. Never
        // remove their ports while recovering our own connection.
        for port in ports {
            let _ = run_process(
                self.command(&[
                    "-s".into(),
                    trimmed.into(),
                    "forward".into(),
                    "--remove".into(),
                    format!("tcp:{port}").into(),
                ]),
                None,
                Duration::from_millis(1500),
            );
        }
    }

    pub fn connect_forwarded_tcp(
        &self,
        serial: &str,
        remote_spec: &str,
        connect_timeout: Duration,
    ) -> Result<(TcpStream, u16, bool)> {
        let (port, was_cached) = self.forward_tcp_cached(serial, remote_spec)?;
        let addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port));
        match TcpStream::connect_timeout(&addr, connect_timeout) {
            Ok(stream) => {
                let _ = stream.set_nodelay(true);
                Ok((stream, port, was_cached))
            }
            Err(first_err) if was_cached => {
                self.invalidate_forwarded_tcp(serial, remote_spec, Some(port));
                let (fresh_port, _) = self.forward_tcp_cached(serial, remote_spec)?;
                let fresh_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, fresh_port));
                match TcpStream::connect_timeout(&fresh_addr, connect_timeout) {
                    Ok(stream) => {
                        let _ = stream.set_nodelay(true);
                        Ok((stream, fresh_port, false))
                    }
                    Err(err) => {
                        self.invalidate_forwarded_tcp(serial, remote_spec, Some(fresh_port));
                        Err(err).with_context(|| {
                            format!("连接本地转发端口 127.0.0.1:{fresh_port} ({remote_spec}) 失败 (前次缓存错误: {first_err})")
                        })
                    }
                }
            }
            Err(err) => {
                self.invalidate_forwarded_tcp(serial, remote_spec, Some(port));
                Err(err).with_context(|| {
                    format!("连接本地转发端口 127.0.0.1:{port} ({remote_spec}) 失败")
                })
            }
        }
    }

    pub fn probe_device_identity(&self, target: &str, expected: &str, timeout: Duration) -> bool {
        let _guard = enter_transport_recovery();
        self.run_timeout(&["-s", target, "shell", "getprop", "ro.serialno"], timeout)
            .is_ok_and(|got| !got.trim().is_empty() && got.trim() == expected)
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_timeout(args, Duration::from_secs(20))
    }

    pub fn run_output_with_input(
        &self,
        args: &[&str],
        input: Option<Vec<u8>>,
        timeout: Duration,
    ) -> Result<Output> {
        let target_serial = if args.len() >= 2 && args[0] == "-s" && !args[1].trim().is_empty() {
            Some(args[1].trim())
        } else {
            None
        };
        let is_retryable_subcmd = can_replay_after_transport_error(args);
        let _mutation = if !is_retryable_subcmd && !in_transport_recovery() {
            target_serial.map(crate::device::protect_transport_mutation).transpose()?
        } else { None };

        let (first_input, retry_input) = if is_retryable_subcmd && !in_transport_recovery() {
            (input.clone(), input)
        } else {
            (input, None)
        };

        let start = Instant::now();
        let first_res = {
            let _slot = target_serial
                .map(|target| {
                    TransportCoordinator::acquire_slot_with_timeout(
                        target,
                        timeout.min(TransportCoordinator::SLOT_WAIT_TIMEOUT),
                    )
                })
                .transpose()?;
            let remaining = timeout.saturating_sub(start.elapsed());
            if remaining.is_zero() {
                bail!("手机请求等待超时，请稍后重试");
            }
            run_process(
                self.command(&args.iter().map(OsString::from).collect::<Vec<_>>()),
                first_input,
                remaining,
            )
        };
        let rtt_ms = start.elapsed().as_millis() as u64;

        let transient_failure = match &first_res {
            Ok(out) => {
                if out.status.success() {
                    if let Some(target) = target_serial {
                        TransportCoordinator::record_command_success(target, rtt_ms);
                    }
                    false
                } else {
                    if let Some(target) = target_serial {
                        TransportCoordinator::record_command_failure(target);
                    }
                    let stderr = String::from_utf8_lossy(&out.stderr);
                    let stdout = String::from_utf8_lossy(&out.stdout);
                    is_transient_transport_error(&stderr, &stdout, "")
                }
            }
            Err(err) => {
                if let Some(target) = target_serial {
                    TransportCoordinator::record_command_failure(target);
                }
                is_transient_transport_error("", "", &err.to_string())
            }
        };

        if transient_failure && is_retryable_subcmd && !in_transport_recovery() {
            if let Some(target) = target_serial {
                let _recovery_guard = enter_transport_recovery();
                TransportCoordinator::record_retry(target);
                // A failed stream does not imply the entire transport is dead.
                // Preserve live MCP/sync/mirroring streams; share one bounded recovery.
                let remaining = timeout.saturating_sub(start.elapsed());
                let Some(resolved_target) = crate::device::recover_transport(
                    self,
                    target,
                    remaining.min(Duration::from_secs(6)),
                ) else {
                    return first_res;
                };
                if timeout.saturating_sub(start.elapsed()).is_zero() {
                    return first_res;
                }

                let mut retry_args: Vec<OsString> = Vec::with_capacity(args.len());
                retry_args.push(OsString::from("-s"));
                retry_args.push(OsString::from(&resolved_target));
                for arg in &args[2..] {
                    retry_args.push(OsString::from(arg));
                }

                let retry_start = Instant::now();
                let retry_res = {
                    let remaining = timeout.saturating_sub(start.elapsed());
                    let _slot = TransportCoordinator::acquire_slot_with_timeout(
                        &resolved_target,
                        remaining.min(TransportCoordinator::SLOT_WAIT_TIMEOUT),
                    )?;
                    let remaining = timeout.saturating_sub(start.elapsed());
                    if remaining.is_zero() {
                        return first_res;
                    }
                    run_process(self.command(&retry_args), retry_input, remaining)
                };
                let retry_rtt_ms = retry_start.elapsed().as_millis() as u64;

                match &retry_res {
                    Ok(out) if out.status.success() => {
                        TransportCoordinator::record_command_success(
                            &resolved_target,
                            retry_rtt_ms,
                        );
                        if resolved_target != target {
                            record_transport_failover(target);
                        }
                    }
                    _ => {
                        TransportCoordinator::record_command_failure(&resolved_target);
                    }
                }
                return retry_res;
            }
        }

        first_res
    }

    pub fn run_timeout(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let output = self.run_output_with_input(args, None, timeout)?;
        checked_text(output)
    }

    pub fn pair(&self, endpoint: &str, code: &str) -> Result<String> {
        // Feed the secret via stdin: it never appears in process arguments or logs.
        let output = self.run_output_with_input(
            &["pair", endpoint],
            Some(format!("{code}\n").into_bytes()),
            Duration::from_secs(25),
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout)
            .replace("\r\n", "\n")
            .replace('\r', "\n");
        if output.status.success() && stdout.contains("Successfully paired") {
            Ok("配对成功。正在查找连接端口…".into())
        } else {
            // Do not echo pairing output: vendor ADBs may echo the password.
            bail!("配对失败：请检查配对端口、配对码是否已过期，以及手机和电脑是否在同一网络。")
        }
    }

    pub fn shell(&self, serial: &str, arguments: &[&str]) -> Result<String> {
        let command = arguments
            .iter()
            .map(|s| shell_quote(s))
            .collect::<Vec<_>>()
            .join(" ");
        self.run(&["-s", serial, "shell", &command])
    }

    pub fn script(&self, serial: &str, script: &str) -> Result<String> {
        self.run(&["-s", serial, "shell", script])
    }

    pub fn screenshot(&self, serial: &str) -> Result<Vec<u8>> {
        let output = self.run_output_with_input(
            &["-s", serial, "exec-out", "screencap", "-p"],
            None,
            Duration::from_secs(20),
        )?;
        if !output.status.success() || !output.stdout.starts_with(b"\x89PNG\r\n\x1a\n") {
            bail!("无法获取屏幕。请解锁手机并检查调试授权；受保护画面可能不允许截屏。")
        }
        Ok(output.stdout)
    }

    pub fn transfer(&self, serial: &str, verb: &str, source: &Path, target: &Path) -> Result<()> {
        if let (Some(src_str), Some(dst_str)) = (source.to_str(), target.to_str()) {
            let output = self.run_output_with_input(
                &["-s", serial, verb, src_str, dst_str],
                None,
                Duration::from_secs(600),
            )?;
            checked_text(output)?;
            return Ok(());
        }
        let args = vec![
            "-s".into(),
            serial.into(),
            verb.into(),
            source.as_os_str().into(),
            target.as_os_str().into(),
        ];
        let _slot = TransportCoordinator::acquire_slot(serial)?;
        checked_text(run_process(
            self.command(&args),
            None,
            Duration::from_secs(600),
        )?)?;
        Ok(())
    }
}

pub fn shell_quote(value: &str) -> String {
    format!("'{}'", value.replace('\'', "'\\''"))
}

pub fn checked_text(output: Output) -> Result<String> {
    let text = String::from_utf8(output.stdout)
        .context("ADB 返回了非 UTF-8 文本")?
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    let err = String::from_utf8_lossy(&output.stderr)
        .replace("\r\n", "\n")
        .replace('\r', "\n");
    if !output.status.success()
        || [
            "SecurityException",
            "Permission Denial",
            "Permission denied",
            "Error:",
            "Error while accessing",
            "failed to connect",
            "cannot connect",
        ]
        .iter()
        .any(|needle| {
            text.lines()
                .any(|line| line.trim_start().starts_with(needle))
        })
        || text
            .lines()
            .any(|line| line.starts_with("java.lang.SecurityException"))
    {
        bail!("{}{}", text.trim(), err.trim());
    }
    Ok(text)
}

pub fn run_process(
    mut command: Command,
    input: Option<Vec<u8>>,
    timeout: Duration,
) -> Result<Output> {
    let started = Instant::now();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        command.process_group(0);
    }
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }
    let program = command.get_program().to_owned();
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut attempts = 0u8;
    let mut child = loop {
        match command.spawn() {
            Ok(child) => break child,
            #[cfg(unix)]
            Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) && attempts < 20 => {
                attempts += 1;
                thread::sleep(Duration::from_millis(2));
            }
            Err(e) => {
                return Err(anyhow::anyhow!(
                    "无法启动 {}：{}。请完整解压安装包，并检查安全软件是否隔离了组件。",
                    Path::new(&program)
                        .file_name()
                        .unwrap_or(&program)
                        .to_string_lossy(),
                    e
                ));
            }
        }
    };
    let mut stdout = child.stdout.take().unwrap();
    let mut stderr = child.stderr.take().unwrap();
    let waiting_thread = thread::current();
    let out = thread::spawn(move || {
        let res = read_bounded(&mut stdout);
        waiting_thread.unpark();
        res
    });
    let err = thread::spawn(move || read_bounded(&mut stderr));
    let stdin = child.stdin.take();
    let writer = thread::spawn(move || -> std::io::Result<()> {
        if let (Some(mut stdin), Some(input)) = (stdin, input) {
            stdin.write_all(&input)?;
        }
        Ok(())
    });
    const BACKOFF_MS: [u64; 6] = [1, 2, 4, 8, 15, 25];
    let mut poll_step = 0usize;
    let mut status = None;
    loop {
        if status.is_none() {
            status = child.try_wait()?;
        }
        if status.is_some() && out.is_finished() && err.is_finished() && writer.is_finished() {
            break;
        }
        // Include stdin writes and inherited output pipes in the same deadline.
        // A child that never reads input or leaves a writer descendant behind
        // must not pin a background worker forever.
        if started.elapsed() >= timeout {
            #[cfg(unix)]
            unsafe {
                libc::kill(-(child.id() as i32), libc::SIGKILL);
            }
            let _ = child.kill();
            let _ = child.wait();
            return Err(ProcessTimeout.into());
        }
        let wait_ms = BACKOFF_MS[poll_step.min(BACKOFF_MS.len() - 1)];
        poll_step = poll_step.saturating_add(1);
        thread::park_timeout(Duration::from_millis(wait_ms));
    }
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("读取工具输出失败"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("读取工具错误失败"))??;
    let input_result = writer
        .join()
        .map_err(|_| anyhow::anyhow!("写入工具输入失败"))?;
    let status = status.context("工具未返回退出状态")?;
    if status.success() {
        input_result.context("工具未完整接收输入")?;
    }
    Ok(Output {
        status,
        stdout,
        stderr,
    })
}

fn read_bounded(reader: &mut impl Read) -> Result<Vec<u8>> {
    let mut result = Vec::new();
    let mut overflow = false;
    let mut buffer = [0; 8192];
    loop {
        let n = reader.read(&mut buffer)?;
        if n == 0 {
            break;
        }
        if result.len() + n <= 32 * 1024 * 1024 {
            result.extend_from_slice(&buffer[..n]);
        } else {
            overflow = true;
        }
    }
    if overflow {
        bail!("工具输出超过 32 MiB，请缩小查询范围");
    }
    Ok(result)
}

pub fn find_executable(name: &str) -> Option<PathBuf> {
    std::env::var_os("PATH").and_then(|paths| {
        std::env::split_paths(&paths)
            .map(|p| p.join(exe(name)))
            .find(|p| p.is_file())
    })
}

pub fn bundled_tool(name: &str) -> Option<PathBuf> {
    let binary = exe(name);
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let installed = exe_dir
        .as_ref()
        .map(|p| p.join("tools/bin").join(&binary))
        .filter(|p| p.is_file());
    let macos_bundle = exe_dir
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.join("Resources/tools/bin").join(&binary))
        .filter(|p| p.is_file());
    let resource_root = crate::resources::root();
    installed
        .or(macos_bundle)
        .or_else(|| {
            let path = resource_root.join("tools/bin").join(&binary);
            path.is_file().then_some(path)
        })
        .or_else(|| {
            let path = resource_root.join(".tools/bin").join(&binary);
            path.is_file().then_some(path)
        })
        .or_else(|| {
            let path = Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../.tools/bin")
                .join(&binary);
            path.is_file().then_some(path)
        })
}

pub fn platform_tool_candidate(name: &str) -> Option<PathBuf> {
    let binary = exe(name);
    let home = crate::resources::user_home().map(PathBuf::from);
    let local_appdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let mut candidates = Vec::new();
    if let Some(home) = &home {
        candidates.push(
            home.join(".local/opt/stepx-companion/tools/bin")
                .join(&binary),
        );
        candidates.push(
            home.join("Library/Android/sdk/platform-tools")
                .join(&binary),
        );
        candidates.push(home.join("Android/Sdk/platform-tools").join(&binary));
        candidates.push(
            home.join("AppData/Local/Android/Sdk/platform-tools")
                .join(&binary),
        );
        candidates.push(home.join("scoop/shims").join(&binary));
    }
    if let Some(local) = &local_appdata {
        candidates.push(local.join("Android/Sdk/platform-tools").join(&binary));
        candidates.push(local.join("Microsoft/WinGet/Links").join(&binary));
    }
    for dir in [
        "/opt/homebrew/bin",
        "/usr/local/bin",
        "/usr/bin",
        "/snap/bin",
        "C:\\ProgramData\\chocolatey\\bin",
    ] {
        candidates.push(Path::new(dir).join(&binary));
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn adb_missing_help() -> &'static str {
    "缺少内置 ADB。请重新下载并完整解压 StepX Companion 安装包；不要只移动 EXE 或 .app 内的单个文件。无需安装 Android Studio。"
}

pub fn exe(name: &str) -> OsString {
    if cfg!(windows) {
        OsString::from(format!("{name}.exe"))
    } else {
        OsStr::new(name).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::TcpListener;

    #[cfg(unix)]
    fn fake_adb(body: &str) -> (tempfile::TempDir, Adb, std::path::PathBuf) {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let calls = dir.path().join("calls");
        let path = dir.path().join("adb");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\n{body}\n",
                calls.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        (dir, Adb { path }, calls)
    }

    #[test]
    fn app_errors_do_not_trigger_transport_recovery() {
        assert!(is_data_transport_failure(&anyhow::Error::new(ProcessTimeout).context("读取资料")));
        assert!(is_data_transport_failure(&std::io::Error::from(std::io::ErrorKind::TimedOut).into()));
        assert!(!is_data_transport_failure(&anyhow::anyhow!("手机未授予通知权限")));
        assert!(!is_transient_transport_error(
            "/system/bin/sh: missing-tool: not found",
            "",
            ""
        ));
        assert!(!is_transient_transport_error("file not found", "", ""));
        assert!(is_transient_transport_error(
            "adb: device 'old' not found",
            "",
            ""
        ));
        assert!(is_transient_transport_error("error: closed", "", ""));
    }

    #[test]
    fn wireless_route_loss_is_a_data_failure_without_becoming_a_write_retry() {
        for kind in [
            std::io::ErrorKind::NetworkUnreachable,
            std::io::ErrorKind::HostUnreachable,
            std::io::ErrorKind::ConnectionAborted,
            std::io::ErrorKind::NotConnected,
        ] {
            let error = anyhow::Error::from(std::io::Error::from(kind)).context("读取手机资料");
            assert!(is_data_transport_failure(&error), "{kind:?}");
        }
        for error in ["failed to connect: No route to host", "connect: Network is unreachable"] {
            assert!(is_data_transport_failure(&anyhow::anyhow!("{error}")));
        }
        assert!(!can_replay_after_transport_error(&["-s", "192.0.2.165:5555", "shell", "input", "tap", "1", "2"]));
    }

    #[cfg(unix)]
    #[test]
    fn closed_read_retries_without_tearing_down_live_streams() {
        let (dir, adb, calls) = fake_adb(
            r#"
case "$*" in
  "devices -l") printf 'List of devices attached\nREAD_CLOSED_UNIT device\n'; exit 0;;
  "-s READ_CLOSED_UNIT shell getprop ro.serialno") echo READ_CLOSED_UNIT; exit 0;;
esac
marker="$(dirname "$0")/failed"
if [ ! -e "$marker" ]; then touch "$marker"; echo 'error: closed' >&2; exit 1; fi
echo 'ok'
"#,
        );
        assert_eq!(
            adb.run_timeout(
                &[
                    "-s",
                    "READ_CLOSED_UNIT",
                    "shell",
                    "getprop",
                    "ro.product.model"
                ],
                Duration::from_secs(2)
            )
            .unwrap()
            .trim(),
            "ok"
        );
        let log = std::fs::read_to_string(calls).unwrap();
        assert_eq!(
            log.lines()
                .filter(|l| l.contains("ro.product.model"))
                .count(),
            2
        );
        assert!(
            !log.contains("disconnect") && !log.contains("reconnect") && !log.contains("forward")
        );
        drop(dir);
    }

    #[cfg(unix)]
    #[test]
    fn concurrent_recovery_joins_one_probe() {
        let (_dir, adb, calls) = fake_adb(
            r#"
case "$*" in
  "devices -l") printf 'List of devices attached\nRECOVERY_SHARED_UNIT device\n'; exit 0;;
  "-s RECOVERY_SHARED_UNIT shell getprop ro.serialno") sleep 0.2; echo RECOVERY_SHARED_UNIT; exit 0;;
esac
exit 1
"#,
        );
        let barrier = std::sync::Arc::new(std::sync::Barrier::new(6));
        std::thread::scope(|scope| {
            let handles: Vec<_> = (0..6)
                .map(|_| {
                    let adb = &adb;
                    let gate = barrier.clone();
                    scope.spawn(move || {
                        gate.wait();
                        crate::device::recover_transport(
                            adb,
                            "RECOVERY_SHARED_UNIT",
                            Duration::from_secs(2),
                        )
                    })
                })
                .collect();
            for h in handles {
                assert_eq!(h.join().unwrap().as_deref(), Some("RECOVERY_SHARED_UNIT"));
            }
        });
        let log = std::fs::read_to_string(calls).unwrap();
        assert_eq!(
            log.lines().filter(|l| l.contains("ro.serialno")).count(),
            1,
            "{log}"
        );
    }

    #[cfg(unix)]
    #[test]
    fn recovery_respects_deadline_and_does_not_select_unknown_peer() {
        let (_dir, adb, calls) = fake_adb(
            r#"
case "$*" in
  "devices -l") printf 'List of devices attached\n192.0.2.201:5555 device\n'; exit 0;;
  "-s 192.0.2.201:5555 shell getprop ro.serialno") echo OTHER_PHONE; exit 0;;
  *) echo 'error: closed' >&2; exit 1;;
esac
"#,
        );
        assert!(
            crate::device::recover_transport(&adb, "192.0.2.200:5555", Duration::from_secs(1))
                .is_none()
        );
        assert!(!std::fs::read_to_string(calls)
            .unwrap()
            .contains("-s 192.0.2.201"));
        let (_dir, adb, _calls) = fake_adb("sleep 5");
        let start = Instant::now();
        assert!(crate::device::recover_transport(
            &adb,
            "DEADLINE_UNIT",
            Duration::from_millis(100)
        )
        .is_none());
        assert!(start.elapsed() < Duration::from_millis(600));
    }

    #[test]
    fn transport_retry_is_limited_to_known_reads() {
        for args in [
            vec!["-s", "phone", "shell", "'getprop' 'ro.serialno'"],
            vec!["-s", "phone", "shell", "echo HELLO_FAILOVER"],
            vec![
                "-s",
                "phone",
                "shell",
                "settings get system screen_brightness",
            ],
            vec!["-s", "phone", "exec-out", "screencap", "-p"],
            vec!["-s", "phone", "pull", "/sdcard/file", "local"],
        ] {
            assert!(can_replay_after_transport_error(&args), "{args:?}");
        }
        for args in [
            vec!["-s", "phone", "shell", "input tap 20 30"],
            vec![
                "-s",
                "phone",
                "shell",
                "content call --uri content://sms --method send",
            ],
            vec!["-s", "phone", "shell", "echo ok; input tap 20 30"],
            vec!["-s", "phone", "shell", "settings put system value 1"],
            vec!["-s", "phone", "shell", "echo $(input keyevent 26)"],
            vec!["-s", "phone", "exec-out", "sh", "-c", "send"],
            vec!["-s", "phone", "push", "local", "/sdcard/file"],
            vec!["-s", "phone", "forward", "tcp:0", "tcp:1234"],
        ] {
            assert!(!can_replay_after_transport_error(&args), "{args:?}");
        }
    }

    #[test]
    fn exhausted_transport_slots_fail_without_exceeding_the_limit_and_recover_after_release() {
        for target in ["192.0.2.239:59991", "slot-timeout-usb-unit"] {
            let limit = TransportCoordinator::max_concurrency_for(target);
            let mut guards: Vec<_> = (0..limit)
                .map(|_| TransportCoordinator::acquire_slot(target).unwrap())
                .collect();
            let started = Instant::now();
            let result =
                TransportCoordinator::acquire_slot_with_timeout(target, Duration::from_millis(20));
            assert!(
                result.is_err(),
                "A timed-out waiter must not become an extra in-flight command"
            );
            assert!(started.elapsed() < Duration::from_secs(1));
            assert_eq!(TransportCoordinator::active_in_flight(target), limit);
            drop(guards.pop());
            let next =
                TransportCoordinator::acquire_slot_with_timeout(target, Duration::from_millis(20))
                    .unwrap();
            assert_eq!(TransportCoordinator::active_in_flight(target), limit);
            drop(next);
            drop(guards);
            assert_eq!(TransportCoordinator::active_in_flight(target), 0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn uncertain_mutation_is_not_executed_twice_and_other_adb_forwards_survive() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let calls = dir.path().join("calls");
        let path = dir.path().join("adb");
        std::fs::write(
            &path,
            format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\necho 'adb: error: closed' >&2\nexit 1\n",
                calls.display()
            ),
        )
        .unwrap();
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755)).unwrap();
        let adb = Adb { path };
        assert!(adb
            .script("phone-mutation-unit", "input tap 20 30")
            .is_err());
        assert_eq!(std::fs::read_to_string(&calls).unwrap().lines().count(), 1);
        // A transient failure must not call --remove-all, which also destroys
        // unrelated debugger/scrcpy forwards managed by other clients.
        let key = (
            adb.path.clone(),
            "phone-mutation-unit".into(),
            "localabstract:ours".into(),
        );
        FORWARD_POOL
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap()
            .insert(key, 32123);
        adb.invalidate_forwards_for_transport("phone-mutation-unit");
        let log = std::fs::read_to_string(&calls).unwrap();
        assert!(log.contains("forward --remove tcp:32123"));
        assert!(!log.contains("--remove-all"));
        adb.invalidate_forwards_for_transport("phone-mutation-unit");
        assert_eq!(
            std::fs::read_to_string(&calls).unwrap(),
            log,
            "No owned ports left: do not touch any other forward"
        );
    }

    #[cfg(unix)]
    #[test]
    fn process_timeout_covers_blocked_stdin_and_descendant_output_pipes() {
        for (script, input) in [
            ("sleep 3", Some(vec![b'x'; 2 * 1024 * 1024])),
            ("sleep 3 &", None),
        ] {
            let mut command = Command::new("sh");
            command.args(["-c", script]);
            let start = Instant::now();
            let error = run_process(command, input, Duration::from_millis(100)).unwrap_err();
            assert!(error.to_string().contains("超时"));
            assert!(
                start.elapsed() < Duration::from_secs(2),
                "A blocked pipe escaped the process deadline: {script}"
            );
        }
        let bytes = vec![b'a'; 1024 * 1024];
        let output = run_process(
            Command::new("cat"),
            Some(bytes.clone()),
            Duration::from_secs(3),
        )
        .unwrap();
        assert!(output.status.success());
        assert_eq!(output.stdout, bytes);
    }

    #[cfg(unix)]
    #[test]
    fn forward_pool_reuses_active_port_and_recovers_when_stale() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().expect("tempdir");
        let adb_path = dir.path().join("adb");
        let calls_path = dir.path().join("calls.log");
        let port_path = dir.path().join("port.txt");

        let listener1 = TcpListener::bind("127.0.0.1:0").expect("bind listener1");
        let port1 = listener1.local_addr().unwrap().port();
        std::fs::write(&port_path, format!("{port1}\n")).expect("write port1");

        let script = format!(
            "#!/usr/bin/env sh\necho \"$*\" >> '{}'\ncase \"$*\" in\n  *forward\\ tcp:0*)\n    cat '{}'\n    exit 0\n    ;;\n  *forward\\ --remove*)\n    exit 0\n    ;;\nesac\nexit 0\n",
            calls_path.display(),
            port_path.display()
        );
        std::fs::write(&adb_path, script).expect("write fake adb");
        let mut perms = std::fs::metadata(&adb_path).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&adb_path, perms).unwrap();

        let adb = Adb { path: adb_path };
        let test_serial = "serial-adb-forward-pool-unit";
        let test_remote = "localabstract:agentbus-unit";

        let t0 = Instant::now();
        let (stream1, p1, cached1) = adb
            .connect_forwarded_tcp(test_serial, test_remote, Duration::from_secs(1))
            .expect("first connect");
        let cold_us = t0.elapsed().as_micros();
        assert_eq!(p1, port1);
        assert!(!cached1);
        drop(stream1);
        let _ = listener1.accept();

        let t1 = Instant::now();
        let (stream2, p2, cached2) = adb
            .connect_forwarded_tcp(test_serial, test_remote, Duration::from_secs(1))
            .expect("second cached connect");
        let warm_us = t1.elapsed().as_micros();
        assert_eq!(p2, port1);
        assert!(cached2);
        // Scheduler delays can invert wall-clock samples under parallel CI load.
        // The cached flag and exact forward-command count below prove reuse;
        // latency belongs in the repeatable benchmark, not a unit assertion.
        eprintln!("forward-pool diagnostic: cold={cold_us}us warm={warm_us}us");
        drop(stream2);
        let _ = listener1.accept();

        let calls = std::fs::read_to_string(&calls_path).unwrap_or_default();
        assert_eq!(
            calls.lines().count(),
            1,
            "second connect should reuse cached forward without spawning adb: {calls}"
        );

        // Bind listener2 before dropping listener1 so the OS ephemeral port allocator
        // can never reassign port1 to listener2. Then close listener1 and wait for any
        // concurrent fork() child in the test runner to finish execve() and release
        // inherited FD_CLOEXEC socket references.
        let listener2 = TcpListener::bind("127.0.0.1:0").expect("bind listener2");
        let port2 = listener2.local_addr().unwrap().port();
        assert_ne!(port1, port2);
        std::fs::write(&port_path, format!("{port2}\n")).expect("write port2");

        drop(listener1);
        let stale_addr = SocketAddr::V4(SocketAddrV4::new(Ipv4Addr::LOCALHOST, port1));
        for _ in 0..250 {
            if TcpStream::connect_timeout(&stale_addr, Duration::from_millis(10)).is_err() {
                break;
            }
            thread::sleep(Duration::from_millis(2));
        }

        let (_stream3, p3, cached3) = adb
            .connect_forwarded_tcp(test_serial, test_remote, Duration::from_secs(1))
            .expect("stale recovery connect");
        assert_eq!(p3, port2);
        assert!(!cached3);
        let calls_after = std::fs::read_to_string(&calls_path).unwrap_or_default();
        assert!(
            calls_after.contains(&format!("forward --remove tcp:{port1}")),
            "stale port should be removed via adb forward --remove: {calls_after}"
        );
        // Late invalidation of old port1 from another worker must not evict fresh port2
        adb.invalidate_forwarded_tcp(test_serial, test_remote, Some(port1));
        let (p4, cached4) = adb
            .forward_tcp_cached(test_serial, test_remote)
            .expect("cached port2 must remain after late port1 invalidation");
        assert_eq!(p4, port2);
        assert!(cached4);
        adb.invalidate_forwarded_tcp(test_serial, test_remote, None);
    }

    #[test]
    fn live_phone_workstream1_latency_benchmarks() {
        let Ok(discovered_adb) = Adb::discover() else {
            return;
        };
        let Ok(device) = crate::device::Device::inspect(&discovered_adb, "STEPXTEST8DB89E4B8927")
        else {
            return;
        };
        let Ok(transport) = device.resolve_active_transport(&discovered_adb) else {
            return;
        };

        // Use a dedicated temp symlink to the real ADB binary so our (adb.path, transport, spec)
        // keys in FORWARD_POOL and SCREEN_RES_CACHE never collide with or evict entries used by
        // concurrently running live-device tests (e.g., home_hub / bridge).
        let dir = tempfile::tempdir().expect("tempdir for isolated adb link");
        let isolated_adb_path = dir.path().join(exe("adb"));
        #[cfg(unix)]
        let adb = if std::os::unix::fs::symlink(&discovered_adb.path, &isolated_adb_path).is_ok() {
            Adb {
                path: isolated_adb_path,
            }
        } else {
            discovered_adb.clone()
        };
        #[cfg(not(unix))]
        let adb = discovered_adb.clone();

        // 1. Port-forward pool reuse vs cold spawn + remove
        adb.invalidate_forwarded_tcp(&transport, "localabstract:agentbus", None);
        let t_cold = Instant::now();
        if let Ok((stream, port, cached)) =
            adb.connect_forwarded_tcp(&transport, "localabstract:agentbus", Duration::from_secs(2))
        {
            let cold_us = t_cold.elapsed().as_micros();
            assert!(!cached);
            drop(stream);

            let t_warm = Instant::now();
            let (stream2, port2, cached2) = adb
                .connect_forwarded_tcp(&transport, "localabstract:agentbus", Duration::from_secs(2))
                .expect("warm forward connect");
            let warm_us = t_warm.elapsed().as_micros();
            assert_eq!(port, port2);
            assert!(cached2);
            drop(stream2);
            adb.invalidate_forwarded_tcp(&transport, "localabstract:agentbus", Some(port2));
            eprintln!(
                "[Workstream1 Bench] ADB forward connect: cold={} us, warm_pooled={} us",
                cold_us, warm_us
            );
        }

        // 2. amoo_context TTL cache vs uncached call_bridge (use a dedicated cache key via isolated serial)
        let mut bench_ctx_device = device.clone();
        bench_ctx_device.transport = transport.clone();
        bench_ctx_device.serial = format!("{}-ws1-bench", device.serial);
        crate::amoo::invalidate_amoo_context_cache(&bench_ctx_device);
        let t_ctx_cold = Instant::now();
        if crate::amoo::cached_amoo_context(&adb, &bench_ctx_device).is_ok() {
            let ctx_cold_us = t_ctx_cold.elapsed().as_micros();
            let t_ctx_warm = Instant::now();
            let _ = crate::amoo::cached_amoo_context(&adb, &bench_ctx_device)
                .expect("warm amoo_context");
            let ctx_warm_us = t_ctx_warm.elapsed().as_micros();
            crate::amoo::invalidate_amoo_context_cache(&bench_ctx_device);
            eprintln!(
                "[Workstream1 Bench] amoo_context: cold_bridge={} us, warm_ttl_cache={} us",
                ctx_cold_us, ctx_warm_us
            );
            assert!(ctx_warm_us < ctx_cold_us);
        }

        // 3. screen_resolution_on_transport cache vs cold wm size + serialno
        let t_res_cold = Instant::now();
        let raw_wm = adb.shell(&transport, &["wm", "size"]).unwrap_or_default();
        let _ = crate::device::parse_wm_size(&raw_wm);
        let _ = device.resolve_active_transport(&adb);
        let res_unoptimized_us = t_res_cold.elapsed().as_micros();

        let _ = device.screen_resolution_on_transport(&adb, &transport);
        let t_res_warm = Instant::now();
        let dims = device.screen_resolution_on_transport(&adb, &transport);
        let res_warm_us = t_res_warm.elapsed().as_micros();
        assert!(dims.0 > 0 && dims.1 > 0);
        eprintln!(
            "[Workstream1 Bench] screen_resolution: uncached_with_serialno={} us, cached_on_transport={} us",
            res_unoptimized_us, res_warm_us
        );

        // 4. inspect_stepos_ai_suite parallel + transport-reused
        let t_suite = Instant::now();
        if let Ok(report) = crate::toolbox::inspect_stepos_ai_suite(&adb, &device) {
            let suite_ms = t_suite.elapsed().as_millis();
            eprintln!(
                "[Workstream1 Bench] inspect_stepos_ai_suite parallel latency={} ms (mcp_services={})",
                suite_ms,
                report.mcp_services.len()
            );
        }
    }

    #[test]
    fn transport_classification_and_transient_error_detection() {
        assert_eq!(
            classify_transport("STEPXTEST8DB89E4B8927"),
            TransportKind::Usb
        );
        assert!(!is_wireless_transport("STEPXTEST8DB89E4B8927"));

        assert_eq!(
            classify_transport("10.0.0.42:34883"),
            TransportKind::WirelessTcp
        );
        assert!(is_wireless_transport("10.0.0.42:34883"));

        assert_eq!(
            classify_transport("adb-STEPXTEST8DB89E4B8927-jYyV3X._adb-tls-connect._tcp"),
            TransportKind::WirelessMdnsTls
        );
        assert!(is_wireless_transport(
            "adb-STEPXTEST8DB89E4B8927-jYyV3X._adb-tls-connect._tcp"
        ));

        for transient in [
            "adb: device offline",
            "error: device '10.0.0.42:5555' not found",
            "error: closed",
            "adb: protocol fault (couldn't read status)",
            "Connection reset by peer",
            "Broken pipe",
            "cannot bind listener: Address already in use",
            "error: more than one device/emulator",
        ] {
            assert!(
                is_transient_transport_error(transient, "", ""),
                "expected transient error for: {transient}"
            );
        }
        assert!(!is_transient_transport_error(
            "/system/bin/sh: non_existent_binary: inaccessible or not found",
            "",
            ""
        ));
        assert!(!is_transient_transport_error(
            "java.lang.SecurityException: Permission Denial",
            "",
            ""
        ));
    }

    #[test]
    fn transport_coordinator_concurrency_limits_and_telemetry() {
        use std::sync::{
            atomic::{AtomicUsize, Ordering},
            Arc,
        };

        let wireless_target = "10.0.1.199:45555";
        let usb_target = "STEPX_USB_CONC_UNIT";
        assert_eq!(
            TransportCoordinator::max_concurrency_for(wireless_target),
            3
        );
        assert_eq!(TransportCoordinator::max_concurrency_for(usb_target), 6);

        let active = Arc::new(AtomicUsize::new(0));
        let max_seen = Arc::new(AtomicUsize::new(0));
        let mut handles = Vec::new();
        for _ in 0..8 {
            let active_cl = active.clone();
            let max_cl = max_seen.clone();
            handles.push(thread::spawn(move || {
                let _guard = TransportCoordinator::acquire_slot(wireless_target)
                    .expect("bounded slot acquisition");
                let cur = active_cl.fetch_add(1, Ordering::SeqCst) + 1;
                max_cl.fetch_max(cur, Ordering::SeqCst);
                thread::sleep(Duration::from_millis(25));
                active_cl.fetch_sub(1, Ordering::SeqCst);
            }));
        }
        for h in handles {
            h.join().unwrap();
        }
        assert!(
            max_seen.load(Ordering::SeqCst) <= 3,
            "wireless concurrency exceeded 3: {}",
            max_seen.load(Ordering::SeqCst)
        );
        assert_eq!(TransportCoordinator::active_in_flight(wireless_target), 0);

        let tel_target = "10.0.1.199:46666";
        TransportCoordinator::record_command_success(tel_target, 40);
        TransportCoordinator::record_command_failure(tel_target);
        TransportCoordinator::record_retry(tel_target);
        record_transport_failover(tel_target);
        let tel = transport_telemetry(tel_target);
        assert_eq!(tel.total_commands, 2);
        assert_eq!(tel.consecutive_failures, 1);
        assert_eq!(tel.total_retries, 1);
        assert_eq!(tel.total_failovers, 1);
        assert_eq!(tel.last_rtt_ms, 40);

        TransportCoordinator::record_command_success(tel_target, 20);
        let tel2 = transport_telemetry(tel_target);
        assert_eq!(tel2.consecutive_failures, 0);
        assert_eq!(tel2.total_commands, 3);
    }

    #[cfg(unix)]
    #[test]
    fn transient_error_automatically_fails_over_and_retries_command() {
        use std::os::unix::fs::PermissionsExt;

        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        let calls_log = dir.path().join("calls.log");

        let script = format!(
            r#"#!/usr/bin/env sh
echo "$*" >> "{calls}"
case "$*" in
  *"-s 10.0.1.250:39999 shell echo HELLO_FAILOVER"*)
    echo "adb: error: closed" >&2
    exit 1
    ;;
  *"-s STEPX_FAILOVER_USB_UNIT shell getprop ro.serialno"*)
    echo "STEPX_FAILOVER_USB_UNIT"
    exit 0
    ;;
  *"-s STEPX_FAILOVER_USB_UNIT shell echo HELLO_FAILOVER"*)
    echo "HELLO_FAILOVER"
    exit 0
    ;;
  "devices -l")
    echo "List of devices attached"
    echo "STEPX_FAILOVER_USB_UNIT device product:pandora model:PZ261A device:pandora"
    exit 0
    ;;
esac
exit 0
"#,
            calls = calls_log.display()
        );
        std::fs::write(&fake_adb, script).unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let adb = Adb { path: fake_adb };
        crate::device::set_backup_transports_for(
            &["10.0.1.250:39999", "STEPX_FAILOVER_USB_UNIT"],
            vec!["STEPX_FAILOVER_USB_UNIT".to_string()],
        );

        let out = adb
            .script("10.0.1.250:39999", "echo HELLO_FAILOVER")
            .expect("auto-retry must fail over to STEPX_FAILOVER_USB_UNIT");
        assert_eq!(out.trim(), "HELLO_FAILOVER");

        let tel = transport_telemetry("10.0.1.250:39999");
        assert!(tel.total_retries >= 1);
        assert!(tel.total_failovers >= 1);
    }
}
