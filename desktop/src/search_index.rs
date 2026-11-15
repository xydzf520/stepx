//! Rebuildable, device-filtered search index. Unicode uni/bigrams preserve short
//! Chinese queries; candidates still pass exact matching and the original ranking.
use crate::search::{Document, Hit, Kind};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::{
    cell::RefCell,
    collections::{BTreeSet, HashMap, HashSet},
    path::{Path, PathBuf},
};

pub struct Index {
    conn: Connection,
}
pub(crate) fn ngrams(text: &str, query: bool) -> BTreeSet<String> {
    let lower = text.to_lowercase();
    let normalized = crate::search::normalize_han_mid_punct(&lower);
    let mut tokens = BTreeSet::new();
    if query {
        let chars: Vec<_> = normalized.chars().collect();
        if chars.len() == 1 {
            for c in &chars {
                tokens.insert(format!("u{:x}", *c as u32));
            }
        }
        for pair in chars.windows(2) {
            tokens.insert(format!("b{:x}x{:x}", pair[0] as u32, pair[1] as u32));
        }
        return tokens;
    }
    let chars: Vec<_> = lower.chars().collect();
    for c in &chars {
        tokens.insert(format!("u{:x}", *c as u32));
    }
    for pair in chars.windows(2) {
        tokens.insert(format!("b{:x}x{:x}", pair[0] as u32, pair[1] as u32));
    }
    if normalized != lower {
        let norm_chars: Vec<_> = normalized.chars().collect();
        for pair in norm_chars.windows(2) {
            tokens.insert(format!("b{:x}x{:x}", pair[0] as u32, pair[1] as u32));
        }
    }
    tokens
}
impl Index {
    pub fn open(path: &Path) -> Result<Self> {
        Self::initialize(crate::data_store::open(path)?)
    }
    /// Rebuildable index failures must not block search or modify business data.
    pub fn memory() -> Result<Self> {
        Self::initialize(Connection::open_in_memory()?)
    }
    fn initialize(conn: Connection) -> Result<Self> {
        conn.execute_batch("CREATE TABLE IF NOT EXISTS search_docs (id TEXT PRIMARY KEY, position INTEGER NOT NULL, digest TEXT NOT NULL, kind TEXT NOT NULL, phone INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS search_filter ON search_docs(phone,kind);
            CREATE VIRTUAL TABLE IF NOT EXISTS search_fts USING fts5(id UNINDEXED, grams, tokenize='ascii');")?;
        let app_id: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if app_id != crate::data_store::CACHE_APPLICATION_ID {
            conn.execute_batch(&format!(
                "DELETE FROM search_fts; DELETE FROM search_docs; PRAGMA application_id={};",
                crate::data_store::CACHE_APPLICATION_ID
            ))?;
        }
        Ok(Self { conn })
    }
    /// Runs only when the source snapshot changes, never on every keystroke.
    pub fn sync(&mut self, docs: &[Document]) -> Result<usize> {
        let tx = self.conn.transaction()?;
        let mut old: HashMap<String, (i64, String)> = {
            let mut s = tx.prepare("SELECT id,position,digest FROM search_docs")?;
            let v = s
                .query_map([], |r| Ok((r.get(0)?, (r.get(1)?, r.get(2)?))))?
                .collect::<rusqlite::Result<_>>()?;
            v
        };
        let mut changed = 0;
        for (position, d) in docs.iter().enumerate() {
            let digest = crate::data_store::sha256(&serde_json::to_vec(d)?);
            let previous = old.remove(&d.id);
            if previous
                .as_ref()
                .is_some_and(|(p, h)| *p == position as i64 && h == &digest)
            {
                continue;
            }
            tx.execute("INSERT INTO search_docs VALUES(?1,?2,?3,?4,?5) ON CONFLICT(id) DO UPDATE SET position=excluded.position,digest=excluded.digest,kind=excluded.kind,phone=excluded.phone",params![d.id,position as i64,digest,d.kind.label(),d.phone])?;
            if !previous.is_some_and(|(_, h)| h == digest) {
                let text = format!(
                    "{}\n{}\n{} {} {}",
                    d.title,
                    d.body,
                    d.source,
                    d.kind.label(),
                    if d.phone { "手机" } else { "电脑 desktop" }
                );
                let terms = ngrams(&text, false)
                    .into_iter()
                    .collect::<Vec<_>>()
                    .join(" ");
                tx.execute(
                    "DELETE FROM search_fts WHERE rowid=(SELECT rowid FROM search_docs WHERE id=?)",
                    [&d.id],
                )?;
                tx.execute("INSERT INTO search_fts(rowid,id,grams) SELECT rowid,?1,?2 FROM search_docs WHERE id=?1",params![d.id,terms])?;
            }
            changed += 1;
        }
        for id in old.keys() {
            tx.execute(
                "DELETE FROM search_fts WHERE rowid=(SELECT rowid FROM search_docs WHERE id=?)",
                [id],
            )?;
            tx.execute("DELETE FROM search_docs WHERE id=?", [id])?;
            changed += 1;
        }
        tx.commit()?;
        Ok(changed)
    }
    pub fn find(
        &self,
        docs: &[Document],
        query: &str,
        kind: Option<Kind>,
        phone: Option<bool>,
    ) -> Result<Vec<Hit>> {
        let mut terms = BTreeSet::new();
        for term in query.split_whitespace() {
            if matches!(term, "今天" | "昨天" | "本周")
                || chrono::NaiveDate::parse_from_str(term, "%Y-%m-%d").is_ok()
            {
                continue;
            }
            terms.extend(ngrams(term, true));
        }
        let matching = terms.into_iter().collect::<Vec<_>>().join(" AND ");
        let filter = kind.map(Kind::label);
        let sql = if matching.is_empty() {
            "SELECT position FROM search_docs WHERE (?2 IS NULL OR kind=?2) AND (?3 IS NULL OR phone=?3) AND ?1=''"
        } else {
            "SELECT d.position FROM search_fts JOIN search_docs d ON d.rowid=search_fts.rowid WHERE search_fts MATCH ?1 AND (?2 IS NULL OR d.kind=?2) AND (?3 IS NULL OR d.phone=?3)"
        };
        let mut stmt = self.conn.prepare_cached(sql)?;
        let mut rows = stmt.query(params![matching, filter, phone])?;
        let mut indices = Vec::new();
        while let Some(row) = rows.next()? {
            indices.push(row.get::<_, i64>(0)? as usize);
            if indices.len() > 2048 && indices.len() > docs.len() / 8 {
                return Ok(crate::search::find(docs, query, kind, phone));
            }
        }
        Ok(crate::search::find_indices(
            docs,
            query,
            kind,
            phone,
            indices.into_iter(),
        ))
    }
}

thread_local! {
    static FILE_TEXT_SCHEMA_READY: RefCell<HashSet<PathBuf>> = RefCell::new(HashSet::new());
}

const FILE_TEXT_SEEN_REFRESH_MS: i64 = 86_400_000;

/// Cache only small text previews. Original files are always re-statted and never
/// modified by indexing. ctime also catches same-size writes restoring mtime.
pub fn file_text(path: &Path, meta: &std::fs::Metadata) -> Result<String> {
    let db = crate::resources::user_state().join("cache/file-text.sqlite3");
    file_text_in(&db, path, meta)
}

pub(crate) fn file_text_in(db: &Path, path: &Path, meta: &std::fs::Metadata) -> Result<String> {
    let stamp = format!("{:?}:{}:{}", meta.modified()?, meta.len(), {
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            format!("{}:{}", meta.ctime(), meta.ctime_nsec())
        }
        #[cfg(not(unix))]
        {
            format!("{:?}", meta.created())
        }
    });
    if std::fs::symlink_metadata(db).is_err() {
        FILE_TEXT_SCHEMA_READY.with(|ready| {
            ready.borrow_mut().remove(db);
        });
    }
    crate::data_store::with_connection(db, |conn| {
        use rusqlite::OptionalExtension;
        let needs_ddl = FILE_TEXT_SCHEMA_READY.with(|ready| !ready.borrow().contains(db));
        if needs_ddl {
            conn.execute_batch(&format!(
                "CREATE TABLE IF NOT EXISTS file_text(path TEXT PRIMARY KEY,stamp TEXT NOT NULL,body TEXT NOT NULL,seen INTEGER NOT NULL);
                 CREATE INDEX IF NOT EXISTS file_text_seen ON file_text(seen);
                 PRAGMA application_id={};",
                crate::data_store::CACHE_APPLICATION_ID
            ))?;
            FILE_TEXT_SCHEMA_READY.with(|ready| {
                let mut set = ready.borrow_mut();
                if set.len() >= 64 {
                    set.clear();
                }
                set.insert(db.to_path_buf());
            });
        }
        let now = chrono::Utc::now().timestamp_millis();
        let key_cow = path.to_string_lossy();
        let key: &str = &key_cow;
        let cached = {
            let mut stmt = conn
                .prepare_cached("SELECT body, seen FROM file_text WHERE path=?1 AND stamp=?2")?;
            stmt.query_row(params![key, &stamp], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, i64>(1)?))
            })
            .optional()?
        };
        if let Some((body, seen)) = cached {
            if now.saturating_sub(seen) > FILE_TEXT_SEEN_REFRESH_MS {
                let mut touch =
                    conn.prepare_cached("UPDATE file_text SET seen=?1 WHERE path=?2")?;
                touch.execute(params![now, key])?;
            }
            return Ok(body);
        }
        let body = String::from_utf8(crate::local_store::read_bounded(path, 256 * 1024)?)?;
        let mut upsert = conn.prepare_cached(
            "INSERT INTO file_text VALUES(?1,?2,?3,?4) ON CONFLICT(path) DO UPDATE SET stamp=excluded.stamp,body=excluded.body,seen=excluded.seen",
        )?;
        upsert.execute(params![key, stamp, &body, now])?;
        Ok(body)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn indexed_matches_reference_including_short_chinese_and_scope_changes() {
        let dir = tempfile::tempdir().unwrap();
        let mut index = Index::open(&dir.path().join("index.db")).unwrap();
        let mut docs: Vec<_> = (0..1500)
            .map(|i| Document {
                id: format!("id-{i}"),
                kind: if i % 3 == 0 {
                    Kind::Message
                } else {
                    Kind::File
                },
                phone: i % 2 == 0,
                title: format!("设计方案 {i}"),
                body: if i % 19 == 0 {
                    "请明天交付新版设计图 O'Brien 100% _测试".into()
                } else {
                    "会议讨论记录，不代表完成".into()
                },
                source: "微信".into(),
                at: chrono::Local::now().timestamp_millis(),
            })
            .collect();
        assert_eq!(index.sync(&docs).unwrap(), docs.len());
        assert_eq!(index.sync(&docs).unwrap(), 0);
        for query in [
            "图",
            "设计",
            "设计图",
            "O'Brien",
            "100%",
            "_",
            "讨论 记录",
            "今天 会议",
            "不存在",
            "",
            "电脑",
            "微信",
        ] {
            for phone in [None, Some(true), Some(false)] {
                let a = crate::search::find(&docs, query, None, phone);
                let b = index.find(&docs, query, None, phone).unwrap();
                assert_eq!(
                    a.iter()
                        .map(|h| (h.index, h.score, &h.snippet))
                        .collect::<Vec<_>>(),
                    b.iter()
                        .map(|h| (h.index, h.score, &h.snippet))
                        .collect::<Vec<_>>(),
                    "query {query}"
                );
            }
        }
        docs.truncate(10);
        docs[0].body = "新账号独立消息".into();
        index.sync(&docs).unwrap();
        assert!(index.find(&docs, "旧账号", None, None).unwrap().is_empty());
        assert_eq!(index.find(&docs, "新账号", None, None).unwrap()[0].index, 0);
        let again = Index::open(&dir.path().join("index.db")).unwrap();
        assert_eq!(again.find(&docs, "新账号", None, None).unwrap().len(), 1);
    }

    #[test]
    fn han_middle_punctuation_bridges_bigrams_and_matches_reference_search() {
        let mut index = Index::memory().unwrap();
        let docs = vec![
            Document {
                id: "dot".into(),
                kind: Kind::Contact,
                phone: true,
                title: "买买提·艾力".into(),
                body: "年度预算-评审_纪要".into(),
                source: "通讯录".into(),
                at: 100,
            },
            Document {
                id: "plain".into(),
                kind: Kind::File,
                phone: false,
                title: "普通笔记".into(),
                body: "无关内容".into(),
                source: "桌面".into(),
                at: 90,
            },
        ];
        index.sync(&docs).unwrap();
        for q in [
            "买买提艾力",
            "买买提·艾力",
            "预算评审纪要",
            "预算-评审_纪要",
        ] {
            let ref_hits = crate::search::find(&docs, q, None, None);
            let idx_hits = index.find(&docs, q, None, None).unwrap();
            assert_eq!(ref_hits.len(), 1, "query {q}");
            assert_eq!(idx_hits.len(), 1, "query {q}");
            assert_eq!(ref_hits[0].score, idx_hits[0].score);
        }
    }

    #[test]
    fn file_text_cache_hits_beyond_ten_minutes_without_reindexing_storm() {
        let dir = tempfile::tempdir().unwrap();
        let db = dir.path().join("cache/file-text.sqlite3");
        let file = dir.path().join("note.md");
        std::fs::write(&file, "稳态缓存内容").unwrap();
        let meta = std::fs::metadata(&file).unwrap();

        assert_eq!(file_text_in(&db, &file, &meta).unwrap(), "稳态缓存内容");

        // Backdate `seen` to 2 hours ago (> 10 minutes, < 24 hours).
        let two_hours_ago = chrono::Utc::now().timestamp_millis() - 2 * 3_600_000;
        let changes_before = crate::data_store::with_connection(&db, |c| {
            c.execute("UPDATE file_text SET seen=?1", [two_hours_ago])?;
            Ok(c.total_changes())
        })
        .unwrap();

        assert_eq!(file_text_in(&db, &file, &meta).unwrap(), "稳态缓存内容");
        let changes_after =
            crate::data_store::with_connection(&db, |c| Ok(c.total_changes())).unwrap();
        assert_eq!(
            changes_before, changes_after,
            "unchanged stamp within 24h must not rewrite SQLite"
        );

        // Backdate `seen` to 2 days ago (> 24 hours): `seen` refreshes once without re-reading.
        let two_days_ago = chrono::Utc::now().timestamp_millis() - 2 * 86_400_000;
        crate::data_store::with_connection(&db, |c| {
            c.execute("UPDATE file_text SET seen=?1", [two_days_ago])?;
            Ok(())
        })
        .unwrap();
        assert_eq!(file_text_in(&db, &file, &meta).unwrap(), "稳态缓存内容");
        let refreshed_seen: i64 = crate::data_store::with_connection(&db, |c| {
            Ok(c.query_row("SELECT seen FROM file_text", [], |r| r.get(0))?)
        })
        .unwrap();
        assert!(refreshed_seen > two_hours_ago);
    }
}
