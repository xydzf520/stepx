use super::super::{
    icons::UiIcon,
    studio_lab_state::{StudioLabAction as Lab, LAB_TEMPLATES},
    studio_state::StudioAction,
    theme::*,
    widgets::*,
    Action, Companion, F_LAB_ENDPOINT, F_LAB_KEY, F_LAB_MODEL, F_LAB_PROMPT,
};
use facloud_ui::{assets::asset_cache::AssetSource, elements::*, fonts::Weight, platform::Cursor};

impl Companion {
    fn lab_button(&self, id: &str, title: &str, action: Lab, kind: Btn) -> El {
        let (parsed_icon, clean_title) = super::super::icons::label_icon(title)
            .map(|(ic, r)| (Some(ic), r))
            .unwrap_or((None, title));
        let icon = parsed_icon.or(match &action {
            Lab::Start => Some(UiIcon::Play),
            Lab::Stop => Some(UiIcon::Close),
            Lab::Fresh => Some(UiIcon::Plus),
            Lab::Refresh | Lab::Models | Lab::CloudModels => Some(UiIcon::Refresh),
            Lab::Preview | Lab::PreviewAt(_) => Some(UiIcon::Play),
            Lab::OpenFolder => Some(UiIcon::Folder),
            Lab::DeliverPhone | Lab::DeliverPhoneAt(_) => Some(UiIcon::Smartphone),
            Lab::OpenInWorkbench(_) => Some(UiIcon::Tools),
            Lab::ImportFromWork => Some(UiIcon::Download),
            Lab::SyncCloud => Some(UiIcon::Upload),
            Lab::OpenCloud => Some(UiIcon::ExternalLink),
            Lab::Iterate => Some(UiIcon::Retry),
            Lab::Template(0) => Some(UiIcon::Play),
            Lab::Template(1) => Some(UiIcon::Apps),
            Lab::Template(2) => Some(UiIcon::Bolt),
            Lab::Template(3) => Some(UiIcon::Star),
            Lab::Template(4) => Some(UiIcon::Clock),
            Lab::Template(5) => Some(UiIcon::Chart),
            Lab::Template(6) => Some(UiIcon::Check),
            Lab::Template(7) => Some(UiIcon::Matrix),
            _ => None,
        });
        if !self.studio_lab_action_allowed(&action) {
            self.disabled_button(title)
        } else if let Some(icon) = icon {
            self.button_icon(
                format!("studio-lab-{id}"),
                clean_title,
                icon,
                Action::StudioLab(action),
                kind,
            )
        } else {
            self.button(
                format!("studio-lab-{id}"),
                clean_title,
                Action::StudioLab(action),
                kind,
            )
        }
    }
    fn lab_prompt(&self) -> El {
        let p = self.p();
        let field = &self.fields[F_LAB_PROMPT];
        let focused = self.focus == Some(F_LAB_PROMPT);
        let mut value = field.text.clone();
        let mut caret = 0;
        if focused {
            let cursor = field.cursor.min(value.len());
            caret = cursor + field.preedit.len();
            value.insert_str(cursor, &format!("{}│", field.preedit));
        }
        let empty = field.text.is_empty() && field.preedit.is_empty();
        if value.is_empty() {
            value = if self.studio_lab.source.is_some() {
                "描述要继续改进的交互、功能或视觉效果…"
            } else {
                "想做什么小游戏或小工具？写下玩法和要点，或点上方的示例需求…"
            }
            .into();
        }
        Hoverable::new(self.mouse_state("studio-lab-prompt"), move |_| {
            ConstrainedBox::new(
                Container::new(
                    self.pane_scroll(
                        self.studio_lab.prompt_scroll.clone(),
                        Text::new(value, self.font, BODY)
                            .with_saved_char_position(caret, format!("input-caret:{F_LAB_PROMPT}"))
                            .with_color(if empty && !focused { p.text3 } else { p.text })
                            .with_line_height_ratio(1.4)
                            .finish(),
                    ),
                )
                .with_uniform_padding(S3)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(if focused {
                    p.accent
                } else {
                    p.border
                }))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            )
            .with_height(100.)
            .finish()
        })
        .with_cursor(Cursor::IBeam)
        .on_mouse_down(|ctx, _, _| ctx.dispatch_typed_action(Action::Focus(F_LAB_PROMPT)))
        .on_right_click(|ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::Input(F_LAB_PROMPT),
                at,
            ))
        })
        .finish()
    }
    pub(in super::super) fn studio_lab_page(&self) -> El {
        let p = self.p();
        let running = self.studio_lab.handle.is_some();
        let configured = self.studio_lab.config.has_key && !self.studio_lab.config.model.is_empty();
        let phone_ready = self.selected.is_some();
        // 不再重复「对话 / 交付 / 工坊」三选一切换栏：做应用平时直接在 Amoo 里说，
        // 这里只是用自己的模型反复调整的高级入口。
        let workbench_bar = self.panel(vec![self.between(
            self.hstack(
                S2,
                vec![
                    self.button_icon(
                        "lab-back-amoo",
                        "返回 Amoo",
                        UiIcon::ArrowLeft,
                        Action::Page(super::super::Page::Amoo),
                        Btn::Ghost,
                    ),
                    self.line("平时直接在 Amoo 里说“做个小游戏”就行", CAPTION, p.text2),
                ],
            ),
            self.lab_button(
                "import-from-work",
                "用已有文件做应用",
                Lab::ImportFromWork,
                Btn::Secondary,
            ),
        )]);
        let mut left = vec![self.panel(vec![
            self.between(
                self.heading("高级：用自己的模型做应用"),
                if configured {
                    self.pill_badge(
                        format!("自有模型 · {}", self.studio_lab.config.model),
                        p.success,
                        p.surface,
                    )
                } else if phone_ready {
                    self.pill_badge("手机已连接", p.success, p.surface)
                } else {
                    self.pill_badge("内置 4 个示例", p.accent, p.surface)
                },
            ),
            self.text(
                "内置 4 个可以直接玩的示例。这里适合用手机上的 Amoo 或自己的模型反复调整，做好后会自动检查，也能发到手机。",
                BODY,
                p.text2,
            ),
            self.caption("示例需求（点一下填入）："),
            self.wrap(
                LAB_TEMPLATES
                    .iter()
                    .enumerate()
                    .map(|(i, (title, _))| {
                        self.lab_button(
                            &format!("template-{i}"),
                            title,
                            Lab::Template(i),
                            Btn::Secondary,
                        )
                    })
                    .collect(),
            ),
        ])];
        let has_prompt =
            !self.fields[F_LAB_PROMPT].text.trim().is_empty() || self.studio_lab.source.is_some();
        let mut composer = vec![self.between(
            self.heading(if self.studio_lab.source.is_some() {
                "继续改进"
            } else {
                "创作需求"
            }),
            if has_prompt {
                self.lab_button("fresh", "清空输入", Lab::Fresh, Btn::Ghost)
            } else {
                Empty::new().finish()
            },
        )];
        if let Some(name) = &self.studio_lab.source_name {
            composer.push(self.text(format!("基于：{name}"), CAPTION, p.text2));
        }
        composer.push(self.lab_prompt());
        composer.push(self.between(
            self.wrap(vec![self.button(
                "lab-config",
                "模型服务",
                Action::Studio(StudioAction::Tab(3)),
                Btn::Ghost,
            )]),
            if running {
                self.lab_button("stop", "停止", Lab::Stop, Btn::Secondary)
            } else if self.fields[F_LAB_PROMPT].text.trim().is_empty() {
                self.disabled_button("开始创作")
            } else {
                self.lab_button("start", "开始创作", Lab::Start, Btn::Primary)
            },
        ));
        composer.push(self.text(
            if !configured && phone_ready {
                "Enter 开始 · Shift+Enter 换行 · 出错会自动修复，最多 3 次 · 现在用手机上的 Amoo 来做"
            } else {
                "Enter 开始 · Shift+Enter 换行 · 出错会自动修复，最多 3 次"
            },
            MICRO,
            p.text3,
        ));
        left.push(self.panel(composer));
        if let Some(error) = &self.studio_lab.error {
            left.push(self.panel(vec![self.text(error, BODY, p.danger)]));
        }
        if running || !self.studio_lab.status.is_empty() {
            let mut progress = vec![
                self.between(
                    self.heading(if running {
                        "正在创作与验证"
                    } else {
                        "本次进展"
                    }),
                    self.pill_badge(
                        if running {
                            "运行中"
                        } else if self.studio_lab.error.is_some() {
                            "待处理"
                        } else {
                            "已结束"
                        },
                        if running { p.accent } else { p.text2 },
                        p.surface,
                    ),
                ),
                self.text(&self.studio_lab.status, BODY, p.text),
            ];
            for event in self
                .studio_lab
                .events
                .iter()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .into_iter()
                .rev()
            {
                progress.push(self.text(event, CAPTION, p.text3));
            }
            left.push(self.panel(progress));
        }
        let mut history = vec![self.between(
            self.heading("这台电脑上的作品"),
            self.lab_button("refresh", "刷新", Lab::Refresh, Btn::Ghost),
        )];
        if self.studio_lab.artifacts.is_empty() {
            history.push(self.text(
                "还没有作品。点上方的示例需求就能做出第一个。",
                CAPTION,
                p.text2,
            ));
            history.push(self.text(
                "每次做出的版本和检查结果都会保留在这里，关掉程序也不会丢。",
                MICRO,
                p.text3,
            ));
        }
        let page = self
            .studio_lab
            .history_page
            .min(self.studio_lab.artifacts.len().saturating_sub(1) / 6);
        for (i, a) in self
            .studio_lab
            .artifacts
            .iter()
            .enumerate()
            .skip(page * 6)
            .take(6)
        {
            history.push(self.list_row_with_menu(
                format!("lab-artifact-{i}"),
                self.between(
                    self.vstack(
                        S1,
                        vec![
                            self.line_w(&a.name, BODY, p.text, Weight::Medium),
                            self.line(format!("第 {} 轮 · {}", a.round, a.summary), MICRO, p.text3),
                        ],
                    ),
                    self.hstack(
                        S2,
                        vec![
                            self.pill_badge(
                                if a.passed {
                                    "检查通过"
                                } else {
                                    "可试玩"
                                },
                                if a.passed { p.success } else { p.text3 },
                                p.surface,
                            ),
                            self.lab_button(
                                &format!("round-play-{i}"),
                                "试玩",
                                Lab::PreviewAt(i),
                                Btn::Secondary,
                            ),
                            self.lab_button(
                                &format!("round-reuse-{i}"),
                                "复用提示词",
                                Lab::ReusePrompt(i),
                                Btn::Ghost,
                            ),
                        ],
                    ),
                ),
                self.studio_lab.selected == Some(i),
                Some(Action::StudioLab(Lab::Select(i))),
                Some(Action::StudioLab(Lab::PreviewAt(i))),
                Some(crate::ui::context_menu::Target::StudioWork(
                    crate::ui::context_menu::StudioWorkTarget::LocalArtifact {
                        round_idx: i,
                        path: a.html_path.clone(),
                        title: a.name.clone(),
                    },
                )),
            ));
        }
        let pages = self.studio_lab.artifacts.len().max(1).div_ceil(6);
        if pages > 1 {
            history.push(self.between(
                if page > 0 {
                    self.lab_button(
                        "history-prev",
                        "上一页",
                        Lab::HistoryPage(page - 1),
                        Btn::Ghost,
                    )
                } else {
                    self.disabled_button("上一页")
                },
                self.hstack(
                    S2,
                    vec![
                        self.caption(format!("{} / {}", page + 1, pages)),
                        if page + 1 < pages {
                            self.lab_button(
                                "history-next",
                                "下一页",
                                Lab::HistoryPage(page + 1),
                                Btn::Ghost,
                            )
                        } else {
                            self.disabled_button("下一页")
                        },
                    ],
                ),
            ));
        }
        left.push(self.panel(history));
        let left_col = self.pane_scroll(
            self.scroll.clone(),
            Container::new(self.vstack(S3, left))
                .with_padding_bottom(S5)
                .finish(),
        );
        let right_col = self.pane_scroll(
            self.scroll_aux.clone(),
            Container::new(self.sub_enter(self.studio_lab_preview()))
                .with_padding_bottom(S5)
                .finish(),
        );
        Expanded::new(
            1.,
            Container::new(
                self.sub_enter(
                    Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_spacing(S3)
                        .with_child(workbench_bar)
                        .with_child(
                            Expanded::new(
                                1.,
                                Flex::row()
                                    .with_main_axis_size(MainAxisSize::Max)
                                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                                    .with_spacing(S4)
                                    .with_child(Expanded::new(1.08, left_col).finish())
                                    .with_child(Expanded::new(1., right_col).finish())
                                    .finish(),
                            )
                            .finish(),
                        )
                        .finish(),
                ),
            )
            .with_horizontal_padding(S5)
            .with_padding_top(S2)
            .with_padding_bottom(S5)
            .finish(),
        )
        .finish()
    }
    fn studio_lab_preview(&self) -> El {
        let p = self.p();
        let Some(a) = self.studio_lab_selected() else {
            return self.panel(vec![
                self.ui_icon(UiIcon::Monitor, 36., p.text3),
                self.heading("成果与预览"),
                self.text(
                    "做好后，这里会显示截图、检查结果和发到手机的按钮。",
                    BODY,
                    p.text2,
                ),
                self.caption("生成 → 试运行 → 检查 → 修复 → 完成"),
            ]);
        };
        let has_html = cached_is_file(&a.html_path);
        let can_deliver_phone = has_html && self.selected.is_some();
        let mut rows = Vec::new();
        if !self.studio_lab.artifacts.is_empty() {
            rows.push(self.between(
                self.caption("切换作品："),
                self.pill_badge(
                    format!("共 {} 款", self.studio_lab.artifacts.len()),
                    p.accent,
                    p.surface,
                ),
            ));
            rows.push(
                self.wrap(
                    self.studio_lab
                        .artifacts
                        .iter()
                        .enumerate()
                        .take(6)
                        .map(|(i, item)| {
                            let is_selected = self.studio_lab.selected == Some(i)
                                || (self.studio_lab.selected.is_none() && i == 0);
                            self.lab_button(
                                &format!("switch-artifact-{i}"),
                                &item.name,
                                Lab::Select(i),
                                if is_selected {
                                    Btn::Primary
                                } else {
                                    Btn::Secondary
                                },
                            )
                        })
                        .collect(),
                ),
            );
            rows.push(self.divider());
        }
        rows.push(self.between(
            self.heading(&a.name),
            self.pill_badge(
                if a.passed {
                    "可交付"
                } else if has_html {
                    "可试玩"
                } else {
                    "待验证"
                },
                if a.passed { p.success } else { p.text3 },
                p.surface,
            ),
        ));
        if let Some(path) = &a.screenshot_path {
            rows.push(
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
                .with_max_width(320.)
                .with_height(180.)
                .finish(),
            );
        } else {
            rows.push(self.empty("预览尚未生成", "先查看验收结果并继续改进。", None));
        }
        let html_path = a.html_path.to_string_lossy().into_owned();
        let prompt_text = self.studio_lab_prompt_for(a);
        let selected_idx = self.studio_lab.selected.unwrap_or(0);
        let mut preview_actions = vec![
            if has_html {
                self.lab_button("preview", "打开预览", Lab::Preview, Btn::Primary)
            } else {
                self.disabled_button("打开预览")
            },
            self.button_icon(
                "lab-continue-in-amoo",
                "在 Amoo 中继续改",
                UiIcon::Chat,
                Action::Studio(StudioAction::ContinueLabInAmoo(selected_idx)),
                Btn::Secondary,
            ),
            if can_deliver_phone {
                self.lab_button("phone", "发到手机并打开", Lab::DeliverPhone, Btn::Secondary)
            } else {
                self.disabled_button(if self.selected.is_none() {
                    "请先连接手机"
                } else {
                    "作品文件还没准备好"
                })
            },
            self.button(
                "lab-preview-copy-path",
                "复制文件路径",
                Action::CopyText(html_path, "已复制文件路径".into()),
                Btn::Ghost,
            ),
        ];
        if let Some(path) = &a.screenshot_path {
            preview_actions.push(self.button(
                "lab-preview-zoom",
                "放大查看",
                Action::Work(super::super::work_state::WorkAction::OpenPreview(
                    path.to_string_lossy().into_owned(),
                )),
                Btn::Ghost,
            ));
        }
        preview_actions.extend([
            self.button(
                "lab-preview-copy-prompt",
                "复制需求",
                Action::CopyText(prompt_text, "已复制这一版的需求".into()),
                Btn::Ghost,
            ),
            self.lab_button("iterate", "继续改进", Lab::Iterate, Btn::Ghost),
            self.lab_button("folder", "作品目录", Lab::OpenFolder, Btn::Ghost),
        ]);
        rows.push(self.wrap(preview_actions));
        if let Some(note) = self.studio_lab_phone_note() {
            rows.push(match note {
                Ok(note) => self.text(note, CAPTION, p.success),
                Err(error) => self.text(error, CAPTION, p.danger),
            });
        }
        rows.push(self.divider());
        rows.push(self.between(
            self.heading("检查结果"),
            self.lab_button(
                "report",
                if self.studio_lab.report_open {
                    "收起"
                } else {
                    "全部结果"
                },
                Lab::ToggleReport,
                Btn::Ghost,
            ),
        ));
        if let Some((_, report)) = self
            .studio_lab
            .report
            .as_ref()
            .filter(|(id, _)| id == &a.id)
        {
            if let Some(checks) = report["checks"].as_array() {
                let passed = checks.iter().filter(|c| c["passed"] == true).count();
                rows.push(self.text(
                    format!("{passed} / {} 项通过", checks.len()),
                    CAPTION,
                    if a.passed { p.success } else { p.text2 },
                ));
                for check in checks
                    .iter()
                    .filter(|c| self.studio_lab.report_open || c["passed"] != true)
                    .take(if self.studio_lab.report_open { 32 } else { 2 })
                {
                    let pass = check["passed"] == true;
                    let label = check["label"].as_str().unwrap_or("检查项");
                    rows.push(self.lead(
                        S2,
                        self.ui_icon(
                            if pass { UiIcon::Check } else { UiIcon::Close },
                            14.,
                            if pass { p.success } else { p.danger },
                        ),
                        Expanded::new(1., self.text(label, CAPTION, p.text2)).finish(),
                    ));
                    if !pass {
                        if let Some(detail) = check["detail"].as_str().filter(|s| !s.is_empty()) {
                            rows.push(self.text(detail, CAPTION, p.danger));
                        }
                    }
                }
            }
        } else if let Some(error) = &self.studio_lab.report_error {
            rows.push(self.text(error, CAPTION, p.danger));
        } else {
            rows.push(self.caption("正在读取检查报告…"));
        }
        rows.push(self.text(
            "会自动检查页面、手机上的显示和主要操作；发到手机后不连电脑也能打开。",
            MICRO,
            p.text3,
        ));
        let mut all = vec![self.panel(rows)];
        let web = self.studio.web.as_ref().filter(|w| w.authorized);
        let current = web.is_some_and(|w| self.studio_lab.cloud_scope.as_ref() == Some(&w.scope));
        let mut cloud = vec![
            self.between(
                self.heading("在官网继续编辑"),
                self.lab_button("cloud-models", "读取官网模型", Lab::CloudModels, Btn::Ghost),
            ),
            self.text(
                "把通过检查的作品建成官网项目，在官网继续改。会用官网账号的额度。",
                CAPTION,
                p.text2,
            ),
        ];
        if current && !self.studio_lab.cloud_models.is_empty() {
            cloud.push(
                self.wrap(
                    self.studio_lab
                        .cloud_models
                        .iter()
                        .map(|m| {
                            self.lab_button(
                                &format!("cloud-model-{}", m.id),
                                &m.name,
                                Lab::CloudModel(m.id.clone()),
                                if self.studio_lab.cloud_model.as_ref() == Some(&m.id) {
                                    Btn::Primary
                                } else {
                                    Btn::Secondary
                                },
                            )
                        })
                        .collect(),
                ),
            );
        } else {
            cloud.push(self.caption(if web.is_some() {
                "先读取官网可用模型，再创建项目。"
            } else {
                "请先在「连接设置」里连接官网。"
            }));
        }
        cloud.push(self.wrap(vec![
            if self
                .studio_lab_cloud_feedback()
                .is_some_and(|f| f.receipt.is_some())
            {
                self.lab_button(
                    "cloud-open",
                    "继续编辑官网项目",
                    Lab::OpenCloud,
                    Btn::Primary,
                )
            } else if self
                .studio_lab_cloud_feedback()
                .is_some_and(|f| f.recovery_url.is_some())
            {
                self.lab_button(
                    "cloud-recover",
                    "打开已创建的项目",
                    Lab::OpenCloud,
                    Btn::Secondary,
                )
            } else if a.passed && current && self.studio_lab.cloud_model.is_some() {
                self.lab_button(
                    "cloud-sync",
                    "新建官网项目并同步",
                    Lab::SyncCloud,
                    Btn::Secondary,
                )
            } else {
                self.disabled_button("新建官网项目并同步")
            },
        ]));
        if let Some(feedback) = self.studio_lab_cloud_feedback() {
            if let Some(note) = &feedback.note {
                cloud.push(self.text(note, CAPTION, p.success));
            }
            if let Some(error) = &feedback.error {
                cloud.push(self.text(error, CAPTION, p.danger));
            }
        }
        all.push(self.panel(cloud));
        self.vstack(S3, all)
    }
    pub(in super::super) fn studio_lab_settings(&self) -> El {
        let p = self.p();
        let mut rows = vec![
            self.between(
                self.heading("自己的模型服务"),
                self.pill_badge(
                    if self.studio_lab.config.has_key {
                        "已保存密钥"
                    } else if self.selected.is_some() {
                        "手机已连接"
                    } else {
                        "未配置"
                    },
                    if self.studio_lab.config.has_key || self.selected.is_some() {
                        p.success
                    } else {
                        p.text2
                    },
                    p.surface,
                ),
            ),
            self.text(
                "连接手机时默认用手机上的 Amoo 来做应用和小游戏；也可以在下面填写自己的模型服务，用自己的额度。",
                BODY,
                p.text2,
            ),
            self.hstack(
                6.,
                vec![
                    self.ui_icon(UiIcon::Bolt, 14., p.accent),
                    self.caption("一键选择常用大模型服务商 (免手动查接口地址)"),
                ],
            ),
            self.wrap(
                [
                    (0u8, "阶跃星辰 StepFun"),
                    (1, "DeepSeek"),
                    (2, "通义千问 Qwen"),
                    (3, "Kimi"),
                    (4, "智谱 GLM"),
                    (5, "OpenAI"),
                    (6, "Claude"),
                    (7, "Ollama 本地免Key"),
                ]
                .into_iter()
                .map(|(idx, name)| {
                    self.lab_button(
                        &format!("provider-preset-{idx}"),
                        name,
                        Lab::ApplyProviderPreset(idx),
                        Btn::Secondary,
                    )
                })
                .collect(),
            ),
            self.caption("API Base URL"),
            self.input(F_LAB_ENDPOINT, "https://你的服务地址/v1", None),
            self.segmented(
                "lab-protocol",
                [("OpenAI 兼容", 0), ("Anthropic", 1)]
                    .iter()
                    .map(|(name, i)| {
                        (
                            (*name).into(),
                            self.studio_lab.protocol == *i,
                            Action::StudioLab(Lab::Protocol(*i)),
                        )
                    })
                    .collect(),
            ),
            self.caption("API Key"),
            self.input(
                F_LAB_KEY,
                if self.studio_lab.config.has_key {
                    "已安全保存；留空保留，粘贴新密钥可替换"
                } else {
                    "粘贴密钥，仅保存到本机"
                },
                None,
            ),
            self.caption("模型名称"),
            self.input(F_LAB_MODEL, "填写模型 ID，或读取下方模型目录", None),
            self.wrap(vec![
                self.lab_button("config-save", "保存设置", Lab::Save, Btn::Primary),
                self.lab_button("config-test", "测试实际调用", Lab::Test, Btn::Secondary),
                self.lab_button("config-models", "读取模型", Lab::Models, Btn::Secondary),
                self.lab_button("config-clear", "清除密钥", Lab::ClearKey, Btn::Ghost),
            ]),
        ];
        if self.studio_lab_config_dirty() {
            rows.push(self.text("有尚未保存的模型设置", CAPTION, p.text3));
        }
        if let Some(note) = &self.studio_lab.model_note {
            rows.push(self.text(note, CAPTION, p.success));
        }
        if let Some(error) = &self.studio_lab.error {
            rows.push(self.text(error, CAPTION, p.danger));
        }
        let query = self.fields[F_LAB_MODEL].text.trim().to_lowercase();
        let query = if self
            .studio_lab
            .models
            .iter()
            .any(|m| m.to_lowercase() == query)
        {
            String::new()
        } else {
            query
        };
        let filtered: Vec<_> = self
            .studio_lab
            .models
            .iter()
            .filter(|m| query.is_empty() || m.to_lowercase().contains(&query))
            .take(12)
            .collect();
        if !filtered.is_empty() {
            rows.push(self.divider());
            rows.push(self.caption("模型目录 · 输入模型名筛选，选择后保存"));
            for m in filtered {
                rows.push(self.list_row(
                    format!("lab-model-{m}"),
                    self.line(m, CAPTION, p.text),
                    self.fields[F_LAB_MODEL].text == *m,
                    Some(Action::StudioLab(Lab::Model(m.clone()))),
                    None,
                ));
            }
        }
        self.panel(rows)
    }
}
