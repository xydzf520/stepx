use super::super::{
    amoo_display::{retry_prompt, session_title, ToolRecord},
    icons::UiIcon,
    theme::*,
    widgets::*,
    Action, AmooChatItem, AmooUiAction, Companion, SafetyDecision, F_AMOO_GUI_APP, F_AMOO_SEARCH,
};
use facloud_ui::{
    assets::asset_cache::AssetSource, color::ColorU, elements::*, fonts::Weight, platform::Cursor,
};
use pathfinder_geometry::vector::vec2f;
use stepx_companion::{amoo_gui::GuiRunState, device::BackScreenPattern};

fn ensure_amoo_tti_portrait_path() -> String {
    static PORTRAIT_PATH: std::sync::OnceLock<String> = std::sync::OnceLock::new();
    PORTRAIT_PATH
        .get_or_init(|| {
            const PORTRAIT_PNG: &[u8] = include_bytes!("../../../assets/amoo_tti_portrait.png");
            let path = std::env::temp_dir().join("stepx_amoo_tti_portrait.png");
            if !path.exists() {
                let _ = std::fs::write(&path, PORTRAIT_PNG);
            }
            path.to_string_lossy().into_owned()
        })
        .clone()
}

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

const CAPABILITY_PILLS: [(&str, &str); 5] = [
    ("深度研究", "请针对以下主题进行深度研究并整理核心结论："),
    (
        "背屏图案",
        "帮我设计一个 23×17 的背屏赛博闪电像素图案并同步到背屏",
    ),
    (
        "AI 剪辑",
        "分析手机相册最新视频并生成 15 秒卡点高光剪辑方案",
    ),
    ("拍摄指导", "开启拍摄构图指导，并在背屏显示 3 秒倒计时闪光"),
    ("帮我写", "帮我起草一段礼貌简洁的会议改期确认回复："),
];

// A finished model turn is not proof that a phone operation succeeded. Only
// inspect the latest substantive assistant reply, never tool logs or old turns.
fn session_waits_for_unlock(items: &[AmooChatItem]) -> bool {
    for item in items.iter().rev() {
        match item {
            AmooChatItem::UserText { .. } | AmooChatItem::UserVoice { .. } => return false,
            AmooChatItem::AssistantRich {
                text: Some(body), ..
            } => {
                let Some(body) = stepx_companion::work_protocol::display_text(body, false)
                    .filter(|text| !text.trim().is_empty())
                else {
                    continue;
                };
                return reply_reports_lock_block(&body);
            }
            _ => {}
        }
    }
    false
}

fn reply_reports_lock_block(body: &str) -> bool {
    if ["解锁后已完成", "解锁后已经完成", "现已解锁并完成"]
        .iter()
        .any(|phrase| body.contains(phrase))
    {
        return false;
    }
    let locked = ["锁屏", "已锁定", "锁着", "手机锁定"]
        .iter()
        .any(|phrase| body.contains(phrase));
    let blocked = [
        "拦截",
        "被拦",
        "阻止",
        "无法",
        "不能",
        "失败",
        "卡在",
        "未能",
        "还没完成",
        "无法完成",
    ]
    .iter()
    .any(|phrase| body.contains(phrase));
    let unlock_request = [
        "请先解锁",
        "请解锁",
        "需要先解锁",
        "等你解锁",
        "手动解锁后",
        "你先解锁",
    ]
    .iter()
    .any(|phrase| body.contains(phrase));
    (locked && blocked) || unlock_request
}

impl Companion {
    pub(super) fn amoo_action_icon(
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
            let t = self.hover_t(&mouse);
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
            let tooltip = Container::new(self.line(label, 11., p.text))
                .with_horizontal_padding(8.)
                .with_vertical_padding(5.)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
            Stack::new()
                .with_child(button)
                .with_positioned_child(
                    tooltip,
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

    fn amoo_execution_records(&self, key: &str, records: &[ToolRecord]) -> El {
        let p = self.p();
        let expanded = self.amoo_expanded_tool_records.contains(key);
        let authorization_failed = records.iter().any(|record| {
            record
                .detail
                .as_deref()
                .is_some_and(|detail| detail.starts_with("授权超时："))
        });
        let action = Action::AmooToggleToolRecords(key.into());
        let mut names = Vec::<(String, usize)>::new();
        for record in records {
            if let Some((_, count)) = names.iter_mut().find(|(name, _)| *name == record.name) {
                *count += 1;
            } else {
                names.push((record.name.clone(), 1));
            }
        }
        let summary = names
            .iter()
            .map(|(name, count)| {
                if *count == 1 {
                    name.clone()
                } else {
                    format!("{name} × {count}")
                }
            })
            .collect::<Vec<_>>()
            .join(" · ");
        let header = Hoverable::new(
            self.mouse_state(format!("amoo-records:{key}")),
            move |mouse| {
                let _hovered = mouse.is_mouse_over_element();
                let t = self.hover_t(&mouse);
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else {
                    mix_color(p.surface, p.hover, t)
                };
                Container::new(self.hstack(
                    8.,
                    vec![
                        self.ui_icon(
                            if expanded {
                                UiIcon::ChevronDown
                            } else {
                                UiIcon::ChevronRight
                            },
                            14.,
                            p.text2,
                        ),
                        self.line_w(
                            format!(
                                "执行记录 · {} 条{}",
                                records.len(),
                                if authorization_failed {
                                    " · 授权超时"
                                } else {
                                    ""
                                }
                            ),
                            CAPTION,
                            p.text2,
                            Weight::Medium,
                        ),
                        Expanded::new(1., self.line(summary, 11., p.text3)).finish(),
                    ],
                ))
                .with_uniform_padding(10.)
                .with_background_color(bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish()
            },
        )
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let mut children = vec![header];
        if expanded {
            let entries = records
                .iter()
                .enumerate()
                .map(|(index, record)| {
                    let mut parts = vec![self.between(
                        self.hstack(
                            8.,
                            vec![
                                self.mono(format!("{:02}", index + 1), 11., p.text3),
                                self.line(record.name.clone(), CAPTION, p.text2),
                            ],
                        ),
                        self.mono(record.time_label.clone(), 11., p.text3),
                    )];
                    if let Some(detail) = record.detail.as_ref().filter(|s| !s.is_empty()) {
                        parts.push(self.text(detail.clone(), 11., p.text2));
                    }
                    Container::new(self.vstack(4., parts))
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(8.)
                        .with_border(Border::top(1.).with_border_color(p.surface_border))
                        .finish()
                })
                .collect();
            children.push(self.vstack(0., entries));
        }
        Container::new(
            Container::new(self.vstack(0., children))
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.surface_border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
        )
        .with_margin_left(34.)
        .finish()
    }

    pub(in super::super) fn amoo_actions(&self) -> Vec<El> {
        if !self.demo_mode {
            return vec![
                self.button(
                    "amoo-schedules",
                    if self.amoo_show_schedules {
                        "返回对话"
                    } else {
                        "定时任务"
                    },
                    Action::AmooToggleSchedules,
                    Btn::Secondary,
                ),
                self.button(
                    "amoo-mode-phone",
                    if self.amoo_ui.screen_mode {
                        "返回对话"
                    } else {
                        "操作手机"
                    },
                    Action::AmooUi(AmooUiAction::GuiMode(!self.amoo_ui.screen_mode)),
                    Btn::Secondary,
                ),
                self.button(
                    "amoo-sync-real",
                    "刷新",
                    Action::AmooSnapshot,
                    Btn::Secondary,
                ),
                self.button(
                    "amoo-new-chat",
                    "新建对话",
                    Action::AmooNewChat,
                    Btn::Primary,
                ),
            ];
        }
        let mut actions = vec![self.button(
            "amoo-schedules",
            if self.amoo_show_schedules {
                "返回对话"
            } else {
                "定时任务"
            },
            Action::AmooToggleSchedules,
            Btn::Secondary,
        )];
        actions.push(self.button_icon(
            "amoo-express-quick",
            "查快递 (2)",
            UiIcon::Package,
            Action::AmooToggleExpressCard,
            Btn::Secondary,
        ));
        actions.push(self.button_icon(
            "amoo-read",
            "圈选感知",
            UiIcon::Plus,
            Action::ReadVisible,
            Btn::Secondary,
        ));
        actions.push(self.button(
            "amoo-details",
            if self.amoo_show_details {
                "收起详情"
            } else {
                "会话详情"
            },
            Action::AmooToggleDetails,
            Btn::Secondary,
        ));
        actions.push(self.button_icon(
            "amoo-new-chat",
            "新建对话",
            UiIcon::Plus,
            Action::AmooNewChat,
            Btn::Primary,
        ));
        actions
    }

    fn agent_chip(&self, idx: usize, label: &'static str) -> El {
        let p = self.p();
        let active = self.amoo_agent_idx == idx;
        let state = self.mouse_state(format!("amoo-agent:{idx}"));
        let row = if active {
            self.lead(
                6.,
                self.dot(p.success),
                self.line_w(label, CAPTION, p.accent_fg, Weight::Medium),
            )
        } else {
            self.line(label, CAPTION, p.text2)
        };
        Hoverable::new(state, move |mouse| {
            let _hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let bg = if active {
                if mouse.is_clicked() {
                    p.accent_hover
                } else {
                    mix_color(p.accent, p.accent_hover, t)
                }
            } else if mouse.is_clicked() {
                p.accent_soft
            } else {
                mix_color(p.surface, p.hover, t)
            };
            let border = if active {
                p.accent
            } else if mouse.is_clicked() {
                p.border_strong
            } else {
                mix_color(p.surface_border, p.border_strong, t)
            };
            Container::new(row)
                .with_horizontal_padding(10.)
                .with_vertical_padding(6.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(Action::AmooSelectAgent(idx)))
        .finish()
    }

    fn session_card(
        &self,
        idx: usize,
        title: String,
        agent: String,
        status: String,
        status_kind: u8,
        meta: String,
    ) -> El {
        let p = self.p();
        let active = self.amoo_session_idx == idx;
        let state = self.mouse_state(format!("amoo-sess-card:{idx}"));
        let menu_title = title.clone();
        let badge = match status_kind {
            1 => Container::new(self.line_w(status, 11., p.warning, Weight::Medium))
                .with_horizontal_padding(6.)
                .with_vertical_padding(2.)
                .with_background_color(rgba(0xF5, 0x9E, 0x0B, 26))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
            2 => Container::new(self.line_w(status, 11., p.success_text, Weight::Medium))
                .with_horizontal_padding(6.)
                .with_vertical_padding(2.)
                .with_background_color(p.success_soft)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                .finish(),
            _ => self.line(status, 11., p.text2),
        };
        let content = self.vstack(
            4.,
            vec![
                self.between(self.line_w(title, CAPTION, p.text, Weight::Semibold), badge),
                self.between(
                    self.line(agent, 11., p.text2),
                    self.mono(meta, 11., p.text2),
                ),
            ],
        );
        Hoverable::new(state, move |mouse| {
            let _hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let active_bg = if p.dark {
                rgb(0x22, 0x22, 0x27)
            } else {
                rgb(0xF4, 0xF4, 0xF6)
            };
            let bg = if mouse.is_clicked() {
                p.accent_soft
            } else if active {
                mix_color(active_bg, p.hover, t * 0.5)
            } else {
                mix_color(p.bg, p.hover, t)
            };
            let border = if active || mouse.is_clicked() {
                p.border_strong
            } else {
                mix_color(p.border, p.border_strong, t)
            };
            Container::new(content)
                .with_uniform_padding(10.)
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(Action::AmooSelectSession(idx)))
        .on_right_click(move |ctx, _, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                crate::ui::context_menu::Target::ChatSession {
                    workspace_task_id: None,
                    phone_session_idx: Some(idx),
                    title: menu_title.clone(),
                },
                at,
            ));
        })
        .finish()
    }

    pub(in super::super) fn compact_amoo_composer(&self) -> El {
        self.amoo_message_composer(true, false)
    }

    pub(in super::super) fn amoo_page(&self) -> El {
        if let Some(picker) = self.amoo_album_picker() {
            return self.pane_scroll(
                self.scroll_aux.clone(),
                Container::new(picker)
                    .with_horizontal_padding(S5)
                    .with_padding_top(S5)
                    .with_padding_bottom(S6)
                    .finish(),
            );
        }
        let p = self.p();
        let query = self.fields[F_AMOO_SEARCH].text.trim().to_lowercase();

        // 1. Col 1 (3 cols): Session Center, Parallel Tasks & Agent Switcher
        let agent_selector = Container::new(self.vstack(
            8.,
            vec![
                    self.between(
                        self.heading("手机助手"),
                        Container::new(self.line_w(
                            if self.demo_mode || self.amoo_gateway_online {
                                "在线"
                            } else {
                                "未连接"
                            },
                            11.,
                            p.success_text,
                            Weight::Medium,
                        ))
                        .with_horizontal_padding(6.)
                        .with_vertical_padding(2.)
                        .with_background_color(p.success_soft)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish(),
                    ),
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(6.)
                        .with_child(Expanded::new(1., self.agent_chip(0, "Amoo 助手")).finish())
                        .with_child(Expanded::new(1., self.agent_chip(1, "桌面伙伴")).finish())
                        .finish(),
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(6.)
                        .with_child(Expanded::new(1., self.agent_chip(2, "飞书 / 企微")).finish())
                        .with_child(Expanded::new(1., self.agent_chip(3, "Claude 协同")).finish())
                        .finish(),
                ],
        ))
        .with_uniform_padding(10.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let agent_selector = if self.demo_mode {
            agent_selector
        } else {
            Container::new(self.vstack(
                S2,
                vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.ui_icon(UiIcon::Star, 20., p.text),
                                self.heading("Amoo"),
                            ],
                        ),
                        self.pill_badge(
                            if self.amoo_gateway_online {
                                "已连接"
                            } else {
                                "未连接"
                            },
                            if self.amoo_gateway_online {
                                p.success_text
                            } else {
                                p.text3
                            },
                            if self.amoo_gateway_online {
                                p.success_soft
                            } else {
                                p.surface
                            },
                        ),
                    ),
                    self.text("与手机同步的日常对话", CAPTION, p.text2),
                ],
            ))
            .with_uniform_padding(S3)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
        };
        let search_box = self.input(F_AMOO_SEARCH, "搜索会话或定时任务…", None);

        let filtered_sessions: Vec<(usize, &super::super::AmooSessionItem)> = self
            .amoo_sessions
            .iter()
            .enumerate()
            .filter(|(_, s)| {
                query.is_empty()
                    || session_title(&s.title).to_lowercase().contains(&query)
                    || s.agent.to_lowercase().contains(&query)
                    || s.status.to_lowercase().contains(&query)
            })
            .collect();

        let raw_running = self
            .amoo_sessions
            .iter()
            .filter(|s| s.status == "执行中" || s.status == "进行中" || s.status == "请求中")
            .count();
        let running_label = if self.demo_mode {
            let c = self
                .amoo_sessions
                .iter()
                .filter(|s| s.status_kind == 2 || s.status == "执行中" || s.status == "进行中")
                .count()
                .max(1);
            format!("{c} 项运行中")
        } else if raw_running > 0 {
            format!("{raw_running} 项执行中")
        } else {
            format!("{} 个定时任务", self.amoo_cron_count)
        };

        let running_state = self.mouse_state("amoo-running-link");
        let running_badge = Hoverable::new(running_state, move |mouse| {
            let _hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let fg = if mouse.is_clicked() {
                p.accent
            } else {
                mix_color(p.success, p.text, t * 0.35)
            };
            self.line_w(running_label.clone(), 11., fg, Weight::Medium)
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::AmooOpenTasks))
        .finish();

        let sessions_header = Container::new(self.between(
            self.heading(format!("会话与任务 ({})", filtered_sessions.len())),
            running_badge,
        ))
        .with_horizontal_padding(4.)
        .finish();

        let mut session_cards: Vec<El> = Vec::new();
        let max_cards = if self.demo_mode { 6 } else { 50 };
        for (idx, sess) in filtered_sessions.into_iter().take(max_cards) {
            let waiting_unlock = !self.demo_mode && session_waits_for_unlock(&sess.items);
            session_cards.push(self.session_card(
                idx,
                session_title(&sess.title),
                sess.agent.clone(),
                if waiting_unlock {
                    "等待解锁".into()
                } else {
                    sess.status.clone()
                },
                if waiting_unlock { 1 } else { sess.status_kind },
                sess.meta.clone(),
            ));
        }

        let left_bottom = Container::new(
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_spacing(6.)
                .with_child(
                    Expanded::new(
                        1.,
                        self.button_icon(
                            "amoo-skill-store",
                            if self.amoo_skill_manage {
                                "完成管理"
                            } else {
                                "技能市场"
                            },
                            if self.amoo_skill_manage {
                                UiIcon::Check
                            } else {
                                UiIcon::Apps
                            },
                            if self.demo_mode {
                                Action::AmooToggleSkillManage
                            } else {
                                Action::Work(super::super::WorkAction::Tab(2))
                            },
                            if self.amoo_skill_manage {
                                Btn::Primary
                            } else {
                                Btn::Secondary
                            },
                        ),
                    )
                    .finish(),
                )
                .with_child(
                    Expanded::new(
                        1.,
                        self.button(
                            "amoo-npu-models",
                            "模型目录",
                            if self.demo_mode {
                                Action::AmooOpenModels
                            } else {
                                Action::Work(super::super::WorkAction::Tab(1))
                            },
                            Btn::Secondary,
                        ),
                    )
                    .finish(),
                )
                .finish(),
        )
        .with_uniform_padding(12.)
        .with_background_color(if p.dark { p.sidebar } else { p.surface })
        .with_border(Border::top(1.).with_border_color(p.border))
        .finish();

        let left_col = if self.demo_mode {
            let mut session_els = vec![sessions_header];
            session_els.extend(session_cards);
            let sessions_block = self.vstack(6., session_els);
            let left_top =
                Container::new(self.vstack(12., vec![agent_selector, search_box, sessions_block]))
                    .with_uniform_padding(14.)
                    .finish();
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(left_top)
                    .with_child(left_bottom)
                    .finish(),
            )
            .with_border(Border::right(1.).with_border_color(p.border))
            .finish()
        } else {
            let search_header = Container::new(self.input(F_AMOO_SEARCH, "搜索对话…", None))
                .with_horizontal_padding(S4)
                .with_vertical_padding(S3)
                .with_border(Border::bottom(1.).with_border_color(p.border))
                .finish();

            let mut history_rows: Vec<El> = Vec::new();
            let filtered_real: Vec<(usize, &super::super::AmooSessionItem)> = self
                .amoo_sessions
                .iter()
                .enumerate()
                .filter(|(_, s)| {
                    !self.workspace.deleted_sessions.contains(&s.session_key)
                        && (query.is_empty()
                            || session_title(&s.title).to_lowercase().contains(&query)
                            || s.meta.to_lowercase().contains(&query)
                            || s.status.to_lowercase().contains(&query))
                })
                .collect();

            for (idx, sess) in filtered_real.into_iter().take(60) {
                let active = self.amoo_session_idx == idx;
                let waiting_unlock = session_waits_for_unlock(&sess.items);
                let title_str = session_title(&sess.title);
                let menu_title = title_str.clone();
                let status_str = if waiting_unlock {
                    "等待解锁".to_string()
                } else {
                    sess.status.clone()
                };
                let status_badge = if waiting_unlock {
                    self.pill_badge(status_str, p.warning, p.warning_soft)
                } else if sess.status_kind == 2 {
                    self.pill_badge(status_str, p.success_text, p.success_soft)
                } else {
                    self.line(status_str, MICRO, p.text3)
                };
                let row_content = self.vstack(
                    4.,
                    vec![
                        self.between(
                            self.line_w(
                                title_str,
                                BODY,
                                p.text,
                                if active {
                                    Weight::Semibold
                                } else {
                                    Weight::Medium
                                },
                            ),
                            status_badge,
                        ),
                        self.line(sess.meta.clone(), CAPTION, p.text2),
                    ],
                );
                let state = self.mouse_state(format!("amoo-sess-row:{idx}"));
                let row_el = Hoverable::new(state, move |mouse| {
                    let _hovered = mouse.is_mouse_over_element();
                    let t = self.hover_t(&mouse);
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else if active {
                        mix_color(p.selected, p.hover, t * 0.5)
                    } else {
                        mix_color(ColorU::transparent_black(), p.hover, t)
                    };
                    Container::new(row_content)
                        .with_uniform_padding(12.)
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| {
                    ctx.dispatch_typed_action(Action::Workspace(
                        super::super::workspace_state::WorkspaceAction::PhoneSession(idx),
                    ))
                })
                .on_right_click(move |ctx, _, at| {
                    ctx.dispatch_typed_action(Action::ContextOpen(
                        crate::ui::context_menu::Target::ChatSession {
                            workspace_task_id: None,
                            phone_session_idx: Some(idx),
                            title: menu_title.clone(),
                        },
                        at,
                    ));
                })
                .finish();
                history_rows.push(row_el);
            }

            if history_rows.is_empty() {
                history_rows.push(self.empty(
                    if self.busy && self.amoo_sessions.is_empty() {
                        "正在同步对话历史…"
                    } else {
                        "暂无对话记录"
                    },
                    "点击右上角「新建对话」或在右侧直接输入问题开始对话。",
                    None,
                ));
            }

            let scrollable_sessions = Expanded::new(
                1.,
                self.pane_scroll(
                    self.scroll_aux.clone(),
                    Container::new(self.vstack(4., history_rows))
                        .with_horizontal_padding(S3)
                        .with_vertical_padding(S2)
                        .finish(),
                ),
            )
            .finish();
            Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_child(search_header)
                    .with_child(scrollable_sessions)
                    .finish(),
            )
            .with_background_color(if p.dark { p.sidebar } else { p.bg })
            .with_border(Border::right(1.).with_border_color(p.border))
            .finish()
        };

        // 2. Col 2 (6 cols): Main Multimodal Conversation + Interactive Composer
        let scene_pills: Vec<El> = CAPABILITY_PILLS
            .iter()
            .enumerate()
            .map(|(i, (label, tpl))| {
                let state = self.mouse_state(format!("amoo-cap-pill:{i}"));
                let act = Action::AmooSelectCap(i, (*tpl).into());
                let is_active = self.amoo_cap_idx == i;
                Hoverable::new(state, move |mouse| {
                    let _hovered = mouse.is_mouse_over_element();
                    let t = self.hover_t(&mouse);
                    let active_bg = if p.dark {
                        rgb(0x27, 0x27, 0x2A)
                    } else {
                        rgb(0xF4, 0xF4, 0xF6)
                    };
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else if is_active {
                        mix_color(active_bg, p.hover, t * 0.5)
                    } else {
                        mix_color(p.bg, p.hover, t)
                    };
                    let border = if is_active || mouse.is_clicked() {
                        p.border_strong
                    } else {
                        mix_color(p.border, p.border_strong, t)
                    };
                    Container::new(self.line_w(
                        *label,
                        CAPTION,
                        p.text,
                        if is_active {
                            Weight::Semibold
                        } else {
                            Weight::Normal
                        },
                    ))
                    .with_horizontal_padding(10.)
                    .with_vertical_padding(4.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(act.clone()))
                .finish()
            })
            .collect();

        let tts_state = self.mouse_state("amoo-tts-toggle");
        let tts_on = self.amoo_tts_on;
        let tts_toggle = Hoverable::new(tts_state, move |mouse| {
            let _hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let fg = if mouse.is_clicked() {
                p.accent
            } else if tts_on {
                p.success
            } else {
                mix_color(p.text2, p.text, t)
            };
            self.hstack(
                4.,
                vec![
                    self.ui_icon(
                        if tts_on {
                            UiIcon::Speaker
                        } else {
                            UiIcon::Mute
                        },
                        13.,
                        fg,
                    ),
                    self.line_w(
                        if tts_on { "朗读: 开" } else { "朗读: 关" },
                        11.,
                        fg,
                        if tts_on {
                            Weight::Semibold
                        } else {
                            Weight::Normal
                        },
                    ),
                ],
            )
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::AmooToggleTts))
        .finish();

        let model_selector_row = self.between(
            self.hstack(
                6.,
                vec![
                    self.line_w("模型：自动", 11., p.text2, Weight::Medium),
                    self.line(self.active_amoo_model_short(), 11., p.text),
                    self.link_muted(
                        "amoo-real-models",
                        "模型与账号",
                        Action::Work(super::super::WorkAction::Tab(1)),
                    ),
                ],
            ),
            if self.demo_mode {
                tts_toggle
            } else {
                self.line("和手机同步", 11., p.text3)
            },
        );

        let phone_mode = self.amoo_ui.screen_mode;
        let mode_row = self.hstack(
            6.,
            vec![
                self.button(
                    "amoo-mode-chat",
                    "聊天",
                    Action::AmooUi(AmooUiAction::GuiMode(false)),
                    if phone_mode {
                        Btn::Ghost
                    } else {
                        Btn::Secondary
                    },
                ),
                self.button(
                    "amoo-mode-phone",
                    if self.amoo_ui.gui_running {
                        "操作手机 · 等待结果"
                    } else if self.amoo_ui.gui_unconfirmed {
                        "操作手机 · 待确认"
                    } else {
                        "操作手机"
                    },
                    Action::AmooUi(AmooUiAction::GuiMode(true)),
                    if phone_mode {
                        Btn::Secondary
                    } else {
                        Btn::Ghost
                    },
                ),
            ],
        );
        let mut top_rows = vec![self.between(
            mode_row,
            if !phone_mode
                && !self.demo_mode
                && (self.amoo_ui.gui_running
                    || self.amoo_ui.gui_paused
                    || self.amoo_ui.gui_unconfirmed)
            {
                self.hstack(
                    4.,
                    [("pause", "暂停手机"), ("resume", "继续"), ("stop", "停止")]
                        .into_iter()
                        .map(|(op, label)| {
                            self.button(
                                format!("amoo-native-task-{op}"),
                                label,
                                Action::AmooUi(AmooUiAction::GuiPhoneControl(op.into())),
                                Btn::Ghost,
                            )
                        })
                        .collect(),
                )
            } else if phone_mode {
                self.button(
                    "amoo-top-back-chat",
                    "返回对话",
                    Action::AmooUi(AmooUiAction::GuiMode(false)),
                    Btn::Secondary,
                )
            } else {
                Empty::new().finish()
            },
        )];
        if !phone_mode {
            top_rows.push(model_selector_row);
            top_rows.push(self.between(
                self.wrap(scene_pills),
                if self.demo_mode {
                    self.link_icon(
                        "amoo-top-express",
                        "快递取件码 (4-2-1086)",
                        UiIcon::Package,
                        Action::AmooToggleExpressCard,
                    )
                } else {
                    Empty::new().finish()
                },
            ));
        }
        let top_cap_bar = Container::new(self.vstack(6., top_rows))
            .with_padding_bottom(6.)
            .with_border(Border::bottom(1.).with_border_color(p.border))
            .finish();

        let stream = if phone_mode {
            self.amoo_gui_records()
        } else if self.amoo_show_schedules {
            self.amoo_schedule_center()
        } else {
            self.amoo_conversation()
        };

        let mut bottom_stack: Vec<El> = Vec::new();
        if !phone_mode {
            if let Some(panel) = self.screen_text_panel() {
                bottom_stack.push(panel);
            }
        }

        let composer_input_row = self.amoo_message_composer(false, phone_mode);

        if !self.demo_mode {
            let current = self.amoo_sessions.get(self.amoo_session_idx);
            let cur_title = if phone_mode {
                "操作手机 · 实时触控与智能导航".to_string()
            } else {
                current
                    .map(|s| session_title(&s.title))
                    .unwrap_or_else(|| "新对话".into())
            };
            let cur_meta = if phone_mode {
                "左侧画面支持单击触控，右侧支持一键指令与智能执行".to_string()
            } else {
                current
                    .map(|s| s.meta.clone())
                    .unwrap_or_else(|| "在下方输入问题，按回车发送".into())
            };

            let subheader = ConstrainedBox::new(
                Container::new(self.between(
                    self.hstack(
                        S2,
                        vec![
                            self.line_w(cur_title, SUBHEAD, p.text, Weight::Semibold),
                            self.pill_badge(
                                if self.amoo_gateway_online || self.selected.is_some() {
                                    "已连接"
                                } else {
                                    "待连接"
                                },
                                if self.amoo_gateway_online || self.selected.is_some() {
                                    p.success_text
                                } else {
                                    p.text2
                                },
                                if self.amoo_gateway_online || self.selected.is_some() {
                                    p.success_soft
                                } else {
                                    p.tag_bg
                                },
                            ),
                        ],
                    ),
                    self.hstack(
                        S2,
                        vec![
                            self.line(cur_meta, CAPTION, p.text2),
                            if phone_mode && (self.amoo_ui.gui_running || self.amoo_ui.gui_paused) {
                                self.hstack(
                                    6.,
                                    vec![
                                        self.button(
                                            "amoo-subheader-gui-pause",
                                            if self.amoo_ui.gui_paused {
                                                "继续"
                                            } else {
                                                "暂停"
                                            },
                                            Action::AmooUi(AmooUiAction::GuiPauseResume),
                                            Btn::Secondary,
                                        ),
                                        self.button(
                                            "amoo-subheader-gui-stop",
                                            "停止",
                                            Action::AmooUi(AmooUiAction::GuiStopWaiting),
                                            Btn::Secondary,
                                        ),
                                    ],
                                )
                            } else {
                                Empty::new().finish()
                            },
                            self.button(
                                "amoo-subheader-mode-toggle",
                                if phone_mode {
                                    "返回对话"
                                } else {
                                    "操作手机"
                                },
                                Action::AmooUi(AmooUiAction::GuiMode(!phone_mode)),
                                Btn::Secondary,
                            ),
                        ],
                    ),
                ))
                .with_horizontal_padding(S5)
                .with_background_color(p.bg)
                .with_border(Border::bottom(1.).with_border_color(p.border))
                .finish(),
            )
            .with_height(48.)
            .finish();

            let mut right_children: Vec<El> = Vec::new();
            if !self.amoo_show_schedules {
                right_children.push(subheader);
            }

            if !phone_mode
                && !self.amoo_show_schedules
                && current.is_some_and(|session| session_waits_for_unlock(&session.items))
            {
                right_children.push(
                    Container::new(self.vstack(
                        4.,
                        vec![
                            self.line_w("等待解锁", CAPTION, p.warning, Weight::Semibold),
                            self.text(
                                "最近的手机操作被锁屏阻止，请解锁手机后继续。",
                                CAPTION,
                                p.text2,
                            ),
                        ],
                    ))
                    .with_horizontal_padding(S5)
                    .with_vertical_padding(S2)
                    .with_background_color(p.warning_soft)
                    .with_border(Border::bottom(1.).with_border_color(p.warning_border))
                    .finish(),
                );
            }

            let mut real_composer_rows: Vec<El> = Vec::new();
            if !phone_mode {
                if let Some(panel) = self.screen_text_panel() {
                    real_composer_rows.push(panel);
                }
            }
            if !phone_mode && (self.amoo_ui.gui_running || self.amoo_ui.gui_paused) {
                real_composer_rows.push(self.hstack(
                    8.,
                    vec![
                        self.text(&self.amoo_ui.gui_message, MICRO, p.text2),
                        self.button(
                            "amoo-chat-gui-pause",
                            if self.amoo_ui.gui_paused {
                                "继续"
                            } else {
                                "暂停"
                            },
                            Action::AmooUi(AmooUiAction::GuiPauseResume),
                            Btn::Secondary,
                        ),
                        self.button(
                            "amoo-chat-gui-stop",
                            "停止",
                            Action::AmooUi(AmooUiAction::GuiStopWaiting),
                            Btn::Secondary,
                        ),
                    ],
                ));
            } else if !phone_mode
                && self.amoo_ui.gui_unconfirmed
                && !self.amoo_ui.gui_message.is_empty()
            {
                real_composer_rows.push(self.text(&self.amoo_ui.gui_message, MICRO, p.text2));
            }
            real_composer_rows.push(composer_input_row);

            let composer_footer = Container::new(self.vstack(S2, real_composer_rows))
                .with_horizontal_padding(S5)
                .with_vertical_padding(S3)
                .with_background_color(p.bg)
                .with_border(Border::top(1.).with_border_color(p.border))
                .finish();

            if phone_mode {
                let left_studio_col = self.fixed_w(
                    290.,
                    Container::new(self.amoo_gui_phone_studio())
                        .with_padding_left(S5)
                        .with_padding_right(S2)
                        .with_padding_top(S5)
                        .with_padding_bottom(S5)
                        .finish(),
                );
                let right_controls_scroll = Expanded::new(
                    1.,
                    self.pane_scroll(
                        self.scroll.clone(),
                        Container::new(
                            self.vstack(S3, vec![self.amoo_gui_controls_panel(), stream]),
                        )
                        .with_padding_left(S3)
                        .with_padding_right(S5)
                        .with_padding_top(S5)
                        .with_padding_bottom(S5)
                        .finish(),
                    ),
                )
                .finish();
                let split_body = Expanded::new(
                    1.,
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(left_studio_col)
                        .with_child(right_controls_scroll)
                        .finish(),
                )
                .finish();
                right_children.push(split_body);
                right_children.push(composer_footer);
            } else {
                right_children.push(
                    Expanded::new(
                        1.,
                        if self.amoo_show_schedules {
                            self.pane_scroll(
                                self.scroll.clone(),
                                Container::new(stream)
                                    .with_horizontal_padding(S5)
                                    .with_padding_top(S5)
                                    .with_padding_bottom(S6)
                                    .finish(),
                            )
                        } else {
                            self.chat_scroll(
                                Container::new(self.selectable_messages(stream))
                                    .with_horizontal_padding(S5)
                                    .with_padding_top(S5)
                                    .with_padding_bottom(S5)
                                    .finish(),
                            )
                        },
                    )
                    .finish(),
                );
                right_children.push(composer_footer);
            }

            let right_pane = Container::new(
                Flex::column()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                    .with_children(right_children)
                    .finish(),
            )
            .with_background_color(p.bg)
            .finish();

            return Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(
                    ConstrainedBox::new(left_col)
                        .with_width(LIST_PANE_W)
                        .finish(),
                )
                .with_child(Expanded::new(1., right_pane).finish())
                .finish();
        }

        let mut composer_children = Vec::new();
        if !phone_mode && (self.amoo_ui.gui_running || self.amoo_ui.gui_paused) {
            composer_children.push(self.hstack(
                8.,
                vec![
                    self.text(&self.amoo_ui.gui_message, MICRO, p.text2),
                    self.button(
                        "amoo-chat-gui-pause",
                        if self.amoo_ui.gui_paused {
                            "继续"
                        } else {
                            "暂停"
                        },
                        Action::AmooUi(AmooUiAction::GuiPauseResume),
                        Btn::Secondary,
                    ),
                    self.button(
                        "amoo-chat-gui-stop",
                        "停止操作",
                        Action::AmooUi(AmooUiAction::GuiStopWaiting),
                        Btn::Secondary,
                    ),
                ],
            ));
        } else if !phone_mode
            && self.amoo_ui.gui_unconfirmed
            && !self.amoo_ui.gui_message.is_empty()
        {
            composer_children.push(self.text(&self.amoo_ui.gui_message, MICRO, p.text2));
        }
        composer_children.push(composer_input_row);
        if phone_mode {
            composer_children.push(self.text(
                if self.amoo_attachments.is_empty() {
                    "执行期间手机前台界面会变化，请保持解锁。不会自动通过锁屏或重发失败的操作。"
                } else {
                    "附件已保留。操作手机时不能带附件，切回“聊天”就能发送。"
                },
                MICRO,
                p.text3,
            ));
        } else {
            composer_children.push(self.amoo_usage_footer());
        }
        if !phone_mode && (self.amoo_plus_open || !self.amoo_attachments.is_empty()) {
            composer_children.push(self.text(
                "可以加图片、文档和表格，最多 8 个文件、共 8 MB",
                MICRO,
                p.text3,
            ));
        }

        bottom_stack.push(self.vstack(S2, composer_children));

        let mut center_children: Vec<El> = vec![top_cap_bar];

        if !phone_mode
            && !self.amoo_show_schedules
            && self
                .amoo_sessions
                .get(self.amoo_session_idx)
                .is_some_and(|session| session_waits_for_unlock(&session.items))
        {
            center_children.push(Container::new(self.vstack(5., vec![
                self.line_w("等待解锁", CAPTION, p.warning, Weight::Semibold),
                self.text("Amoo 最近的回复说明手机操作被锁屏阻止，任务尚未完成。请手动解锁手机后再下达操作；桌面不会自动重试。", CAPTION, p.text2),
            ])).with_uniform_padding(S3).with_background_color(p.surface)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL))).finish());
        }

        if !phone_mode && self.amoo_express_open && !self.amoo_lightbox_open {
            let mut parcel_cols: Vec<El> = Vec::new();
            for (p_idx, parcel) in self.express_parcels.iter().enumerate() {
                let code = parcel.pickup_code.clone();
                let copy_str = format!(
                    "{} · 运单 {} · 取件码 {} ({})",
                    parcel.carrier, parcel.tracking_no, parcel.pickup_code, parcel.station
                );
                let short_station: String = parcel.station.chars().take(9).collect();
                let cell = Container::new(self.vstack(
                    3.,
                    vec![
                        self.between(
                            self.line_w(&parcel.carrier, CAPTION, p.text, Weight::Semibold),
                            self.pill_badge(&parcel.status, p.success_text, p.success_soft),
                        ),
                        self.hstack(
                            6.,
                            vec![
                                self.line("取件码:", MICRO, p.text2),
                                self.mono(&parcel.pickup_code, SUBHEAD, p.accent),
                                self.line(format!("· {short_station}"), MICRO, p.text2),
                            ],
                        ),
                        self.between(
                            self.mono(format!("单号 {}", parcel.tracking_no), MICRO, p.text3),
                            self.hstack(
                                6.,
                                vec![
                                    self.link_action(
                                        format!("exp-copy-{p_idx}"),
                                        "复制",
                                        Action::CopyText(
                                            copy_str,
                                            "已复制快递取件码与驿站位置".into(),
                                        ),
                                    ),
                                    self.link_icon(
                                        format!("exp-push-{p_idx}"),
                                        "投背屏",
                                        UiIcon::Matrix,
                                        Action::AmooPushPickupCode(code),
                                    ),
                                ],
                            ),
                        ),
                    ],
                ))
                .with_horizontal_padding(9.)
                .with_vertical_padding(5.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.surface_border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
                parcel_cols.push(Expanded::new(1., cell).finish());
            }

            let express_card = Container::new(self.vstack(
                5.,
                vec![
                    self.between(
                        self.hstack(
                            6.,
                            vec![
                                self.ui_icon(UiIcon::Package, 14., p.text),
                                self.line_w(
                                    "PC 端快速获取快递 · 实时物流与取件码",
                                    CAPTION,
                                    p.text,
                                    Weight::Semibold,
                                ),
                                self.pill_badge("已同步手机短信 & Amoo", p.accent, p.accent_soft),
                            ],
                        ),
                        self.link_muted(
                            "exp-card-hide",
                            "收起卡片",
                            Action::AmooToggleExpressCard,
                        ),
                    ),
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_spacing(8.)
                        .with_children(parcel_cols)
                        .finish(),
                ],
            ))
            .with_horizontal_padding(10.)
            .with_vertical_padding(7.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish();

            center_children.push(Container::new(express_card).with_padding_top(6.).finish());
        }

        center_children.push(
            Expanded::new(
                1.,
                Container::new(if phone_mode {
                    self.pane_scroll(
                        self.scroll.clone(),
                        Container::new(self.vstack(S3, vec![self.amoo_gui_controls(), stream]))
                            .with_padding_bottom(S3)
                            .finish(),
                    )
                } else {
                    self.chat_scroll(self.selectable_messages(stream))
                })
                .with_padding_top(8.)
                .with_padding_bottom(6.)
                .finish(),
            )
            .finish(),
        );
        center_children.push(self.vstack(6., bottom_stack));

        let center_col = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_children(center_children)
                .finish(),
        )
        .with_horizontal_padding(S5)
        .with_padding_top(S4)
        .with_padding_bottom(S4)
        .with_border(Border::right(1.).with_border_color(p.border))
        .finish();

        // 3. Col 3 (3 cols): Pure Amoo Space (Memory, Skills & Safety)
        let mem_item = |idx: usize, title: String, sub: String| -> El {
            let state = self.mouse_state(format!("amoo-mem-item:{idx}"));
            let t_clone = title.clone();
            let content = self.vstack(
                2.,
                vec![
                    self.line_w(title, CAPTION, p.text, Weight::Medium),
                    self.line(sub, 11., p.text2),
                ],
            );
            Hoverable::new(state, move |mouse| {
                let _hovered = mouse.is_mouse_over_element();
                let t = self.hover_t(&mouse);
                let bg = if mouse.is_clicked() {
                    p.accent_soft
                } else {
                    mix_color(p.surface, p.hover, t)
                };
                let border = if mouse.is_clicked() {
                    p.border_strong
                } else {
                    mix_color(p.surface_border, p.border_strong, t)
                };
                Container::new(content)
                    .with_uniform_padding(8.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
            })
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| {
                ctx.dispatch_typed_action(Action::AmooUseMemory(t_clone.clone()))
            })
            .finish()
        };

        let clickable_pref_row =
            |id: &str, k: &'static str, v: &'static str, v_fg: ColorU, act: Action| -> El {
                let state = self.mouse_state(id);
                let row = self.between(
                    self.line(k, CAPTION, p.text2),
                    self.line_w(v, CAPTION, v_fg, Weight::Medium),
                );
                Hoverable::new(state, move |mouse| {
                    let _hovered = mouse.is_mouse_over_element();
                    let t = self.hover_t(&mouse);
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else {
                        mix_color(ColorU::transparent_black(), p.hover, t)
                    };
                    Container::new(row)
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(act.clone()))
                .finish()
            };

        let style_label = match self.amoo_style_idx {
            0 => "直接精炼 · 不长篇解释",
            1 => "温和详细 · 分步拆解",
            _ => "极客专业 · 附带命令",
        };
        let commute_label = match self.amoo_commute_idx {
            0 => "08:30 播报路况",
            1 => "09:00 播报日程",
            _ => "18:30 晚高峰提醒",
        };

        let mem_count_state = self.mouse_state("amoo-mem-badge");
        let mem_total = if self.demo_mode {
            26 + self.amoo_memories.len()
        } else {
            self.amoo_bm25_entries.max(self.amoo_memories.len())
        };
        let mem_badge = Hoverable::new(mem_count_state, move |mouse| {
            let _hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let bg = if mouse.is_clicked() {
                p.accent_soft
            } else {
                mix_color(p.success_soft, p.hover, t)
            };
            Container::new(self.line_w(
                if self.demo_mode || mem_total > 0 {
                    format!("{mem_total} 条")
                } else {
                    "未同步".into()
                },
                11.,
                p.success_text,
                Weight::Medium,
            ))
            .with_horizontal_padding(6.)
            .with_vertical_padding(2.)
            .with_background_color(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::AmooOpenMemory))
        .finish();

        let filtered_memories: Vec<(usize, &super::super::AmooMemoryEntry)> = self
            .amoo_memories
            .iter()
            .enumerate()
            .filter(|(_, m)| {
                query.is_empty()
                    || m.title.to_lowercase().contains(&query)
                    || m.sub.to_lowercase().contains(&query)
            })
            .take(2)
            .collect();

        let mut mem_list_els = vec![self.between(
            self.line("为你记录的重要内容", 11., p.text2),
            self.link_muted("amoo-add-mem", "＋ 记录", Action::AmooAddMemory),
        )];
        for (i, m) in filtered_memories {
            mem_list_els.push(mem_item(i, m.title.clone(), m.sub.clone()));
        }

        let memory_card = Container::new(self.vstack(
            8.,
            vec![
                self.between(
                    self.hstack(
                        6.,
                        vec![
                            self.ui_icon(UiIcon::Lightbulb, 14., p.success),
                            self.heading("Amoo 记忆与偏好"),
                        ],
                    ),
                    mem_badge,
                ),
                self.vstack(
                    5.,
                    vec![
                        clickable_pref_row(
                            "amoo-pref-mem",
                            "跨会话记忆",
                            if self.amoo_memory_enabled {
                                "已开启"
                            } else {
                                "已关闭"
                            },
                            if self.amoo_memory_enabled {
                                p.success
                            } else {
                                p.text2
                            },
                            Action::AmooToggleMemory,
                        ),
                        clickable_pref_row(
                            "amoo-pref-style",
                            "交流风格",
                            style_label,
                            p.text,
                            Action::AmooCycleStyle,
                        ),
                        clickable_pref_row(
                            "amoo-pref-commute",
                            "通勤偏好",
                            commute_label,
                            p.text,
                            Action::AmooCycleCommute,
                        ),
                    ],
                ),
                Container::new(self.vstack(6., mem_list_els))
                    .with_padding_top(8.)
                    .with_border(Border::top(1.).with_border_color(p.border))
                    .finish(),
            ],
        ))
        .with_uniform_padding(12.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let skill_colors = [p.blue, p.success, p.indigo, p.warning, p.pink, p.accent];
        let skill_pill = |idx: usize| -> El {
            let entry = &self.amoo_skills[idx];
            let enabled = entry.enabled;
            let dot_color = if enabled {
                skill_colors[idx % skill_colors.len()]
            } else {
                p.text3
            };
            let fg = if enabled { p.text } else { p.text3 };
            let name = entry.name.clone();
            let state = self.mouse_state(format!("amoo-skill-pill:{idx}"));
            Expanded::new(
                1.,
                Hoverable::new(state, move |mouse| {
                    let _hovered = mouse.is_mouse_over_element();
                    let t = self.hover_t(&mouse);
                    let bg = if mouse.is_clicked() {
                        p.accent_soft
                    } else {
                        mix_color(p.surface, p.hover, t)
                    };
                    let border = if mouse.is_clicked() {
                        p.border_strong
                    } else {
                        mix_color(p.surface_border, p.border_strong, t)
                    };
                    Container::new(self.lead(
                        6.,
                        self.dot(dot_color),
                        self.line_w(name.clone(), CAPTION, fg, Weight::Medium),
                    ))
                    .with_horizontal_padding(10.)
                    .with_vertical_padding(6.)
                    .with_background_color(bg)
                    .with_border(Border::all(1.).with_border_color(border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(Action::AmooClickSkill(idx)))
                .finish(),
            )
            .finish()
        };

        let enabled_skills_count = if self.demo_mode {
            2 + self.amoo_skills.iter().filter(|s| s.enabled).count()
        } else {
            self.amoo_skills.iter().filter(|s| s.enabled).count()
        };

        let mut skill_rows: Vec<El> = vec![self.between(
            self.heading(if self.demo_mode || !self.amoo_skills.is_empty() {
                format!("已启用技能 ({enabled_skills_count})")
            } else {
                "技能尚未同步".into()
            }),
            self.link_muted(
                "amoo-skill-mgr",
                if self.amoo_skill_manage {
                    "完成"
                } else {
                    "管理"
                },
                Action::AmooToggleSkillManage,
            ),
        )];
        let skill_len = self.amoo_skills.len().min(4);
        let mut s_idx = 0;
        while s_idx < skill_len {
            let left_pill = skill_pill(s_idx);
            let right_pill = if s_idx + 1 < skill_len {
                skill_pill(s_idx + 1)
            } else {
                Expanded::new(1., Empty::new().finish()).finish()
            };
            skill_rows.push(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_spacing(6.)
                    .with_child(left_pill)
                    .with_child(right_pill)
                    .finish(),
            );
            s_idx += 2;
        }

        let skills_card = Container::new(self.vstack(8., skill_rows))
            .with_uniform_padding(12.)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish();

        let policy_label = match self.amoo_safety_policy_idx {
            0 => "始终确认",
            1 => "仅高危询问",
            _ => "严格拦截",
        };

        let safety_card = Container::new(self.vstack(
            8.,
            vec![
                self.between(
                    self.heading("安全守护"),
                    self.line_w(
                        format!("今日守护 {} 次", self.amoo_guard_count),
                        11.,
                        p.success,
                        Weight::Medium,
                    ),
                ),
                clickable_pref_row(
                    "amoo-safety-policy",
                    "敏感操作前询问",
                    policy_label,
                    p.text,
                    Action::AmooCycleSafetyPolicy,
                ),
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_spacing(6.)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.button(
                                "amoo-safety-undo",
                                "撤回操作",
                                Action::AmooUndoSafety,
                                Btn::Secondary,
                            ),
                        )
                        .finish(),
                    )
                    .with_child(
                        Expanded::new(
                            1.,
                            self.button(
                                "amoo-mem-export",
                                "导出记忆",
                                Action::AmooExportMemory,
                                Btn::Secondary,
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
            ],
        ))
        .with_uniform_padding(12.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let engine_title = match self.amoo_agent_idx {
            0 => {
                if self.demo_mode {
                    "手机端助手"
                } else {
                    "Amoo 后台连接"
                }
            }
            1 => "电脑协同服务",
            2 => "飞书 / 企微网关",
            _ => "Claude / OpenClaw",
        };
        let engine_ver_label = if self.demo_mode {
            "v1.0.7 就绪".to_string()
        } else {
            let short_ver = self
                .amoo_gateway_version
                .split('-')
                .next()
                .unwrap_or(&self.amoo_gateway_version);
            format!(
                "v{short_ver} {}",
                if self.amoo_gateway_online {
                    "在线"
                } else {
                    "未连接"
                }
            )
        };
        let engine_state = self.mouse_state("amoo-engine-bar");
        let engine_bar = Hoverable::new(engine_state, move |mouse| {
            let _hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let bg = if mouse.is_clicked() {
                p.accent_soft
            } else {
                mix_color(p.bg, p.hover, t)
            };
            let border = if mouse.is_clicked() {
                p.border_strong
            } else {
                mix_color(p.border, p.border_strong, t)
            };
            Container::new(self.between(
                self.line(engine_title, CAPTION, p.text2),
                self.mono(engine_ver_label.clone(), CAPTION, p.success),
            ))
            .with_horizontal_padding(12.)
            .with_vertical_padding(8.)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::AmooOpenModels))
        .finish();

        let pc_ctrl_card = Container::new(self.vstack(
            6.,
            vec![
                self.between(
                    self.hstack(
                        6.,
                        vec![
                            self.ui_icon(UiIcon::Monitor, 14., p.text),
                            self.line_w("手机控电脑 & 常驻", CAPTION, p.text, Weight::Semibold),
                            self.pill_badge(
                                if self.amoo_pc_control_enabled {
                                    "双向远控已开启"
                                } else {
                                    "已暂停"
                                },
                                p.success_text,
                                p.success_soft,
                            ),
                        ],
                    ),
                    self.link_action(
                        "amoo-pc-toggle",
                        if self.amoo_pc_control_enabled {
                            "暂停"
                        } else {
                            "开启"
                        },
                        Action::AmooTogglePcControl,
                    ),
                ),
                self.line(
                    "在手机上对 Amoo 说一声，就能让电脑截屏、同步剪贴板、静音或锁屏",
                    10.5,
                    p.text2,
                ),
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_spacing(6.)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.button_icon(
                                "pc-cmd-shot",
                                "截屏发手机",
                                UiIcon::Monitor,
                                Action::AmooExecPcCommand("screenshot_to_phone".into()),
                                Btn::Secondary,
                            ),
                        )
                        .finish(),
                    )
                    .with_child(
                        Expanded::new(
                            1.,
                            self.button_icon(
                                "pc-cmd-clip",
                                "同步剪贴板",
                                UiIcon::Clipboard,
                                Action::AmooExecPcCommand("sync_clipboard".into()),
                                Btn::Secondary,
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_spacing(6.)
                    .with_child(
                        Expanded::new(
                            1.,
                            self.button_icon(
                                "pc-cmd-mute",
                                "电脑静音",
                                UiIcon::Mute,
                                Action::AmooExecPcCommand("pc_mute_toggle".into()),
                                Btn::Secondary,
                            ),
                        )
                        .finish(),
                    )
                    .with_child(
                        Expanded::new(
                            1.,
                            self.button_icon(
                                "pc-cmd-lock",
                                "远程锁屏",
                                UiIcon::Lock,
                                Action::AmooExecPcCommand("pc_lock_screen".into()),
                                Btn::Secondary,
                            ),
                        )
                        .finish(),
                    )
                    .finish(),
            ],
        ))
        .with_uniform_padding(10.)
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let right_col = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_main_axis_alignment(MainAxisAlignment::SpaceBetween)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_child(self.vstack(
                    7.,
                    vec![pc_ctrl_card, memory_card, skills_card, safety_card],
                ))
                .with_child(Container::new(engine_bar).with_padding_top(6.).finish())
                .finish(),
        )
        .with_uniform_padding(12.)
        .with_background_color(if p.dark { p.sidebar } else { p.surface })
        .finish();

        let right_col = if self.demo_mode {
            right_col
        } else {
            let current = self.amoo_sessions.get(self.amoo_session_idx);
            let mut detail_parts = vec![
                self.heading("会话详情"),
                self.text(
                    current
                        .map(|s| session_title(&s.title))
                        .unwrap_or_else(|| "尚未选择会话".into()),
                    BODY,
                    p.text,
                ),
            ];
            if let Some(session) = current {
                detail_parts.push(self.text(&session.meta, CAPTION, p.text2));
                detail_parts.push(self.line(format!("状态：{}", session.status), CAPTION, p.text2));
            }
            detail_parts.push(self.divider());
            detail_parts.push(self.hstack(
                S2,
                vec![
                    self.dot(if self.amoo_gateway_online {
                        p.success
                    } else {
                        p.text3
                    }),
                    self.line(
                        if self.amoo_gateway_online {
                            "手机 Amoo 已连接"
                        } else {
                            "手机 Amoo 未连接"
                        },
                        CAPTION,
                        p.text,
                    ),
                ],
            ));
            detail_parts.push(self.text(
                "文字、图片和文档由手机 Amoo 处理，并同步到当前会话。",
                CAPTION,
                p.text2,
            ));
            detail_parts.push(self.button(
                "amoo-details-schedules",
                "查看定时任务",
                Action::AmooOpenTasks,
                Btn::Secondary,
            ));
            detail_parts.push(self.divider());
            detail_parts.push(self.heading("更多能力"));
            detail_parts.push(self.text(
                "要做 PPT、文档或表格，直接在对话里说就行。",
                CAPTION,
                p.text2,
            ));
            detail_parts.push(self.button(
                "amoo-details-work",
                "生成文件",
                Action::Work(super::super::WorkAction::Tab(0)),
                Btn::Secondary,
            ));
            detail_parts.push(self.button(
                "amoo-details-skills",
                "浏览与安装技能",
                Action::Work(super::super::WorkAction::Tab(2)),
                Btn::Ghost,
            ));
            detail_parts.push(self.button(
                "amoo-details-models",
                "查看可用模型",
                Action::Work(super::super::WorkAction::Tab(1)),
                Btn::Ghost,
            ));
            Container::new(self.vstack(S3, detail_parts))
                .with_uniform_padding(S4)
                .with_background_color(p.surface)
                .with_border(Border::left(1.).with_border_color(p.border))
                .finish()
        };
        let mut layout = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(ConstrainedBox::new(left_col).with_width(260.).finish())
            .with_child(Expanded::new(1., center_col).finish());
        if (!phone_mode && self.demo_mode) || self.amoo_show_details {
            layout = layout.with_child(
                ConstrainedBox::new(self.sub_enter(right_col))
                    .with_width(286.)
                    .finish(),
            );
        }
        layout.finish()
    }

    pub(in super::super) fn amoo_schedule_center(&self) -> El {
        let p = self.p();
        let enabled = self.amoo_cron_jobs.iter().filter(|job| job.enabled).count();
        let mut sections = vec![
            self.between(
                self.hstack(S2, vec![
                    self.ui_icon(UiIcon::Calendar, 22., p.text),
                    self.heading("定时任务"),
                    self.pill_badge(
                        format!("{} 项 · {} 项已启用", self.amoo_cron_jobs.len(), enabled),
                        p.text2,
                        p.surface,
                    ),
                ]),
                self.button("amoo-schedule-refresh", "刷新", Action::AmooSnapshot, Btn::Secondary),
            ),
            self.text(
                "查看手机 Amoo 的任务安排与执行结果。进入任务会话，可以继续对话、修改要求或查看历史回复。",
                CAPTION,
                p.text2,
            ),
            self.wrap(vec![
                self.button(
                    "amoo-schedule-create",
                    "新建定时任务",
                    Action::AmooTemplate("请帮我创建一个定时任务：\n任务内容：\n执行时间和重复规则：\n请先确认安排，再创建任务。".into()),
                    Btn::Primary,
                ),
                self.button("amoo-schedule-back", "返回对话", Action::AmooToggleSchedules, Btn::Secondary),
            ]),
        ];
        if self.amoo_cron_jobs.is_empty() {
            sections.push(
                Container::new(self.vstack(
                    S3,
                    vec![
                        self.ui_icon(UiIcon::Clock, 32., p.text3),
                        self.heading(if self.busy {
                            "正在同步定时任务…"
                        } else if !self.amoo_gateway_online {
                            "连接手机后查看定时任务"
                        } else {
                            "还没有定时任务"
                        }),
                        self.text(
                            if self.amoo_gateway_online {
                                "例如：每天早上 8 点汇总 AI 新闻，或每周五提醒我整理工作总结。"
                            } else {
                                "保持手机连接，点击刷新即可从 Amoo 获取任务。"
                            },
                            CAPTION,
                            p.text2,
                        ),
                    ],
                ))
                .with_uniform_padding(S5)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .finish(),
            );
        } else {
            let query = self.fields[F_AMOO_SEARCH].text.trim().to_lowercase();
            let mut count = 0;
            for (index, job) in self.amoo_cron_jobs.iter().enumerate() {
                if !query.is_empty()
                    && !job.name.to_lowercase().contains(&query)
                    && !job.schedule.to_lowercase().contains(&query)
                {
                    continue;
                }
                count += 1;
                let session = self
                    .amoo_sessions
                    .iter()
                    .find(|s| s.session_key == job.session_key);
                let name = if job.name.trim().is_empty() {
                    session.map(|s| s.title.as_str()).unwrap_or("定时任务")
                } else {
                    &job.name
                };
                let mut parts = vec![
                    self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.ui_icon(UiIcon::Clock, 18., p.text2),
                                self.heading(session_title(name)),
                            ],
                        ),
                        self.pill_badge(
                            if job.enabled {
                                "已启用"
                            } else {
                                match job.status.as_str() {
                                    "paused" | "disabled" | "inactive" => "已暂停",
                                    "done" | "completed" => "已完成",
                                    "error" | "failed" => "执行失败",
                                    "" => "状态未返回",
                                    status => status,
                                }
                            },
                            if job.enabled { p.success_text } else { p.text3 },
                            if job.enabled {
                                p.success_soft
                            } else {
                                p.surface
                            },
                        ),
                    ),
                    self.text(
                        if job.schedule.trim().is_empty() {
                            "手机尚未提供执行时间"
                        } else {
                            &job.schedule
                        },
                        BODY,
                        p.text,
                    ),
                ];
                if let Some(session) = session {
                    parts.push(self.line(
                        format!("最近会话状态：{}", session.status),
                        MICRO,
                        p.text3,
                    ));
                }
                if !job.session_key.is_empty() {
                    parts.push(self.wrap(vec![self.button(
                        format!("amoo-schedule-session-{index}"),
                        "查看任务与执行记录",
                        Action::AmooOpenScheduledSession(job.session_key.clone()),
                        Btn::Secondary,
                    )]));
                }
                sections.push(
                    Container::new(self.vstack(S3, parts))
                        .with_uniform_padding(S4)
                        .with_background_color(p.bg)
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                        .finish(),
                );
            }
            if count == 0 {
                sections.push(self.text("没有匹配的定时任务，试试其他关键词。", CAPTION, p.text3));
            }
        }
        self.sub_enter(self.vstack(S4, sections))
    }

    fn amoo_gui_phone_studio(&self) -> El {
        let p = self.p();
        Container::new(
            self.vstack(
                S2,
                vec![
                    self.between(
                        self.line_w("手机实时触控台", CAPTION, p.text, Weight::Semibold),
                        self.hstack(
                            6.,
                            vec![
                                self.link_icon(
                                    "amoo-gui-live-sync",
                                    if self.live_screen_stream {
                                        "暂停同步"
                                    } else {
                                        "实时同步"
                                    },
                                    UiIcon::Refresh,
                                    Action::ToggleLiveScreenStream,
                                ),
                                self.link_icon(
                                    "amoo-gui-shot-now",
                                    "刷新画面",
                                    UiIcon::Camera,
                                    Action::AmooUi(AmooUiAction::GuiRefreshScreen),
                                ),
                            ],
                        ),
                    ),
                    Align::new(self.interactive_phone_bezel(
                        "amoo-gui",
                        168.,
                        346.,
                        self.amoo_ui.gui_screenshot.as_deref(),
                    ))
                    .finish(),
                    Align::new(self.line("单击画面直接触控 · 双击打开独立投屏", MICRO, p.text3))
                        .finish(),
                ],
            ),
        )
        .with_uniform_padding(S3)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish()
    }

    fn amoo_gui_controls_panel(&self) -> El {
        let p = self.p();
        let state = &self.amoo_ui;
        let locked = state
            .gui_status
            .as_ref()
            .is_some_and(|s| s.locked != Some(false));
        let screen_off = state
            .gui_status
            .as_ref()
            .is_some_and(|s| s.interactive != Some(true));
        let status = if state.gui_checking {
            "正在检查手机状态并同步实时画面…"
        } else if state.gui_running {
            "正在操作手机并同步画面…"
        } else if !state.gui_desktop && state.gui_unavailable_reason.is_some() {
            "当前手机系统未开放屏幕自动操作"
        } else if state.gui_unconfirmed {
            "需确认手机状态"
        } else if let Some(status) = &state.gui_status {
            if !status.supported && !state.gui_desktop {
                "未发现操作接口"
            } else if status.interactive != Some(true) {
                "手机屏幕未点亮 · 可点击下方「一键点亮并滑出解锁」"
            } else if status.locked != Some(false) {
                "手机处于锁屏 · 可一键上滑解锁或打开实时投屏解锁"
            } else if state.gui_desktop {
                "手机屏幕已就绪 · 可直接点击上方快捷按钮或输入操控指令"
            } else {
                "已解锁 · 接口待验证"
            }
        } else {
            "支持零配置秒级操控（点亮、截图、提字、打开应用、开关设置）与多步智能导航"
        };

        let quick_chips: Vec<El> = [
            ("wake", UiIcon::Sun, "点亮解锁", "当前屏幕", "点亮屏幕"),
            (
                "shot",
                UiIcon::Camera,
                "截取画面",
                "当前屏幕",
                "截取手机画面",
            ),
            (
                "ocr",
                UiIcon::ScanText,
                "提取屏幕文字",
                "当前屏幕",
                "提取屏幕文字",
            ),
            ("mute", UiIcon::Mute, "手机静音", "当前屏幕", "手机静音"),
            ("wifi", UiIcon::Wifi, "Wi-Fi 设置", "设置", "Wi-Fi 设置"),
            ("bt", UiIcon::Bluetooth, "蓝牙设置", "设置", "蓝牙设置"),
            (
                "settings",
                UiIcon::Settings,
                "系统设置",
                "设置",
                "打开系统设置",
            ),
            ("home", UiIcon::Home, "返回桌面", "当前屏幕", "返回桌面"),
        ]
        .into_iter()
        .map(|(id, icon, label, app, goal)| {
            self.button_icon(
                format!("amoo-gui-quick-{id}"),
                label,
                icon,
                Action::QuickGuiScenario {
                    app: app.into(),
                    goal: goal.into(),
                },
                Btn::Secondary,
            )
        })
        .collect();

        let quick_bar = Container::new(self.vstack(
            S2,
            vec![
                self.between(
                    self.hstack(
                        6.,
                        vec![
                            self.ui_icon(UiIcon::Bolt, 14., p.accent),
                            self.line_w(
                                "零门槛一键操控 · 即点即达（免配 AI 模型）",
                                CAPTION,
                                p.text,
                                Weight::Semibold,
                            ),
                        ],
                    ),
                    self.hstack(
                        6.,
                        vec![
                            if state.gui_running || state.gui_paused {
                                self.button(
                                    "amoo-gui-top-pause",
                                    if state.gui_paused { "继续" } else { "暂停" },
                                    Action::AmooUi(AmooUiAction::GuiPauseResume),
                                    Btn::Secondary,
                                )
                            } else {
                                Empty::new().finish()
                            },
                            if state.gui_running || state.gui_paused {
                                self.button(
                                    "amoo-gui-top-stop",
                                    "停止",
                                    Action::AmooUi(AmooUiAction::GuiStopWaiting),
                                    Btn::Secondary,
                                )
                            } else {
                                Empty::new().finish()
                            },
                            self.button_icon(
                                "amoo-gui-refresh-screen",
                                "刷新画面",
                                UiIcon::Refresh,
                                Action::AmooUi(AmooUiAction::GuiRefreshScreen),
                                Btn::Ghost,
                            ),
                            self.button_icon(
                                "amoo-gui-open-scrcpy",
                                "实时投屏",
                                UiIcon::Monitor,
                                Action::Mirror,
                                Btn::Ghost,
                            ),
                            self.button(
                                "amoo-gui-back-to-chat",
                                "返回对话",
                                Action::AmooUi(AmooUiAction::GuiMode(false)),
                                Btn::Secondary,
                            ),
                        ],
                    ),
                ),
                self.wrap(quick_chips),
            ],
        ))
        .with_uniform_padding(S3)
        .with_background_color(p.surface)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish();

        let mut rows = vec![
            quick_bar,
            self.between(
                self.hstack(
                    6.,
                    [false, true]
                        .into_iter()
                        .map(|desktop| {
                            let label = if desktop {
                                "智能屏幕操控（推荐）"
                            } else {
                                "手机原生接口"
                            };
                            if state.gui_running || state.gui_unconfirmed {
                                self.disabled_button(label)
                            } else {
                                self.button(
                                    format!("amoo-gui-method:{desktop}"),
                                    label,
                                    Action::AmooUi(AmooUiAction::GuiDesktopMode(desktop)),
                                    if state.gui_desktop == desktop {
                                        Btn::Secondary
                                    } else {
                                        Btn::Ghost
                                    },
                                )
                            }
                        })
                        .collect(),
                ),
                self.button(
                    "amoo-gui-enable-tools",
                    "查看手机 Agent 状态",
                    Action::AmooUi(AmooUiAction::GuiEnableTools),
                    Btn::Ghost,
                ),
            ),
            Flex::row()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(8.)
                .with_child(self.ui_icon(UiIcon::Smartphone, 16., p.text2))
                .with_child(
                    Expanded::new(
                        1.,
                        self.input(
                            F_AMOO_GUI_APP,
                            "目标应用（选填，如：微信 / 设置 / 支付宝；留空直接操作当前屏幕）",
                            None,
                        ),
                    )
                    .finish(),
                )
                .with_child(if state.gui_checking || state.gui_running {
                    self.disabled_button("检查手机")
                } else {
                    self.button(
                        "amoo-gui-check",
                        if locked {
                            "重新检查状态"
                        } else {
                            "检查手机"
                        },
                        Action::AmooUi(AmooUiAction::GuiRefresh),
                        Btn::Secondary,
                    )
                })
                .finish(),
        ];
        if (locked || screen_off) && !state.gui_running {
            rows.push(
                Container::new(self.between(
                    self.hstack(
                        6.,
                        vec![
                            self.ui_icon(UiIcon::Lightbulb, 14., p.warning),
                            self.line_w(
                                if screen_off {
                                    "手机屏幕当前未亮起，可直接在电脑上一键唤醒解锁"
                                } else {
                                    "手机当前处于锁屏界面，可一键上滑解锁或打开投屏输入密码"
                                },
                                CAPTION,
                                p.warning,
                                Weight::Medium,
                            ),
                        ],
                    ),
                    self.hstack(
                        6.,
                        vec![
                            self.button_icon(
                                "amoo-gui-wake-unlock",
                                "一键点亮并滑出解锁",
                                UiIcon::Sun,
                                Action::AmooUi(AmooUiAction::GuiWakeAndSwipeUnlock),
                                Btn::Primary,
                            ),
                            self.button_icon(
                                "amoo-gui-mirror-unlock",
                                "打开投屏用鼠标解锁",
                                UiIcon::Monitor,
                                Action::Mirror,
                                Btn::Secondary,
                            ),
                        ],
                    ),
                ))
                .with_uniform_padding(10.)
                .with_background_color(p.surface)
                .with_border(Border::all(1.).with_border_color(p.warning))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish(),
            );
        }
        if !state.gui_desktop && (state.gui_running || state.gui_paused || state.gui_unconfirmed) {
            rows.push(
                self.hstack(
                    6.,
                    [
                        ("pause", "暂停手机任务"),
                        ("resume", "继续手机任务"),
                        ("stop", "停止手机任务"),
                    ]
                    .into_iter()
                    .map(|(op, label)| {
                        self.button(
                            format!("gui-phone-{op}"),
                            label,
                            Action::AmooUi(AmooUiAction::GuiPhoneControl(op.into())),
                            Btn::Ghost,
                        )
                    })
                    .collect(),
                ),
            );
        }
        rows.push(self.line(
            status,
            MICRO,
            if locked || screen_off || state.gui_unconfirmed {
                p.warning
            } else {
                p.text3
            },
        ));
        self.vstack(S2, rows)
    }

    fn amoo_gui_controls(&self) -> El {
        let left_phone_studio = self.fixed_w(268., self.amoo_gui_phone_studio());
        Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(S3)
            .with_child(left_phone_studio)
            .with_child(Expanded::new(1., self.amoo_gui_controls_panel()).finish())
            .finish()
    }

    fn amoo_gui_step_list(&self, steps: &[(usize, String)], key: &str) -> El {
        let p = self.p();
        let expanded = self.amoo_expanded_tool_records.contains(key);
        let mut rows = vec![self.link_muted(
            format!("{key}-toggle"),
            format!(
                "{} 执行记录 · {} 步",
                if expanded { "收起" } else { "展开" },
                steps.len()
            ),
            Action::AmooToggleToolRecords(key.into()),
        )];
        if expanded {
            rows.extend(steps.iter().map(|(number, detail)| {
                self.text(format!("{number}. {detail}"), CAPTION, p.text2)
            }));
        } else if let Some((number, detail)) = steps.last() {
            let short: String = detail.chars().take(140).collect();
            rows.push(self.text(
                format!(
                    "{number}. {short}{}",
                    if detail.chars().count() > 140 {
                        "…"
                    } else {
                        ""
                    }
                ),
                CAPTION,
                p.text2,
            ));
        }
        self.vstack(5., rows)
    }

    fn amoo_gui_screenshot(&self, path: &std::path::Path) -> El {
        let p = self.p();
        self.vstack(
            5.,
            vec![
                self.line("最近一次手机画面 · 已缓存在本次操作中", MICRO, p.text3),
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
                .with_height(220.)
                .with_max_width(180.)
                .finish(),
            ],
        )
    }

    fn amoo_gui_records(&self) -> El {
        let p = self.p();
        let state = &self.amoo_ui;
        let mut rows = Vec::new();
        if state.gui_records.is_empty() && !state.gui_running {
            let app_demos: Vec<El> = [
                ("wechat", UiIcon::Chat, "打开微信", "微信", "打开微信"),
                ("camera", UiIcon::Camera, "打开相机", "相机", "打开相机"),
                ("gallery", UiIcon::Image, "打开相册", "相册", "打开相册"),
                (
                    "alipay",
                    UiIcon::CreditCard,
                    "打开支付宝",
                    "支付宝",
                    "打开支付宝",
                ),
            ]
            .into_iter()
            .map(|(id, icon, label, app, goal)| {
                self.button_icon(
                    format!("amoo-gui-demo-{id}"),
                    label,
                    icon,
                    Action::QuickGuiScenario {
                        app: app.into(),
                        goal: goal.into(),
                    },
                    Btn::Ghost,
                )
            })
            .collect();
            rows.push(Container::new(self.vstack(10., vec![
                self.hstack(8., vec![self.ui_icon(UiIcon::Smartphone, 22., p.text), self.heading("让 Amoo 操作手机屏幕与应用")]),
                self.text(if state.gui_desktop {
                    "点亮屏幕、截取画面、提取屏幕文字、打开应用、点击按钮、开关设置等常用操作支持零门槛秒级执行；复杂跨页面任务会自动调用视觉模型看屏导航。"
                } else {
                    "填写手机应用名称和具体目标，Amoo 将请求手机原生能力查看界面并执行操作。是否开放取决于当前手机系统的实际支持情况。"
                }, BODY, p.text2),
                self.wrap(app_demos),
                self.text("提示：上方的目标应用可以留空（默认直接操作手机当前画面）。左侧手机画面支持单击直接触控、双击开启独立投屏，右侧与底部按键操作后自动刷新。", CAPTION, p.text3),
            ])).with_uniform_padding(S4).with_background_color(p.surface)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL))).finish());
        }
        if !state.gui_message.is_empty() {
            let mut message = vec![self.text(
                &state.gui_message,
                CAPTION,
                if state.gui_unconfirmed {
                    p.warning
                } else {
                    p.text2
                },
            )];
            if state.gui_running || state.gui_paused {
                if let Some((app, task)) = &state.gui_current {
                    message.insert(0, self.heading(format!("正在请求 · {app}")));
                    message.insert(1, self.text(task, BODY, p.text));
                }
                let elapsed = state
                    .gui_started
                    .map(|t| t.elapsed().as_secs())
                    .unwrap_or(0);
                message.push(self.between(
                    self.line(
                        format!(
                            "已等待 {}分{:02}秒 · 最长 20 分钟",
                            elapsed / 60,
                            elapsed % 60
                        ),
                        MICRO,
                        p.text3,
                    ),
                    self.hstack(
                        6.,
                        vec![
                            self.button(
                                "amoo-gui-pause",
                                if state.gui_paused {
                                    "继续任务"
                                } else {
                                    "暂停任务"
                                },
                                Action::AmooUi(AmooUiAction::GuiPauseResume),
                                Btn::Secondary,
                            ),
                            self.button(
                                "amoo-gui-stop-wait",
                                if state.gui_desktop {
                                    "停止操作"
                                } else {
                                    "停止等待"
                                },
                                Action::AmooUi(AmooUiAction::GuiStopWaiting),
                                Btn::Secondary,
                            ),
                        ],
                    ),
                ));
                message.push(self.text(
                    if state.gui_desktop {
                        "停止后立即结束本次操作；暂停期间你可以随时手动接管手机。"
                    } else {
                        "停止等待只关闭桌面等待，手机上的操作可能继续。"
                    },
                    MICRO,
                    p.text3,
                ));
                if !state.gui_steps.is_empty() {
                    message.push(self.amoo_gui_step_list(&state.gui_steps, "amoo-gui-live-steps"));
                }
                if let Some(path) = &state.gui_screenshot {
                    message.push(self.amoo_gui_screenshot(path));
                }
            }
            if state.gui_unconfirmed {
                message.push(self.text("先在手机上查看并结束上一次操作。检查手机只能读取锁屏与接口状态，不能证明任务已结束。", CAPTION, p.text2));
                message.push(self.button(
                    "amoo-gui-ack-finished",
                    "已在手机确认结束",
                    Action::AmooUi(AmooUiAction::GuiAcknowledgeFinished),
                    Btn::Secondary,
                ));
            }
            if !state.gui_desktop && state.gui_unavailable_reason.is_some() {
                message.push(self.text("当前手机系统拒绝了原生屏幕自动操作，刷新接口或再次提交无法解除限制。可以先通过投屏手动查看手机。", CAPTION, p.text2));
            }
            if (state.gui_unconfirmed
                || (!state.gui_desktop && state.gui_unavailable_reason.is_some()))
                && !self.busy
            {
                message.push(self.button(
                    "amoo-gui-open-mirror",
                    "投屏查看手机",
                    Action::Mirror,
                    Btn::Secondary,
                ));
            }
            rows.push(
                Container::new(self.vstack(8., message))
                    .with_uniform_padding(S3)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(if state.gui_unconfirmed {
                        p.warning
                    } else {
                        p.border
                    }))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
            );
        }
        for (index, record) in state.gui_records.iter().enumerate().rev() {
            let result = &record.result;
            let (label, color) = match result.state {
                GuiRunState::Completed => ("已完成", p.success),
                GuiRunState::Failed => ("执行失败", p.danger),
                GuiRunState::Submitted => ("已接收 · 未确认完成", p.warning),
                GuiRunState::WaitingForUnlock => ("等待手动解锁", p.warning),
                GuiRunState::ScreenOff => ("等待点亮屏幕", p.warning),
                GuiRunState::Unavailable => ("手机暂未开放", p.warning),
                GuiRunState::CancelledBeforeStart => ("发送前已取消", p.text3),
                GuiRunState::WaitingCancelled => (
                    if record.desktop {
                        "已停止后续步骤"
                    } else {
                        "已停止桌面等待"
                    },
                    p.warning,
                ),
                GuiRunState::TimedOut => ("等待超时", p.warning),
                GuiRunState::Disconnected => ("连接中断", p.warning),
                GuiRunState::Unknown => ("结果待确认", p.warning),
            };
            let mut content = vec![
                self.between(
                    self.line_w(&record.app_name, BODY, p.text, Weight::Semibold),
                    self.line(label, MICRO, color),
                ),
                self.text(&record.task, BODY, p.text),
                self.text(&result.message, CAPTION, p.text2),
            ];
            if !record.steps.is_empty() {
                content.push(
                    self.amoo_gui_step_list(&record.steps, &format!("amoo-gui-steps:{index}")),
                );
            }
            if let Some(path) = &record.screenshot {
                content.push(self.amoo_gui_screenshot(path));
            }
            if let Some(raw) = &result.raw_result {
                let key = format!("amoo-gui-result:{index}:{}", result.invocation_id);
                let expanded = self.amoo_expanded_tool_records.contains(&key);
                let toggle = self.link_muted(
                    format!("{key}-toggle"),
                    if expanded {
                        "收起执行详情"
                    } else {
                        "查看执行详情"
                    },
                    Action::AmooToggleToolRecords(key.clone()),
                );
                if expanded {
                    let raw = serde_json::to_string_pretty(raw).unwrap_or_default();
                    content.push(self.between(
                        toggle,
                        self.amoo_action_icon(
                            format!("{key}-copy"),
                            "复制执行结果",
                            UiIcon::Copy,
                            Action::CopyText(raw.clone(), "已复制执行结果".into()),
                        ),
                    ));
                    let visible: String = raw.chars().take(20000).collect();
                    content.push(self.text(visible, MICRO, p.text2));
                    if raw.chars().count() > 20000 {
                        content.push(self.text(
                            "预览已截取前 20000 字；复制可获取完整结果。",
                            MICRO,
                            p.text3,
                        ));
                    }
                } else {
                    content.push(toggle);
                }
            }
            content.push(self.line(
                format!(
                    "{} · 耗时 {:.1} 秒 · 本次桌面记录",
                    if record.desktop {
                        "Amoo GUI Agent"
                    } else {
                        "手机原生"
                    },
                    result.elapsed_ms as f64 / 1000.
                ),
                MICRO,
                p.text3,
            ));
            rows.push(
                Container::new(self.vstack(8., content))
                    .with_uniform_padding(S4)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
            );
        }
        self.vstack(12., rows)
    }

    pub(in super::super) fn amoo_conversation(&self) -> El {
        let p = self.p();
        let Some(sess) = self.amoo_sessions.get(self.amoo_session_idx) else {
            return self.text(
                "点击“新建对话”即可从后台使用 Amoo，无需打开手机界面。",
                CAPTION,
                p.text3,
            );
        };
        if sess.items.is_empty() {
            return self.text("此会话暂无消息，可在下方直接输入文字。", CAPTION, p.text3);
        }

        let mut rows: Vec<El> = Vec::new();
        for (idx, item) in sess.items.iter().enumerate() {
            match item {
                AmooChatItem::Attachment {
                    name,
                    kind,
                    mime,
                    url,
                    path,
                    size_bytes,
                } => {
                    let card =
                        self.amoo_phone_attachment_card(name, kind, mime, url, path, *size_bytes);
                    let dbl_path = path.clone();
                    let dbl_url = url.clone();
                    let menu_path = path.clone();
                    let menu_url = url.clone();
                    let interactive_card = Hoverable::new(
                        self.mouse_state(format!("amoo-phone-att-{idx}")),
                        move |_| card,
                    )
                    .with_defer_events_to_children()
                    .with_cursor(Cursor::PointingHand)
                    .on_double_click(move |ctx, _, _| {
                        if !dbl_path.is_empty() {
                            ctx.dispatch_typed_action(Action::Work(
                                super::super::WorkAction::OpenPreview(dbl_path.clone()),
                            ));
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
                    .finish();
                    rows.push(interactive_card);
                }
                AmooChatItem::ToolRecords { key, records } => {
                    rows.push(self.amoo_execution_records(key, records));
                }
                AmooChatItem::UserText { attachment, text } => {
                    let Some(text) = stepx_companion::work_protocol::display_text(text, true)
                    else {
                        continue;
                    };
                    let mut col = Flex::column()
                        .with_spacing(4.)
                        .with_cross_axis_alignment(CrossAxisAlignment::End);
                    if let Some((att_name, att_meta, is_img)) = attachment {
                        let img_card = Container::new(self.hstack(
                            8.,
                            vec![
                                self.ui_icon_box(
                                    if *is_img { UiIcon::Image } else { UiIcon::File },
                                    28.,
                                    if *is_img { p.blue } else { p.success },
                                    ColorU::white(),
                                    None,
                                ),
                                Flex::column()
                                    .with_spacing(1.)
                                    .with_cross_axis_alignment(CrossAxisAlignment::Start)
                                    .with_child(self.line_w(
                                        att_name.clone(),
                                        CAPTION,
                                        p.text,
                                        Weight::Medium,
                                    ))
                                    .with_child(self.mono(att_meta.clone(), 11., p.text2))
                                    .finish(),
                            ],
                        ))
                        .with_padding_left(6.)
                        .with_padding_right(10.)
                        .with_vertical_padding(6.)
                        .with_background_color(p.bg)
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                        .finish();
                        let preview_path = att_name.clone();
                        let menu_path = att_name.clone();
                        let interactive_att = Hoverable::new(
                            self.mouse_state(format!("amoo-user-att-{idx}")),
                            move |_| img_card,
                        )
                        .with_cursor(Cursor::PointingHand)
                        .on_double_click(move |ctx, _, _| {
                            ctx.dispatch_typed_action(Action::Work(
                                super::super::WorkAction::OpenPreview(preview_path.clone()),
                            ));
                        })
                        .on_right_click(move |ctx, _, at| {
                            ctx.dispatch_typed_action(Action::ContextOpen(
                                crate::ui::context_menu::Target::ChatAttachment {
                                    path: menu_path.clone(),
                                    url: String::new(),
                                    remove_idx: None,
                                },
                                at,
                            ));
                        })
                        .finish();
                        col = col.with_child(interactive_att);
                    }
                    let user_text_bubble =
                        Container::new(self.text(text.clone(), CAPTION, p.accent_fg))
                            .with_horizontal_padding(14.)
                            .with_vertical_padding(8.)
                            .with_background_color(p.accent)
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
                            .finish();
                    col = col.with_child(
                        ConstrainedBox::new(user_text_bubble)
                            .with_max_width(430.)
                            .finish(),
                    );
                    rows.push(
                        self.context_target(
                            format!("amoo-user-msg-{idx}"),
                            crate::ui::context_menu::Target::ChatMessage {
                                assistant: false,
                                text: text.clone(),
                                session_key: Some(sess.session_key.clone()),
                                retry_text: None,
                            },
                            Flex::row()
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_main_axis_alignment(MainAxisAlignment::End)
                                .with_child(col.finish())
                                .finish(),
                        ),
                    );
                }
                AmooChatItem::UserVoice {
                    transcript,
                    duration_sec,
                } => {
                    let voice_bars: Vec<El> =
                        [5., 11., 14., 8., 13., 15., 9., 6., 12., 14., 10., 7.]
                            .into_iter()
                            .map(|h| {
                                ConstrainedBox::new(
                                    Rect::new()
                                        .with_background_color(p.accent_fg)
                                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                            1.,
                                        )))
                                        .finish(),
                                )
                                .with_width(2.)
                                .with_height(h)
                                .finish()
                            })
                            .collect();

                    let play_icon = if self.amoo_voice_playing {
                        UiIcon::Pause
                    } else {
                        UiIcon::Play
                    };
                    let dur_label = format!("0:{duration_sec:02}\"");
                    let v_state = self.mouse_state(format!("amoo-voice-pill:{idx}"));
                    let voice_pill = Hoverable::new(v_state, move |mouse| {
                        let _hovered = mouse.is_mouse_over_element();
                        let t = self.hover_t(&mouse);
                        let bg = if mouse.is_clicked() {
                            p.accent_hover
                        } else {
                            mix_color(p.accent, p.accent_hover, t)
                        };
                        Container::new(self.hstack(
                            8.,
                            vec![
                                ConstrainedBox::new(
                                    Container::new(
                                        Flex::row()
                                            .with_main_axis_size(MainAxisSize::Max)
                                            .with_main_axis_alignment(MainAxisAlignment::Center)
                                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                            .with_child(self.ui_icon(play_icon, 12., p.accent_fg))
                                            .finish(),
                                    )
                                    .with_background_color(rgba(0xFF, 0xFF, 0xFF, 38))
                                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                        10.,
                                    )))
                                    .finish(),
                                )
                                .with_width(20.)
                                .with_height(20.)
                                .finish(),
                                self.hstack(2., voice_bars),
                                self.mono(dur_label, CAPTION, p.accent_fg),
                            ],
                        ))
                        .with_horizontal_padding(12.)
                        .with_vertical_padding(6.)
                        .with_background_color(bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
                        .finish()
                    })
                    .with_cursor(Cursor::PointingHand)
                    .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::AmooToggleVoicePlay))
                    .finish();

                    rows.push(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::End)
                            .with_child(self.hstack(
                                8.,
                                vec![self.line(transcript.clone(), CAPTION, p.text2), voice_pill],
                            ))
                            .finish(),
                    );
                }
                AmooChatItem::AssistantRich {
                    thinking_time,
                    thinking_steps,
                    tools_summary,
                    text,
                    has_bolt_artifact,
                    has_invoice_artifact,
                    has_safety_confirm,
                    time_label,
                } => {
                    let think_state = self.mouse_state(format!("amoo-think-btn:{idx}"));
                    let expanded = self.amoo_thinking_expanded;
                    let think_label = if self.demo_mode {
                        format!("已深度思考 {thinking_time}")
                    } else if !thinking_steps.is_empty() {
                        "思考过程".into()
                    } else {
                        thinking_time.clone()
                    };
                    let think_chip = if thinking_steps.is_empty() {
                        self.line_w(
                            think_label,
                            11.,
                            if thinking_time == "请求失败" {
                                p.danger
                            } else {
                                p.text2
                            },
                            Weight::Medium,
                        )
                    } else {
                        Hoverable::new(think_state, move |mouse| {
                            let _hovered = mouse.is_mouse_over_element();
                            let t = self.hover_t(&mouse);
                            let bg = if mouse.is_clicked() {
                                p.accent_soft
                            } else {
                                mix_color(p.surface, p.hover, t)
                            };
                            let border = if mouse.is_clicked() {
                                p.border_strong
                            } else {
                                mix_color(p.surface_border, p.border_strong, t)
                            };
                            Container::new(self.hstack(
                                4.,
                                vec![
                                    self.line_w(think_label, 11., p.text2, Weight::Medium),
                                    self.ui_icon(
                                        if expanded {
                                            UiIcon::ChevronDown
                                        } else {
                                            UiIcon::ChevronRight
                                        },
                                        12.,
                                        p.text2,
                                    ),
                                ],
                            ))
                            .with_horizontal_padding(6.)
                            .with_vertical_padding(2.)
                            .with_background_color(bg)
                            .with_border(Border::all(1.).with_border_color(border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                            .finish()
                        })
                        .with_cursor(Cursor::PointingHand)
                        .on_click(|ctx, _, _| ctx.dispatch_typed_action(Action::AmooToggleThinking))
                        .finish()
                    };

                    let thinking_bar = Container::new(self.between(
                        self.hstack(
                            6.,
                            vec![
                                think_chip,
                                if self.demo_mode {
                                    self.hstack(
                                        4.,
                                        vec![
                                            self.ui_icon(UiIcon::Check, 12., p.success),
                                            self.line_w(
                                                format!("已调用 {} 项工具", thinking_steps.len()),
                                                11.,
                                                p.success,
                                                Weight::Medium,
                                            ),
                                        ],
                                    )
                                } else {
                                    Empty::new().finish()
                                },
                            ],
                        ),
                        self.line(tools_summary.clone(), 11., p.text2),
                    ))
                    .with_padding_bottom(6.)
                    .with_border(Border::bottom(1.).with_border_color(p.border))
                    .finish();

                    let mut card_children: Vec<El> = Vec::new();
                    if !thinking_steps.is_empty()
                        || thinking_time == "请求失败"
                        || !tools_summary.is_empty()
                    {
                        card_children.push(thinking_bar);
                    }

                    if expanded && !thinking_steps.is_empty() {
                        let step_lines: Vec<El> = thinking_steps
                            .iter()
                            .map(|s| self.text(s.clone(), 11., p.text2))
                            .collect();
                        card_children.push(
                            Container::new(self.vstack(4., step_lines))
                                .with_uniform_padding(8.)
                                .with_background_color(p.surface)
                                .with_border(Border::all(1.).with_border_color(p.surface_border))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish(),
                        );
                    }

                    let display = text
                        .as_deref()
                        .and_then(|body| stepx_companion::work_protocol::display_text(body, false));
                    if let Some(body_txt) = display.as_deref() {
                        let body_txt = self
                            .live_reply_text(idx, body_txt)
                            .replace("\u{1f5bc}\u{fe0f} ", "")
                            .replace("\u{1f5bc} ", "")
                            .replace('\u{1f5bc}', "");
                        let marker = "[AI 生成图片卡片 · 手机端已渲染高清写真]";
                        if self.demo_mode && body_txt.contains(marker) {
                            let parts: Vec<&str> = body_txt.splitn(2, marker).collect();
                            let before = parts.first().copied().unwrap_or("").trim();
                            let after = parts.get(1).copied().unwrap_or("").trim();
                            if !before.is_empty() {
                                card_children.push(
                                    self.work_rich_text(before, &format!("amoo-{idx}-before")),
                                );
                            }
                            let img_path = ensure_amoo_tti_portrait_path();
                            let tti_state = self.mouse_state(format!("tti-img-zoom:{idx}"));
                            let is_open = self.amoo_lightbox_open;
                            let zoom_pct = self.amoo_lightbox_zoom;
                            let img_sz = if is_open {
                                if zoom_pct > 100 {
                                    330.
                                } else {
                                    275.
                                }
                            } else {
                                186.
                            };
                            let tti_image = Hoverable::new(tti_state, move |mouse| {
                                let hovered = mouse.is_mouse_over_element();
                                let t = self.hover_t(&mouse);
                                let bg = if is_open {
                                    rgb(0x10, 0x12, 0x18)
                                } else if mouse.is_clicked() {
                                    p.accent_soft
                                } else {
                                    mix_color(p.surface, p.hover, t)
                                };
                                let border = if is_open || mouse.is_clicked() {
                                    p.accent
                                } else {
                                    mix_color(p.border, p.accent, t)
                                };
                                Container::new(self.vstack(
                                    6.,
                                    vec![
                                        ConstrainedBox::new(
                                            Image::new(
                                                AssetSource::LocalFile {
                                                    path: img_path.clone(),
                                                },
                                                CacheOption::BySize,
                                            )
                                            .contain()
                                            .finish(),
                                        )
                                        .with_width(img_sz)
                                        .with_height(img_sz)
                                        .finish(),
                                        self.between(
                                            self.line_w(
                                                if is_open {
                                                    format!(
                                                        "大图 · {}%",
                                                        zoom_pct
                                                    )
                                                } else {
                                                    "点击查看大图".into()
                                                },
                                                11.,
                                                if is_open || mouse.is_clicked() || hovered {
                                                    p.accent
                                                } else {
                                                    mix_color(p.text2, p.accent, t)
                                                },
                                                Weight::Medium,
                                            ),
                                            self.mono("2K PNG", 10.5, p.accent),
                                        ),
                                    ],
                                ))
                                .with_uniform_padding(if is_open { 10. } else { 6. })
                                .with_background_color(bg)
                                .with_border(
                                    Border::all(if is_open { 1.5 } else { 1. })
                                        .with_border_color(border),
                                )
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                                .finish()
                            })
                            .with_cursor(Cursor::PointingHand)
                            .on_click(|ctx, _, _| {
                                ctx.dispatch_typed_action(Action::AmooToggleLightbox)
                            })
                            .finish();

                            let mut action_btns = vec![self.button(
                                format!("tti-btn-zoom:{idx}"),
                                if self.amoo_lightbox_open {
                                    "收起大图"
                                } else {
                                    "查看大图"
                                },
                                Action::AmooToggleLightbox,
                                Btn::Primary,
                            )];
                            if self.amoo_lightbox_open {
                                action_btns.push(self.button_icon(
                                    format!("tti-btn-scale:{idx}"),
                                    if self.amoo_lightbox_zoom > 100 {
                                        "100% 原始比例"
                                    } else {
                                        "150% 细节放大"
                                    },
                                    UiIcon::Search,
                                    Action::AmooToggleLightboxZoom,
                                    Btn::Secondary,
                                ));
                            }
                            action_btns.extend([
                                self.button_icon(
                                    format!("tti-btn-save:{idx}"),
                                    "保存到电脑",
                                    UiIcon::Download,
                                    Action::AmooSaveLightboxImage,
                                    Btn::Secondary,
                                ),
                                self.button(
                                    format!("tti-btn-matrix:{idx}"),
                                    "做成背屏图案",
                                    Action::AmooPushArtifactBolt,
                                    Btn::Secondary,
                                ),
                            ]);
                            let img_actions = self.hstack(6., action_btns);
                            card_children.push(self.vstack(
                                6.,
                                vec![
                                        Flex::row()
                                            .with_main_axis_size(MainAxisSize::Min)
                                            .with_child(tti_image)
                                            .finish(),
                                        img_actions,
                                    ],
                            ));
                            if !after.is_empty() {
                                card_children
                                    .push(self.work_rich_text(after, &format!("amoo-{idx}-after")));
                            }
                        } else {
                            card_children.push(self.work_rich_text(
                                &body_txt,
                                &format!("amoo-{}-{idx}", sess.session_key),
                            ));
                        }
                    }

                    if *has_bolt_artifact || *has_invoice_artifact {
                        let mut art_row = Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_spacing(8.);

                        if *has_bolt_artifact {
                            let bolt_frame = BackScreenPattern::Bolt.frame_391(7, false);
                            let mini_bolt_box = ConstrainedBox::new(
                                Container::new(
                                    Flex::row()
                                        .with_main_axis_size(MainAxisSize::Max)
                                        .with_main_axis_alignment(MainAxisAlignment::Center)
                                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                        .with_child(self.matrix_grid(&bolt_frame, 0.9, 0.25, false))
                                        .finish(),
                                )
                                .with_background_color(rgb(0x09, 0x09, 0x0C))
                                .with_border(
                                    Border::all(1.).with_border_color(rgb(0x2E, 0x2E, 0x38)),
                                )
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                                .finish(),
                            )
                            .with_width(28.)
                            .with_height(32.)
                            .finish();

                            let art_1 = Container::new(self.between(
                                self.hstack(
                                    8.,
                                    vec![
                                        mini_bolt_box,
                                        Flex::column()
                                            .with_spacing(1.)
                                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                                            .with_child(self.line_w(
                                                "赛博闪电",
                                                CAPTION,
                                                p.text,
                                                Weight::Semibold,
                                            ))
                                            .with_child(self.line("23×17 点阵", 11., p.text2))
                                            .finish(),
                                    ],
                                ),
                                if self.amoo_bolt_pushed {
                                    self.button_icon(
                                        format!("amoo-art-push:{idx}"),
                                        "已投背屏",
                                        UiIcon::Check,
                                        Action::AmooPushArtifactBolt,
                                        Btn::Primary,
                                    )
                                } else {
                                    self.button(
                                        format!("amoo-art-push:{idx}"),
                                        "投到背屏",
                                        Action::AmooPushArtifactBolt,
                                        Btn::Primary,
                                    )
                                },
                            ))
                            .with_uniform_padding(8.)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.surface_border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish();
                            art_row = art_row.with_child(Expanded::new(1., art_1).finish());
                        }

                        if *has_invoice_artifact {
                            let invoice_icon_box = ConstrainedBox::new(
                                Container::new(
                                    Flex::row()
                                        .with_main_axis_size(MainAxisSize::Max)
                                        .with_main_axis_alignment(MainAxisAlignment::Center)
                                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                                        .with_child(self.ui_icon(
                                            UiIcon::Chart,
                                            16.,
                                            p.success_text,
                                        ))
                                        .finish(),
                                )
                                .with_background_color(p.success_soft)
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish(),
                            )
                            .with_width(28.)
                            .with_height(32.)
                            .finish();

                            let art_2 = Container::new(self.between(
                                self.hstack(
                                    8.,
                                    vec![
                                        invoice_icon_box,
                                        Flex::column()
                                            .with_spacing(1.)
                                            .with_cross_axis_alignment(CrossAxisAlignment::Start)
                                            .with_child(self.line_w(
                                                "10月报销单",
                                                CAPTION,
                                                p.text,
                                                Weight::Semibold,
                                            ))
                                            .with_child(self.line("已记 ¥386.00", 11., p.text2))
                                            .finish(),
                                    ],
                                ),
                                self.button(
                                    format!("amoo-art-open:{idx}"),
                                    if self.amoo_invoice_open {
                                        "收起"
                                    } else {
                                        "打开"
                                    },
                                    Action::AmooToggleInvoiceDetail,
                                    Btn::Secondary,
                                ),
                            ))
                            .with_uniform_padding(8.)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.surface_border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish();
                            art_row = art_row.with_child(Expanded::new(1., art_2).finish());
                        }

                        card_children.push(art_row.finish());
                    }

                    if *has_invoice_artifact && self.amoo_invoice_open {
                        let inv_detail = Container::new(self.vstack(
                            4.,
                            vec![
                                self.between(
                                    self.line_w(
                                        "10月差旅与日常报销台账（共 3 笔）",
                                        CAPTION,
                                        p.text,
                                        Weight::Semibold,
                                    ),
                                    self.link_action(
                                        format!("amoo-inv-copy:{idx}"),
                                        "复制台账",
                                        Action::CopyText(
                                            "10月报销单：\n1. 高德打车电子发票 ¥68.00\n2. 差旅餐饮发票 ¥128.00\n3. 办公耗材发票 ¥190.00\n合计：¥386.00".into(),
                                            "已复制 10 月报销单台账明细".into(),
                                        ),
                                    ),
                                ),
                                self.mono(
                                    "1. 10-21 高德打车电子发票 · ¥68.00   2. 10-19 差旅餐饮 · ¥128.00   3. 10-15 办公耗材 · ¥190.00",
                                    11.,
                                    p.text2,
                                ),
                            ],
                        ))
                        .with_uniform_padding(8.)
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(p.surface_border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish();
                        card_children.push(inv_detail);
                    }

                    if *has_safety_confirm {
                        let safety_card_el = match self.amoo_safety_decision {
                            SafetyDecision::Pending => Container::new(
                                self.between(
                                    Flex::column()
                                        .with_spacing(2.)
                                        .with_cross_axis_alignment(CrossAxisAlignment::Start)
                                        .with_child(self.lead(
                                            6.,
                                            self.dot(p.warning),
                                            self.line_w(
                                                "待确认 · 发送短信给李明",
                                                CAPTION,
                                                p.text,
                                                Weight::Semibold,
                                            ),
                                        ))
                                        .with_child(self.line(
                                            "138 0013 8000 · “我取完快递马上过去。”",
                                            11.,
                                            p.text2,
                                        ))
                                        .finish(),
                                    self.hstack(
                                        6.,
                                        vec![
                                            self.button(
                                                format!("amoo-confirm-reject:{idx}"),
                                                "拒绝",
                                                Action::AmooConfirmSafety(false),
                                                Btn::Secondary,
                                            ),
                                            self.button(
                                                format!("amoo-confirm-allow:{idx}"),
                                                "允许执行",
                                                Action::AmooConfirmSafety(true),
                                                Btn::Primary,
                                            ),
                                        ],
                                    ),
                                ),
                            )
                            .with_uniform_padding(10.)
                            .with_background_color(p.warning_soft)
                            .with_border(Border::all(1.).with_border_color(p.warning_border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish(),
                            SafetyDecision::Approved => Container::new(self.between(
                                self.lead(
                                    6.,
                                    self.ui_icon(UiIcon::Check, 14., p.success_text),
                                    self.line_w(
                                        "已允许执行 · 已向李明 (138 0013 8000) 发送短信",
                                        CAPTION,
                                        p.success_text,
                                        Weight::Semibold,
                                    ),
                                ),
                                self.link_muted(
                                    format!("amoo-safety-undo-inline:{idx}"),
                                    "撤回",
                                    Action::AmooUndoSafety,
                                ),
                            ))
                            .with_uniform_padding(10.)
                            .with_background_color(p.success_soft)
                            .with_border(Border::all(1.).with_border_color(p.success))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish(),
                            SafetyDecision::Rejected => Container::new(self.between(
                                self.lead(
                                    6.,
                                    self.ui_icon(UiIcon::Close, 14., p.text3),
                                    self.line_w(
                                        "已拒绝执行 · 已拦截向李明发送短信",
                                        CAPTION,
                                        p.text2,
                                        Weight::Medium,
                                    ),
                                ),
                                self.link_muted(
                                    format!("amoo-safety-reopen:{idx}"),
                                    "重新审议",
                                    Action::AmooUndoSafety,
                                ),
                            ))
                            .with_uniform_padding(10.)
                            .with_background_color(p.surface)
                            .with_border(Border::all(1.).with_border_color(p.surface_border))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                            .finish(),
                        };
                        card_children.push(safety_card_el);
                    }

                    let copy_payload = display.unwrap_or_else(|| thinking_steps.join("\n"));
                    let mut reply_actions = vec![self.amoo_action_icon(
                        format!("amoo-reply-copy:{idx}"),
                        "复制回复",
                        UiIcon::Copy,
                        Action::CopyText(copy_payload.clone(), "已复制 Amoo 回复内容".into()),
                    )];
                    reply_actions.push(self.speech_icon(
                        format!("amoo-speech:{}:{idx}", sess.session_key),
                        copy_payload.clone(),
                    ));
                    if self.page == super::super::Page::Amoo && !self.amoo_ui.screen_mode {
                        reply_actions.push(self.amoo_action_icon(
                            format!("amoo-reply-quote:{idx}"),
                            "引用这段继续问",
                            UiIcon::Chat,
                            Action::Workspace(
                                super::super::workspace_state::WorkspaceAction::QuoteTurn(
                                    copy_payload.clone(),
                                ),
                            ),
                        ));
                    }
                    let retry_text_for_menu = retry_prompt(&sess.items, idx);
                    let menu_text = copy_payload.clone();
                    if let Some(text) = retry_text_for_menu.clone() {
                        reply_actions.push(self.amoo_action_icon(
                            format!("amoo-reply-retry:{idx}"),
                            "重新生成",
                            UiIcon::Retry,
                            Action::AmooRetryMessage {
                                session_key: sess.session_key.clone(),
                                text,
                            },
                        ));
                    }
                    let reply_footer = self.between(
                        self.hstack(2., reply_actions),
                        self.mono(time_label.clone(), 11., p.text2),
                    );
                    card_children.push(reply_footer);

                    let reply_card = Container::new(self.vstack(12., card_children))
                        .with_vertical_padding(10.)
                        .with_horizontal_padding(4.)
                        .finish();

                    let avatar_badge = ConstrainedBox::new(
                        Container::new(
                            Align::new(self.ui_icon(UiIcon::Star, 15., p.accent)).finish(),
                        )
                        .with_background_color(p.surface)
                        .with_border(Border::all(1.).with_border_color(p.border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(14.)))
                        .finish(),
                    )
                    .with_width(28.)
                    .with_height(28.)
                    .finish();
                    let avatar_a = Container::new(avatar_badge).with_padding_top(6.).finish();

                    rows.push(
                        self.context_target(
                            format!("amoo-ai-msg-{idx}"),
                            crate::ui::context_menu::Target::ChatMessage {
                                assistant: true,
                                text: menu_text,
                                session_key: Some(sess.session_key.clone()),
                                retry_text: retry_text_for_menu,
                            },
                            Flex::row()
                                .with_main_axis_size(MainAxisSize::Max)
                                .with_cross_axis_alignment(CrossAxisAlignment::Start)
                                .with_spacing(10.)
                                .with_child(avatar_a)
                                .with_child(Expanded::new(1., reply_card).finish())
                                .finish(),
                        ),
                    );
                }
            }
        }

        self.vstack(10., rows)
    }

    /// Lines read from the phone screen; click a line to add it to the question.
    fn screen_text_panel(&self) -> Option<El> {
        if self.content.is_empty() || self.output_page != super::super::Page::Amoo {
            return None;
        }
        let p = self.p();
        let lines: Vec<&str> = self
            .content
            .lines()
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .collect();
        let shown: Vec<El> = lines
            .iter()
            .take(3)
            .enumerate()
            .map(|(i, line)| {
                self.list_row(
                    format!("screen-{i}"),
                    self.line(*line, CAPTION, p.text2),
                    false,
                    Some(Action::AppendVisibleLine((*line).to_string())),
                    None,
                )
            })
            .collect();
        let header = self.between(
            self.line("屏幕文字 · 点击一行加入问题", CAPTION, p.text2),
            self.hstack(
                S3,
                vec![
                    self.link("screen-copy", "复制全部", Action::Copy),
                    self.link("screen-close", "关闭", Action::ClearContent),
                ],
            ),
        );
        Some(self.vstack(S2, vec![header, self.list(shown)]))
    }
}

#[cfg(test)]
mod gui_display_tests {
    use super::*;

    #[test]
    fn explicit_lock_blocks_are_not_shown_as_task_completion() {
        assert!(reply_reports_lock_block(
            "手机还锁着，截图被拦截了，天气任务无法继续。"
        ));
        assert!(reply_reports_lock_block("请先解锁手机，然后我再查询天气。"));
        assert!(!reply_reports_lock_block("已查到天气，无需解锁手机。"));
        assert!(!reply_reports_lock_block("解锁后已完成，天气是晴天。"));
        assert!(!session_waits_for_unlock(&[AmooChatItem::ToolRecords {
            key: "logs".into(),
            records: Vec::new(),
        }]));
    }
}
