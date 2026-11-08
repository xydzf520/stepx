//! Local, deterministic search. No model calls and no network upload of a query.
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Kind {
    #[default]
    Function,
    Calendar,
    Alarm,
    Work,
    Amoo,
    Message,
    Call,
    Contact,
    Image,
    BackScreen,
    File,
    App,
    Store,
    Summary,
    Setting,
}
impl Kind {
    pub const ALL: [Self; 15] = [
        Self::Message,
        Self::Amoo,
        Self::File,
        Self::Work,
        Self::Image,
        Self::Calendar,
        Self::Alarm,
        Self::Contact,
        Self::Call,
        Self::Summary,
        Self::BackScreen,
        Self::App,
        Self::Store,
        Self::Function,
        Self::Setting,
    ];
    pub fn label(self) -> &'static str {
        match self {
            Self::Function => "功能",
            Self::Calendar => "日历",
            Self::Alarm => "闹钟",
            Self::Work => "作品",
            Self::Amoo => "Amoo",
            Self::Message => "消息",
            Self::Call => "电话",
            Self::Contact => "联系人",
            Self::Image => "图片",
            Self::BackScreen => "背屏",
            Self::File => "文件",
            Self::App => "应用",
            Self::Store => "应用商店",
            Self::Summary => "方案与总结",
            Self::Setting => "设置",
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Document {
    pub id: String,
    pub kind: Kind,
    pub phone: bool,
    pub title: String,
    pub body: String,
    pub source: String,
    pub at: i64,
}
#[derive(Clone, Debug)]
pub struct Hit {
    pub index: usize,
    pub snippet: String,
    pub score: u32,
}

pub fn find(docs: &[Document], query: &str, kind: Option<Kind>, phone: Option<bool>) -> Vec<Hit> {
    find_indices(docs, query, kind, phone, 0..docs.len())
}

/// Opening global search browses real files. Explicit type filters and typed
/// queries still search their full scope, including functional destinations.
pub fn browsing_files(query: &str, kind: Option<Kind>) -> bool {
    query.trim().is_empty() && kind.is_none()
}

pub(crate) fn find_indices(docs: &[Document], query: &str, kind: Option<Kind>, phone: Option<bool>, indices: impl Iterator<Item=usize>) -> Vec<Hit> {
    let query = query.trim().to_lowercase();
    use chrono::{Datelike, TimeZone};
    let now = chrono::Local::now();
    let today = now.date_naive();
    let mut date_range = None;
    let terms: Vec<_> = query
        .split_whitespace()
        .filter(|t| {
            let range = match *t {
                "今天" => Some((today, today)),
                "昨天" => today.pred_opt().map(|d| (d, d)),
                "本周" => today
                    .checked_sub_signed(chrono::Duration::days(
                        now.weekday().num_days_from_monday() as i64,
                    ))
                    .map(|d| (d, today)),
                _ => chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
                    .ok()
                    .map(|d| (d, d)),
            };
            if let Some((start, end)) = range {
                date_range = start
                    .and_hms_opt(0, 0, 0)
                    .and_then(|d| chrono::Local.from_local_datetime(&d).earliest())
                    .zip(
                        end.succ_opt()
                            .and_then(|d| d.and_hms_opt(0, 0, 0))
                            .and_then(|d| chrono::Local.from_local_datetime(&d).latest()),
                    )
                    .map(|(a, b)| (a.timestamp_millis(), b.timestamp_millis()));
                false
            } else {
                true
            }
        })
        .collect();
    let mut hits = Vec::new();
    for index in indices {
        let Some(d) = docs.get(index) else { continue; };
        if kind.is_some_and(|k| k != d.kind) || phone.is_some_and(|p| p != d.phone) {
            continue;
        }
        if browsing_files(&query, kind)
            && !matches!(d.kind, Kind::File | Kind::Image)
            && !(d.kind == Kind::Work && d.id.starts_with("local:"))
        {
            continue;
        }
        if date_range.is_some_and(|(a, b)| d.at < a || d.at >= b) {
            continue;
        }
        let title = d.title.to_lowercase();
        let body = d.body.to_lowercase();
        let meta = format!(
            "{} {} {}",
            d.source,
            d.kind.label(),
            if d.phone { "手机" } else { "电脑 desktop" }
        )
        .to_lowercase();
        if !terms
            .iter()
            .all(|t| title.contains(t) || body.contains(t) || meta.contains(t))
        {
            continue;
        }
        let score = if query.is_empty() {
            0
        } else if title == query {
            1000
        } else if title.starts_with(&query) {
            700
        } else {
            terms
                .iter()
                .map(|t| {
                    if title.contains(t) {
                        150
                    } else if body.contains(t) {
                        50
                    } else {
                        10
                    }
                })
                .sum()
        };
        hits.push(Hit {
            index,
            snippet: snippet(&d.body, &terms, 100),
            score,
        });
    }
    hits.sort_by(|a, b| {
        b.score
            .cmp(&a.score)
            .then(docs[b.index].at.cmp(&docs[a.index].at))
            .then(docs[a.index].id.cmp(&docs[b.index].id))
    });
    hits
}

fn snippet(text: &str, terms: &[&str], max: usize) -> String {
    let clean = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let chars: Vec<char> = clean.chars().collect();
    // Match through char indices rather than slicing Unicode with lowercased byte offsets.
    let lower = clean.to_lowercase();
    let offset = terms
        .iter()
        .filter_map(|t| lower.find(t))
        .min()
        .unwrap_or(0);
    let mut lower_bytes = 0;
    let mut char_index: usize = 0;
    for c in &chars {
        if lower_bytes >= offset {
            break;
        }
        lower_bytes += c.to_lowercase().map(char::len_utf8).sum::<usize>();
        char_index += 1;
    }
    let start = char_index.saturating_sub(18);
    let end = (start + max).min(chars.len());
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        chars[start..end].iter().collect::<String>(),
        if end < chars.len() { "…" } else { "" }
    )
}

#[derive(Clone, Debug)]
pub struct FileRecord {
    pub path: PathBuf,
    pub name: String,
    pub text: String,
    pub at: i64,
    pub image: bool,
}
pub fn image_path(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "heic" | "bmp"
        )
    })
}
pub fn safe_document(path: &Path) -> bool {
    path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
        matches!(
            s.to_ascii_lowercase().as_str(),
            "txt"
                | "md"
                | "pdf"
                | "doc"
                | "docx"
                | "xlsx"
                | "xls"
                | "csv"
                | "ppt"
                | "pptx"
                | "png"
                | "jpg"
                | "jpeg"
                | "webp"
                | "gif"
                | "heic"
                | "mp3"
                | "m4a"
                | "wav"
                | "mp4"
                | "mov"
                | "json"
                | "log"
        )
    })
}
pub fn desktop_files(roots: &[PathBuf]) -> (Vec<FileRecord>, bool) {
    let mut stack = roots.to_vec();
    let mut seen = std::collections::HashSet::new();
    let mut files = Vec::new();
    let start = std::time::Instant::now();
    let mut visited = 0;
    let mut text_bytes = 0;
    while let Some(dir) = stack.pop() {
        if start.elapsed().as_secs() > 8 || visited >= 20_000 {
            return (files, true);
        }
        let Ok(canonical) = dir.canonicalize() else {
            continue;
        };
        if !seen.insert(canonical) {
            continue;
        }
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            if visited > 20_000 || start.elapsed().as_secs() > 8 {
                return (files, true);
            }
            let name = entry.file_name().to_string_lossy().to_string();
            if name.starts_with('.')
                || matches!(
                    name.as_str(),
                    "node_modules" | "target" | "vendor" | "build"
                )
            {
                continue;
            }
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                stack.push(entry.path());
                continue;
            }
            if !kind.is_file() {
                continue;
            }
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let at = meta
                .modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as i64)
                .unwrap_or(0);
            let path = entry.path();
            let text = if meta.len() <= 256 * 1024
                && text_bytes + meta.len() < 8 * 1024 * 1024
                && path.extension().and_then(|s| s.to_str()).is_some_and(|s| {
                    matches!(s.to_ascii_lowercase().as_str(), "md" | "txt" | "csv")
                }) {
                crate::search_index::file_text(&path, &meta).unwrap_or_else(|_| uncached_text_preview(&path))
            } else {
                String::new()
            };
            text_bytes += text.len() as u64;
            files.push(FileRecord {
                image: image_path(&path),
                path,
                name,
                text,
                at,
            });
        }
    }
    (files, false)
}

fn uncached_text_preview(path: &Path) -> String {
    // A cache failure is optional. It must not bypass the same bounds when a
    // file grows or changes type after its discovery metadata was captured.
    crate::local_store::read_bounded(path, 256 * 1024)
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

/// The firmware's filemanager_search_file opens a phone page; it is not a
/// background result API. This verified shared-storage read leaves the screen off.
pub fn phone_files(
    adb: &crate::adb::Adb,
    device: &crate::device::Device,
) -> anyhow::Result<(Vec<FileRecord>, bool)> {
    let transport = device.resolve_active_transport(adb)?;
    let script = r#"test -r /storage/emulated/0 || exit 1
find /storage/emulated/0 -type d \( -name '.*' -o -path '/storage/emulated/0/Android/data' -o -path '/storage/emulated/0/Android/obb' \) -prune -o -type f -print0 2>/dev/null | {
 n=0; while IFS= read -r -d '' p; do printf '%s\0' "$p"; n=$((n+1)); [ "$n" -ge 20001 ] && break; done
 :
}"#;
    let result = adb.run_timeout(
        &[
            "-s",
            &transport,
            "shell",
            "sh",
            "-c",
            &crate::adb::shell_quote(script),
        ],
        std::time::Duration::from_secs(12),
    )?;
    parse_phone_files(&result)
}
pub fn parse_phone_files(raw: &str) -> anyhow::Result<(Vec<FileRecord>, bool)> {
    let paths: Vec<_> = raw.split('\0').filter(|s| !s.is_empty()).collect();
    let limited = paths.len() > 20_000;
    let mut files = Vec::new();
    for path in paths.into_iter().take(20_000) {
        let Some(rest) = path.strip_prefix("/storage/emulated/0/") else {
            continue;
        };
        let logical = format!("/sdcard/{rest}");
        if crate::storage::validate_path(&logical).is_err() {
            continue;
        }
        let path = PathBuf::from(logical);
        let Some(name) = path.file_name().and_then(|s| s.to_str()).map(str::to_owned) else {
            continue;
        };
        files.push(FileRecord {
            image: image_path(&path),
            path,
            name,
            text: String::new(),
            at: 0,
        });
    }
    Ok((files, limited))
}

#[cfg(test)]
mod tests {
    use super::*;
    fn doc(id: &str, title: &str, body: &str, phone: bool) -> Document {
        Document {
            id: id.into(),
            title: title.into(),
            body: body.into(),
            kind: Kind::Message,
            phone,
            source: "微信".into(),
            at: 1,
        }
    }
    #[test]
    fn blank_search_lists_real_files_newest_first_and_respects_source() {
        let make = |id: &str, kind, phone, at| Document {
            id: id.into(), kind, phone, title: "项目方案".into(),
            body: String::new(), source: String::new(), at,
        };
        let docs = vec![
            make("page:Files", Kind::Function, false, 999),
            make("message:1", Kind::Message, true, 998),
            make("local:report.pdf", Kind::File, false, 100),
            make("phone:photo.png", Kind::Image, true, 200),
            make("local:draft.docx", Kind::Work, false, 150),
            make("studio:1", Kind::Work, false, 997),
        ];
        let indices = |query, kind, phone| find(&docs, query, kind, phone)
            .iter().map(|hit| hit.index).collect::<Vec<_>>();
        assert_eq!(indices("  ", None, None), vec![3, 4, 2]);
        assert_eq!(indices("", None, Some(true)), vec![3]);
        assert_eq!(indices("", None, Some(false)), vec![4, 2]);
        assert_eq!(indices("", Some(Kind::Message), None), vec![1]);
        assert_eq!(indices("项目方案", None, None).len(), docs.len(),
            "typing must restore all categories, including messages and functions");
    }
    #[test]
    fn exact_title_beats_body_and_preserves_unicode() {
        let d = vec![
            doc("a", "会议", "其他", true),
            doc("b", "同事", "周五会议商议计划", false),
        ];
        assert_eq!(find(&d, "会议", None, None)[0].index, 0);
        assert_eq!(find(&d, "周五 计划", None, None)[0].index, 1);
        assert_eq!(find(&d, "会议", None, Some(true)).len(), 1);
        assert!(find(&d, "会议", Some(Kind::File), None).is_empty());
    }
    #[test]
    fn date_tokens_filter_by_time_without_losing_other_keywords() {
        let mut a = doc("a", "会议", "方案", true);
        a.at = chrono::Local::now().timestamp_millis();
        let mut b = a.clone();
        b.id = "b".into();
        b.at -= 10 * 24 * 3600 * 1000;
        let data = vec![a, b];
        assert_eq!(find(&data, "今天 会议", None, None).len(), 1);
        assert_eq!(find(&data, "本周 方案", None, None).len(), 1);
        let date = chrono::Local::now().format("%Y-%m-%d").to_string();
        assert_eq!(find(&data, &date, None, None).len(), 1);
    }
    #[test]
    fn snippet_handles_case_expanding_unicode_and_long_chinese() {
        let text = format!("{}İ 中文关键词 结尾", "前文".repeat(80));
        assert!(snippet(&text, &["中文关键词"], 100).contains("中文关键词"));
        assert!(find(&[doc("a", "ABC", &text, true)], "abc", None, None).len() == 1);
    }
    #[test]
    fn phone_paths_are_literal_and_shared_storage_only() {
        let (r,_)=parse_phone_files("/storage/emulated/0/Download/报价 '新' $(x).pdf\0/data/token\0/storage/emulated/0/../secret\0").unwrap();
        assert_eq!(r.len(), 1);
        assert!(r[0].name.contains("$(x)"));
    }
    #[test]
    fn scan_does_not_follow_symlinks_or_hidden_credentials() {
        let t = tempfile::tempdir().unwrap();
        std::fs::write(t.path().join("说明.txt"), "中文全文可检索").unwrap();
        std::fs::write(t.path().join(".env"), "SECRET").unwrap();
        let (r, limited) = desktop_files(&[t.path().into(), t.path().into()]);
        assert!(!limited);
        assert_eq!(r.len(), 1);
        assert_eq!(r[0].text, "中文全文可检索");
        assert!(!safe_document(Path::new("tool.desktop")));
    }
    #[test]
    fn uncached_previews_do_not_read_files_that_grew_past_the_scan_limit() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("growing.txt");
        std::fs::write(&path, b"short").unwrap();
        let discovered = std::fs::metadata(&path).unwrap();
        assert!(discovered.len() < 256 * 1024);
        assert_eq!(uncached_text_preview(&path), "short");
        std::fs::write(&path, vec![b'x'; 256 * 1024 + 1]).unwrap();
        assert!(uncached_text_preview(&path).is_empty());
        assert_eq!(std::fs::metadata(&path).unwrap().len(), 256 * 1024 + 1);
    }
}
