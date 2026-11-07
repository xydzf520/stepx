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
            Lab::Source(stepx_companion::model_source::Source::PhoneAccount) => {
                Some(UiIcon::Smartphone)
            }
            Lab::CheckPhoneConnection | Lab::ProbePhoneText => Some(UiIcon::Refresh),
            Lab::CancelPhoneTextProbe => Some(UiIcon::Close),
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
        let phone_source = self.studio_lab_uses_phone();
        let configured = !phone_source
            && self.studio_lab.config.has_key
            && !self.studio_lab.config.model.is_empty();
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
                self.heading("应用创作"),
                if configured {
                    self.pill_badge(
                        format!("自有模型 · {}", self.studio_lab.config.model),
                        p.success,
                        p.surface,
                    )
                } else if phone_source {
                    self.pill_badge("使用手机账号", p.accent, p.surface)
                } else {
                    self.pill_badge("独立 API 待配置", p.text2, p.surface)
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
            if phone_source && phone_ready {
                "Enter 开始 · Shift+Enter 换行 · 本次使用手机账号，由手机路由模型"
            } else if phone_source {
                "请先通过 USB 或无线连接手机；不会自动切换到独立 API"
            } else {
                "Enter 开始 · Shift+Enter 换行 · 本次使用已选择的独立 API"
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
                self.content_icon(UiIcon::Monitor, 36., p.text3),
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
                        self.content_icon(
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
            let range = self.list_page_range("studio-lab-cloud-models",self.studio_lab.cloud_models.len(),40);
            cloud.push(self.list_page_controls("studio-lab-cloud-models",self.studio_lab.cloud_models.len(),40));
            cloud.push(
                self.wrap(
                    self.studio_lab
                        .cloud_models
                        .iter()
                        .skip(range.start).take(range.len())
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
    fn lab_settings_paragraph(
        &self,
        text: impl Into<String>,
        size: f32,
        color: facloud_ui::color::ColorU,
    ) -> El {
        Text::new(text.into(), self.font, size)
            .with_color(color)
            .with_line_height_ratio(1.45)
            .soft_wrap(true)
            .finish()
    }
    pub(in super::super) fn studio_lab_settings(&self) -> El {
        use stepx_companion::model_source::Source;
        let p = self.p();
        let phone = self.studio_lab_uses_phone();
        let (automatic_ready, automatic_reason) = self.context_automatic_source_status();
        let mut rows = vec![
            self.between(self.heading("AI 来源"),self.pill_badge(self.studio_lab.model_source.label(),p.accent,p.surface)),
            self.lab_settings_paragraph("默认使用手机账号，电脑延续手机上的 AI 能力。只有明确选择独立 API 后，才使用单独配置的服务与额度。",BODY,p.text2),
            self.wrap(vec![
                self.lab_button("source-phone","手机账号",Lab::Source(Source::PhoneAccount),if phone {Btn::Primary} else {Btn::Secondary}),
                self.lab_button("source-api","独立 API",Lab::Source(Source::IndependentApi),if !phone {Btn::Primary} else {Btn::Secondary}),
            ]),
        ];
        rows.push(self.lab_settings_paragraph(
            "模型用量与可用额度由手机或所选服务决定；服务未返回的额度不显示。",
            CAPTION,
            p.text3,
        ));
        if let Some(error) = &self.studio_lab.model_source_error {
            rows.push(self.lab_settings_paragraph(error, CAPTION, p.danger));
        }
        if phone {
            rows.push(self.heading("沿用手机账号"));
            rows.push(self.lab_settings_paragraph(
                self.studio_lab_phone_connection_label(),
                BODY,
                p.text,
            ));
            rows.push(self.lab_settings_paragraph(if automatic_ready {
                "可以点击生成或批量整理当前资料，任务会逐份完成。当前设备账号的无人值守通道已核验；如需跟随新资料自动更新，可在首页来源设置中另外开启。USB与无线均支持。"
            } else {"手机账号支持点击生成、批量整理当前资料。当前客户端尚未支持此账号的无人值守生成；通知、短信和讨论资料仍可自动同步。USB 与无线均支持。"},BODY,p.text2));
            rows.push(self.lab_settings_paragraph(&automatic_reason, CAPTION, p.text2));
            if let Some(meta) = self.context.accepted_phone_inference(
                self.selected.as_ref(),
                self.amoo_ui.account_scope.as_deref(),
            ) {
                let model = if meta.model.trim().is_empty() {
                    "手机未报告模型标识".to_owned()
                } else {
                    meta.model
                        .chars()
                        .map(|c| if c.is_control() { ' ' } else { c })
                        .take(100)
                        .collect()
                };
                rows.push(self.lab_settings_paragraph(
                    format!("最近已接受成果 · 实际手机模型：{model}"),
                    BODY,
                    p.text,
                ));
                let count = |value: Option<u64>| {
                    value
                        .map(|n| n.to_string())
                        .unwrap_or_else(|| "未知".into())
                };
                let input = meta.usage.as_ref().and_then(|usage| usage.input_tokens);
                let output = meta.usage.as_ref().and_then(|usage| usage.output_tokens);
                rows.push(self.lab_settings_paragraph(
                    format!(
                        "服务返回计数 · 输入 {} Token · 输出 {} Token",
                        count(input),
                        count(output)
                    ),
                    CAPTION,
                    p.text2,
                ));
                rows.push(self.lab_settings_paragraph(
                    "以上是该已接受结果的推理计数，不表示账户余额或账单。",
                    CAPTION,
                    p.text3,
                ));
                rows.push(self.lab_settings_paragraph(
                    meta.session_cleanup_label(),
                    CAPTION,
                    p.text2,
                ));
            } else {
                rows.push(self.lab_settings_paragraph("尚未读取到当前账号的成稿调用凭证；成功生成并完成读取后，会显示实际手机模型与服务返回用量。",CAPTION,p.text3));
            }

            rows.push(self.wrap(vec![
                self.button_icon(
                    "lab-phone-account",
                    "账号与用量",
                    UiIcon::Smartphone,
                    Action::Page(super::super::Page::Account),
                    Btn::Secondary,
                ),
                if self.selected.is_some() {
                    self.lab_button(
                        "phone-check",
                        "检查连接",
                        Lab::CheckPhoneConnection,
                        Btn::Secondary,
                    )
                } else {
                    self.disabled_button("检查连接")
                },
            ]));
            if self.studio_lab.phone_probe_cancel.is_some() {
                rows.push(self.lab_button(
                    "phone-probe-cancel",
                    "取消通道验证",
                    Lab::CancelPhoneTextProbe,
                    Btn::Secondary,
                ));
                rows.push(self.lab_settings_paragraph(
                    "正在等待手机模型；取消后不会自动重发。",
                    CAPTION,
                    p.text2,
                ));
            } else if !stepx_companion::home_ai::phone_text_probe_supported() {
                rows.push(self.disabled_button("当前客户端尚未适配后台账号通道"));
            } else if self.selected.is_some() && self.amoo_ui.account_scope.is_some() {
                rows.push(self.lab_button(
                    "phone-text-probe",
                    "验证手机纯文本通道（至多一次模型调用）",
                    Lab::ProbePhoneText,
                    Btn::Secondary,
                ));
            } else {
                rows.push(self.disabled_button("连接手机并确认账号后可验证纯文本通道"));
            }
            if let Some((serial, account, result)) = &self.studio_lab.phone_probe_note {
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|device| &device.serial == serial)
                    && self.amoo_ui.account_scope.as_deref() == Some(account.as_str())
                {
                    match result {
                        Ok(meta) => {
                            let model = if meta.model.is_empty() {
                                "手机未报告模型标识"
                            } else {
                                meta.model.as_str()
                            };
                            rows.push(self.lab_settings_paragraph(
                                format!("通道验证通过 · 实际模型：{model}"),
                                BODY,
                                p.text,
                            ));
                            let count = |value: Option<u64>| {
                                value
                                    .map(|n| n.to_string())
                                    .unwrap_or_else(|| "未知".into())
                            };
                            rows.push(self.lab_settings_paragraph(
                                format!(
                                    "验证调用的服务计数 · 输入 {} · 输出 {} Token",
                                    count(meta.usage.as_ref().and_then(|u| u.input_tokens)),
                                    count(meta.usage.as_ref().and_then(|u| u.output_tokens))
                                ),
                                CAPTION,
                                p.text2,
                            ));
                            rows.push(self.lab_settings_paragraph(
                                meta.session_cleanup_label(),
                                CAPTION,
                                p.text2,
                            ));
                            rows.push(self.lab_settings_paragraph(
                                "验证不会开启后台生成；需要在首页来源设置中明确开启。",
                                CAPTION,
                                p.text3,
                            ));
                        }
                        Err(error) => rows.push(self.lab_settings_paragraph(
                            format!("通道验证未通过：{error}"),
                            CAPTION,
                            p.danger,
                        )),
                    }
                }
            }
            if let Some((serial, result)) = &self.studio_lab.phone_connection_note {
                if self
                    .selected
                    .as_ref()
                    .is_some_and(|device| &device.serial == serial)
                {
                    match result {
                        Ok(note) => rows.push(self.lab_settings_paragraph(note, CAPTION, p.text2)),
                        Err(note) => rows.push(self.lab_settings_paragraph(
                            format!("上次连接检查未通过：{note}"),
                            CAPTION,
                            p.danger,
                        )),
                    }
                }
            }
            rows.push(self.lab_settings_paragraph("检查连接不会调用模型。手机不可用时会提示处理，不会自动改用独立 API。已有 API 配置保留。",CAPTION,p.text3));
        } else {
            rows.push(self.heading("独立 API 配置"));
            rows.push(self.lab_settings_paragraph(
                "以下设置仅用于已选择的独立 API。实际调用测试会使用该服务的额度。",
                BODY,
                p.text2,
            ));
            rows.extend(vec![
                self.hstack(
                    6.,
                    vec![
                        self.content_icon(UiIcon::Bolt, 14., p.accent),
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
            ]);
            if self.studio_lab_config_dirty() {
                rows.push(self.lab_settings_paragraph("有尚未保存的 API 设置", CAPTION, p.text3));
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
        }
        if let Some(note) = &self.studio_lab.model_note {
            rows.push(self.lab_settings_paragraph(note, CAPTION, p.text2));
        }
        if let Some(error) = &self.studio_lab.error {
            rows.push(self.lab_settings_paragraph(error, CAPTION, p.danger));
        }
        self.panel(rows)
    }
}
