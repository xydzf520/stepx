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

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum MirrorAudioRoute {
    /// 声音从电脑扬声器/耳机播放（scrcpy 默认转发音频并静音手机外放）
    #[default]
    Pc,
    /// 声音保留在手机端扬声器播放（--no-audio）
    Phone,
    /// 电脑与手机双端同时播放（--audio-dup，Android 13+ 支持）
    Both,
}

impl MirrorAudioRoute {
    #[allow(non_upper_case_globals)]
    pub const DesktopOnly: Self = Self::Pc;
    #[allow(non_upper_case_globals)]
    pub const PhoneOnly: Self = Self::Phone;

    pub fn label(self) -> &'static str {
        match self {
            Self::Pc => "电脑端播放",
            Self::Phone => "手机端播放",
            Self::Both => "双端同播",
        }
    }

    pub fn short_label(self) -> &'static str {
        match self {
            Self::Pc => "电脑发声",
            Self::Phone => "手机发声",
            Self::Both => "双端发声",
        }
    }

    pub fn detail_label(self) -> &'static str {
        match self {
            Self::Pc => "声音从电脑播放（手机自动静音）",
            Self::Phone => "声音从手机扬声器播放",
            Self::Both => "电脑与手机同时播放声音",
        }
    }

    pub fn next(self) -> Self {
        match self {
            Self::Pc => Self::Phone,
            Self::Phone => Self::Both,
            Self::Both => Self::Pc,
        }
    }
}

pub fn resolve_effective_audio_route(
    requested: MirrorAudioRoute,
    force_no_audio: bool,
    has_audio_dup: bool,
) -> (MirrorAudioRoute, bool) {
    if force_no_audio {
        (
            MirrorAudioRoute::PhoneOnly,
            requested != MirrorAudioRoute::PhoneOnly,
        )
    } else if requested == MirrorAudioRoute::Both && !has_audio_dup {
        (MirrorAudioRoute::DesktopOnly, true)
    } else {
        (requested, false)
    }
}

pub fn format_started_message(
    frame_received: bool,
    audio_route: MirrorAudioRoute,
    audio_downgraded: bool,
) -> &'static str {
    match (frame_received, audio_route, audio_downgraded) {
        (true, MirrorAudioRoute::Pc, false) => {
            "独立投屏已启动（声音：电脑端播放）· 手机画面已就绪；投屏期间保持手机屏幕活跃"
        }
        (true, MirrorAudioRoute::Pc, true) => {
            "独立投屏已启动（声音：电脑端播放 · 当前组件不支持双端同播）· 手机画面已就绪；投屏期间保持手机屏幕活跃"
        }
        (true, MirrorAudioRoute::Phone, false) => {
            "独立投屏已启动（声音：手机端播放）· 手机画面已就绪；投屏期间保持手机屏幕活跃"
        }
        (true, MirrorAudioRoute::Phone, true) => {
            "独立投屏已启动（声音：手机端播放 · 音频转发受限已自动降级）· 手机画面已就绪；投屏期间保持手机屏幕活跃"
        }
        (true, MirrorAudioRoute::Both, _) => {
            "独立投屏已启动（声音：电脑+手机双端同播）· 手机画面已就绪；投屏期间保持手机屏幕活跃"
        }
        (false, MirrorAudioRoute::Pc, false) => {
            "独立投屏已启动（声音：电脑端播放）· 请在独立窗口确认画面；投屏期间保持手机屏幕活跃"
        }
        (false, MirrorAudioRoute::Pc, true) => {
            "独立投屏已启动（声音：电脑端播放 · 当前组件不支持双端同播）· 请在独立窗口确认画面；投屏期间保持手机屏幕活跃"
        }
        (false, MirrorAudioRoute::Phone, false) => {
            "独立投屏已启动（声音：手机端播放）· 请在独立窗口确认画面；投屏期间保持手机屏幕活跃"
        }
        (false, MirrorAudioRoute::Phone, true) => {
            "独立投屏已启动（声音：手机端播放 · 音频转发受限已自动降级）· 请在独立窗口确认画面；投屏期间保持手机屏幕活跃"
        }
        (false, MirrorAudioRoute::Both, _) => {
            "独立投屏已启动（声音：电脑+手机双端同播）· 请在独立窗口确认画面；投屏期间保持手机屏幕活跃"
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MirrorOptions {
    /// 0 = 720p Smooth (1280, 30fps, 4M), 1 = 1080p HD (1600, 60fps, 8M), 2 = Ultra Native (2400, 120fps, 16M)
    pub quality: u8,
    pub audio_route: MirrorAudioRoute,
    pub always_on_top: bool,
    pub turn_screen_off: bool,
    pub stay_awake: bool,
    pub show_touches: bool,
}

impl Default for MirrorOptions {
    fn default() -> Self {
        Self {
            quality: 1,
            audio_route: MirrorAudioRoute::Pc,
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

    pub fn quality_params_for_transport(
        self,
        is_wireless: bool,
    ) -> (&'static str, &'static str, &'static str) {
        if is_wireless {
            match self.quality {
                0 => ("1280", "30", "3M"),
                1 => ("1440", "60", "6M"),
                _ => ("1920", "90", "10M"),
            }
        } else {
            self.quality_params()
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
    audio_route: MirrorAudioRoute,
    audio_downgraded: bool,
    window: crate::mirror_window::Watcher,
}

impl MirrorSession {
    pub fn start(adb: &Adb, serial: &str) -> Result<Self> {
        Self::start_with_options(adb, serial, MirrorOptions::default())
    }

    pub fn audio_route(&self) -> MirrorAudioRoute {
        self.audio_route
    }

    pub fn audio_downgraded(&self) -> bool {
        self.audio_downgraded
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
        let has_audio_dup = help_text.contains("--audio-dup");
        let resolved = crate::device::resolve_online_transport(adb, serial, "", "", &[])
            .unwrap_or_else(|_| serial.to_string());
        // Wake phone display if it went to sleep on wireless standby so SurfaceFlinger emits the initial frame immediately.
        let _ = adb.run_timeout(
            &["-s", &resolved, "shell", "input", "keyevent", "224"],
            Duration::from_secs(2),
        );
        let build_cmd = |target_serial: &str, force_no_audio: bool| -> Command {
            let is_wireless =
                target_serial.contains(':') || target_serial.starts_with("adb-");
            let (max_size, max_fps, bit_rate) =
                options.quality_params_for_transport(is_wireless);
            let mut command = Command::new(&binary);
            #[cfg(target_os = "linux")]
            if std::env::var_os("DISPLAY").is_some() {
                // XWayland permits the control rail to follow the SDL window.
                command.env("SDL_VIDEODRIVER", "x11");
            }
            command.env("ADB", &adb.path).args([
                "--serial",
                target_serial,
                "--window-title",
                "StepX Companion · 手机投屏",
                "--max-size",
                max_size,
                "--video-codec=h264",
            ]);
            if force_no_audio {
                command.arg("--no-audio");
            } else {
                match options.audio_route {
                    MirrorAudioRoute::Phone => {
                        command.arg("--no-audio");
                    }
                    MirrorAudioRoute::Pc => {}
                    MirrorAudioRoute::Both => {
                        if has_audio_dup {
                            command.arg("--audio-dup");
                        }
                    }
                }
            }
            if is_wireless {
                command.arg("--video-codec-options=i-frame-interval=2");
                if !force_no_audio && options.audio_route != MirrorAudioRoute::Phone {
                    command.arg("--audio-buffer=80");
                }
            }
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
            command
        };

        let first_try = Self::spawn(build_cmd(&resolved, false), Duration::from_secs(20));
        let can_retry_no_audio = options.audio_route != MirrorAudioRoute::Phone;
        let (mut session, used_no_audio_fallback) = match first_try {
            Ok(s) => (s, false),
            Err(first_err) => {
                // If wireless audio forwarding, an audio encoder failure, or an mDNS transport
                // caused startup failure, try alternate online transport or fallback with `--no-audio`.
                let alt = crate::device::resolve_online_transport(adb, "", "", "PZ261A", &[])
                    .ok()
                    .filter(|t| t != &resolved);
                let is_wireless_or_audio_err = resolved.contains(':')
                    || resolved.starts_with("adb-")
                    || first_err.to_string().to_ascii_lowercase().contains("audio");
                if let Some(alt_serial) = alt {
                    let _ = adb.run_timeout(
                        &["-s", &alt_serial, "shell", "input", "keyevent", "224"],
                        Duration::from_secs(2),
                    );
                    match Self::spawn(build_cmd(&alt_serial, false), Duration::from_secs(16)) {
                        Ok(s) => (s, false),
                        Err(_) if can_retry_no_audio => (
                            Self::spawn(build_cmd(&alt_serial, true), Duration::from_secs(14))
                                .map_err(|_| first_err)?,
                            true,
                        ),
                        Err(_) => return Err(first_err),
                    }
                } else if can_retry_no_audio && is_wireless_or_audio_err {
                    (
                        Self::spawn(build_cmd(&resolved, true), Duration::from_secs(14))
                            .map_err(|_| first_err)?,
                        true,
                    )
                } else {
                    return Err(first_err);
                }
            }
        };
        let (effective_route, audio_downgraded) = resolve_effective_audio_route(
            options.audio_route,
            used_no_audio_fallback,
            has_audio_dup,
        );
        session.audio_route = effective_route;
        session.audio_downgraded = audio_downgraded;
        Ok(session)
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
            audio_route: MirrorAudioRoute::default(),
            audio_downgraded: false,
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
        format_started_message(self.frame_received, self.audio_route, self.audio_downgraded)
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
        assert_eq!(def.audio_route, MirrorAudioRoute::Pc);
        assert!(def.stay_awake);
        assert!(!def.always_on_top);
        assert!(!def.turn_screen_off);
        assert!(!def.show_touches);
        assert_eq!(def.quality_params(), ("1600", "60", "8M"));
        assert_eq!(def.quality_params_for_transport(false), ("1600", "60", "8M"));
        assert_eq!(def.quality_params_for_transport(true), ("1440", "60", "6M"));
        let smooth = MirrorOptions { quality: 0, ..def };
        assert_eq!(smooth.quality_params(), ("1280", "30", "4M"));
        assert_eq!(smooth.quality_params_for_transport(true), ("1280", "30", "3M"));
        let ultra = MirrorOptions { quality: 2, ..def };
        assert_eq!(ultra.quality_params(), ("2400", "120", "16M"));
        assert_eq!(ultra.quality_params_for_transport(true), ("1920", "90", "10M"));
    }

    #[test]
    fn mirror_audio_route_cycling_and_labels() {
        let pc = MirrorAudioRoute::default();
        assert_eq!(pc, MirrorAudioRoute::Pc);
        assert_eq!(pc.label(), "电脑端播放");
        assert_eq!(pc.short_label(), "电脑发声");
        assert_eq!(pc.detail_label(), "声音从电脑播放（手机自动静音）");

        let phone = pc.next();
        assert_eq!(phone, MirrorAudioRoute::Phone);
        assert_eq!(phone.label(), "手机端播放");
        assert_eq!(phone.short_label(), "手机发声");
        assert_eq!(phone.detail_label(), "声音从手机扬声器播放");

        let both = phone.next();
        assert_eq!(both, MirrorAudioRoute::Both);
        assert_eq!(both.label(), "双端同播");
        assert_eq!(both.short_label(), "双端发声");
        assert_eq!(both.detail_label(), "电脑与手机同时播放声音");

        assert_eq!(both.next(), MirrorAudioRoute::Pc);
    }

    #[test]
    fn audio_downgrade_updates_effective_route_and_started_message() {
        let (route, downgraded) =
            resolve_effective_audio_route(MirrorAudioRoute::Pc, true, true);
        assert_eq!(route, MirrorAudioRoute::PhoneOnly);
        assert!(downgraded);
        let msg = format_started_message(true, route, downgraded);
        assert!(msg.contains("手机端播放"), "{msg}");
        assert!(!msg.contains("电脑端播放"), "{msg}");

        let (route2, downgraded2) =
            resolve_effective_audio_route(MirrorAudioRoute::Both, false, false);
        assert_eq!(route2, MirrorAudioRoute::DesktopOnly);
        assert!(downgraded2);
        let msg2 = format_started_message(true, route2, downgraded2);
        assert!(msg2.contains("电脑端播放"), "{msg2}");
        assert!(msg2.contains("不支持双端同播"), "{msg2}");

        let (route3, downgraded3) =
            resolve_effective_audio_route(MirrorAudioRoute::Phone, true, true);
        assert_eq!(route3, MirrorAudioRoute::Phone);
        assert!(!downgraded3);

        // End-to-end test of MirrorSession::start_with_options with a fake scrcpy binary
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let fake_adb = dir.path().join("adb");
        std::fs::write(&fake_adb, "#!/usr/bin/env bash\nexit 0\n").unwrap();
        std::fs::set_permissions(&fake_adb, std::fs::Permissions::from_mode(0o755)).unwrap();

        let fake_scrcpy = dir.path().join("scrcpy");
        let scrcpy_script = r#"#!/usr/bin/env bash
if [[ "$*" == *"--help"* ]]; then
  echo "scrcpy 5.0.1 --keep-active --max-fps --video-bit-rate"
  exit 0
fi
if [[ "$*" != *"--no-audio"* ]]; then
  echo "ERROR: Audio capture failed on wireless transport" >&2
  exit 1
fi
echo "INFO: Texture: 1080x2400"
sleep 5
"#;
        std::fs::write(&fake_scrcpy, scrcpy_script).unwrap();
        std::fs::set_permissions(&fake_scrcpy, std::fs::Permissions::from_mode(0o755)).unwrap();

        std::env::set_var("STEPX_SCRCPY", &fake_scrcpy);
        let adb = Adb { path: fake_adb };
        let session = MirrorSession::start_with_options(
            &adb,
            "192.168.1.88:5555",
            MirrorOptions {
                audio_route: MirrorAudioRoute::Pc,
                ..MirrorOptions::default()
            },
        )
        .expect("start_with_options should succeed via --no-audio fallback");
        std::env::remove_var("STEPX_SCRCPY");

        assert_eq!(session.audio_route(), MirrorAudioRoute::PhoneOnly);
        assert!(session.audio_downgraded());
        let started_msg = session.started_message();
        assert!(started_msg.contains("手机端播放"), "{started_msg}");
        assert!(!started_msg.contains("声音：电脑端播放"), "{started_msg}");
    }
}
