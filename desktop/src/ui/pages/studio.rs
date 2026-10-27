use super::super::{
    icons::UiIcon,
    studio_state::{StudioAction as St, CATEGORIES},
    theme::*,
    widgets::*,
    Action, Companion, F_STUDIO_NAME, F_STUDIO_SEARCH, F_STUDIO_URL,
};
use facloud_ui::{assets::asset_cache::AssetSource, elements::*};
use stepx_companion::studio::{CloudWork, Work};

impl Companion {
    pub(in super::super) fn studio_actions(&self) -> Vec<El> {
        vec![
            self.st_button(
                "st-import-local",
                "导入本地作品",
                St::ImportLocalFile,
                Btn::Secondary,
            ),
            self.st_button(
                "st-open-works-dir",
                "打开作品目录",
                St::OpenWorksFolder,
                Btn::Secondary,
            ),
            self.st_button("st-sync", "同步", St::CloudRefresh, Btn::Secondary),
            if self.studio_lab.handle.is_some() {
                self.st_button("st-create", "查看运行任务", St::Tab(2), Btn::Secondary)
            } else if self.busy {
                self.disabled_button("新建作品")
            } else {
                // 新建作品 = 回到 Amoo 开新对话，直接说要做什么。
                self.button_icon(
                    "st-create",
                    "新建作品",
                    UiIcon::Plus,
                    Action::Studio(St::NewInAmoo),
                    Btn::Primary,
                )
            },
        ]
    }
    fn st_button(&self, id: &str, label: &str, action: St, style: Btn) -> El {
        let navigation = matches!(
            action,
            St::Tab(_)
                | St::Source(_)
                | St::SetFilter(_)
                | St::SetDeliverableFilter(_)
                | St::SetSortMode(_)
                | St::ImportLocalFile
                | St::OpenWorksFolder
                | St::CloudCategory(_)
                | St::CloudSelect(_)
                | St::WebSelect(_)
                | St::Select(_)
                | St::ListPage(_)
                | St::Edit(_)
                | St::BackToTask(_)
                | St::ContinueOutputInAmoo(..)
                | St::ContinueLabInAmoo(_)
                | St::OpenPath(_)
                | St::RevealPath(_)
                | St::AmooWorks
                | St::NewInAmoo
        );
        if self.busy && !navigation {
            self.disabled_button(label)
        } else {
            let icon = match &action {
                St::CloudRefresh | St::Refresh => Some(UiIcon::Refresh),
                St::New | St::NewInAmoo => Some(UiIcon::Plus),
                St::UsePhone(_) | St::OpenPhone | St::PhoneLibrary => Some(UiIcon::Smartphone),
                St::SaveQr | St::ImportLocalFile => Some(UiIcon::Download),
                St::PasteLink => Some(UiIcon::Clipboard),
                St::ContinueOutputInAmoo(..) | St::ContinueLabInAmoo(_) => Some(UiIcon::Chat),
                St::OpenPath(_) => Some(UiIcon::Play),
                St::RevealPath(_) | St::OpenWorksFolder => Some(UiIcon::Folder),
                St::SetDeliverableFilter(1) => Some(UiIcon::Chart),
                St::SetDeliverableFilter(2) => Some(UiIcon::File),
                St::SetDeliverableFilter(3) => Some(UiIcon::Chart),
                St::SetDeliverableFilter(4) => Some(UiIcon::Apps),
                St::SetDeliverableFilter(5) => Some(UiIcon::Image),
                _ => None,
            };
            match icon {
                Some(icon) => self.button_icon(id, label, icon, Action::Studio(action), style),
                None => self.button(id, label, Action::Studio(action), style),
            }
        }
    }
    fn studio_sub_enter(&self, child: El) -> El {
        self.sub_enter(child)
    }
    pub(in super::super) fn studio_page(&self) -> El {
        let p = self.p();
        let active_tab = if self.studio.tab == 4 {
            0
        } else {
            self.studio.tab
        };
        let tabs = self.segmented(
            "studio-tabs",
            [
                (0, "我的作品库"),
                (2, "高级创作工坊"),
                (3, "连接与模型设置"),
            ]
            .iter()
            .map(|(i, t)| ((*t).into(), active_tab == *i, Action::Studio(St::Tab(*i))))
            .collect(),
        );
        let web = self.studio.web.as_ref().is_some_and(|c| c.authorized);
        let phone = self.studio.cloud.as_ref().is_some_and(|c| c.authorized);
        let device_connected = self.selected.is_some() || self.amoo_gateway_online;
        let connection = if let Some(d) = &self.selected {
            let model = if d.model.contains("StepX") {
                d.model.clone()
            } else if d.model.is_empty() {
                "手机".to_string()
            } else {
                format!("StepX {}", d.model)
            };
            format!("已连接 {model}")
        } else if self.amoo_gateway_online {
            "手机已连接".to_string()
        } else if web {
            "官网已连接".to_string()
        } else if phone {
            "手机作品已连接".to_string()
        } else {
            "未连接手机 · 本机可用".to_string()
        };
        let is_online = web || phone || device_connected;
        let status_pill = Container::new(self.hstack(
            S2,
            vec![
                self.dot(if is_online { p.success } else { p.text3 }),
                self.line(connection, MICRO, if is_online { p.text2 } else { p.text3 }),
            ],
        ))
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(999.)))
        .with_horizontal_padding(10.)
        .with_vertical_padding(4.)
        .finish();
        let top_bar = Container::new(self.between(self.wrap(vec![tabs]), status_pill))
            .with_horizontal_padding(S5)
            .with_padding_top(S4)
            .with_padding_bottom(S3)
            .finish();
        let body_el = if self.studio.tab == 2 {
            self.studio_lab_page()
        } else {
            let query = self.fields[F_STUDIO_SEARCH].text.trim().to_lowercase();
            let inner = match self.studio.tab {
                3 => self.studio_connections(),
                4 => self.studio_all_outputs(&query),
                _ => self.studio_workspace(),
            };
            Expanded::new(
                1.,
                self.pane_scroll(
                    self.scroll.clone(),
                    Container::new(self.studio_sub_enter(inner))
                        .with_horizontal_padding(S5)
                        .with_padding_top(S2)
                        .with_padding_bottom(S6)
                        .finish(),
                ),
            )
            .finish()
        };
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(top_bar)
            .with_child(body_el)
            .finish()
    }
    fn studio_outputs_count(&self) -> usize {
        self.workspace
            .journal
            .tasks
            .iter()
            .filter(|t| !t.outputs.is_empty() && self.workspace_visible(t))
            .map(|t| t.outputs.len())
            .sum::<usize>()
            .min(80)
    }
    fn output_deliverable_cat(output: &stepx_companion::amoo_workspace::Output) -> u8 {
        let ext = output
            .path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_ascii_lowercase();
        if output.lab_id.is_some()
            || output.kind.contains("应用")
            || output.kind.contains("网页")
            || matches!(ext.as_str(), "html" | "htm")
        {
            4
        } else if matches!(ext.as_str(), "ppt" | "pptx")
            || output.kind.contains("演示")
            || output.kind.contains("PPT")
        {
            1
        } else if matches!(ext.as_str(), "xls" | "xlsx" | "csv")
            || output.kind.contains("表格")
            || output.kind.contains("数据")
        {
            3
        } else if matches!(
            ext.as_str(),
            "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg"
        ) || output.kind.contains("图片")
            || output.kind.contains("海报")
        {
            5
        } else {
            2
        }
    }

    fn studio_deliverable_filter_bar(&self) -> El {
        let active_cat = self.studio.deliverable_filter;
        let active_sort = self.studio.sort_mode;
        let cat_btns: Vec<El> = [
            (0u8, "全部类型"),
            (1u8, "PPT 演示"),
            (2u8, "文档报告"),
            (3u8, "表格数据"),
            (4u8, "网页与应用"),
            (5u8, "海报图片"),
        ]
        .into_iter()
        .map(|(cat, label)| {
            self.st_button(
                &format!("st-deliv-cat-{cat}"),
                label,
                St::SetDeliverableFilter(cat),
                if active_cat == cat {
                    Btn::Primary
                } else {
                    Btn::Secondary
                },
            )
        })
        .collect();
        let sort_btns: Vec<El> = [(0u8, "按最新"), (1u8, "按名称"), (2u8, "按类型")]
            .into_iter()
            .map(|(mode, label)| {
                self.st_button(
                    &format!("st-deliv-sort-{mode}"),
                    label,
                    St::SetSortMode(mode),
                    if active_sort == mode {
                        Btn::Secondary
                    } else {
                        Btn::Ghost
                    },
                )
            })
            .collect();
        self.between(self.wrap(cat_btns), self.wrap(sort_btns))
    }

    /// 「对话成果」：Amoo 对话里做出的所有文件和应用。支持直接打开、在 Amoo 中继续改或回到原对话。
    fn studio_all_outputs(&self, query: &str) -> El {
        let p = self.p();
        let mut rows = vec![self.between(
            self.vstack(
                2.,
                vec![
                    self.heading("Amoo 对话成果"),
                    self.text(
                        "在 Amoo 对话里做出的应用、文档、表格、PPT 与图片。可一键预览或在 Amoo 中接着改。",
                        CAPTION,
                        p.text2,
                    ),
                ],
            ),
            self.st_button("st-all-new", "去 Amoo 做新作品", St::NewInAmoo, Btn::Secondary),
        )];
        if self.studio.tab == 4 {
            rows.push(self.studio_deliverable_filter_bar());
        }
        let mut tasks: Vec<_> = self
            .workspace
            .journal
            .tasks
            .iter()
            .filter(|t| !t.outputs.is_empty() && self.workspace_visible(t))
            .collect();
        tasks.sort_by(|a, b| b.updated.cmp(&a.updated));
        let mut matched_outputs = Vec::new();
        'tasks: for task in tasks {
            let raw_title = task.title.trim();
            let title: String = if raw_title.is_empty() {
                "未命名对话".into()
            } else if raw_title.chars().count() > 26 {
                format!("{}…", raw_title.chars().take(26).collect::<String>())
            } else {
                raw_title.into()
            };
            for (i, output) in task.outputs.iter().enumerate().rev() {
                if matched_outputs.len() >= 80 {
                    break 'tasks;
                }
                let cat = Self::output_deliverable_cat(output);
                if self.studio.deliverable_filter != 0 && self.studio.deliverable_filter != cat {
                    continue;
                }
                if !query.is_empty()
                    && !output.name.to_lowercase().contains(query)
                    && !output.kind.to_lowercase().contains(query)
                    && !raw_title.to_lowercase().contains(query)
                {
                    continue;
                }
                matched_outputs.push((task, title.clone(), i, output, cat));
            }
        }
        match self.studio.sort_mode {
            1 => matched_outputs.sort_by(|a, b| {
                a.3.name
                    .to_lowercase()
                    .cmp(&b.3.name.to_lowercase())
                    .then_with(|| b.0.updated.cmp(&a.0.updated))
            }),
            2 => matched_outputs.sort_by(|a, b| {
                a.4.cmp(&b.4)
                    .then_with(|| a.3.name.to_lowercase().cmp(&b.3.name.to_lowercase()))
            }),
            _ => {}
        }
        let mut cards = vec![];
        for (task, title, i, output, _) in matched_outputs {
            {
                let ext = output
                    .path
                    .extension()
                    .and_then(|e| e.to_str())
                    .unwrap_or("")
                    .to_ascii_lowercase();
                let is_app = output.lab_id.is_some()
                    || output.kind.contains("应用")
                    || matches!(ext.as_str(), "html" | "htm");
                let (icon, badge_label, badge_color) = if is_app {
                    (UiIcon::Apps, "交互应用", p.accent)
                } else if matches!(
                    ext.as_str(),
                    "png" | "jpg" | "jpeg" | "webp" | "gif" | "svg"
                ) {
                    (UiIcon::Image, "图片成果", p.accent)
                } else if matches!(ext.as_str(), "doc" | "docx") || output.kind.contains("文档") {
                    (UiIcon::File, "Word 文档", p.accent)
                } else if matches!(ext.as_str(), "xls" | "xlsx" | "csv")
                    || output.kind.contains("表格")
                {
                    (UiIcon::File, "Excel 表格", p.success)
                } else if matches!(ext.as_str(), "ppt" | "pptx") || output.kind.contains("演示") {
                    (UiIcon::File, "PPT 演示", p.warning)
                } else {
                    let k = if output.kind.trim().is_empty() {
                        "文件成果"
                    } else {
                        output.kind.as_str()
                    };
                    (UiIcon::File, k, p.text2)
                };
                let tid = task.id.clone();
                let path = output.path.clone();
                let menu_path = path.clone();
                let open_click = path.clone();
                let open_double = path.clone();
                let menu_tid = tid.clone();
                let menu_title = output.name.clone();
                cards.push(self.hover_panel_interactive(
                    format!("st-out-card-{tid}-{i}"),
                    self.vstack(
                        S3,
                        vec![
                            self.between(
                                self.lead(
                                    S3,
                                    self.studio_icon_box(icon, 40., badge_color, p.surface),
                                    Expanded::new(
                                        1.,
                                        self.vstack(
                                            2.,
                                            vec![
                                                self.heading(&output.name),
                                                self.line(
                                                    format!("来自对话「{title}」"),
                                                    CAPTION,
                                                    p.text2,
                                                ),
                                            ],
                                        ),
                                    )
                                    .finish(),
                                ),
                                self.pill_badge(badge_label, badge_color, p.surface),
                            ),
                            self.wrap(vec![
                                self.st_button(
                                    &format!("st-out-continue-{tid}-{i}"),
                                    "在 Amoo 中继续改",
                                    St::ContinueOutputInAmoo(tid.clone(), i),
                                    Btn::Primary,
                                ),
                                self.st_button(
                                    &format!("st-out-open-{tid}-{i}"),
                                    if is_app { "立即试玩" } else { "打开预览" },
                                    St::OpenPath(path.clone()),
                                    Btn::Secondary,
                                ),
                                self.st_button(
                                    &format!("st-out-reveal-{tid}-{i}"),
                                    "所在文件夹",
                                    St::RevealPath(path),
                                    Btn::Ghost,
                                ),
                                self.st_button(
                                    &format!("st-out-task-{tid}-{i}"),
                                    "回到原对话",
                                    St::BackToTask(tid),
                                    Btn::Ghost,
                                ),
                            ]),
                        ],
                    ),
                    Some(Action::Studio(St::OpenPath(open_click))),
                    Some(Action::Studio(St::OpenPath(open_double))),
                    Some(crate::ui::context_menu::Target::StudioWork(
                        crate::ui::context_menu::StudioWorkTarget::WorkspaceOutput {
                            task_id: menu_tid,
                            output_idx: i,
                            path: menu_path,
                            title: menu_title,
                            is_app,
                        },
                    )),
                ));
            }
        }
        if cards.is_empty() {
            rows.push(self.studio_empty(
                if query.is_empty() {
                    "还没有对话成果"
                } else {
                    "没有匹配的对话成果"
                },
                if query.is_empty() {
                    "回到 Amoo 说一句“帮我做个番茄钟应用”或“整理一份双周报表格”，生成后的成果会自动收录在这里。"
                } else {
                    "试试其他关键词，或清空搜索框查看全部成果。"
                },
            ));
        } else {
            rows.push(self.vstack(S3, cards));
        }
        self.vstack(S4, rows)
    }
    fn studio_workspace(&self) -> El {
        let p = self.p();
        let source = self.studio.source;
        let count = |web: bool| {
            if web {
                self.studio.web.as_ref()
            } else {
                self.studio.cloud.as_ref()
            }
            .filter(|c| c.authorized)
            .map(|c| c.items.len())
        };
        let label = |title: &str, n: Option<usize>| {
            n.map(|n| format!("{title} · {n}")).unwrap_or(title.into())
        };
        let lab_count = self.studio_lab.artifacts.len();
        let outputs_count = self.studio_outputs_count();
        let saved_count = self.studio.works.len();
        let total_local = lab_count + outputs_count + saved_count;
        let sources = self.segmented(
            "st-sources",
            [
                (3u8, format!("全部作品 · {total_local}")),
                (5u8, format!("本机应用 · {lab_count}")),
                (4u8, format!("对话成果 · {outputs_count}")),
                (2u8, format!("我的收藏 · {saved_count}")),
                (1u8, label("手机可用", count(false))),
                (0u8, label("官网项目", count(true))),
            ]
            .into_iter()
            .map(|(id, text)| (text, source == id, Action::Studio(St::Source(id))))
            .collect(),
        );
        let right_action = if matches!(source, 3 | 5) {
            Empty::new().finish()
        } else if source == 4 {
            self.st_button(
                "st-all-new",
                "去 Amoo 做新作品",
                St::NewInAmoo,
                Btn::Secondary,
            )
        } else {
            self.st_button("st-add-link", "添加链接", St::New, Btn::Ghost)
        };
        let mut content = vec![self.between(self.wrap(vec![sources]), right_action)];
        let query = self.fields[F_STUDIO_SEARCH].text.trim().to_lowercase();
        let search_input = self.input(F_STUDIO_SEARCH, "搜索作品名称、应用描述或对话标题…", None);
        if matches!(source, 0..=2) {
            content.push(
                self.between(
                    search_input,
                    self.segmented(
                        "st-types",
                        [(3, "全部"), (0, "应用"), (1, "智能体"), (2, "技能")]
                            .iter()
                            .map(|(i, t)| {
                                (
                                    (*t).into(),
                                    self.studio.cloud_category == *i,
                                    Action::Studio(St::CloudCategory(*i)),
                                )
                            })
                            .collect(),
                    ),
                ),
            );
        } else {
            content.push(search_input);
            content.push(self.studio_deliverable_filter_bar());
        }
        if self.studio.editing {
            content.push(self.studio_link_form());
        }
        match source {
            3 => {
                content.push(self.studio_unified_gallery(&query));
                return self.vstack(S4, content);
            }
            4 => {
                content.push(self.studio_all_outputs(&query));
                return self.vstack(S4, content);
            }
            5 => {
                content.push(self.studio_local_offline_works(&query));
                return self.vstack(S4, content);
            }
            2 => {
                content.push(self.studio_saved(&query));
                return self.vstack(S4, content);
            }
            _ => {}
        }
        let library = if source == 0 {
            self.studio.web.as_ref()
        } else {
            self.studio.cloud.as_ref()
        };
        let error = if source == 0 {
            self.studio.web_error.as_ref()
        } else {
            self.studio.cloud_error.as_ref()
        };
        if let Some(e) = error {
            content.push(self.panel(vec![
                self.heading("暂未同步成功"),
                self.text(e, BODY, p.text2),
                self.wrap(vec![self.st_button(
                    "st-retry",
                    "重新同步",
                    St::CloudRefresh,
                    Btn::Secondary,
                )]),
            ]));
        }
        let Some(library) = library.filter(|c| c.authorized) else {
            content.push(self.panel(vec![
                self.ui_icon(if source==0{UiIcon::Monitor}else{UiIcon::Smartphone},36.,p.text2),
                self.heading(if source==0{"把官网项目带到工作台"}else{"连接 Amoo 的手机作品"}),
                self.text(if source==0{"在手机伴侣中完成一次 Studio 官网登录，应用、技能与智能体项目会自动出现在这里，包含尚未在手机上试用的草稿。"}else{"使用手机已有的 Amoo 账号授权，自动读取可在手机打开的作品。"},BODY,p.text2),
                self.wrap(vec![
                    if self.selected.is_some(){self.st_button("st-connect-source",if source==0{"连接 Studio 官网"}else{"连接手机账号"},if source==0{St::WebsiteLogin}else{St::ConnectAccount},Btn::Primary)}else{self.button("st-device","连接手机",Action::Page(super::super::Page::Connect),Btn::Primary)},
                    self.st_button("st-back-all-works", "先看本机与对话作品", St::Source(3), Btn::Secondary),
                ]),
                self.text(if source==0{"官网仍要求首次身份验证；连接 ADB 不代表已登录官网。登录会话保留在手机伴侣中。"}else{"官网创作 → 在手机上试用 → 自动同步"},CAPTION,p.text3),
            ]));
            return self.vstack(S4, content);
        };
        let mut filtered: Vec<_> = library
            .items
            .iter()
            .enumerate()
            .filter(|(_, w)| {
                self.studio_matches(
                    &w.category,
                    &format!("{} {}", w.name, w.description),
                    &query,
                )
            })
            .collect();
        filtered.sort_by(|a, b| b.1.updated.cmp(&a.1.updated).then(a.1.name.cmp(&b.1.name)));
        let current = if source == 0 {
            self.studio.web_selected
        } else {
            self.studio.cloud_selected
        };
        let page = self
            .studio
            .list_page
            .min(filtered.len().saturating_sub(1) / 12);
        let selected = filtered
            .iter()
            .skip(page * 12)
            .take(12)
            .find(|(i, _)| Some(*i) == current)
            .copied()
            .or_else(|| filtered.get(page * 12).copied());
        let mut list = vec![];
        for (i, w) in filtered.iter().skip(page * 12).take(12) {
            let editor_url = w
                .editor()
                .or_else(|| (!w.url.is_empty()).then(|| w.url.clone()))
                .unwrap_or_default();
            let menu_url = w
                .experience()
                .map(|e| e.url)
                .filter(|u| !u.is_empty())
                .unwrap_or_else(|| editor_url.clone());
            list.push(
                self.list_row_with_menu(
                    format!("st-item-{source}-{i}"),
                    self.between(
                        self.lead(
                            S3,
                            self.ui_icon(Self::studio_icon(&w.category), 20., p.text2),
                            Expanded::new(
                                1.,
                                self.vstack(
                                    S1,
                                    vec![
                                        self.heading(&w.name),
                                        self.line(
                                            format!(
                                                "{} · {}",
                                                w.category,
                                                if source == 0 {
                                                    "官网项目"
                                                } else if w.experience().is_some() {
                                                    "可上机体验"
                                                } else {
                                                    "在 Amoo 管理"
                                                }
                                            ),
                                            MICRO,
                                            p.text3,
                                        ),
                                    ],
                                ),
                            )
                            .finish(),
                        ),
                        self.ui_icon(UiIcon::ChevronRight, 14., p.text3),
                    ),
                    selected.is_some_and(|(j, _)| j == *i),
                    Some(Action::Studio(if source == 0 {
                        St::WebSelect(*i)
                    } else {
                        St::CloudSelect(*i)
                    })),
                    if source == 0 && !editor_url.is_empty() {
                        Some(Action::StudioOpenUrl(editor_url.clone()))
                    } else {
                        Some(Action::Studio(St::UsePhone(*i)))
                    },
                    Some(crate::ui::context_menu::Target::StudioWork(
                        crate::ui::context_menu::StudioWorkTarget::CloudProject {
                            source,
                            idx: *i,
                            url: menu_url,
                            title: w.name.clone(),
                        },
                    )),
                ),
            );
        }
        if list.is_empty() {
            content.push(self.studio_empty(
                if query.is_empty() {
                    "此分类还没有作品"
                } else {
                    "没有匹配的作品"
                },
                if query.is_empty() {
                    "从“新建作品”开始创作；官网完成后，可在手机上试用。"
                } else {
                    "试试其他名称，或切换到“全部”。"
                },
            ));
        } else {
            let (selected_index, selected_work) = selected.unwrap();
            let details = self.studio_sub_enter(self.studio_cloud_detail(
                selected_work,
                source,
                selected_index,
            ));
            let mut left = vec![self.list(list)];
            if filtered.len() > 12 {
                left.push(self.between(
                    self.st_button(
                        "st-prev",
                        "上一页",
                        St::ListPage(page.saturating_sub(1)),
                        Btn::Ghost,
                    ),
                    self.hstack(
                        S2,
                        vec![
                            self.line(
                                format!("{} / {}", page + 1, filtered.len().div_ceil(12)),
                                MICRO,
                                p.text3,
                            ),
                            self.st_button(
                                "st-next",
                                "下一页",
                                St::ListPage((page + 1).min(filtered.len().saturating_sub(1) / 12)),
                                Btn::Ghost,
                            ),
                        ],
                    ),
                ));
            }
            content.push(self.studio_columns(self.vstack(S3, left), details));
        }
        let time = chrono::DateTime::from_timestamp(library.synced_at, 0)
            .map(|t| t.with_timezone(&chrono::Local).format("%H:%M").to_string())
            .unwrap_or_default();
        content.push(self.line(
            format!(
                "{} 个作品  ·  {} 更新  ·  当前页面自动同步",
                filtered.len(),
                time
            ),
            MICRO,
            p.text3,
        ));
        self.vstack(S4, content)
    }
    fn studio_unified_gallery(&self, query: &str) -> El {
        let p = self.p();
        let mut sections = vec![];
        let show_apps = matches!(self.studio.deliverable_filter, 0 | 4);
        let mut matching_lab: Vec<_> = if show_apps {
            self.studio_lab
                .artifacts
                .iter()
                .enumerate()
                .filter(|(_, a)| {
                    query.is_empty()
                        || a.name.to_lowercase().contains(query)
                        || a.summary.to_lowercase().contains(query)
                })
                .collect()
        } else {
            Vec::new()
        };
        match self.studio.sort_mode {
            1 => matching_lab.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase())),
            2 => matching_lab.sort_by(|a, b| {
                b.1.passed
                    .cmp(&a.1.passed)
                    .then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()))
            }),
            _ => matching_lab.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at)),
        }
        if !matching_lab.is_empty() {
            let mut lab_rows = vec![self.between(
                self.hstack(
                    S2,
                    vec![
                        self.heading("本机离线应用"),
                        self.pill_badge(
                            format!("{} 款可玩", matching_lab.len()),
                            p.accent,
                            p.surface,
                        ),
                    ],
                ),
                if matching_lab.len() > 6 {
                    self.st_button(
                        "st-view-all-lab",
                        "查看全部本机应用",
                        St::Source(5),
                        Btn::Ghost,
                    )
                } else {
                    self.st_button("st-go-lab", "进入高级创作工坊", St::Tab(2), Btn::Ghost)
                },
            )];
            for (i, a) in matching_lab.iter().take(6) {
                lab_rows.push(self.studio_local_artifact_card(*i, a));
            }
            sections.push(self.vstack(S3, lab_rows));
        }
        if self.studio_outputs_count() > 0 {
            sections.push(self.studio_all_outputs(query));
        }
        let matching_saved: Vec<_> = self
            .studio
            .works
            .iter()
            .filter(|w| {
                query.is_empty()
                    || w.name.to_lowercase().contains(query)
                    || w.category.to_lowercase().contains(query)
            })
            .collect();
        if !matching_saved.is_empty() {
            sections.push(self.vstack(
                S3,
                vec![
                    self.between(
                        self.heading("我的收藏"),
                        self.st_button("st-view-saved", "管理收藏", St::Source(2), Btn::Ghost),
                    ),
                    self.studio_saved(query),
                ],
            ));
        }
        if sections.is_empty() {
            sections.push(self.studio_empty(
                "没有找到匹配的作品",
                "试试搜索其他关键词，或点击右上角“新建作品”在 Amoo 中直接说出你的想法。",
            ));
        }
        let web_ok = self.studio.web.as_ref().is_some_and(|c| c.authorized);
        let phone_ok = self.studio.cloud.as_ref().is_some_and(|c| c.authorized);
        if !web_ok || !phone_ok {
            sections.push(self.panel(vec![
                self.between(
                    self.lead(
                        S3,
                        self.studio_icon_box(UiIcon::Smartphone, 36., p.accent, p.accent_soft),
                        Expanded::new(
                            1.,
                            self.vstack(
                                2.,
                                vec![
                                    self.heading("连接手机与 Studio 官网，同步云端作品库"),
                                    self.text(
                                        "连接后可在此直接打开 Studio 官网草稿、同步手机端已安装作品与技能。",
                                        CAPTION,
                                        p.text2,
                                    ),
                                ],
                            ),
                        )
                        .finish(),
                    ),
                    self.wrap(vec![
                        if !phone_ok {
                            self.st_button(
                                "st-footer-phone",
                                "手机可用作品",
                                St::Source(1),
                                Btn::Secondary,
                            )
                        } else {
                            Empty::new().finish()
                        },
                        if !web_ok {
                            self.st_button(
                                "st-footer-web",
                                "Studio 官网项目",
                                St::Source(0),
                                Btn::Ghost,
                            )
                        } else {
                            Empty::new().finish()
                        },
                    ]),
                ),
            ]));
        }
        self.vstack(S5, sections)
    }
    fn studio_matches(&self, category: &str, text: &str, query: &str) -> bool {
        (self.studio.cloud_category == 3 || category == CATEGORIES[self.studio.cloud_category])
            && (query.is_empty() || format!("{category} {text}").to_lowercase().contains(query))
    }
    fn studio_icon(category: &str) -> UiIcon {
        match category {
            "技能" => UiIcon::Bolt,
            "智能体" => UiIcon::Star,
            _ => UiIcon::Apps,
        }
    }
    fn studio_columns(&self, list: El, details: El) -> El {
        Flex::row()
            .with_spacing(S5)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_child(Expanded::new(1., list).finish())
            .with_child(Expanded::new(1.25, details).finish())
            .finish()
    }
    fn studio_empty(&self, title: &str, description: &str) -> El {
        self.panel(vec![
            self.ui_icon(UiIcon::Folder, 28., self.p().text3),
            self.heading(title),
            self.text(description, BODY, self.p().text2),
        ])
    }
    fn studio_cloud_detail(&self, w: &CloudWork, source: u8, index: usize) -> El {
        let p = self.p();
        let website = source == 0;
        let title = self.between(
            self.lead(
                S3,
                self.ui_icon(Self::studio_icon(&w.category), 32., p.text),
                Expanded::new(
                    1.,
                    self.vstack(
                        S1,
                        vec![
                            self.heading(&w.name),
                            self.line(&w.category, CAPTION, p.text3),
                        ],
                    ),
                )
                .finish(),
            ),
            self.pill_badge(
                if website {
                    "官网项目"
                } else if w.experience().is_some() {
                    "可在手机打开"
                } else {
                    "手机作品"
                },
                p.text2,
                p.surface,
            ),
        );
        let mut children = vec![
            title,
            self.text(
                if w.description.is_empty() {
                    "打开作品，继续你的创作。"
                } else {
                    &w.description
                },
                BODY,
                p.text2,
            ),
            self.divider(),
        ];
        if website {
            if let Some(url) = w.editor() {
                children.push(self.wrap(vec![self.button_icon(
                    "st-edit-project",
                    "继续编辑",
                    UiIcon::ExternalLink,
                    Action::StudioOpenUrl(url),
                    Btn::Primary,
                )]));
            }
            children.push(self.text(
                "直接在独立工作窗口打开对应项目，自动复用伴侣内的官网登录。",
                CAPTION,
                p.text3,
            ));
            let phone = self.studio.cloud.as_ref().and_then(|c| {
                c.items
                    .iter()
                    .enumerate()
                    .find(|(_, p)| p.id == w.id && p.category == w.category)
            });
            if let Some((i, item)) = phone {
                children.push(self.heading("在手机上试用"));
                children.push(self.text("已在 Amoo 我的作品找到此项目。", BODY, p.text2));
                children.push(self.wrap(vec![self.st_button(
                    "st-switch-phone",
                    "查看手机版本",
                    St::PhoneItem(i),
                    Btn::Secondary,
                )]));
                if item.experience().is_none() {
                    children.push(self.caption("技能与智能体需要在 Amoo 中确认安装。"));
                }
            } else {
                children.push(self.heading("下一步 · 在手机上试用"));
                children.push(self.text("在官网点击“在手机上试用”后可同步到手机打开；未生成体验链接的草稿不会作为公开作品分享。",BODY,p.text2));
            }
            children.push(self.wrap(vec![
                    self.st_button(
                        "st-keep-project",
                        "收藏项目入口",
                        St::KeepWeb(index),
                        Btn::Ghost,
                    ),
                    if let Some(url) = w
                        .editor()
                        .or_else(|| (!w.url.is_empty()).then(|| w.url.clone()))
                    {
                        self.button(
                            format!("st-copy-web-url-{index}"),
                            "复制项目链接",
                            Action::CopyText(url, "已复制官网项目链接".into()),
                            Btn::Secondary,
                        )
                    } else {
                        Empty::new().finish()
                    },
                ]));
        } else {
            children.push(self.wrap(vec![self.st_button(
                "st-use-phone",
                if w.experience().is_some() {
                    "在手机打开"
                } else {
                    "在 Amoo 管理"
                },
                St::UsePhone(index),
                Btn::Primary,
            )]));
            let mut actions = vec![];
            if w.experience().is_some() {
                actions.push(self.st_button(
                    "st-share-phone",
                    "分享作品",
                    St::KeepPhone(index),
                    Btn::Secondary,
                ));
            }
            if let Some(url) = w.editor() {
                actions.push(self.button_icon(
                    "st-edit-phone",
                    "继续编辑",
                    UiIcon::ExternalLink,
                    Action::StudioOpenUrl(url),
                    Btn::Secondary,
                ));
            }
            if let Some(url) = w
                .experience()
                .map(|e| e.url)
                .or_else(|| w.editor())
                .or_else(|| (!w.url.is_empty()).then(|| w.url.clone()))
            {
                actions.push(self.button(
                    format!("st-copy-cloud-url-{index}"),
                    "复制体验链接",
                    Action::CopyText(url, "已复制作品体验链接".into()),
                    Btn::Secondary,
                ));
            }
            if !actions.is_empty() {
                children.push(self.wrap(actions));
            }
            if !w.version.is_empty() {
                children.push(self.line(format!("版本 {}", w.version), CAPTION, p.text3));
            }
            children.push(self.text(
                if w.experience().is_some() {
                    "打开后会保留手机入口；官方体验作品是否需要联网，由作品本身决定。"
                } else {
                    "安装和更新由 Amoo 官方页面完成，请在手机确认权限。"
                },
                CAPTION,
                p.text3,
            ));
        }
        if let Some(note) = &self.studio.phone_note {
            children.push(self.caption(note));
        }
        self.panel(children)
    }
    fn studio_link_form(&self) -> El {
        self.panel(vec![
            self.between(
                self.heading("添加作品链接"),
                self.st_button("st-cancel-link", "收起", St::Edit(false), Btn::Ghost),
            ),
            self.input(F_STUDIO_NAME, "作品名称", None),
            self.input(F_STUDIO_URL, "官方项目或体验链接", None),
            self.between(
                self.segmented(
                    "st-link-kind",
                    CATEGORIES
                        .iter()
                        .enumerate()
                        .map(|(i, c)| {
                            (
                                (*c).into(),
                                self.studio.category == i,
                                Action::Studio(St::Category(i)),
                            )
                        })
                        .collect(),
                ),
                self.wrap(vec![
                    self.st_button("st-paste", "粘贴链接", St::PasteLink, Btn::Ghost),
                    self.st_button("st-save", "保存", St::Save, Btn::Primary),
                ]),
            ),
        ])
    }
    fn studio_saved(&self, query: &str) -> El {
        let p = self.p();
        let filtered: Vec<_> = self
            .studio
            .works
            .iter()
            .enumerate()
            .filter(|(_, w)| self.studio_matches(&w.category, &w.name, query))
            .collect();
        if filtered.is_empty() {
            return self.studio_empty(
                "还没有收藏的作品",
                "在官网项目中收藏编辑入口，或从手机作品中选择分享；也可以添加官方体验链接。",
            );
        }
        let page = self
            .studio
            .list_page
            .min(filtered.len().saturating_sub(1) / 12);
        let selected = filtered
            .iter()
            .skip(page * 12)
            .take(12)
            .find(|(i, _)| Some(*i) == self.studio.selected)
            .copied()
            .or_else(|| filtered.get(page * 12).copied());
        let rows = filtered
            .iter()
            .skip(page * 12)
            .take(12)
            .map(|(i, w)| {
                self.list_row_with_menu(
                    format!("st-saved-{i}"),
                    self.between(
                        self.lead(
                            S3,
                            self.ui_icon(Self::studio_icon(&w.category), 18., p.accent),
                            Expanded::new(
                                1.,
                                self.vstack(
                                    S1,
                                    vec![
                                        self.heading(&w.name),
                                        self.line(
                                            format!(
                                                "{} · {}",
                                                w.category,
                                                if w.shareable() {
                                                    "体验链接"
                                                } else {
                                                    "项目入口"
                                                }
                                            ),
                                            MICRO,
                                            p.text3,
                                        ),
                                    ],
                                ),
                            )
                            .finish(),
                        ),
                        self.ui_icon(UiIcon::ChevronRight, 14., p.text3),
                    ),
                    selected.is_some_and(|(j, _)| j == *i),
                    Some(Action::Studio(St::Select(*i))),
                    Some(if !w.url.is_empty() {
                        Action::StudioOpenUrl(w.url.clone())
                    } else {
                        Action::Studio(St::UseSavedOnPhone(*i))
                    }),
                    Some(crate::ui::context_menu::Target::StudioWork(
                        crate::ui::context_menu::StudioWorkTarget::SavedWork {
                            idx: *i,
                            url: w.url.clone(),
                            title: w.name.clone(),
                        },
                    )),
                )
            })
            .collect();
        let mut left = vec![self.list(rows)];
        if filtered.len() > 12 {
            left.push(self.wrap(vec![
                self.st_button(
                    "st-saved-prev",
                    "上一页",
                    St::ListPage(page.saturating_sub(1)),
                    Btn::Ghost,
                ),
                self.st_button(
                    "st-saved-next",
                    "下一页",
                    St::ListPage((page + 1).min(filtered.len().saturating_sub(1) / 12)),
                    Btn::Ghost,
                ),
            ]));
        }
        self.studio_columns(
            self.vstack(S3, left),
            selected
                .map(|(selected_idx, w)| {
                    self.studio_sub_enter(self.studio_saved_detail(selected_idx, w))
                })
                .unwrap_or_else(|| {
                    self.studio_empty("选择一个作品", "打开、发送到手机，或生成分享二维码。")
                }),
        )
    }
    fn studio_saved_detail(&self, selected_idx: usize, w: &Work) -> El {
        let p = self.p();
        let mut content = vec![
            self.heading(&w.name),
            self.line(
                format!(
                    "{} · {}",
                    w.category,
                    if w.shareable() {
                        "体验链接"
                    } else {
                        "项目编辑页"
                    }
                ),
                CAPTION,
                p.text3,
            ),
        ];
        if w.shareable() {
            content.push(self.wrap(vec![self.st_button(
                "st-open-saved",
                "在手机打开",
                St::OpenPhone,
                Btn::Primary,
            )]));
            content.push(self.wrap(vec![
                self.st_button(
                    "st-send-phone",
                    "同步到手机作品库",
                    St::Send,
                    Btn::Secondary,
                ),
                self.st_button("st-qr", "分享二维码", St::Share, Btn::Secondary),
                self.st_button("st-pin", "添加手机桌面图标", St::Pin, Btn::Secondary),
            ]));
        } else {
            content.push(self.wrap(vec![self.button_icon(
                "st-open-editor",
                "继续编辑",
                UiIcon::ExternalLink,
                Action::StudioOpenUrl(w.url.clone()),
                Btn::Primary,
            )]));
            content.push(self.caption("项目入口需要官网账号，不能当作公开体验链接分享。"));
        }
        content.push(self.wrap(vec![
            self.button(
                "st-copy-url",
                "复制作品链接",
                Action::CopyText(w.url.clone(), "已复制收藏作品链接".into()),
                Btn::Secondary,
            ),
            self.st_button("st-edit-link", "编辑收藏", St::Edit(true), Btn::Ghost),
            self.st_button(
                "st-delete-saved",
                "移除收藏",
                St::DeleteSaved(selected_idx),
                Btn::Ghost,
            ),
        ]));
        if let Some((id, path)) = &self.studio.qr {
            if id == &w.id {
                let qr = ConstrainedBox::new(
                    Image::new(
                        AssetSource::LocalFile {
                            path: path.to_string_lossy().into_owned(),
                        },
                        CacheOption::BySize,
                    )
                    .contain()
                    .finish(),
                )
                .with_width(180.)
                .with_height(180.)
                .finish();
                content.extend(vec![
                    self.divider(),
                    qr,
                    self.caption("扫码体验；链接有效期与访问权限由官网决定。"),
                    self.wrap(vec![
                        self.st_button("st-save-qr", "保存二维码", St::SaveQr, Btn::Secondary),
                        self.button(
                            "st-copy-share",
                            "复制分享文案",
                            Action::CopyText(
                                format!("试试我用 Step Studio 制作的「{}」\n{}", w.name, w.url),
                                "已复制分享文案".into(),
                            ),
                            Btn::Secondary,
                        ),
                    ]),
                ]);
            }
        }
        if let Some(note) = &self.studio.phone_note {
            content.push(self.caption(note));
        }
        self.panel(content)
    }
    fn studio_icon_box(
        &self,
        icon: UiIcon,
        size: f32,
        fg: facloud_ui::color::ColorU,
        bg: facloud_ui::color::ColorU,
    ) -> El {
        let p = self.p();
        Container::new(
            ConstrainedBox::new(Align::new(self.ui_icon(icon, (size * 0.48).round(), fg)).finish())
                .with_width(size)
                .with_height(size)
                .finish(),
        )
        .with_background_color(bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish()
    }
    fn studio_local_artifact_card(
        &self,
        i: usize,
        a: &stepx_companion::studio_lab::LabArtifact,
    ) -> El {
        use super::super::studio_lab_state::StudioLabAction;
        let p = self.p();
        let has_html = cached_is_file(&a.html_path);
        let thumb = if let Some(path) = a.screenshot_path.as_ref().filter(|s| cached_is_file(s)) {
            Container::new(
                ConstrainedBox::new(
                    Image::new(
                        AssetSource::LocalFile {
                            path: path.to_string_lossy().into_owned(),
                        },
                        CacheOption::BySize,
                    )
                    .contain()
                    .finish(),
                )
                .with_width(116.)
                .with_height(74.)
                .finish(),
            )
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
        } else {
            self.studio_icon_box(UiIcon::Apps, 48., p.accent, p.accent_soft)
        };
        self.hover_panel_interactive(
            format!("st-offline-card-{i}"),
            self.vstack(
                S3,
                vec![
                    self.between(
                        self.lead(
                            S3,
                            thumb,
                            Expanded::new(
                                1.,
                                self.vstack(
                                    S1,
                                    vec![
                                        self.heading(&a.name),
                                        self.line(
                                            format!("版本 v{} · {}", a.round, a.summary),
                                            CAPTION,
                                            p.text2,
                                        ),
                                    ],
                                ),
                            )
                            .finish(),
                        ),
                        if a.passed {
                            self.pill_badge_icon("可直接运行", UiIcon::Check, p.success, p.surface)
                        } else {
                            self.pill_badge("离线可玩", p.accent, p.surface)
                        },
                    ),
                    self.wrap(vec![
                        self.button_icon(
                            format!("st-offline-preview-{i}"),
                            "立即试玩",
                            UiIcon::Play,
                            Action::StudioLab(StudioLabAction::PreviewAt(i)),
                            Btn::Primary,
                        ),
                        self.st_button(
                            &format!("st-offline-amoo-{i}"),
                            "在 Amoo 中继续改",
                            St::ContinueLabInAmoo(i),
                            Btn::Secondary,
                        ),
                        if self.selected.is_some() && has_html {
                            self.button_icon(
                                format!("st-offline-phone-{i}"),
                                "发送到手机",
                                UiIcon::Smartphone,
                                Action::StudioLab(StudioLabAction::DeliverPhoneAt(i)),
                                Btn::Secondary,
                            )
                        } else {
                            self.disabled_button("连接手机后可发送")
                        },
                        self.st_button(
                            &format!("st-offline-folder-{i}"),
                            "打开文件夹",
                            St::RevealPath(a.html_path.clone()),
                            Btn::Ghost,
                        ),
                        self.button_icon(
                            format!("st-offline-open-lab-{i}"),
                            "高级工坊",
                            UiIcon::Tools,
                            Action::StudioLab(StudioLabAction::OpenInWorkbench(i)),
                            Btn::Ghost,
                        ),
                    ]),
                ],
            ),
            Some(Action::StudioLab(StudioLabAction::Select(i))),
            Some(Action::StudioLab(StudioLabAction::PreviewAt(i))),
            Some(crate::ui::context_menu::Target::StudioWork(
                crate::ui::context_menu::StudioWorkTarget::LocalArtifact {
                    round_idx: i,
                    path: a.html_path.clone(),
                    title: a.name.clone(),
                },
            )),
        )
    }
    fn studio_local_offline_works(&self, query: &str) -> El {
        let show_apps = matches!(self.studio.deliverable_filter, 0 | 4);
        let mut filtered: Vec<_> = if show_apps {
            self.studio_lab
                .artifacts
                .iter()
                .enumerate()
                .filter(|(_, a)| {
                    query.is_empty()
                        || a.name.to_lowercase().contains(query)
                        || a.summary.to_lowercase().contains(query)
                })
                .collect()
        } else {
            Vec::new()
        };
        match self.studio.sort_mode {
            1 => filtered.sort_by(|a, b| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase())),
            2 => filtered.sort_by(|a, b| {
                b.1.passed
                    .cmp(&a.1.passed)
                    .then_with(|| a.1.name.to_lowercase().cmp(&b.1.name.to_lowercase()))
            }),
            _ => filtered.sort_by(|a, b| b.1.created_at.cmp(&a.1.created_at)),
        }
        if filtered.is_empty() {
            return self.studio_empty(
                "暂无匹配的本机离线应用",
                "在「高级创作工坊」或 Amoo 对话中输入想法即可生成可直接运行的离线 HTML 应用。",
            );
        }
        let mut rows = vec![];
        for (i, a) in filtered.iter().take(16) {
            rows.push(self.studio_local_artifact_card(*i, a));
        }
        self.vstack(S3, rows)
    }
    fn studio_connections(&self) -> El {
        let p = self.p();
        let web = self.studio.web.as_ref().filter(|c| c.authorized);
        let phone = self.studio.cloud.as_ref().is_some_and(|c| c.authorized);
        self.vstack(S4,vec![
            self.studio_lab_settings(),
            self.panel(vec![self.between(self.heading("Studio 官网"),self.pill_badge(if web.is_some(){"已连接"}else{"未登录"},p.text2,p.surface)),self.text(web.map(|c|format!("{} · {} 个官网项目",c.name,c.items.len())).unwrap_or("同步应用、技能与智能体项目，包含官网草稿。首次需在手机完成官网验证。".into()),BODY,p.text2),self.wrap(vec![self.st_button("st-web-login",if web.is_some(){"管理官网登录"}else{"连接官网"},St::WebsiteLogin,Btn::Primary),self.st_button("st-web-off","断开官网",St::WebsiteDisconnect,Btn::Ghost)]),self.caption("点击继续编辑会连接独立的桌面工作窗口，复用伴侣内的官网登录，无需再次登录；不会读取系统浏览器账号。")]),
            self.panel(vec![self.between(self.heading("Amoo 手机作品"),self.pill_badge(if phone{"已授权"}else{"未连接"},p.text2,p.surface)),self.text("读取可在手机打开的作品。应用可直接在手机上试用，技能与智能体可前往 Amoo 确认安装。",BODY,p.text2),self.wrap(vec![self.st_button("st-native-login","连接手机账号",St::ConnectAccount,Btn::Secondary),self.st_button("st-native-off","断开作品同步",St::DisconnectAccount,Btn::Ghost)])]),
            self.panel(vec![self.heading("手机与分享"),self.text("作品发送到手机后可独立打开、分享和添加桌面图标。分享只使用官方体验地址；编辑入口和登录信息不会进入二维码。",BODY,p.text2),self.wrap(vec![self.st_button("st-phone-library","手机作品库",St::PhoneLibrary,Btn::Secondary),self.st_button("st-amoo-library","Amoo 我的作品",St::AmooWorks,Btn::Secondary),self.st_button("st-phone-check","检查连接",St::CheckPhone,Btn::Ghost)])]),
            self.panel(vec![self.heading("Studio 构建器创作模板"),self.text("一键复制面向手机端体验优化的应用、智能体与技能创作提示词简报。",BODY,p.text2),self.wrap((0..3).map(|i| self.st_button(&format!("st-brief-{i}"), match i { 0 => "随手记账简报", 1 => "差旅助手简报", _ => "票据整理简报" }, St::Brief(i), Btn::Ghost)).collect())]),
        ])
    }
}
