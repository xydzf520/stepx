//! Result-oriented Home views. Rendering reads memory only; all device, model,
//! filesystem and state transitions run on scoped, cancellable background jobs.
use super::*;
use super::{
    context_state::{row_between_expanded, ContextAction},
    icons::UiIcon,
    theme::*,
    widgets::{Btn, El},
};
use anyhow::Context;
use chrono::{Datelike, Local, TimeZone};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    Arc,
};
use stepx_companion::{
    action_ledger::{self, State as ActionState},
    collaboration_status as phone_gui,
    context_ledger::{self as ledger, Period},
    material_inbox as inbox, meeting_brief,
};

fn active_productivity_actions(snapshot: Option<&action_ledger::Snapshot>) -> Vec<&action_ledger::ActionRecord> {
    snapshot.into_iter().flat_map(|snapshot| &snapshot.actions)
        .filter(|action| matches!(action.state,
            ActionState::Accepted | ActionState::InProgress | ActionState::Deferred))
        .collect()
}

// Borrow status only: callers deciding whether to show a notice must not build
// and discard its complete UI tree. The renderer uses the same state snapshot.
pub(super) struct ProductivityHealthStatus<'a> {
    context_problem: Option<&'a str>,
    domain_errors: [Option<&'a str>; super::auto_sync::AutoSyncDomain::ALL.len()],
    missing_account: bool,
    blocked: bool,
}

#[derive(Debug, PartialEq, Eq)]
pub(super) enum ProductivityPreservationStatus<'a> {
    Error(&'a str),
    Failed(usize),
    LowStorage,
}

#[derive(Clone, Debug, PartialEq, Eq)]
struct Scope {
    device: String,
    account: Option<String>,
}
impl Scope {
    fn id(&self) -> String {
        ledger::scope_id(&self.device, self.account.as_deref())
    }
    fn materials(&self) -> Result<inbox::Scope> {
        Ok(inbox::Scope {
            device: self.device.clone(),
            account: self
                .account
                .clone()
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!("请先连接手机并确认 Amoo 账号，再读取手机资料"))?,
        })
    }
}

#[derive(Clone, Debug)]
pub(super) enum ProductivityAction {
    Tab(u8),
    Refresh,
    RetrySources,
    RefreshCalendar,
    ToggleDismissed,
    ListPage(usize, usize),
    Collect,
    Stop,
    MaterialProcess(String),
    MaterialIgnore(String),
    MaterialRestore(String),
    MaterialRead(String),
    MaterialDraft(String),
    MaterialPickLink(String),
    MaterialLink(String, String),
    ActionState {
        id: String,
        revision: u64,
        state: ActionState,
    },
    RecordProgress {
        id: String,
        revision: u64,
    },
    UndoAction {
        id: String,
        revision: u64,
    },
    ActionSource(String),
    OpenMatter(String),
    Progress(Period),
    FullPeriod(Period),
    MeetingCreate(String),
    MeetingPickLink(String),
    MeetingLink(String, String, bool),
    PhoneControl {
        task: String,
        op: String,
    },
    PhoneReview,
    ClosePreview,
    PreviewPage(usize),
    SendPreview,
}

struct Preview {
    title: String,
    text: String,
    path: Option<PathBuf>,
    page: usize,
    matters: Vec<String>,
}
enum Output {
    Refreshed {
        actions: Option<action_ledger::Snapshot>,
        materials: Option<inbox::Snapshot>,
        notes: Vec<String>,
        meetings: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
    },
    Actions(action_ledger::Snapshot),
    Materials(inbox::Snapshot),
    Document {
        title: String,
        text: String,
        path: Option<PathBuf>,
    },
    Generated {
        title: String,
        text: String,
        path: PathBuf,
        actions: action_ledger::Snapshot,
    },
    Brief(meeting_brief::Brief),
    Sent(String),
    Linked(std::collections::BTreeMap<String, std::collections::BTreeSet<String>>),
    Phone(phone_gui::PhoneGuiSnapshot),
}
struct Worker {
    generation: u64,
    receiver: mpsc::Receiver<Result<Output, String>>,
    cancel: Arc<AtomicBool>,
    cancellable: bool,
}
struct PhonePoll {
    generation: u64,
    receiver: mpsc::Receiver<(phone_gui::StatusCache, phone_gui::PhoneGuiSnapshot)>,
}

pub(super) struct ProductivityUi {
    scope: Option<Scope>,
    generation: u64,
    worker: Option<Worker>,
    phone_poll: Option<PhonePoll>,
    phone_cache: phone_gui::StatusCache,
    phone: Option<phone_gui::PhoneGuiSnapshot>,
    pub(super) tab: u8,
    actions: Option<action_ledger::Snapshot>,
    materials: Option<inbox::Snapshot>,
    preview: Option<Preview>,
    material_link: Option<String>,
    meeting_link: Option<String>,
    feedback: Option<(String, bool)>,
    last_refresh: Option<Instant>,
    last_phone_poll: Option<Instant>,
    source_revision: String,
    needs_refresh: bool,
    show_dismissed: bool,
    list_pages: [usize; 5],
    meetings: std::collections::BTreeMap<String, std::collections::BTreeSet<String>>,
}
impl Default for ProductivityUi {
    fn default() -> Self {
        Self {
            scope: None,
            generation: 0,
            worker: None,
            phone_poll: None,
            phone_cache: Default::default(),
            phone: None,
            tab: 0,
            actions: None,
            materials: None,
            preview: None,
            material_link: None,
            meeting_link: None,
            feedback: None,
            last_refresh: None,
            last_phone_poll: None,
            source_revision: String::new(),
            needs_refresh: true,
            show_dismissed: false,
            list_pages: [0; 5],
            meetings: Default::default(),
        }
    }
}
impl ProductivityUi {
    fn change_scope(&mut self, scope: Option<Scope>) -> bool {
        if self.scope == scope {
            return false;
        }
        if let Some(worker) = self.worker.as_ref() {
            worker.cancel.store(true, Ordering::Relaxed);
        }
        self.generation = self.generation.wrapping_add(1);
        self.scope = scope;
        self.actions = None;
        self.materials = None;
        self.preview = None;
        self.material_link = None;
        self.meeting_link = None;
        self.meetings.clear();
        self.list_pages = [0; 5];
        self.phone_poll = None;
        self.phone_cache.invalidate();
        self.phone = None;
        self.feedback = None;
        self.last_refresh = None;
        self.last_phone_poll = None;
        self.needs_refresh = true;
        true
    }
    fn start(
        &mut self,
        message: &str,
        job: impl FnOnce(Arc<AtomicBool>) -> Result<Output> + Send + 'static,
    ) {
        if self.worker.is_some() {
            self.feedback = Some(("正在处理上一项操作，请稍候或停止后再试".into(), true));
            return;
        }
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = cancel.clone();
        self.feedback = Some((message.into(), false));
        std::thread::spawn(move || {
            let result = if worker_cancel.load(Ordering::Relaxed) {
                Err(anyhow::anyhow!("操作已停止"))
            } else {
                job(worker_cancel)
            };
            let _ = tx.send(result.map_err(|e| format!("{e:#}")));
        });
        self.worker = Some(Worker {
            generation: self.generation,
            receiver,
            cancel,
            cancellable: false,
        });
    }
    fn start_cancellable(
        &mut self,
        message: &str,
        job: impl FnOnce(Arc<AtomicBool>) -> Result<Output> + Send + 'static,
    ) {
        let vacant = self.worker.is_none();
        self.start(message, job);
        if vacant {
            if let Some(worker) = self.worker.as_mut() {
                worker.cancellable = true;
            }
        }
    }
}

fn context_root() -> PathBuf {
    stepx_companion::resources::user_state().join("stepx-companion/context")
}
fn short(text: &str, limit: usize) -> String {
    let value: String = text.chars().take(limit).collect();
    if text.chars().count() > limit {
        format!("{value}…")
    } else {
        value
    }
}
fn progress_bounds(period: Period, now: chrono::DateTime<Local>) -> (i64, i64) {
    let date = now.date_naive();
    let start = match period {
        Period::Day => date,
        Period::Week => date - chrono::Duration::days(date.weekday().num_days_from_monday() as i64),
        Period::Month => date.with_day(1).unwrap_or(date),
    };
    let end = match period {
        Period::Day => start + chrono::Duration::days(1),
        Period::Week => start + chrono::Duration::days(7),
        Period::Month => {
            if start.month() == 12 {
                chrono::NaiveDate::from_ymd_opt(start.year() + 1, 1, 1).unwrap()
            } else {
                chrono::NaiveDate::from_ymd_opt(start.year(), start.month() + 1, 1).unwrap()
            }
        }
    };
    let midnight = |d: chrono::NaiveDate| {
        Local
            .from_local_datetime(&d.and_hms_opt(0, 0, 0).unwrap())
            .earliest()
            .map(|t| t.timestamp_millis())
            .unwrap_or(now.timestamp_millis())
    };
    (midnight(start), midnight(end))
}
fn save_progress(scope: &Scope, period: Period) -> Result<(String, String, PathBuf)> {
    use std::io::Write;
    let now = Local::now();
    let (begin, end) = progress_bounds(period, now);
    let progress = action_ledger::summarize_period(
        &context_root(),
        &scope.device,
        scope.account.as_deref(),
        begin,
        end,
    )?;
    let title = format!("{} · 行动进展 · {}", period.label(), now.format("%Y-%m-%d"));
    let text=format!("# {title}\n\n{}\n\n此页只统计你已确认的行动状态；完整讨论、成果与来源覆盖见对应周期回顾。\n",progress.markdown);
    let dir = context_root().join(scope.id()).join("action-progress");
    std::fs::create_dir_all(&dir)?;
    anyhow::ensure!(
        !std::fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "进展目录不能是链接"
    );
    let hash = stepx_companion::device::sha256_reader(text.as_bytes())?;
    let path = dir.join(format!(
        "{}-行动进展-{}-{}.md",
        period.label(),
        now.format("%Y%m%d"),
        &hash[..12]
    ));
    if let Ok(meta) = std::fs::symlink_metadata(&path) {
        anyhow::ensure!(
            meta.is_file() && !meta.file_type().is_symlink(),
            "进展文件不可覆盖"
        );
    }
    let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
    temp.write_all(text.as_bytes())?;
    temp.as_file().sync_all()?;
    temp.persist(&path)?;
    Ok((title, text, path))
}
fn save_material_preview(scope: &Scope, title: &str, text: &str) -> Result<PathBuf> {
    use std::io::Write;
    let dir = context_root().join(scope.id()).join("material-previews");
    std::fs::create_dir_all(&dir)?;
    anyhow::ensure!(
        !std::fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "资料预览目录不可为链接"
    );
    let hash = stepx_companion::device::sha256_reader(text.as_bytes())?;
    let title: String = title
        .chars()
        .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
        .take(28)
        .collect();
    let path = dir.join(format!("资料正文-{title}-{}.md", &hash[..12]));
    if path.exists() {
        anyhow::ensure!(std::fs::read(&path)? == text.as_bytes(), "资料预览发生变化");
    } else {
        let mut temp = tempfile::NamedTempFile::new_in(&dir)?;
        temp.write_all(text.as_bytes())?;
        temp.as_file().sync_all()?;
        temp.persist(&path)?;
    }
    Ok(path)
}
fn send_document(
    device: &Device,
    scope: &Scope,
    path: &std::path::Path,
    expected_text: &str,
    cancel: &AtomicBool,
) -> Result<String> {
    let adb = Adb::discover()?;
    anyhow::ensure!(
        device.serial == scope.device
            && scope.account.as_ref().is_some_and(|s| !s.trim().is_empty()),
        "手机或账号尚未确认，文档保留在电脑"
    );
    anyhow::ensure!(
        stepx_companion::amoo_usage::account_scope_fresh(&adb, device)? == scope.account,
        "手机账号已变化，未回传文档"
    );
    let scoped_root = context_root().join(scope.id()).canonicalize()?;
    let canonical = path.canonicalize()?;
    anyhow::ensure!(
        canonical.starts_with(&scoped_root),
        "文档不属于当前设备与账号"
    );
    let meta = std::fs::symlink_metadata(path)?;
    anyhow::ensure!(
        meta.is_file() && !meta.file_type().is_symlink() && meta.len() <= 4 * 1024 * 1024,
        "文档不可回传或超过4MB"
    );
    anyhow::ensure!(
        std::fs::read(path)? == expected_text.as_bytes(),
        "文档已变化，重新打开后再回传"
    );
    let hash = stepx_companion::device::sha256_reader(expected_text.as_bytes())?;
    let desktop = stepx_companion::companion_link::desktop_id()?;
    let key = stepx_companion::device::sha256_reader(
        serde_json::to_vec(&(&scope.id(), &desktop, &hash, path.file_name()))?.as_slice(),
    )?;
    let id = format!(
        "{}-{}-{}-{}-{}",
        &key[..8],
        &key[8..12],
        &key[12..16],
        &key[16..20],
        &key[20..32]
    );
    stepx_companion::companion_link::send_staged_guarded(
        &adb,
        device,
        path,
        &id,
        &hash,
        meta.len(),
        cancel,
        scope.account.as_deref(),
    )?;
    anyhow::ensure!(
        stepx_companion::amoo_usage::account_scope_fresh(&adb, device)? == scope.account,
        "手机账号变化，交付已发出，接收范围需核对"
    );
    Ok(format!(
        "手机已接收并校验：{}",
        path.file_name().unwrap_or_default().to_string_lossy()
    ))
}

impl Companion {
    #[cfg(feature = "ui-capture")]
    pub(super) fn productivity_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({
            "tab": self.productivity.tab, "busy": self.productivity.worker.is_some(),
            "scope": self.productivity.scope.as_ref().map(Scope::id),
            "feedback": self.productivity.feedback,
            "preview": self.productivity.preview.as_ref().map(|p| serde_json::json!({"title":p.title,"chars":p.text.chars().count(),"page":p.page,"path":p.path})),
            "actions": self.productivity.actions.as_ref().map(|s| serde_json::json!({"revision":s.revision,"progress_revision":s.progress_revision,"items":s.actions.iter().map(|a|serde_json::json!({"id":a.id,"state":a.state,"needs_review":a.needs_review})).collect::<Vec<_>>()})),
            "materials": self.productivity.materials.as_ref().map(|s| s.items.iter().map(|m|serde_json::json!({"id":m.id,"kind":m.kind,"state":m.state,"bytes":m.bytes,"linked":m.linked_item.is_some(),"error":m.error})).collect::<Vec<_>>()),
            "phone": self.productivity.phone.as_ref().map(|p|serde_json::json!({"phase":p.phase,"verified":p.verified,"stale":p.stale,"error":p.error})),
        })
    }
    pub(super) fn productivity_is_busy(&self) -> bool {
        self.productivity.worker.is_some()
    }
    pub(super) fn productivity_preview_is_open(&self) -> bool {
        self.productivity.preview.is_some()
    }
    pub(super) fn invalidate_productivity_for_clear(&mut self) {
        if let Some(worker) = &self.productivity.worker {
            worker.cancel.store(true, Ordering::Relaxed);
        }
        self.productivity.generation = self.productivity.generation.wrapping_add(1);
        self.productivity.preview = None;
        self.productivity.actions = None;
        self.productivity.materials = None;
        self.productivity.material_link = None;
        self.productivity.meeting_link = None;
        self.productivity.meetings.clear();
        self.productivity.phone_poll = None;
        self.productivity.needs_refresh = true;
        self.productivity.source_revision.clear();
        self.productivity.last_refresh = None;
        self.fields[F_PRODUCTIVITY_NOTE].clear();
        if self.focus == Some(F_PRODUCTIVITY_NOTE) {
            self.focus = None;
        }
    }
    fn productivity_scope_now(&self) -> Option<Scope> {
        if self.context.productivity_clearing() {
            return None;
        }
        if let Some(device) = &self.selected {
            return Some(Scope {
                device: device.serial.clone(),
                account: self.amoo_ui.account_scope.clone(),
            });
        }
        self.context
            .productivity_scope()
            .map(|(device, account)| Scope { device, account })
    }
    pub(super) fn tick_productivity(&mut self, ctx: &mut ViewContext<Self>) {
        let clearing = self.context.productivity_clearing();
        let scope = if clearing {
            self.productivity.scope.clone()
        } else {
            self.productivity_scope_now()
        };
        let mut changed = if clearing {
            false
        } else {
            self.productivity.change_scope(scope.clone())
        };
        if changed {
            self.fields[F_PRODUCTIVITY_NOTE].clear();
            if self.focus == Some(F_PRODUCTIVITY_NOTE) {
                self.focus = None;
                self.sync_ime(ctx);
            }
        }
        let completed =
            self.productivity
                .worker
                .as_ref()
                .and_then(|worker| match worker.receiver.try_recv() {
                    Ok(result) => Some((
                        worker.generation,
                        worker.cancel.load(Ordering::Relaxed),
                        result,
                    )),
                    Err(mpsc::TryRecvError::Disconnected) => Some((
                        worker.generation,
                        worker.cancel.load(Ordering::Relaxed),
                        Err("操作中断；已有记录保留，请重试".into()),
                    )),
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some((generation, cancelled, result)) = completed {
            self.productivity.worker = None;
            changed = true;
            if generation == self.productivity.generation {
                if cancelled
                    && matches!(
                        &result,
                        Ok(Output::Generated { .. } | Output::Document { .. } | Output::Brief(_))
                    )
                {
                    self.productivity.feedback =
                        Some(("操作已停止，未继续生成或打开内容".into(), false));
                } else {
                    match result {
                        Ok(Output::Refreshed {
                            actions,
                            materials,
                            notes,
                            meetings,
                        }) => {
                            self.productivity.meetings = meetings;
                            self.productivity.actions = actions;
                            self.productivity.materials = materials;
                            if !notes.is_empty() {
                                self.productivity.feedback = Some((notes.join("；"), true));
                            } else if self
                                .productivity
                                .feedback
                                .as_ref()
                                .is_some_and(|(s, _)| s == "正在读取已保存的资料与行动…")
                            {
                                self.productivity.feedback = None;
                            }
                        }
                        Ok(Output::Actions(actions)) => {
                            self.productivity.actions = Some(actions);
                            self.productivity.feedback = Some((
                                "行动记录已保存，可撤销；周期回顾会按真实状态更新".into(),
                                false,
                            ));
                            self.context.needs_load = true;
                            self.fields[F_PRODUCTIVITY_NOTE].clear();
                        }
                        Ok(Output::Materials(materials)) => {
                            self.productivity.materials = Some(materials);
                            self.productivity.feedback = Some((
                                "资料状态已保存；可查看正文、关联事项并生成文件".into(),
                                false,
                            ));
                            self.context.needs_load = true;
                        }
                        Ok(Output::Document { title, text, path }) => {
                            self.scroll = Default::default();
                            self.productivity.preview = Some(Preview {
                                title,
                                text,
                                path,
                                page: 0,
                                matters: vec![],
                            });
                            self.productivity.feedback = None;
                        }
                        Ok(Output::Brief(brief)) => {
                            self.scroll = Default::default();
                            self.productivity.preview = Some(Preview {
                                title: format!("会前准备包 · {}", brief.title),
                                text: brief.markdown,
                                path: Some(brief.path),
                                page: 0,
                                matters: vec![],
                            });
                            self.productivity.feedback = None;
                        }
                        Ok(Output::Generated {
                            title,
                            text,
                            path,
                            actions,
                        }) => {
                            self.scroll = Default::default();
                            self.productivity.actions = Some(actions);
                            self.productivity.preview = Some(Preview {
                                title,
                                text,
                                path: Some(path),
                                page: 0,
                                matters: vec![],
                            });
                            self.productivity.feedback = Some((
                                "方案与待办已保存；行动仍是待核对候选，可在行动跟踪中采纳。".into(),
                                false,
                            ));
                            self.context.needs_load = true;
                        }
                        Ok(Output::Sent(message)) => {
                            self.productivity.feedback = Some((message, false))
                        }
                        Ok(Output::Linked(meetings)) => {
                            self.productivity.meetings = meetings;
                            self.productivity.feedback =
                                Some(("已保存会议关联，下次准备包将使用所选资料".into(), false));
                            self.productivity.meeting_link = None;
                            self.context.needs_load = true;
                        }
                        Ok(Output::Phone(phone)) => {
                            let read_failed = phone.error.is_some();
                            self.productivity.phone = Some(phone);
                            self.productivity.phone_cache.invalidate();
                            self.productivity.phone_poll = None;
                            self.productivity.last_phone_poll = None;
                            self.productivity.feedback = Some((
                                if read_failed {
                                    "手机操作已提交，当前状态尚未更新，请刷新核对"
                                } else {
                                    "手机已返回操作后的状态"
                                }
                                .into(),
                                read_failed,
                            ));
                        }
                        Err(error) => {
                            self.productivity.feedback = Some((error, true));
                            self.productivity.needs_refresh = true;
                        }
                    }
                }
                changed = true;
            }
        }
        let phone_result =
            self.productivity
                .phone_poll
                .as_ref()
                .and_then(|poll| match poll.receiver.try_recv() {
                    Ok(value) => Some((poll.generation, Some(value))),
                    Err(mpsc::TryRecvError::Disconnected) => Some((poll.generation, None)),
                    Err(mpsc::TryRecvError::Empty) => None,
                });
        if let Some((generation, value)) = phone_result {
            self.productivity.phone_poll = None;
            if generation == self.productivity.generation {
                if let Some((cache, snapshot)) = value {
                    self.productivity.phone_cache = cache;
                    self.productivity.phone = Some(snapshot);
                    changed = true;
                }
            }
        }
        if !self.demo_mode && !cfg!(test) && !clearing {
            if let Some(scope) = scope {
                let source_revision: String = self
                    .context
                    .overview
                    .items
                    .iter()
                    .map(|i| format!("{}:{};", i.id, i.input_revision))
                    .collect();
                let backfill = self.productivity.needs_refresh
                    || self.productivity.source_revision != source_revision;
                let refresh_interval =
                    Duration::from_secs(if self.page == Page::Home { 30 } else { 60 });
                let due = self.productivity.needs_refresh
                    || self
                        .productivity
                        .last_refresh
                        .is_none_or(|at| at.elapsed() > refresh_interval)
                    || self.productivity.source_revision != source_revision;
                if due && self.productivity.worker.is_none() {
                    self.productivity.needs_refresh = false;
                    self.productivity.last_refresh = Some(Instant::now());
                    self.productivity.source_revision = source_revision;
                    let previous_feedback = self.productivity.feedback.clone();
                    self.productivity.start(
                        "正在读取已保存的资料与行动…",
                        move |_| {
                            let root = context_root();
                            let mut notes = vec![];
                            let actions = match if backfill {
                                stepx_companion::context_pipeline::sync_action_candidates(
                                    &root,
                                    &scope.device,
                                    scope.account.as_deref(),
                                )
                            } else {
                                action_ledger::load(&root, &scope.device, scope.account.as_deref())
                            } {
                                Ok(v) => Some(v),
                                Err(error) => {
                                    notes.push(format!("候选行动未更新：{error:#}"));
                                    match action_ledger::load(
                                        &root,
                                        &scope.device,
                                        scope.account.as_deref(),
                                    ) {
                                        Ok(v) => Some(v),
                                        Err(error) => {
                                            notes.push(format!("行动记录不可读取：{error:#}"));
                                            None
                                        }
                                    }
                                }
                            };
                            let materials = if let Ok(material_scope) = scope.materials() {
                                match inbox::snapshot(&material_scope) {
                                    Ok(v) => Some(v),
                                    Err(error) => {
                                        notes.push(format!("资料收件箱不可读取：{error:#}"));
                                        None
                                    }
                                }
                            } else {
                                None
                            };
                            let meetings = meeting_brief::manual_links(
                                &root,
                                &scope.device,
                                scope.account.as_deref(),
                            )?;
                            Ok(Output::Refreshed {
                                actions,
                                materials,
                                notes,
                                meetings,
                            })
                        },
                    );
                    if previous_feedback.is_some() {
                        self.productivity.feedback = previous_feedback;
                    }
                    changed = true;
                }
            }
            let interval = if self
                .productivity
                .phone
                .as_ref()
                .is_some_and(|p| p.phase.active())
            {
                5
            } else if self.page == Page::Home {
                30
            } else {
                60
            };
            if self.productivity.phone_poll.is_none()
                && self
                    .productivity
                    .last_phone_poll
                    .is_none_or(|at| at.elapsed() > Duration::from_secs(interval))
            {
                if let Some(device) = self.selected.clone() {
                    let mut cache = std::mem::take(&mut self.productivity.phone_cache);
                    let account = self.amoo_ui.account_scope.clone();
                    let (tx, receiver) = mpsc::channel();
                    let generation = self.productivity.generation;
                    self.productivity.last_phone_poll = Some(Instant::now());
                    std::thread::spawn(move || {
                        if let Ok(adb) = Adb::discover() {
                            let snapshot = cache.refresh(&adb, &device, account.as_deref());
                            let _ = tx.send((cache, snapshot));
                        }
                    });
                    self.productivity.phone_poll = Some(PhonePoll {
                        generation,
                        receiver,
                    });
                }
            }
        }
        if changed {
            ctx.notify();
        }
    }

    pub(super) fn handle_productivity_action(
        &mut self,
        action: &ProductivityAction,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.context.productivity_clearing() {
            self.productivity.feedback = Some((
                "正在停止当前处理，随后清理本地整理记录；完成后可重新选择资料。".into(),
                false,
            ));
            ctx.notify();
            return;
        }
        match action {
            ProductivityAction::Tab(tab) => {
                self.productivity.tab = (*tab).min(3);
                if self.focus == Some(F_PRODUCTIVITY_NOTE) {
                    self.focus = None;
                    self.sync_ime(ctx);
                }
                self.productivity.preview = None;
                self.scroll = Default::default();
                ctx.notify();
                return;
            }
            ProductivityAction::ClosePreview => {
                self.productivity.preview = None;
                self.scroll = Default::default();
                ctx.notify();
                return;
            }
            ProductivityAction::PreviewPage(page) => {
                if let Some(preview) = &mut self.productivity.preview {
                    preview.page = *page;
                }
                self.scroll = Default::default();
                ctx.notify();
                return;
            }
            ProductivityAction::Stop => {
                if let Some(worker) = &self.productivity.worker.as_ref().filter(|w| w.cancellable) {
                    worker.cancel.store(true, Ordering::Relaxed);
                    self.productivity.feedback =
                        Some(("已请求停止；等待处理器退出，已保存内容保留".into(), false));
                }
                ctx.notify();
                return;
            }
            ProductivityAction::FullPeriod(period) => {
                self.productivity.preview = None;
                self.scroll = Default::default();
                self.handle_context_action(&ContextAction::Period(*period));
                ctx.notify();
                return;
            }
            ProductivityAction::OpenMatter(id) => {
                if self.context.overview.items.iter().any(|i| {
                    &i.id == id
                        && self
                            .productivity
                            .scope
                            .as_ref()
                            .is_some_and(|s| s.id() == i.scope_id)
                }) {
                    self.productivity.preview = None;
                    self.scroll = Default::default();
                    self.handle_context_action(&ContextAction::ItemDetails(id.clone()));
                }
                ctx.notify();
                return;
            }
            ProductivityAction::ListPage(kind, page) => {
                if let Some(current) = self.productivity.list_pages.get_mut(*kind) {
                    *current = (*page).min(10_000);
                }
                ctx.notify();
                return;
            }
            ProductivityAction::ToggleDismissed => {
                self.productivity.list_pages[1] = 0;
                self.productivity.show_dismissed = !self.productivity.show_dismissed;
                ctx.notify();
                return;
            }
            ProductivityAction::RefreshCalendar => {
                self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
                ctx.notify();
                return;
            }
            ProductivityAction::RetrySources => {
                self.productivity.needs_refresh = true;
                self.productivity.last_phone_poll = None;
                self.apply_home_action(&Action::HomeRefreshAll, Some(ctx));
                return;
            }
            ProductivityAction::Refresh => {
                self.productivity.feedback = None;
                self.productivity.needs_refresh = true;
                self.productivity.last_phone_poll = None;
                ctx.notify();
                return;
            }
            ProductivityAction::MaterialPickLink(id) => {
                self.productivity.material_link =
                    if self.productivity.material_link.as_ref() == Some(id) {
                        None
                    } else {
                        Some(id.clone())
                    };
                ctx.notify();
                return;
            }
            ProductivityAction::MeetingPickLink(id) => {
                self.productivity.meeting_link =
                    if self.productivity.meeting_link.as_ref() == Some(id) {
                        None
                    } else {
                        Some(id.clone())
                    };
                ctx.notify();
                return;
            }
            ProductivityAction::PhoneReview => {
                self.page = Page::Tools;
                self.tool_page = Some(ToolPage::Neo);
                self.auto_load_for_page();
                ctx.notify();
                return;
            }
            _ => {}
        }
        let Some(scope) = self.productivity.scope.clone() else {
            self.productivity.feedback = Some(("请先选择设备或读取已有资料".into(), true));
            ctx.notify();
            return;
        };
        let device = self.selected.clone().filter(|d| d.serial == scope.device);
        match action.clone() {
            ProductivityAction::Collect => {
                let Ok(material_scope) = scope.materials() else {
                    self.productivity.feedback =
                        Some(("请先确认手机账号，再读取资料".into(), true));
                    return;
                };
                let Some(device) = device else {
                    self.productivity.feedback =
                        Some(("连接手机后可读取已选资料；已有资料仍可查看".into(), true));
                    return;
                };
                self.productivity
                    .start("正在接收手机已选资料清单…", move |_| {
                        Ok(Output::Materials(inbox::collect(
                            &Adb::discover()?,
                            &device,
                            &material_scope,
                        )?))
                    });
            }
            ProductivityAction::MaterialProcess(id) => {
                let Ok(material_scope) = scope.materials() else {
                    return;
                };
                let Some(device) = device else {
                    self.productivity.feedback = Some(("手机离线，未开始读取原件".into(), true));
                    return;
                };
                self.productivity.start_cancellable(
                    "正在取回并解析选中的资料；图片和录音可能需要更长时间…",
                    move |cancel| {
                        Ok(Output::Materials(inbox::process(
                            &Adb::discover()?,
                            &device,
                            &material_scope,
                            &id,
                            &cancel,
                        )?))
                    },
                );
            }
            ProductivityAction::MaterialIgnore(id) | ProductivityAction::MaterialRestore(id) => {
                let Ok(material_scope) = scope.materials() else {
                    return;
                };
                let ignored = matches!(action, ProductivityAction::MaterialIgnore(_));
                self.productivity.start("正在保存资料状态…", move |_| {
                    Ok(Output::Materials(if ignored {
                        inbox::ignore(&material_scope, &id)?
                    } else {
                        inbox::retry(&material_scope, &id)?
                    }))
                });
            }
            ProductivityAction::MaterialLink(id, matter) => {
                let Ok(material_scope) = scope.materials() else {
                    return;
                };
                let valid = self
                    .context
                    .overview
                    .items
                    .iter()
                    .any(|i| i.id == matter && i.scope_id == scope.id());
                if !valid {
                    self.productivity.feedback = Some(("事项已变化，请刷新后选择".into(), true));
                    return;
                }
                self.productivity.material_link = None;
                self.productivity.start("正在关联资料与事项…", move |_| {
                    Ok(Output::Materials(inbox::link(
                        &material_scope,
                        &id,
                        Some(&matter),
                    )?))
                });
            }
            ProductivityAction::MaterialRead(id) | ProductivityAction::MaterialDraft(id) => {
                let Ok(material_scope) = scope.materials() else {
                    return;
                };
                let draft = matches!(action, ProductivityAction::MaterialDraft(_));
                let item = self
                    .productivity
                    .materials
                    .as_ref()
                    .and_then(|s| s.items.iter().find(|i| i.id == id))
                    .cloned();
                let Some(item) = item else {
                    return;
                };
                if draft && self.context.productivity_generation_running() {
                    self.productivity.feedback = Some(("另一份讨论成果正在生成或排队；完成后可生成这份资料，也可在讨论与成果中停止当前生成。".into(), false));
                    return;
                }
                if draft && device.is_none() {
                    self.productivity.feedback = Some((
                        "生成前需要连接原手机并确认账号；已解析正文仍可离线阅读。".into(),
                        true,
                    ));
                    return;
                }
                self.productivity.start_cancellable(
                    if draft {
                        "正在结合关联的讨论与资料生成方案、待办…"
                    } else {
                        "正在校验并读取已解析的正文…"
                    },
                    move |cancel| {
                        anyhow::ensure!(!cancel.load(Ordering::Relaxed), "已停止读取资料");
                        let text = inbox::read_content(&material_scope, &id)?;
                        anyhow::ensure!(!cancel.load(Ordering::Relaxed), "已停止读取资料");
                        Ok(if draft {
                            let device = device.context("手机未连接")?;
                            let adb = Adb::discover()?;
                            // A user may have cleared prepared context while keeping
                            // the inbox originals. Explicit generation reindexes only
                            // this selected, receipt-verified material.
                            inbox::process(&adb, &device, &material_scope, &id, &cancel)?;
                            let root = context_root();
                            let result = stepx_companion::context_pipeline::run_material(
                                &root,
                                &adb,
                                &device,
                                scope.account.as_deref(),
                                &id,
                                &cancel,
                                false,
                            )?;
                            anyhow::ensure!(
                                !cancel.load(Ordering::Relaxed),
                                "生成已停止，已保存的成果仍可在讨论与成果中查看"
                            );
                            let title = format!("方案与待办 · {}", item.title);
                            let mut text = format!("# {title}\n\n");
                            let canonical_scope = root.join(scope.id()).canonicalize()?;
                            for file in result
                                .bundle
                                .files
                                .iter()
                                .filter(|f| f.path.extension().is_some_and(|e| e == "md"))
                            {
                                anyhow::ensure!(
                                    file.path.canonicalize()?.starts_with(&canonical_scope),
                                    "成果不属于当前来源范围"
                                );
                                anyhow::ensure!(
                                    stepx_companion::device::sha256_file(&file.path)?
                                        == file.sha256,
                                    "成果正文已变化"
                                );
                                text.push_str(&std::fs::read_to_string(&file.path)?);
                                text.push_str("\n\n---\n\n");
                            }
                            anyhow::ensure!(
                                text.len() <= 256 * 1024 && !result.bundle.files.is_empty(),
                                "成果为空或过大"
                            );
                            let path = save_material_preview(&scope, &title, &text)?;
                            let actions = action_ledger::load(
                                &root,
                                &scope.device,
                                scope.account.as_deref(),
                            )?;
                            Output::Generated {
                                title,
                                text,
                                path,
                                actions,
                            }
                        } else {
                            Output::Document {
                                path: Some(save_material_preview(&scope, &item.title, &text)?),
                                title: item.title,
                                text,
                            }
                        })
                    },
                );
            }
            ProductivityAction::ActionState {
                id,
                revision,
                state,
            } => {
                let note = self.fields[F_PRODUCTIVITY_NOTE].text.trim().to_owned();
                if state == ActionState::Deferred && note.is_empty() {
                    self.productivity.feedback =
                        Some(("请先填写延期原因或新期限，保留原约定以便复盘".into(), true));
                    self.focus_field(F_PRODUCTIVITY_NOTE, ctx);
                    return;
                }
                self.productivity.start("正在保存行动状态…", move |_| {
                    Ok(Output::Actions(action_ledger::set_state(
                        &context_root(),
                        &scope.device,
                        scope.account.as_deref(),
                        &id,
                        revision,
                        state,
                        &note,
                        Local::now().timestamp_millis(),
                    )?))
                });
            }
            ProductivityAction::RecordProgress { id, revision } => {
                let note = self.fields[F_PRODUCTIVITY_NOTE].text.trim().to_owned();
                if note.is_empty() {
                    self.productivity.feedback = Some(("先填写进展或阻塞原因，再点击对应行动的“记录进展”".into(), true));
                    self.focus_field(F_PRODUCTIVITY_NOTE, ctx);
                    return;
                }
                self.productivity.start("正在保存这项行动的进展…", move |_| {
                    Ok(Output::Actions(action_ledger::record_progress(
                        &context_root(), &scope.device, scope.account.as_deref(), &id,
                        revision, &note, Local::now().timestamp_millis(),
                    )?))
                });
            }
            ProductivityAction::UndoAction { id, revision } => {
                self.productivity
                    .start("正在撤销最近一次行动记录…", move |_| {
                        Ok(Output::Actions(action_ledger::undo_state(
                            &context_root(),
                            &scope.device,
                            scope.account.as_deref(),
                            &id,
                            revision,
                            Local::now().timestamp_millis(),
                        )?))
                    })
            }
            ProductivityAction::ActionSource(id) => {
                if let Some(item) = self
                    .productivity
                    .actions
                    .as_ref()
                    .and_then(|s| s.actions.iter().find(|a| a.id == id))
                {
                    let mut text = format!(
                        "# {}\n\n状态：{}\n\n## 引用原文\n\n",
                        item.action,
                        item.state.label()
                    );
                    for r in &item.evidence {
                        text.push_str(&format!("- {}\n", r.quote));
                    }
                    text.push_str("\n## 用户记录的进展（最近20次）\n\n");
                    for change in item.history.iter().rev().take(20) {
                        text.push_str(&format!(
                            "- {}：{}\n",
                            if change.progress_note {
                                if change.undo_of.is_some() { "撤销进展记录".to_owned() }
                                else { format!("记录进展（状态保持{}）", change.to.label()) }
                            } else { format!("{} → {}", change.from.label(), change.to.label()) },
                            change.note
                        ));
                    }
                    self.productivity.preview = Some(Preview {
                        title: "行动来源与进展".into(),
                        text,
                        path: None,
                        page: 0,
                        matters: item.matter_ids.clone(),
                    });
                }
            }
            ProductivityAction::Progress(period) => {
                self.productivity
                    .start("正在按真实行动状态整理进展…", move |_| {
                        let (title, text, path) = save_progress(&scope, period)?;
                        Ok(Output::Document {
                            title,
                            text,
                            path: Some(path),
                        })
                    })
            }
            ProductivityAction::MeetingCreate(id) | ProductivityAction::MeetingLink(id, _, _) => {
                let event = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| meeting_brief::event_key(e) == id)
                    .cloned();
                let Some(event) = event else {
                    self.productivity.feedback =
                        Some(("日程已变化，请刷新日历后重试".into(), true));
                    return;
                };
                if let ProductivityAction::MeetingLink(_, matter, include) = action.clone() {
                    self.productivity.start(
                        "正在保存本场会议的资料关联…",
                        move |_| {
                            meeting_brief::link_matter(
                                &context_root(),
                                &scope.device,
                                scope.account.as_deref(),
                                &event,
                                &matter,
                                include,
                            )?;
                            Ok(Output::Linked(meeting_brief::manual_links(
                                &context_root(),
                                &scope.device,
                                scope.account.as_deref(),
                            )?))
                        },
                    );
                } else {
                    let Some(device) = device else {
                        self.productivity.feedback = Some((
                            "连接手机后核对最新日程，再生成准备包；已有文件仍可阅读。".into(),
                            true,
                        ));
                        return;
                    };
                    self.productivity.start_cancellable(
                        "正在核对最新日程，并结合关联原文与行动整理准备包…",
                        move |cancel| {
                            let adb = Adb::discover()?;
                            anyhow::ensure!(
                                stepx_companion::amoo_usage::account_scope_fresh(&adb, &device)?
                                    == scope.account,
                                "手机账号变化，未读取会议资料"
                            );
                            let fresh =
                                stepx_companion::toolbox::inspect_stepos_ai_suite(&adb, &device)?;
                            let event = fresh
                                .calendar_events
                                .iter()
                                .find(|e| meeting_brief::event_key(e) == id)
                                .context("日程已改期或移除，请刷新手机日历后重新选择")?;
                            anyhow::ensure!(!cancel.load(Ordering::Relaxed), "准备包已停止");
                            anyhow::ensure!(
                                stepx_companion::amoo_usage::account_scope_fresh(&adb, &device)?
                                    == scope.account,
                                "手机账号变化，未生成会议资料"
                            );
                            Ok(Output::Brief(meeting_brief::create(
                                &context_root(),
                                &scope.device,
                                scope.account.as_deref(),
                                event,
                                Local::now().timestamp_millis(),
                            )?))
                        },
                    );
                }
            }
            ProductivityAction::SendPreview => {
                let Some(path) = self
                    .productivity
                    .preview
                    .as_ref()
                    .and_then(|p| p.path.clone())
                else {
                    return;
                };
                let expected_text = self.productivity.preview.as_ref().unwrap().text.clone();
                let Some(device) = device else {
                    self.productivity.feedback =
                        Some(("手机离线，文档已保存在电脑，连接后可重试回传".into(), true));
                    return;
                };
                self.productivity.start_cancellable(
                    "正在把这份文档送回手机并核对内容…",
                    move |cancel| {
                        Ok(Output::Sent(send_document(
                            &device,
                            &scope,
                            &path,
                            &expected_text,
                            &cancel,
                        )?))
                    },
                );
            }
            ProductivityAction::PhoneControl { task, op } => {
                if !["pause", "resume", "stop"].contains(&op.as_str()) {
                    return;
                }
                let snapshot = self
                    .productivity
                    .phone
                    .as_ref()
                    .filter(|p| {
                        p.task.as_deref() == Some(&task)
                            && !p.stale
                            && p.scope.device == scope.device
                            && p.scope.account == scope.account
                            && Local::now()
                                .timestamp_millis()
                                .saturating_sub(p.observed_ms)
                                <= 15_000
                    })
                    .cloned();
                let Some(device) = device else {
                    return;
                };
                let Some(snapshot) = snapshot else {
                    self.productivity.feedback =
                        Some(("手机任务状态已变化，请先刷新".into(), true));
                    return;
                };
                let control = match op.as_str() {
                    "pause" => phone_gui::PhoneGuiControl::Pause,
                    "resume" => phone_gui::PhoneGuiControl::Resume,
                    _ => phone_gui::PhoneGuiControl::Stop,
                };
                let Some(request) = snapshot.control_request(control) else {
                    self.productivity.feedback =
                        Some(("当前手机状态不允许这项操作，请先刷新核对".into(), true));
                    return;
                };
                self.productivity
                    .start("正在等待手机返回任务状态…", move |_| {
                        let adb = Adb::discover()?;
                        stepx_companion::companion_gui::call(&adb, &device, request)?;
                        let mut cache = phone_gui::StatusCache::default();
                        Ok(Output::Phone(cache.refresh(
                            &adb,
                            &device,
                            scope.account.as_deref(),
                        )))
                    });
            }
            _ => {}
        }
        ctx.notify();
    }

    fn productivity_button(
        &self,
        id: impl Into<String>,
        label: &str,
        icon: UiIcon,
        action: ProductivityAction,
        style: Btn,
    ) -> El {
        let always_clickable = matches!(
            action,
            ProductivityAction::Tab(_)
                | ProductivityAction::ListPage(_, _)
                | ProductivityAction::MaterialRead(_)
                | ProductivityAction::ActionSource(_)
                | ProductivityAction::ClosePreview
                | ProductivityAction::PreviewPage(_)
                | ProductivityAction::MeetingPickLink(_)
                | ProductivityAction::MaterialPickLink(_)
                | ProductivityAction::OpenMatter(_)
                | ProductivityAction::FullPeriod(_)
                | ProductivityAction::ToggleDismissed
                | ProductivityAction::PhoneReview
                | ProductivityAction::RefreshCalendar
                | ProductivityAction::Stop
        );
        if self.productivity.worker.is_some() && !always_clickable {
            self.pill_badge(label, self.p().text3, self.p().surface)
        } else {
            self.button_icon(id, label, icon, Action::Productivity(action), style)
        }
    }
    fn productivity_empty(&self, title: &str, description: &str) -> El {
        let p = self.p();
        self.card_box(
            self.vstack(
                5.,
                vec![
                    self.text_w(title, BODY, p.text, facloud_ui::fonts::Weight::Semibold),
                    self.text(description, CAPTION, p.text2),
                ],
            ),
            p.surface,
            p.border,
            R_SM,
            12.,
            12.,
        )
    }
    fn productivity_pager(&self, kind: usize, count: usize, size: usize) -> El {
        let last = count.saturating_sub(1) / size;
        let page = self.productivity.list_pages[kind].min(last);
        let mut controls = vec![];
        if page > 0 {
            controls.push(self.productivity_button(
                format!("productivity-list-{kind}-prev"),
                "上一页",
                UiIcon::ArrowLeft,
                ProductivityAction::ListPage(kind, page - 1),
                Btn::Ghost,
            ));
        }
        if last > 0 {
            controls.push(self.text(
                format!("第{} / {}页 · 共{}项", page + 1, last + 1, count),
                MICRO,
                self.p().text3,
            ));
        }
        if page < last {
            controls.push(self.productivity_button(
                format!("productivity-list-{kind}-next"),
                "下一页",
                UiIcon::ArrowRight,
                ProductivityAction::ListPage(kind, page + 1),
                Btn::Ghost,
            ));
        }
        self.wrap(controls)
    }
    fn productivity_source_choices(&self, material: Option<&str>, event: Option<&str>) -> El {
        let scope_id = self.productivity.scope.as_ref().map(Scope::id);
        let mut choices = vec![self.text(
            "选择需要关联的事项；不会按相似标题自动合并资料",
            CAPTION,
            self.p().text2,
        )];
        let items: Vec<_> = self
            .context
            .overview
            .items
            .iter()
            .filter(|i| Some(&i.scope_id) == scope_id.as_ref())
            .collect();
        let page = self.productivity.list_pages[2].min(items.len().saturating_sub(1) / 16);
        for (n, item) in items.iter().skip(page * 16).take(16).enumerate() {
            let included = event
                .and_then(|e| self.productivity.meetings.get(e))
                .is_some_and(|ids| ids.contains(&item.id));
            let action = if let Some(id) = material {
                ProductivityAction::MaterialLink(id.into(), item.id.clone())
            } else {
                ProductivityAction::MeetingLink(
                    event.unwrap_or_default().to_owned(),
                    item.id.clone(),
                    !included,
                )
            };
            choices.push(self.productivity_button(
                format!("productivity-link-{n}"),
                &format!(
                    "{}{}",
                    if included {
                        "移除关联："
                    } else {
                        "关联："
                    },
                    short(&item.title, 36)
                ),
                if included {
                    UiIcon::Close
                } else {
                    UiIcon::Plus
                },
                action,
                Btn::Secondary,
            ));
        }
        if choices.len() == 1 {
            choices.push(self.text(
                "还没有可关联事项，可以先在“讨论与成果”整理来源。",
                CAPTION,
                self.p().text2,
            ));
        }
        choices.push(self.productivity_pager(2, items.len(), 16));
        self.card_box(
            self.vstack(S2, choices),
            self.p().surface,
            self.p().border,
            R_SM,
            12.,
            12.,
        )
    }
    pub(super) fn productivity_upcoming_meetings_count(&self) -> usize {
        let now = Local::now().timestamp_millis();
        self.back_screen
            .stepos_suite
            .calendar_events
            .iter()
            .filter(|e| {
                (e.dtstart == 0 && e.dtend == 0)
                    || (e.dtend.max(e.dtstart) >= now && e.dtstart < now + 14 * 86_400_000)
            })
            .count()
    }

    pub(super) fn productivity_effective_height(&self) -> super::pages::home::HomeModuleHeight {
        use super::pages::home::HomeModuleId;
        self.resolve_effective_height(
            HomeModuleId::ProductivityHub,
            self.home_modules.module_height(HomeModuleId::ProductivityHub),
        )
    }

    fn productivity_char_budgets(
        mod_w: super::pages::home::HomeModuleWidth,
    ) -> (usize, usize) {
        use super::pages::home::HomeModuleWidth;
        match mod_w {
            HomeModuleWidth::OneThird => (18, 28),
            HomeModuleWidth::OneHalf => (28, 44),
            HomeModuleWidth::TwoThirds => (40, 64),
            HomeModuleWidth::Full => (58, 96),
        }
    }

    fn productivity_per_page(
        mod_w: super::pages::home::HomeModuleWidth,
        effective_h: super::pages::home::HomeModuleHeight,
    ) -> usize {
        use super::pages::home::{HomeModuleHeight, HomeModuleWidth};
        match effective_h {
            HomeModuleHeight::Compact => match mod_w {
                HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
                HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
            },
            HomeModuleHeight::Tall => match mod_w {
                HomeModuleWidth::OneThird => 4,
                HomeModuleWidth::OneHalf | HomeModuleWidth::TwoThirds => 6,
                HomeModuleWidth::Full => 8,
            },
            HomeModuleHeight::Standard | HomeModuleHeight::Auto => match mod_w {
                HomeModuleWidth::OneThird => 2,
                HomeModuleWidth::OneHalf => 3,
                HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 4,
            },
        }
    }

    pub(super) fn productivity_value_tier_spec(
        &self,
    ) -> (usize, usize, usize, usize, bool, bool) {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let mod_w = self.home_module_content_width(HomeModuleId::ProductivityHub);
        let effective_h = self.productivity_effective_height();
        let bento_cols = match mod_w {
            HomeModuleWidth::OneThird => 1,
            HomeModuleWidth::OneHalf => 2,
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 3,
        };
        let item_cols = match mod_w {
            HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf => 1,
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => 2,
        };
        let per_page = Self::productivity_per_page(mod_w, effective_h);
        let (title_budget, _) = Self::productivity_char_budgets(mod_w);
        let is_compact_headline = effective_h == HomeModuleHeight::Compact;
        let has_tall_workbench = effective_h == HomeModuleHeight::Tall;
        (
            bento_cols,
            item_cols,
            per_page,
            title_budget,
            is_compact_headline,
            has_tall_workbench,
        )
    }

    fn render_productivity_materials(&self) -> El {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let p = self.p();
        let mod_w = self.home_module_content_width(HomeModuleId::ProductivityHub);
        let effective_h = self.productivity_effective_height();
        let is_compact = effective_h == HomeModuleHeight::Compact;
        let is_tall = effective_h == HomeModuleHeight::Tall;
        let is_narrow = matches!(mod_w, HomeModuleWidth::OneThird | HomeModuleWidth::OneHalf);
        let (title_budget, detail_budget) = Self::productivity_char_budgets(mod_w);
        let per_page = Self::productivity_per_page(mod_w, effective_h);

        let mut top_bar = vec![self.productivity_button(
            "productivity-collect",
            "接收手机已选资料",
            UiIcon::Download,
            ProductivityAction::Collect,
            Btn::Primary,
        )];
        if !is_compact {
            top_bar.push(self.text(
                Self::truncate_chars(
                    "在手机选择资料，或从已授权的截屏、录音线索开始。点击处理后才读取内容。",
                    detail_budget * 2,
                ),
                CAPTION,
                p.text2,
            ));
        }
        let mut rows = vec![self.wrap(top_bar)];
        let Some(snapshot) = &self.productivity.materials else {
            rows.push(self.productivity_empty(
                "资料尚未读取",
                "连接手机并确认账号后接收；现有账号范围之外的资料不会显示在这里。",
            ));
            return self.vstack(S2, rows);
        };
        if !is_compact && !snapshot.note.is_empty() {
            rows.push(self.text(
                Self::truncate_chars(&snapshot.note, detail_budget * 2),
                CAPTION,
                p.text2,
            ));
        }
        let items: Vec<_> = snapshot
            .items
            .iter()
            .filter(|i| i.state != inbox::MaterialState::Ignored)
            .collect();
        let page = self.productivity.list_pages[0].min(items.len().saturating_sub(1) / per_page);
        if items.is_empty() {
            rows.push(self.productivity_empty(
                "还没有待整理资料",
                "手机伴侣中选择文件并授权给这台电脑；已接收的资料会保留处理状态。",
            ));
        }
        let max_buttons = match mod_w {
            HomeModuleWidth::OneThird => 2,
            HomeModuleWidth::OneHalf => 3,
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => usize::MAX,
        };
        let mut material_cards = Vec::new();
        for (n, item) in items.iter().skip(page * per_page).take(per_page).enumerate() {
            let mut buttons = vec![];
            if item.state == inbox::MaterialState::Ready && item.error.is_some() {
                buttons.push(self.productivity_button(
                    format!("productivity-material-reindex-{n}"),
                    "重试收录正文",
                    UiIcon::Refresh,
                    ProductivityAction::MaterialProcess(item.id.clone()),
                    Btn::Secondary,
                ));
            }
            if item.state == inbox::MaterialState::Ready {
                buttons.push(self.productivity_button(
                    format!("productivity-material-read-{n}"),
                    "查看正文",
                    UiIcon::File,
                    ProductivityAction::MaterialRead(item.id.clone()),
                    Btn::Secondary,
                ));
                buttons.push(self.productivity_button(
                    format!("productivity-material-draft-{n}"),
                    "生成方案与待办",
                    UiIcon::Sparkles,
                    ProductivityAction::MaterialDraft(item.id.clone()),
                    Btn::Primary,
                ));
            } else {
                buttons.push(self.productivity_button(
                    format!("productivity-material-process-{n}"),
                    if item.state == inbox::MaterialState::Failed {
                        "重新处理"
                    } else {
                        "读取并整理"
                    },
                    UiIcon::Sparkles,
                    ProductivityAction::MaterialProcess(item.id.clone()),
                    Btn::Primary,
                ));
            }
            if item.state == inbox::MaterialState::Ready {
                buttons.push(self.productivity_button(
                    format!("productivity-material-link-{n}"),
                    if item.linked_item.is_some() {
                        "调整关联事项"
                    } else {
                        "关联事项"
                    },
                    UiIcon::Layers,
                    ProductivityAction::MaterialPickLink(item.id.clone()),
                    Btn::Ghost,
                ));
            } else if item.state != inbox::MaterialState::Processing {
                buttons.push(self.productivity_button(
                    format!("productivity-material-ignore-{n}"),
                    "暂不处理",
                    UiIcon::Close,
                    ProductivityAction::MaterialIgnore(item.id.clone()),
                    Btn::Ghost,
                ));
            }
            buttons.truncate(max_buttons);
            let mut badges = vec![
                self.text_w(
                    Self::truncate_chars(&item.title, title_budget),
                    BODY,
                    p.text,
                    facloud_ui::fonts::Weight::Semibold,
                ),
                self.pill_badge(item.kind.label(), p.text2, p.surface),
                self.pill_badge(
                    item.state.label(),
                    if item.state == inbox::MaterialState::Ready {
                        p.success_text
                    } else {
                        p.text2
                    },
                    p.surface,
                ),
            ];
            if is_tall {
                badges.push(self.pill_badge(
                    &item.source_label,
                    p.indigo,
                    p.indigo_soft,
                ));
            }
            let mut copy = vec![
                self.wrap(badges),
                self.text(
                    format!(
                        "{} · {} · {}",
                        item.source_label,
                        item.content_method,
                        if item.content_truncated {
                            "正文已截短"
                        } else {
                            "保留来源引用"
                        }
                    ),
                    MICRO,
                    p.text2,
                ),
                self.text(
                    short(
                        item.error.as_deref().unwrap_or(&item.summary),
                        detail_budget * 2,
                    ),
                    CAPTION,
                    if item.error.is_some() {
                        p.danger
                    } else {
                        p.text2
                    },
                ),
                self.wrap(buttons),
            ];
            if self.productivity.material_link.as_ref() == Some(&item.id) {
                copy.push(self.productivity_source_choices(Some(&item.id), None));
            }
            material_cards.push(self.card_box(
                self.vstack(S2, copy),
                p.card,
                p.border,
                R_SM,
                12.,
                12.,
            ));
        }
        if !material_cards.is_empty() {
            let cols = if !is_narrow && material_cards.len() >= 2 {
                2
            } else {
                1
            };
            rows.push(self.grid_cols(material_cards, cols, S2));
        }
        rows.push(self.productivity_pager(0, items.len(), per_page));
        let ignored: Vec<_> = snapshot
            .items
            .iter()
            .filter(|i| i.state == inbox::MaterialState::Ignored)
            .collect();
        let ignored_count = ignored.len();
        let ignored_page = self.productivity.list_pages[4].min(ignored_count.saturating_sub(1) / 8);
        if is_tall && !ignored.is_empty() {
            rows.push(self.text("暂不处理的资料", MICRO, p.text3));
            rows.push(self.productivity_pager(4, ignored_count, 8));
            rows.push(
                self.wrap(
                    ignored
                        .into_iter()
                        .skip(ignored_page * 8)
                        .take(8)
                        .enumerate()
                        .map(|(n, i)| {
                            self.productivity_button(
                                format!("productivity-restore-{n}"),
                                &format!("恢复：{}", short(&i.title, title_budget)),
                                UiIcon::Refresh,
                                ProductivityAction::MaterialRestore(i.id.clone()),
                                Btn::Ghost,
                            )
                        })
                        .collect(),
                ),
            );
        }
        self.vstack(S2, rows)
    }

    fn render_productivity_actions(&self) -> El {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let p = self.p();
        let hub_w = self.home_module_content_width(HomeModuleId::ProductivityHub);
        let effective_h = self.productivity_effective_height();
        let is_compact = effective_h == HomeModuleHeight::Compact;
        let is_tall = effective_h == HomeModuleHeight::Tall;
        let is_narrow = matches!(hub_w, HomeModuleWidth::OneHalf | HomeModuleWidth::OneThird);
        let (title_budget, detail_budget) = Self::productivity_char_budgets(hub_w);
        let per_page = Self::productivity_per_page(hub_w, effective_h);

        let mut rows = Vec::new();
        if !is_compact {
            rows.push(self.text(
                Self::truncate_chars(
                    "先核对原文，再采纳行动。负责人和原约定期限保留来源；状态变化会进入日周月进展。",
                    detail_budget * 2,
                ),
                CAPTION,
                p.text2,
            ));
            rows.push(self.input(
                F_PRODUCTIVITY_NOTE,
                "填写进展或延期原因，再点击对应行动的按钮（500字内）",
                None,
            ));
        }
        let mut progress_btns = vec![
            self.productivity_button(
                "productivity-open-inbox",
                "打开资料收件箱",
                UiIcon::Download,
                ProductivityAction::Tab(1),
                Btn::Primary,
            ),
            self.productivity_button(
                "productivity-check-candidates",
                "查看并采纳",
                UiIcon::Check,
                ProductivityAction::Tab(2),
                Btn::Secondary,
            ),
        ];
        progress_btns.extend(
            [Period::Day, Period::Week, Period::Month]
                .into_iter()
                .enumerate()
                .map(|(n, period)| {
                    self.productivity_button(
                        format!("productivity-progress-{n}"),
                        match period {
                            Period::Day => "今日行动进展",
                            Period::Week => "本周行动进展",
                            Period::Month => "本月行动进展",
                        },
                        UiIcon::Activity,
                        ProductivityAction::Progress(period),
                        Btn::Secondary,
                    )
                }),
        );
        rows.push(self.wrap(progress_btns));
        if !is_compact {
            rows.push(
                self.wrap(
                    [Period::Day, Period::Week, Period::Month]
                        .into_iter()
                        .enumerate()
                        .map(|(n, period)| {
                            self.productivity_button(
                                format!("productivity-full-period-{n}"),
                                &format!("{} · 讨论与成果", period.label()),
                                UiIcon::File,
                                ProductivityAction::FullPeriod(period),
                                Btn::Ghost,
                            )
                        })
                        .collect(),
                ),
            );
        }
        let Some(snapshot) = &self.productivity.actions else {
            rows.push(self.productivity_empty(
                "行动记录尚未读取",
                "先读取当前设备与账号的资料；已有记录读取失败时会保留错误原因。",
            ));
            return self.vstack(S2, rows);
        };
        if snapshot.actions.is_empty() {
            rows.push(self.productivity_empty(
                "还没有行动候选",
                "讨论生成的方案与待办经来源校验后会进入这里，也可从事项中继续生成成果。",
            ));
        }
        let dismissed = snapshot
            .actions
            .iter()
            .filter(|a| a.state == ActionState::Dismissed)
            .count();
        if is_tall && dismissed > 0 {
            rows.push(self.productivity_button(
                "productivity-toggle-dismissed",
                &format!(
                    "{}已忽略的行动（{}）",
                    if self.productivity.show_dismissed {
                        "收起"
                    } else {
                        "查看"
                    },
                    dismissed
                ),
                UiIcon::Refresh,
                ProductivityAction::ToggleDismissed,
                Btn::Ghost,
            ));
        }
        let mut actions: Vec<_> = snapshot
            .actions
            .iter()
            .filter(|a| {
                (is_tall && self.productivity.show_dismissed) || a.state != ActionState::Dismissed
            })
            .collect();
        actions.sort_by_key(|a| match a.state {
            ActionState::InProgress => 0,
            ActionState::Accepted => 1,
            ActionState::Deferred => 2,
            ActionState::Candidate => 3,
            _ => 4,
        });
        let action_count = actions.len();
        let max_buttons = match hub_w {
            HomeModuleWidth::OneThird => 2,
            HomeModuleWidth::OneHalf => 3,
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => usize::MAX,
        };
        let page = self.productivity.list_pages[1].min(action_count.saturating_sub(1) / per_page);
        let mut action_cards = Vec::new();
        for (n, action) in actions
            .into_iter()
            .skip(page * per_page)
            .take(per_page)
            .enumerate()
        {
            let set = |label: &str, state| {
                self.productivity_button(
                    format!("productivity-action-{n}-{state:?}"),
                    label,
                    UiIcon::Check,
                    ProductivityAction::ActionState {
                        id: action.id.clone(),
                        revision: snapshot.revision,
                        state,
                    },
                    Btn::Secondary,
                )
            };
            let mut buttons = vec![];
            match action.state {
                ActionState::Candidate => {
                    buttons.push(set("核对后采纳", ActionState::Accepted));
                    buttons.push(set("忽略", ActionState::Dismissed));
                }
                ActionState::Accepted | ActionState::Deferred => {
                    buttons.push(set("开始推进", ActionState::InProgress));
                    buttons.push(set("确认完成", ActionState::Completed));
                    buttons.push(set("延期", ActionState::Deferred));
                }
                ActionState::InProgress => {
                    buttons.push(set("确认完成", ActionState::Completed));
                    buttons.push(set("延期", ActionState::Deferred));
                }
                ActionState::Dismissed => buttons.push(set("恢复为候选", ActionState::Candidate)),
                ActionState::Completed if action.needs_review => {
                    buttons.push(set("重新核对后保持完成", ActionState::Completed))
                }
                _ => {}
            }
            buttons.push(self.productivity_button(
                format!("productivity-action-source-{n}"),
                "来源与进展",
                UiIcon::File,
                ProductivityAction::ActionSource(action.id.clone()),
                Btn::Ghost,
            ));
            if action.state.confirmed() {
                buttons.push(self.productivity_button(
                    format!("productivity-action-progress-{n}"),
                    "记录进展",
                    UiIcon::File,
                    ProductivityAction::RecordProgress {
                        id: action.id.clone(),
                        revision: snapshot.revision,
                    },
                    Btn::Secondary,
                ));
            }
            if !action.history.is_empty() {
                buttons.push(self.productivity_button(
                    format!("productivity-action-undo-{n}"),
                    "撤销最近记录",
                    UiIcon::Refresh,
                    ProductivityAction::UndoAction {
                        id: action.id.clone(),
                        revision: snapshot.revision,
                    },
                    Btn::Ghost,
                ));
            }
            buttons.truncate(max_buttons);
            let mut header_badges = vec![
                self.pill_badge(
                    action.state.label(),
                    if action.state == ActionState::Completed && !action.needs_review {
                        p.success_text
                    } else {
                        p.text2
                    },
                    p.surface,
                ),
                self.text_w(
                    Self::truncate_chars(&action.action, title_budget),
                    BODY,
                    p.text,
                    facloud_ui::fonts::Weight::Semibold,
                ),
            ];
            if is_tall {
                header_badges.push(self.pill_badge(
                    if action.kind == "suggested" {
                        "建议候选"
                    } else {
                        "原文溯源"
                    },
                    p.teal,
                    p.teal_soft,
                ));
            }
            let detail_line = match (&action.owner, &action.due) {
                (Some(owner), Some(due)) => {
                    format!("原文分工：{} · 原约定：{}", owner.value, due.value)
                }
                (Some(owner), None) => format!("原文分工：{}", owner.value),
                (None, Some(due)) => format!("原约定：{}", due.value),
                (None, None) if action.kind == "suggested" => {
                    "备选建议，采纳前不计作你的计划。".into()
                }
                _ => "原文未约定负责人或期限。".into(),
            };
            let mut copy = vec![
                self.wrap(header_badges),
                self.text(
                    Self::truncate_chars(&detail_line, detail_budget),
                    CAPTION,
                    p.text2,
                ),
            ];
            if action.needs_review {
                copy.push(self.text(
                    "来源已变化，需要重新核对这项行动。",
                    CAPTION,
                    p.warning_text,
                ));
            }
            if let Some(last) = action.history.last() {
                if !last.note.is_empty() {
                    copy.push(self.text(
                        Self::truncate_chars(&format!("最近进展：{}", last.note), detail_budget),
                        CAPTION,
                        p.text2,
                    ));
                }
            }
            copy.push(self.wrap(buttons));
            action_cards.push(self.card_box(
                self.vstack(S2, copy),
                p.card,
                p.border,
                R_SM,
                12.,
                12.,
            ));
        }
        if !action_cards.is_empty() {
            let cols = if !is_narrow && action_cards.len() >= 2 {
                2
            } else {
                1
            };
            rows.push(self.grid_cols(action_cards, cols, S2));
        }
        rows.push(self.productivity_pager(1, action_count, per_page));
        self.vstack(S2, rows)
    }

    fn render_productivity_meetings(&self) -> El {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let p = self.p();
        let mod_w = self.home_module_content_width(HomeModuleId::ProductivityHub);
        let effective_h = self.productivity_effective_height();
        let is_compact = effective_h == HomeModuleHeight::Compact;
        let is_tall = effective_h == HomeModuleHeight::Tall;
        let is_narrow = matches!(mod_w, HomeModuleWidth::OneHalf | HomeModuleWidth::OneThird);
        let (title_budget, detail_budget) = Self::productivity_char_budgets(mod_w);
        let per_page = Self::productivity_per_page(mod_w, effective_h);

        let now = Local::now().timestamp_millis();
        let scope_id = self.productivity.scope.as_ref().map(Scope::id);
        let mut events: Vec<_> = self
            .back_screen
            .stepos_suite
            .calendar_events
            .iter()
            .filter(|e| {
                (e.dtstart == 0 && e.dtend == 0)
                    || (e.dtend.max(e.dtstart) >= now && e.dtstart < now + 14 * 86_400_000)
            })
            .collect();
        events.sort_by_key(|e| e.dtstart);
        let mut rows = Vec::new();
        if !is_compact {
            rows.push(self.text(
                Self::truncate_chars(
                    "把最新日程、已关联的原文与行动汇成一页。可以明确选择相关事项，生成后阅读或带回手机。",
                    detail_budget * 2,
                ),
                CAPTION,
                p.text2,
            ));
        }
        rows.push(self.wrap(vec![
            self.productivity_button(
                "productivity-calendar-refresh",
                "刷新手机日历",
                UiIcon::Refresh,
                ProductivityAction::RefreshCalendar,
                Btn::Secondary,
            ),
            self.text(
                self.back_screen
                    .stepos_suite_synced_at
                    .map(|t| {
                        format!(
                            "日历快照 · {}秒前读取；生成时再次核对改期",
                            t.elapsed().as_secs()
                        )
                    })
                    .unwrap_or_else(|| "日历尚未刷新，生成前会再次核对手机".into()),
                MICRO,
                p.text3,
            ),
        ]));
        if events.is_empty() {
            rows.push(self.productivity_empty(
                "近两周暂无已读取的日程",
                "可先同步日历或添加会议；没有会议时不会编造准备资料。",
            ));
            rows.push(self.button_icon(
                "productivity-calendar",
                "打开日历",
                UiIcon::Calendar,
                Action::Page(Page::Calendar),
                Btn::Secondary,
            ));
        }
        let event_count = events.len();
        let page = self.productivity.list_pages[3].min(event_count.saturating_sub(1) / per_page);
        let mut meeting_cards = Vec::new();
        for (n, event) in events.into_iter().skip(page * per_page).take(per_page).enumerate() {
            let mut linked_ids: std::collections::BTreeSet<_> = scope_id
                .as_ref()
                .map(|scope| {
                    meeting_brief::automatic_matters(&self.context.overview, scope, event)
                        .iter()
                        .map(|m| m.id.clone())
                        .collect()
                })
                .unwrap_or_default();
            if let Some(ids) = self
                .productivity
                .meetings
                .get(&meeting_brief::event_key(event))
            {
                linked_ids.extend(
                    ids.iter()
                        .filter(|id| {
                            self.context
                                .overview
                                .items
                                .iter()
                                .any(|m| &m.id == *id && Some(&m.scope_id) == scope_id.as_ref())
                        })
                        .cloned(),
                );
            }
            let linked = linked_ids.len();
            let mut title_row = vec![self.text_w(
                Self::truncate_chars(&event.title, title_budget),
                BODY,
                p.text,
                facloud_ui::fonts::Weight::Semibold,
            )];
            if is_tall {
                title_row.push(self.pill_badge(
                    format!("{linked} 项关联"),
                    p.indigo,
                    p.indigo_soft,
                ));
            }
            let detail_str = format!(
                "{} · {} · {linked}件已关联事项",
                event.time_range_label(),
                if event.location.is_empty() {
                    "地点待确认"
                } else {
                    &event.location
                }
            );
            let mut copy = vec![
                self.wrap(title_row),
                self.text(
                    Self::truncate_chars(&detail_str, detail_budget),
                    CAPTION,
                    p.text2,
                ),
                self.wrap(vec![
                    self.productivity_button(
                        format!("productivity-meeting-create-{n}"),
                        "准备资料包",
                        UiIcon::File,
                        ProductivityAction::MeetingCreate(meeting_brief::event_key(event)),
                        Btn::Primary,
                    ),
                    self.productivity_button(
                        format!("productivity-meeting-link-{n}"),
                        "选择相关事项",
                        UiIcon::Layers,
                        ProductivityAction::MeetingPickLink(meeting_brief::event_key(event)),
                        Btn::Secondary,
                    ),
                ]),
            ];
            if self.productivity.meeting_link.as_deref()
                == Some(meeting_brief::event_key(event).as_str())
            {
                copy.push(self.productivity_source_choices(
                    None,
                    Some(meeting_brief::event_key(event).as_str()),
                ));
            }
            meeting_cards.push(self.card_box(
                self.vstack(S2, copy),
                p.card,
                p.border,
                R_SM,
                12.,
                12.,
            ));
        }
        if !meeting_cards.is_empty() {
            let cols = if !is_narrow && meeting_cards.len() >= 2 {
                2
            } else {
                1
            };
            rows.push(self.grid_cols(meeting_cards, cols, S2));
        }
        rows.push(self.productivity_pager(3, event_count, per_page));
        self.vstack(S2, rows)
    }

    fn render_productivity_overview(&self) -> El {
        use super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let p = self.p();
        let mod_w = self.home_module_content_width(HomeModuleId::ProductivityHub);
        let effective_h = self.productivity_effective_height();
        let (title_budget, detail_budget) = Self::productivity_char_budgets(mod_w);

        let mut phone_card: Option<El> = None;
        if let Some(phone) = &self.productivity.phone {
            if phone.phase.active()
                || matches!(
                    phone.phase,
                    phone_gui::PhoneGuiPhase::NeedsInput
                        | phone_gui::PhoneGuiPhase::Paused
                        | phone_gui::PhoneGuiPhase::Failed
                        | phone_gui::PhoneGuiPhase::Reported
                        | phone_gui::PhoneGuiPhase::Interrupted
                        | phone_gui::PhoneGuiPhase::Unknown
                )
                || phone.error.is_some()
            {
                let mut buttons = vec![
                    self.productivity_button(
                        "productivity-phone-read",
                        "刷新手机状态",
                        UiIcon::Refresh,
                        ProductivityAction::Refresh,
                        Btn::Secondary,
                    ),
                    self.productivity_button(
                        "productivity-phone-review",
                        "查看手机执行器",
                        UiIcon::Smartphone,
                        ProductivityAction::PhoneReview,
                        Btn::Ghost,
                    ),
                ];
                if !phone.stale
                    && Local::now()
                        .timestamp_millis()
                        .saturating_sub(phone.observed_ms)
                        <= 15_000
                {
                    if let Some(task) = &phone.task {
                        if phone
                            .control_request(phone_gui::PhoneGuiControl::Resume)
                            .is_some()
                        {
                            buttons.push(self.productivity_button(
                                "productivity-phone-resume",
                                "继续原任务",
                                UiIcon::Refresh,
                                ProductivityAction::PhoneControl {
                                    task: task.clone(),
                                    op: "resume".into(),
                                },
                                Btn::Primary,
                            ));
                        } else if phone
                            .control_request(phone_gui::PhoneGuiControl::Pause)
                            .is_some()
                        {
                            buttons.push(self.productivity_button(
                                "productivity-phone-pause",
                                "暂停原任务",
                                UiIcon::Clock,
                                ProductivityAction::PhoneControl {
                                    task: task.clone(),
                                    op: "pause".into(),
                                },
                                Btn::Secondary,
                            ));
                        }
                        if phone
                            .control_request(phone_gui::PhoneGuiControl::Stop)
                            .is_some()
                        {
                            buttons.push(self.productivity_button(
                                "productivity-phone-stop",
                                "停止原任务",
                                UiIcon::Close,
                                ProductivityAction::PhoneControl {
                                    task: task.clone(),
                                    op: "stop".into(),
                                },
                                Btn::Ghost,
                            ));
                        }
                    }
                }
                phone_card = Some(self.card_box(
                    self.vstack(
                        S2,
                        vec![
                            self.text_w(
                                Self::truncate_chars(phone.phase.label(), title_budget),
                                BODY,
                                p.text,
                                facloud_ui::fonts::Weight::Semibold,
                            ),
                            self.text(
                                Self::truncate_chars(
                                    phone.error.as_deref().unwrap_or(if phone.stale {
                                        "保留上次状态，需刷新确认后才能继续"
                                    } else {
                                        "执行在手机上进行，电脑只显示对应任务的真实状态"
                                    }),
                                    detail_budget,
                                ),
                                CAPTION,
                                p.text2,
                            ),
                            self.wrap(buttons),
                        ],
                    ),
                    p.surface,
                    p.border,
                    R_SM,
                    12.,
                    12.,
                ));
            }
        }

        let candidates = self
            .productivity
            .actions
            .as_ref()
            .map(|s| {
                s.actions
                    .iter()
                    .filter(|a| a.state == ActionState::Candidate)
                    .count()
            })
            .unwrap_or(0);
        // Count the full ledger; only individual preview rows are limited.
        let active_actions = active_productivity_actions(self.productivity.actions.as_ref());
        let pending_material_items: Vec<_> = self
            .productivity
            .materials
            .as_ref()
            .map(|s| {
                s.items
                    .iter()
                    .filter(|i| {
                        matches!(
                            i.state,
                            inbox::MaterialState::Discovered | inbox::MaterialState::Failed
                        )
                    })
                    .collect()
            })
            .unwrap_or_default();
        let pending = pending_material_items.len();
        let total_materials = self
            .productivity
            .materials
            .as_ref()
            .map(|s| {
                s.items
                    .iter()
                    .filter(|i| i.state != inbox::MaterialState::Ignored)
                    .count()
            })
            .unwrap_or(0);
        let cal_events = self.productivity_upcoming_meetings_count();

        // Compact density: 1-row summary pill bar + single #1 highest-priority actionable card
        if effective_h == HomeModuleHeight::Compact {
            let mut summary_pills = vec![
                self.pill_badge(
                    format!("待整理 {pending} · 待核对 {candidates} · 备会 {cal_events}"),
                    p.accent,
                    p.accent_soft,
                ),
            ];
            if matches!(mod_w, HomeModuleWidth::TwoThirds | HomeModuleWidth::Full) {
                summary_pills.push(self.pill_badge(
                    format!("资料库 {total_materials} 份"),
                    p.teal,
                    p.teal_soft,
                ));
            }
            if mod_w == HomeModuleWidth::Full {
                summary_pills.push(self.pill_badge(
                    format!("推进中 {} 项", active_actions.len()),
                    p.indigo,
                    p.indigo_soft,
                ));
            }
            let top_card = if let Some(card) = phone_card {
                card
            } else if let Some(mat) = pending_material_items.first() {
                self.card_box(
                    row_between_expanded(
                        self.hstack(
                            6.,
                            vec![
                                self.icon_badge(
                                    UiIcon::Download,
                                    14.,
                                    p.accent,
                                    p.accent_soft,
                                    26.,
                                ),
                                self.text_w(
                                    Self::truncate_chars(&mat.title, title_budget),
                                    BODY,
                                    p.text,
                                    facloud_ui::fonts::Weight::Semibold,
                                ),
                                self.text(
                                    Self::truncate_chars(&mat.summary, detail_budget),
                                    CAPTION,
                                    p.text2,
                                ),
                            ],
                        ),
                        self.productivity_button(
                            "productivity-compact-top-material",
                            "读取并整理",
                            UiIcon::Sparkles,
                            ProductivityAction::MaterialProcess(mat.id.clone()),
                            Btn::Primary,
                        ),
                        S1,
                    ),
                    p.card,
                    p.border,
                    R_SM,
                    12.,
                    10.,
                )
            } else if let Some((rev, act)) = self.productivity.actions.as_ref().and_then(|s| {
                s.actions
                    .iter()
                    .find(|a| a.state == ActionState::Candidate)
                    .or_else(|| active_actions.first().copied())
                    .map(|a| (s.revision, a))
            }) {
                let (cta_label, target_state) = if act.state == ActionState::Candidate {
                    ("核对后采纳", ActionState::Accepted)
                } else {
                    ("确认完成", ActionState::Completed)
                };
                self.card_box(
                    row_between_expanded(
                        self.hstack(
                            6.,
                            vec![
                                self.icon_badge(UiIcon::Check, 14., p.teal, p.teal_soft, 26.),
                                self.text_w(
                                    Self::truncate_chars(&act.action, title_budget),
                                    BODY,
                                    p.text,
                                    facloud_ui::fonts::Weight::Semibold,
                                ),
                            ],
                        ),
                        self.productivity_button(
                            "productivity-compact-top-action",
                            cta_label,
                            UiIcon::Check,
                            ProductivityAction::ActionState {
                                id: act.id.clone(),
                                revision: rev,
                                state: target_state,
                            },
                            Btn::Primary,
                        ),
                        S1,
                    ),
                    p.card,
                    p.border,
                    R_SM,
                    12.,
                    10.,
                )
            } else {
                self.card_box(
                    row_between_expanded(
                        self.hstack(
                            6.,
                            vec![
                                self.icon_badge(
                                    UiIcon::Download,
                                    14.,
                                    p.accent,
                                    p.accent_soft,
                                    26.,
                                ),
                                self.text_w(
                                    Self::truncate_chars("手机资料收件箱与行动就绪", title_budget),
                                    BODY,
                                    p.text,
                                    facloud_ui::fonts::Weight::Semibold,
                                ),
                            ],
                        ),
                        self.productivity_button(
                            "productivity-open-inbox",
                            "打开资料收件箱",
                            UiIcon::Download,
                            ProductivityAction::Tab(1),
                            Btn::Primary,
                        ),
                        S1,
                    ),
                    p.card,
                    p.border,
                    R_SM,
                    12.,
                    10.,
                )
            };
            return self.vstack(S2, vec![self.wrap(summary_pills), top_card]);
        }

        let mut rows = Vec::new();
        if let Some(card) = phone_card {
            rows.push(card);
        }

        let materials_count = if pending > 0 { pending } else { total_materials };
        let materials_card = self.card_box(
            self.vstack(
                6.,
                vec![
                    row_between_expanded(
                        self.hstack(
                            6.,
                            vec![
                                self.icon_badge(
                                    UiIcon::Download,
                                    14.,
                                    p.accent,
                                    p.accent_soft,
                                    26.,
                                ),
                                self.text_w(
                                    if pending > 0 { "待整理资料" } else { "手机资料" },
                                    BODY,
                                    p.text,
                                    facloud_ui::fonts::Weight::Semibold,
                                ),
                            ],
                        ),
                        self.pill_badge(materials_count.to_string(), p.accent, p.accent_soft),
                        S1,
                    ),
                    self.wrap(vec![self.productivity_button(
                        "productivity-open-inbox",
                        "打开",
                        UiIcon::Download,
                        ProductivityAction::Tab(1),
                        Btn::Primary,
                    )]),
                ],
            ),
            p.card,
            p.border,
            R_SM,
            12.,
            10.,
        );

        let actions_count = if candidates > 0 { candidates } else { active_actions.len() };
        let actions_card = self.card_box(
            self.vstack(
                6.,
                vec![
                    row_between_expanded(
                        self.hstack(
                            6.,
                            vec![
                                self.icon_badge(UiIcon::Check, 14., p.teal, p.teal_soft, 26.),
                                self.text_w(
                                    if candidates > 0 { "待核对行动" } else { "行动跟踪" },
                                    BODY,
                                    p.text,
                                    facloud_ui::fonts::Weight::Semibold,
                                ),
                            ],
                        ),
                        self.pill_badge(actions_count.to_string(), p.teal, p.teal_soft),
                        S1,
                    ),
                    self.wrap(vec![self.productivity_button(
                        "productivity-check-candidates",
                        if candidates > 0 { "核对" } else { "跟进" },
                        UiIcon::Check,
                        ProductivityAction::Tab(2),
                        Btn::Secondary,
                    )]),
                ],
            ),
            p.card,
            p.border,
            R_SM,
            12.,
            10.,
        );

        let meetings_card = self.card_box(
            self.vstack(
                6.,
                vec![
                    row_between_expanded(
                        self.hstack(
                            6.,
                            vec![
                                self.icon_badge(
                                    UiIcon::Calendar,
                                    14.,
                                    p.indigo,
                                    p.indigo_soft,
                                    26.,
                                ),
                                self.text_w(
                                    "会前准备",
                                    BODY,
                                    p.text,
                                    facloud_ui::fonts::Weight::Semibold,
                                ),
                            ],
                        ),
                        self.pill_badge(cal_events.to_string(), p.indigo, p.indigo_soft),
                        S1,
                    ),
                    self.wrap(vec![self.productivity_button(
                        "productivity-empty-meetings",
                        "准备会议",
                        UiIcon::Calendar,
                        ProductivityAction::Tab(3),
                        Btn::Secondary,
                    )]),
                ],
            ),
            p.card,
            p.border,
            R_SM,
            12.,
            10.,
        );

        let (bento_cards, cols) = match mod_w {
            HomeModuleWidth::OneThird => {
                let mut prioritized = vec![
                    (if pending > 0 { 0usize } else { 2 }, 0usize, materials_card),
                    (
                        if candidates > 0 || !active_actions.is_empty() {
                            0
                        } else {
                            2
                        },
                        1,
                        actions_card,
                    ),
                    (if cal_events > 0 { 1 } else { 3 }, 2, meetings_card),
                ];
                prioritized.sort_by_key(|(prio, order, _)| (*prio, *order));
                (
                    prioritized.into_iter().take(2).map(|(_, _, c)| c).collect(),
                    1,
                )
            }
            HomeModuleWidth::OneHalf => (vec![materials_card, actions_card], 2),
            HomeModuleWidth::TwoThirds | HomeModuleWidth::Full => (
                vec![materials_card, actions_card, meetings_card],
                3,
            ),
        };
        rows.push(self.grid_cols(bento_cards, cols, S2));

        // Tall density: unlock Inline Live Workbench Split Preview below bento cards
        if effective_h == HomeModuleHeight::Tall {
            let mut mat_preview_rows = vec![row_between_expanded(
                self.text_w(
                    "待整理资料",
                    CAPTION,
                    p.text,
                    facloud_ui::fonts::Weight::Semibold,
                ),
                self.pill_badge(format!("待处理 {pending}"), p.accent, p.accent_soft),
                S1,
            )];
            let top_mats: Vec<_> = self
                .productivity
                .materials
                .as_ref()
                .map(|s| {
                    let mut mats: Vec<_> = s
                        .items
                        .iter()
                        .filter(|i| i.state != inbox::MaterialState::Ignored)
                        .collect();
                    mats.sort_by_key(|i| match i.state {
                        inbox::MaterialState::Discovered | inbox::MaterialState::Failed => 0,
                        inbox::MaterialState::Processing => 1,
                        inbox::MaterialState::Ready => 2,
                        inbox::MaterialState::Ignored => 3,
                    });
                    mats.into_iter().take(2).collect()
                })
                .unwrap_or_default();
            if top_mats.is_empty() {
                mat_preview_rows.push(self.text("暂无待整理手机资料", MICRO, p.text2));
            } else {
                for (idx, item) in top_mats.into_iter().enumerate() {
                    let btn = if item.state == inbox::MaterialState::Ready {
                        self.productivity_button(
                            format!("productivity-tall-mat-draft-{idx}"),
                            "生成方案与待办",
                            UiIcon::Sparkles,
                            ProductivityAction::MaterialDraft(item.id.clone()),
                            Btn::Primary,
                        )
                    } else {
                        self.productivity_button(
                            format!("productivity-tall-mat-proc-{idx}"),
                            "读取并整理",
                            UiIcon::Sparkles,
                            ProductivityAction::MaterialProcess(item.id.clone()),
                            Btn::Primary,
                        )
                    };
                    mat_preview_rows.push(row_between_expanded(
                        self.text(
                            Self::truncate_chars(&item.title, title_budget),
                            CAPTION,
                            p.text,
                        ),
                        btn,
                        S1,
                    ));
                }
            }

            let mut act_preview_rows = vec![row_between_expanded(
                self.text_w(
                    "待办行动",
                    CAPTION,
                    p.text,
                    facloud_ui::fonts::Weight::Semibold,
                ),
                self.pill_badge(
                    format!("待核对 {candidates}"),
                    p.teal,
                    p.teal_soft,
                ),
                S1,
            )];
            let top_acts: Vec<_> = self
                .productivity
                .actions
                .as_ref()
                .map(|s| {
                    let mut acts: Vec<_> = s
                        .actions
                        .iter()
                        .filter(|a| {
                            matches!(
                                a.state,
                                ActionState::Candidate
                                    | ActionState::InProgress
                                    | ActionState::Accepted
                                    | ActionState::Deferred
                            )
                        })
                        .collect();
                    acts.sort_by_key(|a| match a.state {
                        ActionState::Candidate => 0,
                        ActionState::InProgress => 1,
                        ActionState::Accepted => 2,
                        ActionState::Deferred => 3,
                        _ => 4,
                    });
                    acts.into_iter().take(2).map(|a| (s.revision, a)).collect()
                })
                .unwrap_or_default();
            if top_acts.is_empty() {
                act_preview_rows.push(self.text("暂无待核对或推进中的行动", MICRO, p.text2));
            } else {
                for (idx, (rev, act)) in top_acts.into_iter().enumerate() {
                    let (label, target_state) = if act.state == ActionState::Candidate {
                        ("核对后采纳", ActionState::Accepted)
                    } else {
                        ("确认完成", ActionState::Completed)
                    };
                    act_preview_rows.push(row_between_expanded(
                        self.text(
                            Self::truncate_chars(&act.action, title_budget),
                            CAPTION,
                            p.text,
                        ),
                        self.productivity_button(
                            format!("productivity-tall-act-{idx}"),
                            label,
                            UiIcon::Check,
                            ProductivityAction::ActionState {
                                id: act.id.clone(),
                                revision: rev,
                                state: target_state,
                            },
                            Btn::Secondary,
                        ),
                        S1,
                    ));
                }
            }

            let split_cols = if mod_w == HomeModuleWidth::OneThird {
                1
            } else {
                2
            };
            rows.push(self.grid_cols(
                vec![
                    self.card_box(
                        self.vstack(S1, mat_preview_rows),
                        p.surface,
                        p.border,
                        R_SM,
                        10.,
                        8.,
                    ),
                    self.card_box(
                        self.vstack(S1, act_preview_rows),
                        p.surface,
                        p.border,
                        R_SM,
                        10.,
                        8.,
                    ),
                ],
                split_cols,
                S2,
            ));
        }

        self.vstack(S2, rows)
    }

    pub(super) fn render_productivity_hub(&self) -> El {
        let p = self.p();
        let mod_w = self.home_module_content_width(super::pages::home::HomeModuleId::ProductivityHub);
        let material_count = self
            .productivity
            .materials
            .as_ref()
            .map(|s| {
                s.items
                    .iter()
                    .filter(|i| i.state != inbox::MaterialState::Ignored)
                    .count()
            })
            .unwrap_or(0);
        let action_count = self
            .productivity
            .actions
            .as_ref()
            .map(|s| {
                s.actions
                    .iter()
                    .filter(|a| a.state != ActionState::Dismissed)
                    .count()
            })
            .unwrap_or(0);
        let labels: [String; 4] = [
            "概览".into(),
            format!("资料 {material_count}"),
            format!("行动 {action_count}"),
            "备会".into(),
        ];
        let mut right_controls: Vec<El> = labels
            .into_iter()
            .enumerate()
            .map(|(i, label)| {
                self.chip(
                    format!("productivity-tab-{i}"),
                    label,
                    None,
                    vec![Action::Productivity(ProductivityAction::Tab(i as u8))],
                    self.productivity.tab == i as u8,
                    false,
                )
            })
            .collect();
        if self
            .productivity
            .worker
            .as_ref()
            .is_some_and(|w| w.cancellable)
        {
            right_controls.push(self.button_icon(
                "productivity-stop",
                "停止",
                UiIcon::Close,
                Action::Productivity(ProductivityAction::Stop),
                Btn::Ghost,
            ));
        } else if self.productivity.worker.is_none() {
            right_controls.push(self.button_icon(
                "productivity-refresh",
                "更新",
                UiIcon::Refresh,
                Action::Productivity(ProductivityAction::Refresh),
                Btn::Ghost,
            ));
        }

        let is_narrow = matches!(
            mod_w,
            super::pages::home::HomeModuleWidth::OneThird
                | super::pages::home::HomeModuleWidth::OneHalf
        );
        let left_header = self.home_section_heading(UiIcon::Briefcase, "资料与行动");
        let header_el = if is_narrow {
            self.vstack(6., vec![left_header, self.bounded_wrap(right_controls)])
        } else {
            row_between_expanded(left_header, self.wrap(right_controls), S2)
        };
        let mut children = vec![header_el];
        if let Some((message, error)) = &self.productivity.feedback {
            let is_routine_bg_read = !*error
                && self.productivity.materials.is_some()
                && self.productivity.actions.is_some()
                && (message.contains("正在读取已保存的资料与行动")
                    || message.contains("正在读取手机任务状态"));
            if !is_routine_bg_read {
                children.push(self.card_box(
                    self.text(message, CAPTION, if *error { p.danger } else { p.text2 }),
                    if *error { p.danger_soft } else { p.surface },
                    p.border,
                    R_SM,
                    10.,
                    8.,
                ));
            }
        }
        if let Some(preview) = &self.productivity.preview {
            let chars: Vec<_> = preview.text.chars().collect();
            let pages = chars.len().div_ceil(4200).max(1);
            let page = preview.page.min(pages - 1);
            let text: String = chars.iter().skip(page * 4200).take(4200).collect();
            let mut controls = vec![self.productivity_button(
                "productivity-preview-close",
                "返回",
                UiIcon::ArrowLeft,
                ProductivityAction::ClosePreview,
                Btn::Secondary,
            )];
            for (n, matter) in preview.matters.iter().take(8).enumerate() {
                if let Some(item) = self.context.overview.items.iter().find(|i| &i.id == matter) {
                    controls.push(self.productivity_button(
                        format!("productivity-preview-matter-{n}"),
                        &format!("打开讨论：{}", short(&item.title, 24)),
                        UiIcon::MessageCircle,
                        ProductivityAction::OpenMatter(matter.clone()),
                        Btn::Ghost,
                    ));
                }
            }
            if preview.path.is_some() {
                controls.push(self.productivity_button(
                    "productivity-preview-send",
                    "带回手机",
                    UiIcon::Smartphone,
                    ProductivityAction::SendPreview,
                    Btn::Primary,
                ));
            }
            if page > 0 {
                controls.push(self.productivity_button(
                    "productivity-preview-prev",
                    "上一页",
                    UiIcon::ArrowLeft,
                    ProductivityAction::PreviewPage(page - 1),
                    Btn::Ghost,
                ));
            }
            if page + 1 < pages {
                controls.push(self.productivity_button(
                    "productivity-preview-next",
                    "下一页",
                    UiIcon::ArrowUpRight,
                    ProductivityAction::PreviewPage(page + 1),
                    Btn::Ghost,
                ));
            }
            children.extend([
                self.text_w(
                    &preview.title,
                    SUBHEAD,
                    p.text,
                    facloud_ui::fonts::Weight::Semibold,
                ),
                self.wrap(controls),
                self.text(format!("第 {} / {pages} 页", page + 1), MICRO, p.text3),
                self.card_box(
                    self.render_context_preview_text(
                        &super::context_state::context_preview_plain_text(&text),
                    ),
                    p.surface,
                    p.border,
                    R_SM,
                    16.,
                    16.,
                ),
            ]);
        } else {
            children.push(match self.productivity.tab {
                1 => self.render_productivity_materials(),
                2 => self.render_productivity_actions(),
                3 => self.render_productivity_meetings(),
                _ => self.render_productivity_overview(),
            });
        }
        let element = self.panel(children);
        #[cfg(feature = "ui-capture")]
        let element = super::scenario_check::control("productivity-hub".into(), element);
        element
    }
    pub(super) fn productivity_health_status(&self) -> Option<ProductivityHealthStatus<'_>> {
        let context_problem = self.context.productivity_problem();
        let has_retained_content = self.productivity.materials.is_some()
            || self.productivity.actions.is_some();
        let domain_errors = super::auto_sync::AutoSyncDomain::ALL.map(|domain| {
            self.auto_sync.domain_status(domain).error().filter(|error| {
                !(has_retained_content && error.contains("已保留上次内容"))
            })
        });
        let missing_account = self.selected.is_some() && self.amoo_ui.account_scope.is_none();
        let has_domain_error = domain_errors.iter().any(Option::is_some);
        if context_problem.is_none() && !has_domain_error && !missing_account {
            return None;
        }
        Some(ProductivityHealthStatus {
            context_problem,
            domain_errors,
            missing_account,
            blocked: (context_problem.is_some() && !self.context.has_only_coverage_warning())
                || has_domain_error || missing_account,
        })
    }

    pub(super) fn render_productivity_health(&self) -> Option<El> {
        let status = self.productivity_health_status()?;
        let p = self.p();
        let mut messages = vec![];
        if let Some(problem) = status.context_problem {
            messages.push(format!("讨论与成果：{}", short(problem, 220)));
        }
        for (domain, error) in super::auto_sync::AutoSyncDomain::ALL.into_iter().zip(status.domain_errors) {
            if let Some(error) = error {
                messages.push(format!("{}：{}", domain.short_label(), short(error, 140)));
            }
        }
        if status.missing_account {
            messages.push("手机已连接，Amoo账号尚未确认；账号资料与回传暂不可用。".into());
        }
        let mut copy = vec![self.text_w(
            if status.blocked { "资料与成果更新受阻" } else { "资料已更新，部分来源受限" },
            BODY,
            p.text,
            facloud_ui::fonts::Weight::Semibold,
        )];
        copy.extend(
            messages
                .into_iter()
                .take(4)
                .map(|s| self.text(s, CAPTION, p.text2)),
        );
        copy.push(self.wrap(vec![
            self.button_icon(
                "productivity-health-retry",
                "重试资料更新",
                UiIcon::Refresh,
                Action::Productivity(ProductivityAction::RetrySources),
                Btn::Secondary,
            ),
            self.button_icon(
                "productivity-health-sources",
                "查看来源覆盖",
                UiIcon::File,
                Action::Context(ContextAction::Coverage),
                Btn::Ghost,
            ),
        ]));
        Some(self.card_box(
            self.vstack(S2, copy),
            p.warning_soft,
            p.border,
            R_SM,
            12.,
            12.,
        ))
    }
    pub(super) fn productivity_preservation_status(&self) -> Option<ProductivityPreservationStatus<'_>> {
        let snapshot = &self.expansion.snapshot;
        if snapshot.error {
            return Some(ProductivityPreservationStatus::Error(&snapshot.phase));
        }
        let status = &snapshot.status;
        let free = status["availableBytes"].as_u64().or_else(|| {
            status["storage"]["availableBytes"]
                .as_u64()
                .or_else(|| status["storage"]["freeBytes"].as_u64())
        });
        let failed = snapshot
            .files
            .iter()
            .filter(|f| matches!(f["state"].as_str(), Some("failed" | "error")))
            .count();
        if failed > 0 {
            return Some(ProductivityPreservationStatus::Failed(failed));
        }
        free.filter(|n| *n < 5 * 1024 * 1024 * 1024)
            .map(|_| ProductivityPreservationStatus::LowStorage)
    }

    pub(super) fn render_productivity_preservation(&self) -> Option<El> {
        let status = self.productivity_preservation_status()?;
        let p = self.p();
        let message = match status {
            ProductivityPreservationStatus::Error(phase) => short(phase, 200),
            ProductivityPreservationStatus::Failed(failed) => format!("{failed}份资料归档未完成，原件状态需要核对。"),
            ProductivityPreservationStatus::LowStorage => "手机可用空间偏少，可以预览可归档资料并确认保存位置。".into(),
        };
        Some(self.card_box(
            self.vstack(
                S2,
                vec![
                    self.text_w(
                        "先保全资料，再释放空间",
                        BODY,
                        p.text,
                        facloud_ui::fonts::Weight::Semibold,
                    ),
                    self.text(message, CAPTION, p.text2),
                    self.button_icon(
                        "productivity-preservation-open",
                        "检查归档与原件",
                        UiIcon::Folder,
                        Action::Page(Page::Expansion),
                        Btn::Secondary,
                    ),
                ],
            ),
            p.surface,
            p.border,
            R_SM,
            12.,
            12.,
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn productivity_health_notice_and_card_share_retained_content_filter() {
        use super::super::auto_sync::AutoSyncDomainStatus;
        let mut app = Companion::new_for_test();
        app.selected = None;
        app.auto_sync.domains = std::array::from_fn(|_| AutoSyncDomainStatus::Pending);
        app.auto_sync.domains[0] = AutoSyncDomainStatus::Failed {
            error: "读取超时，已保留上次内容".into(), at_ms: 0,
        };
        app.productivity.materials = None;
        app.productivity.actions = None;
        assert!(app.productivity_health_status().unwrap().domain_errors[0].is_some());
        assert!(app.render_productivity_health().is_some());

        app.productivity.materials = Some(inbox::Snapshot {
            items: vec![], updated_ms: 0, note: String::new(),
        });
        assert!(app.productivity_health_status().is_none_or(|status| status.domain_errors[0].is_none()));
        assert_eq!(app.productivity_health_status().is_some(), app.render_productivity_health().is_some());

        app.auto_sync.domains[0] = AutoSyncDomainStatus::Failed {
            error: "手机连接不可用".into(), at_ms: 0,
        };
        assert!(app.productivity_health_status().unwrap().blocked);
        assert!(app.render_productivity_health().is_some());
    }

    #[test]
    fn productivity_preservation_notice_keeps_error_failure_and_storage_precedence() {
        use ProductivityPreservationStatus::*;
        let mut app = Companion::new_for_test();
        app.expansion.snapshot.error = false;
        app.expansion.snapshot.files.clear();
        app.expansion.snapshot.status = serde_json::json!({});
        assert!(app.productivity_preservation_status().is_none());
        assert!(app.render_productivity_preservation().is_none());

        app.expansion.snapshot.status = serde_json::json!({"storage": {"freeBytes": 1}});
        assert_eq!(app.productivity_preservation_status(), Some(LowStorage));
        app.expansion.snapshot.files = vec![serde_json::json!({"state": "failed"}), serde_json::json!({"state": "error"})];
        assert_eq!(app.productivity_preservation_status(), Some(Failed(2)));
        app.expansion.snapshot.error = true;
        app.expansion.snapshot.phase = "归档连接已中断".into();
        assert_eq!(app.productivity_preservation_status(), Some(Error("归档连接已中断")));
        assert!(app.render_productivity_preservation().is_some());

        app.expansion.snapshot.error = false;
        app.expansion.snapshot.files.clear();
        app.expansion.snapshot.status = serde_json::json!({"availableBytes": 5_u64 * 1024 * 1024 * 1024});
        assert!(app.productivity_preservation_status().is_none());
        assert!(app.render_productivity_preservation().is_none());
    }

    #[test]
    fn overview_counts_every_active_action_without_counting_completed_or_candidate_items() {
        let states = [ActionState::Accepted, ActionState::InProgress, ActionState::Deferred,
            ActionState::InProgress, ActionState::Accepted, ActionState::Completed,
            ActionState::Dismissed, ActionState::Candidate];
        let snapshot = action_ledger::Snapshot {
            schema: 1, scope_id: "home-overview".into(), revision: 1, progress_revision: 1,
            updated_ms: 1,
            actions: states.into_iter().enumerate().map(|(index, state)| action_ledger::ActionRecord {
                id: format!("action-{index}"), matter_ids: vec![], action: "推进项目".into(),
                kind: "committed".into(), owner: None, due: None, evidence: vec![],
                source_revision: "1".into(), first_seen_ms: 1, last_seen_ms: 1, state,
                needs_review: false, history: vec![], origins: vec![], source_history: vec![],
                source_check_revision: None,
            }).collect(),
        };
        let active = active_productivity_actions(Some(&snapshot));
        assert_eq!(active.len(), 5);
        assert_eq!(active.first().unwrap().id, "action-0");
        assert_eq!(active.iter().take(2).count(), 2);
        assert!(active_productivity_actions(None).is_empty());
    }

    #[test]
    fn scope_switch_drops_private_previews_and_cancels_content_processing() {
        let mut ui = ProductivityUi::default();
        ui.change_scope(Some(Scope {
            device: "one".into(),
            account: Some("a".into()),
        }));
        let cancel = Arc::new(AtomicBool::new(false));
        let (_, rx) = mpsc::channel();
        ui.worker = Some(Worker {
            generation: ui.generation,
            receiver: rx,
            cancel: cancel.clone(),
            cancellable: true,
        });
        ui.preview = Some(Preview {
            title: "private".into(),
            text: "one".into(),
            path: None,
            page: 0,
            matters: vec![],
        });
        assert!(ui.change_scope(Some(Scope {
            device: "one".into(),
            account: Some("b".into())
        })));
        assert!(cancel.load(Ordering::Relaxed));
        assert!(ui.preview.is_none() && ui.worker.is_some() && ui.materials.is_none());
    }
    #[test]
    fn navigation_tabs_stay_interactive_while_background_worker_runs() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.init_demo_push();
        let cancel = Arc::new(AtomicBool::new(false));
        let (_tx, rx) = mpsc::channel();
        app.productivity.worker = Some(Worker {
            generation: app.productivity.generation,
            receiver: rx,
            cancel,
            cancellable: true,
        });
        for tab in 0..=3u8 {
            app.productivity.tab = tab;
            let _ = std::hint::black_box(app.render_productivity_hub());
        }
    }
    #[test]
    fn period_ranges_use_local_calendar_boundaries() {
        let now = Local
            .with_ymd_and_hms(2026, 11, 2, 12, 0, 0)
            .single()
            .unwrap();
        let (start, end) = progress_bounds(Period::Week, now);
        assert_eq!(
            Local
                .timestamp_millis_opt(start)
                .single()
                .unwrap()
                .weekday(),
            chrono::Weekday::Mon
        );
        assert_eq!(
            Local
                .timestamp_millis_opt(end)
                .single()
                .unwrap()
                .date_naive(),
            chrono::NaiveDate::from_ymd_opt(2026, 11, 9).unwrap()
        );
        let (_, month_end) = progress_bounds(Period::Month, now);
        assert_eq!(
            Local
                .timestamp_millis_opt(month_end)
                .single()
                .unwrap()
                .day(),
            1
        );
    }

    #[test]
    fn productivity_hub_adapts_budgets_and_renders_across_4x4_width_height_matrix() {
        use super::super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.init_demo_push();
        app.page = Page::Overview;

        // 1. Auto keeps a stable viewport budget regardless of arriving data.
        app.home_modules
            .set_module_height(HomeModuleId::ProductivityHub, HomeModuleHeight::Auto);
        app.productivity.materials = None;
        app.productivity.actions = None;
        app.back_screen.stepos_suite.calendar_events.clear();
        assert_eq!(
            app.productivity_effective_height(),
            HomeModuleHeight::Standard
        );

        app.productivity.materials = Some(inbox::Snapshot {
            note: "收到手机截图与录音".into(),
            updated_ms: 1,
            items: (0..2)
                .map(|i| inbox::MaterialItem {
                    id: format!("mat-{i}"),
                    title: format!("第{i}份跨端技术评审长标题资料文件用于测试宽度截断预算"),
                    summary: "包含关键架构决议、负责人分工以及上线节点安排的详细正文摘要".into(),
                    kind: inbox::MaterialKind::Document,
                    state: inbox::MaterialState::Discovered,
                    source_label: "手机伴侣".into(),
                    content_method: "全文读取".into(),
                    ..Default::default()
                })
                .collect(),
        });
        assert_eq!(
            app.productivity_effective_height(),
            HomeModuleHeight::Standard
        );

        app.productivity.actions = Some(action_ledger::Snapshot {
            schema: 1,
            scope_id: "demo".into(),
            revision: 1,
            progress_revision: 1,
            updated_ms: 1,
            actions: (0..4)
                .map(|i| action_ledger::ActionRecord {
                    id: format!("act-{i}"),
                    matter_ids: vec![],
                    action: format!("推进第{i}项跨端协同核心待办并完成联调核验与回归测试"),
                    state: if i % 2 == 0 {
                        ActionState::Candidate
                    } else {
                        ActionState::InProgress
                    },
                    kind: "committed".into(),
                    owner: None,
                    due: None,
                    evidence: vec![],
                    source_revision: "v1".into(),
                    first_seen_ms: 1,
                    last_seen_ms: 1,
                    needs_review: false,
                    history: vec![],
                    origins: vec![],
                    source_history: vec![],
                    source_check_revision: None,
                })
                .collect(),
        });
        assert_eq!(app.productivity_effective_height(), HomeModuleHeight::Standard);

        // 2. Verify distinct value tier specs, per_page, and character budgets across (OneThird, Compact), (OneHalf, Standard), (TwoThirds, Standard), (Full, Tall)
        let mut tier_specs = Vec::new();
        for (w, h, expected_spec) in [
            (
                HomeModuleWidth::OneThird,
                HomeModuleHeight::Compact,
                (1, 1, 1, 18, true, false),
            ),
            (
                HomeModuleWidth::OneHalf,
                HomeModuleHeight::Standard,
                (2, 1, 3, 28, false, false),
            ),
            (
                HomeModuleWidth::TwoThirds,
                HomeModuleHeight::Standard,
                (3, 2, 4, 40, false, false),
            ),
            (
                HomeModuleWidth::Full,
                HomeModuleHeight::Tall,
                (3, 2, 8, 58, false, true),
            ),
        ] {
            app.home_modules
                .set_module_width(HomeModuleId::ProductivityHub, w);
            app.home_modules
                .set_module_height(HomeModuleId::ProductivityHub, h);
            let spec = app.productivity_value_tier_spec();
            assert_eq!(spec, expected_spec);
            assert!(!tier_specs.contains(&spec));
            tier_specs.push(spec);
        }

        // 3. Render all 4 widths x 4 heights across all 4 tabs without panicking
        for w in HomeModuleWidth::ALL {
            for h in HomeModuleHeight::ALL {
                app.home_modules
                    .set_module_width(HomeModuleId::ProductivityHub, w);
                app.home_modules
                    .set_module_height(HomeModuleId::ProductivityHub, h);
                for tab in 0..=3u8 {
                    app.productivity.tab = tab;
                    let _ = std::hint::black_box(app.render_productivity_hub());
                }
            }
        }
    }
}
