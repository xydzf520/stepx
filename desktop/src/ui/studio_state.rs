use super::*;
use facloud_ui::{clipboard::ClipboardContent, TypedActionView};
use stepx_companion::studio::{self, CloudLibrary, Work};

#[derive(Clone, Debug)]
pub(super) enum StudioAction {
    Tab(u8),
    Source(u8),
    SetFilter(u8),
    WebsiteLogin,
    WebsiteDisconnect,
    WebSelect(usize),
    WebKeep,
    Edit(bool),
    ListPage(usize),
    PhoneItem(usize),
    KeepWeb(usize),
    UsePhone(usize),
    KeepPhone(usize),
    Category(usize),
    Select(usize),
    New,
    PasteLink,
    Save,
    Refresh,
    Send,
    OpenPhone,
    Pin,
    PhoneLibrary,
    AmooWorks,
    CheckPhone,
    Share,
    SaveQr,
    Brief(usize),
    CloudRefresh,
    ConnectAccount,
    DisconnectAccount,
    CloudCategory(usize),
    CloudSelect(usize),
    CloudOpen,
    CloudKeep,
    /// 「全部成果」里点某个成果：回到它所在的 Amoo 对话。
    BackToTask(String),
    /// 「在 Amoo 中继续改」某个对话成果：切回该对话并选中该成果继续修订。
    ContinueOutputInAmoo(String, usize),
    /// 「在 Amoo 中继续改」某个本机离线应用：关联或创建对话并进入应用继续改进。
    ContinueLabInAmoo(usize),
    /// 打开本机作品/成果文件或目录。
    OpenPath(PathBuf),
    /// 在系统文件管理器中定位该成果所在文件夹。
    RevealPath(PathBuf),
    /// 「新建作品」：回到 Amoo 开一段新对话，直接说要做什么。
    NewInAmoo,
    UseSavedOnPhone(usize),
    DeleteSaved(usize),
    SetDeliverableFilter(u8),
    SetSortMode(u8),
    ImportLocalFile,
    OpenWorksFolder,
}
pub(super) enum StudioOutcome {
    Library(Vec<Work>, Option<String>),
    Qr(String, PathBuf),
    Phone(String, String),
    Cloud(String, Result<CloudLibrary, String>),
    Workspace(
        String,
        Result<CloudLibrary, String>,
        Result<CloudLibrary, String>,
    ),
}
pub(super) struct StudioUiState {
    pub tab: u8,
    pub source: u8,
    pub filter: u8,
    pub deliverable_filter: u8,
    pub sort_mode: u8,
    pub editing: bool,
    pub list_page: usize,
    pub web: Option<CloudLibrary>,
    pub web_error: Option<String>,
    pub web_selected: Option<usize>,
    pub category: usize,
    pub works: Vec<Work>,
    pub selected: Option<usize>,
    pub loaded: bool,
    pub qr: Option<(String, PathBuf)>,
    pub phone_note: Option<String>,
    pub cloud: Option<CloudLibrary>,
    pub cloud_error: Option<String>,
    pub cloud_selected: Option<usize>,
    pub cloud_category: usize,
    pub cloud_checked: Option<Instant>,
    pub connecting_until: Option<Instant>,
}
impl Default for StudioUiState {
    fn default() -> Self {
        Self {
            // 「我的作品」默认展示全部作品（Amoo 对话成果 + 本机离线应用 + 我的收藏）。
            tab: 0,
            source: 3,
            filter: 0,
            deliverable_filter: 0,
            sort_mode: 0,
            editing: false,
            list_page: 0,
            web: None,
            web_error: None,
            web_selected: None,
            category: 0,
            works: Vec::new(),
            selected: None,
            loaded: false,
            qr: None,
            phone_note: None,
            cloud: None,
            cloud_error: None,
            cloud_selected: None,
            cloud_category: 3,
            cloud_checked: None,
            connecting_until: None,
        }
    }
}
pub(super) const CATEGORIES: [&str; 3] = ["应用", "智能体", "技能"];
pub(super) const BRIEFS: [(&str,&str,&str);3] = [
    ("随手记账", "应用 · 单手操作、离线记录、导出数据", "请在应用构建器制作一个适合 STEPX 手机的轻量记账应用。手机竖屏优先，底部导航不超过三项；支持金额、分类、备注、编辑撤销、本地保存、按月统计及 CSV 导出。清楚区分本地数据与云端数据，第一次打开显示可跳过的引导，不填入假账单。请实际测试添加、修改、导出和重新打开后数据保留，给出可真机测试的版本。"),
    ("差旅随行助手", "智能体 · 行程、待办、必要时提醒", "请在智能体构建器创建差旅随行助手。先询问目的地、日期和用户已提供的行程；优先使用平台可用连接器与技能，输出分日行程、出行清单和待确认事项。手机阅读时结论在前，详情折叠。不能访问的数据明确告知，禁止编造订单或天气；订票、付款和发消息前请用户确认。请测试信息不足、用户修改日期、没有连接器权限三个场景，再提供手机真机测试。"),
    ("票据整理", "技能 · 提取字段、核对异常、交付表格", "请在技能构建器制作票据整理技能，供手机 Amoo 调用。输入用户授权的票据图片或 PDF，提取日期、商户、金额、币种及票据号，标记模糊、重复或缺失字段，不猜测金额。输出适合手机阅读的汇总与可下载 CSV；修改和上传外部系统前需要确认。定义清晰的输入输出，使用平台支持的文件能力，实际验证正常票据、模糊图片和重复票据，并给出真机测试步骤。"),
];
impl Companion {
    pub(super) fn sync_studio_cloud(&mut self) {
        self.studio.cloud_checked = Some(Instant::now());
        self.with_device("正在用手机账号同步 Studio 作品…", |adb, d| {
            let phone = studio::sync_cloud(adb, &d).map_err(|e| e.to_string());
            let website = studio::sync_web(adb, &d).map_err(|e| e.to_string());
            Ok(Outcome::Studio(StudioOutcome::Workspace(
                d.serial.clone(),
                phone,
                website,
            )))
        });
    }
    pub(super) fn tick_studio_cloud(&mut self) {
        if self.page != Page::Studio
            || self.busy
            || self.demo_mode
            || self.selected.is_none()
            || !self.studio.loaded
        {
            return;
        }
        let interval = if self
            .studio
            .connecting_until
            .is_some_and(|t| t > Instant::now())
        {
            3
        } else {
            60
        };
        if self
            .studio
            .cloud_checked
            .is_none_or(|t| t.elapsed() >= Duration::from_secs(interval))
        {
            self.sync_studio_cloud();
        }
    }
    pub(super) fn refresh_studio(&mut self) {
        self.studio.loaded = true;
        if self.page == Page::Amoo {
            if let Ok(works) = studio::load() {
                let prev_status = self.status.clone();
                let prev_error = self.error;
                self.apply_studio(StudioOutcome::Library(works, None));
                self.status = prev_status;
                self.error = prev_error;
            }
            return;
        }
        self.submit("正在读取本机 Studio 作品…", |_| {
            Ok(Outcome::Studio(StudioOutcome::Library(
                studio::load()?,
                None,
            )))
        });
    }
    pub(super) fn handle_studio_action(
        &mut self,
        action: &StudioAction,
        ctx: &mut ViewContext<Self>,
    ) {
        if self.busy
            && !matches!(
                action,
                StudioAction::Tab(_)
                    | StudioAction::Source(_)
                    | StudioAction::SetFilter(_)
                    | StudioAction::CloudCategory(_)
                    | StudioAction::CloudSelect(_)
                    | StudioAction::WebSelect(_)
                    | StudioAction::Select(_)
                    | StudioAction::ListPage(_)
                    | StudioAction::Edit(_)
                    | StudioAction::BackToTask(_)
                    | StudioAction::ContinueOutputInAmoo(..)
                    | StudioAction::ContinueLabInAmoo(_)
                    | StudioAction::OpenPath(_)
                    | StudioAction::RevealPath(_)
                    | StudioAction::AmooWorks
                    | StudioAction::NewInAmoo
                    | StudioAction::DeleteSaved(_)
                    | StudioAction::SetDeliverableFilter(_)
                    | StudioAction::SetSortMode(_)
                    | StudioAction::ImportLocalFile
                    | StudioAction::OpenWorksFolder
            )
        {
            return;
        }
        self.error = false;
        match action {
            StudioAction::SetDeliverableFilter(f) => {
                self.studio.deliverable_filter = (*f).min(5);
                self.studio.list_page = 0;
                self.trigger_subtab_switch();
            }
            StudioAction::SetSortMode(m) => {
                self.studio.sort_mode = (*m).min(2);
                self.studio.list_page = 0;
                self.trigger_subtab_switch();
            }
            StudioAction::OpenWorksFolder => {
                let dir = stepx_companion::studio_lab::runs_directory()
                    .ok()
                    .filter(|p| p.exists())
                    .or_else(|| Some(stepx_companion::settings::downloads()));
                if let Some(folder) = dir {
                    if let Err(e) = super::work_state::open_work_path(&folder) {
                        self.status = e.to_string();
                        self.error = true;
                    }
                }
            }
            StudioAction::ImportLocalFile => {
                let picked = native_dialog::FileDialog::new()
                    .set_title("导入本地作品到我的作品库")
                    .add_filter(
                        "作品与成果文件",
                        &[
                            "pptx", "ppt", "docx", "doc", "xlsx", "xls", "csv", "pdf", "html",
                            "htm", "png", "jpg", "jpeg", "webp", "md", "txt",
                        ],
                    )
                    .show_open_single_file();
                match picked {
                    Ok(Some(src)) if src.is_file() => {
                        let name = src
                            .file_name()
                            .and_then(|s| s.to_str())
                            .unwrap_or("未命名作品")
                            .to_string();
                        let ext = src
                            .extension()
                            .and_then(|s| s.to_str())
                            .unwrap_or("")
                            .to_ascii_lowercase();
                        let dest_dir =
                            stepx_companion::resources::user_state().join("studio-imported");
                        let _ = std::fs::create_dir_all(&dest_dir);
                        let dest_path = dest_dir.join(&name);
                        let final_path =
                            if src != dest_path && std::fs::copy(&src, &dest_path).is_ok() {
                                dest_path
                            } else {
                                src.clone()
                            };
                        let kind = if matches!(ext.as_str(), "ppt" | "pptx") {
                            "PPT 演示"
                        } else if matches!(ext.as_str(), "xls" | "xlsx" | "csv") {
                            "Excel 表格"
                        } else if matches!(ext.as_str(), "html" | "htm") {
                            "应用 · 本机导入"
                        } else if matches!(
                            ext.as_str(),
                            "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg"
                        ) {
                            "图片成果"
                        } else {
                            "文档报告"
                        };
                        let mut task = stepx_companion::amoo_workspace::Task::new(
                            self.selected.as_ref().map(|d| d.serial.clone()),
                            self.amoo_ui.account_scope.clone(),
                        );
                        task.title = format!("本地导入 · {name}");
                        task.add_output(stepx_companion::amoo_workspace::Output {
                            path: final_path.clone(),
                            name: name.clone(),
                            kind: kind.into(),
                            lab_id: None,
                        });
                        self.workspace.journal.tasks.insert(0, task);
                        let journal_path = stepx_companion::resources::user_state()
                            .join("amoo-workspace/tasks.json");
                        let _ = self.workspace.journal.save(&journal_path);
                        self.studio.tab = 0;
                        self.studio.source = 3;
                        self.status = format!("已导入作品「{name}」到我的作品库");
                        self.record_notice("我的作品", self.status.clone());
                        self.motion_started = Instant::now();
                    }
                    Err(e) => {
                        self.status = format!("导入失败：{e}");
                        self.error = true;
                    }
                    _ => {}
                }
            }
            StudioAction::SetFilter(f) => {
                self.studio.filter = (*f).min(2);
                self.studio.source = match *f {
                    1 => 5,
                    2 => 4,
                    _ => 3,
                };
                self.studio.list_page = 0;
                self.scroll = Default::default();
                self.trigger_subtab_switch();
            }
            StudioAction::ListPage(page) => {
                self.studio.list_page = *page;
                self.trigger_subtab_switch();
            }
            StudioAction::PhoneItem(i) => {
                self.studio.cloud_selected = Some(*i);
                self.studio.source = 1;
                self.studio.cloud_category = 3;
                self.fields[F_STUDIO_SEARCH].clear();
                self.trigger_subtab_switch();
            }
            StudioAction::KeepWeb(i) => {
                self.studio.web_selected = Some(*i);
                self.handle_studio_action(&StudioAction::WebKeep, ctx);
            }
            StudioAction::UsePhone(i) => {
                self.studio.cloud_selected = Some(*i);
                self.handle_studio_action(&StudioAction::CloudOpen, ctx);
            }
            StudioAction::KeepPhone(i) => {
                self.studio.cloud_selected = Some(*i);
                self.handle_studio_action(&StudioAction::CloudKeep, ctx);
            }
            StudioAction::Source(source) => {
                self.studio.list_page = 0;
                self.studio.source = (*source).min(5);
                self.studio.filter = 0;
                self.studio.tab = 0;
                self.fields[F_STUDIO_SEARCH].clear();
                self.scroll = Default::default();
                self.trigger_subtab_switch();
            }
            StudioAction::Edit(value) => {
                self.studio.editing = *value;
                self.trigger_subtab_switch();
            }
            StudioAction::WebsiteLogin => {
                self.studio.connecting_until = Some(Instant::now() + Duration::from_secs(300));
                self.studio.cloud_checked = None;
                self.with_device("正在打开手机 Studio 官方登录…", |adb, d| {
                    studio::open_phone(adb, &d, "website", None)?;
                    Ok(Outcome::Studio(StudioOutcome::Phone(
                        d.serial,
                        "请在手机完成官网验证，完成后将自动同步项目".into(),
                    )))
                });
            }
            StudioAction::WebsiteDisconnect => {
                self.studio.web = None;
                self.studio.web_selected = None;
                self.studio.connecting_until = None;
                self.with_device("正在断开 Studio 官网…", |adb, d| {
                    stepx_companion::studio_browser::close_session();
                    studio::phone_call(adb, &d, serde_json::json!({"op":"web-disconnect"}))?;
                    Ok(Outcome::Studio(StudioOutcome::Phone(
                        d.serial,
                        "已断开官网同步并清除伴侣中的网页登录会话".into(),
                    )))
                });
            }
            StudioAction::WebSelect(i) => {
                self.studio.web_selected = Some(*i);
                self.trigger_subtab_switch();
            }
            StudioAction::WebKeep => {
                let Some(w) = self
                    .studio
                    .web
                    .as_ref()
                    .and_then(|c| self.studio.web_selected.and_then(|i| c.items.get(i)))
                    .cloned()
                else {
                    return;
                };
                if let Some(url) = w.editor() {
                    let work = match Work::new(&w.name, &url, &w.category) {
                        Ok(w) => w,
                        Err(e) => {
                            self.status = e.to_string();
                            self.error = true;
                            return;
                        }
                    };
                    self.submit("正在收藏项目…", move |_| {
                        Ok(Outcome::Studio(StudioOutcome::Library(
                            studio::save(work)?,
                            Some(url),
                        )))
                    });
                }
            }
            StudioAction::CloudRefresh => self.sync_studio_cloud(),
            StudioAction::ConnectAccount => {
                self.studio.connecting_until = Some(Instant::now() + Duration::from_secs(120));
                self.studio.cloud_checked = None;
                self.with_device(
                    "请在手机选择账号并完成系统授权…",
                    |adb, d| {
                        studio::open_phone(adb, &d, "account", None)?;
                        Ok(Outcome::Studio(StudioOutcome::Phone(
                            d.serial,
                            "请在手机确认账号授权；完成后自动同步作品".into(),
                        )))
                    },
                );
            }
            StudioAction::DisconnectAccount => {
                self.studio.cloud = None;
                self.studio.cloud_selected = None;
                self.studio.connecting_until = None;
                self.with_device("正在断开作品同步…", |adb, d| {
                    studio::phone_call(adb, &d, serde_json::json!({"op":"cloud-disconnect"}))?;
                    Ok(Outcome::Studio(StudioOutcome::Cloud(
                        d.serial,
                        Ok(CloudLibrary::default()),
                    )))
                });
            }
            StudioAction::CloudCategory(i) => {
                self.studio.list_page = 0;
                self.studio.cloud_category = (*i).min(3);
                self.studio.cloud_selected = self.studio.cloud.as_ref().and_then(|c| {
                    c.items.iter().position(|w| {
                        self.studio.cloud_category == 3
                            || w.category == CATEGORIES[self.studio.cloud_category]
                    })
                });
                self.trigger_subtab_switch();
            }
            StudioAction::CloudSelect(i) => {
                self.studio.cloud_selected = Some(*i);
                self.trigger_subtab_switch();
            }
            StudioAction::CloudOpen | StudioAction::CloudKeep => {
                let Some(cloud) = self.studio.cloud.as_ref().filter(|c| c.authorized) else {
                    return;
                };
                let Some(item) = self
                    .studio
                    .cloud_selected
                    .and_then(|i| cloud.items.get(i))
                    .cloned()
                else {
                    return;
                };
                let scope = cloud.scope.clone();
                let keep = matches!(action, StudioAction::CloudKeep);
                if keep {
                    self.studio.tab = 0;
                    self.studio.source = 2;
                }
                self.with_device("正在核对当前手机账号…", move |adb, d| {
                    let status =
                        studio::phone_call(adb, &d, serde_json::json!({"op":"cloud-status"}))?;
                    anyhow::ensure!(
                        status["authorized"] == true && status["scope"] == scope,
                        "手机账号已变化，请刷新作品后重试"
                    );
                    if let Some(w) = item.experience() {
                        if keep {
                            return Ok(Outcome::Studio(StudioOutcome::Library(
                                studio::save(w.clone())?,
                                Some(w.url),
                            )));
                        }
                        studio::send(adb, &d, &w)?;
                        let note = studio::open_phone(adb, &d, "open", Some(&w.id))?;
                        Ok(Outcome::Studio(StudioOutcome::Phone(d.serial, note)))
                    } else {
                        let note = studio::open_phone(adb, &d, "amoo", None)?;
                        Ok(Outcome::Studio(StudioOutcome::Phone(d.serial, note)))
                    }
                });
            }
            StudioAction::BackToTask(id) => {
                self.handle_action(&Action::Page(Page::Amoo), ctx);
                self.handle_action(&Action::Workspace(WorkspaceAction::Select(id.clone())), ctx);
                self.focus_field(F_AMOO, ctx);
            }
            StudioAction::ContinueOutputInAmoo(task_id, output_idx) => {
                self.handle_action(&Action::Page(Page::Amoo), ctx);
                self.handle_action(
                    &Action::Workspace(WorkspaceAction::Select(task_id.clone())),
                    ctx,
                );
                self.handle_action(
                    &Action::Workspace(WorkspaceAction::ContinueEdit(*output_idx)),
                    ctx,
                );
                self.focus_field(F_AMOO, ctx);
            }
            StudioAction::ContinueLabInAmoo(i) => {
                let Some(a) = self.studio_lab.artifacts.get(*i).cloned() else {
                    return;
                };
                self.handle_action(&Action::Page(Page::Amoo), ctx);
                let existing = self
                    .workspace
                    .journal
                    .tasks
                    .iter()
                    .find(|t| {
                        self.workspace_visible(t)
                            && t.outputs.iter().any(|o| o.lab_id.as_ref() == Some(&a.id))
                    })
                    .map(|t| t.id.clone());
                let task_id = if let Some(id) = existing {
                    id
                } else {
                    let mut task = stepx_companion::amoo_workspace::Task::new(
                        self.selected.as_ref().map(|d| d.serial.clone()),
                        self.amoo_ui.account_scope.clone(),
                    );
                    task.title = a.name.clone();
                    task.route = Some(stepx_companion::amoo_workspace::Intent::App);
                    task.add_output(stepx_companion::amoo_workspace::Output {
                        path: a.html_path.clone(),
                        name: a.name.clone(),
                        kind: if a.passed {
                            "应用 · 已检查"
                        } else {
                            "应用 · 待修复"
                        }
                        .into(),
                        lab_id: Some(a.id.clone()),
                    });
                    let id = task.id.clone();
                    self.workspace.journal.tasks.push(task);
                    id
                };
                self.handle_action(&Action::Workspace(WorkspaceAction::Select(task_id)), ctx);
                if let Some(out_idx) = self.workspace_task().and_then(|t| {
                    t.outputs
                        .iter()
                        .position(|o| o.lab_id.as_ref() == Some(&a.id) || o.path == a.html_path)
                }) {
                    self.handle_action(
                        &Action::Workspace(WorkspaceAction::ContinueEdit(out_idx)),
                        ctx,
                    );
                }
                self.focus_field(F_AMOO, ctx);
            }
            StudioAction::OpenPath(path) => {
                if let Some(idx) = self
                    .studio_lab
                    .artifacts
                    .iter()
                    .position(|a| &a.html_path == path)
                {
                    self.handle_studio_lab_action(&StudioLabAction::PreviewAt(idx), ctx);
                    return;
                }
                if let Err(e) = super::work_state::open_work_path(path) {
                    self.status = e.to_string();
                    self.error = true;
                }
            }
            StudioAction::RevealPath(path) => {
                let folder = if path.is_dir() {
                    path.clone()
                } else {
                    path.parent()
                        .map(std::path::Path::to_path_buf)
                        .unwrap_or_else(|| path.clone())
                };
                if let Err(e) = super::work_state::open_work_path(&folder) {
                    self.status = e.to_string();
                    self.error = true;
                }
            }
            StudioAction::NewInAmoo => {
                self.handle_action(&Action::Page(Page::Amoo), ctx);
                self.handle_action(&Action::Workspace(WorkspaceAction::New), ctx);
            }
            StudioAction::Tab(tab) => {
                self.studio.tab = match *tab {
                    1 | 4 => 0,
                    t => t,
                };
                if *tab == 1 {
                    self.studio.source = 2;
                } else if *tab == 4 {
                    self.studio.source = 4;
                }
                self.scroll = Default::default();
                self.trigger_subtab_switch();
            }
            StudioAction::Category(i) => {
                self.studio.category = (*i).min(2);
                self.trigger_subtab_switch();
            }
            StudioAction::Select(i) => {
                if let Some(w) = self.studio.works.get(*i) {
                    self.fields[F_STUDIO_NAME].set(&w.name);
                    self.fields[F_STUDIO_URL].set(&w.url);
                    self.studio.category = CATEGORIES
                        .iter()
                        .position(|c| *c == w.category)
                        .unwrap_or(0);
                    self.studio.selected = Some(*i);
                    self.studio.qr = None;
                    self.studio.phone_note = None;
                    self.trigger_subtab_switch();
                }
            }
            StudioAction::PasteLink => {
                let text = ctx.clipboard().read().plain_text;
                let raw = text
                    .split_whitespace()
                    .find(|v| v.starts_with("https://"))
                    .unwrap_or(text.trim());
                match studio::parse_link(raw) {
                    Ok((url, _)) => {
                        self.studio.qr = None;
                        if let Some(i) = self.studio.works.iter().position(|w| w.url == url) {
                            self.handle_studio_action(&StudioAction::Select(i), ctx);
                        } else {
                            self.fields[F_STUDIO_URL].set(&url);
                            self.studio.selected = None;
                        }
                        self.status = "已粘贴官方作品链接，请填写名称后保存".into();
                    }
                    Err(e) => {
                        self.status = e.to_string();
                        self.error = true;
                    }
                }
            }
            StudioAction::New => {
                self.studio.editing = true;
                self.studio.source = 2;
                self.studio.tab = 0;
                self.studio.selected = None;
                self.studio.qr = None;
                self.studio.phone_note = None;
                self.fields[F_STUDIO_NAME].clear();
                self.fields[F_STUDIO_URL].clear();
            }
            StudioAction::Save => {
                match Work::new(
                    &self.fields[F_STUDIO_NAME].text,
                    &self.fields[F_STUDIO_URL].text,
                    CATEGORIES[self.studio.category],
                ) {
                    Ok(w) => {
                        let url = w.url.clone();
                        self.submit("正在保存作品链接…", move |_| {
                            Ok(Outcome::Studio(StudioOutcome::Library(
                                studio::save(w)?,
                                Some(url),
                            )))
                        });
                    }
                    Err(e) => {
                        self.status = e.to_string();
                        self.error = true;
                    }
                }
            }
            StudioAction::Refresh => self.refresh_studio(),
            StudioAction::Send
            | StudioAction::OpenPhone
            | StudioAction::Pin
            | StudioAction::Share => {
                let Some(w) = self
                    .studio
                    .selected
                    .and_then(|i| self.studio.works.get(i))
                    .cloned()
                else {
                    self.status = "请先保存或选择一个作品".into();
                    return;
                };
                if self.fields[F_STUDIO_NAME].text.trim() != w.name
                    || studio::parse_link(&self.fields[F_STUDIO_URL].text)
                        .ok()
                        .is_none_or(|(url, _)| url != w.url)
                    || CATEGORIES[self.studio.category] != w.category
                {
                    self.status = "作品信息已修改，请先保存链接再上机或分享".into();
                    self.error = true;
                    return;
                }
                if !w.shareable() {
                    self.status =
                        "编辑地址仅用于继续创作；请在官网生成体验链接，再发送到手机或分享".into();
                    self.error = true;
                    return;
                }
                if matches!(action, StudioAction::Share) {
                    self.submit("正在生成分享二维码…", move |_| {
                        let qr = studio::qr(&w)?;
                        Ok(Outcome::Studio(StudioOutcome::Qr(w.id, qr)))
                    });
                } else {
                    let action = action.clone();
                    self.with_device("正在与手机作品库通信…", move |adb, d| {
                        studio::send(adb, &d, &w)?;
                        let message = match action {
                            StudioAction::OpenPhone => {
                                studio::open_phone(adb, &d, "open", Some(&w.id))?
                            }
                            StudioAction::Pin => studio::open_phone(adb, &d, "pin", Some(&w.id))?,
                            _ => format!(
                                "手机已确认保存「{}」；可从 StepX伴侣的 Studio 作品打开",
                                w.name
                            ),
                        };
                        Ok(Outcome::Studio(StudioOutcome::Phone(d.serial, message)))
                    });
                }
            }
            StudioAction::AmooWorks => {
                self.studio.tab = 0;
                self.studio.source = 4;
                self.studio.filter = 0;
                self.scroll = Default::default();
                self.motion_started = Instant::now();
            }
            StudioAction::PhoneLibrary => {
                self.with_device("正在打开手机作品入口…", move |adb, d| {
                    Ok(Outcome::Studio(StudioOutcome::Phone(
                        d.serial.clone(),
                        studio::open_phone(adb, &d, "library", None)?,
                    )))
                });
            }
            StudioAction::CheckPhone => {
                self.with_device("正在回读手机作品与桌面图标…", |adb, d| {
                    let v = studio::phone_call(adb, &d, serde_json::json!({"op":"status"}))?;
                    let n = v["pinned"].as_array().map_or(0, Vec::len);
                    let note = format!(
                        "手机已保存 {} 个作品 · 已确认 {} 个桌面图标{}",
                        v["count"],
                        n,
                        if v["pinSupported"] == false {
                            " · 当前桌面不支持固定图标"
                        } else {
                            ""
                        }
                    );
                    Ok(Outcome::Studio(StudioOutcome::Phone(d.serial, note)))
                })
            }
            StudioAction::SaveQr => {
                if let Some((_, path)) = self.studio.qr.clone() {
                    self.submit("正在导出分享二维码…", move |_| {
                        let destination = native_dialog::FileDialog::new()
                            .set_filename("Studio-作品二维码.png")
                            .add_filter("PNG", &["png"])
                            .show_save_single_file()?;
                        if let Some(dest) = destination {
                            if dest != path {
                                std::fs::copy(&path, &dest)?;
                            }
                            Ok(Outcome::Notice(format!("二维码已保存：{}", dest.display())))
                        } else {
                            Ok(Outcome::Notice("已取消导出".into()))
                        }
                    });
                }
            }
            StudioAction::Brief(i) => {
                if let Some((_, _, brief)) = BRIEFS.get(*i) {
                    ctx.clipboard()
                        .write(ClipboardContent::plain_text((*brief).into()));
                    self.handle_action(
                        &Action::StudioOpenUrl("https://studio.stepx.com/".into()),
                        ctx,
                    );
                    self.status = format!(
                        "已复制创作需求；请在官网选择{}构建器并粘贴",
                        CATEGORIES[(*i).min(2)]
                    );
                }
            }
            StudioAction::UseSavedOnPhone(i) => {
                if *i < self.studio.works.len() {
                    self.studio.selected = Some(*i);
                    self.handle_studio_action(&StudioAction::OpenPhone, ctx);
                }
            }
            StudioAction::DeleteSaved(i) => {
                if *i < self.studio.works.len() {
                    let removed = self.studio.works.remove(*i);
                    if self.studio.selected == Some(*i) {
                        self.studio.selected = (!self.studio.works.is_empty()).then_some(0);
                    } else if let Some(sel) = self.studio.selected {
                        if sel > *i {
                            self.studio.selected = Some(sel - 1);
                        }
                    }
                    let _ = studio::save_all(&self.studio.works);
                    self.status = format!("已移除收藏「{}」", removed.name);
                }
            }
        }
    }
    pub(super) fn apply_studio(&mut self, result: StudioOutcome) {
        match result {
            StudioOutcome::Workspace(serial, phone, website) => {
                if !self.selected.as_ref().is_some_and(|d| d.serial == serial) {
                    return;
                }
                self.apply_studio(StudioOutcome::Cloud(serial, phone));
                match website {
                    Ok(web) => {
                        let old = self
                            .studio
                            .web
                            .as_ref()
                            .filter(|c| c.scope == web.scope)
                            .and_then(|c| self.studio.web_selected.and_then(|i| c.items.get(i)))
                            .map(|w| w.id.clone());
                        self.studio.web_selected = old
                            .and_then(|id| web.items.iter().position(|w| w.id == id))
                            .or_else(|| if web.items.is_empty() { None } else { Some(0) });
                        self.studio.web_error = None;
                        if web.authorized {
                            self.studio.connecting_until = None;
                            self.status = format!(
                                "官网项目已同步：{} 个应用、{} 个智能体、{} 个技能",
                                web.totals[0], web.totals[1], web.totals[2]
                            );
                        }
                        self.studio.web = Some(web);
                    }
                    Err(e) => {
                        self.studio.web = None;
                        self.studio.web_selected = None;
                        self.studio.web_error = Some(e);
                    }
                }
            }
            StudioOutcome::Cloud(serial, result) => {
                if !self.selected.as_ref().is_some_and(|d| d.serial == serial) {
                    return;
                }
                self.studio.cloud_checked = Some(Instant::now());
                match result {
                    Ok(cloud) => {
                        let old_id = self
                            .studio
                            .cloud
                            .as_ref()
                            .filter(|c| c.scope == cloud.scope)
                            .and_then(|c| self.studio.cloud_selected.and_then(|i| c.items.get(i)))
                            .map(|w| w.id.clone());
                        self.studio.cloud_selected =
                            old_id.and_then(|id| cloud.items.iter().position(|w| w.id == id));
                        self.studio.cloud_selected = self.studio.cloud_selected.or_else(|| {
                            cloud.items.iter().position(|w| {
                                self.studio.cloud_category == 3
                                    || w.category == CATEGORIES[self.studio.cloud_category]
                            })
                        });
                        self.studio.cloud_error = None;
                        if cloud.authorized {
                            self.status = format!(
                                "已同步 {} 个应用、{} 个智能体、{} 个技能",
                                cloud.totals[0], cloud.totals[1], cloud.totals[2]
                            );
                        } else {
                            self.status = "手机账号尚未连接 Studio 作品同步".into();
                        }
                        self.studio.cloud = Some(cloud);
                    }
                    Err(error) => {
                        self.studio.cloud = None;
                        self.studio.cloud_selected = None;
                        self.studio.cloud_error = Some(error.clone());
                        self.status = error;
                        self.error = true;
                    }
                }
            }
            StudioOutcome::Library(works, url) => {
                let selected_url = url.clone().or_else(|| {
                    self.studio
                        .selected
                        .and_then(|i| self.studio.works.get(i))
                        .map(|w| w.url.clone())
                });
                self.studio.works = works;
                if url.is_some() {
                    self.studio.editing = false;
                }
                self.studio.loaded = true;
                self.studio.selected =
                    selected_url.and_then(|u| self.studio.works.iter().position(|w| w.url == u));
                if self.studio.selected.is_none()
                    && self.fields[F_STUDIO_NAME].text.is_empty()
                    && self.fields[F_STUDIO_URL].text.is_empty()
                {
                    if let Some(w) = self.studio.works.first() {
                        self.fields[F_STUDIO_NAME].set(&w.name);
                        self.fields[F_STUDIO_URL].set(&w.url);
                        self.studio.category = CATEGORIES
                            .iter()
                            .position(|c| *c == w.category)
                            .unwrap_or(0);
                        self.studio.selected = Some(0);
                    }
                }
                if let Some(w) = self.studio.selected.and_then(|i| self.studio.works.get(i)) {
                    self.fields[F_STUDIO_NAME].set(&w.name);
                    self.fields[F_STUDIO_URL].set(&w.url);
                    self.studio.category = CATEGORIES
                        .iter()
                        .position(|c| *c == w.category)
                        .unwrap_or(0);
                }
                if url.is_some() {
                    self.studio.source = 2;
                    self.studio.tab = 0;
                }
                self.studio.qr = None;
                self.status = if url.is_some() {
                    "作品链接已保存到本机".into()
                } else {
                    format!("已读取 {} 个本机作品", self.studio.works.len())
                };
            }
            StudioOutcome::Qr(id, path) => {
                self.studio.qr = Some((id, path));
                self.status = "分享二维码已生成；体验链接有效期与访问权限由 Studio 决定".into();
            }
            StudioOutcome::Phone(serial, note) => {
                if self.selected.as_ref().is_some_and(|d| d.serial == serial) {
                    self.studio.phone_note = Some(note.clone());
                    self.status = note;
                }
            }
        }
    }
}
