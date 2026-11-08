//! Top-right Notification Bell, Floating System Toast Popup & Concise Graphical Notification Center Drawer.
//!
//! The drawer keeps messages in the foreground: source pictures, quiet metadata,
//! a single filter strip, and fixed controls. Reminder preferences live in Settings.

use super::super::{
    auto_sync::{AutoSyncAction, AutoSyncDomain, AutoSyncDomainStatus},
    icons::{self, UiIcon},
    notification_center_state::{
        NotificationAlertMode, NotificationCategory, NotificationCenterAction,
        NotificationPopupWidget, NotificationPrimaryAction, NotificationVisualKind,
        UnifiedNotificationItem,
    },
    theme::*,
    widgets::*,
    Action, Companion,
};
use facloud_ui::{
    assets::asset_cache::AssetSource,
    color::ColorU,
    elements::*,
    event::{DispatchedEvent, Event},
    fonts::{Properties, Weight},
    platform::Cursor,
};
use pathfinder_geometry::{
    rect::RectF,
    vector::{vec2f, Vector2F},
};
use std::{cell::Cell, rc::Rc};

/// Standard width of the compact banner popup notification card (`PopupMode::Banner`).
pub(crate) const POPUP_CARD_W: f32 = 380.;
/// Standard width of the full interactive reply popup card (`PopupMode::FullReply`).
pub(crate) const POPUP_CARD_W_FULL: f32 = 380.;

fn popup_display_title(item: &UnifiedNotificationItem, full: bool) -> String {
    let title = if item.title.trim().is_empty() { &item.sender } else { &item.title };
    if let NotificationPopupWidget::OtpCode { code, .. } = &item.popup_widget {
        // Codes belong in the copyable content area, never the banner title.
        if !full || (!code.is_empty() && title.contains(code)) {
            return "验证码".into();
        }
    }
    super::super::notification_center_state::truncate_concise(title, 24)
}

fn with_control_alias(el: El, alias_id: &str) -> El {
    let el = SavePosition::new(el, alias_id).for_single_frame().finish();
    #[cfg(feature = "ui-capture")]
    let el = super::super::scenario_check::control(alias_id.to_string(), el);
    el
}

/// Measure previews at their natural height, even when the previous notification
/// made the native window shorter. Only the allocated viewport is painted.
struct PopupPreview {
    child: El,
    size: Option<Vector2F>,
}

impl facloud_ui::Element for PopupPreview {
    fn layout(
        &mut self,
        c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        let natural = self.child.layout(
            SizeConstraint {
                min: vec2f(c.min.x(), 0.),
                max: vec2f(c.max.x(), 64.),
            },
            ctx,
            app,
        );
        let size = c.apply(natural);
        self.size = Some(size);
        size
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.child.after_layout(ctx, app);
    }
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        self.child.paint(origin, ctx, app);
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        self.child.dispatch_event(e, ctx, app)
    }
    fn size(&self) -> Option<Vector2F> {
        self.size
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
}

/// Track the whole card before nested buttons consume their hover-out event.
/// Clicks still belong to the child, so menu/close never open message details.
struct PopupPointerGuard {
    child: El,
    bounds: Option<RectF>,
}

impl facloud_ui::Element for PopupPointerGuard {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &facloud_ui::AppContext) -> Vector2F {
        self.child.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.child.after_layout(ctx, app);
    }
    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        self.bounds = self.child.size().map(|size| RectF::new(origin, size));
        self.child.paint(origin, ctx, app);
    }
    fn dispatch_event(&mut self, e: &DispatchedEvent, ctx: &mut EventContext, app: &facloud_ui::AppContext) -> bool {
        if let Event::MouseMoved { position, .. } = e.raw_event() {
            let hovered = self.bounds.is_some_and(|bounds| bounds.contains_point(*position));
            ctx.dispatch_typed_action(Action::NotificationCenter(NotificationCenterAction::PopupHoverChanged(hovered)));
        }
        self.child.dispatch_event(e, ctx, app)
    }
    fn size(&self) -> Option<Vector2F> { self.child.size() }
    fn origin(&self) -> Option<Point> { self.child.origin() }
}

impl Companion {
    /// Semantic `(UiIcon, icon_fg, avatar_bg)` for each notification visual kind.
    pub(crate) fn notification_visual_style(
        &self,
        visual: NotificationVisualKind,
    ) -> (UiIcon, ColorU, ColorU) {
        let p = self.p();
        match visual {
            NotificationVisualKind::WeChat
            | NotificationVisualKind::Feishu
            | NotificationVisualKind::DingTalk => {
                let channel = match visual {
                    NotificationVisualKind::WeChat => stepx_companion::push::ImChannel::WeChat,
                    NotificationVisualKind::Feishu => stepx_companion::push::ImChannel::Feishu,
                    _ => stepx_companion::push::ImChannel::DingTalk,
                };
                let style = super::super::icon_style::channel_source_style(channel, &p);
                (style.icon, style.foreground, style.background)
            }
            NotificationVisualKind::ImChat => {
                let style = super::super::icon_style::neutral_source_style(UiIcon::Chat, &p);
                (style.icon, style.foreground, style.background)
            }
            NotificationVisualKind::IncomingCall => (UiIcon::Phone, p.success_text, p.success_soft),
            NotificationVisualKind::MissedCall => (UiIcon::PhoneOff, p.danger, p.danger_soft),
            NotificationVisualKind::SmsOtp => (UiIcon::Key, p.warning, p.warning_soft),
            NotificationVisualKind::Express => (
                UiIcon::Package,
                rgb(0xF9, 0x73, 0x16),
                if p.dark {
                    rgba(0xF9, 0x73, 0x16, 40)
                } else {
                    rgb(0xFF, 0xF7, 0xED)
                },
            ),
            NotificationVisualKind::CalendarAlarm => (UiIcon::Clock, p.teal, p.teal_soft),
            NotificationVisualKind::AppUpdate => (
                UiIcon::Download,
                p.blue,
                if p.dark {
                    rgba(0x3B, 0x82, 0xF6, 40)
                } else {
                    rgb(0xEF, 0xF6, 0xFF)
                },
            ),
            NotificationVisualKind::TaskReady => (UiIcon::Sparkles, p.indigo, p.indigo_soft),
            NotificationVisualKind::SafetyApproval => (UiIcon::Shield, p.warning, p.warning_soft),
            NotificationVisualKind::DeviceLink => {
                (UiIcon::Smartphone, p.success_text, p.success_soft)
            }
            NotificationVisualKind::BatteryWarn => (UiIcon::Battery, p.danger, p.danger_soft),
            NotificationVisualKind::CompanionInfo => (
                UiIcon::Monitor,
                p.blue,
                if p.dark {
                    rgba(0x3B, 0x82, 0xF6, 36)
                } else {
                    rgb(0xEF, 0xF6, 0xFF)
                },
            ),
            NotificationVisualKind::SystemAlert => (UiIcon::AlertTriangle, p.danger, p.danger_soft),
        }
    }

    fn notification_avatar(&self, visual: NotificationVisualKind, unread: bool, size: f32) -> El {
        let p = self.p();
        let (icon, fg, bg) = self.notification_visual_style(visual);
        let icon_sz = (size * 0.52).round().max(15.);
        let radius = (size * 0.28).round().max(8.);
        let base = ConstrainedBox::new(
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_main_axis_alignment(MainAxisAlignment::Center)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_child(self.content_icon(icon, icon_sz, fg))
                    .finish(),
            )
            .with_background_color(bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(radius)))
            .finish(),
        )
        .with_width(size)
        .with_height(size)
        .finish();

        if !unread {
            return base;
        }

        let red_dot = ConstrainedBox::new(
            Container::new(Empty::new().finish())
                .with_background_color(p.danger)
                .with_border(Border::all(1.5).with_border_color(p.bg))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
                .finish(),
        )
        .with_width(9.)
        .with_height(9.)
        .finish();

        Stack::new()
            .with_child(base)
            .with_positioned_child(
                red_dot,
                OffsetPositioning::offset_from_parent(
                    vec2f(2., -2.),
                    ParentOffsetBounds::Unbounded,
                    ParentAnchor::TopRight,
                    ChildAnchor::TopRight,
                ),
            )
            .finish()
    }

    /// Compact pill action button used in notification cards and floating popup toasts.
    fn notif_action_pill(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        primary: bool,
        action: Action,
    ) -> El {
        let id = id.into();
        let label = label.into();
        let p = self.p();
        let font = self.font;
        let state = self.mouse_state(format!("notif-pill:{id}"));
        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let (bg, border, fg, weight) = if primary {
                let base_bg = p.accent;
                let hov_bg = p.accent_hover;
                let cur_bg = if clicked {
                    lerp_color(base_bg, p.text2, 0.25)
                } else {
                    lerp_color(base_bg, hov_bg, t)
                };
                (cur_bg, cur_bg, p.on_accent, Weight::Semibold)
            } else {
                let cur_bg = if clicked {
                    p.accent_soft
                } else {
                    lerp_color(p.surface, p.hover, t)
                };
                let cur_border = if clicked {
                    p.border_strong
                } else {
                    lerp_color(p.border, p.border_strong, t)
                };
                let cur_fg = lerp_color(p.text2, p.text, t);
                (cur_bg, cur_border, cur_fg, Weight::Medium)
            };
            Container::new(
                Text::new(label.clone(), font, CAPTION)
                    .with_color(fg)
                    .with_style(Properties::default().weight(weight))
                    .soft_wrap(false)
                    .finish(),
            )
            .with_horizontal_padding(11.)
            .with_vertical_padding(4.5)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_TAG)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let control = SavePosition::new(control, &id).for_single_frame().finish();
        #[cfg(feature = "ui-capture")]
        let control = super::super::scenario_check::control(id, control);
        control
    }

    /// Compact header pill button (`horizontal_padding: 6.`, `vertical_padding: 2.`) for popup Zone 1 controls.
    /// Styled as a quiet ghost pill that shows a soft background on hover so it doesn't clutter the header.
    #[allow(dead_code)]
    fn notif_header_pill(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        action: Action,
    ) -> El {
        let id = id.into();
        let label = label.into();
        let p = self.p();
        let font = self.font;
        let state = self.mouse_state(format!("notif-hdr-pill:{id}"));
        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let cur_bg = if clicked {
                p.accent_soft
            } else {
                lerp_color(ColorU::transparent_black(), p.hover, t)
            };
            let cur_border = if clicked {
                p.border_strong
            } else {
                lerp_color(ColorU::transparent_black(), p.border, t * 0.85)
            };
            let cur_fg = lerp_color(p.text3, p.text, t);
            Container::new(
                Text::new(label.clone(), font, MICRO)
                    .with_color(cur_fg)
                    .with_style(Properties::default().weight(Weight::Medium))
                    .soft_wrap(false)
                    .finish(),
            )
            .with_horizontal_padding(6.)
            .with_vertical_padding(2.)
            .with_background_color(cur_bg)
            .with_border(Border::all(1.).with_border_color(cur_border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_TAG)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let control = SavePosition::new(control, &id).for_single_frame().finish();
        #[cfg(feature = "ui-capture")]
        let control = super::super::scenario_check::control(id, control);
        control
    }

    /// Subtle interactive hint chip inside Zone 3 scene cards so the scene card is clickable
    /// without rendering a second heavy button right above the Zone 4 primary button.
    #[allow(dead_code)]
    fn notif_widget_hint_tag(
        &self,
        id: impl Into<String>,
        label: impl Into<String>,
        fg: ColorU,
        bg: ColorU,
        action: Action,
    ) -> El {
        let id = id.into();
        let label = label.into();
        let p = self.p();
        let font = self.font;
        let state = self.mouse_state(format!("notif-wtag:{id}"));
        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let cur_bg = if clicked {
                p.accent_soft
            } else {
                lerp_color(bg, p.hover, t * 0.45)
            };
            Container::new(
                Text::new(label.clone(), font, MICRO)
                    .with_color(fg)
                    .with_style(Properties::default().weight(Weight::Semibold))
                    .soft_wrap(false)
                    .finish(),
            )
            .with_horizontal_padding(7.)
            .with_vertical_padding(2.)
            .with_background_color(cur_bg)
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_TAG)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let control = SavePosition::new(control, &id).for_single_frame().finish();
        #[cfg(feature = "ui-capture")]
        let control = super::super::scenario_check::control(id, control);
        control
    }

    /// Subtle icon-only dismiss / close button (`×`).
    fn notif_close_btn(&self, id: impl Into<String>, action: Action) -> El {
        let id = id.into();
        let p = self.p();
        let state = self.mouse_state(format!("notif-close:{id}"));
        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if clicked {
                p.accent_soft
            } else {
                lerp_color(ColorU::transparent_black(), p.hover, t)
            };
            let fg = if clicked {
                p.text
            } else {
                lerp_color(p.text3, p.text, t)
            };
            ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(icons::draw_icon(UiIcon::Close, 12., fg))
                        .finish(),
                )
                .with_background_color(bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                .finish(),
            )
            .with_width(22.)
            .with_height(22.)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let control = SavePosition::new(control, &id).for_single_frame().finish();
        #[cfg(feature = "ui-capture")]
        let control = super::super::scenario_check::control(id, control);
        control
    }

    /// Top-right global header Live Auto-Sync Status & Progress Pill (`id = "top-bar-auto-sync-pill"`).
    ///
    /// - When Syncing (`is_syncing() == true`): primary/accent-tinted `UiIcon::Refresh`, concise stage
    ///   label (`全量同步 4/7 · 57%`), and an inline `48px × 5px` graphical progress bar.
    /// - When All 7 Domains Synced (`overall_percent() == 100`): crisp green `8×8` dot (`p.success_text`)
    ///   + concise label (`双端实时同步 · 100%`).
    /// - When Disconnected (`self.selected.is_none()`): muted dot + `未连接手机`.
    /// - Clicking toggles the 7-Domain Sync Progress Popover (`Action::AutoSync(AutoSyncAction::ToggleDetails)`).
    #[allow(dead_code)]
    pub(crate) fn render_top_bar_auto_sync_pill(&self) -> El {
        let p = self.p();
        let font = self.font;
        let connected = self.selected.is_some();
        let syncing = self.auto_sync.is_syncing();
        let pct = self.auto_sync.overall_percent();
        let enabled = self.auto_sync.enabled;
        let details_open = self.auto_sync.details_open;
        let label = self.auto_sync.top_bar_label(connected);
        let id = "top-bar-auto-sync-pill".to_string();
        let state = self.mouse_state("top-bar-auto-sync-pill");

        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if details_open {
                if clicked {
                    p.press_color(p.hover)
                } else {
                    lerp_color(p.accent_soft, p.hover, t * 0.5)
                }
            } else if clicked {
                p.press_color(p.hover)
            } else if connected && syncing {
                lerp_color(p.accent_soft, p.hover, t * 0.45)
            } else {
                lerp_color(p.bg, p.hover, t)
            };
            let border = if details_open || (connected && syncing) {
                p.accent
            } else if connected && enabled && pct >= 100 {
                lerp_color(p.border, p.success_text, 0.35)
            } else {
                lerp_color(p.border, p.border_strong, t)
            };

            let inner = if !connected {
                let dot = ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(p.text3)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish(),
                )
                .with_width(7.)
                .with_height(7.)
                .finish();
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(5.)
                    .with_child(dot)
                    .with_child(
                        Text::new(label.clone(), font, MICRO)
                            .with_color(p.text2)
                            .with_style(Properties::default().weight(Weight::Medium))
                            .soft_wrap(false)
                            .finish(),
                    )
                    .finish()
            } else if !enabled {
                let dot = ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(p.warning)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish(),
                )
                .with_width(7.)
                .with_height(7.)
                .finish();
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(5.)
                    .with_child(dot)
                    .with_child(
                        Text::new(label.clone(), font, MICRO)
                            .with_color(p.warning)
                            .with_style(Properties::default().weight(Weight::Semibold))
                            .soft_wrap(false)
                            .finish(),
                    )
                    .finish()
            } else if syncing || pct < 100 {
                let track_w = 48.0_f32;
                let fill_w = (track_w * (pct as f32 / 100.0)).round().clamp(6.0, track_w);
                let progress_bar = ConstrainedBox::new(
                    Container::new(
                        Flex::row()
                            .with_main_axis_size(MainAxisSize::Max)
                            .with_main_axis_alignment(MainAxisAlignment::Start)
                            .with_cross_axis_alignment(CrossAxisAlignment::Center)
                            .with_child(
                                ConstrainedBox::new(
                                    Container::new(Empty::new().finish())
                                        .with_background_color(p.accent)
                                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(
                                            2.5,
                                        )))
                                        .finish(),
                                )
                                .with_width(fill_w)
                                .with_height(5.)
                                .finish(),
                            )
                            .finish(),
                    )
                    .with_background_color(if p.dark {
                        rgba(255, 255, 255, 28)
                    } else {
                        rgba(15, 23, 42, 22)
                    })
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(2.5)))
                    .finish(),
                )
                .with_width(track_w)
                .with_height(5.)
                .finish();

                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(6.)
                    .with_child(icons::draw_icon(UiIcon::Refresh, 13., p.accent))
                    .with_child(
                        Text::new(label.clone(), font, MICRO)
                            .with_color(p.text)
                            .with_style(Properties::default().weight(Weight::Semibold))
                            .soft_wrap(false)
                            .finish(),
                    )
                    .with_child(progress_bar)
                    .finish()
            } else {
                let dot = ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(p.success_text)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(4.)))
                        .finish(),
                )
                .with_width(8.)
                .with_height(8.)
                .finish();
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Min)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(5.)
                    .with_child(dot)
                    .with_child(
                        Text::new(label.clone(), font, MICRO)
                            .with_color(lerp_color(p.text2, p.text, t))
                            .with_style(Properties::default().weight(Weight::Semibold))
                            .soft_wrap(false)
                            .finish(),
                    )
                    .finish()
            };

            Container::new(
                ConstrainedBox::new(Align::new(inner).finish())
                    .with_height(28.)
                    .finish(),
            )
            .with_horizontal_padding(9.)
            .with_background_color(bg)
            .with_border(Border::all(1.).with_border_color(border))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(Action::AutoSync(AutoSyncAction::ToggleDetails))
        })
        .finish();

        let control = SavePosition::new(control, &id).for_single_frame().finish();
        #[cfg(feature = "ui-capture")]
        let control = super::super::scenario_check::control(id, control);
        control
    }

    pub(in crate::ui) fn auto_sync_domain_visual(
        &self,
        domain: AutoSyncDomain,
    ) -> (UiIcon, ColorU, ColorU) {
        let icon = match domain {
            AutoSyncDomain::DeviceAndSummary => UiIcon::Smartphone,
            AutoSyncDomain::MessagesAndCalls => UiIcon::Chat,
            AutoSyncDomain::CalendarAndSuite => UiIcon::Calendar,
            AutoSyncDomain::AmooAndTasks => UiIcon::Sparkles,
            AutoSyncDomain::AppsAndUpdates => UiIcon::Apps,
            AutoSyncDomain::GalleryAndStorage => UiIcon::Image,
            AutoSyncDomain::ContextAndPanorama => UiIcon::Shield,
        };
        let style = super::super::icon_style::neutral_source_style(icon, &self.p());
        (style.icon, style.foreground, style.background)
    }

    fn auto_sync_domain_live_summary(&self, domain: AutoSyncDomain) -> String {
        let status = self.auto_sync.domain_status(domain);
        if self.selected.is_none() {
            return "等待手机连接".into();
        }
        match status {
            AutoSyncDomainStatus::Pending => "等待后台队列…".into(),
            AutoSyncDomainStatus::Paused { reason } => reason.clone(),
            AutoSyncDomainStatus::Syncing { .. } => "正在读取本轮更新，已有内容仍可使用".into(),
            AutoSyncDomainStatus::Partial {
                item_count,
                warning,
                ..
            } => format!("已保存 {item_count} 条资料。{warning}"),
            AutoSyncDomainStatus::Failed { error, .. } => {
                format!("{error}。已有内容仍可使用")
            }
            AutoSyncDomainStatus::Synced { duration_ms, .. } => {
                let ms = (*duration_ms).max(1);
                match domain {
                    AutoSyncDomain::DeviceAndSummary => {
                        let m = self.cached_device_metrics();
                        match m.battery_level {
                            Some(b) => format!("{b}% 电量 · {ms}ms"),
                            None => format!("设备遥测就绪 · {ms}ms"),
                        }
                    }
                    AutoSyncDomain::MessagesAndCalls => {
                        let threads = self.push.snapshot.threads.len();
                        let sms = self.sms.len();
                        let calls = self.call_logs.len();
                        format!("{threads} 会话 / {sms} 短信 / {calls} 通话")
                    }
                    AutoSyncDomain::CalendarAndSuite => {
                        let cal = self.back_screen.stepos_suite.calendar_events.len();
                        let alarms = self.back_screen.stepos_suite.alarms.len();
                        format!("{cal} 项日程 / {alarms} 个闹钟")
                    }
                    AutoSyncDomain::AmooAndTasks => {
                        let sess = self.amoo_sessions.len();
                        let tasks = self.workspace.journal.tasks.len();
                        format!("{sess} 个会话 / {tasks} 项任务")
                    }
                    AutoSyncDomain::AppsAndUpdates => {
                        let apps = self.apps.len();
                        let upd = self.store_updatable_count();
                        if upd > 0 {
                            format!("{apps} 款应用 ({upd} 款可更新)")
                        } else {
                            format!("{apps} 款已装应用")
                        }
                    }
                    AutoSyncDomain::GalleryAndStorage => {
                        let photos = self.back_screen.gallery_items.len();
                        format!("{photos} 张媒体 · {ms}ms")
                    }
                    AutoSyncDomain::ContextAndPanorama => {
                        let ev = self.context.overview.evidence_count;
                        let notifs = self.notification_center.items.len();
                        format!("{ev} 条证据 / {notifs} 条提醒")
                    }
                }
            }
        }
    }

    fn render_auto_sync_domain_row(&self, domain: AutoSyncDomain) -> El {
        let p = self.p();
        let status = self.auto_sync.domain_status(domain);
        let (icon, icon_fg, _) = self.auto_sync_domain_visual(domain);
        let (badge, fg, bg) = match status {
            AutoSyncDomainStatus::Synced { .. } => ("已更新", p.success_text, p.success_soft),
            AutoSyncDomainStatus::Partial { .. } => {
                (status.badge_label(), p.warning, p.warning_soft)
            }
            AutoSyncDomainStatus::Syncing { .. } => ("同步中", p.accent, p.accent_soft),
            AutoSyncDomainStatus::Failed { .. } => ("待重试", p.warning, p.warning_soft),
            AutoSyncDomainStatus::Pending => ("等待更新", p.text3, p.tag_bg),
            AutoSyncDomainStatus::Paused { .. } => ("已暂停", p.text3, p.tag_bg),
        };
        let mut controls = vec![self.link_action(
            format!("auto-sync-view-{}", domain.key()),
            "查看内容",
            Action::Page(domain.page_target()),
        )];
        if status.is_failed() || matches!(status, AutoSyncDomainStatus::Partial { .. }) {
            controls.push(self.link_action(
                format!("auto-sync-retry-{}", domain.key()),
                "重新读取",
                Action::AutoSync(AutoSyncAction::SyncDomain(domain)),
            ));
        }
        let updated = status
            .synced_at_ms()
            .and_then(chrono::DateTime::from_timestamp_millis)
            .map(|at| {
                format!(
                    "更新于 {}",
                    at.with_timezone(&chrono::Local).format("%H:%M:%S")
                )
            })
            .unwrap_or_else(|| {
                if status.is_syncing() {
                    format!("已等待 {} 秒", status.duration_ms().unwrap_or(0) / 1000)
                } else {
                    String::new()
                }
            });
        let status_glyph = match status {
            AutoSyncDomainStatus::Synced { .. } => UiIcon::Check,
            AutoSyncDomainStatus::Partial { .. } | AutoSyncDomainStatus::Failed { .. } => {
                UiIcon::AlertTriangle
            }
            AutoSyncDomainStatus::Syncing { .. } => UiIcon::Refresh,
            AutoSyncDomainStatus::Paused { .. } => UiIcon::Pause,
            AutoSyncDomainStatus::Pending => UiIcon::Clock,
        };
        let row = Container::new(self.vstack(
            4.,
            vec![
                self.between(
                    self.lead(
                        7.,
                        self.content_icon(icon, 16., icon_fg),
                        self.line_w(domain.short_label(), BODY, p.text, Weight::Semibold),
                    ),
                    self.hstack(
                        3.,
                        vec![
                            self.status_icon(
                                format!("sync-status-{}", domain.key()),
                                badge,
                                status_glyph,
                                fg,
                            ),
                            self.hstack(0., controls),
                        ],
                    ),
                ),
                self.text(self.auto_sync_domain_live_summary(domain), MICRO, p.text2),
                self.line(updated, MICRO, p.text3),
            ],
        ))
        .with_uniform_padding(10.)
        .with_background_color(
            if status.is_failed() || matches!(status, AutoSyncDomainStatus::Partial { .. }) {
                bg
            } else {
                p.surface
            },
        )
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
        .finish();
        #[cfg(feature = "ui-capture")]
        let row = super::super::scenario_check::control(
            format!("auto-sync-domain-{}", domain.key()),
            row,
        );
        row
    }

    /// Sync details live alongside connection details; neither entry opens an overlay.
    pub(crate) fn render_device_sync_panel(&self) -> El {
        let p = self.p();
        let connected = self.selected.is_some();
        let enabled = self.auto_sync.enabled;
        let partial = self.auto_sync.partial_count();
        let status = if !connected {
            "等待连接"
        } else if !enabled {
            "自动更新已暂停"
        } else if self.auto_sync.is_syncing() {
            "正在更新"
        } else if self
            .auto_sync
            .domains
            .iter()
            .any(AutoSyncDomainStatus::is_failed)
        {
            "部分更新失败"
        } else if partial > 0 {
            "已更新 · 来源受限"
        } else {
            "已更新 · 自动守护中"
        };
        let mut actions = vec![self.notif_action_pill(
            "auto-sync-toggle-enabled",
            if enabled {
                "暂停自动更新"
            } else {
                "恢复自动更新"
            },
            false,
            Action::AutoSync(AutoSyncAction::SetEnabled(!enabled)),
        )];
        if connected && !self.auto_sync.is_syncing() {
            actions.push(self.notif_action_pill(
                "auto-sync-now-btn",
                "立即更新",
                true,
                Action::AutoSync(AutoSyncAction::SyncAllNow),
            ));
        }
        let mut rows = vec![
            self.between(
                self.heading("数据同步"),
                self.pill_badge(
                    status,
                    if partial > 0 { p.warning } else { p.text2 },
                    p.tag_bg,
                ),
            ),
            self.between(
                self.text(
                    format!(
                        "本轮已处理 {}/{} 类资料{}",
                        self.auto_sync.completed_count(),
                        self.auto_sync.participating_count(),
                        if self.auto_sync.paused_count() > 0 {
                            format!(" · {} 项暂停", self.auto_sync.paused_count())
                        } else {
                            String::new()
                        }
                    ),
                    CAPTION,
                    p.text2,
                ),
                self.wrap(actions),
            ),
        ];
        // Put actionable limitations first, so they cannot hide below seven large cards.
        let mut domains = AutoSyncDomain::ALL.to_vec();
        domains.sort_by_key(|d| {
            !matches!(
                self.auto_sync.domain_status(*d),
                AutoSyncDomainStatus::Partial { .. } | AutoSyncDomainStatus::Failed { .. }
            )
        });
        for pair in domains.chunks(2) {
            rows.push(
                self.responsive_columns(
                    900.,
                    S2,
                    S2,
                    CrossAxisAlignment::Start,
                    pair.iter()
                        .map(|domain| (1., self.render_auto_sync_domain_row(*domain)))
                        .collect(),
                ),
            );
        }
        rows.push(self.text(
            "资料在后台自动更新；生成方案和总结可同时进行。",
            MICRO,
            p.text3,
        ));
        let panel = self.panel(rows);
        #[cfg(feature = "ui-capture")]
        let panel = super::super::scenario_check::control("device-sync-details".into(), panel);
        panel
    }

    /// A fixed toolbar target. The badge is an overlay, never a reserved text
    /// column: an empty inbox leaves the bell centered and no surrounding frame.
    pub(crate) fn render_top_bar_notification_bell(&self) -> El {
        let p = self.p();
        let font = self.font;
        let mode = self.notification_center.alert_mode;
        let unread = self.notification_unread_count();
        let drawer_open = self.notification_center.drawer_open;
        let id = "top-bar-notification-bell".to_string();
        let state = self.mouse_state(&id);
        let tooltip = if mode == NotificationAlertMode::Muted {
            "消息中心 · 提醒已关闭".to_string()
        } else if unread > 0 {
            format!("消息中心 · {unread} 项未读")
        } else {
            "消息中心 · 全部已读".to_string()
        };

        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if clicked {
                p.press_color(p.hover)
            } else if drawer_open {
                lerp_color(p.accent_soft, p.hover, t * 0.5)
            } else {
                lerp_color(ColorU::transparent_black(), p.hover, t)
            };
            let fg = if drawer_open {
                p.accent
            } else if mode == NotificationAlertMode::Muted {
                p.text3
            } else {
                lerp_color(p.text2, p.text, t)
            };
            let bell = with_control_alias(
                icons::draw_icon(UiIcon::Bell, 18., fg),
                "top-bar-notification-glyph",
            );
            let button = ConstrainedBox::new(
                Container::new(Align::new(bell).finish())
                    .with_background_color(bg)
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            )
            .with_width(36.)
            .with_height(32.)
            .finish();

            let indicator = match mode {
                NotificationAlertMode::BadgeOnly if unread > 0 => Some(
                    ConstrainedBox::new(
                        Container::new(Empty::new().finish())
                            .with_background_color(p.danger)
                            .with_border(Border::all(1.5).with_border_color(p.bg))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
                            .finish(),
                    )
                    .with_width(9.)
                    .with_height(9.)
                    .finish(),
                ),
                NotificationAlertMode::PopupAndBadge if unread > 0 => {
                    let count = if unread > 9 {
                        "9+".to_string()
                    } else {
                        unread.to_string()
                    };
                    Some(
                        ConstrainedBox::new(
                            Container::new(
                                Align::new(
                                    Text::new(count, font, 8.5)
                                        .with_color(ColorU::white())
                                        .with_style(Properties::default().weight(Weight::Bold))
                                        .soft_wrap(false)
                                        .finish(),
                                )
                                .finish(),
                            )
                            .with_background_color(p.danger)
                            .with_border(Border::all(1.5).with_border_color(p.bg))
                            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                            .finish(),
                        )
                        .with_width(if unread > 9 { 20. } else { 16. })
                        .with_height(16.)
                        .finish(),
                    )
                }
                NotificationAlertMode::Muted => Some(
                    ConstrainedBox::new(
                        Container::new(
                            Align::new(icons::draw_icon(UiIcon::Mute, 9., p.text3)).finish(),
                        )
                        .with_background_color(p.bg)
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(6.)))
                        .finish(),
                    )
                    .with_width(12.)
                    .with_height(12.)
                    .finish(),
                ),
                _ => None,
            };
            let mut stack = Stack::new().with_child(button);
            if let Some(indicator) = indicator {
                stack = stack.with_positioned_child(
                    with_control_alias(indicator, "top-bar-notification-indicator"),
                    OffsetPositioning::offset_from_parent(
                        vec2f(2., -3.),
                        ParentOffsetBounds::Unbounded,
                        ParentAnchor::TopRight,
                        ChildAnchor::TopRight,
                    ),
                );
            }
            if mouse.is_mouse_over_element() && !drawer_open {
                let tip = Container::new(self.line(tooltip.clone(), MICRO, p.text))
                    .with_horizontal_padding(S2)
                    .with_vertical_padding(S1)
                    .with_background_color(p.bg)
                    .with_border(Border::all(1.).with_border_color(p.border))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish();
                stack = stack.with_positioned_child(
                    tip,
                    OffsetPositioning::offset_from_parent(
                        vec2f(0., S1),
                        ParentOffsetBounds::WindowByPosition,
                        ParentAnchor::BottomMiddle,
                        ChildAnchor::TopMiddle,
                    ),
                );
            }
            stack.finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(|ctx, _, _| {
            ctx.dispatch_typed_action(Action::NotificationCenter(
                NotificationCenterAction::ToggleDrawer,
            ))
        })
        .on_right_click(move |ctx, _, _| {
            ctx.dispatch_typed_action(Action::NotificationCenter(
                NotificationCenterAction::SetAlertMode(mode.next()),
            ))
        })
        .finish();

        let control = SavePosition::new(control, &id).for_single_frame().finish();
        #[cfg(feature = "ui-capture")]
        let control = super::super::scenario_check::control(id, control);
        control
    }

    /// Compact icon pager button (`<` / `>`) used in the 360-style popup stack switcher.
    fn notif_pager_btn(&self, id: impl Into<String>, icon: UiIcon, action: Action) -> El {
        let id = id.into();
        let p = self.p();
        let state = self.mouse_state(format!("notif-pager:{id}"));
        let control = Hoverable::new(state, move |mouse| {
            let t = self.hover_t(&mouse);
            let clicked = mouse.is_clicked();
            let bg = if clicked {
                p.accent_soft
            } else {
                lerp_color(p.surface, p.hover, t)
            };
            let border = if clicked {
                p.accent
            } else {
                lerp_color(p.border, p.border_strong, t)
            };
            let fg = if clicked {
                p.accent
            } else {
                lerp_color(p.text2, p.text, t)
            };
            ConstrainedBox::new(
                Container::new(
                    Flex::row()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Center)
                        .with_cross_axis_alignment(CrossAxisAlignment::Center)
                        .with_child(icons::draw_icon(icon, 11., fg))
                        .finish(),
                )
                .with_background_color(bg)
                .with_border(Border::all(1.).with_border_color(border))
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(5.)))
                .finish(),
            )
            .with_width(20.)
            .with_height(18.)
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        let control = SavePosition::new(control, &id).for_single_frame().finish();
        #[cfg(feature = "ui-capture")]
        let control = super::super::scenario_check::control(id, control);
        control
    }

    #[allow(dead_code)]
    fn render_popup_scene_widget(&self, _item: &UnifiedNotificationItem) -> Option<El> {
        None
    }

    /// Minimalist Popup Notification Card (`width: 380px`) focused strictly on core content:
    /// Avatar + Title/Source + Core Message + Actions; no expiration or countdown.
    pub(crate) fn render_360_popup_card(&self) -> Option<El> {
        self.render_360_popup_card_for_window(false)
    }

    fn popup_thread(
        &self,
        item: &UnifiedNotificationItem,
    ) -> Option<&stepx_companion::push::ImThread> {
        let thread_id = item.id.strip_prefix("im-thread:")?;
        self.push
            .snapshot
            .threads
            .iter()
            .find(|t| t.thread_id == thread_id && t.package_name == item.package_name)
    }

    fn popup_message_binding(&self, item: &UnifiedNotificationItem) -> Option<(&str, &str)> {
        if !self
            .push
            .prefs
            .effective_popup_mode(&item.package_name)
            .shows_full_body_and_reply()
        {
            return None;
        }
        let thread = self.popup_thread(item)?;
        // Only this notification's latest incoming message may supply a preview.
        let message = thread
            .messages
            .iter()
            .filter(|m| !m.outgoing)
            .max_by_key(|m| m.occurred_at_ms().unwrap_or(m.observed_at_ms))?;
        let timestamp = message.occurred_at_ms().unwrap_or(message.observed_at_ms);
        if message.text.trim() != item.full_body.trim() || timestamp != item.sort_ts_ms {
            return None;
        }
        Some((&thread.thread_id, &message.id))
    }

    fn popup_message_image(&self, item: &UnifiedNotificationItem) -> Option<std::path::PathBuf> {
        let (thread_id, message_id) = self.popup_message_binding(item)?;
        stepx_companion::push::cached_im_media_path(thread_id, message_id)
    }

    fn popup_source_picture(&self, item: &UnifiedNotificationItem) -> El {
        let avatar = self
            .popup_thread(item)
            .filter(|t| !t.avatar_b64.trim().is_empty())
            .and_then(|t| t.avatar_path.as_ref())
            .filter(|p| p.is_file());
        if let Some(path) = avatar {
            ConstrainedBox::new(
                Image::new(
                    AssetSource::LocalFile {
                        path: path.to_string_lossy().into_owned(),
                    },
                    CacheOption::BySize,
                )
                .cover()
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(12.)))
                .finish(),
            )
            .with_width(44.)
            .with_height(44.)
            .finish()
        } else {
            self.notification_avatar(item.visual, false, 44.)
        }
    }

    /// Native popup controls stay inside the surface; tooltips must not be
    /// painted beyond a small window's clipping boundary.
    fn popup_header_control(&self, id: &'static str, icon: UiIcon, action: Action) -> El {
        let p = self.p();
        let button = Hoverable::new(self.mouse_state(format!("popup-header:{id}")), move |mouse| {
            ConstrainedBox::new(
                Container::new(Align::new(self.ui_icon(icon, 16., if mouse.is_mouse_over_element() { p.text } else { p.text2 })).finish())
                    .with_background_color(if mouse.is_mouse_over_element() { p.hover } else { ColorU::transparent_black() })
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                    .finish(),
            ).with_width(32.).with_height(32.).finish()
        })
        .with_cursor(Cursor::PointingHand)
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
        .finish();
        with_control_alias(button, id)
    }

    fn popup_reminder_menu(&self, item: &UnifiedNotificationItem, standalone: bool) -> El {
        let p = self.p();
        let header = self.between(
                self.line_w("弹窗管理", BODY, p.text, Weight::Semibold),
                self.popup_header_control(
                    "notif-popup-menu-back",
                    UiIcon::ChevronLeft,
                    Action::NotificationCenter(
                        NotificationCenterAction::PopupMenu(None),
                    ),
                ),
            );
        let protects_call = self.notification_center.items.iter().any(|entry|
            matches!(entry.popup_widget, NotificationPopupWidget::LiveCall { .. })
                && (self.notification_center.popup_queue.contains(&entry.id)
                    || self.notification_center.active_popup_id.as_ref() == Some(&entry.id)));
        let mut rows = Vec::new();
        let mut options: Vec<(&str, &str, NotificationCenterAction)> = vec![
            (
                "clear-batch",
                "清理本次弹窗",
                NotificationCenterAction::ClearPopupBatch { receipts: self.popup_batch_receipts() },
            ),
            (
                "30m",
                "30 分钟",
                NotificationCenterAction::SnoozePopups {
                    duration_ms: 30 * 60_000,
                },
            ),
            (
                "1h",
                "1 小时",
                NotificationCenterAction::SnoozePopups {
                    duration_ms: 60 * 60_000,
                },
            ),
            (
                "3h",
                "3 小时",
                NotificationCenterAction::SnoozePopups {
                    duration_ms: 3 * 60 * 60_000,
                },
            ),
            (
                "6h",
                "6 小时",
                NotificationCenterAction::SnoozePopups {
                    duration_ms: 6 * 60 * 60_000,
                },
            ),
            ("today", "今天", NotificationCenterAction::SnoozeToday),
        ];
        if !item.package_name.trim().is_empty() {
            options.push((
                "app",
                "此应用不提醒",
                NotificationCenterAction::MutePopupApp(item.id.clone()),
            ));
        }
        for (id, label, action) in options {
            let application = id == "app";
            let clear_batch = id == "clear-batch";
            if id == "30m" {
                rows.push(self.divider());
                rows.push(self.line("暂停提醒", MICRO, p.text3));
            }
            if application {
                rows.push(self.divider());
            }
            let action = Action::NotificationCenter(action);
            let control =
                Hoverable::new(self.mouse_state(format!("popup-menu-{id}")), move |mouse| {
                    let content = self.between(
                        self.hstack(
                            S2,
                            vec![
                                self.content_icon(
                                    if clear_batch {
                                        UiIcon::Trash
                                    } else if application {
                                        UiIcon::BellOff
                                    } else {
                                        UiIcon::Clock
                                    },
                                    15.,
                                    p.text2,
                                ),
                                self.line(label, BODY, p.text),
                            ],
                        ),
                        self.line(
                            if clear_batch {
                                if protects_call { "通话保留" } else { "消息保留" }
                            } else if application {
                                item.source_badge.as_str()
                            } else {
                                ""
                            },
                            MICRO,
                            p.text3,
                        ),
                    );
                    Container::new(content)
                        .with_horizontal_padding(S2)
                        .with_vertical_padding(7.)
                        .with_background_color(if mouse.is_mouse_over_element() {
                            p.hover
                        } else {
                            ColorU::transparent_black()
                        })
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
                        .finish()
                })
                .with_cursor(Cursor::PointingHand)
                .on_click(move |ctx, _, _| ctx.dispatch_typed_action(action.clone()))
                .finish();
            rows.push(with_control_alias(
                control,
                &if clear_batch { "notif-popup-clear-batch".to_string() } else { format!("notif-popup-snooze-{id}") },
            ));
        }
        let full = self.push.prefs.effective_popup_mode(&item.package_name).shows_full_body_and_reply()
            || matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. });
        let height = if full { 194. } else { 118. };
        // Keep the native popup's current size. The close/back control and the
        // clear command are visible immediately; longer pause choices scroll.
        let menu = ConstrainedBox::new(self.pane_scroll(
            self.notification_center.popup_menu_scroll.clone(), self.vstack(4., rows)))
            .with_height(height - 68.).finish();
        let body = ConstrainedBox::new(Container::new(self.vstack(6., vec![header, menu]))
            .with_uniform_padding(14.).finish()).with_height(height - 2.).finish();
        let card = if standalone {
            self.standalone_popup_card(body, POPUP_CARD_W, item.is_red_packet())
        } else {
            self.elevated_overlay_card(body, POPUP_CARD_W, false)
        };
        with_control_alias(card, "notification-popup-menu")
    }

    pub(crate) fn render_360_popup_card_for_window(&self, standalone_window: bool) -> Option<El> {
        if self.notification_center.drawer_open
            && (!standalone_window || self.desktop_popup.main_window_active)
        {
            return None;
        }
        let item = self.notification_active_popup()?;
        let mode = self.push.prefs.effective_popup_mode(&item.package_name);
        if !mode.allows_popup() {
            return None;
        }
        if self.notification_center.popup_menu_item_id.as_deref() == Some(item.id.as_str()) {
            return Some(self.popup_reminder_menu(item, standalone_window));
        }
        let full = mode.shows_full_body_and_reply();
        let p = self.p();
        let red_packet = item.is_red_packet();
        let (packet_ink, packet_bg, _) = red_packet_colors(p.dark);
        let title = popup_display_title(item, full);
        let open =
            Action::NotificationCenter(NotificationCenterAction::OpenPopupDetails(item.id.clone()));
        let mut source = vec![self.line(&item.source_badge, 11., p.text3)];
        if let Some(thread) = self.popup_thread(item).filter(|t| t.unread_count > 1) {
            source.push(self.pill_badge(
                format!("{} 条", thread.unread_count),
                p.accent,
                p.accent_soft,
            ));
        }
        let header = Flex::row()
            .with_main_axis_size(MainAxisSize::Max)
            .with_cross_axis_alignment(CrossAxisAlignment::Start)
            .with_spacing(10.)
            .with_children(vec![
                self.popup_source_picture(item),
                Expanded::new(
                    1.,
                    self.vstack(
                        3.,
                        vec![
                            self.line_w(title, 15., if red_packet { packet_ink } else { p.text }, Weight::Semibold),
                            self.hstack(6., source),
                        ],
                    ),
                )
                .finish(),
                self.hstack(4., vec![self.popup_header_control(
                    "notif-popup-menu",
                    UiIcon::MoreHorizontal,
                    Action::NotificationCenter(
                        NotificationCenterAction::PopupMenu(Some(item.id.clone())),
                    ),
                ),
                self.popup_header_control(
                    "notif-popup-dismiss",
                    UiIcon::Close,
                    Action::NotificationCenter(
                        NotificationCenterAction::DismissPopup,
                    ),
                )]),
            ])
            .finish();
        let mut rows = vec![header];
        let queue_len = self.notification_center.popup_queue_len();
        let pager = if queue_len > 1 {
            self.hstack(
                2.,
                vec![
                    self.icon_action(
                        "notif-popup-prev",
                        "上一条",
                        UiIcon::ChevronLeft,
                        vec![Action::NotificationCenter(
                            NotificationCenterAction::PopupPrev,
                        )],
                        false,
                    ),
                    self.line(
                        format!(
                            "{}/{}",
                            self.notification_center.popup_cursor + 1,
                            queue_len
                        ),
                        11.,
                        p.text3,
                    ),
                    self.icon_action(
                        "notif-popup-next",
                        "下一条",
                        UiIcon::ChevronRight,
                        vec![Action::NotificationCenter(
                            NotificationCenterAction::PopupNext,
                        )],
                        false,
                    ),
                ],
            )
        } else {
            self.line(&item.time_label, 11., p.text3)
        };
        // Queue growth and the first/last card never move the reply field.
        let pager = ConstrainedBox::new(pager)
            .with_width(94.)
            .with_height(30.)
            .finish();
        if let NotificationPopupWidget::LiveCall { id, ringing } = &item.popup_widget {
            let route = if *ringing { "选择接听设备" }
                else if self.call_audio_on_computer() { "接听设备：电脑" }
                else { "接听设备：手机" };
            rows.push(self.between(self.line(route, 12., p.text2), pager));
            let feedback = if self.live_calls.owner == *id { self.live_calls.feedback.as_str() } else { "" };
            rows.push(self.line(super::super::notification_center_state::truncate_concise(feedback, 42), 11., p.text2));
            rows.push(self.live_call_controls(id, *ringing));
        } else if !full {
            // Hidden-content mode never builds media, codes, body text or reply controls.
            rows.push(self.between(self.line("新消息 · 内容已隐藏", 12., p.text2), pager));
        } else {
            let code = match &item.popup_widget {
                NotificationPopupWidget::OtpCode { code, .. } if !code.trim().is_empty() => {
                    Some((code.as_str(), "验证码"))
                }
                NotificationPopupWidget::ExpressPickup { pickup_code, .. }
                    if super::super::notification_center_state::is_real_pickup_code_label(
                        pickup_code,
                    ) =>
                {
                    Some((pickup_code.as_str(), "取件码"))
                }
                _ => None,
            };
            let raw = if item.full_body.trim().is_empty() {
                item.summary.trim()
            } else {
                item.full_body.trim()
            };
            let mut display_text = if matches!(
                item.popup_widget,
                NotificationPopupWidget::InlineImReply { .. }
            ) && !item.sender.is_empty()
                && item.sender != item.title
                && !raw.starts_with(&format!("{}：", item.sender))
                && !raw.starts_with(&format!("{}:", item.sender))
            {
                format!("{}：{}", item.sender, raw)
            } else {
                raw.to_owned()
            };
            if let NotificationPopupWidget::LiveCall { id, .. } = &item.popup_widget {
                if self.live_calls.owner == *id && !self.live_calls.feedback.is_empty() {
                    display_text = format!("{}\n{}", self.live_calls.feedback, display_text);
                }
            }
            let summary =
                super::super::notification_center_state::truncate_concise(&display_text, 90);
            let content: El = if red_packet {
                self.hstack(12., vec![
                    self.icon_badge(UiIcon::Gift, 24., packet_ink, p.danger_soft, 48.),
                    Expanded::new(1., self.vstack(4., vec![
                        self.line_w("红包消息", 14., packet_ink, Weight::Semibold),
                        self.text(summary, 12., p.text2),
                    ])).finish(),
                ])
            } else if let Some((code, label)) = code {
                self.between(
                    self.vstack(
                        3.,
                        vec![
                            self.line(label, 11., p.text3),
                            self.line_w(code, 26., p.text, Weight::Semibold),
                        ],
                    ),
                    self.icon_action(
                        "notif-popup-widget-copy-code",
                        format!("复制{label}"),
                        UiIcon::Copy,
                        vec![Action::CopyText(code.to_owned(), format!("已复制{label}"))],
                        true,
                    ),
                )
            } else {
                let text = Text::new(summary, self.font, 13.)
                    .with_color(p.text2)
                    .with_line_height_ratio(1.4)
                    .finish();
                let preview = self.popup_message_image(item);
                if let Some(path) = preview {
                    let picture = ConstrainedBox::new(
                        Image::new(
                            AssetSource::LocalFile {
                                path: path.to_string_lossy().into_owned(),
                            },
                            CacheOption::BySize,
                        )
                        .contain()
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(8.)))
                        .finish(),
                    )
                    .with_width(92.)
                    .with_height(64.)
                    .finish();
                    let picture =
                        Hoverable::new(self.mouse_state("popup-message-picture"), move |_| picture)
                            .with_cursor(Cursor::PointingHand)
                            .on_click(move |ctx, _, _| {
                                ctx.dispatch_typed_action(Action::Studio(
                                    super::super::studio_state::StudioAction::OpenPath(
                                        path.clone(),
                                    ),
                                ));
                            })
                            .finish();
                    self.hstack(12., vec![Expanded::new(1., text).finish(), picture])
                } else if raw == "[图片]" || raw == "[动画表情]" || raw == "[视频]" {
                    self.hstack(
                        12.,
                        vec![
                            self.icon_badge(UiIcon::Image, 24., p.accent, p.accent_soft, 52.),
                            self.vstack(
                                3.,
                                vec![
                                    self.line("图片消息", 13., p.text),
                                    self.line("预览暂不可用", 11., p.text3),
                                ],
                            ),
                        ],
                    )
                } else {
                    text
                }
            };
            // Clip the preview inside fixed native bounds; loading images or
            // changing text must not resize the window or displace its footer.
            let content = with_control_alias(content, &format!("notif-popup-content:{}", item.id));
            let preview: El = Box::new(PopupPreview {
                child: content,
                size: None,
            });
            rows.push(Expanded::new(1., Clipped::new(preview).finish()).finish());
            let can_reply = (!red_packet || !self.fields[super::super::F_TOAST_REPLY].text.is_empty()) && matches!(
                item.popup_widget,
                NotificationPopupWidget::InlineImReply {
                    can_reply: true,
                    ..
                }
            );
            let footer = if can_reply {
                self.hstack(
                    6.,
                    vec![
                        pager,
                        Expanded::new(1., self.input(super::super::F_TOAST_REPLY, "回复…", None))
                            .finish(),
                        self.icon_action(
                            "notif-popup-send-reply",
                            "发送回复",
                            UiIcon::Send,
                            vec![Action::NotificationCenter(
                                NotificationCenterAction::SubmitPopupReply(item.id.clone()),
                            )],
                            true,
                        ),
                    ],
                )
            } else if let NotificationPopupWidget::LiveCall { id, ringing } = &item.popup_widget {
                self.live_call_controls(id, *ringing)
            } else if let NotificationPopupWidget::CallerBadge { number, .. } = &item.popup_widget {
                self.between(pager, self.button_icon("missed-call-callback", "回拨", UiIcon::Phone,
                    Action::LiveCall(super::super::live_calls::LiveCallAction::CallBack(number.clone())), Btn::Primary))
            } else {
                let action_label = match item.popup_widget {
                    NotificationPopupWidget::SafetyConfirm { .. } => "查看确认",
                    NotificationPopupWidget::AppUpdateStrip { .. } => "查看更新",
                    _ if red_packet => "查看红包",
                    _ => "查看",
                };
                self.between(
                    pager,
                    self.notif_action_pill("notif-popup-primary-action", action_label, false, open),
                )
            };
            rows.push(with_control_alias(
                ConstrainedBox::new(Align::new(footer).bottom_left().finish())
                    .with_height(32.)
                    .finish(),
                "notif-popup-footer",
            ));
        }
        let body = Container::new(
            Flex::column()
                .with_main_axis_size(MainAxisSize::Max)
                .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                .with_spacing(10.)
                .with_children(rows)
                .finish(),
        )
        .with_uniform_padding(S4)
        .with_background_color(if standalone_window { ColorU::transparent_black() } else if red_packet { packet_bg } else { p.bg })
        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(concentric_radius(R_PANEL, 1.))))
        .finish();
        let body = if standalone_window {
            body
        } else {
            ConstrainedBox::new(body)
                .with_height(if full || matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. }) {
                    super::super::desktop_popup_window::POPUP_WIN_H_FULL_REPLY as f32 - 2.
                } else {
                    super::super::desktop_popup_window::POPUP_WIN_H_BANNER as f32 - 2.
                })
                .finish()
        };
        let open_details =
            Action::NotificationCenter(NotificationCenterAction::OpenPopupDetails(item.id.clone()));
        let body = Hoverable::new(self.mouse_state("notif-popup-card"), move |_| body)
            .with_defer_events_to_children()
            .with_cursor(Cursor::PointingHand)
            .on_click(move |ctx, _, _| ctx.dispatch_typed_action(open_details.clone()))
            .finish();
        let body: El = Box::new(PopupPointerGuard { child: body, bounds: None });
        let body = with_control_alias(body, "notif-popup-card");
        let width = if full {
            POPUP_CARD_W_FULL
        } else {
            POPUP_CARD_W
        };
        let card = if standalone_window {
            self.standalone_popup_card(body, width, red_packet)
        } else {
            self.elevated_overlay_card(body, width, false)
        };
        Some(with_control_alias(card, "notification-popup-toast"))
    }

    /// Crisp standalone OS popup card container (`width: 380px`) for `DesktopPopupWindowView`,
    /// sharing the exact same `12px` rounded corner structure and outer border as the main window.
    fn standalone_popup_card(&self, inner: El, width: f32, red_packet: bool) -> El {
        let p = self.p();
        let (_, packet_bg, packet_border) = red_packet_colors(p.dark);
        let outer_border_color = if red_packet { packet_border } else if p.dark {
            p.border_strong
        } else {
            rgb(0xDC, 0xDC, 0xE2)
        };
        ConstrainedBox::new(
            Clipped::new(
                Container::new(inner)
                    .with_background_color(if red_packet { packet_bg } else { p.bg })
                    .with_border(Border::all(1.).with_border_color(outer_border_color))
                    .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                    .finish(),
            )
            .finish(),
        )
        .with_width(width)
        .finish()
    }

    /// Notification drawers share the anchored-popover surface. Other modal callers
    /// retain their existing elevation treatment.
    pub(crate) fn elevated_overlay_card(&self, inner: El, width: f32, is_drawer: bool) -> El {
        let p = self.p();
        // One surface and one soft shadow, shared with search and live capsules.
        if is_drawer {
            return ConstrainedBox::new(popover_surface(inner, p))
            .with_width(width)
            .finish();
        }
        let border_color = if p.dark {
            p.border_strong
        } else {
            rgba(15, 23, 42, 32)
        };
        let key_shadow = if p.dark {
            DropShadow {
                color: rgba(0, 0, 0, 140),
                offset: vec2f(0., 8.),
                blur_radius: 18.,
                spread_radius: 4.,
            }
        } else {
            DropShadow {
                color: rgba(15, 23, 42, 42),
                offset: vec2f(0., 8.),
                blur_radius: 18.,
                spread_radius: 4.,
            }
        };
        let contact_shadow = if p.dark {
            DropShadow {
                color: rgba(0, 0, 0, 85),
                offset: vec2f(0., 2.),
                blur_radius: 6.,
                spread_radius: 1.,
            }
        } else {
            DropShadow {
                color: rgba(15, 23, 42, 22),
                offset: vec2f(0., 2.),
                blur_radius: 6.,
                spread_radius: 1.,
            }
        };

        let surface = Container::new(inner)
            .with_background_color(p.bg)
            .with_border(Border::all(1.).with_border_color(border_color))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
            .with_drop_shadow(contact_shadow)
            .finish();

        ConstrainedBox::new(
            Container::new(surface)
                .with_background_color(p.bg)
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_PANEL)))
                .with_drop_shadow(key_shadow)
                .finish(),
        )
        .with_width(width)
        .finish()
    }

    fn notif_category_tab(&self, id: &str, cat: NotificationCategory) -> El {
        self.chip(
            id,
            cat.short_label(),
            None,
            vec![Action::NotificationCenter(NotificationCenterAction::SelectCategory(cat))],
            self.notification_center.selected_category == cat,
            false,
        )
    }

    fn notification_row_avatar(&self, item: &UnifiedNotificationItem) -> El {
        // A conversation picture must belong to this exact thread and package.
        // Never guess from recent gallery pictures or another notification.
        if let Some(path) = self
            .popup_thread(item)
            .filter(|t| !t.avatar_b64.trim().is_empty())
            .and_then(|t| t.avatar_path.as_ref())
            .filter(|path| path.is_file())
        {
            return ConstrainedBox::new(
                Image::new(
                    AssetSource::LocalFile {
                        path: path.to_string_lossy().into_owned(),
                    },
                    CacheOption::BySize,
                )
                .cover()
                .with_corner_radius(CornerRadius::with_all(Radius::Pixels(10.)))
                .finish(),
            )
            .with_width(36.)
            .with_height(36.)
            .finish();
        }
        let (icon, foreground, background) = self.notification_visual_style(item.visual);
        self.icon_badge(icon, 17., foreground, background, 36.)
    }

    /// Fixed geometry: neither hovering nor becoming read inserts controls into the row.
    fn render_notification_item_card(&self, item: &UnifiedNotificationItem) -> El {
        let p = self.p();
        let avatar = self.notification_row_avatar(item);
        let title = self.hstack(
            6.,
            vec![
                Expanded::new(
                    1.,
                    self.line_w(
                        &item.title,
                        BODY,
                        p.text,
                        if item.unread {
                            Weight::Semibold
                        } else {
                            Weight::Medium
                        },
                    ),
                )
                .finish(),
                ConstrainedBox::new(
                    Container::new(Empty::new().finish())
                        .with_background_color(if item.unread {
                            p.danger
                        } else {
                            ColorU::transparent_black()
                        })
                        .with_corner_radius(CornerRadius::with_all(Radius::Pixels(3.)))
                        .finish(),
                )
                .with_width(6.)
                .with_height(6.)
                .finish(),
                self.line(&item.time_label, MICRO, p.text2),
            ],
        );
        let mut metadata = vec![item.source_badge.trim()];
        if item.unread {
            if let Some(status) = item
                .status_tag
                .as_deref()
                .filter(|tag| !tag.is_empty() && *tag != "未读")
            {
                metadata.push(status.trim_start_matches("AI "));
            }
        }
        let metadata = metadata
            .into_iter()
            .filter(|v| !v.is_empty())
            .collect::<Vec<_>>()
            .join(" · ");
        let center = self.vstack(
            3.,
            vec![
                title,
                with_control_alias(
                    self.line(&item.summary, CAPTION, p.text2),
                    &format!("notif-summary-{}", item.id),
                ),
                self.line(metadata, MICRO, p.text2),
            ],
        );
        let opens_view = notification_row_opens_view(item);
        let item_id = item.id.clone();
        let row_action = Action::NotificationCenter(if opens_view {
            NotificationCenterAction::TriggerItem(item_id)
        } else {
            NotificationCenterAction::MarkRead(item_id)
        });
        let mut controls = Vec::new();
        if let NotificationPopupWidget::CallerBadge { number, .. } = &item.popup_widget {
            if stepx_companion::phone::normalize_number(number).is_ok() {
                controls.push(self.icon_action(format!("notif-callback-{}", item.id), "回拨", UiIcon::Phone,
                    vec![Action::LiveCall(super::super::live_calls::LiveCallAction::CallBack(number.clone()))], true));
            }
        }
        // Navigation is on the whole row. Copy/update/approval still require an explicit control.
        if !opens_view {
            if let Some((label, primary)) = &item.primary_action {
                let trigger = Action::NotificationCenter(NotificationCenterAction::TriggerItem(
                    item.id.clone(),
                ));
                controls.push(
                    if matches!(primary, NotificationPrimaryAction::CopyText { .. }) {
                        self.icon_action(
                            format!("notif-trigger-{}", item.id),
                            label,
                            UiIcon::Copy,
                            vec![trigger],
                            false,
                        )
                    } else {
                        self.notif_action_pill(
                            format!("notif-trigger-{}", item.id),
                            label,
                            false,
                            trigger,
                        )
                    },
                );
            }
        } else {
            controls.push(
                ConstrainedBox::new(
                    Align::new(self.ui_icon(UiIcon::ChevronRight, 13., p.text3)).finish(),
                )
                .with_width(18.)
                .finish(),
            );
        }
        controls.push(self.icon_action(
            format!("notif-dismiss-{}", item.id),
            "移除此提醒",
            UiIcon::Close,
            vec![Action::NotificationCenter(
                NotificationCenterAction::DismissItem(item.id.clone()),
            )],
            false,
        ));
        let row_id = format!("notif-item-{}", item.id);
        let row = Hoverable::new(self.mouse_state(format!("notif-card:{row_id}")), |mouse| {
            Container::new(
                Flex::row()
                    .with_main_axis_size(MainAxisSize::Max)
                    .with_cross_axis_alignment(CrossAxisAlignment::Center)
                    .with_spacing(S3)
                    .with_child(avatar)
                    .with_child(
                        Expanded::new(
                            1.,
                            with_control_alias(center, &format!("notif-content-{}", item.id)),
                        )
                        .finish(),
                    )
                    .with_child(with_control_alias(
                        self.hstack(2., controls),
                        &format!("notif-controls-{}", item.id),
                    ))
                    .finish(),
            )
            .with_horizontal_padding(S2)
            .with_vertical_padding(S2)
            .with_background_color(lerp_color(
                if item.unread { p.surface } else { p.bg },
                p.hover,
                self.hover_t(mouse),
            ))
            .with_corner_radius(CornerRadius::with_all(Radius::Pixels(R_CONTROL)))
            .finish()
        })
        .with_cursor(Cursor::PointingHand)
        .with_defer_events_to_children()
        .on_click(move |ctx, _, _| ctx.dispatch_typed_action(row_action.clone()))
        .finish();
        with_control_alias(row, &row_id)
    }

    /// A quiet inbox: heading, one filter strip, message list and secondary maintenance.
    pub(crate) fn render_notification_center_drawer(&self) -> Option<El> {
        if !self.notification_center.drawer_open {
            return None;
        }
        let p = self.p();
        let raw_unread = self.notification_raw_unread_count();
        let visible_items = self.notification_visible_items();
        let page_key = format!("notification-drawer:{:?}",self.notification_center.selected_category);
        let range = self.list_page_range(&page_key,visible_items.len(),30);
        let paginated = visible_items.len() > 30;
        let (width, list_height) = notification_drawer_dimensions(self.workspace.viewport, paginated);
        let title = self.hstack(
            S2,
            vec![
                self.line_w("消息中心", TITLE, p.text, Weight::Semibold),
                self.line(
                    if raw_unread == 0 { "全部已读".into() } else { format!("{raw_unread} 未读") },
                    MICRO,
                    p.text2,
                ),
            ],
        );
        let header = self.between(
            title,
            self.hstack(
                2.,
                vec![
                    self.icon_action(
                        "notif-drawer-mark-all-read",
                        "全部标为已读",
                        UiIcon::Check,
                        vec![Action::NotificationCenter(
                            NotificationCenterAction::MarkAllRead,
                        )],
                        false,
                    ),
                    self.icon_action(
                        "notif-drawer-settings",
                        "提醒设置",
                        UiIcon::Settings,
                        vec![
                            Action::NotificationCenter(NotificationCenterAction::CloseDrawer),
                            Action::Settings(super::super::settings_state::SettingsAction::Tab(3)),
                        ],
                        false,
                    ),
                    self.icon_action(
                        "notif-drawer-close",
                        "关闭消息中心",
                        UiIcon::Close,
                        vec![Action::NotificationCenter(
                            NotificationCenterAction::CloseDrawer,
                        )],
                        false,
                    ),
                ],
            ),
        );
        let categories = [
            ("notif-cat-all", NotificationCategory::All),
            ("notif-cat-phone-apps", NotificationCategory::PhoneApps),
            ("notif-cat-phone-comm", NotificationCategory::PhoneComm),
            ("notif-cat-app-updates", NotificationCategory::AppUpdates),
            (
                "notif-cat-companion",
                NotificationCategory::CompanionAndTasks,
            ),
            ("notif-cat-system", NotificationCategory::System),
        ];
        let tabs = self.bounded_wrap(
            categories
                .into_iter()
                .map(|(id, cat)| self.notif_category_tab(id, cat))
                .collect(),
        );
        let mut top_rows = vec![header, tabs];
        if paginated {
            top_rows.push(ConstrainedBox::new(self.list_page_controls(&page_key,visible_items.len(),30))
                .with_height(32.)
                .finish());
        }
        let top = self.vstack(S2, top_rows);
        let list = if visible_items.is_empty() {
            ConstrainedBox::new(
                Align::new(self.vstack(
                    S2,
                    vec![
                        Align::new(self.content_icon(UiIcon::Bell, 24., p.text3)).finish(),
                        self.line_w("暂无消息", BODY, p.text2, Weight::Medium),
                    ],
                ))
                .finish(),
            )
            .with_height(160_f32.min(list_height))
            .finish()
        } else {
            ConstrainedBox::new(
                self.pane_scroll(
                    self.notification_center.list_scroll.clone(),
                    self.vstack(
                        S1,
                        visible_items
                            .into_iter()
                            .skip(range.start).take(range.len())
                            .map(|item| self.render_notification_item_card(item))
                            .collect(),
                    ),
                ),
            )
            .with_max_height(list_height)
            .finish()
        };
        let (mode_icon, mode_label) = if self.notification_center.is_currently_snoozed() {
            (UiIcon::BellOff, "弹窗已暂停")
        } else {
            match self.notification_center.alert_mode {
                NotificationAlertMode::PopupAndBadge => (UiIcon::Bell, "弹窗提醒"),
                NotificationAlertMode::BadgeOnly => (UiIcon::BellOff, "仅显示红点"),
                NotificationAlertMode::Muted => (UiIcon::Mute, "提醒已关闭"),
            }
        };
        let footer = Container::new(self.between(
            self.hstack(
                6.,
                vec![
                    self.content_icon(mode_icon, 13., p.text3),
                    self.line(mode_label, MICRO, p.text2),
                ],
            ),
            self.notif_header_pill(
                "notif-drawer-clear",
                "清理已读",
                Action::NotificationCenter(NotificationCenterAction::ClearReadAndDismissed),
            ),
        ))
        .with_padding_top(S2)
        .with_horizontal_padding(S1)
        .with_border(Border::top(1.).with_border_color(p.border))
        .finish();
        let list = if let Some(started) = self.notification_center.drawer_list_started {
            crate::ui::motion::sub_enter(list, started, !self.animations_disabled())
        } else {
            list
        };
        let body = Container::new(self.vstack(S2, vec![top, self.divider(), list, footer]))
            .with_uniform_padding(S3)
            .finish();
        let drawer = self.elevated_overlay_card(body, width, true);
        Some(with_control_alias(drawer, "notification-center-drawer"))
    }

    /// Wraps the main viewport body (outside `self.scroll`) with the notification drawer.
    ///
    /// Desktop reminders are delivered by the OS or a separate window. Only the drawer
    /// belongs in the main viewport. Uses `OverlayPriorityStack` so it stays at the top-right
    /// of the viewport regardless of page scroll, paints on top of `inner`,
    /// isolates mouse-wheel scrolling from the underlying page, and
    /// supports click-outside / Escape to close the drawer.
    pub(crate) fn with_notification_center_overlay(&self, inner: El) -> El {
        let drawer_open = self.notification_center.drawer_open;
        let auto_sync_open = false;
        let overlay = self.render_notification_center_drawer();
        let Some(overlay_el) = overlay else {
            return inner;
        };
        let card_bounds: Rc<Cell<Option<RectF>>> = Rc::new(Cell::new(None));
        let tracked_overlay = OverlayCardBounds {
            child: overlay_el,
            bounds: card_bounds.clone(),
        }
        .finish();

        let backdrop = Align::new(Empty::new().finish()).finish();

        let (offset, parent_anchor, child_anchor) = (
            vec2f(-18., 10.),
            ParentAnchor::TopRight,
            ChildAnchor::TopRight,
        );
        let overlay_stack = Stack::new()
            .with_child(backdrop)
            .with_positioned_child(
                tracked_overlay,
                OffsetPositioning::offset_from_parent(
                    offset,
                    ParentOffsetBounds::Unbounded,
                    parent_anchor,
                    child_anchor,
                ),
            )
            .finish();
        OverlayPriorityStack {
            base: inner,
            overlay_stack,
            card_bounds,
            drawer_open,
            auto_sync_open,
            popup_drag_last_pos: self.notification_center.popup_drag_last_pos.clone(),
        }
        .finish()
    }
}

/// Row navigation must never approve a task, install an app, or copy data implicitly.
fn notification_row_opens_view(item: &UnifiedNotificationItem) -> bool {
    matches!(
        item.primary_action.as_ref().map(|(_, action)| action),
        Some(
            NotificationPrimaryAction::OpenImThread { .. }
                | NotificationPrimaryAction::OpenSms
                | NotificationPrimaryAction::OpenAppUpdates
                | NotificationPrimaryAction::OpenPage(_)
        )
    )
}

fn notification_drawer_dimensions(viewport: Vector2F, paginated: bool) -> (f32, f32) {
    // The 32px pager and its 8px gap share the existing vertical budget;
    // the list yields space so the drawer keeps its bottom safe margin.
    let pager_height = if paginated { 32. + S2 } else { 0. };
    (
        (viewport.x() - SIDEBAR_W - 36.).clamp(260., 420.),
        (viewport.y() - 270. - pager_height).clamp(0., 420.),
    )
}

struct OverlayCardBounds {
    child: El,
    bounds: Rc<Cell<Option<RectF>>>,
}

impl facloud_ui::Element for OverlayCardBounds {
    fn layout(
        &mut self,
        c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        self.child.layout(c, ctx, app)
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.child.after_layout(ctx, app);
        if let (Some(origin), Some(sz)) = (self.child.origin(), self.child.size()) {
            self.bounds
                .set(Some(RectF::new(vec2f(origin.x(), origin.y()), sz)));
        }
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        if let Some(sz) = self.child.size() {
            self.bounds.set(Some(RectF::new(origin, sz)));
        }
        self.child.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }

    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        self.child.dispatch_event(event, ctx, app)
    }
}

struct OverlayPriorityStack {
    base: El,
    overlay_stack: El,
    card_bounds: Rc<Cell<Option<RectF>>>,
    drawer_open: bool,
    auto_sync_open: bool,
    popup_drag_last_pos: Rc<Cell<Option<(f32, f32)>>>,
}

impl OverlayPriorityStack {
    fn modal_open(&self) -> bool {
        self.drawer_open || self.auto_sync_open
    }

    fn effective_overlay_rect(&self) -> Option<RectF> {
        if let Some(rect) = self.card_bounds.get() {
            return Some(rect);
        }
        let origin_pt = self.base.origin()?;
        let origin = vec2f(origin_pt.x(), origin_pt.y());
        let sz = self.base.size()?;
        let card_w = if self.auto_sync_open {
            452.0_f32.min(sz.x())
        } else {
            444.0_f32.min(sz.x())
        };
        let card_h = if self.drawer_open {
            580.0_f32.min(sz.y())
        } else if self.auto_sync_open {
            480.0_f32.min(sz.y())
        } else {
            216.0_f32.min(sz.y())
        };
        let top_left = if self.auto_sync_open {
            vec2f(origin.x() + 14., origin.y() + 10.)
        } else {
            vec2f(
                (origin.x() + sz.x() - 18. - card_w).max(origin.x()),
                origin.y() + 10.,
            )
        };
        Some(RectF::new(top_left, vec2f(card_w, card_h)))
    }

    fn point_in_overlay(&self, pos: Vector2F) -> bool {
        let Some(rect) = self.effective_overlay_rect() else {
            return self.modal_open();
        };
        let margin = 12.0;
        let min_x = rect.origin().x() - margin;
        let max_x = rect.origin().x() + rect.width() + margin;
        let min_y = rect.origin().y() - margin;
        let max_y = rect.origin().y() + rect.height() + margin;
        pos.x() >= min_x && pos.x() <= max_x && pos.y() >= min_y && pos.y() <= max_y
    }
}

impl facloud_ui::Element for OverlayPriorityStack {
    fn layout(
        &mut self,
        c: SizeConstraint,
        ctx: &mut LayoutContext,
        app: &facloud_ui::AppContext,
    ) -> Vector2F {
        let sz = self.base.layout(c, ctx, app);
        self.overlay_stack
            .layout(SizeConstraint::strict(sz), ctx, app);
        sz
    }

    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &facloud_ui::AppContext) {
        self.base.after_layout(ctx, app);
        self.overlay_stack.after_layout(ctx, app);
    }

    fn paint(&mut self, origin: Vector2F, ctx: &mut PaintContext, app: &facloud_ui::AppContext) {
        self.base.paint(origin, ctx, app);
        self.overlay_stack.paint(origin, ctx, app);
    }

    fn size(&self) -> Option<Vector2F> {
        self.base.size()
    }

    fn origin(&self) -> Option<Point> {
        self.base.origin()
    }

    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &facloud_ui::AppContext,
    ) -> bool {
        if self.modal_open() {
            if let Event::KeyDown { keystroke, .. } = event.raw_event() {
                if keystroke.key.eq_ignore_ascii_case("escape") {
                    if self.drawer_open {
                        ctx.dispatch_typed_action(Action::NotificationCenter(
                            NotificationCenterAction::CloseDrawer,
                        ));
                    } else if self.auto_sync_open {
                        ctx.dispatch_typed_action(Action::AutoSync(
                            AutoSyncAction::SetDetailsOpen(false),
                        ));
                    }
                    return true;
                }
            }
        }

        let pointer_pos = match event.raw_event() {
            Event::ScrollWheel { position, .. }
            | Event::MouseMoved { position, .. }
            | Event::LeftMouseDown { position, .. }
            | Event::LeftMouseUp { position, .. }
            | Event::LeftMouseDragged { position, .. }
            | Event::RightMouseDown { position, .. } => Some(*position),
            _ => None,
        };

        if let Some(pos) = pointer_pos {
            if !self.modal_open() {
                match event.raw_event() {
                    Event::LeftMouseDragged { .. } => {
                        if let Some((lx, ly)) = self.popup_drag_last_pos.get() {
                            let dx = (pos.x() - lx).round() as i32;
                            let dy = (pos.y() - ly).round() as i32;
                            if dx != 0 || dy != 0 {
                                self.popup_drag_last_pos
                                    .set(Some((lx + dx as f32, ly + dy as f32)));
                                ctx.dispatch_typed_action(Action::NotificationCenter(
                                    NotificationCenterAction::MovePopupBy { dx, dy },
                                ));
                            }
                            return true;
                        }
                    }
                    Event::LeftMouseUp { .. } => {
                        if self.popup_drag_last_pos.take().is_some() {
                            let _ = self.overlay_stack.dispatch_event(event, ctx, app);
                            return true;
                        }
                    }
                    _ => {}
                }
            }
            if self.point_in_overlay(pos) {
                let handled = self.overlay_stack.dispatch_event(event, ctx, app);
                if !self.modal_open() {
                    if let Event::LeftMouseDown { .. } = event.raw_event() {
                        if !handled {
                            self.popup_drag_last_pos.set(Some((pos.x(), pos.y())));
                        } else {
                            self.popup_drag_last_pos.set(None);
                        }
                    }
                }
                return true;
            }
            if self.modal_open() {
                match event.raw_event() {
                    Event::ScrollWheel { .. } => {
                        return true;
                    }
                    Event::LeftMouseDown { .. } | Event::RightMouseDown { .. } => {
                        if self.drawer_open {
                            ctx.dispatch_typed_action(Action::NotificationCenter(
                                NotificationCenterAction::CloseDrawer,
                            ));
                        } else if self.auto_sync_open {
                            ctx.dispatch_typed_action(Action::AutoSync(
                                AutoSyncAction::SetDetailsOpen(false),
                            ));
                        }
                        return true;
                    }
                    _ => {
                        let _ = self.overlay_stack.dispatch_event(event, ctx, app);
                        return true;
                    }
                }
            }
            if self.overlay_stack.dispatch_event(event, ctx, app) {
                return true;
            }
            return self.base.dispatch_event(event, ctx, app);
        }

        if self.overlay_stack.dispatch_event(event, ctx, app) || self.modal_open() {
            return true;
        }
        self.base.dispatch_event(event, ctx, app)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn otp_banner_title_never_exposes_code_and_full_view_does_not_repeat_it() {
        let mut item = super::UnifiedNotificationItem {
            title: "验证码 735291".into(),
            popup_widget: super::NotificationPopupWidget::OtpCode { code: "735291".into(), source_app: "短信".into() },
            ..Default::default()
        };
        assert_eq!(super::popup_display_title(&item, false), "验证码");
        assert_eq!(super::popup_display_title(&item, true), "验证码");
        item.title = "账户登录验证".into();
        assert_eq!(super::popup_display_title(&item, false), "验证码");
        assert_eq!(super::popup_display_title(&item, true), "账户登录验证");
    }
    use super::*;
    use crate::ui::{
        notification_center_state::{NotificationCenterState, NotificationPriority},
        Page, TestUiConfigDirGuard,
    };
    use stepx_companion::push::{ImThread, OtpBanner};

    #[test]
    fn group_image_preview_keeps_original_body_through_notification_sync() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        let now = chrono::Local::now().timestamp_millis();
        app.push.snapshot.threads = vec![ImThread {
            thread_id: "group-image".into(),
            package_name: "com.tencent.mm".into(),
            title: "群聊".into(),
            last_sender: "张三".into(),
            last_message: "[图片]".into(),
            last_timestamp_ms: now,
            unread_count: 1,
            messages: vec![stepx_companion::push::ImMessage {
                id: "current-image".into(),
                sender: "张三".into(),
                text: "[图片]".into(),
                timestamp_ms: now,
                ..Default::default()
            }],
            ..Default::default()
        }];
        app.sync_notification_center();
        let item = app
            .notification_center
            .items
            .iter()
            .find(|i| i.id == "im-thread:group-image")
            .unwrap();
        assert_eq!(item.full_body, "[图片]");
        assert_eq!(
            app.popup_message_binding(item),
            Some(("group-image", "current-image"))
        );
    }

    #[test]
    fn popup_media_follows_privacy_app_rules_and_exact_message_identity() {
        use stepx_companion::{desktop_notify::PopupMode, push::ImMessage};
        let mut app = Companion::new_for_test();
        app.push.prefs = Default::default();
        app.push.snapshot.threads = vec![ImThread {
            thread_id: "media-exact".into(),
            package_name: "com.tencent.mm".into(),
            messages: vec![ImMessage {
                id: "image-1".into(),
                text: "[图片]".into(),
                timestamp_ms: 123,
                ..Default::default()
            }],
            ..Default::default()
        }];
        let mut item = UnifiedNotificationItem {
            id: "im-thread:media-exact".into(),
            package_name: "com.tencent.mm".into(),
            full_body: "[图片]".into(),
            sort_ts_ms: 123,
            ..Default::default()
        };
        assert_eq!(
            app.popup_message_binding(&item),
            Some(("media-exact", "image-1"))
        );
        app.push.prefs.set_popup_mode(PopupMode::Banner);
        assert!(app.popup_message_binding(&item).is_none());
        app.push
            .prefs
            .set_app_popup_mode("com.tencent.mm", Some(PopupMode::FullReply));
        assert!(
            app.popup_message_binding(&item).is_none(),
            "global privacy still masks an app override"
        );
        app.push.prefs.privacy_mask = false;
        assert!(app.popup_message_binding(&item).is_some());
        item.sort_ts_ms += 1;
        assert!(
            app.popup_message_binding(&item).is_none(),
            "an older picture cannot illustrate a newer notification"
        );
        item.sort_ts_ms = 123;
        item.id = "im-thread:someone-else".into();
        assert!(app.popup_message_binding(&item).is_none());
    }

    #[test]
    fn notification_rows_only_navigate_on_body_click() {
        let mut item = UnifiedNotificationItem::default();
        for action in [
            NotificationPrimaryAction::OpenImThread {
                thread_id: "exact-thread".into(),
            },
            NotificationPrimaryAction::OpenSms,
            NotificationPrimaryAction::OpenAppUpdates,
            NotificationPrimaryAction::OpenPage(Page::Phone),
        ] {
            item.primary_action = Some(("打开".into(), action));
            assert!(notification_row_opens_view(&item));
        }
        for action in [
            NotificationPrimaryAction::CopyText {
                label: "复制".into(),
                text: "123456".into(),
            },
            NotificationPrimaryAction::UpdateSingleApp {
                package_name: "test.package".into(),
            },
            NotificationPrimaryAction::ApproveGuiSafety(true),
            NotificationPrimaryAction::ResumeTask {
                task_id: "task".into(),
            },
        ] {
            item.primary_action = Some(("执行".into(), action));
            assert!(
                !notification_row_opens_view(&item),
                "side effects need their own explicit control"
            );
        }
        item.primary_action = None;
        assert!(!notification_row_opens_view(&item));
    }

    #[test]
    fn renders_bell_popup_and_drawer_across_modes_and_categories() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let _guard = TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.notification_center = NotificationCenterState::new_empty();

        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "654321".into(),
            source_app: "招商银行".into(),
            title: "招商银行".into(),
            raw_text: "您的验证码为 654321".into(),
            timestamp_ms: chrono::Local::now().timestamp_millis(),
            notification_key: "otp-test".into(),
        });
        app.push.snapshot.threads.push(ImThread {
            thread_id: "wx:1".into(),
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            title: "张晓峰".into(),
            unread_count: 2,
            last_message: "收到".into(),
            last_timestamp_ms: chrono::Local::now().timestamp_millis(),
            ..Default::default()
        });
        app.sync_notification_center();

        // 1. The independent popup renders when the notification drawer is closed.
        let _bell = app.render_top_bar_notification_bell();
        assert!(app.render_360_popup_card_for_window(true).is_some());
        assert!(app.render_notification_center_drawer().is_none());

        // 2. A foreground drawer handles notifications without a duplicate desktop card.
        app.apply_notification_center_action(NotificationCenterAction::ToggleDrawer);
        app.desktop_popup.main_window_active = true;
        assert!(app.notification_center.drawer_open);
        assert!(app.render_360_popup_card_for_window(true).is_none());
        assert!(app.render_notification_center_drawer().is_some());

        // 3. Cycle through all 6 categories and render drawer
        for cat in NotificationCategory::ALL {
            app.apply_notification_center_action(NotificationCenterAction::SelectCategory(cat));
            assert!(app.render_notification_center_drawer().is_some());
        }

        // 4. Switch between PopupAndBadge, BadgeOnly, and Muted and render bell + settings tab 3
        for mode in NotificationAlertMode::ALL {
            app.apply_notification_center_action(NotificationCenterAction::SetAlertMode(mode));
            let _bell = app.render_top_bar_notification_bell();
            app.page = Page::Settings;
            app.settings.tab = 3;
            let _settings = app.page_content();
        }

        // 5. Mark all read and clear -> empty state renders cleanly
        app.apply_notification_center_action(NotificationCenterAction::SelectCategory(
            NotificationCategory::All,
        ));
        app.apply_notification_center_action(NotificationCenterAction::MarkAllRead);
        app.apply_notification_center_action(NotificationCenterAction::ClearReadAndDismissed);
        assert!(app.notification_visible_items().is_empty());
        assert!(app.render_notification_center_drawer().is_some());
    }

    #[test]
    fn overlay_priority_stack_bounds_and_elevation_in_light_and_dark_themes() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let _guard = TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.notification_center = NotificationCenterState::new_empty();
        app.push.snapshot.otp_banners.push(OtpBanner {
            code: "889900".into(),
            source_app: "顺丰速运".into(),
            title: "顺丰速运".into(),
            raw_text: "取件码 889900".into(),
            timestamp_ms: 1_700_000_020_000,
            notification_key: "otp-sf".into(),
        });
        app.sync_notification_center();

        // Light & Dark palettes render elevated cards and viewport overlay cleanly
        for dark in [false, true] {
            app.palette.set(Palette::for_theme(dark));
            app.page = Page::Home;
            app.notification_center.drawer_open = false;
            let _toast_overlay = app.with_notification_center_overlay(app.page_content());
            app.notification_center.drawer_open = true;
            let _drawer_overlay = app.with_notification_center_overlay(app.page_content());
        }

        // Verify OverlayPriorityStack hit-test with 12px shadow margin
        let card_bounds = Rc::new(Cell::new(Some(RectF::new(
            vec2f(700., 60.),
            vec2f(420., 460.),
        ))));
        let stack = OverlayPriorityStack {
            base: Empty::new().finish(),
            overlay_stack: Empty::new().finish(),
            card_bounds,
            drawer_open: true,
            auto_sync_open: false,
            popup_drag_last_pos: Rc::new(Cell::new(None)),
        };
        // Inside drawer and inside 12px shadow margin -> captured by overlay
        assert!(stack.point_in_overlay(vec2f(850., 220.)));
        assert!(stack.point_in_overlay(vec2f(690., 55.)));
        assert!(stack.point_in_overlay(vec2f(1130., 530.)));
        // Far outside drawer (e.g. left side of Page::Home) -> outside overlay
        assert!(!stack.point_in_overlay(vec2f(240., 300.)));
    }

    #[test]
    fn renders_unified_device_sync_details_across_sync_states() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let _guard = TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.selected = None;
        app.auto_sync.on_device_disconnected();

        // 1. Disconnected state: pill renders, popover hidden until details_open is true
        let _details = app.render_device_sync_panel();
        let _pill_disconnected = app.render_top_bar_auto_sync_pill();

        // 2. Connect device & start syncing domains
        app.selected = Some(crate::ui::Device {
            serial: "DEV-SYNC-01".into(),
            transport: "DEV-SYNC-01".into(),
            model: "StepX PZ261A".into(),
            android: "16".into(),
            product: "pandora".into(),
        });
        app.auto_sync.on_device_connected("DEV-SYNC-01");
        app.auto_sync.mark_syncing(AutoSyncDomain::DeviceAndSummary);
        app.auto_sync
            .mark_synced(AutoSyncDomain::MessagesAndCalls, 4);
        let _pill_syncing = app.render_top_bar_auto_sync_pill();
        let _device_card_syncing = app.device_block();

        // 3. Open 7-domain sync details popover and verify rendering in light & dark palettes
        app.auto_sync.details_open = true;
        for dark in [false, true] {
            app.palette.set(Palette::for_theme(dark));
            let _details = app.render_device_sync_panel();
            let _overlay = app.with_notification_center_overlay(app.page_content());
        }

        // 4. Mark all 7 domains synced (100%) and verify pill + popover + sidebar card render cleanly
        for domain in AutoSyncDomain::ALL {
            app.auto_sync.mark_synced(domain, 1);
        }
        assert_eq!(app.auto_sync.overall_percent(), 100);
        let _pill_complete = app.render_top_bar_auto_sync_pill();
        let _details = app.render_device_sync_panel();
        let _device_card_complete = app.device_block();
    }

    #[test]
    fn popup_menu_keeps_fixed_size_and_first_action_inside_safe_insets() {
        struct MenuOnly(Companion);
        impl facloud_ui::Entity for MenuOnly { type Event = (); }
        impl facloud_ui::TypedActionView for MenuOnly { type Action = (); }
        impl facloud_ui::View for MenuOnly {
            fn ui_name() -> &'static str { "popup_menu_geometry" }
            fn render(&self, _: &facloud_ui::AppContext) -> El {
                Stack::new().with_child(self.0.render_360_popup_card_for_window(true).unwrap()).finish()
            }
        }
        let dir = tempfile::tempdir().unwrap();
        let _guard = TestUiConfigDirGuard::set(dir.path());
        facloud_ui::App::test((), |mut app| async move {
            for (mode, height) in [
                (stepx_companion::desktop_notify::PopupMode::Banner, 118.),
                (stepx_companion::desktop_notify::PopupMode::FullReply, 194.),
            ] {
                let mut companion = Companion::new_for_test();
                companion.notification_center = NotificationCenterState::new_empty();
                companion.push.prefs.popup_mode = mode;
                let item = UnifiedNotificationItem {
                    id: "menu-geometry".into(), package_name: "com.tencent.mm".into(),
                    source_badge: "微信".into(), title: "消息".into(), unread: true, ..Default::default()
                };
                companion.notification_center.active_popup_id = Some(item.id.clone());
                companion.notification_center.popup_menu_item_id = Some(item.id.clone());
                companion.notification_center.popup_queue.push_back(item.id.clone());
                companion.notification_center.items = vec![item];
                let (window, _) = app.add_window(facloud_ui::platform::WindowStyle::PopupToast, |_| MenuOnly(companion));
                let root = app.root_view_id(window).unwrap();
                app.update(move |ctx| {
                    let mut presenter = facloud_ui::Presenter::new(window);
                    presenter.invalidate(facloud_ui::WindowInvalidation {
                        updated: std::collections::HashSet::from([root]), ..Default::default()
                    }, ctx);
                    presenter.build_scene(vec2f(380., height), 1., None, ctx);
                    let positions = presenter.position_cache();
                    let card = positions.get_position("notification-popup-menu").unwrap();
                    assert!(card.max_x() <= 380. && card.max_y() <= height + 1., "{card:?}");
                    for id in ["notif-popup-menu-back", "notif-popup-clear-batch"] {
                        let control = positions.get_position(id).unwrap();
                        assert!(control.origin().x() >= 12. && control.max_x() <= 368., "{id}: {control:?}");
                        assert!(control.origin().y() >= 12. && control.max_y() <= height - 12., "{id}: {control:?}");
                    }
                });
            }
        });
    }

    #[test]
    fn renders_compact_popup_all_scene_widgets_and_bounds() {
        let dir = tempfile::tempdir().expect("create temp dir");
        let _guard = TestUiConfigDirGuard::set(dir.path());
        let mut app = Companion::new_for_test();
        app.notification_center = NotificationCenterState::new_empty();

        // Verify bottom-right workarea bounds calculation for FullReply (380x194) & Banner (380x118)
        let workarea = RectF::new(vec2f(0., 28.), vec2f(1920., 1020.));
        let bounds_with_widget = Companion::compute_desktop_popup_bounds(workarea, true);
        assert_eq!(bounds_with_widget.width(), 380.);
        assert_eq!(bounds_with_widget.height(), 194.);
        assert_eq!(bounds_with_widget.origin().x(), 1920. - 380. - 16.);
        assert_eq!(bounds_with_widget.origin().y(), 28. + 1020. - 194. - 16.);

        let bounds_plain = Companion::compute_desktop_popup_bounds(workarea, false);
        assert_eq!(bounds_plain.width(), 380.);
        assert_eq!(bounds_plain.height(), 118.);

        // Verify all 8 NotificationPopupWidget variants render cleanly in light & dark themes
        let widgets = vec![
            NotificationPopupWidget::OtpCode {
                code: "839201".into(),
                source_app: "招商银行".into(),
            },
            NotificationPopupWidget::ExpressPickup {
                pickup_code: "6-1-2048".into(),
                carrier: "顺丰速运".into(),
                station: "科技园丰巢柜".into(),
            },
            NotificationPopupWidget::InlineImReply {
                thread_id: "wx:product".into(),
                package_name: "com.tencent.mm".into(),
                can_reply: true,
                quick_phrases: vec!["收到".into(), "好的，马上处理".into(), "稍后回复".into()],
            },
            NotificationPopupWidget::AppUpdateStrip {
                count: 5,
                app_names: vec!["微信".into(), "飞书".into()],
                package_name: Some("com.tencent.mm".into()),
            },
            NotificationPopupWidget::CallerBadge {
                name: "林婉清".into(),
                number: "138 0000 0041".into(),
            },
            NotificationPopupWidget::SafetyConfirm {
                prompt: "需要确认提交订单操作".into(),
            },
            NotificationPopupWidget::ClipboardPreview {
                text: "https://stepx.ai/docs/popup-360".into(),
            },
            NotificationPopupWidget::None,
        ];

        for (idx, widget) in widgets.into_iter().enumerate() {
            let item_id = format!("test-popup-{idx}");
            app.notification_center.items = vec![
                UnifiedNotificationItem {
                    id: item_id.clone(),
                    category: NotificationCategory::PhoneComm,
                    visual: NotificationVisualKind::SmsOtp,
                    priority: NotificationPriority::P0Immediate,
                    package_name: "com.tencent.mm".into(),
                    app_name: "微信".into(),
                    sender: "测试发送人".into(),
                    full_body: "360 风格五区图形化桌面通知卡片完整未截断正文内容".into(),
                    banner_body: "微信 · 收到来自「测试发送人」的新消息".into(),
                    source_badge: "测试提醒".into(),
                    title: format!("测试卡片 #{idx}"),
                    summary: "360 风格五区图形化桌面通知卡片".into(),
                    time_label: "刚刚".into(),
                    sort_ts_ms: 1_700_000_000_000,
                    unread: true,
                    status_tag: Some("未读".into()),
                    primary_action: Some((
                        "立即处理".into(),
                        crate::ui::NotificationPrimaryAction::OpenSms,
                    )),
                    popup_widget: widget,
                },
                UnifiedNotificationItem {
                    id: "test-popup-second".into(),
                    category: NotificationCategory::PhoneApps,
                    visual: NotificationVisualKind::WeChat,
                    priority: NotificationPriority::P1Digest,
                    package_name: "com.tencent.mm".into(),
                    app_name: "微信".into(),
                    sender: "第二条发送人".into(),
                    full_body: "支持多条消息叠层翻页".into(),
                    banner_body: "微信 · 收到来自「第二条发送人」的新消息".into(),
                    source_badge: "微信".into(),
                    title: "第二条消息".into(),
                    summary: "支持多条消息叠层翻页".into(),
                    time_label: "刚刚".into(),
                    sort_ts_ms: 1_700_000_000_000,
                    unread: true,
                    status_tag: Some("未读".into()),
                    primary_action: None,
                    popup_widget: NotificationPopupWidget::None,
                },
            ];
            app.notification_center.active_popup_id = Some(item_id.clone());
            app.notification_center.popup_queue =
                vec![item_id.clone(), "test-popup-second".into()].into();
            app.notification_center.popup_cursor = 0;
            app.notification_center.popup_total_ms = 14_000;
            app.notification_center.popup_remaining_ms = 9_500;
            app.notification_center.popup_hovered = idx % 2 == 1;

            for mode in [
                stepx_companion::desktop_notify::PopupMode::FullReply,
                stepx_companion::desktop_notify::PopupMode::Banner,
            ] {
                app.push.prefs.popup_mode = mode;
                for dark in [false, true] {
                    app.palette.set(Palette::for_theme(dark));
                    assert!(app.render_360_popup_card().is_some());
                }
            }
        }
    }
}
