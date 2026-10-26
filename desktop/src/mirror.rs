//! Managed scrcpy session: capture failures, await video, and keep the phone active.
use crate::adb::{bundled_tool, find_executable, platform_tool_candidate, run_process, Adb};
use anyhow::{bail, Context, Result};
use std::{
    collections::HashMap,
    io::Read,
    path::PathBuf,
    process::{Child, Command, ExitStatus, Stdio},
    sync::{Arc, Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

const LOG_LIMIT: usize = 64 * 1024;
type Log = Arc<Mutex<Vec<u8>>>;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirrorOptions {
    /// 0 = 720p Smooth (1280, 30fps, 4M), 1 = 1080p HD (1600, 60fps, 8M), 2 = Ultra Native (2400, 120fps, 16M)
    pub quality: u8,
    pub always_on_top: bool,
    pub turn_screen_off: bool,
    pub stay_awake: bool,
    pub show_touches: bool,
}

impl Default for MirrorOptions {
    fn default() -> Self {
        Self {
            quality: 1,
            always_on_top: false,
            turn_screen_off: false,
            stay_awake: true,
            show_touches: false,
        }
    }
}

impl MirrorOptions {
    pub fn quality_params(self) -> (&'static str, &'static str, &'static str) {
        match self.quality {
            0 => ("1280", "30", "4M"),
            1 => ("1600", "60", "8M"),
            _ => ("2400", "120", "16M"),
        }
    }
}

fn scrcpy_capabilities(binary: &PathBuf) -> Result<(bool, bool, String)> {
    static CACHE: OnceLock<Mutex<HashMap<PathBuf, (bool, bool, String)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Ok(guard) = cache.lock() {
        if let Some(caps) = guard.get(binary) {
            return Ok(caps.clone());
        }
    }
    let mut help_command = Command::new(binary);
    help_command.arg("--help");
    let help = run_process(help_command, None, Duration::from_secs(5))
        .context("无法运行投屏组件，请重新安装 desktop")?;
    let help_text = String::from_utf8_lossy(&help.stdout).into_owned();
    let caps = (
        help.status.success() && help_text.contains("--keep-active"),
        help_text.contains("--hwdec="),
        help_text,
    );
    if caps.0 {
        if let Ok(mut guard) = cache.lock() {
            guard.insert(binary.clone(), caps.clone());
        }
    }
    Ok(caps)
}

pub fn start_with_options(
    adb: &Adb,
    serial: &str,
    options: MirrorOptions,
) -> Result<MirrorSession> {
    MirrorSession::start_with_options(adb, serial, options)
}

pub struct MirrorSession {
    child: Child,
    log: Log,
    exited: bool,
    frame_received: bool,
    window: crate::mirror_window::Watcher,
}

impl MirrorSession {
    pub fn start(adb: &Adb, serial: &str) -> Result<Self> {
        Self::start_with_options(adb, serial, MirrorOptions::default())
    }

    pub fn start_with_options(adb: &Adb, serial: &str, options: MirrorOptions) -> Result<Self> {
        let binary = std::env::var_os("STEPX_SCRCPY")
            .map(PathBuf::from)
            .or_else(|| bundled_tool("scrcpy"))
            .or_else(|| find_executable("scrcpy"))
            .or_else(|| platform_tool_candidate("scrcpy"))
            .context("缺少投屏组件 scrcpy，请重新安装完整版 desktop，或运行 scripts/setup-desktop-tools.sh。")?;
        let (has_keep_active, has_hwdec, help_text) = scrcpy_capabilities(&binary)?;
        if !has_keep_active {
            bail!("投屏组件版本不兼容，无法在投屏时保持手机亮屏。请更新 scrcpy（当前验证版本 5.0.1）或重新安装完整版 desktop。");
        }
        let (max_size, max_fps, bit_rate) = options.quality_params();
        let mut command = Command::new(binary);
        #[cfg(target_os = "linux")]
        if std::env::var_os("DISPLAY").is_some() {
            // XWayland permits the control rail to follow the SDL window.
            command.env("SDL_VIDEODRIVER", "x11");
        }
        command.env("ADB", &adb.path).args([
            "--serial",
            serial,
            "--window-title",
            "StepX Companion · 手机投屏",
            "--max-size",
            max_size,
            "--no-audio",
            "--video-codec=h264",
        ]);
        if options.stay_awake || has_keep_active {
            // Signals activity only during this session, including over Wi-Fi.
            command.arg("--keep-active");
        }
        if help_text.contains("--max-fps") {
            command.arg(format!("--max-fps={max_fps}"));
        }
        if help_text.contains("--video-bit-rate") {
            command.arg(format!("--video-bit-rate={bit_rate}"));
        }
        if options.always_on_top && help_text.contains("--always-on-top") {
            command.arg("--always-on-top");
        }
        if options.turn_screen_off && help_text.contains("--turn-screen-off") {
            command.arg("--turn-screen-off");
        }
        if options.show_touches && help_text.contains("--show-touches") {
            command.arg("--show-touches");
        }
        // Avoid probing broken VA-API drivers on Linux. GPU window rendering is
        // still enabled; only video decoding uses the portable software path.
        if cfg!(target_os = "linux") && has_hwdec {
            command.arg("--hwdec=disabled");
        }
        let renderer = std::env::var("STEPX_RENDER_DRIVER").ok().or_else(|| {
            cfg!(all(target_os = "linux", target_arch = "aarch64")).then(|| "software".into())
        });
        if let Some(renderer) = renderer {
            command.arg(format!("--render-driver={renderer}"));
        }
        Self::spawn(command, Duration::from_secs(20))
    }

    fn spawn(mut command: Command, timeout: Duration) -> Result<Self> {
        crate::resources::quiet_command(&mut command);
        let log = Arc::new(Mutex::new(Vec::new()));
        command.stdin(Stdio::null());
        // scrcpy buffers stdout when redirected to a pipe/file. A terminal keeps
        // the first decoded frame notification visible before the process exits.
        #[cfg(unix)]
        let output = {
            use std::{fs::File, os::fd::FromRawFd};
            let (mut master, mut slave) = (-1, -1);
            let rc = unsafe {
                libc::openpty(
                    &mut master,
                    &mut slave,
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                    std::ptr::null_mut(),
                )
            };
            if rc != 0 {
                return Err(std::io::Error::last_os_error()).context("无法创建投屏日志通道");
            }
            let reader = unsafe { File::from_raw_fd(master) };
            let writer = unsafe { File::from_raw_fd(slave) };
            // Only stdout/stderr should survive exec, not these original PTY fds.
            for fd in [master, slave] {
                if unsafe { libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC) } == -1 {
                    return Err(std::io::Error::last_os_error()).context("无法配置投屏日志通道");
                }
            }
            command.stdout(writer.try_clone()?).stderr(writer);
            reader
        };
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x0800_0000);
            command.stdout(Stdio::piped()).stderr(Stdio::piped());
        }
        let child = command.spawn().context("无法启动投屏窗口")?;
        drop(command);
        let window = crate::mirror_window::Watcher::new(child.id());
        let mut session = Self {
            child,
            window,
            log,
            exited: false,
            frame_received: false,
        };
        #[cfg(unix)]
        collect_log(output, session.log.clone());
        #[cfg(windows)]
        {
            collect_log(session.child.stdout.take().unwrap(), session.log.clone());
            collect_log(session.child.stderr.take().unwrap(), session.log.clone());
        }
        let started = Instant::now();
        loop {
            if let Some(status) = session.try_wait()? {
                bail!("投屏启动失败：{}", session.exit_detail(status));
            }
            if session
                .log_text()
                .lines()
                .any(|line| line.starts_with("INFO: Texture: "))
            {
                session.frame_received = true;
                return Ok(session);
            }
            // CREATE_NO_WINDOW on Windows does not provide a terminal. Its stdout
            // may stay buffered, so report process launch only, never verified video.
            #[cfg(windows)]
            if started.elapsed() >= Duration::from_secs(3) {
                return Ok(session);
            }
            if started.elapsed() >= timeout {
                bail!(
                    "投屏连接超时，尚未收到手机画面：{}",
                    failure_detail(&session.log_text())
                );
            }
            thread::sleep(Duration::from_millis(100));
        }
    }

    pub fn started_message(&self) -> &'static str {
        if self.frame_received {
            "投屏已连接，手机画面已就绪；投屏期间保持手机屏幕活跃"
        } else {
            "投屏进程已启动，请在独立窗口确认画面；投屏期间保持手机屏幕活跃"
        }
    }

    pub fn window_bounds(&self) -> Option<crate::mirror_window::MirrorWindow> {
        self.window.latest()
    }

    pub fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.exited |= status.is_some();
        Ok(status)
    }

    pub fn exit_detail(&self, status: ExitStatus) -> String {
        // Give the log reader a chance to consume the last error written at exit.
        thread::sleep(Duration::from_millis(30));
        format!("{}（{}）", failure_detail(&self.log_text()), status)
    }

    fn log_text(&self) -> String {
        self.log
            .lock()
            .map(|s| String::from_utf8_lossy(&s).into_owned())
            .unwrap_or_default()
    }

    /// Signal immediately, then reap off the UI thread so a slow scrcpy shutdown cannot freeze it.
    pub fn stop_in_background(mut self) {
        if !self.exited {
            #[cfg(unix)]
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGTERM);
            }
            #[cfg(windows)]
            {
                let _ = self.child.kill();
            }
        }
        thread::spawn(move || self.stop());
    }

    pub fn stop(&mut self) {
        if self.exited || matches!(self.try_wait(), Ok(Some(_))) {
            return;
        }
        #[cfg(unix)]
        {
            unsafe {
                libc::kill(self.child.id() as i32, libc::SIGTERM);
            }
            let started = Instant::now();
            while started.elapsed() < Duration::from_secs(2) {
                if matches!(self.try_wait(), Ok(Some(_))) {
                    return;
                }
                thread::sleep(Duration::from_millis(20));
            }
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        self.exited = true;
    }
}

impl Drop for MirrorSession {
    fn drop(&mut self) {
        self.stop();
    }
}

fn collect_log(mut reader: impl Read + Send + 'static, log: Log) {
    thread::spawn(move || {
        let mut buf = [0; 4096];
        loop {
            match reader.read(&mut buf) {
                Ok(0) => break,
                Ok(n) => {
                    if let Ok(mut log) = log.lock() {
                        log.extend_from_slice(&buf[..n]);
                        let excess = log.len().saturating_sub(LOG_LIMIT);
                        log.drain(..excess);
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => break,
            }
        }
    });
}

fn failure_detail(log: &str) -> String {
    let errors: Vec<_> = log
        .lines()
        .filter(|l| l.contains("ERROR:") || l.contains("Exception") || l.contains("error:"))
        .collect();
    let detail = if errors.is_empty() {
        "请解锁手机并检查调试连接，然后重试".to_owned()
    } else {
        errors[errors.len().saturating_sub(3)..].join("；")
    };
    detail.chars().take(600).collect()
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn command(script: &str) -> Command {
        let mut command = Command::new("sh");
        command.args(["-c", script]);
        command
    }

    #[test]
    fn immediate_failure_reports_real_cause() {
        let result = MirrorSession::spawn(
            command("echo 'ERROR: encoder unavailable' >&2; exit 1"),
            Duration::from_secs(2),
        );
        let error = result.err().expect("startup must fail").to_string();
        assert!(error.contains("encoder unavailable"), "{error}");
    }

    #[test]
    fn process_without_video_times_out_and_is_reaped() {
        let file = tempfile::NamedTempFile::new().unwrap();
        let mut cmd = command("echo $$ > \"$PID_PATH\"; exec sleep 30");
        cmd.env("PID_PATH", file.path());
        let result = MirrorSession::spawn(cmd, Duration::from_millis(200));
        assert!(result
            .err()
            .unwrap()
            .to_string()
            .contains("尚未收到手机画面"));
        let pid: i32 = std::fs::read_to_string(file.path())
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        assert_eq!(unsafe { libc::kill(pid, 0) }, -1);
    }

    #[test]
    fn slow_shutdown_never_blocks_caller_and_still_reaps_process() {
        let session = MirrorSession::spawn(
            command("trap '' TERM; echo 'INFO: Texture: 718x1600'; exec sleep 30"),
            Duration::from_secs(2),
        )
        .unwrap();
        let pid = session.child.id() as i32;
        let start = Instant::now();
        session.stop_in_background();
        assert!(start.elapsed() < Duration::from_millis(200));
        while unsafe { libc::kill(pid, 0) } == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(4),
                "child was not reaped"
            );
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn waits_for_video_and_reports_later_disconnect() {
        let mut session = MirrorSession::spawn(command("echo 'INFO: Texture: 718x1600'; sleep 0.3; echo 'ERROR: Device disconnected' >&2; exit 1"), Duration::from_secs(2)).unwrap();
        assert!(session.frame_received);
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            if let Some(status) = session.try_wait().unwrap() {
                assert!(!status.success());
                assert!(session.exit_detail(status).contains("Device disconnected"));
                break;
            }
            assert!(Instant::now() < deadline);
            thread::sleep(Duration::from_millis(20));
        }
    }

    #[test]
    fn mirror_options_quality_presets() {
        let def = MirrorOptions::default();
        assert_eq!(def.quality, 1);
        assert!(def.stay_awake);
        assert!(!def.always_on_top);
        assert!(!def.turn_screen_off);
        assert!(!def.show_touches);
        assert_eq!(def.quality_params(), ("1600", "60", "8M"));
        let smooth = MirrorOptions { quality: 0, ..def };
        assert_eq!(smooth.quality_params(), ("1280", "30", "4M"));
        let ultra = MirrorOptions { quality: 2, ..def };
        assert_eq!(ultra.quality_params(), ("2400", "120", "16M"));
    }
}
