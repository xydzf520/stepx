//! 文件处理页（Page::Work）的状态与执行编排。所有手机请求都使用当前选中的设备。
use super::*;
use anyhow::Context;
use facloud_ui::TypedActionView;
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use stepx_companion::{amoo_catalog, amoo_media, work};

#[derive(Clone, Debug)]
pub(super) enum WorkAction {
    Refresh,
    VerifyModels,
    SearchSkills,
    ImportDesktopSkill,
    UsePhoneSkill(String),
    CreateAgent,
    Tab(u8),
    Panel(u8),
    Capabilities,
    Mode(u8),
    ChooseWorkspace,
    RefreshFiles,
    ToggleAttachMenu,
    Attach,
    AttachImages,
    RemoveAttachment(usize),
    Send,
    Cancel,
    NewChat,
    Session(usize),
    OpenRemote(String),
    PreviewAttachment {
        name: String,
        kind: String,
        mime: String,
        url: String,
    },
    SaveAttachment {
        name: String,
        kind: String,
        mime: String,
        url: String,
    },
    OpenPreview(String),
    CloseImagePreview,
    ZoomImagePreview(u16),
    ZoomInImagePreview,
    ZoomOutImagePreview,
    RotateImagePreview,
    SelectModel(Option<String>),
    Template(u8),
    OpenArtifact(usize),
    OpenFile(usize),
    PreviewArtifact(usize),
    PreviewFile(usize),
    PreviewRemote(usize),
    ClosePreview,
    OpenLocalPreview,
    ExportLocalPreview,
    OpenWorkspace,
    InstallSkill {
        key: String,
        install_id: String,
    },
    ConfirmSkillInstall,
    CancelSkillInstall,
    ToggleSkill {
        key: String,
        enabled: bool,
    },
    SelectAgent(String),
    CatalogPage(usize),
    QuickStarter(u8),
    OpenStudioWorkbench,
    ReviseFile(PathBuf),
    DeliverFileToPhone(PathBuf),
    TurnIntoApp(PathBuf),
}

#[derive(Clone, Debug)]
pub(super) struct WorkFileItem {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub is_dir: bool,
}
#[derive(Clone, Debug)]
pub(super) struct WorkDesktopSkillItem {
    pub name: String,
    pub description: String,
    pub path: PathBuf,
}

#[derive(Clone, Debug)]
#[allow(dead_code)]
pub(super) struct WorkArtifactItem {
    pub path: PathBuf,
    pub name: String,
    pub kind: String,
}

#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub(super) struct WorkLocalPreview {
    pub path: PathBuf,
    pub name: String,
    pub bytes: u64,
    pub image: bool,
    pub text: Option<String>,
    pub markdown: bool,
    pub note: String,
}

pub(super) struct WorkUiState {
    pub tab: u8,
    pub panel: u8,
    pub capabilities_open: bool,
    pub mode: u8,
    pub workspace: Option<PathBuf>,
    pub attachments: Vec<PathBuf>,
    pub attach_menu_open: bool,
    pub local_preview: Option<WorkLocalPreview>,
    pub local_preview_loading: Option<PathBuf>,
    pub local_preview_error: Option<String>,
    pub remote_preview_url: Option<String>,
    pub desktop_skills: Vec<WorkDesktopSkillItem>,
    pub market_skills: Vec<amoo_catalog::SkillInfo>,
    pub files: Vec<WorkFileItem>,
    pub artifacts: Vec<WorkArtifactItem>,
    pub events: Vec<String>,
    final_reply: Option<String>,
    pub status: String,
    pub running: bool,
    pub selected_model: Option<String>,
    pub selected_agent: Option<String>,
    pub session_models: HashMap<String, String>,
    pub catalog: Option<amoo_catalog::WorkCatalog>,
    pub catalog_loading: bool,
    pub catalog_page: usize,
    pub error: Option<String>,
    pub pending_install: Option<(String, String)>,
    pub files_scroll: ClippedScrollStateHandle,
    pub prompt_scroll: ClippedScrollStateHandle,
    pub preview_scroll: ClippedScrollStateHandle,
    pub image_previews: HashMap<String, PathBuf>,
    pub preview_errors: HashMap<String, String>,
    pub image_viewer: Option<String>,
    pub image_viewer_zoom: u16,
    pub image_viewer_rotated_path: Option<PathBuf>,
    pub image_viewer_rotation: u16,
    pub preview_pending: HashSet<String>,
    preview_tx: Option<mpsc::Sender<(String, std::result::Result<PathBuf, String>)>>,
    preview_identities: HashMap<String, String>,
    preview_attempted: HashSet<String>,
    preview_cache: Option<Arc<tempfile::TempDir>>,
    preview_rx: Option<mpsc::Receiver<(String, std::result::Result<PathBuf, String>)>>,
    pub(super) handle: Option<work::WorkHandle>,
    active_key: Option<String>,
    submitted_draft: Option<String>,
    /// 文件处理页自己的会话。不再借用 Amoo 当前对话（`amoo_session_idx`），
    /// 避免在这里发的任务进入用户正在聊的 Amoo 对话。由 Amoo 自动驱动的运行
    /// （`workspace.running`）仍使用对话任务自己的会话。
    pub session_key: Option<String>,
    /// 需要交回 Amoo 的草稿（`Some("")` 表示只切回 Amoo）。在会话切换结束后处理。
    amoo_handoff: Option<String>,
    /// 「让 Amoo 修订」时的输入框占位提示。
    pub revise_hint: Option<String>,
}

/// 「做游戏」模板的提示词；会交给 Amoo 自动生成应用。
const WORK_GAME_PROMPT: &str = "请制作一个可离线打开直接玩的方块沙盒小游戏：包括移动、放置和移除方块、材料切换、中文操作说明与重置。生成完整 HTML 文件，读回检查交互逻辑，交付文件并简短说明玩法，不要在对话里重复粘贴全部代码。";

fn default_office_workspace_dir() -> Option<PathBuf> {
    if cfg!(test) || std::env::args().any(|a| a == "--verify-settings") {
        return None;
    }
    let dir = stepx_companion::resources::user_state().join("amoo-workspace/office-default");
    let _ = std::fs::create_dir_all(&dir);
    let _ = work::ensure_office_starters(&dir);
    Some(dir.canonicalize().unwrap_or(dir))
}

fn deliverable_kind_for_path(path: &std::path::Path) -> Option<String> {
    let file_name = path.file_name()?.to_string_lossy();
    if work::is_ignored_workspace_entry(&file_name) {
        return None;
    }
    if path.components().any(|c| match c {
        std::path::Component::Normal(s) => {
            let name = s.to_string_lossy();
            !matches!(name.as_ref(), ".local" | ".config" | ".cache" | ".var")
                && !name.starts_with(".tmp")
                && work::is_ignored_workspace_entry(&name)
        }
        _ => false,
    }) {
        return None;
    }
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xlsx" => Some("Excel 表格".into()),
        "pptx" => Some("演示文稿".into()),
        "docx" => Some("Word 文档".into()),
        "md" => Some("Markdown".into()),
        "html" | "htm" => Some("网页".into()),
        "csv" | "tsv" => Some("数据表".into()),
        "pdf" => Some("PDF".into()),
        "svg" => Some("矢量图".into()),
        _ => None,
    }
}

fn deliverable_ext_rank(path: &std::path::Path) -> u8 {
    let ext = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_ascii_lowercase();
    match ext.as_str() {
        "xlsx" => 0,
        "pptx" => 1,
        "docx" => 2,
        "md" => 3,
        "html" | "htm" => 4,
        "csv" | "tsv" => 5,
        "pdf" => 6,
        "svg" => 7,
        _ => 8,
    }
}

pub(super) const MAX_WORK_IMAGE_PREVIEWS: usize = 16;
pub(super) const IDLE_WORK_IMAGE_PREVIEWS: usize = 4;

impl WorkUiState {
    /// Insert an image preview into `image_previews` while strictly capping the map at
    /// `MAX_WORK_IMAGE_PREVIEWS` (16) entries. Active `image_viewer` entries are preserved.
    pub(super) fn insert_image_preview(&mut self, key: String, path: PathBuf) {
        if !self.image_previews.contains_key(&key) {
            while self.image_previews.len() >= MAX_WORK_IMAGE_PREVIEWS {
                let active_viewer = self.image_viewer.as_deref();
                let victim = self
                    .image_previews
                    .keys()
                    .find(|k| Some(k.as_str()) != active_viewer)
                    .or_else(|| self.image_previews.keys().next())
                    .cloned();
                let Some(victim) = victim else {
                    break;
                };
                self.image_previews.remove(&victim);
                self.preview_attempted.remove(&victim);
            }
        }
        self.image_previews.insert(key, path);
    }

    /// Release idle transient image previews when navigating away from `Page::Work`
    /// (or `Page::Gallery`), retaining at most `IDLE_WORK_IMAGE_PREVIEWS` (4) entries
    /// and always preserving any active `image_viewer` entry.
    pub(super) fn trim_idle_previews(&mut self) {
        while self.image_previews.len() > IDLE_WORK_IMAGE_PREVIEWS {
            let active_viewer = self.image_viewer.as_deref();
            let victim = self
                .image_previews
                .keys()
                .find(|k| Some(k.as_str()) != active_viewer)
                .or_else(|| self.image_previews.keys().next())
                .cloned();
            let Some(victim) = victim else {
                break;
            };
            self.image_previews.remove(&victim);
            self.preview_attempted.remove(&victim);
        }
        self.preview_errors.clear();
        if let Ok(mut mem) = work_preview_memory_cache().lock() {
            while mem.len() > IDLE_WORK_IMAGE_PREVIEWS {
                let Some(oldest) = mem
                    .iter()
                    .min_by_key(|(_, (_, mtime, _))| *mtime)
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                mem.remove(&oldest);
            }
        }
    }
}

impl Default for WorkUiState {
    fn default() -> Self {
        let mut local_preview = initial_local_preview();
        let image_previews = initial_image_previews();
        let mut workspace = stepx_companion::settings::load()
            .work_directory
            .or_else(|| {
                local_preview
                    .as_ref()
                    .and_then(|preview| preview.path.parent())
                    .map(std::path::Path::to_path_buf)
            });
        if workspace.is_none() {
            workspace = default_office_workspace_dir();
        }
        let files: Vec<WorkFileItem> = workspace
            .as_ref()
            .and_then(|root| {
                work::list_workspace(root).ok().map(|files| {
                    files
                        .into_iter()
                        .map(|file| WorkFileItem {
                            path: root.join(&file.relative_path),
                            name: file.relative_path,
                            bytes: file.bytes,
                            is_dir: file.directory,
                        })
                        .collect()
                })
            })
            .unwrap_or_default();
        let mut artifacts: Vec<WorkArtifactItem> = if cfg!(test) {
            Vec::new()
        } else {
            files
                .iter()
                .filter(|f| !f.is_dir && !work::is_ignored_workspace_entry(&f.name))
                .filter_map(|f| {
                    deliverable_kind_for_path(&f.path).map(|kind| WorkArtifactItem {
                        path: f.path.clone(),
                        name: f.name.clone(),
                        kind,
                    })
                })
                .collect()
        };
        artifacts.sort_by(|a, b| {
            deliverable_ext_rank(&a.path)
                .cmp(&deliverable_ext_rank(&b.path))
                .then_with(|| a.name.cmp(&b.name))
        });
        if !cfg!(test) && local_preview.is_none() && image_previews.is_empty() {
            if let Some(root) = workspace.as_ref() {
                if let Some(first) = artifacts.first() {
                    local_preview = read_work_preview(root, &first.path).ok();
                }
            }
        }
        Self {
            tab: std::env::args()
                .position(|arg| arg == "--work-tab")
                .and_then(|index| std::env::args().nth(index + 1))
                .map(|tab| match tab.as_str() {
                    "models" => 1,
                    "skills" => 2,
                    "agents" => 3,
                    "apps" => 4,
                    _ => 0,
                })
                .unwrap_or(0),
            panel: 2,
            capabilities_open: std::env::args().any(|arg| arg == "--work-tab"),
            mode: 1,
            workspace,
            attachments: Vec::new(),
            attach_menu_open: false,
            local_preview,
            local_preview_loading: None,
            local_preview_error: None,
            remote_preview_url: None,
            desktop_skills: Vec::new(),
            market_skills: Vec::new(),
            files,
            artifacts,
            events: Vec::new(),
            final_reply: None,
            status: "已就绪：可一键生成 Word/Excel/PPT 样板，或描述办公需求交给 Amoo".into(),
            running: false,
            selected_model: None,
            selected_agent: None,
            session_models: HashMap::new(),
            catalog: None,
            catalog_loading: false,
            catalog_page: 0,
            error: None,
            pending_install: None,
            files_scroll: Default::default(),
            prompt_scroll: Default::default(),
            preview_scroll: Default::default(),
            image_previews,
            preview_errors: HashMap::new(),
            image_viewer: None,
            image_viewer_zoom: 100,
            image_viewer_rotated_path: None,
            image_viewer_rotation: 0,
            preview_pending: HashSet::new(),
            preview_tx: None,
            preview_identities: HashMap::new(),
            preview_attempted: HashSet::new(),
            preview_cache: None,
            preview_rx: None,
            handle: None,
            active_key: None,
            submitted_draft: None,
            session_key: None,
            amoo_handoff: None,
            revise_hint: None,
        }
    }
}

// Explicit local preview fixture for native layout captures; absent from release behavior.
fn initial_local_preview() -> Option<WorkLocalPreview> {
    #[cfg(feature = "ui-capture")]
    {
        let args: Vec<_> = std::env::args().collect();
        if let Some(path) = args
            .iter()
            .position(|arg| arg == "--work-local-preview")
            .and_then(|index| args.get(index + 1))
            .map(PathBuf::from)
            .and_then(|path| path.canonicalize().ok())
        {
            return path
                .parent()
                .and_then(|root| read_work_preview(root, &path).ok());
        }
    }
    None
}

// Local-only image fixture for the native capture build. Production starts empty.
fn initial_image_previews() -> HashMap<String, PathBuf> {
    #[cfg(feature = "ui-capture")]
    {
        let args: Vec<_> = std::env::args().collect();
        if args.iter().any(|arg| arg == "--work-format-fixture") {
            if let Some(path) = args
                .iter()
                .position(|arg| arg == "--work-preview-file")
                .and_then(|index| args.get(index + 1))
                .map(PathBuf::from)
                .filter(|path| path.is_file())
            {
                return HashMap::from([(
                    "https://example.invalid/amoo-work-native-preview.png".into(),
                    path,
                )]);
            }
        }
    }
    HashMap::new()
}

impl WorkUiState {
    pub(super) fn reset_account_work(&mut self) {
        self.disconnect();
        self.handle = None;
        self.running = false;
        self.active_key = None;
        self.submitted_draft = None;
        self.local_preview = None;
        self.local_preview_loading = None;
        self.local_preview_error = None;
        self.events.clear();
        self.artifacts.clear();
        self.attachments.clear();
        self.error = None;
        self.status = "手机账号已变化，请同步当前账号后开始工作".into();
    }

    pub(super) fn clear_account_previews(&mut self) {
        self.preview_rx = None;
        self.preview_tx = None;
        self.preview_pending.clear();
        self.image_viewer = None;
        self.image_viewer_zoom = 100;
        self.image_viewer_rotated_path = None;
        self.image_viewer_rotation = 0;
        self.preview_scroll = Default::default();
        self.preview_cache = None;
        self.preview_attempted.clear();
        self.image_previews.clear();
        self.preview_errors.clear();
        self.preview_identities.clear();
        self.remote_preview_url = None;
    }

    pub fn disconnect(&mut self) {
        if let Some(handle) = &self.handle {
            handle.cancel.store(true, Ordering::Relaxed);
        }
        self.session_key = None;
        self.revise_hint = None;
        self.catalog = None;
        self.market_skills.clear();
        self.catalog_loading = false;
        self.selected_model = None;
        self.selected_agent = None;
        self.session_models.clear();
        self.pending_install = None;
        self.preview_rx = None;
        self.preview_tx = None;
        self.preview_pending.clear();
        self.image_viewer = None;
        self.image_viewer_zoom = 100;
        self.image_viewer_rotated_path = None;
        self.image_viewer_rotation = 0;
        self.preview_scroll = Default::default();
        self.preview_cache = None;
        self.preview_attempted.clear();
        self.image_previews = initial_image_previews();
        self.preview_errors.clear();
        self.preview_identities.clear();
        self.remote_preview_url = None;
        self.local_preview = None;
        self.local_preview_loading = None;
        self.local_preview_error = None;
        self.events.clear();
        self.artifacts.clear();
        self.attachments.clear();
        self.error = None;
        self.active_key = None;
        self.submitted_draft = None;
        self.handle = None;
        self.running = false;
        self.status = "手机已断开，请连接设备后同步工作".into();
    }
    pub(super) fn log(&mut self, message: String) {
        self.events.push(message);
        if self.events.len() > 200 {
            self.events.remove(0);
        }
    }
}

pub(super) enum WorkOutcome {
    ModelsVerified {
        models: Vec<amoo_catalog::ModelInfo>,
        account_scope: Option<String>,
    },
    LocalPreview {
        workspace: PathBuf,
        path: PathBuf,
        result: std::result::Result<WorkLocalPreview, String>,
    },
    AttachmentSaved {
        path: PathBuf,
        bytes: u64,
    },
    SkillImported {
        workspace: PathBuf,
        name: String,
        omitted: usize,
    },
    MarketSkills(Vec<amoo_catalog::SkillInfo>),
    Refreshed {
        data: RealAmooData,
        catalog: std::result::Result<amoo_catalog::WorkCatalog, String>,
    },
    Sent {
        receipt: amoo::PendingReply,
        draft: String,
        attachments: Vec<PathBuf>,
    },
    CatalogChanged(amoo_catalog::WorkCatalog, String),
}

impl WorkAction {
    pub(super) fn keeps_current_shell(&self) -> bool {
        matches!(
            self,
            Self::OpenRemote(_)
                | Self::PreviewAttachment { .. }
                | Self::SaveAttachment { .. }
                | Self::OpenPreview(_)
                | Self::CloseImagePreview
                | Self::ZoomImagePreview(_)
                | Self::ZoomInImagePreview
                | Self::ZoomOutImagePreview
                | Self::RotateImagePreview
                | Self::OpenLocalPreview
        )
    }
}

fn conversation_images(items: &[AmooChatItem]) -> Vec<stepx_companion::openclaw::RealAttachment> {
    let mut seen = HashSet::new();
    items
        .iter()
        .rev()
        .flat_map(|item| match item {
            AmooChatItem::Attachment {
                name,
                kind,
                mime,
                url,
                path,
                ..
            } if (kind == "image" || mime.starts_with("image/")) && !url.is_empty() => {
                vec![stepx_companion::openclaw::RealAttachment {
                    name: name.clone(),
                    kind: kind.clone(),
                    mime: mime.clone(),
                    url: url.clone(),
                    path: path.clone(),
                    ..Default::default()
                }]
            }
            AmooChatItem::AssistantRich {
                text: Some(text), ..
            } => pages::markdown_images(text),
            _ => vec![],
        })
        .filter(|a| seen.insert(a.url.clone()))
        .collect()
}

impl Companion {
    /// Only a stable native message ID can alias renewed attachment URLs.
    pub(super) fn record_work_attachment_identities(&mut self, real: &RealAmooData) {
        for session in &real.sessions {
            for message in &session.messages {
                if message.message_id.is_empty() {
                    continue;
                }
                for attachment in pages::markdown_images(&message.text) {
                    let identity = serde_json::json!([
                        session.key,
                        message.message_id,
                        "inline",
                        attachment.url
                    ])
                    .to_string();
                    self.work
                        .preview_identities
                        .insert(attachment.url, identity);
                }
                for (index, attachment) in message.attachments.iter().enumerate() {
                    if attachment.url.is_empty() {
                        continue;
                    }
                    let identity = serde_json::json!([
                        session.key,
                        message.message_id,
                        index,
                        attachment.name,
                        attachment.mime,
                        attachment.size_bytes
                    ])
                    .to_string();
                    self.work
                        .preview_identities
                        .insert(attachment.url.clone(), identity);
                }
            }
        }
    }

    pub(super) fn refresh_work(&mut self) {
        if self.busy || self.work.catalog_loading {
            return;
        }
        if self.selected.is_none() {
            self.work.error = Some("连接手机后即可同步 Amoo 任务；本地工作文件夹仍可查看".into());
            return;
        }
        self.work.catalog_loading = true;
        self.work.error = None;
        let key = if self.page == Page::Work {
            self.work_session()
        } else {
            self.amoo_sessions.get(self.amoo_session_idx)
        }
        .map(|s| s.session_key.clone());
        self.with_device(
            "正在同步 Amoo 工作会话与能力…",
            move |adb, device| {
                let data = amoo::fetch(adb, &device, key.as_deref())?;
                let catalog = amoo_catalog::fetch(adb, &device).map_err(|e| format!("{e:#}"));
                Ok(Outcome::Work(WorkOutcome::Refreshed { data, catalog }))
            },
        );
    }

    pub(super) fn apply_work(&mut self, result: WorkOutcome) {
        self.with_work_session(|me| me.apply_work_in_session(result));
    }

    fn apply_work_in_session(&mut self, result: WorkOutcome) {
        match result {
            WorkOutcome::ModelsVerified {
                models,
                account_scope,
            } => {
                self.record_amoo_usage(&RealAmooData {
                    account_scope,
                    ..Default::default()
                });
                let count = models.len();
                let catalog = self.work.catalog.get_or_insert_with(Default::default);
                catalog.models = models;
                catalog.can_select_model = false;
                catalog.models_status = amoo_catalog::CatalogAvailability {
                    available: true,
                    message: format!("已验证 {count} 个手机 Amoo 云端路由；手机自动选择模型，当前版本未开放指定接口。"),
                };
                self.work.catalog_page = 0;
                self.work.status = "已完成真实模型调用，只显示当前账号可用的结果".into();
                self.status = self.work.status.clone();
            }
            WorkOutcome::LocalPreview {
                workspace,
                path,
                result,
            } => {
                if self.work.workspace.as_ref() == Some(&workspace)
                    && self.work.local_preview_loading.as_ref() == Some(&path)
                {
                    self.work.local_preview_loading = None;
                    match result {
                        Ok(preview) => self.work.local_preview = Some(preview),
                        Err(error) => self.work.local_preview_error = Some(error),
                    }
                }
            }
            WorkOutcome::AttachmentSaved { path, bytes } => {
                self.work.status = format!(
                    "已保存 {}（{}）",
                    path.display(),
                    stepx_companion::storage::format_bytes(bytes)
                );
                self.status = self.work.status.clone();
            }
            WorkOutcome::SkillImported {
                workspace,
                name,
                omitted,
            } => {
                if self.work.workspace.as_ref() == Some(&workspace) {
                    let _ = self.work_files();
                }
                self.work.status = if omitted == 0 {
                    format!("已将 {name} 加入桌面工作流程，下一条桌面任务可使用")
                } else {
                    format!("已导入 {name} 的流程文档；{omitted} 个脚本或非文本文件未导入")
                };
                self.workspace_skill_imported(&workspace);
                self.work.mode = 1;
                self.work.tab = 0;
                self.work.capabilities_open = false;
                self.status = self.work.status.clone();
            }
            WorkOutcome::MarketSkills(skills) => {
                self.work.status = format!("已查询到 {} 个技能市场条目", skills.len());
                self.work.market_skills = skills;
                self.status = self.work.status.clone();
            }
            WorkOutcome::Refreshed { data, catalog } => {
                self.apply_real_amoo_data(data);
                if !self.work.running {
                    let _ = self.work_files();
                }
                if self.work.tab == 0 {
                    self.reveal_latest_chat();
                }
                self.work.catalog_loading = false;
                match catalog {
                    Ok(catalog) => {
                        if !catalog.skills.is_empty() {
                            self.amoo_total_skills = catalog.skills.len();
                            for sk in &catalog.skills {
                                if let Some(existing) = self
                                    .amoo_skills
                                    .iter_mut()
                                    .find(|e| e.skill_key == sk.id || e.name == sk.name)
                                {
                                    existing.enabled = sk.enabled;
                                } else if sk.installed {
                                    self.amoo_skills.push(super::AmooSkillEntry {
                                        skill_key: sk.id.clone(),
                                        name: sk.name.clone(),
                                        enabled: sk.enabled,
                                        prompt: format!("使用 {} 技能帮我处理：", sk.name),
                                    });
                                }
                            }
                        }
                        self.work.catalog = Some(catalog);
                        self.work.error = None;
                    }
                    Err(error) => self.work.error = Some(error),
                }
                self.work.status = format!("已同步 {} 个手机会话", self.amoo_sessions.len());
                self.status = self.work.status.clone();
            }
            WorkOutcome::Sent {
                receipt,
                draft,
                attachments,
            } => {
                self.amoo_ui
                    .session_started
                    .insert(receipt.key.clone(), Instant::now());
                self.work.active_key = Some(receipt.key.clone());
                if !self
                    .amoo_sessions
                    .iter()
                    .any(|session| session.session_key == receipt.key)
                {
                    self.apply(Outcome::AmooCreated(receipt.key.clone()));
                }
                if self.fields[F_WORK_PROMPT].text.trim() == draft.trim() {
                    self.fields[F_WORK_PROMPT].clear();
                }
                if self.work.attachments == attachments {
                    self.work.attachments.clear();
                }
                if self
                    .workspace
                    .running
                    .as_ref()
                    .is_some_and(|r| r.executor == stepx_companion::amoo_workspace::Executor::Image)
                {
                    if self.fields[F_AMOO].text.trim() == draft.trim() {
                        self.fields[F_AMOO].clear();
                    }
                    if self
                        .amoo_attachments
                        .iter()
                        .filter_map(|a| a.local_path.as_ref())
                        .eq(attachments.iter())
                    {
                        self.amoo_attachments.clear();
                    }
                }
                self.apply(Outcome::AmooSent(receipt));
                self.work.status = "手机已接收，正在等待回复".into();
            }
            WorkOutcome::CatalogChanged(catalog, message) => {
                if !catalog.skills.is_empty() {
                    self.amoo_total_skills = catalog.skills.len();
                    for sk in &catalog.skills {
                        if let Some(existing) = self
                            .amoo_skills
                            .iter_mut()
                            .find(|e| e.skill_key == sk.id || e.name == sk.name)
                        {
                            existing.enabled = sk.enabled;
                        } else if sk.installed {
                            self.amoo_skills.push(super::AmooSkillEntry {
                                skill_key: sk.id.clone(),
                                name: sk.name.clone(),
                                enabled: sk.enabled,
                                prompt: format!("使用 {} 技能帮我处理：", sk.name),
                            });
                        }
                    }
                }
                self.work.catalog = Some(catalog);
                self.work.catalog_loading = false;
                self.work.pending_install = None;
                self.work.status = message.clone();
                self.status = message;
            }
        }
    }

    pub(super) fn ensure_work_default_workspace(&mut self) -> Result<PathBuf> {
        if self.work.workspace.is_none() {
            let dir =
                stepx_companion::resources::user_state().join("amoo-workspace/office-default");
            std::fs::create_dir_all(&dir)?;
            let _ = work::ensure_office_starters(&dir);
            let canonical = dir.canonicalize().unwrap_or(dir);
            self.work.workspace = Some(canonical);
            if !cfg!(test) && !std::env::args().any(|a| a == "--verify-settings") {
                let _ = self.remember_work_directory();
            }
            let _ = self.work_files();
        }
        self.work
            .workspace
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先选择工作文件夹"))
    }

    #[allow(dead_code)]
    pub(super) fn sync_work_office_workspace(&mut self) -> Result<()> {
        let _ = self.ensure_work_default_workspace()?;
        self.work_files()
    }

    pub(super) fn work_files(&mut self) -> Result<()> {
        let Some(root) = self.work.workspace.as_ref() else {
            return Ok(());
        };
        let root = root.clone();
        self.work.files = work::list_workspace(&root)?
            .into_iter()
            .map(|file| WorkFileItem {
                path: root.join(&file.relative_path),
                name: file.relative_path,
                bytes: file.bytes,
                is_dir: file.directory,
            })
            .collect();
        self.work.desktop_skills = work::list_skills(&root)?
            .into_iter()
            .map(|skill| WorkDesktopSkillItem {
                name: skill.name,
                description: skill.description,
                path: skill.path,
            })
            .collect();
        let key = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .map(|session| session.session_key.as_str())
            .unwrap_or("");
        let mut artifacts: Vec<WorkArtifactItem> = work::load_session_artifacts(&root, key)?
            .into_iter()
            .map(|file| WorkArtifactItem {
                name: file.relative_path,
                path: file.path,
                kind: file.kind,
            })
            .collect();
        let mut workspace_deliverables = Vec::new();
        for file in &self.work.files {
            if file.is_dir || work::is_ignored_workspace_entry(&file.name) {
                continue;
            }
            if let Some(kind) = deliverable_kind_for_path(&file.path) {
                if !artifacts
                    .iter()
                    .any(|a| a.path == file.path || a.name == file.name)
                {
                    workspace_deliverables.push(WorkArtifactItem {
                        name: file.name.clone(),
                        path: file.path.clone(),
                        kind,
                    });
                }
            }
        }
        workspace_deliverables.sort_by(|a, b| {
            deliverable_ext_rank(&a.path)
                .cmp(&deliverable_ext_rank(&b.path))
                .then_with(|| a.name.cmp(&b.name))
        });
        artifacts.extend(workspace_deliverables);
        self.work.artifacts = artifacts;
        if !cfg!(test)
            && self.work.local_preview.is_none()
            && self.work.local_preview_loading.is_none()
            && self.work.remote_preview_url.is_none()
            && self.work.image_previews.is_empty()
        {
            if let Some(first) = self.work.artifacts.first() {
                self.work.local_preview = read_work_preview(&root, &first.path).ok();
            }
        }
        Ok(())
    }

    /// 文件处理页当前会话在 `amoo_sessions` 中的位置。
    pub(super) fn work_session_index(&self) -> Option<usize> {
        if self.workspace.running.is_some() {
            return (self.amoo_session_idx < self.amoo_sessions.len())
                .then_some(self.amoo_session_idx);
        }
        let key = self.work.session_key.as_deref()?;
        self.amoo_sessions
            .iter()
            .position(|session| session.session_key == key)
    }

    pub(super) fn work_session(&self) -> Option<&AmooSessionItem> {
        self.work_session_index()
            .and_then(|index| self.amoo_sessions.get(index))
    }

    /// 在文件处理页自己的会话里执行 `f`，结束后恢复 Amoo 当前对话。
    /// Amoo 自动驱动的运行直接沿用对话任务的会话，不做切换。
    fn with_work_session<R>(&mut self, f: impl FnOnce(&mut Self) -> R) -> R {
        if self.workspace.running.is_some() {
            return f(self);
        }
        let amoo_key = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .map(|session| session.session_key.clone());
        let amoo_idx = self.amoo_session_idx;
        let amoo_len = self.amoo_sessions.len();
        self.amoo_session_idx = self.work_session_index().unwrap_or(usize::MAX);
        let result = f(self);
        if self.workspace.running.is_some() {
            // 期间启动了 Amoo 自动运行，会话由它接管。
            return result;
        }
        self.work.session_key = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .map(|session| session.session_key.clone());
        self.amoo_session_idx = match amoo_key {
            Some(key) => self
                .amoo_sessions
                .iter()
                .position(|session| session.session_key == key)
                .unwrap_or(usize::MAX),
            None if self.amoo_sessions.len() == amoo_len => amoo_idx,
            None => usize::MAX,
        };
        result
    }

    /// 把草稿交给唯一的 AI 入口 Amoo：仅在 Amoo 输入框为空时填入并自动开始，
    /// 否则保留用户已有的草稿，只切回 Amoo。
    fn work_handoff_to_amoo(&mut self, draft: String, ctx: &mut ViewContext<Self>) {
        let draft = draft.trim().to_string();
        let fill = !draft.is_empty() && self.fields[F_AMOO].text.trim().is_empty();
        if fill {
            self.fields[F_AMOO].set(&draft);
        }
        self.handle_action(&Action::Page(Page::Amoo), ctx);
        if fill {
            self.handle_action(&Action::Workspace(WorkspaceAction::Prepare), ctx);
        } else if !draft.is_empty() {
            self.status = "Amoo 输入框里还有没发送的内容，已为你保留".into();
            self.error = false;
        }
    }

    pub(super) fn handle_work_action(&mut self, action: &WorkAction, ctx: &mut ViewContext<Self>) {
        self.work.error = None;
        let result = self.with_work_session(|me| me.do_work_action(action, ctx));
        if result.is_ok() && !self.busy {
            self.status = self.work.status.clone();
            self.error = false;
        }
        if let Err(error) = result {
            let message = format!("{error:#}");
            self.work.error = Some(message.clone());
            self.error = true;
            self.status = message;
        }
        if let Some(draft) = self.work.amoo_handoff.take() {
            self.work_handoff_to_amoo(draft, ctx);
        }
    }

    fn do_work_action(&mut self, action: &WorkAction, ctx: &mut ViewContext<Self>) -> Result<()> {
        match action {
            WorkAction::Tab(tab) => {
                self.clear_text_interaction();
                self.page = Page::Work;
                self.work.capabilities_open = *tab > 0;
                self.work.tab = if *tab == 0 { 0 } else { (*tab).clamp(1, 4) };
                self.work.catalog_page = 0;
                self.scroll = Default::default();
                self.trigger_subtab_switch();
                if *tab == 0 {
                    self.reveal_latest_chat();
                }
                if self.work.catalog.is_none() && !self.busy && self.selected.is_some() {
                    self.refresh_work();
                }
            }
            WorkAction::Panel(panel) => {
                self.work.panel = if self.work.panel == *panel { 0 } else { *panel };
                self.trigger_subtab_switch();
            }
            WorkAction::Capabilities => {
                self.work.capabilities_open = !self.work.capabilities_open;
                self.work.tab = if self.work.capabilities_open { 2 } else { 0 };
                self.scroll = Default::default();
                self.trigger_subtab_switch();
                if !self.work.capabilities_open {
                    self.reveal_latest_chat();
                }
                if self.work.capabilities_open && self.work.catalog.is_none() && !self.busy {
                    self.refresh_work();
                }
            }
            WorkAction::CatalogPage(page) => {
                self.work.catalog_page = *page;
                self.scroll = Default::default();
                self.trigger_subtab_switch();
            }
            WorkAction::Mode(mode) => {
                anyhow::ensure!(
                    !self.busy && !self.work.running,
                    "请先等待当前工作完成，或停止桌面执行"
                );
                if *mode == 1 && !self.work.attachments.is_empty() {
                    // 没选文件夹时直接用默认工作文件夹，不再弹出系统选择框。
                    let root = self.ensure_work_default_workspace()?;
                    let root = &root;
                    let mut local_files = Vec::new();
                    for path in &self.work.attachments {
                        let path = path.canonicalize()?;
                        if path.starts_with(root) {
                            local_files.push(path);
                        } else {
                            local_files
                                .push(root.join(work::import_file(root, &path)?.relative_path));
                        }
                    }
                    self.work.attachments = local_files;
                    self.work_files()?;
                }
                if self.work.mode != *mode {
                    self.work.mode = *mode;
                    self.trigger_subtab_switch();
                }
            }
            WorkAction::Refresh => self.refresh_work(),
            WorkAction::VerifyModels => {
                anyhow::ensure!(
                    !self.busy && !self.work.running,
                    "请等待当前任务完成后验证模型"
                );
                if let Some(catalog) = &mut self.work.catalog {
                    catalog.models.clear();
                    catalog.models_status = amoo_catalog::CatalogAvailability {
                        available: false,
                        message: "本次验证尚未确认可用模型；验证成功后会显示结果。".into(),
                    };
                }
                self.work.selected_model = None;
                self.with_device(
                    "正在验证账号模型权限，发送一条短测试请求…",
                    |adb, device| {
                        let models = amoo_catalog::verify_models(adb, &device)?;
                        let account_scope =
                            stepx_companion::amoo_usage::account_scope(adb, &device)?;
                        anyhow::ensure!(
                            models
                                .iter()
                                .any(|model| model.is_verified_for(account_scope.as_deref())),
                            "模型验证后账号身份已变化或无法核实，请重新验证"
                        );
                        Ok(Outcome::Work(WorkOutcome::ModelsVerified {
                            models,
                            account_scope,
                        }))
                    },
                );
            }
            WorkAction::SearchSkills => {
                anyhow::ensure!(!self.busy, "请等待当前同步完成");
                let query = self.fields[F_WORK_CAP_SEARCH].text.trim().to_string();
                anyhow::ensure!(!query.is_empty(), "请输入要搜索的技能名称或工作场景");
                self.with_device("正在搜索手机技能市场…", move |adb, device| {
                    Ok(Outcome::Work(WorkOutcome::MarketSkills(
                        amoo_catalog::search_skills(adb, &device, &query)?,
                    )))
                });
            }
            WorkAction::ImportDesktopSkill => {
                anyhow::ensure!(!self.work.running, "请等待当前工作完成后导入工作流程");
                let root = self
                    .work
                    .workspace
                    .clone()
                    .ok_or_else(|| anyhow::anyhow!("请先选择工作文件夹"))?;
                if let Some(path) = native_dialog::FileDialog::new()
                    .set_title("选择包含 SKILL.md 的工作流程文件夹")
                    .show_open_single_dir()?
                {
                    let skill = work::import_skill(&root, &path)?;
                    self.work.status = format!("已导入工作流程 {}，桌面任务可读取使用", skill.name);
                    self.work_files()?;
                }
            }
            WorkAction::UsePhoneSkill(key) => {
                anyhow::ensure!(!self.busy && !self.work.running, "请等待当前操作完成");
                let workspace = self.ensure_work_default_workspace()?;
                let key = key.clone();
                self.with_device(
                    "正在从手机导入工作流程…",
                    move |adb, device| {
                        let bundle = amoo_catalog::export_skill(adb, &device, &key)?;
                        let documents = bundle
                            .files
                            .into_iter()
                            .map(|file| (file.path, file.content))
                            .collect::<Vec<_>>();
                        let skill = work::import_skill_documents(&workspace, &documents)?;
                        Ok(Outcome::Work(WorkOutcome::SkillImported {
                            workspace,
                            name: skill.name,
                            omitted: bundle.omitted,
                        }))
                    },
                );
            }
            WorkAction::CreateAgent => {
                anyhow::ensure!(!self.busy && !self.work.running, "请等待当前操作完成");
                let name = self.fields[F_WORK_AGENT_NAME].text.trim().to_string();
                anyhow::ensure!(!name.is_empty(), "请输入新智能体名称");
                self.with_device("正在创建手机智能体…", move |adb, device| {
                    let catalog = amoo_catalog::create_agent(adb, &device, &name)?;
                    Ok(Outcome::Work(WorkOutcome::CatalogChanged(
                        catalog,
                        format!("已在手机 OpenClaw 创建智能体 {name}"),
                    )))
                });
            }
            WorkAction::ChooseWorkspace => {
                anyhow::ensure!(
                    !self.busy && !self.work.running,
                    "工作进行中，请先停止桌面执行再切换文件夹"
                );
                if let Some(path) = native_dialog::FileDialog::new()
                    .set_title("选择工作文件夹")
                    .show_open_single_dir()?
                {
                    let path = path.canonicalize()?;
                    self.work.workspace = Some(path);
                    self.remember_work_directory()?;
                    self.work.local_preview = None;
                    self.work.local_preview_loading = None;
                    self.work.local_preview_error = None;
                    self.work_files()?;
                    self.work.status =
                        "已选择工作文件夹；桌面任务仅在该目录内读取和生成文件".into();
                }
            }
            WorkAction::RefreshFiles => self.work_files()?,
            WorkAction::ToggleAttachMenu => {
                self.work.attach_menu_open = !self.work.attach_menu_open;
            }
            WorkAction::Attach | WorkAction::AttachImages => {
                anyhow::ensure!(
                    !self.busy && !self.work.running,
                    "请等待当前操作完成后添加文件"
                );
                if self.work.mode == 1 {
                    // 没选文件夹时直接用默认工作文件夹，不再弹出系统选择框。
                    self.ensure_work_default_workspace()?;
                }
                self.work.attach_menu_open = false;
                let images_only = matches!(action, WorkAction::AttachImages);
                let dialog = native_dialog::FileDialog::new().set_title(if images_only {
                    "上传本地图片"
                } else {
                    "添加文档与附件"
                });
                let dialog = if images_only {
                    dialog.add_filter("图片", &["png", "jpg", "jpeg", "webp", "gif", "bmp"])
                } else {
                    dialog
                };
                let selected = dialog.show_open_multiple_file()?;
                if !selected.is_empty() {
                    anyhow::ensure!(
                        self.work.attachments.len() + selected.len() <= 8,
                        "最多添加 8 个附件，请减少选择数量"
                    );
                    if self.work.mode != 1 {
                        let mut all = self.work.attachments.clone();
                        all.extend(selected.iter().cloned());
                        amoo_media::prepare_attachments(&all)?;
                    }
                    for path in selected {
                        let local = if self.work.mode == 1 {
                            let root =
                                self.work.workspace.as_ref().context("请先选择工作文件夹")?;
                            let file = work::import_file(root, &path)?;
                            root.join(file.relative_path)
                        } else {
                            let path = path.canonicalize()?;
                            anyhow::ensure!(path.is_file(), "请选择文件作为附件");
                            path
                        };
                        if !self.work.attachments.contains(&local) {
                            self.work.attachments.push(local);
                        }
                    }
                    self.work_files()?;
                    self.work.status = "附件已加入草稿，发送后交给 Amoo 处理".into();
                }
            }
            WorkAction::RemoveAttachment(index) => {
                anyhow::ensure!(!self.busy && !self.work.running, "任务进行中，附件已经提交");
                if *index < self.work.attachments.len() {
                    self.work.attachments.remove(*index);
                }
            }
            WorkAction::SelectModel(model) => {
                anyhow::ensure!(!self.work.running, "工作进行中，模型不能中途变更");
                if let Some(id) = model {
                    let catalog = self
                        .work
                        .catalog
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("请先刷新模型目录"))?;
                    anyhow::ensure!(
                        catalog.can_select_model,
                        "当前手机 Amoo 云会话没有开放模型指定接口，将使用手机默认路由"
                    );
                    anyhow::ensure!(
                        catalog.models.iter().any(|m| m.id == *id
                            && m.is_verified_for(self.amoo_ui.account_scope.as_deref())),
                        "该模型未在手机配置，不能用于当前任务"
                    );
                }
                self.work.selected_model = model.clone();
                self.work.status = "下一条任务将使用所选模型；发送前会校验手机回执".into();
            }
            WorkAction::SelectAgent(id) => {
                let agent = self
                    .work
                    .catalog
                    .as_ref()
                    .and_then(|catalog| catalog.agents.iter().find(|agent| agent.id == *id))
                    .ok_or_else(|| anyhow::anyhow!("智能体已不在当前手机列表，请刷新"))?;
                self.work.status = format!(
                    "{} 属于手机 OpenClaw 环境，当前 Amoo 云会话不支持切换到该智能体",
                    agent.name
                );
            }
            WorkAction::NewChat => {
                anyhow::ensure!(
                    !self.busy && !self.work.running,
                    "请等待当前操作完成后新建任务"
                );
                self.work.tab = 0;
                self.work.capabilities_open = false;
                self.work.panel = 2;
                self.work.events.clear();
                self.work.artifacts.clear();
                self.work.error = None;
                self.work.active_key = None;
                self.work.mode = 1;
                self.fields[F_WORK_PROMPT].clear();
                self.work.attachments.clear();
                self.work.attach_menu_open = false;
                self.work.local_preview = None;
                self.work.local_preview_loading = None;
                self.work.local_preview_error = None;
                self.work.remote_preview_url = None;
                self.work.revise_hint = None;
                self.focus_field(F_WORK_PROMPT, ctx);
                // 只清空文件处理页自己的会话；发送第一条任务时再创建，
                // 不会改动 Amoo 当前对话。
                self.amoo_session_idx = usize::MAX;
                self.work.status = "已新建，描述要处理的事情即可".into();
            }
            WorkAction::Session(index) => {
                anyhow::ensure!(
                    !self.busy && !self.work.running,
                    "正在同步或执行工作，请等待完成后切换会话"
                );
                if *index < self.amoo_sessions.len() {
                    self.clear_text_interaction();
                    self.work.panel = 2;
                    self.work.capabilities_open = false;
                    self.work.remote_preview_url = None;
                    self.work.local_preview = None;
                    self.work.local_preview_loading = None;
                    self.work.local_preview_error = None;
                    self.amoo_session_idx = *index;
                    self.work.revise_hint = None;
                    self.work.events.clear();
                    self.work.artifacts.clear();
                    self.work.error = None;
                    self.work.attachments.clear();
                    self.work.active_key = None;
                    self.work.status = "已切换工作会话".into();
                    self.fields[F_WORK_PROMPT].clear();
                    self.work_files()?;
                    self.work.tab = 0;
                    self.work.selected_model = None;
                    self.scroll = Default::default();
                    self.reveal_latest_chat();
                    if !self.busy {
                        let key = self.amoo_sessions[*index].session_key.clone();
                        self.with_device("正在同步工作会话…", move |adb, device| {
                            Ok(Outcome::AmooRealData(
                                amoo::fetch(adb, &device, Some(&key))?,
                                "已同步工作会话".into(),
                            ))
                        });
                    }
                }
            }
            WorkAction::Template(index) => {
                anyhow::ensure!(
                    !self.work.running,
                    "请等待当前任务完成或停止执行后开始新任务"
                );
                let prompts = [
                    "请整理工作文件夹中的资料：先列出资料清单，提炼关键结论，标出来源文件和不确定点，生成可阅读的总结报告并保存到输出文件夹。",
                    "请分析工作文件夹中的表格：核对列含义与缺失值，统计关键指标，指出异常及依据，生成分析报告与可核对的结果表。",
                    "请根据工作文件夹中的资料撰写一份工作文档：先明确目标和结构，生成完整正文，核查数字和来源，保存成可打开的文档。",
                    "请使用手机上实际可用的图片生成能力完成图片创作，并返回生成图片及可访问的产物位置。开始前确认主题、尺寸和风格；缺少能力时请明确说明。",
                    WORK_GAME_PROMPT,
                    "请根据工作资料制作一份结构清楚、版面简洁的 PowerPoint：提炼结论和依据，每页一个主题，生成真正可编辑的 PPTX 文件并检查页面内容。资料不足的地方明确标注，不要编造数据。",
                ];
                let prompt = prompts[usize::from(*index).min(prompts.len() - 1)];
                if *index == 4 {
                    // 做游戏交给 Amoo 自动处理，结果回到对话里。
                    self.clear_text_interaction();
                    self.work.status = "已交给 Amoo 来做这个小游戏".into();
                    self.work.amoo_handoff = Some(prompt.to_string());
                } else {
                    let _ = self.ensure_work_default_workspace();
                    self.fields[F_WORK_PROMPT].set(prompt);
                    self.work.mode = if *index == 3 { 2 } else { 1 };
                    self.work.tab = 0;
                    self.work.capabilities_open = false;
                    self.trigger_subtab_switch();
                    self.focus_field(F_WORK_PROMPT, ctx);
                }
            }
            WorkAction::QuickStarter(index) => {
                anyhow::ensure!(
                    !self.work.running,
                    "请等待当前任务完成或停止执行后生成新样板"
                );
                if *index == 4 {
                    // 做游戏交给 Amoo 自动处理，结果回到对话里。
                    self.clear_text_interaction();
                    self.work.status = "已交给 Amoo 来做这个小游戏".into();
                    self.work.amoo_handoff = Some(WORK_GAME_PROMPT.to_string());
                    return Ok(());
                }
                let kind = match *index {
                    0 => "docx",
                    1 => "xlsx",
                    2 => "pptx",
                    _ => "md",
                };
                let root = self.ensure_work_default_workspace()?;
                let artifact = work::generate_office_starter(&root, kind)?;
                self.workspace_output(stepx_companion::amoo_workspace::Output {
                    path: artifact.path.clone(),
                    name: artifact
                        .path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                        .into_owned(),
                    kind: artifact.kind.clone(),
                    lab_id: None,
                });
                self.work_files()?;
                self.work.panel = 2;
                self.trigger_subtab_switch();
                self.work.remote_preview_url = None;
                self.work.local_preview_loading = None;
                self.work.local_preview_error = None;
                self.work.local_preview = read_work_preview(&root, &artifact.path).ok();
                self.work.log(format!(
                    "已生成样板 {}（{}）",
                    artifact.relative_path, artifact.kind
                ));
                self.work.status = format!(
                    "已生成 {}（{}），右侧可以预览、修改或发到手机",
                    artifact.relative_path, artifact.kind
                );
            }
            WorkAction::OpenStudioWorkbench => {
                // 做应用不再是单独的页面：回到 Amoo 直接说需求即可。
                self.clear_text_interaction();
                self.work.amoo_handoff = Some(String::new());
            }
            WorkAction::ReviseFile(path) => {
                anyhow::ensure!(
                    !self.work.running,
                    "请等待当前任务完成或停止执行后再发起修订"
                );
                let root = self.ensure_work_default_workspace()?;
                validate_open_path(Some(&root), path)?;
                if !self.work.attachments.contains(path) && self.work.attachments.len() < 8 {
                    self.work.attachments.push(path.clone());
                }
                let filename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                self.work.mode = 1;
                self.work.tab = 0;
                self.work.capabilities_open = false;
                // 不再替用户写固定的修订要求，只清空输入框并提示要改哪个文件。
                self.fields[F_WORK_PROMPT].clear();
                self.work.revise_hint = Some(format!("说说要怎么改「{filename}」"));
                self.focus_field(F_WORK_PROMPT, ctx);
                self.work.status = format!("已附上「{filename}」，说说要怎么改");
            }
            WorkAction::DeliverFileToPhone(path) => {
                let root = self.ensure_work_default_workspace()?;
                validate_open_path(Some(&root), path)?;
                let filename = path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Amoo交付物");
                let Some(device) = self.selected.clone() else {
                    anyhow::bail!(
                        "请先连接手机后再同步文件；当前文件已保存在本地工作区 {}",
                        path.display()
                    );
                };
                let adb = Adb::discover()?;
                let remote_dir = "/sdcard/Download/StepX-Work";
                let remote_path = format!("{remote_dir}/{filename}");
                let _ = adb.shell(&device.transport, &["mkdir", "-p", remote_dir]);
                adb.transfer(
                    &device.transport,
                    "push",
                    path,
                    std::path::Path::new(&remote_path),
                )?;
                let _ = adb.shell(
                    &device.transport,
                    &[
                        "am",
                        "broadcast",
                        "-a",
                        "android.intent.action.MEDIA_SCANNER_SCAN_FILE",
                        "-d",
                        &format!("file://{remote_path}"),
                    ],
                );
                let _ = stepx_companion::work_sync::enqueue(
                    &root,
                    std::slice::from_ref(path),
                    &device.serial,
                );
                self.work
                    .log(format!("已同步到手机 Download/StepX-Work/{filename}"));
                self.work.status = format!("已同步到手机 Download/StepX-Work/{filename}");
            }
            WorkAction::TurnIntoApp(path) => {
                let root = self.ensure_work_default_workspace()?;
                validate_open_path(Some(&root), path)?;
                let text = self
                    .work
                    .local_preview
                    .as_ref()
                    .filter(|p| &p.path == path)
                    .and_then(|p| p.text.clone())
                    .or_else(|| read_work_preview(&root, path).ok().and_then(|p| p.text))
                    .unwrap_or_else(|| {
                        work::Workspace::open(&root)
                            .and_then(|ws| ws.relative(path).and_then(|rel| ws.read_text(&rel)))
                            .unwrap_or_default()
                    });
                let excerpt: String = text.chars().take(3000).collect();
                let filename = path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned();
                let prompt = if excerpt.trim().is_empty() {
                    format!("请围绕办公文档「{filename}」制作一个可交互的移动端 Web 数据看板与应用，包含核心指标卡片、图表可视化与筛选交互。")
                } else {
                    format!("请将以下办公交付物「{filename}」转化为可交互的移动端 Web 应用/数据看板（包含指标卡片、可视化视图、筛选切换与现代深色界面）：\n\n{excerpt}")
                };
                // 交给 Amoo 自动做成应用，结果回到对话里。
                self.clear_text_interaction();
                self.work.amoo_handoff = Some(prompt);
                self.work.status = format!("已把「{filename}」交给 Amoo 做成应用");
            }
            WorkAction::Send => self.send_work()?,
            WorkAction::Cancel => {
                if let Some(handle) = self.work.handle.take() {
                    handle.cancel.store(true, Ordering::Relaxed);
                    self.work.running = false;
                    self.work.handle = None;
                    if let Some(draft) = self.work.submitted_draft.take() {
                        if self.fields[F_WORK_PROMPT].text.is_empty() {
                            self.fields[F_WORK_PROMPT].set(&draft);
                        }
                    }
                    self.work.status = "已停止当前任务".into();
                    self.work.log(self.work.status.clone());
                } else if let Some(session) = self.amoo_sessions.get(self.amoo_session_idx) {
                    self.amoo_pending_reply.remove(&session.session_key);
                    self.work.status =
                        "已停止等待；手机任务可能仍在后台继续，可稍后同步查看结果".into();
                }
                if self.workspace.running.is_some() {
                    self.workspace_finish(
                        stepx_companion::amoo_workspace::Phase::Stopped,
                        "已停止".into(),
                    );
                    self.workspace.running = None;
                }
            }
            WorkAction::OpenRemote(url) => {
                anyhow::ensure!(
                    url.starts_with("https://") || url.starts_with("http://"),
                    "手机未提供可打开的网页链接"
                );
                anyhow::ensure!(!url.chars().any(char::is_control), "附件链接无效");
                let command = if cfg!(target_os = "macos") {
                    "open"
                } else if cfg!(windows) {
                    "explorer.exe"
                } else {
                    "xdg-open"
                };
                let mut child = std::process::Command::new(command).arg(url).spawn()?;
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                self.work.status = "已在浏览器打开链接".into();
            }
            WorkAction::PreviewAttachment {
                name,
                kind,
                mime,
                url,
            } => {
                if self.work.preview_pending.contains(url) {
                    return Ok(());
                }
                self.work.preview_attempted.remove(url);
                self.work.preview_errors.remove(url);
                self.begin_work_previews(vec![stepx_companion::openclaw::RealAttachment {
                    name: name.clone(),
                    kind: kind.clone(),
                    mime: mime.clone(),
                    url: url.clone(),
                    ..Default::default()
                }])?;
            }
            WorkAction::SaveAttachment {
                name,
                kind,
                mime,
                url,
            } => {
                anyhow::ensure!(!self.busy, "请等待当前同步完成后保存附件");
                let filename = std::path::Path::new(name)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Amoo附件");
                if let Some(path) = native_dialog::FileDialog::new()
                    .set_title("保存 Amoo 附件")
                    .set_filename(filename)
                    .set_location(&stepx_companion::settings::downloads())
                    .show_save_single_file()?
                {
                    anyhow::ensure!(!path.exists(), "该文件已存在，请选择新名称保存");
                    let attachment = stepx_companion::openclaw::RealAttachment {
                        name: name.clone(),
                        kind: kind.clone(),
                        mime: mime.clone(),
                        url: url.clone(),
                        ..Default::default()
                    };
                    let cached = self.work.image_previews.get(url).cloned();
                    let scope = self.amoo_ui.account_scope.clone();
                    let identity = self.work.preview_identities.get(url).cloned();
                    self.with_device("正在保存手机附件…", move |adb, device| {
                        if let Some(expected) = scope.as_ref() {
                            anyhow::ensure!(
                                stepx_companion::amoo_usage::account_scope(adb, &device)?.as_ref()
                                    == Some(expected),
                                "手机账号已变化，请重新同步后保存附件"
                            );
                        }
                        let persistent = preview_cache::root().zip(scope);
                        let stored = persistent.as_ref().and_then(|(root, scope)| {
                            preview_cache::load_with_identity(
                                root,
                                scope,
                                &attachment.url,
                                identity.as_deref(),
                            )
                        });
                        let bytes = if let Some(source) = stored.or(cached.filter(|p| p.is_file()))
                        {
                            let mut input = std::fs::File::open(source)?;
                            let mut output = std::fs::OpenOptions::new()
                                .write(true)
                                .create_new(true)
                                .open(&path)?;
                            std::io::copy(&mut input, &mut output)?
                        } else {
                            let file =
                                amoo_media::download_artifact(adb, &device, &attachment, &path)?;
                            if attachment.kind == "image" || attachment.mime.starts_with("image/") {
                                if let Some((root, scope)) = persistent.as_ref() {
                                    let _ = preview_cache::save_with_identity(
                                        root,
                                        scope,
                                        &attachment.url,
                                        &file.path,
                                        identity.as_deref(),
                                    );
                                }
                            }
                            file.bytes
                        };
                        Ok(Outcome::Work(WorkOutcome::AttachmentSaved { path, bytes }))
                    });
                }
            }
            WorkAction::OpenPreview(url) => {
                if !self.work.image_previews.contains_key(url)
                    && std::path::Path::new(url).is_file()
                {
                    self.work
                        .insert_image_preview(url.clone(), PathBuf::from(url));
                }
                anyhow::ensure!(
                    self.work
                        .image_previews
                        .get(url)
                        .is_some_and(|p| p.is_file()),
                    "图片尚未载入，请重试"
                );
                self.work.image_viewer = Some(url.clone());
                self.work.image_viewer_zoom = 100;
                self.work.image_viewer_rotated_path = None;
                self.work.image_viewer_rotation = 0;
                self.work.preview_scroll = Default::default();
            }
            WorkAction::CloseImagePreview => {
                self.work.image_viewer = None;
                self.work.image_viewer_zoom = 100;
                self.work.image_viewer_rotated_path = None;
                self.work.image_viewer_rotation = 0;
                self.work.preview_scroll = Default::default();
            }
            WorkAction::ZoomImagePreview(zoom) => {
                self.work.image_viewer_zoom = (*zoom).clamp(50, 400);
            }
            WorkAction::ZoomInImagePreview => {
                self.work.image_viewer_zoom = match self.work.image_viewer_zoom {
                    z if z < 100 => 100,
                    100 => 150,
                    150 => 200,
                    200 => 300,
                    _ => 400,
                };
            }
            WorkAction::ZoomOutImagePreview => {
                self.work.image_viewer_zoom = match self.work.image_viewer_zoom {
                    z if z > 300 => 300,
                    z if z > 200 => 200,
                    z if z > 150 => 150,
                    z if z > 100 => 100,
                    _ => 50,
                };
            }
            WorkAction::RotateImagePreview => {
                let url = self
                    .work
                    .image_viewer
                    .as_ref()
                    .context("请先打开图片预览")?
                    .clone();
                let base_path = self
                    .work
                    .image_previews
                    .get(&url)
                    .cloned()
                    .or_else(|| {
                        let p = PathBuf::from(&url);
                        p.is_file().then_some(p)
                    })
                    .context("图片文件不存在")?;
                let next_rot = (self.work.image_viewer_rotation + 90) % 360;
                if next_rot == 0 {
                    self.work.image_viewer_rotation = 0;
                    self.work.image_viewer_rotated_path = None;
                    self.work.status = "图片已恢复默认方向".into();
                } else {
                    let src_path = self
                        .work
                        .image_viewer_rotated_path
                        .clone()
                        .unwrap_or(base_path);
                    let img = image::open(&src_path).context("读取待旋转图片失败")?;
                    let rotated = image::imageops::rotate90(&img);
                    if self.work.preview_cache.is_none() {
                        self.work.preview_cache = Some(Arc::new(tempfile::tempdir()?));
                    }
                    let cache = self
                        .work
                        .preview_cache
                        .as_ref()
                        .context("无法创建图片缓存")?;
                    let out_path = cache.path().join(format!(
                        "amoo-rot-{next_rot}-{:016x}.png",
                        rand::random::<u64>()
                    ));
                    rotated.save(&out_path).context("保存旋转后图片失败")?;
                    self.work.image_viewer_rotation = next_rot;
                    self.work.image_viewer_rotated_path = Some(out_path);
                    self.work.status = format!("图片已顺时针旋转至 {next_rot}°");
                }
            }
            WorkAction::OpenWorkspace => {
                let path = self
                    .work
                    .workspace
                    .as_ref()
                    .ok_or_else(|| anyhow::anyhow!("请先选择工作文件夹"))?;
                open_work_path(path)?;
            }
            WorkAction::OpenFile(index) => {
                let file = self
                    .work
                    .files
                    .get(*index)
                    .ok_or_else(|| anyhow::anyhow!("文件已变化，请刷新"))?;
                validate_open_path(self.work.workspace.as_deref(), &file.path)?;
                open_work_path(&file.path)?;
            }
            WorkAction::OpenArtifact(index) => {
                let file = self
                    .work
                    .artifacts
                    .get(*index)
                    .ok_or_else(|| anyhow::anyhow!("产物已变化，请刷新"))?;
                validate_open_path(self.work.workspace.as_deref(), &file.path)?;
                open_work_path(&file.path)?;
            }
            WorkAction::PreviewArtifact(index) => {
                let path = self
                    .work
                    .artifacts
                    .get(*index)
                    .context("产物已变化，请刷新")?
                    .path
                    .clone();
                self.preview_work_file(path)?;
            }
            WorkAction::PreviewFile(index) => {
                let file = self.work.files.get(*index).context("文件已变化，请刷新")?;
                if file.is_dir {
                    validate_open_path(self.work.workspace.as_deref(), &file.path)?;
                    open_work_path(&file.path)?;
                } else {
                    self.preview_work_file(file.path.clone())?;
                }
            }
            WorkAction::PreviewRemote(index) => {
                let item = self
                    .amoo_sessions
                    .get(self.amoo_session_idx)
                    .and_then(|session| session.items.get(*index))
                    .context("附件已变化，请同步会话")?;
                if let AmooChatItem::Attachment { url, path, .. } = item {
                    self.work.remote_preview_url = Some(if url.is_empty() {
                        path.clone()
                    } else {
                        url.clone()
                    });
                    self.work.local_preview = None;
                    self.work.local_preview_loading = None;
                    self.work.local_preview_error = None;
                    self.work.panel = 2;
                }
            }
            WorkAction::ClosePreview => {
                self.work.local_preview = None;
                self.work.local_preview_loading = None;
                self.work.local_preview_error = None;
                self.work.remote_preview_url = None;
            }
            WorkAction::OpenLocalPreview => {
                if let Some(url) = &self.work.image_viewer {
                    if let Some(path) = self
                        .work
                        .image_viewer_rotated_path
                        .as_ref()
                        .or_else(|| self.work.image_previews.get(url))
                        .filter(|p| p.is_file())
                    {
                        open_work_path(path)?;
                        return Ok(());
                    }
                }
                let preview = self
                    .work
                    .local_preview
                    .as_ref()
                    .context("请先选择文件预览")?;
                validate_open_path(self.work.workspace.as_deref(), &preview.path)?;
                open_work_path(&preview.path)?;
            }
            WorkAction::ExportLocalPreview => {
                anyhow::ensure!(!self.busy, "请等待当前操作完成");
                let preview = self
                    .work
                    .local_preview
                    .as_ref()
                    .context("请先选择文件预览")?;
                validate_open_path(self.work.workspace.as_deref(), &preview.path)?;
                let filename = preview
                    .path
                    .file_name()
                    .and_then(|s| s.to_str())
                    .unwrap_or("Amoo工作成果");
                if let Some(destination) = native_dialog::FileDialog::new()
                    .set_title("导出工作成果")
                    .set_filename(filename)
                    .set_location(&stepx_companion::settings::downloads())
                    .show_save_single_file()?
                {
                    let source = preview.path.clone();
                    let root = self.work.workspace.clone();
                    self.submit("正在导出工作成果…", move |_| {
                        validate_open_path(root.as_deref(), &source)?;
                        let mut input = std::fs::File::open(&source)?;
                        let mut output = std::fs::OpenOptions::new()
                            .write(true)
                            .create_new(true)
                            .open(&destination)
                            .context("目标文件已存在或无法写入，请使用新名称")?;
                        let bytes = std::io::copy(&mut input, &mut output)?;
                        Ok(Outcome::Work(WorkOutcome::AttachmentSaved {
                            path: destination,
                            bytes,
                        }))
                    });
                }
            }
            WorkAction::InstallSkill { key, install_id } => {
                anyhow::ensure!(!self.busy && !self.work.running, "请等待当前操作完成");
                self.work.pending_install = Some((key.clone(), install_id.clone()));
            }
            WorkAction::CancelSkillInstall => self.work.pending_install = None,
            WorkAction::ConfirmSkillInstall => self.install_work_skill()?,
            WorkAction::ToggleSkill { key, enabled } => self.toggle_work_skill(key, *enabled)?,
        }
        Ok(())
    }

    pub(super) fn preview_work_file(&mut self, path: PathBuf) -> Result<()> {
        anyhow::ensure!(!self.busy, "请等待当前操作完成后预览");
        let workspace = self.work.workspace.clone().context("请先选择工作文件夹")?;
        validate_open_path(Some(&workspace), &path)?;
        self.work.panel = 2;
        self.work.local_preview = None;
        self.work.local_preview_error = None;
        self.work.remote_preview_url = None;
        self.work.local_preview_loading = Some(path.clone());
        self.submit("正在读取文件预览…", move |_| {
            let result = read_work_preview(&workspace, &path).map_err(|error| format!("{error:#}"));
            Ok(Outcome::Work(WorkOutcome::LocalPreview {
                workspace,
                path,
                result,
            }))
        });
        Ok(())
    }

    fn send_work(&mut self) -> Result<()> {
        anyhow::ensure!(
            !self.busy && !self.work.running,
            "当前操作尚未完成，草稿已保留"
        );
        let draft = self.fields[F_WORK_PROMPT].text.trim().to_string();
        anyhow::ensure!(!draft.is_empty(), "请输入工作任务");
        self.work.revise_hint = None;
        let session = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .map(|s| s.session_key.clone());
        anyhow::ensure!(
            !session
                .as_ref()
                .is_some_and(|key| self.amoo_pending_reply.contains_key(key)),
            "该会话仍在等待手机回复，请勿重复发送"
        );
        if self.work.mode != 1 {
            let _device = self
                .selected
                .clone()
                .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?;
            let model = self.work.selected_model.clone();
            let attachments = self.work.attachments.clone();
            let image_task = self.work.mode == 2;
            self.with_device(
                if attachments.is_empty() {
                    "正在把任务交给手机 Amoo…"
                } else {
                    "正在上传附件并交给手机 Amoo…"
                },
                move |adb, device| {
                    let mut bus = amoo::AgentBus::connect(adb, &device)?;
                    let key = match session {
                        Some(key) => key,
                        None => bus.create_session()?,
                    };
                    if let Some(model) = model {
                        bus.set_session_model(&key, &model)?;
                    }
                    let message = if image_task {
                        amoo_media::image_request(&draft)
                    } else {
                        draft.clone()
                    };
                    let mut receipt = if attachments.is_empty() {
                        bus.send(&key, &message)?
                    } else {
                        amoo_media::send_files(adb, &device, &key, &message, &attachments)?
                    };
                    receipt.text = draft.clone();
                    Ok(Outcome::Work(WorkOutcome::Sent {
                        receipt,
                        draft,
                        attachments,
                    }))
                },
            );
        } else {
            let workspace = self.ensure_work_default_workspace()?;
            let attachments = self.work.attachments.clone();
            let request = work::WorkRequest {
                prompt: draft.clone(),
                workspace,
                session_key: session,
                model: self.work.selected_model.clone(),
                attachments,
            };
            let handle = if let Some(device) = self.selected.clone() {
                work::start(Adb::discover()?, device, request)
            } else if work::has_local_model_backend() {
                work::start_local(request)
            } else {
                anyhow::bail!("请先连接手机，或在「模型与账号」里填写自己的模型服务");
            };
            self.work.handle = Some(handle);
            self.work.submitted_draft = Some(draft);
            self.work.final_reply = None;
            self.work.running = true;
            self.work.events.clear();
            self.work.artifacts.clear();
            self.work.status = if self.selected.is_some() {
                "正在启动桌面协作，使用手机模型处理工作文件…".into()
            } else {
                "正在启动桌面协作，使用本地配置模型处理工作文件…".into()
            };
            self.fields[F_WORK_PROMPT].clear();
        }
        Ok(())
    }

    pub(super) fn poll_work(&mut self) -> bool {
        if self.work.handle.is_none() && self.work.preview_rx.is_none() {
            return false;
        }
        self.with_work_session(Self::poll_work_in_session)
    }

    fn poll_work_in_session(&mut self) -> bool {
        let mut preview_changed = false;
        let mut preview_done = false;
        let mut preview_outcomes = Vec::new();
        if let Some(receiver) = &self.work.preview_rx {
            loop {
                match receiver.try_recv() {
                    Ok(item) => preview_outcomes.push(item),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        preview_done = true;
                        break;
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                }
            }
        }
        for (url, res) in preview_outcomes {
            match res {
                Ok(path) => {
                    self.work.preview_pending.remove(&url);
                    self.work.preview_errors.remove(&url);
                    self.work.insert_image_preview(url, path);
                    preview_changed = true;
                }
                Err(error) => {
                    self.work.preview_pending.remove(&url);
                    self.work.preview_errors.insert(url, error);
                    preview_changed = true;
                }
            }
        }
        if preview_done || (preview_changed && self.work.preview_pending.is_empty()) {
            self.work.preview_rx = None;
            self.work.preview_tx = None;
        }
        if preview_changed {
            self.load_work_previews();
        }
        let mut events = Vec::new();
        if let Some(handle) = &self.work.handle {
            loop {
                match handle.events.try_recv() {
                    Ok(event) => events.push(event),
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        if self.work.running
                            && !events.iter().any(|event| {
                                matches!(
                                    event,
                                    work::WorkEvent::Complete(_)
                                        | work::WorkEvent::Failed(_)
                                        | work::WorkEvent::Cancelled
                                )
                            })
                        {
                            events.push(work::WorkEvent::Failed(
                                "桌面执行连接已结束，请检查日志后继续；未自动重发手机任务".into(),
                            ));
                        }
                        break;
                    }
                }
            }
        }
        let changed = preview_changed || !events.is_empty();
        for event in events {
            match event {
                work::WorkEvent::SessionReady(key) => {
                    self.workspace_bind_session(&key);
                    self.work.active_key = Some(key.clone());
                    if let Some(index) = self
                        .amoo_sessions
                        .iter()
                        .position(|session| session.session_key == key)
                    {
                        self.amoo_session_idx = index;
                    } else {
                        self.apply(Outcome::AmooCreated(key));
                    }
                }
                work::WorkEvent::ModelResponse {
                    data,
                    key,
                    run_id,
                    elapsed_ms,
                } => {
                    if self.amoo_ui.account_scope.is_some()
                        && self.amoo_ui.account_scope != data.account_scope
                    {
                        self.work.reset_account_work();
                        break;
                    }
                    self.record_work_response(&data, &key, &run_id, elapsed_ms);
                }
                work::WorkEvent::Status(status) => {
                    self.work.status = status.clone();
                    self.work.log(status);
                }
                work::WorkEvent::Assistant(text) => {
                    self.work.final_reply = Some(text.clone());
                    self.work.log(text);
                }
                work::WorkEvent::Tool(log) => {
                    let friendly = Self::work_step_name(&log.tool);
                    self.work.status = format!("{} · {}", log.step, friendly);
                    self.work.log(format!(
                        "{} · {} · {}{}",
                        log.step,
                        friendly,
                        if log.success { "" } else { "失败：" },
                        log.detail
                    ));
                }
                work::WorkEvent::Artifact(file) => {
                    self.workspace_output(stepx_companion::amoo_workspace::Output {
                        path: file.path.clone(),
                        name: file
                            .path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned(),
                        kind: file.kind.clone(),
                        lab_id: None,
                    });
                    self.work.log(format!("已生成 {}", file.relative_path));
                    self.work.artifacts.push(WorkArtifactItem {
                        name: file.relative_path,
                        path: file.path,
                        kind: file.kind,
                    });
                }
                work::WorkEvent::Complete(summary) => {
                    if let Some(key) = &self.work.active_key {
                        self.reply_motion.live.insert(key.clone());
                    }
                    if self.workspace.running.is_none()
                        && self.settings.value.sync_work_outputs
                        && !self.work.artifacts.is_empty()
                    {
                        if let (Some(root), Some(device)) =
                            (self.work.workspace.clone(), self.selected.clone())
                        {
                            let paths = self
                                .work
                                .artifacts
                                .iter()
                                .map(|a| a.path.clone())
                                .collect::<Vec<_>>();
                            let (tx, rx) = mpsc::channel();
                            self.settings.staging = Some(rx);
                            std::thread::spawn(move || {
                                let _ = tx.send(
                                    stepx_companion::work_sync::enqueue(
                                        &root,
                                        &paths,
                                        &device.serial,
                                    )
                                    .map(|n| format!("已准备 {n} 个成果，连接手机后发送"))
                                    .map_err(|e| e.to_string()),
                                );
                            });
                        }
                    }
                    let delivered = !self.work.artifacts.is_empty();
                    let reply = self.work.final_reply.take().unwrap_or_default();
                    self.workspace_finish(
                        if delivered {
                            stepx_companion::amoo_workspace::Phase::Done
                        } else {
                            stepx_companion::amoo_workspace::Phase::Failed
                        },
                        if delivered {
                            format!("{reply}\n\n{summary}")
                        } else {
                            format!("尚未收到实际文件，请补充要求后继续。{summary}")
                        },
                    );
                    self.work.running = false;
                    self.work.submitted_draft = None;
                    self.work.status = summary.clone();
                    self.work.log(summary);
                    self.work.attachments.clear();
                    let _ = self.work_files();
                    self.last_amoo_poll = Instant::now() - Duration::from_secs(11);
                }
                work::WorkEvent::Failed(error) => {
                    self.workspace_finish(
                        stepx_companion::amoo_workspace::Phase::Failed,
                        error.clone(),
                    );
                    self.work.running = false;
                    if self.fields[F_WORK_PROMPT].text.is_empty() {
                        if let Some(draft) = self.work.submitted_draft.take() {
                            self.fields[F_WORK_PROMPT].set(&draft);
                        }
                    }
                    self.work.error = Some(error.clone());
                    self.work.log(error);
                    self.last_amoo_poll = Instant::now() - Duration::from_secs(11);
                }
                work::WorkEvent::Cancelled => {
                    self.workspace_finish(
                        stepx_companion::amoo_workspace::Phase::Stopped,
                        "桌面执行已停止；手机已接收的任务可能仍继续".into(),
                    );
                    self.work.running = false;
                    if let Some(draft) = self.work.submitted_draft.take() {
                        if self.fields[F_WORK_PROMPT].text.is_empty() {
                            self.fields[F_WORK_PROMPT].set(&draft);
                        }
                    }
                    self.work.status = "桌面执行已停止；已提交到手机的任务仍可能继续".into();
                    self.work.log(self.work.status.clone());
                }
            }
        }
        if !self.work.running {
            self.work.handle = None;
        }
        changed
    }

    pub(super) fn load_work_previews(&mut self) {
        if !matches!(self.page, Page::Amoo | Page::Work)
            || self.selected.is_none()
            || self.work.preview_pending.len() >= 3
        {
            return;
        }
        let session = if self.page == Page::Work {
            self.work_session()
        } else {
            self.amoo_sessions.get(self.amoo_session_idx)
        };
        let Some(session) = session else {
            return;
        };
        let attachments = conversation_images(&session.items)
            .into_iter()
            .filter(|a| {
                !self.work.preview_attempted.contains(&a.url)
                    && !self.work.image_previews.contains_key(&a.url)
            })
            .take(3 - self.work.preview_pending.len())
            .collect::<Vec<_>>();
        if !attachments.is_empty() {
            if let Err(e) = self.begin_work_previews(attachments.clone()) {
                for attachment in attachments {
                    self.work.preview_attempted.insert(attachment.url.clone());
                    self.work
                        .preview_errors
                        .insert(attachment.url, e.to_string());
                }
            }
        }
    }

    pub(super) fn work_previews_loading(&self) -> bool {
        !self.work.preview_pending.is_empty()
    }

    fn begin_work_previews(
        &mut self,
        attachments: Vec<stepx_companion::openclaw::RealAttachment>,
    ) -> Result<()> {
        let device = self.selected.clone().context("请连接手机后载入图片")?;
        let adb = Adb::discover()?;
        if self.work.preview_cache.is_none() {
            self.work.preview_cache = Some(Arc::new(tempfile::tempdir()?));
        }
        let cache = self
            .work
            .preview_cache
            .clone()
            .context("无法创建图片缓存")?;
        anyhow::ensure!(
            self.work.preview_pending.len() + attachments.len() <= 3,
            "其他图片正在载入，稍后自动重试"
        );
        if self.work.preview_rx.is_none() {
            let (tx, rx) = mpsc::channel();
            self.work.preview_tx = Some(tx);
            self.work.preview_rx = Some(rx);
        }
        for attachment in attachments {
            self.work.preview_attempted.insert(attachment.url.clone());
            self.work.preview_pending.insert(attachment.url.clone());
            let sender = self
                .work
                .preview_tx
                .as_ref()
                .context("图片队列不可用")?
                .clone();
            let expected_scope = self.amoo_ui.account_scope.clone();
            let identity = self.work.preview_identities.get(&attachment.url).cloned();
            let adb = adb.clone();
            let device = device.clone();
            let cache = cache.clone();
            std::thread::spawn(move || {
                let started = Instant::now();
                let mut cached = false;
                let scope_matches = || {
                    expected_scope.is_none()
                        || stepx_companion::amoo_usage::account_scope(&adb, &device)
                            .ok()
                            .flatten()
                            == expected_scope
                };
                let result = (|| -> Result<PathBuf> {
                    anyhow::ensure!(scope_matches(), "手机账号已变化，请重新同步后查看图片");
                    let persistent = expected_scope
                        .as_ref()
                        .and_then(|scope| preview_cache::root().map(|root| (root, scope.clone())));
                    let stored = persistent.as_ref().and_then(|(root, scope)| {
                        preview_cache::load_with_identity(
                            root,
                            scope,
                            &attachment.url,
                            identity.as_deref(),
                        )
                    });
                    let path = if let Some(path) = stored {
                        cached = true;
                        path
                    } else {
                        let output = cache
                            .path()
                            .join(format!("amoo-{:016x}.png", rand::random::<u64>()));
                        let file =
                            amoo_media::download_artifact(&adb, &device, &attachment, &output)?;
                        persistent
                            .as_ref()
                            .and_then(|(root, scope)| {
                                preview_cache::save_with_identity(
                                    root,
                                    scope,
                                    &attachment.url,
                                    &file.path,
                                    identity.as_deref(),
                                )
                                .ok()
                            })
                            .unwrap_or(file.path)
                    };
                    anyhow::ensure!(scope_matches(), "手机账号已变化，请重新同步后查看图片");
                    Ok(path)
                })()
                .map_err(|e| format!("{e:#}"));
                if std::env::var_os("STEPX_IMAGE_TIMING").is_some() {
                    eprintln!(
                        "IMAGE_PREVIEW cached={cached} ok={} elapsed_ms={}",
                        result.is_ok(),
                        started.elapsed().as_millis()
                    );
                }
                let _ = sender.send((attachment.url, result));
            });
        }
        Ok(())
    }

    fn install_work_skill(&mut self) -> Result<()> {
        anyhow::ensure!(!self.busy && !self.work.running, "请等待当前操作完成");
        let (key, installer) = self
            .work
            .pending_install
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先选择技能安装项"))?;
        self.with_device("正在安装手机技能依赖…", move |adb, device| {
            let catalog = amoo_catalog::install_skill(adb, &device, &key, &installer)?;
            Ok(Outcome::Work(WorkOutcome::CatalogChanged(
                catalog,
                "技能安装完成，已重新读取手机状态".into(),
            )))
        });
        Ok(())
    }

    fn toggle_work_skill(&mut self, key: &str, enabled: bool) -> Result<()> {
        anyhow::ensure!(!self.busy && !self.work.running, "请等待当前操作完成");
        let key = key.to_string();
        self.with_device("正在更新手机技能…", move |adb, device| {
            let catalog = amoo_catalog::set_skill_enabled(adb, &device, &key, enabled)?;
            Ok(Outcome::Work(WorkOutcome::CatalogChanged(
                catalog,
                "已更新手机技能状态".into(),
            )))
        });
        Ok(())
    }
}

fn work_preview_memory_cache(
) -> &'static std::sync::Mutex<HashMap<PathBuf, (u64, std::time::SystemTime, WorkLocalPreview)>> {
    static CACHE: std::sync::OnceLock<
        std::sync::Mutex<HashMap<PathBuf, (u64, std::time::SystemTime, WorkLocalPreview)>>,
    > = std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(HashMap::new()))
}

fn insert_work_preview_memory_cache(
    canonical_path: PathBuf,
    file_len: u64,
    mtime: std::time::SystemTime,
    preview: WorkLocalPreview,
) {
    if let Ok(mut mem) = work_preview_memory_cache().lock() {
        if !mem.contains_key(&canonical_path) {
            while mem.len() >= MAX_WORK_IMAGE_PREVIEWS {
                let Some(oldest) = mem
                    .iter()
                    .min_by_key(|(_, (_, entry_mtime, _))| *entry_mtime)
                    .map(|(k, _)| k.clone())
                else {
                    break;
                };
                mem.remove(&oldest);
            }
        }
        mem.insert(canonical_path, (file_len, mtime, preview));
    }
}

fn read_work_preview(root: &std::path::Path, path: &std::path::Path) -> Result<WorkLocalPreview> {
    validate_open_path(Some(root), path)?;
    let workspace = work::Workspace::open(root)?;
    let canonical_path = path.canonicalize()?;
    let relative = canonical_path
        .strip_prefix(workspace.root())
        .context("文件不在工作文件夹内")?;
    let metadata = std::fs::metadata(path)?;
    anyhow::ensure!(metadata.is_file(), "请选择普通文件预览");
    let file_len = metadata.len();
    let mtime = metadata.modified().unwrap_or(std::time::UNIX_EPOCH);
    if let Ok(mem) = work_preview_memory_cache().lock() {
        if let Some((cached_len, cached_mtime, cached_preview)) = mem.get(&canonical_path) {
            if *cached_len == file_len && *cached_mtime == mtime {
                let mut hit = cached_preview.clone();
                hit.path = path.to_path_buf();
                return Ok(hit);
            }
        }
    }
    let mtime_nanos = mtime
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let cache_key_input = format!(
        "{}|{}|{}",
        canonical_path.to_string_lossy(),
        file_len,
        mtime_nanos
    );
    let digest = work::cache_key_digest(cache_key_input.as_bytes());
    let disk_cache_dir = stepx_companion::resources::user_state().join("office-preview-cache");
    let disk_cache_path = disk_cache_dir.join(format!("{digest}.json"));
    if let Ok(bytes) = std::fs::read(&disk_cache_path) {
        if let Ok(mut hit) = serde_json::from_slice::<WorkLocalPreview>(&bytes) {
            hit.path = path.to_path_buf();
            insert_work_preview_memory_cache(canonical_path, file_len, mtime, hit.clone());
            return Ok(hit);
        }
    }
    let extension = path
        .extension()
        .and_then(|s| s.to_str())
        .unwrap_or("")
        .to_lowercase();
    let image = matches!(
        extension.as_str(),
        "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
    );
    let mut preview = WorkLocalPreview {
        path: path.to_path_buf(),
        name: path
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned(),
        bytes: metadata.len(),
        image,
        text: None,
        markdown: matches!(
            extension.as_str(),
            "md" | "markdown" | "csv" | "tsv" | "docx"
        ),
        note: String::new(),
    };
    let mut cacheable = true;
    if metadata.len() > 32 * 1024 * 1024 {
        preview.image = false;
        preview.note = "文件大于 32 MB，可使用本地应用打开或导出。".into();
    } else if image {
        let (width, height) = image::image_dimensions(path).context("图片无法解码")?;
        anyhow::ensure!(
            u64::from(width) * u64::from(height) <= 32_000_000,
            "图片分辨率超过预览上限，请打开原图查看"
        );
        preview.note = format!("{width} × {height} · 原始图片");
    } else if extension == "xlsx" {
        preview.markdown = true;
        match workspace.spreadsheet_preview(&relative.to_string_lossy()) {
            Ok(data) => {
                preview.text = Some(spreadsheet_preview_markdown(&data));
                preview.note = "表格预览 · 显示已有计算结果，最多 3 张表、20 行、8 列；公式与完整内容请打开原文件。".into();
            }
            Err(error) => {
                cacheable = false;
                preview.note = format!("此表格暂不能内嵌预览：{error}。可打开原文件或导出。")
            }
        }
    } else if matches!(extension.as_str(), "html" | "htm") {
        preview.note =
            "网页或小游戏已保存。点击“在浏览器运行”打开实际交互；源文件保留在工作文件夹内。".into();
    } else {
        match workspace.read_text(&relative.to_string_lossy()) {
            Ok(text) => {
                if matches!(extension.as_str(), "csv" | "tsv") {
                    preview.text = Some(delimited_preview(
                        &text,
                        if extension == "tsv" { '\t' } else { ',' },
                    ));
                    preview.note = "表格预览 · 最多显示 20 行、8 列，完整内容可打开原文件。".into();
                } else if extension == "pptx" {
                    preview.markdown = true;
                    preview.text = Some(
                        text.lines()
                            .map(|line| {
                                if let Some(page) = line.strip_prefix("幻灯片: ") {
                                    format!("\n### 第 {page} 页\n")
                                } else {
                                    line.to_owned()
                                }
                            })
                            .collect::<Vec<_>>()
                            .join("\n"),
                    );
                    preview.note = "逐页内容预览 · 点击打开原文件查看排版与编辑幻灯片。".into();
                } else {
                    let truncated = text.chars().count() > 12_000;
                    preview.text = Some(text.chars().take(12_000).collect());
                    preview.note = match extension.as_str() {
                        "docx" | "xlsx" | "pptx" | "pdf" => {
                            "文档文本预览 · 原始排版请打开文件查看。".into()
                        }
                        _ if truncated => "内容较长，仅显示前 12,000 字符。".into(),
                        _ => "文件内容预览".into(),
                    };
                }
            }
            Err(error) => {
                cacheable = false;
                preview.note = format!("此文件暂不能内嵌预览：{error}。可打开原文件或导出。")
            }
        }
    }
    if cacheable {
        if let Ok(json_bytes) = serde_json::to_vec(&preview) {
            let _ = std::fs::create_dir_all(&disk_cache_dir);
            let _ = std::fs::write(&disk_cache_path, json_bytes);
        }
        insert_work_preview_memory_cache(canonical_path, file_len, mtime, preview.clone());
    }
    Ok(preview)
}

fn spreadsheet_preview_markdown(data: &serde_json::Value) -> String {
    let escape = |s: &str| {
        s.chars()
            .take(160)
            .collect::<String>()
            .replace('\\', "\\\\")
            .replace('|', "\\|")
            .replace(['\r', '\n'], " ")
            .replace('`', "\\`")
            .replace('*', "\\*")
            .replace('[', "\\[")
            .replace(']', "\\]")
            .replace('<', "&lt;")
    };
    let mut out = String::new();
    if let Some(sheets) = data["sheets"].as_array() {
        for sheet in sheets.iter().take(3) {
            out.push_str(&format!(
                "### {}\n\n",
                escape(sheet["name"].as_str().unwrap_or("工作表"))
            ));
            let Some(rows) = sheet["rows"].as_array() else {
                continue;
            };
            let columns = rows
                .iter()
                .take(20)
                .filter_map(|r| r["cells"].as_array())
                .flat_map(|r| r.iter().take(8).enumerate())
                .filter(|(_, c)| !c.as_str().unwrap_or("").is_empty())
                .map(|(i, _)| i + 1)
                .max()
                .unwrap_or(1);
            out.push_str(&format!(
                "| 行 | {} |\n| {} |\n",
                (0..columns)
                    .map(|i| ((b'A' + i as u8) as char).to_string())
                    .collect::<Vec<_>>()
                    .join(" | "),
                vec!["---"; columns + 1].join(" | ")
            ));
            for row in rows.iter().take(20) {
                out.push_str(&format!(
                    "| {} | {} |\n",
                    escape(row["row"].as_str().unwrap_or("")),
                    (0..columns)
                        .map(|i| escape(row["cells"][i].as_str().unwrap_or("")))
                        .collect::<Vec<_>>()
                        .join(" | ")
                ));
            }
            out.push('\n');
        }
    }
    out
}

/// Parse quoted CSV/TSV fields without evaluating formulas or embedding HTML.
fn delimited_preview(source: &str, delimiter: char) -> String {
    let mut rows = Vec::new();
    let mut row = Vec::new();
    let mut field = String::new();
    let mut quoted = false;
    let mut chars = source.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            '"' if quoted && chars.peek() == Some(&'"') => {
                field.push('"');
                chars.next();
            }
            '"' if quoted || field.is_empty() => quoted = !quoted,
            c if c == delimiter && !quoted => row.push(std::mem::take(&mut field)),
            '\n' if !quoted => {
                row.push(std::mem::take(&mut field));
                rows.push(std::mem::take(&mut row));
                if rows.len() >= 21 {
                    break;
                }
            }
            '\r' if !quoted => {}
            c => field.push(c),
        }
    }
    if rows.len() < 21 && (!field.is_empty() || !row.is_empty()) {
        row.push(field);
        rows.push(row);
    }
    let columns = rows.iter().map(Vec::len).max().unwrap_or(1).min(8);
    let render = |row: &[String]| -> String {
        (0..columns)
            .map(|index| {
                row.get(index)
                    .map(|cell| {
                        cell.chars()
                            .take(160)
                            .collect::<String>()
                            .replace('\\', "\\\\")
                            .replace('|', "\\|")
                            .replace(['\r', '\n'], " ")
                            .replace('`', "\\`")
                            .replace('*', "\\*")
                            .replace('[', "\\[")
                            .replace(']', "\\]")
                            .replace('<', "&lt;")
                    })
                    .unwrap_or_default()
            })
            .collect::<Vec<_>>()
            .join(" | ")
    };
    let Some(header) = rows.first() else {
        return "空表格".into();
    };
    let mut markdown = format!(
        "| {} |\n| {} |\n",
        render(header),
        vec!["---"; columns].join(" | ")
    );
    for row in rows.iter().skip(1).take(20) {
        markdown.push_str(&format!("| {} |\n", render(row)));
    }
    markdown
}

fn validate_open_path(root: Option<&std::path::Path>, path: &std::path::Path) -> Result<()> {
    let selected_root = root.ok_or_else(|| anyhow::anyhow!("请先选择工作文件夹"))?;
    let root = selected_root.canonicalize()?;
    anyhow::ensure!(
        path.canonicalize()?.starts_with(&root),
        "该文件已移出当前工作文件夹，请刷新"
    );
    let relative = path
        .strip_prefix(selected_root)
        .or_else(|_| path.strip_prefix(&root))
        .context("文件不在当前工作文件夹内")?;
    let mut candidate = root;
    for component in relative.components() {
        anyhow::ensure!(
            matches!(component, std::path::Component::Normal(_)),
            "文件路径无效，请刷新工作资料"
        );
        candidate.push(component);
        anyhow::ensure!(
            !std::fs::symlink_metadata(&candidate)?
                .file_type()
                .is_symlink(),
            "工作预览不跟随符号链接"
        );
    }
    Ok(())
}

#[cfg(test)]
mod preview_tests {
    use super::*;
    #[test]
    fn spreadsheet_grid_preserves_gaps_results_and_literal_cell_text() {
        let data = serde_json::json!({"sheets":[{"name":"金额汇总","rows":[{"row":"2","cells":["A | B","","249","[链接](https://example.test)"]}]}]});
        let text = spreadsheet_preview_markdown(&data);
        assert!(text.contains("| 行 | A | B | C | D |"));
        assert!(text.contains("A \\| B |  | 249"));
        assert!(text.contains("\\[链接\\]"));
    }
    #[test]
    fn latest_inline_images_are_prioritized_and_duplicates_do_not_block_queue() {
        let reply = |text: &str| AmooChatItem::AssistantRich {
            text: Some(text.into()),
            thinking_time: String::new(),
            thinking_steps: vec![],
            tools_summary: String::new(),
            has_bolt_artifact: false,
            has_invoice_artifact: false,
            has_safety_confirm: false,
            time_label: String::new(),
        };
        let images = conversation_images(&[
            reply("![old](https://example.test/old)"),
            AmooChatItem::Attachment {
                name: "latest".into(),
                kind: "image".into(),
                mime: String::new(),
                url: "https://example.test/latest".into(),
                path: String::new(),
                size_bytes: None,
            },
            reply("![latest](https://example.test/latest)"),
        ]);
        assert_eq!(
            images.iter().map(|i| i.url.as_str()).collect::<Vec<_>>(),
            ["https://example.test/latest", "https://example.test/old"]
        );
        assert!(WorkAction::OpenPreview("image".into()).keeps_current_shell());
        assert!(WorkAction::CloseImagePreview.keeps_current_shell());
    }
    #[test]
    fn device_disconnect_drops_previous_session_results_and_inflight_previews() {
        let workspace = tempfile::tempdir().unwrap();
        let document = workspace.path().join("private-result.md");
        std::fs::write(&document, "# Previous phone's private result").unwrap();
        let mut state = WorkUiState::default();
        state.workspace = Some(workspace.path().to_path_buf());
        state.files.push(WorkFileItem {
            path: document.clone(),
            name: "private-result.md".into(),
            bytes: std::fs::metadata(&document).unwrap().len(),
            is_dir: false,
        });
        state.local_preview = Some(read_work_preview(workspace.path(), &document).unwrap());
        state.local_preview_loading = Some(document.clone());
        state.local_preview_error = Some("Previous phone preview error".into());
        state.remote_preview_url = Some("https://example.test/previous-phone.png".into());
        state.artifacts.push(WorkArtifactItem {
            path: document.clone(),
            name: "private-result.md".into(),
            kind: "document".into(),
        });
        state.events.push("Previous phone task content".into());
        state.attachments.push(document.clone());
        state.active_key = Some("previous-phone-session".into());
        state.submitted_draft = Some("Previous phone task draft".into());
        state.error = Some("Previous phone task failure".into());
        state.running = true;
        let url = "https://example.test/previous-phone.png".to_string();
        state.image_previews.insert(url.clone(), document.clone());
        state.preview_attempted.insert(url.clone());
        state.preview_pending.insert(url.clone());
        state.image_viewer = Some(url.clone());
        state.preview_errors.insert(url.clone(), "expired".into());
        state
            .preview_identities
            .insert(url.clone(), "previous-session/message/0".into());
        state.preview_cache = Some(Arc::new(tempfile::tempdir().unwrap()));
        let (preview_tx, preview_rx) = mpsc::channel();
        state.preview_rx = Some(preview_rx);
        let (event_tx, event_rx) = mpsc::channel();
        let cancel = Arc::new(std::sync::atomic::AtomicBool::new(false));
        state.handle = Some(work::WorkHandle {
            events: event_rx,
            cancel: cancel.clone(),
        });

        state.disconnect();

        // Disconnect must both erase visible results and detach producers, so
        // a late completion from the previous phone cannot repopulate them.
        assert!(cancel.load(Ordering::SeqCst));
        assert!(event_tx
            .send(work::WorkEvent::Complete("stale".into()))
            .is_err());
        assert!(preview_tx
            .send((url.clone(), Ok(document.clone())))
            .is_err());
        assert!(!state.running && state.handle.is_none() && state.preview_rx.is_none());
        assert!(
            state.preview_pending.is_empty()
                && state.image_viewer.is_none()
                && state.preview_tx.is_none()
        );
        assert!(state.local_preview.is_none());
        assert!(state.local_preview_loading.is_none());
        assert!(state.local_preview_error.is_none());
        assert!(state.remote_preview_url.is_none());
        assert!(
            state.artifacts.is_empty() && state.events.is_empty() && state.attachments.is_empty()
        );
        assert!(
            state.active_key.is_none() && state.submitted_draft.is_none() && state.error.is_none()
        );
        assert!(!state.image_previews.contains_key(&url));
        assert!(state.preview_attempted.is_empty() && state.preview_errors.is_empty());
        assert!(state.preview_identities.is_empty() && state.preview_cache.is_none());

        // The explicitly selected computer workspace and its files survive;
        // disconnect is neither a file deletion nor a workspace reset.
        assert_eq!(state.workspace.as_deref(), Some(workspace.path()));
        assert_eq!(state.files.len(), 1);
        assert_eq!(state.files[0].path, document);
        assert_eq!(
            std::fs::read_to_string(&document).unwrap(),
            "# Previous phone's private result"
        );
    }
    #[test]
    fn csv_preview_handles_quoted_delimiters_and_multiline_cells() {
        let table = delimited_preview(
            "名称,说明\r\n甲,\"带,逗号\"\r\n乙,\"第一行\n第二行\"\n丙,\"a\"\"b\"",
            ',',
        );
        assert!(table.contains("| 甲 | 带,逗号 |"));
        assert!(table.contains("| 乙 | 第一行 第二行 |"));
        assert!(table.contains("a\"b"));
    }
    #[test]
    fn local_previews_stay_in_workspace_and_do_not_execute_content() {
        let root = tempfile::tempdir().unwrap();
        let outside = tempfile::tempdir().unwrap();
        let text = root.path().join("任务.md");
        std::fs::write(&text, "# 真实文档\n\n**完成**").unwrap();
        let result = read_work_preview(root.path(), &text).unwrap();
        assert!(result.markdown && result.text.unwrap().contains("**完成**"));
        let other = outside.path().join("secret.txt");
        std::fs::write(&other, "private").unwrap();
        assert!(read_work_preview(root.path(), &other).is_err());
        let binary = root.path().join("unknown.bin");
        std::fs::write(&binary, [0, 1, 2]).unwrap();
        assert!(read_work_preview(root.path(), &binary)
            .unwrap()
            .text
            .is_none());
    }
    #[test]
    #[cfg(unix)]
    fn previews_reject_symlinks_even_when_target_is_inside_workspace() {
        let root = tempfile::tempdir().unwrap();
        let text = root.path().join("note.md");
        std::fs::write(&text, "private").unwrap();
        let linked = root.path().join("linked.md");
        std::os::unix::fs::symlink(text, &linked).unwrap();
        assert!(read_work_preview(root.path(), &linked).is_err());
    }

    #[test]
    #[cfg(unix)]
    fn workspace_aliases_allow_previews_but_child_symlinks_stay_blocked() {
        let parent = tempfile::tempdir().unwrap();
        let real = parent.path().join("实际工作目录");
        std::fs::create_dir(&real).unwrap();
        let alias = parent.path().join("工作目录别名");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        std::fs::write(real.join("说明.md"), "# 跨平台预览").unwrap();
        let preview = read_work_preview(&alias, &alias.join("说明.md")).unwrap();
        assert_eq!(preview.text.as_deref(), Some("# 跨平台预览"));
        assert!(read_work_preview(&alias, &real.join("说明.md").canonicalize().unwrap()).is_ok());
        std::os::unix::fs::symlink(real.join("说明.md"), real.join("内部链接.md")).unwrap();
        assert!(read_work_preview(&alias, &alias.join("内部链接.md")).is_err());
    }

    #[test]
    fn preview_cache_serves_unchanged_files_and_invalidates_on_edit() {
        let root = tempfile::tempdir().unwrap();
        let doc = root.path().join("cached.md");
        std::fs::write(&doc, "# 第一版内容").unwrap();
        let first = read_work_preview(root.path(), &doc).unwrap();
        assert_eq!(first.text.as_deref(), Some("# 第一版内容"));
        let second = read_work_preview(root.path(), &doc).unwrap();
        assert_eq!(second.text.as_deref(), Some("# 第一版内容"));
        std::fs::write(&doc, "# 第二版修订内容更长").unwrap();
        let updated = read_work_preview(root.path(), &doc).unwrap();
        assert_eq!(updated.text.as_deref(), Some("# 第二版修订内容更长"));
    }

    #[test]
    fn work_session_and_studio_defaults_stay_isolated_from_amoo_chat() {
        let mut ui = Companion::new_for_test();
        assert_eq!(Page::Studio.title(), "我的作品");
        assert_eq!(Page::Work.title(), "文件处理");
        assert_eq!(ui.studio.tab, 0);

        ui.amoo_sessions = super::super::default_amoo_sessions();
        ui.amoo_session_idx = 0;
        let amoo_key = ui.amoo_sessions[0].session_key.clone();
        let work_key = ui.amoo_sessions[2].session_key.clone();
        ui.work.session_key = Some(work_key.clone());

        ui.with_work_session(|me| {
            assert_eq!(me.amoo_session_idx, 2);
            me.apply(Outcome::AmooCreated("work:new".into()));
        });

        assert_eq!(ui.amoo_sessions[ui.amoo_session_idx].session_key, amoo_key);
        assert_eq!(ui.work.session_key.as_deref(), Some("work:new"));
        assert_eq!(
            ui.work_session().map(|s| s.session_key.as_str()),
            Some("work:new")
        );
    }

    #[test]
    fn deliverable_hygiene_and_friendly_work_step_names() {
        assert_eq!(
            deliverable_kind_for_path(std::path::Path::new("季度经营分析.xlsx")).as_deref(),
            Some("Excel 表格")
        );
        assert!(deliverable_kind_for_path(std::path::Path::new("~$季度经营分析.xlsx")).is_none());
        assert!(deliverable_kind_for_path(std::path::Path::new(".hidden.md")).is_none());
        assert!(deliverable_kind_for_path(std::path::Path::new(".git/HEAD.md")).is_none());
        assert!(deliverable_kind_for_path(std::path::Path::new("StepX Companion")).is_none());
        assert_eq!(
            Companion::work_step_name("write_document"),
            "撰写 Word 文档"
        );
        assert_eq!(
            Companion::work_step_name("write_spreadsheet"),
            "生成 Excel 表格"
        );
        assert_eq!(
            Companion::work_step_name("write_presentation"),
            "制作 PPT 演示稿"
        );
        assert_eq!(Companion::work_step_name("search_files"), "检索工作区文件");
    }
}
pub(super) fn open_work_path(path: &std::path::Path) -> Result<()> {
    let command = if cfg!(target_os = "macos") {
        "open"
    } else if cfg!(windows) {
        "explorer"
    } else {
        "xdg-open"
    };
    let mut child = std::process::Command::new(command).arg(path).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}

pub(super) fn reveal_in_folder(path: &std::path::Path) -> Result<()> {
    let target = if path.is_file() {
        path.parent().unwrap_or(path)
    } else {
        path
    };
    #[cfg(target_os = "macos")]
    let mut child = if path.is_file() {
        std::process::Command::new("open")
            .arg("-R")
            .arg(path)
            .spawn()?
    } else {
        std::process::Command::new("open").arg(target).spawn()?
    };
    #[cfg(windows)]
    let mut child = if path.is_file() {
        std::process::Command::new("explorer")
            .arg("/select,")
            .arg(path)
            .spawn()?
    } else {
        std::process::Command::new("explorer").arg(target).spawn()?
    };
    #[cfg(all(not(target_os = "macos"), not(windows)))]
    let mut child = std::process::Command::new("xdg-open").arg(target).spawn()?;
    std::thread::spawn(move || {
        let _ = child.wait();
    });
    Ok(())
}
