//! Rebuildable, device-filtered search index. Unicode uni/bigrams preserve short
//! Chinese queries; candidates still pass exact matching and the original ranking.
use crate::search::{Document, Hit, Kind};
use anyhow::Result;
use rusqlite::{params, Connection};
use std::{
    collections::{BTreeSet, HashMap},
    path::Path,
};

pub struct Index {
    conn: Connection,
}
fn grams(text: &str, query: bool) -> BTreeSet<String> {
    let chars: Vec<_> = text.to_lowercase().chars().collect();
    let mut tokens = BTreeSet::new();
    if !query || chars.len() == 1 {
        for c in &chars {
            tokens.insert(format!("u{:x}", *c as u32));
        }
    }
    for pair in chars.windows(2) {
        tokens.insert(format!("b{:x}x{:x}", pair[0] as u32, pair[1] as u32));
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
        let version: i64 = conn.query_row("PRAGMA application_id", [], |r| r.get(0))?;
        if version != 2 {
            conn.execute_batch(
                "DELETE FROM search_fts; DELETE FROM search_docs; PRAGMA application_id=2;",
            )?;
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
                let terms = grams(&text, false)
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
            terms.extend(grams(term, true));
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

/// Cache only small text previews. Original files are always re-statted and never
/// modified by indexing. ctime also catches same-size writes restoring mtime.
pub fn file_text(path: &Path, meta: &std::fs::Metadata) -> Result<String> {
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
    let db = crate::resources::user_state().join("cache/file-text.sqlite3");
    crate::data_store::with_connection(&db, |conn| {
        use rusqlite::OptionalExtension;
        conn.execute_batch("CREATE TABLE IF NOT EXISTS file_text(path TEXT PRIMARY KEY,stamp TEXT NOT NULL,body TEXT NOT NULL,seen INTEGER NOT NULL);
            CREATE INDEX IF NOT EXISTS file_text_seen ON file_text(seen);")?;
        let now = chrono::Utc::now().timestamp_millis();
        let key = path.to_string_lossy();
        if let Some(body) = conn
            .query_row(
                "SELECT body FROM file_text WHERE path=? AND stamp=? AND seen>?",
                params![key, stamp, now - 600_000],
                |r| r.get::<_, String>(0),
            )
            .optional()?
        {
            return Ok(body);
        }
        let body = String::from_utf8(crate::local_store::read_bounded(path, 256 * 1024)?)?;
        conn.execute("INSERT INTO file_text VALUES(?1,?2,?3,?4) ON CONFLICT(path) DO UPDATE SET stamp=excluded.stamp,body=excluded.body,seen=excluded.seen",params![key,stamp,body,now])?;
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
}
