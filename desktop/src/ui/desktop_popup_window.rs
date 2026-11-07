//! StepX-owned desktop notification window, independent of the main window.
use super::widgets::El;
use super::*;
use facloud_ui::{platform::WindowBounds, TypedActionView, WeakViewHandle, WindowId};
use pathfinder_geometry::rect::RectF;
use std::sync::mpsc;

pub(crate) const POPUP_WIN_W_BANNER: f64 = 380.0;
pub(crate) const POPUP_WIN_H_BANNER: f64 = 118.0;
pub(crate) const POPUP_WIN_W_FULL_REPLY: f64 = 380.0;
pub(crate) const POPUP_WIN_H_FULL_REPLY: f64 = 194.0;
pub(crate) const POPUP_WIN_W: f64 = POPUP_WIN_W_FULL_REPLY;
pub(crate) const POPUP_WIN_H_WIDGET: f64 = POPUP_WIN_H_FULL_REPLY;
pub(crate) const POPUP_WIN_H_PLAIN: f64 = POPUP_WIN_H_BANNER;

fn popup_editor_action(action: &Action) -> bool {
    matches!(
        action,
        Action::Focus(super::F_TOAST_REPLY)
            | Action::InputPointer(super::F_TOAST_REPLY, ..)
            | Action::InputPointerUp
            | Action::Insert(_)
            | Action::Preedit(_)
            | Action::Backspace
            | Action::BackspaceLine
            | Action::Delete
            | Action::ExtendSelection(_)
            | Action::Left
            | Action::Right
            | Action::Home
            | Action::End
            | Action::SelectAll
            | Action::UndoInput
            | Action::RedoInput
            | Action::CopyInput
            | Action::CutInput
            | Action::Paste
    )
}

fn popup_window_action(action: &Action) -> bool {
    popup_editor_action(action) || matches!(action,
        Action::SubmitFocused
            | Action::LiveCall(_)
            | Action::Escape
            | Action::CopyText(..)
            | Action::VoiceInput(voice_input_state::VoiceInputAction::Inspect(F_TOAST_REPLY))
            | Action::Studio(studio_state::StudioAction::OpenPath(_))
            | Action::NotificationCenter(
                NotificationCenterAction::DismissPopup
                    | NotificationCenterAction::ClearPopupBatch { .. }
                    | NotificationCenterAction::PopupPrev
                    | NotificationCenterAction::PopupNext
                    | NotificationCenterAction::PopupHoverChanged(_)
                    | NotificationCenterAction::SubmitPopupReply(_)
                    | NotificationCenterAction::PopupMenu(_)
                    | NotificationCenterAction::MutePopupApp(_)
                    | NotificationCenterAction::SnoozePopups { .. }
                    | NotificationCenterAction::SnoozeToday
                    | NotificationCenterAction::OpenPopupDetails(_)
            )
    )
}

struct PopupAction {
    action: Action,
    // Bind queued keystrokes to the notification that was painted at dispatch.
    item_id: Option<String>,
    reply_draft: Option<String>,
}

pub(crate) struct DesktopPopupWindowView {
    companion: WeakViewHandle<Companion>,
    tx: mpsc::Sender<PopupAction>,
}

impl Entity for DesktopPopupWindowView {
    type Event = ();
}

impl TypedActionView for DesktopPopupWindowView {
    type Action = Action;

    fn handle_action(&mut self, action: &Action, ctx: &mut ViewContext<Self>) {
        // A shared Keyboard also knows main-window shortcuts. This window has
        // no tab order, file selection or global search field to route them to.
        if !popup_window_action(action) { return; }
        let Some(owner) = self.companion.upgrade(ctx) else { return; };
        if let Action::NotificationCenter(NotificationCenterAction::PopupHoverChanged(hovered)) = action {
            // Claim pointer ownership before an incoming snapshot can swap cards.
            let changed = owner.update(ctx, |view, owner_ctx| {
                if view.notification_center.popup_hovered == *hovered { return false; }
                view.notification_center.popup_hovered = *hovered;
                if !hovered && view.tick_notification_popup_timer(0) { owner_ctx.notify(); }
                true
            });
            if changed { ctx.notify(); }
            return;
        }
        let (item_id, reply_focused, composing) = owner.read(ctx, |view, _| (
            view.notification_active_popup().map(|i| i.id.clone()),
            view.desktop_popup.reply_focused,
            !view.fields[F_TOAST_REPLY].preedit.is_empty(),
        ));
        if matches!(action, Action::Focus(F_TOAST_REPLY) | Action::InputPointer(F_TOAST_REPLY, ..)) {
            // Only an explicit click activates this window's editor.
            ctx.focus_self();
            #[cfg(any(target_os = "linux", target_os = "windows"))]
            if let Some(window) = facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id()) {
                window.set_ime_allowed(true);
                window.focus_window();
            }
        }
        if popup_editor_action(action) {
            // The owner's timer can be idle for 500 ms. Apply editor state and
            // clipboard actions now, before the next native input event arrives.
            // Do not synchronize windows here: this popup view is already borrowed.
            owner.update(ctx, |view, owner_ctx| {
                view.apply_popup_editor_action(item_id.as_deref(), action, owner_ctx);
            });
            ctx.report_active_cursor_position_update();
            ctx.notify();
            return;
        }
        if matches!(action, Action::CopyText(..)) {
            owner.update(ctx, |view, owner_ctx| {
                view.apply_popup_copy_action(action, owner_ctx);
            });
            ctx.notify();
            return;
        }
        let submit_id = match action {
            Action::SubmitFocused if reply_focused => item_id.clone(),
            Action::NotificationCenter(NotificationCenterAction::SubmitPopupReply(id)) => Some(id.clone()),
            _ => None,
        };
        if let Some(id) = submit_id {
            if composing { return; }
            let draft = owner.update(ctx, |view, owner_ctx| {
                let draft = view.take_popup_reply_draft(&id);
                owner_ctx.notify();
                draft
            });
            if let Some(draft) = draft {
                if self.tx.send(PopupAction {
                    action: Action::NotificationCenter(NotificationCenterAction::SubmitPopupReply(id.clone())),
                    item_id: Some(id.clone()), reply_draft: Some(draft.clone()),
                }).is_err() {
                    owner.update(ctx, |view, owner_ctx| {
                        view.desktop_popup.pending_reply_item = None;
                        view.restore_popup_reply_draft(&id, &draft);
                        owner_ctx.notify();
                    });
                }
            }
            ctx.report_active_cursor_position_update();
            ctx.notify();
            return;
        }
        if matches!(action, Action::SubmitFocused | Action::NotificationCenter(NotificationCenterAction::SubmitPopupReply(_))) {
            return;
        }
        let _ = self.tx.send(PopupAction { action: action.clone(), item_id, reply_draft: None });
        ctx.notify();
    }
}

impl View for DesktopPopupWindowView {
    fn ui_name() -> &'static str {
        "DesktopPopupWindowView"
    }

    fn active_cursor_position(&self, ctx: &ViewContext<Self>) -> Option<facloud_ui::CursorInfo> {
        let owner = self.companion.upgrade(ctx)?;
        if !owner.read(ctx, |view, _| view.desktop_popup.reply_focused) {
            return None;
        }
        Some(facloud_ui::CursorInfo {
            // Position caches are native-window-local. Never use the main
            // Companion view's cursor when the popup owns the IME session.
            position: ctx.element_position_by_id(format!("input-caret:{F_TOAST_REPLY}"))?,
            font_size: theme::BODY,
        })
    }

    fn on_window_closed(&mut self, _: &mut ViewContext<Self>) {
        let _ = self.tx.send(PopupAction {
            action: Action::NotificationCenter(NotificationCenterAction::DismissPopup),
            item_id: None,
            reply_draft: None,
        });
    }

    fn render(&self, app: &AppContext) -> El {
        self.companion
            .upgrade(app)
            .map(|owner| {
                owner.read(app, |view, _| {
                    let Some(card) = view.render_360_popup_card_for_window(true) else {
                        return Empty::new().finish();
                    };
                    let tree = Flex::column()
                        .with_main_axis_size(MainAxisSize::Max)
                        .with_main_axis_alignment(MainAxisAlignment::Start)
                        .with_cross_axis_alignment(CrossAxisAlignment::Stretch)
                        .with_child(Expanded::new(1., card).finish())
                        .finish();
                    let stack = Stack::new().with_child(tree).finish();
                    Box::new(super::keyboard_input::Keyboard {
                        search: false,
                        context_menu: false,
                        interaction: false,
                        shortcut: view.settings.value.compact_shortcut.clone(),
                        recording_shortcut: false,
                        child: stack,
                        active: view.desktop_popup.reply_focused,
                        multiline: false,
                        accept_attachments: false,
                        composing: !view.fields[F_TOAST_REPLY].preedit.is_empty(),
                    }) as El
                })
            })
            .unwrap_or_else(|| Empty::new().finish())
    }
}

struct PopupWindowRoot {
    child: ViewHandle<DesktopPopupWindowView>,
}

impl Entity for PopupWindowRoot {
    type Event = ();
}

impl TypedActionView for PopupWindowRoot {
    type Action = ();
}

impl View for PopupWindowRoot {
    fn ui_name() -> &'static str {
        "PopupWindowRoot"
    }

    fn render(&self, _: &AppContext) -> El {
        ChildView::new(&self.child).finish()
    }
}

struct PopupWindowHandle {
    id: WindowId,
    view: WeakViewHandle<DesktopPopupWindowView>,
    rx: mpsc::Receiver<PopupAction>,
    visible: bool,
    last_bounds: RectF,
}

#[derive(Default)]
pub struct DesktopPopupController {
    window: Option<PopupWindowHandle>,
    pub(crate) main_window_active: bool,
    pub(crate) reply_focused: bool,
    pending_reply_item: Option<String>,
}

impl DesktopPopupController {
    pub fn is_created(&self) -> bool {
        self.window.is_some()
    }

    pub fn is_visible(&self) -> bool {
        self.window.as_ref().is_some_and(|w| w.visible)
    }

    pub fn bounds_array(&self) -> [f32; 4] {
        self.window
            .as_ref()
            .map(|w| {
                [
                    w.last_bounds.origin().x(),
                    w.last_bounds.origin().y(),
                    w.last_bounds.width(),
                    w.last_bounds.height(),
                ]
            })
            .unwrap_or([0.0, 0.0, 0.0, 0.0])
    }

    pub fn window_id(&self) -> Option<WindowId> {
        self.window.as_ref().map(|w| w.id)
    }

    pub fn element_position_by_id(&self, ctx: &AppContext, id: &str) -> Option<RectF> {
        let handle = self.window.as_ref().filter(|w| w.visible)?;
        ctx.element_position_by_id_at_last_frame(handle.id, id)
    }

    pub fn notify(&self, ctx: &mut AppContext) {
        if let Some(handle) = self.window.as_ref().filter(|w| w.visible) {
            if let Some(view) = handle.view.upgrade(ctx) {
                view.update(ctx, |_, vctx| vctx.notify());
            }
        }
    }
}

impl Companion {
    fn take_popup_reply_draft(&mut self, item_id: &str) -> Option<String> {
        if !self.popup_editor_target_is_current(Some(item_id))
            || self.desktop_popup.pending_reply_item.is_some()
            || !self.fields[F_TOAST_REPLY].preedit.is_empty()
            || self.fields[F_TOAST_REPLY].text.trim().is_empty()
        { return None; }
        let draft = self.fields[F_TOAST_REPLY].text.clone();
        self.fields[F_TOAST_REPLY].clear();
        self.notification_center.popup_reply_drafts.remove(item_id);
        self.desktop_popup.pending_reply_item = Some(item_id.to_string());
        Some(draft)
    }

    fn restore_popup_reply_draft(&mut self, item_id: &str, submitted: &str) {
        if self.notification_center.popup_reply_item_id.as_deref() == Some(item_id) {
            let field = &mut self.fields[F_TOAST_REPLY];
            let prefix = if field.text.is_empty() { submitted.to_string() } else { format!("{submitted}\n") };
            // Preserve text/IME typed after Enter, including its caret/selection.
            field.text.insert_str(0, &prefix);
            field.cursor += prefix.len();
            field.anchor = field.anchor.map(|anchor| anchor + prefix.len());
        } else {
            let next = self.notification_center.popup_reply_drafts.entry(item_id.to_string()).or_default();
            *next = if next.is_empty() { submitted.to_string() } else { format!("{submitted}\n{next}") };
        }
    }

    fn submit_popup_reply_draft(&mut self, item_id: &str, submitted: &str) {
        self.desktop_popup.pending_reply_item = None;
        self.error = false;
        self.submit_popup_quick_reply(item_id, submitted);
        if self.error {
            self.restore_popup_reply_draft(item_id, submitted);
        }
        // Successful dispatch may move to another card. Its existing draft
        // transition preserves the new input; never clear that field/map here.
    }

    fn apply_popup_copy_action(&mut self, action: &Action, ctx: &mut ViewContext<Self>) -> bool {
        let Action::CopyText(text, message) = action else { return false; };
        // A popup copy is local to this window, including while the main window
        // owns a confirmation. Do not pass it through the main interaction gate.
        ctx.clipboard().write(facloud_ui::clipboard::ClipboardContent::plain_text(text.clone()));
        self.status = message.clone();
        self.error = false;
        self.notification_center.popup_feedback_text =
            Some((message.clone(), chrono::Local::now().timestamp_millis() + 5_000));
        ctx.notify();
        true
    }

    fn popup_editor_target_is_current(&self, item_id: Option<&str>) -> bool {
        item_id.is_some() && item_id == self.notification_center.popup_reply_item_id.as_deref()
            && item_id == self.notification_center.active_popup_id.as_deref()
    }

    fn apply_popup_editor_action(
        &mut self,
        item_id: Option<&str>,
        action: &Action,
        ctx: &mut ViewContext<Self>,
    ) {
        if !self.popup_editor_target_is_current(item_id) { return; }
        match action {
            Action::Focus(F_TOAST_REPLY) => {
                if !self.desktop_popup.reply_focused {
                    let field = &mut self.fields[F_TOAST_REPLY];
                    field.cursor = field.text.len();
                    field.all = false;
                    field.anchor = None;
                }
                self.desktop_popup.reply_focused = true;
            }
            Action::InputPointer(F_TOAST_REPLY, position, extend, clicks) => {
                self.fields[F_TOAST_REPLY].place_pointer(*position, *extend, *clicks);
                self.desktop_popup.reply_focused = true;
            }
            Action::InputPointerUp => self.fields[F_TOAST_REPLY].dragging = false,
            Action::ExtendSelection(direction) if self.desktop_popup.reply_focused => {
                self.fields[F_TOAST_REPLY].extend(*direction);
            }
            _ if self.desktop_popup.reply_focused => self.edit_field(F_TOAST_REPLY, action, ctx),
            _ => {}
        }
        // Only the native popup may focus itself or update its IME session.
        ctx.notify();
    }

    pub(crate) fn compute_desktop_popup_bounds(workarea: RectF, full_reply_mode: bool) -> RectF {
        let (width, height) = if full_reply_mode {
            (POPUP_WIN_W_FULL_REPLY as f32, POPUP_WIN_H_FULL_REPLY as f32)
        } else {
            (POPUP_WIN_W_BANNER as f32, POPUP_WIN_H_BANNER as f32)
        };
        let effective_workarea = if workarea.width() >= 504.0 && workarea.height() >= 260.0 {
            workarea
        } else {
            RectF::new(vec2f(0.0, 0.0), vec2f(1920.0, 1080.0))
        };
        let ox = effective_workarea.origin().x();
        let oy = effective_workarea.origin().y();
        let x = (ox + effective_workarea.width() - width - 16.0).max(ox + 8.0);
        let y = (oy + effective_workarea.height() - height - 16.0).max(oy + 8.0);
        RectF::new(vec2f(x, y), vec2f(width, height))
    }

    pub(crate) fn sync_desktop_popup_window(&mut self, ctx: &mut ViewContext<Self>) {
        self.desktop_popup.main_window_active =
            !self.settings.hidden && ctx.windows().active_window() == Some(ctx.window_id());
        // Clicking back into another window ends an empty reply session. A
        // stale editor flag must not pin an old card over later verification codes.
        if self.desktop_popup.reply_focused && self.fields[F_TOAST_REPLY].preedit.is_empty() {
            let unfocused = self.desktop_popup.window.as_ref().is_some_and(|popup| {
                #[cfg(any(target_os = "linux", target_os = "windows"))]
                { facloud_ui::windowing::winit::winit_window_for_id(ctx, popup.id).is_some_and(|window| !window.has_focus()) }
                #[cfg(not(any(target_os = "linux", target_os = "windows")))]
                { ctx.windows().active_window() != Some(popup.id) }
            });
            if unfocused { self.desktop_popup.reply_focused = false; }
        }
        let pending_actions: Vec<PopupAction> = self
            .desktop_popup
            .window
            .as_ref()
            .map(|w| w.rx.try_iter().collect())
            .unwrap_or_default();
        for PopupAction { action: act, item_id, reply_draft } in pending_actions {
            if !popup_window_action(&act) { continue; }
            if let (Some(id), Some(draft)) = (item_id.as_deref(), reply_draft.as_deref()) {
                self.submit_popup_reply_draft(id, draft);
                ctx.notify();
                continue;
            }
            if self.apply_popup_copy_action(&act, ctx) { continue; }
            // This is a separate native window. A main-window confirmation must
            // not swallow its close/pager/reply controls or approve that review.
            if let Action::NotificationCenter(ref action) = act {
                if matches!(
                    action,
                    NotificationCenterAction::DismissPopup
                        | NotificationCenterAction::ClearPopupBatch { .. }
                        | NotificationCenterAction::PopupPrev
                        | NotificationCenterAction::PopupNext
                        | NotificationCenterAction::PopupHoverChanged(_)
                        | NotificationCenterAction::SubmitPopupReply(_)
                        | NotificationCenterAction::PopupMenu(_)
                        | NotificationCenterAction::MutePopupApp(_)
                        | NotificationCenterAction::SnoozePopups { .. }
                        | NotificationCenterAction::SnoozeToday
                ) {
                    self.apply_notification_center_action(action.clone());
                    ctx.notify();
                    continue;
                }
            }
            if popup_editor_action(&act) {
                self.apply_popup_editor_action(item_id.as_deref(), &act, ctx);
                continue;
            }
            if matches!(act, Action::Escape) {
                self.apply_notification_center_action(
                    if self.notification_center.popup_menu_item_id.is_some() {
                        NotificationCenterAction::PopupMenu(None)
                    } else {
                        NotificationCenterAction::DismissPopup
                    },
                );
                ctx.notify();
                continue;
            }
            if (self.interaction.review.is_some() || self.interaction.history_open)
                && matches!(act,
                    Action::NotificationCenter(NotificationCenterAction::OpenPopupDetails(_))
                        | Action::Studio(studio_state::StudioAction::OpenPath(_)))
            {
                // Opening the main view keeps its outstanding confirmation intact.
                self.settings.hidden = false;
                ctx.windows().show_window_and_focus_app(ctx.window_id());
            }
            self.handle_action(&act, ctx);
        }

        let active_item =
            if self.notification_center.drawer_open && self.desktop_popup.main_window_active
                && !self.notification_active_popup().is_some_and(|item| matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. })) {
                None
            } else {
                self.notification_active_popup().cloned()
            };

        let Some(item) = active_item else {
            // An activated native window can regain OS focus when mapped again,
            // even through show_without_focus. End its lifetime when the queue
            // hides; the next alert gets a fresh nonactivating window. Keep the
            // same window throughout a visible burst so typing and bounds stay put.
            self.close_desktop_popup_window(ctx);
            return;
        };

        let mode = self.push.prefs.effective_popup_mode(&item.package_name);
        if mode == stepx_companion::desktop_notify::PopupMode::Silent {
            self.close_desktop_popup_window(ctx);
            return;
        }

        let is_full_reply = mode == stepx_companion::desktop_notify::PopupMode::FullReply
            || matches!(item.popup_widget, NotificationPopupWidget::LiveCall { .. });
        let raw_workarea = ctx.windows().active_display_workarea_bounds();
        let target_bounds = Self::compute_desktop_popup_bounds(raw_workarea, is_full_reply);
        // Native geometry is policy-based, never measured from the last frame.
        // Text wrapping and late images cannot feed back into window resizing.
        // The menu scrolls inside this same card; opening it must neither move
        // nor enlarge the native window beside another application's editor.

        if let Some(existing) = self.desktop_popup.window.as_mut() {
            if let Some(view) = existing.view.upgrade(ctx) {
                let size_changed =
                    (existing.last_bounds.size() - target_bounds.size()).square_length() > 0.5;
                let preserved_bounds = if size_changed {
                    target_bounds
                } else {
                    let current_origin = ctx
                        .windows()
                        .platform_window(existing.id)
                        .map(|pw| pw.origin())
                        .filter(|o| {
                            o.x().is_finite() && o.y().is_finite() && (o.x() != 0.0 || o.y() != 0.0)
                        })
                        .unwrap_or_else(|| existing.last_bounds.origin());
                    RectF::new(current_origin, target_bounds.size())
                };
                if size_changed {
                    ctx.windows()
                        .set_window_bounds(existing.id, preserved_bounds);
                }
                existing.last_bounds = preserved_bounds;
                if !existing.visible {
                    ctx.windows().show_window_without_focus(existing.id);
                    existing.visible = true;
                }
                view.update(ctx, |_, vctx| vctx.notify());
                return;
            }
            self.desktop_popup.window = None;
        }

        let owner = ctx.handle();
        let (tx, rx) = mpsc::channel();
        let (id, root_view) = ctx.add_window(
            facloud_ui::AddWindowOptions {
                title: Some("StepX 伴侣提醒".into()),
                window_style: platform::WindowStyle::PopupToast,
                window_bounds: WindowBounds::ExactPosition(target_bounds),
                min_inner_size: Some(vec2f(320.0, 84.0)),
                skip_taskbar: true,
                focus_on_show: false,
                ..Default::default()
            },
            move |ctx| PopupWindowRoot {
                child: ctx.add_typed_action_view(move |_| DesktopPopupWindowView {
                    companion: owner,
                    tx,
                }),
            },
        );
        let popup_view = root_view.read(ctx, |root, _| root.child.downgrade());
        ctx.windows().show_window_without_focus(id);
        self.desktop_popup.window = Some(PopupWindowHandle {
            id,
            view: popup_view,
            rx,
            visible: true,
            last_bounds: target_bounds,
        });
    }

    pub(crate) fn close_desktop_popup_window(&mut self, ctx: &mut ViewContext<Self>) {
        self.notification_center.popup_hovered = false;
        if let Some(mut handle) = self.desktop_popup.window.take() {
            if handle.visible {
                ctx.windows().hide_window(handle.id);
                handle.visible = false;
            }
            ctx.windows()
                .close_window(handle.id, platform::TerminationMode::Cancellable);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ui::notification_center_state::UnifiedNotificationItem;

    #[test]
    fn popup_submit_snapshots_enter_and_button_drafts_and_preserves_later_input() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(platform::WindowStyle::NotStealFocus, |_| Companion::new_for_test());
            let (tx, rx) = mpsc::channel();
            let weak_owner = owner.downgrade();
            let (_, popup) = app.add_window(platform::WindowStyle::PopupToast, |_| {
                DesktopPopupWindowView { companion: weak_owner, tx }
            });
            for button in [false, true] {
                for fail in [false, true] {
                    let id = "im-thread:wx:snapshot";
                    owner.update(&mut app, |view, _| {
                        // Successful cases are entirely local demo sends; failure
                        // cases are disconnected before any phone I/O is possible.
                        view.demo_mode = !fail;
                        view.selected = None;
                        view.push.snapshot.threads = vec![stepx_companion::push::ImThread {
                            thread_id: "wx:snapshot".into(), package_name: "com.tencent.mm".into(),
                            title: "测试联系人".into(), unread_count: 1, can_remote_reply: true,
                            last_message: "本地测试".into(), last_timestamp_ms: chrono::Local::now().timestamp_millis(),
                            ..Default::default()
                        }];
                        let mut item = fixture();
                        item.id = id.into();
                        item.popup_widget = NotificationPopupWidget::InlineImReply {
                            thread_id: "wx:snapshot".into(), package_name: "com.tencent.mm".into(),
                            can_reply: true, quick_phrases: Vec::new(),
                        };
                        view.notification_center.items = vec![item];
                        view.notification_center.active_popup_id = Some(id.into());
                        view.notification_center.popup_reply_item_id = Some(id.into());
                        view.notification_center.popup_reply_drafts.clear();
                        view.desktop_popup.reply_focused = true;
                        view.fields[F_TOAST_REPLY].set("第一条");
                    });
                    let action = if button {
                        Action::NotificationCenter(NotificationCenterAction::SubmitPopupReply(id.into()))
                    } else { Action::SubmitFocused };
                    popup.update(&mut app, |view, ctx| view.handle_action(&action, ctx));
                    owner.update(&mut app, |view, _| assert!(view.fields[F_TOAST_REPLY].text.is_empty()));
                    popup.update(&mut app, |view, ctx| view.handle_action(&Action::Insert("第二条".into()), ctx));
                    // A second Enter before the first queued dispatch must leave
                    // the next draft intact instead of submitting/clearing it.
                    popup.update(&mut app, |view, ctx| view.handle_action(&action, ctx));
                    popup.update(&mut app, |view, ctx| view.handle_action(&Action::Preedit("zhong".into()), ctx));
                    let queued = rx.try_recv().expect("one captured reply");
                    assert!(rx.try_recv().is_err());
                    assert_eq!(queued.reply_draft.as_deref(), Some("第一条"));
                    owner.update(&mut app, |view, _| {
                        view.submit_popup_reply_draft(queued.item_id.as_deref().unwrap(), queued.reply_draft.as_deref().unwrap());
                        let sent: Vec<_> = view.push.snapshot.threads.iter().flat_map(|thread| &thread.messages)
                            .filter(|message| message.outgoing).map(|message| message.text.as_str()).collect();
                        if fail {
                            assert!(view.error && sent.is_empty());
                            assert_eq!(view.fields[F_TOAST_REPLY].text, "第一条\n第二条");
                            assert_eq!(view.fields[F_TOAST_REPLY].preedit, "zhong");
                        } else {
                            assert_eq!(sent, vec!["第一条"]);
                            let remaining = if view.notification_center.popup_reply_item_id.as_deref() == Some(id) {
                                view.fields[F_TOAST_REPLY].text.as_str()
                            } else { view.notification_center.popup_reply_drafts.get(id).unwrap().as_str() };
                            assert_eq!(remaining, "第二条");
                        }
                        assert!(view.desktop_popup.pending_reply_item.is_none());
                        assert!(view.push.active_reply_job.is_none() && view.push.reply_queue.is_empty());
                    });
                }
            }
        });
    }

    #[test]
    fn popup_copy_preserves_main_modal_focus_and_composition() {
        use facloud_ui::clipboard::ClipboardContent;
        facloud_ui::App::test((), |mut app| async move {
            let (main_id, owner) = app.add_window(platform::WindowStyle::NotStealFocus, |_| {
                Companion::new_for_test()
            });
            let (tx, rx) = mpsc::channel();
            let weak_owner = owner.downgrade();
            let (popup_id, popup) = app.add_window(platform::WindowStyle::PopupToast, |_| {
                DesktopPopupWindowView { companion: weak_owner, tx }
            });
            owner.update(&mut app, |view, ctx| {
                let item = fixture();
                view.notification_center.active_popup_id = Some(item.id.clone());
                view.notification_center.items = vec![item];
                view.desktop_popup.window = Some(PopupWindowHandle {
                    id: popup_id, view: popup.downgrade(), rx, visible: true,
                    last_bounds: RectF::new(vec2f(0., 0.), vec2f(380., 194.)),
                });
                ctx.focus_self();
            });
            popup.update(&mut app, |_, ctx| ctx.focus_self());
            for history_open in [false, true] {
                owner.update(&mut app, |view, ctx| {
                    view.settings.hidden = true;
                    view.focus = Some(F_AMOO);
                    view.fields[F_AMOO].set("主窗口草稿");
                    view.fields[F_AMOO].preedit = "zhong".into();
                    view.interaction.history_open = history_open;
                    view.interaction.review = (!history_open).then(|| interaction::Review {
                        title: "仍待确认".into(), target: String::new(), details: Vec::new(),
                        accept: "确认".into(), action: Action::Trash, serial: None,
                        discard: false, context: String::new(),
                    });
                    ctx.clipboard().write(ClipboardContent::plain_text("原剪贴板".into()));
                });
                popup.update(&mut app, |view, ctx| view.handle_action(
                    &Action::CopyText("123456".into(), "已复制验证码".into()), ctx,
                ));
                owner.update(&mut app, |view, ctx| {
                    assert_eq!(ctx.clipboard().read().plain_text, "123456");
                    assert!(view.desktop_popup.window.as_ref().unwrap().rx.try_recv().is_err());
                    assert_eq!(view.notification_center.popup_feedback_text.as_ref().unwrap().0, "已复制验证码");
                });
                // A blocked non-opening action must not reveal/focus the main
                // window merely because its confirmation/history is present.
                popup.update(&mut app, |view, ctx| view.handle_action(
                    &Action::VoiceInput(voice_input_state::VoiceInputAction::Inspect(F_TOAST_REPLY)), ctx,
                ));
                owner.update(&mut app, |view, ctx| {
                    view.sync_desktop_popup_window(ctx);
                    assert!(view.settings.hidden);
                    assert_eq!(view.focus, Some(F_AMOO));
                    assert_eq!(view.fields[F_AMOO].text, "主窗口草稿");
                    assert_eq!(view.fields[F_AMOO].preedit, "zhong");
                    assert_eq!(view.interaction.history_open, history_open);
                    assert_eq!(view.interaction.review.is_some(), !history_open);
                });
                assert_eq!(app.focused_view_id(main_id), Some(owner.id()));
                assert_eq!(app.focused_view_id(popup_id), Some(popup.id()));
            }
        });
    }

    #[test]
    fn popup_menu_and_clear_preserve_native_bounds_main_draft_and_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let _guard = TestUiConfigDirGuard::set(dir.path());
        facloud_ui::App::test((), |mut app| async move {
            for mode in [
                stepx_companion::desktop_notify::PopupMode::Banner,
                stepx_companion::desktop_notify::PopupMode::FullReply,
            ] {
                let (main_id, owner) = app.add_window(platform::WindowStyle::NotStealFocus, |_| Companion::new_for_test());
                let (popup_id, popup, before) = owner.update(&mut app, |view, ctx| {
                    view.notification_center = notification_center_state::NotificationCenterState::new_empty();
                    view.push.prefs.popup_mode = mode;
                    view.focus = Some(F_AMOO);
                    view.fields[F_AMOO].set("主窗口草稿");
                    view.fields[F_AMOO].preedit = "zhong".into();
                    view.interaction.review = Some(interaction::Review {
                        title: "仍待确认".into(), target: String::new(), details: Vec::new(),
                        accept: "确认".into(), action: Action::Trash, serial: None,
                        discard: false, context: String::new(),
                    });
                    let item = fixture();
                    view.notification_center.popup_queue.push_back(item.id.clone());
                    view.notification_center.active_popup_id = Some(item.id.clone());
                    view.notification_center.items = vec![item];
                    ctx.focus_self();
                    view.sync_desktop_popup_window(ctx);
                    let window = view.desktop_popup.window.as_ref().unwrap();
                    (window.id, window.view.upgrade(ctx).unwrap(), window.last_bounds)
                });
                let initial_popup_focus = app.focused_view_id(popup_id);
                popup.update(&mut app, |view, ctx| view.handle_action(
                    &Action::NotificationCenter(NotificationCenterAction::PopupMenu(Some(fixture().id))), ctx));
                owner.update(&mut app, |view, ctx| {
                    view.sync_desktop_popup_window(ctx);
                    let window = view.desktop_popup.window.as_ref().unwrap();
                    assert_eq!(window.id, popup_id);
                    assert_eq!(window.last_bounds.size(), before.size());
                    assert!(view.notification_center.popup_menu_item_id.is_some());
                });
                assert_eq!(app.focused_view_id(main_id), Some(owner.id()));
                assert_eq!(app.focused_view_id(popup_id), initial_popup_focus, "menu must not claim text focus");
                let receipts = owner.read(&app, |view, _| view.popup_batch_receipts());
                popup.update(&mut app, |view, ctx| view.handle_action(
                    &Action::NotificationCenter(NotificationCenterAction::ClearPopupBatch { receipts }), ctx));
                owner.update(&mut app, |view, ctx| {
                    view.sync_desktop_popup_window(ctx);
                    assert!(!view.desktop_popup.is_created());
                    assert!(view.notification_center.popup_queue.is_empty());
                    assert_eq!(view.notification_center.items.len(), 1);
                    assert_eq!(view.focus, Some(F_AMOO));
                    assert_eq!(view.fields[F_AMOO].text, "主窗口草稿");
                    assert_eq!(view.fields[F_AMOO].preedit, "zhong");
                    assert!(view.interaction.review.is_some(), "clear must not accept the main-window confirmation");
                });
                assert_eq!(app.focused_view_id(main_id), Some(owner.id()));
            }
        });
    }

    #[test]
    fn hidden_popup_retires_its_native_focus_lifetime_before_the_next_alert() {
        facloud_ui::App::test((), |mut app| async move {
            let (_, owner) = app.add_window(platform::WindowStyle::NotStealFocus, |_| Companion::new_for_test());
            owner.update(&mut app, |view, ctx| {
                view.push.prefs.popup_mode = stepx_companion::desktop_notify::PopupMode::FullReply;
                view.notification_center.alert_mode = NotificationAlertMode::PopupAndBadge;
                let item = fixture();
                view.notification_center.active_popup_id = Some(item.id.clone());
                view.notification_center.items = vec![item];
                view.sync_desktop_popup_window(ctx);
                let first = view.desktop_popup.window_id().unwrap();
                view.notification_center.active_popup_id = None;
                view.sync_desktop_popup_window(ctx);
                assert!(!view.desktop_popup.is_created());
                view.notification_center.active_popup_id = Some(fixture().id);
                view.sync_desktop_popup_window(ctx);
                assert_ne!(view.desktop_popup.window_id().unwrap(), first);
            });
        });
    }

    #[test]
    fn native_popup_edits_and_clipboard_apply_before_the_main_timer() {
        use facloud_ui::clipboard::ClipboardContent;
        facloud_ui::App::test((), |mut app| async move {
            let (main_id, owner) = app.add_window(platform::WindowStyle::NotStealFocus, |_| {
                Companion::new_for_test()
            });
            owner.update(&mut app, |view, ctx| {
                view.focus = Some(F_AMOO);
                view.fields[F_AMOO].set("主窗口草稿");
                view.fields[F_AMOO].preedit = "zhong".into();
                view.push.prefs.popup_mode = stepx_companion::desktop_notify::PopupMode::FullReply;
                view.notification_center.alert_mode = NotificationAlertMode::PopupAndBadge;
                let item = fixture();
                view.notification_center.active_popup_id = Some(item.id.clone());
                view.notification_center.popup_reply_item_id = Some(item.id.clone());
                view.notification_center.items = vec![item];
                ctx.focus_self();
                ctx.clipboard().write(ClipboardContent::plain_text("粘贴内容".into()));
            });
            let (tx, rx) = mpsc::channel();
            let weak_owner = owner.downgrade();
            let (popup_id, popup) = app.add_window(platform::WindowStyle::PopupToast, |_| {
                DesktopPopupWindowView { companion: weak_owner, tx }
            });
            for action in [
                Action::Focus(F_TOAST_REPLY),
                Action::Paste,
                Action::Preedit("ni".into()),
                Action::Insert("你好".into()),
                Action::SelectAll,
                Action::CopyInput,
            ] {
                popup.update(&mut app, |view, ctx| view.handle_action(&action, ctx));
            }
            for action in [
                Action::NextField(false),
                Action::NextField(true),
                Action::CompactChat,
                Action::FocusSearch,
                Action::GlobalSearch(SearchAction::Open),
                Action::Page(Page::Files),
                Action::ActivateSelectedFile,
                Action::Trash,
            ] {
                popup.update(&mut app, |view, ctx| view.handle_action(&action, ctx));
            }
            // No tick or queued-action drain has run. The first paste and IME
            // commit must already be visible, and Copy must already be usable.
            assert!(rx.try_recv().is_err());
            owner.update(&mut app, |view, ctx| {
                assert!(view.desktop_popup.reply_focused);
                assert_eq!(view.fields[F_TOAST_REPLY].text, "粘贴内容你好");
                assert!(view.fields[F_TOAST_REPLY].preedit.is_empty());
                assert_eq!(ctx.clipboard().read().plain_text, "粘贴内容你好");
                assert_eq!(view.focus, Some(F_AMOO));
                assert_eq!(view.fields[F_AMOO].text, "主窗口草稿");
                assert_eq!(view.fields[F_AMOO].preedit, "zhong");
            });
            assert_eq!(app.focused_view_id(main_id), Some(owner.id()));
            assert_eq!(app.focused_view_id(popup_id), Some(popup.id()));
        });
    }

    #[test]
    fn popup_keyboard_scope_keeps_explicit_controls_and_blocks_main_shortcuts() {
        for action in [
            Action::NextField(false), Action::CompactChat, Action::FocusSearch,
            Action::GlobalSearch(SearchAction::Open), Action::Page(Page::Files),
            Action::ParentDirectory, Action::Trash, Action::OpenRename,
            Action::ActivateSelectedFile, Action::Mirror, Action::ContextRefresh,
        ] {
            assert!(!popup_window_action(&action));
        }
        for action in [
            Action::Escape, Action::Paste, Action::CopyInput, Action::CutInput,
            Action::SubmitFocused,
            Action::NotificationCenter(NotificationCenterAction::OpenPopupDetails("item".into())),
            Action::NotificationCenter(NotificationCenterAction::PopupNext),
            Action::NotificationCenter(NotificationCenterAction::ClearPopupBatch { receipts: Vec::new() }),
            Action::NotificationCenter(NotificationCenterAction::SubmitPopupReply("item".into())),
            Action::CopyText("123456".into(), "已复制验证码".into()),
        ] {
            assert!(popup_window_action(&action));
        }
    }

    #[test]
    fn delayed_popup_edits_cannot_target_a_different_notification() {
        let mut app = Companion::new_for_test();
        app.focus = Some(F_AMOO);
        app.fields[F_AMOO].set("主窗口草稿");
        app.fields[F_AMOO].preedit = "zhong".into();
        app.notification_center.active_popup_id = Some("new".into());
        app.notification_center.popup_reply_item_id = Some("new".into());
        assert!(!app.popup_editor_target_is_current(Some("old")));
        assert!(!app.popup_editor_target_is_current(None));
        assert!(app.popup_editor_target_is_current(Some("new")));
        app.desktop_popup.reply_focused = true;
        assert!(app.input_has_focus(F_TOAST_REPLY));
        assert!(app.input_has_focus(F_AMOO));
        assert_eq!(app.focus, Some(F_AMOO));
        assert_eq!(app.fields[F_AMOO].preedit, "zhong");
    }

    fn fixture() -> UnifiedNotificationItem {
        UnifiedNotificationItem {
            id: "desktop-popup-test".into(),
            category: NotificationCategory::PhoneComm,
            visual: NotificationVisualKind::WeChat,
            priority: NotificationPriority::P0Immediate,
            popup_widget: NotificationPopupWidget::None,
            package_name: "com.tencent.mm".into(),
            app_name: "微信".into(),
            sender: "测试联系人".into(),
            source_badge: "微信".into(),
            title: "测试提醒".into(),
            summary: "private message".into(),
            full_body: "private message".into(),
            banner_body: "收到新消息，正文已隐藏".into(),
            time_label: "刚刚".into(),
            sort_ts_ms: 123,
            unread: true,
            status_tag: None,
            primary_action: None,
        }
    }

    #[test]
    fn hidden_main_drawer_does_not_hide_persistent_desktop_popup() {
        let mut app = Companion::new_for_test();
        let item = fixture();
        app.notification_center.items = vec![item.clone()];
        app.notification_center
            .popup_queue
            .push_back(item.id.clone());
        app.notification_center.active_popup_id = Some(item.id);
        app.notification_center.popup_total_ms = 60_000;
        app.notification_center.popup_remaining_ms = 60_000;
        app.notification_center.drawer_open = true;
        app.desktop_popup.main_window_active = false;
        assert!(app.render_360_popup_card_for_window(true).is_some());
        assert!(!app.tick_notification_popup_timer(500));
        assert_eq!(app.notification_center.popup_remaining_ms, 60_000);
        app.desktop_popup.main_window_active = true;
        assert!(app.render_360_popup_card_for_window(true).is_none());
        assert!(!app.tick_notification_popup_timer(500));
    }

    #[test]
    fn bottom_right_popup_bounds_fit_inside_screen_workarea() {
        let workarea = RectF::new(vec2f(0.0, 28.0), vec2f(1920.0, 1020.0));
        let b_full = Companion::compute_desktop_popup_bounds(workarea, true);
        assert_eq!(b_full.width(), POPUP_WIN_W_FULL_REPLY as f32);
        assert_eq!(b_full.height(), POPUP_WIN_H_FULL_REPLY as f32);
        assert_eq!(
            b_full.origin().x(),
            1920.0 - POPUP_WIN_W_FULL_REPLY as f32 - 16.0
        );
        assert_eq!(
            b_full.origin().y(),
            28.0 + 1020.0 - POPUP_WIN_H_FULL_REPLY as f32 - 16.0
        );

        let b_banner = Companion::compute_desktop_popup_bounds(workarea, false);
        assert_eq!(b_banner.width(), POPUP_WIN_W_BANNER as f32);
        assert_eq!(b_banner.height(), POPUP_WIN_H_BANNER as f32);
        assert_eq!(
            b_banner.origin().x(),
            1920.0 - POPUP_WIN_W_BANNER as f32 - 16.0
        );
        assert_eq!(
            b_banner.origin().y(),
            28.0 + 1020.0 - POPUP_WIN_H_BANNER as f32 - 16.0
        );
    }
}
