//! Durable, opt-in delivery of Work results. Never scans or mirrors a project.
use crate::{adb::Adb, device::Device};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
static LOCK: Mutex<()> = Mutex::new(());
#[derive(Clone, Serialize, Deserialize)]
struct Delivery {
    id: String,
    serial: String,
    name: String,
    path: PathBuf,
    sha256: String,
    size: u64,
    done: bool,
    error: String,
    attempt: u64,
}
fn root() -> PathBuf {
    crate::resources::user_state().join("stepx-companion/work-delivery")
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn read(root: &Path) -> Result<Vec<Delivery>> {
    let path = root.join("queue.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    let b = fs::read(path)?;
    ensure!(b.len() < 2 * 1024 * 1024, "传输记录过大");
    Ok(serde_json::from_slice(&b)?)
}
fn save(root: &Path, entries: &[Delivery]) -> Result<()> {
    fs::create_dir_all(root)?;
    let file = tempfile::NamedTempFile::new_in(root)?;
    serde_json::to_writer(file.as_file(), entries)?;
    file.as_file().sync_all()?;
    file.persist(root.join("queue.json"))?;
    Ok(())
}
pub fn enqueue(workspace: &Path, paths: &[PathBuf], serial: &str) -> Result<usize> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    enqueue_in(&root(), workspace, paths, serial)
}
fn enqueue_in(root: &Path, workspace: &Path, paths: &[PathBuf], serial: &str) -> Result<usize> {
    ensure!(!serial.is_empty(), "未指定接收手机");
    let _ = fs::create_dir_all(workspace);
    let workspace = workspace.canonicalize()?;
    let output = workspace.join("Amoo Work Outputs");
    ensure!(
        !fs::symlink_metadata(&output)?.file_type().is_symlink(),
        "交付文件夹不能是链接"
    );
    let output = output.canonicalize()?;
    ensure!(output.starts_with(&workspace), "交付目录越界");
    let mut entries = read(root)?;
    let mut count = 0;
    for path in paths {
        ensure!(
            entries.iter().filter(|e| !e.done).count() < 200,
            "待发送文件超过 200 个，请先完成传输"
        );
        ensure!(fs::symlink_metadata(path)?.is_file(), "仅发送普通交付文件");
        let path = path.canonicalize()?;
        ensure!(
            path.starts_with(&output),
            "仅自动发送 Amoo Work Outputs 中的交付成果"
        );
        let name = path
            .file_name()
            .and_then(|n| n.to_str())
            .context("无效文件名")?
            .to_owned();
        let size = fs::metadata(&path)?.len();
        ensure!(size <= 256 * 1024 * 1024, "单个成果超过 256 MB，请手动传输");
        let hash = crate::companion_link::digest(&path)?;
        if entries
            .iter()
            .any(|e| e.serial == serial && e.path == path && e.sha256 == hash)
        {
            continue;
        }
        let id = crate::companion_link::uuid();
        let cache = root.join(&id);
        fs::create_dir_all(&cache)?;
        let cached = cache.join(&name);
        fs::copy(&path, &cached)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(root, fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(&cache, fs::Permissions::from_mode(0o700))?;
            fs::set_permissions(&cached, fs::Permissions::from_mode(0o600))?;
        }
        if crate::companion_link::digest(&cached)? != hash {
            let _ = fs::remove_dir_all(cache);
            anyhow::bail!("成果在准备期间发生变化，请稍后重试");
        }
        entries.push(Delivery {
            id,
            serial: serial.into(),
            name,
            path,
            sha256: hash,
            size,
            done: false,
            error: String::new(),
            attempt: 0,
        });
        // Save each staged file before another is copied; recoverable after interruption.
        save(root, &entries)?;
        count += 1;
    }
    // Keep bounded history without removing pending work.
    if entries.len() > 400 {
        let mut done = entries.iter().filter(|e| e.done).count();
        entries.retain(|e| {
            if e.done && done > 200 {
                done -= 1;
                false
            } else {
                true
            }
        });
        save(root, &entries)?;
    }
    Ok(count)
}
pub fn process(adb: &Adb, device: &Device) -> Result<usize> {
    if !crate::settings::load().sync_work_outputs {
        return Ok(0);
    }
    let Ok(_g) = LOCK.try_lock() else {
        return Ok(0);
    };
    let root = root();
    let mut entries = read(&root)?;
    let now_ts = now();
    let indices: Vec<usize> = entries
        .iter()
        .enumerate()
        .filter(|(_, e)| {
            !e.done && e.serial == device.serial && now_ts.saturating_sub(e.attempt) > 60
        })
        .map(|(idx, _)| idx)
        .take(8)
        .collect();
    if indices.is_empty() {
        return Ok(0);
    }
    let mut delivered = 0usize;
    let mut last_err = None;
    for index in indices {
        let item = &mut entries[index];
        let path = root.join(&item.id).join(&item.name);
        item.attempt = now();
        let result = crate::companion_link::send_staged(
            adb,
            device,
            &path,
            &item.id,
            &item.sha256,
            item.size,
        );
        match result {
            Ok(()) => {
                item.done = true;
                item.error.clear();
                save(&root, &entries)?;
                let _ = fs::remove_dir_all(path.parent().unwrap());
                delivered += 1;
            }
            Err(e) => {
                item.error = format!("{e:#}");
                save(&root, &entries)?;
                last_err = Some(e);
                break;
            }
        }
    }
    if delivered == 0 {
        if let Some(e) = last_err {
            return Err(e);
        }
    }
    Ok(delivered)
}
pub fn retry() -> Result<()> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let root = root();
    let mut entries = read(&root)?;
    for e in &mut entries {
        if !e.done {
            e.attempt = 0;
            e.error.clear();
        }
    }
    save(&root, &entries)
}
pub fn summary() -> Option<String> {
    let _g = LOCK.try_lock().ok()?;
    let entries = read(&root()).ok()?;
    let done = entries.iter().filter(|e| e.done).count();
    let pending = entries.iter().filter(|e| !e.done).count();
    let error = entries.iter().find(|e| !e.done && !e.error.is_empty());
    Some(if let Some(e) = error {
        format!("待发送 {pending} 个 · {}：{}", e.name, e.error)
    } else if entries.is_empty() {
        "还没有自动发送记录".into()
    } else {
        format!("已送达 {done} 个 · 待发送 {pending} 个")
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn stages_only_outputs_deduplicates_and_preserves_original() {
        let t = tempfile::tempdir().unwrap();
        let w = t.path().join("project");
        let o = w.join("Amoo Work Outputs/run");
        fs::create_dir_all(&o).unwrap();
        let p = o.join("报告.txt");
        fs::write(&p, "成果").unwrap();
        let root = t.path().join("queue");
        assert_eq!(enqueue_in(&root, &w, &[p.clone()], "phone-a").unwrap(), 1);
        assert_eq!(enqueue_in(&root, &w, &[p.clone()], "phone-a").unwrap(), 0);
        let entries = read(&root).unwrap();
        fs::write(&p, "后来修改").unwrap();
        assert_eq!(
            fs::read_to_string(root.join(&entries[0].id).join(&entries[0].name)).unwrap(),
            "成果"
        );
        assert_eq!(fs::read_to_string(&p).unwrap(), "后来修改");
        let source = w.join("secret.env");
        fs::write(&source, "secret").unwrap();
        assert!(enqueue_in(&root, &w, &[source], "phone-a").is_err());
        assert_eq!(read(&root).unwrap().len(), 1);
    }
    #[cfg(unix)]
    #[test]
    fn rejects_linked_outputs() {
        let t = tempfile::tempdir().unwrap();
        let w = t.path().join("project");
        let o = w.join("Amoo Work Outputs");
        fs::create_dir_all(&o).unwrap();
        let secret = t.path().join("key");
        fs::write(&secret, "private").unwrap();
        let link = o.join("link");
        std::os::unix::fs::symlink(&secret, &link).unwrap();
        assert!(enqueue_in(&t.path().join("queue"), &w, &[link], "phone").is_err());
    }
}
