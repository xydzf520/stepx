//! Shared Agent workbench. The model produces a preview, never a desktop action.
use super::{icons::UiIcon, settings_state::AGENT_SETTINGS_TAB, theme::*, widgets::*, *};
use facloud_ui::TypedActionView;
use std::sync::atomic::{AtomicBool, Ordering};
use stepx_companion::{
    agent_workflows::{self as agent, Draft, Route, Scenario, PRESETS},
    home_hub::HomeAiIntent,
    model_source::Source,
};

#[derive(Clone, Debug)]
pub(super) enum AgentAction {
    Preset(Scenario),
    Auto,
    Run,
    Cancel,
    OpenSearch,
    SaveDraft,
}

struct Job {
    receiver: mpsc::Receiver<anyhow::Result<Generated>>,
    cancel: Arc<AtomicBool>,
    source: Source,
    phone: Option<(String, String)>,
    prompt: String,
}

struct Generated {
    draft: Draft,
    model: String,
    session_cleaned: bool,
}

impl Drop for Job {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Release);
    }
}

#[derive(Default)]
pub(super) struct AgentWorkbench {
    preset: Option<Scenario>,
    job: Option<Job>,
    draft: Option<Draft>,
    status: String,
    error: bool,
    from_home: bool,
    request: String,
    saving: Option<mpsc::Receiver<anyhow::Result<std::path::PathBuf>>>,
    saved_path: Option<std::path::PathBuf>,
    model: String,
    session_cleaned: bool,
}

impl Companion {
    pub(super) fn handle_agent_action(
        &mut self,
        action: &AgentAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match *action {
            AgentAction::Preset(scenario) => {
                let text = self.fields[F_AGENT_REQUEST].text.trim();
                if text.is_empty() || PRESETS.iter().any(|preset| preset.example == text) {
                    self.fields[F_AGENT_REQUEST].set(scenario.preset().example);
                }
                self.agent_workbench.preset = Some(scenario);
                self.focus_field(F_AGENT_REQUEST, ctx);
            }
            AgentAction::Auto => self.agent_workbench.preset = None,
            AgentAction::Run => {
                let request = self.fields[F_AGENT_REQUEST].text.trim().to_string();
                self.run_agent_request(request, false, ctx);
            }
            AgentAction::Cancel => {
                if self.agent_workbench.job.take().is_some() {
                    self.agent_workbench.status = "已取消，原文可继续编辑".into();
                    self.agent_workbench.error = false;
                }
            }
            AgentAction::OpenSearch => {
                if let Some(query) = self
                    .agent_workbench
                    .draft
                    .as_ref()
                    .and_then(|draft| draft.search_query.clone())
                {
                    self.open_agent_search(&query, ctx);
                }
            }
            AgentAction::SaveDraft => {
                if self.agent_workbench.saving.is_none()
                    && self.agent_workbench.saved_path.is_none()
                {
                    if let Some(draft) = self.agent_workbench.draft.clone() {
                        let directory = stepx_companion::settings::downloads();
                        let (tx, rx) = mpsc::channel();
                        self.agent_workbench.saving = Some(rx);
                        std::thread::spawn(move || {
                            let _ = tx.send(agent::save_draft(&directory, &draft));
                        });
                    }
                }
            }
        }
        self.sync_ime(ctx);
        ctx.notify();
    }

    fn open_agent_search(&mut self, query: &str, ctx: &mut ViewContext<Self>) {
        self.handle_action(&Action::GlobalSearch(SearchAction::Open), ctx);
        self.handle_action(&Action::GlobalSearch(SearchAction::ResetFilters), ctx);
        self.handle_action(
            &Action::GlobalSearch(SearchAction::Query(query.into())),
            ctx,
        );
        self.handle_action(&Action::GlobalSearch(SearchAction::AllResults), ctx);
    }

    pub(super) fn run_agent_request(
        &mut self,
        request: String,
        from_home: bool,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        if self.agent_workbench.job.is_some() || self.agent_workbench.saving.is_some() {
            return false;
        }
        self.agent_workbench.request = request.clone();
        self.agent_workbench.saved_path = None;
        let route = if from_home {
            agent::route(&request)
        } else {
            self.agent_workbench
                .preset
                .map_or_else(|| agent::route(&request), Route::Agent)
        };
        let scenario = match route {
            Route::Search(query) => {
                self.agent_workbench.draft = None;
                self.agent_workbench.status.clear();
                self.open_agent_search(&query, ctx);
                return true;
            }
            Route::Existing(intent) => {
                self.agent_workbench.draft = None;
                self.agent_workbench.status.clear();
                if matches!(&intent, HomeAiIntent::OpenPage(page) if page == "calendar-today") {
                    self.handle_action(&Action::Page(Page::Calendar), ctx);
                    if self.page == Page::Calendar {
                        self.handle_action(
                            &Action::BackScreen(BackScreenAction::CalendarToday),
                            ctx,
                        );
                    }
                } else {
                    self.handle_action(&Action::HomeExecuteIntent(intent), ctx);
                }
                return !self.error;
            }
            Route::CalendarDraft { date, time, title } => {
                use chrono::Datelike;
                self.handle_action(&Action::Page(Page::Calendar), ctx);
                if self.page != Page::Calendar {
                    return false;
                }
                self.handle_action(
                    &Action::BackScreen(BackScreenAction::CalendarSelectDate(
                        date.year(),
                        date.month(),
                        date.day(),
                    )),
                    ctx,
                );
                self.handle_action(
                    &Action::BackScreen(BackScreenAction::CalendarPrefillFromText {
                        title,
                        time_hint: time.format("%H:%M").to_string(),
                    }),
                    ctx,
                );
                self.status = "日程草稿已填入，核对后点击添加".into();
                self.agent_workbench.draft = None;
                self.agent_workbench.status.clear();
                return true;
            }
            Route::Agent(scenario) => scenario,
        };
        let date = chrono::Local::now()
            .format("%Y-%m-%d %A %H:%M %:z")
            .to_string();
        let prompt = match agent::prompt(
            scenario,
            &self.settings.value.home_agent_prompt,
            &request,
            &date,
        ) {
            Ok(prompt) => prompt,
            Err(error) => return self.agent_failure(from_home, ctx, error.to_string()),
        };
        let source = match stepx_companion::model_source::load() {
            Ok(preferences) => preferences.source,
            Err(_) => {
                return self.agent_failure(
                    from_home,
                    ctx,
                    "模型来源配置无效，请在模型设置中重新选择".into(),
                )
            }
        };
        let device = self.selected.clone();
        let account = self.amoo_ui.account_scope.clone();
        let phone = match source {
            Source::PhoneAccount => {
                let Some((device, account)) = device
                    .as_ref()
                    .zip(account.as_ref())
                    .filter(|_| !self.is_demo_device())
                else {
                    return self.agent_failure(
                        from_home,
                        ctx,
                        "手机账号暂不可用。连接手机并登录 Amoo 后重试。".into(),
                    );
                };
                if !stepx_companion::home_ai::source_capabilities(source).manual_available {
                    return self.agent_failure(
                        from_home,
                        ctx,
                        "当前手机账号暂不可用，请在模型设置中检查。".into(),
                    );
                }
                Some((device.serial.clone(), account.clone()))
            }
            Source::IndependentApi => {
                if !stepx_companion::home_ai::source_capabilities(source).automatic_available {
                    return self.agent_failure(
                        from_home,
                        ctx,
                        "独立 API 尚未配置完整，请在模型设置中检查。".into(),
                    );
                }
                None
            }
        };
        let (tx, receiver) = mpsc::channel();
        let cancel = Arc::new(AtomicBool::new(false));
        self.agent_workbench.job = Some(Job {
            receiver,
            source,
            phone,
            cancel: cancel.clone(),
            prompt: self.settings.value.home_agent_prompt.clone(),
        });
        self.agent_workbench.draft = None;
        self.agent_workbench.model.clear();
        self.agent_workbench.from_home = from_home;
        self.agent_workbench.error = false;
        self.agent_workbench.status =
            format!("{}正在处理 · {}", source.label(), scenario.preset().label);
        ctx.notify();
        std::thread::spawn(move || {
            let run = || -> anyhow::Result<Generated> {
                anyhow::ensure!(
                    stepx_companion::model_source::load()?.source == source,
                    "模型来源已变更，请重试"
                );
                let response = match source {
                    Source::IndependentApi => {
                        stepx_companion::home_ai::generate_local_json(&prompt, &cancel)
                    }
                    Source::PhoneAccount => {
                        let device = device
                            .as_ref()
                            .ok_or_else(|| anyhow::anyhow!("手机已断开"))?;
                        let adb = Adb::discover()?;
                        anyhow::ensure!(
                            stepx_companion::amoo_usage::account_scope_fresh(&adb, device)?
                                == account,
                            "手机账号已变化，请重试"
                        );
                        stepx_companion::home_ai::generate_selected_json_recorded(
                            &adb,
                            device,
                            source,
                            &prompt,
                            &cancel,
                            &mut |_| Ok(()),
                        )
                        .map(|value| value.brief)
                    }
                }
                .map_err(|error| {
                    let detail = format!("{error:#}");
                    let reason = if detail.contains("Amoo 云连接已中断或超时") {
                        "手机 Amoo 云连接中断或超时，请检查手机网络及后台协作状态后重试。"
                    } else {
                        "模型暂未返回可用结果，请检查连接或重试。"
                    };
                    if detail.contains("临时分析会话未确认删除") {
                        anyhow::anyhow!("{reason}临时会话清理尚未确认，请在手机 Amoo 中查看。")
                    } else {
                        anyhow::anyhow!("{reason}")
                    }
                })?;
                Ok(Generated {
                    draft: agent::validate_output(scenario, &request, &response.markdown)?,
                    model: response.model,
                    session_cleaned: response.session_cleaned,
                })
            };
            let _ = tx.send(run());
        });
        true
    }

    fn agent_failure(
        &mut self,
        from_home: bool,
        ctx: &mut ViewContext<Self>,
        reason: String,
    ) -> bool {
        self.agent_workbench.status = reason;
        self.agent_workbench.error = true;
        self.agent_workbench.from_home = from_home;
        self.agent_workbench.draft = None;
        ctx.notify();
        false
    }

    pub(super) fn tick_agent_workbench(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(rx) = &self.agent_workbench.saving {
            match rx.try_recv() {
                Err(mpsc::TryRecvError::Empty) => {}
                result => {
                    self.agent_workbench.saving = None;
                    match result {
                        Ok(Ok(path)) => self.agent_workbench.saved_path = Some(path),
                        _ => {
                            self.agent_workbench.status = "草稿未保存，请检查下载目录后重试".into();
                            self.agent_workbench.error = true;
                        }
                    }
                    ctx.notify();
                }
            }
        }
        let Some(job) = self.agent_workbench.job.as_ref() else {
            return;
        };
        let result = match job.receiver.try_recv() {
            Ok(result) => result,
            Err(mpsc::TryRecvError::Empty) => return,
            Err(mpsc::TryRecvError::Disconnected) => Err(anyhow::anyhow!("Agent 已停止，请重试")),
        };
        let current_source = stepx_companion::model_source::load().map(|p| p.source).ok();
        let scope_ok = job.prompt == self.settings.value.home_agent_prompt
            && current_source == Some(job.source)
            && job.phone.as_ref().is_none_or(|(serial, account)| {
                self.selected
                    .as_ref()
                    .is_some_and(|device| &device.serial == serial)
                    && self.amoo_ui.account_scope.as_ref() == Some(account)
            });
        let result = if scope_ok {
            result
        } else {
            Err(anyhow::anyhow!("模型来源、手机账号或提示词已变更，请重试"))
        };
        let bench = &mut self.agent_workbench;
        bench.job = None;
        match result {
            Ok(generated) => {
                bench.status = if generated.draft.needs_input {
                    "需要补充信息".into()
                } else {
                    "草稿已就绪".into()
                };
                bench.draft = Some(generated.draft);
                bench.model = generated.model;
                bench.session_cleaned = generated.session_cleaned;
                bench.error = false;
            }
            Err(error) => {
                bench.status = error.to_string();
                bench.error = true;
            }
        }
        ctx.notify();
    }

    pub(super) fn render_agent_workbench(&self) -> El {
        let p = self.p();
        let bench = &self.agent_workbench;
        let mut presets = vec![self.chip(
            "agent-preset-auto",
            "自动识别",
            Some(UiIcon::Sparkles),
            vec![Action::Agent(AgentAction::Auto)],
            bench.preset.is_none(),
            false,
        )];
        presets.extend(PRESETS.iter().enumerate().map(|(index, preset)| {
            self.chip(
                format!("agent-preset-{index}"),
                preset.label,
                None,
                vec![Action::Agent(AgentAction::Preset(preset.scenario))],
                bench.preset == Some(preset.scenario),
                false,
            )
        }));
        let run = if bench.job.is_some() {
            self.button(
                "agent-cancel",
                "取消",
                Action::Agent(AgentAction::Cancel),
                Btn::Secondary,
            )
        } else if bench.saving.is_some() {
            self.disabled_button("正在保存…")
        } else {
            self.button_icon(
                "agent-run",
                "运行",
                UiIcon::Sparkles,
                Action::Agent(AgentAction::Run),
                Btn::Primary,
            )
        };
        let mut rows = vec![
            self.section_heading(UiIcon::Sparkles, "场景工作台"),
            self.text(
                "点选示例可直接试用，也可以换成自己的资料。运行时只提交这个输入框里的文字。",
                CAPTION,
                p.text2,
            ),
            self.bounded_wrap(presets),
            self.text(
                bench.preset.map_or(
                    "自动识别会先检查本地规则，再选择需要理解的场景。",
                    |scenario| scenario.preset().instruction,
                ),
                MICRO,
                p.text2,
            ),
            self.input(F_AGENT_REQUEST, "写下你要处理的事项，或粘贴相关文字…", None),
            self.between(
                self.text(
                    "沿用已选模型来源。手机账号通过 Amoo 临时会话生成，仅在点击运行后提交。",
                    MICRO,
                    p.text2,
                ),
                run,
            ),
        ];
        if let Some(output) = self.agent_result(false) {
            rows.push(output);
        }
        self.panel(rows)
    }

    pub(super) fn agent_result(&self, from_home: bool) -> Option<El> {
        let p = self.p();
        let bench = &self.agent_workbench;
        if bench.status.is_empty() || (from_home && !bench.from_home) {
            return None;
        }
        let mut rows = vec![self.line_w(
            &bench.status,
            CAPTION,
            if bench.error { p.danger } else { p.text2 },
            facloud_ui::fonts::Weight::Medium,
        )];
        rows.push(self.line(
            format!("本次请求 · {}", Self::truncate_chars(&bench.request, 72)),
            MICRO,
            p.text2,
        ));
        if let Some(draft) = &bench.draft {
            rows.push(self.line(
                format!(
                    "实际模型 · {}",
                    if bench.model.is_empty() {
                        "服务未返回名称"
                    } else {
                        &bench.model
                    }
                ),
                MICRO,
                p.text2,
            ));
            if !bench.session_cleaned {
                rows.push(self.text(
                    "临时会话清理尚未确认，请在手机 Amoo 中查看。",
                    MICRO,
                    p.warning,
                ));
            }
            rows.push(self.heading(&draft.title));
            rows.push(self.text(&draft.draft, BODY, p.text));
            if !draft.quotes.is_empty() {
                rows.push(self.text(
                    format!("原文依据 · {}", draft.quotes.join(" / ")),
                    MICRO,
                    p.text2,
                ));
            }
            let mut actions = vec![self.button_icon(
                "agent-copy-draft",
                "复制内容",
                UiIcon::Copy,
                Action::CopyText(draft.draft.clone(), "已复制".into()),
                Btn::Secondary,
            )];
            if let Some(path) = &bench.saved_path {
                rows.push(self.text(format!("草稿已保存 · {}", path.display()), MICRO, p.text2));
            } else if bench.saving.is_some() {
                rows.push(self.line("正在保存草稿…", MICRO, p.text2));
            } else {
                actions.push(self.button_icon(
                    "agent-save-draft",
                    "保存草稿",
                    UiIcon::Download,
                    Action::Agent(AgentAction::SaveDraft),
                    Btn::Secondary,
                ));
            }
            if draft.search_query.is_some() {
                actions.push(self.button_icon(
                    "agent-open-search",
                    "打开全局搜索",
                    UiIcon::Search,
                    Action::Agent(AgentAction::OpenSearch),
                    Btn::Secondary,
                ));
            }
            rows.push(self.bounded_wrap(actions));
        } else if from_home {
            let action = if bench.job.is_some() {
                self.button(
                    "agent-home-cancel",
                    "取消",
                    Action::Agent(AgentAction::Cancel),
                    Btn::Secondary,
                )
            } else {
                self.button(
                    "agent-home-settings",
                    "打开 Agent 设置",
                    Action::Settings(SettingsAction::Tab(AGENT_SETTINGS_TAB)),
                    Btn::Secondary,
                )
            };
            rows.push(action);
        }
        Some(self.panel(rows))
    }

    #[cfg(feature = "ui-capture")]
    pub(super) fn agent_diagnostics(&self) -> serde_json::Value {
        let bench = &self.agent_workbench;
        serde_json::json!({
            "running": bench.job.is_some(),
            "has_draft": bench.draft.is_some(),
            "needs_input": bench.draft.as_ref().map(|draft| draft.needs_input),
            "error": bench.error,
            "status": bench.status,
            "home_enabled": self.settings.value.home_agent_enabled,
            "saved": bench.saved_path.is_some(),
            "request_chars": bench.request.chars().count(),
        })
    }
}
