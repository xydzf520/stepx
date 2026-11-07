//! Task records shared by chat, desktop file work and local app creation.
//! Requests are routed automatically (see `amoo_route`); the chosen route is
//! recorded on the task so the UI can show it and the user can correct it.
pub use crate::amoo_route::{detect_doctor_or_google_guide_query, DoctorQueryKind};
use anyhow::{ensure, Context, Result};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// Short, actionable text for the conversation; the step retains diagnostics.
pub fn failure_summary(detail: &str) -> String {
    if detail.contains("模型连续未给出有效文件操作") || detail.contains("桌面执行协议不是有效 JSON")
    {
        "这次文件生成没有完成：模型返回的文件结构有误。已生成的文件会保留，可以点击“重试”。".into()
    } else if detail.contains("未生成实际文件") || detail.contains("未写入实际文件") {
        "还没有收到可打开的文件，因此没有标记为完成。请点击“重试”，重新生成并检查成果。".into()
    } else {
        detail.to_owned()
    }
}

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Intent {
    /// Default: route automatically from the message text and attachments.
    #[default]
    Auto,
    /// Forced ordinary chat (e.g. "改为普通聊天").
    Chat,
    Files,
    Image,
    App,
    Screen,
}
impl Intent {
    pub fn label(self) -> &'static str {
        match self {
            Self::Auto => "自动",
            Self::Chat => "对话",
            Self::Files => "处理文件",
            Self::Image => "生成图片",
            Self::App => "制作应用",
            Self::Screen => "背屏创作",
        }
    }
}
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Executor {
    Phone,
    Files,
    Image,
    App,
    Screen,
}
impl Executor {
    pub fn label(self) -> &'static str {
        match self {
            Self::Phone => "手机 Amoo 回复",
            Self::Files => "处理资料并生成文件",
            Self::Image => "生成图片并接收原图",
            Self::App => "制作应用并运行检查",
            Self::Screen => "生成背屏点阵图案",
        }
    }
    pub fn source(self) -> &'static str {
        if self == Self::App {
            "手机 Amoo / 本机引擎 · 浏览器自检"
        } else {
            "手机 Amoo · 手机账号"
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
pub enum Phase {
    #[default]
    Ready,
    Running,
    Done,
    Failed,
    Stopped,
    Unconfirmed,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Step {
    pub executor: Executor,
    pub phase: Phase,
    pub note: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Turn {
    /// Source time of this turn. Old journals have no per-turn time; preserve that
    /// uncertainty instead of assigning the migration or latest task-update time.
    #[serde(default)]
    pub at_ms: Option<i64>,
    pub user: bool,
    pub text: String,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Output {
    pub path: PathBuf,
    pub name: String,
    pub kind: String,
    pub lab_id: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Receipt {
    pub path: PathBuf,
    pub note: String,
    pub success: bool,
    pub device: String,
    pub account: Option<String>,
    pub cloud_url: Option<String>,
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct Task {
    pub id: String,
    pub title: String,
    pub device: Option<String>,
    pub account: Option<String>,
    pub session: Option<String>,
    pub draft: String,
    pub attachments: Vec<PathBuf>,
    #[serde(default)]
    pub submitted_files: Vec<PathBuf>,
    pub intent: Intent,
    pub workspace: Option<PathBuf>,
    pub turns: Vec<Turn>,
    pub steps: Vec<Step>,
    pub outputs: Vec<Output>,
    #[serde(default)]
    pub receipts: Vec<Receipt>,
    pub note: String,
    pub updated: i64,
    /// Route chosen automatically for the latest request, shown as an inline,
    /// correctable hint (e.g. "生成 PPT"). Empty for plain chat.
    #[serde(default)]
    pub route_note: String,
    /// Sticky route for follow-up edits ("改成蓝色" continues the last artifact).
    #[serde(default)]
    pub route: Option<Intent>,
    #[serde(default)]
    pub pinned: bool,
}
impl Task {
    /// Keep retries in the journal, but present one latest card per app build.
    /// Documents and distinct builds remain separate deliverables.
    pub fn current_output_indices(&self) -> Vec<usize> {
        let mut builds = std::collections::HashSet::new();
        let mut indices = Vec::new();
        for (i, output) in self.outputs.iter().enumerate().rev() {
            let build = output
                .lab_id
                .as_deref()
                .and_then(|id| id.rsplit_once('-'))
                .filter(|(_, round)| round.parse::<usize>().is_ok())
                .map(|(id, _)| id);
            if build.is_none_or(|id| builds.insert(id)) {
                indices.push(i);
            }
        }
        indices.reverse();
        indices
    }
    pub fn new(device: Option<String>, account: Option<String>) -> Self {
        Self {
            id: format!("task-{:032x}", rand::random::<u128>()),
            title: "新对话".into(),
            device,
            account,
            session: None,
            draft: String::new(),
            attachments: vec![],
            submitted_files: vec![],
            intent: Intent::Auto,
            workspace: None,
            turns: vec![],
            steps: vec![],
            outputs: vec![],
            receipts: vec![],
            note: String::new(),
            updated: chrono::Utc::now().timestamp(),
            route_note: String::new(),
            route: None,
            pinned: false,
        }
    }
    pub fn visible(&self, device: Option<&str>, account: Option<&str>) -> bool {
        if let Some(d) = self.device.as_deref() {
            if Some(d) != device {
                return false;
            }
        }
        self.account.as_deref().is_none_or(|a| Some(a) == account)
    }
    pub fn add_output(&mut self, output: Output) {
        if let Some(existing) = self.outputs.iter_mut().find(|v| v.path == output.path) {
            *existing = output;
        } else {
            self.outputs.push(output);
        }
    }
    /// Record an automatic or forced routing decision for the latest request.
    /// Chat clears the inline note; an explicit "chat only" also clears the
    /// sticky follow-up route, while ordinary chat keeps it for later edits.
    pub fn apply_route(&mut self, decision: &crate::amoo_route::RouteDecision) {
        self.route_note = decision.label.to_string();
        if decision.opt_out {
            self.route = None;
        } else if let Some(sticky) = decision.sticky {
            self.route = Some(sticky);
        }
    }
    pub fn meaningful(&self) -> bool {
        !self.draft.trim().is_empty()
            || !self.attachments.is_empty()
            || !self.turns.is_empty()
            || !self.outputs.is_empty()
            || self.session.is_some()
    }
}

pub fn clean_task_title(raw: &str) -> String {
    let mut s = raw
        .trim()
        .trim_start_matches("[仅普通对话，不启动本地文档或网页生成]")
        .trim()
        .lines()
        .next()
        .unwrap_or("")
        .trim();
    if let Some(rest) = s.strip_prefix("围绕我的手机日程/提醒「") {
        if let Some((inner, _)) = rest.split_once('」') {
            let inner = inner.trim();
            if !inner.is_empty() {
                return format!("日程：{inner}");
            }
        }
        s = rest.trim_end_matches('」').trim();
    } else if let Some(rest) = s.strip_prefix("请帮我分析这张来自手机相册的图片") {
        let rest = rest
            .trim()
            .trim_start_matches('（')
            .trim_start_matches('(')
            .trim_end_matches('）')
            .trim_end_matches(')')
            .trim();
        if rest.is_empty() {
            return "分析手机相册图片".into();
        }
        return format!("相册图片：{rest}")
            .chars()
            .take(22)
            .collect::<String>();
    }
    if s.is_empty() {
        "新对话".into()
    } else {
        let mut out: String = s.chars().take(24).collect();
        if s.chars().count() > 24 || out.ends_with("，2") {
            if let Some((prefix, tail)) = out.rsplit_once(['，', '、', ',']) {
                if !prefix.is_empty() && tail.chars().count() <= 2 {
                    out = prefix.to_string();
                }
            }
        }
        out
    }
}

pub fn is_automated_test_task(task: &Task) -> bool {
    const TEST_MARKERS: &[&str] = &[
        "只修改附件 Word：把项目预算从2580改为300",
        "这是 StepX Companion 统一任务验收",
        "这是 StepX Companion 对话验收",
        "这是 StepX Companion 聊天验收",
        "这是 StepX Companion 多附件验收",
        "这是第二条测试对话，请只回答 23 加 19",
        "验收办公场景：读取附件 CSV",
        "这是键盘发送验收。记住测试数 47",
        "请直接阅读这三个附件，不要创建任务或生成文件",
        "根据附件做三份可编辑文件：1. sales.xlsx",
        "图片生成时输入的新草稿，必须保留",
        "保留草稿TEST",
        "打开关于手机，查看 Android 版本，不修改设置",
        "STEPX_VERIFY",
    ];
    let matches_marker = |text: &str| TEST_MARKERS.iter().any(|m| text.contains(m));
    if matches_marker(&task.title) || matches_marker(&task.draft) {
        return true;
    }
    task.turns.iter().any(|t| matches_marker(&t.text))
}

pub fn is_deduplicatable_draft_task(task: &Task) -> bool {
    if task.session.is_some() || !task.outputs.is_empty() || !task.receipts.is_empty() {
        return false;
    }
    let unsent_draft_only = task.turns.is_empty()
        && task.steps.is_empty()
        && (!task.draft.trim().is_empty() || !task.attachments.is_empty());
    let uncompleted_single_turn = !task.steps.is_empty()
        && !task.steps.iter().any(|s| s.phase == Phase::Done)
        && task.turns.iter().filter(|t| t.user).count() <= 1;
    unsent_draft_only || uncompleted_single_turn
}

pub fn draft_task_signature(task: &Task) -> Option<(String, Vec<PathBuf>)> {
    if !is_deduplicatable_draft_task(task) {
        return None;
    }
    let prompt = if !task.draft.trim().is_empty() {
        task.draft.trim().to_string()
    } else {
        task.turns
            .iter()
            .find(|t| t.user)
            .map(|t| t.text.trim().to_string())
            .unwrap_or_default()
    };
    let files = if !task.attachments.is_empty() {
        task.attachments.clone()
    } else {
        task.submitted_files.clone()
    };
    if prompt.is_empty() && files.is_empty() {
        None
    } else {
        Some((prompt, files))
    }
}

fn merge_draft_task(target: &mut Task, source: Task) {
    let prefer_source_turns = (target.turns.is_empty() && !source.turns.is_empty())
        || (!source.turns.is_empty() && source.updated > target.updated);
    if prefer_source_turns {
        target.turns = source.turns;
        target.steps = source.steps;
        target.note = source.note;
        target.route = source.route;
        target.route_note = source.route_note;
        if target.workspace.is_none() {
            target.workspace = source.workspace;
        }
        if target.submitted_files.is_empty() {
            target.submitted_files = source.submitted_files;
        }
    }
    let target_has_active_or_done_step = target
        .steps
        .iter()
        .any(|s| matches!(s.phase, Phase::Ready | Phase::Running | Phase::Done));
    if target_has_active_or_done_step {
        target.draft.clear();
        target.attachments.clear();
    } else {
        if target.draft.trim().is_empty() && !source.draft.trim().is_empty() {
            target.draft = source.draft;
        }
        if target.attachments.is_empty() && !source.attachments.is_empty() {
            target.attachments = source.attachments;
        }
    }
    if target.device.is_none() {
        target.device = source.device;
    }
    if target.account.is_none() {
        target.account = source.account;
    }
    target.pinned |= source.pinned;
    target.updated = target.updated.max(source.updated);
}

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
pub struct Journal {
    #[serde(skip)]
    persisted_revision: std::cell::Cell<Option<i64>>,
    pub tasks: Vec<Task>,
    #[serde(default)]
    pub deleted_sessions: Vec<String>,
    #[serde(default)]
    pub pinned_sessions: std::collections::HashSet<String>,
}
impl Journal {
    /// Deduplicate sessionless draft-only or failed single-turn tasks that share
    /// the same normalized prompt, attachments, and device/account scope.
    /// If `active_id` was merged into another task, returns the surviving task ID.
    pub fn deduplicate_draft_tasks(&mut self, active_id: Option<&str>) -> Option<String> {
        let mut replacement_active: Option<String> = None;
        let mut i = 0;
        while i < self.tasks.len() {
            let Some(sig_i) = draft_task_signature(&self.tasks[i]) else {
                i += 1;
                continue;
            };
            let mut j = i + 1;
            let mut removed_i = false;
            while j < self.tasks.len() {
                let is_dup = draft_task_signature(&self.tasks[j])
                    .is_some_and(|sig_j| sig_j == sig_i)
                    && (self.tasks[i].device.is_none()
                        || self.tasks[j].device.is_none()
                        || self.tasks[i].device == self.tasks[j].device)
                    && (self.tasks[i].account.is_none()
                        || self.tasks[j].account.is_none()
                        || self.tasks[i].account == self.tasks[j].account);
                if !is_dup {
                    j += 1;
                    continue;
                }
                if active_id == Some(self.tasks[j].id.as_str()) {
                    let src = self.tasks.remove(i);
                    // Now the former `j` is at index `j - 1`
                    merge_draft_task(&mut self.tasks[j - 1], src);
                    removed_i = true;
                    break;
                } else {
                    let src = self.tasks.remove(j);
                    if active_id == Some(src.id.as_str()) {
                        replacement_active = Some(self.tasks[i].id.clone());
                    }
                    merge_draft_task(&mut self.tasks[i], src);
                }
            }
            if !removed_i {
                i += 1;
            }
        }
        replacement_active
    }

    pub fn load(path: &Path) -> Result<Self> {
        let (mut value, _) = read_journal(path)?;
        let before_snapshot = value.clone();
        let mut deleted_sessions = value.deleted_sessions;
        value.tasks.retain(|task| {
            if is_automated_test_task(task) {
                if let Some(key) = &task.session {
                    if !key.trim().is_empty() && !deleted_sessions.contains(key) {
                        deleted_sessions.push(key.clone());
                    }
                }
                return false;
            }
            let completely_empty = task.turns.is_empty()
                && task.outputs.is_empty()
                && task.session.is_none()
                && task.steps.is_empty()
                && task.submitted_files.is_empty()
                && task.attachments.is_empty()
                && task.draft.trim().is_empty();
            !completely_empty
        });
        value.deleted_sessions = deleted_sessions;
        // A closed process is never evidence that a remote task completed.
        for task in &mut value.tasks {
            while task.turns.last().is_some_and(|t| {
                !t.user && t.text.trim() == "No such file or directory (os error 2)"
            }) {
                task.turns.pop();
                if task.turns.last().is_some_and(|t| t.user) {
                    task.turns.pop();
                }
            }
            task.turns
                .retain(|t| t.text.trim() != "No such file or directory (os error 2)");
            task.steps
                .retain(|s| s.note.trim() != "No such file or directory (os error 2)");
            if task.note.trim() == "No such file or directory (os error 2)" {
                task.note.clear();
            }
            if task.turns.is_empty() {
                if let Some(first_out) = task.outputs.first() {
                    if !first_out.name.trim().is_empty() {
                        task.title = clean_task_title(&first_out.name);
                    }
                }
            }
            task.title = clean_task_title(&task.title);
            if matches!(task.title.as_str(), "新对话" | "新任务") {
                if let Some(first_user) = task.turns.iter().find(|t| t.user) {
                    task.title = clean_task_title(&first_user.text);
                } else if let Some(first_out) = task.outputs.first() {
                    if !first_out.name.trim().is_empty() {
                        task.title = clean_task_title(&first_out.name);
                    }
                }
            } else if let Some(first_user) = task.turns.iter().find(|t| t.user) {
                let better = clean_task_title(&first_user.text);
                if better.starts_with("日程：") || better.starts_with("相册图片：") {
                    task.title = better;
                }
            }
            for step in &mut task.steps {
                if step.phase == Phase::Running {
                    step.phase = Phase::Unconfirmed;
                    step.note = "上次执行已中断；手机任务需同步核对，不会自动重发".into();
                }
            }
            if !task.steps.is_empty()
                && task.steps.iter().all(|s| s.phase == Phase::Done)
                && task
                    .turns
                    .iter()
                    .rev()
                    .find(|t| t.user)
                    .is_some_and(|u| u.text.trim() == task.draft.trim())
            {
                task.draft.clear();
            }
            // Routing is automatic. Older records stored `Chat` as the default
            // mode; a forced route is one-shot and never restored on launch.
            if task.intent == Intent::Chat
                || (task.draft.trim().is_empty() && task.attachments.is_empty())
            {
                task.intent = Intent::Auto;
            }
        }
        value.deduplicate_draft_tasks(None);
        if !value.same_contents(&before_snapshot) {
            value.save(path)?;
        }
        Ok(value)
    }
    fn same_contents(&self, other: &Self) -> bool {
        self.tasks == other.tasks
            && self.deleted_sessions == other.deleted_sessions
            && self.pinned_sessions == other.pinned_sessions
    }
    /// Synchronous callers keep the revision captured by load/previous save.
    /// The UI uses JournalWriter directly to retain its typed baseline.
    pub fn save(&self, path: &Path) -> Result<()> {
        let (saved, created) = read_journal(path)?;
        ensure!(
            self.persisted_revision.get() == saved.persisted_revision.get()
                || (self.persisted_revision.get().is_none() && created),
            "任务记录已变化，请重新读取；未覆盖其他写入或未保存草稿"
        );
        let mut writer = JournalWriter::from_loaded(path.to_path_buf(), saved)?;
        let mut pending = self.clone();
        writer.commit(&mut pending)?;
        self.persisted_revision
            .set(writer.saved.persisted_revision.get());
        Ok(())
    }
}

const JOURNAL_LIMIT: usize = 16 * 1024 * 1024;

fn journal_key(path: &Path) -> Result<String> {
    crate::data_store::key(path)
}

/// Import the original JSON once, retaining it unchanged. Missing sections from
/// older journals are added without rewriting existing task rows. Read rows and
/// their optimistic revision in one transaction, including on first startup.
fn read_journal(path: &Path) -> Result<(Journal, bool)> {
    use rusqlite::{params, OptionalExtension, TransactionBehavior};
    let db = crate::data_store::database_for(path);
    if !crate::data_store::has_snapshot(path)? {
        let _: Option<Journal> = crate::data_store::read(path, JOURNAL_LIMIT as u64)?;
    }
    let name = journal_key(path)?;
    crate::data_store::with_connection(&db, |conn| {
        let tx = conn.transaction_with_behavior(TransactionBehavior::Immediate)?;
        let existing: Option<String> = tx
            .query_row("SELECT shape FROM snapshots WHERE name=?", [&name], |r| {
                r.get(0)
            })
            .optional()?;
        let created = existing.is_none();
        let mut shape: serde_json::Value = match existing.as_ref() {
            Some(raw) => serde_json::from_str(raw)?,
            None => serde_json::json!({"root_array":false,"sections":{}}),
        };
        ensure!(shape["root_array"] == false, "任务记录结构无效，原数据保留");
        let sections = shape["sections"].as_object_mut().context("任务分区无效")?;
        let mut upgraded = false;
        for section in ["tasks", "deleted_sessions", "pinned_sessions"] {
            match sections.get(section) {
                Some(v) => ensure!(*v == true, "任务分区不是数组，原数据保留"),
                None => {
                    sections.insert(section.into(), true.into());
                    upgraded = true;
                }
            }
        }
        let now = chrono::Utc::now().timestamp_millis();
        if created {
            tx.execute(
                "INSERT INTO snapshots(name,head,shape,updated) VALUES(?1,'{}',?2,?3)",
                params![name, serde_json::to_string(&shape)?, now],
            )?;
        } else if upgraded {
            tx.execute(
                "UPDATE snapshots SET shape=?2,revision=revision+1,updated=?3 WHERE name=?1",
                params![name, serde_json::to_string(&shape)?, now],
            )?;
        }
        let revision: i64 = tx.query_row(
            "SELECT revision FROM snapshots WHERE name=?",
            [&name],
            |r| r.get(0),
        )?;
        let mut journal = Journal::default();
        let mut bytes = 0usize;
        {
            let mut stmt = tx.prepare_cached(
                "SELECT section,body FROM records WHERE snapshot=? ORDER BY section,position,id",
            )?;
            let mut rows = stmt.query([&name])?;
            while let Some(row) = rows.next()? {
                let section = row.get_ref(0)?.as_str()?;
                let body = row.get_ref(1)?.as_str()?;
                bytes = bytes.saturating_add(body.len() + 1);
                ensure!(bytes < JOURNAL_LIMIT, "任务记录过大，原数据保留");
                match section {
                    "tasks" => journal.tasks.push(serde_json::from_str(body)?),
                    "deleted_sessions" => {
                        journal.deleted_sessions.push(serde_json::from_str(body)?)
                    }
                    "pinned_sessions" => {
                        journal.pinned_sessions.insert(serde_json::from_str(body)?);
                    }
                    _ => {}
                }
            }
        }
        tx.commit()?;
        journal.persisted_revision.set(Some(revision));
        Ok((journal, created))
    })
}

/// A worker-owned committed baseline. Incoming UI journals remain intact on
/// failure. Success swaps ownership, avoiding another full history clone.
pub struct JournalWriter {
    path: PathBuf,
    saved: Journal,
    sizes: JournalSize,
}
struct JournalSize {
    rows: std::collections::HashMap<(String, String), usize>,
    sections: std::collections::HashMap<String, (bool, usize)>,
    framing: usize,
    row_bytes: usize,
}
impl JournalWriter {
    pub fn from_loaded(path: PathBuf, saved: Journal) -> Result<Self> {
        ensure!(
            saved.persisted_revision.get().is_some(),
            "任务记录未成功读取，不能启动保存"
        );
        let name = journal_key(&path)?;
        let sizes = crate::data_store::with_connection(&crate::data_store::database_for(&path), |conn| {
            let tx = conn.transaction()?;
            let (head, shape): (String,String) = tx.query_row(
                "SELECT head,shape FROM snapshots WHERE name=?", [&name],
                |r| Ok((r.get(0)?,r.get(1)?)),
            )?;
            // Keep an initially stale writer alive so its queued draft can be
            // reported/recovered. apply_delta checks the original revision
            // atomically; this size account never permits rebasing that draft.
            let shape: serde_json::Value = serde_json::from_str(&shape)?;
            let fields = shape["sections"].as_object().context("任务分区无效")?;
            let mut sections = std::collections::HashMap::new();
            let mut framing = head.len();
            for (section,array) in fields {
                framing += serde_json::to_string(section)?.len() + 3; // key, colon, [] / {}
                sections.insert(section.clone(), (array.as_bool().context("任务分区无效")?,0usize));
            }
            framing += if head == "{}" { fields.len().saturating_sub(1) } else { fields.len() };
            let mut rows = std::collections::HashMap::new();
            let mut row_bytes = 0usize;
            {
                let mut statement = tx.prepare("SELECT section,id,length(CAST(body AS BLOB)) FROM records WHERE snapshot=?")?;
                let mut values = statement.query([&name])?;
                while let Some(row) = values.next()? {
                    let section: String=row.get(0)?;let id:String=row.get(1)?;
                    let (array,count)=sections.get_mut(&section).context("任务分区无效")?;
                    let bytes=usize::try_from(row.get::<_,i64>(2)?).context("任务容量状态无效")?
                        + if *array {0} else {serde_json::to_string(&id)?.len()+1};
                    row_bytes += bytes;*count+=1;rows.insert((section,id),bytes);
                }
            }
            tx.commit()?;
            Ok(JournalSize {rows,sections,framing,row_bytes})
        })?;
        Ok(Self { path, saved, sizes })
    }
    /// On success `pending` receives the previous baseline and may be dropped.
    /// On error it is unchanged and must be retained for retry/recovery.
    pub fn commit(&mut self, pending: &mut Journal) -> Result<usize> {
        use std::collections::{HashMap, HashSet};
        let revision = self
            .saved
            .persisted_revision
            .get()
            .context("任务修订缺失")?;
        let previous: HashMap<_, _> = self
            .saved
            .tasks
            .iter()
            .enumerate()
            .map(|(i, t)| (t.id.as_str(), (i, t)))
            .collect();
        ensure!(
            previous.len() == self.saved.tasks.len(),
            "旧任务标识重复，原数据保留"
        );
        let mut ids = HashSet::new();
        let mut changes: Vec<(&str, String, Option<i64>, serde_json::Value)> = Vec::new();
        for (position, task) in pending.tasks.iter().enumerate() {
            ensure!(
                !task.id.is_empty() && ids.insert(task.id.as_str()),
                "任务标识重复或为空，未覆盖原数据"
            );
            let old = previous.get(task.id.as_str());
            if old.is_some_and(|(i, t)| *i == position && *t == task) {
                continue;
            }
            changes.push((
                "tasks",
                task.id.clone(),
                if old.is_some_and(|(i, _)| *i == position) {
                    None
                } else {
                    Some(position as i64)
                },
                serde_json::to_value(task)?,
            ));
        }
        let mut deletes: Vec<(&str, String)> = self
            .saved
            .tasks
            .iter()
            .filter(|t| !ids.contains(t.id.as_str()))
            .map(|t| ("tasks", t.id.clone()))
            .collect();
        fn strings(
            section: &'static str,
            old: &[String],
            new: &[String],
            changes: &mut Vec<(&'static str, String, Option<i64>, serde_json::Value)>,
            deletes: &mut Vec<(&'static str, String)>,
        ) {
            for (i, value) in new.iter().enumerate() {
                if old.get(i) != Some(value) {
                    changes.push((
                        section,
                        format!("#{i}"),
                        Some(i as i64),
                        value.clone().into(),
                    ));
                }
            }
            deletes.extend((new.len()..old.len()).map(|i| (section, format!("#{i}"))));
        }
        strings(
            "deleted_sessions",
            &self.saved.deleted_sessions,
            &pending.deleted_sessions,
            &mut changes,
            &mut deletes,
        );
        if self.saved.pinned_sessions != pending.pinned_sessions {
            // HashSet serialization order is not stable. Rewrite this small
            // metadata section by explicit position only when the set changes.
            let mut new: Vec<_> = pending.pinned_sessions.iter().cloned().collect();
            new.sort();
            for (i, value) in new.iter().enumerate() {
                changes.push((
                    "pinned_sessions",
                    format!("#{i}"),
                    Some(i as i64),
                    value.clone().into(),
                ));
            }
            deletes.extend(
                (new.len()..self.saved.pinned_sessions.len())
                    .map(|i| ("pinned_sessions", format!("#{i}"))),
            );
        }
        // Track the exact persisted JSON size, including framing, escaped keys,
        // UTF-8 bodies and unknown retained sections. Only changed rows are
        // serialized; do not rewrite the whole journal to check each keystroke.
        let mut size_changes = std::collections::HashMap::new();
        for (section,id) in &deletes { size_changes.insert(((*section).to_string(),id.clone()),None); }
        for (section,id,_,value) in &changes {
            let array=self.sizes.sections.get(*section).context("任务分区无效")?.0;
            let bytes=serde_json::to_vec(value)?.len()+if array {0}else{serde_json::to_string(id)?.len()+1};
            size_changes.insert(((*section).to_string(),id.clone()),Some(bytes));
        }
        let mut sections=self.sizes.sections.clone();
        let mut row_bytes=self.sizes.row_bytes;
        for (key,next) in &size_changes {
            let old=self.sizes.rows.get(key).copied();
            row_bytes=row_bytes.checked_sub(old.unwrap_or(0)).context("任务容量状态无效")?
                .checked_add(next.unwrap_or(0)).context("任务容量溢出")?;
            let count=&mut sections.get_mut(&key.0).context("任务分区无效")?.1;
            *count=*count-usize::from(old.is_some())+usize::from(next.is_some());
        }
        let total=self.sizes.framing.saturating_add(row_bytes)
            .saturating_add(sections.values().map(|(_,count)|count.saturating_sub(1)).sum::<usize>());
        ensure!(total<=JOURNAL_LIMIT,"任务记录超过保存容量，未覆盖原数据；当前草稿保留，请先另存整理已有对话");
        let records: Vec<_> = changes
            .iter()
            .map(
                |(section, id, position, value)| crate::data_store::RecordChange {
                    section,
                    id,
                    position: *position,
                    value: value.clone(),
                },
            )
            .collect();
        let removals: Vec<_> = deletes
            .iter()
            .map(|(section, id)| (*section, id.as_str()))
            .collect();
        let changed =
            crate::data_store::apply_delta(&self.path, revision, None, &records, &removals)?;
        self.sizes.row_bytes=row_bytes;self.sizes.sections=sections;
        for (key,next) in size_changes {
            if let Some(bytes)=next {self.sizes.rows.insert(key,bytes);} else {self.sizes.rows.remove(&key);}
        }
        pending
            .persisted_revision
            .set(Some(revision + i64::from(changed > 0)));
        std::mem::swap(&mut self.saved, pending);
        Ok(changed)
    }
}

/// Debounced persistence with a retained pending journal and visible failures.
/// A conflicting writer is never silently rebased onto stale UI contents.
pub fn write_journal_updates(
    path: PathBuf,
    initial: Journal,
    updates: std::sync::mpsc::Receiver<Journal>,
    status: std::sync::mpsc::Sender<std::result::Result<(), String>>,
) {
    use std::{sync::mpsc::RecvTimeoutError, time::Duration};
    let mut writer = match JournalWriter::from_loaded(path.clone(), initial) {
        Ok(writer) => writer,
        Err(error) => {
            let _ = status.send(Err(error.to_string()));
            return;
        }
    };
    let mut pending = None;
    let mut disconnected = false;
    let mut last_error = None;
    loop {
        if pending.is_none() {
            match updates.recv() {
                Ok(next) => pending = Some(next),
                Err(_) => break,
            }
        }
        while !disconnected {
            match updates.recv_timeout(Duration::from_millis(if last_error.is_some() {
                1000
            } else {
                300
            })) {
                Ok(next) => pending = Some(next),
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => disconnected = true,
            }
        }
        match writer.commit(pending.as_mut().unwrap()) {
            Ok(_) => {
                pending = None;
                last_error = None;
                let _ = status.send(Ok(()));
            }
            Err(error) => {
                let message = format!("对话草稿尚未保存：{error}");
                if last_error.as_ref() != Some(&message) {
                    let _ = status.send(Err(message.clone()));
                    last_error = Some(message);
                }
                if disconnected {
                    // Keep an explicit recovery artifact when closing after a
                    // conflict/error. Never replace the committed database or
                    // original migration JSON with the uncommitted snapshot.
                    let recovery = path.with_file_name(format!(
                        "tasks-unsaved-{}.json",
                        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default()
                    ));
                    let result = (|| -> Result<()> {
                        let raw = serde_json::to_vec(
                            &serde_json::json!({"base_revision":writer.saved.persisted_revision.get(),"journal":pending.as_ref().unwrap()}),
                        )?;
                        crate::local_store::write_if_changed(&recovery, &raw)?;
                        Ok(())
                    })();
                    let _ = status.send(Err(match result {
                        Ok(()) => format!("对话未提交；草稿恢复副本：{}", recovery.display()),
                        Err(error) => format!("对话未提交，草稿恢复副本也未能保存：{error}"),
                    }));
                    break;
                }
            }
        }
        if disconnected {
            break;
        }
    }
}

pub fn is_unconfirmed_placeholder(text: &str) -> bool {
    let trimmed = text.trim();
    matches!(
        trimmed,
        "已停止等待；手机任务可能仍继续，可同步查看结果"
            | "上次执行已中断；手机任务需同步核对，不会自动重发"
            | "手机连接中断，尚未确认回复完成。重连后同步任务记录；未自动重发。"
            | "桌面生成流程已中断；请核对当前结果后再继续"
    )
}
/// Live, side-effect-free preview of what Enter will do with the current draft.
/// Returns `None` for plain chat; otherwise the route and a short human label
/// (e.g. "生成 PPT", "做小游戏") for the composer hint chip.
/// SHARED CONTRACT: routing agent owns the implementation; UI only calls it.
pub fn preview_route(
    prompt: &str,
    has_files: bool,
    sticky: Option<Intent>,
) -> Option<(Intent, &'static str)> {
    if prompt.trim().is_empty() {
        return None;
    }
    let ctx = crate::amoo_route::RouteCtx {
        forced: None,
        sticky,
        last: None,
    };
    let d = crate::amoo_route::route_attached(
        prompt,
        crate::amoo_route::Attached::unknown(has_files),
        &ctx,
    );
    (!d.is_chat()).then_some((d.intent, d.label))
}

/// Primary automatic route of a message without conversation context.
pub fn detect_explicit_intent(prompt: &str, has_files: bool) -> Intent {
    crate::amoo_route::route_attached(
        prompt,
        crate::amoo_route::Attached::unknown(has_files),
        &crate::amoo_route::RouteCtx::default(),
    )
    .intent
}

/// Ordered executors for a message. `Intent::Auto` routes automatically;
/// any other intent is an explicit, forced route (`Chat` = chat only).
pub fn propose(prompt: &str, intent: Intent, attachments: bool) -> Vec<Executor> {
    propose_followup(prompt, intent, attachments, None)
}

/// Follow-up edits continue the last executor; questions stay ordinary chat.
pub fn propose_followup(
    prompt: &str,
    intent: Intent,
    attachments: bool,
    last: Option<Executor>,
) -> Vec<Executor> {
    let ctx = crate::amoo_route::RouteCtx {
        forced: Some(intent),
        sticky: None,
        last,
    };
    crate::amoo_route::route_attached(
        prompt,
        crate::amoo_route::Attached::unknown(attachments),
        &ctx,
    )
    .steps
}

/// Only text results from the accepted task are sent to the app model, bounded
/// in size. No arbitrary directory scan or executable contents are read.
pub fn app_context(task: &Task) -> Result<String> {
    let Some(root) = &task.workspace else {
        return Ok(String::new());
    };
    if !root.exists() {
        return Ok(String::new());
    }
    let Ok(root) = root.canonicalize() else {
        return Ok(String::new());
    };
    let mut text = String::new();
    for file in &task.outputs {
        if file.lab_id.is_some() {
            continue;
        }
        let extension = file
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        let is_text = ["txt", "md", "csv", "json"].contains(&extension.as_str());
        let is_office = ["docx", "xlsx", "pptx"].contains(&extension.as_str());
        if !is_text && !is_office {
            continue;
        }
        let Ok(path) = file.path.canonicalize() else {
            continue;
        };
        ensure!(path.starts_with(&root), "成果不在此任务的资料目录中");
        let value = if is_office {
            let Ok(rel) = path.strip_prefix(&root) else {
                continue;
            };
            match crate::work::Workspace::open(&root)
                .and_then(|ws| ws.read_text(&rel.to_string_lossy()))
            {
                Ok(v) => v.chars().take(3000).collect::<String>(),
                Err(_) => continue,
            }
        } else {
            let Ok(meta) = std::fs::metadata(&path) else {
                continue;
            };
            ensure!(meta.len() <= 1024 * 1024, "文本成果过大，请选择更小的资料");
            let Ok(v) = std::fs::read_to_string(path) else {
                continue;
            };
            v
        };
        let remaining = 24000usize.saturating_sub(text.chars().count());
        if remaining == 0 {
            break;
        }
        text.push_str(&format!(
            "\n资料 {}：\n{}\n",
            file.name,
            value.chars().take(remaining.min(12000)).collect::<String>()
        ));
    }
    Ok(text)
}

#[cfg(test)]
mod tests {
    use super::*;
    fn journal_fixture(count: usize) -> Journal {
        let mut journal = Journal::default();
        journal.tasks = (0..count)
            .map(|i| {
                let mut task = Task::new(None, None);
                task.id = format!("task-{i}");
                task.title = format!("记录 {i}");
                task.draft = format!("第 {i} 项未发送草稿🙂");
                task.turns = vec![Turn {
                    at_ms: Some(100 + i as i64),
                    user: false,
                    text: format!("历史正文 {i}"),
                }];
                task
            })
            .collect();
        journal
    }
    #[test]
    fn journal_capacity_includes_json_framing_unicode_and_retained_sections() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tasks.json");
        let original=journal_fixture(2);
        let mut raw=serde_json::to_value(&original).unwrap();
        raw["保留的\"头🙂"]=serde_json::json!("原头\"\\🙂");
        raw["未知\"分区🙂"]=serde_json::json!({"转义\"键🙂":"旧内容\"\\🙂"});
        crate::data_store::write(&path,&raw).unwrap();
        let loaded=read_journal(&path).unwrap().0;
        let mut writer=JournalWriter::from_loaded(path.clone(),loaded.clone()).unwrap();
        let measured=|writer:&JournalWriter|writer.sizes.framing+writer.sizes.row_bytes
            +writer.sizes.sections.values().map(|(_,count)|count.saturating_sub(1)).sum::<usize>();
        assert_eq!(measured(&writer),crate::data_store::read_bytes(&path,u64::MAX).unwrap().len());
        let mut pending=loaded.clone();
        pending.tasks[0].draft="汉字\"\\🙂".repeat(40);
        pending.tasks.remove(1);
        pending.deleted_sessions=vec!["已删除\"\\🙂".into()];
        pending.pinned_sessions.insert("置顶\"\\🙂".into());
        writer.commit(&mut pending).unwrap();
        assert_eq!(measured(&writer),crate::data_store::read_bytes(&path,u64::MAX).unwrap().len());
        let stored:serde_json::Value=crate::data_store::read(&path,u64::MAX).unwrap().unwrap();
        assert_eq!(stored["保留的\"头🙂"],raw["保留的\"头🙂"]);
        assert_eq!(stored["未知\"分区🙂"],raw["未知\"分区🙂"]);
        let revision=writer.saved.persisted_revision.get();
        let before=crate::data_store::read_bytes(&path,u64::MAX).unwrap();
        let mut pending=writer.saved.clone();
        pending.tasks[0].draft="x".repeat(JOURNAL_LIMIT);
        let retained=pending.tasks[0].draft.clone();
        assert!(writer.commit(&mut pending).unwrap_err().to_string().contains("容量"));
        assert_eq!(pending.tasks[0].draft,retained);
        assert_eq!(writer.saved.persisted_revision.get(),revision);
        assert_eq!(crate::data_store::read_bytes(&path,u64::MAX).unwrap(),before);
        assert_eq!(measured(&writer),before.len());
        pending.tasks[0].draft="较小的重试草稿".into();
        writer.commit(&mut pending).unwrap();
        assert_eq!(read_journal(&path).unwrap().0.tasks[0].draft,"较小的重试草稿");
        assert_eq!(measured(&writer),crate::data_store::read_bytes(&path,u64::MAX).unwrap().len());
    }
    #[test]
    fn journal_sql_migration_and_single_draft_keep_all_other_rows() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tasks.json");
        let mut original = journal_fixture(24);
        original.deleted_sessions = vec!["deleted-old".into()];
        original.pinned_sessions.insert("pinned-old".into());
        let legacy = serde_json::to_vec(&original).unwrap();
        std::fs::write(&path, &legacy).unwrap();
        let loaded = Journal::load(&path).unwrap();
        assert!(loaded.same_contents(&original));
        assert_eq!(std::fs::read(&path).unwrap(), legacy);
        let mut writer = JournalWriter::from_loaded(path.clone(), loaded.clone()).unwrap();
        let mut pending = loaded.clone();
        pending.tasks[7].draft.push('新');
        pending.tasks[7].turns[0].at_ms = Some(777); // formerly omitted by UI equality
        let expected = pending.clone();
        let db = crate::data_store::database_for(&path);
        let before = crate::data_store::with_connection(&db, |c| Ok(c.total_changes())).unwrap();
        assert_eq!(writer.commit(&mut pending).unwrap(), 1);
        let after = crate::data_store::with_connection(&db, |c| Ok(c.total_changes())).unwrap();
        assert_eq!(after - before, 2, "only target task plus snapshot revision");
        assert!(read_journal(&path).unwrap().0.same_contents(&expected));
        assert_eq!(std::fs::read(&path).unwrap(), legacy);
        let mut unchanged = expected.clone();
        assert_eq!(writer.commit(&mut unchanged).unwrap(), 0);
        crate::data_store::close_thread_connections();
        assert!(read_journal(&path).unwrap().0.same_contents(&expected));
    }
    #[test]
    fn journal_deletes_pins_and_conflicts_are_atomic() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tasks.json");
        let original = journal_fixture(8);
        original.save(&path).unwrap();
        let loaded = Journal::load(&path).unwrap();
        let mut first = JournalWriter::from_loaded(path.clone(), loaded.clone()).unwrap();
        let mut stale = JournalWriter::from_loaded(path.clone(), loaded.clone()).unwrap();
        let mut pending = loaded.clone();
        pending.tasks.remove(2);
        pending.tasks.swap(0, 4);
        pending.deleted_sessions.push("removed-session".into());
        pending.pinned_sessions.insert("pinned-session".into());
        let expected = pending.clone();
        first.commit(&mut pending).unwrap();
        assert!(read_journal(&path).unwrap().0.same_contents(&expected));
        let mut stale_draft = loaded;
        stale_draft.tasks[0].draft = "不能丢失的冲突草稿".into();
        let retained = stale_draft.clone();
        assert!(stale.commit(&mut stale_draft).is_err());
        assert!(stale_draft.same_contents(&retained));
        assert!(read_journal(&path).unwrap().0.same_contents(&expected));
        assert!(
            retained.save(&path).is_err(),
            "a stale direct save is rejected too"
        );
        assert!(
            Journal::default().save(&path).is_err(),
            "unbased empty defaults never overwrite records"
        );
    }
    #[test]
    fn journal_failed_commit_retains_draft_retries_and_corruption_stays_untouched() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tasks.json");
        journal_fixture(4).save(&path).unwrap();
        let loaded = Journal::load(&path).unwrap();
        let mut writer = JournalWriter::from_loaded(path.clone(), loaded.clone()).unwrap();
        let mut pending = loaded.clone();
        pending.tasks[0].draft = "重试草稿".into();
        pending.tasks[1].draft = "第二条变化".into();
        pending.deleted_sessions.push("tombstone".into());
        let expected = pending.clone();
        let db = crate::data_store::database_for(&path);
        crate::data_store::with_connection(&db,|c| { c.execute_batch("CREATE TEMP TRIGGER reject_journal BEFORE UPDATE ON records WHEN NEW.section='tasks' AND NEW.id='task-1' BEGIN SELECT RAISE(ABORT,'injected failure'); END;")?; Ok(()) }).unwrap();
        assert!(writer.commit(&mut pending).is_err());
        assert!(pending.same_contents(&expected));
        assert!(read_journal(&path).unwrap().0.same_contents(&loaded));
        crate::data_store::with_connection(&db, |c| {
            c.execute_batch("DROP TRIGGER reject_journal")?;
            Ok(())
        })
        .unwrap();
        writer.commit(&mut pending).unwrap();
        assert!(read_journal(&path).unwrap().0.same_contents(&expected));
        let broken = root.path().join("broken/tasks.json");
        std::fs::create_dir_all(broken.parent().unwrap()).unwrap();
        std::fs::write(&broken, b"corrupt original").unwrap();
        assert!(Journal::load(&broken).is_err());
        assert!(Journal::default().save(&broken).is_err());
        assert_eq!(std::fs::read(broken).unwrap(), b"corrupt original");
    }
    #[test]
    fn journal_worker_retries_pending_draft_without_another_ui_edit() {
        let root=tempfile::tempdir().unwrap();
        let path=root.path().join("tasks.json");
        journal_fixture(2).save(&path).unwrap();
        let initial=Journal::load(&path).unwrap();
        let mut pending=initial.clone();
        pending.tasks[0].draft="不需再次编辑即可重试的草稿".into();
        let expected=pending.clone();
        let db=crate::data_store::database_for(&path);
        crate::data_store::with_connection(&db,|c| { c.execute_batch("CREATE TRIGGER reject_worker BEFORE UPDATE ON records BEGIN SELECT RAISE(ABORT,'retry probe'); END;")?; Ok(()) }).unwrap();
        let (tx,rx)=std::sync::mpsc::channel();
        let (status_tx,status_rx)=std::sync::mpsc::channel();
        let worker_path=path.clone();
        let worker=std::thread::spawn(move||write_journal_updates(worker_path,initial,rx,status_tx));
        tx.send(pending).unwrap();
        assert!(status_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap().is_err());
        crate::data_store::with_connection(&db,|c| { c.execute_batch("DROP TRIGGER reject_worker")?; Ok(()) }).unwrap();
        assert!(status_rx.recv_timeout(std::time::Duration::from_secs(3)).unwrap().is_ok());
        drop(tx);
        worker.join().unwrap();
        assert!(read_journal(&path).unwrap().0.same_contents(&expected));
    }
    #[test]
    fn journal_worker_retains_conflicting_draft_in_explicit_recovery_copy() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("tasks.json");
        journal_fixture(2).save(&path).unwrap();
        let initial = Journal::load(&path).unwrap();
        let mut other = Journal::load(&path).unwrap();
        other.tasks[1].draft = "另一个写入".into();
        other.save(&path).unwrap();
        let mut pending = initial.clone();
        pending.tasks[0].draft = "关闭时仍未提交的草稿".into();
        let (tx, rx) = std::sync::mpsc::channel();
        let (status_tx, status_rx) = std::sync::mpsc::channel();
        tx.send(pending.clone()).unwrap();
        drop(tx);
        write_journal_updates(path.clone(), initial, rx, status_tx);
        assert!(status_rx.try_iter().any(|r| r.is_err()));
        assert!(read_journal(&path).unwrap().0.same_contents(&other));
        let recovery = std::fs::read_dir(root.path())
            .unwrap()
            .map(|e| e.unwrap().path())
            .find(|p| {
                p.file_name()
                    .unwrap()
                    .to_string_lossy()
                    .starts_with("tasks-unsaved-")
            })
            .unwrap();
        let saved: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&recovery).unwrap()).unwrap();
        assert_eq!(
            saved["journal"]["tasks"][0]["draft"],
            pending.tasks[0].draft
        );
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(recovery).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }
    #[test]
    fn old_turn_time_remains_unknown_and_new_turn_time_round_trips() {
        let old: Turn = serde_json::from_str(r#"{"user":true,"text":"昨天讨论的方案"}"#).unwrap();
        assert_eq!(old.at_ms, None);
        let new = Turn {
            at_ms: Some(1_793_232_000_000),
            user: true,
            text: "新的决定".into(),
        };
        let decoded: Turn = serde_json::from_str(&serde_json::to_string(&new).unwrap()).unwrap();
        assert_eq!(decoded.at_ms, new.at_ms);
        assert_eq!(decoded.text, new.text);
    }
    use crate::amoo_route::{route_intent, RouteCtx};
    fn auto(
        text: &str,
        files: &[&str],
        last: Option<Executor>,
        sticky: Option<Intent>,
    ) -> Vec<Executor> {
        let files = files.iter().map(PathBuf::from).collect::<Vec<_>>();
        route_intent(
            text,
            &files,
            &RouteCtx {
                forced: None,
                sticky,
                last,
            },
        )
        .steps
    }
    #[test]
    fn app_retry_cards_collapse_but_distinct_deliverables_survive() {
        let mut task = Task::new(None, None);
        for (name, id) in [
            ("first", Some("build-a-1")),
            ("report", None),
            ("second", Some("build-a-2")),
            ("other", Some("build-b-1")),
        ] {
            task.outputs.push(Output {
                path: PathBuf::from(name),
                name: name.into(),
                kind: String::new(),
                lab_id: id.map(str::to_owned),
            });
        }
        assert_eq!(task.current_output_indices(), vec![1, 2, 3]);
        assert_eq!(task.outputs.len(), 4);
    }
    #[test]
    fn auto_is_default_and_explicit_deliverables_route_cleanly() {
        assert_eq!(Intent::default(), Intent::Auto);
        assert_eq!(Task::new(None, None).intent, Intent::Auto);
        assert_eq!(Intent::Auto.label(), "自动");
        // Without a previous artifact, edits and questions stay chat.
        for text in ["改成蓝色", "这张图片是什么", "今天天气怎么样"] {
            assert_eq!(
                propose_followup(text, Intent::default(), true, None),
                vec![Executor::Phone],
                "{text}"
            );
        }
        // Follow-up edits continue the previous executor; questions do not.
        for last in [Executor::Files, Executor::App, Executor::Image] {
            assert_eq!(
                propose_followup("改成蓝色", Intent::default(), false, Some(last)),
                vec![last]
            );
            for q in ["这张图片是什么", "今天天气怎么样"] {
                assert_eq!(
                    propose_followup(q, Intent::default(), false, Some(last)),
                    vec![Executor::Phone],
                    "{q}"
                );
            }
        }
        assert_eq!(
            propose_followup("分析这个表格", Intent::default(), true, None),
            vec![Executor::Files]
        );
        assert_eq!(
            propose_followup("生成 PPT", Intent::default(), false, None),
            vec![Executor::Files]
        );
        assert_eq!(
            propose_followup("做个游戏", Intent::default(), false, None),
            vec![Executor::App]
        );
    }
    #[test]
    fn detect_explicit_intent_covers_games_office_apps_and_chat() {
        // Games -> Intent::App
        for p in [
            "帮我做个游戏",
            "写个贪吃蛇小游戏",
            "做一个 2048",
            "做一个太空射击游戏",
            "写个俄罗斯方块",
        ] {
            assert_eq!(detect_explicit_intent(p, false), Intent::App, "{p}");
        }
        // Build App -> Intent::App
        for p in [
            "做个应用：番茄钟",
            "做一个App记录习惯打卡",
            "生成应用：记账本",
            "做一个背屏控制器可交互网页",
            "做个网页",
            "做个计算器",
        ] {
            assert_eq!(detect_explicit_intent(p, false), Intent::App, "{p}");
        }
        // Office -> Intent::Files
        for p in [
            "生成一份季度汇报 PPT",
            "导出 Word 文档报告.docx",
            "生成 Excel 表格汇总.xlsx",
            "整理成文档并生成报告文件",
            "帮我做个PPT",
            "生成一份 Word 报告",
            "写一份周报",
            "季度汇报PPT",
        ] {
            assert_eq!(detect_explicit_intent(p, false), Intent::Files, "{p}");
        }
        assert_eq!(
            detect_explicit_intent("帮我汇总这份资料", true),
            Intent::Files
        );
        // Images
        for p in ["画一只猫", "设计一张新品海报", "生成一张图片：海边日落"] {
            assert_eq!(detect_explicit_intent(p, false), Intent::Image, "{p}");
        }
        // Screen -> Intent::Screen
        assert_eq!(
            detect_explicit_intent("设计一个背屏图案并发送到背屏", false),
            Intent::Screen
        );
        // Questions & explicit chat-only -> Intent::Chat
        for p in [
            "如何制作一个贪吃蛇小游戏？",
            "2048 怎么做？",
            "这张图片是什么",
            "阅读附件，只用文字回答，不生成文件",
            "今天上海会下雨吗？",
            "这份表格讲了什么",
            "俄罗斯方块",
        ] {
            assert_eq!(detect_explicit_intent(p, true), Intent::Chat, "{p}");
        }
    }
    #[test]
    fn polite_shells_still_generate() {
        assert_eq!(
            auto("能不能帮我做个PPT", &[], None, None),
            vec![Executor::Files]
        );
        assert_eq!(
            auto("帮我做个游戏可以吗", &[], None, None),
            vec![Executor::App]
        );
        assert_eq!(
            auto("可以帮我写一份周报吗？", &[], None, None),
            vec![Executor::Files]
        );
        assert_eq!(auto("麻烦画一只猫", &[], None, None), vec![Executor::Image]);
        assert_eq!(
            preview_route("能不能帮我做个PPT", false, None),
            Some((Intent::Files, "生成 PPT"))
        );
        assert_eq!(
            preview_route("帮我做个游戏可以吗", false, None),
            Some((Intent::App, "做小游戏"))
        );
    }
    #[test]
    fn knowledge_questions_stay_chat() {
        for p in [
            "如何制作一个贪吃蛇小游戏？",
            "2048 怎么做？",
            "这张图片是什么",
            "PPT 和 Keynote 有什么区别？",
            "你能做PPT吗",
        ] {
            assert_eq!(auto(p, &[], None, None), vec![Executor::Phone], "{p}");
            assert_eq!(preview_route(p, false, None), None, "{p}");
        }
        assert_eq!(
            auto("这张图片是什么", &["photo.png"], None, None),
            vec![Executor::Phone]
        );
    }
    #[test]
    fn preview_labels_are_short_and_plain() {
        let cases = [
            ("生成一份季度汇报 PPT", "生成 PPT"),
            ("生成一份 Word 报告", "生成 Word 文档"),
            ("做一个带公式的 Excel 表格", "生成 Excel 表格"),
            ("写个贪吃蛇小游戏", "做小游戏"),
            ("做个番茄钟应用", "做小应用"),
            ("画一只猫", "生成图片"),
            (
                "分析销售表格，生成汇报 PPT，再做一个手机看板",
                "先整理文件再做应用",
            ),
            ("设计一个背屏图案并发送到背屏", "背屏图案"),
        ];
        for (p, label) in cases {
            assert_eq!(
                preview_route(p, false, None).map(|v| v.1),
                Some(label),
                "{p}"
            );
        }
        assert_eq!(preview_route("今天聊聊天", false, None), None);
        assert_eq!(preview_route("", true, None), None);
    }
    #[test]
    fn sticky_route_continues_short_edits() {
        for sticky in [Intent::Files, Intent::App, Intent::Image] {
            let expected = match sticky {
                Intent::Files => Executor::Files,
                Intent::App => Executor::App,
                _ => Executor::Image,
            };
            for p in ["改成蓝色", "第三页加个图表", "标题再大一点"] {
                assert_eq!(
                    auto(p, &[], Some(Executor::Phone), Some(sticky)),
                    vec![expected],
                    "{p}"
                );
            }
            // Questions are not edits.
            assert_eq!(
                auto("第三页讲了什么？", &[], None, Some(sticky)),
                vec![Executor::Phone]
            );
            assert_eq!(auto("谢谢", &[], None, Some(sticky)), vec![Executor::Phone]);
        }
        assert_eq!(
            preview_route("改成蓝色", false, Some(Intent::App)),
            Some((Intent::App, "继续修改应用"))
        );
        // A new explicit request beats the sticky route.
        assert_eq!(
            auto("再做一个贪吃蛇游戏", &[], None, Some(Intent::Files)),
            vec![Executor::App]
        );
    }
    #[test]
    fn chat_only_overrides_and_clears_sticky() {
        let ctx = RouteCtx {
            forced: None,
            sticky: Some(Intent::Files),
            last: Some(Executor::Files),
        };
        for p in [
            "只聊天，改成蓝色是什么感觉",
            "不要生成文件，直接告诉我第三页怎么改",
            "现在继续普通对话，不再生成文件。刚才结果表中 B 项的金额是多少？",
            "帮我做个PPT的大纲，只用文字回答",
        ] {
            let d = route_intent(p, &[], &ctx);
            assert_eq!(d.steps, vec![Executor::Phone], "{p}");
            assert!(d.opt_out, "{p}");
            let mut task = Task::new(None, None);
            task.route = Some(Intent::Files);
            task.route_note = "生成 PPT".into();
            task.apply_route(&d);
            assert_eq!(task.route, None);
            assert!(task.route_note.is_empty());
        }
    }
    #[test]
    fn attachments_with_actions_become_files() {
        assert_eq!(
            auto("帮我整理一下", &["销售.xlsx"], None, None),
            vec![Executor::Files]
        );
        let d = route_intent(
            "汇总这几份数据",
            &[PathBuf::from("a.csv"), PathBuf::from("b.xlsx")],
            &RouteCtx::default(),
        );
        assert_eq!(d.label, "生成 Excel 表格");
        assert_eq!(
            auto("分析这份合同的风险", &["合同.docx"], None, None),
            vec![Executor::Files]
        );
        // Attachment + pure question stays chat; images alone stay chat.
        assert_eq!(
            auto("这份表格合计多少？", &["a.csv"], None, None),
            vec![Executor::Phone]
        );
        assert_eq!(
            auto("请查看这张照片", &["a.png"], None, None),
            vec![Executor::Phone]
        );
        // Office attachment + app request digests the files first.
        assert_eq!(
            auto("根据附件做一个手机看板", &["sales.xlsx"], None, None),
            vec![Executor::Files, Executor::App]
        );
    }
    #[test]
    fn reroute_updates_route_note_and_sticky() {
        let text = "帮我做个PPT";
        let mut task = Task::new(None, None);
        let first = route_intent(text, &[], &RouteCtx::default());
        task.apply_route(&first);
        assert_eq!(task.route_note, "生成 PPT");
        assert_eq!(task.route, Some(Intent::Files));
        // "改为制作应用"
        let app = route_intent(
            text,
            &[],
            &RouteCtx {
                forced: Some(Intent::App),
                ..Default::default()
            },
        );
        assert_eq!(app.steps, vec![Executor::App]);
        task.apply_route(&app);
        assert_eq!(task.route_note, "做小应用");
        assert_eq!(task.route, Some(Intent::App));
        // "改为普通聊天"
        let chat = route_intent(
            text,
            &[],
            &RouteCtx {
                forced: Some(Intent::Chat),
                ..Default::default()
            },
        );
        assert_eq!(chat.steps, vec![Executor::Phone]);
        task.apply_route(&chat);
        assert!(task.route_note.is_empty());
        assert_eq!(task.route, None);
        // Ordinary chat keeps the sticky route for later edits.
        task.route = Some(Intent::Files);
        task.apply_route(&route_intent("今天天气怎么样", &[], &RouteCtx::default()));
        assert_eq!(task.route, Some(Intent::Files));
        assert!(task.route_note.is_empty());
    }
    #[test]
    fn migrate_legacy_chat_default_to_auto_without_changing_draft_or_outputs() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("journal.json");
        let mut task = Task::new(None, None);
        task.intent = Intent::Chat;
        task.draft = "生成一个游戏".into();
        let mut forced = Task::new(None, None);
        forced.intent = Intent::Files;
        forced.draft = "整理表格".into();
        Journal {
            tasks: vec![task, forced],
            ..Default::default()
        }
        .save(&path)
        .unwrap();
        let journal = Journal::load(&path).unwrap();
        assert_eq!(journal.tasks[0].intent, Intent::Auto);
        assert_eq!(journal.tasks[0].draft, "生成一个游戏");
        assert_eq!(journal.tasks[1].intent, Intent::Files);
    }
    #[test]
    fn combined_request_has_ordered_real_executors() {
        assert_eq!(
            propose(
                "分析销售表格，生成汇报 PPT，再做一个手机看板",
                Intent::Auto,
                true
            ),
            vec![Executor::Files, Executor::App]
        );
    }
    #[test]
    fn ordinary_questions_do_not_start_local_harness() {
        for p in ["如何制作一个游戏？", "今天上海会下雨吗？", "这张图片是什么"]
        {
            assert_eq!(propose(p, Intent::Auto, true), vec![Executor::Phone]);
            assert_eq!(propose(p, Intent::Chat, true), vec![Executor::Phone]);
        }
    }
    #[test]
    fn explicit_choice_and_explicit_intent_work() {
        assert_eq!(
            propose("今天聊聊天", Intent::Auto, false),
            vec![Executor::Phone]
        );
        assert_eq!(
            propose("做个贪吃蛇小游戏", Intent::Auto, false),
            vec![Executor::App]
        );
        // Forced chat beats an explicit deliverable.
        assert_eq!(
            propose("做个贪吃蛇小游戏", Intent::Chat, false),
            vec![Executor::Phone]
        );
        assert_eq!(
            propose("修改这一项", Intent::App, false),
            vec![Executor::App]
        );
        assert_eq!(
            propose("随便写点", Intent::Files, false),
            vec![Executor::Files]
        );
    }
    #[test]
    fn history_keeps_unsent_text_and_attachment_drafts_but_not_empty_conversations() {
        let mut task = Task::new(None, None);
        assert!(!task.meaningful());
        task.draft = "  \n".into();
        assert!(!task.meaningful());
        task.draft = "尚未发送的草稿".into();
        assert!(task.meaningful());
        task.draft.clear();
        task.attachments.push(PathBuf::from("待发送.csv"));
        assert!(task.meaningful());
    }

    #[test]
    fn task_records_keep_drafts_outputs_and_do_not_resume_jobs() {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join("tasks.json");
        let mut t = Task::new(Some("phone-a".into()), Some("account-a".into()));
        t.draft = "中文草稿".into();
        t.attachments.push(PathBuf::from("照片.png"));
        t.steps.push(Step {
            executor: Executor::App,
            phase: Phase::Running,
            note: String::new(),
        });
        let journal = Journal {
            tasks: vec![t],
            ..Default::default()
        };
        journal.save(&path).unwrap();
        let saved = Journal::load(&path).unwrap();
        assert_eq!(saved.tasks[0].draft, "中文草稿");
        assert_eq!(saved.tasks[0].attachments, journal.tasks[0].attachments);
        assert_eq!(saved.tasks[0].steps[0].phase, Phase::Unconfirmed);
        assert!(!saved.tasks[0].visible(Some("phone-b"), Some("account-a")));
        assert!(!saved.tasks[0].visible(Some("phone-a"), Some("account-b")));
    }
    #[test]
    fn app_context_only_contains_this_tasks_text_outputs() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("report.md");
        std::fs::write(&p, "实际核对结果").unwrap();
        std::fs::write(root.path().join("private.txt"), "不应上传").unwrap();
        let mut t = Task::new(None, None);
        t.workspace = Some(root.path().into());
        t.add_output(Output {
            path: p,
            name: "report.md".into(),
            kind: "document".into(),
            lab_id: None,
        });
        let result = app_context(&t).unwrap();
        assert!(result.contains("实际核对结果"));
        assert!(!result.contains("不应上传"));
    }
    #[test]
    fn followup_edit_keeps_task_executor_but_questions_do_not() {
        assert_eq!(
            propose_followup("把按钮改成蓝色", Intent::Auto, false, Some(Executor::App)),
            vec![Executor::App]
        );
        assert_eq!(
            propose_followup(
                "增加一列累计金额",
                Intent::Auto,
                false,
                Some(Executor::Files)
            ),
            vec![Executor::Files]
        );
        assert_eq!(
            propose_followup(
                "这个应用如何修改？",
                Intent::Auto,
                false,
                Some(Executor::App)
            ),
            vec![Executor::Phone]
        );
        assert_eq!(
            propose_followup("改成蓝色", Intent::Auto, false, Some(Executor::Phone)),
            vec![Executor::Phone]
        );
    }
    #[test]
    fn accepted_materials_survive_draft_clearing() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("tasks.json");
        let mut t = Task::new(None, None);
        t.submitted_files.push("input.csv".into());
        t.attachments.clear();
        Journal {
            tasks: vec![t],
            ..Default::default()
        }
        .save(&p)
        .unwrap();
        assert_eq!(
            Journal::load(&p).unwrap().tasks[0].submitted_files,
            vec![PathBuf::from("input.csv")]
        );
    }
    #[test]
    fn context_rejects_files_outside_selected_task_directory() {
        let root = tempfile::tempdir().unwrap();
        let other = tempfile::tempdir().unwrap();
        let file = other.path().join("outside.md");
        std::fs::write(&file, "outside").unwrap();
        let mut t = Task::new(None, None);
        t.workspace = Some(root.path().into());
        t.add_output(Output {
            path: file,
            name: "outside.md".into(),
            kind: "text".into(),
            lab_id: None,
        });
        assert!(app_context(&t).is_err());
    }
    #[test]
    fn test_app_context_with_nonexistent_workspace_and_app_output_succeeds() {
        let root = tempfile::tempdir().unwrap();
        let missing_ws = root.path().join("nonexistent-task-dir");
        let mut t = Task::new(None, None);
        t.workspace = Some(missing_ws);
        t.add_output(Output {
            path: root.path().join("starter-neon-snake/round-1/index.html"),
            name: "霓虹贪吃蛇大作战".into(),
            kind: "应用 · 已检查".into(),
            lab_id: Some("starter-neon-snake".into()),
        });
        assert_eq!(app_context(&t).unwrap(), "");
    }

    #[test]
    fn journal_load_deduplicates_identical_draft_tasks_and_clears_completed_task_stale_drafts() {
        let root = tempfile::tempdir().unwrap();
        let p = root.path().join("tasks.json");

        let mut d1 = Task::new(Some("dev-1".into()), None);
        d1.id = "task-d1".into();
        d1.title = "请帮我汇总最近的快递通知并生成取件提醒".into();
        d1.draft = "请帮我汇总最近的快递通知并生成取件提醒".into();

        let mut d2 = Task::new(Some("dev-1".into()), None);
        d2.id = "task-d2".into();
        d2.title = "请帮我汇总最近的快递通知并生成取件提醒".into();
        d2.draft = "请帮我汇总最近的快递通知并生成取件提醒".into();

        let mut done_task = Task::new(Some("dev-1".into()), Some("s-weather".into()));
        done_task.id = "task-weather".into();
        done_task.title = "上海天气".into();
        done_task.draft = "上海天气".into();
        done_task.turns.push(Turn {
            user: true,
            text: "上海天气".into(),
            at_ms: Some(1),
        });
        done_task.turns.push(Turn {
            user: false,
            text: "上海今天多云，22°C".into(),
            at_ms: Some(2),
        });
        done_task.steps.push(Step {
            executor: Executor::Phone,
            phase: Phase::Done,
            note: "已完成".into(),
        });

        Journal {
            tasks: vec![d1, d2, done_task],
            ..Default::default()
        }
        .save(&p)
        .unwrap();

        let loaded = Journal::load(&p).unwrap();
        let courier_tasks: Vec<_> = loaded
            .tasks
            .iter()
            .filter(|t| t.title.contains("快递通知"))
            .collect();
        assert_eq!(courier_tasks.len(), 1);
        let weather = loaded
            .tasks
            .iter()
            .find(|t| t.id == "task-weather")
            .unwrap();
        assert!(
            weather.draft.is_empty(),
            "Completed task should not keep a stale draft matching its last user turn"
        );

        let raw_on_disk: Journal = crate::data_store::read(&p, 16 * 1024 * 1024)
            .unwrap()
            .unwrap();
        assert_eq!(
            raw_on_disk.tasks.len(),
            2,
            "Journal::load must persist deduplicated tasks back to disk"
        );

        // Merging an unsent draft task into an active Running/Ready task must not re-populate draft
        let mut stale_draft = Task::new(Some("dev-1".into()), None);
        stale_draft.id = "task-stale-draft".into();
        stale_draft.title = "上海天气".into();
        stale_draft.draft = "上海天气".into();

        let mut active_sending = Task::new(Some("dev-1".into()), None);
        active_sending.id = "task-active-sending".into();
        active_sending.title = "上海天气".into();
        active_sending.turns.push(Turn {
            at_ms: Some(10),
            user: true,
            text: "上海天气".into(),
        });
        active_sending.steps.push(Step {
            executor: Executor::Phone,
            phase: Phase::Running,
            note: String::new(),
        });

        let mut journal = Journal {
            tasks: vec![stale_draft, active_sending],
            deleted_sessions: vec![],
            pinned_sessions: Default::default(),
            ..Default::default()
        };
        journal.deduplicate_draft_tasks(Some("task-active-sending"));
        assert_eq!(journal.tasks.len(), 1);
        assert_eq!(journal.tasks[0].id, "task-active-sending");
        assert!(
            journal.tasks[0].draft.is_empty(),
            "Merging an old unsent draft into a running task must keep draft cleared"
        );
    }
}
