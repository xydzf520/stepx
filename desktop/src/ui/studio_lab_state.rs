use super::*;
use std::sync::atomic::Ordering;
use stepx_companion::{studio_delivery, studio_lab};

#[derive(Clone, Debug)]
pub(super) enum StudioLabAction {
    Protocol(u8),
    Save,
    Test,
    Models,
    Model(String),
    ClearKey,
    Start,
    Stop,
    Fresh,
    Template(usize),
    Select(usize),
    HistoryPage(usize),
    Iterate,
    ReusePrompt(usize),
    Preview,
    PreviewAt(usize),
    OpenFolder,
    Refresh,
    ToggleReport,
    CloudModels,
    CloudModel(String),
    DeliverPhone,
    DeliverPhoneAt(usize),
    OpenInWorkbench(usize),
    ImportFromWork,
    SyncCloud,
    OpenCloud,
    DeleteArtifactAt(usize),
    ApplyProviderPreset(u8),
}
impl StudioLabAction {
    fn available(&self, busy: bool, running: bool) -> bool {
        use StudioLabAction::*;
        if running
            && matches!(
                self,
                Start
                    | Fresh
                    | Template(_)
                    | Iterate
                    | ReusePrompt(_)
                    | OpenInWorkbench(_)
                    | ImportFromWork
                    | Save
                    | Test
                    | Models
                    | ClearKey
                    | ApplyProviderPreset(_)
            )
        {
            return false;
        }
        !busy
            || matches!(
                self,
                Stop | Select(_)
                    | HistoryPage(_)
                    | ToggleReport
                    | Protocol(_)
                    | Model(_)
                    | CloudModel(_)
                    | DeleteArtifactAt(_)
                    | ApplyProviderPreset(_)
            )
    }
}
pub(super) enum StudioLabOutcome {
    Loaded(Result<(studio_lab::PublicConfig, Vec<studio_lab::LabArtifact>), String>),
    Config(Result<studio_lab::PublicConfig, String>),
    Test(
        Result<
            (
                studio_lab::PublicConfig,
                Result<studio_lab::ModelTest, String>,
            ),
            String,
        >,
    ),
    Models(Result<(studio_lab::PublicConfig, Result<Vec<String>, String>), String>),
    Artifacts(Result<Vec<studio_lab::LabArtifact>, String>),
    Report(String, Result<serde_json::Value, String>),
    Note(Result<String, String>),
    CloudModels(
        String,
        String,
        Result<Vec<studio_delivery::StudioModel>, String>,
    ),
    Phone(String, Option<String>, Result<String, String>),
    Cloud(
        String,
        String,
        String,
        Result<studio_delivery::CloudReceipt, String>,
    ),
}
#[derive(Default)]
pub(super) struct CloudFeedback {
    pub note: Option<String>,
    pub receipt: Option<studio_delivery::CloudReceipt>,
    pub recovery_url: Option<String>,
    pub error: Option<String>,
}
pub(super) struct StudioLabUiState {
    pub loaded: bool,
    pub config: studio_lab::PublicConfig,
    pub protocol: u8,
    pub models: Vec<String>,
    pub model_note: Option<String>,
    pub pending_config: Option<(String, u8, String)>,
    pub artifacts: Vec<studio_lab::LabArtifact>,
    pub history_page: usize,
    pub selected: Option<usize>,
    pub handle: Option<studio_lab::LabHandle>,
    pub events: Vec<String>,
    pub status: String,
    pub error: Option<String>,
    pub source: Option<PathBuf>,
    pub source_name: Option<String>,
    pub prompt_scroll: ClippedScrollStateHandle,
    pub report: Option<(String, serde_json::Value)>,
    pub report_loading: Option<String>,
    pub report_error: Option<String>,
    pub report_open: bool,
    pub cloud_models: Vec<studio_delivery::StudioModel>,
    pub cloud_model: Option<String>,
    pub cloud_scope: Option<String>,
    pub phone_feedback: std::collections::HashMap<(String, String), Result<String, String>>,
    pub cloud_feedback: std::collections::HashMap<(String, String, String), CloudFeedback>,
    pub last_prompt: String,
    pub artifact_prompts: std::collections::HashMap<String, String>,
}
impl Default for StudioLabUiState {
    fn default() -> Self {
        let config = studio_lab::load_config().unwrap_or_default();
        let protocol = u8::from(config.protocol == "anthropic");
        let artifacts = studio_lab::list_artifacts().unwrap_or_default();
        let selected = (!artifacts.is_empty()).then_some(0);
        let report = artifacts
            .first()
            .and_then(|a| studio_lab::read_report(a).ok().map(|r| (a.id.clone(), r)));
        Self {
            loaded: !artifacts.is_empty(),
            config,
            protocol,
            models: Vec::new(),
            model_note: None,
            pending_config: None,
            artifacts,
            history_page: 0,
            selected,
            handle: None,
            events: Vec::new(),
            status: String::new(),
            error: None,
            source: None,
            source_name: None,
            prompt_scroll: ClippedScrollStateHandle::default(),
            report,
            report_loading: None,
            report_error: None,
            report_open: false,
            cloud_models: Vec::new(),
            cloud_model: None,
            cloud_scope: None,
            phone_feedback: std::collections::HashMap::new(),
            cloud_feedback: std::collections::HashMap::new(),
            last_prompt: String::new(),
            artifact_prompts: std::collections::HashMap::new(),
        }
    }
}
pub(super) const LAB_TEMPLATES: [(&str, &str); 8] = [
    (
        "贪吃蛇大作战",
        "制作一款适合手机竖屏(390×844)与桌面自适应的深色霓虹贪吃蛇 HTML5 Canvas 游戏。包含实时积分、最高纪录(localStorage)、氮气加速双倍积分开关、暂停/继续与重开按钮；同时提供键盘方向键/WASD控制与屏幕底部触控方向盘(上/下/左/右)，确保手机和电脑都能100%流畅试玩。",
    ),
    (
        "2048 益智方块",
        "制作一款赛博深色风格的 4×4 2048 益智方块游戏。实现完整的四向滑动合并算法、当前积分与最高分(localStorage)、悔棋一步(Undo)和新开一局按钮；同时支持键盘方向键、触摸屏手势滑动以及屏幕四向按钮操作。",
    ),
    (
        "霓虹打砖块",
        "制作一款复古街机风的霓虹打砖块 HTML5 Canvas 游戏。包含多排彩色能量砖块、挡板反弹物理角度计算、连击倍率积分、生命值显示、开始/暂停/重开按钮，支持鼠标/触摸拖拽挡板以及左右方向键与屏幕左右触控按钮。",
    ),
    (
        "太空弹幕射击",
        "制作一款竖屏太空战机弹幕射击小游戏。包含星空粒子背景、自动连发激光、敌机波次生成、护盾能量条、得分与最高分统计、暂停与重开按钮；支持触屏拖拽战机、键盘方向键以及屏幕左/右/护盾技能按钮。",
    ),
    (
        "番茄专注时钟",
        "制作一款适合手机和电脑的离线番茄专注时钟 App。顶部包含 SVG 环形倒计时进度圈、25min深度专注与5min休息模式切换、开始/暂停/重置按钮及已完成番茄计数器；底部包含可新增、勾选完成、删除并保存在 localStorage 的今日专注待办清单。",
    ),
    (
        "随手记账看板",
        "制作一款适合手机竖屏的随手记账与月度预算看板 App。顶部展示本月已支出总额、月度预算进度条与四大分类(餐饮/交通/数码/娱乐)汇总卡片；中部提供快速记账表单(金额、分类、备注)；底部支持按分类筛选与删除账单，并持久化到 localStorage。",
    ),
    (
        "每日习惯打卡",
        "制作一款深色极简风的每日习惯打卡与连续天数追踪 App。支持新增习惯(如早起饮水、阅读30分钟、有氧运动)、点击今日打卡自动累加连续打卡天数(Streak)、展示今日完成率进度条与近7天打卡热力矩阵，并持久化到 localStorage。",
    ),
    (
        "背屏点阵控制器",
        "制作一款 StepX 23×17 背屏光显点阵图案编辑器与控制器 App。提供 23×17 可点击/涂抹绘制的 LED 像素网格、亮度滑块、跑马灯滚动预览、4款内置预设图案(赛博闪电/猫咪眨眼/专注时钟/能量爱心)一键切换与导出 JSON 点阵数据。",
    ),
];

impl Companion {
    pub(super) fn studio_lab_phone_note(&self) -> Option<&Result<String, String>> {
        self.studio_lab.phone_feedback.get(&(
            self.selected.as_ref()?.serial.clone(),
            self.studio_lab_selected()?.id.clone(),
        ))
    }
    pub(super) fn studio_lab_cloud_feedback(&self) -> Option<&CloudFeedback> {
        let web = self.studio.web.as_ref().filter(|w| w.authorized)?;
        self.studio_lab.cloud_feedback.get(&(
            self.selected.as_ref()?.serial.clone(),
            web.scope.clone(),
            self.studio_lab_selected()?.id.clone(),
        ))
    }
    pub(super) fn studio_lab_action_allowed(&self, action: &StudioLabAction) -> bool {
        action.available(self.busy, self.studio_lab.handle.is_some())
    }
    pub(super) fn studio_lab_config_dirty(&self) -> bool {
        self.fields[F_LAB_ENDPOINT]
            .text
            .trim()
            .trim_end_matches('/')
            != self.studio_lab.config.endpoint.trim_end_matches('/')
            || self.fields[F_LAB_MODEL].text.trim() != self.studio_lab.config.model
            || !self.fields[F_LAB_KEY].text.is_empty()
            || (if self.studio_lab.protocol == 1 {
                "anthropic"
            } else {
                "openai"
            }) != self.studio_lab.config.protocol
    }
    pub(super) fn studio_lab_selected(&self) -> Option<&studio_lab::LabArtifact> {
        self.studio_lab
            .selected
            .and_then(|i| self.studio_lab.artifacts.get(i))
    }
    pub(super) fn studio_lab_prompt_for(&self, a: &studio_lab::LabArtifact) -> String {
        if let Some(p) = self
            .studio_lab
            .artifact_prompts
            .get(&a.id)
            .filter(|s| !s.trim().is_empty())
        {
            return p.clone();
        }
        match a.id.as_str() {
            "starter-neon-snake" => return LAB_TEMPLATES[0].1.into(),
            "starter-cyber-2048" => return LAB_TEMPLATES[1].1.into(),
            "starter-pomodoro-app" => return LAB_TEMPLATES[4].1.into(),
            "starter-expense-app" => return LAB_TEMPLATES[5].1.into(),
            _ => {}
        }
        if !self.studio_lab.last_prompt.trim().is_empty() {
            return self.studio_lab.last_prompt.clone();
        }
        format!(
            "请继续改进离线应用「{}」（第 {} 轮 · {}），保留已有核心功能并优化交互体验。",
            a.name, a.round, a.summary
        )
    }
    pub(super) fn tick_studio_lab(&mut self) -> bool {
        let mut changed = false;
        let mut events = Vec::new();
        let mut disconnected = false;
        if let Some(handle) = &self.studio_lab.handle {
            loop {
                match handle.events.try_recv() {
                    Ok(event) => events.push(event),
                    Err(std::sync::mpsc::TryRecvError::Empty) => break,
                    Err(std::sync::mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            }
        }
        for event in events {
            changed = true;
            match event {
                studio_lab::LabEvent::Phone(event) => match event {
                    stepx_companion::work::WorkEvent::SessionReady(key) => {
                        self.workspace_bind_session(&key);
                        if !self.amoo_sessions.iter().any(|s| s.session_key == key) {
                            self.apply(Outcome::AmooCreated(key));
                        }
                    }
                    stepx_companion::work::WorkEvent::ModelResponse {
                        data,
                        key,
                        run_id,
                        elapsed_ms,
                    } => {
                        if self.amoo_ui.account_scope == data.account_scope {
                            self.record_work_response(&data, &key, &run_id, elapsed_ms);
                        }
                    }
                    _ => {}
                },
                studio_lab::LabEvent::Status(note) => {
                    self.studio_lab.status = note.clone();
                    self.studio_lab.events.push(note);
                    if self.studio_lab.events.len() > 40 {
                        self.studio_lab.events.remove(0);
                    }
                }
                studio_lab::LabEvent::Artifact(artifact) => {
                    self.studio_lab_receive_artifact(artifact);
                }
                studio_lab::LabEvent::Complete(artifact) => {
                    self.studio_lab_receive_artifact(artifact);
                    // Only the terminal event releases the handle: dropping a live
                    // handle cancels its worker, even when an earlier round passed.
                    self.studio_lab.handle = None;
                    self.studio_lab.status = "作品已完成，本机自动检查通过".into();
                    self.workspace_finish(
                        stepx_companion::amoo_workspace::Phase::Done,
                        self.studio_lab.status.clone(),
                    );
                    self.studio_lab.error = None;
                }
                studio_lab::LabEvent::Failed(note) => {
                    self.workspace_finish(
                        stepx_companion::amoo_workspace::Phase::Failed,
                        note.clone(),
                    );
                    self.studio_lab.handle = None;
                    self.studio_lab.error = Some(note.clone());
                    self.studio_lab.status = note;
                }
                studio_lab::LabEvent::Cancelled => {
                    self.workspace_finish(
                        stepx_companion::amoo_workspace::Phase::Stopped,
                        "已停止，已生成版本保留在电脑".into(),
                    );
                    self.studio_lab.handle = None;
                    self.studio_lab.status = "已停止，已生成的版本保留在本机".into();
                }
            }
        }
        if disconnected && self.studio_lab.handle.is_some() {
            self.studio_lab.handle = None;
            self.studio_lab.error =
                Some("创作进程已结束，未收到完成结果；已保存的版本仍可检查与继续改进".into());
            self.studio_lab.status = "任务未完成".into();
            self.workspace_finish(
                stepx_companion::amoo_workspace::Phase::Failed,
                self.studio_lab.error.clone().unwrap_or_default(),
            );
            changed = true;
        }
        if !matches!(self.page, Page::Studio | Page::Amoo | Page::Settings) || self.busy {
            return changed;
        }
        if self.fields[F_LAB_ENDPOINT].text.is_empty()
            && !self.studio_lab.config.endpoint.is_empty()
            && self.focus != Some(F_LAB_ENDPOINT)
        {
            self.fields[F_LAB_ENDPOINT].set(&self.studio_lab.config.endpoint);
        }
        if self.fields[F_LAB_MODEL].text.is_empty()
            && !self.studio_lab.config.model.is_empty()
            && self.focus != Some(F_LAB_MODEL)
        {
            self.fields[F_LAB_MODEL].set(&self.studio_lab.config.model);
        }
        if !self.studio_lab.loaded {
            self.studio_lab.loaded = true;
            if self.demo_mode || self.page == Page::Amoo {
                if !self.demo_mode {
                    if let Ok(config) = studio_lab::load_config() {
                        self.fields[F_LAB_ENDPOINT].set(&config.endpoint);
                        self.fields[F_LAB_MODEL].set(&config.model);
                        self.studio_lab.protocol = u8::from(config.protocol == "anthropic");
                        self.studio_lab.config = config;
                    }
                }
                if let Ok(artifacts) = studio_lab::list_artifacts() {
                    self.studio_lab.artifacts = artifacts;
                    self.studio_lab.selected = (!self.studio_lab.artifacts.is_empty()).then_some(0);
                    if let Some(a) = self.studio_lab_selected().cloned() {
                        if let Ok(report) = studio_lab::read_report(&a) {
                            self.studio_lab.report = Some((a.id, report));
                        }
                    }
                }
                return true;
            }
            self.submit("正在读取作品…", |_| {
                Ok(Outcome::StudioLab(StudioLabOutcome::Loaded(
                    (|| Ok((studio_lab::load_config()?, studio_lab::list_artifacts()?)))()
                        .map_err(|e: anyhow::Error| e.to_string()),
                )))
            });
            return true;
        }
        if let Some(artifact) = self.studio_lab_selected().cloned() {
            if self
                .studio_lab
                .report
                .as_ref()
                .is_none_or(|(id, _)| id != &artifact.id)
                && self.studio_lab.report_loading.as_ref() != Some(&artifact.id)
            {
                if self.demo_mode || self.page == Page::Amoo {
                    if let Ok(report) = studio_lab::read_report(&artifact) {
                        self.studio_lab.report = Some((artifact.id, report));
                        return true;
                    }
                    return changed;
                }
                self.studio_lab.report_loading = Some(artifact.id.clone());
                self.submit("正在读取作品验证结果…", move |_| {
                    Ok(Outcome::StudioLab(StudioLabOutcome::Report(
                        artifact.id.clone(),
                        studio_lab::read_report(&artifact).map_err(|e| e.to_string()),
                    )))
                });
                return true;
            }
        }
        changed
    }
    fn studio_lab_receive_artifact(&mut self, artifact: studio_lab::LabArtifact) {
        if !self.studio_lab.last_prompt.trim().is_empty() {
            self.studio_lab
                .artifact_prompts
                .insert(artifact.id.clone(), self.studio_lab.last_prompt.clone());
        }
        self.workspace_output(stepx_companion::amoo_workspace::Output {
            path: artifact.html_path.clone(),
            name: artifact.name.clone(),
            kind: if artifact.passed {
                "应用 · 已检查"
            } else {
                "应用 · 待修复"
            }
            .into(),
            lab_id: Some(artifact.id.clone()),
        });
        self.studio_lab.status = artifact.summary.clone();
        self.studio_lab.artifacts.retain(|a| a.id != artifact.id);
        self.studio_lab.artifacts.insert(0, artifact);
        self.studio_lab.selected = Some(0);
        self.studio_lab.report = None;
        self.studio_lab.report_loading = None;
        self.studio_lab.report_error = None;
    }
    fn studio_lab_store_action(&mut self, kind: u8) {
        let endpoint = self.fields[F_LAB_ENDPOINT].text.clone();
        let model = self.fields[F_LAB_MODEL].text.clone();
        let mut key = self.fields[F_LAB_KEY].text.clone();
        if kind != 3
            && key.trim().is_empty()
            && !self.studio_lab.config.has_key
            && (endpoint.contains("127.0.0.1")
                || endpoint.contains("localhost")
                || endpoint.contains("11434"))
        {
            key = "ollama".into();
        }
        let protocol = if self.studio_lab.protocol == 1 {
            "anthropic"
        } else {
            "openai"
        };
        self.studio_lab.model_note = None;
        self.studio_lab.pending_config =
            Some((endpoint.clone(), self.studio_lab.protocol, model.clone()));
        self.submit(
            match kind {
                1 => "正在验证模型实际响应…",
                2 => "正在读取模型目录…",
                _ => "正在保存模型设置…",
            },
            move |_| {
                let saved = studio_lab::save_config(&endpoint, protocol, &model, &key, kind == 3);
                let result = match kind {
                    1 => StudioLabOutcome::Test(
                        saved
                            .map(|c| (c, studio_lab::test_connection().map_err(|e| e.to_string())))
                            .map_err(|e| e.to_string()),
                    ),
                    2 => StudioLabOutcome::Models(
                        saved
                            .map(|c| (c, studio_lab::list_models().map_err(|e| e.to_string())))
                            .map_err(|e| e.to_string()),
                    ),
                    _ => StudioLabOutcome::Config(saved.map_err(|e| e.to_string())),
                };
                Ok(Outcome::StudioLab(result))
            },
        );
    }
    pub(super) fn studio_lab_start(&mut self) -> bool {
        self.studio_lab.error = None;
        if self.studio_lab.handle.is_some() {
            return true;
        }
        let use_phone = !self.studio_lab.config.has_key;
        if !use_phone && (self.studio_lab.config.model.is_empty() || self.studio_lab_config_dirty())
        {
            self.studio_lab.error = Some("需要先填写模型服务".into());
            return true;
        }
        let prompt = self.fields[F_LAB_PROMPT].text.trim().to_string();
        if prompt.is_empty() {
            return false;
        }
        self.studio_lab.last_prompt = prompt.clone();
        self.studio_lab.events.clear();
        self.studio_lab.status = "正在连接模型，开始创作…".into();
        let phone = if use_phone {
            match self.selected.clone().zip(Adb::discover().ok()) {
                Some((device, adb)) => Some((adb, device)),
                None if self.demo_mode => {
                    return true;
                }
                None => {
                    self.studio_lab.error = Some("请先连接手机".into());
                    return true;
                }
            }
        } else {
            None
        };
        self.studio_lab.handle = Some(studio_lab::start_with_phone(
            studio_lab::LabRequest {
                prompt,
                source: self.studio_lab.source.clone(),
            },
            phone,
        ));
        true
    }
    pub(super) fn handle_studio_lab_action(
        &mut self,
        action: &StudioLabAction,
        ctx: &mut ViewContext<Self>,
    ) {
        use StudioLabAction as A;
        if !self.studio_lab_action_allowed(action) {
            return;
        }
        self.studio_lab.error = None;
        self.error = false;
        match action {
            A::Protocol(v) => {
                self.studio_lab.protocol = (*v).min(1);
                self.trigger_subtab_switch();
            }
            A::ApplyProviderPreset(idx) => {
                let (endpoint, model, proto, note) = match idx {
                    0 => (
                        "https://api.stepfun.com/v1",
                        "step-2-16k",
                        0,
                        "已填入阶跃星辰 StepFun 接口预设，请粘贴 API Key 后点击「保存设置」",
                    ),
                    1 => (
                        "https://api.deepseek.com/v1",
                        "deepseek-chat",
                        0,
                        "已填入 DeepSeek 接口预设，请粘贴 API Key 后点击「保存设置」",
                    ),
                    2 => (
                        "https://dashscope.aliyuncs.com/compatible-mode/v1",
                        "qwen-plus",
                        0,
                        "已填入通义千问 DashScope 兼容接口预设，请粘贴 API Key 后点击「保存设置」",
                    ),
                    3 => (
                        "https://api.moonshot.cn/v1",
                        "moonshot-v1-32k",
                        0,
                        "已填入 Kimi (Moonshot) 接口预设，请粘贴 API Key 后点击「保存设置」",
                    ),
                    4 => (
                        "https://open.bigmodel.cn/api/paas/v4",
                        "glm-4-plus",
                        0,
                        "已填入智谱 GLM 接口预设，请粘贴 API Key 后点击「保存设置」",
                    ),
                    5 => (
                        "https://api.openai.com/v1",
                        "gpt-4o",
                        0,
                        "已填入 OpenAI 官方接口预设，请粘贴 API Key 后点击「保存设置」",
                    ),
                    6 => (
                        "https://api.anthropic.com",
                        "claude-3-5-sonnet-latest",
                        1,
                        "已填入 Anthropic Claude 接口预设，请粘贴 API Key 后点击「保存设置」",
                    ),
                    _ => (
                        "http://127.0.0.1:11434/v1",
                        "qwen2.5:7b",
                        0,
                        "已填入本机 Ollama (127.0.0.1:11434) 免密钥预设，可直接点击「保存设置」或「读取模型」",
                    ),
                };
                self.fields[F_LAB_ENDPOINT].set(endpoint);
                self.fields[F_LAB_MODEL].set(model);
                self.studio_lab.protocol = proto;
                self.studio_lab.model_note = Some(note.into());
                self.status = note.into();
                self.trigger_subtab_switch();
            }
            A::Save => self.studio_lab_store_action(0),
            A::Test => self.studio_lab_store_action(1),
            A::Models => self.studio_lab_store_action(2),
            A::ClearKey => self.studio_lab_store_action(3),
            A::Model(name) => self.fields[F_LAB_MODEL].set(name),
            A::Start => {
                if !self.studio_lab_start() {
                    self.focus_field(F_LAB_PROMPT, ctx);
                }
            }
            A::Stop => {
                if let Some(handle) = &self.studio_lab.handle {
                    handle.cancel.store(true, Ordering::SeqCst);
                    self.studio_lab.status = "正在停止任务…".into();
                }
            }
            A::HistoryPage(page) => {
                self.studio_lab.history_page =
                    (*page).min(self.studio_lab.artifacts.len().saturating_sub(1) / 6);
                self.trigger_subtab_switch();
            }
            A::Fresh => {
                self.studio.tab = 2;
                self.studio_lab.events.clear();
                self.studio_lab.selected = None;
                self.studio_lab.report = None;
                self.studio_lab.report_loading = None;
                self.studio_lab.report_error = None;
                self.trigger_subtab_switch();
                if self.studio_lab.handle.is_some() {
                    return;
                }
                self.studio_lab.source = None;
                self.studio_lab.source_name = None;
                self.fields[F_LAB_PROMPT].clear();
                self.studio_lab.status.clear();
                self.focus_field(F_LAB_PROMPT, ctx);
            }
            A::Template(i) => {
                if self.studio_lab.handle.is_some() {
                    return;
                }
                if let Some((_, prompt)) = LAB_TEMPLATES.get(*i) {
                    self.studio.tab = 2;
                    self.studio_lab.source = None;
                    self.studio_lab.source_name = None;
                    self.fields[F_LAB_PROMPT].set(prompt);
                    self.trigger_subtab_switch();
                    self.focus_field(F_LAB_PROMPT, ctx);
                }
            }
            A::Select(i) => {
                if *i < self.studio_lab.artifacts.len() {
                    self.studio_lab.selected = Some(*i);
                    self.studio_lab.report = None;
                    self.studio_lab.report_loading = None;
                    self.studio_lab.report_error = None;
                    self.trigger_subtab_switch();
                }
            }
            A::Iterate => {
                if let Some(a) = self.studio_lab_selected().cloned() {
                    self.studio_lab.source = Some(a.html_path);
                    self.studio_lab.source_name = Some(a.name);
                    self.fields[F_LAB_PROMPT].clear();
                    self.focus_field(F_LAB_PROMPT, ctx);
                }
            }
            A::ReusePrompt(i) | A::OpenInWorkbench(i) => {
                if let Some(a) = self.studio_lab.artifacts.get(*i).cloned() {
                    self.studio.tab = 2;
                    self.studio_lab.selected = Some(*i);
                    self.studio_lab.report = None;
                    self.studio_lab.report_loading = None;
                    self.studio_lab.report_error = None;
                    let prompt = self.studio_lab_prompt_for(&a);
                    self.studio_lab.source = Some(a.html_path);
                    self.studio_lab.source_name = Some(a.name.clone());
                    self.fields[F_LAB_PROMPT].set(&prompt);
                    self.trigger_subtab_switch();
                    self.focus_field(F_LAB_PROMPT, ctx);
                    self.status = format!("已在工作台载入「{}」，可立即预览或继续改进", a.name);
                }
            }
            A::ImportFromWork => {
                if self.studio_lab.handle.is_some() {
                    return;
                }
                self.studio.tab = 2;
                self.studio_lab.source = None;
                self.studio_lab.source_name = None;
                let latest = self
                    .work
                    .artifacts
                    .first()
                    .map(|a| (a.name.clone(), a.path.clone()))
                    .or_else(|| {
                        self.work
                            .files
                            .iter()
                            .find(|f| !f.is_dir)
                            .map(|f| (f.name.clone(), f.path.clone()))
                    });
                let prompt = if let Some((name, path)) = latest {
                    let snippet = std::fs::read_to_string(&path)
                        .ok()
                        .map(|s| s.chars().take(2000).collect::<String>())
                        .filter(|s| !s.trim().is_empty());
                    if let Some(text) = snippet {
                        format!(
                            "请基于以下专业交付文档/数据（{}），制作一个可交互的移动端数据看板与实用应用：\n\n{}",
                            name,
                            text.trim()
                        )
                    } else {
                        format!(
                            "请基于以下专业交付文档/数据（文件：{}），制作一个可交互的移动端数据看板与实用应用：\n包含关键指标卡片、分类筛选、交互式图表和本地数据持久化。",
                            name
                        )
                    }
                } else {
                    "请基于以下专业交付文档/数据，制作一个可交互的移动端数据看板与实用应用：\n1. 顶部展示核心 KPI 指标卡片与达成率进度条；\n2. 支持按分类/周期筛选明细数据并实时重算汇总；\n3. 支持新增记录、状态标记与本地 localStorage 持久化；\n4. 适配 390×844 手机竖屏与桌面深色主题。".into()
                };
                self.fields[F_LAB_PROMPT].set(&prompt);
                self.focus_field(F_LAB_PROMPT, ctx);
                self.status = "已导入专业交付数据看板需求，可点击开始创作生成交互应用".into();
            }
            A::ToggleReport => self.studio_lab.report_open = !self.studio_lab.report_open,
            A::DeleteArtifactAt(i) => {
                if *i < self.studio_lab.artifacts.len() {
                    let removed = self.studio_lab.artifacts.remove(*i);
                    remove_artifact_root_dir(&removed.root);
                    if self.studio_lab.selected == Some(*i) {
                        self.studio_lab.selected =
                            (!self.studio_lab.artifacts.is_empty()).then_some(0);
                    } else if let Some(sel) = self.studio_lab.selected {
                        if sel > *i {
                            self.studio_lab.selected = Some(sel - 1);
                        }
                    }
                    self.status = format!("已删除本机作品「{}」", removed.name);
                }
            }
            A::Refresh => self.submit("正在读取本机作品…", |_| {
                Ok(Outcome::StudioLab(StudioLabOutcome::Artifacts(
                    studio_lab::list_artifacts().map_err(|e| e.to_string()),
                )))
            }),
            A::PreviewAt(i) => {
                if *i < self.studio_lab.artifacts.len() {
                    self.studio_lab.selected = Some(*i);
                    self.studio_lab.report = None;
                    self.studio_lab.report_loading = None;
                    self.studio_lab.report_error = None;
                    self.handle_studio_lab_action(&A::Preview, ctx);
                }
            }
            A::Preview => {
                if let Some(a) = self.studio_lab_selected().cloned() {
                    self.submit("正在打开交互预览…", move |_| {
                        Ok(Outcome::StudioLab(StudioLabOutcome::Note(
                            studio_lab::open_preview_window(&a)
                                .map(|_| "已打开独立离线预览窗口，可实际操作试玩".into())
                                .map_err(|e| e.to_string()),
                        )))
                    });
                }
            }
            A::OpenFolder => {
                if let Some(a) = self.studio_lab_selected().cloned() {
                    self.submit("正在打开作品目录…", move |_| {
                        let cmd = if cfg!(target_os = "macos") {
                            "open"
                        } else if cfg!(windows) {
                            "explorer"
                        } else {
                            "xdg-open"
                        };
                        let result = std::process::Command::new(cmd)
                            .arg(a.root)
                            .spawn()
                            .map(|mut child| {
                                std::thread::spawn(move || {
                                    let _ = child.wait();
                                });
                                "已打开作品目录".into()
                            })
                            .map_err(|_| "无法打开作品目录".into());
                        Ok(Outcome::StudioLab(StudioLabOutcome::Note(result)))
                    });
                }
            }
            A::CloudModels => {
                let Some(web) = self.studio.web.as_ref().filter(|w| w.authorized) else {
                    self.studio_lab.error = Some("请先连接 Studio 官网并同步项目".into());
                    self.studio.tab = 3;
                    return;
                };
                let scope = web.scope.clone();
                self.with_device("正在读取官网模型…", move |adb, d| {
                    Ok(Outcome::StudioLab(StudioLabOutcome::CloudModels(
                        d.serial.clone(),
                        scope.clone(),
                        studio_delivery::cloud_models(adb, &d, &scope).map_err(|e| e.to_string()),
                    )))
                });
            }
            A::CloudModel(model) => {
                if self.studio_lab.cloud_models.iter().any(|m| &m.id == model) {
                    self.studio_lab.cloud_model = Some(model.clone());
                }
            }
            A::DeliverPhoneAt(i) => {
                if *i < self.studio_lab.artifacts.len() {
                    self.studio_lab.selected = Some(*i);
                    self.studio_lab.report = None;
                    self.studio_lab.report_loading = None;
                    self.studio_lab.report_error = None;
                    self.handle_studio_lab_action(&A::DeliverPhone, ctx);
                }
            }
            A::DeliverPhone => {
                let Some(a) = self
                    .studio_lab_selected()
                    .filter(|a| a.html_path.is_file())
                    .cloned()
                else {
                    self.studio_lab.error = Some("请先选择有效的本机作品文件".into());
                    return;
                };
                self.with_device(
                    "正在发送并打开手机离线作品…",
                    move |adb, d| {
                        let result = (|| {
                            let html = studio_lab::verified_html(&a)?;
                            let r = studio_delivery::install_local(adb, &d, &a.name, &html)?;
                            studio_delivery::open_local(adb, &d, &r.id)?;
                            Ok(format!(
                                "「{}」已发送到手机并打开；可在伴侣作品库随时离线试玩",
                                a.name
                            ))
                        })();
                        Ok(Outcome::StudioLab(StudioLabOutcome::Phone(
                            d.serial,
                            Some(a.id),
                            result.map_err(|e: anyhow::Error| e.to_string()),
                        )))
                    },
                );
            }
            A::SyncCloud => {
                if self
                    .studio_lab_cloud_feedback()
                    .is_some_and(|f| f.receipt.is_some() || f.recovery_url.is_some())
                {
                    self.handle_studio_lab_action(&A::OpenCloud, ctx);
                    return;
                }
                let Some(a) = self.studio_lab_selected().filter(|a| a.passed).cloned() else {
                    self.studio_lab.error = Some("请先选择自动检查通过的作品".into());
                    return;
                };
                let Some(web) = self.studio.web.as_ref().filter(|w| w.authorized) else {
                    self.studio_lab.error = Some("请先连接官网".into());
                    return;
                };
                let scope = web.scope.clone();
                let Some(model) = self
                    .studio_lab
                    .cloud_model
                    .clone()
                    .filter(|_| self.studio_lab.cloud_scope.as_ref() == Some(&scope))
                else {
                    self.studio_lab.error = Some("请先读取并选择官网模型".into());
                    return;
                };
                self.with_device(
                    "正在新建官网项目并回读验证源码…",
                    move |adb, d| {
                        let result = (|| {
                            let html = studio_lab::verified_html(&a)?;
                            studio_delivery::cloud_sync_new(adb, &d, &scope, &a.name, &html, &model)
                        })();
                        Ok(Outcome::StudioLab(StudioLabOutcome::Cloud(
                            d.serial,
                            scope,
                            a.id,
                            result.map_err(|e: anyhow::Error| format!("{e:#}")),
                        )))
                    },
                );
            }
            A::OpenCloud => {
                if let Some(url) = self.studio_lab_cloud_feedback().and_then(|f| {
                    f.receipt
                        .as_ref()
                        .map(|r| r.editor_url.clone())
                        .or_else(|| f.recovery_url.clone())
                }) {
                    self.with_device("正在连接官网编辑器…", move |adb, d| {
                        Ok(Outcome::StudioLab(StudioLabOutcome::Phone(
                            d.serial.clone(),
                            None,
                            stepx_companion::studio::open_editor(adb, &d, &url)
                                .map_err(|e| e.to_string()),
                        )))
                    });
                }
            }
        }
    }
    fn studio_lab_apply_saved_config(&mut self, config: studio_lab::PublicConfig) {
        self.fields[F_LAB_KEY].clear();
        self.record_notice(
            "模型服务已保存",
            "后续任务将使用已确认的模型服务；密钥仅保存在本机。",
        );
        if self
            .studio_lab
            .pending_config
            .take()
            .is_some_and(|(endpoint, protocol, model)| {
                self.fields[F_LAB_ENDPOINT].text == endpoint
                    && self.studio_lab.protocol == protocol
                    && self.fields[F_LAB_MODEL].text == model
            })
        {
            self.fields[F_LAB_ENDPOINT].set(&config.endpoint);
            self.fields[F_LAB_MODEL].set(&config.model);
            self.studio_lab.protocol = u8::from(config.protocol == "anthropic");
        }
        self.studio_lab.config = config;
    }
    pub(super) fn apply_studio_lab(&mut self, outcome: StudioLabOutcome) {
        use StudioLabOutcome as O;
        let mut error = None;
        match outcome {
            O::Loaded(result) => match result {
                Ok((config, artifacts)) => {
                    self.fields[F_LAB_ENDPOINT].set(&config.endpoint);
                    self.fields[F_LAB_MODEL].set(&config.model);
                    self.studio_lab.protocol = u8::from(config.protocol == "anthropic");
                    self.studio_lab.config = config;
                    self.studio_lab.artifacts = artifacts;
                    self.studio_lab.selected = (!self.studio_lab.artifacts.is_empty()).then_some(0);
                }
                Err(e) => error = Some(e),
            },
            O::Config(result) => match result {
                Ok(config) => {
                    self.studio_lab_apply_saved_config(config);
                    self.studio_lab.model_note =
                        Some("模型设置已保存，密钥仅存于本机私有配置".into());
                }
                Err(e) => error = Some(e),
            },
            O::Test(result) => match result {
                Ok((config, result)) => {
                    self.studio_lab_apply_saved_config(config);
                    match result {
                        Ok(test) => {
                            self.studio_lab.model_note = Some(format!(
                                "实际请求通过 · {} · {:.1} 秒",
                                test.model,
                                test.elapsed_ms as f64 / 1000.
                            ))
                        }
                        Err(e) => error = Some(e),
                    }
                }
                Err(e) => error = Some(e),
            },
            O::Models(result) => match result {
                Ok((config, result)) => {
                    self.studio_lab_apply_saved_config(config);
                    match result {
                        Ok(models) => {
                            self.studio_lab.model_note = Some(format!(
                                "已读取 {} 个模型；请按模型名筛选并选择，使用前可测试连接",
                                models.len()
                            ));
                            self.studio_lab.models = models;
                        }
                        Err(e) => error = Some(e),
                    }
                }
                Err(e) => error = Some(e),
            },
            O::Artifacts(result) => match result {
                Ok(artifacts) => {
                    let selected = self.studio_lab_selected().map(|a| a.id.clone());
                    self.studio_lab.artifacts = artifacts;
                    self.studio_lab.selected = selected
                        .and_then(|id| self.studio_lab.artifacts.iter().position(|a| a.id == id))
                        .or_else(|| (!self.studio_lab.artifacts.is_empty()).then_some(0));
                    self.studio_lab.report = None;
                    self.studio_lab.report_loading = None;
                    self.studio_lab.report_error = None;
                }
                Err(e) => error = Some(e),
            },
            O::Report(id, result) => {
                if self.studio_lab_selected().is_some_and(|a| a.id == id) {
                    match result {
                        Ok(report) => {
                            self.studio_lab.report = Some((id, report));
                            self.studio_lab.report_error = None;
                        }
                        Err(e) => {
                            self.studio_lab.report_error = Some(e.clone());
                            error = Some(e);
                        }
                    }
                }
            }
            O::Note(result) => match result {
                Ok(note) => {
                    self.status = note.clone();
                }
                Err(e) => error = Some(e),
            },
            O::Phone(serial, artifact_id, result) => {
                let current_device = self.selected.as_ref().is_some_and(|d| d.serial == serial);
                let visible = current_device
                    && artifact_id
                        .as_ref()
                        .is_none_or(|id| self.studio_lab_selected().is_some_and(|a| &a.id == id));
                if visible {
                    match &result {
                        Ok(note) => self.status = note.clone(),
                        Err(e) => error = Some(e.clone()),
                    }
                }
                if let Some(id) = artifact_id {
                    self.workspace_app_receipt(
                        &id,
                        &serial,
                        None,
                        result
                            .clone()
                            .unwrap_or_else(|e| format!("手机交付未完成：{e}")),
                        result.is_ok(),
                        None,
                    );
                    self.studio_lab.phone_feedback.insert((serial, id), result);
                }
            }
            O::CloudModels(serial, scope, result) => {
                if self.selected.as_ref().is_none_or(|d| d.serial != serial)
                    || self.studio.web.as_ref().is_none_or(|w| w.scope != scope)
                {
                    return;
                }
                match result {
                    Ok(models) => {
                        self.studio_lab.cloud_model = models
                            .iter()
                            .find(|m| m.is_default)
                            .or_else(|| models.first())
                            .map(|m| m.id.clone());
                        self.studio_lab.cloud_models = models;
                        self.studio_lab.cloud_scope = Some(scope);
                    }
                    Err(e) => error = Some(e),
                }
            }
            O::Cloud(serial, scope, artifact_id, result) => {
                let visible = self.selected.as_ref().is_some_and(|d| d.serial == serial)
                    && self
                        .studio
                        .web
                        .as_ref()
                        .is_some_and(|w| w.authorized && w.scope == scope)
                    && self
                        .studio_lab_selected()
                        .is_some_and(|a| a.id == artifact_id);
                // Retain an issued write's receipt even if selection changes while it runs.
                // Every result is bound to the device, account and immutable artifact ID.
                let feedback = match result {
                    Ok(receipt) => CloudFeedback {
                        note: Some(format!(
                            "源码已同步并保存版本，已核对 {} 个文件；官网测试与发布另行完成",
                            receipt.files.len()
                        )),
                        receipt: Some(receipt),
                        ..Default::default()
                    },
                    Err(e) => {
                        if visible {
                            error = Some(e.clone());
                        }
                        CloudFeedback {
                            recovery_url: created_editor_url(&e),
                            error: Some(e),
                            ..Default::default()
                        }
                    }
                };
                self.workspace_app_receipt(
                    &artifact_id,
                    &serial,
                    Some(scope.clone()),
                    feedback
                        .note
                        .clone()
                        .or(feedback.error.clone())
                        .unwrap_or_default(),
                    feedback.receipt.is_some(),
                    feedback
                        .receipt
                        .as_ref()
                        .map(|r| r.editor_url.clone())
                        .or(feedback.recovery_url.clone()),
                );
                self.studio_lab
                    .cloud_feedback
                    .insert((serial, scope, artifact_id), feedback);
                self.studio.cloud_checked = None;
            }
        }
        if let Some(error) = error {
            self.status = error.clone();
            self.error = true;
            self.studio_lab.error = Some(error);
        }
    }
}

fn remove_artifact_root_dir(root: &std::path::Path) {
    if root
        .file_name()
        .and_then(|s| s.to_str())
        .is_some_and(|name| name.starts_with("round-"))
    {
        if let Some(parent) = root.parent() {
            let _ = std::fs::remove_dir_all(parent);
            return;
        }
    }
    let _ = std::fs::remove_dir_all(root);
}

// Only a validated official editor route may become a recovery action. Never
// offer retry-as-new after creation succeeded but source upload failed.
fn created_editor_url(error: &str) -> Option<String> {
    let (_, tail) = error.split_once("项目已创建，可继续编辑：")?;
    let url = tail.split_whitespace().next()?.trim_end_matches(':');
    stepx_companion::studio::parse_link(url)
        .ok()
        .filter(|(_, kind)| *kind == stepx_companion::studio::LinkKind::Project)
        .map(|(url, _)| url)
}
#[cfg(test)]
mod tests {
    use super::{created_editor_url, remove_artifact_root_dir};
    #[test]
    fn recovery_link_is_only_an_official_project_route() {
        let url = "https://studio.stepx.com/builder/app/p_1234567890abcdef";
        assert_eq!(
            created_editor_url(&format!("项目已创建，可继续编辑：{url}: 上传失败")),
            Some(url.into())
        );
        assert!(created_editor_url(
            "项目已创建，可继续编辑：https://evil.example/builder/app/p_1234567890abcdef: error"
        )
        .is_none());
        assert!(created_editor_url("HTTP 403").is_none());
    }

    #[test]
    fn deleting_round_artifact_removes_entire_parent_run_directory() {
        let temp = tempfile::tempdir().unwrap();
        let run_dir = temp.path().join("runs").join("starter-analytics-cockpit");
        let round_dir = run_dir.join("round-1");
        std::fs::create_dir_all(&round_dir).unwrap();
        std::fs::write(round_dir.join("index.html"), "<html></html>").unwrap();
        remove_artifact_root_dir(&round_dir);
        assert!(!run_dir.exists());
    }
}

#[cfg(test)]
mod interaction_tests {
    use super::*;
    #[test]
    fn running_creation_cannot_be_reset_or_reconfigured() {
        for action in [
            StudioLabAction::Start,
            StudioLabAction::Fresh,
            StudioLabAction::Template(0),
            StudioLabAction::Iterate,
            StudioLabAction::Save,
            StudioLabAction::ClearKey,
        ] {
            assert!(!action.available(false, true), "{action:?}");
            assert!(action.available(false, false), "{action:?}");
        }
        assert!(StudioLabAction::Stop.available(true, true));
        assert!(StudioLabAction::Select(1).available(true, true));
        assert!(!StudioLabAction::DeliverPhone.available(true, false));
        assert!(!StudioLabAction::SyncCloud.available(true, false));
    }
}
