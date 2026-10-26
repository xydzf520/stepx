//! Shared review sheet: edits stay local; only the primary button commits effects.
use super::{theme::*, widgets::*, *};
use facloud_ui::TypedActionView;
#[derive(Default)]
pub(super) struct InteractionUi {
    pub review: Option<Review>,
    pub approved: bool,
    pub history_open: bool,
    pub notices: Vec<(String, String, String)>,
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
    pub(super) fn record_notice(&mut self, title: impl Into<String>, detail: impl Into<String>) {
        self.interaction.notices.insert(
            0,
            (
                chrono::Local::now().format("%H:%M:%S").to_string(),
                title.into(),
                detail.into(),
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
                self.interaction.history_open = !self.interaction.history_open;
                ctx.notify();
                return true;
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
                PushAction::SetPopupMode(v) => p.popup_mode = *v,
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
                if self.page!=Page::Files||self.busy{return false;}
                let Some(entry)=self.selected_file.and_then(|i|self.files.get(i))else{return false;};
                let path=entry.path.clone();self.trash_confirm=true;self.permanent_delete=matches!(action,Action::DeleteFile);
                self.review_action(&Action::ConfirmTrash,if self.permanent_delete{"永久删除手机文件？"}else{"移到手机回收站？"},phone,vec![format!("文件：{path}"),if self.permanent_delete{"删除后无法恢复，请确认已不再需要。".into()}else{"移到手机的 StepX 回收站。需要时可恢复；此操作不会释放文件占用的空间。".into()}],if self.permanent_delete{"确认永久删除"}else{"移到回收站"},true,false);ctx.notify();return true;
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
            Action::Push(PushAction::DismissAll|PushAction::DismissNotification(_))=>("清除手机通知？",vec!["这会同步清除手机通知栏中的对应通知，不会删除聊天记录。".into()],"确认清除",true),
            Action::Push(PushAction::OpenOnPhone(_))|Action::Studio(StudioAction::OpenPhone)|Action::OpenApp(_)=>("在手机上打开？",vec!["手机会切换到对应应用或页面，可能打断你正在手机上进行的操作。".into()],"在手机打开",true),
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
            (
                r.title.as_str(),
                body,
                vec![
                    self.button(
                        "review-cancel",
                        if r.discard {
                            "返回编辑"
                        } else {
                            "取消，继续编辑"
                        },
                        Action::ReviewCancel,
                        Btn::Secondary,
                    ),
                    self.button(
                        "review-confirm",
                        &r.accept,
                        Action::ReviewConfirm,
                        Btn::Primary,
                    ),
                ],
            )
        } else if self.interaction.history_open {
            let mut body = vec![self.text(
                "本次运行的操作记录。提交请求与实际完成会分别显示。",
                CAPTION,
                p.text2,
            )];
            if self.interaction.notices.is_empty() {
                body.push(self.text("暂无操作提醒", BODY, p.text3));
            }
            for (at, title, detail) in self.interaction.notices.iter().take(12) {
                body.push(self.vstack(
                    S1,
                    vec![
                        self.heading(format!("{at}  {title}")),
                        self.text(detail, CAPTION, p.text2),
                        self.divider(),
                    ],
                ));
            }
            (
                "操作提醒",
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
            Container::new(
                Align::new(ConstrainedBox::new(sheet).with_width(620.).finish()).finish(),
            )
            .with_uniform_padding(S4)
            .with_background_color(rgba(0, 0, 0, 96))
            .finish(),
        )
    }
}
