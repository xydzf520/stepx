//! Shared review sheet: edits stay local; only the primary button commits effects.
use super::{theme::*, widgets::*, *};
use facloud_ui::TypedActionView;
#[derive(Default)]
pub(super) struct InteractionUi {
    pub storage_status: Option<String>,
    pub storage_rx: Option<std::sync::mpsc::Receiver<Result<String,String>>>,
    storage_checked: Option<Instant>,
    pub review: Option<Review>,
    pub approved: bool,
    pub history_open: bool,
    pub notices: Vec<(String, String, String)>,
    pub diagnostics: Vec<stepx_companion::diagnostics::Entry>,
    diagnostic_writer: Option<stepx_companion::diagnostics::Writer>,
    diagnostics_loaded: bool,
    diagnostics_error: Option<String>,
    history_entries: Vec<stepx_companion::diagnostics::Entry>,
    history_page: usize,
    history_total: usize,
    history_rx: Option<std::sync::mpsc::Receiver<Result<stepx_companion::diagnostics::Page, String>>>,
    list_pages: std::collections::BTreeMap<String, usize>,
    history_feedback: Option<String>,
    pub active_operation: Option<OperationFailure>,
    failures: std::collections::HashMap<String, OperationFailure>,
}

#[derive(Clone, Debug)]
pub(super) enum ReadRetry {
    Scan,
    Summary,
    Messages,
    Calls,
    Apps,
    Directory(String),
}
#[derive(Clone, Debug)]
pub(super) struct OperationFailure {
    pub label: String,
    pub page: Page,
    pub serial: Option<String>,
    pub retry: Option<ReadRetry>,
}
impl std::fmt::Display for OperationFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}
impl std::error::Error for OperationFailure {}
impl OperationFailure {
    pub fn for_action(action: &Action, page: Page, serial: Option<String>) -> Self {
        let retry = match action {
            Action::Refresh => Some(ReadRetry::Scan),
            Action::Summary => Some(ReadRetry::Summary),
            Action::ReadSms => Some(ReadRetry::Messages),
            Action::ReadPhone => Some(ReadRetry::Calls),
            Action::Apps => Some(ReadRetry::Apps),
            Action::Browse(path) => Some(ReadRetry::Directory(path.clone())),
            _ => None,
        };
        Self {
            label: String::new(),
            page,
            serial,
            retry,
        }
    }
    fn action(&self) -> Option<Action> {
        Some(match self.retry.as_ref()? {
            ReadRetry::Scan => Action::Refresh,
            ReadRetry::Summary => Action::Summary,
            ReadRetry::Messages => Action::ReadSms,
            ReadRetry::Calls => Action::ReadPhone,
            ReadRetry::Apps => Action::Apps,
            ReadRetry::Directory(path) => Action::Browse(path.clone()),
        })
    }
}
pub(super) struct Review {
    pub title: String,
    pub target: String,
    pub details: Vec<String>,
    pub accept: String,
    pub action: Action,
    pub serial: Option<String>,
    pub discard: bool,
    pub context: String,
}
impl Companion {
    fn start_storage_maintenance(&mut self,manual:bool){
        if self.interaction.storage_rx.is_some() || self.demo_mode || cfg!(test){return;}
        let (tx,rx)=std::sync::mpsc::channel();self.interaction.storage_rx=Some(rx);self.interaction.storage_checked=Some(Instant::now());
        std::thread::spawn(move ||{let _=tx.send(stepx_companion::storage_maintenance::run(manual).map_err(|e|format!("{e:#}")));});
    }
    pub(super) fn poll_storage_maintenance(&mut self,ctx:&mut ViewContext<Self>){
        if let Some(result) = self.interaction.history_rx.as_ref().and_then(|rx|rx.try_recv().ok()) {
            self.interaction.history_rx = None;
            match result {
                Ok(mut page) => {
                    if self.interaction.history_page == 0 {
                        for entry in &self.interaction.diagnostics {
                            if !page.entries.iter().any(|stored| stored.id == entry.id) { page.entries.push(entry.clone()); }
                        }
                        stepx_companion::diagnostics::retain(&mut page.entries, chrono::Utc::now().timestamp_millis());
                    }
                    page.entries.truncate(30);
                    self.interaction.history_total = page.total.max(self.interaction.diagnostics.len());
                    self.interaction.history_entries = page.entries;
                    self.interaction.diagnostics_error = None;
                }
                Err(error) => self.interaction.diagnostics_error = Some(error),
            }
            ctx.notify();
        }
        if self.interaction.storage_checked.is_none_or(|t|t.elapsed()>Duration::from_secs(3600)) {self.start_storage_maintenance(false);}
        if let Some(result)=self.interaction.storage_rx.as_ref().and_then(|r|r.try_recv().ok()) {
            self.interaction.storage_rx=None;
            self.interaction.storage_status=Some(match result{Ok(s)=>s,Err(e)=>{self.record_notice("数据维护未完成",e.clone());format!("维护未完成：{e}。可重试，原数据保留。")}});
            ctx.notify();
        }
    }

    fn ensure_diagnostics_loaded(&mut self) {
        if self.interaction.diagnostics_loaded {
            return;
        }
        self.interaction.diagnostics_loaded = true;
        if cfg!(test) || self.demo_mode {
            return;
        }
        let path = stepx_companion::diagnostics::path();
        self.interaction.diagnostic_writer = Some(stepx_companion::diagnostics::Writer::start(path));
    }
    fn load_diagnostics_page(&mut self, page: usize) {
        if self.interaction.history_rx.is_some() { return; }
        self.interaction.history_page = page;
        self.interaction.history_entries.clear();
        if cfg!(test) || self.demo_mode {
            self.interaction.history_total = self.interaction.diagnostics.len();
            self.interaction.history_entries = self.interaction.diagnostics.iter().skip(page.saturating_mul(30)).take(30).cloned().collect();
            return;
        }
        let (tx,rx) = std::sync::mpsc::channel();
        self.interaction.history_rx = Some(rx);
        std::thread::spawn(move || {
            let result = stepx_companion::diagnostics::load_page(&stepx_companion::diagnostics::path(), page.saturating_mul(30), 30).map_err(|error|format!("{error:#}"));
            let _ = tx.send(result);
        });
    }
    /// Bound widget construction before formatting rows. Filters are part of the
    /// caller's key, so changing a search starts at the first matching page.
    pub(super) fn list_page_range(&self, key: &str, total: usize, size: usize) -> std::ops::Range<usize> {
        let size = size.max(1);
        let last = total.saturating_sub(1) / size;
        let page = self.interaction.list_pages.get(key).copied().unwrap_or(0).min(last);
        let begin = page.saturating_mul(size).min(total);
        begin..begin.saturating_add(size).min(total)
    }
    pub(super) fn reset_list_page(&mut self, key: &str) {
        self.interaction.list_pages.remove(key);
    }

    fn reset_paged_list_scroll(&mut self, key: &str) {
        // Paging a floating or embedded list must not move the page underneath
        // it, nor discard the reading position of unrelated conversation panes.
        if key.starts_with("notification-drawer:") {
            self.notification_center.list_scroll = Default::default();
        } else if key.starts_with("message-threads:") {
            self.sms_threads_scroll = Default::default();
        } else if key.starts_with("sms-history:") {
            self.sms_messages_scroll = Default::default();
        } else if self.page == Page::Home {
            // Home module pagers replace a bounded section in place.
        } else if self.page == Page::Notifications {
            self.push_list_scroll = Default::default();
        } else {
            self.scroll = Default::default();
            self.scroll_aux = Default::default();
        }
    }
    pub(super) fn list_page_controls(&self, key: &str, total: usize, size: usize) -> El {
        if total <= size.max(1) { return Empty::new().finish(); }
        let range = self.list_page_range(key, total, size);
        let page = range.start / size.max(1);
        self.between(self.text(format!("{}–{} / {total}",range.start+1,range.end),CAPTION,self.p().text2), self.hstack(S2,vec![
            if page > 0 { self.button(format!("{key}-previous"),"上一页",Action::ListPage(key.into(),page-1),Btn::Ghost) } else { self.disabled_button("上一页") },
            if range.end < total { self.button(format!("{key}-next"),"下一页",Action::ListPage(key.into(),page+1),Btn::Ghost) } else { self.disabled_button("下一页") },
        ]))
    }
    pub(super) fn record_operation_error(&mut self, error: &anyhow::Error) {
        let context = error.downcast_ref::<OperationFailure>().cloned();
        self.record_notice("操作未完成", format!("{error:#}"));
        self.attach_operation_failure(context);
    }
    pub(super) fn attach_operation_failure(&mut self, context: Option<OperationFailure>) {
        if let (Some(context), Some(entry)) = (context, self.interaction.diagnostics.first()) {
            self.interaction.failures.insert(entry.id.clone(), context);
        }
    }
    pub(super) fn record_notice(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        self.ensure_diagnostics_loaded();
        let title = title.into();
        let detail = detail.into();
        let entry = stepx_companion::diagnostics::Entry::new(&title, &detail);
        if let Some(writer) = &self.interaction.diagnostic_writer {
            writer.send(entry.clone());
        }
        if self.interaction.history_open && self.interaction.history_page == 0 {
            self.interaction.history_entries.insert(0, entry.clone());
            self.interaction.history_entries.truncate(30);
        }
        self.interaction.diagnostics.insert(0, entry);
        stepx_companion::diagnostics::retain(
            &mut self.interaction.diagnostics,
            chrono::Utc::now().timestamp_millis(),
        );
        self.interaction
            .failures
            .retain(|id, _| self.interaction.diagnostics.iter().any(|e| &e.id == id));
        self.interaction.notices.insert(
            0,
            (
                chrono::Local::now().format("%H:%M:%S").to_string(),
                title,
                detail,
            ),
        );
        self.interaction.notices.truncate(30);
    }
    pub(super) fn review_action(
        &mut self,
        action: &Action,
        title: &str,
        target: String,
        details: Vec<String>,
        accept: &str,
        phone: bool,
        discard: bool,
    ) {
        self.interaction.history_open = false;
        self.interaction.review = Some(Review {
            title: title.into(),
            target,
            details,
            accept: accept.into(),
            context: self.review_context(action),
            action: action.clone(),
            serial: if phone {
                self.selected.as_ref().map(|d| d.serial.clone())
            } else {
                None
            },
            discard,
        });
    }
    pub(super) fn interaction_gate(
        &mut self,
        action: &Action,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if matches!(action, Action::QuitApplication) {
            self.cancel_review_state();
            self.interaction.review = None;
            self.interaction.history_open = false;
            return false;
        }
        if self.interaction.approved {
            self.interaction.approved = false;
            return false;
        }
        match action {
            Action::RetryOperation(id) => {
                if self.busy || self.bulk_busy {
                    self.interaction.history_feedback =
                        Some("仍有操作进行中，请完成后再重试".into());
                    ctx.notify();
                    return true;
                }
                if let Some(failure) = self.interaction.failures.get(id).cloned() {
                    if failure.retry.is_some()
                        && failure.serial.is_some()
                        && failure.serial != self.selected.as_ref().map(|d| d.serial.clone())
                    {
                        self.interaction.history_feedback =
                            Some("请连接原操作的手机后再重试".into());
                        ctx.notify();
                        return true;
                    }
                    self.interaction.history_open = false;
                    if let Some(action) = failure.action() {
                        self.handle_action(&action, ctx);
                    } else {
                        self.handle_action(&Action::Page(failure.page), ctx);
                    }
                }
                ctx.notify();
                return true;
            }
            Action::CopyDiagnostic(id) => {
                if let Some(entry) = self.interaction.diagnostics.iter().chain(self.interaction.history_entries.iter()).find(|e| &e.id == id) {
                    use facloud_ui::clipboard::ClipboardContent;
                    ctx.clipboard().write(ClipboardContent::plain_text(format!(
                        "{}\n{}",
                        entry.title, entry.detail
                    )));
                }
                return true;
            }
            Action::ReviewCancel => {
                self.cancel_review_state();
                self.interaction.review = None;
                self.interaction.history_open = false;
                ctx.notify();
                return true;
            }
            Action::ReviewConfirm => {
                if let Some(r) = self.interaction.review.take() {
                    if (r.serial.is_some()
                        && r.serial != self.selected.as_ref().map(|d| d.serial.clone()))
                        || r.context != self.review_context(&r.action)
                    {
                        self.record_notice("未执行", "目标或选项已变化，请返回页面重新核对后操作");
                        self.interaction.history_open = true;
                        ctx.notify();
                        return true;
                    }
                    if r.discard {
                        self.discard_settings();
                        self.expansion.discard();
                        self.discard_studio_config();
                    }
                    self.record_notice(
                        if r.discard {
                            "已放弃更改"
                        } else {
                            "已确认，等待执行结果"
                        },
                        format!("{} · {}", r.title, r.target),
                    );
                    self.interaction.approved = true;
                    self.handle_action(&r.action, ctx);
                }
                ctx.notify();
                return true;
            }
            Action::ReviewHistory => {
                self.ensure_diagnostics_loaded();
                self.interaction.history_open = !self.interaction.history_open;
                if self.interaction.history_open { self.load_diagnostics_page(0); }
                self.interaction.history_feedback = None;
                ctx.notify();
                return true;
            }
            Action::StorageMaintain => {self.start_storage_maintenance(true);ctx.notify();return true;}
            Action::MoreDiagnostics => {
                self.load_diagnostics_page(self.interaction.history_page.saturating_add(1));
                ctx.notify();
                return true;
            }
            Action::ListPage(key, page) => {
                if key == "diagnostics" { self.load_diagnostics_page(*page); }
                else {
                    if self.interaction.list_pages.len() > 128 { self.interaction.list_pages.clear(); }
                    self.interaction.list_pages.insert(key.clone(), *page);
                    if key.starts_with("search-results:") { self.search.selected = page.saturating_mul(60); }
                    self.reset_paged_list_scroll(key);
                }
                ctx.notify();
                return true;
            }
            Action::OpenAppInMirror(_) if self.interaction.review.is_some() => {
                self.interaction.review = None;
                return false;
            }
            Action::Escape
                if self.interaction.review.is_some() || self.interaction.history_open =>
            {
                self.cancel_review_state();
                self.interaction.review = None;
                self.interaction.history_open = false;
                ctx.notify();
                return true;
            }
            _ => {}
        }
        if self.interaction.review.is_some() || self.interaction.history_open {
            return !matches!(action, Action::MinimizeWindow | Action::MaximizeWindow);
        }
        if self.expansion.pending.is_some()
            && matches!(
                action,
                Action::Select(_)
                    | Action::Forget
                    | Action::SetPreferredDevice(_)
                    | Action::CloseWindow
            )
        {
            self.status = "正在确认手机迁移设置，请稍后再切换或退出".into();
            ctx.notify();
            return true;
        }
        let leaving = matches!(
            action,
            Action::CompactChat
                | Action::Page(_)
                | Action::BackScreen(BackScreenAction::CalendarOpenOccurrence(_, _))
                | Action::GlobalSearch(
                    SearchAction::Select(_) | SearchAction::AllResults | SearchAction::Back
                )
                | Action::NotificationCenter(NotificationCenterAction::OpenPopupDetails(_))
                | Action::Workspace(
                    WorkspaceAction::New
                        | WorkspaceAction::Menu(_)
                        | WorkspaceAction::ClosePanel
                        | WorkspaceAction::History
                        | WorkspaceAction::Select(_)
                        | WorkspaceAction::PhoneSession(_)
                )
                | Action::OpenTool(_)
                | Action::Studio(StudioAction::Tab(_))
                | Action::CloseWindow
                | Action::Push(
                    PushAction::SetShellMode(_)
                        | PushAction::OpenMiniAiWindow
                        | PushAction::OpenMiniNotifyWindow
                )
                | Action::Settings(SettingsAction::Expansion | SettingsAction::Models)
                | Action::Expansion(expansion_state::ExpansionAction::Back)
                | Action::Select(_)
                | Action::Forget
                | Action::SetPreferredDevice(_)
        );
        if leaving
            && (self.settings_dirty() || self.expansion.dirty() || self.studio_config_dirty())
        {
            self.review_action(
                action,
                "还有未保存的更改",
                "仅保存在当前编辑界面".into(),
                vec![
                    "离开会放弃尚未保存的设置，手机与电脑继续使用原规则。".into(),
                    "选择“返回编辑”可继续修改并确认保存。".into(),
                ],
                "放弃更改并离开",
                false,
                true,
            );
            ctx.notify();
            return true;
        }
        // Notification preferences use the same draft/save contract on Settings,
        // while direct toggles on notification surfaces apply in place without jumping away.
        if let Action::Push(a) = action {
            let p = &mut self.settings.notify_draft;
            match a {
                PushAction::SetAlertScope(v) => p.scope = *v,
                PushAction::SetPopupMode(v) => p.set_popup_mode(*v),
                PushAction::SetAppPopupMode { package_name, mode } => {
                    p.set_app_popup_mode(package_name, *mode)
                }
                PushAction::TogglePrivacyMask => p.privacy_mask = !p.privacy_mask,
                PushAction::ToggleAutoCopyOtp => p.auto_copy_otp = !p.auto_copy_otp,
                _ => return self.review_effect(action, ctx),
            }
            if self.page == Page::Settings {
                self.set_compact_mode(ShellMode::Full, ctx);
                self.page = Page::Settings;
                self.settings.tab = 3;
                self.settings.feedback = "通知偏好尚未保存，请确认保存。只影响这台电脑。".into();
                ctx.notify();
                return true;
            }
            return false;
        }
        self.review_effect(action, ctx)
    }
    fn review_effect(&mut self, action: &Action, ctx: &mut ViewContext<Self>) -> bool {
        use expansion_state::ExpansionAction as E;
        let phone = self
            .selected
            .as_ref()
            .map(|d| format!("手机：{} · {}", d.model, d.serial))
            .unwrap_or_else(|| "尚未连接手机".into());
        let (title,details,accept,on_phone)= match action {
            Action::RequestUninstallApp(pkg)=>{
                if self.busy || !self.apps.iter().any(|a|a.package==*pkg&&a.can_uninstall()){return false;}
                self.pending_app_uninstall=Some(pkg.clone());
                self.review_action(&Action::ConfirmUninstallApp(pkg.clone()),"卸载手机应用？",phone,vec![format!("应用：{pkg}"),"会删除手机当前用户中的应用及其数据。此操作不能通过伴侣撤销。".into()],"确认卸载",true,false);ctx.notify();return true;
            },
            Action::Trash|Action::DeleteFile=>{
                if self.page!=Page::Files||self.recent_files_view||self.busy{return false;}
                let Some(entry)=self.selected_file.and_then(|i|self.files.get(i))else{return false;};
                let path=entry.path.clone();self.trash_confirm=true;self.permanent_delete=matches!(action,Action::DeleteFile);
                self.review_action(&Action::ConfirmTrash,if self.permanent_delete{"永久删除手机文件？"}else{"移到手机回收站？"},phone,vec![format!("文件：{path}"),if self.permanent_delete{"删除后无法恢复，请确认已不再需要。".into()}else{"优先移到手机系统回收站；旧系统使用 StepX 回收站。需要时可恢复，此操作不会立即释放文件空间。".into()}],if self.permanent_delete{"确认永久删除"}else{"移到回收站"},true,false);ctx.notify();return true;
            },
            Action::PrepareUtility(inner)=>return self.review_effect(inner,ctx),
            Action::Settings(SettingsAction::Apply) => {
                if !self.settings_dirty(){return true;}
                let mut lines=self.settings_changes();
                if self.settings.draft.sync_work_outputs && !self.settings.value.sync_work_outputs {lines.push(format!("开启后，后续桌面任务的交付文件将自动发送到任务所连接的手机，断线会排队续传。{phone}"));}
                lines.push("确认后生效；取消会保留当前编辑内容。".into());
                ("保存这些设置？",lines,"保存并应用",false)
            },
            Action::Expansion(E::Confirm|E::Enable) => {
                if let Err(e)=self.expansion.validate_draft(){self.expansion.feedback=e.to_string();ctx.notify();return true;}
                self.review_action(&Action::Expansion(E::Enable),"开启自动迁移并清理手机原件？",phone,self.expansion.review_lines(),"确认并开启迁移",true,false);ctx.notify();return true;
            },
            Action::Expansion(E::Pause)=>("暂停自动迁移？",vec!["保存暂停状态到当前手机，不再开始新文件的迁移。已完成的电脑原件和手机入口保留；正在校验的单个文件可能先完成。".into()],"确认暂停",true),
            Action::Expansion(E::Restore(id))=>("将原件恢复到手机？",vec![self.expansion.snapshot.files.iter().find(|j|j["id"]==*id).map(|j|format!("文件：{} · {}",j["name"].as_str().unwrap_or(""),storage::format_bytes(j["size"].as_u64().unwrap_or(0)))).unwrap_or_else(||"文件记录需要重新检查".into()),"通过当前连接传回手机，占用手机存储空间。电脑副本保留。".into()],"恢复到手机",true),
            Action::BackScreen(BackScreenAction::Play|BackScreenAction::SaveMarquee|BackScreenAction::GameArm|BackScreenAction::GameStart)=>("应用到手机背屏？",vec![format!("选择：{}",self.back_screen.title),format!("亮度：{}/8 · 帧率：{} 帧/秒 · 临时播放时长：{} 秒",self.back_screen.config.brightness,self.back_screen.config.fps,self.back_screen.config.duration_secs),"将应用到手机背屏，替换当前效果。原生功能与游戏按各自运行规则持续；关闭电脑预览不会撤销手机设置。".into(),"可以在背屏页面停止或重新选择效果。".into()],"确认应用到手机",true),
            Action::Studio(StudioAction::Send|StudioAction::Pin|StudioAction::UsePhone(_))|Action::StudioLab(StudioLabAction::DeliverPhone)=>("发送作品到手机？",vec!["将所选作品传入当前手机，安装或更新对应入口。传输与手机接收结果会在页面显示。".into()],"确认发送",true),
            Action::StudioLab(StudioLabAction::SyncCloud)=>("同步作品到 Studio？",vec!["将所选本地作品上传到已登录的 Studio 账号。同步成功后可继续编辑官网项目。".into()],"确认同步",false),
            Action::StudioLab(StudioLabAction::Save|StudioLabAction::Test|StudioLabAction::Models)=>("保存模型服务配置？",vec!["将保存当前 API 地址、协议和模型到本机；测试或获取模型目录会使用此配置连接服务。后续任务会将输入与附件发送给该服务。密钥不会显示在操作记录中。".into()],"确认保存",false),
            Action::StudioLab(StudioLabAction::ClearKey)=>("移除本机模型密钥？",vec!["移除后，需要重新填写密钥才能使用该模型服务。".into()],"确认移除",false),
            Action::Push(PushAction::DismissAll) => ("清除手机通知？",vec!["这会同步清除手机通知栏中的对应通知，不会删除聊天记录。".into()],"确认清除",true),
            Action::Push(PushAction::DismissNotification(_)) if self.page != Page::Home => ("清除手机通知？",vec!["这会同步清除手机通知栏中的对应通知，不会删除聊天记录。".into()],"确认清除",true),
            Action::Push(PushAction::OpenOnPhone(_)) if self.page != Page::Home => ("在手机上打开？",vec!["手机会切换到对应应用或页面，可能打断你正在手机上进行的操作。".into()],"在手机打开",true),
            Action::Studio(StudioAction::OpenPhone)|Action::OpenApp(_) => ("在手机上打开？",vec!["手机会切换到对应应用或页面，可能打断你正在手机上进行的操作。".into()],"在手机打开",true),
            Action::TransferFiles(paths,directory)=>{
                let mut rows=vec![format!("{} 个文件将从电脑复制到手机。电脑原件保留。",paths.len()),format!("手机保存位置：{}",directory.as_deref().unwrap_or("StepX伴侣 → 文件 → 收到的文件"))];
                rows.extend(paths.iter().take(6).map(|p|p.file_name().unwrap_or_default().to_string_lossy().into_owned()));
                if paths.len()>6{rows.push(format!("另有 {} 个文件",paths.len()-6));}
                if directory.is_some(){rows.push("目标目录存在同名文件时会被覆盖，请核对保存位置。".into());}
                ("发送这些文件到手机？",rows,"确认发送",true)
            },
            Action::CompanionBackground(enabled)=>("更改手机后台连接？",vec![if *enabled{"将在手机启用 Amoo 后台连接服务，可能增加耗电。".into()}else{"关闭后，正在处理的后台任务可能失去连接。".into()}],"确认更改",true),
            Action::ForceStopApp(_)=>("停止手机应用？",vec!["应用中尚未保存的操作可能丢失，后台任务和通知会暂停。".into()],"确认停止",true),
            Action::SwitchInputMethodFcitx5(true)=>("安装并切换手机输入法？",vec!["使用官方小企鹅 Fcitx5 0.1.3；已安装时保留现有版本与词库。".into(),"先确认小企鹅成为默认，再停用搜狗；保留系统安全输入法。".into(),"可以在此页面恢复搜狗，操作不会删除输入法数据。".into()],"确认安装并切换",true),
            Action::SwitchInputMethodFcitx5(false)=>("恢复手机搜狗输入法？",vec!["重新启用系统搜狗并设为默认；小企鹅与现有词库仍保留。".into()],"确认恢复",true),
            Action::ToggleSystemService(_,_)|Action::DebloatPresetServices|Action::RestoreAllSystemServices=>("更改手机系统服务？",vec!["此操作会修改当前手机上的系统服务状态，可能影响相关应用。请核对页面中选择的服务。".into()],"确认更改",true),
            Action::NeoConnectAmoo=>("连接 Amoo 后台操作？",vec!["向手机 Amoo 的当前账号发送技能安装请求；同名技能会先备份，再更新。".into(),"只安装调用说明，不启动手机操作。需等待 Amoo 验证安装成功，之后在新会话中使用；手机要求授权时请本人确认。".into()],"确认并查看安装对话",true),
            Action::NeoExecutionMode(background)=>("切换任务执行位置？",vec![if *background {"后续任务会使用手机系统的后台虚拟屏；同一应用可能迁入虚拟屏。需要解锁手机，截图发给已配置的模型，任务结束自动释放。".into()} else {"后续任务会操作手机当前屏幕，可随时暂停或停止。".into()},"此设置立即保存到手机，用于伴侣、电脑与 Amoo 的新任务。".into()],"确认并应用",true),
            Action::CompanionInstall|Action::NeoInstall|Action::GoogleSuiteInstall|Action::BackScreen(BackScreenAction::CompanionInstall)=>("安装到当前手机？",vec!["将安装或更新页面所选组件，并配置所需权限。请保持连接；手机有系统授权提示时需要你确认。".into()],"确认安装",true),
            _=>return false,
        };
        if on_phone && self.selected.is_none() {
            self.status = "请先连接目标手机".into();
            ctx.notify();
            return true;
        }
        self.review_action(
            action,
            title,
            if on_phone {
                phone
            } else {
                if matches!(action, Action::StudioLab(StudioLabAction::SyncCloud)) {
                    "已登录的 Studio 账号".into()
                } else {
                    "仅这台电脑".into()
                }
            },
            details,
            accept,
            on_phone,
            false,
        );
        ctx.notify();
        true
    }
    fn review_context(&self, a: &Action) -> String {
        match a {
            Action::Expansion(expansion_state::ExpansionAction::Enable) => {
                self.expansion.snapshot.status["policy"]["epoch"].to_string()
            }
            Action::ConfirmTrash => format!(
                "{:?}|{}",
                self.selected_file
                    .and_then(|i| self.files.get(i))
                    .map(|e| &e.path),
                self.permanent_delete
            ),
            Action::BackScreen(_) => format!(
                "{}|{}|{}|{}|{}",
                self.back_screen.title,
                self.back_screen.revision,
                self.back_screen.config.brightness,
                self.back_screen.config.fps,
                self.back_screen.config.duration_secs
            ),
            Action::Studio(StudioAction::Send | StudioAction::Pin | StudioAction::OpenPhone) => {
                self.studio
                    .selected
                    .and_then(|i| self.studio.works.get(i))
                    .map(|w| w.url.clone())
                    .unwrap_or_default()
            }
            Action::StudioLab(StudioLabAction::DeliverPhone | StudioLabAction::SyncCloud) => self
                .studio_lab
                .selected
                .and_then(|i| self.studio_lab.artifacts.get(i))
                .map(|w| w.id.clone())
                .unwrap_or_default(),
            _ => String::new(),
        }
    }
    fn cancel_review_state(&mut self) {
        if let Some(r) = &self.interaction.review {
            match r.action {
                Action::ConfirmUninstallApp(_) => self.pending_app_uninstall = None,
                Action::ConfirmTrash => self.trash_confirm = false,
                Action::ConfirmDirectSms => self.pending_sms_plan = None,
                Action::PhoneConfirmCall => self.pending_call = None,
                _ => {}
            }
        }
    }
    pub(super) fn studio_config_dirty(&self) -> bool {
        ((self.page == Page::Studio && self.studio.tab == 3)
            || (self.page == Page::Amoo && self.workspace.panel == 5))
            && self.studio_lab.loaded
            && self.studio_lab.pending_config.is_none()
            && (self.fields[F_LAB_ENDPOINT].text != self.studio_lab.config.endpoint
                || self.fields[F_LAB_MODEL].text != self.studio_lab.config.model
                || self.studio_lab.protocol
                    != u8::from(self.studio_lab.config.protocol == "anthropic")
                || !self.fields[F_LAB_KEY].text.is_empty())
    }
    fn discard_studio_config(&mut self) {
        if self.studio_config_dirty() {
            self.fields[F_LAB_ENDPOINT].set(&self.studio_lab.config.endpoint);
            self.fields[F_LAB_MODEL].set(&self.studio_lab.config.model);
            self.fields[F_LAB_KEY].clear();
            self.studio_lab.protocol = u8::from(self.studio_lab.config.protocol == "anthropic");
        }
    }
    pub(super) fn prepare_file_transfer(&mut self, paths: Vec<PathBuf>, directory: Option<String>) {
        let target = self
            .selected
            .as_ref()
            .map(|d| format!("手机：{} · {}", d.model, d.serial))
            .unwrap_or_else(|| "尚未连接手机".into());
        let mut rows = vec![
            format!("将 {} 个文件从电脑复制到手机，电脑原件保留。", paths.len()),
            format!(
                "保存位置：{}",
                directory
                    .as_deref()
                    .unwrap_or("StepX伴侣 → 文件 → 收到的文件")
            ),
        ];
        rows.extend(paths.iter().take(6).map(|p| {
            p.file_name()
                .unwrap_or_default()
                .to_string_lossy()
                .into_owned()
        }));
        if paths.len() > 6 {
            rows.push(format!("另有 {} 个文件", paths.len() - 6));
        }
        if directory.is_some() {
            rows.push("目标目录存在同名文件时会被覆盖。".into());
        }
        self.review_action(
            &Action::TransferFiles(paths, directory),
            "发送这些文件到手机？",
            target,
            rows,
            "确认发送",
            true,
            false,
        );
    }
    pub(super) fn edit_footer(
        &self,
        title: String,
        note: &str,
        dirty: bool,
        can_apply: bool,
        cancel: Action,
        apply: Action,
        label: &str,
    ) -> El {
        let p = self.p();
        Container::new(self.between(
            self.vstack(
                S1,
                vec![self.heading(title), self.text(note, CAPTION, p.text2)],
            ),
            self.hstack(
                S2,
                vec![
                    if dirty {
                        self.button("edit-discard", "放弃更改", cancel, Btn::Secondary)
                    } else {
                        self.disabled_button("放弃更改")
                    },
                    if can_apply {
                        self.button("edit-apply", label, apply, Btn::Primary)
                    } else {
                        self.disabled_button(label)
                    },
                ],
            ),
        ))
        .with_uniform_padding(S4)
        .with_background_color(p.surface)
        .with_border(Border::top(1.).with_border_color(p.border))
        .finish()
    }
    pub(super) fn interaction_overlay(&self) -> Option<El> {
        let p = self.p();
        let (title, mut body, buttons) = if let Some(r) = &self.interaction.review {
            let body = vec![
                self.pill_badge(&r.target, p.text2, p.hover),
                self.vstack(
                    S3,
                    r.details
                        .iter()
                        .map(|s| self.text(s, BODY, p.text))
                        .collect(),
                ),
            ];
            (r.title.as_str(), body, {
                let mut btns = vec![self.button(
                    "review-cancel",
                    if r.discard {
                        "返回编辑"
                    } else {
                        "取消，继续编辑"
                    },
                    Action::ReviewCancel,
                    Btn::Secondary,
                )];
                if let Action::OpenApp(pkg) = &r.action {
                    btns.push(self.button_icon(
                        "review-open-mirror",
                        "投屏到电脑运行",
                        icons::UiIcon::Monitor,
                        Action::OpenAppInMirror(pkg.clone()),
                        Btn::Secondary,
                    ));
                }
                btns.push(self.button(
                    "review-confirm",
                    &r.accept,
                    Action::ReviewConfirm,
                    Btn::Primary,
                ));
                btns
            })
        } else if self.interaction.history_open {
            let mut body = vec![self.text(
                "最近 30 天，最多 300 条。日志仅保存在本机；过期记录自动清理。",
                CAPTION,
                p.text2,
            )];
            let writer_error = self
                .interaction
                .diagnostic_writer
                .as_ref()
                .and_then(|w| w.error.lock().ok().and_then(|e| e.clone()));
            if let Some(error) = writer_error
                .as_ref()
                .or(self.interaction.diagnostics_error.as_ref())
            {
                body.push(self.text(error, CAPTION, p.danger));
            }
            if let Some(message) = &self.interaction.history_feedback {
                body.push(self.text(message, CAPTION, p.warning));
            }
            if self.interaction.history_entries.is_empty() {
                body.push(self.text(if self.interaction.history_rx.is_some() {"正在读取日志…"} else {"暂无排障日志"}, BODY, p.text3));
            }
            for entry in self
                .interaction
                .history_entries
                .iter()
            {
                let at = chrono::DateTime::from_timestamp_millis(entry.at)
                    .map(|t| {
                        t.with_timezone(&chrono::Local)
                            .format("%m-%d %H:%M:%S")
                            .to_string()
                    })
                    .unwrap_or_default();
                let mut buttons = vec![self.button_icon(
                    format!("diagnostic-copy-{}", entry.id),
                    "复制",
                    icons::UiIcon::Copy,
                    Action::CopyDiagnostic(entry.id.clone()),
                    Btn::Ghost,
                )];
                if let Some(failure) = self.interaction.failures.get(&entry.id) {
                    buttons.push(self.button(
                        format!("diagnostic-retry-{}", entry.id),
                        if failure.retry.is_some() {
                            "重试"
                        } else {
                            "返回操作"
                        },
                        Action::RetryOperation(entry.id.clone()),
                        Btn::Secondary,
                    ));
                }
                body.push(self.vstack(
                    S1,
                    vec![
                        self.heading(format!("{at}  {}", entry.title)),
                        self.text(&entry.detail, CAPTION, p.text2),
                        self.wrap(buttons),
                        self.divider(),
                    ],
                ));
            }
            if self.interaction.history_total > 30 {
                let page = self.interaction.history_page;
                body.push(self.between(self.text(format!("第 {} 页 · 共 {} 条",page+1,self.interaction.history_total),CAPTION,p.text2),self.hstack(S2,vec![
                    if page > 0 && self.interaction.history_rx.is_none() { self.button("diagnostic-previous","较新记录",Action::ListPage("diagnostics".into(),page-1),Btn::Ghost) } else {self.disabled_button("较新记录")},
                    if (page+1)*30 < self.interaction.history_total && self.interaction.history_rx.is_none() { self.button("diagnostic-more","更早记录",Action::MoreDiagnostics,Btn::Ghost) } else {self.disabled_button("更早记录")},
                ])));
            }
            (
                "排障日志",
                body,
                vec![self.button("review-close", "关闭", Action::ReviewCancel, Btn::Primary)],
            )
        } else {
            return None;
        };
        let content = self.pane_scroll(
            Default::default(),
            self.vstack(S4, body.drain(..).collect()),
        );
        let sheet = Container::new(self.vstack(
            S4,
            vec![
                self.line_w(title, 22., p.text, facloud_ui::fonts::Weight::Semibold),
                ConstrainedBox::new(content).with_max_height(420.).finish(),
                self.divider(),
                self.between(Empty::new().finish(), self.hstack(S2, buttons)),
            ],
        ))
        .with_uniform_padding(S5)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();
        Some(
            Container::new(Align::new(self.elevated_overlay_card(sheet, 620., false)).finish())
                .with_uniform_padding(S4)
                .with_background_color(if p.dark {
                    rgba(0, 0, 0, 148)
                } else {
                    rgba(15, 23, 42, 108)
                })
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                .finish(),
        )
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;
    #[test]
    fn embedded_and_floating_pagers_preserve_unrelated_reading_positions() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                let px = facloud_ui::units::Pixels::new;
                view.page = Page::Home;
                view.scroll.scroll_to(px(340.));
                view.sms_threads_scroll.scroll_to(px(120.));
                view.sms_messages_scroll.scroll_to(px(220.));
                view.notification_center.list_scroll.scroll_to(px(80.));
                view.handle_action(&Action::ListPage("home-month-agenda:2026-11-07".into(), 1), ctx);
                assert_eq!(view.scroll.scroll_start(), px(340.));
                assert_eq!(view.sms_messages_scroll.scroll_start(), px(220.));
                view.handle_action(&Action::ListPage("notification-drawer:All".into(), 1), ctx);
                assert_eq!(view.notification_center.list_scroll.scroll_start(), px(0.));
                assert_eq!(view.scroll.scroll_start(), px(340.));
                view.page = Page::Sms;
                view.handle_action(&Action::ListPage("message-threads:All:".into(), 1), ctx);
                assert_eq!(view.sms_threads_scroll.scroll_start(), px(0.));
                assert_eq!(view.sms_messages_scroll.scroll_start(), px(220.));
                view.sms_threads_scroll.scroll_to(px(140.));
                view.handle_action(&Action::ListPage("sms-history:10086".into(), 1), ctx);
                assert_eq!(view.sms_messages_scroll.scroll_start(), px(0.));
                assert_eq!(view.sms_threads_scroll.scroll_start(), px(140.));
            });
        });
    }

    #[test]
    fn list_pages_cover_the_whole_source_and_reset_for_a_new_filter() {
        let mut app = Companion::new_for_test();
        let mut indices = vec![];
        for page in 0..6 {
            app.interaction.list_pages.insert("messages:all".into(),page);
            indices.extend(app.list_page_range("messages:all",213,40));
        }
        assert_eq!(indices,(0..213).collect::<Vec<_>>());
        assert_eq!(app.list_page_range("messages:search",213,40),0..40);
        assert_eq!(app.list_page_range("messages:all",3,40),0..3);
        assert_eq!(app.list_page_range("messages:all",0,40),0..0);
    }
    #[test]
    fn diagnostic_history_loads_a_fixed_page_without_discarding_session_failures() {
        let mut app = Companion::new_for_test();
        for i in 0..75 { app.record_notice(format!("事件 {i}"),"保留完整细节"); }
        app.load_diagnostics_page(1);
        assert_eq!(app.interaction.history_entries.len(),30);
        assert_eq!(app.interaction.history_total,75);
        assert_eq!(app.interaction.diagnostics.len(),75);
        assert_eq!(app.interaction.history_entries[0].id,app.interaction.diagnostics[30].id);
    }
    #[test]
    fn failed_job_keeps_its_exact_read_target_and_original_device() {
        let mut app = Companion::new_for_test();
        let operation = OperationFailure::for_action(
            &Action::Browse("/sdcard/Download/missing".into()),
            Page::Files,
            Some("original-phone".into()),
        );
        let error = anyhow::anyhow!("No such file or directory").context(operation);
        app.record_operation_error(&error);
        let entry = &app.interaction.diagnostics[0];
        assert!(entry.detail.contains("No such file or directory"));
        let retry = &app.interaction.failures[&entry.id];
        assert_eq!(retry.serial.as_deref(), Some("original-phone"));
        assert!(
            matches!(retry.action(),Some(Action::Browse(path)) if path=="/sdcard/Download/missing")
        );
    }
    #[test]
    fn mutations_are_never_automatically_replayed_from_logs() {
        for action in [
            Action::ConfirmDirectSms,
            Action::InstallApk,
            Action::ConfirmTrash,
            Action::AmooSend,
            Action::Restore,
        ] {
            let operation = OperationFailure::for_action(&action, Page::Sms, None);
            assert!(operation.action().is_none());
        }
    }
}
