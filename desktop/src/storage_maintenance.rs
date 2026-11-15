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
    let file = crate::data_store::open_lock_nofollow(&root.join("maintenance.lock"))?;
    file.try_lock().context("数据维护正在进行，请稍后重试")?;
    Ok(Lock(file))
}

const FILE_PREVIEW_MAX_AGE: Duration = Duration::from_secs(3 * 86_400);
const FILE_PREVIEW_MAX_BYTES: u64 = 512 * 1024 * 1024;

pub fn clean_file_previews(root: &Path, now: SystemTime) -> Result<(usize, u64)> {
    if fs::symlink_metadata(root).is_err() {
        return Ok((0, 0));
    }
    let root_meta = fs::symlink_metadata(root)?;
    ensure!(
        root_meta.is_dir() && !root_meta.file_type().is_symlink(),
        "文件预览缓存目录不能是链接"
    );
    let canon_root = root.canonicalize()?;
    for protected in [
        crate::companion_link::receive_directory().ok(),
        Some(crate::settings::downloads()),
    ]
    .into_iter()
    .flatten()
    {
        if let Ok(canon_protected) = protected.canonicalize() {
            if canon_root == canon_protected
                || canon_root.starts_with(&canon_protected)
                || canon_protected.starts_with(&canon_root)
            {
                return Ok((0, 0));
            }
        }
    }

    let mut queue = std::collections::VecDeque::from([(root.to_path_buf(), 0usize)]);
    let mut visited = 0usize;
    let mut candidates: Vec<(SystemTime, PathBuf, u64)> = Vec::new();
    let mut subdirs: Vec<(usize, PathBuf)> = Vec::new();

    while let Some((dir, depth)) = queue.pop_front() {
        if visited >= 8192 {
            break;
        }
        let Ok(entries) = fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > 8192 {
                break;
            }
            let Ok(ft) = entry.file_type() else {
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                if depth < 3 {
                    subdirs.push((depth + 1, path.clone()));
                    queue.push_back((path, depth + 1));
                }
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let modified = meta.modified().unwrap_or(SystemTime::UNIX_EPOCH);
            candidates.push((modified, path, meta.len()));
        }
    }

    let (mut removed, mut freed_bytes) = (0usize, 0u64);
    let mut retained: Vec<(SystemTime, PathBuf, u64)> = Vec::new();
    for (modified, path, size) in candidates {
        let age = now.duration_since(modified).unwrap_or_default();
        if age > FILE_PREVIEW_MAX_AGE {
            if fs::remove_file(&path).is_ok() {
                removed += 1;
                freed_bytes = freed_bytes.saturating_add(size);
            }
        } else {
            retained.push((modified, path, size));
        }
    }

    // Keep newest preview files within the 512 MiB cap; prune oldest excess.
    retained.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.1.cmp(&b.1)));
    let mut kept_bytes = 0u64;
    for (_, path, size) in retained {
        if kept_bytes.saturating_add(size) <= FILE_PREVIEW_MAX_BYTES {
            kept_bytes = kept_bytes.saturating_add(size);
        } else if fs::remove_file(&path).is_ok() {
            removed += 1;
            freed_bytes = freed_bytes.saturating_add(size);
        }
    }

    subdirs.sort_by(|a, b| b.0.cmp(&a.0));
    for (_, dir) in subdirs {
        let _ = fs::remove_dir(dir);
    }
    Ok((removed, freed_bytes))
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
        &crate::resources::user_state().join("recent-thumbnails"),
        now,
    )?;
    let (file_previews, file_preview_bytes) = clean_file_previews(
        &crate::resources::user_state().join("file-previews"),
        now,
    )?;
    let total_previews = previews.saturating_add(file_previews);
    let total_freed = freed
        .saturating_add(thumbnail_bytes)
        .saturating_add(file_preview_bytes);
    if db.exists() {
        crate::data_store::maintain(&db)?;
    }
    Ok(format!("数据库 {:.1} MB · {} · 清理 {total_previews} 份文字与文件预览、{thumbnails} 份图片预览、{:.1} MB 无引用缓存与暂存。原文件、成果和待发送文件保留。",database_bytes(&db) as f64/1048576.,if backed_up{"备份已校验"}else{"备份有效"},total_freed as f64/1048576.))
}

fn is_cache_database_path(source: &Path) -> bool {
    if source.file_name().and_then(|n| n.to_str()).is_some_and(|n| {
        n.eq_ignore_ascii_case("search.sqlite3") || n.eq_ignore_ascii_case("file-text.sqlite3")
    }) {
        return true;
    }
    source.components().any(|c| {
        matches!(c, std::path::Component::Normal(part) if part.eq_ignore_ascii_case("cache"))
    })
}

/// Offline-only recovery. A durable marker blocks ordinary opens after an
/// interrupted restore, so the client cannot create an empty replacement DB.
pub fn restore(db: &Path, source: &Path) -> Result<PathBuf> {
    ensure!(
        source.is_file() && !fs::symlink_metadata(source)?.file_type().is_symlink(),
        "请选择完整数据库备份"
    );
    ensure!(
        !is_cache_database_path(source),
        "所选文件是缓存数据库，不是 StepX 业务数据库备份"
    );
    let source_conn =
        rusqlite::Connection::open_with_flags(source, rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
    let app_id: i64 = source_conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
    ensure!(
        app_id != crate::data_store::CACHE_APPLICATION_ID && app_id != 2,
        "所选文件是缓存数据库，不是 StepX 业务数据库备份"
    );
    let check: String = source_conn.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
    ensure!(check == "ok", "备份完整性检查失败，未修改当前数据");
    let version: i64 = source_conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(version == 1, "备份版本不兼容，未修改当前数据");
    let tables:i64=source_conn.query_row("SELECT count(*) FROM sqlite_master WHERE name IN ('snapshots','records','migrations','assets')",[],|r|r.get(0))?;
    ensure!(tables == 4, "不是 StepX 业务数据库备份");
    let cache_tables: i64 = source_conn.query_row(
        "SELECT count(*) FROM sqlite_master WHERE type='table' AND name IN ('docs','doc_ngrams','file_text','search_docs','search_fts')",
        [],
        |r| r.get(0),
    )?;
    if cache_tables > 0 {
        let business_rows: i64 = source_conn.query_row(
            "SELECT (SELECT count(*) FROM snapshots) + (SELECT count(*) FROM records) + (SELECT count(*) FROM migrations) + (SELECT count(*) FROM assets)",
            [],
            |r| r.get(0),
        )?;
        ensure!(
            business_rows > 0,
            "所选文件是缓存索引数据库，不能用于恢复业务数据"
        );
    }
    let parent = db.parent().context("恢复目录无效")?;
    let _maintenance = lock(parent)?;
    let gate = crate::data_store::open_lock_nofollow(&parent.join("database-use.lock"))?;
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
    // FlushFileBuffers on Windows requires a writable handle.
    fs::OpenOptions::new()
        .write(true)
        .open(&stage)?
        .sync_all()?;
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
    crate::data_store::invalidate_quick_check(db);
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

    #[test]
    fn restore_refuses_search_and_file_text_cache_databases() {
        let dir = tempfile::tempdir().unwrap();
        let business_json = dir.path().join("business/ledger.json");
        crate::data_store::write(&business_json, &serde_json::json!({"rows":[{"id":"k","v":42}]}))
            .unwrap();
        let business_db = crate::data_store::database_for(&business_json);

        let cache_dir = dir.path().join("cache");
        fs::create_dir_all(&cache_dir).unwrap();
        let search_db = cache_dir.join("search.sqlite3");
        let mut idx = crate::search_index::Index::open(&search_db).unwrap();
        idx.sync(&[crate::search::Document {
            id: "doc-1".into(),
            kind: crate::search::Kind::File,
            phone: false,
            title: "缓存条目".into(),
            body: "不应覆盖业务库".into(),
            source: "桌面".into(),
            at: 100,
        }])
        .unwrap();
        drop(idx);

        let sample_txt = dir.path().join("sample.txt");
        fs::write(&sample_txt, "cached preview text").unwrap();
        let sample_meta = fs::metadata(&sample_txt).unwrap();
        let file_text_db = cache_dir.join("file-text.sqlite3");
        crate::search_index::file_text_in(&file_text_db, &sample_txt, &sample_meta).unwrap();

        // Also test copies renamed outside `cache/` AND a legacy-style cache DB
        // with application_id=0 and `docs`/`doc_ngrams`/`file_text` tables.
        let disguised_search = dir.path().join("disguised-search-backup.sqlite3");
        let disguised_text = dir.path().join("disguised-text-backup.sqlite3");
        let legacy_cache = dir.path().join("legacy-cache-backup.sqlite3");
        crate::data_store::close_thread_connections();
        fs::copy(&search_db, &disguised_search).unwrap();
        fs::copy(&file_text_db, &disguised_text).unwrap();
        {
            let c = crate::data_store::open(&legacy_cache).unwrap();
            c.execute_batch(
                "CREATE TABLE docs(id TEXT); CREATE TABLE doc_ngrams(g TEXT); CREATE TABLE file_text(p TEXT);",
            )
            .unwrap();
        }
        crate::data_store::close_thread_connections();

        for bad_candidate in [
            &search_db,
            &file_text_db,
            &disguised_search,
            &disguised_text,
            &legacy_cache,
        ] {
            assert!(
                restore(&business_db, bad_candidate).is_err(),
                "restore must reject cache DB {}",
                bad_candidate.display()
            );
        }

        assert_eq!(
            crate::data_store::read::<serde_json::Value>(&business_json, 4096)
                .unwrap()
                .unwrap()["rows"][0]["v"],
            42
        );
    }

    #[test]
    fn clean_file_previews_prunes_old_and_excess_files_safely() {
        let dir = tempfile::tempdir().unwrap();
        let previews = dir.path().join("file-previews");
        let sub = previews.join("view-1234");
        fs::create_dir_all(&sub).unwrap();
        let now = SystemTime::UNIX_EPOCH + Duration::from_secs(10_000_000);

        // Expired (> 3 days old)
        let expired = sub.join("old.pdf");
        fs::write(&expired, b"expired-preview").unwrap();
        fs::File::options()
            .write(true)
            .open(&expired)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(now - Duration::from_secs(4 * 86_400)))
            .unwrap();

        // Recent preview kept
        let recent = previews.join("inline-photo.png");
        fs::write(&recent, b"recent-preview").unwrap();
        fs::File::options()
            .write(true)
            .open(&recent)
            .unwrap()
            .set_times(fs::FileTimes::new().set_modified(now - Duration::from_secs(3600)))
            .unwrap();

        // User original linked via symlink must never be touched
        let user_original = dir.path().join("important.pdf");
        fs::write(&user_original, b"keep-me").unwrap();
        #[cfg(unix)]
        std::os::unix::fs::symlink(&user_original, previews.join("link.pdf")).unwrap();

        let (removed, bytes) = clean_file_previews(&previews, now).unwrap();
        assert_eq!(removed, 1);
        assert_eq!(bytes, b"expired-preview".len() as u64);
        assert!(!expired.exists());
        assert!(!sub.exists(), "empty preview temp subdir should be cleaned");
        assert!(recent.exists());
        assert_eq!(fs::read(&user_original).unwrap(), b"keep-me");
    }
}
