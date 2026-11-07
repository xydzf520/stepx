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
struct QueueLock(fs::File);
impl Drop for QueueLock {fn drop(&mut self){let _=self.0.unlock();}}
fn queue_lock(root:&Path)->Result<QueueLock>{
    fs::create_dir_all(root)?;
    ensure!(!fs::symlink_metadata(root)?.file_type().is_symlink(),"投递目录不能是链接");
    let path=root.join("queue.lock");
    if path.exists(){ensure!(!fs::symlink_metadata(&path)?.file_type().is_symlink(),"投递锁不能是链接");}
    let f=fs::OpenOptions::new().create(true).truncate(false).read(true).write(true).open(path)?;
    f.try_lock().context("投递队列正在由另一进程更新，请稍后重试")?;Ok(QueueLock(f))
}
/// Only abandoned staging directories are disposable. Pending files and originals
/// remain protected, including when delivery has failed or the phone is offline.
pub fn clean_staging()->Result<u64>{clean_staging_in(&root(),std::time::SystemTime::now())}
fn clean_staging_in(root:&Path,now:std::time::SystemTime)->Result<u64>{
    let _lock=queue_lock(root)?;let entries=read(root)?;let mut bytes=0;
    for item in fs::read_dir(root)? {
        let item=item?;let id=item.file_name().to_string_lossy().into_owned();
        if id.len()!=36 || !id.chars().all(|c|c.is_ascii_hexdigit()||c=='-') || entries.iter().any(|v|v.id==id&&!v.done){continue;}
        let meta=fs::symlink_metadata(item.path())?;
        if !meta.is_dir() || meta.file_type().is_symlink() || now.duration_since(meta.modified()?).unwrap_or_default().as_secs()<7*86400 {continue;}
        // Refuse recursive deletion if an unexpected child or symlink appeared.
        let children=fs::read_dir(item.path())?.collect::<std::io::Result<Vec<_>>>()?;
        if children.iter().any(|p|p.file_type().map_or(true,|t|!t.is_file()||t.is_symlink())){continue;}
        let size:u64=children.iter().map(|p|p.metadata().map_or(0,|m|m.len())).sum();
        fs::remove_dir_all(item.path())?;bytes+=size;
    }
    Ok(bytes)
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}
fn read(root: &Path) -> Result<Vec<Delivery>> {
    let path = root.join("queue.json");
    Ok(crate::data_store::read(&path, 2 * 1024 * 1024)?.unwrap_or_default())
}
#[cfg(test)]
fn save(root: &Path, entries: &[Delivery]) -> Result<()> {
    fs::create_dir_all(root)?;
    crate::data_store::write(&root.join("queue.json"), entries)?;
    Ok(())
}
fn save_changes(root: &Path, entries: &[Delivery], removed: &[String]) -> Result<()> {
    fs::create_dir_all(root)?;
    let path = root.join("queue.json");
    if !crate::data_store::ensure_imported(&path, 2 * 1024 * 1024)? { crate::data_store::write(&path, &Vec::<Delivery>::new())?; }
    let head = crate::data_store::read_head(&path)?.context("投递队列不存在")?;
    let changes = entries.iter().map(|entry| Ok(crate::data_store::RecordChange {
        section: "$root", id: &entry.id, position: None, value: serde_json::to_value(entry)?,
    })).collect::<Result<Vec<_>>>()?;
    let deletes = removed.iter().map(|id|("$root",id.as_str())).collect::<Vec<_>>();
    crate::data_store::apply_delta(&path, head.revision, None, &changes, &deletes)?;
    Ok(())
}
pub fn enqueue(workspace: &Path, paths: &[PathBuf], serial: &str) -> Result<usize> {
    let _g = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    enqueue_in(&root(), workspace, paths, serial)
}
fn enqueue_in(root: &Path, workspace: &Path, paths: &[PathBuf], serial: &str) -> Result<usize> {
    let _lock=queue_lock(root)?;
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
        // Windows FlushFileBuffers requires a writable handle.
        fs::OpenOptions::new().write(true).open(&cached)?.sync_all()?;
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
        save_changes(root, &entries[entries.len()-1..], &[])?;
        count += 1;
    }
    // Keep bounded history without removing pending work.
    if entries.len() > 400 {
        let mut done = entries.iter().filter(|e| e.done).count();
        let mut removed = vec![];
        entries.retain(|e| {
            if e.done && done > 200 {
                done -= 1;
                removed.push(e.id.clone());
                false
            } else {
                true
            }
        });
        save_changes(root, &[], &removed)?;
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
    let _lock=queue_lock(&root)?;
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
                save_changes(&root, std::slice::from_ref(item), &[])?;
                let _ = fs::remove_dir_all(path.parent().unwrap());
                delivered += 1;
            }
            Err(e) => {
                item.error = format!("{e:#}");
                save_changes(&root, std::slice::from_ref(item), &[])?;
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
    let _lock=queue_lock(&root)?;
    let mut entries = read(&root)?;
    for e in &mut entries {
        if !e.done {
            e.attempt = 0;
            e.error.clear();
        }
    }
    let changed: Vec<_> = entries.into_iter().filter(|entry| !entry.done).collect();
    save_changes(&root, &changed, &[])
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
    fn delivery_progress_commits_only_the_changed_delivery() {
        let dir = tempfile::tempdir().unwrap();
        let original: Vec<_> = (0..20).map(|i|Delivery {id:format!("file-{i}"),serial:"phone".into(),name:format!("{i}.txt"),path:dir.path().join(format!("{i}.txt")),sha256:"digest".into(),size:20,done:false,error:String::new(),attempt:0}).collect();
        save(dir.path(),&original).unwrap();
        let db = crate::data_store::database_for(&dir.path().join("queue.json"));
        let before = crate::data_store::with_connection(&db,|c|Ok(c.total_changes())).unwrap();
        let mut changed = original[8].clone(); changed.done=true; changed.attempt=120;
        save_changes(dir.path(),&[changed],&[]).unwrap();
        let after = crate::data_store::with_connection(&db,|c|Ok(c.total_changes())).unwrap();
        assert_eq!(after-before,2,"one delivery and the shared snapshot revision");
        let saved = read(dir.path()).unwrap();
        assert_eq!(saved.len(),20);
        assert!(saved[8].done);
        assert!(saved.iter().enumerate().filter(|(i,_)|*i!=8).all(|(_,entry)|!entry.done&&entry.attempt==0));
    }
    #[test]
    fn staging_cleanup_keeps_pending_originals_and_unknown_directories(){
        let tmp=tempfile::tempdir().unwrap();let root=tmp.path();let pending=crate::companion_link::uuid();let orphan=crate::companion_link::uuid();
        for id in [&pending,&orphan,"user-folder"]{fs::create_dir(root.join(id)).unwrap();fs::write(root.join(id).join("document.pdf"),b"data").unwrap();}
        save(root,&[Delivery{id:pending.clone(),serial:"phone".into(),name:"document.pdf".into(),path:root.join("original.pdf"),sha256:"hash".into(),size:4,done:false,error:"offline".into(),attempt:0}]).unwrap();
        let future=std::time::SystemTime::now()+std::time::Duration::from_secs(8*86400);
        assert_eq!(clean_staging_in(root,future).unwrap(),4);
        assert!(root.join(pending).join("document.pdf").is_file());assert!(!root.join(orphan).exists());assert!(root.join("user-folder").is_dir());
    }
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
