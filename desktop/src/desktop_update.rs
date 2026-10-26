//! Desktop GitHub releases are independent of phone OTA. All I/O is off the UI thread.
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Stdio,
    sync::mpsc,
};
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Release {
    pub version: String,
    pub tag: String,
    pub url: String,
    pub name: String,
    pub size: u64,
    pub sha256: String,
    pub checksum_url: Option<String>,
    pub notes: String,
    pub page: String,
    pub platform: String,
}
#[derive(Clone, Debug, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Release {
        release: Release,
    },
    Progress {
        phase: String,
        received: u64,
        total: u64,
        speed: u64,
    },
    Ready {
        job: PathBuf,
    },
    Prepared {
        job: PathBuf,
    },
    Cancelled,
    Error {
        message: String,
    },
}
pub struct Job {
    pub directory: PathBuf,
    receiver: mpsc::Receiver<Event>,
}
impl Job {
    pub fn poll(&self) -> Option<Event> {
        self.receiver.try_recv().ok()
    }
    pub fn cancel(&self) {
        let _ = std::fs::write(self.directory.join("cancel"), b"");
    }
}
pub fn platform() -> String {
    let arch = match std::env::consts::ARCH {
        "aarch64" => "arm64",
        v => v,
    };
    format!("{}-{arch}", std::env::consts::OS)
}
pub fn newer(release: &Release) -> Result<bool> {
    Ok(
        crate::updates::compare_versions(&release.version, crate::build_info::VERSION)?
            == std::cmp::Ordering::Greater,
    )
}
pub fn state_root() -> PathBuf {
    crate::resources::user_state().join("desktop-updates")
}
fn job_dir() -> Result<PathBuf> {
    let root = state_root();
    std::fs::create_dir_all(&root)?;
    let path = root.join(format!("job-{:032x}", rand::random::<u128>()));
    std::fs::create_dir(&path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700))?;
    }
    Ok(path)
}
fn spawn(op: &str, data: serde_json::Value, directory: PathBuf) -> Result<Job> {
    let helper = directory.join("updater.py");
    std::fs::write(&helper, include_str!("desktop_update.py"))?;
    let mut cmd = crate::resources::python_command();
    cmd.arg(helper)
        .arg(op)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = cmd
        .spawn()
        .context("无法启动更新工具，请检查安装包的 Python 运行时")?;
    let mut input = child.stdin.take().context("无法写入更新任务")?;
    input.write_all(&serde_json::to_vec(&data)?)?;
    drop(input);
    let stdout = child.stdout.take().unwrap();
    let (tx, receiver) = mpsc::channel();
    let is_check = op == "check";
    let dir_for_cleanup = directory.clone();
    std::thread::spawn(move || {
        let mut terminal = false;
        for line in BufReader::new(stdout).lines() {
            let event = line
                .ok()
                .and_then(|s| serde_json::from_str::<Event>(&s).ok());
            if let Some(event) = event {
                terminal |= !matches!(event, Event::Progress { .. });
                if tx.send(event).is_err() {
                    let _ = child.kill();
                    break;
                }
            }
        }
        let _ = child.wait();
        if !terminal {
            let _ = tx.send(Event::Error {
                message: "更新工具意外退出，请重试".into(),
            });
        }
        if is_check {
            let _ = std::fs::remove_dir_all(&dir_for_cleanup);
        }
    });
    Ok(Job {
        directory,
        receiver,
    })
}
pub fn check() -> Result<Job> {
    spawn(
        "check",
        serde_json::json!({"platform":platform()}),
        job_dir()?,
    )
}
pub fn download(release: &Release) -> Result<Job> {
    ensure!(newer(release)?, "当前版本无需更新，禁止降级安装");
    let dir = job_dir()?;
    spawn(
        "download",
        serde_json::json!({"release":release,"job":dir}),
        dir,
    )
}
pub fn remember_ready(directory: &Path) -> Result<()> {
    ensure!(directory.starts_with(state_root()), "更新暂存位置无效");
    atomic_write(
        &state_root().join("last-ready.json"),
        &serde_json::json!({"job":directory}),
    )
}
pub fn cached_ready() -> Option<(PathBuf, Release)> {
    let value: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state_root().join("last-ready.json")).ok()?).ok()?;
    let dir = PathBuf::from(value["job"].as_str()?);
    if !dir.starts_with(state_root()) || !dir.join("package.part").is_file() {
        return None;
    }
    let ready: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("ready.json")).ok()?).ok()?;
    let r: Release = serde_json::from_value(ready["release"].clone()).ok()?;
    (newer(&r).ok()? && r.platform == platform()).then_some((dir, r))
}
pub fn prepare(directory: &Path) -> Result<Job> {
    ensure!(directory.starts_with(state_root()), "更新暂存位置无效");
    let ready: serde_json::Value =
        serde_json::from_slice(&std::fs::read(directory.join("ready.json"))?)?;
    let release: Release = serde_json::from_value(ready["release"].clone())?;
    ensure!(newer(&release)?, "禁止安装相同或更低版本");
    atomic_write(
        &state_root().join("last-job.json"),
        &serde_json::json!({"job":directory}),
    )?;
    spawn(
        "prepare",
        serde_json::json!({"job":directory,"executable":std::env::current_exe()?,"pid":std::process::id()}),
        directory.into(),
    )
}
fn atomic_write(path: &Path, value: &serde_json::Value) -> Result<()> {
    let temp = path.with_extension("tmp");
    std::fs::write(&temp, serde_json::to_vec(value)?)?;
    // Windows rename does not replace an existing file. Receipts are unique per task.
    if path.exists() {
        std::fs::remove_file(path)?;
    }
    std::fs::rename(temp, path)?;
    Ok(())
}
/// Called after the native window has initialized, never merely on process start.
pub fn acknowledge_startup() -> Result<()> {
    let args: Vec<_> = std::env::args_os().collect();
    let Some(i) = args.iter().position(|a| a == "--update-receipt") else {
        return Ok(());
    };
    let path = PathBuf::from(args.get(i + 1).context("缺少更新回执")?);
    let nonce = args
        .iter()
        .position(|a| a == "--update-nonce")
        .and_then(|i| args.get(i + 1))
        .context("缺少更新标识")?
        .to_string_lossy();
    ensure!(
        path.starts_with(state_root()) && path.file_name().is_some_and(|v| v == "install.json"),
        "更新回执位置无效"
    );
    let config: serde_json::Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    ensure!(config["nonce"].as_str() == Some(&nonce), "更新标识不匹配");
    // Rolled-back versions must not falsely acknowledge the failed newer install.
    if config["version"].as_str() != Some(crate::build_info::VERSION) {
        return Ok(());
    }
    let exe = PathBuf::from(config["target"].as_str().context("缺少更新目标")?)
        .join(config["relative_exe"].as_str().context("缺少启动文件")?);
    ensure!(
        exe.canonicalize()? == std::env::current_exe()?.canonicalize()?,
        "更新启动路径不匹配"
    );
    atomic_write(
        &path.with_file_name("ack.json"),
        &serde_json::json!({"nonce":nonce,"version":crate::build_info::VERSION}),
    )
}
pub fn last_result() -> Option<String> {
    let job: serde_json::Value =
        serde_json::from_slice(&std::fs::read(state_root().join("last-job.json")).ok()?).ok()?;
    let dir = PathBuf::from(job["job"].as_str()?);
    if !dir.starts_with(state_root()) {
        return None;
    }
    let status: serde_json::Value =
        serde_json::from_slice(&std::fs::read(dir.join("status.json")).ok()?).ok()?;
    match status["state"].as_str()? {
        "installed" => Some(format!("已更新至 v{}", status["version"].as_str()?)),
        "rolled_back" | "failed" => status["message"].as_str().map(str::to_owned),
        _ => None,
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn maps_release_platform_names() {
        assert!(!platform().contains("aarch64"));
    }
    #[test]
    fn parses_progress_and_completion() {
        let e: Event = serde_json::from_str(
            r#"{"kind":"progress","phase":"download","received":120,"total":240,"speed":20}"#,
        )
        .unwrap();
        assert!(matches!(
            e,
            Event::Progress {
                received: 120,
                total: 240,
                ..
            }
        ));
        assert!(serde_json::from_str::<Event>(r#"{"kind":"ready"}"#).is_err());
    }
}
