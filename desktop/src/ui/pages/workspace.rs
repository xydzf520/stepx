//! Amoo: one task flow, contextual resources and a dismissible result preview.
use super::super::workspace_state::WorkspaceAction as W;
use super::super::{icons::UiIcon, theme::*, widgets::*, *};
use facloud_ui::{assets::asset_cache::AssetSource, fonts::Weight};
use stepx_companion::amoo_workspace::{Executor, Intent, Output, Phase, Task};

impl Companion {
    fn workspace_column(&self, content: El, compact: bool, id: &str) -> El {
        Align::new(
            ConstrainedBox::new(
                Container::new(SavePosition::new(content, id).for_single_frame().finish())
                    .with_horizontal_padding(if compact { 12. } else { 24. })
                    .finish(),
            )
            .with_max_width(928.)
            .finish(),
        )
        .top_center()
        .finish()
    }
    fn ws_button(&self, id: &str, label: &str, icon: UiIcon, action: W, kind: Btn) -> El {
        self.button_icon(
            format!("workspace-{id}"),
            label,
            icon,
            Action::Workspace(action),
            kind,
        )
    }
    pub(in super::super) fn workspace_actions(&self) -> Vec<El> {
        if self.amoo_ui.screen_mode {
            return vec![self.button_icon(
                "workspace-back-gui",
                "返回对话",
                UiIcon::ArrowLeft,
                Action::AmooUi(AmooUiAction::GuiMode(false)),
                Btn::Secondary,
            )];
        }
        // Only "新对话" stays visible; the rest sits behind "…". A panel that
        // is open keeps its button visible so it can be closed again.
        let more_open = self.amoo_expanded_tool_records.contains(HEADER_MORE_KEY);
        let toggle = Action::AmooToggleToolRecords(HEADER_MORE_KEY.into());
        let side = !self.workspace.history;
        let entries = [
            (
                "history",
                "会话记录",
                UiIcon::Clock,
                W::History,
                self.workspace.history,
            ),
            (
                "memory",
                "个人记忆",
                UiIcon::Star,
                W::Menu(9),
                side && matches!(self.workspace.panel, 7 | 8 | 9),
            ),
            (
                "resources",
                "对话附件",
                UiIcon::Attach,
                W::Menu(2),
                side && self.workspace.panel == 2,
            ),
            (
                "model",
                "模型与账号",
                UiIcon::Cpu,
                W::Menu(1),
                side && matches!(self.workspace.panel, 1 | 5),
            ),
        ];
        let mut items = Vec::new();
        for (id, label, icon, action, active) in entries {
            if !more_open && !active {
                continue;
            }
            let mut actions = Vec::new();
            if more_open {
                actions.push(toggle.clone());
            }
            actions.push(Action::Workspace(action));
            items.push(self.chip(
                format!("workspace-{id}"),
                label,
                Some(icon),
                actions,
                active,
                false,
            ));
        }
        items.push(self.button_icon(
            "workspace-open-gui",
            "操作手机",
            UiIcon::Smartphone,
            Action::AmooUi(AmooUiAction::GuiMode(true)),
            Btn::Secondary,
        ));
        items.push(self.icon_action(
            "workspace-more",
            if more_open { "收起" } else { "更多" },
            UiIcon::Menu,
            vec![toggle],
            more_open,
        ));
        items.push(self.ws_button("new", "新对话", UiIcon::Plus, W::New, Btn::Primary));
        items
    }
    pub(in super::super) fn workspace_page(&self, compact: bool) -> El {
        let p = self.p();
        if (compact || self.workspace.viewport.x() < 1080.)
            && (self.workspace.panel != 0 || self.workspace.history)
        {
            return self.workspace_side(compact);
        }
        if !compact && self.amoo_ui.screen_mode {
            return self.amoo_page();
        }
        if let Some(album) = self.amoo_album_picker() {
            return self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(album)
                    .with_horizontal_padding(S5)
                    .with_padding_top(S5)
                    .with_padding_bottom(S6)
                    .finish(),
            );
        }
        if self.amoo_show_schedules {
            return self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(self.amoo_schedule_center())
                    .with_horizontal_padding(S5)
                    .with_padding_top(S5)
                    .with_padding_bottom(S6)
                    .finish(),
            );
        }
        let active = self.workspace_task();
        let started = active
            .is_some_and(|t| !t.turns.is_empty() || t.session.is_some() || !t.outputs.is_empty())
            || self.workspace.proposal.is_some()
            || self.amoo_ui.gui_running
            || self.amoo_ui.gui_paused;
        let mut column = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
        let title = if started {
            active
                .map(|t| amoo_display::session_title(&t.title))
                .unwrap_or_else(|| "新对话".into())
        } else {
            "新对话 · Amoo 智能助手".into()
        };
        column = column.with_child(self.workspace_thread_bar(&title, compact));
        if started {
            let content = self.workspace_thread();
            column = column.with_child(
                Expanded::new(
                    1.,
                    self.chat_scroll(
                        self.workspace_column(
                            Container::new(self.selectable_messages(content))
                                .with_padding_bottom(S4)
                                .finish(),
                            compact,
                            "workspace-chat-messages",
                        ),
                    ),
                )
                .finish(),
            );
            column = column.with_child(
                Container::new(self.workspace_column(
                    self.amoo_message_composer(compact, false),
                    compact,
                    "workspace-chat-input",
                ))
                .with_vertical_padding(if compact && self.workspace.viewport.y() < 520. {
                    S2
                } else {
                    S3
                })
                .with_border(Border::top(1.).with_border_color(p.border))
                .finish(),
            );
        } else {
            let welcome = self.workspace_welcome(compact);
            column = column
                .with_child(Expanded::new(1., self.chat_pane_scroll(welcome, false)).finish());
        }
        let main = column.finish();
        let show_left_rail = !super::super::workspace_state::is_verification_harness()
            && !compact
            && self.workspace.viewport.x() >= 1000.0
            && self.workspace.left_rail_open
            && !self.workspace.history;
        let show_right_side = !compact
            && self.workspace.viewport.x() >= 1080.
            && (self.workspace.panel != 0 || self.workspace.history);
        if show_left_rail || show_right_side {
            let mut row = Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch);
            if show_left_rail {
                row = row.with_child(self.workspace_left_sidebar());
            }
            row = row.with_child(Expanded::new(1., main).finish());
            if show_right_side {
                row = row.with_child(self.fixed_w(352., self.workspace_side(false)));
            }
            row.finish()
        } else {
            main
        }
    }
    fn workspace_thread_bar(&self, title: &str, compact: bool) -> El {
        let p = self.p();
        let right = if compact {
            self.workspace_connection_badge()
        } else {
            self.hstack(
                S2,
                vec![
                    self.workspace_connection_badge(),
                    self.chip(
                        "workspace-thread-open-gui",
                        "操作手机",
                        Some(UiIcon::Smartphone),
                        vec![Action::AmooUi(AmooUiAction::GuiMode(true))],
                        self.amoo_ui.screen_mode,
                        false,
                    ),
                    self.chip(
                        "workspace-thread-model",
                        "模型与设置",
                        Some(UiIcon::Cpu),
                        vec![Action::Workspace(W::Menu(1))],
                        !self.workspace.history && matches!(self.workspace.panel, 1 | 5),
                        false,
                    ),
                ],
            )
        };
        self.workspace_column(
            Container::new(self.between(
                self.line_w(
                    Self::truncate_chars(title, if compact { 20 } else { 32 }),
                    SUBHEAD,
                    p.text,
                    Weight::Semibold,
                ),
                right,
            ))
            .with_vertical_padding(if compact && self.workspace.viewport.y() < 520. {
                S2
            } else {
                S3
            })
            .finish(),
            compact,
            "workspace-chat-heading",
        )
    }
    fn workspace_welcome(&self, compact: bool) -> El {
        let p = self.p();
        let mut content = vec![
            self.vstack(
                S2,
                vec![
                    Align::new(self.line_w(
                        "你好，需要我做什么？",
                        if compact { 24. } else { 30. },
                        p.text,
                        Weight::Semibold,
                    ))
                    .finish(),
                    Align::new(self.text(
                        "直接输入需求按 Enter 发送，或点击下方场景卡片自动填入示例（支持手机对话、生成 Office 文档、制作小游戏、实时操控手机与背屏点阵）",
                        if compact { CAPTION } else { BODY },
                        p.text2,
                    ))
                    .finish(),
                ],
            ),
            self.amoo_message_composer(compact, false),
        ];
        if !super::super::workspace_state::is_verification_harness() {
            content.push(self.workspace_welcome_suggestions(compact));
        }
        if let Some(error) = &self.workspace.error {
            content.push(self.workspace_error_banner(error));
        }
        // Anchor against the viewport, not the draft's changing measured height.
        // Reserve enough growth room so adding attachments never shifts the field.
        let available = self.workspace.viewport.y() - if compact { 164. } else { 124. };
        let intro = if compact { 80. } else { 88. };
        let (editor, attachments) = self.composer_limits(compact);
        let minimum = if compact { 114. } else { 134. };
        let maximum = editor + attachments + 98.;
        let top = ((available - intro - minimum) / 2.)
            .min(available - intro - maximum - 16.)
            .max(16.);
        self.workspace_column(
            Container::new(self.vstack(S4, content))
                .with_padding_top(top)
                .with_padding_bottom(16.)
                .finish(),
            compact,
            "workspace-chat-welcome",
        )
    }
    fn workspace_welcome_suggestions(&self, compact: bool) -> El {
        let p = self.p();
        let draft_trimmed = self.fields[F_AMOO].text.trim();
        let scenarios: [(u8, &str, &str, UiIcon, &str); 5] = [
            (
                2,
                "suggest-ppt",
                "做个 PPT",
                UiIcon::Chart,
                "请根据以下主题制作一份结构完整的汇报 PPT",
            ),
            (
                3,
                "suggest-doc",
                "写份报告",
                UiIcon::File,
                "请帮我写一份分析报告并生成 Word 文档",
            ),
            (
                4,
                "suggest-sheet",
                "做个表格",
                UiIcon::Clipboard,
                "请帮我设计并生成一份带汇总公式的 Excel 表格",
            ),
            (
                0,
                "suggest-game",
                "做小游戏",
                UiIcon::Play,
                "帮我做一个可在手机运行的 2048 或贪吃蛇小游戏",
            ),
            (
                5,
                "suggest-screen",
                "背屏图案",
                UiIcon::Sparkles,
                "帮我设计一个手机背屏点阵图案并发送到背屏",
            ),
        ];
        let mut chips: Vec<El> = scenarios
            .into_iter()
            .map(|(scenario_id, id, label, icon, prefix)| {
                let active = !draft_trimmed.is_empty() && draft_trimmed.starts_with(prefix);
                self.chip(
                    format!("workspace-{id}"),
                    label,
                    Some(icon),
                    vec![Action::Workspace(W::QuickScenario(scenario_id))],
                    active,
                    false,
                )
            })
            .collect();
        chips.push(self.chip(
            "workspace-suggest-phone",
            "操作手机",
            Some(UiIcon::Smartphone),
            vec![Action::AmooUi(AmooUiAction::GuiMode(true))],
            self.amoo_ui.screen_mode,
            false,
        ));
        let quick_row = Align::new(self.wrap(chips)).finish();
        if compact {
            return quick_row;
        }
        let guides: [(&str, UiIcon, &str, &str, Action); 5] = [
            (
                "guide-chat",
                UiIcon::Chat,
                "💬 手机对话与查信息",
                "直接提问或查天气、日程、手机信息",
                Action::Workspace(W::Intent(Intent::Chat)),
            ),
            (
                "guide-files",
                UiIcon::File,
                "📄 生成 PPT / Word / 表格",
                "输入主题自动生成本机可编辑文档",
                Action::Workspace(W::QuickScenario(2)),
            ),
            (
                "guide-app",
                UiIcon::Play,
                "🎮 制作小游戏 / 网页应用",
                "一句话生成可试玩、可发手机的应用",
                Action::Workspace(W::QuickScenario(0)),
            ),
            (
                "guide-gui",
                UiIcon::Smartphone,
                "📱 实时操作手机屏幕",
                "投屏接管手机，AI 自动点击滑动",
                Action::AmooUi(AmooUiAction::GuiMode(true)),
            ),
            (
                "guide-screen",
                UiIcon::Sparkles,
                "✨ 手机背屏像素点阵",
                "输入图案描述生成并推送到背屏",
                Action::Workspace(W::QuickScenario(5)),
            ),
        ];
        let guide_cards: Vec<El> = guides
            .into_iter()
            .map(|(id, icon, title, desc, action)| {
                let clean_title = title.split_once(' ').map(|(_, r)| r).unwrap_or(title);
                let card = self.hover_panel_interactive(
                    format!("workspace-{id}"),
                    self.vstack(
                        S1,
                        vec![
                            self.hstack(
                                S1 + 2.,
                                vec![
                                    self.ui_icon(icon, 15., p.accent),
                                    Expanded::new(
                                        1.,
                                        self.line_w(clean_title, CAPTION, p.text, Weight::Semibold),
                                    )
                                    .finish(),
                                ],
                            ),
                            self.line(desc, MICRO, p.text2),
                        ],
                    ),
                    Some(action),
                    None,
                    None,
                );
                ConstrainedBox::new(card).with_width(168.).finish()
            })
            .collect();
        self.vstack(
            S3,
            vec![
                quick_row,
                self.vstack(
                    S2,
                    vec![
                        Align::new(self.line_w(
                            "Amoo 5 大用法速览",
                            MICRO,
                            p.text3,
                            Weight::Semibold,
                        ))
                        .finish(),
                        Align::new(self.wrap(guide_cards)).finish(),
                    ],
                ),
            ],
        )
    }
    fn workspace_error_banner(&self, error: &str) -> Box<dyn Element> {
        let p = self.p();
        Container::new(self.hstack(
            S2,
            vec![
                self.ui_icon(UiIcon::Info, 16., p.danger),
                Expanded::new(1., self.text(error, CAPTION, p.danger)).finish(),
                self.chip(
                    "workspace-dismiss-error",
                    "知道了 ×",
                    Some(UiIcon::Close),
                    vec![Action::Workspace(W::DismissError)],
                    false,
                    false,
                ),
            ],
        ))
        .with_horizontal_padding(S3)
        .with_vertical_padding(S2)
        .with_background_color(p.danger_soft)
        .with_border(Border::all(1.).with_border_color(lerp_color(p.border, p.danger, 0.35)))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish()
    }
    fn workspace_connection_badge(&self) -> El {
        let p = self.p();
        let online = self.selected.is_some() || self.amoo_gateway_online || self.demo_mode;
        let color = if online { p.success } else { p.text3 };
        self.hstack(
            S1 + 2.,
            vec![
                self.dot(color),
                self.line(
                    if online {
                        "手机已连接"
                    } else {
                        "手机未连接"
                    },
                    CAPTION,
                    color,
                ),
            ],
        )
    }
    fn workspace_left_sidebar(&self) -> El {
        let p = self.p();
        let header = self.between(
            self.heading("对话历史"),
            self.button_icon(
                "workspace-left-sync",
                "同步",
                UiIcon::Refresh,
                Action::AmooSnapshot,
                Btn::Ghost,
            ),
        );
        let search = self.input(F_AMOO_SEARCH, "搜索对话记录", None);
        let list = Expanded::new(
            1.,
            self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(self.vstack(S2, self.workspace_recent(80)))
                    .with_padding_bottom(S3)
                    .finish(),
            ),
        )
        .finish();
        let footer = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(S1)
                .with_child(
                    Expanded::new(
                        1.,
                        self.ws_button(
                            "left-model",
                            "模型与账号",
                            UiIcon::Cpu,
                            W::Menu(1),
                            if matches!(self.workspace.panel, 1 | 5) {
                                Btn::Secondary
                            } else {
                                Btn::Ghost
                            },
                        ),
                    )
                    .finish(),
                )
                .with_child(
                    Expanded::new(
                        1.,
                        self.button_icon(
                            "workspace-left-gui",
                            "手机操控",
                            UiIcon::Smartphone,
                            Action::AmooUi(AmooUiAction::GuiMode(true)),
                            Btn::Ghost,
                        ),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_padding_top(S2)
        .with_border(Border::top(1.).with_border_color(p.border))
        .finish();
        self.fixed_w(
            248.,
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_spacing(S3)
                    .with_child(header)
                    .with_child(search)
                    .with_child(list)
                    .with_child(footer)
                    .finish(),
            )
            .with_horizontal_padding(S2)
            .with_vertical_padding(S3)
            .with_background_color(p.sidebar)
            .with_border(Border::right(1.).with_border_color(p.border))
            .finish(),
        )
    }
    fn workspace_recent(&self, max: usize) -> Vec<El> {
        let p = self.p();
        let q = self.fields[F_AMOO_SEARCH].text.trim().to_lowercase();
        let mut tasks = self
            .workspace
            .journal
            .tasks
            .iter()
            .filter(|t| {
                t.meaningful()
                    && self.workspace_visible(t)
                    && (q.is_empty()
                        || t.title.to_lowercase().contains(&q)
                        || t.outputs.iter().any(|o| o.name.to_lowercase().contains(&q))
                        || t.turns
                            .iter()
                            .any(|turn| turn.text.to_lowercase().contains(&q)))
            })
            .collect::<Vec<_>>();
        tasks.sort_by_key(|t| std::cmp::Reverse(t.updated));
        let mut rows = tasks
            .into_iter()
            .take(max)
            .map(|t| {
                let selected = self.workspace.active.as_ref() == Some(&t.id);
                let icon = if !t.outputs.is_empty() {
                    UiIcon::File
                } else if t.session.is_some() {
                    UiIcon::Smartphone
                } else {
                    UiIcon::Chat
                };
                let subtitle = if !t.outputs.is_empty() {
                    format!("{} 个成果", t.outputs.len())
                } else if t.session.is_some() {
                    if !t.draft.is_empty() {
                        "手机会话 · 有草稿".into()
                    } else {
                        "手机会话".into()
                    }
                } else if !t.draft.is_empty() {
                    "有草稿".into()
                } else {
                    "继续对话".into()
                };
                let label = amoo_display::session_title(&t.title);
                self.list_row_with_menu(
                    format!("workspace-recent-{}", t.id),
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(icon, 16., if selected { p.accent } else { p.text2 }),
                            Expanded::new(
                                1.,
                                ConstrainedBox::new(self.vstack(
                                    S1,
                                    vec![
                                        self.line_w(
                                            label,
                                            BODY,
                                            p.text,
                                            if selected {
                                                Weight::Semibold
                                            } else {
                                                Weight::Medium
                                            },
                                        ),
                                        self.line(subtitle, MICRO, p.text3),
                                    ],
                                ))
                                .with_min_width(168.)
                                .finish(),
                            )
                            .finish(),
                            self.icon_action(
                                format!("workspace-delete-{}", t.id),
                                "删除对话",
                                UiIcon::Trash,
                                vec![Action::Workspace(W::Delete(t.id.clone()))],
                                false,
                            ),
                        ],
                    ),
                    selected,
                    Some(Action::Workspace(W::Select(t.id.clone()))),
                    None,
                    Some(crate::ui::context_menu::Target::ChatSession {
                        workspace_task_id: Some(t.id.clone()),
                        phone_session_idx: None,
                        title: t.title.clone(),
                    }),
                )
            })
            .collect::<Vec<_>>();
        if rows.len() < max {
            for (i, s) in self
                .amoo_sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    !self.workspace.deleted_sessions.contains(&s.session_key)
                        && (q.is_empty()
                            || s.title.to_lowercase().contains(&q)
                            || amoo_display::session_title(&s.title)
                                .to_lowercase()
                                .contains(&q))
                        && !self.workspace.journal.tasks.iter().any(|t| {
                            self.workspace_visible(t) && t.session.as_ref() == Some(&s.session_key)
                        })
                })
                .take(max - rows.len())
            {
                let label = amoo_display::session_title(&s.title);
                rows.push(self.list_row_with_menu(
                    format!("workspace-phone-session-{i}"),
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(UiIcon::Smartphone, 16., p.text2),
                            Expanded::new(
                                1.,
                                ConstrainedBox::new(self.vstack(
                                    S1,
                                    vec![
                                        self.line_w(label, BODY, p.text, Weight::Medium),
                                        self.line("手机会话", MICRO, p.text3),
                                    ],
                                ))
                                .with_min_width(168.)
                                .finish(),
                            )
                            .finish(),
                            self.icon_action(
                                format!("workspace-delete-phone-{i}"),
                                "删除对话",
                                UiIcon::Trash,
                                vec![Action::Workspace(W::Delete(format!(
                                    "phone:{}",
                                    s.session_key
                                )))],
                                false,
                            ),
                        ],
                    ),
                    false,
                    Some(Action::Workspace(W::PhoneSession(i))),
                    None,
                    Some(crate::ui::context_menu::Target::ChatSession {
                        workspace_task_id: None,
                        phone_session_idx: Some(i),
                        title: s.title.clone(),
                    }),
                ));
            }
        }
        if rows.is_empty() {
            rows.push(
                Container::new(
                    Align::new(self.line(
                        if q.is_empty() {
                            "还没有对话记录"
                        } else {
                            "没有匹配的对话"
                        },
                        CAPTION,
                        p.text3,
                    ))
                    .finish(),
                )
                .with_vertical_padding(S5)
                .finish(),
            );
        }
        rows
    }
    fn workspace_thread(&self) -> El {
        let mut rows = Vec::new();
        if let Some(task) = self.workspace_task() {
            let has_local_steps = task.steps.iter().any(|s| {
                matches!(
                    s.executor,
                    Executor::Files | Executor::App | Executor::Screen
                )
            });
            let phone_session = task.session.as_ref().and_then(|key| {
                self.amoo_sessions
                    .iter()
                    .find(|s| &s.session_key == key && !s.items.is_empty())
            });
            let phone_loaded =
                !(has_local_steps && !task.turns.is_empty()) && phone_session.is_some();
            if let Some(sess) = phone_session.filter(|_| phone_loaded) {
                // The phone transcript renders its own bubbles (copy / read
                // aloud / retry / quote icons on every reply).
                if self
                    .amoo_sessions
                    .get(self.amoo_session_idx)
                    .is_some_and(|s| s.session_key == sess.session_key)
                {
                    rows.push(self.amoo_conversation());
                } else {
                    for (i, item) in sess.items.iter().enumerate() {
                        match item {
                            AmooChatItem::UserText { text, .. } => {
                                rows.push(self.context_target(
                                    format!("ws-phone-user-{i}"),
                                    crate::ui::context_menu::Target::ChatMessage {
                                        assistant: false,
                                        text: text.clone(),
                                        session_key: Some(sess.session_key.clone()),
                                        retry_text: None,
                                    },
                                    self.chat_user_bubble(text),
                                ));
                            }
                            AmooChatItem::AssistantRich {
                                text: Some(text), ..
                            } if !text.trim().is_empty() => {
                                rows.push(self.context_target(
                                    format!("ws-phone-assistant-{i}"),
                                    crate::ui::context_menu::Target::ChatMessage {
                                        assistant: true,
                                        text: text.clone(),
                                        session_key: Some(sess.session_key.clone()),
                                        retry_text: None,
                                    },
                                    self.chat_assistant_bubble(
                                        self.work_rich_text(text, &format!("ws-phone-reply-{i}")),
                                        Vec::new(),
                                    ),
                                ));
                            }
                            _ => {}
                        }
                    }
                }
                if let Some(chip) = self.route_note_chip(task) {
                    rows.push(chip);
                }
            } else {
                let last_user = task.turns.iter().rposition(|t| t.user);
                for (i, turn) in task.turns.iter().enumerate() {
                    if !turn.user
                        && stepx_companion::amoo_workspace::is_unconfirmed_placeholder(&turn.text)
                    {
                        continue;
                    }
                    if turn.user {
                        let bubble = self.context_target(
                            format!("ws-user-msg-{i}"),
                            crate::ui::context_menu::Target::ChatMessage {
                                assistant: false,
                                text: turn.text.clone(),
                                session_key: None,
                                retry_text: None,
                            },
                            self.chat_user_bubble(&turn.text),
                        );
                        if Some(i) == last_user {
                            if let Some(chip) = self.route_note_chip(task) {
                                rows.push(self.vstack(S1 + 2., vec![bubble, chip]));
                            } else {
                                rows.push(bubble);
                            }
                        } else {
                            rows.push(bubble);
                        }
                    } else {
                        let bubble = self.chat_assistant_bubble(
                            self.work_rich_text(
                                &turn.text,
                                &format!("workspace-turn-{}-{i}", task.id),
                            ),
                            vec![
                                self.amoo_action_icon(
                                    format!("workspace-turn-copy-{}-{i}", task.id),
                                    "复制",
                                    UiIcon::Copy,
                                    Action::CopyText(turn.text.clone(), "已复制".into()),
                                ),
                                self.amoo_action_icon(
                                    format!("workspace-turn-retry-{}-{i}", task.id),
                                    "重新生成",
                                    UiIcon::Retry,
                                    Action::Workspace(W::Retry),
                                ),
                                self.amoo_action_icon(
                                    format!("workspace-turn-quote-{}-{i}", task.id),
                                    "引用这段继续问",
                                    UiIcon::Chat,
                                    Action::Workspace(W::QuoteTurn(turn.text.clone())),
                                ),
                            ],
                        );
                        rows.push(self.context_target(
                            format!("ws-ai-msg-{i}"),
                            crate::ui::context_menu::Target::ChatMessage {
                                assistant: true,
                                text: turn.text.clone(),
                                session_key: None,
                                retry_text: None,
                            },
                            bubble,
                        ));
                    }
                }
            }
            let running = self
                .workspace
                .running
                .as_ref()
                .is_some_and(|r| r.task == task.id);
            let failed = task
                .steps
                .iter()
                .any(|s| matches!(s.phase, Phase::Failed | Phase::Stopped | Phase::Unconfirmed));
            let unconfirmed_only = failed
                && !task
                    .steps
                    .iter()
                    .any(|s| matches!(s.phase, Phase::Failed | Phase::Stopped));
            let has_work = task.steps.iter().any(|s| s.executor != Executor::Phone);
            let active_progress = running || failed;
            if has_work && !active_progress {
                rows.push(self.turn_progress(task, false, false, false));
            }
            // Results follow the latest reply as inline cards.
            for (i, o) in task.outputs.iter().enumerate() {
                rows.push(self.result_card(task, i, o));
            }
            if active_progress {
                rows.push(self.turn_progress(task, running, failed, unconfirmed_only));
            }
        }
        if let Some(proposal) = &self.workspace.proposal {
            rows.push(self.proposal_bar(proposal));
        }
        if let Some(pip) = self.workspace_gui_pip_card() {
            rows.push(pip);
        }
        if let Some(error) = &self.workspace.error {
            rows.push(self.workspace_error_banner(error));
        }
        self.vstack(S4, rows)
    }
    /// User message: right-aligned accent bubble, same as the phone transcript.
    fn chat_user_bubble(&self, text: &str) -> El {
        let p = self.p();
        let bubble = Container::new(self.text(text, CAPTION, p.accent_fg))
            .with_horizontal_padding(14.)
            .with_vertical_padding(8.)
            .with_background_color(p.accent)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
            .finish();
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_main_axis_alignment(MainAxisAlignment::End)
            .with_child(ConstrainedBox::new(bubble).with_max_width(430.).finish())
            .finish()
    }
    /// Assistant message: avatar + content + quiet icon actions underneath.
    fn chat_assistant_bubble(&self, body: El, actions: Vec<El>) -> El {
        let p = self.p();
        let mut children = vec![body];
        if !actions.is_empty() {
            children.push(self.hstack(2., actions));
        }
        let badge = ConstrainedBox::new(
            Container::new(Align::new(self.ui_icon(UiIcon::Star, 15., p.accent)).finish())
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
                .finish(),
        )
        .with_width(28.)
        .with_height(28.)
        .finish();
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(10.)
            .with_child(Container::new(badge).with_padding_top(6.).finish())
            .with_child(
                Expanded::new(
                    1.,
                    Container::new(self.vstack(S2, children))
                        .with_vertical_padding(10.)
                        .with_horizontal_padding(4.)
                        .finish(),
                )
                .finish(),
            )
            .finish()
    }
    /// "已自动识别为：生成 PPT · 改为普通聊天" under the latest request.
    fn route_note_chip(&self, task: &Task) -> Option<El> {
        let note = task.route_note.trim();
        if note.is_empty() {
            return None;
        }
        let p = self.p();
        Some(
            Align::new(
                Container::new(self.hstack(
                    S1 + 2.,
                    vec![
                        self.ui_icon(UiIcon::Bolt, 12., p.accent),
                        self.line(format!("已自动识别为：{note}"), CAPTION, p.text2),
                        self.line("·", CAPTION, p.text3),
                        self.link_muted(
                            format!("workspace-reroute-chat-{}", task.id),
                            "改为普通聊天",
                            Action::Workspace(W::Reroute(Intent::Chat)),
                        ),
                    ],
                ))
                .with_horizontal_padding(S2 + 2.)
                .with_vertical_padding(3.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                .finish(),
            )
            .right()
            .finish(),
        )
    }
    /// Plain one-line status of the current run, with elapsed seconds.
    fn run_status_line(&self) -> String {
        let Some(run) = &self.workspace.running else {
            return "正在处理…".into();
        };
        let elapsed = run.started_at.elapsed().as_secs();
        let base = match run.executor {
            Executor::Files => "正在生成文件",
            Executor::App => "正在制作并检查应用",
            Executor::Screen => "正在生成背屏图案",
            Executor::Image => {
                if self.work.status.contains("保存原图") {
                    "正在保存图片"
                } else {
                    "正在生成图片"
                }
            }
            Executor::Phone => {
                if !run.accepted {
                    "正在发给手机"
                } else if self.selected.is_none() {
                    "手机连接不稳定，正在重连"
                } else if self.workspace.stream_delta_count > 0 {
                    "正在回复"
                } else {
                    "正在思考"
                }
            }
        };
        format!("{base}（{elapsed} 秒）")
    }
    /// Inline progress: compact stepper, one plain status line, a progress
    /// bar and stop / retry / sync. Raw executor status lives behind "详情".
    fn turn_progress(
        &self,
        task: &Task,
        running: bool,
        failed: bool,
        unconfirmed_only: bool,
    ) -> El {
        let p = self.p();
        let key = format!("workspace-log-{}", task.id);
        let expanded = self.amoo_expanded_tool_records.contains(&key);
        let details_link = self.link_muted(
            "workspace-progress-details",
            if expanded { "收起详情" } else { "详情" },
            Action::AmooToggleToolRecords(key),
        );
        let has_stepper = task.steps.len() > 1
            || (task.steps.len() == 1 && task.steps[0].executor != Executor::Phone);
        if !running && !failed && !expanded {
            if has_stepper {
                let stepper = self.stepper(
                    task.steps
                        .iter()
                        .map(|s| {
                            (
                                step_label(s.executor).to_string(),
                                match s.phase {
                                    Phase::Done => StepMark::Done,
                                    Phase::Running => StepMark::Active,
                                    Phase::Ready | Phase::Unconfirmed => StepMark::Pending,
                                    Phase::Failed | Phase::Stopped => StepMark::Failed,
                                },
                            )
                        })
                        .collect(),
                );
                return self.hstack(
                    S2,
                    vec![stepper, self.line("·", CAPTION, p.text3), details_link],
                );
            }
            return self.hstack(
                S2,
                vec![self.line("已完成", CAPTION, p.text3), details_link],
            );
        }
        let mut rows = Vec::new();
        if has_stepper {
            rows.push(
                self.stepper(
                    task.steps
                        .iter()
                        .map(|s| {
                            (
                                step_label(s.executor).to_string(),
                                match s.phase {
                                    Phase::Done => StepMark::Done,
                                    Phase::Running => StepMark::Active,
                                    Phase::Ready | Phase::Unconfirmed => StepMark::Pending,
                                    Phase::Failed | Phase::Stopped => StepMark::Failed,
                                },
                            )
                        })
                        .collect(),
                ),
            );
        }
        let (status, color) = if running {
            (self.run_status_line(), p.text)
        } else if unconfirmed_only {
            ("还没收到手机的结果".to_string(), p.warning)
        } else if failed {
            ("没有完成".to_string(), p.warning)
        } else {
            ("已完成".to_string(), p.text3)
        };
        rows.push(self.between(
            self.hstack(S2, vec![self.line(status, CAPTION, color), details_link]),
            if running {
                self.ws_button("stop", "停止", UiIcon::Close, W::Stop, Btn::Ghost)
            } else {
                self.line("", MICRO, p.text3)
            },
        ));
        if running {
            let total = task.steps.len().max(1) as f32;
            let done = task.steps.iter().filter(|s| s.phase == Phase::Done).count() as f32;
            let within = self
                .workspace
                .running
                .as_ref()
                .map(|r| 1. - (-(r.started_at.elapsed().as_secs_f32()) / 45.).exp())
                .unwrap_or(0.)
                .min(0.9);
            let pct = (((done + within) / total) * 100.).clamp(3., 97.) as u8;
            rows.push(self.progress(Some(pct), p.accent));
        }
        if expanded {
            let mut details = Vec::new();
            for step in &task.steps {
                let phase = match step.phase {
                    Phase::Ready => "待开始",
                    Phase::Running => "进行中",
                    Phase::Done => "已完成",
                    Phase::Failed => "未完成",
                    Phase::Stopped => "已停止",
                    Phase::Unconfirmed => "待确认",
                };
                details.push(self.text(
                    format!("{} · {}", step_label(step.executor), phase),
                    CAPTION,
                    p.text2,
                ));
                if !step.note.is_empty() {
                    details.push(self.text(&step.note, CAPTION, p.text3));
                }
                if step.phase == Phase::Running {
                    let raw = match step.executor {
                        Executor::Files | Executor::Image => self.work.status.as_str(),
                        Executor::App => self.studio_lab.status.as_str(),
                        _ => "",
                    };
                    if !raw.is_empty() {
                        details.push(self.text(
                            raw.chars().take(240).collect::<String>(),
                            CAPTION,
                            p.text3,
                        ));
                    }
                }
            }
            if running {
                if let Some(tool) = &self.workspace.stream_tool_status {
                    details.push(self.text(tool, CAPTION, p.text3));
                }
                if self.workspace.stream_delta_count > 0 {
                    details.push(self.text(
                        format!("已收到 {} 段回复", self.workspace.stream_delta_count),
                        CAPTION,
                        p.text3,
                    ));
                }
            }
            if !details.is_empty() {
                rows.push(
                    Container::new(self.vstack(S1, details))
                        .with_uniform_padding(S3)
                        .with_background_color(if running || failed { p.bg } else { p.surface })
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish(),
                );
            }
        }
        if failed && !running {
            let mut action_btns = Vec::new();
            if unconfirmed_only && task.session.is_some() {
                action_btns.push(self.ws_button(
                    "sync-task",
                    "同步手机结果",
                    UiIcon::Refresh,
                    W::SyncTask,
                    Btn::Primary,
                ));
            }
            action_btns.push(self.ws_button(
                "retry",
                if unconfirmed_only {
                    "重新发送"
                } else {
                    "重试"
                },
                UiIcon::Retry,
                W::Retry,
                Btn::Secondary,
            ));
            if unconfirmed_only {
                action_btns.push(self.ws_button(
                    "dismiss-unconfirmed",
                    "忽略",
                    UiIcon::Close,
                    W::DismissUnconfirmed,
                    Btn::Ghost,
                ));
            }
            rows.push(self.wrap(action_btns));
        }
        if !running && !failed {
            // A finished run only needs a quiet one-liner.
            return self.vstack(S1, rows);
        }
        Container::new(self.vstack(S2, rows))
            .with_uniform_padding(S3)
            .with_background_color(p.surface)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
    }
    fn is_playable_app_output(kind: &str, path: &std::path::Path) -> bool {
        kind.contains("应用")
            || path
                .extension()
                .and_then(|e| e.to_str())
                .is_some_and(|e| e.eq_ignore_ascii_case("html") || e.eq_ignore_ascii_case("htm"))
    }
    /// One result card, shaped by what the output is.
    fn result_card(&self, task: &Task, i: usize, o: &Output) -> El {
        let p = self.p();
        let ext = o
            .path
            .extension()
            .and_then(|e| e.to_str())
            .map(|s| s.to_ascii_lowercase())
            .unwrap_or_default();
        let is_image = (o.kind == "图片"
            || matches!(
                ext.as_str(),
                "png" | "jpg" | "jpeg" | "webp" | "gif" | "bmp"
            ))
            && cached_is_file(&o.path);
        let is_app = o.lab_id.is_some() || Self::is_playable_app_output(&o.kind, &o.path);
        let mut card = if is_app {
            self.result_app_card(task, i, o)
        } else if is_image {
            self.result_image_card(task, i, o)
        } else {
            self.result_doc_card(task, i, o, &ext)
        };
        if let Some(confirm) = self.inline_delivery_confirm(i, o) {
            card.push(confirm);
        }
        let is_selected = self.workspace.selected_output == Some(i);
        let panel = if is_selected {
            self.context_target(
                format!("ws-res-card-sel-{i}"),
                crate::ui::context_menu::Target::WorkspaceOutput(i),
                Container::new(self.vstack(S3, card))
                    .with_uniform_padding(S4)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.5).with_border_color(p.accent))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
            )
        } else {
            self.hover_panel_interactive(
                format!("ws-res-card-{i}"),
                self.vstack(S3, card),
                None,
                Some(Action::Workspace(W::OpenOutput(i))),
                Some(crate::ui::context_menu::Target::WorkspaceOutput(i)),
            )
        };
        super::super::motion::sub_enter(panel, self.motion_started, !self.animations_disabled())
    }
    fn result_title_row(&self, i: usize, icon: UiIcon, o: &Output, meta: String) -> El {
        let p = self.p();
        let is_selected = self.workspace.selected_output == Some(i);
        let mut row_items = vec![
            self.ui_icon(icon, 20., p.accent),
            Expanded::new(
                1.,
                self.vstack(
                    S1,
                    vec![
                        self.line_w(&o.name, BODY, p.text, Weight::Semibold),
                        self.line(meta, MICRO, p.text2),
                    ],
                ),
            )
            .finish(),
        ];
        if is_selected {
            row_items.push(self.pill_badge("正在预览 / 修改", p.accent, p.accent_soft));
        }
        self.list_row_with_menu(
            format!("workspace-output-{i}"),
            self.hstack(S3, row_items),
            is_selected,
            Some(Action::Workspace(W::Preview(i))),
            Some(Action::Workspace(
                if Self::is_playable_app_output(&o.kind, &o.path) || o.lab_id.is_some() {
                    W::PlayAppOutput(i)
                } else {
                    W::OpenOutput(i)
                },
            )),
            Some(crate::ui::context_menu::Target::WorkspaceOutput(i)),
        )
    }
    fn result_more_toggle(&self, task: &Task, i: usize) -> (El, bool) {
        let key = format!("workspace-output-more-{}-{i}", task.id);
        let open = self.amoo_expanded_tool_records.contains(&key);
        (
            self.button_icon(
                format!("workspace-output-more-{i}"),
                "更多",
                if open {
                    UiIcon::ChevronUp
                } else {
                    UiIcon::ChevronDown
                },
                Action::AmooToggleToolRecords(key),
                Btn::Ghost,
            ),
            open,
        )
    }
    fn copy_path_button(&self, i: usize, o: &Output) -> El {
        self.button_icon(
            format!("workspace-output-copy-path-{i}"),
            "复制路径",
            UiIcon::Copy,
            Action::CopyText(o.path.display().to_string(), "已复制文件路径".into()),
            Btn::Ghost,
        )
    }
    fn result_doc_card(&self, task: &Task, i: usize, o: &Output, ext: &str) -> Vec<El> {
        let type_label = match ext {
            "docx" | "doc" => "Word 文档",
            "xlsx" | "xls" => "Excel 表格",
            "pptx" | "ppt" => "PPT 演示",
            "csv" => "表格",
            "md" | "txt" => "文档",
            "pdf" => "PDF 文档",
            _ => o.kind.as_str(),
        };
        let meta = match cached_file_len(&o.path) {
            Some(len) => format!("{type_label} · {}", storage::format_bytes(len)),
            None => type_label.to_string(),
        };
        let icon = if matches!(ext, "xlsx" | "xls" | "csv" | "pptx" | "ppt") {
            UiIcon::Chart
        } else {
            UiIcon::File
        };
        let (more, more_open) = self.result_more_toggle(task, i);
        let mut rows = vec![
            self.result_title_row(i, icon, o, meta),
            self.wrap(vec![
                self.ws_button(
                    &format!("output-open-local-{i}"),
                    "打开",
                    UiIcon::ExternalLink,
                    W::OpenOutput(i),
                    Btn::Primary,
                ),
                self.ws_button(
                    &format!("output-preview-{i}"),
                    "预览",
                    UiIcon::Search,
                    W::Preview(i),
                    Btn::Secondary,
                ),
                self.ws_button(
                    &format!("output-edit-{i}"),
                    "继续修改",
                    UiIcon::Tools,
                    W::ContinueEdit(i),
                    Btn::Secondary,
                ),
                more,
            ]),
        ];
        if more_open {
            rows.push(self.wrap(vec![
                self.ws_button(
                    &format!("output-reveal-{i}"),
                    "在文件夹中显示",
                    UiIcon::Folder,
                    W::RevealOutput(i),
                    Btn::Ghost,
                ),
                self.ws_button(
                    &format!("output-send-phone-{i}"),
                    "发送到手机",
                    UiIcon::Smartphone,
                    W::SendOutputToPhone(i),
                    Btn::Ghost,
                ),
                self.copy_path_button(i, o),
                self.ws_button(
                    &format!("output-flow-app-{i}"),
                    "做成小应用",
                    UiIcon::Apps,
                    W::FlowApp,
                    Btn::Ghost,
                ),
            ]));
        }
        rows
    }
    fn result_app_card(&self, task: &Task, i: usize, o: &Output) -> Vec<El> {
        let p = self.p();
        let artifact = o
            .lab_id
            .as_ref()
            .and_then(|id| self.studio_lab.artifacts.iter().find(|a| &a.id == id));
        let checks = o.lab_id.as_ref().and_then(|id| {
            let (report_id, report) = self.studio_lab.report.as_ref()?;
            let checks = report["checks"].as_array().filter(|_| report_id == id)?;
            let passed = checks
                .iter()
                .filter(|c| c["passed"].as_bool().unwrap_or(false))
                .count();
            Some((passed, checks.len()))
        });
        let passed = artifact.is_some_and(|a| a.passed) || o.kind.contains("已检查");
        let meta = match (checks, artifact) {
            (Some((ok, total)), _) if total > 0 => format!("检查通过 {ok}/{total}"),
            (_, Some(_)) if passed => "已检查，可以运行".to_string(),
            (_, Some(_)) => "检查没通过，可以让我继续修".to_string(),
            _ => "小应用 · 可在电脑运行，也能装到手机".to_string(),
        };
        let mut rows = Vec::new();
        if let Some(shot) = artifact
            .and_then(|a| a.screenshot_path.as_ref())
            .filter(|p| cached_is_file(p))
        {
            let target_h = 168.0_f32;
            let target_w = cached_image_dimensions(shot)
                .map(|(w, h)| (target_h * (w as f32 / h.max(1) as f32)).clamp(140.0, 296.0))
                .unwrap_or(270.0);
            rows.push(
                ConstrainedBox::new(
                    Container::new(
                        ConstrainedBox::new(
                            Image::new(
                                AssetSource::LocalFile {
                                    path: shot.to_string_lossy().into_owned(),
                                },
                                CacheOption::BySize,
                            )
                            .contain()
                            .finish(),
                        )
                        .with_width(target_w)
                        .with_height(target_h)
                        .finish(),
                    )
                    .with_uniform_padding(S1)
                    .with_background_color(p.surface)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
                )
                .with_max_width(target_w + S1 * 2.0 + 2.0)
                .finish(),
            );
        }
        let _ = task;
        rows.push(self.result_title_row(i, UiIcon::Apps, o, meta));
        rows.push(self.wrap(vec![
            self.ws_button(
                &format!("output-play-{i}"),
                "运行预览",
                UiIcon::Play,
                W::PlayAppOutput(i),
                Btn::Primary,
            ),
            self.ws_button(
                &format!("output-phone-{i}"),
                "发送到手机",
                UiIcon::Smartphone,
                W::PushAppOutputToPhone(i),
                Btn::Secondary,
            ),
            self.ws_button(
                &format!("output-edit-{i}"),
                "继续修改",
                UiIcon::Tools,
                W::ContinueEdit(i),
                Btn::Secondary,
            ),
            self.ws_button(
                &format!("output-reveal-{i}"),
                "在文件夹中显示",
                UiIcon::Folder,
                W::RevealOutput(i),
                Btn::Ghost,
            ),
        ]));
        rows
    }
    fn result_image_card(&self, task: &Task, i: usize, o: &Output) -> Vec<El> {
        let p = self.p();
        let path_str = o.path.to_string_lossy().into_owned();
        let dims = cached_image_dimensions(&o.path);
        let meta = match (dims, cached_file_len(&o.path)) {
            (Some((w, h)), Some(len)) => {
                format!("图片 · {w}×{h} · {}", storage::format_bytes(len))
            }
            (_, Some(len)) => format!("图片 · {}", storage::format_bytes(len)),
            _ => "图片".to_string(),
        };
        let (more, more_open) = self.result_more_toggle(task, i);
        let target_h = 168.0_f32;
        let target_w = dims
            .map(|(w, h)| (target_h * (w as f32 / h.max(1) as f32)).clamp(140.0, 296.0))
            .unwrap_or(270.0);
        let thumb = ConstrainedBox::new(
            Container::new(
                ConstrainedBox::new(
                    Image::new(
                        AssetSource::LocalFile {
                            path: path_str.clone(),
                        },
                        CacheOption::BySize,
                    )
                    .contain()
                    .finish(),
                )
                .with_width(target_w)
                .with_height(target_h)
                .finish(),
            )
            .with_uniform_padding(S1)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish(),
        )
        .with_max_width(target_w + S1 * 2.0 + 2.0)
        .finish();
        let thumb_interactive = ConstrainedBox::new(
            Hoverable::new(
                self.mouse_state(format!("workspace-output-thumb-{i}")),
                move |_| thumb,
            )
            .with_cursor(crate::ui::platform::Cursor::PointingHand)
            .on_double_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::Workspace(W::OpenOutput(i)));
            })
            .on_right_click(move |ctx, _, at| {
                ctx.dispatch_typed_action(Action::ContextOpen(
                    crate::ui::context_menu::Target::WorkspaceOutput(i),
                    at,
                ));
            })
            .finish(),
        )
        .with_max_width(target_w + S1 * 2.0 + 2.0)
        .finish();
        let mut rows = vec![
            thumb_interactive,
            self.result_title_row(i, UiIcon::Image, o, meta),
            self.wrap(vec![
                self.button_icon(
                    format!("workspace-output-zoom-full-{i}"),
                    "查看大图",
                    UiIcon::Maximize,
                    Action::Work(WorkAction::OpenPreview(path_str)),
                    Btn::Primary,
                ),
                self.ws_button(
                    &format!("output-open-local-{i}"),
                    "打开",
                    UiIcon::ExternalLink,
                    W::OpenOutput(i),
                    Btn::Secondary,
                ),
                self.ws_button(
                    &format!("output-edit-{i}"),
                    "继续修改",
                    UiIcon::Tools,
                    W::ContinueEdit(i),
                    Btn::Secondary,
                ),
                more,
            ]),
        ];
        if more_open {
            let mut extra = vec![self.ws_button(
                &format!("output-reveal-{i}"),
                "在文件夹中显示",
                UiIcon::Folder,
                W::RevealOutput(i),
                Btn::Ghost,
            )];
            if self
                .work
                .local_preview
                .as_ref()
                .is_some_and(|v| v.path == o.path)
            {
                extra.push(self.button_icon(
                    format!("workspace-output-export-{i}"),
                    "另存为",
                    UiIcon::Download,
                    Action::Work(WorkAction::ExportLocalPreview),
                    Btn::Ghost,
                ));
            }
            extra.push(self.ws_button(
                &format!("output-send-phone-{i}"),
                "发送到手机",
                UiIcon::Smartphone,
                W::SendOutputToPhone(i),
                Btn::Ghost,
            ));
            extra.push(self.copy_path_button(i, o));
            rows.push(self.wrap(extra));
        }
        rows
    }
    /// "Send to phone" stays an explicit confirmation, shown right under the
    /// card that asked for it.
    fn inline_delivery_confirm(&self, i: usize, o: &Output) -> Option<El> {
        if self.workspace.delivery != Some(false) || self.workspace.selected_output != Some(i) {
            return None;
        }
        if self.push.shell_mode == ShellMode::Full
            && self.workspace.viewport.x() >= 1080.
            && matches!(self.workspace.panel, 3 | 6)
        {
            return None;
        }
        let device = self
            .selected
            .as_ref()
            .map(|d| d.model.as_str())
            .unwrap_or("手机");
        let message = if o.lab_id.is_some() {
            format!("发送到 {device} 的 StepX 伴侣 › 作品？")
        } else {
            format!("发送到 {device} 的 StepX 伴侣 › 接收文件？")
        };
        let mut buttons = Vec::new();
        if o.lab_id.is_some() {
            buttons.push(self.chip(
                format!("workspace-inline-open-after-{i}"),
                "发送后打开",
                Some(if self.workspace.open_phone {
                    UiIcon::Check
                } else {
                    UiIcon::Smartphone
                }),
                vec![Action::Workspace(W::ToggleOpenPhone)],
                self.workspace.open_phone,
                false,
            ));
        }
        buttons.push(self.ws_button(
            &format!("inline-cancel-delivery-{i}"),
            "取消",
            UiIcon::Close,
            W::CancelDelivery,
            Btn::Ghost,
        ));
        buttons.push(if self.busy || self.selected.is_none() {
            self.disabled_button(if self.busy {
                "正在发送"
            } else {
                "请先连接手机"
            })
        } else {
            self.ws_button(
                &format!("inline-confirm-delivery-{i}"),
                "发送",
                UiIcon::Check,
                W::ConfirmDelivery,
                Btn::Primary,
            )
        });
        Some(self.confirm_bar(message, false, buttons))
    }
    /// A multi-step request waits for one tap: "我会先…，再…  [改一下] [开始]".
    fn proposal_bar(&self, proposal: &super::super::workspace_state::Proposal) -> El {
        let p = self.p();
        let steps = proposal
            .steps
            .iter()
            .map(|s| proposal_step_label(*s))
            .collect::<Vec<_>>();
        let message = match steps.as_slice() {
            [] => "我会按你的要求开始处理。".to_string(),
            [only] => format!("我会先{only}。"),
            [first, rest @ ..] => format!("我会先{first}，再{}。", rest.join("，再")),
        };
        let key = "workspace-proposal-details".to_string();
        let expanded = self.amoo_expanded_tool_records.contains(&key);
        let mut rows = vec![self.confirm_bar(
            message,
            false,
            vec![
                self.ws_button(
                    "adjust",
                    "改一下",
                    UiIcon::ArrowLeft,
                    W::EditPlan,
                    Btn::Ghost,
                ),
                if self.busy {
                    self.disabled_button("正在准备")
                } else {
                    self.ws_button("confirm", "开始", UiIcon::Play, W::Confirm, Btn::Primary)
                },
            ],
        )];
        rows.push(self.link_muted(
            "workspace-proposal-details",
            if expanded { "收起详情" } else { "详情" },
            Action::AmooToggleToolRecords(key),
        ));
        if expanded {
            if !proposal.text.trim().is_empty() {
                rows.push(self.text(&proposal.text, CAPTION, p.text2));
            }
            if !proposal.files.is_empty() {
                rows.push(self.text(
                    format!(
                        "资料：{}",
                        proposal
                            .files
                            .iter()
                            .map(|p| p.file_name().unwrap_or_default().to_string_lossy())
                            .collect::<Vec<_>>()
                            .join("、")
                    ),
                    CAPTION,
                    p.text2,
                ));
            }
            if proposal
                .steps
                .iter()
                .any(|s| matches!(s, Executor::Files | Executor::App | Executor::Image))
            {
                rows.push(
                    self.between(
                        self.text(
                            self.workspace_task()
                                .and_then(|t| t.workspace.as_ref())
                                .map(|p| format!("保存到：{}", p.display()))
                                .unwrap_or("保存到：电脑上的独立文件夹".into()),
                            CAPTION,
                            p.text2,
                        ),
                        self.ws_button(
                            "folder",
                            "更改",
                            UiIcon::Folder,
                            W::ChooseFolder,
                            Btn::Ghost,
                        ),
                    ),
                );
            }
            if proposal.steps.contains(&Executor::App) {
                rows.push(self.text(
                    if self.studio_lab.config.has_key {
                        "做应用会用你自己设置的模型服务和额度。"
                    } else {
                        "做应用会用已连接手机的 Amoo，电脑负责保存和试运行检查。"
                    },
                    CAPTION,
                    p.text3,
                ));
            }
        }
        super::super::motion::sub_enter(
            self.vstack(S1, rows),
            self.motion_started,
            !self.animations_disabled(),
        )
    }
    pub(in super::super) fn workspace_gui_pip_card(&self) -> Option<El> {
        if !self.amoo_ui.gui_running && !self.amoo_ui.gui_paused {
            return None;
        }
        let p = self.p();
        let (badge_label, badge_fg, badge_bg) = if self.amoo_ui.gui_paused {
            ("已暂停，你可以接手", p.warning, p.warning_soft)
        } else if self.amoo_ui.gui_running {
            ("执行中", p.success_text, p.success_soft)
        } else {
            ("最近画面", p.text2, p.surface)
        };
        let mut content = vec![self.between(
            self.hstack(
                S2,
                vec![
                    self.ui_icon(UiIcon::Smartphone, 18., p.text),
                    self.heading(if self.amoo_ui.gui_paused {
                        "已暂停操作手机"
                    } else {
                        "正在操作手机"
                    }),
                ],
            ),
            self.pill_badge(badge_label, badge_fg, badge_bg),
        )];
        if let Some((app, task)) = &self.amoo_ui.gui_current {
            content.push(self.line_w(format!("目标应用：{app}"), BODY, p.text, Weight::Medium));
            content.push(self.text(task, CAPTION, p.text2));
        } else if !self.amoo_ui.gui_message.is_empty() {
            content.push(self.text(&self.amoo_ui.gui_message, CAPTION, p.text2));
        }
        if let Some((step_no, detail)) = self.amoo_ui.gui_steps.last() {
            content.push(self.text(format!("步骤 {step_no}：{detail}"), CAPTION, p.text2));
        }
        if let Some(path) = self
            .amoo_ui
            .gui_screenshot
            .as_ref()
            .or_else(|| self.screenshot.as_ref().map(|(_, p)| p))
            .filter(|p| cached_is_file(p))
        {
            content.push(
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
                .with_max_width(220.)
                .with_height(360.)
                .finish(),
            );
        }
        content.push(self.wrap(vec![
            self.button_icon(
                "workspace-gui-pip-pause",
                if self.amoo_ui.gui_paused {
                    "继续"
                } else {
                    "暂停"
                },
                if self.amoo_ui.gui_paused {
                    UiIcon::Play
                } else {
                    UiIcon::Pause
                },
                Action::AmooUi(AmooUiAction::GuiPauseResume),
                Btn::Secondary,
            ),
            self.button_icon(
                "workspace-gui-pip-stop",
                "停止",
                UiIcon::Close,
                Action::AmooUi(AmooUiAction::GuiStopWaiting),
                Btn::Secondary,
            ),
            self.button_icon(
                "workspace-gui-pip-open",
                "展开",
                UiIcon::Smartphone,
                Action::AmooUi(AmooUiAction::GuiMode(true)),
                Btn::Primary,
            ),
        ]));
        Some(self.panel(content))
    }
    fn workspace_history(&self) -> El {
        let mut rows = vec![self.input(F_AMOO_SEARCH, "搜索对话", None)];
        rows.push(self.button_icon(
            "workspace-sync-history",
            "同步手机记录",
            UiIcon::Refresh,
            Action::AmooSnapshot,
            Btn::Secondary,
        ));
        rows.extend(self.workspace_recent(100));
        self.vstack(S2, rows)
    }
    fn workspace_model(&self) -> El {
        self.workspace_models()
    }
    fn workspace_side(&self, _compact: bool) -> El {
        let p = self.p();
        let title = if self.workspace.history {
            "会话记录"
        } else {
            match self.workspace.panel {
                1 => "模型与账号",
                2 => "对话附件",
                3 | 6 => "预览",
                4 => "我的作品",
                5 => "模型与账号",
                7 | 8 | 9 => "个人记忆",
                _ => "Amoo",
            }
        };
        let header = self.between(
            self.heading(title),
            self.ws_button(
                "close-side",
                "关闭",
                UiIcon::Close,
                W::ClosePanel,
                Btn::Ghost,
            ),
        );
        let body = if self.workspace.history {
            self.workspace_history()
        } else {
            match self.workspace.panel {
                1 => self.workspace_model(),
                2 => self.workspace_resources(),
                3 | 6 => self.workspace_preview(),
                4 => self.workspace_projects(),
                5 => self.studio_lab_settings(),
                7 | 8 | 9 => self.workspace_memory_and_safety(),
                _ => self.workspace_resources(),
            }
        };
        motion::enter(
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_spacing(S4)
                    .with_child(header)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.pane_scroll(
                                self.workspace.preview_scroll.clone(),
                                Container::new(body).with_padding_bottom(S4).finish(),
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
            )
            .with_uniform_padding(S4)
            .with_background_color(p.sidebar)
            .with_border(Border::left(1.).with_border_color(p.border))
            .finish(),
            self.workspace.panel_entered,
            !self.settings.value.reduced_motion,
        )
    }
    fn workspace_models(&self) -> El {
        let p = self.p();
        self.vstack(
            S4,
            vec![
                self.panel(vec![
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(UiIcon::Smartphone, 20., p.text),
                            self.heading("手机 Amoo"),
                        ],
                    ),
                    self.text(
                        "聊天、看图、做文件和生成图片都用手机账号，模型由手机自动选择。",
                        BODY,
                        p.text2,
                    ),
                    self.button(
                        "workspace-usage",
                        "查看账号与用量",
                        Action::Page(Page::Account),
                        Btn::Ghost,
                    ),
                ]),
                self.panel(vec![
                    self.hstack(
                        S2,
                        vec![
                            self.ui_icon(UiIcon::Cpu, 20., p.text),
                            self.heading("自己的模型服务（可选）"),
                        ],
                    ),
                    self.text(
                        if self.studio_lab.config.has_key {
                            format!("{} · 用于做应用", self.studio_lab.config.model)
                        } else if self.selected.is_some() {
                            "不用设置：连接手机后直接用手机 Amoo".into()
                        } else {
                            "连接手机即可使用，也可以填自己的模型服务".into()
                        },
                        BODY,
                        p.text2,
                    ),
                    self.ws_button(
                        "api-settings",
                        "设置",
                        UiIcon::Settings,
                        W::Settings,
                        Btn::Secondary,
                    ),
                ]),
            ],
        )
    }
    fn workspace_resources(&self) -> El {
        let p = self.p();
        let mut rows = vec![
            self.button_icon(
                "workspace-add-local",
                "本机文件",
                UiIcon::Attach,
                Action::AmooAttachFile(false),
                Btn::Secondary,
            ),
            self.button_icon(
                "workspace-add-photo",
                "手机相册",
                UiIcon::Image,
                Action::AmooAttachPhoneAlbum,
                Btn::Secondary,
            ),
            self.ws_button(
                "resource-folder",
                "任务文件夹",
                UiIcon::Folder,
                W::ChooseFolder,
                Btn::Secondary,
            ),
        ];
        if let Some(task) = self.workspace_task() {
            if !task.outputs.is_empty() {
                rows.push(self.divider());
                rows.push(self.heading("继续使用已有成果"));
                for (i, o) in task.outputs.iter().enumerate() {
                    rows.push(self.ws_button(
                        &format!("attach-output-{i}"),
                        &o.name,
                        UiIcon::Plus,
                        W::AddOutput(i),
                        Btn::Ghost,
                    ));
                }
            }
        }
        rows.push(self.text(
            "发图片、文档或表格与 Amoo 对话，当前对话与草稿会自动保留。",
            CAPTION,
            p.text3,
        ));
        self.vstack(S3, rows)
    }
    fn workspace_projects(&self) -> El {
        let p = self.p();
        let source_tabs = self.segmented(
            "workspace-studio-source",
            [("官网项目", 0), ("手机可用", 1), ("本机创作", 2)]
                .iter()
                .map(|(label, idx)| {
                    (
                        (*label).into(),
                        self.studio.source == *idx,
                        Action::Studio(StudioAction::Source(*idx)),
                    )
                })
                .collect(),
        );
        let mut rows = vec![source_tabs];
        match self.studio.source {
            0 => {
                rows.push(self.text(
                    "连接手机上的 Studio 官网账号，自动读取项目、技能和智能体。",
                    CAPTION,
                    p.text2,
                ));
                rows.push(self.wrap(vec![
                    self.button_icon(
                        "workspace-studio-refresh",
                        "同步官网",
                        UiIcon::Refresh,
                        Action::Studio(StudioAction::CloudRefresh),
                        Btn::Secondary,
                    ),
                    self.button(
                        "workspace-studio-login",
                        "连接官网账号",
                        Action::Studio(StudioAction::WebsiteLogin),
                        Btn::Ghost,
                    ),
                ]));
                if let Some(web) = self.studio.web.as_ref().filter(|w| w.authorized) {
                    rows.push(self.line(
                        format!("已连接 · {} 个项目", web.items.len()),
                        CAPTION,
                        p.success,
                    ));
                    for (i, item) in web.items.iter().enumerate().take(60) {
                        rows.push(self.panel(vec![
                            self.line_w(&item.name, BODY, p.text, Weight::Medium),
                            self.text(
                                format!("{} · {}", item.category, item.description),
                                CAPTION,
                                p.text2,
                            ),
                            self.ws_button(
                                &format!("open-project-{i}"),
                                "继续编辑",
                                UiIcon::ExternalLink,
                                W::OpenProject(i),
                                Btn::Secondary,
                            ),
                        ]));
                    }
                } else {
                    rows.push(self.text(
                        self.studio.web_error.as_deref().unwrap_or(
                            "首次连接需要在手机官网完成登录，之后使用已保存的登录会话。",
                        ),
                        CAPTION,
                        p.text2,
                    ));
                }
            }
            1 => {
                rows.push(self.text(
                    "读取手机端已授权的 Studio 应用、智能体与伴侣作品库，支持在手机端直接打开或带入当前对话。",
                    CAPTION,
                    p.text2,
                ));
                rows.push(self.wrap(vec![
                    self.button_icon(
                        "workspace-studio-phone-refresh",
                        "刷新手机列表",
                        UiIcon::Refresh,
                        Action::Studio(StudioAction::Refresh),
                        Btn::Secondary,
                    ),
                    self.button(
                        "workspace-studio-phone-lib",
                        "打开手机作品库",
                        Action::Studio(StudioAction::PhoneLibrary),
                        Btn::Ghost,
                    ),
                    self.button(
                        "workspace-studio-connect-acc",
                        "关联手机账号",
                        Action::Studio(StudioAction::ConnectAccount),
                        Btn::Ghost,
                    ),
                ]));
                if let Some(cloud) = &self.studio.cloud {
                    if cloud.items.is_empty() {
                        rows.push(self.text(
                            "手机暂未同步可用 Studio 作品，可先在「本机创作」生成并发送到手机。",
                            CAPTION,
                            p.text2,
                        ));
                    } else {
                        for (i, item) in cloud.items.iter().enumerate().take(40) {
                            rows.push(self.panel(vec![
                                self.between(
                                    self.line_w(&item.name, BODY, p.text, Weight::Medium),
                                    self.pill_badge(&item.category, p.accent, p.surface),
                                ),
                                self.text(&item.description, CAPTION, p.text2),
                                self.button_icon(
                                    format!("workspace-studio-use-phone-{i}"),
                                    "在手机使用",
                                    UiIcon::Smartphone,
                                    Action::Studio(StudioAction::UsePhone(i)),
                                    Btn::Secondary,
                                ),
                            ]));
                        }
                    }
                } else {
                    rows.push(self.text(
                        "点击「刷新手机列表」读取手机端已安装的 Studio 作品与能力。",
                        CAPTION,
                        p.text2,
                    ));
                }
            }
            _ => {
                rows.push(self.panel(vec![
                    self.line_w("做个小应用", BODY, p.text, Weight::Semibold),
                    self.text(
                        "做好后会自动试运行检查，可装到手机离线用。",
                        CAPTION,
                        p.text2,
                    ),
                    self.wrap(
                        crate::ui::studio_lab_state::LAB_TEMPLATES
                            .iter()
                            .enumerate()
                            .map(|(i, (title, _))| {
                                self.button(
                                    format!("workspace-lab-tpl-{i}"),
                                    title.to_string(),
                                    Action::StudioLab(StudioLabAction::Template(i)),
                                    Btn::Secondary,
                                )
                            })
                            .collect(),
                    ),
                    self.wrap(vec![
                        self.ws_button(
                            "studio-lab-new-app",
                            "在对话里做一个",
                            UiIcon::Bolt,
                            W::Suggest(4),
                            Btn::Primary,
                        ),
                    ]),
                ]));
                if !self.studio_lab.artifacts.is_empty() {
                    rows.push(self.heading("我做过的应用"));
                    for (i, a) in self.studio_lab.artifacts.iter().enumerate().take(24) {
                        rows.push(self.panel(vec![
                            self.between(
                                self.line_w(&a.name, BODY, p.text, Weight::Medium),
                                self.pill_badge(
                                    if a.passed {
                                        "已检查"
                                    } else {
                                        "需要修复"
                                    },
                                    if a.passed { p.success } else { p.warning },
                                    p.surface,
                                ),
                            ),
                            self.text(&a.summary, CAPTION, p.text2),
                            self.wrap(vec![
                                self.ws_button(
                                    &format!("lab-select-{}", a.id),
                                    "查看",
                                    UiIcon::Apps,
                                    W::SelectLab(a.id.clone()),
                                    Btn::Secondary,
                                ),
                                self.ws_button(
                                    &format!("lab-preview-{}", a.id),
                                    "运行预览",
                                    UiIcon::Monitor,
                                    W::StudioPreviewArtifact(i),
                                    Btn::Ghost,
                                ),
                                self.ws_button(
                                    &format!("lab-phone-{}", a.id),
                                    "发送到手机",
                                    UiIcon::Smartphone,
                                    W::StudioDeliverArtifact(i),
                                    Btn::Ghost,
                                ),
                            ]),
                        ]));
                    }
                }
                if !self.studio.works.is_empty() {
                    rows.push(self.heading("手机上已装的作品"));
                    for (i, w) in self.studio.works.iter().enumerate().take(20) {
                        rows.push(self.panel(vec![
                            self.line_w(&w.name, BODY, p.text, Weight::Medium),
                            self.text(format!("{} · {}", w.category, w.updated), CAPTION, p.text2),
                            self.button(
                                format!("workspace-studio-open-local-{i}"),
                                "选择并同步",
                                Action::Studio(StudioAction::Select(i)),
                                Btn::Secondary,
                            ),
                        ]));
                    }
                }
            }
        }
        self.vstack(S3, rows)
    }
    fn workspace_memory_and_safety(&self) -> El {
        let p = self.p();
        let mem_count = self.amoo_memories.len();
        let active_cat = self.workspace.memory_category;
        let mut memory_rows = vec![
            self.between(
                self.line_w(
                    "Amoo 个人记忆中心",
                    BODY,
                    p.text,
                    Weight::Semibold,
                ),
                self.pill_badge(
                    &format!(
                        "{mem_count} 条记忆 · {}",
                        if self.amoo_memory_enabled {
                            "自动关联中"
                        } else {
                            "已暂停关联"
                        }
                    ),
                    if self.amoo_memory_enabled {
                        p.success
                    } else {
                        p.text3
                    },
                    p.surface,
                ),
            ),
            self.text(
                "同步管理手机 Amoo 的「关于我 / 交流偏好 / 备忘事项」三类记忆，提问时自动带入偏好。",
                CAPTION,
                p.text2,
            ),
            self.wrap(
                [
                    (0u8, "全部"),
                    (1u8, "关于我"),
                    (2u8, "交流偏好"),
                    (3u8, "备忘事项"),
                ]
                .into_iter()
                .map(|(cat, label)| {
                    self.ws_button(
                        &format!("memory-cat-{cat}"),
                        label,
                        UiIcon::Star,
                        W::MemoryCategory(cat),
                        if active_cat == cat {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    )
                })
                .collect(),
            ),
            self.input(
                F_AMOO_MEMORY_INPUT,
                "输入想让 Amoo 记住的个人习惯、称呼偏好或备忘事项…",
                None,
            ),
            self.wrap(vec![
                self.ws_button(
                    "memory-add-btn",
                    "＋ 添加记忆",
                    UiIcon::Plus,
                    W::AddMemory,
                    Btn::Primary,
                ),
                self.ws_button(
                    "memory-toggle-auto",
                    if self.amoo_memory_enabled {
                        "已开启自动记忆关联"
                    } else {
                        "开启自动记忆关联"
                    },
                    UiIcon::Check,
                    W::ToggleMemory,
                    if self.amoo_memory_enabled {
                        Btn::Secondary
                    } else {
                        Btn::Ghost
                    },
                ),
                self.button_icon(
                    "workspace-open-phone-memory",
                    "手机记忆中心",
                    UiIcon::Smartphone,
                    Action::BackScreen(BackScreenAction::TriggerMcpQuickAction(
                        "open_amoo_memory".into(),
                    )),
                    Btn::Ghost,
                ),
            ]),
        ];
        let mut shown_memories = 0usize;
        for (idx, mem) in self.amoo_memories.iter().enumerate() {
            let matches_cat = match active_cat {
                1 => mem.sub.contains("关于我") || mem.sub.contains("画像"),
                2 => {
                    mem.sub.contains("偏好")
                        || mem.title.contains("偏好")
                        || mem.title.contains("风格")
                }
                3 => {
                    mem.sub.contains("备忘")
                        || mem.title.contains("待办")
                        || mem.title.contains("提醒")
                }
                _ => true,
            };
            if !matches_cat {
                continue;
            }
            shown_memories += 1;
            memory_rows.push(self.panel(vec![
                self.line_w(&mem.title, BODY, p.text, Weight::Medium),
                self.text(&mem.sub, CAPTION, p.text2),
                self.wrap(vec![
                    self.ws_button(
                        &format!("memory-inject-{idx}"),
                        "带入当前对话",
                        UiIcon::Star,
                        W::UseMemory(mem.title.clone()),
                        Btn::Secondary,
                    ),
                    self.ws_button(
                        &format!("memory-delete-{idx}"),
                        "删除",
                        UiIcon::Trash,
                        W::DeleteMemory(idx),
                        Btn::Ghost,
                    ),
                ]),
            ]));
        }
        if shown_memories == 0 {
            memory_rows.push(self.text(
                "当前分类下暂无记忆，可在上方输入框直接添加。",
                CAPTION,
                p.text3,
            ));
        }

        let persona_card = self.panel(vec![
            self.line_w("Amoo 角色人设与沟通风格", BODY, p.text, Weight::Semibold),
            self.text(
                "切换 Amoo 在对话、文档生成与手机操控时的角色定位和回复风格。",
                CAPTION,
                p.text2,
            ),
            self.line("角色定位", MICRO, p.text3),
            self.wrap(
                [
                    (0, "Amoo 全能助手"),
                    (1, "桌面交付伙伴"),
                    (2, "办公协同专员"),
                    (3, "研发架构专家"),
                ]
                .into_iter()
                .map(|(idx, label)| {
                    self.ws_button(
                        &format!("persona-role-{idx}"),
                        label,
                        UiIcon::Star,
                        W::SelectAgentRole(idx),
                        if self.amoo_agent_idx == idx {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    )
                })
                .collect(),
            ),
            self.line("沟通风格", MICRO, p.text3),
            self.wrap(
                [
                    (0, "直接精炼 · 不长篇解释"),
                    (1, "温和详细 · 分步拆解"),
                    (2, "极客专业 · 附带命令"),
                ]
                .into_iter()
                .map(|(idx, label)| {
                    self.ws_button(
                        &format!("persona-style-{idx}"),
                        label,
                        UiIcon::Chat,
                        W::SelectPersona(idx),
                        if self.amoo_style_idx == idx {
                            Btn::Primary
                        } else {
                            Btn::Secondary
                        },
                    )
                })
                .collect(),
            ),
        ]);

        self.vstack(S3, vec![self.panel(memory_rows), persona_card])
    }
    fn workspace_preview(&self) -> El {
        let p = self.p();
        let Some(output) = self
            .workspace_task()
            .and_then(|t| t.outputs.get(self.workspace.selected_output?))
        else {
            return self.empty(
                "还没有选中文件",
                "点一下对话里的文件或应用，就能在这里预览。",
                None,
            );
        };
        let mut rows = vec![self.line_w(&output.name, SUBHEAD, p.text, Weight::Semibold)];
        if let Some(id) = &output.lab_id {
            if let Some(a) = self.studio_lab.artifacts.iter().find(|a| &a.id == id) {
                rows.push(self.pill_badge(
                    if a.passed {
                        "已检查"
                    } else {
                        "需要修复"
                    },
                    if a.passed { p.success } else { p.warning },
                    p.surface,
                ));
                if let Some(path) = &a.screenshot_path {
                    let zoom = self.workspace.preview_zoom.clamp(100, 300);
                    let scaled_h = 260.0 * (zoom as f32 / 100.0);
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
                        .with_height(scaled_h)
                        .finish(),
                    );
                    if cached_is_file(path) {
                        rows.push(self.button(
                            "workspace-app-screenshot-zoom",
                            "查看大图",
                            Action::Work(WorkAction::OpenPreview(
                                path.to_string_lossy().into_owned(),
                            )),
                            Btn::Ghost,
                        ));
                    }
                }
                rows.push(self.button_icon(
                    "workspace-app-interact",
                    "运行预览",
                    UiIcon::Monitor,
                    Action::StudioLab(StudioLabAction::Preview),
                    Btn::Secondary,
                ));
                rows.push(self.text(&a.summary, CAPTION, p.text2));
                if let Some((report_id, report)) = &self.studio_lab.report {
                    if report_id == id {
                        if let Some(checks) = report["checks"].as_array() {
                            for check in checks.iter().take(12) {
                                let pass = check["passed"].as_bool().unwrap_or(false);
                                rows.push(self.hstack(
                                    S2,
                                    vec![
                                            self.ui_icon(
                                                if pass { UiIcon::Check } else { UiIcon::Info },
                                                14.,
                                                if pass { p.success } else { p.warning },
                                            ),
                                            Expanded::new(
                                                1.,
                                                self.text(
                                                    check["label"].as_str().unwrap_or("检查项"),
                                                    CAPTION,
                                                    p.text2,
                                                ),
                                            )
                                            .finish(),
                                        ],
                                ));
                            }
                        }
                    }
                }
                rows.push(self.button(
                    "workspace-app-folder",
                    "打开作品文件夹",
                    Action::StudioLab(StudioLabAction::OpenFolder),
                    Btn::Ghost,
                ));
                rows.push(self.text("已存在电脑上；发送到手机后可以离线使用。", CAPTION, p.text3));
                if a.passed {
                    rows.push(self.ws_button(
                        "phone-delivery",
                        "发送到手机",
                        UiIcon::Smartphone,
                        W::Deliver(false),
                        Btn::Primary,
                    ));
                    rows.push(self.ws_button(
                        "cloud-delivery",
                        "同步到 Studio",
                        UiIcon::Upload,
                        W::Deliver(true),
                        Btn::Secondary,
                    ));
                } else {
                    rows.push(self.ws_button(
                        "fix-app",
                        "修复这一项",
                        UiIcon::Retry,
                        W::Retry,
                        Btn::Secondary,
                    ));
                }
                if let Some(note) = self.studio_lab_phone_note() {
                    rows.push(match note {
                        Ok(n) => self.text(n, CAPTION, p.success),
                        Err(n) => self.text(n, CAPTION, p.danger),
                    });
                }
                if let Some(feedback) = self.studio_lab_cloud_feedback() {
                    if let Some(note) = &feedback.note {
                        rows.push(self.text(note, CAPTION, p.success));
                    }
                    if let Some(error) = &feedback.error {
                        rows.push(self.text(error, CAPTION, p.danger));
                    }
                    if feedback.receipt.is_some() || feedback.recovery_url.is_some() {
                        rows.push(self.button(
                            "workspace-open-cloud",
                            "继续编辑官网项目",
                            Action::StudioLab(StudioLabAction::OpenCloud),
                            Btn::Secondary,
                        ));
                    }
                }
            } else {
                rows.push(self.text("正在读取作品记录…", CAPTION, p.text3));
            }
        } else {
            if let Some(preview) = self
                .work
                .local_preview
                .as_ref()
                .filter(|v| v.path == output.path)
            {
                if preview.image {
                    let zoom = self.workspace.preview_zoom.clamp(100, 300);
                    let scaled_h = 260.0 * (zoom as f32 / 100.0);
                    let path_str = preview.path.to_string_lossy().into_owned();
                    if let Some((w, h)) = cached_image_dimensions(&preview.path) {
                        rows.push(self.wrap(vec![
                            self.pill_badge(format!("{w} × {h} px"), p.text2, p.tag_bg),
                            self.pill_badge(format!("缩放 {zoom}%"), p.accent, p.accent_soft),
                        ]));
                    }
                    rows.push(
                        ConstrainedBox::new(
                            Image::new(
                                AssetSource::LocalFile {
                                    path: path_str.clone(),
                                },
                                CacheOption::BySize,
                            )
                            .contain()
                            .finish(),
                        )
                        .with_height(scaled_h)
                        .finish(),
                    );
                    let mut zoom_bar = Vec::new();
                    for preset in [100u16, 150, 200, 300] {
                        zoom_bar.push(self.button(
                            format!("workspace-preview-zoom-{preset}"),
                            format!("{preset}%"),
                            Action::Workspace(W::PreviewZoom(preset)),
                            if zoom == preset {
                                Btn::Primary
                            } else {
                                Btn::Ghost
                            },
                        ));
                    }
                    zoom_bar.push(self.button(
                        "workspace-preview-fullscreen",
                        "查看大图",
                        Action::Work(WorkAction::OpenPreview(path_str)),
                        Btn::Secondary,
                    ));
                    rows.push(self.wrap(zoom_bar));
                }
                if let Some(text) = &preview.text {
                    let body = match text.char_indices().nth(6000) {
                        Some((idx, _)) => &text[..idx],
                        None => text.as_str(),
                    };
                    rows.push(
                        Container::new(self.work_rich_text(body, "workspace-preview-text"))
                            .with_uniform_padding(S3)
                            .with_background_color(p.bg)
                            .with_border(Border::all(1.).with_border_color(p.border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish(),
                    );
                }
                rows.push(self.text(&preview.note, CAPTION, p.text3));
                let mut file_actions = vec![
                    self.button_icon(
                        "workspace-open-file",
                        "打开",
                        UiIcon::ExternalLink,
                        Action::Work(WorkAction::OpenLocalPreview),
                        Btn::Secondary,
                    ),
                    self.button_icon(
                        "workspace-export-file",
                        "另存为",
                        UiIcon::Download,
                        Action::Work(WorkAction::ExportLocalPreview),
                        Btn::Secondary,
                    ),
                    self.button(
                        "workspace-copy-file-path",
                        "复制路径",
                        Action::CopyText(
                            preview.path.display().to_string(),
                            "已复制文件路径".into(),
                        ),
                        Btn::Secondary,
                    ),
                ];
                if let Some(text) = &preview.text {
                    file_actions.push(self.button(
                        "workspace-copy-preview-text",
                        "复制内容",
                        Action::CopyText(text.clone(), "已复制文件内容".into()),
                        Btn::Secondary,
                    ));
                }
                rows.push(self.wrap(file_actions));
                rows.push(self.ws_button(
                    "file-phone",
                    "发送到手机",
                    UiIcon::Smartphone,
                    W::Deliver(false),
                    Btn::Primary,
                ));
            } else {
                rows.push(
                    self.text(
                        self.work
                            .local_preview_error
                            .as_deref()
                            .unwrap_or("正在读取文件预览…"),
                        CAPTION,
                        p.text2,
                    ),
                );
            }
        }
        if let Some(task) = self.workspace_task() {
            for receipt in task
                .receipts
                .iter()
                .rev()
                .filter(|r| r.path == output.path)
                .take(3)
            {
                rows.push(self.text(
                    &receipt.note,
                    CAPTION,
                    if receipt.success {
                        p.success
                    } else {
                        p.warning
                    },
                ));
            }
        }
        if let Some(error) = &self.studio_lab.error {
            if output.lab_id.is_some() {
                rows.push(self.text(error, CAPTION, p.danger));
            }
        }
        if let Some(cloud) = self.workspace.delivery {
            rows.push(self.divider());
            rows.push(self.heading(if cloud {
                "创建官网草稿？"
            } else {
                "发送到手机？"
            }));
            rows.push(self.text(
                if cloud {
                    "只创建草稿，不会公开发布。".into()
                } else {
                    format!(
                        "目标：{}\n保存到：{}",
                        self.selected
                            .as_ref()
                            .map(|d| d.model.as_str())
                            .unwrap_or("未连接手机"),
                        if output.lab_id.is_some() {
                            "StepX伴侣 → 作品"
                        } else {
                            "StepX伴侣 → 文件 → 接收文件"
                        }
                    )
                },
                BODY,
                p.text2,
            ));
            if cloud {
                rows.push(self.button(
                    "workspace-get-cloud-models",
                    "读取官网模型",
                    Action::StudioLab(StudioLabAction::CloudModels),
                    Btn::Secondary,
                ));
                for m in &self.studio_lab.cloud_models {
                    let id = format!("workspace-cloud-model-{}", m.id);
                    let action = Action::StudioLab(StudioLabAction::CloudModel(m.id.clone()));
                    rows.push(if self.studio_lab.cloud_model.as_ref() == Some(&m.id) {
                        self.button_icon(id, &m.name, UiIcon::Check, action, Btn::Ghost)
                    } else {
                        self.button(id, &m.name, action, Btn::Ghost)
                    });
                }
            } else if output.lab_id.is_some() {
                rows.push(self.ws_button(
                    "open-after",
                    if self.workspace.open_phone {
                        "已勾选：发送后在手机打开"
                    } else {
                        "发送后在手机打开"
                    },
                    if self.workspace.open_phone {
                        UiIcon::Check
                    } else {
                        UiIcon::Smartphone
                    },
                    W::ToggleOpenPhone,
                    Btn::Ghost,
                ));
            }
            rows.push(self.hstack(
                S2,
                vec![
                    self.ws_button(
                        "cancel-delivery",
                        "取消",
                        UiIcon::Close,
                        W::CancelDelivery,
                        Btn::Ghost,
                    ),
                    if self.busy || self.selected.is_none() {
                        self.disabled_button(if self.busy {
                            "正在处理"
                        } else {
                            "请先连接手机"
                        })
                    } else {
                        self.ws_button(
                            "confirm-delivery",
                            if cloud { "创建草稿" } else { "发送" },
                            UiIcon::Check,
                            W::ConfirmDelivery,
                            Btn::Primary,
                        )
                    },
                ],
            ));
        }
        if let Some(e) = &self.workspace.error {
            rows.push(self.workspace_error_banner(e));
        }
        self.vstack(S3, rows)
    }
}

/// Plain label for a step in the inline progress stepper.
fn step_label(executor: Executor) -> &'static str {
    match executor {
        Executor::Phone => "等手机回复",
        Executor::Files => "生成文件",
        Executor::Image => "生成图片",
        Executor::App => "制作并检查应用",
        Executor::Screen => "生成背屏图案",
    }
}

/// Verb phrase for "我会先{…}，再{…}" in the start bar.
fn proposal_step_label(executor: Executor) -> &'static str {
    match executor {
        Executor::Phone => "让手机 Amoo 回复",
        Executor::Files => "生成文件",
        Executor::Image => "生成图片",
        Executor::App => "做好应用并自动检查",
        Executor::Screen => "生成背屏图案",
    }
}

/// Toggle key (in `amoo_expanded_tool_records`) for the header "…" menu.
const HEADER_MORE_KEY: &str = "workspace-header-more";
