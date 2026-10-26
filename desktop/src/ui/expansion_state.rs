use super::*;
use serde_json::{json, Value};
use stepx_companion::storage_expansion::{self as expansion, Command, Config, Snapshot, Worker};
#[derive(Clone, Debug)]
pub(super) enum ExpansionAction {
    Folder,
    OpenFolder,
    Settings,
    Back,
    Discard,
    Type(String),
    Days(u64),
    Confirm,
    Next,
    Previous,
    Inspect,
    Enable,
    Pause,
    Filter(String),
    Sort(String),
    Descending,
    State(String),
    More,
    Open(String),
    Restore(String),
    AnalyzeWithAmoo(String),
    SetGalleryGrid(bool),
    SelectMedia(Option<String>),
    MediaZoomIn,
    MediaZoomOut,
    MediaZoomSet(u16),
    MediaRotateRight(String),
    MediaPrev,
    MediaNext,
    SetGridCols(u8),
    FilesSort(String),
    FilesToggleDesc,
    FilesFilter(String),
    FilesZoomSet(u16),
    FilesClosePreview,
    FilesPreview(String),
    RevealInFolder(String),
    ManualImportLocalFiles,
    Refresh,
}
pub(super) struct ExpansionUi {
    pub worker: Option<Worker>,
    pub local: mpsc::Receiver<Vec<Value>>,
    pub snapshot: Snapshot,
    pub config: Config,
    pub draft: Option<Value>,
    pub draft_folder: Option<PathBuf>,
    pub settings: bool,
    pub confirm: bool,
    pub gallery_grid_mode: bool,
    pub selected_media_id: Option<String>,
    pub media_zoom_pct: u16,
    pub media_rotations: std::collections::HashMap<String, u16>,
    pub grid_cols: u8,
    pub files_sort: String,
    pub files_desc: bool,
    pub files_filter: String,
    pub files_preview_path: Option<String>,
    pub files_preview_zoom: u16,
    pub limit: usize,
    pub feedback: String,
    pub pending: Option<mpsc::Receiver<std::result::Result<Value, String>>>,
    pub step: u8,
    pub preview: Option<Value>,
    pub preview_error: String,
    pub preview_pending: Option<mpsc::Receiver<std::result::Result<Value, String>>>,
}
pub(super) const MAX_MEDIA_ROTATIONS: usize = 256;

impl ExpansionUi {
    /// Bound `media_rotations` at `MAX_MEDIA_ROTATIONS` (256) entries so rotating
    /// many media files across long sessions never leaks memory.
    pub(super) fn insert_media_rotation(&mut self, id: String, deg: u16) {
        if !self.media_rotations.contains_key(&id) {
            while self.media_rotations.len() >= MAX_MEDIA_ROTATIONS {
                let selected = self.selected_media_id.as_deref();
                let victim = self
                    .media_rotations
                    .keys()
                    .find(|k| Some(k.as_str()) != selected)
                    .or_else(|| self.media_rotations.keys().next())
                    .cloned();
                let Some(victim) = victim else {
                    break;
                };
                self.media_rotations.remove(&victim);
            }
        }
        self.media_rotations.insert(id, deg);
    }
}

impl Default for ExpansionUi {
    fn default() -> Self {
        let config = expansion::config();
        let demo =
            std::env::args().any(|a| a == "--demo") && std::env::var_os("STEPX_EMPTY").is_none();
        let mut snapshot = Snapshot::default();
        if demo {
            snapshot.online = true;
            snapshot.status = demo_status();
            snapshot.files = demo_files();
        }
        let (tx, local) = mpsc::channel();
        let disk = config.clone();
        if !demo {
            std::thread::spawn(move || {
                let _ = tx.send(expansion::records(&disk).unwrap_or_default());
            });
        }
        Self {
            worker: None,
            local,
            snapshot,
            config: config.clone(),
            draft: None,
            draft_folder: config.folder.clone(),
            settings: false,
            confirm: false,
            gallery_grid_mode: true,
            selected_media_id: None,
            media_zoom_pct: 100,
            media_rotations: std::collections::HashMap::new(),
            grid_cols: 3,
            files_sort: "name".into(),
            files_desc: false,
            files_filter: "all".into(),
            files_preview_path: None,
            files_preview_zoom: 100,
            limit: 40,
            feedback: String::new(),
            pending: None,
            step: 0,
            preview: None,
            preview_error: String::new(),
            preview_pending: None,
        }
    }
}
impl Companion {
    fn save_expansion_view(&mut self) -> Result<()> {
        if self.expansion.config.folder.is_none() {
            self.expansion.config.folder = expansion::config().folder;
        }
        expansion::save_config(&self.expansion.config)
    }
    pub(super) fn tick_expansion(&mut self) -> bool {
        let matches = self
            .selected
            .as_ref()
            .zip(self.expansion.worker.as_ref())
            .is_some_and(|(d, w)| d.serial == w.serial && d.transport == w.transport);
        if !self.demo_mode && !matches {
            let different = self
                .selected
                .as_ref()
                .zip(self.expansion.worker.as_ref())
                .is_some_and(|(d, w)| d.serial != w.serial);
            self.expansion.worker = None;
            self.expansion.preview = None;
            self.expansion.preview_pending = None;
            if self.selected.is_some() && different {
                self.expansion.snapshot = Snapshot::default();
                self.expansion.draft = None;
            } else {
                self.expansion.snapshot.online = false;
            }
            if let Some(d) = self.selected.clone() {
                if let Ok(adb) = Adb::discover() {
                    self.expansion.worker = Some(Worker::start(adb, d));
                }
            }
        }
        let mut changed = false;
        if let Some(result) =
            self.expansion
                .preview_pending
                .as_ref()
                .and_then(|rx| match rx.try_recv() {
                    Ok(r) => Some(r),
                    Err(mpsc::TryRecvError::Disconnected) => {
                        Some(Err("手机连接已改变，请重新检查".into()))
                    }
                    Err(_) => None,
                })
        {
            self.expansion.preview_pending = None;
            match result {
                Ok(r) => {
                    self.expansion.preview = Some(r);
                    self.expansion.preview_error.clear();
                }
                Err(e) => self.expansion.preview_error = e,
            }
            changed = true;
        }
        if let Ok(files) = self.expansion.local.try_recv() {
            if !self.demo_mode {
                self.expansion.snapshot.files = files;
                changed = true;
            }
        }
        while let Some(s) = self
            .expansion
            .worker
            .as_ref()
            .and_then(|w| w.events.try_recv().ok())
        {
            if self.expansion.settings
                && self.expansion.draft.is_none()
                && s.status["policy"].is_object()
            {
                self.expansion.draft = Some(s.status["policy"].clone());
            }
            self.expansion.snapshot = s;
            if self.expansion.config.folder.is_none() {
                self.expansion.config = expansion::config();
            }
            if self.expansion.feedback == "正在处理…" && self.expansion.snapshot.active.is_none()
            {
                self.expansion.feedback.clear();
            }
            changed = true;
        }
        if let Some(r) = self
            .expansion
            .pending
            .as_ref()
            .and_then(|rx| match rx.try_recv() {
                Ok(r) => Some(r),
                Err(mpsc::TryRecvError::Disconnected) => {
                    Some(Err("手机连接已改变，请重新连接并核对设置".into()))
                }
                Err(_) => None,
            })
        {
            self.expansion.pending = None;
            self.expansion.feedback = match r {
                Ok(s) => {
                    let on = s["policy"]["enabled"] == true;
                    self.expansion.snapshot.status = s;
                    if let Some(folder) = self.expansion.draft_folder.clone() {
                        self.expansion.config.folder = Some(folder);
                    } else if self.expansion.config.folder.is_none() {
                        self.expansion.config = expansion::config();
                    }
                    self.expansion.discard();
                    self.expansion.settings = false;
                    if on {
                        "已保存并经手机回读确认，连接期间将按此规则迁移。".into()
                    } else {
                        "已确认暂停，电脑原件与手机入口保留。".into()
                    }
                }
                Err(e) => format!("保存未完成：{e}；更改已保留，请检查后重试。"),
            };
            self.record_notice("存储扩容", self.expansion.feedback.clone());
            if let Some(w) = &self.expansion.worker {
                let _ = w.commands.send(Command::Refresh);
            }
            changed = true;
        }
        changed
    }
    fn inspect_expansion_draft(&mut self) -> Result<()> {
        let (tx, rx) = mpsc::channel();
        let policy = self
            .expansion
            .draft
            .clone()
            .ok_or_else(|| anyhow::anyhow!("请先读取手机规则"))?;
        self.expansion
            .worker
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("请先连接手机"))?
            .commands
            .send(Command::Inspect { policy, reply: tx })?;
        self.expansion.preview = None;
        self.expansion.preview_pending = None;
        self.expansion.preview_error.clear();
        self.expansion.preview_error.clear();
        self.expansion.preview_pending = Some(rx);
        Ok(())
    }
    fn expansion_command(&mut self, c: Command) {
        if let Some(w) = &self.expansion.worker {
            let _ = w.commands.send(c);
            self.expansion.feedback = "正在处理…".into();
        } else {
            self.expansion.feedback = "请先连接手机".into();
        }
    }
    pub(super) fn handle_expansion_action(
        &mut self,
        action: &ExpansionAction,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.expansion.pending.is_some() {
            self.expansion.feedback = "正在核对手机设置，请等待完成后再操作".into();
            ctx.notify();
            return;
        }
        let result = (|| -> Result<()> {
            match action {
                ExpansionAction::Settings => {
                    if !self.expansion.settings || self.expansion.draft.is_none() {
                        self.expansion.draft_folder = self.expansion.config.folder.clone();
                        self.expansion.draft = self
                            .expansion
                            .snapshot
                            .status
                            .get("policy")
                            .cloned()
                            .filter(|v| v.is_object());
                    }
                    self.expansion.settings = true;
                    self.expansion.step = 0;
                    self.expansion.preview = None;
                    self.expansion.preview_pending = None;
                    self.expansion.preview_error.clear();
                    self.expansion.feedback.clear();
                    self.page = Page::Expansion;
                    self.scroll = Default::default();
                }
                ExpansionAction::Next => {
                    if self.expansion.step == 0 {
                        let q = self
                            .expansion
                            .draft
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("请等待读取手机规则"))?;
                        anyhow::ensure!(
                            ["photos", "videos", "documents"]
                                .iter()
                                .any(|k| q[*k] == true),
                            "至少选择一种文件类型，再点击下一步"
                        );
                        self.expansion.step = 1;
                    } else {
                        self.expansion.validate_draft()?;
                        self.expansion.step = 2;
                        self.inspect_expansion_draft()?;
                    }
                    self.expansion.feedback.clear();
                    self.scroll = Default::default();
                }
                ExpansionAction::Previous => {
                    self.expansion.step = self.expansion.step.saturating_sub(1);
                    self.scroll = Default::default();
                }
                ExpansionAction::Inspect => self.inspect_expansion_draft()?,
                ExpansionAction::Discard => {
                    self.expansion.discard();
                    self.expansion.feedback = "已放弃更改，手机原规则未改变".into();
                    self.expansion.step = 0;
                }
                ExpansionAction::Back => {
                    self.expansion.settings = false;
                    self.expansion.confirm = false;
                }
                ExpansionAction::Folder => {
                    if let Some(p) = native_dialog::FileDialog::new()
                        .set_title("选择扩容文件保存位置")
                        .show_open_single_dir()?
                    {
                        self.expansion.draft_folder =
                            Some(stepx_companion::settings::choose_directory(&p)?);
                        self.expansion.preview = None;
                        self.expansion.preview_pending = None;
                        self.expansion.preview_error.clear();
                        self.expansion.feedback = "保存位置尚未应用，请确认规则后保存".into();
                    }
                }
                ExpansionAction::OpenFolder => {
                    let base = if self.expansion.settings {
                        if self.expansion.draft_folder.is_none() {
                            self.expansion.draft_folder =
                                Some(stepx_companion::settings::downloads());
                        }
                        self.expansion.draft_folder.clone()
                    } else {
                        if self.expansion.config.folder.is_none() {
                            let dl = stepx_companion::settings::downloads();
                            self.expansion.config.folder = Some(dl);
                            let _ = expansion::save_config(&self.expansion.config);
                        }
                        self.expansion.config.folder.clone()
                    };
                    if let Some(p) = base {
                        let vault = p.join("StepX Storage");
                        let _ = std::fs::create_dir_all(&vault);
                        super::work_state::open_work_path(&vault)?;
                    }
                }
                ExpansionAction::ManualImportLocalFiles => {
                    if self.expansion.config.folder.is_none() {
                        let dl = stepx_companion::settings::downloads();
                        self.expansion.config.folder = Some(dl);
                        let _ = expansion::save_config(&self.expansion.config);
                    }
                    let base = self
                        .expansion
                        .config
                        .folder
                        .clone()
                        .ok_or_else(|| anyhow::anyhow!("未设置电脑扩容目录"))?;
                    let picked = native_dialog::FileDialog::new()
                        .set_title("选择要归档到电脑扩容库的文件")
                        .show_open_multiple_file()?;
                    if !picked.is_empty() {
                        let vault = base.join("StepX Storage");
                        let mut imported = 0usize;
                        let now_ms = chrono::Utc::now().timestamp_millis();
                        let today_label =
                            format!("{} · 今天", chrono::Local::now().format("%Y-%m-%d"));
                        for src in picked {
                            if !src.is_file() {
                                continue;
                            }
                            let Some(name) = src
                                .file_name()
                                .and_then(|s| s.to_str())
                                .map(|s| s.to_string())
                            else {
                                continue;
                            };
                            let ext = src
                                .extension()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_ascii_lowercase();
                            let (kind, sub_folder, rel_prefix) = if matches!(
                                ext.as_str(),
                                "jpg" | "jpeg" | "png" | "webp" | "gif" | "heic" | "bmp" | "dng"
                            ) {
                                ("photos", "照片与视频", "Pictures")
                            } else if matches!(
                                ext.as_str(),
                                "mp4" | "mov" | "mkv" | "avi" | "webm" | "3gp" | "flv"
                            ) {
                                ("videos", "照片与视频", "Movies")
                            } else if matches!(
                                ext.as_str(),
                                "mp3" | "wav" | "flac" | "m4a" | "aac" | "ogg" | "opus" | "amr"
                            ) {
                                ("audio", "录音与音频", "Music")
                            } else {
                                ("documents", "社交与文档", "Documents")
                            };
                            let dest_dir = vault.join(sub_folder);
                            let _ = std::fs::create_dir_all(&dest_dir);
                            let dest_file = dest_dir.join(&name);
                            if src != dest_file {
                                let _ = std::fs::copy(&src, &dest_file);
                            }
                            let size = std::fs::metadata(&dest_file)
                                .or_else(|_| std::fs::metadata(&src))
                                .map(|m| m.len())
                                .unwrap_or(0);
                            let id = format!("local-{}-{imported}", now_ms as u64);
                            let entry = json!({
                                "id": id,
                                "name": name,
                                "path": format!("{rel_prefix}/{name}"),
                                "kind": kind,
                                "state": "archived",
                                "size": size,
                                "modified": now_ms,
                                "mtimeMs": now_ms,
                                "archivedAt": now_ms,
                                "archivedAtMs": now_ms,
                                "dateGroup": today_label,
                                "localPath": dest_file.to_string_lossy().to_string(),
                            });
                            self.expansion.snapshot.files.insert(0, entry);
                            imported += 1;
                        }
                        if imported > 0 {
                            self.expansion.feedback =
                                format!("已手动归档 {imported} 个文件到电脑扩容库");
                            self.record_notice("存储扩容", self.expansion.feedback.clone());
                        }
                    }
                }
                ExpansionAction::Type(k) => {
                    anyhow::ensure!(
                        self.expansion.snapshot.status["policy"].is_object(),
                        "请等待手机规则读取完成后再选择"
                    );
                    let fallback = self.expansion.snapshot.status["policy"].clone();
                    let q = self.expansion.draft.get_or_insert(fallback);
                    q[k] = json!(!q[k].as_bool().unwrap_or(false));
                    self.expansion.confirm = false;
                    self.expansion.preview = None;
                    self.expansion.preview_pending = None;
                    self.expansion.preview_error.clear();
                }
                ExpansionAction::Days(d) => {
                    anyhow::ensure!(
                        self.expansion.snapshot.status["policy"].is_object(),
                        "请等待手机规则读取完成后再选择"
                    );
                    let fallback = self.expansion.snapshot.status["policy"].clone();
                    let q = self.expansion.draft.get_or_insert(fallback);
                    q["retentionDays"] = json!(d);
                    self.expansion.confirm = false;
                    self.expansion.preview = None;
                    self.expansion.preview_pending = None;
                    self.expansion.preview_error.clear();
                }
                ExpansionAction::Confirm => self.expansion.confirm = true,
                ExpansionAction::Pause | ExpansionAction::Enable => {
                    anyhow::ensure!(self.expansion.pending.is_none(), "设置正在保存");
                    anyhow::ensure!(self.selected.is_some(), "请先连接手机");
                    let mut q = if matches!(action, ExpansionAction::Pause) {
                        self.expansion.snapshot.status["policy"].clone()
                    } else {
                        self.expansion.draft.clone().unwrap_or_else(default_policy)
                    };
                    q["enabled"] = json!(matches!(action, ExpansionAction::Enable));
                    q["expectedEpoch"] = self.expansion.snapshot.status["policy"]["epoch"].clone();
                    let (tx, rx) = mpsc::channel();
                    self.expansion
                        .worker
                        .as_ref()
                        .ok_or_else(|| anyhow::anyhow!("手机连接尚未就绪"))?
                        .commands
                        .send(Command::ApplyPolicy {
                            folder: self.expansion.draft_folder.clone(),
                            policy: q,
                            reply: tx,
                        })?;
                    self.expansion.pending = Some(rx);
                    self.expansion.feedback = "正在保存并核对手机设置，请保持连接…".into();
                }
                ExpansionAction::Filter(k) => {
                    self.expansion.config.kind = k.clone();
                    self.expansion.limit = 40;
                    self.trigger_subtab_switch();
                    self.save_expansion_view()?;
                }
                ExpansionAction::Sort(s) => {
                    self.expansion.config.sort = s.clone();
                    self.save_expansion_view()?;
                }
                ExpansionAction::Descending => {
                    self.expansion.config.descending = !self.expansion.config.descending;
                    self.save_expansion_view()?;
                }
                ExpansionAction::State(s) => {
                    self.expansion.config.filter = s.clone();
                    self.expansion.limit = 40;
                    self.trigger_subtab_switch();
                    self.save_expansion_view()?;
                }
                ExpansionAction::More => self.expansion.limit += 40,
                ExpansionAction::SetGalleryGrid(grid) => {
                    self.expansion.gallery_grid_mode = *grid;
                    self.trigger_subtab_switch();
                }
                ExpansionAction::SelectMedia(id) => {
                    self.expansion.selected_media_id = id.clone();
                    if id.is_some() {
                        self.expansion.media_zoom_pct = 100;
                    }
                }
                ExpansionAction::MediaZoomIn => {
                    let steps = [50u16, 100, 150, 200, 300, 400];
                    let next = steps
                        .iter()
                        .copied()
                        .find(|&s| s > self.expansion.media_zoom_pct)
                        .unwrap_or(400);
                    self.expansion.media_zoom_pct = next;
                }
                ExpansionAction::MediaZoomOut => {
                    let steps = [50u16, 100, 150, 200, 300, 400];
                    let prev = steps
                        .iter()
                        .rev()
                        .copied()
                        .find(|&s| s < self.expansion.media_zoom_pct)
                        .unwrap_or(50);
                    self.expansion.media_zoom_pct = prev;
                }
                ExpansionAction::MediaZoomSet(pct) => {
                    self.expansion.media_zoom_pct = (*pct).clamp(50, 400);
                }
                ExpansionAction::MediaRotateRight(id) => {
                    let current = self.expansion.media_rotations.get(id).copied().unwrap_or(0);
                    let next_deg = (current + 90) % 360;
                    self.expansion.insert_media_rotation(id.clone(), next_deg);
                    self.ensure_rotated_media_preview(id, next_deg);
                    self.expansion.feedback = format!("已将图片顺时针旋转至 {next_deg}°");
                }
                ExpansionAction::MediaPrev => {
                    let filtered = self.expansion_filtered_files();
                    if !filtered.is_empty() {
                        let cur = self.expansion.selected_media_id.as_deref().unwrap_or("");
                        let pos = filtered
                            .iter()
                            .position(|j| j["id"].as_str() == Some(cur))
                            .unwrap_or(0);
                        let prev_idx = (pos + filtered.len() - 1) % filtered.len();
                        if let Some(id) = filtered[prev_idx]["id"].as_str() {
                            self.expansion.selected_media_id = Some(id.to_string());
                        }
                    }
                }
                ExpansionAction::MediaNext => {
                    let filtered = self.expansion_filtered_files();
                    if !filtered.is_empty() {
                        let cur = self.expansion.selected_media_id.as_deref().unwrap_or("");
                        let pos = filtered
                            .iter()
                            .position(|j| j["id"].as_str() == Some(cur))
                            .unwrap_or(0);
                        let next_idx = (pos + 1) % filtered.len();
                        if let Some(id) = filtered[next_idx]["id"].as_str() {
                            self.expansion.selected_media_id = Some(id.to_string());
                        }
                    }
                }
                ExpansionAction::SetGridCols(cols) => {
                    self.expansion.grid_cols = (*cols).clamp(2, 4);
                    self.expansion.gallery_grid_mode = true;
                }
                ExpansionAction::FilesSort(s) => {
                    self.expansion.files_sort = s.clone();
                    self.file_sort = match s.as_str() {
                        "time" => FileSort::Time,
                        "size" => FileSort::Size,
                        "kind" => FileSort::Kind,
                        _ => FileSort::Name,
                    };
                    self.file_page = 0;
                }
                ExpansionAction::FilesToggleDesc => {
                    self.expansion.files_desc = !self.expansion.files_desc;
                    self.file_page = 0;
                }
                ExpansionAction::FilesFilter(f) => {
                    self.expansion.files_filter = f.clone();
                    self.file_page = 0;
                }
                ExpansionAction::FilesZoomSet(z) => {
                    self.expansion.files_preview_zoom = (*z).clamp(50, 400);
                }
                ExpansionAction::FilesClosePreview => {
                    self.expansion.files_preview_path = None;
                }
                ExpansionAction::FilesPreview(path) => {
                    self.expansion.files_preview_path = Some(path.clone());
                    self.expansion.files_preview_zoom = 100;
                }
                ExpansionAction::Open(id) => {
                    let j = self
                        .expansion
                        .snapshot
                        .files
                        .iter()
                        .find(|j| j["id"] == *id)
                        .cloned()
                        .or_else(|| demo_files().into_iter().find(|j| j["id"] == *id))
                        .ok_or_else(|| anyhow::anyhow!("文件记录已变化"))?;
                    if let Some(p) = resolve_media_local_path(&self.expansion.config, &j) {
                        super::work_state::open_work_path(&p)?;
                    } else if self.demo_mode || j["demo"] == true {
                        self.expansion.feedback = format!(
                            "已定位电脑归档原件：{}",
                            j["name"].as_str().unwrap_or("原件")
                        );
                    } else {
                        super::work_state::open_work_path(&expansion::local_file(
                            &self.expansion.config,
                            &j,
                        )?)?;
                    }
                }
                ExpansionAction::RevealInFolder(id) => {
                    let j = self
                        .expansion
                        .snapshot
                        .files
                        .iter()
                        .find(|j| j["id"] == *id)
                        .cloned()
                        .or_else(|| demo_files().into_iter().find(|j| j["id"] == *id))
                        .ok_or_else(|| anyhow::anyhow!("文件记录已变化"))?;
                    if let Some(p) = resolve_media_local_path(&self.expansion.config, &j) {
                        super::work_state::reveal_in_folder(&p)?;
                    } else if let Some(folder) = &self.expansion.config.folder {
                        super::work_state::reveal_in_folder(folder)?;
                    } else {
                        let target = expansion::local_file(&self.expansion.config, &j)?;
                        super::work_state::reveal_in_folder(&target)?;
                    }
                }
                ExpansionAction::AnalyzeWithAmoo(id) => {
                    if let Some(j) = self
                        .expansion
                        .snapshot
                        .files
                        .iter()
                        .find(|j| j["id"] == *id)
                        .cloned()
                        .or_else(|| demo_files().into_iter().find(|j| j["id"] == *id))
                    {
                        let name = j["name"].as_str().unwrap_or("归档文件").to_string();
                        let size_bytes = j["size"].as_u64().unwrap_or(0);
                        let kind = j["kind"].as_str().unwrap_or("");
                        let local_path = resolve_media_local_path(&self.expansion.config, &j);
                        if let Some(path) = &local_path {
                            let is_img = kind == "photos"
                                || path.extension().and_then(|e| e.to_str()).is_some_and(|e| {
                                    matches!(
                                        e.to_ascii_lowercase().as_str(),
                                        "png" | "jpg" | "jpeg" | "webp" | "gif"
                                    )
                                });
                            let staged_path = if path
                                .file_name()
                                .and_then(|s| s.to_str())
                                .is_some_and(|n| n == name)
                            {
                                path.clone()
                            } else {
                                let cache_dir = std::env::temp_dir().join("stepx-gallery-cache");
                                let _ = std::fs::create_dir_all(&cache_dir);
                                let dest = cache_dir.join(&name);
                                if std::fs::copy(path, &dest).is_ok() {
                                    dest
                                } else {
                                    path.clone()
                                }
                            };
                            if is_img {
                                self.work.insert_image_preview(
                                    staged_path.to_string_lossy().into_owned(),
                                    staged_path.clone(),
                                );
                            }
                            if !self.amoo_attachments.iter().any(|a| {
                                a.local_path.as_ref() == Some(&staged_path)
                                    || a.local_path.as_ref() == Some(path)
                            }) {
                                self.amoo_attachments.push(AmooAttachment {
                                    name: name.clone(),
                                    size_label: storage::format_bytes(size_bytes),
                                    is_image: is_img,
                                    local_path: Some(staged_path),
                                });
                            }
                        }
                        if self.fields[F_AMOO].text.trim().is_empty() {
                            self.fields[F_AMOO].set(&format!("请帮我分析这份扩容归档文件：{name}"));
                        }
                        self.status = format!("已载入扩容原件「{name}」至 Amoo 对话框");
                    } else {
                        let name = PathBuf::from(id)
                            .file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or(id.as_str())
                            .to_string();
                        self.fields[F_AMOO]
                            .set(&format!("请帮我分析手机中的文件「{name}」（路径：{id}）"));
                        self.status = format!("已将手机文件「{name}」带入 Amoo 对话框");
                    }
                    self.workspace.sub_view = 0;
                    self.workspace.panel = 0;
                    self.page = Page::Amoo;
                    self.scroll = Default::default();
                    self.reveal_latest_chat();
                    self.focus_field(F_AMOO, ctx);
                }
                ExpansionAction::Restore(id) => {
                    self.expansion_command(Command::Restore(id.clone()))
                }
                ExpansionAction::Refresh => self.expansion_command(Command::Refresh),
            }
            Ok(())
        })();
        if let Err(e) = result {
            self.expansion.feedback = format!("{e:#}");
        }
        ctx.notify();
    }

    pub(super) fn expansion_filtered_files(&self) -> Vec<Value> {
        let e = &self.expansion;
        let fallback_demo = if e.snapshot.files.is_empty()
            && self.demo_mode
            && std::env::var_os("STEPX_EMPTY").is_none()
        {
            demo_files()
        } else {
            Vec::new()
        };
        let all_files = if !e.snapshot.files.is_empty() {
            &e.snapshot.files
        } else {
            &fallback_demo
        };
        let query = self.fields[F_GALLERY_SEARCH].text.trim().to_lowercase();
        let mut files: Vec<Value> = all_files
            .iter()
            .filter(|j| {
                let item_kind = expansion_item_kind(j);
                let kind_ok = e.config.kind == "all"
                    || j["kind"] == e.config.kind
                    || item_kind == e.config.kind.as_str();
                let state_ok = e.config.filter == "all" || j["state"] == e.config.filter;
                let query_ok = query.is_empty()
                    || j["name"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
                    || j["path"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query)
                    || j["dateGroup"]
                        .as_str()
                        .unwrap_or("")
                        .to_lowercase()
                        .contains(&query);
                kind_ok && state_ok && query_ok
            })
            .cloned()
            .collect();
        files.sort_by(|a, b| {
            let order = match e.config.sort.as_str() {
                "name" => a["name"]
                    .as_str()
                    .unwrap_or("")
                    .to_lowercase()
                    .cmp(&b["name"].as_str().unwrap_or("").to_lowercase()),
                "size" => a["size"].as_u64().cmp(&b["size"].as_u64()),
                _ => a["modified"].as_u64().cmp(&b["modified"].as_u64()),
            };
            if e.config.descending {
                order.reverse()
            } else {
                order
            }
        });
        files
    }

    fn ensure_rotated_media_preview(&self, id: &str, deg: u16) {
        if deg % 360 == 0 {
            return;
        }
        let Some(j) = self
            .expansion
            .snapshot
            .files
            .iter()
            .find(|j| j["id"].as_str() == Some(id))
            .cloned()
            .or_else(|| {
                demo_files()
                    .into_iter()
                    .find(|j| j["id"].as_str() == Some(id))
            })
        else {
            return;
        };
        let img_opt = resolve_media_local_path(&self.expansion.config, &j)
            .and_then(|p| image::open(p).ok())
            .or_else(|| {
                let asset = match j["previewAsset"].as_str() {
                    Some("neo_front_frame.png") => "neo_front_frame.png",
                    Some("neo_rear_housing.png") => "neo_rear_housing.png",
                    _ => "stepx_home_screen.png",
                };
                use facloud_ui::assets::AssetProvider;
                DesktopAssets
                    .get(asset)
                    .ok()
                    .and_then(|bytes| image::load_from_memory(&bytes).ok())
            });
        let Some(img) = img_opt else {
            return;
        };
        let rotated = match deg % 360 {
            90 => img.rotate90(),
            180 => img.rotate180(),
            270 => img.rotate270(),
            _ => img,
        };
        let cache_dir = std::env::temp_dir().join("stepx-expansion-rot");
        let _ = std::fs::create_dir_all(&cache_dir);
        let safe_id: String = id
            .chars()
            .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
            .collect();
        let target = cache_dir.join(format!("{safe_id}_{deg}.png"));
        let _ = rotated.save(&target);
        widgets::invalidate_cached_file(&target);
    }
}

pub(super) fn expansion_item_kind(j: &Value) -> &'static str {
    match j["kind"].as_str().unwrap_or("") {
        "photos" | "photo" | "image" => "photos",
        "videos" | "video" => "videos",
        "audio" | "audiobooks" | "music" => "audio",
        "documents" | "document" | "docs" => "documents",
        _ => {
            let lower = j["name"].as_str().unwrap_or("").to_ascii_lowercase();
            if lower.ends_with(".jpg")
                || lower.ends_with(".jpeg")
                || lower.ends_with(".png")
                || lower.ends_with(".webp")
                || lower.ends_with(".heic")
                || lower.ends_with(".gif")
            {
                "photos"
            } else if lower.ends_with(".mp4")
                || lower.ends_with(".mov")
                || lower.ends_with(".mkv")
                || lower.ends_with(".webm")
            {
                "videos"
            } else if lower.ends_with(".mp3")
                || lower.ends_with(".wav")
                || lower.ends_with(".flac")
                || lower.ends_with(".m4a")
                || lower.ends_with(".aac")
                || lower.ends_with(".ogg")
                || lower.ends_with(".opus")
                || lower.ends_with(".amr")
            {
                "audio"
            } else {
                "documents"
            }
        }
    }
}

pub(super) fn resolve_media_local_path(config: &Config, j: &Value) -> Option<PathBuf> {
    if let Ok(p) = expansion::local_file(config, j) {
        if widgets::cached_is_file(&p) {
            return Some(p);
        }
    }
    if let Some(raw) = j["localPath"].as_str().filter(|s| !s.is_empty()) {
        let direct = PathBuf::from(raw);
        if widgets::cached_is_file(&direct) {
            return Some(direct);
        }
        if let Some(stripped) = raw.strip_prefix("desktop/") {
            let alt = PathBuf::from(stripped);
            if widgets::cached_is_file(&alt) {
                return Some(alt);
            }
        }
    }
    None
}

pub(super) fn rotated_media_cache_path(id: &str, deg: u16) -> Option<PathBuf> {
    if deg % 360 == 0 {
        return None;
    }
    let safe_id: String = id
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_' || *c == '-')
        .collect();
    let target = std::env::temp_dir()
        .join("stepx-expansion-rot")
        .join(format!("{safe_id}_{deg}.png"));
    widgets::cached_is_file(&target).then_some(target)
}

fn default_policy() -> Value {
    json!({"enabled":false,"photos":true,"videos":false,"documents":false,"retentionDays":7})
}
impl ExpansionUi {
    pub fn discard(&mut self) {
        self.draft = self
            .snapshot
            .status
            .get("policy")
            .cloned()
            .filter(|v| v.is_object());
        self.draft_folder = self.config.folder.clone();
        self.confirm = false;
        self.preview = None;
        self.preview_pending = None;
        self.preview_error.clear();
    }
    pub fn dirty(&self) -> bool {
        self.settings
            && (self.draft_folder != self.config.folder
                || self.draft.as_ref().is_some_and(|q| {
                    ["photos", "videos", "documents", "retentionDays"]
                        .iter()
                        .any(|k| q[*k] != self.snapshot.status["policy"][*k])
                }))
    }
    pub fn validate_draft(&self) -> Result<()> {
        anyhow::ensure!(self.pending.is_none(), "正在保存设置");
        anyhow::ensure!(self.snapshot.online, "请先连接手机并刷新设置");
        anyhow::ensure!(
            self.snapshot.status["permission"] == true,
            "手机未授予文件管理权限，请在 StepX伴侣的存储扩容中开启"
        );
        let q = self
            .draft
            .as_ref()
            .unwrap_or(&self.snapshot.status["policy"]);
        anyhow::ensure!(
            ["photos", "videos", "documents"]
                .iter()
                .any(|k| q[*k] == true),
            "请至少选择一种文件类型"
        );
        anyhow::ensure!(self.draft_folder.is_some(), "请选择电脑保存位置");
        if self.draft_folder != self.config.folder {
            anyhow::ensure!(
                self.snapshot.status["policy"]["enabled"] != true,
                "请先暂停自动迁移，再更换保存位置"
            );
            anyhow::ensure!(
                self.snapshot.files.iter().all(|j| j["demo"] == true),
                "已有原件保存在旧目录，请保留此目录"
            );
        }
        Ok(())
    }
    pub fn policy_summary(q: &Value) -> String {
        let types = [
            ("photos", "拍摄照片"),
            ("videos", "拍摄视频"),
            ("documents", "文档"),
        ]
        .iter()
        .filter(|(k, _)| q[*k] == true)
        .map(|(_, n)| *n)
        .collect::<Vec<_>>()
        .join("、");
        let days = q["retentionDays"].as_u64().unwrap_or(7);
        format!(
            "{} · {}",
            if types.is_empty() {
                "未选择文件"
            } else {
                &types
            },
            if days == 0 {
                "迁移全部符合条件的文件".into()
            } else {
                format!("手机保留最近 {days} 天原件")
            }
        )
    }
    pub fn review_lines(&self) -> Vec<String> {
        let q = self
            .draft
            .as_ref()
            .unwrap_or(&self.snapshot.status["policy"]);
        let mut lines=vec![format!("迁移范围：{}",Self::policy_summary(q)),format!("原件保存到这台电脑：{} / StepX Storage",self.draft_folder.as_ref().map(|p|p.display().to_string()).unwrap_or_default()),"确认后开启；以后连接这台电脑时也会自动迁移符合规则的文件。".into(),"逐个文件复制并校验成功后，才清理手机原件。手机保留访问入口与图片预览；电脑目录是原件所在位置。".into(),"离线时只能查看已缓存内容。相册预览需分享至“取回电脑原图”，不会点击即自动取回。".into()];
        if let Some(r) = &self.preview {
            lines.insert(
                1,
                format!(
                    "本次检查：可迁移 {} 个，共 {}；保留期内 {} 个。执行前还会逐个复核。",
                    r["candidateCount"],
                    storage::format_bytes(r["candidateBytes"].as_u64().unwrap_or(0)),
                    r["retainedCount"]
                ),
            );
        }
        lines
    }
}

pub(super) fn demo_files() -> Vec<Value> {
    vec![
        json!({
            "id": "10000000000000000000000000000001",
            "name": "IMG_20261025_141802_StepX.jpg",
            "path": "DCIM/Camera/IMG_20261025_141802_StepX.jpg",
            "kind": "photos",
            "state": "archived",
            "size": 6_842_112u64,
            "modified": 1792909082000i64,
            "mtimeMs": 1792909082000i64,
            "archivedAt": 1792909500000i64,
            "archivedAtMs": 1792909500000i64,
            "dateGroup": "2026-10-25 · 今天",
            "previewAsset": "stepx_home_screen.png",
            "localPath": "desktop/assets/stepx_home_screen.png",
            "demo": true
        }),
        json!({
            "id": "10000000000000000000000000000002",
            "name": "IMG_20261025_110544_HDR.png",
            "path": "DCIM/Camera/IMG_20261025_110544_HDR.png",
            "kind": "photos",
            "state": "archived",
            "size": 8_388_608u64,
            "modified": 1792897544000i64,
            "mtimeMs": 1792897544000i64,
            "archivedAt": 1792898100000i64,
            "archivedAtMs": 1792898100000i64,
            "dateGroup": "2026-10-25 · 今天",
            "previewAsset": "neo_front_frame.png",
            "localPath": "desktop/assets/neo_front_frame.png",
            "demo": true
        }),
        json!({
            "id": "10000000000000000000000000000003",
            "name": "VID_20261025_093012_4K60.mp4",
            "path": "DCIM/Camera/VID_20261025_093012_4K60.mp4",
            "kind": "videos",
            "state": "archived",
            "size": 1_482_686_464u64,
            "modified": 1792891812000i64,
            "mtimeMs": 1792891812000i64,
            "archivedAt": 1792892400000i64,
            "archivedAtMs": 1792892400000i64,
            "dateGroup": "2026-10-25 · 今天",
            "demo": true
        }),
        json!({
            "id": "10000000000000000000000000000004",
            "name": "IMG_20261024_194210_Matrix.png",
            "path": "DCIM/Camera/IMG_20261024_194210_Matrix.png",
            "kind": "photos",
            "state": "restored",
            "size": 5_242_880u64,
            "modified": 1792842130000i64,
            "mtimeMs": 1792842130000i64,
            "archivedAt": 1792843000000i64,
            "archivedAtMs": 1792843000000i64,
            "dateGroup": "2026-10-24 · 昨天",
            "previewAsset": "neo_rear_housing.png",
            "localPath": "desktop/assets/neo_rear_housing.png",
            "demo": true
        }),
        json!({
            "id": "10000000000000000000000000000005",
            "name": "StepX_Q4_架构评审纪要.pdf",
            "path": "Documents/StepX_Q4_架构评审纪要.pdf",
            "kind": "documents",
            "state": "archived",
            "size": 18_454_937u64,
            "modified": 1792826400000i64,
            "mtimeMs": 1792826400000i64,
            "archivedAt": 1792827000000i64,
            "archivedAtMs": 1792827000000i64,
            "dateGroup": "2026-10-24 · 昨天",
            "demo": true
        }),
        json!({
            "id": "10000000000000000000000000000006",
            "name": "VID_20261024_151208_SlowMo.mp4",
            "path": "DCIM/Camera/VID_20261024_151208_SlowMo.mp4",
            "kind": "videos",
            "state": "archived",
            "size": 671_088_640u64,
            "modified": 1792825928000i64,
            "mtimeMs": 1792825928000i64,
            "archivedAt": 1792826500000i64,
            "archivedAtMs": 1792826500000i64,
            "dateGroup": "2026-10-24 · 昨天",
            "demo": true
        }),
    ]
}

pub(super) fn demo_status() -> Value {
    let recent = demo_files();
    let released: u64 = recent
        .iter()
        .filter(|j| j["state"] == "archived")
        .map(|j| j["size"].as_u64().unwrap_or(0))
        .sum();
    let count = recent.iter().filter(|j| j["state"] == "archived").count() as u64;
    json!({
        "ok": true,
        "permission": true,
        "releasedBytes": released,
        "count": count,
        "recentFiles": recent,
        "policy": {
            "enabled": true,
            "photos": true,
            "videos": true,
            "documents": true,
            "retentionDays": 7,
            "desktopId": "demo-desktop",
            "epoch": 1
        }
    })
}
