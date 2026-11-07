//! Shared Amoo attachment drafts, phone album picker and verified usage presentation.
use stepx_companion::file_dialog::DialogKind;
use super::{icons::UiIcon, theme::*, widgets::*, *};
use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use stepx_companion::{
    amoo_gui::{self, GuiRunResult, GuiRunState, GuiStatus},
    amoo_media, amoo_screen,
    amoo_usage::{self, UsageMetrics},
};

#[derive(Clone, Debug)]
pub(super) enum AmooUiAction {
    GuiMode(bool),
    GuiDesktopMode(bool),
    GuiRefresh,
    GuiWakeAndSwipeUnlock,
    GuiRefreshScreen,
    GuiStopWaiting,
    GuiPauseResume,
    GuiEnableTools,
    GuiPhoneControl(String),
    GuiAcknowledgeFinished,
    AlbumBrowse(String),
    AlbumPick(usize),
    AlbumClose,
    RefreshUsage,
    DownloadPhoneAttachment { path: String, name: String },
}

enum GuiEvent {
    Inspected(std::result::Result<GuiStatus, String>),
    Finished(std::result::Result<GuiRunResult, String>),
    Progress(amoo_screen::ScreenEvent),
}

struct GuiCancellation(Arc<AtomicBool>);
// Dropping a desktop view/observer does not cancel a phone-owned task.
// Cancellation is sent only by an explicit Stop action.

pub(super) struct GuiRecord {
    pub app_name: String,
    pub task: String,
    pub result: GuiRunResult,
    pub desktop: bool,
    pub steps: Vec<(usize, String)>,
    pub screenshot: Option<PathBuf>,
    _screenshot_cache: Option<tempfile::TempDir>,
}

fn gui_requires_confirmation(state: GuiRunState) -> bool {
    matches!(
        state,
        GuiRunState::Submitted
            | GuiRunState::WaitingCancelled
            | GuiRunState::TimedOut
            | GuiRunState::Disconnected
            | GuiRunState::Unknown
    )
}
pub(super) enum AmooUiOutcome {
    GuiControlled(String),
    DraftSent {
        receipt: amoo::PendingReply,
        draft: String,
        attachments: Vec<AmooAttachment>,
    },
    Album(String, Vec<storage::Entry>),
    AlbumFile(tempfile::TempDir, PathBuf),
    Usage(amoo_usage::AccountUsageSnapshot),
}
enum UsageSyncEvent {
    Scope(Option<String>),
    Done(std::result::Result<amoo_usage::AccountUsageSnapshot, String>),
}

#[derive(Default)]
pub(super) struct AmooUiState {
    pub snapshot_generation: u64,
    pub snapshot_worker: Option<super::amoo_sync::SyncWorker>,
    pub snapshot_job: Option<super::amoo_sync::SnapshotJob>,
    pub composer_scroll: ClippedScrollStateHandle,
    pub attachment_scroll: ClippedScrollStateHandle,
    pub screen_mode: bool,
    pub gui_desktop: bool,
    pub gui_status: Option<GuiStatus>,
    pub gui_checking: bool,
    pub gui_running: bool,
    pub gui_paused: bool,
    gui_pause: Option<Arc<AtomicBool>>,
    pub gui_unconfirmed: bool,
    pub gui_unavailable_reason: Option<String>,
    pub gui_message: String,
    pub gui_current: Option<(String, String)>,
    pub gui_started: Option<Instant>,
    pub gui_records: Vec<GuiRecord>,
    pub gui_steps: Vec<(usize, String)>,
    pub gui_screenshot: Option<PathBuf>,
    gui_screenshot_cache: Option<tempfile::TempDir>,
    gui_discard_result: bool,
    gui_last_second: u64,
    gui_rx: Option<mpsc::Receiver<GuiEvent>>,
    gui_cancel: Option<GuiCancellation>,
    pub album_path: Option<String>,
    album_entries: Vec<storage::Entry>,
    album_cache: Vec<tempfile::TempDir>,
    // Keyed by session/message ID so repeated live snapshots never double count.
    messages: BTreeMap<String, BTreeMap<String, UsageMetrics>>,
    session_latest: HashMap<String, UsageMetrics>,
    pub session_started: HashMap<String, Instant>,
    measured: HashMap<String, (String, u64)>,
    pub last_updated: Option<String>,
    pub account_scope: Option<String>,
    pub usage_loading: bool,
    pub usage_error: Option<String>,
    pub usage_attempted: bool,
    usage_rx: Option<mpsc::Receiver<UsageSyncEvent>>,
    coverage_partial: bool,
    sessions_read: usize,
}

impl Companion {
    pub(super) fn choose_amoo_attachments(&mut self, images_only: bool) {
        self.amoo_plus_open = false;
        let mut dialog = stepx_companion::file_dialog::FileDialog::new().set_title(if images_only {
            "添加图片给 Amoo"
        } else {
            "添加文档与附件给 Amoo"
        });
        if images_only {
            dialog = dialog.add_filter("图片", &["png", "jpg", "jpeg", "webp", "gif"]);
        }
        #[cfg(feature = "ui-capture")]
        let fixtures = super::compact_live_check::attachment_selection(images_only);
        #[cfg(not(feature = "ui-capture"))]
        let fixtures: Option<Vec<PathBuf>> = None;
        if let Some(paths) = fixtures {
            if let Err(error) = self.add_amoo_paths(&paths) {
                self.status = format!("{error:#}");
                self.error = true;
            }
            return;
        }
        let expected_task = self.workspace.active.clone();
        let expected_session = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .map(|s| s.session_key.clone());
        let expected_scope = self.amoo_ui.account_scope.clone();
        let expected_device = self.selected.as_ref().map(|d| d.serial.clone());
        self.choose_paths(dialog, DialogKind::OpenFiles, move |app, paths, _| {
            app.ensure_file_dialog_device(&expected_device)?;
            anyhow::ensure!(
                app.workspace.active == expected_task
                    && app
                        .amoo_sessions
                        .get(app.amoo_session_idx)
                        .map(|s| s.session_key.clone())
                        == expected_session
                    && app.amoo_ui.account_scope == expected_scope,
                "当前会话已变化，请在目标会话中重新选择附件；已有草稿保留"
            );
            app.add_amoo_paths(&paths)
        });
    }

    pub(super) fn add_amoo_paths(&mut self, paths: &[PathBuf]) -> Result<()> {
        let mut all: Vec<PathBuf> = self
            .amoo_attachments
            .iter()
            .filter_map(|a| a.local_path.clone())
            .collect();
        for path in paths {
            if !all.contains(path) {
                all.push(path.clone());
            }
        }
        // Use the exact upload validator before altering the draft.
        let prepared = amoo_media::prepare_attachments(&all)?;
        self.amoo_attachments = all
            .into_iter()
            .zip(prepared)
            .map(|(path, meta)| AmooAttachment {
                name: path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                size_label: storage::format_bytes(
                    std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                ),
                is_image: meta["mimeType"]
                    .as_str()
                    .or_else(|| meta["mime"].as_str())
                    .is_some_and(|s| s.starts_with("image/"))
                    || image::ImageFormat::from_path(&path).is_ok(),
                local_path: Some(path),
            })
            .collect();
        self.status = format!(
            "已添加 {} 个附件，点击发送后交给 Amoo 处理",
            self.amoo_attachments.len()
        );
        self.error = false;
        Ok(())
    }

    pub(super) fn send_amoo_draft(&mut self) {
        if self.amoo_ui.screen_mode {
            self.send_amoo_gui_task();
            return;
        }
        if self.busy || self.work.running || self.amoo_ui.gui_running {
            return;
        }
        let draft = self.fields[F_AMOO].text.trim().to_owned();
        if draft.is_empty() && self.amoo_attachments.is_empty() {
            self.status = "请输入消息或添加附件".into();
            self.error = true;
            return;
        }
        if self.demo_mode {
            self.status = "演示模式不能发送真实消息，请连接手机".into();
            return;
        }
        let session = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .map(|s| s.session_key.clone())
            .or_else(|| self.workspace_task().and_then(|t| t.session.clone()));
        if session
            .as_ref()
            .is_some_and(|key| self.amoo_pending_reply.contains_key(key))
        {
            self.status = "上一条消息仍在等待结果，草稿已保留".into();
            return;
        }
        let attachments = self.amoo_attachments.clone();
        let paths: Option<Vec<PathBuf>> =
            attachments.iter().map(|a| a.local_path.clone()).collect();
        let Some(paths) = paths else {
            self.status = "附件来源已失效，请移除后重新选择".into();
            self.error = true;
            return;
        };
        if self.selected.is_none() {
            self.status = "请先连接手机".into();
            self.error = true;
            return;
        }
        // Ordinary chat uses the official installed skill and native ability tool calls.
        self.with_device(
            if paths.is_empty() { "正在发送给 Amoo…" } else { "正在上传附件并发送给 Amoo…" },
            move |adb, device| {
                let prompt = if draft.is_empty() {
                    "请查看并分析这些附件。"
                } else {
                    &draft
                };
                let receipt = send_amoo_with_transient_retry(
                    session,
                    prompt,
                    Duration::from_millis(350),
                    |existing_key| {
                        let mut bus = amoo::AgentBus::connect(adb, &device)?;
                        let key = match existing_key {
                            Some(key) => key,
                            None => bus.create_session()?,
                        };
                        Ok((bus, key))
                    },
                    |bus, key, text| {
                        if paths.is_empty() {
                            bus.send(key, text)
                        } else {
                            amoo_media::send_files(adb, &device, key, text, &paths)
                        }
                    },
                    |bus, key| bus.snapshot(Some(key)),
                ).map_err(|error| error.context(stepx_companion::amoo_background::task_failure_hint(adb, &device)))?;
                Ok(Outcome::AmooUi(AmooUiOutcome::DraftSent {
                    receipt,
                    draft,
                    attachments,
                }))
            },
        );
    }

    pub(super) fn handle_amoo_ui_action(&mut self, action: &AmooUiAction) {
        match action {
            AmooUiAction::GuiMode(enabled) => {
                if *enabled && self.page != Page::Amoo {
                    self.page = Page::Amoo;
                }
                self.amoo_ui.screen_mode = *enabled;
                if *enabled && !self.amoo_ui.gui_running {
                    self.amoo_ui.gui_desktop = true;
                }
                self.amoo_plus_open = false;
                self.amoo_recording = false;
                if *enabled {
                    self.amoo_show_schedules = false;
                    if !self.amoo_ui.gui_running && !self.amoo_ui.gui_unconfirmed {
                        self.refresh_amoo_gui();
                    }
                } else if !self.amoo_ui.gui_running && !self.amoo_ui.gui_paused {
                    self.amoo_ui.gui_checking = false;
                    self.amoo_ui.gui_rx = None;
                    self.amoo_ui.gui_screenshot = None;
                    self.amoo_ui.gui_screenshot_cache = None;
                }
                self.trigger_subtab_switch();
            }
            AmooUiAction::GuiRefresh => self.refresh_amoo_gui(),
            AmooUiAction::GuiWakeAndSwipeUnlock => {
                if self.amoo_ui.gui_running || self.amoo_ui.gui_checking {
                    return;
                }
                let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) else {
                    self.amoo_ui.gui_message =
                        "已发送点亮与上滑解锁指令（连接真实手机后将实时刷新屏幕画面）".into();
                    return;
                };
                let (sender, receiver) = mpsc::channel();
                self.amoo_ui.gui_rx = Some(receiver);
                self.amoo_ui.gui_checking = true;
                self.amoo_ui.gui_message = "正在点亮手机屏幕并执行上滑解锁…".into();
                std::thread::spawn(move || {
                    let result = Adb::discover()
                        .and_then(|adb| {
                            device.verify(&adb)?;
                            let _ = adb.shell(&device.transport, &["input", "keyevent", "224"]);
                            let _ = adb.shell(
                                &device.transport,
                                &["input", "swipe", "540", "1850", "540", "650", "300"],
                            );
                            std::thread::sleep(Duration::from_millis(450));
                            if let Ok(png) = amoo_screen::capture_png(&adb, &device) {
                                let _ = sender.send(GuiEvent::Progress(
                                    amoo_screen::ScreenEvent::Screenshot { number: 0, png },
                                ));
                            }
                            let mut st = amoo_gui::inspect(&adb, &device)?;
                            st.message = if st.locked == Some(false) && st.interactive == Some(true)
                            {
                                "已点亮并解锁手机屏幕，当前画面已刷新，可直接发送操控指令".into()
                            } else {
                                "已点亮屏幕并上滑；若手机设置了锁屏密码，请点击「打开实时投屏」用鼠标解锁".into()
                            };
                            Ok(st)
                        })
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(GuiEvent::Inspected(result));
                });
            }
            AmooUiAction::GuiRefreshScreen => {
                if self.amoo_ui.gui_running || self.amoo_ui.gui_checking {
                    return;
                }
                let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) else {
                    self.amoo_ui.gui_message = "请先连接真实手机以获取实时屏幕画面".into();
                    return;
                };
                let (sender, receiver) = mpsc::channel();
                self.amoo_ui.gui_rx = Some(receiver);
                self.amoo_ui.gui_checking = true;
                self.amoo_ui.gui_message = "正在截取并刷新手机当前画面…".into();
                std::thread::spawn(move || {
                    let result = Adb::discover()
                        .and_then(|adb| {
                            if let Ok(png) = amoo_screen::capture_png(&adb, &device) {
                                let _ = sender.send(GuiEvent::Progress(
                                    amoo_screen::ScreenEvent::Screenshot { number: 0, png },
                                ));
                            }
                            let mut st = amoo_gui::inspect(&adb, &device)?;
                            st.message = format!("已刷新当前手机画面 · {}", st.message);
                            Ok(st)
                        })
                        .map_err(|error| format!("{error:#}"));
                    let _ = sender.send(GuiEvent::Inspected(result));
                });
            }
            AmooUiAction::GuiDesktopMode(desktop) => {
                if !self.amoo_ui.gui_running && !self.amoo_ui.gui_unconfirmed {
                    self.amoo_ui.gui_desktop = *desktop;
                    self.amoo_ui.gui_message = if *desktop {
                        "优先使用手机原生 MCP；复杂操作按 AI 来源规划。选择手机账号时沿用 Amoo 登录态，由 MCP 逐步执行，请保持桌面连接。"
                    } else {
                        "手机原生执行模式：直接调用手机系统内置接口执行。"
                    }.into();
                }
                self.trigger_subtab_switch();
            }
            AmooUiAction::GuiPhoneControl(operation) => {
                let operation = operation.clone();
                self.with_device("正在控制手机 GUI 任务…", move |adb, device| {
                    let state = stepx_companion::companion_gui::call(
                        adb,
                        &device,
                        serde_json::json!({"op":"status"}),
                    )?;
                    anyhow::ensure!(
                        matches!(state["phase"].as_str(), Some("running" | "paused"))
                            || (operation == "resume"
                                && (state["userStopped"] == true || state["userPaused"] == true)),
                        "当前没有正在执行的手机任务"
                    );
                    let next = stepx_companion::companion_gui::call(
                        adb,
                        &device,
                        serde_json::json!({"op":format!("user_{operation}"),"task":state["task"]}),
                    )?;
                    Ok(Outcome::AmooUi(AmooUiOutcome::GuiControlled(
                        next["detail"]
                            .as_str()
                            .unwrap_or("手机任务状态已更新")
                            .into(),
                    )))
                });
            }
            AmooUiAction::GuiEnableTools => {
                self.with_device("正在检查伴侣看屏服务…", |adb, device| {
                    if stepx_companion::model_source::prefers_phone()? {
                        stepx_companion::companion_gui::enable(adb, &device)?;
                        let mut native = stepx_companion::native_mcp::Client::connect(adb, &device)?;
                        native.call("stepx_gui_control", serde_json::json!({"op":"status"}))?;
                        anyhow::ensure!(stepx_companion::amoo_usage::account_scope_fresh(adb, &device)?.is_some(), "请先在手机登录 Amoo 账号");
                        return Ok(Outcome::AmooUi(AmooUiOutcome::GuiControlled(
                            "手机账号与 MCP 操控服务已就绪；无需独立密钥，执行时请保持桌面连接。".into()
                        )));
                    }
                    let state = stepx_companion::companion_gui::call(
                        adb,
                        &device,
                        serde_json::json!({"op":"agent","agentOp":"status"}),
                    )?;
                    let model = if state["modelConfigured"] == true {
                        "模型已配置"
                    } else {
                        "请在手机配置 Step Plan 模型"
                    };
                    let progress = state["progress"].as_str().unwrap_or("暂无任务");
                    Ok(Outcome::AmooUi(AmooUiOutcome::GuiControlled(format!(
                        "手机独立执行 · {model} · {progress}"
                    ))))
                });
            }
            AmooUiAction::GuiPauseResume => {
                if !self.amoo_ui.gui_running && !self.amoo_ui.gui_paused {
                    self.amoo_ui.gui_screenshot = None;
                    self.amoo_ui.gui_screenshot_cache = None;
                    self.status = "当前没有正在执行的手机操控任务".into();
                    return;
                }
                self.amoo_ui.gui_paused = !self.amoo_ui.gui_paused;
                if let Some(pause) = &self.amoo_ui.gui_pause {
                    pause.store(self.amoo_ui.gui_paused, Ordering::Relaxed);
                }
                if self.amoo_ui.gui_paused {
                    self.amoo_ui.gui_message =
                        "已暂停手机操作，你可以随时接手手机；点击「继续」恢复执行。".into();
                    self.status = "已暂停手机操作".into();
                } else {
                    self.amoo_ui.gui_message = "已继续执行手机操作…".into();
                    self.status = "已恢复手机操作".into();
                }
                let paused = self.amoo_ui.gui_paused;
                if let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) {
                    std::thread::spawn(move || {
                        if let Ok(adb) = Adb::discover() {
                            let agent_op = if paused { "pause" } else { "resume" };
                            let _ = stepx_companion::companion_gui::call(
                                &adb,
                                &device,
                                serde_json::json!({"op":"agent","agentOp":agent_op}),
                            );
                            if let Ok(state) = stepx_companion::companion_gui::call(
                                &adb,
                                &device,
                                serde_json::json!({"op":"status"}),
                            ) {
                                if !state["task"].is_null() {
                                    let user_op = if paused { "user_pause" } else { "user_resume" };
                                    let _ = stepx_companion::companion_gui::call(
                                        &adb,
                                        &device,
                                        serde_json::json!({"op":user_op,"task":state["task"]}),
                                    );
                                }
                            }
                        }
                    });
                }
            }
            AmooUiAction::GuiStopWaiting => {
                if let Some(cancel) = &self.amoo_ui.gui_cancel {
                    cancel.0.store(true, Ordering::Relaxed);
                }
                if let Some(pause) = &self.amoo_ui.gui_pause {
                    pause.store(false, Ordering::Relaxed);
                }
                let was_running = self.amoo_ui.gui_running || self.amoo_ui.gui_paused;
                let elapsed_ms = self
                    .amoo_ui
                    .gui_started
                    .map(|t| t.elapsed().as_millis() as u64)
                    .unwrap_or(0);
                self.amoo_ui.gui_running = false;
                self.amoo_ui.gui_paused = false;
                self.amoo_ui.gui_checking = false;
                self.amoo_ui.gui_pause = None;
                self.amoo_ui.gui_cancel = None;
                self.amoo_ui.gui_rx = None;
                self.amoo_ui.gui_discard_result = true;
                if let Some((app_name, task)) = self.amoo_ui.gui_current.take() {
                    self.amoo_ui.gui_records.push(GuiRecord {
                        app_name,
                        task,
                        result: GuiRunResult {
                            state: GuiRunState::WaitingCancelled,
                            message: "已手动停止手机操作。".into(),
                            invocation_id: String::new(),
                            raw_result: None,
                            elapsed_ms,
                        },
                        desktop: self.amoo_ui.gui_desktop,
                        steps: std::mem::take(&mut self.amoo_ui.gui_steps),
                        screenshot: self.amoo_ui.gui_screenshot.take(),
                        _screenshot_cache: self.amoo_ui.gui_screenshot_cache.take(),
                    });
                    if self.amoo_ui.gui_records.len() > 20 {
                        self.amoo_ui.gui_records.remove(0);
                    }
                }
                self.amoo_ui.gui_steps.clear();
                self.amoo_ui.gui_screenshot = None;
                self.amoo_ui.gui_screenshot_cache = None;
                self.amoo_ui.gui_started = None;
                self.amoo_ui.gui_message = "已停止手机操作。".into();
                self.status = "已停止手机操作".into();
                self.error = false;
                if was_running {
                    if let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) {
                        std::thread::spawn(move || {
                            if let Ok(adb) = Adb::discover() {
                                let _ = stepx_companion::companion_gui::call(
                                    &adb,
                                    &device,
                                    serde_json::json!({"op":"agent","agentOp":"stop"}),
                                );
                                if let Ok(state) = stepx_companion::companion_gui::call(
                                    &adb,
                                    &device,
                                    serde_json::json!({"op":"status"}),
                                ) {
                                    if !state["task"].is_null() {
                                        let _ = stepx_companion::companion_gui::call(
                                            &adb,
                                            &device,
                                            serde_json::json!({"op":"user_stop","task":state["task"]}),
                                        );
                                    }
                                }
                            }
                        });
                    }
                }
            }
            AmooUiAction::GuiAcknowledgeFinished => {
                if self.amoo_ui.gui_unconfirmed && !self.amoo_ui.gui_running {
                    self.amoo_ui.gui_unconfirmed = false;
                    self.amoo_ui.gui_message =
                        "你已确认手机上的上一次操作结束，可以填写新的任务。原执行记录已保留。"
                            .into();
                    self.refresh_amoo_gui();
                }
            }
            AmooUiAction::AlbumClose => {
                self.amoo_ui.album_path = None;
                self.trigger_subtab_switch();
            }
            AmooUiAction::AlbumBrowse(path) => {
                if self.busy {
                    return;
                }
                self.trigger_subtab_switch();
                let path = path.clone();
                if !["/sdcard/DCIM", "/sdcard/Pictures"]
                    .iter()
                    .any(|root| path == *root || path.starts_with(&format!("{root}/")))
                {
                    return;
                }
                self.with_device("正在读取手机相册…", move |adb, device| {
                    let rows = storage::list(adb, &device, &path)?
                        .into_iter()
                        .filter(|e| {
                            !e.symlink
                                && !e.name.starts_with('.')
                                && !e.name.ends_with(".stepxlink")
                                && e.name != "StepX Companion"
                                && !stepx_companion::toolbox::is_expansion_stub_filename(&e.name)
                                && !(e.name.starts_with("SC_") && e.name.len() >= 36)
                                && (e.directory
                                    || matches!(
                                        std::path::Path::new(&e.name)
                                            .extension()
                                            .and_then(|s| s.to_str())
                                            .map(str::to_ascii_lowercase)
                                            .as_deref(),
                                        Some("png" | "jpg" | "jpeg" | "gif" | "webp")
                                    ))
                        })
                        .collect();
                    Ok(Outcome::AmooUi(AmooUiOutcome::Album(path, rows)))
                });
            }
            AmooUiAction::AlbumPick(index) => {
                if self.busy {
                    return;
                }
                let Some(entry) = self.amoo_ui.album_entries.get(*index).cloned() else {
                    return;
                };
                if entry.directory {
                    self.handle_amoo_ui_action(&AmooUiAction::AlbumBrowse(entry.path));
                    return;
                }
                if entry.bytes > amoo_media::MAX_ATTACHMENT_BYTES {
                    self.status = "附件总大小不能超过 8 MiB".into();
                    self.error = true;
                    return;
                }
                self.with_device("正在读取所选手机照片…", move |adb, device| {
                    let cache = tempfile::tempdir()?;
                    let target = cache.path().join(&entry.name);
                    storage::pull(adb, &device, &entry.path, &target)?;
                    amoo_media::prepare_attachments(&[target.clone()])?;
                    Ok(Outcome::AmooUi(AmooUiOutcome::AlbumFile(cache, target)))
                });
            }
            AmooUiAction::DownloadPhoneAttachment { path, name } => {
                if self.busy {
                    return;
                }
                if !path.starts_with("/sdcard/") || storage::validate_path(path).is_err() {
                    self.status = "附件不是手机公开存储中的可下载文件".into();
                    self.error = true;
                    return;
                }
                let filename = std::path::Path::new(name)
                    .file_name()
                    .and_then(|s| s.to_str())
                    .filter(|s| !s.is_empty())
                    .unwrap_or("Amoo附件");
                let path = path.clone();
                let expected_device = self.selected.as_ref().map(|d| d.serial.clone());
                self.choose_path(
                    stepx_companion::file_dialog::FileDialog::new()
                        .set_title("保存手机附件")
                        .set_filename(filename)
                        .set_location(&stepx_companion::settings::downloads()),
                    DialogKind::SaveFile,
                    move |app, output, _| {
                        app.ensure_file_dialog_device(&expected_device)?;
                        anyhow::ensure!(!app.busy, "当前操作尚未完成，请重新保存附件");
                        app.with_device("正在下载手机附件…", move |adb, device| {
                            adb.shell(&device.transport, &["test", "-f", &path])?;
                            storage::pull(adb, &device, &path, &output)?;
                            Ok(Outcome::Notice(format!("附件已保存：{}", output.display())))
                        });
                        Ok(())
                    },
                );
            }
            AmooUiAction::RefreshUsage => self.start_account_usage_sync(),
        }
    }

    fn refresh_amoo_gui(&mut self) {
        if self.amoo_ui.gui_running || self.amoo_ui.gui_checking {
            return;
        }
        let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) else {
            self.amoo_ui.gui_message = "请先连接真实手机，再检查屏幕操作能力".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        self.amoo_ui.gui_rx = Some(receiver);
        self.amoo_ui.gui_checking = true;
        self.amoo_ui.gui_message = "正在检查手机屏幕状态并同步实时画面…".into();
        std::thread::spawn(move || {
            let result =
                Adb::discover()
                    .and_then(|adb| {
                        if let Ok(png) = amoo_screen::capture_png(&adb, &device) {
                            let _ = sender.send(GuiEvent::Progress(
                                amoo_screen::ScreenEvent::Screenshot { number: 0, png },
                            ));
                        }
                        amoo_gui::inspect(&adb, &device)
                    })
                    .map_err(|error| format!("{error:#}"));
            let _ = sender.send(GuiEvent::Inspected(result));
        });
    }

    fn send_amoo_gui_task(&mut self) {
        if self.busy || self.work.running || self.amoo_ui.gui_checking || self.amoo_ui.gui_running {
            return;
        }
        if self.amoo_ui.gui_unconfirmed {
            self.amoo_ui.gui_message =
                "上一次手机操作尚未确认结束，请先在手机上检查，避免重复执行。".into();
            return;
        }
        if let Some(reason) = self
            .amoo_ui
            .gui_unavailable_reason
            .as_ref()
            .filter(|_| !self.amoo_ui.gui_desktop)
        {
            self.amoo_ui.gui_message = format!("{reason}。当前手机系统未开放屏幕自动操作，重复提交不会解除限制。更新手机端后请重新连接。 ");
            return;
        }
        if !self.amoo_attachments.is_empty() {
            self.amoo_ui.gui_message =
                "图片与文档附件请在“后台对话”发送。当前附件已保留，未发送手机操作。".into();
            return;
        }
        let raw_app = self.fields[F_AMOO_GUI_APP].text.trim().to_owned();
        let raw_task = self.fields[F_AMOO].text.trim().to_owned();
        if raw_app.is_empty() && raw_task.is_empty() {
            self.amoo_ui.gui_message =
                "请输入要让手机执行的操作（如：点亮屏幕、截取画面、提取屏幕文字、打开微信）".into();
            self.focus = Some(F_AMOO);
            return;
        }
        let (app_name, task) = amoo_screen::infer_app_and_task(&raw_app, &raw_task);
        if app_name.chars().count() > 120
            || app_name.chars().any(char::is_control)
            || task.chars().count() > 12000
            || task
                .chars()
                .any(|c| c.is_control() && c != '\n' && c != '\t')
        {
            self.amoo_ui.gui_message =
                "应用名最多 120 字，操作目标最多 12000 字，请检查输入。".into();
            return;
        }
        let Some(device) = self.selected.clone().filter(|_| !self.demo_mode) else {
            self.amoo_ui.gui_message = "请先连接真实手机".into();
            return;
        };
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        self.amoo_ui.gui_pause = Some(pause.clone());
        self.amoo_ui.gui_paused = false;
        self.amoo_ui.gui_rx = Some(receiver);
        self.amoo_ui.gui_cancel = Some(GuiCancellation(cancel.clone()));
        self.amoo_ui.gui_running = true;
        self.amoo_ui.gui_started = Some(Instant::now());
        self.amoo_ui.gui_last_second = 0;
        self.amoo_ui.gui_current = Some((app_name.clone(), task.clone()));
        self.amoo_ui.gui_discard_result = false;
        self.amoo_ui.gui_steps.clear();
        self.amoo_ui.gui_screenshot = None;
        self.amoo_ui.gui_screenshot_cache = None;
        let desktop = self.amoo_ui.gui_desktop;
        self.amoo_ui.gui_message = if desktop {
            "正在连接手机屏幕并执行操作…"
        } else {
            "正在检查锁屏状态并请求手机原生接口执行…"
        }
        .into();
        std::thread::spawn(move || {
            let result = Adb::discover()
                .and_then(|adb| {
                    if desktop {
                        stepx_companion::gui_agent::run(
                            &adb,
                            &device,
                            &app_name,
                            &task,
                            &stepx_companion::gui_agent::Control {
                                stop: cancel.clone(),
                                pause: pause.clone(),
                            },
                            |event| {
                                let _ = sender.send(GuiEvent::Progress(event));
                            },
                        )
                    } else {
                        amoo_gui::run(&adb, &device, &app_name, &task, &cancel)
                    }
                })
                .map_err(|error| format!("{error:#}"));
            let _ = sender.send(GuiEvent::Finished(result));
        });
    }

    pub(super) fn poll_amoo_gui(&mut self) -> bool {
        if let Some(pause) = &self.amoo_ui.gui_pause {
            self.amoo_ui.gui_paused = pause.load(Ordering::Relaxed);
        }
        let event = match self.amoo_ui.gui_rx.as_ref().map(|rx| rx.try_recv()) {
            Some(Ok(event)) => Some(event),
            Some(Err(mpsc::TryRecvError::Disconnected)) => Some(if self.amoo_ui.gui_running {
                GuiEvent::Finished(Err("桌面等待连接已结束，无法确认手机任务状态".into()))
            } else {
                GuiEvent::Inspected(Err("手机状态检查已中断，请重新检查".into()))
            }),
            _ => None,
        };
        let Some(event) = event else {
            if self.amoo_ui.gui_running {
                let second = self
                    .amoo_ui
                    .gui_started
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                if second != self.amoo_ui.gui_last_second {
                    self.amoo_ui.gui_last_second = second;
                    return true;
                }
            }
            return false;
        };
        match event {
            GuiEvent::Progress(event) => {
                if self.amoo_ui.gui_discard_result {
                    return true;
                }
                match event {
                    amoo_screen::ScreenEvent::Status(message) => self.amoo_ui.gui_message = message,
                    amoo_screen::ScreenEvent::Step {
                        number,
                        description,
                    } => {
                        self.amoo_ui.gui_steps.push((number, description));
                    }
                    amoo_screen::ScreenEvent::Screenshot { number, png } => {
                        if (self.amoo_ui.screen_mode || self.amoo_ui.gui_running)
                            && png.len() <= 16 * 1024 * 1024
                            && image::guess_format(&png).ok() == Some(image::ImageFormat::Png)
                        {
                            if self.amoo_ui.gui_screenshot_cache.is_none() {
                                self.amoo_ui.gui_screenshot_cache = tempfile::tempdir().ok();
                            }
                            if let Some(cache) = &self.amoo_ui.gui_screenshot_cache {
                                // Before/after frames share a step number. Give changed pixels
                                // a new cache key so the preview cannot keep the previous frame.
                                let digest = amoo_media::artifact_content_key(&png);
                                let path =
                                    cache.path().join(format!("screen-{number}-{digest}.png"));
                                if std::fs::write(&path, png).is_ok() {
                                    if let Some(previous) = &self.amoo_ui.gui_screenshot {
                                        if *previous != path {
                                            let _ = std::fs::remove_file(previous);
                                        }
                                    }
                                    self.amoo_ui.gui_screenshot = Some(path);
                                }
                            }
                        }
                    }
                }
                return true;
            }
            GuiEvent::Inspected(result) => {
                self.amoo_ui.gui_rx = None;
                self.amoo_ui.gui_checking = false;
                match result {
                    Ok(status) => {
                        self.amoo_ui.gui_message = status.message.clone();
                        self.amoo_ui.gui_status = Some(status);
                    }
                    Err(error) => {
                        self.amoo_ui.gui_status = None;
                        self.amoo_ui.gui_message = format!("无法检查手机状态：{error}");
                    }
                }
            }
            GuiEvent::Finished(result) => {
                self.amoo_ui.gui_rx = None;
                self.amoo_ui.gui_running = false;
                self.amoo_ui.gui_cancel = None;
                self.amoo_ui.gui_pause = None;
                self.amoo_ui.gui_paused = false;
                let result = result.unwrap_or_else(|error| GuiRunResult {
                    state: GuiRunState::Unknown,
                    message: if self.amoo_ui.gui_desktop {
                        format!("{error}。未能确认手机任务结果，请在 StepX伴侣查看状态；没有自动重复发起任务。")
                    } else {
                        format!("{error}。尚未确认手机是否已执行，请在手机上核对；未自动重试。")
                    },
                    invocation_id: String::new(),
                    raw_result: None,
                    elapsed_ms: self
                        .amoo_ui
                        .gui_started
                        .map(|t| t.elapsed().as_millis() as u64)
                        .unwrap_or(0),
                });
                self.amoo_ui.gui_unconfirmed = gui_requires_confirmation(result.state);
                if self.amoo_ui.gui_discard_result {
                    self.amoo_ui.gui_discard_result = false;
                    self.amoo_ui.gui_started = None;
                    self.amoo_ui.gui_message = if self.amoo_ui.gui_unconfirmed {
                        "账号已变化，旧账号的画面与记录已清除。上一次原生操作尚未确认结束，请在手机上核对。"
                    } else {
                        "账号已变化，已结束旧操作的桌面等待并清除画面与记录。"
                    }.into();
                    return true;
                }
                if !self.amoo_ui.gui_desktop
                    && result.state == GuiRunState::Unavailable
                    && result.raw_result.is_some()
                {
                    // A discovered method can still be disabled by the phone build.
                    // Catalog refresh is not evidence that the native gate opened.
                    self.amoo_ui.gui_unavailable_reason = Some(result.message.clone());
                }
                self.amoo_ui.gui_message = result.message.clone();
                if let Some((app_name, task)) = self.amoo_ui.gui_current.take() {
                    if result.state == GuiRunState::Completed
                        && self.fields[F_AMOO].text.trim() == task
                    {
                        self.fields[F_AMOO].clear();
                    }
                    self.amoo_ui.gui_records.push(GuiRecord {
                        app_name,
                        task,
                        result,
                        desktop: self.amoo_ui.gui_desktop,
                        steps: std::mem::take(&mut self.amoo_ui.gui_steps),
                        screenshot: self.amoo_ui.gui_screenshot.take(),
                        _screenshot_cache: self.amoo_ui.gui_screenshot_cache.take(),
                    });
                    if self.amoo_ui.gui_records.len() > 20 {
                        self.amoo_ui.gui_records.remove(0);
                    }
                }
                self.amoo_ui.gui_started = None;
                // A previously unlocked snapshot is no longer current after an operation.
                self.amoo_ui.gui_status = None;
            }
        }
        true
    }

    pub(super) fn apply_amoo_ui(&mut self, result: AmooUiOutcome) {
        match result {
            AmooUiOutcome::GuiControlled(message) => {
                self.amoo_ui.gui_message = message;
            }
            AmooUiOutcome::DraftSent {
                receipt,
                draft,
                attachments,
            } => {
                if self.fields[F_AMOO].text.trim() == draft {
                    self.fields[F_AMOO].clear();
                }
                for attachment in &attachments {
                    if let Some(index) = self.amoo_attachments.iter().position(|a| a == attachment)
                    {
                        self.amoo_attachments.remove(index);
                    }
                }
                let key = receipt.key.clone();
                if let Some(existing_session) =
                    self.amoo_sessions.iter_mut().find(|s| s.session_key == key)
                {
                    if existing_session.items.last().is_some_and(|item| {
                        matches!(
                            item,
                            AmooChatItem::UserText { text, .. }
                                if text.trim() == receipt.text.trim()
                        )
                    }) {
                        existing_session.items.pop();
                    }
                } else {
                    let initial_title = self
                        .workspace_task()
                        .map(|t| t.title.trim())
                        .filter(|t| !t.is_empty())
                        .unwrap_or("新对话")
                        .to_string();
                    self.amoo_sessions.insert(
                        0,
                        AmooSessionItem {
                            session_key: key.clone(),
                            title: initial_title,
                            agent: "Amoo 助手".into(),
                            status: "等待回复".into(),
                            status_kind: 1,
                            meta: String::new(),
                            items: Vec::new(),
                        },
                    );
                    self.amoo_session_idx = 0;
                }
                self.apply(Outcome::AmooSent(receipt));
                for task in &mut self.workspace.journal.tasks {
                    if task.session.as_deref() == Some(key.as_str())
                        && task.draft.trim() == draft.trim()
                    {
                        task.draft.clear();
                        task.attachments.clear();
                    }
                }
                self.workspace_persist();
                if let Some(session) = self.amoo_sessions.iter_mut().find(|s| s.session_key == key)
                {
                    if let Some(AmooChatItem::UserText { attachment, .. }) =
                        session.items.last_mut()
                    {
                        if !attachments.is_empty() {
                            *attachment = Some((
                                attachments
                                    .iter()
                                    .map(|a| a.name.as_str())
                                    .collect::<Vec<_>>()
                                    .join("、"),
                                format!("{} 个附件", attachments.len()),
                                attachments.iter().all(|a| a.is_image),
                            ));
                        }
                    }
                }
            }
            AmooUiOutcome::Album(path, rows) => {
                self.amoo_ui.album_path = Some(path);
                self.amoo_ui.album_entries = rows;
                self.scroll = Default::default();
                self.status = "选择手机照片，添加到消息草稿".into();
            }
            AmooUiOutcome::AlbumFile(cache, path) => match self.add_amoo_paths(&[path]) {
                Ok(()) => {
                    self.amoo_ui.album_cache.push(cache);
                    self.amoo_ui.album_path = None;
                }
                Err(error) => {
                    self.status = format!("{error:#}");
                    self.error = true;
                }
            },
            AmooUiOutcome::Usage(snapshot) => {
                self.clear_changed_account_views(Some(snapshot.scope.as_str()));
                self.amoo_ui.set_account_scope(Some(snapshot.scope));
                for entry in snapshot.entries {
                    self.amoo_ui
                        .messages
                        .entry(entry.session_key)
                        .or_default()
                        .insert(entry.message_id, entry.usage);
                }
                self.amoo_ui.coverage_partial = snapshot.partial;
                self.amoo_ui.sessions_read = snapshot.sessions_read;
                self.amoo_ui.last_updated =
                    Some(chrono::Local::now().format("%H:%M:%S").to_string());
                self.amoo_ui.save_usage();
                self.status = format!(
                    "已读取 {} 个会话的真实用量{}",
                    snapshot.sessions_read,
                    if snapshot.partial {
                        "（部分历史或缺少计数）"
                    } else {
                        ""
                    }
                );
            }
        }
    }

    pub(super) fn amoo_album_picker(&self) -> Option<El> {
        let path = self.amoo_ui.album_path.as_ref()?;
        let p = self.p();
        let mut rows = vec![
            self.between(
                self.heading("从手机相册添加"),
                self.button(
                    "album-close",
                    "返回对话",
                    Action::AmooUi(AmooUiAction::AlbumClose),
                    Btn::Secondary,
                ),
            ),
            self.wrap(vec![
                self.button(
                    "album-camera",
                    "相机照片",
                    Action::AmooUi(AmooUiAction::AlbumBrowse("/sdcard/DCIM".into())),
                    Btn::Secondary,
                ),
                self.button(
                    "album-pictures",
                    "图片与截图",
                    Action::AmooUi(AmooUiAction::AlbumBrowse("/sdcard/Pictures".into())),
                    Btn::Secondary,
                ),
            ]),
            self.text(path, CAPTION, p.text2),
        ];
        if path != "/sdcard/DCIM" && path != "/sdcard/Pictures" {
            let parent = std::path::Path::new(path)
                .parent()?
                .to_string_lossy()
                .into_owned();
            rows.push(self.button(
                "album-up",
                "上一级",
                Action::AmooUi(AmooUiAction::AlbumBrowse(parent)),
                Btn::Ghost,
            ));
        }
        let page_key = format!("amoo-album:{path}");
        let range = self.list_page_range(&page_key, self.amoo_ui.album_entries.len(), 40);
        rows.push(self.list_page_controls(&page_key, self.amoo_ui.album_entries.len(), 40));
        for (index, entry) in self.amoo_ui.album_entries.iter().enumerate().skip(range.start).take(range.len()) {
            rows.push(self.between(
                self.lead(
                    S2,
                    self.ui_icon(
                        if entry.directory {
                            UiIcon::Folder
                        } else {
                            UiIcon::Image
                        },
                        20.,
                        p.text2,
                    ),
                    self.vstack(
                        S1,
                        vec![
                            self.line(&entry.name, BODY, p.text),
                            self.line(
                                if entry.directory {
                                    "文件夹".into()
                                } else {
                                    storage::format_bytes(entry.bytes)
                                },
                                CAPTION,
                                p.text3,
                            ),
                        ],
                    ),
                ),
                self.button(
                    format!("album-pick-{index}"),
                    if entry.directory {
                        "打开"
                    } else {
                        "添加照片"
                    },
                    Action::AmooUi(AmooUiAction::AlbumPick(index)),
                    Btn::Secondary,
                ),
            ));
        }
        if self.amoo_ui.album_entries.is_empty() {
            rows.push(self.text("此目录没有可选照片，可以切换相册目录。", BODY, p.text2));
        }
        Some(self.scrollable(self.scroll.clone(), self.panel(rows)))
    }

    fn clear_changed_account_views(&mut self, next: Option<&str>) {
        if self.amoo_ui.account_scope.as_deref() != next {
            self.clear_text_interaction();
            self.speech = Default::default();
            if self.amoo_ui.account_scope.is_some()
                || !self.amoo_ui.gui_records.is_empty()
                || self.amoo_ui.gui_running
            {
                self.amoo_ui.clear_gui_account();
            }
            self.work.clear_account_previews();
            if self.amoo_ui.account_scope.is_some()
                || !self.amoo_sessions.is_empty()
                || !self.amoo_pending_reply.is_empty()
                || self.work.running
            {
                // Never reuse a previous account's same-named session, attachment
                // or in-flight response after an account-only refresh.
                self.amoo_sessions.clear();
                self.amoo_session_idx = 0;
                self.amoo_pending_reply.clear();
                self.amoo_cron_jobs.clear();
                self.amoo_cron_count = 0;
                self.amoo_expanded_tool_records.clear();
                self.work.reset_account_work();
            }
        }
    }

    pub(super) fn record_amoo_usage(&mut self, real: &RealAmooData) {
        self.clear_changed_account_views(real.account_scope.as_deref());
        self.amoo_ui.set_account_scope(real.account_scope.clone());
        let mut changed = false;
        for session in &real.sessions {
            if !session.history_loaded {
                continue;
            }
            changed |= self.amoo_ui.merge_usage_messages(session);
            if let Some(msg) = session.messages.iter().rev().find(|m| m.usage.has_tokens()) {
                let mut metrics = msg.usage.clone();
                if let Some((run_id, elapsed)) = self.amoo_ui.measured.get(&session.key) {
                    if msg.run_id == *run_id {
                        metrics = amoo_usage::aggregate(
                            session
                                .messages
                                .iter()
                                .filter(|m| m.run_id == *run_id)
                                .map(|m| &m.usage),
                        );
                        metrics = measured_usage(metrics, *elapsed);
                    }
                }
                self.amoo_ui
                    .session_latest
                    .insert(session.key.clone(), metrics);
            }
            if let Some(pending) = self.amoo_pending_reply.get(&session.key) {
                if pending.finished(session) {
                    if let Some(started) = self.amoo_ui.session_started.remove(&session.key) {
                        let elapsed = started.elapsed().as_millis() as u64;
                        self.amoo_ui
                            .measured
                            .insert(session.key.clone(), (pending.run_id.clone(), elapsed));
                        let metrics = amoo_usage::aggregate(
                            session
                                .messages
                                .iter()
                                .filter(|m| m.run_id == pending.run_id)
                                .map(|m| &m.usage),
                        );
                        self.amoo_ui
                            .session_latest
                            .insert(session.key.clone(), measured_usage(metrics, elapsed));
                    }
                }
            }
        }
        self.amoo_ui.last_updated = Some(chrono::Local::now().format("%H:%M:%S").to_string());
        if !real.sessions.is_empty() || !self.amoo_ui.messages.is_empty() {
            self.amoo_ui.usage_error = None;
        }
        if changed {
            self.amoo_ui.save_usage();
        }
    }

    pub(super) fn record_work_response(
        &mut self,
        data: &RealAmooData,
        key: &str,
        run: &str,
        elapsed_ms: u64,
    ) {
        if self.amoo_ui.account_scope.is_some() && self.amoo_ui.account_scope != data.account_scope
        {
            self.work.reset_account_work();
            return;
        }
        self.record_amoo_usage(data);
        self.amoo_ui.session_started.remove(key);
        self.amoo_ui
            .measured
            .insert(key.into(), (run.into(), elapsed_ms));
        if let Some(session) = data.sessions.iter().find(|session| session.key == key) {
            let metrics = amoo_usage::aggregate(
                session
                    .messages
                    .iter()
                    .filter(|m| m.run_id == run)
                    .map(|m| &m.usage),
            );
            let metrics = measured_usage(metrics, elapsed_ms);
            if metrics.has_tokens() {
                self.amoo_ui.session_latest.insert(key.into(), metrics);
            }
        }
    }

    pub(super) fn amoo_usage_footer(&self) -> El {
        let p = self.p();
        let usage = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .and_then(|s| self.amoo_ui.session_latest.get(&s.session_key));
        let format = |v: Option<u64>| v.map(|n| n.to_string()).unwrap_or_else(|| "—".into());
        let text = if let Some(u) = usage {
            let speed = u
                .tokens_per_second
                .map(|v| format!("{v:.1} tok/s"))
                .unwrap_or_else(|| "— tok/s".into());
            format!(
                "输入 {} · 输出 {} · 平均 {} · 上下文 {} / {}",
                format(u.input_tokens),
                format(u.output_tokens),
                speed,
                format(u.context_used),
                format(u.context_limit)
            )
        } else {
            "Token — · 速度 — · 上下文 —".into()
        };
        self.wrap(vec![
            self.line(text, MICRO, p.text3),
            self.link_action(
                "amoo-account-usage",
                "账号用量",
                Action::Page(Page::Account),
            ),
        ])
    }

    pub(super) fn start_account_usage_sync(&mut self) {
        if self.amoo_ui.usage_loading || self.demo_mode {
            return;
        }
        let Some(mut device) = self.selected.clone() else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        self.amoo_ui.usage_rx = Some(rx);
        self.amoo_ui.usage_loading = true;
        self.amoo_ui.usage_attempted = true;
        self.amoo_ui.usage_error = None;
        // Keep slow account-history reads off the interactive device job queue.
        // Changing devices drops the receiver with AmooUiState; stale results cannot apply.
        std::thread::spawn(move || {
            let result = Adb::discover()
                .and_then(|adb| {
                    if let Ok(active) = device.resolve_active_transport(&adb) {
                        device.transport = active;
                    }
                    if let Ok(scope) = amoo_usage::account_scope(&adb, &device) {
                        let _ = tx.send(UsageSyncEvent::Scope(scope));
                    }
                    amoo_usage::fetch_account_usage(&adb, &device)
                })
                .map_err(|error| format!("{error:#}"));
            let _ = tx.send(UsageSyncEvent::Done(result));
        });
    }

    pub(super) fn poll_account_usage_sync(&mut self) -> bool {
        let mut changed = false;
        while let Some(event) = self.amoo_ui.usage_rx.as_ref().map(|rx| rx.try_recv()) {
            match event {
                Ok(UsageSyncEvent::Scope(scope)) => {
                    if self.amoo_ui.account_scope != scope {
                        self.amoo_ui.account_scope = scope;
                        self.auto_sync.context_triggered_for_cycle = false;
                        self.handle_context_action(&super::context_state::ContextAction::Refresh);
                        changed = true;
                    }
                }
                Ok(UsageSyncEvent::Done(result)) => {
                    self.amoo_ui.usage_rx = None;
                    self.amoo_ui.usage_loading = false;
                    match result {
                        Ok(snapshot) => {
                            let status = self.status.clone();
                            self.apply_amoo_ui(AmooUiOutcome::Usage(snapshot));
                            if self.busy {
                                self.status = status;
                            }
                        }
                        Err(error) => {
                            if self.amoo_ui.messages.is_empty() && !self.amoo_gateway_online {
                                let clean_err = error
                                    .replace("；请检查手机网络及小工具中的伴侣后台协作状态，未确认发送的草稿与附件已保留", "")
                                    .replace("，未确认发送的草稿与附件已保留", "");
                                self.amoo_ui.usage_error = Some(clean_err);
                            } else {
                                self.amoo_ui.usage_error = None;
                            }
                        }
                    }
                    return true;
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    self.amoo_ui.usage_rx = None;
                    self.amoo_ui.usage_loading = false;
                    if self.amoo_ui.messages.is_empty() && !self.amoo_gateway_online {
                        self.amoo_ui.usage_error = Some("用量同步已中断，请重试".into());
                    }
                    return true;
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        changed
    }

    #[cfg(feature = "ui-capture")]
    pub(super) fn seed_demo_usage(&mut self) {
        // Fictional screenshot-only values; never read from or written to an account cache.
        self.amoo_ui.account_scope = Some("demo-usage".into());
        self.amoo_ui.messages.insert(
            "demo:work".into(),
            BTreeMap::from([(
                "demo:reply".into(),
                UsageMetrics {
                    input_tokens: Some(1_200_000),
                    output_tokens: Some(80_000),
                    total_tokens: Some(1_280_000),
                    ..UsageMetrics::default()
                },
            )]),
        );
        self.amoo_ui.last_updated = Some("15:30:00".into());
    }

    pub(super) fn overview_usage_card(&self) -> El {
        let p = self.p();
        let usage = amoo_usage::aggregate(self.amoo_ui.messages.values().flat_map(|m| m.values()));
        let value = |n: Option<u64>| match n {
            Some(n) if n >= 100_000_000 => format!("{:.2} 亿", n as f64 / 100_000_000.),
            Some(n) if n >= 10_000 => format!("{:.2} 万", n as f64 / 10_000.),
            Some(n) => n.to_string(),
            None => "—".into(),
        };
        let mut columns = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_spacing(S4);
        for (label, number) in [
            ("已同步 Token", value(usage.total_tokens)),
            ("输入 Token", value(usage.input_tokens)),
            ("输出 Token", value(usage.output_tokens)),
            ("账号剩余额度", "未提供".into()),
        ] {
            columns = columns.with_child(
                Expanded::new(
                    1.,
                    self.vstack(
                        S1,
                        vec![
                            self.line(label, CAPTION, p.text2),
                            self.line_w(number, 21., p.text, facloud_ui::fonts::Weight::Semibold),
                        ],
                    ),
                )
                .finish(),
            );
        }
        let hint = if self.selected.is_none() {
            "未连接手机".to_string()
        } else if self.amoo_ui.usage_loading {
            "同步中…".into()
        } else if let Some(error) = &self.amoo_ui.usage_error {
            format!("同步未完成：{error}")
        } else if let Some(time) = &self.amoo_ui.last_updated {
            let hm: String = time.chars().take(5).collect();
            format!("{hm} 同步")
        } else {
            "未同步".into()
        };
        self.panel(vec![
            self.between(
                self.heading("Amoo 账号与用量"),
                self.hstack(
                    S2,
                    vec![
                        if self.amoo_ui.usage_loading {
                            self.disabled_button("同步中…")
                        } else if self.selected.is_some() {
                            self.button(
                                "ov-usage-refresh",
                                "刷新用量",
                                Action::AmooUi(AmooUiAction::RefreshUsage),
                                Btn::Ghost,
                            )
                        } else {
                            self.disabled_button("等待连接")
                        },
                        self.link_action(
                            "ov-usage-detail",
                            "查看详情",
                            Action::Page(Page::Account),
                        ),
                    ],
                ),
            ),
            columns.finish(),
            self.text(
                hint,
                CAPTION,
                if self.amoo_ui.usage_error.is_some() {
                    p.warning
                } else {
                    p.text3
                },
            ),
        ])
    }

    pub(super) fn account_usage_page(&self) -> El {
        let p = self.p();
        let aggregate = amoo_usage::aggregate(
            self.amoo_ui
                .messages
                .values()
                .flat_map(|messages| messages.values()),
        );
        let count: usize = self
            .amoo_ui
            .messages
            .values()
            .map(|messages| messages.len())
            .sum();
        let value = |v: Option<u64>| v.map(|n| n.to_string()).unwrap_or_else(|| "未返回".into());
        let mut rows = vec![self.heading("手机 Amoo · Token 用量"), self.text(if self.amoo_ui.account_scope.is_some() { "已识别手机 Amoo 账号，用量按账号隔离保存。Token 来自服务返回的消息记录，不根据字数估算。" } else { "尚未核实手机 Amoo 登录账号，请连接手机后刷新。仅在账号身份可核实时保存用量。" }, BODY, p.text2),
            self.panel(vec![self.heading("账号已读取历史累计"), self.wrap(vec![self.pill_badge(format!("输入 {}", value(aggregate.input_tokens)), p.surface, p.text), self.pill_badge(format!("输出 {}", value(aggregate.output_tokens)), p.surface, p.text), self.pill_badge(format!("合计 {}", value(aggregate.total_tokens)), p.accent_soft, p.text)]),
                self.text(format!("缓存读取 {} Token，缓存写入 {} Token；服务未返回其计费单价。", value(aggregate.cache_read_tokens), value(aggregate.cache_write_tokens)), CAPTION, p.text3),
                self.text(format!("当前账号已同步 {} 个会话、{} 条有用量的回复。多次同步同一条消息不会重复累计。", self.amoo_ui.messages.len(), count), CAPTION, p.text3)]),
            self.panel(vec![self.heading("账号额度与账单"), self.text("手机服务尚未提供可核实的账号总额度、剩余额度与计费周期。已同步消息累计仅覆盖读取过的历史，不代表账号全部消费。", BODY, p.text2)]),
            self.panel(vec![self.heading("速度与上下文"), self.text("平均输出速度按服务返回的输出 Token 和本次任务实际响应时间计算，包含工具执行及等待时间。历史没有可靠耗时时显示 —。上下文显示服务返回的输入 Token；未返回的模型上限显示 —。", BODY, p.text2)]),
        ];
        if self.amoo_ui.usage_loading {
            rows.push(self.text("正在后台同步账号用量…", CAPTION, p.text2));
        } else if let Some(error) = &self.amoo_ui.usage_error {
            rows.push(self.text(format!("同步未完成：{error}"), CAPTION, p.warning));
        }
        if self.amoo_ui.coverage_partial {
            rows.push(self.text(
                "当前统计包含部分历史；部分较早消息或未返回用量的回复未计入。",
                CAPTION,
                p.text3,
            ));
        }
        if let Some(time) = &self.amoo_ui.last_updated {
            rows.push(self.line(format!("最近同步 {time}"), CAPTION, p.text3));
        }
        rows.push(self.wrap(vec![
            self.button(
                "account-to-amoo",
                "Amoo 对话",
                Action::Page(Page::Amoo),
                Btn::Secondary,
            ),
            self.button(
                "account-to-studio",
                "我的作品",
                Action::Page(Page::Studio),
                Btn::Secondary,
            ),
        ]));
        self.scrollable(self.scroll.clone(), self.vstack(S4, rows))
    }
}

impl AmooUiState {
    fn clear_gui_account(&mut self) {
        if let Some(cancel) = &self.gui_cancel {
            cancel.0.store(true, Ordering::Relaxed);
        }
        if let Some(pause) = &self.gui_pause {
            pause.store(false, Ordering::Relaxed);
        }
        self.screen_mode = false;
        self.gui_paused = false;
        self.gui_pause = None;
        self.gui_current = None;
        self.gui_records.clear();
        self.gui_steps.clear();
        self.gui_screenshot = None;
        self.gui_screenshot_cache = None;
        self.gui_status = None;
        self.gui_unavailable_reason = None;
        self.gui_discard_result = self.gui_running;
        if !self.gui_running {
            self.gui_rx = None;
            self.gui_checking = false;
        }
        self.gui_message = if self.gui_running {
            "账号已变化，已清除旧画面并正在结束之前的桌面操作。"
        } else if self.gui_unconfirmed {
            "账号已变化，旧记录已清除；上一次原生操作尚未确认结束，请在手机上核对。"
        } else {
            "账号已变化，旧账号的手机操作记录与画面已清除。"
        }
        .into();
    }

    fn merge_usage_messages(&mut self, session: &openclaw::RealSessionInfo) -> bool {
        if self.account_scope.is_none() {
            return false;
        }
        let mut changed = false;
        for msg in &session.messages {
            if !msg.usage.has_tokens() {
                continue;
            }
            let source = if msg.usage_session_key.is_empty() {
                &session.key
            } else {
                &msg.usage_session_key
            };
            let id = &msg.usage_message_id;
            if id.is_empty() {
                self.coverage_partial = true;
                continue;
            }
            let entries = self.messages.entry(source.clone()).or_default();
            if entries.get(id) != Some(&msg.usage) {
                changed = true;
                entries.insert(id.clone(), msg.usage.clone());
            }
        }
        changed
    }
    pub(super) fn set_account_scope(&mut self, scope: Option<String>) {
        if self.account_scope == scope {
            if scope.is_none() {
                self.messages.clear();
            }
            return;
        }
        self.messages.clear();
        self.session_latest.clear();
        self.measured.clear();
        if self.account_scope.is_some() {
            self.session_started.clear();
        }
        self.account_scope = scope;
        self.coverage_partial = true;
        self.sessions_read = 0;
        if let Some(path) = self.usage_path() {
            if let Some(bytes) = std::fs::read(path)
                .ok()
                .filter(|bytes| bytes.len() <= 8 * 1024 * 1024)
            {
                if let Ok(messages) = serde_json::from_slice(&bytes) {
                    self.messages = messages;
                }
            }
        }
    }
    fn usage_path(&self) -> Option<PathBuf> {
        let scope = self.account_scope.as_ref()?;
        if scope.len() != 64 || !scope.bytes().all(|b| b.is_ascii_hexdigit()) {
            return None;
        }
        Some(
            stepx_companion::resources::user_state()
                .join("amoo-usage-v2")
                .join(format!("{scope}.json")),
        )
    }
    fn save_usage(&self) {
        use std::io::Write;
        let Some(path) = self.usage_path() else {
            return;
        };
        let Some(parent) = path.parent() else {
            return;
        };
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let _ = std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700));
        }
        let Ok(bytes) = serde_json::to_vec(&self.messages) else {
            return;
        };
        // An atomic private temporary file contains counters and opaque IDs only.
        if let Ok(mut file) = tempfile::NamedTempFile::new_in(parent) {
            if file.write_all(&bytes).is_ok() {
                let _ = file.persist(path);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn closing_desktop_does_not_cancel_phone_owned_work() {
        let cancel = Arc::new(AtomicBool::new(false));
        let mut state = AmooUiState::default();
        state.gui_cancel = Some(GuiCancellation(cancel.clone()));
        drop(state);
        assert!(!cancel.load(Ordering::Relaxed));
    }

    #[test]
    fn account_change_hides_old_frames_while_waiting_for_worker_cleanup() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cache = tempfile::tempdir().unwrap();
        let path = cache.path().join("frame.png");
        std::fs::write(&path, b"previous account").unwrap();
        let mut state = AmooUiState::default();
        state.gui_running = true;
        state.gui_desktop = true;
        state.gui_cancel = Some(GuiCancellation(cancel.clone()));
        state.gui_current = Some(("previous app".into(), "previous task".into()));
        state.gui_screenshot = Some(path.clone());
        state.gui_screenshot_cache = Some(cache);
        state.clear_gui_account();
        assert!(cancel.load(Ordering::Relaxed));
        assert!(state.gui_running);
        assert!(state.gui_discard_result);
        assert!(state.gui_current.is_none());
        assert!(state.gui_screenshot.is_none());
        assert!(!path.exists());
    }

    #[test]
    fn unconfirmed_phone_outcomes_never_allow_automatic_resubmission() {
        for state in [
            GuiRunState::Submitted,
            GuiRunState::WaitingCancelled,
            GuiRunState::TimedOut,
            GuiRunState::Disconnected,
            GuiRunState::Unknown,
        ] {
            assert!(gui_requires_confirmation(state), "{state:?}");
        }
        for state in [
            GuiRunState::Completed,
            GuiRunState::Failed,
            GuiRunState::WaitingForUnlock,
            GuiRunState::ScreenOff,
            GuiRunState::Unavailable,
            GuiRunState::CancelledBeforeStart,
        ] {
            assert!(!gui_requires_confirmation(state), "{state:?}");
        }
    }
    fn response(source: &str, native_id: &str, display_id: &str) -> openclaw::RealChatMessage {
        openclaw::RealChatMessage {
            message_id: display_id.into(),
            usage_session_key: source.into(),
            usage_message_id: native_id.into(),
            usage: UsageMetrics {
                input_tokens: Some(30),
                output_tokens: Some(10),
                total_tokens: Some(40),
                ..Default::default()
            },
            ..Default::default()
        }
    }
    #[test]
    fn run_speed_uses_entire_measured_duration_after_aggregation() {
        let first = UsageMetrics {
            output_tokens: Some(20),
            response_ms: Some(1000),
            tokens_per_second: Some(20.),
            ..Default::default()
        };
        let last = UsageMetrics {
            output_tokens: Some(10),
            response_ms: Some(1000),
            tokens_per_second: Some(10.),
            ..Default::default()
        };
        let total = measured_usage(amoo_usage::aggregate([&first, &last]), 2000);
        assert_eq!(total.output_tokens, Some(30));
        assert_eq!(total.tokens_per_second, Some(15.));
        assert_eq!(total.response_ms, Some(2000));
    }
    #[test]
    fn child_history_and_account_sync_share_one_native_usage_record() {
        let mut state = AmooUiState {
            account_scope: Some("test".into()),
            ..Default::default()
        };
        let parent = openclaw::RealSessionInfo {
            key: "parent".into(),
            messages: vec![response("child", "native", "child:child:native")],
            ..Default::default()
        };
        let child = openclaw::RealSessionInfo {
            key: "child".into(),
            messages: vec![response("child", "native", "native")],
            ..Default::default()
        };
        assert!(state.merge_usage_messages(&parent));
        assert!(!state.merge_usage_messages(&parent));
        assert!(!state.merge_usage_messages(&child));
        assert_eq!(state.messages.len(), 1);
        assert_eq!(
            amoo_usage::aggregate(state.messages.values().flat_map(|m| m.values())).total_tokens,
            Some(40)
        );
    }
    #[test]
    fn unknown_account_and_unstable_ids_do_not_enter_account_totals() {
        let session = openclaw::RealSessionInfo {
            key: "one".into(),
            messages: vec![response("one", "id", "id")],
            ..Default::default()
        };
        let mut state = AmooUiState::default();
        assert!(!state.merge_usage_messages(&session));
        state.account_scope = Some("test".into());
        let missing_id = openclaw::RealSessionInfo {
            key: "one".into(),
            messages: vec![response("one", "", "synthetic")],
            ..Default::default()
        };
        assert!(!state.merge_usage_messages(&missing_id));
        assert!(state.coverage_partial);
        assert!(state.messages.is_empty());
    }
    #[test]
    fn account_change_discards_previous_timing_and_usage() {
        let mut state = AmooUiState {
            account_scope: Some("first-test-scope".into()),
            ..Default::default()
        };
        state.session_started.insert("one".into(), Instant::now());
        state
            .messages
            .entry("one".into())
            .or_default()
            .insert("id".into(), UsageMetrics::default());
        state.set_account_scope(Some("second-test-scope".into()));
        assert!(state.messages.is_empty());
        assert!(state.session_started.is_empty());
    }

    #[test]
    fn gui_pause_resume_and_stop_waiting_immediately_update_state() {
        let mut app = Companion::new_for_test();
        let cancel = Arc::new(AtomicBool::new(false));
        let pause = Arc::new(AtomicBool::new(false));
        let cache = tempfile::tempdir().unwrap();
        let screenshot_path = cache.path().join("live.png");
        std::fs::write(&screenshot_path, b"live frame").unwrap();

        app.amoo_ui.gui_running = true;
        app.amoo_ui.gui_desktop = true;
        app.amoo_ui.gui_cancel = Some(GuiCancellation(cancel.clone()));
        app.amoo_ui.gui_pause = Some(pause.clone());
        app.amoo_ui.gui_current = Some(("设置".into(), "查看关于手机".into()));
        app.amoo_ui.gui_screenshot = Some(screenshot_path);
        app.amoo_ui.gui_screenshot_cache = Some(cache);

        app.handle_amoo_ui_action(&AmooUiAction::GuiPauseResume);
        assert!(app.amoo_ui.gui_paused);
        assert!(pause.load(Ordering::Relaxed));
        assert!(app.amoo_ui.gui_message.contains("已暂停"));

        app.handle_amoo_ui_action(&AmooUiAction::GuiPauseResume);
        assert!(!app.amoo_ui.gui_paused);
        assert!(!pause.load(Ordering::Relaxed));
        assert!(app.amoo_ui.gui_message.contains("已继续"));

        app.handle_amoo_ui_action(&AmooUiAction::GuiPauseResume);
        assert!(app.amoo_ui.gui_paused);

        app.handle_amoo_ui_action(&AmooUiAction::GuiStopWaiting);
        assert!(cancel.load(Ordering::Relaxed));
        assert!(!pause.load(Ordering::Relaxed));
        assert!(!app.amoo_ui.gui_running);
        assert!(!app.amoo_ui.gui_paused);
        assert!(app.amoo_ui.gui_screenshot.is_none());
        assert!(app.amoo_ui.gui_current.is_none());
        assert_eq!(app.amoo_ui.gui_records.len(), 1);
        assert_eq!(
            app.amoo_ui.gui_records[0].result.state,
            GuiRunState::WaitingCancelled
        );
    }

    #[test]
    fn uncertain_send_is_not_repeated_when_history_cannot_be_verified() {
        for history_error in [true, false] {
            let mut calls = 0;
            let result = send_amoo_with_transient_retry(
                Some("test-session".into()), "草稿", Duration::ZERO,
                |key| Ok(((), key.unwrap())),
                |_, _, _| { calls += 1; anyhow::bail!("Connection reset by peer") },
                |_, _| if history_error { anyhow::bail!("history timed out") }
                    else { Ok(RealAmooData::default()) },
            );
            assert!(result.unwrap_err().to_string().contains("尚未确认"));
            assert_eq!(calls, 1, "an unverified receipt must not cause duplicate delivery");
        }
    }

    #[test]
    fn transient_amoo_send_error_detection_and_draft_sent_clears_bound_task_draft() {
        assert!(is_transient_amoo_send_error(
            "Amoo 云连接已中断或超时；请检查手机网络及小工具中的伴侣后台协作状态，未确认发送的草稿与附件已保留: Resource temporarily unavailable (os error 11)"
        ));
        assert!(is_transient_amoo_send_error("Connection reset by peer"));
        assert!(!is_transient_amoo_send_error("Amoo 未接受此消息，输入内容已保留"));

        let mut app = Companion::new_for_test();
        let mut task = stepx_companion::amoo_workspace::Task::new(None, None);
        task.title = "上海天气".into();
        task.draft = "上海天气".into();
        task.steps = vec![stepx_companion::amoo_workspace::Step {
            executor: stepx_companion::amoo_workspace::Executor::Phone,
            phase: stepx_companion::amoo_workspace::Phase::Running,
            note: String::new(),
        }];
        let task_id = task.id.clone();
        app.workspace.journal.tasks.push(task);
        app.workspace.active = Some(task_id.clone());
        app.workspace.running = Some(super::super::workspace_state::ActiveRun {
            task: task_id.clone(),
            step: 0,
            executor: stepx_companion::amoo_workspace::Executor::Phone,
            accepted: false,
            started_at: Instant::now(),
        });
        app.fields[F_AMOO].set("上海天气");

        app.apply_amoo_ui(AmooUiOutcome::DraftSent {
            receipt: amoo::PendingReply {
                key: "agent:main:shanghai-weather".into(),
                text: "上海天气".into(),
                run_id: "run-sh-1".into(),
            },
            draft: "上海天气".into(),
            attachments: vec![],
        });

        assert!(app.fields[F_AMOO].text.is_empty());
        let updated = app.workspace_task().unwrap();
        assert_eq!(
            updated.session.as_deref(),
            Some("agent:main:shanghai-weather")
        );
        assert!(updated.draft.is_empty());

        // Sending again on the same session when UserText is already at the tail of items
        // replaces it in-place instead of pushing a duplicate UserText bubble.
        app.apply_amoo_ui(AmooUiOutcome::DraftSent {
            receipt: amoo::PendingReply {
                key: "agent:main:shanghai-weather".into(),
                text: "上海天气".into(),
                run_id: "run-sh-2".into(),
            },
            draft: "上海天气".into(),
            attachments: vec![],
        });
        let session = app
            .amoo_sessions
            .iter()
            .find(|s| s.session_key == "agent:main:shanghai-weather")
            .unwrap();
        let user_bubbles = session
            .items
            .iter()
            .filter(|i| matches!(i, AmooChatItem::UserText { text, .. } if text == "上海天气"))
            .count();
        assert_eq!(
            user_bubbles, 1,
            "DraftSent on retry must not duplicate UserText bubble in amoo_sessions"
        );
    }

    #[test]
    fn send_amoo_with_transient_retry_handles_eagain_and_avoids_stale_turn_false_positive() {
        use stepx_companion::openclaw::{RealAmooData, RealChatMessage, RealSessionInfo};

        // Case 1: Transient os error 11 on connect_and_key, then transient os error 11 on send_once
        // when the session already has an OLD settled turn with the same prompt "上海天气".
        // A readable old/completed turn is not proof this new send was rejected.
        // Keep the draft and never issue another request with a new idempotency key.
        let mut connect_calls = 0usize;
        let mut send_calls = 0usize;
        let receipt = send_amoo_with_transient_retry(
            Some("agent:main:existing".into()),
            "上海天气",
            Duration::ZERO,
            |existing| {
                connect_calls += 1;
                if connect_calls == 1 {
                    anyhow::bail!("Resource temporarily unavailable (os error 11)");
                }
                Ok(((), existing.unwrap_or_else(|| "agent:main:new".into())))
            },
            |_bus, key, text| {
                send_calls += 1;
                if send_calls == 1 {
                    anyhow::bail!(
                        "Amoo 云连接已中断或超时: Resource temporarily unavailable (os error 11)"
                    );
                }
                Ok(amoo::PendingReply {
                    key: key.into(),
                    text: text.into(),
                    run_id: "run-new-2".into(),
                })
            },
            |_bus, key| {
                Ok(RealAmooData {
                    sessions: vec![RealSessionInfo {
                        key: key.into(),
                        history_loaded: true,
                        messages: vec![
                            RealChatMessage {
                                message_id: "m1".into(),
                                role: "user".into(),
                                text: "上海天气".into(),
                                timestamp_ms: 1000,
                                run_id: "run-old-1".into(),
                                stop_reason: String::new(),
                                is_error: false,
                                error_message: None,
                                ..Default::default()
                            },
                            RealChatMessage {
                                message_id: "m2".into(),
                                role: "assistant".into(),
                                text: "上次错误回复".into(),
                                timestamp_ms: 1001,
                                run_id: "run-old-1".into(),
                                stop_reason: "error".into(),
                                is_error: true,
                                error_message: Some("上次连接超时".into()),
                                ..Default::default()
                            },
                        ],
                        ..Default::default()
                    }],
                    ..Default::default()
                })
            },
        )
        .expect_err("ambiguous send must retain draft instead of repeating the request");

        assert_eq!(connect_calls, 3);
        assert_eq!(
            send_calls, 1,
            "a settled or lagging history cannot authorize a second send"
        );
        assert!(receipt.to_string().contains("尚未确认"));

        // Case 2: Transient error after phone already accepted the new user turn (no settled assistant reply after it)
        // -> must recover PendingReply from snapshot without calling send_once a second time.
        let mut send_calls_accepted = 0usize;
        let recovered = send_amoo_with_transient_retry(
            Some("agent:main:existing".into()),
            "上海天气",
            Duration::ZERO,
            |existing| Ok(((), existing.unwrap())),
            |_bus, _key, _text| {
                send_calls_accepted += 1;
                anyhow::bail!("Resource temporarily unavailable (os error 11)")
            },
            |_bus, key| {
                Ok(RealAmooData {
                    sessions: vec![RealSessionInfo {
                        key: key.into(),
                        history_loaded: true,
                        messages: vec![RealChatMessage {
                            message_id: "m3".into(),
                            role: "user".into(),
                            text: "上海天气".into(),
                            timestamp_ms: chrono::Utc::now().timestamp_millis(),
                            run_id: "run-accepted-on-phone".into(),
                            stop_reason: String::new(),
                            is_error: false,
                            error_message: None,
                            ..Default::default()
                        }],
                        ..Default::default()
                    }],
                    ..Default::default()
                })
            },
        )
        .expect("should recover accepted PendingReply from snapshot");

        assert_eq!(
            send_calls_accepted, 1,
            "must not double-send when snapshot confirms phone accepted the turn"
        );
        assert_eq!(recovered.run_id, "run-accepted-on-phone");
    }
}

pub(super) fn recover_accepted_pending_reply(
    snap: stepx_companion::openclaw::RealAmooData,
    key: &str,
    prompt: &str,
    send_started_ms: i64,
) -> Option<amoo::PendingReply> {
    let sess = snap.sessions.into_iter().find(|s| s.key == key)?;
    let last_user_idx = sess.messages.iter().rposition(|m| m.role == "user")?;
    let user_msg = &sess.messages[last_user_idx];
    let trimmed = prompt.trim();
    if user_msg.timestamp_ms < send_started_ms
        || user_msg.run_id.trim().is_empty()
        || (user_msg.text.trim() != trimmed && !user_msg.text.contains(trimmed))
    {
        return None;
    }
    let after_user = &sess.messages[last_user_idx + 1..];
    let already_settled = after_user.iter().any(|m| {
        m.role == "assistant"
            && (m.is_error
                || amoo::is_terminal(&m.stop_reason)
                || !m.attachments.is_empty()
                || (m.stop_reason.trim().is_empty() && !m.text.trim().is_empty()))
    });
    if already_settled {
        return None;
    }
    Some(amoo::PendingReply {
        key: key.to_string(),
        text: prompt.to_string(),
        run_id: user_msg.run_id.clone(),
    })
}

pub(super) fn send_amoo_with_transient_retry<B, ConnectFn, SendFn, SnapshotFn>(
    session: Option<String>,
    prompt: &str,
    retry_delay: Duration,
    mut connect_and_key: ConnectFn,
    mut send_once: SendFn,
    mut snapshot_session: SnapshotFn,
) -> Result<amoo::PendingReply>
where
    ConnectFn: FnMut(Option<String>) -> Result<(B, String)>,
    SendFn: FnMut(&mut B, &str, &str) -> Result<amoo::PendingReply>,
    SnapshotFn: FnMut(&mut B, &str) -> Result<stepx_companion::openclaw::RealAmooData>,
{
    let (mut bus, key) = match connect_and_key(session.clone()) {
        Ok(pair) => pair,
        Err(err) if is_transient_amoo_send_error(&format!("{err:#}")) => {
            if !retry_delay.is_zero() {
                std::thread::sleep(retry_delay);
            }
            connect_and_key(session)?
        }
        Err(err) => return Err(err),
    };
    let send_started_ms = chrono::Utc::now().timestamp_millis();
    let mut receipt = match send_once(&mut bus, &key, prompt) {
        Ok(r) => r,
        Err(err) if is_transient_amoo_send_error(&format!("{err:#}")) => {
            if !retry_delay.is_zero() {
                std::thread::sleep(retry_delay);
            }
            let (mut retry_bus, _) = connect_and_key(Some(key.clone()))?;
            // A timeout does not prove rejection. If history cannot be read,
            // preserve the draft and report uncertainty instead of sending twice.
            let snapshot = snapshot_session(&mut retry_bus, &key)
                .map_err(|check| anyhow::anyhow!("尚未确认 Amoo 是否收到消息，草稿已保留；请刷新会话后再决定是否重试。发送：{err:#}；核对：{check:#}"))?;
            anyhow::ensure!(snapshot.sessions.iter().any(|s| s.key == key && s.history_loaded),
                "尚未确认 Amoo 是否收到消息，草稿已保留；会话历史未读到，请刷新会话后再决定是否重试");
            let already_accepted = recover_accepted_pending_reply(snapshot, &key, prompt, send_started_ms);
            match already_accepted {
                Some(recovered) => recovered,
                None => anyhow::bail!("尚未确认 Amoo 是否收到本次消息，草稿已保留；请刷新会话核对后再决定是否重试"),
            }
        }
        Err(err) => return Err(err),
    };
    receipt.text = prompt.to_string();
    Ok(receipt)
}

pub(super) fn is_transient_amoo_send_error(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("os error 11")
        || lower.contains("resource temporarily unavailable")
        || lower.contains("would block")
        || lower.contains("timed out")
        || lower.contains("connection reset")
        || lower.contains("broken pipe")
        || lower.contains("unexpected eof")
        || err.contains("连接已中断或超时")
        || err.contains("后台连接已关闭")
}

fn measured_usage(mut metrics: UsageMetrics, elapsed_ms: u64) -> UsageMetrics {
    metrics.response_ms = None;
    metrics.tokens_per_second = None;
    metrics.with_observed_response_ms(elapsed_ms)
}
