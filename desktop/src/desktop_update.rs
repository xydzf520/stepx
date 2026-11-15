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
fn prune_old_job_dirs(root: &Path, keep: &[PathBuf]) {
    let mut keep_canonical: Vec<PathBuf> = keep.to_vec();
    for p in keep {
        if let Ok(canon) = p.canonicalize() {
            keep_canonical.push(canon);
        }
    }
    for pointer in ["last-ready.json", "last-job.json"] {
        if let Ok(raw) = std::fs::read(root.join(pointer)) {
            if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&raw) {
                if let Some(dir_str) = val["job"].as_str() {
                    let p = PathBuf::from(dir_str);
                    if let Ok(canon) = p.canonicalize() {
                        keep_canonical.push(canon);
                    }
                    keep_canonical.push(p);
                }
            }
        }
    }
    let Ok(entries) = std::fs::read_dir(root) else {
        return;
    };
    let mut candidates: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        if !name.starts_with("job-") {
            continue;
        }
        let Ok(meta) = std::fs::symlink_metadata(&path) else {
            continue;
        };
        if !meta.is_dir() || meta.file_type().is_symlink() {
            continue;
        }
        let is_kept = keep_canonical.iter().any(|k| {
            k == &path || path.canonicalize().ok().as_ref() == Some(k)
        });
        if is_kept {
            continue;
        }
        let mtime = meta.modified().unwrap_or(std::time::UNIX_EPOCH);
        candidates.push((mtime, path));
    }
    candidates.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, old_path) in candidates.into_iter().skip(1) {
        let _ = std::fs::remove_dir_all(old_path);
    }
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
    prune_old_job_dirs(&root, std::slice::from_ref(&path));
    Ok(path)
}
const CHECK_LIVENESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(45);
const LIVENESS_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(180);

struct ChildGuard(Option<std::process::Child>);

impl ChildGuard {
    fn kill_and_wait(&mut self) {
        let Some(mut child) = self.0.take() else {
            return;
        };
        if matches!(child.try_wait(), Ok(Some(_))) {
            return;
        }
        #[cfg(windows)]
        {
            let mut tk = std::process::Command::new("taskkill");
            crate::resources::quiet_command(&mut tk);
            let _ = tk
                .args(["/PID", &child.id().to_string(), "/T", "/F"])
                .stdin(Stdio::null())
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .output();
        }
        #[cfg(unix)]
        unsafe {
            libc::kill(-(child.id() as i32), libc::SIGKILL);
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    fn wait(&mut self) {
        if let Some(mut child) = self.0.take() {
            let _ = child.wait();
        }
    }
}

impl Drop for ChildGuard {
    fn drop(&mut self) {
        self.kill_and_wait();
    }
}

fn spawn(op: &str, data: serde_json::Value, directory: PathBuf) -> Result<Job> {
    let payload = serde_json::to_vec(&data)?;
    let helper = directory.join("updater.py");
    std::fs::write(&helper, include_str!("desktop_update.py"))?;
    let mut cmd = crate::resources::python_command();
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        cmd.process_group(0);
    }
    cmd.arg(helper)
        .arg(op)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let child = cmd
        .spawn()
        .context("无法启动更新工具，请检查安装包的 Python 运行时")?;
    let mut guard = ChildGuard(Some(child));
    let mut input = guard
        .0
        .as_mut()
        .and_then(|c| c.stdin.take())
        .context("无法写入更新任务")?;
    let stdout = guard
        .0
        .as_mut()
        .and_then(|c| c.stdout.take())
        .context("无法读取更新进度输出")?;
    if let Err(err) = input.write_all(&payload) {
        if err.kind() != std::io::ErrorKind::BrokenPipe {
            return Err(err).context("写入更新任务参数失败");
        }
    }
    drop(input);
    let (tx, receiver) = mpsc::channel();
    let is_check = op == "check";
    let dir_for_cleanup = directory.clone();
    std::thread::spawn(move || {
        let (line_tx, line_rx) = mpsc::channel();
        std::thread::spawn(move || {
            for line in BufReader::new(stdout).lines() {
                if line_tx.send(line).is_err() {
                    break;
                }
            }
        });
        let mut terminal = false;
        let mut last_activity = std::time::Instant::now();
        loop {
            match line_rx.recv_timeout(std::time::Duration::from_millis(500)) {
                Ok(line) => {
                    last_activity = std::time::Instant::now();
                    let event = line
                        .ok()
                        .and_then(|s| serde_json::from_str::<Event>(&s).ok());
                    if let Some(event) = event {
                        terminal |= !matches!(event, Event::Progress { .. });
                        if tx.send(event).is_err() {
                            guard.kill_and_wait();
                            break;
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    let liveness_limit = if is_check {
                        CHECK_LIVENESS_TIMEOUT
                    } else {
                        LIVENESS_TIMEOUT
                    };
                    if last_activity.elapsed() >= liveness_limit {
                        guard.kill_and_wait();
                        let _ = tx.send(Event::Error {
                            message: "更新连接响应超时，已保留下载进度，请重试".into(),
                        });
                        terminal = true;
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    guard.wait();
                    break;
                }
            }
        }
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

    #[test]
    fn install_liveness_timeout_is_180s_and_prunes_old_jobs() {
        assert_eq!(LIVENESS_TIMEOUT, std::time::Duration::from_secs(180));
        let root = std::env::temp_dir().join(format!(
            "stepx-desktop-update-test-{:016x}",
            rand::random::<u64>()
        ));
        std::fs::create_dir_all(&root).unwrap();
        let j1 = root.join("job-0001");
        let j2 = root.join("job-0002");
        let j3 = root.join("job-0003");
        let j_active = root.join("job-active");
        for d in [&j1, &j2, &j3, &j_active] {
            std::fs::create_dir_all(d).unwrap();
            std::thread::sleep(std::time::Duration::from_millis(5));
        }
        std::fs::write(
            root.join("last-ready.json"),
            serde_json::to_vec(&serde_json::json!({"job": j1})).unwrap(),
        )
        .unwrap();
        prune_old_job_dirs(&root, std::slice::from_ref(&j_active));
        assert!(j_active.is_dir(), "active job must remain");
        assert!(j1.is_dir(), "last-ready referenced job must remain");
        assert!(j3.is_dir(), "latest unreferenced job must remain");
        assert!(!j2.exists(), "older unreferenced job must be pruned");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn python_updater_enforces_direct_tls_metadata_os_signing_heartbeat_and_atomic_rollback() {
        let script = r#"
import importlib.util, io, json, os, subprocess, sys, tempfile
from pathlib import Path
from unittest.mock import patch

root = Path(sys.argv[1])
spec = importlib.util.spec_from_file_location('updater', root / 'src/desktop_update.py')
u = importlib.util.module_from_spec(spec)
spec.loader.exec_module(u)

# 1. API metadata and .sha256 sidecars must NEVER use third-party proxies
assert u.candidate_urls(u.API, allow_mirrors=False) == [u.API]
sha_url = u.REPO + '/releases/download/v0.2.19/StepX-Companion-0.2.19-linux-x86_64.tar.gz.sha256'
assert u.candidate_urls(sha_url, allow_mirrors=False) == [sha_url]
assert u.candidate_urls(sha_url, allow_mirrors=True) == [sha_url]
for bad in [
    'https://ghfast.top/' + u.API,
    'https://gh-proxy.com/' + u.API,
    'https://ghfast.top/' + sha_url,
    'https://gh-proxy.com/' + sha_url,
]:
    try:
        u.trusted_url(bad)
        raise AssertionError('should reject mirrored metadata/checksum: ' + bad)
    except ValueError:
        pass

pkg_url = u.REPO + '/releases/download/v0.2.19/StepX-Companion-0.2.19-linux-x86_64.tar.gz'
assert u.candidate_urls(pkg_url, allow_mirrors=False) == [pkg_url]
mirrored_cands = u.candidate_urls(pkg_url, allow_mirrors=True)
assert mirrored_cands[0] == pkg_url and len(mirrored_cands) >= 2

# 2. Reject archive download when expected_sha256 is missing
with tempfile.TemporaryDirectory() as tmp:
    try:
        u.download({'version': '0.2.19', 'name': 'StepX-Companion-0.2.19-linux-x86_64.tar.gz', 'url': pkg_url, 'size': 16, 'sha256': '', 'checksum_url': None, 'platform': 'linux-x86_64'}, tmp)
        raise AssertionError('must reject empty sha256')
    except ValueError:
        pass

# 3. macOS ad-hoc signature rejected unless local dev override
def fake_adhoc_run(cmd, **kw):
    class R:
        returncode = 0
        stdout = ''
        stderr = 'TeamIdentifier=not set\nSignature=adhoc\n'
    return R()

with patch.object(u.subprocess, 'run', side_effect=fake_adhoc_run):
    try:
        u.verify_os_signature(Path('/tmp/Fake.app'), Path('Contents/MacOS/stepx-companion'), 'macos-arm64')
        raise AssertionError('must reject adhoc macOS signature')
    except ValueError:
        pass

# 4. Windows Authenticode invalid signature rejected when signed
def fake_win_run(cmd, **kw):
    class R:
        returncode = 0
        stdout = json.dumps({'Status': 'HashMismatch', 'StatusMessage': 'tampered', 'HasCert': True})
        stderr = ''
    return R()

with patch.dict(os.environ, {'STEPX_FORCE_AUTHENTICODE_CHECK': '1'}), \
     patch.object(u.shutil, 'which', return_value='/usr/bin/pwsh'), \
     patch.object(u.subprocess, 'run', side_effect=fake_win_run):
    try:
        u.verify_os_signature(Path('/tmp/stage'), Path('stepx-companion.exe'), 'windows-x86_64')
        raise AssertionError('must reject invalid Authenticode signature')
    except ValueError:
        pass

# 5. Heartbeat emits periodic JSON progress lines
lines = []
with patch.object(u, 'emit', side_effect=lambda k, **d: lines.append((k, d))):
    with u.heartbeat('extract', received=10, total=20, interval=0.02):
        import time
        time.sleep(0.06)
assert any(k == 'progress' and d.get('phase') == 'extract' for k, d in lines), lines

# 6. Atomic rollback if second os.replace fails + backup/job pruning on success
with tempfile.TemporaryDirectory() as tmp:
    tp = Path(tmp)
    target = tp / 'StepX'; target.mkdir(); (target / 'app.txt').write_text('v1')
    stage = tp / '.stepx-stage-1'; stage.mkdir(); (stage / 'app.txt').write_text('v2')
    backup = tp / 'StepX.backup-2'
    old_backup = tp / 'StepX.backup-1'; old_backup.mkdir(); (old_backup / 'app.txt').write_text('v0')
    old_job = tp / 'job-old'; old_job.mkdir()
    cur_job = tp / 'job-cur'; cur_job.mkdir()
    calls = {'n': 0}
    real_replace = os.replace
    def fail_second_replace(src, dst):
        calls['n'] += 1
        if calls['n'] == 2:
            raise OSError('simulated stage->target failure')
        return real_replace(src, dst)
    cfg = dict(target=str(target), stage=str(stage), backup=str(backup), job=str(cur_job), pid=99999999, version='0.2.19', relative_exe='stepx-companion', nonce='n1')
    with patch.object(u.os, 'replace', side_effect=fail_second_replace), patch.object(u, 'launch'):
        ok = u.install(cfg, wait_seconds=1, ack_seconds=1)
    assert not ok
    assert (target / 'app.txt').read_text() == 'v1', 'must roll back backup->target'

    # Now verify pruning keeps latest 1 backup and latest 1 job
    stage2 = tp / '.stepx-stage-2'; stage2.mkdir(); (stage2 / 'app.txt').write_text('v2')
    backup2 = tp / 'StepX.backup-2'; backup2.mkdir(); (backup2 / 'app.txt').write_text('v1')
    u.prune_old_backups_and_jobs(target, backup2, cur_job)
    assert backup2.is_dir() and not old_backup.exists()
    assert cur_job.is_dir() and not old_job.exists()

# 7. Windows ota_transfer.py parent exit check uses OpenProcess + WaitForSingleObject
ota_spec = importlib.util.spec_from_file_location('ota', root / 'src/ota_transfer.py')
ota = importlib.util.module_from_spec(ota_spec)
ota_spec.loader.exec_module(ota)
assert not ota._parent_exited(os.getppid())
"#;
        let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let out = crate::resources::python_command()
            .args(["-c", script])
            .arg(&manifest_dir)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "python updater test failed: stdout={} stderr={}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
    }
}
