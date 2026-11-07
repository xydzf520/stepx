//! Scoped preparation packs assembled from the actual calendar, source excerpts
//! and user-confirmed actions. No model inference or calendar mutation occurs.
use crate::{
    action_ledger,
    context_ledger::{self as ledger, outcomes},
    toolbox::PhoneCalendarEvent,
};
use anyhow::{ensure, Context, Result};
use chrono::{Local, TimeZone};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs,
    io::Write,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Brief {
    pub id: String,
    pub event_id: i64,
    pub title: String,
    pub path: PathBuf,
    pub markdown: String,
    pub source_count: usize,
    pub action_count: usize,
    pub linked_matters: Vec<String>,
    pub missing: Vec<String>,
    pub created_ms: i64,
}

#[derive(Default, Serialize, Deserialize)]
struct Links {
    schema: u32,
    scope_id: String,
    meetings: BTreeMap<String, BTreeSet<String>>,
}

pub fn event_key(event: &PhoneCalendarEvent) -> String {
    crate::work::cache_key_digest(
        format!("{}:{}:{}", event.source, event.id, event.dtstart).as_bytes(),
    )
}

fn directory(root: &Path, device: &str, account: Option<&str>) -> Result<PathBuf> {
    ensure!(!device.trim().is_empty(), "尚未确认设备，未读取会议资料");
    fs::create_dir_all(root)?;
    ensure!(
        !fs::symlink_metadata(root)?.file_type().is_symlink(),
        "资料目录不能是链接"
    );
    let scope = root.join(ledger::scope_id(device, account));
    fs::create_dir_all(&scope)?;
    ensure!(
        !fs::symlink_metadata(&scope)?.file_type().is_symlink(),
        "会议资料范围不能是链接"
    );
    let dir = scope.join("meeting-briefs");
    fs::create_dir_all(&dir)?;
    ensure!(
        !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "会议目录不能是链接"
    );
    ensure!(
        dir.canonicalize()?.starts_with(root.canonicalize()?),
        "会议资料超出当前范围"
    );
    Ok(dir)
}

fn read_links(dir: &Path, scope_id: &str) -> Result<Links> {
    let path = dir.join("links.json");
    if !path.exists() {
        return Ok(Links {
            schema: 1,
            scope_id: scope_id.into(),
            ..Default::default()
        });
    }
    let meta = fs::symlink_metadata(&path)?;
    ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 512 * 1024,
        "会议关联记录不可用"
    );
    let links: Links = serde_json::from_slice(&fs::read(path)?)?;
    ensure!(
        links.schema == 1 && links.scope_id == scope_id,
        "会议关联不属于当前账号"
    );
    Ok(links)
}

fn atomic_write(path: &Path, bytes: &[u8]) -> Result<()> {
    if let Ok(meta) = fs::symlink_metadata(path) {
        ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "会议文件不可覆盖"
        );
    }
    let mut file = tempfile::NamedTempFile::new_in(path.parent().context("会议路径无效")?)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    file.persist(path)?;
    Ok(())
}

pub fn manual_links(
    root: &Path,
    device: &str,
    account: Option<&str>,
) -> Result<BTreeMap<String, BTreeSet<String>>> {
    let scope = ledger::scope_id(device, account);
    if !root.join(&scope).join("meeting-briefs/links.json").exists() {
        return Ok(BTreeMap::new());
    }
    let dir = directory(root, device, account)?;
    Ok(read_links(&dir, &scope)?.meetings)
}

pub fn link_matter(
    root: &Path,
    device: &str,
    account: Option<&str>,
    event: &PhoneCalendarEvent,
    matter: &str,
    include: bool,
) -> Result<()> {
    let scope_id = ledger::scope_id(device, account);
    let overview = ledger::load(root, device, account)?;
    ensure!(
        overview
            .items
            .iter()
            .any(|i| i.id == matter && i.scope_id == scope_id),
        "事项已变化或不属于当前账号"
    );
    let dir = directory(root, device, account)?;
    let mut links = read_links(&dir, &scope_id)?;
    ensure!(
        links.meetings.len() < 200 || links.meetings.contains_key(&event_key(event)),
        "会议关联已达上限，请先整理历史会议"
    );
    let entry = links.meetings.entry(event_key(event)).or_default();
    if include {
        ensure!(
            entry.len() < 12 || entry.contains(matter),
            "每场会议最多关联12件事"
        );
        entry.insert(matter.into());
    } else {
        entry.remove(matter);
    }
    atomic_write(&dir.join("links.json"), &serde_json::to_vec_pretty(&links)?)
}

fn clipped(text: &str, limit: usize) -> String {
    let mut clean: String = text
        .chars()
        .filter(|ch| !ch.is_control() || matches!(ch, '\n' | '\t'))
        .take(limit)
        .collect();
    if text.chars().count() > limit {
        clean.push_str("…（摘录已截短，可返回来源核对）");
    }
    clean
}

fn quote(text: &str) -> String {
    text.lines().map(|line| format!("> {line}\n")).collect()
}

/// Automatically related matter must contain this exact calendar instance.
/// A shared word such as “会议” or a common participant never establishes a link.
pub fn automatic_matters<'a>(
    overview: &'a ledger::Overview,
    scope_id: &str,
    event: &PhoneCalendarEvent,
) -> Vec<&'a ledger::PreparedItem> {
    let object = format!("calendar:{}:{}:{}", event.source, event.id, event.dtstart);
    overview
        .items
        .iter()
        .filter(|item| {
            item.scope_id == scope_id
                && item.source_objects.iter().any(|source| source.id == object)
        })
        .collect()
}

pub fn create(
    root: &Path,
    device: &str,
    account: Option<&str>,
    event: &PhoneCalendarEvent,
    now_ms: i64,
) -> Result<Brief> {
    ensure!(
        !event.title.trim().is_empty() && event.dtstart > 0,
        "会议安排缺少标题或时间"
    );
    let scope_id = ledger::scope_id(device, account);
    let dir = directory(root, device, account)?;
    let overview = ledger::load(root, device, account)?;
    let links = read_links(&dir, &scope_id)?;
    let mut selected: BTreeSet<String> = automatic_matters(&overview, &scope_id, event)
        .into_iter()
        .map(|i| i.id.clone())
        .collect();
    if let Some(manual) = links.meetings.get(&event_key(event)) {
        selected.extend(manual.iter().cloned());
    }
    let selected: Vec<_> = overview
        .items
        .iter()
        .filter(|i| i.scope_id == scope_id && selected.contains(&i.id))
        .take(12)
        .collect();
    let selected_ids: BTreeSet<_> = selected.iter().map(|i| i.id.clone()).collect();
    let absolute = |at| {
        Local
            .timestamp_millis_opt(at)
            .single()
            .map(|v| v.format("%Y-%m-%d %H:%M %:z").to_string())
            .unwrap_or_else(|| "时间待核对".into())
    };
    let schedule = if event.all_day {
        format!(
            "{}（全天；结束边界 {}）",
            absolute(event.dtstart),
            absolute(event.dtend)
        )
    } else {
        format!("{} — {}", absolute(event.dtstart), absolute(event.dtend))
    };
    let mut markdown = format!(
        "# 会前准备包 · {}\n\n## 当前日历安排\n\n- 时间：{}\n- 地点：{}\n- 来源：{}\n\n",
        clipped(&event.title, 200),
        schedule,
        if event.location.trim().is_empty() {
            "尚未填写"
        } else {
            event.location.as_str()
        },
        event.source_label()
    );
    markdown.push_str(&format!("- 整理时间：{}\n\n", absolute(now_ms)));
    if let Some(url) =
        crate::home_hub::extract_meeting_url(&format!("{} {}", event.location, event.description))
    {
        markdown.push_str(&format!("- 入会链接：{url}\n\n"));
    }
    if !event.description.trim().is_empty() {
        markdown.push_str("日历备注原文：\n\n");
        markdown.push_str(&quote(&clipped(&event.description, 3000)));
        markdown.push('\n');
    }
    markdown.push_str("## 关联讨论与决定原文\n\n");
    let mut source_count = 0usize;
    for item in &selected {
        let packet = outcomes::prepare(
            root,
            device,
            account,
            &outcomes::Target::Item(item.id.clone()),
        )
        .context("关联资料已变化，请刷新会前准备包")?;
        ensure!(packet.scope_id == scope_id, "关联资料不属于当前会议账号");
        markdown.push_str(&format!("### {}\n\n", clipped(&item.title, 160)));
        for source in packet
            .sources
            .iter()
            .filter(|s| (s.authoritative || s.kind.starts_with("资料正文")) && s.kind != "计划日程")
            .take(8)
        {
            source_count += 1;
            markdown.push_str(&format!(
                "**来源{} · {}**（{}）\n\n",
                source_count,
                clipped(&source.source, 80),
                clipped(&source.locator, 160)
            ));
            if !source.authoritative {
                markdown.push_str("资料提取或转写内容，尚待核对，不代表用户决定：\n\n");
            }
            markdown.push_str(&quote(&clipped(&source.text, 1800)));
            markdown.push('\n');
        }
    }
    let mut missing = vec![];
    if selected.is_empty() {
        missing.push("尚未关联相关事项，请选择本场会议需要的讨论与资料。".into());
    }
    if source_count == 0 {
        markdown.push_str("尚无可核对的关联原文。可以先关联事项，或在资料收件箱处理本场会议的文档、截图和录音。\n\n");
    }
    let actions = action_ledger::load(root, device, account)?;
    let actions: Vec<_> = actions
        .actions
        .iter()
        .filter(|a| {
            a.matter_ids.iter().any(|id| selected_ids.contains(id))
                && !matches!(a.state, action_ledger::State::Dismissed)
        })
        .collect();
    markdown.push_str("## 已记录的行动与待决定事项\n\n");
    for action in &actions {
        let state = match action.state {
            action_ledger::State::Candidate => "待核对候选",
            action_ledger::State::Accepted => "已采纳",
            action_ledger::State::InProgress => "进行中",
            action_ledger::State::Completed if action.needs_review => {
                "此前标记完成，来源变化待复核"
            }
            action_ledger::State::Completed => "用户已确认完成",
            action_ledger::State::Deferred => "已延期",
            action_ledger::State::Dismissed => "已忽略",
        };
        markdown.push_str(&format!("- **{state}**：{}", clipped(&action.action, 600)));
        if let Some(owner) = &action.owner {
            markdown.push_str(&format!("；负责人：{}", clipped(&owner.value, 120)));
        }
        if let Some(due) = &action.due {
            markdown.push_str(&format!("；原约定期限：{}", clipped(&due.value, 120)));
        }
        markdown.push('\n');
    }
    if actions.is_empty() {
        markdown
            .push_str("尚无已关联行动。需要在行动跟踪中核对并采纳，或会前明确负责人和下一步。\n");
    }
    missing.push("录音和截图只有在完成内容解析后，才能作为会议内容依据。".into());
    missing.push("日历安排表示计划；关联原文中的变更、意见和行动状态仍可打开来源核对。".into());
    markdown.push_str("\n## 会前需要确认\n\n");
    for note in &missing {
        markdown.push_str(&format!("- {note}\n"));
    }
    ensure!(markdown.len() <= 256 * 1024, "准备包过大，请减少关联事项");
    let hash = crate::device::sha256_reader(markdown.as_bytes())?;
    let id = event_key(event);
    let name: String = event
        .title
        .chars()
        .filter(|c| {
            !c.is_control() && !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|')
        })
        .take(24)
        .collect();
    let path = dir.join(format!("会前准备包-{name}-{}.md", &hash[..12]));
    atomic_write(&path, markdown.as_bytes())?;
    Ok(Brief {
        id,
        event_id: event.id,
        title: event.title.clone(),
        path,
        markdown,
        source_count,
        action_count: actions.len(),
        linked_matters: selected_ids.into_iter().collect(),
        missing,
        created_ms: now_ms,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn calendar_association_requires_exact_scoped_instance() {
        let event = PhoneCalendarEvent {
            id: 3,
            source: "phone".into(),
            dtstart: 1000,
            title: "项目会议".into(),
            ..Default::default()
        };
        let source = ledger::SourceObject {
            id: "calendar:phone:3:1000".into(),
            ..Default::default()
        };
        let overview = ledger::Overview {
            items: vec![
                ledger::PreparedItem {
                    id: "yes".into(),
                    scope_id: "one".into(),
                    source_objects: vec![source.clone()],
                    ..Default::default()
                },
                ledger::PreparedItem {
                    id: "other-account".into(),
                    scope_id: "two".into(),
                    source_objects: vec![source],
                    ..Default::default()
                },
                ledger::PreparedItem {
                    id: "title-only".into(),
                    scope_id: "one".into(),
                    title: event.title.clone(),
                    ..Default::default()
                },
            ],
            ..Default::default()
        };
        assert_eq!(
            automatic_matters(&overview, "one", &event)
                .iter()
                .map(|i| i.id.as_str())
                .collect::<Vec<_>>(),
            vec!["yes"]
        );
        let mut recurring = event;
        recurring.dtstart = 2000;
        assert!(automatic_matters(&overview, "one", &recurring).is_empty());
    }
    #[test]
    fn unlinked_pack_retains_real_schedule_and_explicit_missing_context() {
        let root = tempfile::tempdir().unwrap();
        let event = PhoneCalendarEvent {
            id: 1,
            dtstart: 1_793_332_800_000,
            dtend: 1_793_336_400_000,
            title: "项目评审".into(),
            location: "B302".into(),
            description: "请核对最新预算".into(),
            ..Default::default()
        };
        let brief = create(root.path(), "phone", Some("a"), &event, 1_793_330_000_000).unwrap();
        assert!(brief.markdown.contains("B302") && brief.markdown.contains("请核对最新预算"));
        assert_eq!(brief.source_count, 0);
        assert_eq!(brief.action_count, 0);
        assert!(brief.markdown.contains("尚无可核对的关联原文"));
        assert!(brief.path.is_file());
        let second = create(root.path(), "phone", Some("b"), &event, 1_793_330_000_000).unwrap();
        assert_ne!(brief.path.parent(), second.path.parent());
    }
}
