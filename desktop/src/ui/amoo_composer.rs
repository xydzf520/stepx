//! One composer for full and compact Amoo: draft, attachments and actions share a surface.
use super::{icons::UiIcon, theme::*, widgets::*, *};
use facloud_ui::{
    assets::asset_cache::AssetSource, color::ColorU, fonts::Weight, platform::Cursor,
};

impl Companion {
    /// Keep room to read the conversation on short windows. The input and
    /// attachments scroll independently instead of consuming the entire pane.
    pub(super) fn composer_limits(&self, compact: bool) -> (f32, f32) {
        if self.workspace.viewport.y() < 520. {
            (48., 44.)
        } else if self.workspace.viewport.y() < 560. {
            (64., 52.)
        } else if self.workspace.viewport.y() < 720. {
            (96., 64.)
        } else if compact {
            (126., 88.)
        } else {
            (160., 104.)
        }
    }
    fn composer_icon(
        &self,
        id: impl Into<String>,
        label: &str,
        icon: UiIcon,
        action: Option<Action>,
        primary: bool,
    ) -> El {
        let id = id.into();
        let p = self.p();
        let enabled = action.is_some();
        let label = label.to_owned();
        let control = Hoverable::new(self.mouse_state(id.clone()), move |mouse| {
            let hovered = mouse.is_mouse_over_element();
            let t = self.hover_t(&mouse);
            let clicked = enabled && mouse.is_clicked();
            let fg = if primary && enabled {
                p.accent_fg
            } else if clicked {
                p.text
            } else if enabled {
                lerp_color(p.text2, p.text, t)
            } else {
                p.text3
            };
            let bg = if primary && enabled {
                if clicked {
                    p.accent_hover
                } else {
                    lerp_color(p.accent, p.accent_hover, t)
                }
            } else if clicked {
                p.accent_soft
            } else if primary {
                p.hover
            } else if enabled {
                lerp_color(ColorU::transparent_black(), p.hover, t)
            } else {
                ColorU::transparent_black()
            };
            let button = ConstrainedBox::new(
                Container::new(Align::new(self.ui_icon(icon, 18., fg)).finish())
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(if primary {
                        16.
                    } else {
                        R_CONTROL
                    })))
                    .finish(),
            )
            .with_width(32.)
            .with_height(32.)
            .finish();
            if !hovered {
                return button;
            }
            let tooltip = Container::new(self.line(&label, MICRO, p.text))
                .with_horizontal_padding(S2)
                .with_vertical_padding(S1)
                .with_background_color(p.bg)
                .with_border(Border::all(1.).with_border_color(p.border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                .finish();
            Stack::new()
                .with_child(button)
                .with_positioned_child(
                    tooltip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., -S1),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::TopMiddle,
                        ChildAnchor::BottomMiddle,
                    ),
                )
                .finish()
        })
        .with_cursor(if enabled {
            Cursor::PointingHand
        } else {
            Cursor::Arrow
        })
        .on_click(move |ctx, _, _| {
            if let Some(action) = &action {
                ctx.dispatch_typed_action(action.clone());
            }
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let control = super::scenario_check::control(id, control);
        control
    }

    /// Placeholder follows the conversation: once something was made, invite
    /// the user to keep editing it in place instead of switching pages.
    fn amoo_composer_placeholder(&self, phone_mode: bool) -> String {
        if phone_mode {
            return "描述要在手机上完成的操作…".into();
        }
        let target_output = self.workspace_task().and_then(|t| {
            self.workspace
                .selected_output
                .and_then(|idx| t.outputs.get(idx))
                .or_else(|| t.outputs.last())
        });
        match target_output {
            Some(output) => format!(
                "说说要怎么改「{}」，或问点别的…",
                Self::truncate_chars(&output.name, 24)
            ),
            None => "直接说需求：聊天、做 PPT/文档/表格、做小游戏都可以…".into(),
        }
    }

    /// Live hint of what Enter will do. Keeps the `workspace-intent-anchor`
    /// position so the intent menu (Menu(7)) still opens next to it.
    fn amoo_route_hint(&self) -> El {
        use stepx_companion::amoo_workspace::{preview_route, Intent};
        let draft = self.fields[F_AMOO].text.trim();
        let forced = match self.workspace.intent {
            Intent::Files | Intent::Image | Intent::App | Intent::Screen | Intent::Chat => {
                Some(route_label(self.workspace.intent))
            }
            Intent::Auto => None,
        };
        let detected = if draft.is_empty() {
            None
        } else {
            preview_route(
                draft,
                !self.amoo_attachments.is_empty(),
                self.workspace_task().and_then(|t| t.route),
            )
            .map(|(_, label)| label)
        };
        let (label, active) = match (forced, detected) {
            (Some(label), _) => (label.to_string(), true),
            (None, Some(label)) => (format!("将生成：{label}"), true),
            (None, None) => (route_label(Intent::Auto).to_string(), false),
        };
        SavePosition::new(
            self.chip(
                "workspace-composer-intent",
                label,
                Some(UiIcon::ChevronDown),
                vec![Action::Workspace(WorkspaceAction::Menu(7))],
                active,
                !active,
            ),
            "workspace-intent-anchor",
        )
        .for_single_frame()
        .finish()
    }

    fn amoo_composer_editor(&self, phone_mode: bool, compact: bool) -> El {
        let p = self.p();
        let field = &self.fields[F_AMOO];
        let focused = self.focus == Some(F_AMOO);
        let empty = field.text.is_empty() && field.preedit.is_empty();
        let placeholder = self.amoo_composer_placeholder(phone_mode);
        let placeholder = placeholder.as_str();
        let mut value = field.text.clone();
        let mut caret = 0;
        if focused {
            let cursor = field.cursor.min(value.len());
            // Text's saved positions use Unicode character indices; Input uses
            // UTF-8 byte offsets for editing. Convert before positioning IME/scroll.
            caret = value[..cursor].chars().count() + field.preedit.chars().count();
            value.insert_str(cursor, &field.preedit);
        }
        if empty {
            value.push_str(placeholder);
        }
        let editor = self.pane_scroll(
            self.amoo_ui.composer_scroll.clone(),
            Container::new(self.multiline_field_text(
                &value,
                caret,
                F_AMOO,
                if empty { p.text3 } else { p.text },
            ))
            .with_padding_bottom(S1)
            .finish(),
        );
        // Use measured wrapped text height, not a character-count estimate. Long
        // drafts scroll independently, keeping both the conversation and tools visible.
        SavePosition::new(
            Hoverable::new(self.mouse_state("amoo-composer-editor"), move |_| {
                ConstrainedBox::new(
                    Container::new(editor)
                        .with_uniform_padding(S1)
                        .with_background_color(if field.all {
                            p.accent_soft
                        } else {
                            ColorU::transparent_black()
                        })
                        .finish(),
                )
                .with_min_height(
                    (if compact { 48.0_f32 } else { 68.0 }).min(self.composer_limits(compact).0),
                )
                .with_max_height(self.composer_limits(compact).0)
                .finish()
            })
            .with_cursor(Cursor::IBeam)
            .with_defer_events_to_children()
            .on_mouse_down(|ctx, _, _| ctx.dispatch_typed_action(Action::Focus(F_AMOO)))
            .on_right_click(|ctx, _, at| {
                ctx.dispatch_typed_action(Action::ContextOpen(
                    context_menu::Target::Input(F_AMOO),
                    at,
                ))
            })
            .finish(),
            "composer-editor",
        )
        .for_single_frame()
        .finish()
    }

    pub(super) fn amoo_message_composer(&self, compact: bool, phone_mode: bool) -> El {
        let p = self.p();
        let focused = self.focus == Some(F_AMOO);
        let pending = if let Some(t) = self.workspace_task() {
            t.session
                .as_ref()
                .is_some_and(|key| self.amoo_pending_reply.contains_key(key))
        } else {
            self.amoo_sessions
                .get(self.amoo_session_idx)
                .is_some_and(|s| self.amoo_pending_reply.contains_key(&s.session_key))
        };
        let running = if phone_mode {
            self.busy || self.workspace_busy()
        } else {
            self.workspace_active_busy()
        };
        let occupied = running;
        let connected = self.selected.is_some() || self.demo_mode;
        let gui_blocked = phone_mode
            && (self.amoo_ui.gui_checking
                || self.amoo_ui.gui_unconfirmed
                || (!self.amoo_ui.gui_desktop && self.amoo_ui.gui_unavailable_reason.is_some()));
        let has_content = !self.fields[F_AMOO].text.trim().is_empty()
            || (!phone_mode && !self.amoo_attachments.is_empty());
        let gui_has_attachments = phone_mode && !self.amoo_attachments.is_empty();
        let can_send = connected
            && has_content
            && !occupied
            && !gui_blocked
            && !gui_has_attachments
            && (phone_mode || !pending)
            && self.fields[F_AMOO].preedit.is_empty();
        let can_send = if phone_mode {
            can_send
        } else {
            self.workspace_can_send()
        };
        let can_stop = if phone_mode {
            self.amoo_ui.gui_running || self.amoo_ui.gui_paused
        } else {
            self.workspace.running.is_some()
                || pending
                || self.amoo_ui.gui_running
                || self.amoo_ui.gui_paused
        };
        let stop_label = "停止";
        let mut rows = Vec::new();
        if !self.amoo_attachments.is_empty() && !phone_mode {
            if self.amoo_attachments.len() > 1 {
                rows.push(self.between(
                    self.line(
                        format!("已添加 {} 个附件", self.amoo_attachments.len()),
                        MICRO,
                        p.text2,
                    ),
                    self.line("滚动查看", MICRO, p.text3),
                ));
            }
            let chips = self
                .amoo_attachments
                .iter()
                .enumerate()
                .map(|(index, att)| {
                    let visual = if att.is_image {
                        match &att.local_path {
                            Some(path) => ConstrainedBox::new(
                                Image::new(
                                    AssetSource::LocalFile {
                                        path: path.to_string_lossy().into_owned(),
                                    },
                                    CacheOption::BySize,
                                )
                                .contain()
                                .finish(),
                            )
                            .with_width(32.)
                            .with_height(32.)
                            .finish(),
                            None => self.ui_icon(UiIcon::Image, 24., p.text2),
                        }
                    } else {
                        self.ui_icon(UiIcon::File, 24., p.text2)
                    };
                    let chip_row = self.hstack(
                        S2,
                        vec![
                            visual,
                            ConstrainedBox::new(self.vstack(
                                2.,
                                vec![
                                    self.line_w(&att.name, CAPTION, p.text, Weight::Medium),
                                    self.line(&att.size_label, MICRO, p.text3),
                                ],
                            ))
                            .with_max_width(if compact { 150. } else { 190. })
                            .finish(),
                            self.composer_icon(
                                format!("amoo-composer-remove-{index}"),
                                "移除附件",
                                UiIcon::Close,
                                (!occupied).then_some(Action::AmooRemoveAttachment(index)),
                                false,
                            ),
                        ],
                    );
                    let mut h = Hoverable::new(
                        self.mouse_state(format!("amoo-att-card-{index}")),
                        move |mouse| {
                            let t = self.hover_t(&mouse);
                            Container::new(chip_row)
                                .with_padding_left(S2)
                                .with_padding_right(S1)
                                .with_vertical_padding(S1)
                                .with_background_color(lerp_color(p.surface, p.hover, t * 0.45))
                                .with_border(Border::all(1.).with_border_color(lerp_color(
                                    p.border,
                                    p.border_strong,
                                    t,
                                )))
                                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                    R_CONTROL,
                                )))
                                .finish()
                        },
                    )
                    .with_defer_events_to_children();
                    if let Some(local_path) = &att.local_path {
                        let preview_click = local_path.display().to_string();
                        let preview_double = preview_click.clone();
                        let menu_path = preview_click.clone();
                        h = h
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::Work(WorkAction::OpenPreview(
                                    preview_click.clone(),
                                )));
                            })
                            .on_double_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::Work(WorkAction::OpenPreview(
                                    preview_double.clone(),
                                )));
                            })
                            .on_right_click(move |ctx, _, at| {
                                ctx.dispatch_typed_action(Action::ContextOpen(
                                    crate::ui::context_menu::Target::ChatAttachment {
                                        path: menu_path.clone(),
                                        url: String::new(),
                                        remove_idx: Some(index),
                                    },
                                    at,
                                ));
                            });
                    }
                    h.finish()
                })
                .collect();
            rows.push(
                SavePosition::new(
                    ConstrainedBox::new(
                        self.pane_scroll(self.amoo_ui.attachment_scroll.clone(), self.wrap(chips)),
                    )
                    .with_max_height(self.composer_limits(compact).1)
                    .finish(),
                    "composer-attachments",
                )
                .for_single_frame()
                .finish(),
            );
        }
        rows.push(self.amoo_composer_editor(phone_mode, compact));
        let attachments_enabled = !occupied;
        let attach_open = !phone_mode
            && attachments_enabled
            && self.amoo_expanded_tool_records.contains(ATTACH_MENU_KEY);
        if attach_open {
            let close = Action::AmooToggleToolRecords(ATTACH_MENU_KEY.into());
            let mut items = vec![
                self.chip(
                    "amoo-composer-image",
                    "图片",
                    Some(UiIcon::Image),
                    vec![close.clone(), Action::AmooAttachFile(true)],
                    false,
                    false,
                ),
                self.chip(
                    "amoo-composer-file",
                    "文件",
                    Some(UiIcon::Attach),
                    vec![close.clone(), Action::AmooAttachFile(false)],
                    false,
                    false,
                ),
            ];
            if connected {
                items.push(self.chip(
                    "amoo-composer-album",
                    "手机相册",
                    Some(UiIcon::Smartphone),
                    vec![close, Action::AmooAttachPhoneAlbum],
                    false,
                    false,
                ));
            }
            rows.push(self.wrap(items));
        }
        let hint: String = if !connected {
            if self.connection_watch.detached_serial.is_some() {
                "正在自动重连手机…".into()
            } else {
                "请连接手机".into()
            }
        } else if phone_mode && self.busy {
            "发送中…".into()
        } else if self.amoo_ui.gui_running {
            "手机正在执行…".into()
        } else if !phone_mode && pending {
            if self.workspace.stream_delta_count > 0 {
                "正在回复…".into()
            } else if let Some(run) = &self.workspace.running {
                format!("正在思考（{} 秒）", run.started_at.elapsed().as_secs())
            } else {
                "正在回复…".into()
            }
        } else if gui_has_attachments {
            "操作手机时不能带附件".into()
        } else if gui_blocked {
            "手机操作暂不可用".into()
        } else if occupied {
            "正在处理…".into()
        } else {
            "Enter 发送 · Shift+Enter 换行".into()
        };
        let short_hint: &str = if !connected {
            if self.connection_watch.detached_serial.is_some() {
                "重连中…"
            } else {
                "未连接"
            }
        } else if phone_mode && self.busy {
            "发送中…"
        } else if !phone_mode && pending {
            "正在回复…"
        } else if gui_has_attachments || gui_blocked {
            "暂不可用"
        } else if occupied || self.amoo_ui.gui_running {
            "处理中…"
        } else {
            "Enter 发送"
        };
        let mut actions = Vec::new();
        if !phone_mode {
            actions.push(self.composer_icon(
                "amoo-composer-attach",
                if attach_open {
                    "收起"
                } else {
                    "添加图片或文件"
                },
                if attach_open {
                    UiIcon::Close
                } else {
                    UiIcon::Plus
                },
                attachments_enabled.then(|| Action::AmooToggleToolRecords(ATTACH_MENU_KEY.into())),
                false,
            ));
            actions.push(self.amoo_route_hint());
        }
        let hint_text = self.line(
            if self.workspace.viewport.x() < 520. {
                short_hint
            } else {
                hint.as_str()
            },
            MICRO,
            p.text3,
        );
        let hint_el = if occupied || pending {
            let pulse = self.pulse_t();
            let dot_col = lerp_color(p.accent_soft, p.accent, pulse);
            self.hstack(S1 + 2., vec![self.dot(dot_col), hint_text])
        } else {
            hint_text
        };
        rows.push(
            SavePosition::new(
                self.hstack(
                    S2,
                    vec![
                        self.hstack(S1, actions),
                        Expanded::new(1., Align::new(hint_el).right().finish()).finish(),
                        SavePosition::new(
                            self.composer_icon(
                                "amoo-composer-send",
                                if can_stop {
                                    stop_label
                                } else if can_send {
                                    if phone_mode {
                                        "开始操作"
                                    } else {
                                        "发送"
                                    }
                                } else if has_content {
                                    hint.as_str()
                                } else {
                                    "先输入内容"
                                },
                                if can_stop {
                                    UiIcon::Close
                                } else {
                                    UiIcon::ArrowUp
                                },
                                if can_stop {
                                    Some(if phone_mode {
                                        Action::AmooUi(
                                            super::amoo_ui_state::AmooUiAction::GuiStopWaiting,
                                        )
                                    } else {
                                        Action::Workspace(WorkspaceAction::Stop)
                                    })
                                } else {
                                    can_send.then_some(if phone_mode {
                                        Action::AmooSend
                                    } else {
                                        Action::Workspace(WorkspaceAction::Prepare)
                                    })
                                },
                                true,
                            ),
                            "composer-send",
                        )
                        .for_single_frame()
                        .finish(),
                    ],
                ),
                "composer-toolbar",
            )
            .for_single_frame()
            .finish(),
        );
        let surface_body = self.vstack(S2, rows);
        let surface = SavePosition::new(
            Hoverable::new(self.mouse_state("amoo-composer-surface"), move |mouse| {
                let t = self.hover_t(&mouse);
                let border_color = if focused {
                    p.border_strong
                } else {
                    lerp_color(p.border, p.border_strong, t)
                };
                Container::new(surface_body)
                    .with_uniform_padding(S3)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(border_color))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(16.)))
                    .finish()
            })
            .with_defer_events_to_children()
            .finish(),
            "composer-surface",
        )
        .for_single_frame()
        .finish();
        // Scenario chips live on the welcome screen only; a running conversation
        // keeps a single clean input.
        surface
    }
}

/// Toggle key (in `amoo_expanded_tool_records`) for the composer "+" menu.
const ATTACH_MENU_KEY: &str = "composer-attach-menu";

/// Plain words for a route the user picked from the intent menu.
fn route_label(intent: stepx_companion::amoo_workspace::Intent) -> &'static str {
    use stepx_companion::amoo_workspace::Intent;
    match intent {
        Intent::Files => "做文档",
        Intent::Image => "生图",
        Intent::App => "做应用",
        Intent::Screen => "背屏图案",
        Intent::Chat => "仅聊天",
        Intent::Auto => "自动",
    }
}
