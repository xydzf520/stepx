//! One startup check, explicit downloads, and a fixed-footer entry in both shells.
use super::{theme::*, widgets::*, *};
use facloud_ui::TypedActionView;
use stepx_companion::{desktop_update as updater, storage::format_bytes};
#[derive(Clone, Debug)]
pub(super) enum DesktopUpdateAction {
    Open,
    Close,
    Check,
    Download,
    Cancel,
    Install,
    Resolve(Box<Action>),
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
            self.work.running || self.workspace.running.is_some(),
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

    pub(super) fn initialize_desktop_updates(&mut self, ctx: &mut ViewContext<Self>) {
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
    pub(super) fn handle_desktop_update(
        &mut self,
        action: &DesktopUpdateAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use DesktopUpdateAction as A;
        match action {
            A::Open => {
                self.desktop_update.open = true;
                self.desktop_update.started = Instant::now();
                if self.desktop_update.phase == Phase::Idle {
                    self.start_desktop_update_check();
                }
            }
            A::Close => self.desktop_update.open = false,
            A::Check => {
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
                self.button(
                    "desktop-update-settings",
                    "检查更新",
                    Action::DesktopUpdate(DesktopUpdateAction::Check),
                    Btn::Secondary,
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
    pub(super) fn desktop_update_overlay(&self) -> Option<El> {
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
        let sheet = Container::new(self.vstack(S3, parts))
            .with_uniform_padding(S5)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();
        let sheet = motion::enter(sheet, u.started, !self.settings.value.reduced_motion);
        Some(
            Container::new(
                Align::new(ConstrainedBox::new(sheet).with_max_width(510.).finish())
                    .bottom_right()
                    .finish(),
            )
            .with_uniform_padding(S4)
            .with_padding_bottom(38.)
            .with_background_color(rgba(0, 0, 0, 45))
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
}
