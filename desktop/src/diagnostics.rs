//! Bounded local operation diagnostics. No executable retry commands are persisted.
use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};
use std::{
    path::{Path, PathBuf},
    sync::{mpsc, Arc, Mutex},
};
const MAX_ENTRIES: usize = 300;
const RETENTION_MS: i64 = 30 * 86_400_000;
const MAX_BYTES: u64 = 2 * 1024 * 1024;

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Entry {
    pub id: String,
    pub at: i64,
    pub title: String,
    pub detail: String,
}
impl Entry {
    pub fn new(title: &str, detail: &str) -> Self {
        Self {
            id: format!("{:016x}", rand::random::<u64>()),
            at: chrono::Utc::now().timestamp_millis(),
            title: sanitize(title).chars().take(120).collect(),
            detail: sanitize(detail).chars().take(1200).collect(),
        }
    }
}
pub fn path() -> PathBuf {
    crate::resources::user_state().join("diagnostics/operations.json")
}
pub fn retain(entries: &mut Vec<Entry>, now: i64) {
    entries.retain(|e| e.at >= now.saturating_sub(RETENTION_MS));
    entries.sort_by(|a, b| b.at.cmp(&a.at));
    entries.truncate(MAX_ENTRIES);
}
/// A page is fetched from SQLite; older entries are not deserialized for the UI.
pub struct Page {
    pub entries: Vec<Entry>,
    pub total: usize,
}
pub fn load_page(path: &Path, offset: usize, limit: usize) -> Result<Page> {
    if !crate::data_store::ensure_imported(path, MAX_BYTES)? { return Ok(Page { entries: vec![], total: 0 }); }
    let name = path_key(path)?;
    let cutoff = chrono::Utc::now().timestamp_millis().saturating_sub(RETENTION_MS);
    crate::data_store::with_connection(&crate::data_store::database_for(path), |c| {
        let tx = c.transaction()?;
        let total: i64 = tx.query_row("SELECT count(*) FROM records WHERE snapshot=?1 AND section='$root' AND json_extract(body,'$.at')>=?2", rusqlite::params![name,cutoff], |r|r.get(0))?;
        let entries = {
            let mut statement = tx.prepare_cached("SELECT body FROM records WHERE snapshot=?1 AND section='$root' AND json_extract(body,'$.at')>=?2 ORDER BY json_extract(body,'$.at') DESC,id LIMIT ?3 OFFSET ?4")?;
            let rows = statement.query_map(rusqlite::params![name,cutoff,limit.min(MAX_ENTRIES.saturating_sub(offset)) as i64,offset.min(MAX_ENTRIES) as i64], |r|r.get::<_,String>(0))?;
            rows.map(|row| Ok(serde_json::from_str(&row?)?)).collect::<Result<Vec<Entry>>>()?
        };
        tx.commit()?;
        Ok(Page { entries, total: (total as usize).min(MAX_ENTRIES) })
    })
}
pub fn load(path: &Path) -> Result<Vec<Entry>> { Ok(load_page(path, 0, MAX_ENTRIES)?.entries) }
fn path_key(path: &Path) -> Result<String> {
    crate::data_store::key(path)
}
fn append(path: &Path, mut batch: Vec<Entry>) -> Result<()> {
    let parent = path.parent().context("日志目录无效")?;
    std::fs::create_dir_all(parent)?;
    #[cfg(unix)] {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))?;
    }
    let lock = std::fs::OpenOptions::new().create(true).truncate(false).write(true).open(parent.join("operations.lock"))?;
    lock.lock()?;
    if !crate::data_store::ensure_imported(path, MAX_BYTES)? { crate::data_store::write(path, &Vec::<Entry>::new())?; }
    let head = crate::data_store::read_head(path)?.context("日志记录不存在")?;
    let name = path_key(path)?;
    let now = chrono::Utc::now().timestamp_millis();
    retain(&mut batch, now);
    // Read only identities/timestamps for retention; existing message bodies stay
    // in SQLite and a normal append writes only the new records.
    let mut identities: Vec<(String,i64)> = crate::data_store::with_connection(&crate::data_store::database_for(path), |c| {
        let mut statement = c.prepare_cached("SELECT id,json_extract(body,'$.at') FROM records WHERE snapshot=? AND section='$root'")?;
        let rows = statement.query_map([name], |r|Ok((r.get(0)?,r.get(1)?)))?.collect::<rusqlite::Result<_>>()?;
        Ok(rows)
    })?;
    let existing = identities.clone();
    for entry in &batch { identities.retain(|(id,_)| id!=&entry.id); identities.push((entry.id.clone(),entry.at)); }
    identities.retain(|(_,at)| *at>=now.saturating_sub(RETENTION_MS));
    identities.sort_by(|a,b| b.1.cmp(&a.1).then_with(||a.0.cmp(&b.0)));
    identities.truncate(MAX_ENTRIES);
    let retained: std::collections::HashSet<_> = identities.iter().map(|(id,_)|id.as_str()).collect();
    let deletes: Vec<_> = existing.iter().filter(|(id,_)| !retained.contains(id.as_str())).map(|(id,_)|("$root",id.as_str())).collect();
    let changes = batch.iter().filter(|e|retained.contains(e.id.as_str())).map(|entry| Ok(crate::data_store::RecordChange {
        section: "$root", id: &entry.id, position: Some(entry.at.saturating_neg()), value: serde_json::to_value(entry)?,
    })).collect::<Result<Vec<_>>>()?;
    crate::data_store::apply_delta(path, head.revision, None, &changes, &deletes)?;
    Ok(())
}
pub struct Writer {
    tx: mpsc::SyncSender<Entry>,
    pub error: Arc<Mutex<Option<String>>>,
}
impl Writer {
    pub fn start(path: PathBuf) -> Self {
        let (tx, rx) = mpsc::sync_channel(256);
        let error = Arc::new(Mutex::new(None));
        let status = error.clone();
        std::thread::spawn(move || {
            // Prune at startup and hourly even when no new operation is recorded.
            *status.lock().unwrap_or_else(|e| e.into_inner()) = append(&path, vec![])
                .err()
                .map(|e| format!("日志未保存：{e:#}"));
            loop {
                let mut batch = match rx.recv_timeout(std::time::Duration::from_secs(3600)) {
                    Ok(first) => vec![first],
                    Err(mpsc::RecvTimeoutError::Timeout) => vec![],
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                };
                batch.extend(rx.try_iter().take(63));
                let result = append(&path, batch);
                *status.lock().unwrap_or_else(|e| e.into_inner()) =
                    result.err().map(|e| format!("日志未保存：{e:#}"));
            }
        });
        Self { tx, error }
    }
    pub fn send(&self, entry: Entry) {
        if self.tx.try_send(entry).is_err() {
            *self.error.lock().unwrap_or_else(|e| e.into_inner()) =
                Some("日志写入繁忙，部分记录仅保留在本次运行中".into());
        }
    }
}
pub fn sanitize(text: &str) -> String {
    text.lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "authorization",
                "bearer ",
                "api_key",
                "api-key",
                "apikey",
                "password",
                "access_token",
                "refresh_token",
                "client_secret",
                "cookie:",
            ]
            .iter()
            .any(|key| lower.contains(key))
            {
                return "[已隐藏凭据相关信息]".to_string();
            }
            line.split_whitespace()
                .map(|part| {
                    if part.contains("://") {
                        if let Ok(mut u) = url::Url::parse(part) {
                            let _ = u.set_username("");
                            let _ = u.set_password(None);
                            u.set_query(None);
                            u.set_fragment(None);
                            return u.to_string();
                        }
                    }
                    if part.len() >= 32
                        && part
                            .chars()
                            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
                    {
                        "[已隐藏标识]".into()
                    } else {
                        part.into()
                    }
                })
                .collect::<Vec<String>>()
                .join(" ")
        })
        .collect::<Vec<_>>()
        .join("\n")
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn pages_are_complete_and_append_updates_only_new_and_expired_rows() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("operations.json");
        let now = chrono::Utc::now().timestamp_millis();
        let entries = (0..300).map(|i|Entry {id:format!("entry-{i:03}"),at:now-i,title:format!("事件 {i}"),detail:"保留正文".into()}).collect();
        append(&path,entries).unwrap();
        let mut loaded = Vec::new();
        for offset in (0..300).step_by(30) {
            let page = load_page(&path,offset,30).unwrap();
            assert_eq!(page.total,300);
            assert_eq!(page.entries.len(),30);
            loaded.extend(page.entries.into_iter().map(|entry|entry.id));
        }
        assert_eq!(loaded.iter().collect::<std::collections::HashSet<_>>().len(),300);
        let db = crate::data_store::database_for(&path);
        let before = crate::data_store::with_connection(&db,|c|Ok(c.total_changes())).unwrap();
        append(&path,vec![Entry{id:"latest".into(),at:now+1,title:"新事件".into(),detail:"新正文".into()}]).unwrap();
        let after = crate::data_store::with_connection(&db,|c|Ok(c.total_changes())).unwrap();
        assert_eq!(after-before,3,"one append, one expiry, one snapshot revision; no rewriting old bodies");
        assert_eq!(load_page(&path,0,30).unwrap().entries[0].id,"latest");
        assert_eq!(load(&path).unwrap().len(),300);
    }
    #[test]
    fn retention_restart_and_corruption_preserve_original() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("operations.json");
        let mut batch = (0..350)
            .map(|_| Entry::new("读取失败", "路径不存在，请重试"))
            .collect::<Vec<_>>();
        let mut old = Entry::new("旧日志", "");
        old.at = 0;
        batch.push(old);
        append(&p, batch).unwrap();
        assert_eq!(load(&p).unwrap().len(), 300);
        crate::data_store::remove_scope(d.path()).unwrap();
        std::fs::write(&p, b"broken").unwrap();
        assert!(append(&p, vec![Entry::new("新日志", "")]).is_err());
        assert_eq!(std::fs::read(&p).unwrap(), b"broken");
    }
    #[test]
    fn sensitive_fields_do_not_enter_logs() {
        let text=sanitize("失败\nAuthorization: Bearer SECRET\nhttps://user:pass@example.org/path?token=secret#private\nabcdefghijklmnopqrstuvwxyz0123456789");
        assert!(!text.contains("SECRET") && !text.contains("pass") && !text.contains("token="));
        assert!(text.contains("失败") && text.contains("https://example.org/path"));
    }
}
