//! Session-scoped controls stay accessible while the scrcpy window has focus.
use super::widgets::El;
use super::*;
use facloud_ui::{TypedActionView, WeakViewHandle, WindowId};

struct MirrorControls {
    owner: WeakViewHandle<Companion>,
    actions: mpsc::Sender<Action>,
}

impl Entity for MirrorControls {
    type Event = ();
}
impl TypedActionView for MirrorControls {
    type Action = Action;
    fn handle_action(&mut self, action: &Action, ctx: &mut ViewContext<Self>) {
        if matches!(
            action,
            Action::StopMirror | Action::Key(_) | Action::Screenshot | Action::ScreenGesture(_)
        ) {
            #[cfg(feature = "ui-capture")]
            eprintln!("MIRROR_CONTROL_RECEIVED: {:?}", action);
            let _ = self.actions.send(action.clone());
            ctx.notify();
        }
    }
}
impl View for MirrorControls {
    fn ui_name() -> &'static str {
        "MirrorControls"
    }
    fn on_window_closed(&mut self, _: &mut ViewContext<Self>) {
        // Closing the controller means end this session; never leave an orphan.
        let _ = self.actions.send(Action::StopMirror);
    }
    fn render(&self, app: &AppContext) -> El {
        self.owner
            .upgrade(app)
            .map(|owner| owner.read(app, |view, _| view.mirror_control_panel()))
            .unwrap_or_else(|| Rect::new().finish())
    }
}

struct ControlsRoot {
    child: ViewHandle<MirrorControls>,
}
impl Entity for ControlsRoot {
    type Event = ();
}
impl TypedActionView for ControlsRoot {
    type Action = ();
}
impl View for ControlsRoot {
    fn ui_name() -> &'static str {
        "MirrorControlsRoot"
    }
    fn render(&self, _: &AppContext) -> El {
        ChildView::new(&self.child).finish()
    }
}

pub(super) struct ControlWindow {
    id: WindowId,
    view: WeakViewHandle<MirrorControls>,
    actions: mpsc::Receiver<Action>,
    bounds: [f32; 4],
    visible: bool,
    busy: bool,
    dark: bool,
}

impl Companion {
    fn mirror_tool(
        &self,
        id: &'static str,
        label: &'static str,
        icon: icons::UiIcon,
        action: Action,
        danger: bool,
    ) -> El {
        let p = self.p();
        let enabled = danger || (!self.busy && self.mirror.is_some());
        let font = self.font;
        Hoverable::new(
            self.mouse_state(format!("mirror-tool:{id}")),
            move |mouse| {
                let hovered = enabled && mouse.is_mouse_over_element();
                let fg = if !enabled {
                    p.text3
                } else if danger {
                    p.danger
                } else {
                    p.text2
                };
                let content = Flex::column()
                    .with_spacing(2.)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(icons::draw_icon(icon, 16., fg))
                    .with_child(Text::new(label, font, 10.).with_color(fg).finish())
                    .finish();
                ConstrainedBox::new(
                    Container::new(Align::new(content).finish())
                        .with_background_color(if hovered {
                            if danger {
                                p.danger_soft
                            } else {
                                p.hover
                            }
                        } else {
                            p.sidebar
                        })
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                            theme::R_CONTROL,
                        )))
                        .finish(),
                )
                .with_width(72.)
                .with_height(38.)
                .finish()
            },
        )
        .with_cursor(if enabled {
            platform::Cursor::PointingHand
        } else {
            platform::Cursor::Arrow
        })
        .on_click(move |ctx, _, _| {
            if enabled {
                ctx.dispatch_typed_action(action.clone());
            }
        })
        .finish()
    }

    fn mirror_control_panel(&self) -> El {
        let p = self.p();
        use icons::UiIcon::*;
        Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Center)
                .with_spacing(2.)
                .with_child(
                    Container::new(self.line("投屏助手", 10., p.text2))
                        .with_vertical_padding(4.)
                        .finish(),
                )
                .with_child(self.fixed_w(56., self.divider()))
                .with_child(self.mirror_tool("power", "锁屏", Lock, Action::Key(26), false))
                .with_child(self.mirror_tool("home", "主页", Home, Action::Key(3), false))
                .with_child(self.mirror_tool("back", "返回", ArrowLeft, Action::Key(4), false))
                .with_child(self.mirror_tool("recent", "多任务", Menu, Action::Key(187), false))
                .with_child(self.mirror_tool(
                    "swipe-up",
                    "上滑",
                    ArrowUp,
                    Action::ScreenGesture(0),
                    false,
                ))
                .with_child(self.mirror_tool(
                    "notif",
                    "通知",
                    Bell,
                    Action::ScreenGesture(4),
                    false,
                ))
                .with_child(self.fixed_w(40., self.divider()))
                .with_child(self.mirror_tool("volume-up", "音量 +", Plus, Action::Key(24), false))
                .with_child(self.mirror_tool(
                    "volume-down",
                    "音量 −",
                    Minus,
                    Action::Key(25),
                    false,
                ))
                .with_child(self.mirror_tool("mute", "静音", Mute, Action::Key(164), false))
                .with_child(self.mirror_tool("capture", "截图", Camera, Action::Screenshot, false))
                .with_child(self.spacer())
                .with_child(self.fixed_w(56., self.divider()))
                .with_child(self.mirror_tool("stop", "结束投屏", Close, Action::StopMirror, true))
                .finish(),
        )
        .with_uniform_padding(6.)
        .with_background_color(p.sidebar)
        .with_border(Border::all(1.).with_border_color(p.border))
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(theme::R_PANEL)))
        .finish()
    }

    pub(super) fn close_mirror_controls(&mut self, ctx: &mut ViewContext<Self>) {
        if let Some(window) = self.mirror_controls.take() {
            // Dropping this receiver prevents an old close event stopping a new session.
            ctx.windows()
                .close_window(window.id, platform::TerminationMode::Cancellable);
        }
    }

    pub(super) fn sync_mirror_controls(&mut self, ctx: &mut ViewContext<Self>) {
        let actions: Vec<_> = self
            .mirror_controls
            .as_ref()
            .map(|window| window.actions.try_iter().collect())
            .unwrap_or_default();
        for action in actions {
            self.handle_action(&action, ctx);
        }
        if self.mirror.is_none() {
            self.close_mirror_controls(ctx);
            return;
        }
        let frame = self
            .mirror
            .as_ref()
            .and_then(|session| session.window_bounds());
        let rect = |b: [f32; 4]| {
            pathfinder_geometry::rect::RectF::new(vec2f(b[0], b[1]), vec2f(b[2], b[3]))
        };
        let dark = self.theme_mode.is_dark(ctx.system_theme());
        if let Some(window) = &mut self.mirror_controls {
            if let Some(view) = window.view.upgrade(ctx) {
                if window.busy != self.busy || window.dark != dark {
                    view.update(ctx, |_, ctx| ctx.notify());
                    window.busy = self.busy;
                    window.dark = dark;
                }
            } else {
                self.stop_mirror();
                self.mirror_controls = None;
                return;
            }
            if let Some(frame) = frame {
                let scale = ctx
                    .windows()
                    .platform_window(window.id)
                    .map(|w| w.as_ctx().backing_scale_factor())
                    .unwrap_or(1.);
                let bounds = frame.sidebar(scale);
                if window.bounds != bounds {
                    ctx.windows().set_window_bounds(window.id, rect(bounds));
                    window.bounds = bounds;
                }
                if window.visible != frame.visible {
                    if frame.visible {
                        ctx.windows().show_window_and_focus_app(window.id);
                    } else {
                        ctx.windows().hide_window(window.id);
                    }
                    window.visible = frame.visible;
                }
            }
            return;
        }
        // Create the sidebar only once the owned projection window can be located.
        // It must never appear as an unrelated floating palette in the workspace.
        let Some(frame) = frame.filter(|frame| frame.visible) else {
            return;
        };
        let scale = ctx
            .windows()
            .platform_window(ctx.window_id())
            .map(|w| w.as_ctx().backing_scale_factor())
            .unwrap_or(1.);
        let bounds = frame.sidebar(scale);
        let owner = ctx.handle();
        let (actions, receiver) = mpsc::channel();
        let (id, view) = ctx.add_window(
            facloud_ui::AddWindowOptions {
                title: Some("StepX Companion · 投屏控制".into()),
                window_style: platform::WindowStyle::Pin,
                window_bounds: WindowBounds::ExactPosition(rect(bounds)),
                ..Default::default()
            },
            move |ctx| ControlsRoot {
                child: ctx.add_typed_action_view(move |_| MirrorControls { owner, actions }),
            },
        );
        let controls = view.read(ctx, |root, _| root.child.downgrade());
        self.mirror_controls = Some(ControlWindow {
            id,
            view: controls,
            actions: receiver,
            bounds,
            visible: true,
            busy: self.busy,
            dark,
        });
    }

    /// Opt-in real-device smoke test. Normal releases do not contain this driver.
    #[cfg(feature = "ui-capture")]
    pub(super) fn verify_mirror_controls(&mut self, ctx: &mut ViewContext<Self>) {
        use std::sync::atomic::{AtomicUsize, Ordering};
        static STEP: AtomicUsize = AtomicUsize::new(0);
        static TICKS: AtomicUsize = AtomicUsize::new(0);
        if !std::env::args().any(|arg| arg == "--verify-mirror-controls") {
            return;
        }
        static LAST_TICK: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
        static MAX_GAP: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        static CYCLES: AtomicUsize = AtomicUsize::new(0);
        if let Ok(mut last) = LAST_TICK.lock() {
            if let Some(previous) = last.replace(Instant::now()) {
                MAX_GAP.fetch_max(previous.elapsed().as_millis() as u64, Ordering::Relaxed);
            }
        }
        let Ok(expected) = std::env::var("STEPX_VERIFY_DEVICE") else {
            return;
        };
        if !self
            .selected
            .as_ref()
            .is_some_and(|d| d.serial == expected && d.product == "pandora")
            || self.busy
            || self.initializing
        {
            return;
        }
        let step = STEP.load(Ordering::SeqCst);
        if TICKS.fetch_add(1, Ordering::SeqCst) % 15 != 0 {
            return;
        }
        if step == 0 {
            if self.mirror_controls.is_some() {
                STEP.store(1, Ordering::SeqCst);
            } else if self.mirror.is_none() {
                self.handle_action(&Action::Mirror, ctx);
            }
        } else if std::env::var_os("STEPX_VERIFY_MANUAL").is_some() {
            static CAPTURED: std::sync::atomic::AtomicBool =
                std::sync::atomic::AtomicBool::new(false);
            if !CAPTURED.swap(true, Ordering::SeqCst) {
                if let Some(window) = self
                    .mirror_controls
                    .as_ref()
                    .and_then(|w| ctx.windows().platform_window(w.id))
                {
                    window.as_ctx().request_frame_capture(Box::new(|frame| {
                        let _ = image::save_buffer(
                            "/tmp/stepx-sidebar-manual.png",
                            &frame.data,
                            frame.width,
                            frame.height,
                            image::ColorType::Rgba8,
                        );
                    }));
                }
            }
            if let Ok(path) = std::env::var("STEPX_VERIFY_CLICK_FILE") {
                if let Ok(value) = std::fs::read_to_string(&path) {
                    let _ = std::fs::remove_file(&path);
                    if let (Ok(y), Some(window)) = (
                        value.trim().parse::<f32>(),
                        self.mirror_controls
                            .as_ref()
                            .and_then(|w| ctx.windows().platform_window(w.id)),
                    ) {
                        let position = vec2f(
                            46.,
                            if y < 0. {
                                window.as_ctx().size().y() + y
                            } else {
                                y
                            },
                        );
                        for event in [
                            Event::MouseMoved {
                                position,
                                cmd: false,
                                shift: false,
                                is_synthetic: false,
                            },
                            Event::LeftMouseDown {
                                position,
                                modifiers: Default::default(),
                                click_count: 1,
                                is_first_mouse: false,
                            },
                            Event::LeftMouseUp {
                                position,
                                modifiers: Default::default(),
                            },
                        ] {
                            (window.callbacks().event_callback)(event, ctx);
                        }
                    }
                }
            }
            return;
        } else if step <= 8 {
            let Some(window) = &self.mirror_controls else {
                return;
            };
            let action = match step {
                1 => Action::Key(3),
                2 => Action::Key(187),
                3 => Action::Key(4),
                4 => Action::Key(25),
                5 => Action::Key(24),
                6 => Action::Screenshot,
                7 => {
                    if let Some(platform) = ctx.windows().platform_window(window.id) {
                        platform.as_ctx().request_frame_capture(Box::new(|frame| {
                            let _ = image::save_buffer(
                                "/tmp/stepx-mirror-controls-live.png",
                                &frame.data,
                                frame.width,
                                frame.height,
                                image::ColorType::Rgba8,
                            );
                        }));
                    }
                    STEP.store(8, Ordering::SeqCst);
                    return;
                }
                _ => Action::StopMirror,
            };
            eprintln!("MIRROR_UI_CHECK: action {step}");
            ctx.dispatch_typed_action_for_view(window.id, window.view.id(), &action);
            STEP.store(step + 1, Ordering::SeqCst);
        } else if step == 9 {
            if self.mirror_controls.is_some() {
                STEP.store(10, Ordering::SeqCst);
            } else if self.mirror.is_none() {
                self.handle_action(&Action::Mirror, ctx);
            }
        } else if step == 10 {
            if let Some(window) = &self.mirror_controls {
                ctx.windows()
                    .close_window(window.id, platform::TerminationMode::Cancellable);
                STEP.store(11, Ordering::SeqCst);
            }
        } else if step == 11 && self.mirror.is_none() && self.mirror_controls.is_none() {
            eprintln!("MIRROR_UI_CHECK: PASS — toolbar actions and native close both end the mirror session");
            eprintln!(
                "MIRROR_UI_CHECK: maximum UI tick gap {} ms",
                MAX_GAP.load(Ordering::Relaxed)
            );
            if CYCLES.fetch_add(1, Ordering::SeqCst) < 2 {
                STEP.store(0, Ordering::SeqCst);
            } else {
                std::process::exit(0);
            }
        }
    }
}
