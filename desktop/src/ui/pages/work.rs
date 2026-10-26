//! 文件处理页（Page::Work）：用工作文件夹里的文件干活，带结果预览。
//! 只作为次要入口，从「我的作品」或成果卡进入；主入口是 Amoo 对话。
use super::super::{
    amoo_display::session_title, icons::UiIcon, theme::*, widgets::*, Action, AmooChatItem,
    Companion, Page, WorkAction, F_WORK_AGENT_NAME, F_WORK_CAP_SEARCH, F_WORK_PROMPT,
    F_WORK_SEARCH,
};
use facloud_ui::{color::ColorU, elements::*, fonts::Weight, platform::Cursor};
use pathfinder_geometry::vector::vec2f;
use std::collections::HashSet;
use stepx_companion::storage::format_bytes;

fn mix_color(a: ColorU, b: ColorU, t: f32) -> ColorU {
    let t = t.clamp(0.0, 1.0);
    if t <= 0.0 {
        return a;
    }
    if t >= 1.0 {
        return b;
    }
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * t).round() as u8;
    if a.a == 0 {
        return ColorU::new(b.r, b.g, b.b, mix(0, b.a));
    }
    if b.a == 0 {
        return ColorU::new(a.r, a.g, a.b, mix(a.a, 0));
    }
    ColorU::new(mix(a.r, b.r), mix(a.g, b.g), mix(a.b, b.b), mix(a.a, b.a))
}

// Shared with the desktop harness, including mixed protocol + Markdown replies.
fn work_message_text(text: &str, user: bool) -> Option<String> {
    stepx_companion::work_protocol::display_text(text, user)
}

const WORK_SCENARIOS: [&str; 6] = [
    "资料总结",
    "表格分析",
    "文档修改",
    "图片创作",
    "编写游戏",
    "制作 PPT",
];

fn compact_work_text(text: &str, max: usize) -> String {
    let text = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if text.chars().count() > max {
        format!("{}…", text.chars().take(max).collect::<String>())
    } else {
        text
    }
}

pub(crate) fn work_step_name(name: &str) -> &str {
    match name {
        "exec" | "system.run" | "stepclaw.exec" => "执行任务步骤",
        "ability" | "ability.call" => "调用手机能力",
        "list_files" => "浏览目录结构",
        "read_file" => "读取文件内容",
        "csv_summary" => "分析表格数据",
        "write_file" => "写入文件内容",
        "search_files" => "检索工作区文件",
        "web_search" => "联网搜索资料",
        "fetch_page" => "抓取网页正文",
        "task_create" | "sessions_spawn" => "创建子任务",
        "text_to_image" => "生成图片",
        "write_document" => "撰写 Word 文档",
        "create_document" => "生成文档",
        "edit_document" => "修订文档",
        "edit_text" => "修改文件",
        "write_spreadsheet" => "生成 Excel 表格",
        "create_spreadsheet" => "生成表格",
        "write_presentation" => "制作 PPT 演示稿",
        "create_presentation" => "制作演示文稿",
        _ => name,
    }
}

impl Companion {
    pub(in super::super) fn work_step_name(name: &str) -> &str {
        work_step_name(name)
    }

    fn work_button(&self, id: &str, label: &str, action: WorkAction, kind: Btn) -> El {
        self.button(format!("work-{id}"), label, Action::Work(action), kind)
    }

    pub(in super::super) fn work_action_icon(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        icon: UiIcon,
        action: Action,
    ) -> El {
        let p = self.p();
        let label = label.into();
        Hoverable::new(self.mouse_state(id.into()), move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let t = if self.settings.value.reduced_motion {
                if hovered {
                    1.0
                } else {
                    0.0
                }
            } else {
                mouse.hover_progress().clamp(0.0, 1.0)
            };
            let bg = if mouse.is_clicked() {
                p.accent_soft
            } else {
                mix_color(p.bg, p.hover, t)
            };
            let fg = if mouse.is_clicked() || hovered {
                p.text
            } else {
                mix_color(p.text2, p.text, t)
            };
            let button = ConstrainedBox::new(
                Container::new(Align::new(self.ui_icon(icon, 16., fg)).finish())
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            )
            .with_width(30.)
            .with_height(30.)
            .finish();
            if !hovered {
                return button;
            }
            Stack::new()
                .with_child(button)
                .with_positioned_child(
                    Container::new(self.line(label, MICRO, p.text))
                        .with_horizontal_padding(S2)
                        .with_vertical_padding(S1)
                        .with_background_color(p.bg)
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -5.),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                )
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish()
    }

    pub(in super::super) fn work_actions(&self) -> Vec<El> {
        vec![
            if self.busy || self.work.catalog_loading {
                self.disabled_button("正在同步")
            } else {
                self.work_button("refresh", "同步手机", WorkAction::Refresh, Btn::Secondary)
            },
            if self.busy || self.work.running {
                self.disabled_button("新建工作")
            } else {
                self.button_icon(
                    "work-new",
                    "新建工作",
                    UiIcon::Plus,
                    Action::Work(WorkAction::NewChat),
                    Btn::Primary,
                )
            },
        ]
    }

    fn work_title(&self, icon: UiIcon, title: impl Into<String>) -> El {
        let p = self.p();
        self.hstack(
            S2,
            vec![
                self.ui_icon(icon, 16., p.text2),
                Expanded::new(1., self.line_w(title, HEADING, p.text, Weight::Semibold)).finish(),
            ],
        )
    }

    fn work_session_pane(&self) -> El {
        let p = self.p();
        let query = self.fields[F_WORK_SEARCH].text.trim().to_lowercase();
        let mut rows = Vec::new();
        for (index, session) in self.amoo_sessions.iter().enumerate() {
            if !query.is_empty()
                && !session_title(&session.title)
                    .to_lowercase()
                    .contains(&query)
                && !session.agent.to_lowercase().contains(&query)
            {
                continue;
            }
            let active = self.work_session_index() == Some(index);
            let pending = self.amoo_pending_reply.contains_key(&session.session_key);
            let title = session_title(&session.title);
            let subtitle = if pending {
                "正在处理".to_string()
            } else {
                session.meta.clone()
            };
            let row = self.vstack(
                S1,
                vec![
                    self.line_w(title, BODY, p.text, Weight::Medium),
                    self.line(subtitle, MICRO, if pending { p.success } else { p.text3 }),
                ],
            );
            rows.push(
                Hoverable::new(
                    self.mouse_state(format!("work-session-{index}")),
                    move |mouse| {
                        let hovered = mouse.is_mouse_over_element();
                        let t = if self.settings.value.reduced_motion {
                            if hovered {
                                1.0
                            } else {
                                0.0
                            }
                        } else {
                            mouse.hover_progress().clamp(0.0, 1.0)
                        };
                        let bg = if mouse.is_clicked() {
                            p.accent_soft
                        } else if active {
                            mix_color(p.selected, p.hover, t * 0.5)
                        } else {
                            mix_color(p.bg, p.hover, t)
                        };
                        let border = if active || mouse.is_clicked() {
                            p.border_strong
                        } else {
                            mix_color(p.border, p.border_strong, t)
                        };
                        Container::new(row)
                            .with_uniform_padding(S3)
                            .with_background_color(bg)
                            .with_border(Border::all(1.).with_border_color(border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                    },
                )
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Work(WorkAction::Session(index)))
                })
                .finish(),
            );
        }
        if rows.is_empty() {
            rows.push(self.text(
                if query.is_empty() {
                    "手机会话将在同步后显示。可以新建对话开始工作。"
                } else {
                    "没有匹配的会话"
                },
                CAPTION,
                p.text3,
            ));
        }
        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(S3)
                .with_child(self.work_title(UiIcon::Tools, "处理记录"))
                .with_child(self.input(F_WORK_SEARCH, "搜索工作与会话…", None))
                .with_child(
                    Expanded::new(
                        1.,
                        self.pane_scroll(
                            self.scroll_aux.clone(),
                            Container::new(self.vstack(S2, rows))
                                .with_padding_bottom(S3)
                                .finish(),
                        ),
                    )
                    .finish(),
                )
                .with_child(self.text("后台连接手机 Amoo，无需唤起手机界面。", MICRO, p.text3))
                .finish(),
        )
        .with_uniform_padding(S4)
        .with_background_color(p.sidebar)
        .with_border(Border::right(1.).with_border_color(p.border))
        .finish()
    }

    fn work_chat_bubble(&self, index: usize, item: &AmooChatItem) -> El {
        let p = self.p();
        if let AmooChatItem::Attachment {
            name,
            kind,
            url,
            path,
            size_bytes,
            ..
        } = item
        {
            return self.hover_panel_interactive(
                format!("work-chat-att-{index}"),
                self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(
                                if kind == "image" {
                                    UiIcon::Image
                                } else {
                                    UiIcon::File
                                },
                                18.,
                                p.text2,
                            ),
                            Expanded::new(
                                1.,
                                self.vstack(
                                    S1,
                                    vec![
                                        self.line_w(name, BODY, p.text, Weight::Medium),
                                        self.line(
                                            size_bytes
                                                .map(format_bytes)
                                                .unwrap_or_else(|| "手机任务附件".into()),
                                            MICRO,
                                            p.text3,
                                        ),
                                    ],
                                ),
                            )
                            .finish(),
                        ],
                    ),
                    self.work_button(
                        &format!("preview-result-{index}"),
                        "预览",
                        WorkAction::PreviewRemote(index),
                        Btn::Secondary,
                    ),
                ),
                Some(Action::Work(WorkAction::PreviewRemote(index))),
                Some(if !path.is_empty() {
                    Action::Work(WorkAction::OpenPreview(path.clone()))
                } else if !url.is_empty() {
                    Action::StudioOpenUrl(url.clone())
                } else {
                    Action::Work(WorkAction::PreviewRemote(index))
                }),
                Some(crate::ui::context_menu::Target::ChatAttachment {
                    path: path.clone(),
                    url: url.clone(),
                    remove_idx: None,
                }),
            );
        }
        if let AmooChatItem::ToolRecords { key, records } = item {
            let expanded = self.amoo_expanded_tool_records.contains(key);
            let mut names = Vec::new();
            for record in records {
                let name = work_step_name(&record.name);
                if !names.contains(&name) {
                    names.push(name);
                }
            }
            let mut children = vec![self.between(
                self.button_icon(
                    format!("work-records-{key}"),
                    format!("执行过程 · {} 步", records.len()),
                    if expanded {
                        UiIcon::ChevronDown
                    } else {
                        UiIcon::ChevronRight
                    },
                    Action::AmooToggleToolRecords(key.clone()),
                    Btn::Ghost,
                ),
                ConstrainedBox::new(self.line(
                    compact_work_text(&names.join(" · "), 30),
                    MICRO,
                    p.text3,
                ))
                .with_max_width(180.)
                .finish(),
            )];
            if expanded {
                for (record_index, record) in records.iter().enumerate() {
                    let mut details = vec![self.between(
                        self.line(
                            format!("{:02}  {}", record_index + 1, work_step_name(&record.name)),
                            CAPTION,
                            p.text2,
                        ),
                        self.line(&record.time_label, MICRO, p.text3),
                    )];
                    if let Some(detail) = record.detail.as_ref().filter(|text| !text.is_empty()) {
                        let detail_key = format!("work-record-detail-{key}-{record_index}");
                        let full = self.amoo_expanded_tool_records.contains(&detail_key);
                        details.push(self.text(
                            if full {
                                detail.clone()
                            } else {
                                compact_work_text(detail, 160)
                            },
                            MICRO,
                            p.text3,
                        ));
                        if detail.chars().count() > 160 {
                            details.push(self.link_muted(
                                format!("{detail_key}-toggle"),
                                if full { "收起详情" } else { "查看详情" },
                                Action::AmooToggleToolRecords(detail_key),
                            ));
                        }
                    }
                    children.push(
                        Container::new(self.vstack(S1, details))
                            .with_uniform_padding(S2)
                            .with_border(Border::top(1.).with_border_color(p.border))
                            .finish(),
                    );
                }
            }
            return Container::new(self.vstack(S2, children))
                .with_uniform_padding(S2)
                .with_background_color(p.surface)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish();
        }
        let (user, body, attachment, time) = match item {
            AmooChatItem::UserText { text, attachment } => (
                true,
                text.clone(),
                attachment
                    .as_ref()
                    .map(|(name, size, _)| format!("{name} · {size}")),
                String::new(),
            ),
            AmooChatItem::UserVoice { transcript, .. } => {
                (true, transcript.clone(), None, String::new())
            }
            AmooChatItem::AssistantRich {
                text, time_label, ..
            } => (
                false,
                text.clone().unwrap_or_default(),
                None,
                time_label.clone(),
            ),
            AmooChatItem::ToolRecords { .. } | AmooChatItem::Attachment { .. } => unreachable!(),
        };
        let Some(body) = work_message_text(&body, user) else {
            return Empty::new().finish();
        };
        if body.trim().is_empty() && attachment.is_none() {
            return Empty::new().finish();
        }
        let mut meta = vec![self.line(time, MICRO, p.text3)];
        if !user && !body.is_empty() {
            meta.push(self.work_action_icon(
                format!("work-copy-message-{index}"),
                "复制回复",
                UiIcon::Copy,
                Action::CopyText(body.clone(), "已复制回复".into()),
            ));
        }
        if !user && !body.is_empty() {
            meta.push(self.speech_icon(
                format!(
                    "work-speech:{}:{index}",
                    self.work_session_index().unwrap_or(usize::MAX)
                ),
                body.clone(),
            ));
        }
        let mut children = vec![self.between(
            self.hstack(
                S2,
                vec![
                    self.ui_icon(
                        if user { UiIcon::Chat } else { UiIcon::Star },
                        14.,
                        if user { p.text2 } else { p.accent },
                    ),
                    self.line_w(
                        if user { "任务要求" } else { "Amoo" },
                        CAPTION,
                        p.text2,
                        Weight::Medium,
                    ),
                ],
            ),
            self.hstack(S1, meta),
        )];
        if let Some(attachment) = attachment {
            children.push(self.hstack(
                S2,
                vec![
                    self.ui_icon(UiIcon::Attach, 14., p.text3),
                    Expanded::new(1., self.line(attachment, CAPTION, p.text2)).finish(),
                ],
            ));
        }
        if !body.is_empty() {
            children.push(if user {
                self.text(body.clone(), BODY, p.text)
            } else {
                self.work_rich_text(
                    &self.live_reply_text(index, &body),
                    &format!("message-{index}"),
                )
            });
        }
        Container::new(self.vstack(S2, children))
            .with_uniform_padding(S3)
            .with_background_color(if user { p.surface } else { p.bg })
            .with_border(Border::all(if user { 1. } else { 0. }).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }

    fn work_prompt_input(&self) -> El {
        let p = self.p();
        let field = &self.fields[F_WORK_PROMPT];
        let focused = self.focus == Some(F_WORK_PROMPT);
        let mut value = field.text.clone();
        let mut caret = 0;
        if focused {
            let cursor = field.cursor.min(value.len());
            caret = value[..cursor].chars().count() + field.preedit.chars().count();
            value.insert_str(cursor, &field.preedit);
        }
        let is_empty = field.text.is_empty() && field.preedit.is_empty();
        if value.is_empty() {
            value = if let Some(hint) = &self.work.revise_hint {
                hint.clone()
            } else if self.work.mode == 2 {
                "描述要生成的图片，可添加参考图、指定风格与尺寸…".into()
            } else {
                "描述要处理的事情，可以附上文件…".into()
            };
        }
        Hoverable::new(self.mouse_state("work-prompt"), move |_| {
            ConstrainedBox::new(
                Container::new(self.pane_scroll(
                    self.work.prompt_scroll.clone(),
                    self.multiline_field_text(
                        &value,
                        caret,
                        F_WORK_PROMPT,
                        if is_empty { p.text3 } else { p.text },
                    ),
                ))
                .with_horizontal_padding(S1)
                .with_vertical_padding(S2)
                .with_background_color(p.bg)
                .finish(),
            )
            .with_min_height(48.)
            .with_max_height(112.)
            .finish()
        })
        .with_cursor(Cursor::IBeam)
        .with_defer_events_to_children()
        .on_mouse_down(|ctx, _, _| ctx.dispatch_typed_action(Action::Focus(F_WORK_PROMPT)))
        .on_right_click(|ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::Input(F_WORK_PROMPT),
                at,
            ))
        })
        .finish()
    }

    fn work_office_quick_bar(&self) -> El {
        let p = self.p();
        let starters = [
            (0u8, "Word 方案.docx"),
            (1u8, "Excel 财务表.xlsx"),
            (2u8, "PPT 汇报.pptx"),
            (3u8, "调研报告.md"),
            (4u8, "做小游戏"),
        ];
        self.panel(vec![
            self.between(
                self.hstack(
                    S2,
                    vec![
                        self.ui_icon(UiIcon::Bolt, 15., p.accent),
                        self.line_w("生成示例文件", CAPTION, p.text, Weight::Semibold),
                    ],
                ),
                self.line("保存在工作文件夹里，可以修改或发到手机", MICRO, p.text3),
            ),
            self.wrap(
                starters
                    .iter()
                    .map(|(idx, label)| {
                        self.work_button(
                            &format!("quick-starter-{idx}"),
                            label,
                            WorkAction::QuickStarter(*idx),
                            Btn::Secondary,
                        )
                    })
                    .collect(),
            ),
            self.hstack(
                S2,
                vec![
                    self.ui_icon(UiIcon::Star, 14., p.text2),
                    self.line("或者让 Amoo 按你的资料来做：", MICRO, p.text2),
                ],
            ),
            self.wrap(
                WORK_SCENARIOS
                    .iter()
                    .enumerate()
                    .map(|(index, name)| {
                        self.work_button(
                            &format!("template-{index}"),
                            name,
                            WorkAction::Template(index as u8),
                            Btn::Ghost,
                        )
                    })
                    .collect(),
            ),
        ])
    }

    fn work_conversation_pane(&self) -> El {
        let p = self.p();
        // 文件处理页使用自己的会话，不显示 Amoo 当前对话。
        let session = self.work_session();
        let mut stream = vec![self.work_office_quick_bar()];
        if let Some(session) = session.filter(|s| !s.items.is_empty()) {
            stream.extend(
                session
                    .items
                    .iter()
                    .enumerate()
                    .map(|(i, item)| self.work_chat_bubble(i, item)),
            );
        } else {
            stream.push(self.panel(vec![
                self.work_title(UiIcon::Tools, "要处理哪些文件？"),
                self.text(
                    "这里用工作文件夹里的文件干活。平时直接在 Amoo 对话里说需求就行，做好的文件也会出现在对话里。",
                    BODY,
                    p.text2,
                ),
            ]));
        }
        if !self.work.events.is_empty() {
            let key = "amoo-work-desktop-events";
            let expanded = self.amoo_expanded_tool_records.contains(key);
            let mut events = vec![self.button_icon(
                "work-desktop-events",
                format!("工作过程 · {} 条记录", self.work.events.len()),
                if expanded {
                    UiIcon::ChevronDown
                } else {
                    UiIcon::ChevronRight
                },
                Action::AmooToggleToolRecords(key.into()),
                Btn::Ghost,
            )];
            if expanded {
                events.extend(self.work.events.iter().enumerate().map(|(index, event)| {
                    let detail_key = format!("work-event-detail-{index}");
                    let full = self.amoo_expanded_tool_records.contains(&detail_key);
                    let mut details = vec![self.text(
                        if full {
                            event.clone()
                        } else {
                            compact_work_text(event, 180)
                        },
                        CAPTION,
                        p.text2,
                    )];
                    if event.chars().count() > 180 {
                        details.push(self.link_muted(
                            format!("{detail_key}-toggle"),
                            if full { "收起详情" } else { "查看详情" },
                            Action::AmooToggleToolRecords(detail_key),
                        ));
                    }
                    Container::new(self.vstack(S1, details))
                        .with_uniform_padding(S2)
                        .with_border(Border::top(1.).with_border_color(p.border))
                        .finish()
                }));
            } else if let Some(last) = self.work.events.last() {
                events.push(self.line(compact_work_text(last, 110), CAPTION, p.text2));
            }
            stream.push(self.panel(events));
        }
        if let Some(error) = &self.work.error {
            stream.push(self.panel(vec![self.text(error, BODY, p.danger)]));
        }
        #[cfg(feature = "ui-capture")]
        if std::env::args().any(|arg| arg == "--work-format-fixture") {
            stream = vec![self.panel(vec![
                self.line("格式验收样例 · 本地渲染测试，不发送到手机", MICRO, p.text3),
                self.work_rich_text(
                    concat!(
                        "## 项目周报\n\n",
                        "本周完成 **3 项交付**，执行目录为 `reports/本周`。\n\n",
                        "| 交付项 | 状态 | 数量 |\n| :--- | :---: | ---: |\n",
                        "| 分析报告 | **已完成** | 2 |\n| 核对表 | 待复核 | 1 |\n\n",
                        "> 核对结论：保留来源和计算口径，缺失数据不按零处理。\n\n",
                        "1. 读取资料并核对字段。\n2. 生成报告，保留可追溯结果。\n\n",
                        "```json\n{\"result\": \"已保存\", \"files\": 3}\n```\n\n",
                        "查看 [参考资料](https://example.com/report)。\n"
                    ),
                    "format-fixture",
                ),
            ])];
            if std::env::args().any(|arg| arg == "--work-preview-file") {
                stream.push(self.amoo_attachment_card(
                    "手机实测生成图片 · 本地预览验收",
                    "image",
                    "image/png",
                    "https://example.invalid/amoo-work-native-preview.png",
                    None,
                ));
            }
        }
        if self.work.running
            || session.is_some_and(|s| self.amoo_pending_reply.contains_key(&s.session_key))
        {
            stream.push(self.hstack(
                S2,
                vec![
                    self.dot(p.success),
                    self.line(
                        if self.work.status.is_empty() {
                            "正在处理…"
                        } else {
                            &self.work.status
                        },
                        CAPTION,
                        p.text2,
                    ),
                ],
            ));
        }
        let mut composer = Vec::new();
        if self.work.attach_menu_open {
            composer.push(self.panel(vec![
                self.wrap(vec![
                    self.work_button(
                        "attach-image",
                        "上传图片",
                        WorkAction::AttachImages,
                        Btn::Ghost,
                    ),
                    self.work_button(
                        "attach-document",
                        "添加文件",
                        WorkAction::Attach,
                        Btn::Ghost,
                    ),
                ]),
                self.line(
                    if self.work.mode == 1 {
                        "资料复制到工作文件夹 · 图片将同步给手机识别"
                    } else {
                        "支持图片、文档与表格 · 最多 8 个文件 / 8 MB"
                    },
                    MICRO,
                    p.text3,
                ),
            ]));
        }
        if !self.work.attachments.is_empty() {
            composer.push(
                self.wrap(
                    self.work
                        .attachments
                        .iter()
                        .enumerate()
                        .map(|(index, path)| {
                            let image =
                                stepx_companion::amoo_media::mime_type(path).starts_with("image/");
                            Container::new(self.hstack(
                                S2,
                                vec![
                                        self.ui_icon(
                                            if image { UiIcon::Image } else { UiIcon::File },
                                            15.,
                                            if image { p.success } else { p.text2 },
                                        ),
                                        ConstrainedBox::new(
                                            self.line(
                                                path.file_name()
                                                    .map(|s| s.to_string_lossy())
                                                    .unwrap_or_default(),
                                                CAPTION,
                                                p.text,
                                            ),
                                        )
                                        .with_max_width(180.)
                                        .finish(),
                                        self.work_action_icon(
                                            format!("work-remove-attachment-{index}"),
                                            "移除附件",
                                            UiIcon::Close,
                                            Action::Work(WorkAction::RemoveAttachment(index)),
                                        ),
                                    ],
                            ))
                            .with_padding_left(S2)
                            .with_padding_right(S1)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                            .finish()
                        })
                        .collect(),
                ),
            );
        }
        composer.push(self.work_prompt_input());
        let needs_workspace = self.work.mode == 1 && self.work.workspace.is_none();
        let phone_pending =
            session.is_some_and(|s| self.amoo_pending_reply.contains_key(&s.session_key));
        let local_model_ready = self.studio_lab.config.has_key
            && !self.studio_lab.config.endpoint.trim().is_empty()
            && !self.studio_lab.config.model.trim().is_empty();
        let backend_ready = self.selected.is_some() || (self.work.mode == 1 && local_model_ready);
        let can_send = !self.work.running
            && !phone_pending
            && !self.busy
            && backend_ready
            && !self.fields[F_WORK_PROMPT].text.trim().is_empty();
        let mut composer_actions = vec![self.button_icon(
            "work-attach",
            if self.work.attach_menu_open {
                "收起附件"
            } else {
                "添加文件"
            },
            if self.work.attach_menu_open {
                UiIcon::Close
            } else {
                UiIcon::Plus
            },
            Action::Work(WorkAction::ToggleAttachMenu),
            Btn::Ghost,
        )];
        if needs_workspace && self.work.panel != 2 {
            composer_actions.push(self.work_button(
                "composer-workspace",
                "选择文件夹",
                WorkAction::ChooseWorkspace,
                Btn::Ghost,
            ));
        }
        composer_actions.push(
            Expanded::new(
                1.,
                self.line(
                    if needs_workspace {
                        "没选文件夹时用默认工作文件夹，也可以改用「用手机处理」 · Enter 发送"
                    } else {
                        "Enter 发送 · Shift+Enter 换行"
                    },
                    MICRO,
                    p.text3,
                ),
            )
            .finish(),
        );
        let primary_label = if self.work.mode == 2 {
            "生成图片"
        } else {
            "开始处理"
        };
        composer_actions.push(if self.work.running {
            self.work_button("cancel", "停止任务", WorkAction::Cancel, Btn::Secondary)
        } else if phone_pending {
            self.work_button(
                "cancel-wait",
                "停止等待",
                WorkAction::Cancel,
                Btn::Secondary,
            )
        } else if can_send {
            self.work_button("send", primary_label, WorkAction::Send, Btn::Primary)
        } else {
            self.disabled_button(if !backend_ready {
                "请连接手机"
            } else if self.busy {
                "正在同步"
            } else {
                primary_label
            })
        });
        composer.push(self.hstack(S2, composer_actions));
        let header = self.vstack(
            S2,
            vec![
                self.between(
                    self.line_w(
                        session
                            .map(|s| session_title(&s.title))
                            .unwrap_or_else(|| "新工作".into()),
                        HEADING,
                        p.text,
                        Weight::Semibold,
                    ),
                    self.tag(
                        if self.work.running || phone_pending {
                            "进行中"
                        } else {
                            "文件处理"
                        },
                        if self.work.running || phone_pending {
                            p.success
                        } else {
                            p.text2
                        },
                    ),
                ),
                self.segmented(
                    "work-mode",
                    vec![
                        (
                            "用手机处理".into(),
                            self.work.mode == 0,
                            Action::Work(WorkAction::Mode(0)),
                        ),
                        (
                            "用电脑处理".into(),
                            self.work.mode == 1,
                            Action::Work(WorkAction::Mode(1)),
                        ),
                        (
                            "生成图片".into(),
                            self.work.mode == 2,
                            Action::Work(WorkAction::Mode(2)),
                        ),
                    ],
                ),
            ],
        );
        let children = vec![
            header,
            Expanded::new(
                1.,
                self.chat_scroll(
                    Container::new(self.selectable_messages(self.vstack(S3, stream)))
                        .with_padding_bottom(S3)
                        .finish(),
                ),
            )
            .finish(),
            Container::new(self.vstack(S2, composer))
                .with_uniform_padding(S3)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(
                    if self.focus == Some(F_WORK_PROMPT) {
                        p.border_strong
                    } else {
                        p.border
                    },
                ))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
                .finish(),
            self.amoo_usage_footer(),
        ];
        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(S3)
                .with_children(children)
                .finish(),
        )
        .with_horizontal_padding(S5)
        .with_padding_top(S4)
        .with_padding_bottom(S5)
        .finish()
    }

    fn work_workspace_pane(&self) -> El {
        let p = self.p();
        let mut seen = HashSet::new();
        let remote_files: Vec<_> = self
            .work_session()
            .into_iter()
            .flat_map(|session| session.items.iter().enumerate())
            .filter(|(_, item)| match item {
                AmooChatItem::Attachment { url, path, .. } => {
                    let identity = if url.is_empty() { path } else { url };
                    !identity.is_empty() && seen.insert(identity.clone())
                }
                _ => false,
            })
            .collect();
        let mut items = vec![self.between(
            self.work_title(UiIcon::File, "成果预览"),
            if self.work.local_preview.is_some() {
                self.work_button(
                    "preview-close",
                    "关闭文件",
                    WorkAction::ClosePreview,
                    Btn::Ghost,
                )
            } else {
                Empty::new().finish()
            },
        )];
        if !self.work.artifacts.is_empty() || !self.work.files.is_empty() {
            let pills: Vec<El> = self
                .work
                .artifacts
                .iter()
                .enumerate()
                .take(6)
                .map(|(i, artifact)| {
                    let active = self
                        .work
                        .local_preview
                        .as_ref()
                        .is_some_and(|preview| preview.path == artifact.path);
                    let label = artifact
                        .path
                        .file_name()
                        .map(|s| s.to_string_lossy().into_owned())
                        .unwrap_or_else(|| artifact.name.clone());
                    self.work_button(
                        &format!("switcher-artifact-{i}"),
                        &label,
                        WorkAction::PreviewArtifact(i),
                        if active { Btn::Primary } else { Btn::Secondary },
                    )
                })
                .collect();
            if !pills.is_empty() {
                items.push(self.wrap(pills));
            }
        }
        if let Some(path) = &self.work.local_preview_loading {
            items.push(self.panel(vec![
                self.line("正在读取文件…", CAPTION, p.text2),
                self.line(
                    path.file_name().unwrap_or_default().to_string_lossy(),
                    MICRO,
                    p.text3,
                ),
            ]));
        } else if let Some(preview) = &self.work.local_preview {
            let mut preview_actions = Vec::new();
            if preview.image {
                preview_actions.push(self.work_button(
                    "preview-zoom-open",
                    "放大查看",
                    WorkAction::OpenPreview(preview.path.to_string_lossy().into_owned()),
                    Btn::Primary,
                ));
            } else {
                preview_actions.push(
                    self.work_button(
                        "preview-open",
                        if preview
                            .path
                            .extension()
                            .and_then(|e| e.to_str())
                            .is_some_and(|e| {
                                e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm")
                            })
                        {
                            "在浏览器打开"
                        } else {
                            "打开文件"
                        },
                        WorkAction::OpenLocalPreview,
                        Btn::Secondary,
                    ),
                );
                preview_actions.push(self.work_button(
                    "preview-revise",
                    "让 Amoo 修改",
                    WorkAction::ReviseFile(preview.path.clone()),
                    Btn::Primary,
                ));
            }
            preview_actions.push(self.work_button(
                "preview-deliver-phone",
                "发到手机",
                WorkAction::DeliverFileToPhone(preview.path.clone()),
                Btn::Secondary,
            ));
            if !preview.image {
                preview_actions.push(self.work_button(
                    "preview-turn-into-app",
                    "做成应用",
                    WorkAction::TurnIntoApp(preview.path.clone()),
                    Btn::Secondary,
                ));
            } else {
                preview_actions.push(self.work_button(
                    "preview-open",
                    "打开文件",
                    WorkAction::OpenLocalPreview,
                    Btn::Secondary,
                ));
            }
            preview_actions.push(self.work_button(
                "preview-export",
                "另存为",
                WorkAction::ExportLocalPreview,
                Btn::Secondary,
            ));
            if let Some(text) = &preview.text {
                preview_actions.push(self.button(
                    "work-preview-copy-text",
                    "复制内容",
                    Action::CopyText(text.clone(), "已复制文件内容".into()),
                    Btn::Ghost,
                ));
            }
            preview_actions.push(self.button(
                "work-preview-copy-path",
                "复制路径",
                Action::CopyText(preview.path.display().to_string(), "已复制文件路径".into()),
                Btn::Ghost,
            ));
            let mut content = vec![
                self.between(
                    self.line_w(&preview.name, BODY, p.text, Weight::Semibold),
                    self.line(format_bytes(preview.bytes), MICRO, p.text3),
                ),
                self.wrap(preview_actions),
                self.divider(),
            ];
            if preview.image {
                if let Some((w, h)) = cached_image_dimensions(&preview.path) {
                    content.push(self.pill_badge(format!("{w} × {h} px"), p.text2, p.tag_bg));
                }
                content.push(
                    ConstrainedBox::new(
                        Image::new(
                            facloud_ui::assets::asset_cache::AssetSource::LocalFile {
                                path: preview.path.to_string_lossy().into_owned(),
                            },
                            CacheOption::BySize,
                        )
                        .contain()
                        .finish(),
                    )
                    .with_height(216.)
                    .finish(),
                );
            }
            if let Some(text) = &preview.text {
                let key = "work-local-preview-full";
                let expanded = self.amoo_expanded_tool_records.contains(key);
                let cutoff = text.char_indices().nth(4500).map(|(idx, _)| idx);
                let long = cutoff.is_some();
                if preview.markdown {
                    let body = match cutoff {
                        Some(idx) if !expanded => &text[..idx],
                        _ => text.as_str(),
                    };
                    content.push(self.work_rich_text(body, "workspace-preview"));
                } else {
                    let visible = match cutoff {
                        Some(idx) if !expanded => format!("{}\n\n…", &text[..idx]),
                        _ => text.clone(),
                    };
                    content.push(self.text(visible, MICRO, p.text));
                }
                if long {
                    content.push(self.link_muted(
                        "work-preview-show-full",
                        if expanded {
                            "收起正文"
                        } else {
                            "展开完整正文"
                        },
                        Action::AmooToggleToolRecords(key.into()),
                    ));
                }
            }
            content.push(self.text(&preview.note, MICRO, p.text3));
            items.push(self.panel(content));
        } else if let Some(error) = &self.work.local_preview_error {
            items.push(self.panel(vec![
                self.text(error, CAPTION, p.danger),
                self.work_button(
                    "preview-error-close",
                    "关闭预览",
                    WorkAction::ClosePreview,
                    Btn::Ghost,
                ),
            ]));
        } else {
            let remote = self
                .work_session()
                .into_iter()
                .flat_map(|session| session.items.iter().rev())
                .find(|item| match item {
                    AmooChatItem::Attachment { url, path, .. } => self
                        .work
                        .remote_preview_url
                        .as_ref()
                        .is_none_or(|selected| selected == if url.is_empty() { path } else { url }),
                    _ => false,
                });
            if let Some(AmooChatItem::Attachment {
                name,
                kind,
                mime,
                url,
                size_bytes,
                path,
            }) = remote
            {
                let card =
                    self.amoo_phone_attachment_card(name, kind, mime, url, path, *size_bytes);
                let dbl_path = path.clone();
                let dbl_url = url.clone();
                let menu_path = path.clone();
                let menu_url = url.clone();
                items.push(
                    Hoverable::new(self.mouse_state("work-remote-att-card"), move |_| card)
                        .with_defer_events_to_children()
                        .with_cursor(Cursor::PointingHand)
                        .on_double_click(move |ctx, _, _| {
                            if !dbl_path.is_empty() {
                                ctx.dispatch_typed_action(Action::Work(WorkAction::OpenPreview(
                                    dbl_path.clone(),
                                )));
                            } else if !dbl_url.is_empty() {
                                ctx.dispatch_typed_action(Action::StudioOpenUrl(dbl_url.clone()));
                            }
                        })
                        .on_right_click(move |ctx, _, at| {
                            ctx.dispatch_typed_action(Action::ContextOpen(
                                crate::ui::context_menu::Target::ChatAttachment {
                                    path: menu_path.clone(),
                                    url: menu_url.clone(),
                                    remove_idx: None,
                                },
                                at,
                            ));
                        })
                        .finish(),
                );
            } else {
                items.push(self.panel(vec![
                    self.ui_icon(UiIcon::Image, 30., p.text3),
                    self.line_w("把成果放在眼前", HEADING, p.text, Weight::Medium),
                    self.text(
                        "选中交付文件即可预览。图片、Markdown、表格和文档内容会显示在这里。",
                        CAPTION,
                        p.text3,
                    ),
                ]));
            }
        }
        items.push(self.between(
            self.work_title(
                UiIcon::Download,
                format!(
                    "成果文件 · {}",
                    self.work.artifacts.len() + remote_files.len()
                ),
            ),
            self.work_button(
                "files-refresh",
                "刷新",
                WorkAction::RefreshFiles,
                Btn::Ghost,
            ),
        ));
        if self.work.artifacts.is_empty() && remote_files.is_empty() {
            items.push(self.text("任务产生的图片、文档和表格会归集到这里。", MICRO, p.text3));
        }
        for (index, item) in remote_files {
            if let AmooChatItem::Attachment {
                name,
                kind,
                url,
                path,
                size_bytes,
                ..
            } = item
            {
                let identity = if url.is_empty() { path } else { url };
                let selected = self.work.local_preview.is_none()
                    && self
                        .work
                        .remote_preview_url
                        .as_ref()
                        .is_some_and(|value| value == identity);
                let cached = self.work.image_previews.contains_key(identity);
                items.push(self.list_row_with_menu(
                    format!("work-remote-file-{index}"),
                    self.hstack(
                        S2,
                        vec![
                                self.ui_icon(
                                    if kind == "image" {
                                        UiIcon::Image
                                    } else {
                                        UiIcon::File
                                    },
                                    16.,
                                    p.text2,
                                ),
                                Expanded::new(
                                    1.,
                                    self.vstack(
                                        S1,
                                        vec![
                                            self.line_w(name, CAPTION, p.text, Weight::Medium),
                                            self.line(
                                                format!(
                                                    "{}{}",
                                                    if cached {
                                                        "已缓存到电脑"
                                                    } else {
                                                        "手机任务附件"
                                                    },
                                                    size_bytes
                                                        .map(|size| format!(
                                                            " · {}",
                                                            format_bytes(size)
                                                        ))
                                                        .unwrap_or_default()
                                                ),
                                                MICRO,
                                                p.text3,
                                            ),
                                        ],
                                    ),
                                )
                                .finish(),
                                self.ui_icon(UiIcon::ChevronRight, 14., p.text3),
                            ],
                    ),
                    selected,
                    Some(Action::Work(WorkAction::PreviewRemote(index))),
                    Some(if !path.is_empty() {
                        Action::Work(WorkAction::OpenPreview(path.clone()))
                    } else if !url.is_empty() {
                        Action::StudioOpenUrl(url.clone())
                    } else {
                        Action::Work(WorkAction::PreviewRemote(index))
                    }),
                    Some(crate::ui::context_menu::Target::ChatAttachment {
                        path: path.clone(),
                        url: url.clone(),
                        remove_idx: None,
                    }),
                ));
            }
        }
        for (index, artifact) in self.work.artifacts.iter().enumerate() {
            let selected = self
                .work
                .local_preview
                .as_ref()
                .is_some_and(|preview| preview.path == artifact.path);
            items.push(self.list_row_with_menu(
                format!("work-artifact-{index}"),
                self.hstack(
                    S2,
                    vec![
                            self.ui_icon(UiIcon::File, 16., p.text2),
                            Expanded::new(
                                1.,
                                self.vstack(
                                    S1,
                                    vec![
                                        self.line_w(
                                            artifact
                                                .path
                                                .file_name()
                                                .unwrap_or_default()
                                                .to_string_lossy(),
                                            CAPTION,
                                            p.text,
                                            Weight::Medium,
                                        ),
                                        self.line(
                                            format!("本地文件 · {}", artifact.kind),
                                            MICRO,
                                            p.text3,
                                        ),
                                    ],
                                ),
                            )
                            .finish(),
                            self.ui_icon(UiIcon::ChevronRight, 14., p.text3),
                        ],
                ),
                selected,
                Some(Action::Work(WorkAction::PreviewArtifact(index))),
                Some(Action::Work(WorkAction::OpenArtifact(index))),
                Some(crate::ui::context_menu::Target::ChatAttachment {
                    path: artifact.path.display().to_string(),
                    url: String::new(),
                    remove_idx: None,
                }),
            ));
        }
        items.push(self.divider());
        items.push(self.work_title(UiIcon::Folder, "工作资料"));
        if let Some(path) = &self.work.workspace {
            items.push(self.line(path.to_string_lossy(), MICRO, p.text3));
            items.push(self.wrap(vec![
                self.work_button(
                    "workspace-open",
                    "打开文件夹",
                    WorkAction::OpenWorkspace,
                    Btn::Ghost,
                ),
                self.work_button(
                    "workspace-change",
                    "更换",
                    WorkAction::ChooseWorkspace,
                    Btn::Ghost,
                ),
            ]));
            let files_key = "work-workspace-files-expanded";
            let expanded = self.amoo_expanded_tool_records.contains(files_key);
            items.push(self.button_icon(
                "work-workspace-files-toggle",
                format!("浏览资料 · {} 项", self.work.files.len()),
                if expanded {
                    UiIcon::ChevronDown
                } else {
                    UiIcon::ChevronRight
                },
                Action::AmooToggleToolRecords(files_key.into()),
                Btn::Ghost,
            ));
            if expanded && self.work.files.is_empty() {
                items.push(self.text("文件夹为空，可以添加资料开始工作。", MICRO, p.text3));
            }
            for (index, file) in self.work.files.iter().enumerate().filter(|_| expanded) {
                let selected = self
                    .work
                    .local_preview
                    .as_ref()
                    .is_some_and(|preview| preview.path == file.path);
                items.push(self.list_row(
                    format!("work-file-{index}"),
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(
                                if file.is_dir {
                                    UiIcon::Folder
                                } else {
                                    UiIcon::File
                                },
                                14.,
                                p.text2,
                            ),
                            Expanded::new(1., self.line(&file.name, CAPTION, p.text)).finish(),
                            self.line(
                                if file.is_dir {
                                    "文件夹".into()
                                } else {
                                    format_bytes(file.bytes)
                                },
                                MICRO,
                                p.text3,
                            ),
                        ],
                    ),
                    selected,
                    Some(Action::Work(WorkAction::PreviewFile(index))),
                    Some(Action::Work(WorkAction::OpenFile(index))),
                ));
            }
        } else {
            items.push(self.text(
                "选一个文件夹，用电脑处理时会从这里读取资料，结果也保存在这里。",
                CAPTION,
                p.text3,
            ));
            items.push(self.work_button(
                "workspace-choose",
                "选择工作文件夹",
                WorkAction::ChooseWorkspace,
                Btn::Secondary,
            ));
        }
        Container::new(
            self.pane_scroll(
                self.work.files_scroll.clone(),
                Container::new(self.vstack(S3, items))
                    .with_padding_bottom(S5)
                    .finish(),
            ),
        )
        .with_uniform_padding(S4)
        .with_background_color(p.sidebar)
        .with_border(Border::left(1.).with_border_color(p.border))
        .finish()
    }

    fn work_desktop_workflows(&self) -> El {
        let p = self.p();
        let query = self.fields[F_WORK_CAP_SEARCH].text.trim().to_lowercase();
        let matches = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        let mut desktop_workflows = vec![
            self.between(
                self.work_title(UiIcon::Folder, "桌面工作流"),
                if self.work.workspace.is_some() {
                    self.work_button(
                        "import-workflow",
                        "导入工作流",
                        WorkAction::ImportDesktopSkill,
                        Btn::Secondary,
                    )
                } else {
                    self.work_button(
                        "workflow-choose-workspace",
                        "选择工作文件夹",
                        WorkAction::ChooseWorkspace,
                        Btn::Secondary,
                    )
                },
            ),
            self.text(
                "导入 SKILL.md，为文件任务补充处理流程。读取流程说明，不自动运行附带脚本。",
                CAPTION,
                p.text2,
            ),
        ];
        for skill in &self.work.desktop_skills {
            if !matches(&format!("{} {}", skill.name, skill.description)) {
                continue;
            }
            desktop_workflows.push(self.vstack(
                S1,
                vec![
                    self.line_w(&skill.name, BODY, p.text, Weight::Medium),
                    self.text(&skill.description, CAPTION, p.text2),
                    self.line(skill.path.to_string_lossy(), MICRO, p.text3),
                ],
            ));
        }
        if self.work.desktop_skills.is_empty() {
            desktop_workflows.push(self.text("当前工作文件夹尚未导入工作流。", CAPTION, p.text3));
        }
        self.panel(desktop_workflows)
    }

    pub(in super::super) fn work_catalog_pane(&self) -> El {
        let p = self.p();
        let Some(catalog) = &self.work.catalog else {
            let unavailable = self.empty(
                if self.work.catalog_loading {
                    "正在读取手机能力"
                } else {
                    "能力尚未同步"
                },
                self.work
                    .error
                    .as_deref()
                    .unwrap_or("连接手机后同步模型目录、技能、智能体与 AI 应用。"),
                Some(self.work_button(
                    "catalog-refresh",
                    "重新同步",
                    WorkAction::Refresh,
                    Btn::Secondary,
                )),
            );
            return if self.work.tab == 2 {
                self.pane_scroll(
                    self.scroll.clone(),
                    Container::new(
                        self.vstack(S4, vec![self.work_desktop_workflows(), unavailable]),
                    )
                    .with_horizontal_padding(S5)
                    .with_padding_top(S5)
                    .with_padding_bottom(S6)
                    .finish(),
                )
            } else {
                Container::new(unavailable)
                    .with_uniform_padding(S5)
                    .finish()
            };
        };
        let query = self.fields[F_WORK_CAP_SEARCH].text.trim().to_lowercase();
        let matches = |text: &str| query.is_empty() || text.to_lowercase().contains(&query);
        let availability = match self.work.tab {
            1 => &catalog.models_status,
            2 => &catalog.skills_status,
            3 => &catalog.agents_status,
            _ => &catalog.ai_apps_status,
        };
        let search = self.input(F_WORK_CAP_SEARCH, "搜索名称、说明或标识…", None);
        let mut rows = vec![if self.work.tab == 2 {
            self.between(
                search,
                if self.busy {
                    self.disabled_button("正在搜索")
                } else {
                    self.work_button(
                        "search-skill-market",
                        "搜索市场",
                        WorkAction::SearchSkills,
                        Btn::Secondary,
                    )
                },
            )
        } else {
            search
        }];
        rows.push(self.text(
            if self.work.tab == 1 {
                "来源：手机 Amoo 云端。只展示当前账号通过真实请求验证的模型；验证结果有效期为 30 分钟。"
            } else {
                "来源：手机 OpenClaw。技能与智能体属于手机运行环境，和 Amoo 云端会话能力分别展示。"
            },
            CAPTION, p.text3,
        ));
        if !availability.available || !availability.message.is_empty() {
            rows.push(self.panel(vec![self.text(&availability.message, BODY, p.text2)]));
        }
        if !self.work.status.is_empty() {
            rows.push(self.text(&self.work.status, CAPTION, p.text3));
        }
        if let Some(error) = &self.work.error {
            rows.push(self.panel(vec![self.text(error, BODY, p.danger)]));
        }
        let mut count = 0;
        match self.work.tab {
            1 => {
                rows.push(self.panel(vec![self.between(
                    self.vstack(
                        S1,
                        vec![
                            self.heading("当前账号的模型权限"),
                            self.text(
                                "验证会发送一条短测试请求，Token 计入手机账号用量。",
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                    if self.busy || self.work.running {
                        self.disabled_button("处理中")
                    } else {
                        self.work_button(
                            "verify-models",
                            "验证可用模型",
                            WorkAction::VerifyModels,
                            Btn::Secondary,
                        )
                    },
                )]));
                rows.push(self.panel(vec![self.between(
                    self.vstack(
                        S1,
                        vec![
                            self.heading("跟随手机默认"),
                            self.text(
                                if catalog.can_select_model {
                                    "由手机 Amoo 自动按任务选择模型，也可在下方手动指定已验证模型。"
                                } else {
                                    "由手机 Amoo 自动选择；下方显示实际调用记录，当前不支持手动指定。"
                                },
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ),
                    self.work_button(
                        "model-default",
                        if self.work.selected_model.is_none() {
                            "已选择"
                        } else {
                            "使用默认"
                        },
                        WorkAction::SelectModel(None),
                        Btn::Secondary,
                    ),
                )]));
                let mut models: Vec<_> = catalog
                    .models
                    .iter()
                    .filter(|model| {
                        model.is_verified_for(self.amoo_ui.account_scope.as_deref())
                            && matches(&format!("{} {} {}", model.name, model.id, model.provider))
                    })
                    .collect();
                models.sort_by(|a, b| b.available.cmp(&a.available).then(a.name.cmp(&b.name)));
                let total = models.len();
                let last_page = total.saturating_sub(1) / 40;
                let page = self.work.catalog_page.min(last_page);
                if total > 40 {
                    rows.push(self.between(
                        self.line(
                            format!(
                                "{} 个已验证模型 · 第 {} / {} 页",
                                total,
                                page + 1,
                                last_page + 1
                            ),
                            CAPTION,
                            p.text2,
                        ),
                        self.wrap(vec![
                            if page > 0 {
                                self.work_button(
                                    "models-prev",
                                    "上一页",
                                    WorkAction::CatalogPage(page - 1),
                                    Btn::Secondary,
                                )
                            } else {
                                self.disabled_button("上一页")
                            },
                            if page < last_page {
                                self.work_button(
                                    "models-next",
                                    "下一页",
                                    WorkAction::CatalogPage(page + 1),
                                    Btn::Secondary,
                                )
                            } else {
                                self.disabled_button("下一页")
                            },
                        ]),
                    ));
                } else {
                    rows.push(self.line(
                        format!("{total} 个通过当前账号验证的模型"),
                        CAPTION,
                        p.text2,
                    ));
                }
                for model in models.into_iter().skip(page * 40).take(40) {
                    count += 1;
                    let selected = self.work.selected_model.as_deref() == Some(model.id.as_str());
                    rows.push(self.panel(vec![
                        self.between(
                            self.work_title(UiIcon::Cpu, &model.name),
                            if catalog.can_select_model
                                && model.is_verified_for(self.amoo_ui.account_scope.as_deref())
                            {
                                self.work_button(
                                    &format!("model-{}", model.id),
                                    if selected {
                                        "已选择"
                                    } else {
                                        "使用模型"
                                    },
                                    WorkAction::SelectModel(Some(model.id.clone())),
                                    Btn::Secondary,
                                )
                            } else {
                                self.tag("手机自动选择", p.text3)
                            },
                        ),
                        self.mono(&model.id, CAPTION, p.text2),
                        self.wrap(vec![
                            self.tag(&model.provider, p.text3),
                            self.tag(
                                if model.current {
                                    "最近验证通过"
                                } else if model.available {
                                    "已验证可用"
                                } else {
                                    "目录模型"
                                },
                                p.text3,
                            ),
                        ]),
                        self.text(
                            if model.capabilities.is_empty() {
                                "手机未提供能力标签".into()
                            } else {
                                model.capabilities.join(" · ")
                            },
                            CAPTION,
                            p.text3,
                        ),
                        self.text(&model.availability_reason, CAPTION, p.text3),
                    ]));
                }
            }
            2 => {
                rows.push(self.work_desktop_workflows());
                for (market, skills) in [(false, &catalog.skills), (true, &self.work.market_skills)]
                {
                    if market {
                        rows.push(self.work_title(UiIcon::Search, "手机技能市场"));
                        rows.push(self.text("在上方输入关键词，再搜索手机 OpenClaw 的 ClawHub 技能目录。安装前会显示目标并确认。", CAPTION, p.text3));
                        if self.work.market_skills.is_empty() {
                            rows.push(self.text("尚无市场搜索结果。", CAPTION, p.text3));
                        }
                    } else {
                        rows.push(self.work_title(UiIcon::Smartphone, "已安装手机技能"));
                    }
                    for skill in skills {
                        if !market
                            && !matches(&format!(
                                "{} {} {}",
                                skill.name, skill.id, skill.description
                            ))
                        {
                            continue;
                        }
                        count += 1;
                        let mut actions = Vec::new();
                        if skill.installed {
                            actions.push(self.work_button(
                                &format!("skill-toggle-{}", skill.id),
                                if skill.enabled { "停用" } else { "启用" },
                                WorkAction::ToggleSkill {
                                    key: skill.id.clone(),
                                    enabled: !skill.enabled,
                                },
                                Btn::Secondary,
                            ));
                            actions.push(if self.busy || self.work.running {
                                self.disabled_button("用于桌面任务")
                            } else {
                                self.work_button(
                                    &format!("skill-use-{}", skill.id),
                                    "用于桌面任务",
                                    WorkAction::UsePhoneSkill(skill.id.clone()),
                                    Btn::Primary,
                                )
                            });
                        } else {
                            for installer in &skill.installers {
                                actions.push(self.work_button(
                                    &format!("skill-install-{}-{}", skill.id, installer.id),
                                    &format!("安装 · {}", installer.label),
                                    WorkAction::InstallSkill {
                                        key: skill.id.clone(),
                                        install_id: installer.id.clone(),
                                    },
                                    Btn::Secondary,
                                ));
                            }
                        }
                        let description_key = format!("work-skill-description-{}", skill.id);
                        let description_expanded =
                            self.amoo_expanded_tool_records.contains(&description_key);
                        let long_description = skill.description.chars().count() > 180;
                        let description = if long_description && !description_expanded {
                            format!(
                                "{}…",
                                skill.description.chars().take(180).collect::<String>()
                            )
                        } else {
                            skill.description.clone()
                        };
                        let mut details = vec![
                            self.between(
                                self.work_title(UiIcon::Tools, &skill.name),
                                self.tag(
                                    if skill.installed {
                                        if skill.enabled {
                                            "已启用"
                                        } else {
                                            "已停用"
                                        }
                                    } else {
                                        "未安装"
                                    },
                                    p.text2,
                                ),
                            ),
                            self.text(description, BODY, p.text2),
                            self.mono(&skill.id, MICRO, p.text3),
                            self.line(format!("来源：{}", skill.source), MICRO, p.text3),
                            if actions.is_empty() {
                                self.text("手机未提供可用安装方式", CAPTION, p.text3)
                            } else {
                                self.wrap(actions)
                            },
                        ];
                        if long_description {
                            details.push(self.button(
                                format!("{description_key}-toggle"),
                                if description_expanded {
                                    "收起说明"
                                } else {
                                    "展开说明"
                                },
                                Action::AmooToggleToolRecords(description_key),
                                Btn::Ghost,
                            ));
                        }
                        if let Some((key, installer)) = self
                            .work
                            .pending_install
                            .as_ref()
                            .filter(|(key, _)| *key == skill.id)
                        {
                            details.push(self.confirm_bar(
                                format!(
                                    "将通过 {installer} 在手机 OpenClaw 安装 {}。来源：{} · {}",
                                    skill.name,
                                    skill.source,
                                    if skill.install_reference.is_empty() {
                                        &skill.id
                                    } else {
                                        &skill.install_reference
                                    }
                                ),
                                false,
                                vec![
                                    self.work_button(
                                        &format!("skill-cancel-{key}"),
                                        "取消",
                                        WorkAction::CancelSkillInstall,
                                        Btn::Secondary,
                                    ),
                                    self.work_button(
                                        &format!("skill-confirm-{key}"),
                                        "确认安装",
                                        WorkAction::ConfirmSkillInstall,
                                        Btn::Primary,
                                    ),
                                ],
                            ));
                        }
                        rows.push(self.panel(details));
                    }
                }
                let key = "amoo-work-phone-tool-catalog";
                let expanded = self.amoo_expanded_tool_records.contains(key) || !query.is_empty();
                let tools: Vec<_> = catalog
                    .tools
                    .iter()
                    .filter(|tool| matches(&format!("{} {} {}", tool.id, tool.name, tool.group)))
                    .collect();
                let mut tool_rows = vec![
                    self.button_icon(
                        "work-phone-tools",
                        format!("手机工具目录 · {} 项", tools.len()),
                        if expanded {
                            UiIcon::ChevronDown
                        } else {
                            UiIcon::ChevronRight
                        },
                        Action::AmooToggleToolRecords(key.into()),
                        Btn::Ghost,
                    ),
                    self.text(&catalog.tools_status.message, CAPTION, p.text3),
                ];
                if expanded {
                    for tool in tools {
                        tool_rows.push(self.vstack(
                            S1,
                            vec![
                                self.line_w(&tool.name, BODY, p.text, Weight::Medium),
                                self.mono(&tool.id, MICRO, p.text2),
                                self.line(
                                    format!("{} · {}", tool.group, tool.source),
                                    MICRO,
                                    p.text3,
                                ),
                            ],
                        ));
                    }
                }
                rows.push(self.panel(tool_rows));
            }
            3 => {
                rows.push(self.panel(vec![
                    self.work_title(UiIcon::Plus, "新建手机智能体"),
                    self.text(
                        "新建在手机 OpenClaw 工作空间。当前 Amoo 云端会话仍使用原有通道。",
                        CAPTION,
                        p.text2,
                    ),
                    self.between(
                        self.input(F_WORK_AGENT_NAME, "输入智能体名称…", None),
                        if self.busy || !catalog.agents_status.available {
                            self.disabled_button("新建智能体")
                        } else {
                            self.work_button(
                                "create-agent",
                                "新建智能体",
                                WorkAction::CreateAgent,
                                Btn::Primary,
                            )
                        },
                    ),
                ]));
                for agent in &catalog.agents {
                    if !matches(&format!(
                        "{} {} {}",
                        agent.name, agent.id, agent.description
                    )) {
                        continue;
                    }
                    count += 1;
                    rows.push(self.panel(vec![
                        self.between(
                            self.work_title(UiIcon::Star, &agent.name),
                            self.work_button(
                                &format!("agent-{}", agent.id),
                                "查看信息",
                                WorkAction::SelectAgent(agent.id.clone()),
                                Btn::Secondary,
                            ),
                        ),
                        self.text(&agent.description, BODY, p.text2),
                        self.mono(&agent.id, MICRO, p.text3),
                        self.line(
                            if agent.model.is_empty() {
                                "使用智能体默认模型"
                            } else {
                                &agent.model
                            },
                            CAPTION,
                            p.text3,
                        ),
                    ]));
                }
            }
            _ => {
                rows.push(self.panel(vec![
                    self.work_title(UiIcon::Apps, "安装与部署"),
                    self.text(
                        "安装本地 APK 到手机，或在「我的作品」里管理做过的应用。",
                        BODY,
                        p.text2,
                    ),
                    self.wrap(vec![
                        self.button(
                            "work-app-install",
                            "安装 APK",
                            Action::InstallApk,
                            Btn::Secondary,
                        ),
                        self.button(
                            "work-app-list",
                            "查看已安装应用",
                            Action::Page(Page::Apps),
                            Btn::Secondary,
                        ),
                        self.button(
                            "work-app-studio",
                            "打开我的作品",
                            Action::Page(Page::Studio),
                            Btn::Secondary,
                        ),
                    ]),
                ]));
                for app in &catalog.ai_apps {
                    if !matches(&format!("{} {} {}", app.name, app.id, app.description)) {
                        continue;
                    }
                    count += 1;
                    rows.push(self.panel(vec![
                        self.between(
                            self.work_title(UiIcon::Apps, &app.name),
                            self.tag(
                                if app.installed {
                                    "手机已安装"
                                } else {
                                    "未安装"
                                },
                                p.text2,
                            ),
                        ),
                        self.text(&app.description, BODY, p.text2),
                        self.mono(&app.id, MICRO, p.text3),
                    ]));
                }
            }
        }
        if count == 0 && availability.available {
            rows.push(self.text(
                if query.is_empty() {
                    "手机当前未返回此类能力。"
                } else {
                    "没有匹配的结果。"
                },
                BODY,
                p.text3,
            ));
        }
        self.pane_scroll(
            self.scroll.clone(),
            Container::new(self.vstack(S3, rows))
                .with_horizontal_padding(S5)
                .with_padding_top(S5)
                .with_padding_bottom(S6)
                .finish(),
        )
    }

    pub(in super::super) fn work_page(&self) -> El {
        let p = self.p();
        let capabilities = self.work.capabilities_open;
        // 文件处理页只是次要入口：不再重复「对话 / 交付 / 工坊」三选一切换栏，
        // 只留一个返回唯一 AI 入口的按钮。
        let workbench_bar = self.hstack(
            S2,
            vec![
                self.button_icon(
                    "work-back-amoo",
                    "返回 Amoo",
                    UiIcon::ArrowLeft,
                    Action::Page(Page::Amoo),
                    Btn::Ghost,
                ),
                self.line_w("文件处理", BODY, p.text, Weight::Semibold),
            ],
        );
        let header = Container::new(self.vstack(
            S2,
            vec![
                self.between(workbench_bar, {
                    let phone_connected = self.selected.is_some() || self.amoo_gateway_online;
                    let local_ready = self.studio_lab.config.has_key
                        && !self.studio_lab.config.endpoint.trim().is_empty()
                        && !self.studio_lab.config.model.trim().is_empty();
                    let status_label = if let Some(d) = &self.selected {
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
                    } else if self.work.catalog_loading {
                        "正在同步".to_string()
                    } else if local_ready {
                        "自己的模型服务已就绪".to_string()
                    } else {
                        "手机未连接".to_string()
                    };
                    self.hstack(
                        S2,
                        vec![
                            self.dot(if phone_connected || local_ready {
                                p.success
                            } else {
                                p.text3
                            }),
                            self.line(
                                status_label,
                                CAPTION,
                                if phone_connected || local_ready {
                                    p.success
                                } else {
                                    p.text2
                                },
                            ),
                        ],
                    )
                }),
                if capabilities {
                    self.hstack(
                        S2,
                        vec![
                            self.button_icon(
                                "work-back-work",
                                "返回文件处理",
                                UiIcon::ArrowLeft,
                                Action::Work(WorkAction::Capabilities),
                                Btn::Secondary,
                            ),
                            self.segmented(
                                "work-capability-tabs",
                                ["模型", "技能", "智能体", "AI 应用"]
                                    .iter()
                                    .enumerate()
                                    .map(|(index, label)| {
                                        (
                                            (*label).to_string(),
                                            self.work.tab == index as u8 + 1,
                                            Action::Work(WorkAction::Tab(index as u8 + 1)),
                                        )
                                    })
                                    .collect(),
                            ),
                        ],
                    )
                } else {
                    self.hstack(
                        S2,
                        vec![
                            self.work_button(
                                "toggle-sessions",
                                if self.work.panel == 1 {
                                    "收起记录"
                                } else {
                                    "处理记录"
                                },
                                WorkAction::Panel(1),
                                if self.work.panel == 1 {
                                    Btn::Secondary
                                } else {
                                    Btn::Ghost
                                },
                            ),
                            self.work_button(
                                "toggle-files",
                                if self.work.panel == 2 {
                                    "收起预览"
                                } else {
                                    "结果与预览"
                                },
                                WorkAction::Panel(2),
                                if self.work.panel == 2 {
                                    Btn::Secondary
                                } else {
                                    Btn::Ghost
                                },
                            ),
                            self.work_button(
                                "capabilities",
                                "技能与模型",
                                WorkAction::Capabilities,
                                Btn::Ghost,
                            ),
                        ],
                    )
                },
            ],
        ))
        .with_horizontal_padding(S5)
        .with_vertical_padding(S3)
        .with_border(Border::bottom(1.).with_border_color(p.border))
        .finish();
        let body = if capabilities {
            self.work_catalog_pane()
        } else {
            let mut panes = Vec::new();
            if self.work.panel == 1 {
                panes.push(self.fixed_w(236., self.work_session_pane()));
            }
            panes.push(Expanded::new(1., self.work_conversation_pane()).finish());
            if self.work.panel == 2 {
                panes.push(self.fixed_w(356., self.work_workspace_pane()));
            }
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_children(panes)
                .finish()
        };
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(Expanded::new(1., self.sub_enter(body)).finish())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::work_message_text;

    #[test]
    fn desktop_protocol_preserves_task_and_hides_tool_feedback() {
        let task = "<amoo_work_protocol>\n[Amoo Work 桌面协作]\n用户任务：请整理会议纪要\n保留负责人\n用户已选择一个本地工作目录。附件相对路径：[]\n内部工具说明\n</amoo_work_protocol>";
        assert_eq!(
            work_message_text(task, true).as_deref(),
            Some("请整理会议纪要\n保留负责人")
        );
        assert!(work_message_text(
            "<amoo_work_protocol>\nAmoo Work 桌面执行结果（第 1 轮）\n{}\n</amoo_work_protocol>",
            true
        )
        .is_none());
    }

    #[test]
    fn desktop_plan_displays_real_summary_without_protocol() {
        let text = "<amoo_work_protocol>\n{\"amoo_work\":1,\"summary\":\"已生成会议纪要\",\"done\":true,\"tools\":[]}\n</amoo_work_protocol>";
        assert_eq!(
            work_message_text(text, false).as_deref(),
            Some("已生成会议纪要")
        );
        assert_eq!(
            work_message_text("{\"amoo_work\":1,\"summary\":\"正在读取资料\"}", false).as_deref(),
            Some("正在读取资料")
        );
    }

    #[test]
    fn ordinary_messages_and_json_are_not_filtered() {
        let plain = "请解释 amoo_work 字段的意义";
        assert_eq!(work_message_text(plain, true).as_deref(), Some(plain));
        let json = "{\"summary\":\"普通 JSON 示例\"}";
        assert_eq!(work_message_text(json, false).as_deref(), Some(json));
    }
}
