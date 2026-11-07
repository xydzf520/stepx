//! Conservative lifecycle management: records, originals and pending work are
//! protected. Only explicitly rebuildable previews and abandoned staging expire.
use anyhow::{ensure, Context, Result};
use std::{
    fs,
    path::{Path, PathBuf},
    time::{Duration, SystemTime},
};

struct Lock(fs::File);
impl Drop for Lock {
    fn drop(&mut self) {
        let _ = self.0.unlock();
    }
}
fn lock(root: &Path) -> Result<Lock> {
    fs::create_dir_all(root)?;
    let file = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(root.join("maintenance.lock"))?;
    file.try_lock().context("数据维护正在进行，请稍后重试")?;
    Ok(Lock(file))
}
pub fn database_bytes(db: &Path) -> u64 {
    [
        db.to_path_buf(),
        PathBuf::from(format!("{}-wal", db.display())),
        PathBuf::from(format!("{}-shm", db.display())),
    ]
    .iter()
    .map(|p| fs::metadata(p).map_or(0, |m| m.len()))
    .sum()
}
/// Explicit allowlist; model keys, permissions and unrelated JSON never enter
/// the business database or its automatic backups.
pub fn legacy_paths(state: &Path) -> Result<Vec<PathBuf>> {
    let mut paths = vec![
        state.join("diagnostics/operations.json"),
        state.join("stepx-companion/work-delivery/queue.json"),
    ];
    let context = state.join("stepx-companion/context");
    if context.is_dir() {
        for scope in fs::read_dir(context)? {
            let scope = scope?;
            let name = scope.file_name().to_string_lossy().into_owned();
            if !scope.file_type()?.is_dir()
                || name.len() != 32
                || !name.chars().all(|c| c.is_ascii_hexdigit())
            {
                continue;
            }
            paths.push(scope.path().join("ledger.json"));
            paths.push(scope.path().join("actions/ledger.json"));
            let pipeline = scope.path().join("pipeline");
            if pipeline.is_dir() {
                for file in fs::read_dir(pipeline)? {
                    let file = file?;
                    if file.file_type()?.is_file()
                        && file.path().extension().is_some_and(|v| v == "json")
                    {
                        paths.push(file.path());
                    }
                }
            }
        }
    }
    paths.retain(|p| p.is_file());
    paths.sort();
    Ok(paths)
}
pub fn migrate_legacy(state: &Path) -> Result<usize> {
    let paths = legacy_paths(state)?
        .into_iter()
        .map(|p| Ok((crate::data_store::has_snapshot(&p)?, p)))
        .collect::<Result<Vec<_>>>()?
        .into_iter()
        .filter_map(|(done, p)| (!done).then_some(p))
        .collect::<Vec<_>>();
    // Parse everything first, before switching any of this batch's readers.
    for path in &paths {
        let raw = crate::local_store::read_bounded(path, 128 * 1024 * 1024)?;
        let _: serde_json::Value = serde_json::from_slice(&raw)
            .with_context(|| format!("旧数据结构无效：{}", path.display()))?;
    }
    for path in &paths {
        let _: Option<serde_json::Value> = crate::data_store::read(path, 128 * 1024 * 1024)?;
    }
    Ok(paths.len())
}
pub fn run(force_backup: bool) -> Result<String> {
    let db = crate::data_store::default_path();
    let root = db.parent().context("数据目录无效")?;
    let _lock = lock(root)?;
    // Lazy module imports and this background sweep share idempotent transactions.
    migrate_legacy(&crate::resources::user_state())?;
    let now = SystemTime::now();
    let backup_root = root.join("backups");
    fs::create_dir_all(&backup_root)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&backup_root, fs::Permissions::from_mode(0o700))?;
    }
    let latest = fs::read_dir(&backup_root)?
        .filter_map(Result::ok)
        .filter(|e| e.file_name().to_string_lossy().starts_with("auto-"))
        .filter_map(|e| e.metadata().ok()?.modified().ok())
        .max();
    let mut backed_up = false;
    if db.exists()
        && (force_backup
            || latest.is_none_or(|t| {
                now.duration_since(t).unwrap_or_default() > Duration::from_secs(86400)
            }))
    {
        let name = format!(
            "{}-{}-{:08x}.sqlite3",
            if force_backup { "manual" } else { "auto" },
            chrono::Utc::now().format("%Y%m%dT%H%M%S"),
            rand::random::<u32>()
        );
        crate::data_store::backup(&db, &backup_root.join(name))?;
        backed_up = true;
        // Rotate verified automatic backups only; manual backups are user-owned.
        let mut copies = fs::read_dir(&backup_root)?
            .filter_map(Result::ok)
            .filter(|e| {
                e.file_name().to_string_lossy().starts_with("auto-")
                    && e.path().extension().is_some_and(|x| x == "sqlite3")
            })
            .collect::<Vec<_>>();
        copies.sort_by_key(|e| std::cmp::Reverse(e.file_name()));
        for item in copies.into_iter().skip(7) {
            if item.file_type()?.is_file() {
                fs::remove_file(item.path())?;
            }
        }
    }
    let text_db = crate::resources::user_state().join("cache/file-text.sqlite3");
    let mut previews = 0;
    if text_db.exists() {
        previews = crate::data_store::with_connection(&text_db, |c| {
            let exists: bool = c.query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name='file_text')",
                [],
                |r| r.get(0),
            )?;
            if !exists {
                return Ok(0);
            }
            let n=c.execute("DELETE FROM file_text WHERE seen<?1 OR path NOT IN (SELECT path FROM file_text ORDER BY seen DESC LIMIT 512)",[chrono::Utc::now().timestamp_millis()-7*86400000])?;
            c.execute_batch("PRAGMA optimize; PRAGMA wal_checkpoint(PASSIVE);")?;
            Ok(n)
        })?;
    }
    let freed = crate::work_sync::clean_staging()?;
    let (thumbnails, thumbnail_bytes) = crate::recent_files::clean_thumbnails(
        &crate::resources::user_state().join("recent-thumbnails"), now,
    )?;
    if db.exists() {
        crate::data_store::maintain(&db)?;
    }
    Ok(format!("数据库 {:.1} MB · {} · 清理 {previews} 份文字预览、{thumbnails} 份图片预览、{:.1} MB 无引用缓存与暂存。原文件、成果和待发送文件保留。",database_bytes(&db) as f64/1048576.,if backed_up{"备份已校验"}else{"备份有效"},freed.saturating_add(thumbnail_bytes) as f64/1048576.))
}

/// Offline-only recovery. A durable marker blocks ordinary opens after an
/// interrupted restore, so the client cannot create an empty replacement DB.
pub fn restore(db: &Path, source: &Path) -> Result<PathBuf> {
    ensure!(
        source.is_file() && !fs::symlink_metadata(source)?.file_type().is_symlink(),
        "请选择完整数据库备份"
    );
    let source_conn =
        rusqlite::Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let check: String = source_conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "备份完整性检查失败，未修改当前数据");
    let version: i64 = source_conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(version == 1, "备份版本不兼容，未修改当前数据");
    let tables:i64=source_conn.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('snapshots','records','migrations','assets')",[],|r|r.get(0))?;
    ensure!(tables == 4, "不是 StepX 业务数据库备份");
    let parent = db.parent().context("恢复目录无效")?;
    let _maintenance = lock(parent)?;
    let gate = fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(parent.join("database-use.lock"))?;
    gate.try_lock()
        .context("请先退出所有 StepX 客户端，再恢复备份")?;
    let _gate = Lock(gate);
    let stage = parent.join("restore-staged.sqlite3");
    // SQLite's backup API includes committed WAL pages and creates one consistent
    // snapshot even when the selected source has a WAL sidecar.
    ensure!(source != stage, "不能选择恢复过程中的暂存文件");
    if stage.exists() {
        ensure!(
            !fs::symlink_metadata(&stage)?.file_type().is_symlink(),
            "恢复暂存文件不能是链接"
        );
        fs::remove_file(&stage)?;
    }
    source_conn.backup("main", &stage, None::<fn(rusqlite::backup::Progress)>)?;
    drop(source_conn);
    let staged =
        rusqlite::Connection::open_with_flags(&stage, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let check: String = staged.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "恢复快照校验失败，未修改当前数据");
    drop(staged);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&stage, fs::Permissions::from_mode(0o600))?;
    }
    fs::File::open(&stage)?.sync_all()?;
    let marker = parent.join("restore-pending");
    crate::local_store::write_if_changed(
        &marker,
        b"Restore in progress. Re-run --restore-database with the verified backup.",
    )?;
    let old = parent.join(format!(
        "before-restore-{}-{:08x}",
        chrono::Utc::now().format("%Y%m%dT%H%M%S"),
        rand::random::<u32>()
    ));
    fs::create_dir_all(&old)?;
    for suffix in ["", "-wal", "-shm"] {
        let p = PathBuf::from(format!("{}{suffix}", db.display()));
        if p.exists() {
            fs::rename(&p, old.join(p.file_name().unwrap()))?;
        }
    }
    fs::rename(stage, db)?;
    #[cfg(unix)]
    {
        fs::File::open(parent)?.sync_all()?;
    }
    fs::remove_file(marker)?;
    Ok(old)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn interrupted_restore_resumes_and_includes_committed_wal() {
        let dir = tempfile::tempdir().unwrap();
        let original = dir.path().join("source/data.json");
        crate::data_store::write(&original, &serde_json::json!({"rows":[7,8]})).unwrap();
        let source = crate::data_store::database_for(&original);
        let target = dir.path().join("restored/stepx.sqlite3");
        fs::create_dir_all(target.parent().unwrap()).unwrap();
        fs::write(
            target.parent().unwrap().join("restore-pending"),
            b"interrupted",
        )
        .unwrap();
        assert!(crate::data_store::open(&target).is_err());
        restore(&target, &source).unwrap();
        let conn = crate::data_store::open(&target).unwrap();
        let count: i64 = conn
            .query_row("SELECT count(*) FROM records", [], |r| r.get(0))
            .unwrap();
        assert_eq!(count, 2);
        assert!(!target.parent().unwrap().join("restore-pending").exists());
    }
    #[test]
    fn restore_preserves_previous_database_and_rejects_active_or_invalid_source() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("data.json");
        crate::data_store::write(&path, &serde_json::json!({"rows":[1,2]})).unwrap();
        let db = crate::data_store::database_for(&path);
        let backup = dir.path().join("copy.sqlite3");
        crate::data_store::backup(&db, &backup).unwrap();
        assert!(restore(&db, &backup).is_err());
        crate::data_store::close_thread_connections();
        let old = restore(&db, &backup).unwrap();
        assert!(old.join("stepx.sqlite3").is_file());
        assert_eq!(
            crate::data_store::read::<serde_json::Value>(&path, 10000)
                .unwrap()
                .unwrap()["rows"],
            serde_json::json!([1, 2])
        );
        let bad = dir.path().join("bad.sqlite3");
        fs::write(&bad, b"broken").unwrap();
        assert!(restore(&db, &bad).is_err());
    }
}
