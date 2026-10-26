use anyhow::{bail, Context, Result};
use std::{
    ffi::{OsStr, OsString},
    io::{Read, Write},
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
    sync::{Mutex, OnceLock},
    thread,
    time::{Duration, Instant},
};

#[derive(Clone, Debug)]
pub struct Adb {
    pub path: PathBuf,
}

static DISCOVER_CACHE: OnceLock<Mutex<Option<(Option<OsString>, PathBuf)>>> = OnceLock::new();

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
                if let Some(dir) = std::env::var_os(key) {
                    let path = PathBuf::from(dir).join("platform-tools").join(exe("adb"));
                    if path.is_file() {
                        sdk_path = Some(path);
                        break;
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
        let mut command = Command::new(&self.path);
        command.args(args).env("ADB_MDNS_OPENSCREEN", "1");
        command
    }

    pub fn run(&self, args: &[&str]) -> Result<String> {
        self.run_timeout(args, Duration::from_secs(20))
    }

    pub fn run_timeout(&self, args: &[&str], timeout: Duration) -> Result<String> {
        let output = run_process(
            self.command(&args.iter().map(OsString::from).collect::<Vec<_>>()),
            None,
            timeout,
        )?;
        checked_text(output)
    }

    pub fn pair(&self, endpoint: &str, code: &str) -> Result<String> {
        // Feed the secret via stdin: it never appears in process arguments or logs.
        let output = run_process(
            self.command(&["pair".into(), endpoint.into()]),
            Some(format!("{code}\n").into_bytes()),
            Duration::from_secs(25),
        )?;
        let stdout = String::from_utf8_lossy(&output.stdout);
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
        let args = ["-s", serial, "exec-out", "screencap", "-p"];
        let output = run_process(
            self.command(&args.iter().map(OsString::from).collect::<Vec<_>>()),
            None,
            Duration::from_secs(20),
        )?;
        if !output.status.success() || !output.stdout.starts_with(b"\x89PNG\r\n\x1a\n") {
            bail!("无法获取屏幕。请解锁手机并检查调试授权；受保护画面可能不允许截屏。")
        }
        Ok(output.stdout)
    }

    pub fn transfer(&self, serial: &str, verb: &str, source: &Path, target: &Path) -> Result<()> {
        let args = vec![
            "-s".into(),
            serial.into(),
            verb.into(),
            source.as_os_str().into(),
            target.as_os_str().into(),
        ];
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
    let text = String::from_utf8(output.stdout).context("ADB 返回了非 UTF-8 文本")?;
    let err = String::from_utf8_lossy(&output.stderr);
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
    if let (Some(mut stdin), Some(input)) = (stdin, input) {
        let _ = stdin.write_all(&input);
    }
    let start = Instant::now();
    const BACKOFF_MS: [u64; 6] = [1, 2, 4, 8, 15, 25];
    let mut poll_step = 0usize;
    let status = loop {
        match child.try_wait()? {
            Some(status) => break Some(status),
            None if start.elapsed() >= timeout => {
                #[cfg(unix)]
                unsafe {
                    libc::kill(-(child.id() as i32), libc::SIGKILL);
                }
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            None => {
                let wait_ms = BACKOFF_MS[poll_step.min(BACKOFF_MS.len() - 1)];
                poll_step = poll_step.saturating_add(1);
                thread::park_timeout(Duration::from_millis(wait_ms));
            }
        }
    };
    let stdout = out
        .join()
        .map_err(|_| anyhow::anyhow!("读取工具输出失败"))??;
    let stderr = err
        .join()
        .map_err(|_| anyhow::anyhow!("读取工具错误失败"))??;
    let status = status.context("操作超时；请检查手机连接，然后重试。")?;
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
    let exe_dir = std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf));
    let installed = exe_dir
        .as_ref()
        .map(|p| p.join("tools/bin").join(exe(name)))
        .filter(|p| p.is_file());
    let macos_bundle = exe_dir
        .as_ref()
        .and_then(|p| p.parent())
        .map(|p| p.join("Resources/tools/bin").join(exe(name)))
        .filter(|p| p.is_file());
    installed.or(macos_bundle).or_else(|| {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../.tools/bin")
            .join(exe(name));
        path.is_file().then_some(path)
    })
}

pub fn platform_tool_candidate(name: &str) -> Option<PathBuf> {
    let binary = exe(name);
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .map(PathBuf::from);
    let local_appdata = std::env::var_os("LOCALAPPDATA").map(PathBuf::from);
    let mut candidates = Vec::new();
    if let Some(home) = &home {
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
