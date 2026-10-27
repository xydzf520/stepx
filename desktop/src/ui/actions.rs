//! Input handling: every `Action` is resolved here. UI-only actions return immediately;
//! device actions are queued on the ADB worker thread (one at a time).

use super::*;
use facloud_ui::{clipboard::ClipboardContent, TypedActionView};

impl TypedActionView for Companion {
    type Action = Action;

    fn handle_action(&mut self, action: &Action, ctx: &mut ViewContext<Self>) {
        if self.interaction_gate(action, ctx) {
            return;
        }
        if let Action::DesktopUpdate(action) = action {
            self.handle_desktop_update(action, ctx);
            return;
        }
        if self.desktop_update.open {
            if matches!(action, Action::Escape) {
                self.desktop_update.open = false;
                ctx.notify();
                return;
            }
            if !matches!(
                action,
                Action::MinimizeWindow
                    | Action::MaximizeWindow
                    | Action::CloseWindow
                    | Action::QuitApplication
                    | Action::StudioOpenUrl(_)
            ) {
                return;
            }
        }

        if self.handle_context(action, ctx) {
            self.sync_ime(ctx);
            return;
        }
        if self.shortcuts.recording
            && matches!(
                action,
                Action::Page(_)
                    | Action::CloseWindow
                    | Action::QuitApplication
                    | Action::CompactChat
            )
        {
            let _ = self.shortcuts.resume();
            self.shortcuts.recording = false;
        }
        if matches!(action, Action::QuitApplication) {
            self.context_menu = None;
            self.work.image_viewer = None;
            self.adb_guide_step = None;
            self.handle_ui_action(action, ctx);
            return;
        }
        if self.work.image_viewer.is_some() {
            if matches!(action, Action::Escape) {
                self.work.image_viewer = None;
                ctx.notify();
                return;
            }
            if !matches!(
                action,
                Action::Work(_)
                    | Action::CopyText(..)
                    | Action::StudioOpenUrl(_)
                    | Action::MinimizeWindow
                    | Action::MaximizeWindow
                    | Action::CloseWindow
                    | Action::QuitApplication
            ) {
                return;
            }
        }
        // The guide is modal: no background shortcuts or device operations leak through.
        if self.adb_guide_step.is_some()
            && !matches!(
                action,
                Action::AdbGuide(_)
                    | Action::AdbGuideContinue
                    | Action::Refresh
                    | Action::Escape
                    | Action::MinimizeWindow
                    | Action::MaximizeWindow
                    | Action::CloseWindow
                    | Action::QuitApplication
                    | Action::StudioOpenUrl(_)
            )
        {
            return;
        }
        if matches!(action, Action::CompactChat) {
            self.summon_compact_chat(ctx);
            self.sync_ime(ctx);
            return;
        }
        if (self.back_screen.playing
            || self.back_screen.generating
            || self.back_screen.applying_native
            || self.back_screen.companion_busy)
            && matches!(
                action,
                Action::Select(_) | Action::Forget | Action::SetPreferredDevice(_)
            )
        {
            self.status = "背屏正在播放或创作，请先停止或取消后切换设备".into();
            ctx.notify();
            return;
        }
        if self.busy
            && matches!(self.job_page, Page::Work | Page::Amoo | Page::Account)
            && matches!(
                action,
                Action::Select(_) | Action::Forget | Action::SetPreferredDevice(_)
            )
        {
            self.status = "Amoo 正在上传或同步，请等待结果后切换设备".into();
            ctx.notify();
            return;
        }
        if self.work.running
            && matches!(
                action,
                Action::Select(_)
                    | Action::Forget
                    | Action::SetPreferredDevice(_)
                    | Action::AmooNewChat
                    | Action::AmooSend
                    | Action::AmooSelectSession(_)
                    | Action::AmooRetryMessage { .. }
            )
        {
            self.status = "桌面协作正在执行，请先等待完成或停止执行".into();
            ctx.notify();
            return;
        }
        if self.updates.transfer.is_some()
            && matches!(
                action,
                Action::Select(_) | Action::Forget | Action::SetPreferredDevice(_)
            )
        {
            self.status = "正在准备手机更新包，请先完成或取消后再切换设备".into();
            ctx.notify();
            return;
        }
        if let Action::Settings(action) = action {
            if let Err(error) = self.handle_settings_action(action, ctx) {
                self.settings.feedback = format!("尚未保存：{error}");
                self.record_notice("设置未保存", self.settings.feedback.clone());
                self.error = true;
            } else {
                if let SettingsAction::SetAppsFilter(filter) = action {
                    self.settings.apps_filter = (*filter).min(3);
                }
                self.error = false;
            }
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if let Action::Expansion(action) = action {
            if matches!(
                action,
                expansion_state::ExpansionAction::Settings
                    | expansion_state::ExpansionAction::Back
                    | expansion_state::ExpansionAction::Filter(_)
                    | expansion_state::ExpansionAction::State(_)
                    | expansion_state::ExpansionAction::SetGalleryGrid(_)
            ) {
                self.motion_started = Instant::now();
            }
            self.handle_expansion_action(action, ctx);
            return;
        }
        if let Action::Updates(action) = action {
            self.handle_update_action(action, ctx);
            return;
        }
        if let Action::AppStore(action) = action {
            self.handle_app_store_action(action.clone(), ctx);
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if let Action::BackScreen(action) = action {
            self.handle_backscreen_action(action, ctx);
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if self.page == Page::Amoo
            || matches!(
                action,
                Action::Page(Page::Amoo)
                    | Action::Workspace(_)
                    | Action::AmooUi(_)
                    | Action::AmooSend
                    | Action::AmooSelectSession(_)
                    | Action::AmooNewChat
                    | Action::AmooRetryMessage { .. }
                    | Action::AmooRetryReply
                    | Action::SubmitFocused
                    | Action::QuickGuiScenario { .. }
            )
        {
            self.demote_busy_if_background();
        }
        if let Action::AmooUi(action) = action {
            if matches!(action, AmooUiAction::GuiMode(true))
                && self.push.shell_mode != ShellMode::Full
            {
                self.set_compact_mode(ShellMode::Full, ctx);
            }
            self.handle_amoo_ui_action(action);
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if self.page == Page::Amoo
            && matches!(action, Action::Escape)
            && (self.workspace.panel != 0 || self.workspace.history)
        {
            self.handle_action(&Action::Workspace(WorkspaceAction::ClosePanel), ctx);
            return;
        }
        if let Action::Workspace(action) = action {
            if matches!(action, WorkspaceAction::SendScheduleToAmoo(_)) {
                self.page = Page::Amoo;
            }
            self.handle_workspace(action, ctx);
            self.sync_ime(ctx);
            return;
        }
        if matches!(action, Action::AmooSend) && !self.amoo_ui.screen_mode {
            self.handle_workspace(&WorkspaceAction::Prepare, ctx);
            self.sync_ime(ctx);
            return;
        }
        if let Action::StudioLab(action) = action {
            self.handle_studio_lab_action(action, ctx);
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if let Action::Studio(action) = action {
            if matches!(
                action,
                StudioAction::Tab(_)
                    | StudioAction::Source(_)
                    | StudioAction::Category(_)
                    | StudioAction::Select(_)
                    | StudioAction::WebSelect(_)
                    | StudioAction::CloudCategory(_)
                    | StudioAction::CloudSelect(_)
            ) {
                self.motion_started = Instant::now();
            }
            self.handle_studio_action(action, ctx);
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if let Action::Work(action) = action {
            if matches!(
                action,
                WorkAction::Tab(_)
                    | WorkAction::Panel(_)
                    | WorkAction::Mode(_)
                    | WorkAction::NewChat
                    | WorkAction::Session(_)
            ) {
                self.motion_started = Instant::now();
            }
            if !action.keeps_current_shell()
                && !matches!(self.page, Page::Amoo | Page::Work | Page::Studio)
            {
                self.set_compact_mode(ShellMode::Full, ctx);
            }
            self.handle_work_action(action, ctx);
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if let Action::Push(action) = action {
            if matches!(
                action,
                PushAction::SelectImThread(_)
                    | PushAction::SelectImChannel(_)
                    | PushAction::SelectCategory(_)
                    | PushAction::SelectAppFilter(_)
                    | PushAction::SelectNotification(_)
            ) {
                self.motion_started = Instant::now();
            }
            self.handle_push_action(action, ctx);
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        if self.busy
            && self.google_progress.lock().is_ok_and(|p| !p.is_empty())
            && matches!(
                action,
                Action::CloseWindow
                    | Action::QuitApplication
                    | Action::Forget
                    | Action::Select(_)
                    | Action::SetPreferredDevice(_)
            )
        {
            self.status = "手机组件正在安装，请等待结果后再关闭或切换设备".into();
            ctx.notify();
            return;
        }
        if matches!(action, Action::AmooSelectSession(_) | Action::AmooNewChat) {
            self.compact.history = false;
        }
        if self.handle_ui_action(action, ctx) {
            self.sync_ime(ctx);
            ctx.notify();
            return;
        }
        self.error = false;
        if let Some(action) = self.resolve(action, ctx) {
            // Enter and refresh may resolve to a module action. Those actions
            // need the same dispatcher (and guards) as a button click; the
            // device-only handler silently ignores them.
            match action {
                Action::Workspace(_)
                | Action::Studio(_)
                | Action::AmooUi(_)
                | Action::AppStore(_) => {
                    self.handle_action(&action, ctx);
                }
                _ => self.handle_device_action(&action),
            }
        }
        self.sync_ime(ctx);
        ctx.notify();
    }
}

impl Companion {
    /// Actions that never touch the device. Returns `true` when handled.
    fn handle_ui_action(&mut self, action: &Action, ctx: &mut ViewContext<Self>) -> bool {
        if let Action::Speak(key, text) = action {
            self.speak(key, text);
            return true;
        }
        if self.handle_text_action(action, ctx) {
            return true;
        }
        match action {
            Action::Page(page) => {
                self.clear_text_interaction();
                if self.page != *page {
                    if self.page == Page::Work {
                        self.work.trim_idle_previews();
                    }
                    if self.page == Page::Gallery && !self.back_screen.gallery_lightbox {
                        self.back_screen.gallery_zoom_pct = 100;
                        self.work.trim_idle_previews();
                    }
                }
                if self.page != *page
                    || (*page == Page::Tools && self.tool_page.is_some())
                    || (*page == Page::AppStore && self.app_store.selected_package.is_some())
                {
                    self.trigger_subtab_switch();
                }
                if *page == Page::AppStore && self.page == Page::AppStore {
                    self.app_store.selected_package = None;
                    self.app_store.lightbox = None;
                }
                self.set_compact_mode(ShellMode::Full, ctx);
                if let Some(i) = self.focus {
                    self.fields[i].preedit.clear();
                }
                self.page = *page;
                if *page == Page::AppStore {
                    self.prefetch_current_store_page_icons();
                    if !self.demo_mode {
                        self.ensure_store_online_catalog_synced();
                    }
                }
                if *page == Page::Amoo {
                    self.demote_busy_if_background();
                }
                if *page == Page::Studio && !self.studio.loaded && !self.busy {
                    self.refresh_studio();
                }
                self.back_screen
                    .set_preview_visible(*page == Page::BackScreen);
                if *page == Page::Tools {
                    self.tool_page = None;
                }
                self.tool_confirmation = None;
                self.focus = None;
                self.scroll = Default::default();
                self.scroll_aux = Default::default();
                self.trash_confirm = false;
                self.tools_pending = None;
                if matches!(page, Page::Amoo | Page::Work) {
                    self.reveal_latest_chat();
                }
                if *page == Page::Sms {
                    Self::scroll_to_end(&self.scroll_aux);
                }
                self.load_work_previews();
                self.auto_load_for_page();
                if !self.demo_mode
                    && matches!(
                        self.page,
                        Page::Tools
                            | Page::Overview
                            | Page::Calendar
                            | Page::Schedule
                            | Page::Alarms
                    )
                    && !self.back_screen.stepos_suite.checked
                    && !self.back_screen.stepos_suite_loading
                    && self.selected.is_some()
                {
                    self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
                }
                if !self.demo_mode
                    && self.page == Page::Gallery
                    && !self.back_screen.gallery_checked
                    && !self.back_screen.gallery_loading
                    && self.selected.is_some()
                {
                    self.handle_backscreen_action(&BackScreenAction::GalleryRefresh, ctx);
                }
            }
            Action::OpenTool(tool) => {
                if self.page == Page::Work {
                    self.work.trim_idle_previews();
                }
                if self.page == Page::Gallery && !self.back_screen.gallery_lightbox {
                    self.back_screen.gallery_zoom_pct = 100;
                    self.work.trim_idle_previews();
                }
                self.trigger_subtab_switch();
                self.set_compact_mode(ShellMode::Full, ctx);
                self.page = Page::Tools;
                self.tool_page = *tool;
                self.tools_pending = None;
                self.tool_confirmation = None;
                self.focus = None;
                self.scroll = Default::default();
                if *tool == Some(ToolPage::Screen) && !self.demo_mode {
                    self.trigger_lan_cast_discovery();
                }
                self.auto_load_for_page();
            }
            Action::NeoIme(enabled) => {
                self.neo_enable_ime = *enabled;
            }
            Action::ToolCategory(category) => {
                self.trigger_subtab_switch();
                self.tool_category = *category;
            }
            Action::PrepareUtility(action) => {
                if !self.busy && self.selected.is_some() {
                    self.tool_confirmation = Some(action.clone());
                    self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
                }
            }
            Action::CancelUtility => self.tool_confirmation = None,
            Action::ConfirmUtility => {
                if !self.busy {
                    if let Some(action) = self.tool_confirmation.take() {
                        self.handle_device_action(&action);
                    }
                }
            }
            Action::ToolTab(tab) => {
                self.trigger_subtab_switch();
                self.tools_tab = *tab;
                self.tools_result_page = 0;
                self.tools_pending = None;
            }
            Action::ToolResultPage(page) => {
                self.trigger_subtab_switch();
                self.tools_result_page = *page;
                self.tools_pending = None;
            }
            Action::PrepareToolOperation(operation) => {
                if !self.busy {
                    self.tools_pending = Some(operation.clone());
                    self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
                }
            }
            Action::CancelToolOperation => self.tools_pending = None,
            Action::LocateToolFile(path) => {
                if self.busy {
                    return true;
                }
                if let Some((parent, name)) = path.rsplit_once('/') {
                    self.motion_started = Instant::now();
                    self.page = Page::Files;
                    self.fields[F_FILE_SEARCH].set(name);
                    self.tools_pending = None;
                    self.browse(parent.to_string());
                }
            }
            Action::CycleTheme => {
                self.theme_mode = self.theme_mode.next();
                let appearance = match self.theme_mode {
                    ThemeMode::System => stepx_companion::settings::Appearance::System,
                    ThemeMode::Light => stepx_companion::settings::Appearance::Light,
                    ThemeMode::Dark => stepx_companion::settings::Appearance::Dark,
                };
                if let Err(e) =
                    self.handle_settings_action(&SettingsAction::Appearance(appearance), ctx)
                {
                    self.status = e.to_string();
                    self.error = true;
                }
            }
            Action::AdbGuide(step) => {
                self.adb_guide_step = step.map(|step| step.min(2));
                self.focus = None;
                if step.is_none() {
                    self.preferences.adb_guide_seen = true;
                    if let Err(error) = self.preferences.save() {
                        self.status = format!("保存引导偏好失败：{error}");
                        self.error = true;
                    }
                }
            }
            Action::AdbGuideContinue => {
                if self.selected.is_some() {
                    self.handle_ui_action(&Action::AdbGuide(None), ctx);
                    self.handle_ui_action(&Action::ConnectGuide(Some(2)), ctx);
                }
            }
            Action::ConnectGuide(step) => {
                self.trigger_subtab_switch();
                self.connect_guide_step = step.map(|step| step.min(5));
                self.page = Page::Connect;
                self.focus = None;
                if matches!(step, Some(4)) {
                    self.handle_device_action(&Action::Qr);
                }
                self.auto_load_for_page();
            }
            Action::PairTab(tab) => {
                if self.pair_tab != *tab {
                    self.pair_tab = *tab;
                    self.trigger_subtab_switch();
                }
                match tab {
                    PairTab::Qr => self.focus = None,
                    PairTab::Code => self.focus_field(F_PAIR_CODE, ctx),
                    PairTab::Manual => self.focus_field(F_CONNECT, ctx),
                }
            }
            Action::ToggleShowSecret => self.show_secret = !self.show_secret,
            Action::RenameFile(index) => {
                self.selected_file = Some(*index);
                self.handle_ui_action(&Action::OpenRename, ctx);
            }
            Action::TrashFile(index) => {
                self.selected_file = Some(*index);
                self.handle_ui_action(&Action::Trash, ctx);
            }
            Action::SmsFilter(filter) => {
                if self.sms_filter != *filter {
                    self.trigger_subtab_switch();
                }
                self.sms_filter = *filter;
                self.selected_thread = None;
                self.fields[F_SMS_TO].clear();
                self.pending_sms_plan = None;
            }
            Action::AmooToggleToolRecords(key) => {
                if !self.amoo_expanded_tool_records.remove(key) {
                    self.amoo_expanded_tool_records.insert(key.clone());
                }
            }
            Action::PhoneCancelCall => {
                self.pending_call = None;
            }
            Action::PhoneSelectSim(id) => {
                self.selected_call_sim = Some(*id);
                self.pending_call = None;
            }
            Action::ToggleSystemApps(include) => {
                self.motion_started = Instant::now();
                self.include_system_apps = *include;
                self.scroll = Default::default();
            }
            Action::RequestUninstallApp(pkg) => {
                if self
                    .apps
                    .iter()
                    .any(|app| app.package == *pkg && app.can_uninstall())
                {
                    self.pending_app_uninstall = Some(pkg.clone());
                }
            }
            Action::CancelUninstallApp => self.pending_app_uninstall = None,
            Action::SelectSim(sim) => {
                self.selected_sim = *sim;
                self.pending_sms_plan = None;
            }
            Action::SelectSmsThread(thread) => {
                self.motion_started = Instant::now();
                self.push.im_channel = stepx_companion::push::ImChannel::Sms;
                self.push.selected_im_thread = Some(String::new());
                self.selected_thread = thread.clone();
                self.pending_sms_plan = None;
                self.scroll_aux = Default::default();
                Self::scroll_to_end(&self.scroll_aux);
                if let Some(addr) = thread {
                    self.fields[F_SMS_TO].set(addr);
                    self.focus_field(F_SMS_BODY, ctx);
                }
            }
            Action::NewSms => {
                self.motion_started = Instant::now();
                self.push.im_channel = stepx_companion::push::ImChannel::Sms;
                self.push.selected_im_thread = Some(String::new());
                self.selected_thread = Some("__new__".into());
                self.pending_sms_plan = None;
                self.fields[F_SMS_TO].clear();
                self.fields[F_SMS_BODY].clear();
                self.focus_field(F_SMS_TO, ctx);
                self.status = "新建短信：请在下方选择联系人或输入收件人号码".into();
            }
            Action::CancelDirectSms => {
                self.pending_sms_plan = None;
                self.status = "已取消发送".into();
            }
            Action::SetPreferredDevice(serial) => {
                self.preferences.selected_serial = Some(serial.clone());
                self.save_preferences("已设为默认设备");
            }
            Action::RemoveRememberedDevice(serial) => {
                self.preferences.devices.retain(|p| &p.serial != serial);
                if self.preferences.selected_serial.as_ref() == Some(serial) {
                    self.preferences.selected_serial =
                        self.preferences.devices.first().map(|d| d.serial.clone());
                }
                self.save_preferences("已移除");
            }
            Action::FileSort(sort) => {
                self.motion_started = Instant::now();
                self.file_sort = *sort;
                self.file_page = 0;
            }
            Action::OpenNewFolder => {
                if self.page != Page::Files || self.selected.is_none() {
                    return true;
                }
                self.file_modal = Some(FileModal::NewFolder);
                self.trash_confirm = false;
                self.fields[F_FILE_NAME].clear();
                self.focus_field(F_FILE_NAME, ctx);
            }
            Action::OpenNewTextFile => {
                if self.page != Page::Files || self.selected.is_none() {
                    return true;
                }
                self.file_modal = Some(FileModal::NewTextFile);
                self.trash_confirm = false;
                self.fields[F_FILE_NAME].set("未命名笔记.txt");
                self.focus_field(F_FILE_NAME, ctx);
                self.fields[F_FILE_NAME].all = true;
            }
            Action::OpenRename => {
                if self.page != Page::Files {
                    return true;
                }
                let sel = self
                    .selected_file
                    .or_else(|| self.visible_real_files().first().map(|(i, _)| *i));
                if let Some((idx, name)) =
                    sel.and_then(|i| self.files.get(i).map(|e| (i, e.name.clone())))
                {
                    self.selected_file = Some(idx);
                    self.file_modal = Some(FileModal::Rename(idx));
                    self.trash_confirm = false;
                    self.fields[F_FILE_NAME].set(&name);
                    self.focus_field(F_FILE_NAME, ctx);
                    self.fields[F_FILE_NAME].all = true;
                }
            }
            Action::CancelFileModal => {
                self.file_modal = None;
                self.fields[F_FILE_NAME].clear();
                if self.focus == Some(F_FILE_NAME) {
                    self.focus = None;
                }
            }
            Action::MoveFileSelection(delta) => {
                if self.page == Page::Gallery {
                    let cols = self.back_screen.gallery_grid_cols.clamp(2, 4) as isize;
                    if delta.is_negative() {
                        for _ in 0..cols.abs() {
                            self.handle_backscreen_action(&BackScreenAction::GalleryPrevPhoto, ctx);
                        }
                    } else {
                        for _ in 0..cols.abs() {
                            self.handle_backscreen_action(&BackScreenAction::GalleryNextPhoto, ctx);
                        }
                    }
                    return true;
                }
                if self.page != Page::Files {
                    return true;
                }
                let filtered: Vec<usize> = self
                    .visible_real_files()
                    .into_iter()
                    .map(|(i, _)| i)
                    .collect();
                if filtered.is_empty() {
                    return true;
                }
                let pos = self
                    .selected_file
                    .and_then(|sel| filtered.iter().position(|idx| *idx == sel));
                let next = match pos {
                    Some(pos) => {
                        (pos as isize + *delta).clamp(0, filtered.len() as isize - 1) as usize
                    }
                    None => 0,
                };
                self.selected_file = Some(filtered[next]);
                self.file_page = next / FILES_PER_PAGE;
                self.trash_confirm = false;
            }
            Action::MoveHorizontalSelection(delta) => {
                if self.page == Page::Gallery {
                    let act = if *delta < 0 {
                        BackScreenAction::GalleryPrevPhoto
                    } else {
                        BackScreenAction::GalleryNextPhoto
                    };
                    self.handle_backscreen_action(&act, ctx);
                } else if self.page == Page::Expansion {
                    let act = if *delta < 0 {
                        expansion_state::ExpansionAction::MediaPrev
                    } else {
                        expansion_state::ExpansionAction::MediaNext
                    };
                    self.handle_expansion_action(&act, ctx);
                }
            }
            Action::SelectFile(i) => {
                self.selected_file = Some(*i);
                self.trash_confirm = false;
            }
            Action::FilePage(page) => {
                self.motion_started = Instant::now();
                self.file_page = *page;
                self.selected_file = None;
                self.trash_confirm = false;
                self.scroll = Default::default();
            }
            Action::Trash | Action::DeleteFile => {
                if self.page == Page::Gallery && self.focus.is_none() {
                    if let Some(item) = self
                        .back_screen
                        .gallery_items
                        .get(self.back_screen.gallery_selected)
                        .cloned()
                    {
                        self.handle_backscreen_action(
                            &BackScreenAction::GalleryDelete(item.remote_path),
                            ctx,
                        );
                    }
                    return true;
                }
                if self.page != Page::Files {
                    return true;
                }
                let sel = self
                    .selected_file
                    .or_else(|| self.visible_real_files().first().map(|(i, _)| *i));
                if let Some(i) = sel {
                    self.selected_file = Some(i);
                    self.trash_confirm = true;
                    self.permanent_delete = matches!(action, Action::DeleteFile);
                    self.file_modal = None;
                } else {
                    self.status = "请先选择文件".into();
                }
            }
            Action::CancelTrash => self.trash_confirm = false,
            Action::Focus(i) => self.focus_field(*i, ctx),
            Action::ClearField(i) => {
                self.fields[*i].clear();
                if [F_SMS_TO, F_SMS_BODY].contains(i) {
                    self.pending_sms_plan = None;
                }
                self.focus_field(*i, ctx);
                if *i == F_FILE_SEARCH {
                    self.file_page = 0;
                }
                if *i == F_STORE_SEARCH {
                    self.app_store.page = 0;
                    self.app_store.search_results = None;
                    self.app_store.last_search_query.clear();
                    self.app_store.searching = false;
                    self.prefetch_current_store_page_icons();
                }
            }
            Action::FocusSearch => {
                let target = match self.page {
                    Page::Sms => Some(F_SMS_SEARCH),
                    Page::Notifications => Some(F_PUSH_SEARCH),
                    Page::Phone => Some(F_CONTACT_SEARCH),
                    Page::Files => Some(F_FILE_SEARCH),
                    Page::AppStore => Some(F_STORE_SEARCH),
                    Page::Apps => Some(F_APP_SEARCH),
                    Page::Amoo => Some(F_AMOO_SEARCH),
                    Page::Work => Some(if self.work.tab == 0 {
                        F_WORK_SEARCH
                    } else {
                        F_WORK_CAP_SEARCH
                    }),
                    Page::BackScreen => Some(F_BACK_PROMPT),
                    Page::Studio => Some(if self.studio.tab == 2 {
                        F_LAB_PROMPT
                    } else {
                        F_STUDIO_SEARCH
                    }),
                    Page::Overview => Some(F_OV_TEXT),
                    Page::Gallery => Some(F_GALLERY_SEARCH),
                    Page::Calendar | Page::Schedule => Some(F_PHONE_SCHEDULE_TITLE),
                    Page::Alarms => Some(F_ALARM_TIME),
                    Page::Connect
                    | Page::Tools
                    | Page::Account
                    | Page::About
                    | Page::Settings
                    | Page::Expansion => None,
                };
                if let Some(i) = target {
                    self.focus_field(i, ctx);
                }
            }
            Action::NextField(reverse) => {
                let order: &[usize] = match self.page {
                    Page::Connect => match self.pair_tab {
                        PairTab::Qr => &[],
                        PairTab::Code => &[F_PAIR_ADDR, F_PAIR_CODE],
                        PairTab::Manual => &[F_CONNECT],
                    },
                    Page::Overview => &[F_OV_TEXT],
                    Page::BackScreen => &[F_BACK_PROMPT, F_BACK_MARQUEE],
                    Page::Amoo => &[F_AMOO, F_AMOO_SEARCH],
                    Page::Work => &[
                        F_WORK_PROMPT,
                        F_WORK_SEARCH,
                        F_WORK_CAP_SEARCH,
                        F_WORK_AGENT_NAME,
                    ],
                    Page::Studio if self.studio.tab == 2 => &[F_LAB_PROMPT],
                    Page::Studio if self.studio.tab == 3 => {
                        &[F_LAB_ENDPOINT, F_LAB_KEY, F_LAB_MODEL]
                    }
                    Page::Studio => &[F_STUDIO_SEARCH, F_STUDIO_NAME, F_STUDIO_URL],
                    Page::Sms => &[F_SMS_SEARCH, F_IM_REPLY, F_SMS_TO, F_SMS_BODY],
                    Page::Notifications => &[F_PUSH_SEARCH, F_PUSH_REPLY],
                    Page::Phone => &[F_CONTACT_SEARCH, F_PHONE_DIAL],
                    Page::Files if self.file_modal.is_some() => &[F_FILE_NAME, F_FILE_SEARCH],
                    Page::Files => &[F_FILE_SEARCH],
                    Page::AppStore => &[F_STORE_SEARCH],
                    Page::Apps => &[F_APP_SEARCH],
                    Page::Calendar | Page::Schedule => &[F_PHONE_SCHEDULE_TITLE, F_CALENDAR_TIME],
                    Page::Alarms => &[F_ALARM_TIME, F_ALARM_LABEL],
                    Page::Gallery => &[F_GALLERY_SEARCH],
                    Page::Tools
                    | Page::Account
                    | Page::About
                    | Page::Settings
                    | Page::Expansion => &[],
                };
                if !order.is_empty() {
                    let next = match self.focus.and_then(|f| order.iter().position(|x| *x == f)) {
                        Some(pos) if *reverse => order[(pos + order.len() - 1) % order.len()],
                        Some(pos) => order[(pos + 1) % order.len()],
                        None => order[0],
                    };
                    self.focus_field(next, ctx);
                }
            }
            Action::Escape => {
                if self.adb_guide_step.is_some() {
                    self.handle_ui_action(&Action::AdbGuide(None), ctx);
                    return true;
                }
                self.tool_confirmation = None;
                self.tools_pending = None;
                if self.page == Page::AppStore {
                    if self.app_store.lightbox.is_some() {
                        self.app_store.lightbox = None;
                        return true;
                    }
                    if self.app_store.pending_uninstall.is_some() {
                        self.app_store.pending_uninstall = None;
                        return true;
                    }
                    if self.app_store.pending_downgrade_reinstall.is_some() {
                        self.app_store.pending_downgrade_reinstall = None;
                        return true;
                    }
                    if self.focus.is_none() && self.app_store.selected_package.is_some() {
                        self.app_store.selected_package = None;
                        self.trigger_subtab_switch();
                        return true;
                    }
                }
                if self.amoo_plus_open {
                    self.amoo_plus_open = false;
                } else if self.amoo_recording {
                    self.amoo_recording = false;
                } else if self.file_modal.is_some() {
                    self.file_modal = None;
                    if self.focus == Some(F_FILE_NAME) {
                        self.focus = None;
                    }
                } else if self.pending_sms_plan.is_some() {
                    self.pending_sms_plan = None;
                } else if self.trash_confirm {
                    self.trash_confirm = false;
                } else if self.focus.is_some() {
                    self.focus = None;
                } else {
                    self.selected_file = None;
                }
            }
            Action::MinimizeWindow => {
                ctx.minimize_window();
            }
            Action::MaximizeWindow => {
                ctx.toggle_maximized_window();
            }
            Action::CloseWindow | Action::QuitApplication => {
                let force_quit = matches!(action, Action::QuitApplication);
                if !force_quit && self.settings.value.close_to_tray && self.settings.tray.is_some()
                {
                    self.remember_window(ctx, true);
                    self.settings.hidden = true;
                    let id = ctx.window_id();
                    ctx.windows().hide_window(id);
                    return true;
                }
                if !force_quit && self.back_screen.companion_busy {
                    self.status = "正在同步 StepX伴侣，请完成后关闭窗口".into();
                    ctx.notify();
                    return true;
                }
                if let Some(job) = &self.updates.transfer {
                    job.cancel();
                }
                self.cancel_backscreen();
                self.stop_mirror();
                self.close_mirror_controls(ctx);
                self.workspace_flush();
                self.remember_window(ctx, true);
                self.settings.tray = None;
                ctx.close_window();
                if force_quit {
                    ctx.terminate_app(platform::TerminationMode::ForceTerminate, None);
                }
            }
            Action::OverviewBackPreview(back) => {
                self.overview_back_preview = *back;
                if *back && self.back_screen.frame.is_empty() {
                    self.back_screen.refresh_frame();
                }
                self.status = if *back {
                    "已切换为背屏图案预览".into()
                } else {
                    "已切换为正面主屏预览".into()
                };
            }
            Action::AmooSelectAgent(idx) => {
                self.motion_started = Instant::now();
                self.amoo_agent_idx = *idx;
                let name = match *idx {
                    0 => "Amoo 助手",
                    1 => "桌面伙伴",
                    2 => "飞书 / 企微",
                    _ => "Claude 协同",
                };
                if let Some(pos) = self.amoo_sessions.iter().position(|s| {
                    (*idx == 0 && (s.agent.contains("Amoo") || s.agent.contains("main")))
                        || (*idx == 1 && (s.agent.contains("深度") || s.agent.contains("桌面")))
                        || (*idx == 2 && (s.agent.contains("飞书") || s.agent.contains("Cron")))
                        || (*idx == 3
                            && (s.agent.contains("Claude") || s.agent.contains("OpenClaw")))
                }) {
                    self.amoo_session_idx = pos;
                }
                self.error = false;
                self.status = format!("已切换协同智能体：{name}");
            }
            Action::AmooSelectSession(idx) => {
                self.motion_started = Instant::now();
                self.amoo_ui.screen_mode = false;
                self.amoo_show_schedules = false;
                if let Some(session) = self.amoo_sessions.get(*idx) {
                    let key = session.session_key.clone();
                    if let Some(task_id) = self
                        .workspace
                        .journal
                        .tasks
                        .iter()
                        .find(|t| t.session.as_deref() == Some(key.as_str()))
                        .map(|t| t.id.clone())
                    {
                        self.workspace.active = Some(task_id);
                        self.workspace_restore();
                    }
                    self.clear_text_interaction();
                    self.compact.history = false;
                    self.amoo_session_idx = *idx;
                    self.load_work_previews();
                    self.amoo_thinking_expanded = false;
                    self.amoo_invoice_open = false;
                    self.reveal_latest_chat();
                    self.focus_field(F_AMOO, ctx);
                    if !self.demo_mode {
                        self.request_amoo_session_sync(Some(key));
                    }
                }
            }
            Action::AmooNewChat => {
                self.motion_started = Instant::now();
                self.amoo_ui.screen_mode = false;
                if self.busy {
                    return true;
                }
                if self.demo_mode {
                    self.amoo_sessions.insert(
                        0,
                        AmooSessionItem {
                            session_key: format!("demo:{}", self.amoo_sessions.len()),
                            title: "新对话（演示）".into(),
                            agent: "Amoo 助手".into(),
                            status: "待提问".into(),
                            status_kind: 0,
                            meta: "演示会话".into(),
                            items: Vec::new(),
                        },
                    );
                    self.amoo_session_idx = 0;
                } else {
                    self.with_device("正在后台创建 Amoo 会话…", |adb, d| {
                        Ok(Outcome::AmooCreated(
                            amoo::AgentBus::connect(adb, &d)?.create_session()?,
                        ))
                    });
                }
                self.focus_field(F_AMOO, ctx);
            }
            Action::AmooSelectCap(idx, tpl) => {
                self.amoo_cap_idx = *idx;
                self.fields[F_AMOO].set(tpl);
                self.focus_field(F_AMOO, ctx);
                self.status = "已填入场景指令模板，按 Enter 或点击发送即可执行".into();
            }
            Action::AmooToggleTts => {
                self.amoo_tts_on = !self.amoo_tts_on;
                if !self.amoo_tts_on {
                    self.speech.playback = None;
                    self.speech.key = None;
                }
                self.error = false;
                self.status = format!(
                    "Amoo 语音朗读已{}",
                    if self.amoo_tts_on { "开启" } else { "关闭" }
                );
            }
            Action::AmooToggleVoicePlay => {
                self.amoo_voice_playing = !self.amoo_voice_playing;
                self.status = if self.amoo_voice_playing {
                    "正在播放语音消息：“顺便发短信告诉李明我马上过去” (0:04\")".into()
                } else {
                    "语音播放已暂停".into()
                };
            }
            Action::AmooToggleThinking => {
                self.amoo_thinking_expanded = !self.amoo_thinking_expanded;
                self.status = if self.amoo_thinking_expanded {
                    "已展开 Amoo 深度思考与工具调用链路".into()
                } else {
                    "已收起深度思考详情".into()
                };
            }
            Action::AmooToggleInvoiceDetail => {
                self.amoo_invoice_open = !self.amoo_invoice_open;
                self.status = if self.amoo_invoice_open {
                    "已打开「10月报销单」明细（共 3 笔，合计 ¥386.00）".into()
                } else {
                    "已收起报销单明细".into()
                };
            }
            Action::AmooConfirmSafety(allow) => {
                self.amoo_safety_decision = if self.demo_mode && *allow {
                    SafetyDecision::Approved
                } else {
                    SafetyDecision::Rejected
                };
                if *allow {
                    self.page = Page::Sms;
                    self.selected_thread = Some("__new__".into());
                    self.pending_sms_plan = None;
                    self.fields[F_SMS_TO].clear();
                    self.fields[F_SMS_BODY].clear();
                    self.focus_field(F_SMS_TO, ctx);
                    self.status = "请填写实际收件人和短信正文，再确认发送".into();
                } else {
                    self.status = "已取消短信操作".into();
                }
            }

            Action::AmooRemoveAttachment(idx) => {
                if *idx < self.amoo_attachments.len() {
                    let removed = self.amoo_attachments.remove(*idx);
                    self.status = format!("已移除待发送附件：{}", removed.name);
                }
            }
            Action::AmooToggleMemory => {
                self.amoo_memory_enabled = !self.amoo_memory_enabled;
                self.trigger_subtab_switch();
                self.status = format!(
                    "Amoo 跨会话记忆已{}",
                    if self.amoo_memory_enabled {
                        "开启"
                    } else {
                        "关闭"
                    }
                );
            }
            Action::AmooCycleStyle => {
                self.amoo_style_idx = (self.amoo_style_idx + 1) % 3;
                let label = match self.amoo_style_idx {
                    0 => "直接精炼 · 不长篇解释",
                    1 => "温和详细 · 分步拆解",
                    _ => "极客专业 · 附带命令",
                };
                self.error = false;
                self.status = format!("已切换交流风格偏好：{label}");
            }
            Action::AmooCycleCommute => {
                self.amoo_commute_idx = (self.amoo_commute_idx + 1) % 3;
                let label = match self.amoo_commute_idx {
                    0 => "08:30 播报路况",
                    1 => "09:00 播报日程",
                    _ => "18:30 晚高峰提醒",
                };
                self.error = false;
                self.status = format!("已切换通勤偏好：{label}");
            }
            Action::AmooAddMemory => {
                let mem_input = self.fields[F_AMOO_MEMORY_INPUT].text.trim().to_string();
                let chat_input = self.fields[F_AMOO].text.trim().to_string();
                let new_title = if !mem_input.is_empty() {
                    self.fields[F_AMOO_MEMORY_INPUT].clear();
                    mem_input
                } else if !chat_input.is_empty() {
                    self.fields[F_AMOO].clear();
                    chat_input
                } else {
                    format!(
                        "背屏常显偏好：{} ({}级亮度)",
                        self.back_screen.title, self.back_screen.config.brightness
                    )
                };
                let cat_label = match self.workspace.memory_category {
                    1 => "关于我 · 个人画像",
                    2 => "交流偏好 · 称呼与语气",
                    3 => "备忘事项 · 重要记录",
                    _ => "关于我 · 个人记忆",
                };
                self.amoo_memories.insert(
                    0,
                    AmooMemoryEntry {
                        title: new_title.clone(),
                        sub: format!("{cat_label} · 桌面新增"),
                    },
                );
                self.amoo_bm25_entries = self.amoo_bm25_entries.max(self.amoo_memories.len());
                self.status = format!("已添加个人记忆：「{new_title}」");
            }
            Action::AmooUseMemory(mem) => {
                self.fields[F_AMOO].set(&format!("结合我的偏好「{mem}」，帮我安排："));
                self.focus_field(F_AMOO, ctx);
                self.status = format!("已引用记忆：{mem}");
            }
            Action::AmooToggleSkillManage => {
                self.amoo_skill_manage = !self.amoo_skill_manage;
                self.trigger_subtab_switch();
                self.status = if self.amoo_skill_manage {
                    "已进入技能管理模式：点击下方技能胶囊可启用或停用".into()
                } else {
                    "已保存技能启用配置".into()
                };
            }
            Action::AmooClickSkill(idx) => {
                if let Some((skill_key, name, prompt, new_enabled)) =
                    self.amoo_skills.get_mut(*idx).map(|skill| {
                        if self.amoo_skill_manage {
                            skill.enabled = !skill.enabled;
                        }
                        (
                            skill.skill_key.clone(),
                            skill.name.clone(),
                            skill.prompt.clone(),
                            skill.enabled,
                        )
                    })
                {
                    if self.amoo_skill_manage {
                        let state = if new_enabled {
                            "已启用"
                        } else {
                            "已停用"
                        };
                        self.status = format!("技能「{name}」{state}");
                        if !self.demo_mode && self.selected.is_some() {
                            self.with_device(
                                &format!("正在向手机 OpenClaw 网关同步技能「{name}」状态…"),
                                move |adb, d| {
                                    let real = openclaw::set_openclaw_skill_enabled(
                                        adb,
                                        &d,
                                        &skill_key,
                                        new_enabled,
                                    )?;
                                    Ok(Outcome::AmooRealData(
                                        real,
                                        format!("已同步技能「{name}」状态至手机 OpenClaw 网关"),
                                    ))
                                },
                            );
                        }
                    } else {
                        self.fields[F_AMOO].set(&prompt);
                        self.focus_field(F_AMOO, ctx);
                        self.status = format!("已调用「{name}」技能模板");
                    }
                }
            }
            Action::AmooCycleSafetyPolicy => {
                self.amoo_safety_policy_idx = (self.amoo_safety_policy_idx + 1) % 3;
                let label = match self.amoo_safety_policy_idx {
                    0 => "始终确认",
                    1 => "仅高危询问",
                    _ => "严格拦截",
                };
                self.status = format!("敏感操作守护策略已切换为：{label}");
            }
            Action::AmooUndoSafety => {
                if self.amoo_safety_decision != SafetyDecision::Pending {
                    self.amoo_safety_decision = SafetyDecision::Pending;
                    if let Some(sess) = self.amoo_sessions.get_mut(0) {
                        sess.status = "待确认".into();
                        sess.status_kind = 1;
                        sess.meta = "2 个产物 · 刚刚".into();
                    }
                    self.status = "已撤回上一次安全授权决定，恢复为「待确认」状态".into();
                } else if let Some(sess) = self.amoo_sessions.get_mut(self.amoo_session_idx) {
                    if sess.items.len() > 1 {
                        sess.items.pop();
                        self.status = "已撤回当前会话最后一条操作记录".into();
                    } else {
                        self.status = "当前没有可撤回的敏感操作".into();
                    }
                }
            }
            Action::AmooExportMemory => {
                let mut md = String::from("# Amoo 记忆与偏好导出\n\n");
                for m in &self.amoo_memories {
                    md.push_str(&format!("- **{}** ({})\n", m.title, m.sub));
                }
                let total_count = if self.demo_mode {
                    26 + self.amoo_memories.len()
                } else {
                    self.amoo_memories.len()
                };
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(md.clone()));
                if let Ok(Some(dest)) = native_dialog::FileDialog::new()
                    .set_title("导出 Amoo 记忆库")
                    .set_filename("StepX-Amoo-Memories.md")
                    .set_location(&stepx_companion::settings::downloads())
                    .show_save_single_file()
                {
                    let _ = std::fs::write(&dest, md);
                    self.status = format!("已导出 {total_count} 条记忆到 {}", dest.display());
                } else {
                    self.status = format!("已将全部记忆导出并复制到剪贴板（共 {total_count} 条）");
                }
            }
            Action::AmooToggleDetails => {
                self.amoo_show_details = !self.amoo_show_details;
                self.trigger_subtab_switch();
            }
            Action::AmooToggleSchedules => {
                self.trigger_subtab_switch();
                self.amoo_show_schedules = !self.amoo_show_schedules;
                self.amoo_ui.album_path = None;
                self.scroll = Default::default();
            }
            Action::AmooOpenScheduledSession(key) => {
                self.amoo_show_schedules = false;
                if let Some(index) = self
                    .amoo_sessions
                    .iter()
                    .position(|s| s.session_key == *key)
                {
                    self.handle_workspace(&WorkspaceAction::PhoneSession(index), ctx);
                }
            }
            Action::AmooTogglePlus => {
                self.amoo_plus_open = !self.amoo_plus_open;
            }
            Action::AmooToggleRecording => {
                self.amoo_recording = !self.amoo_recording;
                self.amoo_plus_open = false;
                if self.amoo_recording {
                    self.amoo_record_secs = 1;
                    self.status = "正在录音… 点击「发送语音」投递给 Amoo".into();
                } else {
                    self.status = "录音已结束".into();
                }
            }
            Action::AmooCancelRecording => {
                self.amoo_recording = false;
                self.status = "已取消语音录制".into();
            }
            Action::StudioOpenUrl(url) => {
                if stepx_companion::studio::parse_link(url)
                    .is_ok_and(|(_, kind)| kind == stepx_companion::studio::LinkKind::Project)
                {
                    let url = url.clone();
                    self.with_device("正在连接 Studio 工作窗口…", move |adb, d| {
                        let note = stepx_companion::studio::open_editor(adb, &d, &url)?;
                        Ok(Outcome::Studio(studio_state::StudioOutcome::Phone(
                            d.serial, note,
                        )))
                    });
                    return true;
                }

                let cmd = if cfg!(target_os = "macos") {
                    "open"
                } else if cfg!(windows) {
                    "explorer"
                } else {
                    "xdg-open"
                };
                match std::process::Command::new(cmd).arg(url).spawn() {
                    Ok(mut child) => {
                        std::thread::spawn(move || {
                            let _ = child.wait();
                        });
                        self.status = format!("已请求默认浏览器打开 {url}");
                    }
                    Err(error) => {
                        self.status = format!("无法打开浏览器：{error}");
                        self.error = true;
                    }
                }
            }
            Action::PhoneFilterMissed(missed) => {
                self.motion_started = Instant::now();
                self.call_filter_missed = *missed;
            }
            Action::PhoneSelectContact(idx) => {
                self.motion_started = Instant::now();
                self.pending_call = None;
                self.selected_contact = *idx;
                if let Some(c) = self.filtered_contacts().get(*idx) {
                    let num = c.number.clone();
                    self.fields[F_PHONE_DIAL].set(&num);
                }
            }
            Action::PhoneDialAppend(ch) => {
                self.pending_call = None;
                self.fields[F_PHONE_DIAL].insert(ch);
            }
            Action::PhoneDialBackspace => {
                self.pending_call = None;
                let input = &mut self.fields[F_PHONE_DIAL];
                let p = input.previous();
                input.text.replace_range(p..input.cursor, "");
                input.cursor = p;
            }
            Action::PhoneDialClear => {
                self.pending_call = None;
                self.fields[F_PHONE_DIAL].clear();
            }
            Action::PhoneDialNumber(number) => {
                self.motion_started = Instant::now();
                self.pending_call = None;
                self.fields[F_PHONE_DIAL].set(number);
                self.page = Page::Phone;
                self.focus_field(F_PHONE_DIAL, ctx);
                self.status = format!("已填入号码 {number}，可直接呼叫");
            }
            Action::PhoneSmsTo(number) => {
                self.motion_started = Instant::now();
                self.push.im_channel = stepx_companion::push::ImChannel::Sms;
                self.push.selected_im_thread = Some(String::new());
                self.pending_sms_plan = None;
                self.fields[F_SMS_TO].set(number);
                self.selected_thread = Some(number.clone());
                self.page = Page::Sms;
                self.focus_field(F_SMS_BODY, ctx);
                self.status = format!("正在向 {number} 编写短信");
            }
            Action::FillDetectedPhoneIp(ip) => {
                let endpoint = if ip.contains(':') {
                    ip.clone()
                } else {
                    format!("{ip}:5555")
                };
                self.fields[F_PAIR_ADDR].set(&endpoint);
                self.fields[F_CONNECT].set(&endpoint);
                self.status = format!("已填入检测到的手机局域网地址：{endpoint}");
                self.error = false;
            }
            Action::QuickGuiScenario { app, goal } => {
                self.motion_started = Instant::now();
                self.page = Page::Amoo;
                self.handle_amoo_ui_action(&AmooUiAction::GuiMode(true));
                self.fields[F_AMOO_GUI_APP].set(app);
                self.fields[F_AMOO].set(goal);
                if self.selected.is_some() && !self.amoo_ui.gui_running && !goal.trim().is_empty() {
                    self.send_amoo_draft();
                } else {
                    self.focus_field(F_AMOO, ctx);
                    self.status = format!("已填入手机操控指令「{goal}」，点击执行即可开始");
                }
            }
            Action::ScreenshotToBackScreen => {
                if let Some((_, path)) = &self.screenshot {
                    let path = path.clone();
                    if let Err(err) = self.back_screen.apply_image(path, "手机屏幕截图") {
                        self.error = true;
                        self.status = format!("截图转背屏点阵失败：{err:#}");
                    } else {
                        self.back_screen.tab = 2;
                        self.page = Page::BackScreen;
                        self.error = false;
                        self.status = "已将手机截图载入背屏点阵工坊".into();
                    }
                }
            }
            Action::RevealLocalPath(path) => match work_state::reveal_in_folder(path) {
                Ok(()) => {
                    self.error = false;
                    self.status = format!("已在文件夹中显示：{}", path.display());
                }
                Err(err) => {
                    self.error = true;
                    self.status = format!("无法打开所在文件夹：{err}");
                }
            },
            Action::SendToAmoo(prompt) => {
                self.set_compact_mode(ShellMode::Full, ctx);
                self.page = Page::Amoo;
                self.workspace.sub_view = 0;
                self.workspace.panel = 0;
                self.workspace.proposal = None;
                self.fields[F_AMOO].set(prompt);
                self.focus_field(F_AMOO, ctx);
                self.status = "已将内容带入 Amoo 对话，按 Enter 即可发送".into();
                self.error = false;
            }
            Action::EditUserMessage(text) => {
                self.fields[F_AMOO].set(text);
                self.focus_field(F_AMOO, ctx);
                self.status = "已将消息填入输入框，可修改后重新发送".into();
                self.error = false;
            }
            Action::Insert(_)
            | Action::Preedit(_)
            | Action::Backspace
            | Action::BackspaceLine
            | Action::Delete
            | Action::Left
            | Action::Right
            | Action::Home
            | Action::End
            | Action::SelectAll
            | Action::UndoInput
            | Action::RedoInput
            | Action::CopyInput
            | Action::CutInput
            | Action::Paste => self.edit_focused(action, ctx),
            Action::Copy => {
                if let Some(text) = self
                    .selected_text
                    .clone()
                    .filter(|s| !s.is_empty())
                    .or_else(|| (!self.content.is_empty()).then(|| self.content.clone()))
                {
                    ctx.clipboard().write(ClipboardContent::plain_text(text));
                    self.status = "已复制".into();
                }
            }
            Action::ClearContent => self.content.clear(),
            Action::CopyText(text, msg) => {
                ctx.clipboard()
                    .write(ClipboardContent::plain_text(text.clone()));
                self.status = msg.clone();
                self.error = false;
            }
            Action::AmooTemplate(tpl) => {
                self.amoo_show_schedules = false;
                self.fields[F_AMOO].set(tpl);
                self.focus_field(F_AMOO, ctx);
            }
            Action::AppendVisibleLine(line) => {
                let field = &mut self.fields[F_AMOO];
                if !field.text.is_empty() && !field.text.ends_with([' ', '\n']) {
                    field.cursor = field.text.len();
                    field.insert(" ");
                }
                field.cursor = field.text.len();
                field.insert(line);
                self.focus_field(F_AMOO, ctx);
            }
            Action::StopMirror => {
                self.stop_mirror();
                self.status = "投屏已结束".into();
                self.error = false;
                self.tools_feedback = Some((Some(ToolPage::Screen), self.status.clone(), false));
            }
            Action::ToggleLiveScreenStream => {
                self.live_screen_stream = !self.live_screen_stream;
                self.last_live_screen_poll = Instant::now() - Duration::from_secs(10);
                self.status = if self.live_screen_stream {
                    "已开启内嵌屏幕实时连拍同步（可在桌面直接点按屏幕操控手机）".into()
                } else {
                    "已暂停内嵌屏幕实时连拍同步".into()
                };
                self.error = false;
            }
            Action::SetMirrorQuality(q) => {
                self.mirror_options.quality = (*q).min(2);
                let label = match self.mirror_options.quality {
                    0 => "720P 流畅 (30帧 · 4Mbps)",
                    1 => "1080P 高清 (60帧 · 8Mbps)",
                    _ => "原画超清 (120帧 · 16Mbps)",
                };
                self.restart_active_mirror(format!("已切换独立投屏画质档位：{label}"));
            }
            Action::SetMirrorAudioRoute(route) => {
                self.mirror_options.audio_route = *route;
                self.restart_active_mirror(format!("投屏声音已切换为「{}」", route.label()));
            }
            Action::CycleMirrorAudioRoute => {
                let next = self.mirror_options.audio_route.next();
                self.mirror_options.audio_route = next;
                self.restart_active_mirror(format!("投屏声音已切换为「{}」", next.label()));
            }
            Action::ToggleMirrorAlwaysOnTop => {
                self.mirror_options.always_on_top = !self.mirror_options.always_on_top;
                let msg = format!(
                    "投屏窗口始终置顶：{}",
                    if self.mirror_options.always_on_top {
                        "已开启"
                    } else {
                        "已关闭"
                    }
                );
                self.restart_active_mirror(msg);
            }
            Action::ToggleMirrorTurnScreenOff => {
                self.mirror_options.turn_screen_off = !self.mirror_options.turn_screen_off;
                let msg = format!(
                    "投屏时手机息屏省电：{}",
                    if self.mirror_options.turn_screen_off {
                        "已开启"
                    } else {
                        "已关闭"
                    }
                );
                self.restart_active_mirror(msg);
            }
            Action::ToggleMirrorStayAwake => {
                self.mirror_options.stay_awake = !self.mirror_options.stay_awake;
                let msg = format!(
                    "投屏期间保持手机常亮：{}",
                    if self.mirror_options.stay_awake {
                        "已开启"
                    } else {
                        "已关闭"
                    }
                );
                self.restart_active_mirror(msg);
            }
            Action::ToggleMirrorShowTouches => {
                self.mirror_options.show_touches = !self.mirror_options.show_touches;
                let msg = format!(
                    "投屏显示触控点轨迹：{}",
                    if self.mirror_options.show_touches {
                        "已开启"
                    } else {
                        "已关闭"
                    }
                );
                self.restart_active_mirror(msg);
            }
            Action::LanCast(act) => {
                self.handle_lan_cast_action(act, ctx);
            }
            Action::SaveScreenshot => self.save_screenshot(),
            Action::AmooSelectModel(idx) => {
                self.amoo_model_idx = *idx;
                let model_name = self.active_amoo_model_name();
                self.status = format!("已切换 Amoo 推理模型：{model_name}");
                self.error = false;
            }
            Action::AmooToggleLightbox => {
                self.amoo_lightbox_open = !self.amoo_lightbox_open;
                self.status = if self.amoo_lightbox_open {
                    "已打开高清原图放大预览（支持 100%/150% 缩放、保存原图与转换背屏点阵）".into()
                } else {
                    "已关闭图片放大预览".into()
                };
            }
            Action::AmooToggleLightboxZoom => {
                self.amoo_lightbox_zoom = if self.amoo_lightbox_zoom == 100 {
                    150
                } else {
                    100
                };
                self.status = format!("图片预览缩放比例：{}%", self.amoo_lightbox_zoom);
            }
            Action::AmooSaveLightboxImage => {
                let src =
                    stepx_companion::resources::root().join("desktop/assets/amoo_tti_portrait.png");
                let default_dest = std::env::temp_dir().join("StepX_Amoo_Portrait_HD.png");
                if src.is_file() {
                    let _ = std::fs::copy(&src, &default_dest);
                }
                self.status = format!("高清原图已保存至 {}", default_dest.display());
            }
            Action::AmooToggleExpressCard => {
                self.amoo_express_open = !self.amoo_express_open;
                self.error = false;
                self.status = if self.amoo_express_open {
                    "已展开 PC 端快递速查与取件码卡片".into()
                } else {
                    "已收起快递速查卡片".into()
                };
            }
            Action::AmooQueryExpress => {
                self.amoo_express_open = true;
                self.error = false;
                if let Some(pos) = self
                    .amoo_sessions
                    .iter()
                    .position(|s| s.title.contains("快递"))
                {
                    self.amoo_session_idx = pos;
                }
                if self.demo_mode {
                    self.status =
                        "已从手机 Amoo「快递物流查询」与短信提取 2 件待取包裹（取件码 4-2-1086 / 顺丰 SF14298019283）"
                            .into();
                } else {
                    let codes: Vec<String> = self
                        .sms
                        .iter()
                        .filter_map(|m| pages::sms::extract_pickup_code(&m.body))
                        .take(3)
                        .collect();
                    if !codes.is_empty() {
                        self.status = format!(
                            "已从手机短信提取 {} 件待取包裹（取件码 {}）",
                            codes.len(),
                            codes.join(" / ")
                        );
                    } else {
                        self.fields[F_AMOO].set("请帮我汇总最近的快递物流与驿站取件码");
                        self.focus_field(F_AMOO, ctx);
                        self.status =
                            "已填入快递查询指令，按 Enter 即可向 Amoo 发起物流与取件码汇总".into();
                    }
                }
            }
            Action::AmooTogglePcControl => {
                self.amoo_pc_control_enabled = !self.amoo_pc_control_enabled;
                self.status = if self.amoo_pc_control_enabled {
                    "已开启「手机 Amoo 控制电脑」双向桥接通道（支持手机语音/对话截屏、同步剪贴板、静音与锁屏）"
                        .into()
                } else {
                    "已暂停「手机 Amoo 控制电脑」远控通道".into()
                };
            }
            Action::AmooToggleBgKeepAlive => {
                self.amoo_bg_keepalive = !self.amoo_bg_keepalive;
                if self.amoo_bg_keepalive && self.selected.is_some() {
                    return false; // Fall through to handle_device_action to grant deep perms
                }
                self.status = if self.amoo_bg_keepalive {
                    "已开启 Amoo 后台常驻防杀守护 (oom_score_adj = -800)".into()
                } else {
                    "已关闭 Amoo 后台强制保活".into()
                };
            }
            _ => return false,
        }
        true
    }

    /// Turns context-dependent actions (Enter, refresh, activate) into concrete ones.
    fn resolve(&mut self, action: &Action, ctx: &mut ViewContext<Self>) -> Option<Action> {
        Some(match action {
            Action::SubmitFocused
                if self
                    .focus
                    .is_some_and(|i| !self.fields[i].preedit.is_empty()) =>
            {
                return None
            }
            Action::SubmitFocused => match self.focus {
                Some(F_PAIR_ADDR) | Some(F_PAIR_CODE) => Action::PairCode,
                Some(F_CONNECT) => Action::Connect,
                Some(F_SMS_TO) => {
                    self.focus_field(F_SMS_BODY, ctx);
                    return None;
                }
                Some(F_SMS_BODY) => {
                    if self.pending_sms_plan.is_some() {
                        Action::ConfirmDirectSms
                    } else {
                        Action::PrepareDirectSms
                    }
                }
                Some(F_AMOO) if !self.amoo_ui.screen_mode => {
                    Action::Workspace(WorkspaceAction::Prepare)
                }
                Some(F_AMOO) => Action::AmooSend,
                Some(F_WORK_PROMPT) => {
                    self.handle_work_action(&WorkAction::Send, ctx);
                    return None;
                }
                Some(F_FILE_NAME) => Action::SubmitFileModal,
                Some(F_OV_TEXT) => Action::SendFocusText,
                Some(F_BACK_PROMPT) => {
                    self.handle_backscreen_action(&BackScreenAction::Generate, ctx);
                    return None;
                }
                Some(F_BACK_MARQUEE) => {
                    self.handle_backscreen_action(&BackScreenAction::SaveMarquee, ctx);
                    return None;
                }
                Some(F_LAB_PROMPT) => {
                    self.handle_studio_lab_action(&StudioLabAction::Start, ctx);
                    return None;
                }
                Some(F_LAB_ENDPOINT | F_LAB_KEY | F_LAB_MODEL) => {
                    self.handle_studio_lab_action(&StudioLabAction::Save, ctx);
                    return None;
                }
                Some(F_STUDIO_NAME) | Some(F_STUDIO_URL) => Action::Studio(StudioAction::Save),
                Some(F_STORE_SEARCH) => Action::AppStore(AppStoreAction::SearchSubmit),
                Some(F_PHONE_DIAL) => Action::PhoneCall(self.fields[F_PHONE_DIAL].text.clone()),
                Some(F_CALENDAR_TIME) => {
                    self.handle_backscreen_action(
                        &BackScreenAction::CalendarCreateOnSelectedDate(None),
                        ctx,
                    );
                    return None;
                }
                Some(F_ALARM_TIME) | Some(F_ALARM_LABEL) => {
                    self.handle_backscreen_action(&BackScreenAction::AlarmCreateCustom, ctx);
                    return None;
                }
                Some(F_PHONE_SCHEDULE_TITLE) => {
                    let act = if self.page == Page::Alarms {
                        BackScreenAction::AlarmCreateCustom
                    } else {
                        BackScreenAction::CalendarCreateOnSelectedDate(None)
                    };
                    self.handle_backscreen_action(&act, ctx);
                    return None;
                }
                Some(F_IM_REPLY) => {
                    self.handle_push_action(&PushAction::SendImReply, ctx);
                    return None;
                }
                Some(F_TOAST_REPLY) => {
                    if let Some(first) = self.push.active_toasts.first() {
                        let id = first.id.clone();
                        self.handle_push_action(&PushAction::ToastSendReply(id), ctx);
                    }
                    return None;
                }
                Some(F_PUSH_REPLY) => {
                    let target = if self.push.shell_mode == ShellMode::MiniNotify {
                        self.push.expanded_mini_card.clone().and_then(|id| {
                            self.push
                                .snapshot
                                .notifications
                                .iter()
                                .find(|n| n.key == id || n.thread_id == id)
                                .map(|n| {
                                    (
                                        n.key.clone(),
                                        n.thread_id.clone(),
                                        n.package_name.clone(),
                                        n.title.clone(),
                                    )
                                })
                                .or_else(|| {
                                    self.push
                                        .snapshot
                                        .threads
                                        .iter()
                                        .find(|t| t.thread_id == id)
                                        .map(|t| {
                                            (
                                                t.notification_key.clone(),
                                                t.thread_id.clone(),
                                                t.package_name.clone(),
                                                t.title.clone(),
                                            )
                                        })
                                })
                        })
                    } else {
                        self.push
                            .selected_key
                            .clone()
                            .or_else(|| self.push.replying_key.clone())
                            .and_then(|k| {
                                self.push
                                    .snapshot
                                    .notifications
                                    .iter()
                                    .find(|n| n.key == k)
                                    .map(|n| {
                                        (
                                            n.key.clone(),
                                            n.thread_id.clone(),
                                            n.package_name.clone(),
                                            n.title.clone(),
                                        )
                                    })
                            })
                    };
                    if let Some((key, thread_id, package_name, title)) = target {
                        self.handle_push_action(
                            &PushAction::ReplyInline {
                                key,
                                thread_id,
                                package_name,
                                title,
                            },
                            ctx,
                        );
                    } else {
                        self.handle_push_action(&PushAction::SendImReply, ctx);
                    }
                    return None;
                }
                _ => return None,
            },
            Action::ContextRefresh => match self.page {
                Page::About | Page::Settings | Page::Expansion => return None,
                Page::Overview if self.selected.is_some() => Action::Summary,
                Page::Overview | Page::Connect => Action::Refresh,
                Page::BackScreen => {
                    self.status = "背屏预览已刷新".into();
                    ctx.notify();
                    return None;
                }
                Page::Account => Action::AmooUi(AmooUiAction::RefreshUsage),
                Page::Amoo => Action::AmooSnapshot,
                Page::Calendar | Page::Schedule | Page::Alarms => {
                    self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
                    return None;
                }
                Page::Gallery => {
                    self.handle_backscreen_action(&BackScreenAction::GalleryRefresh, ctx);
                    return None;
                }
                Page::Work => {
                    self.refresh_work();
                    return None;
                }
                Page::Studio => Action::Studio(StudioAction::Refresh),
                Page::Sms => Action::ReadSms,
                Page::Notifications => {
                    self.handle_push_action(&PushAction::Refresh, ctx);
                    return None;
                }
                Page::Phone => Action::ReadPhone,
                Page::Files => Action::Browse(self.directory.clone()),
                Page::AppStore => Action::AppStore(AppStoreAction::RefreshCatalog),
                Page::Apps => Action::Apps,
                Page::Tools => match self.tool_page {
                    Some(ToolPage::Updates) => {
                        self.handle_update_action(&UpdateAction::CheckOfficial, ctx);
                        return None;
                    }
                    Some(ToolPage::Storage) => Action::ScanStorage,
                    Some(ToolPage::Google) => Action::GoogleSuiteInspect,
                    Some(ToolPage::Companion) => Action::CompanionInspect,
                    Some(ToolPage::Neo) => Action::NeoInspect,
                    Some(ToolPage::Services) => Action::InspectServices,
                    Some(ToolPage::Screen) => Action::Screenshot,
                    Some(ToolPage::Device) => Action::Summary,
                    Some(
                        ToolPage::Clipboard
                        | ToolPage::McpHub
                        | ToolPage::AiSafety
                        | ToolPage::Wellbeing,
                    ) => {
                        self.handle_backscreen_action(&BackScreenAction::InspectStepOsSuite, ctx);
                        return None;
                    }
                    _ => return None,
                },
            },
            Action::ActivateSelectedFile => {
                if self.focus.is_some() {
                    return None;
                }
                if self.page == Page::Gallery {
                    if let Some(item) = self
                        .back_screen
                        .gallery_items
                        .get(self.back_screen.gallery_selected)
                        .cloned()
                    {
                        self.handle_backscreen_action(
                            &BackScreenAction::GalleryOpenOnDesktop(item.remote_path),
                            ctx,
                        );
                    }
                    return None;
                }
                if self.page != Page::Files {
                    return None;
                }
                let sel = self
                    .selected_file
                    .or_else(|| self.visible_real_files().first().map(|(i, _)| *i));
                return sel.and_then(|i| self.activate_file(i));
            }
            Action::ActivateFile(i) => return self.activate_file(*i),
            Action::DownloadFile(i) => {
                self.selected_file = Some(*i);
                Action::Download
            }
            Action::ParentDirectory => {
                if self.page != Page::Files {
                    return None;
                }
                let parent = parent_dir(&self.directory);
                if parent == self.directory {
                    return None;
                }
                Action::Browse(parent)
            }
            other => other.clone(),
        })
    }
}
