//! Local SQLite persistence. Files remain files; only structured records and references
//! live here. Legacy JSON is imported once, verified, and retained untouched for recovery.
use anyhow::{ensure, Context, Result};
use rusqlite::{params, Connection, OptionalExtension, TransactionBehavior};
use serde::{de::DeserializeOwned, Serialize};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::{
    cell::RefCell,
    collections::{BTreeMap, HashMap},
    fs,
    path::{Path, PathBuf},
    time::Duration,
};

const SCHEMA: i64 = 1;
const SNAPSHOT_CACHE_LIMIT: usize = 16 * 1024 * 1024;
const SNAPSHOT_READ_LIMIT: u64 = 128 * 1024 * 1024;
struct Span {
    start: usize,
    end: usize,
    section: String,
    id: String,
    array: bool,
}
struct HeadSpan {
    start: usize,
    end: usize,
    key: Option<String>,
}
struct ReadCache {
    name: String,
    version: i64,
    changes: u64,
    raw: String,
}
struct WriteCache {
    name: String,
    version: i64,
    revision: i64,
    changes: u64,
    rows: HashMap<(String, String), (String, i64)>,
    raw: Vec<u8>,
    spans: Vec<Span>,
    heads: Vec<HeadSpan>,
    head: String,
}
struct CachedConnection {
    conn: Connection,
    _gate: fs::File,
    write_cache: Option<WriteCache>,
    read_cache: Option<ReadCache>,
}
thread_local! { static CONNECTIONS: RefCell<HashMap<PathBuf, CachedConnection>> = RefCell::new(HashMap::new()); }
pub fn default_path() -> PathBuf {
    crate::resources::user_state().join("database/stepx.sqlite3")
}
pub fn database_for(legacy: &Path) -> PathBuf {
    if legacy.starts_with(crate::resources::user_state())
        || legacy.starts_with(crate::resources::user_config().join("stepx-companion"))
    {
        default_path()
    } else {
        legacy
            .parent()
            .unwrap_or_else(|| Path::new("."))
            .join("stepx.sqlite3")
    }
}
/// Snapshot names use one separator on Windows as well as Unix. In particular,
/// `join("actions/ledger.json")` and two `join` calls must address the same rows.
pub(crate) fn key(path: &Path) -> Result<String> {
    ensure!(
        !path
            .components()
            .any(|p| matches!(p, std::path::Component::ParentDir)),
        "数据路径不能包含上级目录"
    );
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::env::current_dir()?.join(path)
    };
    let name = absolute.to_string_lossy().into_owned();
    #[cfg(windows)]
    let name = name.replace('\\', "/");
    Ok(name)
}

/// Older Windows builds persisted native and mixed separators. Convert their
/// identities together, including foreign keys and migration receipts. Ambiguous
/// identities are rejected atomically rather than overwriting either history.
#[cfg(any(windows, test))]
fn migrate_windows_keys(conn: &Connection) -> Result<()> {
    let tx = conn.unchecked_transaction()?;
    tx.execute_batch(
        "CREATE TABLE IF NOT EXISTS storage_meta (key TEXT PRIMARY KEY,value TEXT NOT NULL)",
    )?;
    if tx
        .query_row(
            "SELECT value FROM storage_meta WHERE key='windows-path-keys'",
            [],
            |r| r.get::<_, String>(0),
        )
        .optional()?
        .as_deref()
        == Some("slash-v1")
    {
        tx.commit()?;
        return Ok(());
    }
    let names = {
        let mut statement =
            tx.prepare("SELECT name FROM snapshots UNION SELECT name FROM migrations")?;
        let names = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        names
    };
    let mut normalized = HashMap::new();
    for name in &names {
        let canonical = name.replace('\\', "/");
        ensure!(
            normalized.insert(canonical, name).is_none(),
            "旧数据库包含重复路径，未覆盖任何记录；请保留数据库并联系支持"
        );
    }
    tx.execute_batch("PRAGMA defer_foreign_keys=ON")?;
    for name in names {
        let canonical = name.replace('\\', "/");
        if canonical == name {
            continue;
        }
        tx.execute(
            "UPDATE snapshots SET name=?1 WHERE name=?2",
            params![canonical, name],
        )?;
        tx.execute(
            "UPDATE records SET snapshot=?1 WHERE snapshot=?2",
            params![canonical, name],
        )?;
        tx.execute(
            "UPDATE migrations SET name=?1 WHERE name=?2",
            params![canonical, name],
        )?;
    }
    tx.execute(
        "INSERT INTO storage_meta(key,value) VALUES('windows-path-keys','slash-v1')",
        [],
    )?;
    tx.commit()?;
    Ok(())
}
fn protect(path: &Path, directory: bool) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        !meta.file_type().is_symlink()
            && (if directory {
                meta.is_dir()
            } else {
                meta.is_file()
            }),
        "数据目录或文件不能是链接"
    );
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(
            path,
            fs::Permissions::from_mode(if directory { 0o700 } else { 0o600 }),
        )?;
    }
    Ok(())
}
pub fn open(path: &Path) -> Result<Connection> {
    let parent = path.parent().context("数据库目录无效")?;
    fs::create_dir_all(parent)?;
    protect(parent, true)?;
    ensure!(
        !parent.join("restore-pending").exists(),
        "数据库恢复尚未完成，请退出客户端并重新执行 --restore-database"
    );
    if path.exists() {
        protect(path, false)?;
    }
    let conn = Connection::open(path).context("无法打开本机数据库；原数据保留")?;
    protect(path, false)?;
    conn.busy_timeout(Duration::from_secs(5))?;
    let version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
    ensure!(version <= SCHEMA, "数据库由更新版本创建，请使用新版客户端");
    conn.execute_batch("PRAGMA journal_mode=WAL; PRAGMA synchronous=FULL; PRAGMA foreign_keys=ON; PRAGMA wal_autocheckpoint=512; PRAGMA journal_size_limit=8388608; PRAGMA temp_store=MEMORY; PRAGMA cache_size=-8192;")?;
    if version == 0 {
        conn.execute_batch("BEGIN IMMEDIATE")?;
        let locked_version: i64 = conn.query_row("PRAGMA user_version", [], |r| r.get(0))?;
        if locked_version == 0 {
            let tables:i64=conn.query_row("SELECT count(*) FROM sqlite_master WHERE type='table' AND name NOT LIKE 'sqlite_%'",[],|r|r.get(0))?;
            ensure!(tables == 0, "未识别的数据库结构，未修改原数据库");
            conn.execute_batch("
            CREATE TABLE IF NOT EXISTS snapshots (name TEXT PRIMARY KEY, head TEXT NOT NULL, shape TEXT NOT NULL, revision INTEGER NOT NULL DEFAULT 1, updated INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS records (snapshot TEXT NOT NULL REFERENCES snapshots(name) ON DELETE CASCADE, section TEXT NOT NULL, id TEXT NOT NULL, position INTEGER NOT NULL, body TEXT NOT NULL, updated INTEGER NOT NULL, PRIMARY KEY(snapshot,section,id));
            CREATE INDEX IF NOT EXISTS records_page ON records(snapshot,section,position,id);
            CREATE TABLE IF NOT EXISTS migrations (name TEXT PRIMARY KEY, sha256 TEXT NOT NULL, bytes INTEGER NOT NULL, records INTEGER NOT NULL, imported INTEGER NOT NULL);
            CREATE TABLE IF NOT EXISTS assets (path TEXT PRIMARY KEY, category TEXT NOT NULL, sha256 TEXT NOT NULL, bytes INTEGER NOT NULL, accessed INTEGER NOT NULL, protected INTEGER NOT NULL DEFAULT 1);
            CREATE INDEX IF NOT EXISTS assets_lifecycle ON assets(category,protected,accessed);
            CREATE TABLE IF NOT EXISTS source_cursors (scope TEXT NOT NULL, source TEXT NOT NULL, revision TEXT NOT NULL, updated INTEGER NOT NULL, PRIMARY KEY(scope,source));
            PRAGMA user_version=1;")?;
        }
        ensure!(
            locked_version == 0 || locked_version == SCHEMA,
            "数据库版本已变化"
        );
        conn.execute_batch("COMMIT")?;
    }
    #[cfg(windows)]
    migrate_windows_keys(&conn)?;
    let check: String = conn.query_row("PRAGMA quick_check(1)", [], |r| r.get(0))?;
    ensure!(
        check == "ok",
        "数据库完整性检查失败；停止写入，保留数据库和迁移前备份"
    );
    Ok(conn)
}
fn with_cached<T>(path: &Path, f: impl FnOnce(&mut CachedConnection) -> Result<T>) -> Result<T> {
    CONNECTIONS.with(|pool| {
        let mut pool = pool.borrow_mut();
        if !path.exists() {
            pool.remove(path);
        }
        if !pool.contains_key(path) {
            if pool.len() >= 8 {
                pool.clear();
            }
            let parent = path.parent().context("数据库目录无效")?;
            fs::create_dir_all(parent)?;
            let gate = fs::OpenOptions::new()
                .create(true)
                .truncate(false)
                .read(true)
                .write(true)
                .open(parent.join("database-use.lock"))?;
            gate.try_lock_shared()
                .context("数据库正在恢复，请稍后打开客户端")?;
            pool.insert(
                path.to_path_buf(),
                CachedConnection {
                    conn: open(path)?,
                    _gate: gate,
                    write_cache: None,
                    read_cache: None,
                },
            );
        }
        f(pool.get_mut(path).expect("opened"))
    })
}
pub fn with_connection<T>(path: &Path, f: impl FnOnce(&mut Connection) -> Result<T>) -> Result<T> {
    with_cached(path, |c| f(&mut c.conn))
}
pub fn close_thread_connections() {
    CONNECTIONS.with(|pool| pool.borrow_mut().clear());
}
/// Used while holding the owning scope's write lock, before removing its files.
/// Production's shared database is outside the scope and remains open.
pub fn close_thread_connections_under(directory: &Path) {
    CONNECTIONS.with(|pool| {
        pool.borrow_mut()
            .retain(|path, _| !path.starts_with(directory))
    });
}
#[derive(Serialize, serde::Deserialize, Default)]
struct Shape {
    root_array: bool,
    sections: BTreeMap<String, bool>,
}
struct Row {
    section: String,
    id: String,
    position: i64,
    body: String,
}
fn flatten(raw: &[u8]) -> Result<(String, String, Vec<Row>, Vec<Span>, Vec<HeadSpan>)> {
    use serde_json::value::RawValue;
    let mut shape = Shape::default();
    let mut rows = Vec::new();
    let mut spans = Vec::new();
    let first = raw.iter().find(|b| !b.is_ascii_whitespace());
    let mut fields: BTreeMap<String, &RawValue> = if first == Some(&b'{') {
        serde_json::from_slice(raw)?
    } else if first == Some(&b'[') {
        let value: &RawValue = serde_json::from_slice(raw)?;
        shape.root_array = true;
        BTreeMap::from([("$root".into(), value)])
    } else {
        let value: &RawValue = serde_json::from_slice(raw)?;
        return Ok((
            value.get().into(),
            serde_json::to_string(&shape)?,
            rows,
            spans,
            vec![HeadSpan {
                start: value.get().as_ptr() as usize - raw.as_ptr() as usize,
                end: value.get().as_ptr() as usize - raw.as_ptr() as usize + value.get().len(),
                key: None,
            }],
        ));
    };
    let names: Vec<_> = fields
        .iter()
        .filter(|(_, v)| v.get().starts_with('[') || v.get().starts_with('{'))
        .map(|(k, _)| k.clone())
        .collect();
    for section in names {
        let v = fields.remove(&section).unwrap();
        let array = v.get().starts_with('[');
        shape.sections.insert(section.clone(), array);
        if array {
            let values: Vec<&RawValue> = serde_json::from_str(v.get())?;
            let mut used = std::collections::HashSet::new();
            for (i, v) in values.iter().enumerate() {
                #[derive(serde::Deserialize)]
                struct Identity {
                    #[serde(default)]
                    id: Option<String>,
                }
                let identity = serde_json::from_str::<Identity>(v.get())
                    .ok()
                    .and_then(|v| v.id)
                    .unwrap_or_else(|| format!("#{i}"));
                let mut id = identity;
                while !used.insert(id.clone()) {
                    id = format!("#{i}:{id}");
                }
                let start = v.get().as_ptr() as usize - raw.as_ptr() as usize;
                spans.push(Span {
                    start,
                    end: start + v.get().len(),
                    section: section.clone(),
                    id: id.clone(),
                    array: true,
                });
                rows.push(Row {
                    section: section.clone(),
                    id,
                    position: i as i64,
                    body: v.get().into(),
                });
            }
        } else {
            let values: BTreeMap<String, &RawValue> = serde_json::from_str(v.get())?;
            for (i, (id, v)) in values.into_iter().enumerate() {
                let start = v.get().as_ptr() as usize - raw.as_ptr() as usize;
                spans.push(Span {
                    start,
                    end: start + v.get().len(),
                    section: section.clone(),
                    id: id.clone(),
                    array: false,
                });
                rows.push(Row {
                    section: section.clone(),
                    id,
                    position: i as i64,
                    body: v.get().into(),
                });
            }
        }
    }
    spans.sort_unstable_by_key(|s| s.start);
    let mut heads: Vec<_> = fields
        .iter()
        .map(|(key, value)| {
            let start = value.get().as_ptr() as usize - raw.as_ptr() as usize;
            HeadSpan {
                start,
                end: start + value.get().len(),
                key: Some(key.clone()),
            }
        })
        .collect();
    heads.sort_unstable_by_key(|s| s.start);
    Ok((
        serde_json::to_string(&fields)?,
        serde_json::to_string(&shape)?,
        rows,
        spans,
        heads,
    ))
}
fn assemble_raw(conn: &Connection, name: &str) -> Result<Option<String>> {
    assemble_raw_limited(conn, name, SNAPSHOT_READ_LIMIT)
}
fn append_snapshot(out: &mut String, text: &str, limit: usize) -> Result<()> {
    ensure!(
        out.len()
            .checked_add(text.len())
            .is_some_and(|len| len <= limit),
        "本地记录超过读取上限，原数据保留"
    );
    out.push_str(text);
    Ok(())
}
fn assemble_raw_limited(conn: &Connection, name: &str, limit: u64) -> Result<Option<String>> {
    let limit = limit.min(SNAPSHOT_READ_LIMIT) as usize;
    let Some((head, shape)) = conn
        .query_row(
            "SELECT CASE WHEN length(CAST(head AS BLOB))<=?2 THEN head END,shape FROM snapshots WHERE name=?1",
            params![name, limit as i64],
            |r| Ok((r.get::<_, Option<String>>(0)?, r.get::<_, String>(1)?)),
        )
        .optional()?
    else {
        return Ok(None);
    };
    let head = head.context("本地记录超过读取上限，原数据保留")?;
    let shape: Shape = serde_json::from_str(&shape)?;
    if shape.sections.is_empty() {
        return Ok(Some(head));
    }
    let mut out = if shape.root_array {
        String::new()
    } else {
        head.trim_end_matches('}').to_owned()
    };
    for (section, array) in &shape.sections {
        if !shape.root_array {
            if out.len() > 1 {
                append_snapshot(&mut out, ",", limit)?;
            }
            append_snapshot(&mut out, &serde_json::to_string(section)?, limit)?;
            append_snapshot(&mut out, ":", limit)?;
        }
        append_snapshot(&mut out, if *array { "[" } else { "{" }, limit)?;
        let mut stmt=conn.prepare_cached("SELECT id,body FROM records INDEXED BY records_page WHERE snapshot=?1 AND section=?2 ORDER BY position,id")?;
        let mut rows = stmt.query(params![name, section])?;
        let mut first = true;
        while let Some(row) = rows.next()? {
            if !first {
                append_snapshot(&mut out, ",", limit)?;
            }
            first = false;
            if !array {
                append_snapshot(
                    &mut out,
                    &serde_json::to_string(row.get_ref(0)?.as_str()?)?,
                    limit,
                )?;
                append_snapshot(&mut out, ":", limit)?;
            }
            // Borrow SQLite's row buffer and reject before copying it into the
            // assembled JSON. No per-row SQL CASE/length calculation is needed.
            let body = row.get_ref(1)?.as_str()?;
            append_snapshot(&mut out, body, limit)?;
        }
        append_snapshot(&mut out, if *array { "]" } else { "}" }, limit)?;
    }
    if !shape.root_array {
        append_snapshot(&mut out, "}", limit)?;
    }
    Ok(Some(out))
}
fn assemble(conn: &Connection, name: &str) -> Result<Option<Value>> {
    assemble_raw(conn, name)?
        .map(|v| serde_json::from_str(&v).map_err(Into::into))
        .transpose()
}
pub fn sha256(bytes: &[u8]) -> String {
    format!("{:x}", Sha256::digest(bytes))
}
// Reuse a validated snapshot's JSON framing. Every byte outside existing row
// and scalar-header values must match exactly; changed values are independently
// parsed and array identities must stay the same. This proves that no field,
// row, key, order, or section was inserted/deleted. Structural changes still use
// flatten + transactional reconciliation. Work scales with changed bodies, not
// with allocating/parsing every unchanged row in the incoming full snapshot.
struct CachedPatch<'a> {
    rows: Vec<(usize, &'a str)>,
    row_bounds: Vec<(usize, usize)>,
    head_bounds: Vec<(usize, usize)>,
    head: Option<String>,
}
fn array_identity(body: &str) -> Option<String> {
    #[derive(serde::Deserialize)]
    struct Identity {
        #[serde(default)]
        id: Option<String>,
    }
    serde_json::from_str::<Identity>(body)
        .ok()
        .and_then(|v| v.id)
}
fn cached_patch<'a>(saved: &WriteCache, raw: &'a [u8]) -> Option<CachedPatch<'a>> {
    use serde_json::value::RawValue;
    let mut patch = CachedPatch {
        rows: Vec::new(),
        row_bounds: Vec::with_capacity(saved.spans.len()),
        head_bounds: Vec::with_capacity(saved.heads.len()),
        head: None,
    };
    let mut head_values: Option<Value> = None;
    let (mut row, mut header, mut old_at, mut new_at) = (0usize, 0usize, 0usize, 0usize);
    while row < saved.spans.len() || header < saved.heads.len() {
        let is_row = row < saved.spans.len()
            && (header == saved.heads.len() || saved.spans[row].start < saved.heads[header].start);
        let (start, end) = if is_row {
            (saved.spans[row].start, saved.spans[row].end)
        } else {
            (saved.heads[header].start, saved.heads[header].end)
        };
        let gap = &saved.raw[old_at..start];
        if raw.get(new_at..new_at.checked_add(gap.len())?)? != gap {
            return None;
        }
        new_at += gap.len();
        let old_body = &saved.raw[start..end];
        let new_end = new_at.checked_add(old_body.len())?;
        // A numeric value may be a prefix of the new one (1 -> 100).
        // Check the following framing byte before reusing its old boundary.
        let unchanged =
            raw.get(new_at..new_end) == Some(old_body) && raw.get(new_end) == saved.raw.get(end);
        let body_end = if unchanged {
            new_end
        } else {
            let mut stream =
                serde_json::Deserializer::from_slice(raw.get(new_at..)?).into_iter::<&RawValue>();
            let value = stream.next()?.ok()?;
            // Changed whitespace/framing uses the general path as well.
            if value.get().as_ptr() != raw[new_at..].as_ptr() {
                return None;
            }
            let body = value.get();
            let parsed: Value = serde_json::from_str(body).ok()?;
            if is_row {
                if saved.spans[row].array
                    && array_identity(std::str::from_utf8(old_body).ok()?) != array_identity(body)
                {
                    return None;
                }
                patch.rows.push((row, body));
            } else {
                if parsed.is_array() || parsed.is_object() {
                    return None;
                }
                if let Some(key) = &saved.heads[header].key {
                    if head_values.is_none() {
                        head_values = Some(serde_json::from_str(&saved.head).ok()?);
                    }
                    head_values
                        .as_mut()?
                        .as_object_mut()?
                        .insert(key.clone(), parsed);
                } else {
                    head_values = Some(parsed);
                }
            }
            new_at.checked_add(stream.byte_offset())?
        };
        if is_row {
            patch.row_bounds.push((new_at, body_end));
            row += 1;
        } else {
            patch.head_bounds.push((new_at, body_end));
            header += 1;
        }
        old_at = end;
        new_at = body_end;
    }
    if raw.get(new_at..)? != &saved.raw[old_at..] {
        return None;
    }
    if let Some(head) = head_values {
        patch.head = Some(serde_json::to_string(&head).ok()?);
    }
    Some(patch)
}
fn put(
    conn: &mut Connection,
    name: &str,
    raw: &[u8],
    legacy: Option<&[u8]>,
    mut cache: Option<&mut Option<WriteCache>>,
) -> Result<usize> {
    let now = chrono::Utc::now().timestamp_millis();
    let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
    if legacy.is_some()
        && tx.query_row(
            "SELECT EXISTS(SELECT 1 FROM snapshots WHERE name=?)",
            [name],
            |r| r.get::<_, bool>(0),
        )?
    {
        tx.commit()?;
        return Ok(0);
    }
    let version: i64 = tx.query_row("PRAGMA data_version", [], |r| r.get(0))?;
    let revision: i64 = tx
        .query_row("SELECT revision FROM snapshots WHERE name=?", [name], |r| {
            r.get(0)
        })
        .optional()?
        .unwrap_or(0);
    let mut previous_cache = cache.as_mut().and_then(|slot| slot.take()).filter(|v| {
        v.name == name
            && v.version == version
            && v.revision == revision
            && v.changes == tx.total_changes()
    });
    if previous_cache.as_ref().is_some_and(|v| v.raw == raw) {
        tx.commit()?;
        if let Some(slot) = cache {
            *slot = previous_cache;
        }
        return Ok(0);
    }
    if legacy.is_none() {
        if let Some(patch) = previous_cache.as_ref().and_then(|v| cached_patch(v, raw)) {
            let saved = previous_cache.as_ref().unwrap();
            let mut changes = 0;
            // Body-only updates do not touch the pagination index.
            {
                let mut update = tx.prepare_cached("UPDATE records SET body=?1,updated=?2 WHERE snapshot=?3 AND section=?4 AND id=?5")?;
                for (index, body) in &patch.rows {
                    let span = &saved.spans[*index];
                    let count = update.execute(params![body, now, name, span.section, span.id])?;
                    ensure!(count == 1, "本地记录已变化，请重试");
                    changes += count;
                }
            }
            if let Some(head) = &patch.head {
                changes += tx.execute(
                    "UPDATE snapshots SET head=?1,updated=?2 WHERE name=?3 AND head<>?1",
                    params![head, now, name],
                )?;
            }
            if changes > 0 {
                tx.execute(
                    "UPDATE snapshots SET revision=revision+1,updated=? WHERE name=?",
                    params![now, name],
                )?;
            }
            tx.commit()?;
            let mut saved = previous_cache.take().unwrap();
            for (index, body) in patch.rows {
                let span = &saved.spans[index];
                saved
                    .rows
                    .get_mut(&(span.section.clone(), span.id.clone()))
                    .unwrap()
                    .0 = body.into();
            }
            for (span, (start, end)) in saved.spans.iter_mut().zip(patch.row_bounds) {
                span.start = start;
                span.end = end;
            }
            for (span, (start, end)) in saved.heads.iter_mut().zip(patch.head_bounds) {
                span.start = start;
                span.end = end;
            }
            if let Some(head) = patch.head {
                saved.head = head;
            }
            saved.raw.clear();
            saved.raw.extend_from_slice(raw);
            saved.revision += i64::from(changes > 0);
            saved.changes = conn.total_changes();
            if let Some(slot) = cache {
                *slot = (raw.len() <= SNAPSHOT_CACHE_LIMIT).then_some(saved);
            }
            return Ok(changes);
        }
    }
    let (head, shape, rows, spans, heads) = flatten(raw)?;
    let mut changes=tx.execute("INSERT INTO snapshots(name,head,shape,updated) VALUES(?1,?2,?3,?4) ON CONFLICT(name) DO UPDATE SET head=excluded.head,shape=excluded.shape,updated=excluded.updated WHERE head<>excluded.head OR shape<>excluded.shape",params![name,head,shape,now])?;
    let mut previous: HashMap<(String, String), (String, i64)> = if let Some(saved) = previous_cache
    {
        saved.rows
    } else {
        let mut stmt =
            tx.prepare("SELECT section,id,body,position FROM records WHERE snapshot=?")?;
        let rows = stmt
            .query_map([name], |r| {
                Ok(((r.get(0)?, r.get(1)?), (r.get(2)?, r.get(3)?)))
            })?
            .collect::<rusqlite::Result<_>>()?;
        rows
    };
    {
        let mut upsert = tx.prepare_cached("INSERT INTO records(snapshot,section,id,position,body,updated) VALUES(?1,?2,?3,?4,?5,?6) ON CONFLICT(snapshot,section,id) DO UPDATE SET position=excluded.position,body=excluded.body,updated=excluded.updated")?;
        for row in &rows {
            let k = (row.section.clone(), row.id.clone());
            if previous
                .remove(&k)
                .is_some_and(|(body, pos)| body == row.body && pos == row.position)
            {
                continue;
            }
            changes += upsert.execute(params![
                name,
                row.section,
                row.id,
                row.position,
                row.body,
                now
            ])?;
        }
        let mut delete =
            tx.prepare_cached("DELETE FROM records WHERE snapshot=? AND section=? AND id=?")?;
        for ((section, id), _) in previous {
            changes += delete.execute(params![name, section, id])?;
        }
    }
    if changes > 0 {
        tx.execute(
            "UPDATE snapshots SET revision=revision+1,updated=? WHERE name=?",
            params![now, name],
        )?;
    }
    if let Some(raw) = legacy {
        // Verify exact semantic round-trip before the migration marker is committed.
        ensure!(
            assemble(&tx, name)?.as_ref() == Some(&serde_json::from_slice::<Value>(raw)?),
            "旧数据迁移校验失败，保留原文件"
        );
        tx.execute(
            "INSERT OR IGNORE INTO migrations VALUES(?1,?2,?3,?4,?5)",
            params![name, sha256(raw), raw.len() as i64, rows.len() as i64, now],
        )?;
    }
    let final_revision: i64 =
        tx.query_row("SELECT revision FROM snapshots WHERE name=?", [name], |r| {
            r.get(0)
        })?;
    tx.commit()?;
    if let Some(slot) = cache {
        *slot = (raw.len() <= SNAPSHOT_CACHE_LIMIT).then(|| WriteCache {
            name: name.into(),
            version,
            revision: final_revision,
            changes: conn.total_changes(),
            raw: raw.to_vec(),
            spans,
            heads,
            head,
            rows: rows
                .into_iter()
                .map(|r| ((r.section, r.id), (r.body, r.position)))
                .collect(),
        });
    }
    Ok(changes)
}
pub fn read<T: DeserializeOwned + Serialize>(path: &Path, limit: u64) -> Result<Option<T>> {
    let db = database_for(path);
    let name = key(path)?;
    if !db.exists() && !path.exists() {
        return Ok(None);
    }
    with_cached(&db, |cached| {
        let conn = &mut cached.conn;
        // One read transaction gives head/records the same WAL snapshot.
        let tx = conn.transaction()?;
        let version: i64 = tx.query_row("PRAGMA data_version", [], |r| r.get(0))?;
        let changes = tx.total_changes();
        if let Some(saved) = cached
            .read_cache
            .as_ref()
            .filter(|v| v.name == name && v.version == version && v.changes == changes)
        {
            ensure!(
                saved.raw.len() as u64 <= limit.min(SNAPSHOT_READ_LIMIT),
                "本地记录超过读取上限，原数据保留"
            );
            let value = serde_json::from_str(&saved.raw).context("本地记录结构无效；未覆盖数据")?;
            tx.commit()?;
            return Ok(Some(value));
        }
        cached.read_cache = None;
        let value = assemble_raw_limited(&tx, &name, limit)?;
        tx.commit()?;
        if let Some(raw) = value {
            let parsed = serde_json::from_str(&raw).context("本地记录结构无效；未覆盖数据")?;
            if raw.len() <= SNAPSHOT_CACHE_LIMIT {
                cached.read_cache = Some(ReadCache {
                    name: name.clone(),
                    version,
                    changes,
                    raw,
                });
            }
            return Ok(Some(parsed));
        }
        if !path.exists() {
            return Ok(None);
        }
        let raw = crate::local_store::read_bounded(path, limit)?;
        let parsed: T =
            serde_json::from_slice(&raw).context("旧数据损坏；原文件保留，未创建空数据库记录")?;
        put(
            conn,
            &name,
            &serde_json::to_vec(&serde_json::from_slice::<Value>(&raw)?)?,
            Some(&raw),
            None,
        )?;
        let tx = conn.transaction()?;
        let current = assemble_raw_limited(&tx, &name, limit)?.context("迁移记录不存在")?;
        tx.commit()?;
        let _ = parsed;
        Ok(Some(serde_json::from_str(&current)?))
    })
}
pub fn write<T: Serialize + ?Sized>(path: &Path, value: &T) -> Result<usize> {
    write_json(path, &serde_json::to_vec(value)?)
}
pub fn write_json(path: &Path, raw: &[u8]) -> Result<usize> {
    let name = key(path)?;
    with_cached(&database_for(path), |cached| {
        put(
            &mut cached.conn,
            &name,
            raw,
            None,
            Some(&mut cached.write_cache),
        )
    })
}
pub fn read_bytes(path: &Path, limit: u64) -> Result<Vec<u8>> {
    let value: Value = read(path, limit)?.context("数据记录不存在")?;
    Ok(serde_json::to_vec(&value)?)
}
pub fn exists(path: &Path) -> Result<bool> {
    if path.exists() {
        return Ok(true);
    }
    has_snapshot(path)
}
pub fn has_snapshot(path: &Path) -> Result<bool> {
    let db = database_for(path);
    if !db.exists() {
        return Ok(false);
    }
    let name = key(path)?;
    with_connection(&db, |c| {
        Ok(
            c.query_row("SELECT 1 FROM snapshots WHERE name=?", [name], |_| Ok(true))
                .optional()?
                .unwrap_or(false),
        )
    })
}
pub fn snapshot_paths(directory: &Path) -> Result<Vec<PathBuf>> {
    let probe = directory.join("probe.json");
    let db = database_for(&probe);
    let mut paths = std::collections::BTreeSet::new();
    if let Ok(entries) = fs::read_dir(directory) {
        for entry in entries.flatten() {
            if entry.path().extension().is_some_and(|s| s == "json") {
                paths.insert(entry.path());
            }
        }
    }
    if db.exists() {
        let prefix = format!("{}/", key(directory)?);
        with_connection(&db, |c| {
            let mut stmt = c.prepare("SELECT name FROM snapshots WHERE substr(name,1,?1)=?2")?;
            for row in stmt.query_map(params![prefix.chars().count() as i64, prefix], |r| {
                r.get::<_, String>(0)
            })? {
                let p = PathBuf::from(row?);
                if p.parent() == Some(directory) {
                    paths.insert(p);
                }
            }
            Ok(())
        })?;
    }
    Ok(paths.into_iter().collect())
}
/// Page job identities in SQLite. Legacy directory entries are imported once
/// per directory modification, before applying LIMIT; normal refreshes never
/// reconstruct or deserialize every saved job just to select recent ones.
pub fn snapshot_paths_page(directory: &Path, offset: usize, limit: usize) -> Result<Vec<PathBuf>> {
    use std::sync::{Mutex, OnceLock};
    static IMPORTED: OnceLock<Mutex<HashMap<PathBuf, std::time::SystemTime>>> = OnceLock::new();
    let db = database_for(&directory.join("probe.json"));
    if directory.exists() {
        let modified = fs::metadata(directory)?.modified()?;
        let imported = IMPORTED.get_or_init(Default::default);
        let known = imported
            .lock()
            .map_err(|_| anyhow::anyhow!("目录迁移状态不可用"))?
            .get(directory)
            .copied();
        if known != Some(modified) || !db.exists() {
            for entry in fs::read_dir(directory)?.flatten() {
                let path = entry.path();
                if path.extension().is_some_and(|s| s == "json") && !path.is_symlink() {
                    // A corrupt historical job stays untouched and does not hide
                    // other readable jobs, matching the history reader's policy.
                    let _ = ensure_imported(&path, 2 * 1024 * 1024);
                }
            }
            let mut cache = imported
                .lock()
                .map_err(|_| anyhow::anyhow!("目录迁移状态不可用"))?;
            if cache.len() >= 128 {
                cache.clear();
            }
            cache.insert(directory.to_path_buf(), modified);
        }
    }
    if !db.exists() {
        return Ok(vec![]);
    }
    let prefix = format!("{}/", key(directory)?);
    with_connection(&db, |c| {
        c.execute_batch(
            "CREATE INDEX IF NOT EXISTS snapshots_recent ON snapshots(updated DESC,name)",
        )?;
        let mut statement = c.prepare_cached("SELECT name FROM snapshots WHERE name>=?1 AND name<?2 AND instr(substr(name,?3),'/')=0 ORDER BY updated DESC,name LIMIT ?4 OFFSET ?5")?;
        let rows = statement.query_map(
            params![
                prefix,
                format!("{prefix}\u{10ffff}"),
                prefix.chars().count() as i64 + 1,
                limit.min(1000) as i64,
                offset as i64
            ],
            |r| r.get::<_, String>(0),
        )?;
        Ok(rows
            .map(|row| row.map(PathBuf::from))
            .collect::<rusqlite::Result<Vec<_>>>()?)
    })
}
pub fn remove_scope(directory: &Path) -> Result<()> {
    let db = database_for(&directory.join("ledger.json"));
    if !db.exists() {
        return Ok(());
    }
    let prefix = format!("{}/", key(directory)?);
    with_connection(&db, |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        tx.execute(
            "DELETE FROM snapshots WHERE substr(name,1,?1)=?2",
            params![prefix.chars().count() as i64, prefix],
        )?;
        tx.execute(
            "DELETE FROM migrations WHERE substr(name,1,?1)=?2",
            params![prefix.chars().count() as i64, prefix],
        )?;
        tx.commit()?;
        Ok(())
    })
}
pub fn remove_snapshot(path: &Path) -> Result<()> {
    let name = key(path)?;
    with_connection(&database_for(path), |c| {
        c.execute("DELETE FROM snapshots WHERE name=?", [&name])?;
        c.execute("DELETE FROM migrations WHERE name=?", [&name])?;
        Ok(())
    })
}
/// Import a legacy snapshot only once. Indexed consumers must call this before
/// paging; an absent legacy file and absent snapshot is a genuinely empty source.
pub fn ensure_imported(path: &Path, limit: u64) -> Result<bool> {
    if has_snapshot(path)? {
        return Ok(true);
    }
    Ok(read::<Value>(path, limit)?.is_some())
}

#[derive(Clone, Debug)]
pub struct SnapshotHead {
    pub revision: i64,
    pub value: Value,
}
/// Reads only scalar metadata, never materializes record collections.
pub fn read_head(path: &Path) -> Result<Option<SnapshotHead>> {
    let name = key(path)?;
    if !database_for(path).exists() {
        return Ok(None);
    }
    with_connection(&database_for(path), |c| {
        let row: Option<(i64, String)> = c
            .query_row(
                "SELECT revision,head FROM snapshots WHERE name=?",
                [name],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .optional()?;
        row.map(|(revision, raw)| {
            Ok(SnapshotHead {
                revision,
                value: serde_json::from_str(&raw)?,
            })
        })
        .transpose()
    })
}
pub fn count(path: &Path, section: &str) -> Result<usize> {
    let name = key(path)?;
    with_connection(&database_for(path), |c| {
        Ok(c.query_row(
            "SELECT count(*) FROM records WHERE snapshot=? AND section=?",
            params![name, section],
            |r| r.get::<_, i64>(0),
        )? as usize)
    })
}
pub fn typed_page<T: DeserializeOwned>(
    path: &Path,
    section: &str,
    offset: usize,
    limit: usize,
) -> Result<Vec<T>> {
    page(path, section, offset, limit)?
        .into_iter()
        .map(|v| Ok(serde_json::from_value(v)?))
        .collect()
}

/// Caller provides exactly the changed rows; None preserves an existing position
/// or appends a new row. Deletes and scalar metadata commit in the same transaction.
pub struct RecordChange<'a> {
    pub section: &'a str,
    pub id: &'a str,
    pub position: Option<i64>,
    pub value: Value,
}
pub fn apply_delta(
    path: &Path,
    expected_revision: i64,
    head: Option<&Value>,
    changes: &[RecordChange<'_>],
    deletes: &[(&str, &str)],
) -> Result<usize> {
    let name = key(path)?;
    with_connection(&database_for(path), |c| {
        let tx = c.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let (revision, shape_raw): (i64, String) = tx.query_row(
            "SELECT revision,shape FROM snapshots WHERE name=?",
            [&name],
            |r| Ok((r.get(0)?, r.get(1)?)),
        )?;
        ensure!(
            revision == expected_revision,
            "本地记录已变化，请刷新后重试；未覆盖新记录"
        );
        let shape: Shape = serde_json::from_str(&shape_raw)?;
        let now = chrono::Utc::now().timestamp_millis();
        let mut changed = 0;
        for (section, id) in deletes {
            changed += tx.execute(
                "DELETE FROM records WHERE snapshot=? AND section=? AND id=?",
                params![name, section, id],
            )?;
        }
        {
            let mut body_update = tx.prepare_cached("UPDATE records SET body=?4,updated=?5 WHERE snapshot=?1 AND section=?2 AND id=?3 AND body<>?4")?;
            let mut exists = tx.prepare_cached(
                "SELECT EXISTS(SELECT 1 FROM records WHERE snapshot=?1 AND section=?2 AND id=?3)",
            )?;
            let mut upsert = tx.prepare_cached("INSERT INTO records(snapshot,section,id,position,body,updated) VALUES(?1,?2,?3,COALESCE(?4,(SELECT position FROM records WHERE snapshot=?1 AND section=?2 AND id=?3),(SELECT COALESCE(MAX(position),-1)+1 FROM records WHERE snapshot=?1 AND section=?2)),?5,?6) ON CONFLICT(snapshot,section,id) DO UPDATE SET position=excluded.position,body=excluded.body,updated=excluded.updated WHERE records.position<>excluded.position OR records.body<>excluded.body")?;
            for change in changes {
                let array = shape
                    .sections
                    .get(change.section)
                    .context("记录分区不存在；未修改原记录")?;
                if *array {
                    // Stable string IDs identify normal records. Primitive or
                    // id-less arrays use flatten's explicit positional IDs;
                    // requiring the position prevents ambiguous implicit moves.
                    let identity_matches =
                        if let Some(id) = change.value.get("id").and_then(Value::as_str) {
                            id == change.id
                        } else {
                            change
                                .position
                                .is_some_and(|p| p >= 0 && change.id == format!("#{p}"))
                        };
                    ensure!(
                        identity_matches,
                        "增量记录必须有匹配的稳定标识或明确的位置标识"
                    );
                }
                let body = serde_json::to_string(&change.value)?;
                if change.position.is_none() {
                    // Existing bodies can change without touching the pagination
                    // index or executing a position subquery. A zero-row update
                    // can mean unchanged or absent; distinguish before inserting.
                    let updated =
                        body_update.execute(params![name, change.section, change.id, body, now])?;
                    if updated > 0 {
                        changed += updated;
                        continue;
                    }
                    let present: bool =
                        exists.query_row(params![name, change.section, change.id], |r| r.get(0))?;
                    if present {
                        continue;
                    }
                }
                changed += upsert.execute(params![
                    name,
                    change.section,
                    change.id,
                    change.position,
                    body,
                    now
                ])?;
            }
        }
        if let Some(head) = head {
            ensure!(
                head.as_object().is_some_and(|fields| fields
                    .keys()
                    .all(|key| !shape.sections.contains_key(key))),
                "增量头信息不能替换记录分区"
            );
            changed += tx.execute(
                "UPDATE snapshots SET head=?2 WHERE name=?1 AND head<>?2",
                params![name, serde_json::to_string(head)?],
            )?;
        }
        if changed > 0 {
            tx.execute(
                "UPDATE snapshots SET revision=revision+1,updated=?2 WHERE name=?1",
                params![name, now],
            )?;
        }
        tx.commit()?;
        Ok(changed)
    })
}

/// Indexed access for large collections; no reconstruction of an entire JSON history.
pub fn page(path: &Path, section: &str, offset: usize, limit: usize) -> Result<Vec<Value>> {
    let name = key(path)?;
    with_connection(&database_for(path), |c| {
        let mut stmt=c.prepare_cached("SELECT body FROM records INDEXED BY records_page WHERE snapshot=?1 AND section=?2 ORDER BY position,id LIMIT ?3 OFFSET ?4")?;
        let mut out = Vec::new();
        for row in stmt.query_map(
            params![name, section, limit.min(1000) as i64, offset as i64],
            |r| r.get::<_, String>(0),
        )? {
            out.push(serde_json::from_str(&row?)?)
        }
        Ok(out)
    })
}
pub fn record(path: &Path, section: &str, id: &str) -> Result<Option<Value>> {
    let name = key(path)?;
    with_connection(&database_for(path), |c| {
        let raw: Option<String> = c
            .query_row(
                "SELECT body FROM records WHERE snapshot=? AND section=? AND id=?",
                params![name, section, id],
                |r| r.get(0),
            )
            .optional()?;
        raw.map(|v| serde_json::from_str(&v).map_err(Into::into))
            .transpose()
    })
}
pub fn track_asset(path: &Path, category: &str) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink(),
        "成果引用不是普通文件"
    );
    let name = key(path)?;
    let hash = crate::device::sha256_file(path)?;
    with_connection(
        &database_for(
            &path
                .parent()
                .and_then(Path::parent)
                .context("成果范围无效")?
                .join("assets.json"),
        ),
        |c| {
            c.execute("INSERT INTO assets(path,category,sha256,bytes,accessed,protected) VALUES(?1,?2,?3,?4,?5,1) ON CONFLICT(path) DO UPDATE SET sha256=excluded.sha256,bytes=excluded.bytes,accessed=excluded.accessed",params![name,category,hash,meta.len() as i64,chrono::Utc::now().timestamp_millis()])?;
            Ok(())
        },
    )
}
pub fn backup(db: &Path, target: &Path) -> Result<()> {
    ensure!(!target.exists(), "备份目标已存在，不覆盖");
    let parent = target.parent().context("备份目录无效")?;
    fs::create_dir_all(parent)?;
    protect(parent, true)?;
    // Only publish a verified, durable snapshot. A failed/interrupted copy must
    // not acquire an auto-* filename and postpone the next automatic backup.
    let staged = tempfile::Builder::new()
        .prefix(".backup-pending-")
        .tempfile_in(parent)?;
    with_connection(db, |c| {
        c.execute("VACUUM INTO ?1", params![staged.path().to_string_lossy()])?;
        let b =
            Connection::open_with_flags(staged.path(), rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY)?;
        let result: String = b.query_row("PRAGMA integrity_check", [], |r| r.get(0))?;
        ensure!(result == "ok", "数据库备份校验失败");
        Ok(())
    })?;
    protect(staged.path(), false)?;
    staged.as_file().sync_all()?;
    staged.persist_noclobber(target)?;
    #[cfg(unix)]
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
pub fn maintain(db: &Path) -> Result<()> {
    with_connection(db, |c| {
        c.execute_batch("PRAGMA optimize; PRAGMA wal_checkpoint(PASSIVE);")?;
        Ok(())
    })
}
pub fn updated_at(path: &Path) -> Result<i64> {
    let db = database_for(path);
    let name = key(path)?;
    if db.exists() {
        let found = with_connection(&db, |c| {
            Ok(
                c.query_row("SELECT updated FROM snapshots WHERE name=?", [name], |r| {
                    r.get::<_, i64>(0)
                })
                .optional()?,
            )
        })?;
        if let Some(at) = found {
            return Ok(at);
        }
    }
    Ok(fs::metadata(path)?
        .modified()?
        .duration_since(std::time::UNIX_EPOCH)?
        .as_millis() as i64)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn windows_path_migration_preserves_rows_and_import_receipts() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(&dir.path().join("stepx.sqlite3")).unwrap();
        #[cfg(windows)]
        conn.execute("DELETE FROM storage_meta WHERE key='windows-path-keys'", [])
            .unwrap();
        let old = r"C:\Users\example\历史\actions/ledger.json";
        let normalized = "C:/Users/example/历史/actions/ledger.json";
        put(
            &mut conn,
            old,
            br#"{"rows":[{"id":"one","text":"retained"}]}"#,
            None,
            None,
        )
        .unwrap();
        conn.execute("INSERT INTO migrations VALUES(?1,'hash',1,1,1)", [old])
            .unwrap();
        migrate_windows_keys(&conn).unwrap();
        assert!(assemble(&conn, old).unwrap().is_none());
        assert_eq!(
            assemble(&conn, normalized).unwrap().unwrap()["rows"][0]["text"],
            "retained"
        );
        assert_eq!(
            conn.query_row("SELECT name FROM migrations", [], |r| r.get::<_, String>(0))
                .unwrap(),
            normalized
        );
        let changes = conn.total_changes();
        migrate_windows_keys(&conn).unwrap();
        assert_eq!(
            conn.total_changes(),
            changes,
            "migration must be idempotent"
        );
        let broken: i64 = conn
            .query_row("SELECT count(*) FROM pragma_foreign_key_check", [], |r| {
                r.get(0)
            })
            .unwrap();
        assert_eq!(broken, 0);
    }

    #[test]
    fn windows_path_migration_rejects_collision_without_data_loss() {
        let dir = tempfile::tempdir().unwrap();
        let mut conn = open(&dir.path().join("stepx.sqlite3")).unwrap();
        #[cfg(windows)]
        conn.execute("DELETE FROM storage_meta WHERE key='windows-path-keys'", [])
            .unwrap();
        for (name, value) in [(r"C:\fixture\state.json", 1), ("C:/fixture/state.json", 2)] {
            put(
                &mut conn,
                name,
                &serde_json::to_vec(&json!({"value":value})).unwrap(),
                None,
                None,
            )
            .unwrap();
        }
        assert!(migrate_windows_keys(&conn).is_err());
        assert_eq!(
            assemble(&conn, r"C:\fixture\state.json").unwrap().unwrap()["value"],
            1
        );
        assert_eq!(
            assemble(&conn, "C:/fixture/state.json").unwrap().unwrap()["value"],
            2
        );
    }

    #[cfg(windows)]
    #[test]
    fn windows_path_separators_address_the_same_snapshot() {
        let dir = tempfile::tempdir().unwrap();
        let native = dir.path().join("actions").join("ledger.json");
        let mixed = dir.path().join("actions/ledger.json");
        write(&native, &json!({"id":"same"})).unwrap();
        assert_eq!(key(&native).unwrap(), key(&mixed).unwrap());
        assert!(has_snapshot(&mixed).unwrap());
        assert_eq!(read::<Value>(&mixed, 1000).unwrap().unwrap()["id"], "same");
        remove_scope(native.parent().unwrap()).unwrap();
        assert!(!has_snapshot(&mixed).unwrap());
    }

    #[test]
    fn closing_scope_connections_keeps_other_databases_available() {
        let dir = tempfile::tempdir().unwrap();
        let scoped = dir.path().join("scope/ledger.json");
        let elsewhere = dir.path().join("other/ledger.json");
        write(&scoped, &json!({"id":"scoped"})).unwrap();
        write(&elsewhere, &json!({"id":"elsewhere"})).unwrap();
        close_thread_connections_under(scoped.parent().unwrap());
        fs::remove_dir_all(scoped.parent().unwrap()).unwrap();
        assert!(has_snapshot(&elsewhere).unwrap());
        assert!(!scoped.parent().unwrap().exists());
    }

    #[test]
    fn database_and_cached_reads_honor_the_callers_byte_limit_without_modifying_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("limits.json");
        for value in [
            json!({"records":[{"id":"a","text":"汉字🙂".repeat(40)}]}),
            json!({"scalar":"汉字🙂".repeat(40)}),
            json!([{"id":"a","text":"汉字🙂".repeat(40)}]),
            json!({"mapped":{"key\"🙂":"汉字🙂".repeat(40)},"other":1}),
        ] {
            write(&path, &value).unwrap();
            let expected_bytes = serde_json::to_vec(&value).unwrap().len() as u64;
            let revision = read_head(&path).unwrap().unwrap().revision;
            close_thread_connections();
            // Cold SQL assembly, including header-only and multibyte values.
            assert!(read::<Value>(&path, expected_bytes - 1).is_err());
            assert_eq!(
                read::<Value>(&path, expected_bytes).unwrap(),
                Some(value.clone())
            );
            // The same limit must apply when the validated snapshot is cached.
            assert!(read::<Value>(&path, expected_bytes - 1).is_err());
            assert_eq!(read::<Value>(&path, expected_bytes).unwrap(), Some(value));
            assert_eq!(read_head(&path).unwrap().unwrap().revision, revision);
        }
        write(&path, &json!({"records":[]})).unwrap();
        assert!(read::<Value>(&path, 0).is_err());
        assert!(read::<Value>(&path, u64::MAX).is_ok());
    }
    #[test]
    fn cached_row_edits_preserve_unicode_identity_order_and_rollback() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("edits.json");
        let mut data = json!({"revision":1,"messages":[
            {"id":"dup","text":"你好"},{"id":"dup","text":"再见"},
            {"id":7,"text":"第三条"}, {"text":"第四条"}],
            "records":{"key":{"id":"different","text":"old"},"scalar":7}});
        write(&p, &data).unwrap();
        for i in 0..100 {
            data["messages"][i % 4]["text"] = json!("多字节🙂".repeat(i % 11));
            write(&p, &data).unwrap();
            assert_eq!(read::<Value>(&p, 100000).unwrap().unwrap(), data);
        }
        // Changing an ID, adding a row or modifying a header must use the full path.
        data["messages"][1]["id"] = json!("new");
        write(&p, &data).unwrap();
        data["records"]["scalar"] = json!({"nested":[1,2,3]});
        write(&p, &data).unwrap();
        data["messages"].as_array_mut().unwrap().remove(0);
        data["revision"] = json!(2);
        write(&p, &data).unwrap();
        assert_eq!(read::<Value>(&p, 100000).unwrap().unwrap(), data);
        let mut invalid = serde_json::to_vec(&data).unwrap();
        let at = invalid.iter().position(|&b| b == b'2').unwrap();
        invalid[at] = b'!';
        assert!(write_json(&p, &invalid).is_err());
        assert_eq!(read::<Value>(&p, 100000).unwrap().unwrap(), data);
        assert_eq!(write(&p, &data).unwrap(), 0);
    }
    #[test]
    fn cached_batch_patch_preserves_structure_and_updates_only_changed_rows() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("batch.json");
        let mut data = json!({"revision":9,"tail":"before","records":(0..80).map(|i|json!({"id":format!("row-{i}"),"text":"初始🙂","at":i})).collect::<Vec<_>>(),"mapped":{"a":1,"b":"two"}});
        write(&p, &data).unwrap();
        for revision in [10, 1000, 2, 99999] {
            data["revision"] = json!(revision);
            data["tail"] = json!(format!("尾部 {revision}"));
            data["mapped"]["a"] = json!(revision);
            for i in (0..80).step_by(2) {
                data["records"][i]["text"] =
                    json!(format!("中文\"\\🙂 {revision}").repeat(i % 4 + 1));
            }
            let raw = serde_json::to_vec(&data).unwrap();
            with_cached(&database_for(&p), |c| {
                let patch = cached_patch(c.write_cache.as_ref().unwrap(), &raw)
                    .expect("same framing uses the batch path");
                assert_eq!(patch.rows.len(), 41);
                assert!(patch.head.is_some());
                Ok(())
            })
            .unwrap();
            assert_eq!(write_json(&p, &raw).unwrap(), 42);
            assert_eq!(read::<Value>(&p, 100000).unwrap().unwrap(), data);
        }
        // Reordering, deleting/inserting, IDs and section types use reconciliation.
        data["records"].as_array_mut().unwrap().swap(0, 70);
        data["records"].as_array_mut().unwrap().remove(1);
        data["records"]
            .as_array_mut()
            .unwrap()
            .push(json!({"id":"new","text":"new"}));
        data["records"][3]["id"] = json!("new identity");
        data["tail"] = json!(["new section"]);
        let raw = serde_json::to_vec(&data).unwrap();
        with_cached(&database_for(&p), |c| {
            assert!(cached_patch(c.write_cache.as_ref().unwrap(), &raw).is_none());
            Ok(())
        })
        .unwrap();
        write_json(&p, &raw).unwrap();
        assert_eq!(read::<Value>(&p, 100000).unwrap().unwrap(), data);
        assert!(record(&p, "records", "row-1").unwrap().is_none());
        assert_eq!(write(&p, &data).unwrap(), 0);
    }
    #[test]
    fn cached_batch_failure_rolls_back_header_rows_and_retries() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("failure.json");
        let before = json!({"revision":1,"rows":(0..40).map(|i|json!({"id":format!("row-{i:02}"),"text":"old"})).collect::<Vec<_>>()});
        write(&p, &before).unwrap();
        with_connection(&database_for(&p), |c| {
            c.execute_batch("CREATE TEMP TRIGGER fail_batch BEFORE UPDATE ON records WHEN NEW.id='row-30' BEGIN SELECT RAISE(ABORT,'injected batch failure'); END;")?;
            Ok(())
        }).unwrap();
        // The connection's cache remains eligible after creating a trigger.
        let after = json!({"revision":222,"rows":(0..40).map(|i|json!({"id":format!("row-{i:02}"),"text":"new🙂 longer"})).collect::<Vec<_>>()});
        assert!(write(&p, &after).is_err());
        assert_eq!(read::<Value>(&p, 10000).unwrap(), Some(before));
        with_connection(&database_for(&p), |c| {
            c.execute_batch("DROP TRIGGER fail_batch")?;
            Ok(())
        })
        .unwrap();
        write(&p, &after).unwrap();
        assert_eq!(read::<Value>(&p, 10000).unwrap(), Some(after.clone()));
        let mut invalid = serde_json::to_vec(&after).unwrap();
        invalid.extend_from_slice(b"garbage");
        assert!(write_json(&p, &invalid).is_err());
        assert_eq!(read::<Value>(&p, 10000).unwrap(), Some(after));
    }
    #[test]
    fn read_cache_invalidates_on_local_sql_and_other_connection_commits() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("cache.json");
        let first = json!({"records":[{"id":"a","text":"before"}]});
        write(&p, &first).unwrap();
        assert_eq!(read::<Value>(&p, 4096).unwrap(), Some(first));
        with_connection(&database_for(&p), |c| {
            c.execute(
                "UPDATE records SET body=?",
                [r#"{"id":"a","text":"local SQL"}"#],
            )?;
            Ok(())
        })
        .unwrap();
        assert_eq!(
            read::<Value>(&p, 4096).unwrap().unwrap()["records"][0]["text"],
            "local SQL"
        );
        let other = p.clone();
        std::thread::spawn(move || {
            write(&other, &json!({"records":[{"id":"a","text":"remote"}]})).unwrap();
        })
        .join()
        .unwrap();
        assert_eq!(
            read::<Value>(&p, 4096).unwrap().unwrap()["records"][0]["text"],
            "remote"
        );
    }
    #[test]
    fn snapshot_cache_detects_other_writer_and_preserves_same_value_updates() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("state.json");
        let original = json!({"messages":[{"id":"1","text":"first"}]});
        write(&p, &original).unwrap();
        assert_eq!(write(&p, &original).unwrap(), 0);
        let other = p.clone();
        std::thread::spawn(move || {
            write(
                &other,
                &json!({"messages":[{"id":"1","text":"other writer"}]}),
            )
            .unwrap();
        })
        .join()
        .unwrap();
        assert_eq!(write(&p, &original).unwrap(), 1);
        assert_eq!(read::<Value>(&p, 4096).unwrap().unwrap(), original);
    }
    #[test]
    fn crash_child() {
        let Some(path) = std::env::var_os("STEPX_STORAGE_CRASH_PROBE") else {
            return;
        };
        let path = PathBuf::from(path);
        let db = database_for(&path);
        let c = open(&db).unwrap();
        c.execute_batch("BEGIN IMMEDIATE; UPDATE records SET body='\"uncommitted\"'")
            .unwrap();
        fs::write(path.with_extension("ready"), b"ready").unwrap();
        std::thread::sleep(Duration::from_secs(20));
        panic!("parent should terminate uncommitted writer");
    }
    #[test]
    fn killed_process_rolls_back_wal_and_reopens() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("crash.json");
        let data = json!({"messages":[{"id":"one","text":"committed"}]});
        write(&path, &data).unwrap();
        close_thread_connections();
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args(["--exact", "data_store::tests::crash_child", "--nocapture"])
            .env("STEPX_STORAGE_CRASH_PROBE", &path)
            .stdout(std::process::Stdio::null())
            .spawn()
            .unwrap();
        for _ in 0..200 {
            if path.with_extension("ready").exists() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let ready = path.with_extension("ready").exists();
        let _ = child.kill();
        child.wait().unwrap();
        assert!(ready, "child must reach uncommitted transaction");
        assert_eq!(read::<Value>(&path, 4096).unwrap(), Some(data));
    }
    #[test]
    fn migration_roundtrip_idempotence_pagination_and_backup() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ledger.json");
        let data = json!({"device":"p","account":"a","records":[{"id":"one","text":"你好"},{"id":"two","text":"第二条"}],"view":{"a":[1,2]}});
        let raw = serde_json::to_vec_pretty(&data).unwrap();
        fs::write(&p, &raw).unwrap();
        assert_eq!(read::<Value>(&p, 4096).unwrap(), Some(data.clone()));
        assert_eq!(fs::read(&p).unwrap(), raw);
        assert_eq!(write(&p, &data).unwrap(), 0);
        assert_eq!(
            page(&p, "records", 1, 1).unwrap(),
            vec![data["records"][1].clone()]
        );
        let mut updated = data.clone();
        updated["records"][1]["text"] = json!("新消息");
        assert_eq!(write(&p, &updated).unwrap(), 1);
        assert_eq!(
            record(&p, "records", "two").unwrap().unwrap()["text"],
            "新消息"
        );
        // A pre-migration JSON backup must never supersede committed new records.
        assert_eq!(read::<Value>(&p, 4096).unwrap(), Some(updated.clone()));
        let db = database_for(&p);
        let target = dir.path().join("backup.db");
        backup(&db, &target).unwrap();
        let c = open(&target).unwrap();
        assert_eq!(assemble(&c, &key(&p).unwrap()).unwrap(), Some(updated));
        assert!(backup(&db, &target).is_err());
    }
    #[test]
    fn failed_backup_never_publishes_an_automatic_backup() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("broken.sqlite3");
        fs::write(&db, b"not a sqlite database").unwrap();
        let target = dir.path().join("backups/auto-test.sqlite3");
        assert!(backup(&db, &target).is_err());
        assert!(!target.exists());
        assert_eq!(fs::read_dir(target.parent().unwrap()).unwrap().count(), 0);
        assert_eq!(fs::read(db).unwrap(), b"not a sqlite database");
    }
    #[test]
    fn disk_full_and_aborted_transactions_keep_committed_data() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("records.json");
        write(&p, &json!({"records":[{"id":"a","text":"original"}]})).unwrap();
        with_connection(&database_for(&p), |conn| {
            {
                let tx = conn.transaction()?;
                tx.execute("DELETE FROM records", [])?;
            } // drop without commit
            let count: i64 = conn.query_row("PRAGMA page_count", [], |r| r.get(0))?;
            conn.pragma_update(None, "max_page_count", count)?;
            assert!(put(
                conn,
                &key(&p)?,
                &serde_json::to_vec(
                    &json!({"records":[{"id":"a","text":"x".repeat(4*1024*1024)}]})
                )?,
                None,
                None
            )
            .is_err());
            Ok(())
        })
        .unwrap();
        assert_eq!(
            record(&p, "records", "a").unwrap().unwrap()["text"],
            "original"
        );
    }
    #[test]
    fn corruption_future_schema_and_failed_import_do_not_overwrite() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ledger.json");
        fs::write(&p, "broken original").unwrap();
        assert!(read::<Value>(&p, 4096).is_err());
        assert_eq!(fs::read_to_string(&p).unwrap(), "broken original");
        with_connection(&database_for(&p), |c| {
            assert_eq!(
                c.query_row("SELECT count(*) FROM snapshots", [], |r| r.get::<_, i64>(0))?,
                0
            );
            Ok(())
        })
        .unwrap();
        let future = dir.path().join("future.db");
        let c = Connection::open(&future).unwrap();
        c.pragma_update(None, "user_version", 999).unwrap();
        drop(c);
        assert!(open(&future).is_err());
        let corrupt = dir.path().join("bad.db");
        fs::write(&corrupt, "not sqlite").unwrap();
        assert!(open(&corrupt).is_err());
        assert_eq!(fs::read_to_string(corrupt).unwrap(), "not sqlite");
    }
    #[test]
    fn concurrent_readers_never_observe_a_partial_commit() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("ledger.json");
        write(
            &p,
            &json!({"revision":0,"records":[{"id":"a","revision":0}]}),
        )
        .unwrap();
        std::thread::scope(|scope| {
            scope.spawn(|| {
                for revision in 1..40 {
                    write(
                        &p,
                        &json!({"revision":revision,"records":[{"id":"a","revision":revision}]}),
                    )
                    .unwrap();
                }
            });
            for _ in 0..3 {
                let p = &p;
                scope.spawn(move || {
                    for _ in 0..80 {
                        let v: Value = read(p, 4096).unwrap().unwrap();
                        assert_eq!(v["revision"], v["records"][0]["revision"]);
                    }
                });
            }
        });
    }
}

#[cfg(test)]
mod incremental_consumer_tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn delta_commits_head_rows_and_deletes_together_and_rejects_stale_writers() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("ledger.json");
        write(
            &path,
            &json!({"revision":1,"rows":[{"id":"a","value":1},{"id":"b","value":2}]}),
        )
        .unwrap();
        let _: Option<Value> = read(&path, 10000).unwrap();
        let head = read_head(&path).unwrap().unwrap();
        let changed = [
            RecordChange {
                section: "rows",
                id: "b",
                position: None,
                value: json!({"id":"b","value":3}),
            },
            RecordChange {
                section: "rows",
                id: "c",
                position: None,
                value: json!({"id":"c","value":4}),
            },
        ];
        assert_eq!(
            apply_delta(
                &path,
                head.revision,
                Some(&json!({"revision":2})),
                &changed,
                &[("rows", "a")]
            )
            .unwrap(),
            4
        );
        assert_eq!(count(&path, "rows").unwrap(), 2);
        assert_eq!(
            typed_page::<Value>(&path, "rows", 0, 1).unwrap(),
            vec![json!({"id":"b","value":3})]
        );
        assert_eq!(
            read::<Value>(&path, 10000).unwrap().unwrap(),
            json!({"revision":2,"rows":[{"id":"b","value":3},{"id":"c","value":4}]})
        );
        assert!(apply_delta(&path, head.revision, None, &[], &[("rows", "b")]).is_err());
        let head = read_head(&path).unwrap().unwrap();
        let invalid = [RecordChange {
            section: "rows",
            id: "wrong",
            position: None,
            value: json!({"id":"different"}),
        }];
        assert!(apply_delta(&path, head.revision, None, &invalid, &[("rows", "b")]).is_err());
        assert!(
            record(&path, "rows", "b").unwrap().is_some(),
            "invalid batches roll back earlier deletes"
        );
    }

    #[test]
    fn deltas_preserve_positions_skip_unchanged_and_validate_positional_ids() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("deltas.json");
        write(
            &path,
            &json!({"revision":1,"rows":[{"id":"a","value":1}],"tags":["first","second"]}),
        )
        .unwrap();
        with_connection(&database_for(&path), |c| {
            c.execute_batch("CREATE TEMP TRIGGER prevent_position_update BEFORE UPDATE OF position ON records WHEN NEW.section='rows' AND NEW.id='a' BEGIN SELECT RAISE(ABORT,'position should stay untouched'); END;")?;
            Ok(())
        }).unwrap();
        let changes = [
            RecordChange {
                section: "rows",
                id: "a",
                position: None,
                value: json!({"id":"a","value":2}),
            },
            RecordChange {
                section: "rows",
                id: "new",
                position: None,
                value: json!({"id":"new","value":3}),
            },
            RecordChange {
                section: "tags",
                id: "#1",
                position: Some(1),
                value: json!("第二个🙂"),
            },
        ];
        let revision = read_head(&path).unwrap().unwrap().revision;
        assert_eq!(
            apply_delta(&path, revision, None, &changes, &[]).unwrap(),
            3
        );
        let committed: Value = read(&path, 10000).unwrap().unwrap();
        assert_eq!(
            committed,
            json!({"revision":1,"rows":[{"id":"a","value":2},{"id":"new","value":3}],"tags":["first","第二个🙂"]})
        );
        let revision = read_head(&path).unwrap().unwrap().revision;
        assert_eq!(
            apply_delta(&path, revision, None, &changes, &[]).unwrap(),
            0
        );
        assert_eq!(read_head(&path).unwrap().unwrap().revision, revision);
        for (id, position) in [("#1", None), ("#9", Some(1)), ("#-1", Some(-1))] {
            let invalid = [RecordChange {
                section: "tags",
                id,
                position,
                value: json!("bad"),
            }];
            assert!(apply_delta(&path, revision, None, &invalid, &[("rows", "a")]).is_err());
            assert_eq!(read::<Value>(&path, 10000).unwrap().unwrap(), committed);
            assert_eq!(read_head(&path).unwrap().unwrap().revision, revision);
        }
    }

    #[test]
    fn indexed_consumers_import_legacy_once_and_page_unicode_directories() {
        let root = tempfile::tempdir().unwrap();
        let directory = root.path().join("历史");
        fs::create_dir_all(&directory).unwrap();
        for index in 0..15 {
            fs::write(
                directory.join(format!("job-{index:02}.json")),
                format!("{{\"id\":\"{index}\",\"rows\":[{{\"id\":\"a\"}}]}}"),
            )
            .unwrap();
        }
        let first = snapshot_paths_page(&directory, 0, 6).unwrap();
        let second = snapshot_paths_page(&directory, 6, 6).unwrap();
        let last = snapshot_paths_page(&directory, 12, 6).unwrap();
        assert_eq!((first.len(), second.len(), last.len()), (6, 6, 3));
        let ids: std::collections::BTreeSet<_> =
            first.into_iter().chain(second).chain(last).collect();
        assert_eq!(ids.len(), 15);
        let path = directory.join("job-00.json");
        assert!(ensure_imported(&path, 1000).unwrap());
        fs::write(&path, "corrupt legacy remains untouched after migration").unwrap();
        assert!(ensure_imported(&path, 1).unwrap());
        assert_eq!(count(&path, "rows").unwrap(), 1);
    }
}
