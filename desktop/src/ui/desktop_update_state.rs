//! One startup check, explicit downloads, and a fixed-footer entry in both shells.
use stepx_companion::file_dialog::DialogKind;
use super::{theme::*, widgets::*, *};
use facloud_ui::TypedActionView;
use stepx_companion::{community_qr, desktop_update as updater, storage::format_bytes};

#[derive(Clone, Debug)]
pub(super) enum DesktopUpdateAction {
    Open,
    Close,
    OpenWhatsNew,
    CloseWhatsNew,
    Explore(stepx_companion::release_notes::Destination),
    OpenHistory,
    MoreHistory,
    BackToStory,
    PlayVideo,
    SeekVideo(i32),
    SeekVideoTo(u32),
    ExpandVideo,
    Category(String),
    ShowAuthor,
    ChoosePersonalQr,
    Check,
    Download,
    Cancel,
    Install,
    Resolve(Box<Action>),
}

#[derive(Clone, Debug, Default, serde::Serialize, serde::Deserialize)]
#[serde(default)]
pub(super) struct WhatsNewPersistedState {
    pub last_seen_version: String,
    pub personal_qr_custom_path: Option<String>,
}

impl WhatsNewPersistedState {
    pub fn should_pop_once_for_version(&self, current_version: &str) -> bool {
        self.last_seen_version.trim() != current_version.trim()
    }

    pub fn mark_version_seen(&mut self, current_version: &str) -> bool {
        if self.should_pop_once_for_version(current_version) {
            self.last_seen_version = current_version.trim().to_string();
            true
        } else {
            false
        }
    }
}

/// A restart prerequisite is not a download failure. Keep it actionable and
/// recompute it from live state rather than latching a generic red error.
struct RestartBlocker {
    title: &'static str,
    detail: String,
    action: Action,
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) enum Phase {
    #[default]
    Idle,
    Checking,
    Current,
    Ahead,
    Available,
    Downloading,
    Verifying,
    Extracting,
    Ready,
    Preparing,
    Cancelled,
    Failed,
}
pub(super) struct DesktopUpdateState {
    pub phase: Phase,
    pub open: bool,
    pub whats_new_open: bool,
    pub whats_new_scroll: ClippedScrollStateHandle,
    pub release_history: bool,
    pub release_history_limit: usize,
    pub video: Option<stepx_companion::release_video::Player>,
    pub video_revision: u64,
    pub video_frame: u32,
    pub video_playing: bool,
    pub video_ready: bool,
    pub video_expanded: bool,
    pub video_error: Option<String>,
    pub release_category: String,
    pub release_home: bool,
    pub personal_qr_custom_path: Option<PathBuf>,
    pub group_qr: Option<community_qr::CachedQr>,
    pub qr_syncing: bool,
    pub qr_last_check: Option<Instant>,
    pub qr_next_check: Instant,
    pub qr_last_error: Option<String>,
    pub qr_status_note: String,
    pub release: Option<updater::Release>,
    pub error: String,
    pub received: u64,
    pub total: u64,
    pub speed: u64,
    pub checked: String,
    pub notice: String,
    pub ready: Option<PathBuf>,
    pub job: Option<updater::Job>,
    pub started: Instant,
    pub fraction_from: f32,
    pub fraction: f32,
    pub fraction_at: Instant,
    pub blockers_scroll: ClippedScrollStateHandle,
}
impl Default for DesktopUpdateState {
    fn default() -> Self {
        Self {
            phase: Phase::Idle,
            open: false,
            whats_new_open: false,
            whats_new_scroll: Default::default(),
            release_history: false,
            release_history_limit: 8,
            video: None,
            video_revision: 0,
            video_frame: 0,
            video_playing: false,
            video_ready: false,
            video_expanded: false,
            video_error: None,
            release_category: String::new(),
            release_home: true,
            personal_qr_custom_path: None,
            group_qr: None,
            qr_syncing: false,
            qr_last_check: None,
            qr_next_check: Instant::now(),
            qr_last_error: None,
            qr_status_note: String::new(),
            release: None,
            error: String::new(),
            received: 0,
            total: 0,
            speed: 0,
            checked: String::new(),
            notice: String::new(),
            ready: None,
            job: None,
            started: Instant::now(),
            fraction_from: 0.,
            fraction: 0.,
            fraction_at: Instant::now(),
            blockers_scroll: Default::default(),
        }
    }
}
impl DesktopUpdateState {
    pub fn label(&self) -> String {
        match self.phase {
            Phase::Available => format!(
                "检测到新版本 v{}",
                self.release.as_ref().map_or("", |r| r.version.as_str())
            ),
            Phase::Downloading => format!("下载更新 {}%", (self.fraction * 100.) as u8),
            Phase::Verifying => "正在校验更新".into(),
            Phase::Extracting => "正在准备更新".into(),
            Phase::Ready => "更新已准备好".into(),
            Phase::Preparing => "正在准备重启".into(),
            Phase::Checking => "正在检查更新".into(),
            Phase::Current => "已是最新版本".into(),
            Phase::Ahead => "当前版本领先于公开版".into(),
            Phase::Cancelled => "下载已取消".into(),
            Phase::Failed => "更新未完成".into(),
            Phase::Idle => "检查更新".into(),
        }
    }
    pub fn shown(&self) -> bool {
        matches!(
            self.phase,
            Phase::Available
                | Phase::Downloading
                | Phase::Verifying
                | Phase::Extracting
                | Phase::Ready
                | Phase::Preparing
                | Phase::Cancelled
                | Phase::Failed
        )
    }
}
impl Companion {
    fn desktop_restart_blockers(&self) -> Vec<RestartBlocker> {
        let mut rows = Vec::new();
        let mut add = |condition, title, detail: String, action| {
            if condition {
                rows.push(RestartBlocker {
                    title,
                    detail,
                    action,
                });
            }
        };
        // Unsaved settings first: navigation must not discard them just to
        // inspect a task or draft on another page.
        add(
            self.settings_dirty(),
            "设置尚未保存",
            "请确认保存或放弃本次设置更改。".into(),
            Action::Page(Page::Settings),
        );
        add(
            self.expansion.dirty(),
            "迁移规则尚未保存",
            "请在存储扩容中确认保存或放弃更改。".into(),
            Action::Page(Page::Expansion),
        );
        add(
            self.studio_config_dirty(),
            "模型设置尚未保存",
            "请确认保存模型地址、模型名称或密钥。".into(),
            Action::Settings(SettingsAction::Models),
        );
        add(
            self.busy,
            "当前操作尚未结束",
            self.status.clone(),
            Action::Page(self.job_page),
        );
        add(
            self.work.running
                || self.workspace.running.is_some()
                || stepx_companion::desktop_work::is_running(),
            "Amoo 任务正在执行",
            "等待任务完成，或在任务中停止后再更新。".into(),
            Action::Page(Page::Amoo),
        );
        add(
            !self.amoo_pending_reply.is_empty(),
            "Amoo 正在等待回复",
            "请在对话中查看进度；切换页面不会结束任务。".into(),
            Action::Page(Page::Amoo),
        );
        add(
            self.studio_lab.handle.is_some(),
            "应用创作正在执行",
            "请等待创作完成，或先停止任务。".into(),
            Action::Page(Page::Studio),
        );
        add(
            self.updates.busy || self.updates.transfer.is_some(),
            "手机更新任务尚未结束",
            "请等待下载、安装或取消操作完成。".into(),
            Action::OpenTool(Some(ToolPage::Updates)),
        );
        add(
            self.expansion.snapshot.active.is_some() || self.expansion.pending.is_some(),
            "文件迁移或规则保存正在进行",
            "请等待当前文件校验与保存完成。".into(),
            Action::Page(Page::Expansion),
        );
        add(
            self.back_screen.companion_busy
                || self.back_screen.generating
                || self.back_screen.applying_native,
            "背屏任务正在执行",
            "请等待当前背屏生成或应用完成。".into(),
            Action::Page(Page::BackScreen),
        );
        add(
            self.amoo_ui.gui_running,
            "手机看屏任务正在执行",
            "请先在 Amoo 中完成或停止任务。".into(),
            Action::Page(Page::Amoo),
        );
        add(
            self.push.sending_thread.is_some(),
            "消息正在发送",
            "请等待手机返回发送结果，避免重复发送。".into(),
            Action::Page(Page::Sms),
        );
        add(
            self.app_store.tasks.values().any(|t| t.stage.is_active()),
            "应用下载或安装正在进行",
            "请等待应用商店中的任务完成。".into(),
            Action::Page(Page::AppStore),
        );
        add(
            self.bulk_busy,
            "批量应用操作正在进行",
            "请等待当前操作完成。".into(),
            Action::Page(Page::Apps),
        );
        add(
            self.settings.staging.is_some() || self.workspace.delivery.is_some(),
            "文件正在发送到手机",
            "请等待手机接收结果。".into(),
            Action::Page(Page::Amoo),
        );
        for (field, title, page) in [
            (F_AMOO, "Amoo 对话有未发送草稿", Page::Amoo),
            (F_WORK_PROMPT, "文件处理有未发送草稿", Page::Work),
            (F_SMS_BODY, "短信有未发送草稿", Page::Sms),
            (F_PUSH_REPLY, "通知回复有未发送草稿", Page::Notifications),
            (F_IM_REPLY, "消息回复有未发送草稿", Page::Sms),
            (F_TOAST_REPLY, "通知弹窗有未发送草稿", Page::Notifications),
            (F_LAB_PROMPT, "应用创作有未提交描述", Page::Studio),
        ] {
            let text = self.fields[field].text.trim();
            // Studio intentionally retains the submitted prompt for revisions.
            // It is not a new draft once its artifact records that exact input.
            let submitted = field == F_LAB_PROMPT
                && !text.is_empty()
                && self
                    .studio_lab
                    .artifact_prompts
                    .values()
                    .any(|p| p.trim() == text);
            add(
                !text.is_empty() && !submitted,
                title,
                format!(
                    "{} 字未发送。请返回编辑处理；更新不会自动发送或清空。",
                    text.chars().count()
                ),
                Action::Page(page),
            );
        }
        add(
            !self.amoo_attachments.is_empty(),
            "Amoo 对话还有待发送附件",
            format!("{} 个附件仍在输入框中。", self.amoo_attachments.len()),
            Action::Page(Page::Amoo),
        );
        add(
            !self.work.attachments.is_empty(),
            "文件处理还有待发送附件",
            format!("{} 个附件仍在输入框中。", self.work.attachments.len()),
            Action::Page(Page::Work),
        );
        rows
    }

    pub(super) fn whats_new_state_path() -> PathBuf {
        stepx_companion::resources::user_config().join("stepx-companion/whats_new_state.json")
    }

    pub(super) fn qr_custom_dir() -> PathBuf {
        stepx_companion::resources::user_config().join("stepx-companion")
    }

    pub(super) fn load_whats_new_state(&mut self) -> WhatsNewPersistedState {
        let path = Self::whats_new_state_path();
        let persisted: WhatsNewPersistedState = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let default_personal = Self::qr_custom_dir().join("qr_personal.png");
        let personal_candidate = persisted
            .personal_qr_custom_path
            .as_deref()
            .map(PathBuf::from)
            .filter(|p| p.is_file())
            .or_else(|| default_personal.is_file().then_some(default_personal));
        self.desktop_update.personal_qr_custom_path = personal_candidate;

        // Legacy download timestamps cannot prove a QR's expiry. Leave the old
        // files intact, but only use a validated manifest-backed cache.
        self.desktop_update.group_qr = community_qr::load_cached(&community_qr::cache_dir());
        persisted
    }

    pub(super) fn save_whats_new_state(&self, last_seen_version: Option<&str>) {
        let path = Self::whats_new_state_path();
        let existing: WhatsNewPersistedState = std::fs::read_to_string(&path)
            .ok()
            .and_then(|raw| serde_json::from_str(&raw).ok())
            .unwrap_or_default();
        let state = WhatsNewPersistedState {
            last_seen_version: last_seen_version
                .map(str::to_string)
                .unwrap_or(existing.last_seen_version),
            personal_qr_custom_path: self
                .desktop_update
                .personal_qr_custom_path
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned()),
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(&state) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub(super) fn record_desktop_event(
        &mut self,
        _kind: &str,
        title: impl Into<String>,
        detail: impl Into<String>,
    ) {
        self.record_notice(title, detail);
    }

    pub(super) fn initialize_desktop_updates(&mut self, ctx: &mut ViewContext<Self>) {
        let mut persisted = self.load_whats_new_state();
        if std::env::var_os("STEPX_WHATS_NEW_PREVIEW").is_some() {
            self.desktop_update.whats_new_open = true;
            self.desktop_update.started = Instant::now();
        }
        if self.demo_mode || self.capture.is_some() {
            #[cfg(feature = "ui-capture")]
            if self.demo_mode && std::env::var_os("STEPX_UPDATE_PREVIEW").is_some() {
                self.desktop_update.open = true;
                self.desktop_update.phase = Phase::Ready;
                self.busy = false;
                self.work.running = false;
                self.workspace.running = None;
                self.amoo_pending_reply.clear();
                self.discard_settings();
                self.amoo_attachments.clear();
                self.work.attachments.clear();
                for f in [
                    F_AMOO,
                    F_WORK_PROMPT,
                    F_SMS_BODY,
                    F_PUSH_REPLY,
                    F_IM_REPLY,
                    F_TOAST_REPLY,
                    F_LAB_PROMPT,
                ] {
                    self.fields[f].clear();
                }
                if std::env::var("STEPX_UPDATE_PREVIEW").as_deref() == Ok("blocked") {
                    self.settings.draft.reduced_motion = !self.settings.value.reduced_motion;
                    self.fields[F_AMOO].set("测试草稿：周末计划，保留但不要发送。");
                }
            }
            return;
        }
        if persisted.mark_version_seen(stepx_companion::build_info::VERSION)
            || std::env::var_os("STEPX_FORCE_WHATS_NEW").is_some()
        {
            self.desktop_update.whats_new_open = true;
            self.desktop_update.started = Instant::now();
            self.save_whats_new_state(Some(stepx_companion::build_info::VERSION));
            self.record_desktop_event(
                "version_whats_new",
                format!(
                    "StepX Companion 已更新至 v{}",
                    stepx_companion::build_info::VERSION
                ),
                "已升级至新版本，点击可随时查看本次更新内容、作者个人微信与交流群二维码。",
            );
        }
        self.auto_sync_group_qr_silently(ctx);
        self.desktop_update.notice = updater::last_result().unwrap_or_default();
        if let Some((dir, release)) = updater::cached_ready() {
            self.desktop_update.ready = Some(dir);
            self.desktop_update.release = Some(release);
        }
        self.start_desktop_update_check();
        ctx.spawn(
            async {
                facloud_ui::r#async::Timer::after(Duration::from_secs(3)).await;
            },
            |me, _, ctx| {
                let _ = updater::acknowledge_startup();
                if let Some(note) = updater::last_result() {
                    me.desktop_update.notice = note;
                    ctx.notify();
                }
            },
        );
    }
    fn auto_sync_group_qr_silently(&mut self, ctx: &mut ViewContext<Self>) {
        let now = Instant::now();
        if self.desktop_update.qr_syncing
            || self
                .desktop_update
                .qr_last_check
                .is_some_and(|last| now.duration_since(last) < Duration::from_secs(300))
        {
            return;
        }
        self.desktop_update.qr_syncing = true;
        self.desktop_update.qr_last_check = Some(now);
        let dir = community_qr::cache_dir();
        let (tx, rx) = async_channel::bounded(1);
        std::thread::spawn(move || {
            let result = community_qr::sync(&dir, chrono::Local::now().timestamp_millis())
                .map_err(|error| error.to_string());
            let _ = tx.send_blocking(result);
        });
        ctx.spawn(async move { rx.recv().await }, |me, res, ctx| {
            me.desktop_update.qr_syncing = false;
            let success = matches!(&res, Ok(Ok(_)));
            me.desktop_update.qr_next_check =
                Instant::now() + Duration::from_secs(if success { 6 * 3600 } else { 15 * 60 });
            match res {
                Ok(Ok(qr)) => {
                    super::widgets::invalidate_cached_file(&qr.path);
                    me.desktop_update.group_qr = Some(qr);
                    me.desktop_update.qr_last_error = None;
                }
                Ok(Err(error)) => me.desktop_update.qr_last_error = Some(error),
                Err(_) => me.desktop_update.qr_last_error = Some("群码更新任务中断".into()),
            }
            ctx.notify();
        });
    }
    fn start_desktop_update_check(&mut self) {
        if self.desktop_update.job.is_some() {
            return;
        }
        self.desktop_update.error.clear();
        self.desktop_update.phase = Phase::Checking;
        match updater::check() {
            Ok(job) => self.desktop_update.job = Some(job),
            Err(e) => self.desktop_update_failure(e.to_string()),
        }
    }
    fn desktop_update_failure(&mut self, message: String) {
        self.desktop_update.phase = Phase::Failed;
        self.desktop_update.error = message;
    }
    fn reset_release_scroll(&mut self) {
        self.desktop_update.whats_new_scroll = Default::default();
    }
    pub(super) fn handle_desktop_update(
        &mut self,
        action: &DesktopUpdateAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use DesktopUpdateAction as A;
        match action {
            A::Open => {
                self.desktop_update.whats_new_open = false;
                self.desktop_update.open = true;
                self.desktop_update.started = Instant::now();
                if self.desktop_update.phase == Phase::Idle {
                    self.start_desktop_update_check();
                }
            }
            A::Close => self.desktop_update.open = false,
            A::OpenWhatsNew => {
                self.desktop_update.whats_new_open = true;
                self.desktop_update.open = false;
                self.desktop_update.whats_new_scroll = Default::default();
                self.desktop_update.release_history = false;
                self.desktop_update.release_category.clear();
                self.desktop_update.release_home = true;
                self.desktop_update.video_expanded = false;
                self.desktop_update.started = Instant::now();
                self.auto_sync_group_qr_silently(ctx);
            }
            A::CloseWhatsNew => {
                self.desktop_update.whats_new_open = false;
                self.desktop_update.video = None;
                self.desktop_update.video_playing = false;
                self.save_whats_new_state(Some(stepx_companion::build_info::VERSION));
            }
            A::Explore(destination) => {
                use stepx_companion::release_notes::Destination as D;
                if matches!(destination, D::History) {
                    self.handle_desktop_update(&A::OpenHistory, ctx);
                    return;
                }
                self.desktop_update.whats_new_open = false;
                self.desktop_update.video = None;
                self.desktop_update.video_playing = false;
                self.save_whats_new_state(Some(stepx_companion::build_info::VERSION));
                match destination {
                    D::Search => self.handle_action(&Action::GlobalSearch(super::global_search::SearchAction::Open), ctx),
                    D::Agent => self.handle_action(&Action::Settings(SettingsAction::Tab(super::settings_state::AGENT_SETTINGS_TAB)), ctx),
                    D::About => {
                        self.handle_action(&Action::Settings(SettingsAction::Tab(4)), ctx);
                    }
                    D::Settings | D::Notifications | D::Storage => {
                        let tab = match destination { D::Notifications => 3, D::Storage => 1, _ => 0 };
                        self.handle_action(&Action::Settings(SettingsAction::Tab(tab)), ctx);
                    }
                    D::Files => {
                        self.recent_files_view = true;
                        self.handle_action(&Action::Page(Page::Files), ctx);
                    }
                    _ => self.handle_action(&Action::Page(match destination {
                        D::Home => Page::Home, D::Messages => Page::Sms, D::Phone => Page::Phone,
                        D::Amoo => Page::Amoo, D::Studio => Page::Studio, D::Calendar => Page::Calendar,
                        D::Alarms => Page::Alarms, D::Gallery => Page::Gallery, D::BackScreen => Page::BackScreen,
                        D::Tools => Page::Tools, D::Connect => Page::Connect, D::Account => Page::Account, D::AppStore => Page::AppStore,
                        _ => unreachable!(),
                    }), ctx),
                }
            }
            A::OpenHistory => {
                self.desktop_update.open = false;
                self.desktop_update.whats_new_open = true;
                self.desktop_update.release_history = true;
                self.desktop_update.release_home = false;
                self.desktop_update.release_history_limit = 8;
                self.reset_release_scroll();
            }
            A::MoreHistory => {
                self.desktop_update.release_history_limit =
                    (self.desktop_update.release_history_limit + 8)
                        .min(stepx_companion::release_notes::history().len());
            }
            A::BackToStory => {
                self.desktop_update.release_history = false;
                self.desktop_update.release_home = false;
                self.desktop_update.release_category.clear();
                self.desktop_update.video_expanded = false;
                self.reset_release_scroll();
            }
            A::Category(id) => {
                if !id.is_empty()
                    && !stepx_companion::release_notes::current()
                        .is_ok_and(|n| n.categories.iter().any(|c| c.id == *id))
                {
                    return;
                }
                self.desktop_update.release_category = id.clone();
                self.desktop_update.release_history = false;
                self.desktop_update.release_home = false;
                self.reset_release_scroll();
                self.desktop_update.video_expanded = false;
                if let Some(video) = &self.desktop_update.video {
                    video.control(stepx_companion::release_video::Control::Pause);
                }
            }
            A::ShowAuthor => {
                self.desktop_update.release_home = true;
                self.desktop_update.release_history = false;
                self.desktop_update.video_expanded = false;
                self.reset_release_scroll();
                self.desktop_update.video = None;
                self.desktop_update.video_playing = false;
            }
            A::ExpandVideo => {
                self.desktop_update.video_expanded = !self.desktop_update.video_expanded;
                self.reset_release_scroll();
                if self.desktop_update.video_expanded {
                    if !self.desktop_update.video_playing {
                        self.handle_desktop_update(&A::PlayVideo, ctx);
                    }
                } else if let Some(video) = &self.desktop_update.video {
                    video.control(stepx_companion::release_video::Control::Pause);
                }
            }
            A::PlayVideo => {
                use stepx_companion::release_video::{Player, Control};
                if self.desktop_update.video.is_none() || self.desktop_update.video_error.is_some() {
                    self.desktop_update.video = Some(Player::start()); self.desktop_update.video_revision = 0;
                    self.desktop_update.video_error = None;
                }
                if let Some(video) = &self.desktop_update.video { video.control(if self.desktop_update.video_playing { Control::Pause } else { Control::Play }); }
            }
            A::SeekVideoTo(second) => {
                use stepx_companion::release_video::{Player, Control, FPS};
                if self.desktop_update.video.is_none() { self.desktop_update.video = Some(Player::start()); }
                if let Some(video) = &self.desktop_update.video {
                    video.control(Control::Seek(second.saturating_mul(FPS))); video.control(Control::Play);
                }
            }
            A::SeekVideo(seconds) => {
                use stepx_companion::release_video::{Player, Control, FPS};
                if self.desktop_update.video.is_none() { self.desktop_update.video = Some(Player::start()); }
                let frame = (i64::from(self.desktop_update.video_frame) + i64::from(*seconds) * i64::from(FPS)).max(0) as u32;
                if let Some(video) = &self.desktop_update.video { video.control(Control::Seek(frame)); }
            }
            A::ChoosePersonalQr => {
                self.choose_path(
                    stepx_companion::file_dialog::FileDialog::new()
                        .add_filter("二维码图片", &["png", "jpg", "jpeg", "webp"]),
                    DialogKind::OpenFile,
                    |app, src, _| {
                        let dir = Self::qr_custom_dir();
                        std::fs::create_dir_all(&dir)?;
                        let dst = dir.join("qr_personal.png");
                        std::fs::copy(&src, &dst)?;
                        super::widgets::invalidate_cached_file(&dst);
                        app.desktop_update.personal_qr_custom_path = Some(dst);
                        app.desktop_update.qr_status_note = "已更新个人微信二维码".into();
                        app.save_whats_new_state(None);
                        Ok(())
                    },
                );
            }
            A::Check => {
                self.desktop_update.whats_new_open = false;
                self.desktop_update.open = true;
                self.desktop_update.started = Instant::now();
                self.start_desktop_update_check();
            }
            A::Download if self.desktop_update.job.is_none() => {
                if let Some(release) = &self.desktop_update.release {
                    match updater::download(release) {
                        Ok(job) => {
                            self.desktop_update.job = Some(job);
                            self.desktop_update.phase = Phase::Downloading;
                            self.desktop_update.error.clear();
                            self.desktop_update.ready = None;
                            self.desktop_update.fraction = 0.;
                            self.desktop_update.fraction_from = 0.;
                            self.desktop_update.received = 0;
                            self.desktop_update.total = release.size;
                        }
                        Err(e) => self.desktop_update_failure(e.to_string()),
                    }
                }
            }
            A::Cancel => {
                if let Some(job) = &self.desktop_update.job {
                    job.cancel();
                    self.desktop_update.error = "正在取消下载…".into();
                }
            }
            A::Install if self.desktop_update.phase == Phase::Ready => {
                let blockers = self.desktop_restart_blockers();
                if !blockers.is_empty() {
                    // Show the live checklist below; do not turn a prerequisite
                    // into a sticky download error or discard any user input.
                    self.desktop_update.error.clear();
                } else if let Some(dir) = self.desktop_update.ready.clone() {
                    self.desktop_update.error.clear();
                    match updater::prepare(&dir) {
                        Ok(job) => {
                            self.desktop_update.job = Some(job);
                            self.desktop_update.phase = Phase::Preparing;
                        }
                        Err(e) => self.desktop_update_failure(e.to_string()),
                    }
                }
            }
            A::Resolve(action) => {
                self.desktop_update.open = false;
                // Return locally to the editor without discarding staged settings.
                self.set_compact_mode(ShellMode::Full, ctx);
                match action.as_ref() {
                    Action::Page(page) => {
                        self.page = *page;
                        self.focus = None;
                    }
                    Action::OpenTool(tool) => {
                        self.page = Page::Tools;
                        self.tool_page = *tool;
                    }
                    Action::Settings(SettingsAction::Models) => {
                        self.page = Page::Amoo;
                        self.workspace.panel = 5;
                    }
                    _ => self.handle_action(action, ctx),
                }
                self.back_screen
                    .set_preview_visible(self.page == Page::BackScreen);
            }
            _ => {}
        }
        ctx.notify();
    }
    pub(super) fn poll_desktop_update(&mut self, ctx: &mut ViewContext<Self>) {
        if !self.desktop_update.whats_new_open
            || self.desktop_update.release_history
            || self.desktop_update.release_home
        {
            self.desktop_update.video = None;
            self.desktop_update.video_playing = false;
        }
        let snapshot = self
            .desktop_update
            .video
            .as_ref()
            .and_then(|p| p.poll(self.desktop_update.video_revision));
        if let Some(snapshot) = snapshot {
            self.desktop_update.video_revision = snapshot.revision;
            self.desktop_update.video_frame = snapshot.frame;
            self.desktop_update.video_playing = snapshot.playing;
            self.desktop_update.video_error = snapshot.error;
            if let Some(bytes) = snapshot.jpeg {
                use facloud_ui::{
                    assets::asset_cache::AssetCache, image_cache::ImageType, SingletonEntity,
                };
                AssetCache::handle(ctx).update(ctx, |cache, cache_ctx| {
                    cache.insert_raw_asset_bytes::<ImageType>(
                        stepx_companion::release_video::FRAME_ASSET.into(),
                        &bytes,
                        cache_ctx,
                    );
                });
                self.desktop_update.video_ready = true;
            }
            ctx.notify();
        }
        if !self.demo_mode
            && self.capture.is_none()
            && Instant::now() >= self.desktop_update.qr_next_check
        {
            self.auto_sync_group_qr_silently(ctx);
        }
        while let Some(event) = self.desktop_update.job.as_ref().and_then(|j| j.poll()) {
            let terminal = !matches!(event, updater::Event::Progress { .. });
            match event {
                updater::Event::Release { release } => {
                    let cached_matches = self.desktop_update.ready.is_some()
                        && self.desktop_update.release.as_ref().is_some_and(|r| {
                            r.version == release.version
                                && r.sha256 == release.sha256
                                && r.url == release.url
                        });
                    self.desktop_update.checked = chrono::Local::now().format("%H:%M").to_string();
                    match stepx_companion::updates::compare_versions(
                        &release.version,
                        stepx_companion::build_info::VERSION,
                    ) {
                        Ok(std::cmp::Ordering::Greater) => {
                            self.desktop_update.phase = Phase::Available
                        }
                        Ok(std::cmp::Ordering::Equal) => self.desktop_update.phase = Phase::Current,
                        Ok(_) => self.desktop_update.phase = Phase::Ahead,
                        Err(e) => self.desktop_update_failure(e.to_string()),
                    }
                    if cached_matches && self.desktop_update.phase == Phase::Available {
                        self.desktop_update.phase = Phase::Ready;
                    } else {
                        self.desktop_update.ready = None;
                    }
                    self.desktop_update.release = Some(release);
                }
                updater::Event::Progress {
                    phase,
                    received,
                    total,
                    speed,
                } => {
                    let u = &mut self.desktop_update;
                    u.phase = match phase.as_str() {
                        "verify" => Phase::Verifying,
                        "extract" => Phase::Extracting,
                        _ => Phase::Downloading,
                    };
                    u.received = received;
                    u.total = total;
                    u.speed = speed;
                    u.fraction_from = u.fraction;
                    u.fraction = (received as f32 / total.max(1) as f32).clamp(0., 1.);
                    u.fraction_at = Instant::now();
                }
                updater::Event::Ready { job } => {
                    let _ = updater::remember_ready(&job);
                    self.desktop_update.ready = Some(job);
                    self.desktop_update.phase = Phase::Ready;
                    self.desktop_update.error.clear();
                }
                updater::Event::Prepared { job: _ } => {
                    self.remember_window(ctx, true);
                    self.cancel_backscreen();
                    self.stop_mirror();
                    self.close_mirror_controls(ctx);
                    ctx.close_window();
                }
                updater::Event::Cancelled => {
                    self.desktop_update.phase = Phase::Cancelled;
                    self.desktop_update.error.clear();
                }
                updater::Event::Error { message } => self.desktop_update_failure(message),
            }
            if terminal {
                self.desktop_update.job = None;
            }
            ctx.notify();
        }
    }
    pub(super) fn desktop_update_link(&self) -> El {
        if !self.desktop_update.shown() {
            return Empty::new().finish();
        }
        let p = self.p();
        let label = self.desktop_update.label();
        let icon = if self.desktop_update.phase == Phase::Ready {
            icons::UiIcon::Check
        } else {
            icons::UiIcon::Download
        };
        Hoverable::new(self.mouse_state("desktop-update-footer"), move |mouse| {
            Container::new(self.hstack(
                S1,
                vec![
                        self.ui_icon(icon, 13., p.text2),
                        Text::new(label.clone(), self.font, MICRO)
                            .with_color(p.text)
                            .with_line_height_ratio(1.)
                            .soft_wrap(false)
                            .with_saved_char_position(0, "desktop-update-open".into())
                            .finish(),
                    ],
            ))
            .with_horizontal_padding(S2)
            .with_vertical_padding(2.)
            .with_background_color(if mouse.is_mouse_over_element() {
                p.hover
            } else {
                p.bg
            })
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
            .finish()
        })
        .with_cursor(platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(Action::DesktopUpdate(DesktopUpdateAction::Open))
        })
        .finish()
    }
    pub(super) fn desktop_update_card(&self) -> El {
        let p = self.p();
        let mut parts = vec![
            self.between(
                self.heading("软件更新"),
                self.hstack(
                    S2,
                    vec![
                        self.button(
                            "desktop-update-whats-new",
                            "查看本次更新与交流群",
                            Action::DesktopUpdate(DesktopUpdateAction::OpenWhatsNew),
                            Btn::Secondary,
                        ),
                        self.button(
                            "desktop-update-settings",
                            "检查更新",
                            Action::DesktopUpdate(DesktopUpdateAction::Check),
                            Btn::Secondary,
                        ),
                    ],
                ),
            ),
            self.text(self.desktop_update.label(), BODY, p.text2),
            self.text(
                "启动时自动检查 GitHub 正式版本；下载与安装由你确认。",
                CAPTION,
                p.text3,
            ),
        ];
        if !self.desktop_update.notice.is_empty() {
            parts.push(self.text(&self.desktop_update.notice, CAPTION, p.text3));
        }
        self.panel(parts)
    }

    pub(super) fn personal_job_banner(&self, id_prefix: &str) -> El {
        let p = self.p();
        // The project exists to help its author find work. This is the primary
        // message, not muted release metadata. Keep the original paragraphs.
        let headline = self.text_w("我正在找工作，欢迎联系或内推", 26., p.text, facloud_ui::fonts::Weight::Semibold);
        #[cfg(feature = "ui-capture")]
        let headline = super::scenario_check::control(format!("{id_prefix}-job-headline"), headline);
        let banner = Container::new(self.vstack(
            S3,
            vec![
                self.hstack(S2, vec![
                    self.ui_icon(super::icons::UiIcon::Briefcase, 20., p.blue),
                    Expanded::new(1., self.text_w("董正法 · 产品负责人 / 产品专家（上海及周边 · 华东地区均可）", 16., p.text, facloud_ui::fonts::Weight::Semibold)).finish(),
                ]),
                headline,
                self.text_w("求职方向：产品岗 / AI 产品岗 / 产品与 AI 应用开发复合岗", SUBHEAD, p.text, facloud_ui::fonts::Weight::Semibold),
                self.text("能从用户与业务调研出发，完成产品规划、以用户体验为核心的界面与交互设计，借助 AI 完成前后端业务开发、测试、部署上线，并持续做运营与迭代。", BODY, p.text2),
                self.wrap(vec![
                    self.button_icon(
                        &format!("{id_prefix}-copy-wx"), "联系我 · 复制微信", super::icons::UiIcon::Chat,
                        Action::CopyText("164245026".into(), "作者微信号 164245026 已复制".into()), Btn::Primary,
                    ),
                    self.button_icon(
                        &format!("{id_prefix}-copy-email"), "复制邮箱", super::icons::UiIcon::Sms,
                        Action::CopyText("f@ab.cool".into(), "作者邮箱 f@ab.cool 已复制".into()), Btn::Secondary,
                    ),
                    self.text("微信 164245026 · 邮箱 f@ab.cool", BODY, p.text2),
                ]),
                self.text(
                    "说句实在话：家里小孩还不满一岁，眼下非常需要一份工作，在家里也能少一点唠叨。上海附近或整个华东地区都可以，近一点、远一点都行，能干事、能扛事。如果刚好有合适的机会或能帮忙内推，欢迎随时加我微信聊聊，真心感谢！",
                    SUBHEAD, p.text,
                ),
                self.divider(),
                self.text_w("我做过什么，也能为团队做什么", SUBHEAD, p.text, facloud_ui::fonts::Weight::Semibold),
                self.text(
                    "10年+ 产品与管理经验，既独立带过产品团队，也统筹管过完整的产研团队；从 0 到 1 做成过 300 万用户的 App、智能硬件互联系统和企业 AI 平台，既能拆解业务带团队拿结果，也能自己动手把想法直接做成产品（比如你正在用的这款软件和企业的AI平台 SkillForge）。",
                    SUBHEAD, p.text2,
                ),
                self.text_w("软硬件产品全流程交付", SUBHEAD, p.text, facloud_ui::fonts::Weight::Semibold),
                self.text("能独立完成软硬件产品从需求调研、产品规划、开发实现、联调测试到上线交付的全流程，兼顾用户体验、运行稳定性与交付成本，并根据真实使用反馈持续迭代。", BODY, p.text2),
                self.text_w("AI 应用开发与评估", SUBHEAD, p.text, facloud_ui::fonts::Weight::Semibold),
                self.text("从业务场景判断是否适合用 AI，设计和开发 Agent 工作流、接入工具与业务数据，验证实际效果与稳定性，跟踪调用用量与成本。", BODY, p.text2),
                self.wrap(vec![
                    self.button(&format!("{id_prefix}-open-sf"), "企业的AI平台 SkillForge",
                        Action::StudioOpenUrl("https://github.com/xydzf520/skillforge".into()), Btn::Secondary),
                    self.button(&format!("{id_prefix}-open-gh"), "GitHub",
                        Action::StudioOpenUrl("https://github.com/xydzf520/stepx".into()), Btn::Secondary),
                ]),
            ],
        ))
        .with_uniform_padding(S4)
        .with_background_color(lerp_color(p.bg, p.blue, if p.dark { 0.10 } else { 0.035 }))
        .with_border(Border::all(1.).with_border_color(lerp_color(p.border, p.blue, 0.22)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();
        #[cfg(feature = "ui-capture")]
        let banner = super::scenario_check::control(format!("{id_prefix}-job-banner"), banner);
        banner
    }

    pub(super) fn personal_contact_cards(&self, _id_prefix: &str) -> El {
        let p = self.p();
        let personal_source = match &self.desktop_update.personal_qr_custom_path {
            Some(path) if super::widgets::cached_is_file(path) => {
                facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                    path: path.to_string_lossy().into_owned(),
                }
            }
            _ => facloud_ui::assets::asset_cache::AssetSource::Bundled {
                path: "qr_personal.png",
            },
        };
        let personal_qr_frame = Align::new(
            Container::new(
                ConstrainedBox::new(
                    Image::new(personal_source, CacheOption::BySize)
                        .contain()
                        .finish(),
                )
                .with_width(142.)
                .with_height(206.)
                .finish(),
            )
            .with_uniform_padding(6.)
            .with_background_color(rgb(255, 255, 255))
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .finish(),
        )
        .finish();

        let personal_card = Container::new(self.vstack(
            S2,
            vec![
                self.line_w(
                    "个人微信 · 董正法（倒影颠倒）",
                    SUBHEAD,
                    p.text,
                    facloud_ui::fonts::Weight::Semibold,
                ),
                self.text("微信：164245026 · 邮箱：f@ab.cool", MICRO, p.text2),
                personal_qr_frame,
                self.text(
                    "扫码直接添加作者微信 · 欢迎工作机会联系、内推交流与拉群",
                    MICRO,
                    p.text3,
                ),
            ],
        ))
        .with_uniform_padding(S3)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CARD)))
        .finish();

        #[cfg(feature = "ui-capture")]
        let personal_card = super::scenario_check::control(format!("{_id_prefix}-personal-card"), personal_card);

        let now_ms = chrono::Local::now().timestamp_millis();
        let (group_source, expires_at_ms) = match &self.desktop_update.group_qr {
            Some(qr) if super::widgets::cached_is_file(&qr.path) => (
                facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                    path: qr.path.to_string_lossy().into_owned(),
                },
                qr.expires_at_ms,
            ),
            _ => (
                facloud_ui::assets::asset_cache::AssetSource::Bundled {
                    path: "qr_group.png",
                },
                community_qr::bundled_manifest()
                    .expires_at_ms()
                    .unwrap_or(0),
            ),
        };
        let group_valid = expires_at_ms > now_ms;
        let group_qr_frame = Align::new(
            Container::new(
                ConstrainedBox::new(
                    Image::new(group_source, CacheOption::BySize)
                        .contain()
                        .finish(),
                )
                .with_width(142.)
                .with_height(206.)
                .finish(),
            )
            .with_uniform_padding(6.)
            .with_background_color(rgb(255, 255, 255))
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
            .finish(),
        )
        .finish();

        let group_body = if group_valid {
            group_qr_frame
        } else {
            ConstrainedBox::new(
                Align::new(self.vstack(
                    S2,
                    vec![
                        self.ui_icon(super::icons::UiIcon::Users, 36.0, p.text3),
                        self.text("请扫左侧微信联系入群", CAPTION, p.text2),
                    ],
                ))
                .finish(),
            )
            .with_height(218.)
            .finish()
        };
        let group_card = Container::new(self.vstack(
            S2,
            vec![
                self.line_w(
                    "交流群 · StepX伴侣",
                    SUBHEAD,
                    p.text,
                    facloud_ui::fonts::Weight::Semibold,
                ),
                self.text("产品与技术交流，分享使用体验与建议", MICRO, p.text2),
                group_body,
                self.text(
                    if group_valid {
                        "扫码加入交流"
                    } else {
                        "群码暂不可用"
                    },
                    MICRO,
                    p.text3,
                ),
            ],
        ))
        .with_uniform_padding(S3)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CARD)))
        .finish();

        #[cfg(feature = "ui-capture")]
        let group_card = super::scenario_check::control(format!("{_id_prefix}-group-card"), group_card);

        let cards_row = self.responsive_columns(560., S3, S3, CrossAxisAlignment::Stretch, vec![(1., personal_card), (1., group_card)]);

        cards_row
    }

    pub(super) fn personal_and_group_qr_cards(&self, id_prefix: &str) -> El {
        self.vstack(S3, vec![self.personal_job_banner(id_prefix), self.personal_contact_cards(id_prefix)])
    }

    pub(super) fn whats_new_overlay(&self) -> Option<El> {
        self.release_story_overlay()
    }

    pub(super) fn desktop_update_overlay(&self) -> Option<El> {
        if self.desktop_update.whats_new_open && !self.desktop_update.open {
            return self.whats_new_overlay();
        }
        if !self.desktop_update.open {
            return None;
        }
        let u = &self.desktop_update;
        let p = self.p();
        let a = |v| Action::DesktopUpdate(v);
        let blockers = if u.phase == Phase::Ready {
            self.desktop_restart_blockers()
        } else {
            Vec::new()
        };
        let mut parts = vec![
            self.between(
                self.heading("更新 StepX Companion"),
                self.button(
                    "desktop-update-close",
                    "收起",
                    a(DesktopUpdateAction::Close),
                    Btn::Secondary,
                ),
            ),
            self.line_w(
                if blockers.is_empty() {
                    u.label()
                } else {
                    "安装包已就绪，重启前还有待处理事项".into()
                },
                22.,
                p.text,
                facloud_ui::fonts::Weight::Semibold,
            ),
        ];
        if let Some(r) = &u.release {
            parts.push(self.text(
                format!(
                    "当前 v{}  →  GitHub v{}",
                    stepx_companion::build_info::VERSION,
                    r.version
                ),
                BODY,
                p.text2,
            ));
            if matches!(
                u.phase,
                Phase::Available | Phase::Current | Phase::Ahead | Phase::Cancelled | Phase::Failed
            ) {
                let notes = r
                    .notes
                    .lines()
                    .filter(|s| !s.trim().is_empty())
                    .take(6)
                    .map(|s| s.trim_start_matches(['#', ' ', '-']).to_owned())
                    .collect::<Vec<_>>()
                    .join("\n");
                parts.push(self.text(Self::truncate_chars(&notes, 480), CAPTION, p.text2));
            }
        }
        if matches!(
            u.phase,
            Phase::Downloading | Phase::Verifying | Phase::Extracting
        ) {
            parts.push(self.update_progress());
            parts.push(self.between(
                self.mono(
                    format!("{} / {}", format_bytes(u.received), format_bytes(u.total)),
                    CAPTION,
                    p.text2,
                ),
                self.mono(
                    if u.speed > 0 {
                        format!("{}/s", format_bytes(u.speed))
                    } else {
                        "SHA-256 · 安装包校验".into()
                    },
                    CAPTION,
                    p.text3,
                ),
            ));
            parts.push(self.text("可继续使用客户端，收起面板不会中断下载。", CAPTION, p.text3));
        }
        if matches!(u.phase, Phase::Checking | Phase::Preparing) {
            parts.push(self.loading_line());
        }
        if u.phase == Phase::Ready {
            parts.push(self.text(
                if blockers.is_empty() {
                    "安装包已校验，可以安装并重启。已保存的设置与个人文件会保留。"
                } else {
                    "下载和校验已完成。请处理下面列出的事项；安装包会保留，无需重新下载。"
                },
                BODY,
                p.text2,
            ));
            if !blockers.is_empty() {
                let rows = blockers
                    .iter()
                    .enumerate()
                    .map(|(i, b)| {
                        self.vstack(
                            S1,
                            vec![
                                self.between(
                                    self.heading(b.title),
                                    self.button(
                                        &format!("desktop-update-resolve-{i}"),
                                        "前往处理",
                                        a(DesktopUpdateAction::Resolve(Box::new(b.action.clone()))),
                                        Btn::Secondary,
                                    ),
                                ),
                                self.text(&b.detail, CAPTION, p.text2),
                                self.divider(),
                            ],
                        )
                    })
                    .collect();
                parts.push(
                    ConstrainedBox::new(self.pane_scroll(
                        self.desktop_update.blockers_scroll.clone(),
                        self.vstack(S3, rows),
                    ))
                    .with_max_height(220.)
                    .finish(),
                );
            }
        }
        if u.phase == Phase::Ahead {
            parts.push(self.text("本机版本较新，继续使用即可；不会降级安装。", BODY, p.text2));
        }
        if !u.error.is_empty() {
            parts.push(self.text(&u.error, CAPTION, p.danger));
        }
        parts.push(self.divider());
        let mut buttons = vec![];
        match u.phase {
            Phase::Available | Phase::Cancelled => buttons.push(self.button(
                "desktop-update-download",
                "下载更新",
                a(DesktopUpdateAction::Download),
                Btn::Primary,
            )),
            Phase::Failed
                if u.release
                    .as_ref()
                    .is_some_and(|r| updater::newer(r).unwrap_or(false)) =>
            {
                buttons.push(self.button(
                    "desktop-update-download",
                    "重新下载",
                    a(DesktopUpdateAction::Download),
                    Btn::Primary,
                ))
            }
            Phase::Downloading | Phase::Verifying | Phase::Extracting => buttons.push(self.button(
                "desktop-update-cancel",
                "取消下载",
                a(DesktopUpdateAction::Cancel),
                Btn::Secondary,
            )),
            Phase::Ready => buttons.push(self.button(
                "desktop-update-install",
                if blockers.is_empty() {
                    "安装并重启"
                } else {
                    "处理第一项"
                },
                if let Some(b) = blockers.first() {
                    a(DesktopUpdateAction::Resolve(Box::new(b.action.clone())))
                } else {
                    a(DesktopUpdateAction::Install)
                },
                Btn::Primary,
            )),
            Phase::Current | Phase::Ahead | Phase::Idle | Phase::Failed => {
                buttons.push(self.button(
                    "desktop-update-check",
                    "重新检查",
                    a(DesktopUpdateAction::Check),
                    Btn::Secondary,
                ))
            }
            _ => {}
        }
        buttons.push(self.button(
            "desktop-update-open-whats-new",
            "本次更新与交流群",
            a(DesktopUpdateAction::OpenWhatsNew),
            Btn::Secondary,
        ));
        buttons.push(self.button(
            "desktop-update-later",
            if u.phase == Phase::Ready {
                "稍后重启"
            } else {
                "稍后"
            },
            a(DesktopUpdateAction::Close),
            Btn::Secondary,
        ));
        parts.push(self.wrap(buttons));
        parts.push(
            self.link_muted(
                "desktop-update-release",
                "查看 GitHub 发布说明",
                Action::StudioOpenUrl(
                    u.release
                        .as_ref()
                        .map_or("https://github.com/xydzf520/stepx/releases".into(), |r| {
                            r.page.clone()
                        }),
                ),
            ),
        );
        if !u.checked.is_empty() {
            parts.push(self.line(
                format!("上次检查 {} · 正式版渠道", u.checked),
                MICRO,
                p.text3,
            ));
        }
        let sheet_inner = Container::new(self.vstack(S3, parts))
            .with_uniform_padding(S5)
            .finish();
        let sheet = self.elevated_overlay_card(sheet_inner, 510., false);
        let sheet = motion::enter(sheet, u.started, !self.settings.value.reduced_motion);
        Some(
            Container::new(Align::new(sheet).bottom_right().finish())
                .with_uniform_padding(S4)
                .with_padding_bottom(38.)
                .with_background_color(if p.dark {
                    rgba(0, 0, 0, 130)
                } else {
                    rgba(15, 23, 42, 90)
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                .finish(),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn idle() -> Companion {
        let mut app = Companion::new_for_test();
        app.discard_settings();
        app
    }
    #[test]
    fn idle_update_has_no_restart_blockers() {
        let app = idle();
        assert!(app.desktop_restart_blockers().is_empty());
    }
    #[test]
    fn drafts_are_identified_without_exposing_or_clearing_contents() {
        let mut app = idle();
        app.fields[F_SMS_BODY].set("私密短信草稿");
        let blockers = app.desktop_restart_blockers();
        assert_eq!(blockers.len(), 1);
        assert_eq!(blockers[0].title, "短信有未发送草稿");
        assert!(matches!(blockers[0].action, Action::Page(Page::Sms)));
        assert!(!blockers[0].detail.contains("私密"));
        assert_eq!(app.fields[F_SMS_BODY].text, "私密短信草稿");
        app.fields[F_SMS_BODY].clear();
        assert!(app.desktop_restart_blockers().is_empty());
    }
    #[test]
    fn completed_studio_prompt_is_not_a_draft_but_revision_is() {
        let mut app = idle();
        app.fields[F_LAB_PROMPT].set("制作专注时钟");
        app.studio_lab
            .artifact_prompts
            .insert("clock".into(), "制作专注时钟".into());
        assert!(app.desktop_restart_blockers().is_empty());
        app.fields[F_LAB_PROMPT].set("制作专注时钟，再增加提醒");
        assert_eq!(
            app.desktop_restart_blockers()[0].title,
            "应用创作有未提交描述"
        );
    }
    #[test]
    fn pending_reply_blocks_restart_outside_chat_page() {
        let mut app = idle();
        app.page = Page::Overview;
        app.amoo_pending_reply.insert(
            "session".into(),
            stepx_companion::amoo::PendingReply {
                key: "session".into(),
                text: "test".into(),
                run_id: "run".into(),
            },
        );
        assert!(app
            .desktop_restart_blockers()
            .iter()
            .any(|r| r.title == "Amoo 正在等待回复"));
    }
    #[test]
    fn unsaved_settings_are_shown_first() {
        let mut app = idle();
        app.fields[F_AMOO].set("保留草稿");
        app.settings.draft.reduced_motion = !app.settings.value.reduced_motion;
        assert_eq!(app.desktop_restart_blockers()[0].title, "设置尚未保存");
        app.discard_settings();
        assert_eq!(
            app.desktop_restart_blockers()[0].title,
            "Amoo 对话有未发送草稿"
        );
    }
    #[test]
    fn app_install_and_message_send_are_protected() {
        use app_store_state::{StoreTaskStage, StoreTaskState};
        let mut app = idle();
        app.app_store.tasks.insert(
            "pkg".into(),
            StoreTaskState {
                package: "pkg".into(),
                app_name: "应用".into(),
                target_version: "1".into(),
                stage: StoreTaskStage::Installing,
                is_downgrade: false,
            },
        );
        app.push.sending_thread = Some("thread".into());
        let rows = app.desktop_restart_blockers();
        assert!(rows.iter().any(|r| r.title == "应用下载或安装正在进行"));
        assert!(rows.iter().any(|r| r.title == "消息正在发送"));
        app.app_store.tasks.get_mut("pkg").unwrap().stage = StoreTaskStage::Completed("ok".into());
        app.push.sending_thread = None;
        assert!(app.desktop_restart_blockers().is_empty());
    }

    #[test]
    fn footer_is_reserved_for_actionable_updates() {
        let mut state = DesktopUpdateState::default();
        for phase in [Phase::Idle, Phase::Checking, Phase::Current, Phase::Ahead] {
            state.phase = phase;
            assert!(!state.shown());
        }
        for phase in [
            Phase::Available,
            Phase::Downloading,
            Phase::Ready,
            Phase::Failed,
        ] {
            state.phase = phase;
            assert!(state.shown());
        }
        state.phase = Phase::Downloading;
        state.fraction = 0.62;
        assert_eq!(state.label(), "下载更新 62%");
        state.phase = Phase::Ready;
        assert_eq!(state.label(), "更新已准备好");
    }

    #[test]
    fn whats_new_pops_once_per_version() {
        let mut persisted = WhatsNewPersistedState::default();
        assert!(persisted.should_pop_once_for_version("0.2.125"));
        assert!(persisted.mark_version_seen("0.2.125"));
        assert!(!persisted.should_pop_once_for_version("0.2.125"));
        assert!(!persisted.mark_version_seen("0.2.125"));
        assert!(persisted.should_pop_once_for_version("0.2.126"));
        assert!(persisted.mark_version_seen("0.2.126"));
    }
}
