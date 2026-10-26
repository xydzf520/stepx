//! One visible task, composer and result collection; existing executors remain internal.
use super::*;
use anyhow::Context;
use facloud_ui::TypedActionView;
use stepx_companion::amoo_workspace::{
    self as model, Executor, Intent, Journal, Output, Phase, Step, Task, Turn,
};

#[derive(Clone, Debug)]
pub(super) enum WorkspaceAction {
    New,
    History,
    Select(String),
    Delete(String),
    PhoneSession(usize),
    Suggest(u8),
    Prepare,
    Confirm,
    EditPlan,
    Intent(Intent),
    Menu(u8),
    ClosePanel,
    ChooseFolder,
    Preview(usize),
    Retry,
    Stop,
    Deliver(bool),
    ConfirmDelivery,
    CancelDelivery,
    ToggleOpenPhone,
    Settings,
    AddOutput(usize),
    OpenProject(usize),
    SelectLab(String),
    SelectPersona(usize),
    SelectAgentRole(usize),
    ToggleOpenClawSkill(usize),
    UseOpenClawSkill(usize),
    UseMemory(String),
    ToggleMemory,
    MemoryCategory(u8),
    AddMemory,
    DeleteMemory(usize),
    SendScheduleToAmoo(String),
    McpPrompt(u8),
    SubView(u8),
    SyncTask,
    DismissUnconfirmed,
    DismissError,
    QuoteTurn(String),
    PreviewZoom(u16),
    QuickScenario(u8),
    OpenInWork(Option<usize>),
    OpenInStudio(Option<usize>),
    PlayAppOutput(usize),
    PushAppOutputToPhone(usize),
    StudioPreviewArtifact(usize),
    StudioDeliverArtifact(usize),
    OpenOutput(usize),
    SelectOutput(usize),
    SendOutputToPhone(usize),
    FlowApp,
    /// SHARED CONTRACT: re-run the latest request of the current task with a
    /// forced route (e.g. `Reroute(Intent::Chat)` = "改为普通聊天").
    Reroute(Intent),
    /// SHARED CONTRACT: keep iterating on output `i` inside the conversation
    /// (no page switch): targets that artifact and focuses the composer.
    ContinueEdit(usize),
    /// SHARED CONTRACT: reveal output `i` in the system file manager.
    RevealOutput(usize),
    CopyTaskTranscript(String),
    SendTaskOutputToPhone(String, usize),
}
#[derive(Clone)]
pub(super) struct Proposal {
    pub text: String,
    pub files: Vec<PathBuf>,
    pub steps: Vec<Executor>,
}
#[derive(Clone)]
pub(super) struct ActiveRun {
    pub task: String,
    pub step: usize,
    pub executor: Executor,
    pub accepted: bool,
    pub started_at: Instant,
}
struct ImageDelivery {
    task: String,
    scope: Option<String>,
    note: String,
    receiver: mpsc::Receiver<Result<Vec<(String, PathBuf)>, String>>,
}
pub(super) fn is_verification_harness() -> bool {
    std::env::args().any(|a| a.starts_with("--verify-"))
        || [
            "STEPX_SCENARIO_DIR",
            "STEPX_CHAT_FIRST_TEST",
            "STEPX_RESPONSIVE_TEST",
            "STEPX_ENTER_TEST",
        ]
        .into_iter()
        .any(|k| std::env::var_os(k).is_some())
}

pub(super) struct WorkspaceUi {
    pub journal: Journal,
    pub active: Option<String>,
    pub left_rail_open: bool,
    pub history: bool,
    pub panel: u8,
    pub sub_view: u8,
    pub memory_category: u8,
    pub viewport: Vector2F,
    pub intent: Intent,
    pub proposal: Option<Proposal>,
    pub running: Option<ActiveRun>,
    pub next: bool,
    image_delivery: Option<ImageDelivery>,
    pub selected_output: Option<usize>,
    pub delivery: Option<bool>,
    delivery_target: Option<(String, PathBuf, Option<String>, Option<String>)>,
    pub open_phone: bool,
    pub error: Option<String>,
    pub stream_tool_status: Option<String>,
    pub stream_delta_count: usize,
    pub reconciled_sessions: std::collections::HashSet<String>,
    pub stopped_sessions: std::collections::HashSet<String>,
    pub deleted_sessions: std::collections::HashSet<String>,
    pub preview_scroll: ClippedScrollStateHandle,
    pub preview_zoom: u16,
    pub panel_entered: Instant,
    writer: Option<mpsc::Sender<Journal>>,
    writer_thread: Option<std::thread::JoinHandle<()>>,
    last_saved: Instant,
    last_draft: (String, Vec<PathBuf>),
}
fn journal_eq(a: &Journal, b: &Journal) -> bool {
    a.deleted_sessions == b.deleted_sessions
        && a.tasks.len() == b.tasks.len()
        && a.tasks.iter().zip(&b.tasks).all(|(x, y)| {
            x.id == y.id
                && x.title == y.title
                && x.updated == y.updated
                && x.session == y.session
                && x.workspace == y.workspace
                && x.device == y.device
                && x.account == y.account
                && x.draft == y.draft
                && x.attachments == y.attachments
                && x.submitted_files == y.submitted_files
                && x.intent == y.intent
                && x.route_note == y.route_note
                && x.route == y.route
                && x.note == y.note
                && x.turns.len() == y.turns.len()
                && x.turns
                    .iter()
                    .zip(&y.turns)
                    .all(|(tx, ty)| tx.user == ty.user && tx.text == ty.text)
                && x.steps.len() == y.steps.len()
                && x.steps.iter().zip(&y.steps).all(|(sx, sy)| {
                    sx.executor == sy.executor && sx.phase == sy.phase && sx.note == sy.note
                })
                && x.outputs.len() == y.outputs.len()
                && x.outputs.iter().zip(&y.outputs).all(|(ox, oy)| {
                    ox.path == oy.path
                        && ox.name == oy.name
                        && ox.kind == oy.kind
                        && ox.lab_id == oy.lab_id
                })
                && x.receipts.len() == y.receipts.len()
                && x.receipts.iter().zip(&y.receipts).all(|(rx, ry)| {
                    rx.path == ry.path
                        && rx.note == ry.note
                        && rx.success == ry.success
                        && rx.device == ry.device
                        && rx.account == ry.account
                        && rx.cloud_url == ry.cloud_url
                })
        })
}

impl Default for WorkspaceUi {
    fn default() -> Self {
        let path = stepx_companion::resources::user_state().join("amoo-workspace/tasks.json");
        let demo = cfg!(test) || std::env::args().any(|a| a == "--demo");
        let (journal, error) = match if demo {
            Ok(Journal::default())
        } else {
            Journal::load(&path)
        } {
            Ok(v) => (v, None),
            Err(e) => (Journal::default(), Some(e.to_string())),
        };
        let mut writer_thread = None;
        let writer = if error.is_none() && !demo {
            let (tx, rx) = mpsc::channel::<Journal>();
            let initial_saved = journal.clone();
            writer_thread = Some(std::thread::spawn(move || {
                let mut last_saved = Some(initial_saved);
                while let Ok(mut journal) = rx.recv() {
                    while let Ok(newer) = rx.recv_timeout(Duration::from_millis(300)) {
                        journal = newer;
                    }
                    if last_saved
                        .as_ref()
                        .is_some_and(|prev| journal_eq(prev, &journal))
                    {
                        continue;
                    }
                    if journal.save(&path).is_ok() {
                        last_saved = Some(journal);
                    }
                }
            }));
            Some(tx)
        } else {
            None
        };
        let initial_panel = std::env::args()
            .position(|a| a == "--amoo-panel")
            .and_then(|i| std::env::args().nth(i + 1))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(0);
        let initial_subview = std::env::args()
            .position(|a| a == "--amoo-subview")
            .and_then(|i| std::env::args().nth(i + 1))
            .and_then(|s| s.parse::<u8>().ok())
            .unwrap_or(0);
        let deleted_sessions = journal.deleted_sessions.iter().cloned().collect();
        Self {
            journal,
            active: None,
            left_rail_open: !is_verification_harness(),
            history: false,
            panel: initial_panel,
            sub_view: initial_subview,
            memory_category: 0,
            viewport: Vector2F::new(1320., 860.),
            intent: Intent::Auto,
            proposal: None,
            running: None,
            next: false,
            image_delivery: None,
            selected_output: None,
            delivery: None,
            delivery_target: None,
            open_phone: false,
            error,
            stream_tool_status: None,
            stream_delta_count: 0,
            reconciled_sessions: Default::default(),
            stopped_sessions: Default::default(),
            deleted_sessions,
            preview_scroll: Default::default(),
            preview_zoom: 100,
            panel_entered: Instant::now(),
            writer,
            writer_thread,
            last_saved: Instant::now(),
            last_draft: Default::default(),
        }
    }
}
pub(super) fn quick_scenario_prompt(id: u8) -> &'static str {
    match id {
        0 => "帮我做一个可在手机运行的 2048 益智小游戏，带计分板、最高分记录和触控方向键",
        1 => "帮我做一个适合手机使用的番茄专注钟小应用，支持开始、暂停、重置与专注统计",
        2 => {
            "请制作一份《季度产品研发与业务进展汇报》PPT 演示文稿，包含核心指标、里程碑与下阶段计划"
        }
        3 => "请撰写一份《智能终端用户体验优化方案》Word 报告，包含背景分析、核心举措与排期",
        4 => "请生成一份《项目排期与预算明细表》Excel 表格，包含模块、负责人、预算与进度汇总公式",
        _ => "生成一个赛博朋克风格的笑脸像素点阵图案",
    }
}

pub(super) fn is_quick_scenario_template(text: &str) -> bool {
    let trimmed = text.trim();
    !trimmed.is_empty() && (0..=5).any(|id| trimmed == quick_scenario_prompt(id).trim())
}

impl Companion {
    pub(super) fn workspace_task(&self) -> Option<&Task> {
        let id = self.workspace.active.as_ref()?;
        self.workspace.journal.tasks.iter().find(|t| &t.id == id)
    }
    fn workspace_task_mut(&mut self) -> Option<&mut Task> {
        let id = self.workspace.active.as_ref()?;
        self.workspace
            .journal
            .tasks
            .iter_mut()
            .find(|t| &t.id == id)
    }
    pub(super) fn workspace_visible(&self, t: &Task) -> bool {
        if self.selected.is_none() {
            return true;
        }
        t.visible(
            self.selected.as_ref().map(|d| d.serial.as_str()),
            self.amoo_ui.account_scope.as_deref(),
        )
    }
    pub(super) fn workspace_busy(&self) -> bool {
        self.workspace.running.is_some()
            || self.work.running
            || self.studio_lab.handle.is_some()
            || self.amoo_ui.gui_running
    }
    pub(super) fn workspace_active_busy(&self) -> bool {
        let active_running = self
            .workspace
            .running
            .as_ref()
            .is_some_and(|r| self.workspace.active.as_deref() == Some(r.task.as_str()));
        let gui_busy = self.amoo_ui.screen_mode && self.amoo_ui.gui_running;
        let session_pending = if let Some(t) = self.workspace_task() {
            t.session
                .as_ref()
                .is_some_and(|key| self.amoo_pending_reply.contains_key(key))
        } else if self.amoo_session_idx != usize::MAX {
            self.amoo_sessions
                .get(self.amoo_session_idx)
                .is_some_and(|s| self.amoo_pending_reply.contains_key(&s.session_key))
        } else {
            false
        };
        active_running || gui_busy || session_pending
    }
    pub(super) fn workspace_can_send(&self) -> bool {
        !self.workspace_active_busy()
            && self.workspace.proposal.is_none()
            && self.fields[F_AMOO].preedit.is_empty()
            && (!self.fields[F_AMOO].text.trim().is_empty() || !self.amoo_attachments.is_empty())
    }
    pub(super) fn workspace_flush(&mut self) {
        self.workspace_capture_draft();
        self.workspace_persist();
        self.workspace.writer.take();
        if let Some(thread) = self.workspace.writer_thread.take() {
            let _ = thread.join();
        }
    }
    pub(super) fn workspace_persist(&self) {
        if !self.demo_mode {
            if let Some(tx) = &self.workspace.writer {
                let mut journal = self.workspace.journal.clone();
                let mut deleted: Vec<String> =
                    self.workspace.deleted_sessions.iter().cloned().collect();
                deleted.sort();
                journal.deleted_sessions = deleted;
                let _ = tx.send(journal);
            }
        }
    }
    fn workspace_ensure_task(&mut self) {
        if self.workspace.active.is_none() {
            let task = Task::new(None, None);
            self.workspace.active = Some(task.id.clone());
            self.workspace.journal.tasks.push(task);
        }
    }
    fn workspace_capture_draft(&mut self) {
        let draft = self.fields[F_AMOO].text.clone();
        let paths = self
            .amoo_attachments
            .iter()
            .filter_map(|a| a.local_path.clone())
            .collect::<Vec<_>>();
        self.workspace.last_draft = (draft.clone(), paths.clone());
        if self.workspace.active.is_none() {
            if draft.trim().is_empty() && paths.is_empty() {
                return;
            }
            // A new conversation is lazy until it has actual input. Persist
            // that input before history/window switches, even before sending.
            self.workspace_ensure_task();
        }
        let intent = self.workspace.intent;
        if let Some(task) = self.workspace_task_mut() {
            if task.turns.is_empty() && task.session.is_none() {
                task.title = if draft.trim().is_empty() {
                    "新对话".into()
                } else {
                    model::clean_task_title(&draft)
                };
            }
            task.draft = draft;
            task.attachments = paths;
            task.intent = intent;
        }
    }
    pub(super) fn workspace_skill_imported(&mut self, root: &PathBuf) {
        self.workspace_ensure_task();
        if let Some(t) = self.workspace_task_mut() {
            t.workspace = Some(root.clone());
            t.route = Some(Intent::Files);
        }
        self.workspace.intent = Intent::Auto;
        self.workspace.panel = 0;
        self.workspace_capture_draft();
        self.workspace_persist();
    }
    pub(super) fn workspace_bind_session(&mut self, key: &str) {
        self.workspace.stopped_sessions.remove(key);
        if let Some(run) = &self.workspace.running {
            let id = run.task.clone();
            if let Some(task) = self.workspace.journal.tasks.iter_mut().find(|t| t.id == id) {
                task.session = Some(key.into());
            }
            if let Some(run) = &mut self.workspace.running {
                run.accepted = true;
            }
        }
        if let Some(idx) = self.amoo_sessions.iter().position(|s| s.session_key == key) {
            if self.workspace_task().and_then(|t| t.session.as_deref()) == Some(key) {
                self.amoo_session_idx = idx;
            }
        }
    }
    pub(super) fn workspace_restore(&mut self) {
        if let Some(task) = self.workspace_task().cloned() {
            self.fields[F_AMOO].set(&task.draft);
            self.workspace.intent = task.intent;
            self.amoo_attachments = task
                .attachments
                .iter()
                .map(|p| AmooAttachment {
                    name: p
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    size_label: std::fs::metadata(p)
                        .map(|m| storage::format_bytes(m.len()))
                        .unwrap_or("文件已移动，请重新添加".into()),
                    is_image: p.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                        ["png", "jpg", "jpeg", "webp", "gif"].contains(&e.to_lowercase().as_str())
                    }),
                    local_path: Some(p.clone()),
                })
                .collect();
            self.amoo_session_idx = task
                .session
                .as_ref()
                .and_then(|key| {
                    self.amoo_sessions
                        .iter()
                        .position(|s| &s.session_key == key)
                })
                .unwrap_or(usize::MAX);
            self.work.workspace = task.workspace.clone();
            self.work.local_preview = None;
            self.work.local_preview_loading = None;
            self.work.local_preview_error = None;
            self.work.remote_preview_url = None;
        } else {
            self.fields[F_AMOO].clear();
            self.amoo_attachments.clear();
            self.workspace.intent = Intent::Auto;
            self.amoo_session_idx = usize::MAX;
        }
        self.workspace.proposal = None;
        self.workspace.selected_output = None;
        self.workspace.delivery = None;
        self.workspace.panel = 0;
        self.workspace.history = false;
        // Keep `self.workspace.left_rail_open` unchanged when switching sessions
        // so selecting a conversation in the persistent left sidebar never closes it.
        self.clear_text_interaction();
        self.reveal_latest_chat();
        self.load_work_previews();
    }
    pub(super) fn handle_workspace(
        &mut self,
        action: &WorkspaceAction,
        ctx: &mut ViewContext<Self>,
    ) {
        if matches!(
            action,
            WorkspaceAction::Select(_)
                | WorkspaceAction::Delete(_)
                | WorkspaceAction::PhoneSession(_)
                | WorkspaceAction::Menu(_)
                | WorkspaceAction::ClosePanel
                | WorkspaceAction::History
                | WorkspaceAction::New
                | WorkspaceAction::QuickScenario(_)
                | WorkspaceAction::MemoryCategory(_)
                | WorkspaceAction::SubView(_)
                | WorkspaceAction::Settings
                | WorkspaceAction::Preview(_)
                | WorkspaceAction::SelectLab(_)
                | WorkspaceAction::OpenInWork(_)
                | WorkspaceAction::OpenInStudio(_)
                | WorkspaceAction::StudioPreviewArtifact(_)
                | WorkspaceAction::StudioDeliverArtifact(_)
        ) {
            self.trigger_subtab_switch();
        }
        if let Err(error) = self.do_workspace(action, ctx) {
            self.workspace.error = Some(format!("{error:#}"));
            self.status = format!("{error:#}");
            self.error = true;
        }
        self.workspace_capture_draft();
        self.workspace_persist();
        ctx.notify();
    }
    fn do_workspace(
        &mut self,
        action: &WorkspaceAction,
        ctx: &mut ViewContext<Self>,
    ) -> Result<()> {
        use WorkspaceAction as A;
        self.workspace.error = None;
        if matches!(action, A::Menu(_) | A::Preview(_) | A::Settings) {
            self.workspace.panel_entered = Instant::now();
        }
        match action {
            A::History => {
                self.workspace_toggle_history();
            }
            A::ClosePanel => {
                self.workspace.panel = 0;
                self.workspace.delivery = None;
                self.workspace.history = false;
            }
            A::New => {
                self.workspace_switch_new()?;
                self.focus_field(F_AMOO, ctx);
            }
            A::Delete(id) => {
                if let Some(key) = id.strip_prefix("phone:") {
                    let key = key.trim().to_string();
                    if !key.is_empty() {
                        let was_selected_session = self
                            .amoo_sessions
                            .get(self.amoo_session_idx)
                            .is_some_and(|s| s.session_key == key);
                        let was_active_task =
                            self.workspace_task().and_then(|t| t.session.as_deref())
                                == Some(key.as_str());
                        self.workspace.deleted_sessions.insert(key.clone());
                        self.amoo_pending_reply.remove(&key);
                        self.workspace.reconciled_sessions.remove(&key);
                        self.workspace
                            .journal
                            .tasks
                            .retain(|t| t.session.as_deref() != Some(key.as_str()));
                        self.amoo_sessions.retain(|s| s.session_key != key);
                        if was_selected_session || was_active_task {
                            self.amoo_session_idx = usize::MAX;
                            self.workspace.active = None;
                            self.workspace.selected_output = None;
                            self.workspace_restore();
                        }
                    }
                } else {
                    for t in self.workspace.journal.tasks.iter().filter(|t| &t.id == id) {
                        if let Some(ref key) = t.session {
                            self.workspace.deleted_sessions.insert(key.clone());
                            self.amoo_pending_reply.remove(key);
                            self.workspace.reconciled_sessions.remove(key);
                            self.amoo_sessions.retain(|s| &s.session_key != key);
                        }
                    }
                    if self
                        .workspace
                        .running
                        .as_ref()
                        .is_some_and(|r| &r.task == id)
                    {
                        self.workspace.running = None;
                        self.workspace.stream_tool_status = None;
                        self.workspace.stream_delta_count = 0;
                    }
                    self.workspace.journal.tasks.retain(|t| &t.id != id);
                    if self.workspace.active.as_deref() == Some(id.as_str()) {
                        self.workspace.active = None;
                        self.workspace.selected_output = None;
                        self.workspace_restore();
                    }
                }
                let mut deleted: Vec<String> =
                    self.workspace.deleted_sessions.iter().cloned().collect();
                deleted.sort();
                self.workspace.journal.deleted_sessions = deleted;
                self.motion_started = Instant::now();
            }
            A::Select(id) => {
                if let Some(i) = self.workspace_select_task(id)? {
                    self.handle_action(&Action::AmooSelectSession(i), ctx);
                }
            }
            A::PhoneSession(i) => {
                self.workspace_select_phone_session(*i)?;
                self.handle_action(&Action::AmooSelectSession(*i), ctx);
            }
            A::Suggest(i) => {
                let prompt = match *i {
                    0 => "请帮我总结所选资料的核心结论、关键数据与待办事项：",
                    1 => "请分析这份表格数据的核心趋势、异常指标并生成汇总报告：",
                    2 => "请帮我润色并修订这份文档的结构与表述，输出修改后的完整文档：",
                    3 => "请生成一张高质感视觉海报，画面主题为：",
                    4 => "帮我制作一个交互流畅的手机小游戏或小应用，功能需求如下：",
                    5 => "请根据以下主题制作一份结构清晰的 PPT 演示文稿：",
                    6 => "帮我查看今天的手机日历日程并设置提醒：",
                    7 => "帮我在手机相册中搜索以下照片：",
                    _ => "做一个适合手机使用的专注计时器，支持开始、暂停、继续、重置。",
                };
                self.workspace.sub_view = 0;
                self.workspace.intent = Intent::Auto;
                self.fields[F_AMOO].set(prompt);
                self.workspace.proposal = None;
                self.focus_field(F_AMOO, ctx);
            }
            A::Intent(intent) => {
                self.workspace.intent = *intent;
                self.workspace.proposal = None;
            }
            A::Menu(7) => {
                self.open_workspace_intent_menu(ctx);
            }
            A::Menu(panel) => {
                if *panel == 3 && !(1..=4).contains(&self.work.tab) {
                    self.work.tab = 2;
                }
                self.workspace.panel = if self.workspace.panel == *panel {
                    0
                } else {
                    *panel
                };
                self.workspace.history = false;
                self.workspace.preview_scroll = Default::default();
                if *panel == 3
                    && self.work.catalog.is_none()
                    && !self.busy
                    && self.selected.is_some()
                {
                    self.refresh_work();
                }
                if *panel == 4 && !self.studio.loaded && !self.busy {
                    self.refresh_studio();
                }
                if *panel == 8
                    && !self.back_screen.stepos_suite.checked
                    && !self.back_screen.stepos_suite_loading
                    && self.selected.is_some()
                {
                    self.handle_backscreen_action(
                        &super::backscreen_state::BackScreenAction::InspectStepOsSuite,
                        ctx,
                    );
                }
            }
            A::Settings => {
                self.workspace.panel = 5;
                self.page = Page::Amoo;
            }
            A::ChooseFolder => {
                anyhow::ensure!(!self.workspace_busy(), "执行期间不能修改资料目录");
                if let Some(path) = native_dialog::FileDialog::new()
                    .set_title("选择此任务可读取资料、保存成果的文件夹")
                    .show_open_single_dir()?
                {
                    self.workspace_ensure_task();
                    let path = path.canonicalize()?;
                    self.work.workspace = Some(path.clone());
                    self.workspace_task_mut().unwrap().workspace = Some(path);
                }
            }
            A::Prepare => {
                self.workspace_prepare_and_confirm(Some(ctx))?;
            }
            A::EditPlan => {
                self.workspace.proposal = None;
                self.workspace.intent = Intent::Auto;
                self.focus_field(F_AMOO, ctx);
            }
            A::Confirm => self.workspace_confirm(ctx)?,
            A::Stop => {
                if self
                    .workspace
                    .running
                    .as_ref()
                    .is_some_and(|r| r.executor == Executor::Screen)
                {
                    self.handle_backscreen_action(
                        &super::backscreen_state::BackScreenAction::CancelGenerate,
                        ctx,
                    );
                }
                self.workspace_stop();
            }
            A::Retry => {
                anyhow::ensure!(!self.workspace_active_busy(), "请等待当前操作完成");
                let task = self
                    .workspace_task()
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("请选择任务"))?;
                let prompt = task
                    .turns
                    .iter()
                    .rev()
                    .find(|t| t.user)
                    .map(|t| t.text.clone())
                    .unwrap_or_default();
                self.fields[F_AMOO].set(&prompt);
                let start = task
                    .steps
                    .iter()
                    .position(|s| {
                        matches!(s.phase, Phase::Failed | Phase::Stopped | Phase::Unconfirmed)
                    })
                    .unwrap_or(task.steps.len().saturating_sub(1));
                let steps = task
                    .steps
                    .iter()
                    .skip(start)
                    .map(|s| s.executor)
                    .collect::<Vec<_>>();
                let files = if steps.first() == Some(&Executor::App) {
                    vec![]
                } else {
                    task.submitted_files
                };
                self.amoo_attachments.clear();
                self.add_amoo_paths(&files)?;
                self.workspace.proposal = Some(Proposal {
                    text: prompt,
                    files,
                    steps,
                });
            }
            A::Preview(i) => {
                let output = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(*i))
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("此成果不可用"))?;
                self.workspace.selected_output = Some(*i);
                self.workspace.panel = 6;
                self.workspace.preview_scroll = Default::default();
                self.workspace.preview_zoom = 100;
                self.motion_started = Instant::now();
                if let Some(id) = output.lab_id {
                    self.workspace_select_lab(&id)?;
                } else {
                    self.work.workspace = self.workspace_task().and_then(|t| t.workspace.clone());
                    self.preview_work_file(output.path)?;
                }
            }
            A::SelectLab(id) => {
                anyhow::ensure!(!self.workspace_busy(), "请先等待当前任务完成");
                self.workspace_capture_draft();
                self.workspace_select_lab(id)?;
                let artifact = self.studio_lab_selected().cloned().unwrap();
                let existing = self
                    .workspace
                    .journal
                    .tasks
                    .iter()
                    .find(|t| {
                        self.workspace_visible(t)
                            && t.outputs.iter().any(|o| o.lab_id.as_ref() == Some(id))
                    })
                    .map(|t| t.id.clone());
                if let Some(id) = existing {
                    self.workspace.active = Some(id);
                } else {
                    let mut task = Task::new(None, None);
                    task.title = artifact.name.clone();
                    task.add_output(Output {
                        path: artifact.html_path,
                        name: artifact.name,
                        kind: "应用".into(),
                        lab_id: Some(id.clone()),
                    });
                    self.workspace.active = Some(task.id.clone());
                    self.workspace.journal.tasks.push(task);
                }
                self.workspace_restore();
                let i = self
                    .workspace_task()
                    .unwrap()
                    .outputs
                    .iter()
                    .position(|o| o.lab_id.as_ref() == Some(id))
                    .unwrap();
                return self.do_workspace(&A::Preview(i), ctx);
            }
            A::AddOutput(i) => {
                let path = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(*i))
                    .map(|o| o.path.clone())
                    .ok_or_else(|| anyhow::anyhow!("文件不存在"))?;
                self.add_amoo_paths(&[path])?;
                self.workspace.panel = 0;
            }
            A::Deliver(cloud) => {
                anyhow::ensure!(!self.busy, "请等待当前操作完成");
                let output = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(self.workspace.selected_output?))
                    .context("请先选择成果")?;
                self.workspace.delivery_target = Some((
                    self.workspace.active.clone().unwrap_or_default(),
                    output.path.clone(),
                    self.selected.as_ref().map(|d| d.serial.clone()),
                    self.studio.web.as_ref().map(|w| w.scope.clone()),
                ));
                self.workspace.delivery = Some(*cloud);
                self.workspace.open_phone = false;
                if *cloud {
                    self.workspace.panel = 6;
                    if self.studio.web.is_none() {
                        self.sync_studio_cloud();
                    }
                }
            }
            A::CancelDelivery => {
                self.workspace.delivery = None;
                self.workspace.delivery_target = None;
            }
            A::ToggleOpenPhone => self.workspace.open_phone = !self.workspace.open_phone,
            A::ConfirmDelivery => {
                anyhow::ensure!(!self.demo_mode, "演示模式不会发送文件或同步官网");
                anyhow::ensure!(!self.busy, "正在交付，请勿重复点击");
                let cloud = self
                    .workspace
                    .delivery
                    .ok_or_else(|| anyhow::anyhow!("请先确认交付目标"))?;
                let output = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(self.workspace.selected_output?))
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("请选择成果"))?;
                let target = (
                    self.workspace.active.clone().unwrap_or_default(),
                    output.path.clone(),
                    self.selected.as_ref().map(|d| d.serial.clone()),
                    self.studio.web.as_ref().map(|w| w.scope.clone()),
                );
                let expected = self
                    .workspace
                    .delivery_target
                    .as_ref()
                    .context("请重新核对交付目标")?;
                anyhow::ensure!(
                    target.0 == expected.0
                        && target.1 == expected.1
                        && target.2 == expected.2
                        && (!cloud || target.3 == expected.3),
                    "作品或目标账号已变化，请重新点击发送或同步并确认"
                );
                if let Some(id) = &output.lab_id {
                    self.workspace_select_lab(id)?;
                    if cloud {
                        if let Some(url) = self
                            .workspace_task()
                            .and_then(|t| {
                                t.receipts.iter().rev().find(|r| {
                                    r.path == output.path
                                        && r.device == target.2.clone().unwrap_or_default()
                                        && r.account == target.3
                                        && r.cloud_url.is_some()
                                })
                            })
                            .and_then(|r| r.cloud_url.clone())
                        {
                            self.with_device(
                                "正在打开已同步的官网草稿…",
                                move |adb, d| {
                                    Ok(Outcome::Notice(stepx_companion::studio::open_editor(
                                        adb, &d, &url,
                                    )?))
                                },
                            );
                        } else {
                            self.handle_studio_lab_action(&StudioLabAction::SyncCloud, ctx);
                        }
                    } else {
                        let a = self.studio_lab_selected().cloned().unwrap();
                        let open = self.workspace.open_phone;
                        self.with_device("正在保存到手机作品库…", move |adb, d| {
                            let result = (|| {
                                let html = stepx_companion::studio_lab::verified_html(&a)?;
                                let r = stepx_companion::studio_delivery::install_local(
                                    adb, &d, &a.name, &html,
                                )?;
                                if open {
                                    stepx_companion::studio_delivery::open_local(adb, &d, &r.id)?;
                                }
                                Ok(format!(
                                    "{} 已保存到手机作品库，可离线打开{}",
                                    a.name,
                                    if open { "，已打开" } else { "" }
                                ))
                            })();
                            Ok(Outcome::StudioLab(StudioLabOutcome::Phone(
                                d.serial,
                                Some(a.id),
                                result.map_err(|e: anyhow::Error| e.to_string()),
                            )))
                        });
                    }
                } else {
                    anyhow::ensure!(!cloud, "文件请先导出；官网同步支持已验证的应用源码");
                    let path = output.path;
                    let task = self.workspace.active.clone().unwrap_or_default();
                    self.with_device("正在发送文件到手机…", move |adb, d| {
                        let result =
                            stepx_companion::companion_link::send_files(adb, &d, &[path.clone()])
                                .map(|count| {
                                    format!(
                                        "已校验并发送 {count} 个文件到 StepX伴侣 → 文件 → 接收文件"
                                    )
                                })
                                .map_err(|e| e.to_string());
                        Ok(Outcome::WorkspaceDelivered(task, path, d.serial, result))
                    });
                }
                self.workspace.delivery = None;
            }
            A::OpenProject(i) => {
                anyhow::ensure!(!self.demo_mode, "演示模式不会打开真实官网项目");
                self.studio.web_selected = Some(*i);
                let url = self
                    .studio
                    .web
                    .as_ref()
                    .and_then(|w| w.items.get(*i))
                    .and_then(|w| w.editor())
                    .context("这个项目没有可编辑的官网地址")?;
                self.with_device(
                    "正在登录官网并打开对应项目…",
                    move |adb, d| {
                        Ok(Outcome::Notice(stepx_companion::studio::open_editor(
                            adb, &d, &url,
                        )?))
                    },
                );
            }
            A::SelectPersona(idx) => {
                self.amoo_style_idx = *idx % 3;
                let label = match self.amoo_style_idx {
                    0 => "直接精炼 · 不长篇解释",
                    1 => "温和详细 · 分步拆解",
                    _ => "极客专业 · 附带命令",
                };
                self.status = format!("Amoo 沟通风格已切换为：{label}");
                self.error = false;
            }
            A::SelectAgentRole(idx) => {
                self.amoo_agent_idx = *idx % 4;
                let label = match self.amoo_agent_idx {
                    0 => "Amoo 全能助手",
                    1 => "桌面交付伙伴",
                    2 => "办公协同专员",
                    _ => "研发架构专家",
                };
                self.status = format!("Amoo 角色人设已切换为：{label}");
                self.error = false;
            }
            A::ToggleOpenClawSkill(idx) => {
                if let Some(skill) = self.amoo_skills.get_mut(*idx) {
                    skill.enabled = !skill.enabled;
                    let key = skill.skill_key.clone();
                    let name = skill.name.clone();
                    let enabled = skill.enabled;
                    self.status = format!(
                        "OpenClaw 技能「{name}」已{}",
                        if enabled { "启用" } else { "停用" }
                    );
                    self.error = false;
                    if !self.demo_mode && self.selected.is_some() && !self.busy {
                        self.handle_work_action(&WorkAction::ToggleSkill { key, enabled }, ctx);
                    }
                }
            }
            A::UseOpenClawSkill(idx) => {
                if let Some(skill) = self.amoo_skills.get(*idx).cloned() {
                    self.workspace.sub_view = 0;
                    self.fields[F_AMOO].set(&skill.prompt);
                    self.workspace.proposal = None;
                    self.status = format!("已带入技能「{}」指令模板", skill.name);
                    self.error = false;
                    self.focus_field(F_AMOO, ctx);
                }
            }
            A::UseMemory(text) => {
                self.workspace.sub_view = 0;
                let current = self.fields[F_AMOO].text.trim().to_string();
                let next = if current.is_empty() {
                    format!("结合我的个人记忆「{text}」，请帮我：")
                } else {
                    format!("{current}\n（参考记忆：{text}）")
                };
                self.fields[F_AMOO].set(&next);
                self.workspace.proposal = None;
                self.status = "已将个人记忆带入当前对话输入框".into();
                self.error = false;
                self.focus_field(F_AMOO, ctx);
            }
            A::ToggleMemory => {
                self.amoo_memory_enabled = !self.amoo_memory_enabled;
                self.status = if self.amoo_memory_enabled {
                    "已开启个人记忆自动关联".into()
                } else {
                    "已暂停个人记忆自动关联".into()
                };
                self.error = false;
            }
            A::MemoryCategory(cat) => {
                self.workspace.memory_category = *cat % 4;
            }
            A::AddMemory => {
                let raw_input = self.fields[F_AMOO_MEMORY_INPUT].text.trim().to_string();
                let fallback = self.fields[F_AMOO].text.trim().to_string();
                let content = if !raw_input.is_empty() {
                    raw_input
                } else {
                    fallback
                };
                anyhow::ensure!(
                    !content.is_empty(),
                    "请先在记忆输入框填写要记住的偏好或备忘内容"
                );
                let cat_label = match self.workspace.memory_category {
                    1 => "关于我 · 个人画像",
                    2 => "交流偏好 · 称呼与语气",
                    3 => "备忘事项 · 重要记录",
                    _ => "关于我 · 个人记忆",
                };
                self.amoo_memories.insert(
                    0,
                    AmooMemoryEntry {
                        title: content.clone(),
                        sub: format!("{cat_label} · 刚刚添加"),
                    },
                );
                self.amoo_bm25_entries = self.amoo_bm25_entries.max(self.amoo_memories.len());
                self.fields[F_AMOO_MEMORY_INPUT].clear();
                self.status = format!("已添加个人记忆：「{content}」（{cat_label}）");
                self.error = false;
            }
            A::DeleteMemory(idx) => {
                if *idx < self.amoo_memories.len() {
                    let removed = self.amoo_memories.remove(*idx);
                    self.status = format!("已移除个人记忆：「{}」", removed.title);
                    self.error = false;
                }
            }
            A::SendScheduleToAmoo(summary) => {
                self.workspace.sub_view = 0;
                self.workspace.panel = 0;
                self.workspace.intent = Intent::Auto;
                let prompt = format!(
                    "围绕我的手机日程/提醒「{summary}」，请帮我安排准备事项或设置后续提醒："
                );
                self.fields[F_AMOO].set(&prompt);
                self.workspace.proposal = None;
                self.status = format!("已将日程「{summary}」带入 Amoo 对话");
                self.error = false;
                self.focus_field(F_AMOO, ctx);
            }
            A::McpPrompt(idx) => {
                let prompt = match *idx {
                    0 => "请帮我查看今天的手机日历日程安排，并梳理需要提前准备的事项：",
                    1 => "请帮我在手机相册中按内容搜索照片，或查找近期保存的文档：",
                    2 => "请在手机端用语音播报以下提醒内容，并检查当前手机状态：",
                    _ => "请在手机上帮我自动完成以下跨应用操作任务：",
                };
                self.workspace.sub_view = 0;
                self.workspace.intent = Intent::Auto;
                self.fields[F_AMOO].set(prompt);
                self.workspace.proposal = None;
                self.focus_field(F_AMOO, ctx);
            }
            A::SubView(view) => {
                if *view == 1 && !is_verification_harness() {
                    return self.do_workspace(&A::OpenInWork(None), ctx);
                }
                if *view == 2 && !is_verification_harness() {
                    return self.do_workspace(&A::OpenInStudio(None), ctx);
                }
                self.workspace.sub_view = *view;
                if *view == 1
                    && self.work.catalog.is_none()
                    && !self.busy
                    && self.selected.is_some()
                {
                    self.refresh_work();
                }
                if *view == 2 && !self.studio.loaded && !self.busy {
                    self.refresh_studio();
                }
            }
            A::QuickScenario(id) => {
                let prompt = quick_scenario_prompt(*id);
                let target_intent = match *id {
                    0 | 1 => Intent::App,
                    2..=4 => Intent::Files,
                    _ => Intent::Screen,
                };
                self.workspace.sub_view = 0;
                self.workspace.panel = 0;
                if self.fields[F_AMOO].text.trim() == prompt.trim() {
                    self.fields[F_AMOO].clear();
                    self.workspace.intent = Intent::Auto;
                    if let Some(active_id) = self.workspace.active.clone() {
                        if self.workspace.journal.tasks.iter().any(|t| {
                            t.id == active_id
                                && t.turns.is_empty()
                                && t.outputs.is_empty()
                                && t.session.is_none()
                                && t.attachments.is_empty()
                        }) {
                            self.workspace.journal.tasks.retain(|t| t.id != active_id);
                            self.workspace.active = None;
                        }
                    }
                } else {
                    self.workspace.intent = target_intent;
                    self.fields[F_AMOO].set(prompt);
                }
                self.workspace.proposal = None;
                self.motion_started = Instant::now();
                self.focus_field(F_AMOO, ctx);
            }
            A::OpenInWork(output_idx) => {
                let default_root =
                    self.settings
                        .value
                        .work_directory
                        .clone()
                        .unwrap_or_else(|| {
                            stepx_companion::resources::user_state().join("amoo-workspace/files")
                        });
                let root = self
                    .workspace_task()
                    .and_then(|t| t.workspace.clone())
                    .or_else(|| self.work.workspace.clone())
                    .unwrap_or(default_root);
                let _ = std::fs::create_dir_all(&root);
                self.work.workspace = Some(root.clone());
                self.refresh_work_workspace();
                if let Some(idx) = output_idx {
                    if let Some(output) = self
                        .workspace_task()
                        .and_then(|t| t.outputs.get(*idx))
                        .cloned()
                    {
                        if let Some(parent) = output.path.parent() {
                            if !output.path.starts_with(&root) {
                                self.work.workspace = Some(parent.to_path_buf());
                                self.refresh_work_workspace();
                            }
                        }
                        let _ = self.preview_work_file(output.path);
                    }
                }
                let draft = self.fields[F_AMOO].text.trim().to_string();
                if !draft.is_empty() && self.fields[F_WORK_PROMPT].text.trim().is_empty() {
                    self.fields[F_WORK_PROMPT].set(&draft);
                }
                if self.work.catalog.is_none() && !self.busy && self.selected.is_some() {
                    self.refresh_work();
                }
                self.page = Page::Work;
            }
            A::OpenInStudio(output_idx) => {
                if let Some(idx) = output_idx {
                    if let Some(output) = self
                        .workspace_task()
                        .and_then(|t| t.outputs.get(*idx))
                        .cloned()
                    {
                        if let Some(lab_id) = &output.lab_id {
                            let _ = self.workspace_select_lab(lab_id);
                        } else if output
                            .path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| e.eq_ignore_ascii_case("html"))
                        {
                            self.studio_lab.source = Some(output.path);
                        }
                    }
                }
                let draft = self.fields[F_AMOO].text.trim().to_string();
                if !draft.is_empty() && self.fields[F_LAB_PROMPT].text.trim().is_empty() {
                    self.fields[F_LAB_PROMPT].set(&draft);
                }
                if !self.studio.loaded && !self.busy {
                    self.refresh_studio();
                }
                self.page = Page::Studio;
            }
            A::PlayAppOutput(idx) => {
                let output = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(*idx))
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("此应用成果不可用"))?;
                if let Some(id) = &output.lab_id {
                    self.workspace_select_lab(id)?;
                    self.handle_studio_lab_action(&StudioLabAction::Preview, ctx);
                } else {
                    super::work_state::open_work_path(&output.path)?;
                }
            }
            A::PushAppOutputToPhone(idx) => {
                self.workspace.selected_output = Some(*idx);
                return self.do_workspace(&A::Deliver(false), ctx);
            }
            A::StudioPreviewArtifact(idx) => {
                if *idx < self.studio_lab.artifacts.len() {
                    self.studio_lab.selected = Some(*idx);
                    self.studio_lab.source =
                        Some(self.studio_lab.artifacts[*idx].html_path.clone());
                    self.studio_lab.report = None;
                    self.studio_lab.report_loading = None;
                }
                self.page = Page::Studio;
            }
            A::StudioDeliverArtifact(idx) => {
                if *idx < self.studio_lab.artifacts.len() {
                    self.studio_lab.selected = Some(*idx);
                    self.studio_lab.source =
                        Some(self.studio_lab.artifacts[*idx].html_path.clone());
                    self.studio_lab.report = None;
                    self.studio_lab.report_loading = None;
                }
                self.page = Page::Studio;
                self.handle_studio_lab_action(&StudioLabAction::DeliverPhone, ctx);
            }
            A::OpenOutput(idx) => {
                let output = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(*idx))
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("文件不存在"))?;
                super::work_state::open_work_path(&output.path)?;
            }
            A::SelectOutput(idx) => {
                return self.do_workspace(&A::Preview(*idx), ctx);
            }
            A::Reroute(intent) => {
                if self.workspace_busy() || self.workspace_active_busy() {
                    if self
                        .workspace
                        .running
                        .as_ref()
                        .is_some_and(|r| r.executor == Executor::Screen)
                    {
                        self.handle_backscreen_action(
                            &super::backscreen_state::BackScreenAction::CancelGenerate,
                            ctx,
                        );
                    }
                    self.workspace_stop();
                }
                if let Some(handle) = self.work.handle.take() {
                    handle
                        .cancel
                        .store(true, std::sync::atomic::Ordering::Relaxed);
                }
                self.work.running = false;
                if let Some(handle) = self.studio_lab.handle.take() {
                    handle
                        .cancel
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
                if let Some(run) = self.workspace.running.take() {
                    if let Some(session_key) = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| t.id == run.task)
                        .and_then(|t| t.session.clone())
                    {
                        self.amoo_pending_reply.remove(&session_key);
                        self.workspace.reconciled_sessions.remove(&session_key);
                        self.workspace.stopped_sessions.remove(&session_key);
                    }
                }
                self.workspace.next = false;
                self.workspace.image_delivery = None;
                self.workspace.stream_tool_status = None;
                self.workspace.stream_delta_count = 0;
                self.workspace.proposal = None;
                self.workspace.panel = 0;
                self.workspace.intent = Intent::Auto;

                let default_root =
                    self.settings
                        .value
                        .work_directory
                        .clone()
                        .unwrap_or_else(|| {
                            stepx_companion::resources::user_state().join("amoo-workspace/files")
                        });
                let current_draft = self.fields[F_AMOO].text.trim().to_string();
                let (has_user_turn, files, steps) = {
                    let task = self
                        .workspace_task_mut()
                        .ok_or_else(|| anyhow::anyhow!("请选择任务"))?;
                    let last_user_idx = task.turns.iter().rposition(|t| t.user);
                    let (text, files) = if let Some(idx) = last_user_idx {
                        task.turns.truncate(idx + 1);
                        (task.turns[idx].text.clone(), task.submitted_files.clone())
                    } else {
                        let fallback = if !task.draft.trim().is_empty() {
                            task.draft.trim().to_string()
                        } else {
                            current_draft
                        };
                        (fallback, task.attachments.clone())
                    };
                    let decision = stepx_companion::amoo_route::route_intent(
                        &text,
                        &files,
                        &stepx_companion::amoo_route::RouteCtx {
                            forced: Some(*intent),
                            sticky: task.route,
                            last: None,
                        },
                    );
                    task.apply_route(&decision);
                    task.steps = decision
                        .steps
                        .iter()
                        .map(|e| Step {
                            executor: *e,
                            phase: Phase::Ready,
                            note: String::new(),
                        })
                        .collect();
                    task.note.clear();
                    task.updated = chrono::Utc::now().timestamp();
                    if task.workspace.is_none()
                        && decision
                            .steps
                            .iter()
                            .any(|e| matches!(e, Executor::Files | Executor::App | Executor::Image))
                    {
                        task.workspace = Some(default_root.join(&task.id));
                    }
                    (last_user_idx.is_some(), files, decision.steps)
                };
                if has_user_turn && !self.demo_mode {
                    let has_local_model = self.studio_lab.config.has_key
                        || stepx_companion::work::has_local_model_backend();
                    let requires_phone = steps.iter().any(|s| match s {
                        Executor::Phone | Executor::Image => true,
                        Executor::Files | Executor::App => !has_local_model,
                        Executor::Screen => false,
                    });
                    anyhow::ensure!(
                        !requires_phone || self.selected.is_some(),
                        "请连接手机后重试"
                    );
                    if steps.first() == Some(&Executor::Phone)
                        && self.amoo_attachments.is_empty()
                        && !files.is_empty()
                    {
                        let _ = self.add_amoo_paths(&files);
                    }
                    self.workspace.next = true;
                    if let Err(e) = self.workspace_start_next(ctx) {
                        self.workspace_finish(Phase::Failed, e.to_string());
                        return Err(e);
                    }
                }
            }
            A::ContinueEdit(i) => {
                self.workspace_continue_edit(*i)?;
                self.focus_field(F_AMOO, ctx);
            }
            A::RevealOutput(idx) => {
                let output = self
                    .workspace_task()
                    .and_then(|t| t.outputs.get(*idx))
                    .cloned()
                    .ok_or_else(|| anyhow::anyhow!("文件不存在"))?;
                let folder = output
                    .path
                    .parent()
                    .map(std::path::Path::to_path_buf)
                    .unwrap_or(output.path.clone());
                super::work_state::open_work_path(&folder)?;
            }
            A::SendOutputToPhone(idx) => {
                self.workspace.selected_output = Some(*idx);
                return self.do_workspace(&A::Deliver(false), ctx);
            }
            A::CopyTaskTranscript(task_id) => {
                if let Some(task) = self
                    .workspace
                    .journal
                    .tasks
                    .iter()
                    .find(|t| &t.id == task_id)
                {
                    let mut lines = vec![format!("# {}", task.title)];
                    for turn in &task.turns {
                        let role = if turn.user { "我" } else { "Amoo" };
                        lines.push(format!("\n**{role}**：\n{}", turn.text));
                    }
                    let transcript = lines.join("\n");
                    ctx.clipboard()
                        .write(facloud_ui::clipboard::ClipboardContent::plain_text(
                            transcript,
                        ));
                    self.status = format!("已复制「{}」的对话内容", task.title);
                }
            }
            A::SendTaskOutputToPhone(task_id, idx) => {
                let _ = self.do_workspace(&A::Select(task_id.clone()), ctx);
                self.workspace.selected_output = Some(*idx);
                return self.do_workspace(&A::Deliver(false), ctx);
            }
            A::FlowApp => {
                self.workspace.sub_view = 0;
                self.workspace.panel = 0;
                self.workspace.intent = Intent::Auto;
                if self.fields[F_AMOO].text.trim().is_empty() {
                    self.fields[F_AMOO].set("把刚才的内容做成一个可交互的手机小应用：");
                }
                self.workspace.proposal = None;
                self.focus_field(F_AMOO, ctx);
            }
            A::SyncTask => {
                anyhow::ensure!(self.selected.is_some(), "请先连接手机后再同步核对结果");
                let session_key = self.workspace_task().and_then(|t| t.session.clone());
                if let Some(ref k) = session_key {
                    self.workspace.reconciled_sessions.remove(k);
                } else {
                    self.workspace.reconciled_sessions.clear();
                }
                if let Some(worker) = self.amoo_ui.snapshot_worker.as_ref() {
                    worker
                        .reset_baseline
                        .store(true, std::sync::atomic::Ordering::SeqCst);
                }
                self.amoo_ui.snapshot_job = None;
                self.amoo_poll_remaining = self.amoo_poll_remaining.max(20);
                let pending = self.workspace_sync_pending_keys();
                self.begin_amoo_sync(session_key, pending, true);
                self.status = "正在向手机 Amoo 同步核对最新回复结果…".into();
                self.error = false;
            }
            A::DismissUnconfirmed => {
                self.workspace_dismiss_unconfirmed();
            }
            A::DismissError => {
                self.workspace_dismiss_error();
            }
            A::QuoteTurn(text) => {
                self.workspace.sub_view = 0;
                let snippet: String = text
                    .lines()
                    .map(str::trim)
                    .filter(|l| !l.is_empty())
                    .take(3)
                    .collect::<Vec<_>>()
                    .join(" ");
                let snippet = if snippet.chars().count() > 120 {
                    let truncated: String = snippet.chars().take(120).collect();
                    format!("{truncated}…")
                } else {
                    snippet
                };
                let current = self.fields[F_AMOO].text.trim().to_string();
                let next = if current.is_empty() {
                    format!("> {snippet}\n\n请基于以上内容继续帮我：")
                } else {
                    format!("> {snippet}\n\n{current}")
                };
                self.fields[F_AMOO].set(&next);
                self.workspace.proposal = None;
                self.status = "已引用回复内容到输入框，可直接补充追问".into();
                self.error = false;
                self.focus_field(F_AMOO, ctx);
            }
            A::PreviewZoom(zoom) => {
                self.workspace.preview_zoom = (*zoom).clamp(100, 300);
            }
        }
        Ok(())
    }
    pub(super) fn workspace_dismiss_unconfirmed(&mut self) {
        let mut session_key = None;
        if let Some(task) = self.workspace_task_mut() {
            session_key = task.session.clone();
            task.turns
                .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
            for step in &mut task.steps {
                if matches!(step.phase, Phase::Unconfirmed | Phase::Stopped) {
                    step.phase = Phase::Done;
                    step.note = "已手动核对关闭提示".into();
                }
            }
            if model::is_unconfirmed_placeholder(&task.note) {
                task.note.clear();
            }
            task.updated = chrono::Utc::now().timestamp();
        }
        if let Some(key) = session_key {
            self.amoo_pending_reply.remove(&key);
            self.workspace.reconciled_sessions.insert(key);
        }
        self.workspace_persist();
        self.status = "已清除待核对提示，可继续直接对话".into();
        self.error = false;
    }
    fn workspace_select_lab(&mut self, id: &str) -> Result<()> {
        let i = self
            .studio_lab
            .artifacts
            .iter()
            .position(|a| a.id == id)
            .ok_or_else(|| anyhow::anyhow!("作品记录未载入，请刷新本机作品"))?;
        self.studio_lab.selected = Some(i);
        self.studio_lab.source = Some(self.studio_lab.artifacts[i].html_path.clone());
        self.studio_lab.report = None;
        self.studio_lab.report_loading = None;
        Ok(())
    }
    pub(super) fn workspace_continue_edit(&mut self, i: usize) -> Result<()> {
        let output = self
            .workspace_task()
            .and_then(|t| t.outputs.get(i))
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("文件不存在"))?;
        let ext = output
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let is_app = output.lab_id.is_some() || output.kind.contains("应用") || ext == "html";
        let is_image = output.kind == "图片"
            || matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp" | "heic"
            );
        let route = if is_app {
            if let Some(id) = &output.lab_id {
                let _ = self.workspace_select_lab(id);
            } else if ext == "html" {
                self.studio_lab.source = Some(output.path.clone());
            }
            Intent::App
        } else if is_image {
            Intent::Image
        } else {
            Intent::Files
        };
        if !is_app && (output.path.is_file() || output.lab_id.is_none()) {
            self.add_amoo_paths(&[output.path])?;
        }
        if let Some(task) = self.workspace_task_mut() {
            task.route = Some(route);
            task.route_note = match route {
                Intent::App => "继续修改应用".into(),
                Intent::Image => "继续修改图片".into(),
                _ => "继续修改文件".into(),
            };
        }
        self.workspace.selected_output = Some(i);
        self.workspace.panel = 0;
        Ok(())
    }
    pub(super) fn workspace_prepare_and_confirm(
        &mut self,
        mut ctx: Option<&mut ViewContext<Self>>,
    ) -> Result<()> {
        anyhow::ensure!(
            self.workspace_can_send(),
            "请等待当前操作完成；草稿与附件已保留"
        );
        self.workspace_capture_draft();
        let text = if self.fields[F_AMOO].text.trim().is_empty() {
            "请查看并分析这些附件。".into()
        } else {
            self.fields[F_AMOO].text.trim().to_string()
        };
        let files = self
            .amoo_attachments
            .iter()
            .filter_map(|a| a.local_path.clone())
            .collect::<Vec<_>>();
        anyhow::ensure!(
            files.len() == self.amoo_attachments.len() && files.iter().all(|p| p.is_file()),
            "有附件已移动或不可读取，请重新添加"
        );
        if files.is_empty() {
            if let Some((app, goal)) =
                stepx_companion::amoo_route::parse_instant_phone_control(&text)
            {
                self.workspace.intent = Intent::Auto;
                if let Some(ctx) = ctx.as_deref_mut() {
                    self.handle_action(&Action::QuickGuiScenario { app, goal }, ctx);
                }
                return Ok(());
            }
        }
        if matches!(self.workspace.intent, Intent::Files | Intent::App)
            && stepx_companion::amoo_route::breaks_active_route(
                self.workspace.intent,
                &text,
                !files.is_empty(),
            )
        {
            self.workspace.intent = Intent::Auto;
        }
        let route_ctx = stepx_companion::amoo_route::RouteCtx {
            forced: (self.workspace.intent != Intent::Auto).then_some(self.workspace.intent),
            sticky: self.workspace_task().and_then(|t| t.route),
            last: self
                .workspace_task()
                .and_then(|t| t.steps.last())
                .map(|s| s.executor),
        };
        let decision = stepx_companion::amoo_route::route_intent(&text, &files, &route_ctx);
        self.workspace_ensure_task();
        if let Some(task) = self.workspace_task_mut() {
            task.apply_route(&decision);
        }
        self.workspace.proposal = Some(Proposal {
            text,
            files,
            steps: decision.steps,
        });
        self.workspace.panel = 0;
        if let Err(error) = self.workspace_confirm_opt(ctx) {
            self.workspace.proposal = None;
            return Err(error);
        }
        Ok(())
    }
    fn workspace_confirm(&mut self, ctx: &mut ViewContext<Self>) -> Result<()> {
        self.workspace_confirm_opt(Some(ctx))
    }
    fn workspace_confirm_opt(&mut self, ctx: Option<&mut ViewContext<Self>>) -> Result<()> {
        anyhow::ensure!(!self.workspace_active_busy(), "请等待当前操作完成");
        let proposal = self
            .workspace
            .proposal
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先核对执行计划"))?;
        if self
            .workspace
            .running
            .as_ref()
            .is_some_and(|r| r.executor == Executor::Phone)
        {
            self.workspace.running = None;
            self.workspace.stream_tool_status = None;
            self.workspace.stream_delta_count = 0;
        }
        if self.workspace.running.is_some()
            || (proposal
                .steps
                .iter()
                .any(|s| matches!(s, Executor::Files | Executor::Image))
                && self.work.running)
            || (proposal.steps.contains(&Executor::App) && self.studio_lab.handle.is_some())
        {
            self.workspace_stop();
        }
        let has_local_model =
            self.studio_lab.config.has_key || stepx_companion::work::has_local_model_backend();
        let requires_phone = proposal.steps.iter().any(|s| match s {
            Executor::Phone | Executor::Image => true,
            Executor::Files | Executor::App => !has_local_model,
            Executor::Screen => false,
        });
        if self.demo_mode && !proposal.steps.contains(&Executor::App) {
            anyhow::bail!("演示模式仅用于检查界面，不会执行手机任务或模型请求");
        }
        anyhow::ensure!(
            self.demo_mode || !requires_phone || self.selected.is_some(),
            "请连接手机后发送；草稿和附件已保留"
        );
        if proposal.steps.contains(&Executor::App) && !self.demo_mode {
            anyhow::ensure!(self.studio_lab.loaded, "正在读取模型配置，请稍后重试");
            anyhow::ensure!(
                !self.studio_lab.config.has_key
                    || (!self.studio_lab.config.model.is_empty()
                        && !self.studio_lab_config_dirty()),
                "制作应用需先在“模型与账号”中保存并测试自己的 API"
            );
        }
        self.workspace_ensure_task();
        let default_root = self
            .settings
            .value
            .work_directory
            .clone()
            .unwrap_or_else(|| {
                stepx_companion::resources::user_state().join("amoo-workspace/files")
            });
        let device = self.selected.as_ref().map(|d| d.serial.clone());
        let account = self.amoo_ui.account_scope.clone();
        let task = self.workspace_task_mut().unwrap();
        if requires_phone || device.is_some() {
            task.device = device;
            task.account = account;
        }
        if task.turns.is_empty() {
            task.title = model::clean_task_title(&proposal.text);
        }
        task.turns.push(Turn {
            user: true,
            text: proposal.text.clone(),
        });
        task.steps = proposal
            .steps
            .iter()
            .map(|e| Step {
                executor: *e,
                phase: Phase::Ready,
                note: String::new(),
            })
            .collect();
        task.updated = chrono::Utc::now().timestamp();
        if task.workspace.is_none()
            && proposal
                .steps
                .iter()
                .any(|e| matches!(e, Executor::Files | Executor::App | Executor::Image))
        {
            task.workspace = Some(default_root.join(&task.id));
        }
        if let Some(ref dir) = task.workspace {
            let _ = std::fs::create_dir_all(dir);
        }
        task.submitted_files = proposal.files;
        task.draft.clear();
        task.intent = Intent::Auto;
        task.note.clear();
        self.workspace.proposal = None;
        self.workspace.panel = 0;
        // One-shot forced routes reset after sending; sticky route lives on `task.route`.
        self.workspace.intent = Intent::Auto;
        self.workspace.next = true;
        if let Err(e) = self.workspace_start_next_opt(ctx) {
            let msg = e.to_string();
            self.workspace_finish(Phase::Failed, msg.clone());
            self.status = msg;
            self.error = true;
            return Ok(());
        }
        Ok(())
    }
    fn workspace_start_next(&mut self, ctx: &mut ViewContext<Self>) -> Result<()> {
        self.workspace_start_next_opt(Some(ctx))
    }
    fn workspace_start_next_opt(&mut self, ctx: Option<&mut ViewContext<Self>>) -> Result<()> {
        if self.workspace_busy() {
            return Ok(());
        }
        let target_id = self
            .workspace
            .running
            .as_ref()
            .map(|r| r.task.clone())
            .or_else(|| {
                self.workspace.active.clone().filter(|id| {
                    self.workspace
                        .journal
                        .tasks
                        .iter()
                        .any(|t| &t.id == id && t.steps.iter().any(|s| s.phase == Phase::Ready))
                })
            })
            .or_else(|| {
                self.workspace
                    .journal
                    .tasks
                    .iter()
                    .rev()
                    .find(|t| t.steps.iter().any(|s| s.phase == Phase::Ready))
                    .map(|t| t.id.clone())
            });
        let Some(task_id) = target_id else {
            self.workspace.next = false;
            return Ok(());
        };
        let Some(task) = self
            .workspace
            .journal
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .cloned()
        else {
            self.workspace.next = false;
            return Ok(());
        };
        let Some(index) = task.steps.iter().position(|s| s.phase == Phase::Ready) else {
            self.workspace.next = false;
            return Ok(());
        };
        let is_active_task = self.workspace.active.as_deref() == Some(task.id.as_str());
        let executor = task.steps[index].executor;
        let original = task
            .turns
            .iter()
            .rev()
            .find(|t| t.user)
            .map(|t| t.text.clone())
            .unwrap_or_default();
        self.workspace.next = false;
        self.workspace.stream_tool_status = None;
        self.workspace.stream_delta_count = 0;
        if let Some(ref k) = task.session {
            self.workspace.reconciled_sessions.remove(k);
            self.workspace.stopped_sessions.remove(k);
        }
        self.workspace.running = Some(ActiveRun {
            task: task.id.clone(),
            step: index,
            executor,
            accepted: false,
            started_at: Instant::now(),
        });
        if let Some(t) = self
            .workspace
            .journal
            .tasks
            .iter_mut()
            .find(|t| t.id == task.id)
        {
            t.steps[index].phase = Phase::Running;
        }
        if is_active_task || matches!(executor, Executor::Phone) {
            self.amoo_session_idx = task
                .session
                .as_ref()
                .and_then(|k| self.amoo_sessions.iter().position(|s| &s.session_key == k))
                .unwrap_or(usize::MAX);
        }
        match executor {
            Executor::Phone | Executor::Image => {
                // Ensure unrelated background sync flags do not prevent starting the phone/image request.
                self.busy = false;
                self.fields[F_AMOO].set(&original);
                if executor == Executor::Image {
                    self.work.mode = 2;
                    self.work.attachments = task.submitted_files;
                    self.fields[F_WORK_PROMPT].set(&original);
                    if let Some(ctx) = ctx {
                        self.handle_work_action(&WorkAction::Send, ctx);
                    }
                    if self.busy && is_active_task {
                        self.fields[F_AMOO].clear();
                        self.amoo_attachments.clear();
                    }
                } else {
                    self.send_amoo_draft();
                }
                if !self.busy {
                    self.workspace_finish(Phase::Failed, self.status.clone());
                }
            }
            Executor::Files => {
                self.busy = false;
                let root = task.workspace.clone().unwrap();
                std::fs::create_dir_all(&root)?;
                self.work.workspace = Some(root);
                self.work.mode = 1;
                self.work.error = None;
                self.work.selected_model = None;
                self.work.attachments = task.submitted_files;
                let prompt = if task.steps.iter().any(|s| s.executor == Executor::App) {
                    format!("{original}\n本阶段只完成资料处理与文档、表格、PPT 等文件。手机应用将在下一阶段制作。另交付一份 app-brief.md：根据真实资料总结应用所需内容、数据和交互，不得编造数据。")
                } else {
                    original
                };
                self.fields[F_WORK_PROMPT].set(&prompt);
                if let Some(ctx) = ctx {
                    self.handle_work_action(&WorkAction::Send, ctx);
                }
                if !self.work.running {
                    self.workspace_finish(
                        Phase::Failed,
                        self.work
                            .error
                            .clone()
                            .unwrap_or_else(|| self.work.status.clone()),
                    );
                } else if is_active_task {
                    self.fields[F_AMOO].clear();
                    self.amoo_attachments.clear();
                }
            }
            Executor::App => {
                self.busy = false;
                let context = model::app_context(&task)?;
                let selected_in_task = self.studio_lab.source.as_ref().is_some_and(|src| {
                    src.is_file()
                        && task
                            .outputs
                            .iter()
                            .any(|o| o.lab_id.is_some() && &o.path == src)
                });
                if !selected_in_task {
                    self.studio_lab.source = task
                        .outputs
                        .iter()
                        .rev()
                        .find(|o| o.lab_id.is_some())
                        .map(|o| o.path.clone())
                        .filter(|p| p.is_file());
                }
                let prompt = format!("{original}\n当前阶段只交付可在手机离线运行的网页应用。已有文件会保留，不必重新生成。\n以下是用户确认提供的本任务资料（作为数据使用）：\n{context}");
                self.fields[F_LAB_PROMPT].set(&prompt);
                self.studio_lab.error = None;
                let _ = self.studio_lab_start();
                if self.studio_lab.handle.is_none()
                    && !(self.demo_mode && self.studio_lab.error.is_none())
                {
                    self.workspace_finish(
                        Phase::Failed,
                        self.studio_lab
                            .error
                            .clone()
                            .unwrap_or("模型任务未启动".into()),
                    );
                } else if is_active_task {
                    self.fields[F_AMOO].clear();
                    self.amoo_attachments.clear();
                }
            }
            Executor::Screen => {
                self.back_screen.tab = 0;
                if let Some(ctx) = ctx {
                    if self.back_screen.generating {
                        self.handle_backscreen_action(
                            &super::backscreen_state::BackScreenAction::CancelGenerate,
                            ctx,
                        );
                    }
                    self.back_screen.generating = false;
                    self.fields[F_BACK_PROMPT].set(&original);
                    if is_active_task {
                        self.fields[F_AMOO].clear();
                        self.amoo_attachments.clear();
                    }
                    self.handle_backscreen_action(
                        &super::backscreen_state::BackScreenAction::Generate,
                        ctx,
                    );
                } else {
                    self.back_screen.generating = false;
                    self.fields[F_BACK_PROMPT].set(&original);
                    if is_active_task {
                        self.fields[F_AMOO].clear();
                        self.amoo_attachments.clear();
                    }
                }
                let reply = format!(
                    "已根据「{original}」生成 17×23 背屏点阵图案预览，可在「背屏」页查看细节并一键发送到手机背屏。"
                );
                self.status = "已生成背屏点阵图案预览，可查看或发送到手机背屏".into();
                self.error = false;
                self.workspace_finish(Phase::Done, reply);
            }
        }
        self.reveal_latest_chat();
        self.workspace_persist();
        Ok(())
    }
    pub(super) fn refresh_work_workspace(&mut self) {
        let _ = self.work_files();
    }
    pub(super) fn workspace_on_work_finished(&mut self) {
        if let Some(root) = self.workspace_task().and_then(|t| t.workspace.clone()) {
            self.work.workspace = Some(root);
        }
        self.refresh_work_workspace();
    }
    pub(super) fn workspace_on_app_finished(
        &mut self,
        artifact: &stepx_companion::studio_lab::LabArtifact,
    ) {
        let output = Output {
            path: artifact.html_path.clone(),
            name: artifact.name.clone(),
            kind: if artifact.passed {
                "应用 · 已检查"
            } else {
                "应用 · 待修复"
            }
            .into(),
            lab_id: Some(artifact.id.clone()),
        };
        self.workspace_output(output);
    }
    pub(super) fn workspace_app_receipt(
        &mut self,
        id: &str,
        device: &str,
        account: Option<String>,
        note: String,
        success: bool,
        cloud_url: Option<String>,
    ) {
        for task in &mut self.workspace.journal.tasks {
            if let Some(output) = task
                .outputs
                .iter()
                .find(|o| o.lab_id.as_deref() == Some(id))
            {
                task.receipts.push(model::Receipt {
                    path: output.path.clone(),
                    note: note.clone(),
                    success,
                    device: device.into(),
                    account: account.clone(),
                    cloud_url: cloud_url.clone(),
                });
            }
        }
        self.workspace_persist();
    }
    pub(super) fn workspace_output(&mut self, output: Output) {
        if let Some(run) = &self.workspace.running {
            if let Some(task) = self
                .workspace
                .journal
                .tasks
                .iter_mut()
                .find(|t| t.id == run.task)
            {
                task.add_output(output);
            }
        } else if let Some(ref lab_id) = output.lab_id {
            if let Some(task) = self
                .workspace
                .journal
                .tasks
                .iter_mut()
                .find(|t| t.outputs.iter().any(|o| o.lab_id.as_ref() == Some(lab_id)))
            {
                task.route = Some(Intent::App);
                task.add_output(output);
                task.updated = chrono::Utc::now().timestamp();
            } else {
                let mut task = Task::new(
                    self.selected.as_ref().map(|d| d.serial.clone()),
                    self.amoo_ui.account_scope.clone(),
                );
                task.title = format!("创作工坊 · {}", output.name);
                task.route = Some(Intent::App);
                let prompt = self.studio_lab.last_prompt.trim().to_string();
                if !prompt.is_empty() {
                    task.turns.push(Turn {
                        user: true,
                        text: prompt,
                    });
                }
                task.turns.push(Turn {
                    user: false,
                    text: format!("已在创作工坊完成「{}」生成与本机自动自检。", output.name),
                });
                task.steps.push(Step {
                    executor: Executor::App,
                    phase: Phase::Done,
                    note: "已在创作工坊生成并验证".into(),
                });
                task.add_output(output);
                self.workspace.journal.tasks.push(task);
            }
        }
        self.workspace_persist();
    }
    pub(super) fn workspace_toggle_history(&mut self) {
        self.workspace_capture_draft();
        self.workspace_persist();
        self.amoo_ui.screen_mode = false;
        let compact = self.push.shell_mode != ShellMode::Full;
        if !is_verification_harness() && !compact && self.workspace.viewport.x() >= 1000.0 {
            self.workspace.left_rail_open = !self.workspace.left_rail_open;
            self.workspace.history = false;
        } else {
            self.workspace.history = !self.workspace.history;
        }
        self.workspace.panel = 0;
        self.scroll_aux = Default::default();
    }
    fn workspace_prepare_session_switch(&mut self) {
        self.amoo_ui.screen_mode = false;
        if !self.amoo_ui.gui_running && !self.amoo_ui.gui_paused {
            self.amoo_ui.gui_screenshot = None;
        }
        self.workspace.error = None;
        self.error = false;
        if self
            .workspace
            .running
            .as_ref()
            .is_some_and(|r| r.executor == Executor::Phone)
        {
            self.workspace.running = None;
            self.workspace.stream_tool_status = None;
            self.workspace.stream_delta_count = 0;
        }
    }
    pub(super) fn workspace_switch_new(&mut self) -> Result<()> {
        self.workspace_prepare_session_switch();
        let draft = self.fields[F_AMOO].text.clone();
        if self.amoo_attachments.is_empty()
            && (draft.trim().is_empty() || is_quick_scenario_template(&draft))
        {
            if let Some(active_id) = self.workspace.active.clone() {
                if self.workspace.journal.tasks.iter().any(|t| {
                    t.id == active_id
                        && t.turns.is_empty()
                        && t.outputs.is_empty()
                        && t.session.is_none()
                        && t.attachments.is_empty()
                }) {
                    self.workspace.journal.tasks.retain(|t| t.id != active_id);
                    self.workspace.active = None;
                }
            }
        } else {
            self.workspace_capture_draft();
        }
        self.workspace.active = None;
        self.workspace_restore();
        self.motion_started = Instant::now();
        Ok(())
    }
    pub(super) fn workspace_select_task(&mut self, id: &str) -> Result<Option<usize>> {
        anyhow::ensure!(
            self.workspace
                .journal
                .tasks
                .iter()
                .any(|t| t.id == id && self.workspace_visible(t)),
            "此任务属于其他设备或账号"
        );
        self.workspace_prepare_session_switch();
        self.workspace_capture_draft();
        self.workspace.active = Some(id.to_string());
        self.workspace_restore();
        self.motion_started = Instant::now();
        self.workspace_reattach_running_if_pending();
        if let Some(key) = self.workspace_task().and_then(|t| t.session.clone()) {
            let idx = self.amoo_sessions.iter().position(|s| s.session_key == key);
            self.amoo_session_idx = idx.unwrap_or(usize::MAX);
            Ok(idx)
        } else {
            self.amoo_session_idx = usize::MAX;
            Ok(None)
        }
    }
    pub(super) fn workspace_select_phone_session(&mut self, i: usize) -> Result<()> {
        let session = self
            .amoo_sessions
            .get(i)
            .cloned()
            .ok_or_else(|| anyhow::anyhow!("会话已更新，请刷新记录"))?;
        self.workspace_prepare_session_switch();
        self.workspace_capture_draft();
        let existing = self
            .workspace
            .journal
            .tasks
            .iter()
            .find(|t| self.workspace_visible(t) && t.session.as_ref() == Some(&session.session_key))
            .map(|t| t.id.clone());
        let id = existing.unwrap_or_else(|| {
            let mut task = Task::new(
                self.selected.as_ref().map(|d| d.serial.clone()),
                self.amoo_ui.account_scope.clone(),
            );
            task.title = model::clean_task_title(&amoo_display::session_title(&session.title));
            task.session = Some(session.session_key);
            task.updated = chrono::Utc::now().timestamp();
            let id = task.id.clone();
            self.workspace.journal.tasks.push(task);
            id
        });
        self.workspace.active = Some(id);
        self.workspace_restore();
        self.amoo_session_idx = i;
        self.motion_started = Instant::now();
        self.workspace_reattach_running_if_pending();
        Ok(())
    }
    pub(super) fn workspace_stop(&mut self) {
        if self.amoo_ui.gui_running || self.amoo_ui.gui_paused {
            self.handle_amoo_ui_action(&super::amoo_ui_state::AmooUiAction::GuiStopWaiting);
            self.amoo_ui.gui_running = false;
            self.amoo_ui.gui_paused = false;
        }
        if let Some(run) = self.workspace.running.clone() {
            self.workspace.next = false;
            match run.executor {
                Executor::Files => {
                    if let Some(handle) = self.work.handle.take() {
                        handle
                            .cancel
                            .store(true, std::sync::atomic::Ordering::Relaxed);
                    }
                    self.work.running = false;
                    self.work.status = "已停止".into();
                    self.workspace_finish(Phase::Stopped, "已停止".into());
                }
                Executor::App => {
                    if let Some(handle) = self.studio_lab.handle.take() {
                        handle
                            .cancel
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    self.studio_lab.status = "已停止".into();
                    self.workspace_finish(Phase::Stopped, "已停止".into());
                }
                Executor::Screen => {
                    self.back_screen.generating = false;
                    self.workspace_finish(Phase::Stopped, "已停止".into());
                }
                Executor::Phone | Executor::Image => {
                    if run.executor == Executor::Image {
                        if let Some(handle) = self.work.handle.take() {
                            handle
                                .cancel
                                .store(true, std::sync::atomic::Ordering::Relaxed);
                        }
                        self.work.running = false;
                    }
                    let session_key = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| t.id == run.task)
                        .and_then(|t| t.session.clone())
                        .or_else(|| {
                            self.amoo_sessions
                                .get(self.amoo_session_idx)
                                .map(|s| s.session_key.clone())
                        });
                    if let Some(key) = session_key {
                        let run_id = self.amoo_pending_reply.remove(&key).map(|p| p.run_id);
                        self.workspace.stopped_sessions.insert(key.clone());
                        self.workspace.reconciled_sessions.insert(key.clone());
                        if let Some(s) =
                            self.amoo_sessions.iter_mut().find(|s| s.session_key == key)
                        {
                            s.status_kind = 0;
                            s.status = "已停止".into();
                            s.meta = "已停止".into();
                        }
                        if !self.demo_mode {
                            if let Some((device, Ok(adb))) =
                                self.selected.clone().map(|d| (d, Adb::discover()))
                            {
                                std::thread::spawn(move || {
                                    if let Ok(mut bus) =
                                        stepx_companion::amoo::AgentBus::connect(&adb, &device)
                                    {
                                        let _ = bus.abort(&key, run_id.as_deref());
                                    }
                                });
                            }
                        }
                    }
                    self.workspace_finish(Phase::Stopped, "已停止".into());
                }
            }
            self.workspace.stream_tool_status = None;
            self.workspace.stream_delta_count = 0;
            self.busy = false;
            self.status = "已停止当前对话".into();
            self.error = false;
            return;
        }
        let session_key = self
            .workspace_task()
            .and_then(|t| t.session.clone())
            .or_else(|| {
                self.amoo_sessions
                    .get(self.amoo_session_idx)
                    .map(|s| s.session_key.clone())
            });
        if let Some(key) = session_key {
            let run_id = self.amoo_pending_reply.remove(&key).map(|p| p.run_id);
            self.workspace.stopped_sessions.insert(key.clone());
            self.workspace.reconciled_sessions.insert(key.clone());
            if let Some(s) = self.amoo_sessions.iter_mut().find(|s| s.session_key == key) {
                s.status_kind = 0;
                s.status = "已停止".into();
                s.meta = "已停止".into();
            }
            if !self.demo_mode {
                if let Some((device, Ok(adb))) = self.selected.clone().map(|d| (d, Adb::discover()))
                {
                    std::thread::spawn(move || {
                        if let Ok(mut bus) = stepx_companion::amoo::AgentBus::connect(&adb, &device)
                        {
                            let _ = bus.abort(&key, run_id.as_deref());
                        }
                    });
                }
            }
        }
        if let Some(task) = self.workspace_task_mut() {
            task.turns
                .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
            for step in &mut task.steps {
                if matches!(step.phase, Phase::Running | Phase::Unconfirmed) {
                    step.phase = Phase::Stopped;
                    step.note = "已停止".into();
                }
            }
            task.note = "已停止".into();
            task.updated = chrono::Utc::now().timestamp();
        }
        self.workspace.stream_tool_status = None;
        self.workspace.stream_delta_count = 0;
        self.busy = false;
        self.status = "已停止当前对话".into();
        self.error = false;
        self.workspace_persist();
    }
    pub(super) fn workspace_dismiss_error(&mut self) {
        self.workspace.error = None;
        self.error = false;
    }
    pub(super) fn workspace_finish(&mut self, phase: Phase, note: String) {
        // A late download receipt must not complete a later request in the same chat.
        self.workspace.image_delivery = None;
        self.workspace.stream_tool_status = None;
        self.workspace.stream_delta_count = 0;
        let mut finished_files = false;
        if let Some(run) = self.workspace.running.take() {
            finished_files = run.executor == Executor::Files;
            if let Some(task) = self
                .workspace
                .journal
                .tasks
                .iter_mut()
                .find(|t| t.id == run.task)
            {
                if let Some(step) = task.steps.get_mut(run.step) {
                    step.phase = phase;
                    step.note = note.clone();
                }
                task.note = note.clone();
                if run.executor == Executor::Phone && phase == Phase::Unconfirmed {
                    // Keep any real streaming reply in turns; step.note carries the unconfirmed status.
                } else if run.executor == Executor::Phone && phase == Phase::Stopped {
                    task.turns
                        .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
                    if task.turns.last().is_none_or(|t| t.user) {
                        task.turns.push(Turn {
                            user: false,
                            text: note,
                        });
                    }
                } else if run.executor == Executor::Phone
                    && task.turns.last().is_some_and(|t| !t.user)
                {
                    if let Some(last) = task.turns.last_mut() {
                        last.text = note;
                    }
                } else {
                    task.turns.push(Turn {
                        user: false,
                        text: note,
                    });
                }
                task.updated = chrono::Utc::now().timestamp();
                self.workspace.next =
                    phase == Phase::Done && task.steps.iter().any(|s| s.phase == Phase::Ready);
            }
        }
        if finished_files {
            self.workspace_on_work_finished();
        }
        self.workspace_persist();
    }
    pub(super) fn workspace_reattach_running_if_pending(&mut self) {
        if self.workspace.running.is_some() {
            return;
        }
        let Some(task) = self.workspace_task().cloned() else {
            return;
        };
        let Some(ref session_key) = task.session else {
            return;
        };
        if !self.amoo_pending_reply.contains_key(session_key) {
            return;
        }
        if let Some((idx, step)) = task
            .steps
            .iter()
            .enumerate()
            .find(|(_, s)| s.phase == Phase::Running && s.executor == Executor::Phone)
        {
            self.workspace.running = Some(ActiveRun {
                task: task.id,
                step: idx,
                executor: step.executor,
                accepted: true,
                started_at: Instant::now(),
            });
        }
    }
    pub(super) fn workspace_sync_pending_keys(&self) -> Vec<String> {
        let mut keys: Vec<String> = self.amoo_pending_reply.keys().cloned().collect();
        for task in &self.workspace.journal.tasks {
            let visible = if self.amoo_ui.account_scope.is_some() {
                self.workspace_visible(task)
            } else {
                self.selected
                    .as_ref()
                    .is_none_or(|d| task.device.as_deref().is_none_or(|td| td == d.serial))
            };
            if !visible {
                continue;
            }
            let Some(ref session_key) = task.session else {
                continue;
            };
            if self.workspace.deleted_sessions.contains(session_key)
                || self.workspace.stopped_sessions.contains(session_key)
                || self.workspace.reconciled_sessions.contains(session_key)
                || keys.contains(session_key)
            {
                continue;
            }
            if task.steps.iter().any(|s| {
                matches!(s.executor, Executor::Phone | Executor::Image)
                    && matches!(s.phase, Phase::Unconfirmed | Phase::Running)
            }) {
                keys.push(session_key.clone());
            }
        }
        keys
    }
    pub(super) fn reconcile_workspace_with_real_data(
        &mut self,
        real: &stepx_companion::openclaw::RealAmooData,
    ) -> usize {
        let active_running_task = self.workspace.running.as_ref().map(|r| r.task.clone());
        let serial = self.selected.as_ref().map(|d| d.serial.clone());
        let account = self
            .amoo_ui
            .account_scope
            .clone()
            .or_else(|| real.account_scope.clone());
        let mut reconciled = 0usize;
        let mut changed = false;
        let mut clear_active_running = false;

        for task in &mut self.workspace.journal.tasks {
            let visible = if serial.is_none() {
                true
            } else if account.is_some() {
                task.visible(serial.as_deref(), account.as_deref())
            } else {
                task.device
                    .as_deref()
                    .is_none_or(|d| Some(d) == serial.as_deref())
            };
            if !visible {
                continue;
            }
            let Some(session_key) = task.session.clone() else {
                continue;
            };
            if self.workspace.deleted_sessions.contains(&session_key) {
                continue;
            }
            if self.workspace.stopped_sessions.contains(&session_key) {
                for step in &mut task.steps {
                    if matches!(step.executor, Executor::Phone | Executor::Image)
                        && matches!(step.phase, Phase::Unconfirmed | Phase::Running)
                    {
                        step.phase = Phase::Stopped;
                        step.note = "已停止".into();
                        changed = true;
                    }
                }
                if active_running_task.as_deref() == Some(task.id.as_str()) {
                    clear_active_running = true;
                }
                continue;
            }
            if active_running_task.as_deref() == Some(task.id.as_str())
                && self.amoo_pending_reply.contains_key(&session_key)
            {
                continue;
            }
            let needs_reconcile = task.steps.iter().any(|s| {
                matches!(s.executor, Executor::Phone | Executor::Image)
                    && matches!(s.phase, Phase::Unconfirmed | Phase::Running)
            });
            if !needs_reconcile {
                continue;
            }
            let Some(session) = real.sessions.iter().find(|s| s.key == session_key) else {
                if !real.sessions.is_empty() || real.gateway_online {
                    self.amoo_pending_reply.remove(&session_key);
                    self.workspace.reconciled_sessions.insert(session_key);
                    let had_placeholder = task
                        .turns
                        .iter()
                        .any(|t| !t.user && model::is_unconfirmed_placeholder(&t.text));
                    if had_placeholder {
                        task.turns
                            .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
                        changed = true;
                    }
                    let next_note =
                        "已与手机核对：手机端该次回复未完成且已停止，可点击「重新发送」或直接继续提问";
                    for step in &mut task.steps {
                        if matches!(step.executor, Executor::Phone | Executor::Image)
                            && matches!(step.phase, Phase::Unconfirmed | Phase::Running)
                        {
                            if step.phase != Phase::Unconfirmed || step.note != next_note {
                                step.phase = Phase::Unconfirmed;
                                step.note = next_note.into();
                                changed = true;
                            }
                        }
                    }
                    if active_running_task.as_deref() == Some(task.id.as_str()) {
                        clear_active_running = true;
                    }
                }
                continue;
            };
            if !session.history_loaded {
                continue;
            }
            let after_user = match session.messages.iter().rposition(|m| m.role == "user") {
                Some(idx) => &session.messages[idx + 1..],
                None => &session.messages[..],
            };
            let assistant_msgs: Vec<_> = after_user
                .iter()
                .filter(|m| m.role == "assistant")
                .collect();
            let has_completed_reply_after_user =
                !self.amoo_pending_reply.contains_key(&session_key)
                    && !assistant_msgs.is_empty()
                    && !assistant_msgs
                        .iter()
                        .any(|m| m.stop_reason == "child_running")
                    && assistant_msgs.last().is_some_and(|m| {
                        m.is_error
                            || stepx_companion::amoo::is_terminal(&m.stop_reason)
                            || !m.attachments.is_empty()
                            || (m.stop_reason.trim().is_empty() && !m.text.trim().is_empty())
                    });

            let session_running = !has_completed_reply_after_user
                && session.status_kind == 1
                && !matches!(
                    session.status.as_str(),
                    "失败" | "已停止" | "error" | "failed" | "killed"
                );
            let latest_ts = session
                .messages
                .iter()
                .map(|m| m.timestamp_ms)
                .max()
                .unwrap_or(0)
                .max(session.updated_at_ms);
            let child_stale = !session_running
                && !self.amoo_pending_reply.contains_key(&session_key)
                && latest_ts > 0
                && chrono::Utc::now()
                    .timestamp_millis()
                    .saturating_sub(latest_ts)
                    > 300_000;
            let child_running = !child_stale
                && session
                    .messages
                    .iter()
                    .any(|m| m.stop_reason == "child_running");
            let pending_unfinished = self
                .amoo_pending_reply
                .get(&session_key)
                .is_some_and(|p| !p.finished(session))
                && !matches!(
                    session.status.as_str(),
                    "失败" | "已停止" | "error" | "failed" | "killed"
                );
            if session_running || child_running || pending_unfinished {
                self.workspace.reconciled_sessions.remove(&session_key);
                for step in &mut task.steps {
                    if matches!(step.executor, Executor::Phone | Executor::Image)
                        && step.phase == Phase::Unconfirmed
                    {
                        let next_note = "手机端仍在继续处理该任务，正在后台自动同步结果…";
                        if step.note != next_note {
                            step.note = next_note.into();
                            changed = true;
                        }
                    }
                }
                continue;
            }

            if !assistant_msgs.is_empty() {
                let failure = assistant_msgs.iter().rev().find(|m| m.is_error).map(|m| {
                    m.error_message
                        .clone()
                        .unwrap_or_else(|| "手机返回了错误，请查看会话后继续".into())
                });
                let reply_text = assistant_msgs
                    .iter()
                    .filter_map(|m| stepx_companion::work_protocol::display_text(&m.text, false))
                    .filter(|t| !t.trim().is_empty())
                    .collect::<Vec<_>>()
                    .join("\n\n");

                if let Some(error) = failure {
                    for step in &mut task.steps {
                        if matches!(step.executor, Executor::Phone | Executor::Image)
                            && matches!(step.phase, Phase::Unconfirmed | Phase::Running)
                        {
                            step.phase = Phase::Failed;
                            step.note = error.clone();
                        }
                    }
                    task.note = error.clone();
                    task.turns
                        .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
                    if task.turns.last().is_none_or(|t| t.user) {
                        task.turns.push(Turn {
                            user: false,
                            text: error,
                        });
                    }
                    self.amoo_pending_reply.remove(&session_key);
                    self.workspace.reconciled_sessions.insert(session_key);
                    if active_running_task.as_deref() == Some(task.id.as_str()) {
                        clear_active_running = true;
                    }
                    changed = true;
                } else if !reply_text.is_empty()
                    || assistant_msgs.iter().any(|m| {
                        !m.attachments.is_empty()
                            || stepx_companion::amoo::is_terminal(&m.stop_reason)
                    })
                {
                    let note = if reply_text.is_empty() {
                        "已在手机完成回复".to_string()
                    } else {
                        reply_text.clone()
                    };
                    for step in &mut task.steps {
                        if matches!(step.executor, Executor::Phone | Executor::Image)
                            && matches!(step.phase, Phase::Unconfirmed | Phase::Running)
                        {
                            step.phase = Phase::Done;
                            step.note = note.clone();
                        }
                    }
                    task.note = note.clone();
                    task.turns
                        .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
                    if !reply_text.is_empty() {
                        if let Some(last) = task.turns.last_mut().filter(|t| !t.user) {
                            last.text = reply_text;
                        } else {
                            task.turns.push(Turn {
                                user: false,
                                text: reply_text,
                            });
                        }
                    } else if task.turns.last().is_none_or(|t| t.user) {
                        task.turns.push(Turn {
                            user: false,
                            text: note,
                        });
                    }
                    task.updated = chrono::Utc::now().timestamp();
                    self.amoo_pending_reply.remove(&session_key);
                    self.workspace.reconciled_sessions.insert(session_key);
                    if active_running_task.as_deref() == Some(task.id.as_str()) {
                        clear_active_running = true;
                    }
                    reconciled += 1;
                    changed = true;
                } else {
                    self.amoo_pending_reply.remove(&session_key);
                    self.workspace.reconciled_sessions.insert(session_key);
                    let had_placeholder = task
                        .turns
                        .iter()
                        .any(|t| !t.user && model::is_unconfirmed_placeholder(&t.text));
                    if had_placeholder {
                        task.turns
                            .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
                        changed = true;
                    }
                    let next_note =
                        "已与手机核对：手机端该次回复未完成且已停止，可点击「重新发送」或直接继续提问";
                    for step in &mut task.steps {
                        if matches!(step.executor, Executor::Phone | Executor::Image)
                            && matches!(step.phase, Phase::Unconfirmed | Phase::Running)
                        {
                            if step.phase != Phase::Unconfirmed || step.note != next_note {
                                step.phase = Phase::Unconfirmed;
                                step.note = next_note.into();
                                changed = true;
                            }
                        }
                    }
                    if active_running_task.as_deref() == Some(task.id.as_str()) {
                        clear_active_running = true;
                    }
                }
            } else {
                // Phone session is idle and has no assistant reply after the last user message.
                self.amoo_pending_reply.remove(&session_key);
                self.workspace.reconciled_sessions.insert(session_key);
                let had_placeholder = task
                    .turns
                    .iter()
                    .any(|t| !t.user && model::is_unconfirmed_placeholder(&t.text));
                if had_placeholder {
                    task.turns
                        .retain(|t| t.user || !model::is_unconfirmed_placeholder(&t.text));
                    changed = true;
                }
                let next_note =
                    "已与手机核对：手机端该次回复未完成且已停止，可点击「重新发送」或直接继续提问";
                for step in &mut task.steps {
                    if matches!(step.executor, Executor::Phone | Executor::Image)
                        && matches!(step.phase, Phase::Unconfirmed | Phase::Running)
                    {
                        if step.phase != Phase::Unconfirmed || step.note != next_note {
                            step.phase = Phase::Unconfirmed;
                            step.note = next_note.into();
                            changed = true;
                        }
                    }
                }
                if active_running_task.as_deref() == Some(task.id.as_str()) {
                    clear_active_running = true;
                }
            }
        }
        if clear_active_running {
            self.workspace.running = None;
            self.workspace.stream_tool_status = None;
            self.workspace.stream_delta_count = 0;
        }
        if changed {
            self.workspace_persist();
        }
        reconciled
    }
    pub(super) fn workspace_deliver_images(
        &mut self,
        messages: Vec<stepx_companion::openclaw::RealChatMessage>,
        note: String,
    ) {
        if self.workspace.image_delivery.is_some() {
            return;
        }
        let Some(run) = self.workspace.running.clone() else {
            return;
        };
        let mut seen = std::collections::HashSet::new();
        let images = messages
            .iter()
            .flat_map(|m| {
                m.attachments
                    .iter()
                    .cloned()
                    .chain(pages::markdown_images(&m.text))
            })
            .filter(|a| {
                (a.kind == "image" || a.mime.starts_with("image/"))
                    && (!a.url.is_empty() || !a.blob.is_empty())
            })
            .filter(|a| seen.insert((a.url.clone(), a.blob.clone())))
            .collect::<Vec<_>>();
        if images.is_empty() {
            self.workspace_finish(Phase::Unconfirmed, format!("{note}\n\n尚未收到可下载的图片文件，不能确认图片已完成。请同步手机结果或继续要求交付原图。"));
            return;
        }
        let Some(root) = self.workspace_task().and_then(|t| t.workspace.clone()) else {
            self.workspace_finish(
                Phase::Unconfirmed,
                "图片保存目录不可用，请重新选择保存位置".into(),
            );
            return;
        };
        let Some(device) = self.selected.clone() else {
            self.workspace_finish(
                Phase::Unconfirmed,
                "手机已断开，图片尚未保存到电脑，请重连后同步".into(),
            );
            return;
        };
        let adb = match Adb::discover() {
            Ok(adb) => adb,
            Err(error) => {
                self.workspace_finish(Phase::Unconfirmed, format!("图片尚未保存：{error}"));
                return;
            }
        };
        let (tx, rx) = mpsc::channel();
        self.workspace.image_delivery = Some(ImageDelivery {
            task: run.task,
            scope: self.amoo_ui.account_scope.clone(),
            note,
            receiver: rx,
        });
        self.work.status = "已收到图片，正在保存原图到电脑…".into();
        std::thread::spawn(move || {
            let result = (|| -> Result<Vec<(String, PathBuf)>> {
                let folder = root.join(format!("Images-{}", chrono::Utc::now().timestamp_millis()));
                std::fs::create_dir_all(&folder)?;
                let mut saved = Vec::new();
                for (i, a) in images.iter().enumerate() {
                    let path = folder.join(format!("image-{}.png", i + 1));
                    let file =
                        stepx_companion::amoo_media::download_artifact(&adb, &device, a, &path)?;
                    let mut header = [0u8; 64];
                    let n =
                        std::io::Read::read(&mut std::fs::File::open(&file.path)?, &mut header)?;
                    let format =
                        image::guess_format(&header[..n]).context("收到的附件不是有效图片")?;
                    let path = file
                        .path
                        .with_extension(format.extensions_str().first().copied().unwrap_or("png"));
                    if path != file.path {
                        std::fs::rename(&file.path, &path)?;
                    }
                    image::image_dimensions(&path).context("收到的附件不是有效图片")?;
                    saved.push((a.url.clone(), path));
                }
                Ok(saved)
            })()
            .map_err(|e| format!("{e:#}"));
            let _ = tx.send(result);
        });
    }
    pub(super) fn tick_workspace(&mut self, ctx: &mut ViewContext<Self>) -> bool {
        let mut changed = false;
        let image_result =
            self.workspace.image_delivery.as_ref().and_then(|delivery| {
                match delivery.receiver.try_recv() {
                    Ok(result) => Some(result),
                    Err(mpsc::TryRecvError::Empty) => None,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("图片保存任务意外结束，请重新同步".into()))
                    }
                }
            });
        if let Some(result) = image_result {
            let ImageDelivery {
                task, scope, note, ..
            } = self.workspace.image_delivery.take().unwrap();
            if self
                .workspace
                .running
                .as_ref()
                .is_some_and(|r| r.task == task && r.executor == Executor::Image)
            {
                if scope != self.amoo_ui.account_scope {
                    self.workspace_finish(
                        Phase::Unconfirmed,
                        "手机账号已变化，图片未并入当前账号，请切回原账号核对".into(),
                    );
                    return true;
                }
                match result {
                    Ok(files) => {
                        let count = files.len();
                        for (url, path) in files {
                            self.work.image_previews.insert(url, path.clone());
                            self.workspace_output(Output {
                                name: path
                                    .file_name()
                                    .unwrap_or_default()
                                    .to_string_lossy()
                                    .into_owned(),
                                path,
                                kind: "图片".into(),
                                lab_id: None,
                            });
                        }
                        self.workspace_finish(
                            Phase::Done,
                            format!("{note}\n\n{count} 张原图已保存在电脑，可预览或发送到手机。"),
                        );
                    }
                    Err(error) => self.workspace_finish(
                        Phase::Unconfirmed,
                        format!("已收到图片链接，但原图保存未完成：{error}"),
                    ),
                }
                changed = true;
            }
        }
        if let Some(next) = ctx
            .windows()
            .platform_window(ctx.window_id())
            .map(|w| w.size())
        {
            if self.workspace.viewport != next {
                self.workspace.viewport = next;
                changed = true;
            }
        }
        if self
            .workspace_task()
            .is_some_and(|t| !self.workspace_visible(t))
            && !self.workspace_busy()
        {
            self.workspace_capture_draft();
            self.workspace.active = None;
            self.workspace_restore();
            changed = true;
        }
        if self.workspace.next && !self.workspace_busy() {
            if let Err(e) = self.workspace_start_next(ctx) {
                self.workspace_finish(Phase::Failed, e.to_string());
                self.workspace.error = Some(e.to_string());
                self.workspace.next = false;
            }
            changed = true;
        }
        if let Some(run) = self.workspace.running.clone() {
            if matches!(run.executor, Executor::Phone | Executor::Image)
                && run.accepted
                && self.selected.is_none()
            {
                let reconnecting = self.connection_watch.detached_serial.is_some()
                    && self
                        .connection_watch
                        .disconnected_at
                        .is_some_and(|t| t.elapsed() < Duration::from_secs(12));
                if reconnecting {
                    let msg = "手机连接波动，正在尝试无线/USB恢复并继续等待回复…";
                    if self.status != msg {
                        self.status = msg.into();
                        changed = true;
                    }
                } else {
                    if let Some(session_key) = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| t.id == run.task)
                        .and_then(|t| t.session.clone())
                    {
                        self.amoo_pending_reply.remove(&session_key);
                        self.workspace.reconciled_sessions.remove(&session_key);
                    }
                    self.workspace_finish(
                        Phase::Unconfirmed,
                        "手机连接中断，尚未确认回复完成。重连后同步任务记录；未自动重发。".into(),
                    );
                    changed = true;
                }
            } else if run.executor == Executor::Files && !self.work.running {
                self.workspace_finish(
                    Phase::Unconfirmed,
                    "手机账号已变化，原任务未继续执行；已生成文件保留。".into(),
                );
                changed = true;
            }
        }
        if !self.workspace_busy() {
            if self.amoo_pending_reply.is_empty() {
                self.workspace.stream_tool_status = None;
                self.workspace.stream_delta_count = 0;
            }
            self.amoo_session_idx = self
                .workspace_task()
                .and_then(|t| t.session.as_ref())
                .and_then(|k| self.amoo_sessions.iter().position(|s| &s.session_key == k))
                .unwrap_or(usize::MAX);
        }
        if self.workspace.last_saved.elapsed() > Duration::from_millis(700) {
            let draft_changed = self.fields[F_AMOO].text != self.workspace.last_draft.0
                || !self
                    .amoo_attachments
                    .iter()
                    .filter_map(|a| a.local_path.as_ref())
                    .eq(self.workspace.last_draft.1.iter());
            if draft_changed {
                self.workspace_capture_draft();
                self.workspace_persist();
            }
            self.workspace.last_saved = Instant::now();
        }
        #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
        if std::env::var_os("STEPX_CAPTURE_V0247_DIR").is_some() {
            self.tick_v0247_real_device_capture(ctx);
        }
        #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
        if std::env::var_os("STEPX_CAPTURE_V0248_DIR").is_some() {
            self.tick_v0248_real_device_capture(ctx);
        }
        #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
        if std::env::var_os("STEPX_CAPTURE_V0249_DIR").is_some() {
            self.tick_v0249_real_device_capture(ctx);
        }
        #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
        if std::env::var_os("STEPX_CAPTURE_V0250_DIR").is_some() {
            self.tick_v0250_real_device_capture(ctx);
        }
        changed
    }

    #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
    fn tick_v0250_real_device_capture(&mut self, ctx: &mut ViewContext<Self>) {
        thread_local! {
            static STATE: std::cell::RefCell<(usize, Option<Instant>, String)> =
                const { std::cell::RefCell::new((0, None, String::new())) };
        }
        let Some(dir) = std::env::var_os("STEPX_CAPTURE_V0250_DIR").map(PathBuf::from) else {
            return;
        };
        STATE.with(|cell| {
            let mut st = cell.borrow_mut();
            if st
                .1
                .is_some_and(|t| t.elapsed() < Duration::from_millis(700))
            {
                return;
            }
            st.1 = Some(Instant::now());
            #[cfg(feature = "ui-capture")]
            {
                self.capture = None;
            }
            while self.receiver.try_recv().is_ok() {}
            self.amoo_ui.snapshot_job = None;
            self.amoo_poll_remaining = 0;
            self.last_amoo_poll = Instant::now();
            self.last_scan = Instant::now();
            self.amoo_gateway_online = true;
            self.initializing = false;
            self.compact_start_pending = false;
            self.workspace.writer = None;
            self.page = Page::Amoo;

            let save_frame = |ctx: &mut ViewContext<Self>, filename: &str| {
                let path = dir.join(filename);
                if let Some(w) = ctx.windows().platform_window(ctx.window_id()) {
                    w.as_ctx().request_frame_capture(Box::new(move |frame| {
                        image::save_buffer(
                            &path,
                            &frame.data,
                            frame.width,
                            frame.height,
                            image::ColorType::Rgba8,
                        )
                        .expect("save v0.2.50 screenshot");
                    }));
                }
            };

            match st.0 {
                0 => {
                    let _ = std::fs::create_dir_all(&dir);
                    let adb = Adb::discover().expect("adb");
                    let device = Device::inspect(
                        &adb,
                        &std::env::var("STEPX_WORKSPACE_TEST_SERIAL")
                            .expect("explicit test device"),
                    )
                    .expect("inspect device");
                    let summary = device.summary(&adb).unwrap_or_default();
                    let png = adb
                        .screenshot(&device.transport)
                        .expect("capture explicitly selected test phone");
                    let shot_path = dir.join("raw-v0250-phone-screen.png");
                    std::fs::write(&shot_path, &png).expect("write phone screenshot");

                    self.settings.value.reduced_motion = true;
                    self.settings.draft.reduced_motion = true;
                    self.adb_guide_step = None;
                    self.connect_guide_step = None;
                    self.selected = Some(device.clone());
                    self.summary = summary;
                    self.screenshot = Some((tempfile::tempdir().expect("tempdir"), shot_path));
                    self.amoo_ui.gui_running = false;
                    self.amoo_ui.gui_paused = false;
                    self.amoo_ui.gui_screenshot = None;
                    self.amoo_ui.screen_mode = false;

                    if let Ok(artifacts) = stepx_companion::studio_lab::list_artifacts() {
                        self.studio_lab.artifacts = artifacts;
                        self.studio_lab.loaded = true;
                    }
                    let snake_artifact = self
                        .studio_lab
                        .artifacts
                        .iter()
                        .find(|a| a.id == "starter-neon-snake")
                        .cloned()
                        .expect("starter-neon-snake artifact must exist");
                    if let Ok(report) = stepx_companion::studio_lab::read_report(&snake_artifact) {
                        self.studio_lab.report = Some((snake_artifact.id.clone(), report));
                    }

                    let journal_path =
                        stepx_companion::resources::user_state().join("amoo-workspace/tasks.json");
                    let mut journal = Journal::load(&journal_path).unwrap_or_default();
                    let _ = journal.save(&journal_path);
                    self.amoo_ui.account_scope =
                        journal.tasks.iter().find_map(|t| t.account.clone());

                    let nonexistent_ws = PathBuf::from("/tmp/stepx-nonexistent-workspace-v0250");
                    let _ = std::fs::remove_dir_all(&nonexistent_ws);

                    let snake_id = if let Some(t) = journal.tasks.iter_mut().find(|t| {
                        t.outputs
                            .iter()
                            .any(|o| o.lab_id.as_deref() == Some("starter-neon-snake"))
                    }) {
                        t.title = "霓虹贪吃蛇大作战".into();
                        t.workspace = Some(nonexistent_ws.clone());
                        t.turns.clear();
                        t.steps.clear();
                        t.note.clear();
                        t.id.clone()
                    } else {
                        let mut t = Task::new(
                            Some(device.serial.clone()),
                            self.amoo_ui.account_scope.clone(),
                        );
                        t.title = "霓虹贪吃蛇大作战".into();
                        t.workspace = Some(nonexistent_ws.clone());
                        t.outputs.push(Output {
                            path: snake_artifact.html_path.clone(),
                            name: "霓虹贪吃蛇大作战".into(),
                            kind: "应用 · 已检查".into(),
                            lab_id: Some("starter-neon-snake".into()),
                        });
                        let id = t.id.clone();
                        journal.tasks.push(t);
                        id
                    };
                    st.2 = snake_id.clone();
                    self.workspace.journal = journal;
                    self.workspace.active = Some(snake_id);
                    self.amoo_session_idx = usize::MAX;
                    self.workspace.running = None;
                    self.workspace.error = None;
                    self.error = false;
                    self.busy = false;
                    self.status = "已加载作品「霓虹贪吃蛇大作战」".into();

                    self.do_workspace(&WorkspaceAction::ContinueEdit(0), ctx)
                        .expect("ContinueEdit(0) on starter-neon-snake");
                    let task = self.workspace_task().expect("active snake task");
                    assert_eq!(task.route, Some(Intent::App));
                    assert_eq!(task.route_note, "继续修改应用");
                    assert_eq!(model::app_context(task).expect("app_context"), "");

                    self.fields[F_AMOO].set("优化下显示效果");
                    self.push.shell_mode = ShellMode::MiniAi;
                    if let Some(b) = ctx.window_bounds(&ctx.window_id()) {
                        ctx.windows().set_window_bounds(
                            ctx.window_id(),
                            pathfinder_geometry::rect::RectF::new(b.origin(), vec2f(780.0, 920.0)),
                        );
                    }
                    self.workspace.viewport = vec2f(780.0, 920.0);
                }
                1 => {
                    save_frame(ctx, "raw-v0250-panel-left-mini-ai.png");
                }
                2 => {
                    if let Some(t) = self.workspace_task_mut() {
                        let nonexistent_ws =
                            PathBuf::from("/tmp/stepx-nonexistent-workspace-v0250-prepare");
                        let _ = std::fs::remove_dir_all(&nonexistent_ws);
                        t.workspace = Some(nonexistent_ws);
                    }
                    self.do_workspace(&WorkspaceAction::Prepare, ctx)
                        .expect("Prepare follow-up app edit '优化下显示效果'");
                    let task = self
                        .workspace_task()
                        .expect("active snake task after Prepare");
                    assert!(
                        self.workspace.error.is_none(),
                        "workspace.error must be None after Prepare, got {:?}",
                        self.workspace.error
                    );
                    assert!(
                        !task
                            .turns
                            .iter()
                            .any(|t| t.text.contains("No such file or directory")
                                || t.text.contains("os error 2")),
                        "turns must never contain os error 2: {:?}",
                        task.turns
                    );
                    assert!(
                        !task
                            .steps
                            .iter()
                            .any(|s| s.note.contains("No such file or directory")
                                || s.note.contains("os error 2")),
                        "steps must never contain os error 2: {:?}",
                        task.steps
                    );
                    assert!(
                        !task.note.contains("No such file or directory")
                            && !task.note.contains("os error 2"),
                        "task.note must never contain os error 2: {}",
                        task.note
                    );

                    self.push.shell_mode = ShellMode::Full;
                    self.workspace.left_rail_open = true;
                    if let Some(b) = ctx.window_bounds(&ctx.window_id()) {
                        ctx.windows().set_window_bounds(
                            ctx.window_id(),
                            pathfinder_geometry::rect::RectF::new(b.origin(), vec2f(1440.0, 920.0)),
                        );
                    }
                    self.workspace.viewport = vec2f(1440.0, 920.0);
                }
                3 => {
                    save_frame(ctx, "raw-v0250-panel-right-full-executing.png");
                }
                4 => {
                    if let Some(handle) = self.studio_lab.handle.take() {
                        handle
                            .cancel
                            .store(true, std::sync::atomic::Ordering::SeqCst);
                    }
                    let p_left = dir.join("raw-v0250-panel-left-mini-ai.png");
                    let p_right = dir.join("raw-v0250-panel-right-full-executing.png");
                    if p_left.is_file() && p_right.is_file() {
                        let img1 = image::open(&p_left).expect("open left panel").to_rgba8();
                        let img2 = image::open(&p_right).expect("open right panel").to_rgba8();
                        let gap = 16u32;
                        let total_w = img1.width() + img2.width() + gap;
                        let total_h = img1.height().max(img2.height());
                        let mut canvas = image::RgbaImage::from_pixel(
                            total_w,
                            total_h,
                            image::Rgba([15, 23, 42, 255]),
                        );
                        image::imageops::overlay(&mut canvas, &img1, 0, 0);
                        image::imageops::overlay(
                            &mut canvas,
                            &img2,
                            (img1.width() + gap) as i64,
                            0,
                        );
                        canvas
                            .save(dir.join("stepx-v0.2.50-neon-snake-continue-edit-clean.png"))
                            .expect("save composite v0.2.50 neon snake continue edit image");
                    }
                    for temp_name in [
                        "raw-v0250-panel-left-mini-ai.png",
                        "raw-v0250-panel-right-full-executing.png",
                        "raw-v0250-phone-screen.png",
                    ] {
                        let _ = std::fs::remove_file(dir.join(temp_name));
                    }
                    let _ = std::fs::remove_dir_all("/tmp/stepx-nonexistent-workspace-v0250");
                    let _ =
                        std::fs::remove_dir_all("/tmp/stepx-nonexistent-workspace-v0250-prepare");
                    std::process::exit(0);
                }
                _ => {}
            }
            st.0 += 1;
            ctx.notify();
        });
    }

    #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
    fn tick_v0249_real_device_capture(&mut self, ctx: &mut ViewContext<Self>) {
        thread_local! {
            static STATE: std::cell::RefCell<(usize, Option<Instant>, String)> =
                const { std::cell::RefCell::new((0, None, String::new())) };
        }
        let Some(dir) = std::env::var_os("STEPX_CAPTURE_V0249_DIR").map(PathBuf::from) else {
            return;
        };
        STATE.with(|cell| {
            let mut st = cell.borrow_mut();
            if st
                .1
                .is_some_and(|t| t.elapsed() < Duration::from_millis(650))
            {
                return;
            }
            st.1 = Some(Instant::now());
            #[cfg(feature = "ui-capture")]
            {
                self.capture = None;
            }
            while self.receiver.try_recv().is_ok() {}
            self.amoo_ui.snapshot_job = None;
            self.amoo_poll_remaining = 0;
            self.last_amoo_poll = Instant::now();
            self.last_scan = Instant::now();
            self.amoo_gateway_online = true;
            self.initializing = false;
            self.compact_start_pending = false;
            self.workspace.writer = None;
            self.page = Page::Amoo;
            self.push.shell_mode = ShellMode::Full;
            self.workspace.left_rail_open = true;

            let save_frame = |ctx: &mut ViewContext<Self>, filename: &str| {
                let path = dir.join(filename);
                if let Some(w) = ctx.windows().platform_window(ctx.window_id()) {
                    w.as_ctx().request_frame_capture(Box::new(move |frame| {
                        image::save_buffer(
                            &path,
                            &frame.data,
                            frame.width,
                            frame.height,
                            image::ColorType::Rgba8,
                        )
                        .expect("save v0.2.49 screenshot");
                    }));
                }
            };

            match st.0 {
                0 => {
                    let _ = std::fs::create_dir_all(&dir);
                    let adb = Adb::discover().expect("adb");
                    let device = Device::inspect(
                        &adb,
                        &std::env::var("STEPX_WORKSPACE_TEST_SERIAL")
                            .expect("explicit test device"),
                    )
                    .expect("inspect device");
                    let summary = device.summary(&adb).unwrap_or_default();
                    let png = adb
                        .screenshot(&device.transport)
                        .expect("capture explicitly selected test phone");
                    let shot_path = dir.join("raw-v0249-phone-screen.png");
                    std::fs::write(&shot_path, &png).expect("write phone screenshot");

                    self.settings.value.reduced_motion = true;
                    self.settings.draft.reduced_motion = true;
                    self.adb_guide_step = None;
                    self.connect_guide_step = None;
                    self.selected = Some(device.clone());
                    self.summary = summary;
                    self.screenshot = Some((tempfile::tempdir().expect("tempdir"), shot_path));
                    self.amoo_ui.gui_running = false;
                    self.amoo_ui.gui_paused = false;
                    self.amoo_ui.gui_screenshot = None;
                    self.amoo_ui.screen_mode = false;

                    let journal_path =
                        stepx_companion::resources::user_state().join("amoo-workspace/tasks.json");
                    let journal = Journal::load(&journal_path).unwrap_or_default();
                    let _ = journal.save(&journal_path);
                    self.amoo_ui.account_scope =
                        journal.tasks.iter().find_map(|t| t.account.clone());
                    self.workspace.journal = journal;
                    st.2 = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| !t.turns.is_empty() || !t.outputs.is_empty())
                        .or_else(|| self.workspace.journal.tasks.first())
                        .map(|t| t.id.clone())
                        .unwrap_or_default();

                    self.workspace.active = None;
                    self.amoo_session_idx = usize::MAX;
                    self.workspace.running = None;
                    self.workspace.error = None;
                    self.error = false;
                    self.busy = false;
                    self.status = "已连接手机 · Amoo 智能助手就绪".into();
                    self.fields[F_AMOO].clear();
                    self.workspace.intent = Intent::Auto;
                    self.workspace.sub_view = 0;
                    self.workspace.panel = 0;
                    self.context_menu = None;
                }
                1 => {
                    save_frame(ctx, "stepx-v0.2.49-amoo-welcome-guide-clean.png");
                }
                2 => {
                    self.workspace.active = None;
                    self.amoo_session_idx = usize::MAX;
                    self.workspace.error = None;
                    self.error = false;
                    self.busy = false;
                    self.fields[F_AMOO].set(quick_scenario_prompt(2));
                    self.workspace.intent = Intent::Auto;
                    self.open_workspace_intent_menu(ctx);
                }
                3 => {
                    save_frame(ctx, "raw-v0249-panel-left-mode-menu.png");
                }
                4 => {
                    self.context_menu = None;
                    self.fields[F_AMOO].clear();
                    self.workspace.intent = Intent::Auto;
                    self.busy = true;
                    self.status = "正在用手机账号同步 Studio 作品…".into();
                    let target_id = st.2.clone();
                    if !target_id.is_empty() {
                        self.workspace_select_task(&target_id)
                            .expect("seamless session switch while busy == true");
                    }
                    assert!(
                        self.workspace.error.is_none(),
                        "session switch must never set workspace.error"
                    );
                }
                5 => {
                    save_frame(ctx, "raw-v0249-panel-right-switched.png");
                }
                6 => {
                    let p_left = dir.join("raw-v0249-panel-left-mode-menu.png");
                    let p_right = dir.join("raw-v0249-panel-right-switched.png");
                    if p_left.is_file() && p_right.is_file() {
                        let img1 = image::open(&p_left).expect("open left panel").to_rgba8();
                        let img2 = image::open(&p_right).expect("open right panel").to_rgba8();
                        let gap = 16u32;
                        let total_w = img1.width() + img2.width() + gap;
                        let total_h = img1.height().max(img2.height());
                        let mut canvas = image::RgbaImage::from_pixel(
                            total_w,
                            total_h,
                            image::Rgba([15, 23, 42, 255]),
                        );
                        image::imageops::overlay(&mut canvas, &img1, 0, 0);
                        image::imageops::overlay(
                            &mut canvas,
                            &img2,
                            (img1.width() + gap) as i64,
                            0,
                        );
                        canvas
                            .save(dir.join("stepx-v0.2.49-amoo-mode-selector-and-switch.png"))
                            .expect("save composite v0.2.49 mode selector and switch image");
                    }
                    for temp_name in [
                        "raw-v0249-panel-left-mode-menu.png",
                        "raw-v0249-panel-right-switched.png",
                        "raw-v0249-phone-screen.png",
                    ] {
                        let _ = std::fs::remove_file(dir.join(temp_name));
                    }
                    std::process::exit(0);
                }
                _ => {}
            }
            st.0 += 1;
            ctx.notify();
        });
    }

    #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
    fn tick_v0248_real_device_capture(&mut self, ctx: &mut ViewContext<Self>) {
        thread_local! {
            static STATE: std::cell::RefCell<(usize, Option<Instant>, Option<PathBuf>, String, String)> =
                const { std::cell::RefCell::new((0, None, None, String::new(), String::new())) };
        }
        let Some(dir) = std::env::var_os("STEPX_CAPTURE_V0248_DIR").map(PathBuf::from) else {
            return;
        };
        STATE.with(|cell| {
            let mut st = cell.borrow_mut();
            if st.1.is_some_and(|t| t.elapsed() < Duration::from_millis(750)) {
                return;
            }
            st.1 = Some(Instant::now());
            while self.receiver.try_recv().is_ok() {}
            self.amoo_ui.snapshot_job = None;
            self.amoo_poll_remaining = 0;
            self.last_amoo_poll = Instant::now();
            self.last_scan = Instant::now();
            self.amoo_gateway_online = true;
            self.busy = false;
            self.initializing = false;
            self.compact_start_pending = false;
            self.workspace.writer = None;

            let save_frame = |ctx: &mut ViewContext<Self>, filename: &str| {
                let path = dir.join(filename);
                if let Some(w) = ctx.windows().platform_window(ctx.window_id()) {
                    w.as_ctx().request_frame_capture(Box::new(move |frame| {
                        image::save_buffer(
                            &path,
                            &frame.data,
                            frame.width,
                            frame.height,
                            image::ColorType::Rgba8,
                        )
                        .expect("save v0.2.48 screenshot");
                    }));
                }
            };

            match st.0 {
                0 => {
                    let _ = std::fs::create_dir_all(&dir);
                    let adb = Adb::discover().expect("adb");
                    let device =
                        Device::inspect(&adb, &std::env::var("STEPX_WORKSPACE_TEST_SERIAL").expect("explicit test device")).expect("inspect device");
                    let summary = device.summary(&adb).unwrap_or_default();
                    let png = adb
                        .screenshot(&device.transport)
                        .expect("capture explicitly selected test phone");
                    let shot_path = dir.join("raw-v0248-phone-screen.png");
                    std::fs::write(&shot_path, &png).expect("write phone screenshot");
                    st.2 = Some(shot_path.clone());

                    self.settings.value.reduced_motion = true;
                    self.settings.draft.reduced_motion = true;
                    self.adb_guide_step = None;
                    self.connect_guide_step = None;
                    self.selected = Some(device.clone());
                    self.summary = summary;
                    self.screenshot = Some((
                        tempfile::tempdir().expect("tempdir"),
                        shot_path.clone(),
                    ));
                    // Populate gui_screenshot while gui_running/gui_paused are false to verify
                    // no phantom "正在操作手机" card appears in idle conversations.
                    self.amoo_ui.gui_running = false;
                    self.amoo_ui.gui_paused = false;
                    self.amoo_ui.gui_screenshot = Some(shot_path);

                    let weather_reply = "现在上海天气晴，气温 17℃，体感 18℃，吹西北风 2 级，湿度 74%，空气质量为优。\n\n今天上海是晴天，最高 22℃、最低 16℃，北风 2 级，全天降水概率 0%，适合出门；早晚温差有 6℃，可以穿薄外套，紫外线偏弱，但还是建议适当防晒。最近几天也是晴到多云为主，一直到 28 日都干爽；29 日会有小雨，降水概率 60%，可以提前准备好雨具。";
                    self.amoo_sessions = vec![
                        AmooSessionItem {
                            session_key: "agent:main:shanghai-weather".into(),
                            title: "上海天气".into(),
                            agent: "Amoo 助手".into(),
                            status: "已完成".into(),
                            status_kind: 2,
                            meta: "10-26 08:39".into(),
                            items: vec![
                                AmooChatItem::UserText {
                                    attachment: None,
                                    text: "上海天气".into(),
                                },
                                AmooChatItem::ToolRecords {
                                    key: "shanghai-weather-tools".into(),
                                    records: vec![
                                        amoo_display::ToolRecord {
                                            name: "ability".into(),
                                            time_label: "08:39".into(),
                                            detail: None,
                                        },
                                        amoo_display::ToolRecord {
                                            name: "ability".into(),
                                            time_label: "08:39".into(),
                                            detail: None,
                                        },
                                        amoo_display::ToolRecord {
                                            name: "ability".into(),
                                            time_label: "08:39".into(),
                                            detail: None,
                                        },
                                    ],
                                },
                                AmooChatItem::AssistantRich {
                                    thinking_time: String::new(),
                                    thinking_steps: vec![],
                                    tools_summary: String::new(),
                                    text: Some(weather_reply.into()),
                                    has_bolt_artifact: false,
                                    has_invoice_artifact: false,
                                    has_safety_confirm: false,
                                    time_label: "10-26 08:39".into(),
                                },
                            ],
                        },
                        AmooSessionItem {
                            session_key: "agent:main:schedule-sync".into(),
                            title: "整理本周手机日程与闹钟提醒".into(),
                            agent: "Amoo 助手".into(),
                            status: "已完成".into(),
                            status_kind: 2,
                            meta: "10-26 10:15".into(),
                            items: vec![
                                AmooChatItem::UserText {
                                    attachment: None,
                                    text: "帮我核对一下今天和明天的手机日程安排".into(),
                                },
                                AmooChatItem::AssistantRich {
                                    thinking_time: String::new(),
                                    thinking_steps: vec![],
                                    tools_summary: String::new(),
                                    text: Some("已为您核对手机日程：今天 15:00 有产品体验评审，明天 10:00 有晨会提醒，均已同步至桌面日历。".into()),
                                    has_bolt_artifact: false,
                                    has_invoice_artifact: false,
                                    has_safety_confirm: false,
                                    time_label: "10-26 10:15".into(),
                                },
                            ],
                        },
                    ];

                    let mut weather_task = Task::new(None, None);
                    weather_task.title = "上海天气".into();
                    weather_task.session = Some("agent:main:shanghai-weather".into());
                    weather_task.updated = 1_761_468_000;
                    weather_task.turns = vec![
                        Turn {
                            user: true,
                            text: "上海天气".into(),
                        },
                        Turn {
                            user: false,
                            text: weather_reply.into(),
                        },
                    ];
                    weather_task.steps = vec![Step {
                        executor: Executor::Phone,
                        phase: Phase::Done,
                        note: "已完成".into(),
                    }];
                    let weather_id = weather_task.id.clone();

                    let mut schedule_task = Task::new(None, None);
                    schedule_task.title = "整理本周手机日程与闹钟提醒".into();
                    schedule_task.session = Some("agent:main:schedule-sync".into());
                    schedule_task.updated = 1_761_464_000;
                    schedule_task.turns = vec![
                        Turn {
                            user: true,
                            text: "帮我核对一下今天和明天的手机日程安排".into(),
                        },
                        Turn {
                            user: false,
                            text: "已为您核对手机日程：今天 15:00 有产品体验评审，明天 10:00 有晨会提醒，均已同步至桌面日历。".into(),
                        },
                    ];
                    schedule_task.steps = vec![Step {
                        executor: Executor::Phone,
                        phase: Phase::Done,
                        note: "已完成".into(),
                    }];
                    let schedule_id = schedule_task.id.clone();

                    let mut doc_task = Task::new(None, None);
                    doc_task.title = "季度经营汇报 PPT 大纲".into();
                    doc_task.updated = 1_761_460_000;
                    doc_task.turns = vec![
                        Turn {
                            user: true,
                            text: "帮我列一份季度经营汇报大纲".into(),
                        },
                        Turn {
                            user: false,
                            text: "已为您整理包含核心指标、业务进展与下季度规划的汇报大纲。".into(),
                        },
                    ];
                    doc_task.steps = vec![Step {
                        executor: Executor::Phone,
                        phase: Phase::Done,
                        note: "已完成".into(),
                    }];

                    st.3 = weather_id.clone();
                    st.4 = schedule_id;
                    self.workspace.journal.tasks = vec![weather_task, schedule_task, doc_task];
                    self.workspace.active = Some(weather_id);
                    self.amoo_session_idx = 0;
                    self.page = Page::Amoo;
                    self.push.shell_mode = ShellMode::MiniAi;
                    self.push.clipboard_preview = "dshfind.com".into();
                    self.status = "已刷新后台对话".into();
                    self.error = false;
                    assert!(
                        self.workspace_gui_pip_card().is_none(),
                        "idle conversation must NOT show '正在操作手机' PIP card"
                    );
                    if let Some(b) = ctx.window_bounds(&ctx.window_id()) {
                        ctx.windows().set_window_bounds(
                            ctx.window_id(),
                            pathfinder_geometry::rect::RectF::new(
                                b.origin(),
                                vec2f(780.0, 960.0),
                            ),
                        );
                    }
                    self.workspace.viewport = vec2f(780.0, 960.0);
                }
                1 => {
                    assert!(self.workspace_gui_pip_card().is_none());
                    save_frame(ctx, "stepx-v0.2.48-mini-ai-shanghai-weather-clean.png");
                }
                2 => {
                    let shot_path = st.2.clone().expect("shot_path");
                    self.amoo_ui.gui_running = true;
                    self.amoo_ui.gui_paused = false;
                    self.amoo_ui.gui_current = Some((
                        "天气".into(),
                        "打开手机天气应用，查看上海未来三天天气预报".into(),
                    ));
                    self.amoo_ui.gui_steps = vec![(
                        1,
                        "已在手机主屏幕定位「天气」应用图标，正在执行点击".into(),
                    )];
                    self.amoo_ui.gui_screenshot = Some(shot_path);
                    self.amoo_ui.gui_started = Some(Instant::now());
                    self.status = "正在操作手机：天气".into();
                    assert!(
                        self.workspace_gui_pip_card().is_some(),
                        "active GUI task must render '正在操作手机' PIP card"
                    );
                }
                3 => {
                    save_frame(ctx, "raw-v0248-gui-1-running.png");
                }
                4 => {
                    self.handle_amoo_ui_action(&super::amoo_ui_state::AmooUiAction::GuiPauseResume);
                    assert!(self.amoo_ui.gui_paused);
                    assert_eq!(self.status, "已暂停手机操作");
                    assert!(self.workspace_gui_pip_card().is_some());
                }
                5 => {
                    save_frame(ctx, "raw-v0248-gui-2-paused.png");
                }
                6 => {
                    self.handle_amoo_ui_action(&super::amoo_ui_state::AmooUiAction::GuiStopWaiting);
                    assert!(!self.amoo_ui.gui_running && !self.amoo_ui.gui_paused);
                    assert!(self.amoo_ui.gui_screenshot.is_none());
                    assert!(
                        self.workspace_gui_pip_card().is_none(),
                        "stopping GUI task must immediately remove PIP card"
                    );
                    assert_eq!(self.status, "已停止手机操作");
                }
                7 => {
                    save_frame(ctx, "raw-v0248-gui-3-stopped.png");
                }
                8 => {
                    let weather_id = st.3.clone();
                    let schedule_id = st.4.clone();
                    self.push.shell_mode = ShellMode::Full;
                    self.page = Page::Amoo;
                    self.workspace.left_rail_open = true;
                    self.workspace.history = false;
                    self.workspace.panel = 0;
                    // Exercise session switching while screen_mode was active
                    self.amoo_ui.screen_mode = true;
                    let _ = self.workspace_select_task(&schedule_id);
                    assert!(!self.amoo_ui.screen_mode);
                    assert!(self.workspace_gui_pip_card().is_none());
                    let _ = self.workspace_select_task(&weather_id);
                    assert!(!self.amoo_ui.screen_mode);
                    assert!(self.workspace_gui_pip_card().is_none());
                    self.status = "已刷新后台对话".into();
                    if let Some(b) = ctx.window_bounds(&ctx.window_id()) {
                        ctx.windows().set_window_bounds(
                            ctx.window_id(),
                            pathfinder_geometry::rect::RectF::new(
                                b.origin(),
                                vec2f(1480.0, 960.0),
                            ),
                        );
                    }
                    self.workspace.viewport = vec2f(1480.0, 960.0);
                }
                9 => {
                    assert!(self.workspace_gui_pip_card().is_none());
                    save_frame(ctx, "stepx-v0.2.48-amoo-full-workbench-switch.png");
                }
                10 => {
                    let p1 = image::open(dir.join("raw-v0248-gui-1-running.png"))
                        .expect("open panel 1")
                        .to_rgba8();
                    let p2 = image::open(dir.join("raw-v0248-gui-2-paused.png"))
                        .expect("open panel 2")
                        .to_rgba8();
                    let p3 = image::open(dir.join("raw-v0248-gui-3-stopped.png"))
                        .expect("open panel 3")
                        .to_rgba8();
                    let gap = 16u32;
                    let w = p1.width() + gap + p2.width() + gap + p3.width();
                    let h = p1.height().max(p2.height()).max(p3.height());
                    let mut out = image::RgbaImage::from_pixel(
                        w,
                        h,
                        image::Rgba([244, 244, 247, 255]),
                    );
                    image::imageops::replace(&mut out, &p1, 0, 0);
                    image::imageops::replace(&mut out, &p2, (p1.width() + gap) as i64, 0);
                    image::imageops::replace(
                        &mut out,
                        &p3,
                        (p1.width() + gap + p2.width() + gap) as i64,
                        0,
                    );
                    out.save(dir.join("stepx-v0.2.48-amoo-pause-stop-lifecycle.png"))
                        .expect("save 3-panel lifecycle composite");
                    for temp_name in [
                        "raw-v0248-gui-1-running.png",
                        "raw-v0248-gui-2-paused.png",
                        "raw-v0248-gui-3-stopped.png",
                        "raw-v0248-phone-screen.png",
                        "raw-v0248-journal.json",
                    ] {
                        let _ = std::fs::remove_file(dir.join(temp_name));
                    }
                    std::process::exit(0);
                }
                _ => {}
            }
            st.0 += 1;
            ctx.notify();
        });
    }

    #[cfg(all(feature = "ui-capture", not(target_os = "macos")))]
    fn tick_v0247_real_device_capture(&mut self, ctx: &mut ViewContext<Self>) {
        thread_local! {
            static STATE: std::cell::RefCell<(usize, Option<Instant>)> =
                const { std::cell::RefCell::new((0, None)) };
        }
        let Some(dir) = std::env::var_os("STEPX_CAPTURE_V0247_DIR").map(PathBuf::from) else {
            return;
        };
        STATE.with(|cell| {
            let mut st = cell.borrow_mut();
            if st
                .1
                .is_some_and(|t| t.elapsed() < Duration::from_millis(650))
            {
                return;
            }
            st.1 = Some(Instant::now());
            // Clear any background jobs so they do not clobber our deterministic capture sequence
            while self.receiver.try_recv().is_ok() {}
            self.busy = false;
            self.initializing = false;

            let save_frame = |ctx: &mut ViewContext<Self>, filename: &str| {
                let path = dir.join(filename);
                if let Some(w) = ctx.windows().platform_window(ctx.window_id()) {
                    w.as_ctx().request_frame_capture(Box::new(move |frame| {
                        image::save_buffer(
                            &path,
                            &frame.data,
                            frame.width,
                            frame.height,
                            image::ColorType::Rgba8,
                        )
                        .expect("save screenshot");
                    }));
                }
            };

            match st.0 {
                0 => {
                    let _ = std::fs::create_dir_all(&dir);
                    let adb = Adb::discover().expect("adb");
                    let device = Device::inspect(
                        &adb,
                        &std::env::var("STEPX_WORKSPACE_TEST_SERIAL")
                            .expect("explicit test device"),
                    )
                    .expect("inspect device");
                    let summary = device.summary(&adb).unwrap_or_default();
                    let gallery = stepx_companion::toolbox::list_phone_gallery(&adb, &device)
                        .expect("list_phone_gallery");
                    let mut display_gallery = gallery;
                    let top_showcase = [
                        ("expansion", "P20261019-121623.jpg"),
                        ("camera", "P20261019-192404.jpg"),
                        ("expansion", "P20261019-121040.jpg"),
                        ("camera", "P20261021-180251.jpg"),
                        ("expansion", "P20261019-114426.jpg"),
                        ("screenshot", "S61024-11414705_com.android.settings.png"),
                        ("expansion", "P20261018-233911.jpg"),
                        ("camera", "P20261021-175857.jpg"),
                        ("expansion", "P20261018-190048.jpg"),
                    ];
                    let mut insert_idx = 0usize;
                    for (cat, name) in top_showcase {
                        if let Some(pos) = display_gallery
                            .iter()
                            .position(|i| i.category == cat && i.name == name)
                        {
                            let item = display_gallery.remove(pos);
                            let target = insert_idx.min(display_gallery.len());
                            display_gallery.insert(target, item);
                            insert_idx += 1;
                        }
                    }
                    let apps = stepx_companion::apps::catalog(&adb, &device).expect("apps");
                    let push_snap =
                        stepx_companion::push::fetch_snapshot(&adb, &device.transport, 0)
                            .expect("push_snap");
                    let caps = stepx_companion::bridge::capabilities(&adb, &device).ok();
                    let call_logs = device.call_logs(&adb).expect("call_logs");
                    let contacts = device.contacts(&adb).expect("contacts");
                    let stepos = stepx_companion::toolbox::inspect_stepos_ai_suite(&adb, &device)
                        .expect("stepos");

                    self.settings.value.reduced_motion = true;
                    self.settings.draft.reduced_motion = true;
                    self.adb_guide_step = None;
                    self.connect_guide_step = None;
                    self.compact_start_pending = false;
                    self.push.shell_mode = ShellMode::Full;
                    self.selected = Some(device.clone());
                    self.summary = summary;
                    self.status = format!("已连接 {}", device.model);
                    self.apply_capabilities(caps);
                    self.apply_phone_data(Ok(call_logs), Ok(contacts));
                    self.apps = apps;
                    self.apps_loaded = true;
                    self.push.snapshot = push_snap.clone();
                    self.push.loaded = true;
                    self.push.selected_key = push_snap.notifications.first().map(|n| n.key.clone());
                    self.back_screen.stepos_suite = stepos;
                    self.back_screen.stepos_suite.checked = true;
                    self.back_screen.stepos_suite_loading = false;
                    self.back_screen.gallery_items = display_gallery;
                    self.back_screen.gallery_checked = true;
                    self.back_screen.gallery_loading = false;
                    self.back_screen.gallery_category = 0;
                    self.back_screen.gallery_selected = 0;
                    self.back_screen.gallery_lightbox = false;
                    self.sync_gallery_image_previews();
                    self.page = Page::Gallery;
                    if let Some(b) = ctx.window_bounds(&ctx.window_id()) {
                        ctx.windows().set_window_bounds(
                            ctx.window_id(),
                            pathfinder_geometry::rect::RectF::new(
                                b.origin(),
                                vec2f(1480.0, 1180.0),
                            ),
                        );
                    }
                }
                1 => {
                    save_frame(ctx, "stepx-v0.2.47-gallery-expansion.png");
                }
                2 => {
                    self.page = Page::Gallery;
                    self.back_screen.gallery_category = 5;
                    self.back_screen.gallery_selected = self
                        .back_screen
                        .gallery_items
                        .iter()
                        .position(|i| i.category == "expansion" && i.name == "P20261019-121623.jpg")
                        .unwrap_or(0);
                    self.back_screen.gallery_lightbox = true;
                    self.back_screen.gallery_zoom_pct = 100;
                    self.sync_gallery_image_previews();
                    self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.0));
                }
                3 => {
                    save_frame(ctx, "stepx-v0.2.47-gallery-lightbox-expansion.png");
                }
                4 => {
                    self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.0));
                    self.back_screen.gallery_lightbox = false;
                    self.page = Page::Apps;
                    self.include_system_apps = false;
                }
                5 => {
                    save_frame(ctx, "raw-apps.png");
                }
                6 => {
                    self.page = Page::Notifications;
                }
                7 => {
                    save_frame(ctx, "raw-notifications.png");
                }
                8 => {
                    self.page = Page::Phone;
                }
                9 => {
                    save_frame(ctx, "raw-phone.png");
                }
                10 => {
                    self.page = Page::Calendar;
                }
                11 => {
                    save_frame(ctx, "raw-calendar.png");
                }
                12 => {
                    let stitch = |left_name: &str, right_name: &str, out_name: &str| {
                        let left = image::open(dir.join(left_name))
                            .expect("open left")
                            .to_rgba8();
                        let right = image::open(dir.join(right_name))
                            .expect("open right")
                            .to_rgba8();
                        let gap = 16u32;
                        let w = left.width() + gap + right.width();
                        let h = left.height().max(right.height());
                        let mut out =
                            image::RgbaImage::from_pixel(w, h, image::Rgba([244, 244, 247, 255]));
                        image::imageops::replace(&mut out, &left, 0, 0);
                        image::imageops::replace(&mut out, &right, (left.width() + gap) as i64, 0);
                        out.save(dir.join(out_name)).expect("save composite");
                        let _ = std::fs::remove_file(dir.join(left_name));
                        let _ = std::fs::remove_file(dir.join(right_name));
                    };
                    stitch(
                        "raw-apps.png",
                        "raw-notifications.png",
                        "stepx-v0.2.47-apps-notifications-hygiene.png",
                    );
                    stitch(
                        "raw-phone.png",
                        "raw-calendar.png",
                        "stepx-v0.2.47-phone-schedule-hygiene.png",
                    );
                    std::process::exit(0);
                }
                _ => {}
            }
            st.0 += 1;
            ctx.notify();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn workspace_initializes_skills_and_memory_and_preserves_across_amoo_sync() {
        let mut app = Companion::new_for_test();
        assert!(app.amoo_memory_enabled);
        assert!(app.amoo_memories.len() >= 2);
        assert_eq!(app.amoo_skills.len(), 6);

        // Simulate user toggling a skill and keeping memory enabled
        app.amoo_skills[2].enabled = true;
        let snapshot = stepx_companion::openclaw::RealAmooData {
            gateway_online: true,
            gateway_version: "2026.2.15".into(),
            account_scope: Some("acct-test".into()),
            ..Default::default()
        };
        app.apply_real_amoo_data_recorded(snapshot);
        assert!(
            app.amoo_memory_enabled,
            "empty snapshot must not clobber amoo_memory_enabled"
        );
        assert_eq!(
            app.amoo_skills.len(),
            6,
            "empty snapshot skills must not wipe default/catalog skills"
        );
        assert!(app.amoo_skills[2].enabled);
    }

    #[test]
    fn work_catalog_syncs_installed_skills_into_amoo_skills() {
        let mut app = Companion::new_for_test();
        let mut catalog = stepx_companion::amoo_catalog::WorkCatalog::default();
        catalog
            .skills
            .push(stepx_companion::amoo_catalog::SkillInfo {
                id: "stepx_custom_mcp_skill".into(),
                name: "手机日程智能管家".into(),
                description: "自动同步日历与闹钟".into(),
                installed: true,
                enabled: true,
                eligible: true,
                installers: vec![],
                source: "手机 OpenClaw".into(),
                install_reference: String::new(),
            });
        app.apply_work(super::super::work_state::WorkOutcome::CatalogChanged(
            catalog,
            "已刷新技能目录".into(),
        ));
        assert!(app
            .amoo_skills
            .iter()
            .any(|s| s.skill_key == "stepx_custom_mcp_skill" && s.enabled));
    }

    #[test]
    fn all_workspace_subviews_and_panels_and_pages_render_cleanly() {
        let mut app = Companion::new_for_test();
        for sub_view in [0u8, 1, 2] {
            app.workspace.sub_view = sub_view;
            let _ = app.workspace_page(false);
            let _ = app.workspace_page(true);
        }
        app.workspace.sub_view = 0;
        for panel in [0u8, 1, 2, 3, 4, 5, 6, 7, 8, 9] {
            app.workspace.panel = panel;
            let _ = app.workspace_page(false);
        }
        let _ = app.overview_page();
        let _ = app.backscreen_page();
        let _ = app.gallery_page();
        app.back_screen.gallery_lightbox = true;
        app.back_screen.gallery_zoom_pct = 200;
        let _ = app.gallery_page();
        app.back_screen.gallery_lightbox = false;
        app.expansion.gallery_grid_mode = false;
        let _ = app.expansion_page();
        app.expansion.gallery_grid_mode = true;
        let _ = app.expansion_page();
        let _ = app.files_page();
        let _ = app.sms_page();
        let _ = app.phone_page();
        let _ = app.notifications_page();
        let _ = app.schedule_page();
        let _ = app.apps_page();
        let _ = app.studio_page();
        let _ = app.studio_lab_page();
        let _ = app.settings_page();
        let _ = app.tools_page();
        for tool in super::super::ToolPage::ALL {
            app.tool_page = Some(tool);
            let _ = app.tools_page();
        }
        assert_eq!(app.amoo_memories.len(), 3);
        assert!(app.amoo_memories.iter().any(|m| m.sub.contains("交流偏好")));
        assert!(app.amoo_memories.iter().any(|m| m.sub.contains("关于我")));
        assert!(app.amoo_memories.iter().any(|m| m.sub.contains("备忘事项")));
    }

    #[test]
    fn unconfirmed_and_background_phone_tasks_auto_reconcile_when_phone_reply_arrives() {
        let mut app = Companion::new_for_test();
        let mut task = Task::new(None, None);
        task.session = Some("agent:main:reconcile-test".into());
        task.turns = vec![
            Turn {
                user: true,
                text: "今天北京天气怎么样".into(),
            },
            Turn {
                user: false,
                text: "已停止等待；手机任务可能仍继续，可同步查看结果".into(),
            },
        ];
        task.steps = vec![Step {
            executor: Executor::Phone,
            phase: Phase::Unconfirmed,
            note: "已停止等待；手机任务可能仍继续，可同步查看结果".into(),
        }];
        let task_id = task.id.clone();
        app.workspace.journal.tasks.push(task);
        app.workspace.active = Some(task_id.clone());

        // Pending sync keys must include the unconfirmed task's session
        assert_eq!(
            app.workspace_sync_pending_keys(),
            vec!["agent:main:reconcile-test".to_string()]
        );

        let real = stepx_companion::openclaw::RealAmooData {
            gateway_online: true,
            gateway_version: "2026.2.15".into(),
            sessions: vec![stepx_companion::openclaw::RealSessionInfo {
                key: "agent:main:reconcile-test".into(),
                title: "北京天气".into(),
                status: "已完成".into(),
                status_kind: 2,
                history_loaded: true,
                messages: vec![
                    stepx_companion::openclaw::RealChatMessage {
                        role: "user".into(),
                        text: "今天北京天气怎么样".into(),
                        run_id: "r1".into(),
                        ..Default::default()
                    },
                    stepx_companion::openclaw::RealChatMessage {
                        role: "assistant".into(),
                        text: "今天北京晴朗，最高气温 22°C。".into(),
                        run_id: "r1".into(),
                        stop_reason: "STOP".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };

        let count = app.reconcile_workspace_with_real_data(&real);
        assert_eq!(count, 1);

        let updated = app
            .workspace
            .journal
            .tasks
            .iter()
            .find(|t| t.id == task_id)
            .unwrap();
        assert_eq!(updated.steps[0].phase, Phase::Done);
        assert_eq!(updated.steps[0].note, "今天北京晴朗，最高气温 22°C。");
        assert_eq!(updated.turns.len(), 2);
        assert_eq!(updated.turns[1].text, "今天北京晴朗，最高气温 22°C。");
        assert!(app.workspace_sync_pending_keys().is_empty());

        // 2. Test background Phase::Running task detachment, reattach, and auto-reconciliation
        let mut bg_task = Task::new(None, None);
        bg_task.session = Some("agent:main:bg-running-test".into());
        bg_task.turns = vec![Turn {
            user: true,
            text: "帮我查一下上海明天的天气".into(),
        }];
        bg_task.steps = vec![Step {
            executor: Executor::Phone,
            phase: Phase::Running,
            note: String::new(),
        }];
        let bg_id = bg_task.id.clone();
        app.workspace.journal.tasks.push(bg_task);
        app.amoo_pending_reply.insert(
            "agent:main:bg-running-test".into(),
            amoo::PendingReply {
                key: "agent:main:bg-running-test".into(),
                text: "帮我查一下上海明天的天气".into(),
                run_id: "run-bg-1".into(),
            },
        );

        // Switching to the backgrounded task while pending reattaches `workspace.running`
        app.workspace.active = Some(bg_id.clone());
        app.workspace.running = None;
        app.workspace_reattach_running_if_pending();
        assert!(app
            .workspace
            .running
            .as_ref()
            .is_some_and(|r| r.task == bg_id && r.accepted));

        // Detach again to simulate user viewing another session while bg_task runs in background
        app.workspace.running = None;
        app.workspace.active = Some(task_id.clone());
        assert_eq!(
            app.workspace_sync_pending_keys(),
            vec!["agent:main:bg-running-test".to_string()]
        );

        // Intermediate non-terminal tool call must keep background task in Phase::Running
        let real_intermediate = stepx_companion::openclaw::RealAmooData {
            gateway_online: true,
            gateway_version: "2026.2.15".into(),
            sessions: vec![stepx_companion::openclaw::RealSessionInfo {
                key: "agent:main:bg-running-test".into(),
                title: "上海天气".into(),
                status: "进行中".into(),
                status_kind: 1,
                history_loaded: true,
                messages: vec![
                    stepx_companion::openclaw::RealChatMessage {
                        role: "user".into(),
                        text: "帮我查一下上海明天的天气".into(),
                        run_id: "run-bg-1".into(),
                        ..Default::default()
                    },
                    stepx_companion::openclaw::RealChatMessage {
                        role: "assistant".into(),
                        text: "正在查询天气服务…".into(),
                        run_id: "run-bg-1".into(),
                        stop_reason: "TOOL_CALLS".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(
            app.reconcile_workspace_with_real_data(&real_intermediate),
            0
        );
        let bg_still_running = app
            .workspace
            .journal
            .tasks
            .iter()
            .find(|t| t.id == bg_id)
            .unwrap();
        assert_eq!(bg_still_running.steps[0].phase, Phase::Running);

        // Final reply arrives while task is still in the background
        let real_bg_done = stepx_companion::openclaw::RealAmooData {
            gateway_online: true,
            gateway_version: "2026.2.15".into(),
            sessions: vec![stepx_companion::openclaw::RealSessionInfo {
                key: "agent:main:bg-running-test".into(),
                title: "上海天气".into(),
                status: "已完成".into(),
                status_kind: 2,
                history_loaded: true,
                messages: vec![
                    stepx_companion::openclaw::RealChatMessage {
                        role: "user".into(),
                        text: "帮我查一下上海明天的天气".into(),
                        run_id: "run-bg-1".into(),
                        ..Default::default()
                    },
                    stepx_companion::openclaw::RealChatMessage {
                        role: "assistant".into(),
                        text: "上海明天多云转晴，18~23°C。".into(),
                        run_id: "run-bg-1".into(),
                        stop_reason: "STOP".into(),
                        ..Default::default()
                    },
                ],
                ..Default::default()
            }],
            ..Default::default()
        };
        assert_eq!(app.reconcile_workspace_with_real_data(&real_bg_done), 1);
        let bg_finished = app
            .workspace
            .journal
            .tasks
            .iter()
            .find(|t| t.id == bg_id)
            .unwrap();
        assert_eq!(bg_finished.steps[0].phase, Phase::Done);
        assert_eq!(bg_finished.steps[0].note, "上海明天多云转晴，18~23°C。");
        assert_eq!(bg_finished.turns.len(), 2);
        assert_eq!(bg_finished.turns[1].text, "上海明天多云转晴，18~23°C。");
        assert!(!app
            .amoo_pending_reply
            .contains_key("agent:main:bg-running-test"));
        assert!(app.workspace_sync_pending_keys().is_empty());

        // 3. Test missing/idle session reconciliation and WorkspaceAction::DismissUnconfirmed
        let mut stale_task = Task::new(None, None);
        stale_task.session = Some("agent:main:missing-session".into());
        stale_task.note = "已停止等待；手机任务可能仍继续，可同步查看结果".into();
        stale_task.turns = vec![
            Turn {
                user: true,
                text: "未到达手机的测试提问".into(),
            },
            Turn {
                user: false,
                text: "已停止等待；手机任务可能仍继续，可同步查看结果".into(),
            },
        ];
        stale_task.steps = vec![Step {
            executor: Executor::Phone,
            phase: Phase::Unconfirmed,
            note: "已停止等待；手机任务可能仍继续，可同步查看结果".into(),
        }];
        let stale_id = stale_task.id.clone();
        app.workspace.journal.tasks.push(stale_task);
        app.workspace.active = Some(stale_id.clone());
        assert_eq!(
            app.workspace_sync_pending_keys(),
            vec!["agent:main:missing-session".to_string()]
        );

        // Reconciling against an online gateway without that session strips the placeholder turn
        // and marks the session reconciled so background sync does not loop forever.
        assert_eq!(app.reconcile_workspace_with_real_data(&real_bg_done), 0);
        assert!(app.workspace_sync_pending_keys().is_empty());
        let stale_after_sync = app
            .workspace
            .journal
            .tasks
            .iter()
            .find(|t| t.id == stale_id)
            .unwrap();
        assert_eq!(stale_after_sync.turns.len(), 1);
        assert!(stale_after_sync.steps[0].note.contains("已与手机核对"));

        // DismissUnconfirmed clears the unconfirmed banner and marks the step Done
        app.workspace_dismiss_unconfirmed();
        let dismissed = app
            .workspace
            .journal
            .tasks
            .iter()
            .find(|t| t.id == stale_id)
            .unwrap();
        assert_eq!(dismissed.steps[0].phase, Phase::Done);
        assert_eq!(dismissed.steps[0].note, "已手动核对关闭提示");
        assert!(dismissed.note.is_empty());
        assert_eq!(app.status, "已清除待核对提示，可继续直接对话");
    }

    #[test]
    fn cross_flow_deliverables_and_fusion_cards_render_and_sync_cleanly() {
        let mut app = Companion::new_for_test();
        let dir = tempfile::tempdir().unwrap();
        let artifact = stepx_companion::studio_lab::LabArtifact {
            id: "lab-snake-01".into(),
            name: "贪吃蛇小游戏".into(),
            root: dir.path().to_path_buf(),
            html_path: dir.path().join("index.html"),
            screenshot_path: None,
            report_path: dir.path().join("report.json"),
            passed: true,
            round: 1,
            summary: "键盘与触控方向测试通过".into(),
            created_at: 1700000000,
        };
        app.studio_lab.last_prompt = "做一个贪吃蛇小游戏".into();
        app.studio_lab.artifacts.push(artifact.clone());
        app.workspace_on_app_finished(&artifact);
        assert!(app.workspace.journal.tasks.iter().any(|t| t
            .outputs
            .iter()
            .any(|o| o.lab_id.as_deref() == Some("lab-snake-01"))));

        let task_id = app.workspace.journal.tasks[0].id.clone();
        app.workspace.active = Some(task_id);
        let work_dir = dir.path().join("work-docs");
        std::fs::create_dir_all(&work_dir).unwrap();
        let docx_path = work_dir.join("report.docx");
        std::fs::write(&docx_path, b"demo").unwrap();
        if let Some(t) = app.workspace_task_mut() {
            t.workspace = Some(work_dir.clone());
            t.add_output(Output {
                path: docx_path,
                name: "report.docx".into(),
                kind: "Word 文档".into(),
                lab_id: None,
            });
        }
        app.workspace_on_work_finished();
        assert_eq!(app.work.workspace, Some(work_dir));

        let _ = app.workspace_page(false);
        let _ = app.workspace_page(true);
    }

    #[test]
    fn auto_default_inline_deliver_and_reroute_update_task_state_cleanly() {
        let mut app = Companion::new_for_test();
        assert_eq!(app.workspace.intent, Intent::Auto);

        let dir = tempfile::tempdir().unwrap();
        let doc_path = dir.path().join("summary.docx");
        std::fs::write(&doc_path, b"docx").unwrap();
        let mut task = Task::new(None, None);
        task.route = Some(Intent::Files);
        task.route_note = "生成 Word 文档".into();
        task.turns = vec![
            Turn {
                user: true,
                text: "生成一份季度报告 Word 文档".into(),
            },
            Turn {
                user: false,
                text: "正在处理".into(),
            },
        ];
        task.steps = vec![Step {
            executor: Executor::Files,
            phase: Phase::Running,
            note: String::new(),
        }];
        task.add_output(Output {
            path: doc_path.clone(),
            name: "summary.docx".into(),
            kind: "Word 文档".into(),
            lab_id: None,
        });
        let task_id = task.id.clone();
        app.workspace.journal.tasks.push(task);
        app.workspace.active = Some(task_id.clone());

        // Deliver(false) stays on panel 0 so the inline confirm_bar renders under the card
        app.workspace.selected_output = Some(0);
        app.workspace.panel = 0;
        let _ = app.workspace_page(false);
        app.workspace.delivery = Some(false);
        assert_eq!(app.workspace.panel, 0);

        // Reroute(Intent::Chat) truncates old assistant reply without duplicating user turn,
        // clears route_note and sticky route, and sets steps to [Executor::Phone]
        let t = app.workspace_task_mut().unwrap();
        let last_user_idx = t.turns.iter().rposition(|turn| turn.user).unwrap();
        t.turns.truncate(last_user_idx + 1);
        let decision = stepx_companion::amoo_route::route_intent(
            &t.turns[last_user_idx].text,
            &t.submitted_files,
            &stepx_companion::amoo_route::RouteCtx {
                forced: Some(Intent::Chat),
                sticky: t.route,
                last: None,
            },
        );
        t.apply_route(&decision);
        t.steps = decision
            .steps
            .iter()
            .map(|e| Step {
                executor: *e,
                phase: Phase::Ready,
                note: String::new(),
            })
            .collect();
        let updated = app.workspace_task().unwrap();
        assert_eq!(updated.turns.len(), 1);
        assert!(updated.route_note.is_empty());
        assert_eq!(updated.route, None);
        assert_eq!(updated.steps.len(), 1);
        assert_eq!(updated.steps[0].executor, Executor::Phone);
    }

    #[test]
    fn deep_human_centric_features_and_actions_work_end_to_end() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;

        // 1. GUI Agent: optional app field & instant phone command routing
        app.page = Page::Overview;
        app.handle_amoo_ui_action(&super::amoo_ui_state::AmooUiAction::GuiMode(true));
        assert_eq!(app.page, Page::Amoo);
        assert!(app.amoo_ui.screen_mode);
        let (inferred_app, inferred_task) =
            stepx_companion::amoo_screen::infer_app_and_task("", "打开微信发消息");
        assert_eq!(inferred_app, "微信");
        assert_eq!(inferred_task, "打开微信发消息");
        let (inferred_default_app, _) =
            stepx_companion::amoo_screen::infer_app_and_task("", "提取屏幕文字");
        assert_eq!(inferred_default_app, "当前屏幕");
        assert!(
            stepx_companion::amoo_route::parse_instant_phone_control("帮我点亮手机屏幕").is_some()
        );
        let _ = app.amoo_page();

        // 2. Backscreen local badge synthesis & DIY pixel matrix
        let (title, frame) = stepx_companion::backscreen::synthesize_prompt_frame("会议中");
        assert!(title.contains("会议"));
        assert!(frame.iter().any(|&v| v > 0));
        let (cat_title, cat_frame) = stepx_companion::backscreen::synthesize_prompt_frame("猫咪");
        assert!(cat_title.contains("猫"));
        assert!(cat_frame.iter().any(|&v| v > 0));
        app.back_screen.source = stepx_companion::backscreen::Source::Image(frame);
        let _ = app.backscreen_page();

        // 3. Studio deliverable filters & sort modes
        for filter in 0..=5 {
            app.studio.deliverable_filter = filter;
            let _ = app.studio_page();
        }
        for sort in 0..=2 {
            app.studio.sort_mode = sort;
            let _ = app.studio_page();
        }

        // 4. Settings & Studio Lab pages
        let _ = app.settings_page();
        let _ = app.studio_lab_page();

        // 5. Schedule in-place edit state
        app.back_screen.editing_calendar_event_id = Some(42);
        let _ = app.schedule_page();
        app.back_screen.editing_calendar_event_id = None;

        // 6. Gallery categories 0..=4 & Files time sorting
        for cat in 0..=4 {
            app.back_screen.gallery_category = cat;
            let _ = app.filtered_gallery_indices();
            let _ = app.gallery_page();
        }
        app.file_sort = FileSort::Time;
        let _ = app.files_page();

        // 7. SMS pickup code extraction & Notification quick categories
        assert_eq!(
            super::pages::sms::extract_pickup_code(
                "【菜鸟驿站】您的快递已到，凭取件码 5-2-1088 到驿站领取"
            ),
            Some("5-2-1088".into())
        );
        for cat in 0..=3 {
            app.push.notif_category = cat;
            let _ = app.notifications_page();
        }
        let _ = app.sms_page();
        let _ = app.phone_page();
        let _ = app.apps_page();
        let _ = app.overview_page();
        let _ = app.tools_page();
        let _ = app.connect_page();
    }

    #[test]
    fn unified_phone_bezel_and_screen_mirroring_studio_controls_render_cleanly() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.live_screen_stream = true;
        for q in 0..=2 {
            app.mirror_options.quality = q;
            let (max_size, fps, bitrate) = app.mirror_options.quality_params();
            assert!(!max_size.is_empty() && !fps.is_empty() && !bitrate.is_empty());
            app.mirror_options.always_on_top = q == 1;
            app.mirror_options.turn_screen_off = q == 2;
            app.mirror_options.stay_awake = q != 0;
            app.mirror_options.show_touches = q == 2;
            app.page = Page::Tools;
            app.tool_page = Some(ToolPage::Screen);
            let _ = app.tools_page();
        }
        app.page = Page::Overview;
        app.overview_back_preview = false;
        let _ = app.overview_page();
        app.overview_back_preview = true;
        let _ = app.overview_page();
        app.page = Page::Amoo;
        app.amoo_ui.screen_mode = true;
        let _ = app.amoo_page();
    }

    #[test]
    fn gui_pip_card_only_renders_while_running_or_paused_and_stop_aborts_cleanly() {
        let mut app = Companion::new_for_test();
        let dir = tempfile::tempdir().unwrap();
        let shot = dir.path().join("gui-shot.png");
        std::fs::write(&shot, b"png").unwrap();

        // 1. Stale screenshot alone must NOT show the "正在操作手机" PIP card
        app.amoo_ui.gui_running = false;
        app.amoo_ui.gui_paused = false;
        app.amoo_ui.gui_screenshot = Some(shot);
        assert!(app.workspace_gui_pip_card().is_none());

        // When gui_running or gui_paused is true, the PIP card is rendered
        app.amoo_ui.gui_running = true;
        assert!(app.workspace_gui_pip_card().is_some());
        app.amoo_ui.gui_running = false;
        app.amoo_ui.gui_paused = true;
        assert!(app.workspace_gui_pip_card().is_some());

        // 2. Stopping an unaccepted/in-flight phone turn transitions to Phase::Stopped
        // and clears running + amoo_pending_reply
        let mut task = Task::new(None, None);
        task.session = Some("agent:main:stop-test".into());
        task.turns = vec![Turn {
            user: true,
            text: "测试停止".into(),
        }];
        task.steps = vec![Step {
            executor: Executor::Phone,
            phase: Phase::Running,
            note: String::new(),
        }];
        let task_id = task.id.clone();
        app.workspace.journal.tasks.push(task);
        app.workspace.active = Some(task_id.clone());
        app.workspace.running = Some(ActiveRun {
            task: task_id.clone(),
            step: 0,
            executor: Executor::Phone,
            accepted: false,
            started_at: Instant::now(),
        });
        app.amoo_pending_reply.insert(
            "agent:main:stop-test".into(),
            amoo::PendingReply {
                key: "agent:main:stop-test".into(),
                text: "测试停止".into(),
                run_id: "run-stop-1".into(),
            },
        );
        app.workspace_stop();
        assert!(app.workspace.running.is_none());
        assert!(!app.amoo_ui.gui_running && !app.amoo_ui.gui_paused);
        assert!(!app.amoo_pending_reply.contains_key("agent:main:stop-test"));
        assert!(app
            .workspace
            .stopped_sessions
            .contains("agent:main:stop-test"));
        let stopped_task = app.workspace_task().unwrap();
        assert_eq!(stopped_task.steps[0].phase, Phase::Stopped);
        assert_eq!(stopped_task.steps[0].note, "已停止");

        // Reconciling a stopped session must NOT overwrite Phase::Stopped with Running/Unconfirmed
        let real_stale_running = stepx_companion::openclaw::RealAmooData {
            gateway_online: true,
            sessions: vec![stepx_companion::openclaw::RealSessionInfo {
                key: "agent:main:stop-test".into(),
                status: "进行中".into(),
                status_kind: 1,
                history_loaded: true,
                ..Default::default()
            }],
            ..Default::default()
        };
        app.reconcile_workspace_with_real_data(&real_stale_running);
        assert_eq!(app.workspace_task().unwrap().steps[0].phase, Phase::Stopped);
    }

    #[test]
    fn session_switching_exits_screen_mode_and_supports_unaccepted_phone_turns() {
        let mut app = Companion::new_for_test();
        app.amoo_sessions = vec![AmooSessionItem {
            session_key: "agent:main:phone-0".into(),
            title: "手机会话".into(),
            agent: "Amoo 助手".into(),
            status: "空闲".into(),
            status_kind: 0,
            meta: String::new(),
            items: vec![],
        }];
        app.amoo_session_idx = 0;

        let mut phone_task = Task::new(None, None);
        phone_task.session = Some("agent:main:phone-0".into());
        phone_task.steps = vec![Step {
            executor: Executor::Phone,
            phase: Phase::Running,
            note: String::new(),
        }];
        let phone_task_id = phone_task.id.clone();

        let mut local_task = Task::new(None, None);
        local_task.title = "本地应用任务".into();
        local_task.session = None;
        let local_task_id = local_task.id.clone();

        app.workspace.journal.tasks = vec![phone_task, local_task];
        app.workspace.active = Some(phone_task_id);
        app.workspace.running = Some(ActiveRun {
            task: app.workspace.active.clone().unwrap(),
            step: 0,
            executor: Executor::Phone,
            accepted: false,
            started_at: Instant::now(),
        });
        app.amoo_ui.screen_mode = true;

        // Switching to a local-only task clears screen_mode, backgrounds the unaccepted phone turn,
        // and does NOT hijack amoo_session_idx
        let selected_idx = app.workspace_select_task(&local_task_id).unwrap();
        assert_eq!(selected_idx, None);
        assert!(!app.amoo_ui.screen_mode);
        assert!(app.workspace.running.is_none());
        assert_eq!(app.amoo_session_idx, usize::MAX);

        // PhoneSession, New, and History all exit screen_mode
        app.amoo_ui.screen_mode = true;
        app.workspace_select_phone_session(0).unwrap();
        assert!(!app.amoo_ui.screen_mode);
        assert_eq!(app.amoo_session_idx, 0);

        app.amoo_ui.screen_mode = true;
        app.workspace_switch_new().unwrap();
        assert!(!app.amoo_ui.screen_mode);

        app.amoo_ui.screen_mode = true;
        app.workspace_toggle_history();
        assert!(!app.amoo_ui.screen_mode);
    }

    #[test]
    fn v0_2_49_session_switch_never_blocked_by_busy_or_running_and_welcome_preserved() {
        let mut app = Companion::new_for_test();
        let mut t1 = Task::new(None, None);
        t1.title = "把我的行程做成一张周视图，按工作日分组".into();
        t1.session = Some("agent:main:week-schedule".into());
        let t1_id = t1.id.clone();

        let mut t2 = Task::new(None, None);
        t2.title = "帮我写一份季度产品研发汇报提纲".into();
        t2.session = Some("agent:main:q3-outline".into());
        let t2_id = t2.id.clone();

        app.workspace.journal.tasks = vec![t1, t2];
        app.amoo_sessions = vec![
            AmooSessionItem {
                session_key: "agent:main:week-schedule".into(),
                title: "把我的行程做成一张周视图，按工作日分组".into(),
                agent: "Amoo 助手".into(),
                status: "已完成".into(),
                status_kind: 2,
                meta: "刚刚".into(),
                items: vec![],
            },
            AmooSessionItem {
                session_key: "agent:main:q3-outline".into(),
                title: "帮我写一份季度产品研发汇报提纲".into(),
                agent: "Amoo 助手".into(),
                status: "已完成".into(),
                status_kind: 2,
                meta: "刚刚".into(),
                items: vec![],
            },
        ];

        // 1. Even when app.busy = true (background Studio sync / device refresh) AND
        // app.workspace.running is None (Welcome Screen) OR Some(...) (Executor::Files / Executor::Phone),
        // switching tasks, phone sessions, or new conversation ALWAYS succeeds and NEVER sets
        // "任务仍在执行，完成后可以切换记录" or "任务仍在执行，请先等待完成或停止".
        app.busy = true;
        app.status = "正在用手机账号同步 Studio 作品…".into();
        app.workspace.active = None;
        app.amoo_session_idx = usize::MAX;
        app.workspace.running = None;

        assert!(app.workspace_select_task(&t1_id).is_ok());
        assert_eq!(app.workspace.active.as_deref(), Some(t1_id.as_str()));
        assert!(app.workspace.error.is_none());

        app.workspace.running = Some(ActiveRun {
            task: t1_id.clone(),
            step: 0,
            executor: Executor::Files,
            accepted: true,
            started_at: Instant::now(),
        });
        assert!(app.workspace_select_task(&t2_id).is_ok());
        assert_eq!(app.workspace.active.as_deref(), Some(t2_id.as_str()));
        assert!(app.workspace.error.is_none());

        app.workspace.running = Some(ActiveRun {
            task: t2_id.clone(),
            step: 0,
            executor: Executor::Phone,
            accepted: true,
            started_at: Instant::now(),
        });
        assert!(app.workspace_select_phone_session(0).is_ok());
        assert_eq!(app.workspace.active.as_deref(), Some(t1_id.as_str()));
        assert!(app.workspace.error.is_none());

        app.workspace.running = Some(ActiveRun {
            task: t1_id.clone(),
            step: 0,
            executor: Executor::Files,
            accepted: true,
            started_at: Instant::now(),
        });
        assert!(app.workspace_switch_new().is_ok());
        assert!(app.workspace.active.is_none());
        assert!(app.workspace.error.is_none());

        // 2. Even when app.busy = true from a background job, app.workspace_can_send() returns true
        // when F_AMOO has text and the active session is not busy.
        app.busy = true;
        app.fields[F_AMOO].set("帮我整理一份周会纪要");
        assert!(
            app.workspace_can_send(),
            "background app.busy must not block workspace_can_send when active session is idle"
        );

        // 3. When on the Welcome Screen (active == None, amoo_session_idx == usize::MAX),
        // apply_real_amoo_data_recorded(real_data) keeps amoo_session_idx == usize::MAX (never resetting to 0).
        app.workspace.active = None;
        app.amoo_session_idx = usize::MAX;
        let real_data = stepx_companion::openclaw::RealAmooData {
            gateway_online: true,
            account_scope: None,
            sessions: vec![stepx_companion::openclaw::RealSessionInfo {
                key: "agent:main:week-schedule".into(),
                session_id: "week-schedule".into(),
                title: "把我的行程做成一张周视图，按工作日分组".into(),
                agent: "Amoo 助手".into(),
                status: "已完成".into(),
                status_kind: 2,
                meta: "刚刚".into(),
                updated_at_ms: 1_760_000_000_000,
                history_loaded: true,
                messages: vec![],
                ..Default::default()
            }],
            ..Default::default()
        };
        app.apply_real_amoo_data_recorded(real_data);
        assert!(app.workspace.active.is_none());
        assert_eq!(
            app.amoo_session_idx,
            usize::MAX,
            "Welcome Screen must preserve amoo_session_idx == usize::MAX after background Amoo sync"
        );

        // 4. WorkspaceAction::DismissError clears app.workspace.error = None (and app.error = false).
        app.workspace.error = Some("临时错误提示".into());
        app.error = true;
        app.workspace_dismiss_error();
        assert!(app.workspace.error.is_none());
        assert!(!app.error);

        // 5. stepx_companion::amoo_workspace::is_automated_test_task filters out automated E2E fixture tasks.
        let mut fixture1 = Task::new(None, None);
        fixture1.title = "只修改附件 Word：把项目预算从2580改为300".into();
        assert!(stepx_companion::amoo_workspace::is_automated_test_task(
            &fixture1
        ));

        let mut fixture2 = Task::new(None, None);
        fixture2.title = "日常任务".into();
        fixture2.turns.push(Turn {
            user: true,
            text: "这是 StepX Companion 统一任务验收。请输出报告。".into(),
        });
        assert!(stepx_companion::amoo_workspace::is_automated_test_task(
            &fixture2
        ));
    }

    #[test]
    fn v0_2_50_neon_snake_continue_edit_and_app_context_never_os_error_2() {
        for demo_mode in [true] {
            let mut app = Companion::new_for_test();
            app.demo_mode = demo_mode;
            app.studio_lab.loaded = true;
            let isolated = tempfile::tempdir().unwrap();
            let nonexistent_ws = isolated.path().join("missing-workspace");
            let _ = std::fs::remove_dir_all(&nonexistent_ws);

            let snake_html = isolated.path().join("starter-neon-snake/index.html");
            let mut task = Task::new(
                app.selected.as_ref().map(|d| d.serial.clone()),
                app.amoo_ui.account_scope.clone(),
            );
            task.title = "霓虹贪吃蛇大作战".into();
            task.workspace = Some(nonexistent_ws.clone());
            task.outputs = vec![Output {
                path: snake_html,
                name: "霓虹贪吃蛇大作战".into(),
                kind: "应用 · 已检查".into(),
                lab_id: Some("starter-neon-snake".into()),
            }];
            let task_id = task.id.clone();
            app.workspace.journal.tasks = vec![task];
            app.workspace.active = Some(task_id.clone());

            // 1. ContinueEdit(0) sets route = Some(Intent::App) and route_note = "继续修改应用"
            app.workspace_continue_edit(0).expect("ContinueEdit(0)");
            let active = app.workspace_task().expect("active task");
            assert_eq!(active.route, Some(Intent::App));
            assert_eq!(active.route_note, "继续修改应用");

            // 2. model::app_context(&task) with non-existent workspace path returns Ok("")
            assert_eq!(
                model::app_context(active)
                    .expect("app_context must succeed on non-existent workspace"),
                ""
            );

            // 3. Setting F_AMOO to "优化下显示效果" and dispatching Prepare succeeds and NEVER
            // produces "No such file or directory (os error 2)" in turns, steps, note, or workspace.error
            let _ = std::fs::remove_dir_all(&nonexistent_ws);
            app.fields[F_AMOO].set("优化下显示效果");
            app.workspace_prepare_and_confirm(None)
                .expect("Prepare follow-up app edit");

            if let Some(handle) = app.studio_lab.handle.take() {
                handle
                    .cancel
                    .store(true, std::sync::atomic::Ordering::SeqCst);
            }

            let updated = app.workspace_task().expect("updated task");
            assert_eq!(updated.route, Some(Intent::App));
            assert_eq!(updated.route_note, "继续修改应用");
            assert_eq!(
                model::app_context(updated).expect("app_context after Prepare"),
                ""
            );
            assert!(
                app.workspace.error.is_none(),
                "workspace.error must be None (demo_mode={demo_mode}): {:?}",
                app.workspace.error
            );
            assert!(
                !updated.turns.iter().any(|t| {
                    t.text.contains("No such file or directory") || t.text.contains("os error 2")
                }),
                "task.turns must never contain os error 2 (demo_mode={demo_mode}): {:?}",
                updated.turns
            );
            assert!(
                !updated.steps.iter().any(|s| {
                    s.note.contains("No such file or directory") || s.note.contains("os error 2")
                }),
                "task.steps must never contain os error 2 (demo_mode={demo_mode}): {:?}",
                updated.steps
            );
            assert!(
                !updated.note.contains("No such file or directory")
                    && !updated.note.contains("os error 2"),
                "task.note must never contain os error 2 (demo_mode={demo_mode}): {}",
                updated.note
            );
            let _ = std::fs::remove_dir_all(&nonexistent_ws);
        }
    }

    #[test]
    fn unsent_new_conversation_preserves_draft_on_history_and_session_switch() {
        let mut app = Companion::new_for_test();
        app.demo_mode = true;
        app.workspace_switch_new().unwrap();
        assert!(app.workspace.active.is_none());
        app.workspace_capture_draft();
        assert!(app.workspace.active.is_none(), "empty welcome must not create history");
        app.fields[F_AMOO].set("尚未发送的中文草稿\n第二行保持原样");
        app.workspace_toggle_history();
        let id = app.workspace.active.clone().expect("unsent input has a draft record");
        app.workspace_switch_new().unwrap();
        app.fields[F_AMOO].set("第二条草稿");
        app.workspace_select_task(&id).unwrap();
        assert_eq!(app.fields[F_AMOO].text, "尚未发送的中文草稿\n第二行保持原样");
        assert!(app.workspace.journal.tasks.iter().any(|t| t.draft == "第二条草稿"));
    }

    #[test]
    #[ignore = "requires STEPX_WORKSPACE_TEST_SERIAL; reads the explicitly selected phone"]
    fn connected_device_catalog_smoke() {
        let serial = std::env::var("STEPX_WORKSPACE_TEST_SERIAL")
            .expect("set STEPX_WORKSPACE_TEST_SERIAL to the authorized test phone");
        assert!(!serial.trim().is_empty());
        let adb = Adb::discover().expect("ADB runtime");
        let device = Device::inspect(&adb, &serial).expect("authorized test device");
        let summary = device.summary(&adb).expect("read device status");
        let apps = stepx_companion::apps::catalog(&adb, &device).expect("read app catalog");
        assert!(
            !apps.is_empty(),
            "installed applications should be available"
        );
        assert!(apps.iter().all(|a| !a.package.is_empty()));
        let gallery = stepx_companion::toolbox::list_phone_gallery(&adb, &device)
            .expect("read phone gallery");
        assert!(!gallery
            .iter()
            .any(|i| i.name.starts_with('.') || i.remote_path.contains("/.thumbnails")));
        let png = adb
            .screenshot(&device.transport)
            .expect("capture selected phone");
        assert!(
            image::load_from_memory(&png).is_ok(),
            "phone returns a decodable image"
        );
        eprintln!(
            "DEVICE_SMOKE: model={} apps={} gallery={} status_read=true screenshot_valid=true",
            device.model,
            apps.len(),
            gallery.len()
        );
        let _ = summary;
    }
}
