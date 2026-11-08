//! One search surface, shared query and stable deep links across phone and desktop.
use super::*;
use super::{icons::UiIcon, theme::*, widgets::*};
use facloud_ui::{
    color::ColorU,
    fonts::{Properties, Weight},
    platform::Cursor,
    text_layout::{ClipConfig, TextStyle},
    TypedActionView,
};
use stepx_companion::search::{self, Document, FileRecord, Hit, Kind};

#[derive(Clone, Debug)]
pub(super) enum SearchAction {
    Open,
    Close,
    AllResults,
    Back,
    Move(i8),
    Activate,
    Select(String),
    Kind(Option<Kind>),
    Source(Option<bool>),
    ToggleKinds,
    ResetFilters,
    More,
    Query(String),
    Refresh,
}
#[derive(Clone, Debug)]
enum Target {
    Tool(ToolPage),
    Setting(u8),
    Page(Page),
    Im(String, Option<String>),
    Sms(String),
    Contact(String),
    Calendar { id: i64, start: i64 },
    Alarm(i64),
    Session(String),
    Task(String),
    Gallery(String),
    Local(PathBuf),
    PhoneFile(String),
    Store(String),
    App(String),
    Studio(String),
    Notification(String),
    CloudWork(bool, String),
    Context(PathBuf),
    Preset(stepx_companion::backscreen::Preset),
}
struct SearchRequest {
    generation: u64,
    docs: std::sync::Arc<Vec<Document>>,
    query: String,
    kind: Option<Kind>,
    phone: Option<bool>,
}
enum ScanResult {
    Found(u64, Vec<Hit>),
    Desktop(Vec<FileRecord>, bool),
    Phone(String, Result<(Vec<FileRecord>, bool), String>),
    Opened(Result<String, String>),
}
pub(super) struct SearchState {
    pub open: bool,
    pub closing: Option<Instant>,
    pub(super) input_width: std::rc::Rc<Cell<f32>>,
    preview_scroll: ClippedScrollStateHandle,
    preview_height: std::rc::Rc<RefCell<SearchHeight>>,
    started: Instant,
    previous: Page,
    previous_focus: Option<usize>,
    docs: std::sync::Arc<Vec<Document>>,
    targets: HashMap<String, Target>,
    hits: Vec<Hit>,
    pub selected: usize,
    kind: Option<Kind>,
    phone: Option<bool>,
    show_all_kinds: bool,
    limit: usize,
    query: String,
    edited: Instant,
    built: Instant,
    applied: String,
    generation: u64,
    filtering: bool,
    dirty: bool,
    activate_when_ready: bool,
    find_tx: mpsc::Sender<SearchRequest>,
    preserve: Option<String>,
    desktop_files: Vec<FileRecord>,
    phone_files: Vec<FileRecord>,
    scope: Option<(String, Option<String>)>,
    device: Option<String>,
    tx: mpsc::Sender<ScanResult>,
    rx: mpsc::Receiver<ScanResult>,
    desktop_busy: bool,
    phone_busy: bool,
    scanned: Option<Instant>,
    note: String,
    recent: Vec<String>,
    opening: bool,
}
impl Default for SearchState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        let (find_tx, find_rx) = mpsc::channel::<SearchRequest>();
        let result_tx = tx.clone();
        std::thread::spawn(move || {
            let mut index = stepx_companion::search_index::Index::open(&stepx_companion::resources::user_state().join("cache/search.sqlite3"))
                .or_else(|_| stepx_companion::search_index::Index::memory()).ok();
            let mut indexed_docs: Option<std::sync::Arc<Vec<Document>>> = None;
            while let Ok(mut request) = find_rx.recv() {
                while let Ok(newer) = find_rx.try_recv() {
                    request = newer;
                }
                let indexed = (|| -> anyhow::Result<Vec<Hit>> {
                    // Recent files are already in memory. Do not make opening
                    // the popup wait for a full-text index update.
                    if request.query.trim().is_empty() {
                        return Ok(search::find(&request.docs, &request.query, request.kind, request.phone));
                    }
                    let db = index.as_mut().ok_or_else(|| anyhow::anyhow!("搜索索引不可用"))?;
                    if !indexed_docs.as_ref().is_some_and(|old| std::sync::Arc::ptr_eq(old, &request.docs)) {
                        db.sync(&request.docs)?;
                        indexed_docs = Some(request.docs.clone());
                    }
                    db.find(&request.docs, &request.query, request.kind, request.phone)
                })();
                let hits = indexed.unwrap_or_else(|_| search::find(&request.docs, &request.query, request.kind, request.phone));
                if result_tx
                    .send(ScanResult::Found(request.generation, hits))
                    .is_err()
                {
                    break;
                }
            }
        });
        Self {
            input_width: std::rc::Rc::new(Cell::new(400.)),
            preview_scroll: Default::default(),
            preview_height: Default::default(),
            open: false,
            closing: None,
            started: Instant::now(),
            previous: Page::Home,
            previous_focus: None,
            docs: std::sync::Arc::new(vec![]),
            targets: HashMap::new(),
            hits: vec![],
            selected: 0,
            kind: None,
            phone: None,
            show_all_kinds: false,
            limit: 60,
            query: String::new(),
            applied: String::new(),
            generation: 0,
            filtering: false,
            dirty: false,
            activate_when_ready: false,
            find_tx,
            preserve: None,
            edited: Instant::now(),
            built: Instant::now() - Duration::from_secs(10),
            desktop_files: vec![],
            phone_files: vec![],
            scope: None,
            device: None,
            tx,
            rx,
            desktop_busy: false,
            phone_busy: false,
            scanned: None,
            note: String::new(),
            recent: vec![],
            opening: false,
        }
    }
}
fn stamp(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%m-%d %H:%M").to_string())
        .unwrap_or_default()
}
fn kind_icon(k: Kind) -> UiIcon {
    match k {
        Kind::Calendar => UiIcon::Calendar,
        Kind::Alarm => UiIcon::Clock,
        Kind::Work => UiIcon::Package,
        Kind::Amoo => UiIcon::Sparkles,
        Kind::Message => UiIcon::Chat,
        Kind::Call => UiIcon::Phone,
        Kind::Contact => UiIcon::User,
        Kind::Image => UiIcon::Image,
        Kind::BackScreen => UiIcon::Matrix,
        Kind::App => UiIcon::Apps,
        Kind::Store => UiIcon::Download,
        Kind::Setting => UiIcon::Settings,
        Kind::Summary | Kind::File => UiIcon::File,
        Kind::Function => UiIcon::Tools,
    }
}

const PREVIEW_LIMIT: usize = 4;
const SHORTCUT_HEIGHT: f32 = 46.;
const RESULT_HEIGHT: f32 = 66.;

fn matched_characters(text: &str, query: &str) -> Vec<usize> {
    if query.trim().is_empty() {
        return Vec::new();
    }
    let mut folded = String::new();
    let starts: Vec<_> = text
        .chars()
        .map(|c| {
            let start = folded.len();
            folded.extend(c.to_lowercase());
            start
        })
        .collect();
    let mut matched = vec![false; starts.len()];
    for word in query.split_whitespace().take(12) {
        let word = word.to_lowercase();
        for (byte, _) in folded.match_indices(&word) {
            let start = starts.partition_point(|b| *b <= byte).saturating_sub(1);
            let end = starts.partition_point(|b| *b < byte + word.len());
            matched[start..end].fill(true);
        }
    }
    matched
        .into_iter()
        .enumerate()
        .filter_map(|(i, hit)| hit.then_some(i))
        .collect()
}

#[cfg(test)]
#[test]
fn search_highlights_original_character_indices_after_lowercasing() {
    assert_eq!(matched_characters("İ 图标.SVG", "图标 svg"), [2, 3, 5, 6, 7]);
    assert_eq!(matched_characters("İzmir", "i"), [0, 3]);
    assert!(matched_characters("PDF", "svg").is_empty());
}

// Run on a worker: launching a default application must never stall typing or
// claim success when xdg-open/open immediately rejects the file type.
pub(super) fn open_search_file(path: &std::path::Path) -> Result<String> {
    if !search::safe_document(path) {
        work_state::reveal_in_folder(path)?;
        return Ok("已在文件夹中显示文件".into());
    }
    open_with_default_app(path)
}

/// Only trusted, locally created callers may bypass search's document allowlist.
pub(super) fn open_with_default_app(path: &std::path::Path) -> Result<String> {
    #[cfg(windows)]
    {
        work_state::open_work_path(path)?;
        Ok("已交给默认应用打开".into())
    }
    #[cfg(not(windows))]
    {
        let mut child = std::process::Command::new(if cfg!(target_os = "macos") {
            "open"
        } else {
            "xdg-open"
        })
        .arg(path)
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()?;
        let started = Instant::now();
        loop {
            if let Some(status) = child.try_wait()? {
                anyhow::ensure!(
                    status.success(),
                    "默认应用无法打开此文件，请检查文件关联或从文件夹打开"
                );
                return Ok("已在电脑打开文件".into());
            }
            if started.elapsed() > Duration::from_secs(4) {
                std::thread::spawn(move || {
                    let _ = child.wait();
                });
                return Ok("已交给默认应用打开".into());
            }
            std::thread::sleep(Duration::from_millis(40));
        }
    }
}

impl Companion {
    fn rebuild_search(&mut self) {
        self.search.preserve = self
            .search
            .hits
            .get(self.search.selected)
            .and_then(|h| self.search.docs.get(h.index))
            .map(|d| d.id.clone());
        let mut docs: Vec<Document> = Vec::new();
        let mut targets = HashMap::new();
        let mut indices = HashMap::<String, usize>::new();
        let mut add = |id: String,
                       kind: Kind,
                       phone: bool,
                       title: String,
                       body: String,
                       source: String,
                       at: i64,
                       target: Target| {
            if title.trim().is_empty() {
                return;
            }
            if let Some(&index) = indices.get(&id) {
                let existing = &mut docs[index];
                if !body.is_empty() && !existing.body.contains(&body) {
                    existing.body.push('\n');
                    existing.body.push_str(&body);
                }
                existing.at = existing.at.max(at);
                return;
            }
            indices.insert(id.clone(), docs.len());
            targets.insert(id.clone(), target);
            docs.push(Document {
                id,
                kind,
                phone,
                title,
                body,
                source,
                at,
            });
        };
        for (page, aliases) in [
            (Page::Home, "首页 今天 待办 总结 工作台"),
            (Page::Connect, "连接 配对 USB 无线 Wi-Fi 同步"),
            (Page::Calendar, "日历 日程 会议 安排"),
            (Page::Alarms, "闹钟 计时 专注 番茄钟"),
            (Page::Studio, "作品 应用 成果 文档"),
            (Page::Amoo, "amoo 阿莫 AI 对话 讨论 助手"),
            (Page::Sms, "消息 短信 微信 通知 验证码 取件码"),
            (Page::Phone, "电话 通话 联系人 未接来电"),
            (Page::Gallery, "图片 图库 相册 照片 截图"),
            (Page::BackScreen, "背屏 点阵 动画"),
            (Page::Files, "文件 手机 共享存储 下载"),
            (Page::Work, "工作文件 电脑 工作区"),
            (Page::Apps, "应用 已安装 app"),
            (Page::AppStore, "应用商店 下载 安装 更新"),
            (Page::Tools, "功能 小工具 剪贴板 投屏 天气"),
            (Page::Settings, "设置 主题 提醒 弹窗 模型 账号 隐私 快捷键"),
        ] {
            add(
                format!("page:{page:?}"),
                if page == Page::Settings {
                    Kind::Setting
                } else {
                    Kind::Function
                },
                false,
                page.title().into(),
                aliases.into(),
                "功能入口".into(),
                0,
                Target::Page(page),
            );
        }
        for tool in ToolPage::ALL {
            add(
                format!("tool:{tool:?}"),
                Kind::Function,
                false,
                tool.title().into(),
                tool.description().into(),
                "小工具".into(),
                0,
                Target::Tool(tool),
            );
        }
        for (tab, title, terms) in [
            (0, "通用设置", "主题 外观 动画 快捷键 托盘"),
            (1, "文件与存储设置", "下载目录 手机存储 扩容"),
            (2, "生成文件设置", "输出位置 工作目录"),
            (3, "通知设置", "弹窗 免打扰 提醒 消息 红点"),
            (4, "关于与版本", "版本 检查更新 联系作者"),
            (super::settings_state::AGENT_SETTINGS_TAB, "Agent 工作台", "AI 智能体 场景 策略 提示词 待办 草稿"),
        ] {
            add(
                format!("setting:{tab}"),
                Kind::Setting,
                false,
                title.into(),
                terms.into(),
                "设置".into(),
                0,
                Target::Setting(tab),
            );
        }
        for e in &self.back_screen.stepos_suite.calendar_events {
            add(
                format!("event:{}:{}", e.id, e.dtstart),
                Kind::Calendar,
                e.source != "desktop",
                e.title.clone(),
                format!("{} {} {}", stamp(e.dtstart), e.location, e.description),
                "日历".into(),
                e.dtstart,
                Target::Calendar { id: e.id, start: e.dtstart },
            );
        }
        for a in &self.back_screen.stepos_suite.alarms {
            add(
                format!("alarm:{}", a.id),
                Kind::Alarm,
                true,
                format!("{} {}", a.time_label(), a.display_title()),
                format!(
                    "{} {}",
                    a.repeat_label(),
                    if a.enabled { "已开启" } else { "已关闭" }
                ),
                "闹钟".into(),
                0,
                Target::Alarm(a.id),
            );
        }
        for s in &self.amoo_sessions {
            let text = s
                .items
                .iter()
                .filter_map(|item| match item {
                    AmooChatItem::UserText { text, .. } => Some(text.as_str()),
                    AmooChatItem::UserVoice { transcript, .. } => Some(transcript.as_str()),
                    AmooChatItem::AssistantRich { text, .. } => text.as_deref(),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("\n");
            add(
                format!("amoo:{}", s.session_key),
                Kind::Amoo,
                true,
                s.title.clone(),
                text,
                "手机 Amoo".into(),
                0,
                Target::Session(s.session_key.clone()),
            );
        }
        for t in self
            .workspace
            .journal
            .tasks
            .iter()
            .filter(|t| self.workspace_visible(t))
        {
            add(
                format!("task:{}", t.id),
                Kind::Amoo,
                false,
                t.title.clone(),
                t.turns
                    .iter()
                    .map(|x| x.text.as_str())
                    .collect::<Vec<_>>()
                    .join("\n"),
                "电脑 Amoo".into(),
                t.updated,
                Target::Task(t.id.clone()),
            );
            for out in &t.outputs {
                add(
                    format!("local:{}", out.path.display()),
                    Kind::Work,
                    false,
                    out.name.clone(),
                    format!("{} {}", out.kind, t.title),
                    "对话成果".into(),
                    t.updated,
                    Target::Local(out.path.clone()),
                );
            }
        }
        for t in &self.push.snapshot.threads {
            if stepx_companion::push::is_excluded_im_package(&t.package_name) {
                continue;
            }
            for m in &t.messages {
                if m.recalled {
                    continue;
                }
                add(
                    format!("im:{}:{}", t.thread_id, m.id),
                    Kind::Message,
                    true,
                    t.title.clone(),
                    format!("{}: {}", if m.outgoing { "我" } else { &m.sender }, m.text),
                    t.app_name.clone(),
                    m.timestamp_ms,
                    Target::Im(t.thread_id.clone(), Some(m.id.clone())),
                );
            }
            if t.messages.is_empty() {
                add(
                    format!("im:{}", t.thread_id),
                    Kind::Message,
                    true,
                    t.title.clone(),
                    t.last_message.clone(),
                    t.app_name.clone(),
                    t.last_timestamp_ms,
                    Target::Im(t.thread_id.clone(), None),
                );
            }
        }
        for n in self.effective_push_notifications() {
            if stepx_companion::push::should_hide_notification(&n) {
                continue;
            }
            add(
                format!("notification:{}", n.key),
                Kind::Message,
                true,
                n.title.clone(),
                format!("{} {} {}", n.sender, n.text, n.sub_text),
                n.app_name.clone(),
                n.post_time_ms,
                Target::Notification(n.key.clone()),
            );
        }
        for m in &self.sms {
            add(
                format!(
                    "sms:{}:{}:{}",
                    m.address,
                    m.date_ms,
                    m.provider_id.unwrap_or(0)
                ),
                Kind::Message,
                true,
                m.address.clone(),
                m.body.clone(),
                "短信".into(),
                m.date_ms,
                Target::Sms(m.address.clone()),
            );
        }
        for c in &self.effective_contacts() {
            add(
                format!("contact:{}", c.number),
                Kind::Contact,
                true,
                c.name.clone(),
                c.number.clone(),
                "联系人".into(),
                0,
                Target::Contact(c.number.clone()),
            );
        }
        for c in &self.call_logs {
            add(
                format!("call:{}:{}", c.number, c.date_ms),
                Kind::Call,
                true,
                if c.name.is_empty() {
                    c.number.clone()
                } else {
                    c.name.clone()
                },
                format!("{} {} {}", c.number, c.type_label(), stamp(c.date_ms)),
                "通话记录".into(),
                c.date_ms,
                Target::Contact(c.number.clone()),
            );
        }
        for g in &self.back_screen.gallery_items {
            add(
                format!(
                    "{}:{}",
                    if g.category == "expansion" {
                        "local"
                    } else {
                        "phone"
                    },
                    g.remote_path
                ),
                Kind::Image,
                g.category != "expansion",
                g.name.clone(),
                format!("{} {}", g.category_label(), g.remote_path),
                g.category_label().into(),
                g.modified_ms,
                Target::Gallery(g.remote_path.clone()),
            );
        }
        for p in stepx_companion::backscreen::Preset::ALL {
            add(
                format!("back:{}", p.key()),
                Kind::BackScreen,
                false,
                p.label().into(),
                p.description().into(),
                "背屏图案".into(),
                0,
                Target::Preset(p),
            );
        }
        for (web, library) in [(true, &self.studio.web), (false, &self.studio.cloud)] {
            if let Some(library) = library.as_ref().filter(|l| l.authorized) {
                for w in &library.items {
                    add(
                        format!("cloudwork:{web}:{}", w.id),
                        Kind::Work,
                        !web,
                        w.name.clone(),
                        w.description.clone(),
                        if web { "官网作品" } else { "手机作品" }.into(),
                        0,
                        Target::CloudWork(web, w.id.clone()),
                    );
                }
            }
        }
        for w in &self.studio.works {
            add(
                format!("studio:{}", w.id),
                Kind::Work,
                false,
                w.name.clone(),
                w.category.clone(),
                "我的作品".into(),
                w.updated,
                Target::Studio(w.id.clone()),
            );
        }
        // Reuse discovery metadata, especially the phone arrival timestamps
        // absent from its full filename index. No extra device or disk polling.
        for f in self.recent_files.desktop.iter().chain(self.recent_files.phone.iter()) {
            let phone = f.device.is_some();
            if phone && f.device.as_ref() != self.search.device.as_ref() {
                continue;
            }
            add(
                format!("{}:{}", if phone { "phone" } else { "local" }, f.path.display()),
                if f.kind == stepx_companion::recent_files::Kind::Image { Kind::Image } else { Kind::File },
                phone,
                f.name(),
                f.path.display().to_string(),
                format!("{} · {}", f.kind.label(), f.folder),
                f.at,
                if phone { Target::PhoneFile(f.path.to_string_lossy().into()) } else { Target::Local(f.path.clone()) },
            );
        }
        for w in self.work.files.iter().filter(|w| !w.is_dir) {
            add(
                format!("local:{}", w.path.display()),
                if search::image_path(&w.path) {
                    Kind::Image
                } else {
                    Kind::File
                },
                false,
                w.name.clone(),
                w.path.display().to_string(),
                "工作区".into(),
                0,
                Target::Local(w.path.clone()),
            );
        }
        for w in &self.work.artifacts {
            add(
                format!("local:{}", w.path.display()),
                Kind::Work,
                false,
                w.name.clone(),
                w.kind.clone(),
                "工作成果".into(),
                0,
                Target::Local(w.path.clone()),
            );
        }
        for f in self.files.iter().filter(|f| !f.directory) {
            add(
                format!("phone:{}", f.path),
                if search::image_path(std::path::Path::new(&f.path)) {
                    Kind::Image
                } else {
                    Kind::File
                },
                true,
                f.name.clone(),
                f.path.clone(),
                "手机文件".into(),
                0,
                Target::PhoneFile(f.path.clone()),
            );
        }
        for (files, phone) in [
            (&self.search.desktop_files, false),
            (&self.search.phone_files, true),
        ] {
            for f in files {
                add(
                    format!(
                        "{}:{}",
                        if phone { "phone" } else { "local" },
                        f.path.display()
                    ),
                    if f.image { Kind::Image } else { Kind::File },
                    phone,
                    f.name.clone(),
                    format!("{}\n{}", f.path.display(), f.text),
                    if phone {
                        "手机共享存储"
                    } else {
                        "电脑文件"
                    }
                    .into(),
                    f.at,
                    if phone {
                        Target::PhoneFile(f.path.to_string_lossy().into())
                    } else {
                        Target::Local(f.path.clone())
                    },
                );
            }
        }
        for a in &self.apps {
            add(
                format!("app:{}", a.package),
                Kind::App,
                true,
                if a.label.is_empty() {
                    a.package.clone()
                } else {
                    a.label.clone()
                },
                format!("{} {}", a.package, a.version_name),
                "已安装应用".into(),
                a.last_update_ms,
                Target::App(a.package.clone()),
            );
        }
        for a in &self.app_store.catalog {
            add(
                format!("store:{}", a.package),
                Kind::Store,
                false,
                a.name.clone(),
                format!(
                    "{} {} {} {}",
                    a.package, a.tagline, a.category_label, a.description
                ),
                "应用商店".into(),
                0,
                Target::Store(a.package.clone()),
            );
        }
        // Use the same scope-filtered source as the home page; no other account's summaries.
        for (title, body, path, at) in self.context.search_documents() {
            add(
                format!("context:{}", path.display()),
                Kind::Summary,
                false,
                title,
                body,
                "方案、待办与复盘".into(),
                at,
                Target::Context(path),
            );
        }
        let old_hits: Vec<_> = self
            .search
            .hits
            .iter()
            .filter_map(|h| {
                self.search
                    .docs
                    .get(h.index)
                    .map(|d| (d.id.clone(), h.clone()))
            })
            .collect();
        let by_id: HashMap<_, _> = docs
            .iter()
            .enumerate()
            .map(|(i, d)| (d.id.clone(), i))
            .collect();
        self.search.hits = old_hits
            .into_iter()
            .filter_map(|(id, mut h)| {
                h.index = *by_id.get(&id)?;
                Some(h)
            })
            .collect();
        self.search.docs = std::sync::Arc::new(docs);
        self.search.targets = targets;
        self.search.built = Instant::now();
        self.filter_search();
    }
    fn filter_search(&mut self) {
        self.search.dirty = false;
        self.search.generation = self.search.generation.wrapping_add(1);
        self.search.filtering = true;
        if self.search.applied != self.fields[F_GLOBAL_SEARCH].text {
            self.search.hits.clear();
        }
        self.search.applied = self.fields[F_GLOBAL_SEARCH].text.clone();
        let _ = self.search.find_tx.send(SearchRequest {
            generation: self.search.generation,
            docs: self.search.docs.clone(),
            query: self.search.applied.clone(),
            kind: self.search.kind,
            phone: self.search.phone,
        });
    }
    pub(super) fn search_edited(&mut self) {
        // Invalidate even if typing/deleting returns to the previous query before
        // the debounce elapses. Otherwise its cleared result list stays empty.
        self.search.dirty = true;
        self.search.filtering = true;
        self.search.generation = self.search.generation.wrapping_add(1);
        self.search.hits.clear();
        self.search.query = self.fields[F_GLOBAL_SEARCH].text.clone();
        self.search.edited = Instant::now();
        self.search.selected = 0;
        self.search.limit = 60;
        self.search.activate_when_ready = false;
        self.search.preserve = None;
        self.reset_list_page(&self.search_page_key());
        self.search.preview_scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
        if self.page == Page::Search {
            self.scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
        }
    }
    fn start_search_scan(&mut self, force: bool) {
        if !force
            && self
                .search
                .scanned
                .is_some_and(|s| s.elapsed() < Duration::from_secs(60))
        {
            return;
        }
        self.search.scanned = Some(Instant::now());
        self.search.note.clear();
        if !self.search.desktop_busy {
            self.search.desktop_busy = true;
            let tx = self.search.tx.clone();
            let workspace = self.work.workspace.clone();
            std::thread::spawn(move || {
                let roots =
                    stepx_companion::recent_files::Preferences::load().roots(workspace.as_deref());
                let (files, limited) = search::desktop_files(&roots);
                let _ = tx.send(ScanResult::Desktop(files, limited));
            });
        }
        if !self.search.phone_busy {
            if let Some(device) = self.selected.clone() {
                self.search.phone_busy = true;
                let tx = self.search.tx.clone();
                std::thread::spawn(move || {
                    let result = Adb::discover()
                        .and_then(|adb| search::phone_files(&adb, &device))
                        .map_err(|e| e.to_string());
                    let _ = tx.send(ScanResult::Phone(device.serial, result));
                });
            }
        }
    }
    pub(super) fn poll_search(&mut self, ctx: &mut ViewContext<Self>) {
        if self
            .search
            .closing
            .is_some_and(|s| s.elapsed() > Duration::from_millis(180))
        {
            self.search.closing = None;
            ctx.notify();
        }
        let device = self
            .selected
            .as_ref()
            .map(|d| d.serial.clone())
            .or_else(|| self.last_connected_serial.clone());
        let scope = self.context.productivity_scope();
        if self.search.device != device || self.search.scope != scope {
            self.search.scope = scope;
            self.search.device = device;
            self.search.phone_files.clear();
            self.search.docs = std::sync::Arc::new(vec![]);
            self.search.generation = self.search.generation.wrapping_add(1);
            self.search.targets.clear();
            self.search.hits.clear();
            self.search.scanned = None;
            self.search.built = Instant::now() - Duration::from_secs(10);
        }
        let mut changed = false;
        let mut rebuild = false;
        let mut activate = None;
        while let Ok(result) = self.search.rx.try_recv() {
            match result {
                ScanResult::Found(generation, hits) => {
                    if generation != self.search.generation
                        || self.search.applied != self.fields[F_GLOBAL_SEARCH].text
                    {
                        continue;
                    }
                    self.search.hits = hits;
                    self.search.filtering = false;
                    self.search.selected = self
                        .search
                        .preserve
                        .take()
                        .and_then(|id| {
                            self.search
                                .hits
                                .iter()
                                .position(|h| self.search.docs[h.index].id == id)
                        })
                        .unwrap_or(0);
                    if self.search.open && self.search.selected >= PREVIEW_LIMIT {
                        self.search.selected = 0;
                    }
                    if self.search.activate_when_ready {
                        activate = self
                            .selected_search_hit()
                            .and_then(|index| self.search.hits.get(index))
                            .map(|h| self.search.docs[h.index].id.clone());
                        self.search.activate_when_ready = false;
                    }
                }
                ScanResult::Desktop(files, limited) => {
                    rebuild = true;
                    self.search.desktop_busy = false;
                    self.search.desktop_files = files;
                    if limited {
                        self.search.note =
                            "文件较多，本次已建立部分索引；可缩小范围或重新更新".into();
                    }
                }
                ScanResult::Phone(serial, result) => {
                    rebuild = true;
                    self.search.phone_busy = false;
                    if self.search.device.as_ref() != Some(&serial) {
                        self.search.scanned = None;
                        continue;
                    }
                    if self.search.device.as_ref() == Some(&serial) {
                        match result {
                            Ok((files, limited)) => {
                                self.search.phone_files = files;
                                if limited {
                                    self.search.note = "手机文件较多，当前为部分索引".into();
                                }
                            }
                            Err(_) => {
                                self.search.note =
                                    "手机文件暂未读到，已同步内容仍可搜索；连接恢复后可更新".into()
                            }
                        }
                    }
                }
                ScanResult::Opened(result) => {
                    self.search.opening = false;
                    match result {
                        Ok(note) => self.status = note,
                        Err(error) => {
                            self.record_notice("文件打开未完成", &error);
                            self.status = format!("文件打开未完成：{error}");
                            self.error = true;
                            self.search.note = error;
                        }
                    }
                }
            }
            changed = true;
        }
        if self.search.open || self.page == Page::Search {
            self.start_search_scan(false);
            if rebuild
                || (self.search.built.elapsed() > Duration::from_secs(10)
                    && self.search.edited.elapsed() > Duration::from_secs(1))
            {
                self.rebuild_search();
                changed = true;
            } else if self.search.dirty && self.search.edited.elapsed() > Duration::from_millis(120)
            {
                self.search.query = self.fields[F_GLOBAL_SEARCH].text.clone();
                self.filter_search();
                self.search.built = Instant::now();
                changed = true;
            }
        }
        if let Some(id) = activate {
            self.handle_action(&Action::GlobalSearch(SearchAction::Select(id)), ctx);
        }
        if changed {
            ctx.notify();
        }
    }
    /// Header surfaces switch ownership in one frame. Do not leave a closing
    /// search backdrop above the new surface, or retain its keyboard focus.
    pub(super) fn dismiss_search_for_header(&mut self) {
        self.search.open = false;
        self.search.closing = None;
        self.search.activate_when_ready = false;
        self.search.previous_focus = None;
        if self.focus == Some(F_GLOBAL_SEARCH) {
            self.fields[F_GLOBAL_SEARCH].preedit.clear();
            self.focus = None;
        }
    }

    pub(super) fn handle_search(&mut self, a: &SearchAction, ctx: &mut ViewContext<Self>) {
        // A deferred Enter belongs to that exact interaction. Changing filters,
        // browsing all results or moving selection must not later open a result.
        if !matches!(a, SearchAction::Activate) {
            self.search.activate_when_ready = false;
        }
        match a {
            SearchAction::Open => {
                self.push.shell_mode = ShellMode::Full;
                self.search.closing = None;
                if !self.search.open {
                    self.search.previous_focus = self.focus;
                    self.search.started = Instant::now();
                }
                self.search.open = self.page != Page::Search;
                if self.search.open && self.search.selected >= PREVIEW_LIMIT {
                    self.search.selected = 0;
                    self.search.preview_scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
                }
                self.notification_center.drawer_open = false;
                self.back_screen.live_capsule_popover_open = false;
                self.header_actions_open = false;
                if !self.search.open
                    || self.search.built.elapsed() > Duration::from_secs(3)
                    || self.search.docs.is_empty()
                {
                    self.rebuild_search();
                }
                self.start_search_scan(false);
                self.focus_field(F_GLOBAL_SEARCH, ctx);
            }
            SearchAction::Close => {
                self.search.closing = (!self.animations_disabled()).then(Instant::now);
                self.search.activate_when_ready = false;
                self.search.open = false;
                self.focus = self.search.previous_focus.take();
            }
            SearchAction::AllResults => {
                if self.page != Page::Search {
                    self.search.previous = self.page;
                }
                self.search.open = false;
                self.handle_action(&Action::Page(Page::Search), ctx);
                self.focus_field(F_GLOBAL_SEARCH, ctx);
            }
            SearchAction::Back => {
                self.search.open = false;
                self.handle_action(&Action::Page(self.search.previous), ctx);
            }
            SearchAction::Move(delta) => {
                let range = if self.search.open {0..self.search.hits.len().min(PREVIEW_LIMIT)} else {
                    self.list_page_range(&self.search_page_key(),self.search.hits.len(),60)
                };
                if !range.is_empty() {
                    self.search.selected = range.start + (self.search.selected.saturating_sub(range.start) as isize + *delta as isize)
                        .rem_euclid(range.len() as isize) as usize;
                    let scroll = if self.search.open {
                        &self.search.preview_scroll
                    } else {
                        &self.scroll
                    };
                    scroll.scroll_to_position(ScrollTarget {
                        position_id: format!("search-row-{}", self.search.selected),
                        mode: ScrollToPositionMode::FullyIntoView,
                    });
                }
            }
            SearchAction::Activate => {
                if self.search.filtering
                    || self.search.dirty
                    || self.search.applied != self.fields[F_GLOBAL_SEARCH].text
                {
                    self.filter_search();
                    self.search.activate_when_ready = true;
                    return;
                }
                if let Some(id) = self
                    .selected_search_hit()
                    .and_then(|index| self.search.hits.get(index))
                    .map(|h| self.search.docs[h.index].id.clone())
                {
                    self.handle_action(&Action::GlobalSearch(SearchAction::Select(id)), ctx);
                }
            }
            SearchAction::Select(id) => {
                let Some(target) = self.search.targets.get(id).cloned() else {
                    return;
                };
                let query = self.fields[F_GLOBAL_SEARCH].text.trim().to_string();
                if !query.is_empty() {
                    self.search.recent.retain(|s| s != &query);
                    self.search.recent.insert(0, query);
                    self.search.recent.truncate(6);
                }
                self.search.open = false;
                self.focus = None;
                self.open_search_target(target, ctx);
            }
            SearchAction::Kind(k) => {
                self.search.preserve = None;
                self.search.hits.clear();
                self.search.kind = *k;
                self.search.selected = 0;
                self.search.limit = 60;
                self.reset_list_page(&self.search_page_key());
                self.filter_search();
            }
            SearchAction::Source(p) => {
                self.search.preserve = None;
                self.search.hits.clear();
                self.search.phone = *p;
                self.search.selected = 0;
                self.search.limit = 60;
                self.reset_list_page(&self.search_page_key());
                self.filter_search();
            }
            SearchAction::ToggleKinds => self.search.show_all_kinds = !self.search.show_all_kinds,
            SearchAction::ResetFilters => {
                self.search.preserve = None;
                self.search.kind = None;
                self.search.phone = None;
                self.search.hits.clear();
                self.search.selected = 0;
                self.search.limit = 60;
                self.reset_list_page(&self.search_page_key());
                self.filter_search();
            }
            SearchAction::More => self.search.limit += 60,
            SearchAction::Query(q) => {
                self.fields[F_GLOBAL_SEARCH].set(q);
                self.search_edited();
                self.filter_search();
                self.focus_field(F_GLOBAL_SEARCH, ctx);
            }
            SearchAction::Refresh => {
                self.start_search_scan(true);
                self.rebuild_search();
            }
        }
        self.sync_ime(ctx);
        ctx.notify();
    }
    fn open_search_target(&mut self, target: Target, ctx: &mut ViewContext<Self>) {
        match target {
            Target::Tool(tool) => self.handle_action(&Action::OpenTool(Some(tool)), ctx),
            Target::Setting(tab) => {
                self.handle_action(&Action::Page(Page::Settings), ctx);
                self.handle_action(&Action::Settings(SettingsAction::Tab(tab)), ctx);
            }
            Target::Notification(key) => {
                self.handle_action(&Action::Page(Page::Notifications), ctx);
                self.fields[F_PUSH_SEARCH].clear();
                self.handle_action(&Action::Push(PushAction::ResetNotificationFilters), ctx);
                self.handle_action(&Action::Push(PushAction::SelectNotification(key)), ctx);
            }
            Target::CloudWork(web, id) => {
                let index = if web {
                    self.studio.web.as_ref()
                } else {
                    self.studio.cloud.as_ref()
                }
                .and_then(|l| l.items.iter().position(|w| w.id == id));
                if let Some(i) = index {
                    self.handle_action(&Action::Page(Page::Studio), ctx);
                    self.studio.tab = 0;
                    self.studio.source = if web { 0 } else { 1 };
                    self.fields[F_STUDIO_SEARCH].clear();
                    self.handle_action(
                        &Action::Studio(if web {
                            StudioAction::WebSelect(i)
                        } else {
                            StudioAction::PhoneItem(i)
                        }),
                        ctx,
                    );
                }
            }
            Target::Page(p) => self.handle_action(&Action::Page(p), ctx),
            Target::Im(id, message) => {
                self.handle_action(&Action::Page(Page::Sms), ctx);
                self.fields[F_SMS_SEARCH].clear();
                self.push.im_channel = stepx_companion::push::ImChannel::All;
                self.handle_action(&Action::Push(PushAction::SelectImThread(id.clone())), ctx);
                if let Some(first) = message {
                    self.handle_action(
                        &Action::Push(PushAction::ImHistoryPage {
                            thread_id: id,
                            first: Some(first),
                        }),
                        ctx,
                    );
                }
            }
            Target::Sms(address) => {
                self.handle_action(&Action::Page(Page::Sms), ctx);
                self.fields[F_SMS_SEARCH].clear();
                self.handle_action(&Action::SelectSmsThread(Some(address)), ctx);
            }
            Target::Contact(number) => {
                self.handle_action(&Action::Page(Page::Phone), ctx);
                self.fields[F_CONTACT_SEARCH].set(&number);
                self.push.call_filter = 0;
                self.call_filter_missed = false;
                if !self.filtered_contacts().is_empty() {
                    self.handle_action(&Action::PhoneSelectContact(0), ctx);
                } else {
                    self.fields[F_PHONE_DIAL].set(&number);
                }
            }
            Target::Calendar { id, start } => {
                if let Some(e) = self
                    .back_screen
                    .stepos_suite
                    .calendar_events
                    .iter()
                    .find(|e| e.id == id && e.dtstart == start)
                {
                    let (y, m, d) = e.date_ymd();
                    self.handle_action(&Action::Page(Page::Calendar), ctx);
                    self.handle_action(
                        &Action::BackScreen(BackScreenAction::CalendarSelectDate(y, m, d)),
                        ctx,
                    );
                }
            }
            Target::Alarm(id) => {
                self.handle_action(&Action::Page(Page::Alarms), ctx);
                self.back_screen.editing_alarm_id = None;
                self.scroll.scroll_to_position(ScrollTarget {
                    position_id: format!("search-alarm-{id}"),
                    mode: ScrollToPositionMode::FullyIntoView,
                });
            }
            Target::Session(key) => {
                if let Some(i) = self.amoo_sessions.iter().position(|s| s.session_key == key) {
                    self.handle_action(&Action::Page(Page::Amoo), ctx);
                    self.handle_action(&Action::Workspace(WorkspaceAction::PhoneSession(i)), ctx);
                }
            }
            Target::Task(id) => {
                self.handle_action(&Action::Page(Page::Amoo), ctx);
                self.handle_action(&Action::Workspace(WorkspaceAction::Select(id)), ctx);
            }
            Target::Gallery(path) => {
                if let Some(i) = self
                    .back_screen
                    .gallery_items
                    .iter()
                    .position(|g| g.remote_path == path)
                {
                    self.handle_action(&Action::Page(Page::Gallery), ctx);
                    self.fields[F_GALLERY_SEARCH].clear();
                    self.back_screen.gallery_category = 0;
                    self.handle_action(
                        &Action::BackScreen(BackScreenAction::GallerySelect(i)),
                        ctx,
                    );
                }
            }
            Target::Store(package) => {
                self.handle_action(&Action::Page(Page::AppStore), ctx);
                self.handle_action(&Action::AppStore(AppStoreAction::SelectApp(package)), ctx);
            }
            Target::App(package) => {
                self.handle_action(&Action::Page(Page::Apps), ctx);
                self.fields[F_APP_SEARCH].set(&package);
            }
            Target::Studio(id) => {
                if let Some(i) = self.studio.works.iter().position(|w| w.id == id) {
                    self.handle_action(&Action::Page(Page::Studio), ctx);
                    self.studio.tab = 0;
                    self.studio.source = 2;
                    self.fields[F_STUDIO_SEARCH].clear();
                    self.handle_action(&Action::Studio(StudioAction::Select(i)), ctx);
                }
            }
            Target::Context(path) => self.handle_action(
                &Action::Context(context_state::ContextAction::Open(path)),
                ctx,
            ),
            Target::Preset(p) => {
                self.handle_action(&Action::Page(Page::BackScreen), ctx);
                self.handle_action(&Action::BackScreen(BackScreenAction::SelectPreset(p)), ctx);
            }
            Target::Local(path) => {
                if !path.exists() {
                    self.search.note = "文件已移动或删除，请更新搜索结果".into();
                    self.search.open = true;
                    return;
                }
                if self.search.opening {
                    return;
                }
                self.search.opening = true;
                let tx = self.search.tx.clone();
                std::thread::spawn(move || {
                    let result = open_search_file(&path).map_err(|e| e.to_string());
                    let _ = tx.send(ScanResult::Opened(result));
                });
            }
            Target::PhoneFile(path) => {
                let Some(device) = self.selected.clone() else {
                    self.search.note = "连接手机后即可打开这个文件".into();
                    self.search.open = true;
                    return;
                };
                if self.search.device.as_ref() != Some(&device.serial) {
                    self.search.note = "手机已切换，请重新搜索".into();
                    self.search.open = true;
                    return;
                }
                if self.search.opening {
                    return;
                }
                self.search.opening = true;
                self.status = "正在取回手机文件…".into();
                let tx = self.search.tx.clone();
                std::thread::spawn(move || {
                    let result = (|| -> Result<String> {
                        let adb = Adb::discover()?;
                        storage::validate_path(&path)?;
                        let root = stepx_companion::resources::user_state().join("file-previews");
                        std::fs::create_dir_all(&root)?;
                        let temp = tempfile::Builder::new()
                            .prefix("view-search-")
                            .tempdir_in(root)?;
                        let name = std::path::Path::new(&path)
                            .file_name()
                            .context("文件名无效")?;
                        let local = temp.path().join(name);
                        storage::pull(&adb, &device, &path, &local)?;
                        let result = open_search_file(&local);
                        let _ = temp.keep();
                        result
                    })()
                    .map_err(|e| e.to_string());
                    let _ = tx.send(ScanResult::Opened(result));
                });
            }
        }
    }
    #[cfg(feature = "ui-capture")]
    pub(super) fn search_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"page":format!("{:?}",self.page),"open":self.search.open,"query":self.fields[F_GLOBAL_SEARCH].text,"count":self.search.hits.len(),"filtering":self.search.filtering,"phone_files":self.search.phone_files.len(),"desktop_files":self.search.desktop_files.len(),"note":self.search.note,"selected":self.search.selected,"results":self.search.hits.iter().take(60).map(|h|{let d=&self.search.docs[h.index];serde_json::json!({"id":d.id,"title":d.title,"kind":d.kind.label(),"phone":d.phone})}).collect::<Vec<_>>()})
    }
    pub(super) fn search_bar(&self) -> El {
        let row = self.input(F_GLOBAL_SEARCH, "搜索消息、文件、功能…", None);
        #[cfg(feature = "ui-capture")]
        let row = crate::ui::scenario_check::control("global-search-input".into(), row);
        SavePosition::new(
            ConstrainedBox::new(row).with_height(32.).finish(),
            "global-search-anchor",
        )
        .for_single_frame()
        .finish()
    }
    fn search_source_filters(&self) -> El {
        self.bounded_wrap(
            [
                ("综合", None, UiIcon::Layers),
                ("手机", Some(true), UiIcon::Smartphone),
                ("电脑", Some(false), UiIcon::Monitor),
            ]
            .into_iter()
            .map(|(label, source, icon)| {
                self.chip(
                    format!("search-source-{label}"),
                    label,
                    Some(icon),
                    vec![Action::GlobalSearch(SearchAction::Source(source))],
                    self.search.phone == source,
                    false,
                )
            })
            .collect(),
        )
    }

    fn search_kind_filters(&self, full: bool) -> El {
        let common = if full {
            &Kind::ALL[..6]
        } else {
            &Kind::ALL[..3]
        };
        let mut buttons = vec![self.chip(
            if full {
                "search-kind-all"
            } else {
                "search-quick-None"
            },
            "全部",
            None,
            vec![Action::GlobalSearch(SearchAction::Kind(None))],
            self.search.kind.is_none(),
            false,
        )];
        buttons.extend(
            Kind::ALL
                .into_iter()
                .filter(|kind| {
                    common.contains(kind)
                        || (full && self.search.show_all_kinds)
                        || self.search.kind == Some(*kind)
                })
                .map(|kind| {
                    let active = self.search.kind == Some(kind);
                    let id = if full {
                        format!("search-kind-{kind:?}")
                    } else {
                        format!("search-quick-Some({kind:?})")
                    };
                    self.chip(
                        id,
                        kind.label(),
                        Some(kind_icon(kind)),
                        vec![Action::GlobalSearch(SearchAction::Kind(
                            (!active).then_some(kind),
                        ))],
                        active,
                        false,
                    )
                }),
        );
        if full {
            let (label, icon) = if self.search.show_all_kinds {
                ("收起类型", UiIcon::ChevronUp)
            } else {
                ("更多类型", UiIcon::ChevronDown)
            };
            buttons.push(self.chip(
                "search-more-kinds",
                label,
                Some(icon),
                vec![Action::GlobalSearch(SearchAction::ToggleKinds)],
                false,
                false,
            ));
        }
        self.bounded_wrap(buttons)
    }

    fn search_shortcut(&self, d: &Document) -> bool {
        matches!(
            self.search.targets.get(&d.id),
            Some(Target::Page(_) | Target::Setting(_))
        )
    }

    fn search_result_line(&self, text: &str, size: f32, color: ColorU, weight: Weight) -> El {
        let p = self.p();
        let matches = matched_characters(text, &self.search.applied);
        let properties = Properties::default().weight(weight);
        Text::new(text.to_owned(), self.font, size)
            .with_style(properties)
            .with_color(color)
            .soft_wrap(false)
            .with_clip(ClipConfig::ellipsis())
            .with_single_highlight(
                Highlight::new()
                    .with_properties(Properties::default().weight(Weight::Semibold))
                    .with_text_style(
                        TextStyle::new()
                            .with_foreground_color(p.text)
                            .with_background_color(p.indigo_soft),
                    ),
                matches,
            )
            .finish()
    }

    fn search_row(&self, hit: &Hit, index: usize) -> El {
        let d = &self.search.docs[hit.index];
        let p = self.p();
        let action = Action::GlobalSearch(SearchAction::Select(d.id.clone()));
        let shortcut = self.search_shortcut(d);
        let icon = match self.search.targets.get(&d.id) {
            Some(Target::Page(Page::Files)) => UiIcon::Folder,
            Some(Target::Page(page)) => {
                super::icons::symbol(page.icon()).unwrap_or(kind_icon(d.kind))
            }
            Some(Target::Local(_) | Target::PhoneFile(_)) => {
                super::icon_style::file_content_icon(&d.title, false)
            }
            _ => kind_icon(d.kind),
        };
        let selected = self.selected_search_hit() == Some(index);
        let row = Hoverable::new(
            self.mouse_state(format!("search-hit:{}", d.id)),
            move |hover| {
                let mut lines =
                    vec![self.search_result_line(&d.title, HEADING, p.text, Weight::Semibold)];
                if !shortcut {
                    if !search::browsing_files(&self.search.applied, self.search.kind)
                        && !hit.snippet.trim().is_empty() {
                        lines.push(self.search_result_line(
                            &hit.snippet,
                            CAPTION,
                            p.text2,
                            Weight::Normal,
                        ));
                    }
                    let source = if d.source.is_empty() {
                        d.kind.label()
                    } else {
                        &d.source
                    };
                    let device = if d.phone { "手机" } else { "电脑" };
                    let meta = if d.at > 0 {
                        format!("{device} · {source} · {}", stamp(d.at))
                    } else {
                        format!("{device} · {source}")
                    };
                    lines.push(self.hstack(
                        4.,
                        vec![
                            self.ui_icon(
                                if d.phone {
                                    UiIcon::Smartphone
                                } else {
                                    UiIcon::Monitor
                                },
                                11.,
                                p.text2,
                            ),
                            Expanded::new(1., self.line(meta, MICRO, p.text2)).finish(),
                        ],
                    ));
                }
                let row = Container::new(self.hstack(
                    S2,
                    vec![
                        self.section_icon_badge(
                            icon,
                            if shortcut { 16. } else { 18. },
                            if shortcut { 30. } else { 36. },
                        ),
                        Expanded::new(1., Align::new(self.vstack(2., lines)).left().finish())
                            .finish(),
                        self.ui_icon(
                            UiIcon::ChevronRight,
                            14.,
                            if selected { p.accent } else { p.text2 },
                        ),
                    ],
                ))
                .with_horizontal_padding(S2)
                .with_background_color(if selected {
                    p.accent_soft
                } else if hover.is_hovered() {
                    p.hover
                } else {
                    p.bg
                })
                .with_border(Border::left(2.).with_border_color(if selected {
                    p.accent
                } else {
                    ColorU::transparent_black()
                }))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
                ConstrainedBox::new(row)
                    .with_height(if shortcut {
                        SHORTCUT_HEIGHT
                    } else {
                        RESULT_HEIGHT
                    })
                    .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let row = SavePosition::new(row, &format!("search-row-{index}")).finish();
        #[cfg(feature = "ui-capture")]
        let row = crate::ui::scenario_check::control(format!("search-hit-{index}"), row);
        row
    }

    fn search_page_key(&self) -> String {
        format!(
            "search-results:{:?}:{:?}:{}",
            self.search.kind, self.search.phone, self.fields[F_GLOBAL_SEARCH].text
        )
    }

    /// Filtering, source updates and deletion can shrink the currently displayed
    /// page without a keyboard event. Enter must agree with the visible highlight.
    fn selected_search_hit(&self) -> Option<usize> {
        let range = if self.search.open {
            0..self.search.hits.len().min(PREVIEW_LIMIT)
        } else {
            self.list_page_range(&self.search_page_key(), self.search.hits.len(), 60)
        };
        (!range.is_empty()).then(|| self.search.selected.clamp(range.start, range.end - 1))
    }

    fn search_empty(&self, full: bool) -> El {
        let p = self.p();
        let files = search::browsing_files(&self.fields[F_GLOBAL_SEARCH].text, self.search.kind);
        let loading = self.search.filtering || (files && (self.search.desktop_busy || self.search.phone_busy));
        let filtered = self.search.phone.is_some() || self.search.kind.is_some();
        let (icon, title) = if loading {
            (UiIcon::Search, "正在检索")
        } else if files {
            (UiIcon::Folder, "暂无最近文件")
        } else {
            (UiIcon::ScanText, "未找到匹配内容")
        };
        let message = if filtered {
            "试试扩大搜索范围"
        } else {
            "试试人名、关键词或文件名"
        };
        if !full {
            return Container::new(self.hstack(
                S2,
                vec![
                self.section_icon_badge(icon, 16., 30.),
                Expanded::new(1., self.vstack(2., vec![
                    self.heading(title),
                    self.line(if loading { "正在查找已索引内容" } else { message }, MICRO, p.text2),
                ])).finish(),
            ],
            ))
            .with_horizontal_padding(S2)
            .with_vertical_padding(S2)
            .finish();
        }
        let mut items = vec![
            Align::new(self.section_icon_badge(icon, 24., 52.)).finish(),
            Align::new(self.line_w(title, TITLE, p.text, Weight::Semibold)).finish(),
        ];
        if !loading {
            items.push(Align::new(self.text(message, CAPTION, p.text2)).finish());
            let (label, action) = if filtered {
                ("清除筛选", SearchAction::ResetFilters)
            } else {
                ("修改关键词", SearchAction::Open)
            };
            items.push(
                Align::new(
                    ConstrainedBox::new(self.button(
                        "search-empty-action",
                        label,
                        Action::GlobalSearch(action),
                        Btn::Secondary,
                    ))
                    .with_max_width(128.)
                    .finish(),
                )
                .finish(),
            );
        }
        Container::new(self.vstack(S2, items))
            .with_uniform_padding(S6)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }

    fn search_list(&self, full: bool) -> El {
        if self.search.hits.is_empty() {
            return self.search_empty(full);
        }
        let page_key = self.search_page_key();
        let range = if full {
            self.list_page_range(&page_key, self.search.hits.len(), 60)
        } else {
            0..self.search.hits.len().min(PREVIEW_LIMIT)
        };
        let rows = range
            .map(|i| self.search_row(&self.search.hits[i], i))
            .collect();
        let list = self.vstack(S1, rows);
        if !full {
            return list;
        }
        let p = self.p();
        self.vstack(
            S2,
            vec![
                Container::new(list)
                    .with_uniform_padding(S2)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
                self.list_page_controls(&page_key, self.search.hits.len(), 60),
            ],
        )
    }

    fn search_recent(&self, full: bool) -> El {
        self.bounded_wrap(
            self.search
                .recent
                .iter()
                .take(if full { 6 } else { 3 })
                .map(|q| {
                    self.chip(
                        format!("search-recent-{q}"),
                        Self::truncate_chars(q, 10),
                        Some(UiIcon::Clock),
                        vec![Action::GlobalSearch(SearchAction::Query(q.clone()))],
                        false,
                        false,
                    )
                })
                .collect(),
        )
    }

    pub(super) fn search_page(&self) -> El {
        let p = self.p();
        let query = self.fields[F_GLOBAL_SEARCH].text.trim();
        let count = if self.search.filtering && self.search.hits.is_empty() {
            "检索中".to_owned()
        } else {
            format!("{} 条", self.search.hits.len())
        };
        let hero = Container::new(self.vstack(
            S3,
            vec![
                self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.button_icon(
                                "search-back",
                                "返回",
                                UiIcon::ArrowLeft,
                                Action::GlobalSearch(SearchAction::Back),
                                Btn::Ghost,
                            ),
                            self.heading("搜索结果"),
                        ],
                    ),
                    self.hstack(
                        4.,
                        vec![
                            self.status_icon(
                                "search-coverage",
                                "消息按已同步文字检索，图片和手机文件按名称检索",
                                UiIcon::Info,
                                p.text2,
                            ),
                            self.icon_action(
                                "search-refresh",
                                "更新索引",
                                UiIcon::Refresh,
                                vec![Action::GlobalSearch(SearchAction::Refresh)],
                                false,
                            ),
                        ],
                    ),
                ),
                self.hstack(
                    S3,
                    vec![
                self.section_icon_badge(UiIcon::Search, 22., 44.),
                Expanded::new(1., self.line_w(
                    if search::browsing_files(query, self.search.kind) { "最近文件" }
                    else if query.is_empty() { "浏览已索引内容" } else { query },
                    22., p.text, Weight::Semibold,
                )).finish(),
                self.pill_badge(count, p.text2, p.surface),
            ],
                ),
                self.divider(),
                self.search_source_filters(),
                self.search_kind_filters(true),
            ],
        ))
        .with_uniform_padding(S4)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let mut parts = vec![hero];
        let status = if !self.search.note.is_empty() {
            Some((UiIcon::Info, self.search.note.as_str(), p.warning_text))
        } else if self.search.desktop_busy || self.search.phone_busy {
            Some((UiIcon::Refresh, "正在更新文件索引", p.text2))
        } else if self.selected.is_none() {
            Some((UiIcon::Smartphone, "手机未连接 · 可搜索已同步内容", p.text2))
        } else {
            None
        };
        if let Some((icon, message, color)) = status {
            parts.push(self.hstack(
                S2,
                vec![
                    self.ui_icon(icon, 13., color),
                    Expanded::new(1., self.text(message, MICRO, color)).finish(),
                ],
            ));
        }
        if query.is_empty() && self.search.kind.is_some() && !self.search.recent.is_empty() {
            parts.push(self.section_heading(UiIcon::Clock, "最近搜索"));
            parts.push(self.search_recent(true));
        }
        parts.push(self.search_list(true));
        Align::new(
            ConstrainedBox::new(self.vstack(S3, parts))
                .with_max_width(880.)
                .finish(),
        )
        .top_center()
        .finish()
    }
    pub(super) fn with_search_overlay(&self, base: El) -> El {
        if !self.search.open && self.search.closing.is_none() {
            return base;
        }
        let p = self.p();
        let browsing = self.fields[F_GLOBAL_SEARCH].text.trim().is_empty();
        let rows_height: f32 = self
            .search
            .hits
            .iter()
            .take(PREVIEW_LIMIT)
            .map(|hit| {
                let height = if self.search_shortcut(&self.search.docs[hit.index]) {
                    SHORTCUT_HEIGHT
                } else {
                    RESULT_HEIGHT
                };
                height + S1
            })
            .sum();
        let preview_height = if self.search.filtering {
            self.search.preview_height.borrow().to.max(56.)
        } else {
            (rows_height - S1).max(56.)
        };
        let files = search::browsing_files(&self.fields[F_GLOBAL_SEARCH].text, self.search.kind);
        let heading = if files {
            "最近文件"
        } else {
            "快速结果"
        };
        let header = self.hstack(
            S2,
            vec![
                Expanded::new(1., self.search_source_filters()).finish(),
                self.icon_action(
                    "search-close",
                    "关闭",
                    UiIcon::Close,
                    vec![Action::GlobalSearch(SearchAction::Close)],
                    false,
                ),
            ],
        );
        let mut top = vec![header, self.search_kind_filters(false), self.divider()];
        if browsing && !files && !self.search.recent.is_empty() {
            top.push(self.search_recent(false));
        }
        top.push(self.heading(heading));
        let footer = self.between(
            self.hstack(
                S1,
                vec![
                    self.ui_icon(UiIcon::ArrowUp, 11., p.text2),
                    self.ui_icon(UiIcon::ArrowDown, 11., p.text2),
                    Expanded::new(1., self.line("选择 · Enter 打开", MICRO, p.text2)).finish(),
                ],
            ),
            self.button_icon(
                "search-all",
                "全部结果",
                UiIcon::ArrowRight,
                Action::GlobalSearch(SearchAction::AllResults),
                Btn::Secondary,
            ),
        );
        let content = Flex::column()
            .with_main_axis_size(MainAxisSize::Min)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_spacing(S2)
            .with_child(self.vstack(S2, top))
            .with_child(
                Shrinkable::new(
                    1.,
                    Box::new(SearchViewport {
                        child: self.pane_scroll(
                            self.search.preview_scroll.clone(),
                            self.search_list(false),
                        ),
                        state: self.search.preview_height.clone(),
                        target: preview_height,
                        animate: !self.animations_disabled(),
                    }),
                )
                .finish(),
            )
            .with_child(footer)
            .finish();
        let card = popover_surface(Container::new(content).with_uniform_padding(S3).finish(), p);
        let card: El = Box::new(SearchReveal {
            child: card,
            width: self.search.input_width.clone(),
            animate: !self.animations_disabled(),
            started: self.search.closing.unwrap_or(self.search.started),
            closing: self.search.closing.is_some(),
        });
        #[cfg(feature = "ui-capture")]
        let card = crate::ui::scenario_check::control("global-search-popover".into(), card);
        let card = Container::new(card).with_vertical_padding(S2).finish();
        let backdrop = Hoverable::new(self.mouse_state("search-backdrop"), move |_| {
            Align::new(Empty::new().finish()).finish()
        })
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::GlobalSearch(SearchAction::Close)))
        .finish();
        Stack::new()
            .with_event_dispatch_mode(EventDispatchMode::Waterfall)
            .with_child(base)
            // Container records hits over its padding even when transparent.
            // Leave the header out of the backdrop's actual bounds so search
            // editing, its microphone and the header controls remain usable.
            .with_child(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(
                        ConstrainedBox::new(Empty::new().finish())
                            .with_height(56.)
                            .finish(),
                    )
                    .with_child(Expanded::new(1., backdrop).finish())
                    .finish(),
            )
            .with_positioned_child(
                card,
                OffsetPositioning::offset_from_save_position_element(
                    "global-search-anchor",
                    vec2f(0., 4.),
                    PositionedElementOffsetBounds::ParentByPosition,
                    PositionedElementAnchor::BottomMiddle,
                    ChildAnchor::TopMiddle,
                ),
            )
            .finish()
    }
}

#[derive(Default)]
struct SearchHeight {
    from: f32,
    to: f32,
    started: Option<Instant>,
}

#[cfg(test)]
mod search_interaction_tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn recent_file_metadata_is_reused_without_cross_device_results() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, _ctx| {
                let requests = prepare_results(view);
                view.fields[F_GLOBAL_SEARCH].clear();
                view.search.device = Some("current-phone".into());
                let file = |path: &str, device: Option<&str>, at| stepx_companion::recent_files::RecentFile {
                    path: path.into(), device: device.map(str::to_owned), at, modified: at,
                    bytes: 1024, kind: stepx_companion::recent_files::Kind::Pdf,
                    folder: "Download".into(), thumbnail: None,
                };
                view.recent_files.desktop = vec![file("/test/方案.pdf", None, 100)];
                view.recent_files.phone = vec![
                    file("/sdcard/Download/报告.pdf", Some("current-phone"), 200),
                    file("/sdcard/Download/旧设备.pdf", Some("other-phone"), 300),
                ];
                view.rebuild_search();
                let request = requests.try_recv().unwrap();
                let hits = search::find(&request.docs, &request.query, request.kind, request.phone);
                let doc = &request.docs[hits.iter().find(|h| request.docs[h.index].title == "报告.pdf").unwrap().index];
                assert_eq!(doc.at, 200);
                assert!(matches!(view.search.targets.get(&doc.id), Some(Target::PhoneFile(_))));
                assert!(request.docs.iter().all(|d| d.title != "旧设备.pdf"));
                assert!(hits.iter().all(|h| !view.search_shortcut(&request.docs[h.index])));
            });
        });
    }

    fn prepare_results(view: &mut Companion) -> mpsc::Receiver<SearchRequest> {
        // Do not index a real user's directories or let an actual worker race
        // the deterministic result callbacks exercised below.
        let (tx, rx) = mpsc::channel();
        view.search.find_tx = tx;
        view.page = Page::Search;
        view.selected = None;
        view.last_connected_serial = None;
        view.search.device = None;
        view.search.scope = view.context.productivity_scope();
        view.search.scanned = Some(Instant::now());
        view.search.built = Instant::now();
        view.fields[F_GLOBAL_SEARCH].set("needle");
        view.search.applied = "needle".into();
        view.search.query = "needle".into();
        let docs: Vec<_> = (0..130).map(|index| Document {
            id: format!("result:{index}"), kind: Kind::Function, phone: false,
            title: format!("needle {index}"), body: String::new(),
            source: "隔离测试".into(), at: 0,
        }).collect();
        view.search.targets = docs.iter().enumerate().map(|(index, doc)| (
            doc.id.clone(),
            Target::Page(if index == 60 { Page::Calendar } else { Page::Files }),
        )).collect();
        view.search.hits = (0..docs.len()).map(|index| Hit {
            index, snippet: String::new(), score: 1,
        }).collect();
        view.search.docs = std::sync::Arc::new(docs);
        view.search.filtering = false;
        view.search.dirty = false;
        rx
    }

    #[test]
    fn deferred_enter_opens_the_highlight_on_the_visible_page() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                let _requests = prepare_results(view);
                view.handle_action(&Action::ListPage(view.search_page_key(), 1), ctx);
                assert_eq!(view.selected_search_hit(), Some(60));
                view.search.filtering = true;
                view.handle_search(&SearchAction::Activate, ctx);
                assert!(view.search.activate_when_ready);
                view.search.tx.send(ScanResult::Found(view.search.generation, view.search.hits.clone())).unwrap();
                view.poll_search(ctx);
                assert_eq!(view.page, Page::Calendar,
                    "a delayed Enter must not open the invisible first-page file result");
            });
        });
    }

    #[test]
    fn changing_filters_cancels_pending_enter_and_resets_returned_page() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                let _requests = prepare_results(view);
                view.handle_action(&Action::ListPage(view.search_page_key(), 1), ctx);
                for action in [SearchAction::Source(Some(true)), SearchAction::Source(None),
                    SearchAction::Kind(Some(Kind::Message)), SearchAction::ResetFilters] {
                    view.search.activate_when_ready = true;
                    view.handle_search(&action, ctx);
                    assert!(!view.search.activate_when_ready);
                }
                assert_eq!(view.list_page_range(&view.search_page_key(), 130, 60), 0..60);
                view.search.tx.send(ScanResult::Found(view.search.generation, vec![])).unwrap();
                view.poll_search(ctx);
                assert_eq!(view.page, Page::Search);
            });
        });
    }

    #[test]
    fn selection_clamps_to_a_shrunken_visible_page_and_new_query_resets_it() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                let _requests = prepare_results(view);
                view.handle_action(&Action::ListPage(view.search_page_key(), 2), ctx);
                view.search.hits.truncate(75);
                assert_eq!(view.selected_search_hit(), Some(74));
                view.search_edited();
                assert_eq!(view.list_page_range(&view.search_page_key(), 130, 60), 0..60);
                assert_eq!(view.selected_search_hit(), None);
            });
        });
    }

    #[test]
    fn calendar_search_opens_the_matching_occurrence_not_the_first_shared_id() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                let _requests = prepare_results(view);
                let event = |day| {
                    let start = chrono::Local.with_ymd_and_hms(2026, 11, day, 10, 0, 0)
                        .single().unwrap().timestamp_millis();
                    stepx_companion::toolbox::PhoneCalendarEvent {
                        id: 71, title: "项目周会".into(), dtstart: start,
                        dtend: start + 3_600_000, ..Default::default()
                    }
                };
                let later = event(14);
                view.back_screen.stepos_suite.calendar_events = vec![event(7), later.clone()];
                view.open_search_target(Target::Calendar { id: later.id, start: later.dtstart }, ctx);
                assert_eq!(view.page, Page::Calendar);
                assert_eq!((view.back_screen.calendar_year, view.back_screen.calendar_month,
                    view.back_screen.selected_calendar_day), (2026, 11, 14));
            });
        });
    }

    #[test]
    fn delayed_file_open_failure_does_not_reopen_search_or_steal_input_focus() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                let _requests = prepare_results(view);
                view.page = Page::Amoo;
                view.search.open = false;
                view.focus = Some(F_PUSH_REPLY);
                view.fields[F_PUSH_REPLY].set("正在输入的草稿");
                view.search.tx.send(ScanResult::Opened(Err("测试文件关联不可用".into()))).unwrap();
                view.poll_search(ctx);
                assert!(!view.search.open);
                assert_eq!(view.page, Page::Amoo);
                assert_eq!(view.focus, Some(F_PUSH_REPLY));
                assert_eq!(view.fields[F_PUSH_REPLY].text, "正在输入的草稿");
                assert!(view.error && view.status.contains("测试文件关联不可用"));
                assert_eq!(view.interaction.diagnostics[0].title, "文件打开未完成");
            });
        });
    }
}
impl SearchHeight {
    fn value(&self) -> f32 {
        let t = self
            .started
            .map_or(1., |s| (s.elapsed().as_secs_f32() / 0.18).min(1.));
        self.from + (self.to - self.from) * motion::bezier_control(t)
    }
}
/// Keep the input anchored while a short result list smoothly grows or shrinks.
struct SearchViewport {
    child: El,
    state: std::rc::Rc<RefCell<SearchHeight>>,
    target: f32,
    animate: bool,
}
impl Element for SearchViewport {
    fn layout(
        &mut self,
        mut c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        let mut height = self.state.borrow_mut();
        if (height.to - self.target).abs() > 0.5 {
            height.from = if self.animate && height.started.is_some() {
                height.value()
            } else {
                self.target
            };
            height.to = self.target;
            height.started = Some(Instant::now());
        }
        let h = if self.animate {
            height.value()
        } else {
            self.target
        }
        .min(c.max.y());
        c.min.set_y(h);
        c.max.set_y(h);
        self.child.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }
    fn paint(&mut self, o: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(o, ctx, app);
        if self.animate
            && self
                .state
                .borrow()
                .started
                .is_some_and(|s| s.elapsed() < Duration::from_millis(180))
        {
            facloud_ui::animation::schedule_animation_tick(ctx, 16);
        }
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(e, ctx, app)
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}

/// Paint-only reveal: the search field and surrounding layout never move.
struct SearchReveal {
    child: El,
    width: std::rc::Rc<Cell<f32>>,
    animate: bool,
    started: Instant,
    closing: bool,
}
impl Element for SearchReveal {
    fn layout(
        &mut self,
        mut c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &AppContext,
    ) -> Vector2F {
        // The header is laid out first, so this uses its current width even on resize.
        let width = self.width.get().min(c.max.x());
        c.min.set_x(width);
        c.max.set_x(width);
        // Reserve the header, anchor gap, surrounding padding and status bar.
        // Otherwise position clamping can slide a tall popup over the input.
        c.max.set_y(c.max.y().min((ctx.window_size.y() - 104.).max(0.)));
        c.min.set_y(c.min.y().min(c.max.y()));
        self.child.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app)
    }
    fn paint(&mut self, o: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        if !self.animate {
            self.child.paint(o, ctx, app);
            return;
        }
        let t = (self.started.elapsed().as_secs_f32() / 0.18).clamp(0., 1.);
        let opacity = if self.closing {
            1. - motion::bezier_control(t)
        } else {
            motion::bezier_snappy_out(t)
        };
        let snapshot = ctx.scene.layer_snapshot();
        self.child
            .paint(o + vec2f(0., -6. * (1. - opacity)), ctx, app);
        ctx.scene.apply_opacity_since(&snapshot, opacity);
        if t < 1. {
            facloud_ui::animation::schedule_animation_tick(ctx, 16);
        }
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        !self.closing && self.child.dispatch_event(e, ctx, app)
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}
