//! Recent phone and desktop file inbox. Discovery and copies have independent workers, never the sync busy flag.
use super::*;
use super::{icons::UiIcon, theme::*, widgets::*};
use facloud_ui::{assets::asset_cache::AssetSource, fonts::Weight, platform::Cursor};
use stepx_companion::recent_files::{self as files, Kind, Preferences, RecentFile, Scan};

#[derive(Clone, Debug)]
pub(super) enum RecentFilesAction {
    Refresh,
    Source(u8),
    Kind(Option<Kind>),
    More,
    Folders,
    AddFolder,
    ToggleFolder(PathBuf),
    Open(String),
    Reveal(String),
    Transfer(String),
    OpenReceipt,
}
enum Event {
    PreferencesSaved(std::result::Result<(), String>),
    Desktop(Scan),
    Phone(String, std::result::Result<Scan, String>),
    Folder(std::result::Result<Option<PathBuf>, String>),
    Done(
        String,
        std::result::Result<(String, Option<PathBuf>), String>,
    ),
}
pub(super) struct RecentFilesState {
    pub desktop: Vec<RecentFile>,
    pub phone: Vec<RecentFile>,
    // Indices are rebuilt only when discovery returns new metadata.
    ordered: Vec<(bool, usize)>,
    prefs: Preferences,
    default_roots: Vec<PathBuf>,
    prefs_dirty: bool,
    prefs_saving: bool,
    device: Option<String>,
    source: u8,
    kind: Option<Kind>,
    more: bool,
    folders: bool,
    desktop_busy: bool,
    phone_busy: Option<String>,
    operation: Option<String>,
    desktop_at: Option<Instant>,
    phone_at: Option<Instant>,
    desktop_note: String,
    phone_note: String,
    note: String,
    receipt: Option<PathBuf>,
    tx: mpsc::Sender<Event>,
    rx: mpsc::Receiver<Event>,
}
impl Default for RecentFilesState {
    fn default() -> Self {
        let (tx, rx) = mpsc::channel();
        Self {
            desktop: vec![],
            phone: vec![],
            ordered: Vec::new(),
            prefs: Preferences::load(),
            default_roots: files::default_roots(),
            prefs_dirty: false,
            prefs_saving: false,
            device: None,
            source: 0,
            kind: None,
            more: false,
            folders: false,
            desktop_busy: false,
            phone_busy: None,
            operation: None,
            desktop_at: None,
            phone_at: None,
            desktop_note: String::new(),
            phone_note: String::new(),
            note: String::new(),
            receipt: None,
            tx,
            rx,
        }
    }
}
impl RecentFilesState {
    fn rebuild_order(&mut self) {
        let mut ordered: Vec<_> = (0..self.desktop.len()).map(|i| (false, i))
            .chain((0..self.phone.len()).map(|i| (true, i))).collect();
        let file = |(phone, i): (bool, usize)| if phone { &self.phone[i] } else { &self.desktop[i] };
        ordered.sort_by(|&a, &b| {
            let (a, b) = (file(a), file(b));
            b.at.cmp(&a.at).then_with(|| a.path.cmp(&b.path))
        });
        self.ordered = ordered;
    }

    fn visible(&self, limit: usize) -> Vec<&RecentFile> {
        self.ordered.iter().filter_map(|&(phone, i)| {
            let file = if phone { self.phone.get(i) } else { self.desktop.get(i) }?;
            ((self.source == 0 || (self.source == 2) == file.device.is_some())
                && self.kind.is_none_or(|k| file.kind == k)).then_some(file)
        }).take(limit).collect()
    }

}

impl Companion {
    pub(super) fn poll_recent_files(&mut self, ctx: &mut ViewContext<Self>) {
        let selected = self.selected.as_ref().map(|d| d.serial.clone());
        // Disconnect retains the last phone list, with offline state; changing phones clears it.
        if selected.is_some() && selected != self.recent_files.device {
            self.recent_files.device = selected.clone();
            self.recent_files.phone.clear();
            self.recent_files.rebuild_order();
            self.recent_files.phone_at = None;
            self.recent_files.phone_note.clear();
        }
        let mut changed = false;
        while let Ok(event) = self.recent_files.rx.try_recv() {
            changed = true;
            match event {
                Event::PreferencesSaved(result) => {
                    self.recent_files.prefs_saving = false;
                    if !self.recent_files.prefs_dirty {
                        self.recent_files.note = match result {
                            Ok(()) => "文件夹已更新".into(),
                            Err(e) => format!("文件夹设置未保存：{e}。请调整后重试。"),
                        };
                    }
                }
                Event::Desktop(scan) => {
                    self.recent_files.desktop_busy = false;
                    self.recent_files.desktop_note = if scan.limited {
                        "部分文件夹较大，当前显示已发现的最近文件".into()
                    } else {
                        String::new()
                    };
                    // Missing default folders are normal; report inaccessible explicitly chosen folders.
                    if scan
                        .unavailable
                        .iter()
                        .any(|p| self.recent_files.prefs.folders.contains(p))
                    {
                        self.recent_files.desktop_note =
                            "有关注的文件夹暂时无法访问，请检查文件夹设置".into();
                    }
                    self.recent_files.desktop = scan.files;
                    self.recent_files.rebuild_order();
                }
                Event::Phone(device, result) => {
                    if self.recent_files.phone_busy.as_ref() == Some(&device) {
                        self.recent_files.phone_busy = None;
                    }
                    if self.recent_files.device.as_ref() != Some(&device) {
                        continue;
                    }
                    match result {
                        Ok(scan) => {
                            self.recent_files.phone = scan.files;
                            self.recent_files.rebuild_order();
                            self.recent_files.phone_note = if scan.limited {
                                "手机文件较多，当前显示已发现的最近文件".into()
                            } else {
                                String::new()
                            };
                        }
                        Err(_) => {
                            self.recent_files.phone_note = "手机文件暂未更新，稍后自动重试".into()
                        }
                    }
                }
                Event::Folder(result) => {
                    self.recent_files.operation = None;
                    match result {
                        Ok(Some(path)) => {
                            self.recent_files.prefs.hidden.retain(|p| p != &path);
                            if !self.recent_files.prefs.folders.contains(&path) {
                                self.recent_files.prefs.folders.push(path);
                            }
                            self.save_recent_preferences();
                        }
                        Err(e) => self.recent_files.note = e,
                        _ => {}
                    }
                }
                Event::Done(id, result) => {
                    if self.recent_files.operation.as_ref() != Some(&id) {
                        continue;
                    }
                    self.recent_files.operation = None;
                    match result {
                        Ok((note, path)) => {
                            self.recent_files.note = note;
                            self.recent_files.receipt = path;
                            self.recent_files.desktop_at = None;
                            self.recent_files.phone_at = None;
                        }
                        Err(e) => {
                            self.record_notice("文件操作未完成", &e);
                            self.recent_files.note = format!("未完成：{e}");
                            self.recent_files.receipt = None;
                        }
                    }
                }
            }
        }
        // Coalesce edits while one durable write is in flight. Never race an
        // older preference snapshot against a later edit, or fsync on the UI.
        if self.recent_files.prefs_dirty && !self.recent_files.prefs_saving {
            self.recent_files.prefs_dirty = false;
            self.recent_files.prefs_saving = true;
            let prefs = self.recent_files.prefs.clone();
            let tx = self.recent_files.tx.clone();
            std::thread::spawn(move || {
                let _ = tx.send(Event::PreferencesSaved(prefs.save().map_err(|e| e.to_string())));
            });
        }
        // The module updates while other pages/AI jobs are active. No disk walk in render/tick.
        {
            // Demo/catalogue captures must never discover the host's private
            // directories or replace explicit fixtures with background scans.
            if !self.demo_mode
                && !self.recent_files.desktop_busy
                && self
                    .recent_files
                    .desktop_at
                    .is_none_or(|t| t.elapsed() > Duration::from_secs(8))
            {
                self.recent_files.desktop_busy = true;
                self.recent_files.desktop_at = Some(Instant::now());
                let prefs = self.recent_files.prefs.clone();
                let workspace = self.work.workspace.clone();
                let tx = self.recent_files.tx.clone();
                std::thread::spawn(move || {
                    let roots = prefs.roots(workspace.as_deref());
                    let _ = tx.send(Event::Desktop(files::desktop_files(&roots)));
                });
            }
            if !self.demo_mode
                && self.recent_files.phone_busy.is_none()
                && self
                    .recent_files
                    .phone_at
                    .is_none_or(|t| t.elapsed() > Duration::from_secs(30))
            {
                if let Some(device) = self.selected.clone() {
                    self.recent_files.phone_busy = Some(device.serial.clone());
                    self.recent_files.phone_at = Some(Instant::now());
                    let tx = self.recent_files.tx.clone();
                    std::thread::spawn(move || {
                        let result = Adb::discover()
                            .and_then(|adb| files::phone_files(&adb, &device))
                            .map_err(|e| e.to_string());
                        let _ = tx.send(Event::Phone(device.serial, result));
                    });
                }
            }
        }
        if changed {
            ctx.notify();
        }
    }
    fn save_recent_preferences(&mut self) {
        self.recent_files.prefs_dirty = true;
        self.recent_files.note = "正在保存文件夹设置…".into();
        self.recent_files.desktop_at = None;
    }
    pub(super) fn handle_recent_files(
        &mut self,
        action: &RecentFilesAction,
        ctx: &mut ViewContext<Self>,
    ) {
        match action {
            RecentFilesAction::Refresh => {
                self.recent_files.desktop_at = None;
                self.recent_files.phone_at = None;
            }
            RecentFilesAction::Source(s) => {
                self.recent_files.source = *s;
                if self.page != Page::Home {
                    self.scroll = Default::default();
                }
            }
            RecentFilesAction::Kind(k) => {
                self.recent_files.kind = *k;
                if self.page != Page::Home {
                    self.scroll = Default::default();
                }
            }
            RecentFilesAction::More => self.recent_files.more = !self.recent_files.more,
            RecentFilesAction::Folders => self.recent_files.folders = !self.recent_files.folders,
            RecentFilesAction::ToggleFolder(path) => {
                if self.recent_files.prefs.hidden.contains(path) {
                    self.recent_files.prefs.hidden.retain(|p| p != path);
                } else {
                    self.recent_files.prefs.hidden.push(path.clone());
                }
                self.save_recent_preferences();
            }
            RecentFilesAction::AddFolder => {
                if self.recent_files.operation.is_some() {
                    self.recent_files.note = "已有文件操作进行中，完成后可添加文件夹".into();
                    ctx.notify();
                    return;
                }
                self.recent_files.operation = Some("folder".into());
                let tx = self.recent_files.tx.clone();
                std::thread::spawn(move || {
                    let result = stepx_companion::file_dialog::FileDialog::new()
                        .set_title("选择自动发现新文件的文件夹")
                        .show_open_single_dir()
                        .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Folder(result));
                });
            }
            RecentFilesAction::OpenReceipt => {
                if let Some(path) = self.recent_files.receipt.clone() {
                    std::thread::spawn(move || {
                        let _ = work_state::reveal_in_folder(&path);
                    });
                }
            }
            RecentFilesAction::Open(id)
            | RecentFilesAction::Reveal(id)
            | RecentFilesAction::Transfer(id) => {
                if self.recent_files.operation.is_some() {
                    self.recent_files.note = "已有文件操作进行中，完成后可继续；其他页面仍可使用".into();
                    ctx.notify();
                    return;
                }
                let Some(file) = self
                    .recent_files
                    .desktop
                    .iter()
                    .chain(self.recent_files.phone.iter())
                    .find(|f| f.id() == *id)
                    .cloned()
                else {
                    return;
                };
                let device = self.selected.clone();
                let transfer = matches!(action, RecentFilesAction::Transfer(_));
                let reveal = matches!(action, RecentFilesAction::Reveal(_));
                if (transfer || file.device.is_some())
                    && (device.is_none()
                        || file
                            .device
                            .as_ref()
                            .is_some_and(|s| device.as_ref().is_none_or(|d| &d.serial != s)))
                {
                    self.recent_files.note = "连接这台手机后即可继续，已保存的文件仍可使用".into();
                    ctx.notify();
                    return;
                }
                self.recent_files.operation = Some(id.clone());
                self.recent_files.note = if transfer {
                    "正在复制并校验文件…"
                } else {
                    "正在打开文件…"
                }
                .into();
                self.recent_files.receipt = None;
                let id = id.clone();
                let tx = self.recent_files.tx.clone();
                std::thread::spawn(move || {
                    let result = (|| -> Result<(String, Option<PathBuf>)> {
                        if transfer && file.device.is_none() {
                            let adb = Adb::discover()?;
                            let target = files::send_to_phone(&adb, &device.unwrap(), &file)?;
                            Ok((
                                format!("已导入手机 · {}", target.trim_start_matches("/sdcard/")),
                                None,
                            ))
                        } else if file.device.is_some() {
                            let adb = Adb::discover()?;
                            let root = if transfer {
                                stepx_companion::settings::downloads()
                            } else {
                                stepx_companion::resources::user_state()
                                    .join("file-previews/recent")
                            };
                            let path =
                                files::save_to_desktop(&adb, &device.unwrap(), &file, &root)?;
                            if !transfer {
                                super::global_search::open_search_file(&path)?;
                            }
                            Ok((
                                if transfer {
                                    format!("已保存到电脑 · {}", path.display())
                                } else {
                                    "已打开手机文件的电脑副本".into()
                                },
                                Some(path),
                            ))
                        } else {
                            anyhow::ensure!(file.path.is_file(), "文件已移动或删除，请刷新");
                            if reveal {
                                work_state::reveal_in_folder(&file.path)?;
                            } else {
                                super::global_search::open_search_file(&file.path)?;
                            }
                            Ok((
                                if reveal {
                                    "已打开所在文件夹"
                                } else {
                                    "已打开文件"
                                }
                                .into(),
                                Some(file.path),
                            ))
                        }
                    })()
                    .map_err(|e| e.to_string());
                    let _ = tx.send(Event::Done(id, result));
                });
            }
        }
        ctx.notify();
    }
    pub(super) fn render_recent_files(&self) -> El {
        let home = self.page == Page::Home;
        use super::pages::home::{HomeModuleHeight, HomeModuleId};
        let home_height = self.resolve_effective_height(
            HomeModuleId::RecentFiles,
            self.home_modules.module_height(HomeModuleId::RecentFiles),
        );
        let compact = home && home_height == HomeModuleHeight::Compact;
        let home_profile = self.compute_home_module_value_profile(HomeModuleId::RecentFiles,
            self.home_modules.module_width(HomeModuleId::RecentFiles),
            self.home_modules.module_height(HomeModuleId::RecentFiles));
        let p = self.p();
        let state = &self.recent_files;
        let action = |a| Action::RecentFiles(a);
        let mut body = vec![self.bounded_wrap(vec![
            self.home_section_heading(UiIcon::Folder, "最近文件"),
            self.hstack(
                S1,
                vec![
                    if home {
                        self.button(
                            "recent-all",
                            "全部文件",
                            Action::Page(Page::Files),
                            Btn::Ghost,
                        )
                    } else {
                        Empty::new().finish()
                    },
                    self.icon_action(
                        "recent-folders",
                        "关注的文件夹",
                        UiIcon::Settings,
                        vec![action(RecentFilesAction::Folders)],
                        state.folders,
                    ),
                    self.icon_action(
                        "recent-refresh",
                        "刷新文件",
                        UiIcon::Refresh,
                        vec![action(RecentFilesAction::Refresh)],
                        false,
                    ),
                ],
            ),
        ])];
        let source_filters = self.bounded_wrap(
                    [("综合", 0), ("电脑", 1), ("手机", 2)]
                        .into_iter()
                        .map(|(label, source)| {
                            self.chip(
                                format!("recent-source-{source}"),
                                label,
                                None,
                                vec![action(RecentFilesAction::Source(source))],
                                state.source == source,
                                false,
                            )
                        })
                        .collect(),
            );
        let type_filters = if compact { Empty::new().finish() } else { self.bounded_wrap(
                    [
                        ("全部", None),
                        ("图片", Some(Kind::Image)),
                        ("文档", Some(Kind::Document)),
                        ("PDF", Some(Kind::Pdf)),
                        ("影音", Some(Kind::Media)),
                        ("压缩包", Some(Kind::Archive)),
                    ]
                    .into_iter()
                    .map(|(label, kind)| {
                        let button = self.chip(
                            format!("recent-kind-{label}"),
                            if kind.is_none() { "全部类型" } else { label },
                            None,
                            vec![action(RecentFilesAction::Kind(kind))],
                            state.kind == kind,
                            false,
                        );
                        #[cfg(test)]
                        let button = SavePosition::new(button, &format!("recent-filter-test-{label}")).finish();
                        button
                    })
                    .collect(),
            )
        };
        if compact {
            body.push(source_filters);
            // Detailed type filters live on the full files page. Keep an active
            // filter visible and reversible when returning from a larger card.
            if state.kind.is_some() {
                body.push(self.chip("recent-kind-reset", "显示全部类型", Some(UiIcon::Close),
                    vec![action(RecentFilesAction::Kind(None))], false, false));
            }
        } else {
            body.push(self.responsive_columns(640., S3, S1, CrossAxisAlignment::Center,
                vec![(1., source_filters), (2.5, type_filters)]));
        }
        if state.folders {
            body.push(self.caption(
                "自动发现以下文件夹的新建和更新文件。手机读取下载、相册、文档等共享目录。",
            ));
            let mut roots = state.default_roots.clone();
            roots.extend(state.prefs.folders.clone());
            if let Some(w) = &self.work.workspace {
                roots.push(w.clone());
            }
            roots.sort();
            roots.dedup();
            for (i, path) in roots.iter().enumerate() {
                body.push(self.between(
                    self.line(path.to_string_lossy(), MICRO, p.text2),
                    self.button(
                        format!("recent-folder-{i}"),
                        if state.prefs.hidden.contains(path) {
                            "启用"
                        } else {
                            "移除"
                        },
                        action(RecentFilesAction::ToggleFolder(path.clone())),
                        Btn::Ghost,
                    ),
                ));
            }
            body.push(self.button_icon(
                "recent-add-folder",
                "添加文件夹",
                UiIcon::Plus,
                action(RecentFilesAction::AddFolder),
                Btn::Secondary,
            ));
        }
        // A compact, wide card is a small file shelf, not two stretched rows.
        // Keep two actual rows of information at either width; discovery order
        // remains unchanged when resizing and filters still apply before limits.
        let compact_columns = if home && self.home_module_available_width(HomeModuleId::RecentFiles) >= 932. { 2 } else { 1 };
        let home_limit = if compact { compact_columns * 2 } else { home_profile.max_items };
        let visible = state.visible(if home { home_limit } else { usize::MAX });
        let page_key = format!("recent-files:{}:{:?}",state.source,state.kind);
        let range = if home {0..visible.len().min(home_limit)} else {self.list_page_range(&page_key,visible.len(),20)};
        if !home { body.push(self.list_page_controls(&page_key,visible.len(),20)); }
        if visible.is_empty() {
            let hint = if state.source == 2 && self.selected.is_none() {
                "连接手机后，最近下载、拍摄和收到的文件会出现在这里"
            } else if state.desktop_busy && state.desktop.is_empty() {
                "正在发现最近文件…"
            } else if state.kind.is_some() || state.source != 0 {
                "当前筛选下没有文件，试试其他来源或文件类型。"
            } else {
                "这里还没有文件。保存到下载、文档、图片文件夹，或添加你常用的导出目录。"
            };
            body.push(
                Container::new(self.caption(hint))
                    .with_uniform_padding(S4)
                    .finish(),
            );
        }
        let mut rows = Vec::new();
        for (index, file) in visible.iter().enumerate().skip(range.start).take(range.len()) {
            let id = file.id();
            let phone = file.device.is_some();
            let thumb = if let Some(path) = &file.thumbnail {
                Image::new(
                    AssetSource::LocalFile {
                        path: path.to_string_lossy().into_owned(),
                    },
                    CacheOption::BySize,
                )
                .contain()
                .finish()
            } else {
                self.section_icon_badge(
                    super::icon_style::file_content_icon(&file.name(), false),
                    24.,
                    40.,
                )
            };
            let thumb = ConstrainedBox::new(
                Container::new(thumb)
                    .with_uniform_padding(4.)
                    .with_background_color(p.surface)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                    .finish(),
            )
            .with_width(40.)
            .with_height(40.)
            .finish();
            use chrono::TimeZone;
            let stamp = chrono::Local
                .timestamp_millis_opt(file.at)
                .single()
                .map(|d| d.format("%m-%d %H:%M").to_string())
                .unwrap_or_default();
            let info = self.vstack(
                S1,
                vec![
                    self.line_w(file.name(), BODY, p.text, Weight::Semibold),
                    self.line(
                        format!(
                            "{} · {} · {} · {}",
                            if phone { "手机" } else { "电脑" },
                            file.folder,
                            storage::format_bytes(file.bytes),
                            stamp
                        ),
                        MICRO,
                        p.text2,
                    ),
                ],
            );
            let open = self.button_icon(
                format!("recent-open-{index}"),
                "打开",
                UiIcon::ExternalLink,
                action(RecentFilesAction::Open(id.clone())),
                Btn::Ghost,
            );
            #[cfg(test)]
            let open = SavePosition::new(open, &format!("recent-test-open-{index}")).finish();
            let transfer = if state.operation.is_some() {
                self.disabled_button(if state.operation.as_ref() == Some(&id) { "处理中…" } else { "请稍候" })
            } else {
                self.button_icon(
                    format!("recent-transfer-{index}"),
                    if phone {
                        "存到电脑"
                    } else {
                        "传到手机"
                    },
                    if phone {
                        UiIcon::Download
                    } else {
                        UiIcon::Upload
                    },
                    action(RecentFilesAction::Transfer(id.clone())),
                    Btn::Secondary,
                )
            };
            #[cfg(test)]
            let transfer = SavePosition::new(transfer, &format!("recent-test-transfer-{index}")).finish();
            let mut controls = vec![open];
            if !phone {
                controls.push(self.icon_action(
                    format!("recent-reveal-{index}"),
                    "所在文件夹",
                    UiIcon::Folder,
                    vec![action(RecentFilesAction::Reveal(id))],
                    false,
                ));
            }
            controls.push(transfer);
            let open_action = action(RecentFilesAction::Open(file.id()));
            let content = Hoverable::new(
                self.mouse_state(format!("recent-file:{}", file.id())),
                move |_| self.hstack(S3, vec![thumb, Expanded::new(1., info).finish()]),
            )
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(open_action.clone()))
            .finish();
            #[cfg(feature = "ui-capture")]
            let content = super::scenario_check::control(format!("recent-file-{index}"), content);
            let card = if compact {
                self.responsive_columns(
                    640.,
                    S2,
                    S1,
                    CrossAxisAlignment::Center,
                    vec![
                        (1.4, content),
                        (1., Align::new(self.bounded_wrap(controls)).right().finish()),
                    ],
                )
            } else { self.vstack(
                S1,
                vec![
                    content,
                    self.wrap(vec![
                        {
                            let style = super::icon_style::content_style(
                                super::icon_style::file_content_icon(&file.name(), false),
                                &p,
                            );
                            self.wrap(vec![self.pill_badge(
                                super::icon_style::file_content_label(&file.name()),
                                style.foreground,
                                style.background,
                            )])
                        },
                        self.wrap(controls),
                    ]),
                ],
            ) };
            let card = Container::new(card)
                .with_uniform_padding(S2)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                .finish();
            #[cfg(test)]
            let card = SavePosition::new(card, &format!("recent-test-file-{index}")).finish();
            rows.push(card);
        }
        let mut iter = rows.into_iter();
        while let Some(a) = iter.next() {
            if let Some(b) = iter.next() {
                body.push(self.responsive_columns(
                    if compact { 900. } else { 680. },
                    S2,
                    S2,
                    CrossAxisAlignment::Stretch,
                    vec![(1., a), (1., b)],
                ));
            } else {
                body.push(a);
            }
        }
        let status = if !state.note.is_empty() {
            state.note.clone()
        } else if !state.phone_note.is_empty() {
            state.phone_note.clone()
        } else if !state.desktop_note.is_empty() {
            state.desktop_note.clone()
        } else if self.selected.is_none() {
            "电脑自动更新 · 手机未连接".into()
        } else {
            "自动更新 · 传输保留原文件".into()
        };
        let mut footer = vec![];
        if state.receipt.is_some() {
            footer.push(self.button_icon(
                "recent-receipt",
                "接收位置",
                UiIcon::Folder,
                action(RecentFilesAction::OpenReceipt),
                Btn::Ghost,
            ));
        }
        let footer = self.between(self.line(status, MICRO, p.text2), self.hstack(S1, footer));
        if self.page == Page::Home {
            self.home_frame_with_footer(pages::home::HomeModuleId::RecentFiles, body, Some(footer))
        } else {
            body.push(footer);
            self.panel(body)
        }
    }
    #[cfg(feature = "ui-capture")]
    pub(super) fn recent_files_diagnostics(&self) -> serde_json::Value {
        serde_json::json!({"desktop":self.recent_files.desktop.len(),"phone":self.recent_files.phone.len(),"desktop_busy":self.recent_files.desktop_busy,"phone_busy":self.recent_files.phone_busy,"operation":self.recent_files.operation,"note":self.recent_files.note,"phone_note":self.recent_files.phone_note,"files":self.recent_files.visible(24).iter().map(|f|serde_json::json!({"id":f.id(),"name":f.name(),"at":f.at,"phone":f.device.is_some(),"bytes":f.bytes})).collect::<Vec<_>>()})
    }
}

#[cfg(test)]
mod layout_tests {
    use super::*;
    use super::super::pages::home::{HomeModuleHeight, HomeModuleId, HomeModuleWidth};
    #[test]
    fn recent_order_tracks_discovery_and_filters_before_limiting() {
        let mut state = RecentFilesState::default();
        let file = |name: &str, at: i64, phone: bool, kind: Kind| RecentFile {
            path: PathBuf::from(name), device: phone.then(|| "device".into()),
            bytes: 1, at, modified: at, kind, folder: String::new(), thumbnail: None,
        };
        state.desktop = vec![file("b.pdf", 10, false, Kind::Pdf), file("old.png", 1, false, Kind::Image)];
        state.phone = vec![file("a.pdf", 10, true, Kind::Pdf), file("new.png", 20, true, Kind::Image)];
        state.rebuild_order();
        assert_eq!(state.visible(2).iter().map(|f| f.name()).collect::<Vec<_>>(), ["new.png", "a.pdf"]);
        state.source = 1;
        assert_eq!(state.visible(1)[0].name(), "b.pdf");
        state.source = 2; state.kind = Some(Kind::Pdf);
        assert_eq!(state.visible(1)[0].name(), "a.pdf");
        // A rescan can replace an item in-place with an earlier/later timestamp.
        state.phone[0].at = 30; state.kind = None; state.source = 0;
        state.rebuild_order();
        assert_eq!(state.visible(1)[0].name(), "a.pdf");
        // Device change clears phone entries without stale index references.
        state.phone.clear(); state.rebuild_order();
        assert_eq!(state.visible(24).len(), 2);
        assert!(state.visible(0).is_empty());
    }
    struct RecentOnly(Companion);
    impl facloud_ui::Entity for RecentOnly { type Event = (); }
    impl facloud_ui::TypedActionView for RecentOnly { type Action = (); }
    impl facloud_ui::View for RecentOnly {
        fn ui_name() -> &'static str { "recent_files_layout_test" }
        fn render(&self, _: &facloud_ui::AppContext) -> El {
            Stack::new().with_child(Align::new(
                SavePosition::new(self.0.render_recent_files(), "recent-test-panel").finish()
            ).top_left().finish()).finish()
        }
    }
    #[test]
    fn compact_files_use_wide_space_without_overlapping_actions() {
        facloud_ui::App::test((), |mut app| async move {
            for width in [300., 420., 720., 1000., 1420.] {
                let mut companion = Companion::new_for_test();
                companion.page = Page::Home;
                companion.workspace.viewport = vec2f(width + 278., 900.);
                companion.home_modules.enabled = vec![HomeModuleId::RecentFiles];
                companion.home_modules.set_module_size(HomeModuleId::RecentFiles,
                    HomeModuleWidth::Full, HomeModuleHeight::Compact);
                companion.recent_files.desktop = (0..8).map(|index| RecentFile {
                    path: PathBuf::from(format!("/tmp/项目交付文件与详细说明-{index}.pdf")),
                    device: None, bytes: 100, at: 10 - index, modified: 1,
                    kind: Kind::Pdf, folder: "下载".into(), thumbnail: None,
                }).collect();
                companion.recent_files.phone.clear();
                companion.recent_files.rebuild_order();
                let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                    |_| RecentOnly(companion));
                let root = app.root_view_id(window).unwrap();
                app.update(move |ctx| {
                    let mut presenter = facloud_ui::Presenter::new(window);
                    presenter.invalidate(facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]), ..Default::default()
                    }, ctx);
                    presenter.build_scene(vec2f(width, 2000.), 1., None, ctx);
                    let positions = presenter.position_cache();
                    let first = positions.get_position("recent-test-file-0").unwrap();
                    let second = positions.get_position("recent-test-file-1").unwrap();
                    if width >= 1000. {
                        assert!((first.origin().y() - second.origin().y()).abs() < 1.,
                            "wide compact file shelf should have two columns: {first:?} / {second:?}");
                        assert!(positions.get_position("recent-test-file-3").is_some());
                    } else {
                        assert!(second.origin().y() >= first.max_y(), "narrow files must stack");
                        assert!(positions.get_position("recent-test-file-2").is_none());
                    }
                    for index in 0..if width >= 1000. { 4 } else { 2 } {
                        let row = positions.get_position(&format!("recent-test-file-{index}")).unwrap();
                        let open = positions.get_position(&format!("recent-test-open-{index}")).unwrap();
                        let transfer = positions.get_position(&format!("recent-test-transfer-{index}")).unwrap();
                        for control in [open, transfer] {
                            assert!(control.origin().x() >= row.origin().x() && control.max_x() <= row.max_x()
                                && control.max_y() <= row.max_y(), "file action outside {width}px row: {control:?} / {row:?}");
                        }
                        assert!(open.max_x() <= transfer.origin().x() || open.max_y() <= transfer.origin().y(),
                            "file actions overlap at {width}px");
                    }
                });
            }
        });
    }

    #[test]
    fn file_type_filters_wrap_in_narrow_home_cards() {
        facloud_ui::App::test((), |mut app| async move {
            for width in [260., 390., 720.] {
                for height in [HomeModuleHeight::Compact, HomeModuleHeight::Standard, HomeModuleHeight::Tall] {
                    let mut companion = Companion::new_for_test();
                    companion.page = Page::Home;
                    companion.home_modules.set_module_size(HomeModuleId::RecentFiles, HomeModuleWidth::OneThird, height);
                    let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::NotStealFocus,
                        |_| RecentOnly(companion));
                    let root = app.root_view_id(window).unwrap();
                    app.update(move |ctx| {
                        let mut presenter = facloud_ui::Presenter::new(window);
                        presenter.invalidate(facloud_ui::WindowInvalidation {
                            updated: std::collections::HashSet::from([root]), ..Default::default()
                        }, ctx);
                        presenter.build_scene(vec2f(width, 2000.), 1., None, ctx);
                        let positions = presenter.position_cache();
                        let panel = positions.get_position("recent-test-panel").unwrap();
                        assert!(panel.max_x() <= width + 1.);
                        for label in ["全部", "图片", "文档", "PDF", "影音", "压缩包"] {
                            let position = positions.get_position(&format!("recent-filter-test-{label}"));
                            if height == HomeModuleHeight::Compact {
                                assert!(position.is_none(), "summary keeps source filters, file types belong to the expanded view");
                                continue;
                            }
                            let r = position.unwrap();
                            assert!(r.origin().x() >= panel.origin().x() && r.max_x() <= panel.max_x()
                                && r.max_y() <= panel.max_y(), "{label} at {width}: {r:?} outside {panel:?}");
                        }
                    });
                }
            }
        });
    }
}

#[cfg(feature = "ui-capture")]
impl Companion {
    /// Isolated catalogue fixture: real local demo files plus phone metadata.
    /// This only renders rows; it neither reads a phone nor executes transfers.
    pub(super) fn preview_recent_files(&mut self, root: PathBuf) {
        if !self.demo_mode { return; }
        let mut scan = files::desktop_files(&[root]);
        scan.files.sort_by_key(|f| f.name());
        let serial = self.selected.as_ref().map(|d| d.serial.clone()).unwrap_or_else(|| "review-phone".into());
        self.recent_files.device = Some(serial.clone());
        self.recent_files.desktop.clear(); self.recent_files.phone.clear();
        for (index, mut file) in scan.files.into_iter().enumerate() {
            if index % 2 == 0 { self.recent_files.desktop.push(file); }
            else {
                file.device = Some(serial.clone()); file.folder = "Download · 手机演示资料".into();
                file.path = PathBuf::from("/sdcard/Download").join(file.name());
                self.recent_files.phone.push(file);
            }
        }
        self.recent_files.rebuild_order();
        self.recent_files.desktop_at = Some(Instant::now()); self.recent_files.phone_at = Some(Instant::now());
        self.recent_files.desktop_note.clear(); self.recent_files.phone_note.clear();
        self.recent_files.source = 0; self.recent_files.kind = None;
        self.recent_files_view = true; self.page = Page::Files;
    }
}
