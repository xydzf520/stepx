use super::*;
use facloud_ui::TypedActionView;
use stepx_companion::settings::{self, Appearance, Settings};
use tray_icon::{
    menu::{Menu, MenuEvent, MenuItem, PredefinedMenuItem},
    Icon, MouseButton, MouseButtonState, TrayIcon, TrayIconBuilder, TrayIconEvent,
};

#[derive(Clone, Debug)]
pub(super) enum SettingsAction {
    Tab(u8),
    Apply,
    Discard,
    Appearance(Appearance),
    Tray,
    CloseToTray,
    Motion,
    Downloads,
    WorkDirectory,
    OpenDownloads,
    SyncOutputs,
    Expansion,
    Models,
    RetrySync,
    Changelog,
    RecordShortcut,
    CancelShortcut,
    ResetShortcut,
    CaptureShortcut(settings::CompactShortcut),
    SetAppsFilter(u8),
    ToggleAppsSort,
    OpenConfigDir,
    OpenStateDir,
}
pub(super) struct SettingsUi {
    pub value: Settings,
    pub draft: Settings,
    pub notify_draft: stepx_companion::desktop_notify::NotifyPreferences,
    pub tab: u8,
    pub feedback: String,
    pub tray: Option<TrayIcon>,
    pub tray_initialized: bool,
    pub hidden: bool,
    pub changelog: bool,
    pub sync_status: String,
    pub staging: Option<mpsc::Receiver<std::result::Result<String, String>>>,
    pub apps_filter: u8,
    pub apps_sort_by_name: bool,
}
impl Default for SettingsUi {
    fn default() -> Self {
        let value = settings::load();
        let draft = value.clone();
        Self {
            value,
            draft,
            notify_draft: stepx_companion::desktop_notify::load_preferences(),
            tab: std::env::args()
                .position(|a| a == "--settings-tab")
                .and_then(|i| std::env::args().nth(i + 1))
                .and_then(|s| s.parse::<u8>().ok())
                .unwrap_or(0)
                .min(4),
            feedback: String::new(),
            tray: None,
            tray_initialized: false,
            hidden: false,
            changelog: false,
            sync_status: String::new(),
            staging: None,
            apps_filter: 0,
            apps_sort_by_name: true,
        }
    }
}
impl Companion {
    pub(super) fn handle_settings_action(
        &mut self,
        action: &SettingsAction,
        ctx: &mut ViewContext<Self>,
    ) -> Result<()> {
        if self.shortcuts.recording
            && matches!(
                action,
                SettingsAction::Tab(_)
                    | SettingsAction::Apply
                    | SettingsAction::Expansion
                    | SettingsAction::Models
            )
        {
            self.shortcuts.resume()?;
            self.shortcuts.recording = false;
        }
        match action {
            SettingsAction::RecordShortcut => {
                self.shortcuts.suspend()?;
                self.shortcuts.recording = true;
                self.focus = None;
                self.settings.feedback = "请按下组合键；Esc 取消。录入后仍需确认保存。".into();
                return Ok(());
            }
            SettingsAction::CancelShortcut => {
                self.shortcuts.resume()?;
                self.shortcuts.recording = false;
                self.settings.feedback = "已取消录入，原快捷键保持不变".into();
                return Ok(());
            }
            SettingsAction::CaptureShortcut(value) => {
                if !value.valid() {
                    self.settings.feedback =
                        "请使用 Ctrl/Command + Alt/Shift + 字母、数字或 F1–F12；Esc 取消".into();
                    return Ok(());
                }
                self.shortcuts.resume()?;
                self.shortcuts.recording = false;
                self.settings.draft.compact_shortcut = value.clone();
            }
            SettingsAction::ResetShortcut => {
                self.shortcuts.resume()?;
                self.shortcuts.recording = false;
                self.settings.draft.compact_shortcut = Default::default();
            }
            SettingsAction::Discard => {
                self.discard_settings();
                self.settings.feedback = "已放弃更改，原设置保持不变".into();
                return Ok(());
            }
            SettingsAction::Apply => return self.apply_settings(ctx),
            SettingsAction::Tab(tab) => {
                if self.settings.tab != *tab || self.page != Page::Settings {
                    self.trigger_subtab_switch();
                }
                self.page = Page::Settings;
                self.settings.tab = *tab;
                self.settings.feedback.clear();
                self.scroll = Default::default();
                return Ok(());
            }
            SettingsAction::Changelog => {
                self.settings.changelog = !self.settings.changelog;
                self.scroll = Default::default();
                self.trigger_subtab_switch();
                return Ok(());
            }
            SettingsAction::Expansion => {
                self.page = Page::Expansion;
                self.handle_expansion_action(&expansion_state::ExpansionAction::Settings, ctx);
                self.scroll = Default::default();
                self.trigger_subtab_switch();
                return Ok(());
            }
            SettingsAction::Models => {
                if self.fields[F_LAB_ENDPOINT].text.is_empty()
                    && !self.studio_lab.config.endpoint.is_empty()
                {
                    self.fields[F_LAB_ENDPOINT].set(&self.studio_lab.config.endpoint);
                }
                if self.fields[F_LAB_MODEL].text.is_empty()
                    && !self.studio_lab.config.model.is_empty()
                {
                    self.fields[F_LAB_MODEL].set(&self.studio_lab.config.model);
                }
                self.page = Page::Settings;
                self.settings.tab = 2;
                self.scroll = Default::default();
                self.trigger_subtab_switch();
                return Ok(());
            }
            SettingsAction::RetrySync => {
                anyhow::ensure!(self.settings.draft.sync_work_outputs, "请先开启自动发送");
                std::thread::spawn(|| {
                    let _ = stepx_companion::work_sync::retry();
                });
                self.settings.feedback = "将在手机连接后重新发送".into();
                return Ok(());
            }
            SettingsAction::Appearance(value) => self.settings.draft.appearance = *value,
            SettingsAction::Tray => {
                self.settings.draft.tray_icon = !self.settings.draft.tray_icon;
                if !self.settings.draft.tray_icon {
                    self.settings.draft.close_to_tray = false;
                }
            }
            SettingsAction::CloseToTray => {
                anyhow::ensure!(
                    self.settings.draft.tray_icon,
                    "托盘尚未就绪，请先开启托盘图标"
                );
                self.settings.draft.close_to_tray = !self.settings.draft.close_to_tray;
            }
            SettingsAction::Motion => {
                self.settings.draft.reduced_motion = !self.settings.draft.reduced_motion
            }
            SettingsAction::SyncOutputs => {
                self.settings.draft.sync_work_outputs = !self.settings.draft.sync_work_outputs
            }
            SettingsAction::Downloads => {
                if let Some(path) = native_dialog::FileDialog::new()
                    .set_title("选择下载与接收文件夹")
                    .show_open_single_dir()?
                {
                    self.settings.draft.download_directory =
                        Some(settings::choose_directory(&path)?);
                } else {
                    return Ok(());
                }
            }
            SettingsAction::WorkDirectory => {
                anyhow::ensure!(
                    !self.work.running && !self.busy && self.work.attachments.is_empty(),
                    "请先完成当前任务，并发送或移除附件，再更换工作文件夹；草稿会保留"
                );
                if let Some(path) = native_dialog::FileDialog::new()
                    .set_title("选择 Amoo Work 项目文件夹")
                    .show_open_single_dir()?
                {
                    self.settings.draft.work_directory = Some(settings::choose_directory(&path)?);
                } else {
                    return Ok(());
                }
            }
            SettingsAction::OpenDownloads => {
                let p = if let Some(p) = &self.settings.draft.download_directory {
                    p.clone()
                } else {
                    stepx_companion::companion_link::receive_directory()?
                };
                return work_state::open_work_path(&p);
            }
            SettingsAction::SetAppsFilter(filter) => {
                self.settings.apps_filter = (*filter).min(3);
                self.trigger_subtab_switch();
                ctx.notify();
                return Ok(());
            }
            SettingsAction::ToggleAppsSort => {
                self.settings.apps_sort_by_name = !self.settings.apps_sort_by_name;
                self.trigger_subtab_switch();
                ctx.notify();
                return Ok(());
            }
            SettingsAction::OpenConfigDir => {
                let dir = stepx_companion::resources::user_config().join("stepx-companion");
                let _ = std::fs::create_dir_all(&dir);
                return work_state::open_work_path(&dir);
            }
            SettingsAction::OpenStateDir => {
                let dir = stepx_companion::resources::user_state();
                let _ = std::fs::create_dir_all(&dir);
                return work_state::open_work_path(&dir);
            }
        }
        self.settings.feedback = "有未保存的更改，确认保存后才会生效".into();
        ctx.notify();
        Ok(())
    }
    pub(super) fn settings_dirty(&self) -> bool {
        self.settings.value != self.settings.draft || self.push.prefs != self.settings.notify_draft
    }
    pub(super) fn discard_settings(&mut self) {
        let _ = self.shortcuts.resume();
        self.shortcuts.recording = false;
        self.settings.draft = self.settings.value.clone();
        self.settings.notify_draft = self.push.prefs.clone();
    }
    pub(super) fn settings_changes(&self) -> Vec<String> {
        let a = &self.settings.value;
        let b = &self.settings.draft;
        let mut rows = vec![];
        if a.compact_shortcut != b.compact_shortcut {
            rows.push(format!(
                "简洁对话快捷键：{} → {}",
                a.compact_shortcut.label(),
                b.compact_shortcut.label()
            ));
        }
        let flag = |v: bool| if v { "开启" } else { "关闭" };
        if a.appearance != b.appearance {
            rows.push(format!(
                "界面颜色：{}",
                match b.appearance {
                    Appearance::System => "跟随系统",
                    Appearance::Light => "浅色",
                    Appearance::Dark => "深色",
                }
            ));
        }
        for (name, x, y) in [
            ("托盘图标", a.tray_icon, b.tray_icon),
            ("关闭后留在后台", a.close_to_tray, b.close_to_tray),
            ("减少动态效果", a.reduced_motion, b.reduced_motion),
            (
                "工作成果自动发送到手机",
                a.sync_work_outputs,
                b.sync_work_outputs,
            ),
        ] {
            if x != y {
                rows.push(format!("{name}：{} → {}", flag(x), flag(y)));
            }
        }
        if a.download_directory != b.download_directory {
            rows.push(format!(
                "下载与接收位置：{}；已有文件不移动",
                b.download_directory
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
        }
        if a.work_directory != b.work_directory {
            rows.push(format!(
                "Amoo Work 文件夹：{}；已有项目不移动",
                b.work_directory
                    .as_ref()
                    .map(|p| p.display().to_string())
                    .unwrap_or_default()
            ));
        }
        let a = &self.push.prefs;
        let b = &self.settings.notify_draft;
        if a.scope != b.scope {
            rows.push(format!("电脑通知范围：{}", b.scope.label()));
        }
        if a.popup_mode != b.popup_mode {
            rows.push(format!("电脑提醒方式：{}", b.popup_mode.label()));
        }
        if a.privacy_mask != b.privacy_mask {
            rows.push(format!("隐藏通知正文：{}", flag(b.privacy_mask)));
        }
        if a.auto_copy_otp != b.auto_copy_otp {
            rows.push(format!(
                "自动复制验证码到电脑剪贴板：{}",
                flag(b.auto_copy_otp)
            ));
        }
        rows
    }
    fn apply_settings(&mut self, ctx: &mut ViewContext<Self>) -> Result<()> {
        let next = self.settings.draft.clone();
        let old = self.settings.value.clone();
        if old.work_directory != next.work_directory {
            anyhow::ensure!(
                !self.work.running && !self.busy && self.work.attachments.is_empty(),
                "任务或附件仍在处理中，请稍后保存；更改已保留"
            );
        }
        for (a, b) in [
            (&old.download_directory, &next.download_directory),
            (&old.work_directory, &next.work_directory),
        ] {
            if a != b {
                if let Some(p) = b {
                    settings::choose_directory(p)?;
                }
            }
        }
        let shortcut_changed = old.compact_shortcut != next.compact_shortcut;
        if shortcut_changed {
            self.shortcuts.replace(&next.compact_shortcut)?;
        }
        if let Err(error) = settings::save(&next) {
            if shortcut_changed {
                let _ = self.shortcuts.replace(&old.compact_shortcut);
            }
            return Err(error);
        }
        if let Err(e) =
            stepx_companion::desktop_notify::save_preferences(&self.settings.notify_draft)
        {
            if shortcut_changed {
                let _ = self.shortcuts.replace(&old.compact_shortcut);
            }
            let rollback = settings::save(&old);
            anyhow::bail!(
                "通知设置未保存：{e}；电脑设置{}。更改已保留，可重试",
                if rollback.is_ok() {
                    "已恢复"
                } else {
                    "恢复失败，请重新检查"
                }
            );
        }
        self.settings.value = next;
        self.push.prefs = self.settings.notify_draft.clone();
        self.theme_mode = match self.settings.value.appearance {
            Appearance::System => ThemeMode::System,
            Appearance::Light => ThemeMode::Light,
            Appearance::Dark => ThemeMode::Dark,
        };
        if old.tray_icon != self.settings.value.tray_icon {
            self.settings.tray_initialized = false;
            self.sync_tray();
        }
        if old.work_directory != self.settings.value.work_directory {
            self.work.workspace = self.settings.value.work_directory.clone();
            self.work.local_preview = None;
            self.work.local_preview_error = None;
            self.work_files()?;
        }
        self.settings.feedback =
            "已保存并生效。文件位置更改用于后续任务，已有文件保持原位。".into();
        self.record_notice("设置已保存", self.settings.feedback.clone());
        ctx.notify();
        Ok(())
    }
    pub(super) fn sync_tray(&mut self) {
        if self.settings.tray_initialized
            || ((self.demo_mode || self.capture.is_some())
                && !std::env::args().any(|a| a == "--verify-settings"))
        {
            return;
        }
        self.settings.tray_initialized = true;
        self.settings.tray = None;
        if !self.settings.value.tray_icon {
            return;
        }
        let result = (|| -> Result<TrayIcon> {
            let menu = Menu::new();
            for (id, title) in [
                ("stepx-show", "打开 StepX Companion"),
                ("stepx-compact", "简洁对话"),
                ("stepx-settings", "设置"),
            ] {
                menu.append(&MenuItem::with_id(id, title, true, None))?;
            }
            menu.append(&PredefinedMenuItem::separator())?;
            menu.append(&MenuItem::with_id("stepx-quit", "退出", true, None))?;
            let image =
                image::load_from_memory(include_bytes!("../../assets/stepx_companion_icon.png"))?
                    .resize(32, 32, image::imageops::FilterType::Lanczos3)
                    .into_rgba8();
            let (w, h) = image.dimensions();
            let icon = Icon::from_rgba(image.into_raw(), w, h)?;
            Ok(TrayIconBuilder::new()
                .with_id("stepx-companion")
                .with_tooltip("StepX Companion")
                .with_icon(icon)
                .with_menu(Box::new(menu))
                .with_menu_on_left_click(false)
                .build()?)
        })();
        match result {
            Ok(tray) => self.settings.tray = Some(tray),
            Err(e) => {
                self.settings.feedback = format!("当前系统托盘不可用：{e}；关闭窗口会正常退出");
            }
        }
    }
    pub(super) fn tick_settings(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(result) = self
            .settings
            .staging
            .as_ref()
            .and_then(|r| r.try_recv().ok())
        {
            self.settings.staging = None;
            self.settings.sync_status = match result {
                Ok(s) => s,
                Err(e) => format!("准备发送失败：{e}"),
            };
            self.work.log(self.settings.sync_status.clone());
            ctx.notify();
        }
        self.sync_tray();
        if self.page == Page::Settings {
            thread_local! {
                static SUMMARY_CACHE: std::cell::RefCell<Option<(Instant, Option<String>)>> =
                    const { std::cell::RefCell::new(None) };
            }
            let status = if self.settings.staging.is_some() {
                let s = stepx_companion::work_sync::summary();
                SUMMARY_CACHE.with(|c| *c.borrow_mut() = Some((Instant::now(), s.clone())));
                s
            } else {
                SUMMARY_CACHE.with(|c| {
                    let mut guard = c.borrow_mut();
                    if let Some((at, cached)) = guard.as_ref() {
                        if at.elapsed() < Duration::from_secs(2) {
                            return cached.clone();
                        }
                    }
                    let fresh = stepx_companion::work_sync::summary();
                    *guard = Some((Instant::now(), fresh.clone()));
                    fresh
                })
            };
            if let Some(status) = status {
                if self.settings.sync_status != status {
                    self.settings.sync_status = status;
                    ctx.notify();
                }
            }
        }
        let mut menu_ids = Vec::new();
        while let Ok(event) = MenuEvent::receiver().try_recv() {
            menu_ids.push(event.id.as_ref().to_owned());
        }
        while let Ok(event) = TrayIconEvent::receiver().try_recv() {
            if matches!(
                event,
                TrayIconEvent::Click {
                    button: MouseButton::Left,
                    button_state: MouseButtonState::Up,
                    ..
                }
            ) {
                menu_ids.push("stepx-show".to_owned());
            }
        }
        for id in menu_ids {
            self.handle_tray_menu_id(&id, ctx);
            if id == "stepx-quit" {
                break;
            }
        }
    }
    pub(super) fn handle_tray_menu_id(&mut self, menu_id: &str, ctx: &mut ViewContext<Self>) {
        let action = match menu_id {
            "stepx-show" => None,
            "stepx-settings" => Some(Action::Page(Page::Settings)),
            "stepx-compact" => Some(Action::Push(PushAction::SetShellMode(ShellMode::MiniAi))),
            "stepx-quit" => {
                self.handle_action(&Action::QuitApplication, ctx);
                return;
            }
            _ => return,
        };
        self.settings.hidden = false;
        let id = ctx.window_id();
        ctx.windows().show_window_and_focus_app(id);
        if let Some(action) = action {
            self.handle_action(&action, ctx);
        }
        ctx.notify();
    }
    pub(super) fn remember_work_directory(&mut self) -> Result<()> {
        self.settings.value.work_directory = self.work.workspace.clone();
        self.settings.draft.work_directory = self.work.workspace.clone();
        settings::save(&self.settings.value)
    }
}
