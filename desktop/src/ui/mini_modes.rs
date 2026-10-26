//! A single compact window, sharing the real Amoo and phone notification state.
use super::{icons::UiIcon, theme::*, widgets::*, *};
use facloud_ui::{color::ColorU, fonts::Weight};
use pathfinder_geometry::rect::RectF;
use stepx_companion::push::ImChannel;

#[derive(Default)]
pub(super) struct CompactWindow {
    pub(super) restore: Option<(RectF, platform::FullscreenState, Page)>,
    pub history: bool,
    pub notification_detail: bool,
    pub system_notifications: bool,
    previous_screen_mode: bool,
}

pub(super) struct WindowMemory {
    pub preferences: stepx_companion::window_preferences::Preferences,
    path: Option<PathBuf>,
    saved: stepx_companion::window_preferences::Preferences,
    changed: Instant,
    settle_until: Instant,
}
impl WindowMemory {
    pub fn new() -> Self {
        let isolated = std::env::args().any(|a| a == "--capture-ui" || a == "--demo");
        let path = (!isolated)
            .then(|| stepx_companion::resources::user_config().join("stepx-companion/window.json"));
        #[cfg(feature = "ui-capture")]
        let path = if std::env::args().any(|a| a == "--verify-window-memory") && isolated {
            Some(
                PathBuf::from(
                    std::env::var_os("STEPX_WINDOW_TEST_DIR")
                        .expect("isolated window test directory"),
                )
                .join("window.json"),
            )
        } else {
            path
        };
        let preferences = path
            .as_ref()
            .map(|p| stepx_companion::window_preferences::Preferences::load(p))
            .unwrap_or_default();
        Self {
            saved: preferences.clone(),
            preferences,
            path,
            changed: Instant::now(),
            settle_until: Instant::now() + Duration::from_millis(900),
        }
    }
    pub fn startup_mode(&self) -> ShellMode {
        if std::env::args().any(|a| a == "--mini-notify") {
            return ShellMode::MiniNotify;
        }
        if std::env::args().any(|a| a == "--mini-ai" || a == "--compact") {
            return ShellMode::MiniAi;
        }
        if std::env::args().any(|a| a == "--page") {
            return ShellMode::Full;
        }
        match self.preferences.mode {
            stepx_companion::window_preferences::Mode::Full => ShellMode::Full,
            stepx_companion::window_preferences::Mode::MiniAi => ShellMode::MiniAi,
            stepx_companion::window_preferences::Mode::MiniNotify => ShellMode::MiniNotify,
        }
    }
    fn flush(&mut self) {
        if self.saved == self.preferences {
            return;
        }
        if let Some(path) = &self.path {
            if let Err(error) = self.preferences.save(path) {
                eprintln!("Window preferences: {error}");
                return;
            }
        }
        self.saved = self.preferences.clone();
    }
}
fn memory_mode(mode: ShellMode) -> stepx_companion::window_preferences::Mode {
    match mode {
        ShellMode::Full => stepx_companion::window_preferences::Mode::Full,
        ShellMode::MiniAi => stepx_companion::window_preferences::Mode::MiniAi,
        ShellMode::MiniNotify => stepx_companion::window_preferences::Mode::MiniNotify,
    }
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

/// Clamp to the current display, including displays with negative origins.
fn fitted_bounds(current: RectF, display: RectF, size: Vector2F) -> RectF {
    let size = vec2f(
        size.x().min(display.width()),
        size.y().min((display.height() - 48.).max(320.)),
    );
    let origin = vec2f(
        current
            .origin()
            .x()
            .clamp(display.origin().x(), display.max_x() - size.x()),
        current.origin().y().clamp(
            display.origin().y() + 24.,
            (display.max_y() - size.y()).max(display.origin().y() + 24.),
        ),
    );
    RectF::new(origin, size)
}

impl Companion {
    pub(super) fn set_compact_mode(&mut self, mode: ShellMode, ctx: &mut ViewContext<Self>) {
        if self.push.shell_mode == mode {
            return;
        }
        self.context_menu = None;
        self.clear_text_interaction();
        self.remember_window(ctx, true);
        let display = ctx.windows().active_display_bounds();
        let current = ctx
            .window_bounds(&ctx.window_id())
            .unwrap_or(RectF::new(display.origin(), vec2f(1320., 860.)));
        if mode == ShellMode::Full {
            self.push.shell_mode = ShellMode::Full;
            self.amoo_ui.screen_mode = self.compact.previous_screen_mode;
            if let Some((bounds, state, page)) = self.compact.restore.take() {
                self.page = page;
                if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                    match window.fullscreen_state() {
                        platform::FullscreenState::Maximized => window.toggle_maximized(),
                        platform::FullscreenState::Fullscreen => window.toggle_fullscreen(),
                        _ => {}
                    }
                }
                ctx.windows().set_window_bounds(
                    ctx.window_id(),
                    fitted_bounds(bounds, display, {
                        let size = self.window_memory.preferences.full;
                        vec2f(size.width, size.height)
                    }),
                );
                if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                    match state {
                        platform::FullscreenState::Maximized => window.toggle_maximized(),
                        platform::FullscreenState::Fullscreen => window.toggle_fullscreen(),
                        _ => {}
                    }
                }
            }
            self.set_compact_pin(false, ctx);
        } else {
            if self.push.shell_mode == ShellMode::Full {
                let state = ctx
                    .windows()
                    .platform_window(ctx.window_id())
                    .map(|w| w.fullscreen_state())
                    .unwrap_or_default();
                self.compact.restore = Some((current, state, self.page));
                self.compact.previous_screen_mode = self.amoo_ui.screen_mode;
                if let Some(window) = ctx.windows().platform_window(ctx.window_id()) {
                    match state {
                        platform::FullscreenState::Maximized => window.toggle_maximized(),
                        platform::FullscreenState::Fullscreen => window.toggle_fullscreen(),
                        _ => {}
                    }
                }
                ctx.windows().set_window_bounds(
                    ctx.window_id(),
                    fitted_bounds(current, display, {
                        let size = self.window_memory.preferences.compact;
                        vec2f(size.width, size.height)
                    }),
                );
            }
            self.amoo_ui.screen_mode = false;
            self.push.shell_mode = mode;
            self.page = if mode == ShellMode::MiniNotify {
                Page::Notifications
            } else {
                Page::Amoo
            };
            self.compact.history = false;
            if mode == ShellMode::MiniAi && self.push.clipboard_preview.trim().is_empty() {
                let raw_clip = ctx.clipboard().read().plain_text.trim().to_string();
                let effective = if !raw_clip.is_empty() {
                    raw_clip
                } else if !self
                    .back_screen
                    .stepos_suite
                    .phone_clipboard
                    .trim()
                    .is_empty()
                {
                    self.back_screen
                        .stepos_suite
                        .phone_clipboard
                        .trim()
                        .to_string()
                } else {
                    String::new()
                };
                if !effective.is_empty() {
                    self.push.clipboard_preview = Self::truncate_chars(&effective, 240);
                }
            }
            if self.push.pin_window {
                self.set_compact_pin(true, ctx);
            }
        }
        self.window_memory.preferences.mode = memory_mode(mode);
        self.window_memory.settle_until = Instant::now() + Duration::from_millis(650);
        self.window_memory.flush();
        self.back_screen.set_preview_visible(
            self.push.shell_mode == ShellMode::Full && self.page == Page::BackScreen,
        );
        self.focus = None;
        if matches!(self.page, Page::Amoo | Page::Work) {
            self.reveal_latest_chat();
        }
        if mode != ShellMode::Full {
            self.load_work_previews();
            // A layout change must not reload an already connected conversation or block typing.
            if mode == ShellMode::MiniNotify || self.amoo_sessions.is_empty() {
                self.auto_load_for_page();
            }
        }
        ctx.notify();
    }

    pub(super) fn remember_window(&mut self, ctx: &mut ViewContext<Self>, flush: bool) {
        if Instant::now() >= self.window_memory.settle_until && !self.compact_start_pending {
            let mut normal = ctx
                .windows()
                .platform_window(ctx.window_id())
                .is_some_and(|w| matches!(w.fullscreen_state(), platform::FullscreenState::Normal));
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            if let Some(w) = facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id())
            {
                normal &= !w.is_minimized().unwrap_or(false);
            }
            let size = if normal {
                ctx.window_bounds(&ctx.window_id()).map(|b| {
                    stepx_companion::window_preferences::Size {
                        width: b.width(),
                        height: b.height(),
                    }
                })
            } else {
                None
            };
            let old = self.window_memory.preferences.clone();
            self.window_memory
                .preferences
                .record(memory_mode(self.push.shell_mode), size);
            if old != self.window_memory.preferences {
                self.window_memory.changed = Instant::now();
            }
        }
        if flush || self.window_memory.changed.elapsed() >= Duration::from_millis(650) {
            self.window_memory.flush();
        }
    }

    pub(super) fn set_compact_pin(&mut self, pin: bool, ctx: &mut ViewContext<Self>) {
        self.push.pin_window = pin;
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(window) =
            facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id())
        {
            window.set_window_level(if pin {
                winit::window::WindowLevel::AlwaysOnTop
            } else {
                winit::window::WindowLevel::Normal
            });
        }
        #[cfg(target_os = "macos")]
        let _ = ctx;
    }

    fn compact_clip_pill(&self, id: &str, label: &str, action: Action) -> El {
        let p = self.p();
        let state = self.mouse_state(format!("clip-pill:{id}"));
        let label_str = label.to_string();
        let font = self.font;
        let control = Hoverable::new(state, move |mouse| {
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
                mix_color(p.surface, p.hover, t)
            };
            let border = if mouse.is_clicked() {
                p.accent
            } else {
                mix_color(p.border, p.accent, t)
            };
            let fg = if mouse.is_clicked() || hovered {
                p.accent
            } else {
                mix_color(p.text2, p.accent, t)
            };
            Container::new(
                ConstrainedBox::new(
                    Align::new(
                        Text::new(label_str.clone(), font, CAPTION)
                            .with_color(fg)
                            .with_style(
                                facloud_ui::fonts::Properties::default().weight(Weight::Medium),
                            )
                            .soft_wrap(false)
                            .finish(),
                    )
                    .finish(),
                )
                .with_height(24.)
                .finish(),
            )
            .with_horizontal_padding(S2 + 2.)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
            .finish()
        })
        .with_cursor(platform::Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        #[cfg(feature = "ui-capture")]
        let control = super::scenario_check::control(id.to_string(), control);
        control
    }

    fn compact_clipboard_row(&self) -> El {
        let p = self.p();
        let snippet = if self.push.clipboard_preview.trim().is_empty() {
            "剪贴板提问".to_string()
        } else {
            format!(
                "剪贴板 · {}",
                Self::truncate_chars(&self.push.clipboard_preview.replace('\n', " "), 12)
            )
        };
        let capture_state = self.mouse_state("clip-pill:compact-clip-capture");
        let capture_badge = Hoverable::new(capture_state, move |mouse| {
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
                mix_color(ColorU::transparent_black(), p.hover, t)
            };
            let fg = if mouse.is_clicked() || hovered {
                p.accent
            } else {
                mix_color(p.text3, p.accent, t)
            };
            Container::new(self.lead(
                4.,
                self.ui_icon(UiIcon::Clipboard, 13., fg),
                self.line_w(snippet.clone(), MICRO, fg, Weight::Medium),
            ))
            .with_horizontal_padding(6.)
            .with_vertical_padding(3.)
            .with_background_color(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_TAG)))
            .finish()
        })
        .with_cursor(platform::Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(Action::Push(PushAction::CaptureClipboardPreview))
        })
        .finish();
        #[cfg(feature = "ui-capture")]
        let capture_badge =
            super::scenario_check::control("compact-clip-capture".into(), capture_badge);

        let mut pills = vec![
            self.compact_clip_pill(
                "compact-clip-explain",
                "解释剪贴板",
                Action::Push(PushAction::AskClipboard(0)),
            ),
            self.compact_clip_pill(
                "compact-clip-translate",
                "翻译剪贴板",
                Action::Push(PushAction::AskClipboard(1)),
            ),
            self.compact_clip_pill(
                "compact-clip-summary",
                "提炼要点",
                Action::Push(PushAction::AskClipboard(2)),
            ),
        ];
        if self.workspace.viewport.x() >= 440. {
            pills.push(self.compact_clip_pill(
                "compact-clip-polish",
                "润色内容",
                Action::Push(PushAction::AskClipboard(3)),
            ));
        }
        self.between(capture_badge, self.hstack(6., pills))
    }

    fn compact_clipboard_bar(&self) -> Option<El> {
        let is_verify = std::env::args().any(|a| a.starts_with("--verify-"));
        if is_verify
            || self.push.shell_mode != ShellMode::MiniAi
            || self.compact.history
            || self.workspace.viewport.y() < 480.
        {
            return None;
        }
        Some(
            Container::new(self.compact_clipboard_row())
                .with_horizontal_padding(S3)
                .with_padding_bottom(S2)
                .finish(),
        )
    }

    pub(super) fn compact_shell(&self) -> El {
        let p = self.p();
        let mut controls = vec![];
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        controls.push(self.button_icon(
            "compact-pin",
            if self.push.pin_window {
                "已置顶"
            } else {
                "置顶"
            },
            UiIcon::Pin,
            Action::Push(PushAction::TogglePinWindow),
            if self.push.pin_window {
                Btn::Primary
            } else {
                Btn::Ghost
            },
        ));
        controls.push(self.button_icon(
            "compact-expand",
            "完整工作台",
            UiIcon::Maximize,
            Action::Push(PushAction::SetShellMode(ShellMode::Full)),
            Btn::Secondary,
        ));
        controls.push(self.window_control_btn("compact-min", "—", false, Action::MinimizeWindow));
        controls.push(self.window_control_btn("compact-close", "×", true, Action::CloseWindow));
        let header = Container::new(self.between(
            self.line_w("StepX Companion", SUBHEAD, p.text, Weight::Semibold),
            self.hstack(S1, controls),
        ))
        .with_horizontal_padding(S3)
        .with_vertical_padding(if self.workspace.viewport.y() < 520. {
            S2
        } else {
            S3
        })
        .finish();
        let mut tab_items = vec![
            (
                "AI 简洁窗".into(),
                self.push.shell_mode == ShellMode::MiniAi,
                Action::Push(PushAction::SetShellMode(ShellMode::MiniAi)),
            ),
            (
                "通知小窗".into(),
                self.push.shell_mode == ShellMode::MiniNotify,
                Action::Push(PushAction::SetShellMode(ShellMode::MiniNotify)),
            ),
        ];
        if self.workspace.viewport.x() >= 420. {
            tab_items.push((
                "完整工作台".into(),
                self.push.shell_mode == ShellMode::Full,
                Action::Push(PushAction::SetShellMode(ShellMode::Full)),
            ));
        }
        let tabs = self.segmented("compact-tabs", tab_items);
        let links = self.button_icon(
            "compact-workspace-history",
            "会话记录",
            UiIcon::Clock,
            Action::Workspace(WorkspaceAction::History),
            Btn::Ghost,
        );
        let nav = Container::new(self.between(tabs, links))
            .with_horizontal_padding(S3)
            .with_padding_bottom(S2)
            .finish();
        let body = if self.selected.is_none()
            && !self.demo_mode
            && self.push.shell_mode == ShellMode::MiniNotify
        {
            self.empty(
                "连接手机后开始使用",
                "Amoo 对话与通知直接同步手机数据。",
                Some(self.button(
                    "compact-connect",
                    "连接设备",
                    Action::Page(Page::Connect),
                    Btn::Primary,
                )),
            )
        } else if self.push.shell_mode == ShellMode::MiniNotify {
            self.compact_notifications()
        } else {
            self.workspace_page(true)
        };
        let footer = ConstrainedBox::new(
            Container::new(
                self.between(
                    self.lead(
                        S2,
                        self.fixed_w(
                            18.,
                            Align::new(if self.footer_loading() {
                                self.loading_line()
                            } else {
                                self.dot(if self.error {
                                    p.danger
                                } else if self.selected.is_some() {
                                    p.success
                                } else {
                                    p.text3
                                })
                            })
                            .finish(),
                        ),
                        self.line(
                            Self::truncate_chars(&self.footer_detail(), 72),
                            MICRO,
                            if self.error { p.danger } else { p.text3 },
                        ),
                    ),
                    self.desktop_update_link(),
                ),
            )
            .with_horizontal_padding(S3)
            .with_vertical_padding(5.)
            .with_border(Border::top(1.).with_border_color(p.border))
            .finish(),
        )
        .with_height(28.)
        .finish();
        let mut col = Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(header)
            .with_child(nav);
        if let Some(clip_bar) = self.compact_clipboard_bar() {
            col = col.with_child(clip_bar);
        }
        Container::new(
            col.with_child(self.divider())
                .with_child(Expanded::new(1., body).finish())
                .with_child(footer)
                .finish(),
        )
        .with_background_color(p.bg)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
        .finish()
    }

    fn compact_amoo(&self) -> El {
        if let Some(picker) = self.amoo_album_picker() {
            return self.pane_scroll(self.scroll_aux.clone(), picker);
        }
        let p = self.p();
        let title = self
            .amoo_sessions
            .get(self.amoo_session_idx)
            .map(|s| amoo_display::session_title(&s.title))
            .unwrap_or("新对话".into());
        let mut rows = vec![Container::new(self.between(
            self.text(title, BODY, p.text),
            self.hstack(
                S1,
                vec![
                    self.button(
                        "compact-history",
                        if self.compact.history {
                            "收起记录"
                        } else {
                            "会话记录"
                        },
                        Action::Push(PushAction::CompactHistory),
                        Btn::Ghost,
                    ),
                    self.button_icon(
                        "compact-new",
                        "新对话",
                        UiIcon::Plus,
                        Action::AmooNewChat,
                        Btn::Secondary,
                    ),
                ],
            ),
        ))
        .with_uniform_padding(S3)
        .finish()];
        let content = if self.compact.history {
            let mut sessions = vec![];
            for (i, s) in self.amoo_sessions.iter().enumerate() {
                sessions.push(self.list_row(
                    format!("compact-session-{i}"),
                    self.text(amoo_display::session_title(&s.title), BODY, p.text),
                    i == self.amoo_session_idx,
                    Some(Action::AmooSelectSession(i)),
                    None,
                ));
            }
            if sessions.is_empty() {
                sessions.push(self.text("暂无会话", CAPTION, p.text3));
            }
            self.vstack(S2, sessions)
        } else {
            self.amoo_conversation()
        };
        rows.push(
            Expanded::new(
                1.,
                if self.compact.history {
                    self.pane_scroll(
                        self.scroll_aux.clone(),
                        Container::new(content).with_uniform_padding(S3).finish(),
                    )
                } else {
                    self.chat_scroll(
                        Container::new(self.selectable_messages(content))
                            .with_uniform_padding(S3)
                            .finish(),
                    )
                },
            )
            .finish(),
        );
        let mut composer = vec![];
        let is_verify = std::env::args().any(|a| a.starts_with("--verify-"));
        if !is_verify && !self.compact.history && self.workspace.viewport.y() >= 480. {
            composer.push(self.compact_clipboard_row());
        }
        // Reuse the official composer, attachment uploads and pending state.
        composer.push(self.compact_amoo_composer());
        rows.push(
            Container::new(self.vstack(S2, composer))
                .with_uniform_padding(S3)
                .with_border(Border::top(1.).with_border_color(p.border))
                .finish(),
        );
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_children(rows)
            .finish()
    }

    fn compact_notifications(&self) -> El {
        let p = self.p();
        let target = self.push.selected_im_thread.as_ref().and_then(|id| {
            self.push
                .snapshot
                .threads
                .iter()
                .find(|t| &t.thread_id == id)
        });
        if self.compact.notification_detail {
            if let Some(thread) = target {
                return self.compact_notification_conversation(thread);
            }
        }
        let actions = self.between(
            self.vstack(
                S1,
                vec![
                    self.heading("通知收件箱"),
                    self.line("选择消息查看内容并回复", CAPTION, p.text3),
                ],
            ),
            self.hstack(
                S1,
                vec![
                    self.button_icon(
                        "compact-privacy",
                        if self.push.prefs.privacy_mask {
                            "显示正文"
                        } else {
                            "隐藏正文"
                        },
                        UiIcon::Lock,
                        Action::Push(PushAction::TogglePrivacyMask),
                        Btn::Ghost,
                    ),
                    if self.push.syncing {
                        self.disabled_button("同步中")
                    } else {
                        self.button_icon(
                            "compact-sync",
                            "刷新",
                            UiIcon::Refresh,
                            Action::Push(PushAction::Refresh),
                            Btn::Ghost,
                        )
                    },
                ],
            ),
        );
        let mut cards = vec![];
        if let Some(error) = &self.push.error {
            cards.push(self.text(error, CAPTION, p.danger));
        }
        let threads: Vec<_> = self
            .push
            .snapshot
            .threads
            .iter()
            .filter(|t| !t.last_message.trim().is_empty() || !t.messages.is_empty())
            .collect();
        if !threads.is_empty() {
            cards.push(self.line_w(
                format!("消息 · {}", threads.len()),
                CAPTION,
                p.text2,
                Weight::Medium,
            ));
        }
        for thread in threads {
            let title = if thread.title == thread.app_name {
                thread.title.clone()
            } else {
                format!("{} · {}", thread.app_name, thread.title)
            };
            let meta = self.between(
                self.line_w(title, BODY, p.text, Weight::Medium),
                self.line(pages::format_time(thread.last_timestamp_ms), MICRO, p.text3),
            );
            let preview = if self.push.prefs.privacy_mask {
                "消息正文已隐藏".into()
            } else {
                Self::truncate_chars(&thread.last_message, 66)
            };
            let label = if thread.unread_count > 0 {
                format!("{} 条未读", thread.unread_count)
            } else if self
                .push
                .im_drafts
                .get(&thread.thread_id)
                .is_some_and(|s| !s.is_empty())
                || (self.push.selected_im_thread.as_ref() == Some(&thread.thread_id)
                    && !self.fields[F_IM_REPLY].text.is_empty())
            {
                "有草稿".into()
            } else {
                "查看会话".into()
            };
            let content = self.between(
                self.lead(
                    S3,
                    self.ui_icon_box(
                        if thread.channel() == ImChannel::Sms {
                            UiIcon::Sms
                        } else {
                            UiIcon::Chat
                        },
                        36.,
                        p.surface,
                        p.text2,
                        None,
                    ),
                    Expanded::new(
                        1.,
                        self.vstack(
                            S1,
                            vec![
                                meta,
                                self.line(preview, CAPTION, p.text2),
                                self.line(label, MICRO, p.text3),
                            ],
                        ),
                    )
                    .finish(),
                ),
                self.ui_icon(UiIcon::ChevronRight, 16., p.text3),
            );
            // Cards, unlike bare list rows, make the entire clickable target visible.
            let state = self.mouse_state(format!("compact-notice-{}", thread.thread_id));
            let action = Action::Push(PushAction::SelectImThread(thread.thread_id.clone()));
            cards.push(
                Hoverable::new(state, move |mouse| {
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
                    let border = if mouse.is_clicked() {
                        p.border_strong
                    } else {
                        mix_color(p.border, p.border_strong, t)
                    };
                    Container::new(content)
                        .with_uniform_padding(S3)
                        .with_background_color(bg)
                        .with_border(Border::all(1.).with_border_color(border))
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_cursor(platform::Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish(),
            );
        }
        let notices: Vec<_> = self
            .push
            .snapshot
            .notifications
            .iter()
            .filter(|n| {
                !n.dismissed
                    && n.category != "im"
                    && !self
                        .push
                        .snapshot
                        .threads
                        .iter()
                        .any(|t| t.thread_id == n.thread_id && !t.messages.is_empty())
            })
            .collect();
        let ongoing = notices
            .iter()
            .filter(|n| {
                n.is_ongoing
                    || n.package_name == "android"
                    || n.package_name == "com.android.systemui"
            })
            .count();
        if notices.len() > ongoing {
            cards.push(self.line_w("其他通知", CAPTION, p.text2, Weight::Medium));
        }
        for notice in notices.iter().filter(|n| {
            !(n.is_ongoing
                || n.package_name == "android"
                || n.package_name == "com.android.systemui")
        }) {
            cards.push(self.compact_notice_card(notice));
        }
        if ongoing > 0 {
            cards.push(self.button_icon(
                "compact-system-notices",
                format!("系统与常驻通知 · {ongoing}"),
                if self.compact.system_notifications {
                    UiIcon::ChevronUp
                } else {
                    UiIcon::ChevronDown
                },
                Action::Push(PushAction::CompactSystemNotices),
                Btn::Ghost,
            ));
            if self.compact.system_notifications {
                for notice in notices.iter().filter(|n| {
                    n.is_ongoing
                        || n.package_name == "android"
                        || n.package_name == "com.android.systemui"
                }) {
                    cards.push(self.compact_notice_card(notice));
                }
            }
        }
        if cards.is_empty() {
            cards.push(self.empty(
                if self.push.syncing && !self.push.loaded {
                    "正在同步通知"
                } else {
                    "暂无通知"
                },
                if self.push.syncing && !self.push.loaded {
                    "正在读取手机通知，请稍候。"
                } else {
                    "收到手机消息后会自动显示在这里。"
                },
                None,
            ));
        }
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(Container::new(actions).with_uniform_padding(S4).finish())
            .with_child(
                Expanded::new(
                    1.,
                    self.pane_scroll(
                        self.push_list_scroll.clone(),
                        Container::new(self.vstack(S2, cards))
                            .with_horizontal_padding(S4)
                            .with_padding_bottom(S4)
                            .finish(),
                    ),
                )
                .finish(),
            )
            .finish()
    }

    fn compact_notice_card(&self, n: &stepx_companion::push::PushNotification) -> El {
        let p = self.p();
        let title = if n.app_name == n.title {
            n.title.clone()
        } else {
            format!("{} · {}", n.app_name, n.title)
        };
        let mut parts = vec![
            self.between(
                self.line_w(title, BODY, p.text, Weight::Medium),
                self.line(pages::format_time(n.post_time_ms), MICRO, p.text3),
            ),
            self.text(
                if self.push.prefs.privacy_mask {
                    "通知正文已隐藏".into()
                } else {
                    Self::truncate_chars(&n.text, 100)
                },
                CAPTION,
                p.text2,
            ),
        ];
        let mut buttons = vec![];
        if let Some(code) = &n.otp_code {
            buttons.push(self.button_icon(
                format!("compact-otp-{}", n.key),
                "复制验证码",
                UiIcon::Copy,
                Action::Push(PushAction::CopyOtp(code.clone())),
                Btn::Ghost,
            ));
        }
        buttons.push(self.button_icon(
            format!("compact-open-{}", n.key),
            "在手机查看",
            UiIcon::ExternalLink,
            Action::Push(PushAction::OpenOnPhone(n.key.clone())),
            Btn::Ghost,
        ));
        parts.push(self.wrap(buttons));
        Container::new(self.vstack(S2, parts))
            .with_uniform_padding(S3)
            .with_background_color(p.surface)
            .with_border(Border::all(1.).with_border_color(p.border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
    }

    fn compact_notification_conversation(&self, t: &stepx_companion::push::ImThread) -> El {
        let p = self.p();
        let header = self.between(
            self.lead(
                S2,
                self.button_icon(
                    "compact-inbox",
                    "通知",
                    UiIcon::ArrowLeft,
                    Action::Push(PushAction::CompactInbox),
                    Btn::Ghost,
                ),
                Expanded::new(
                    1.,
                    self.vstack(
                        S1,
                        vec![
                            self.line_w(&t.title, BODY, p.text, Weight::Medium),
                            self.line(&t.app_name, CAPTION, p.text3),
                        ],
                    ),
                )
                .finish(),
            ),
            self.button_icon(
                "compact-chat-phone",
                "手机查看",
                UiIcon::ExternalLink,
                Action::Push(PushAction::OpenOnPhone(t.notification_key.clone())),
                Btn::Ghost,
            ),
        );
        let mut messages = vec![self.line("来自手机通知的消息记录", MICRO, p.text3)];
        for m in &t.messages {
            let text = if self.push.prefs.privacy_mask && !m.outgoing {
                "消息正文已隐藏".into()
            } else {
                m.text.clone()
            };
            let (state, color) = match m.delivery_status.as_str() {
                "sending" => ("发送中", p.text3),
                "failed" => ("未发送 · 草稿已保留", p.danger),
                "unknown" => ("未确认 · 请核对手机", p.warning),
                _ if m.outgoing => (
                    if m.via == "remote_input" {
                        "已提交给手机应用"
                    } else {
                        "已发送"
                    },
                    p.text3,
                ),
                _ => ("", p.text3),
            };
            let meta = format!("{} {}", pages::format_time(m.timestamp_ms), state);
            let bubble = Container::new(self.text(
                text,
                BODY,
                if m.outgoing { p.on_accent } else { p.text },
            ))
            .with_uniform_padding(S3)
            .with_background_color(if m.outgoing { p.accent } else { p.surface })
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish();
            messages.push(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(if m.outgoing {
                        MainAxisAlignment::End
                    } else {
                        MainAxisAlignment::Start
                    })
                    .with_child(
                        ConstrainedBox::new(
                            Flex::column()
                                .with_spacing(S1)
                                .with_cross_axis_alignment(if m.outgoing {
                                    CrossAxisAlignment::End
                                } else {
                                    CrossAxisAlignment::Start
                                })
                                .with_child(bubble)
                                .with_child(self.line(meta, MICRO, color))
                                .finish(),
                        )
                        .with_max_width(440.)
                        .finish(),
                    )
                    .finish(),
            );
        }
        let can_reply = t.can_reply();
        let mut compose = vec![self.between(
            self.line(format!("回复给 {}", t.title), CAPTION, p.text2),
            self.line(
                if t.can_remote_reply {
                    "通知快捷回复"
                } else {
                    "手机界面发送"
                },
                MICRO,
                p.text3,
            ),
        )];
        if !t.can_remote_reply {
            compose.push(self.line("需解锁手机；发送前会核对目标会话。", MICRO, p.text3));
        }
        if let Some(status) = &self.push.last_reply_status {
            if status.starts_with("回复未完成") {
                compose.push(self.text(status, CAPTION, p.danger));
            }
        }
        compose.push(self.between(
            self.input(F_IM_REPLY, "输入回复内容…", None),
            if self.push.sending_thread.is_some()
                || !can_reply
                || self.fields[F_IM_REPLY].text.trim().is_empty()
            {
                self.disabled_button(if self.push.sending_thread.is_some() {
                    "发送中"
                } else {
                    "发送"
                })
            } else {
                self.button(
                    "compact-send",
                    "发送",
                    Action::Push(PushAction::SendImReply),
                    Btn::Primary,
                )
            },
        ));
        Flex::column()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
            .with_child(
                Container::new(header)
                    .with_uniform_padding(S3)
                    .with_border(Border::bottom(1.).with_border_color(p.border))
                    .finish(),
            )
            .with_child(
                Expanded::new(
                    1.,
                    self.pane_scroll(
                        self.sms_messages_scroll.clone(),
                        Container::new(self.vstack(S3, messages))
                            .with_uniform_padding(S4)
                            .finish(),
                    ),
                )
                .finish(),
            )
            .with_child(
                Container::new(self.vstack(S2, compose))
                    .with_uniform_padding(S4)
                    .with_border(Border::top(1.).with_border_color(p.border))
                    .finish(),
            )
            .finish()
    }
    pub(super) fn short_app_badge(package_name: &str, fallback_app_name: &str) -> String {
        match ImChannel::from_package(package_name) {
            ImChannel::WeChat => "微信".into(),
            ImChannel::WeCom => "企微".into(),
            ImChannel::QqTim => "QQ".into(),
            ImChannel::Feishu => "飞书".into(),
            ImChannel::DingTalk => "钉钉".into(),
            ImChannel::Sms => "短信".into(),
            ImChannel::SocialDm => {
                if !fallback_app_name.is_empty() {
                    fallback_app_name.to_string()
                } else {
                    "私信".into()
                }
            }
            ImChannel::GlobalIm | ImChannel::All => {
                if !fallback_app_name.is_empty() {
                    fallback_app_name.to_string()
                } else {
                    package_name
                        .rsplit('.')
                        .next()
                        .unwrap_or("应用")
                        .to_string()
                }
            }
        }
    }

    pub(super) fn im_package_colors(package_name: &str, p: &Palette) -> (ColorU, ColorU) {
        match ImChannel::from_package(package_name) {
            ImChannel::WeChat | ImChannel::Sms => (p.success_text, p.success_soft),
            ImChannel::Feishu | ImChannel::DingTalk | ImChannel::WeCom => (p.blue, p.accent_soft),
            ImChannel::QqTim | ImChannel::GlobalIm => (p.indigo, p.accent_soft),
            ImChannel::SocialDm => (p.pink, p.danger_soft),
            ImChannel::All => (p.text2, p.tag_bg),
        }
    }

    pub(super) fn format_rel_time(timestamp_ms: i64) -> String {
        if timestamp_ms <= 0 {
            return "刚刚".into();
        }
        let now_ms = chrono::Utc::now().timestamp_millis();
        let diff_sec = ((now_ms - timestamp_ms) / 1000).max(0);
        if diff_sec < 15 {
            "刚刚".into()
        } else if diff_sec < 60 {
            format!("{diff_sec} 秒前")
        } else if diff_sec < 3600 {
            format!("{} 分钟前", diff_sec / 60)
        } else if diff_sec < 86_400 {
            format!("{} 小时前", diff_sec / 3600)
        } else {
            format!("{} 天前", diff_sec / 86_400)
        }
    }

    pub(super) fn truncate_chars(text: &str, max_chars: usize) -> String {
        let mut chars = text.chars();
        let head: String = chars.by_ref().take(max_chars).collect();
        if chars.next().is_some() {
            format!("{head}…")
        } else {
            head
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_bounds_fit_small_and_offset_displays() {
        let display = RectF::new(vec2f(-1280., 0.), vec2f(1280., 720.));
        let bounds = fitted_bounds(
            RectF::new(vec2f(800., 700.), vec2f(1320., 860.)),
            display,
            vec2f(600., 760.),
        );
        assert_eq!(bounds.size(), vec2f(600., 672.));
        assert!(bounds.origin().x() >= display.origin().x());
        assert!(bounds.max_x() <= display.max_x());
        assert!(bounds.max_y() <= display.max_y());
    }
    #[test]
    fn restore_preserves_normal_window_geometry() {
        let display = RectF::new(vec2f(0., 0.), vec2f(1920., 1080.));
        let before = RectF::new(vec2f(120., 80.), vec2f(1320., 860.));
        assert_eq!(fitted_bounds(before, display, before.size()), before);
    }
}
