//! Per-user desktop ownership. OS locks survive neither crashes nor reboots.
//! A loopback-only channel asks the existing UI to show its main window.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions, TryLockError},
    io::{Read, Write},
    net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream},
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[derive(Serialize, Deserialize)]
struct Endpoint {
    port: u16,
    #[serde(default)]
    token: Option<String>,
    #[serde(default)]
    pid: u32,
}

fn generate_token() -> String {
    format!("{:032x}", rand::random::<u128>())
}

fn accept_error_backoff(kind: std::io::ErrorKind) -> Option<Duration> {
    match kind {
        std::io::ErrorKind::Interrupted
        | std::io::ErrorKind::WouldBlock
        | std::io::ErrorKind::ConnectionAborted
        | std::io::ErrorKind::ConnectionReset
        | std::io::ErrorKind::TimedOut => None,
        _ => Some(Duration::from_millis(50)),
    }
}

fn handle_activation_connection(
    connection: &mut TcpStream,
    expected_token: Option<&str>,
    notify: &AtomicBool,
) {
    let timeout = Some(Duration::from_millis(250));
    let _ = connection.set_read_timeout(timeout);
    let _ = connection.set_write_timeout(timeout);
    let expected = match expected_token.filter(|t| !t.is_empty()) {
        Some(token) => format!("activate {token}"),
        None => "activate".to_string(),
    };
    let mut buf = [0u8; 128];
    let mut filled = 0usize;
    while filled < expected.len() {
        match connection.read(&mut buf[filled..]) {
            Ok(0) => break,
            Ok(n) => {
                filled += n;
                if buf[..filled].contains(&b'\n') {
                    break;
                }
            }
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    let received = std::str::from_utf8(&buf[..filled])
        .ok()
        .map(|s| s.trim_end_matches(['\r', '\n']));
    if received == Some(expected.as_str()) {
        notify.store(true, Ordering::Release);
        let _ = connection.write_all(b"ACK");
        let _ = connection.flush();
    }
}

pub enum Claim {
    Primary(Instance),
    Activated,
}

pub struct Instance {
    // Never unlink this file: other launchers may already hold a handle to it.
    _lock: File,
    address: SocketAddr,
    requested: Arc<AtomicBool>,
    stopped: Arc<AtomicBool>,
    worker: Option<JoinHandle<()>>,
}

impl Instance {
    pub fn claim(directory: &Path) -> Result<Claim> {
        std::fs::create_dir_all(directory).context("无法创建客户端运行目录")?;
        let mut options = OpenOptions::new();
        options.read(true).write(true).create(true).truncate(false);
        #[cfg(unix)]
        {
            use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
            // The app-specific directory contains only activation metadata.
            std::fs::set_permissions(directory, std::fs::Permissions::from_mode(0o700))?;
            options.mode(0o600);
        }
        let lock = options.open(directory.join("desktop.lock"))?;
        let endpoint_path = directory.join("desktop-instance.json");
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            match lock.try_lock() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    if activate(&endpoint_path).is_ok() {
                        return Ok(Claim::Activated);
                    }
                    if Instant::now() >= deadline {
                        anyhow::bail!("StepX Companion 已在运行，暂未响应唤起请求；请稍后再试");
                    }
                    // Only the second launch waits; the existing UI is unaffected.
                    thread::sleep(Duration::from_millis(50));
                }
                Err(TryLockError::Error(error)) => return Err(error.into()),
            }
        }
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0))?;
        let address = listener.local_addr()?;
        let endpoint = Endpoint {
            port: address.port(),
            token: Some(generate_token()),
            pid: std::process::id(),
        };
        let mut metadata = tempfile::NamedTempFile::new_in(directory)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            metadata
                .as_file()
                .set_permissions(std::fs::Permissions::from_mode(0o600))?;
        }
        serde_json::to_writer(&mut metadata, &endpoint)?;
        metadata.flush()?;
        metadata.as_file().sync_all()?;
        metadata.persist(&endpoint_path).map_err(|e| e.error)?;
        #[cfg(unix)]
        if let Ok(dir_file) = File::open(directory) {
            let _ = dir_file.sync_all();
        }
        let requested = Arc::new(AtomicBool::new(false));
        let stopped = Arc::new(AtomicBool::new(false));
        let (notify, stop) = (requested.clone(), stopped.clone());
        let expected_token = endpoint.token;
        let worker = thread::Builder::new()
            .name("desktop-activation".into())
            .spawn(move || loop {
                if stop.load(Ordering::Acquire) {
                    break;
                }
                let mut connection = match listener.accept() {
                    Ok((connection, _peer)) => {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        connection
                    }
                    Err(error) => {
                        if stop.load(Ordering::Acquire) {
                            break;
                        }
                        if let Some(backoff) = accept_error_backoff(error.kind()) {
                            thread::sleep(backoff);
                        }
                        continue;
                    }
                };
                handle_activation_connection(&mut connection, expected_token.as_deref(), &notify);
            })?;
        Ok(Claim::Primary(Self {
            _lock: lock,
            address,
            requested,
            stopped,
            worker: Some(worker),
        }))
    }

    pub fn activation_requests(&self) -> Arc<AtomicBool> {
        self.requested.clone()
    }
}

fn activate(path: &Path) -> Result<()> {
    // Bound the metadata read and never accept a non-loopback address from disk.
    let endpoint: Endpoint = serde_json::from_reader(File::open(path)?.take(1024))?;
    let timeout = Duration::from_millis(250);
    let mut stream = TcpStream::connect_timeout(
        &SocketAddr::from((Ipv4Addr::LOCALHOST, endpoint.port)),
        timeout,
    )?;
    stream.set_read_timeout(Some(timeout))?;
    stream.set_write_timeout(Some(timeout))?;
    #[cfg(windows)]
    if endpoint.pid != 0 {
        unsafe {
            // The launcher has the foreground grant from the user's click. Pass it
            // to the existing process before it restores/focuses the main window.
            #[link(name = "user32")]
            extern "system" {
                fn AllowSetForegroundWindow(process_id: u32) -> i32;
            }
            AllowSetForegroundWindow(endpoint.pid);
        }
    }
    let request = match endpoint.token.as_deref().filter(|t| !t.is_empty()) {
        Some(token) => format!("activate {token}"),
        None => "activate".to_string(),
    };
    stream.write_all(request.as_bytes())?;
    stream.flush()?;
    let _ = stream.shutdown(std::net::Shutdown::Write);
    let mut reply = [0u8; 3];
    stream.read_exact(&mut reply)?;
    anyhow::ensure!(&reply == b"ACK", "客户端唤起响应无效");
    Ok(())
}

impl Drop for Instance {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        // Wake a blocking accept without periodically polling while idle.
        let _ = TcpStream::connect_timeout(&self.address, Duration::from_millis(250));
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn duplicate_launch_activates_owner_and_normal_exit_releases_lock() {
        let dir = tempfile::tempdir().unwrap();
        let Claim::Primary(first) = Instance::claim(dir.path()).unwrap() else {
            panic!()
        };
        assert!(matches!(
            Instance::claim(dir.path()).unwrap(),
            Claim::Activated
        ));
        assert!(first.requested.swap(false, Ordering::AcqRel));
        drop(first);
        assert!(matches!(
            Instance::claim(dir.path()).unwrap(),
            Claim::Primary(_)
        ));
    }
    #[test]
    fn unknown_activation_token_does_not_focus_window() {
        let dir = tempfile::tempdir().unwrap();
        let Claim::Primary(first) = Instance::claim(dir.path()).unwrap() else {
            panic!()
        };
        let mut socket = TcpStream::connect(first.address).unwrap();
        socket
            .set_read_timeout(Some(Duration::from_secs(1)))
            .unwrap();
        socket.write_all(&[0u8; 32]).unwrap();
        let mut reply = [0u8; 3];
        assert!(socket.read_exact(&mut reply).is_err());
        assert!(!first.requested.load(Ordering::Acquire));
    }
    #[test]
    fn authenticated_activate_succeeds_and_rejects_unauthenticated_or_wrong_token() {
        let dir = tempfile::tempdir().unwrap();
        let Claim::Primary(first) = Instance::claim(dir.path()).unwrap() else {
            panic!()
        };
        let endpoint_path = dir.path().join("desktop-instance.json");
        let endpoint: Endpoint =
            serde_json::from_slice(&std::fs::read(&endpoint_path).unwrap()).unwrap();
        let token = endpoint
            .token
            .expect("primary instance must persist hex token");
        assert_eq!(token.len(), 32);
        assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&endpoint_path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777;
            assert_eq!(mode, 0o600);
        }

        // Unauthenticated / wrong-token / scanner probes must all be rejected without killing listener.
        for bad_payload in [
            "".as_bytes(),
            b"activate",
            b"activate\n",
            b"activate 00000000000000000000000000000000",
            b"GET / HTTP/1.1\r\n\r\n",
        ] {
            let mut probe = TcpStream::connect(first.address).unwrap();
            probe
                .set_read_timeout(Some(Duration::from_millis(400)))
                .unwrap();
            if !bad_payload.is_empty() {
                probe.write_all(bad_payload).unwrap();
            }
            let _ = probe.shutdown(std::net::Shutdown::Write);
            let mut reply = [0u8; 3];
            assert!(probe.read_exact(&mut reply).is_err());
            assert!(!first.requested.load(Ordering::Acquire));
        }

        // Valid authenticated command succeeds and returns ACK.
        let mut auth = TcpStream::connect(first.address).unwrap();
        auth.set_read_timeout(Some(Duration::from_secs(1))).unwrap();
        auth.write_all(format!("activate {token}").as_bytes())
            .unwrap();
        let mut reply = [0u8; 3];
        auth.read_exact(&mut reply).unwrap();
        assert_eq!(&reply, b"ACK");
        assert!(first.requested.swap(false, Ordering::AcqRel));

        // Legacy endpoint JSON without `token` still deserializes cleanly via #[serde(default)].
        let legacy: Endpoint = serde_json::from_str(r#"{"port":12345,"pid":42}"#).unwrap();
        assert_eq!(legacy.port, 12345);
        assert_eq!(legacy.token, None);
    }
    #[test]
    fn transient_aborted_connections_and_accept_errors_do_not_kill_listener() {
        assert_eq!(accept_error_backoff(std::io::ErrorKind::Interrupted), None);
        assert_eq!(accept_error_backoff(std::io::ErrorKind::WouldBlock), None);
        assert_eq!(
            accept_error_backoff(std::io::ErrorKind::ConnectionAborted),
            None
        );
        assert_eq!(
            accept_error_backoff(std::io::ErrorKind::ConnectionReset),
            None
        );
        assert_eq!(
            accept_error_backoff(std::io::ErrorKind::Other),
            Some(Duration::from_millis(50))
        );
        #[cfg(unix)]
        {
            let emfile = std::io::Error::from_raw_os_error(libc::EMFILE);
            let enfile = std::io::Error::from_raw_os_error(libc::ENFILE);
            assert_eq!(
                accept_error_backoff(emfile.kind()),
                Some(Duration::from_millis(50))
            );
            assert_eq!(
                accept_error_backoff(enfile.kind()),
                Some(Duration::from_millis(50))
            );
        }

        let dir = tempfile::tempdir().unwrap();
        let Claim::Primary(first) = Instance::claim(dir.path()).unwrap() else {
            panic!()
        };
        for _ in 0..16 {
            if let Ok(conn) = TcpStream::connect(first.address) {
                #[cfg(unix)]
                unsafe {
                    use std::os::fd::AsRawFd;
                    let linger = libc::linger {
                        l_onoff: 1,
                        l_linger: 0,
                    };
                    let _ = libc::setsockopt(
                        conn.as_raw_fd(),
                        libc::SOL_SOCKET,
                        libc::SO_LINGER,
                        (&linger as *const libc::linger).cast(),
                        std::mem::size_of::<libc::linger>() as libc::socklen_t,
                    );
                }
                drop(conn);
            }
        }
        assert!(!first.requested.load(Ordering::Acquire));
        assert!(matches!(
            Instance::claim(dir.path()).unwrap(),
            Claim::Activated
        ));
        assert!(first.requested.swap(false, Ordering::AcqRel));
    }
    #[test]
    fn child_holds_lock() {
        let Some(dir) = std::env::var_os("STEPX_INSTANCE_TEST_CHILD") else {
            return;
        };
        let dir = std::path::PathBuf::from(dir);
        let Claim::Primary(_guard) = Instance::claim(&dir).unwrap() else {
            panic!()
        };
        std::fs::write(dir.join("ready"), b"ready").unwrap();
        thread::sleep(Duration::from_secs(15));
    }
    #[test]
    fn killed_process_does_not_leave_a_stale_lock() {
        let dir = tempfile::tempdir().unwrap();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "single_instance::tests::child_holds_lock"])
            .env("STEPX_INSTANCE_TEST_CHILD", dir.path())
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        let deadline = Instant::now() + Duration::from_secs(5);
        while !dir.path().join("ready").exists() && Instant::now() < deadline {
            thread::sleep(Duration::from_millis(20));
        }
        let ready = dir.path().join("ready").exists();
        let duplicate = ready && matches!(Instance::claim(dir.path()), Ok(Claim::Activated));
        let _ = child.kill();
        let _ = child.wait();
        assert!(
            ready && duplicate,
            "child did not own/activate the instance"
        );
        assert!(matches!(
            Instance::claim(dir.path()).unwrap(),
            Claim::Primary(_)
        ));
    }
}
