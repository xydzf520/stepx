//! Keyboard routing, IME composition handling, and text-field editing state.

use super::*;
use facloud_ui::clipboard::ClipboardContent;

#[derive(Default)]
pub(super) struct TextField {
    pub(super) text: String,
    pub(super) cursor: usize,
    pub(super) all: bool,
    pub(super) preedit: String,
    pub(super) anchor: Option<usize>,
    pub(super) dragging: bool,
    pub(super) undo: Vec<(String, usize)>,
    pub(super) redo: Vec<(String, usize)>,
    pub(super) last_edit: Option<Instant>,
}

pub(super) type Input = TextField;

impl TextField {
    pub(super) fn clear(&mut self) {
        self.text.clear();
        self.cursor = 0;
        self.all = false;
        self.preedit.clear();
        self.anchor = None;
        self.dragging = false;
        self.undo.clear();
        self.redo.clear();
        self.last_edit = None;
    }

    pub(super) fn set(&mut self, text: &str) {
        self.clear();
        self.insert(text);
    }

    pub(super) fn insert(&mut self, text: &str) {
        self.delete_selection();
        let clean: String = text.chars().filter(|c| *c != '\0' && *c != '\r').collect();
        if self.text.len() + clean.len() > 16_384 {
            return;
        }
        self.text.insert_str(self.cursor, &clean);
        self.cursor = (self.cursor + clean.len()).min(self.text.len());
        self.preedit.clear();
    }

    pub(super) fn previous(&self) -> usize {
        self.text[..self.cursor.min(self.text.len())]
            .char_indices()
            .last()
            .map(|(i, _)| i)
            .unwrap_or(0)
    }

    pub(super) fn next(&self) -> usize {
        let cur = self.cursor.min(self.text.len());
        self.text[cur..]
            .chars()
            .next()
            .map(|c| cur + c.len_utf8())
            .unwrap_or(cur)
    }
}

impl Companion {
    pub(super) fn sync_ime(&self, ctx: &mut ViewContext<Self>) {
        #[cfg(any(target_os = "linux", target_os = "windows"))]
        if let Some(window) =
            facloud_ui::windowing::winit::winit_window_for_id(ctx, ctx.window_id())
        {
            window.set_ime_allowed(self.focus.is_some());
        }
        ctx.report_active_cursor_position_update();
    }

    pub(super) fn focus_field(&mut self, index: usize, ctx: &mut ViewContext<Self>) {
        if let Some(previous) = self.focus.filter(|previous| *previous != index) {
            self.fields[previous].preedit.clear();
        }
        self.text_selection.clear();
        self.selected_text = None;
        self.fields[index].anchor = None;
        self.focus = Some(index);
        self.fields[index].cursor = self.fields[index].text.len();
        self.fields[index].all = false;
        if index == F_AMOO {
            Self::scroll_to_end(&self.amoo_ui.composer_scroll);
        }
        ctx.focus_self();
    }

    pub(super) fn edit_focused(&mut self, action: &Action, ctx: &mut ViewContext<Self>) {
        let Some(i) = self.focus else {
            return;
        };
        let input = &mut self.fields[i];
        if matches!(
            action,
            Action::Insert(_)
                | Action::Paste
                | Action::CutInput
                | Action::Backspace
                | Action::BackspaceLine
                | Action::Delete
        ) {
            input.checkpoint(matches!(action, Action::Insert(_)));
        }
        match action {
            Action::UndoInput => input.restore_edit(false),
            Action::RedoInput => input.restore_edit(true),
            Action::Insert(s) => input.insert(s),
            Action::Preedit(s) => input.preedit = s.clone(),
            Action::Paste => input.insert(&ctx.clipboard().read().plain_text),
            Action::SelectAll => {
                input.all = true;
                input.anchor = Some(0);
                input.cursor = input.text.len();
            }
            Action::CopyInput | Action::CutInput => {
                if i != F_LAB_KEY {
                    if let Some(range) = input.selection_range().filter(|r| !r.is_empty()) {
                        ctx.clipboard()
                            .write(ClipboardContent::plain_text(input.text[range].to_owned()));
                        if matches!(action, Action::CutInput) {
                            input.delete_selection();
                        }
                    }
                }
            }
            Action::Backspace => {
                if input.selection_range().is_some() {
                    input.delete_selection();
                } else {
                    let p = input.previous();
                    input.text.replace_range(p..input.cursor, "");
                    input.cursor = p;
                }
            }
            Action::BackspaceLine => {
                if input.selection_range().is_some() {
                    input.delete_selection();
                } else {
                    input.text.replace_range(0..input.cursor, "");
                    input.cursor = 0;
                }
            }
            Action::Delete => {
                if input.selection_range().is_some() {
                    input.delete_selection();
                } else {
                    let p = input.next();
                    input.text.replace_range(input.cursor..p, "");
                }
            }
            Action::Left => {
                input.cursor = input
                    .selection_range()
                    .map(|r| r.start)
                    .unwrap_or_else(|| input.previous());
                input.anchor = None;
                input.all = false;
            }
            Action::Right => {
                input.cursor = input
                    .selection_range()
                    .map(|r| r.end)
                    .unwrap_or_else(|| input.next());
                input.anchor = None;
                input.all = false;
            }
            Action::Home => {
                input.anchor = None;
                input.cursor = 0;
                input.all = false;
            }
            Action::End => {
                input.anchor = None;
                input.cursor = input.text.len();
                input.all = false;
            }
            _ => {}
        }
        if [F_SMS_TO, F_SMS_BODY].contains(&i) {
            self.pending_sms_plan = None;
        }
        // Copy and Select All must not move the text or its scroll viewport.
        if [F_AMOO, F_WORK_PROMPT].contains(&i)
            && !matches!(action, Action::CopyInput | Action::SelectAll)
        {
            let scroll = if i == F_AMOO {
                &self.amoo_ui.composer_scroll
            } else {
                &self.work.prompt_scroll
            };
            if self.fields[i].cursor == self.fields[i].text.len() {
                Self::scroll_to_end(scroll);
            } else if self.fields[i].cursor == 0 {
                scroll.scroll_to(facloud_ui::units::Pixels::new(0.));
            } else {
                scroll.scroll_to_position(ScrollTarget {
                    position_id: format!("input-caret:{i}"),
                    mode: ScrollToPositionMode::FullyIntoView,
                });
            }
        }
        if i == F_PHONE_DIAL {
            self.pending_call = None;
        }
        if i == F_FILE_SEARCH {
            self.file_page = 0;
        }
        if i == F_STORE_SEARCH {
            self.on_store_search_edited();
        }
    }
}

/// Handles committed IME text separately from key presses so Chinese input is neither lost
/// nor inserted twice, and maps Cmd (macOS) / Ctrl (Windows, Linux) shortcuts.
pub(super) struct Keyboard {
    pub(super) accept_attachments: bool,
    pub(super) context_menu: bool,
    pub(super) shortcut: stepx_companion::settings::CompactShortcut,
    pub(super) recording_shortcut: bool,
    pub(super) child: Box<dyn Element>,
    pub(super) active: bool,
    pub(super) multiline: bool,
    pub(super) composing: bool,
}

impl Element for Keyboard {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        self.child.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app);
    }
    fn paint(&mut self, o: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.child.paint(o, ctx, app);
        if self.active {
            app.report_active_cursor_position_update();
        }
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
        app: &AppContext,
    ) -> bool {
        #[cfg(feature = "ui-capture")]
        if std::env::var_os("STEPX_ENTER_TEST").is_some() {
            if let Event::KeyDown {
                keystroke,
                is_composing,
                ..
            } = event.raw_event()
            {
                eprintln!(
                    "ENTER_KEY key={} shift={} active={} composing={}/{}",
                    keystroke.key, keystroke.shift, self.active, self.composing, is_composing
                );
            }
            match event.raw_event() {
                Event::TypedCharacters { chars } => eprintln!(
                    "ENTER_TEXT bytes={} control={:?}",
                    chars.len(),
                    chars.chars().filter(|c| c.is_control()).collect::<Vec<_>>()
                ),
                Event::SetMarkedText { marked_text, .. } => {
                    eprintln!("ENTER_PREEDIT bytes={}", marked_text.len())
                }
                Event::ClearMarkedText => eprintln!("ENTER_PREEDIT clear"),
                _ => {}
            }
        }
        if self.context_menu {
            if let Event::KeyDown { keystroke, .. } = event.raw_event() {
                let a = match keystroke.key.as_str() {
                    "up" => Some(Action::ContextNavigate(-1)),
                    "down" => Some(Action::ContextNavigate(1)),
                    "enter" | "return" => Some(Action::ContextActivate),
                    "escape" | "tab" => Some(Action::ContextClose),
                    _ => None,
                };
                if let Some(a) = a {
                    ctx.dispatch_typed_action(a);
                    return true;
                }
            }
        }
        if let Event::KeyDown {
            keystroke,
            is_composing: false,
            ..
        } = event.raw_event()
        {
            if !self.composing {
                let action = if self.recording_shortcut {
                    Some(Action::Settings(if keystroke.key == "escape" {
                        SettingsAction::CancelShortcut
                    } else {
                        SettingsAction::CaptureShortcut(shortcuts::Shortcuts::captured(keystroke))
                    }))
                } else if shortcuts::Shortcuts::matches(&self.shortcut, keystroke) {
                    Some(Action::CompactChat)
                } else {
                    None
                };
                if let Some(action) = action {
                    ctx.dispatch_typed_action(action);
                    return true;
                }
            }
        }
        if self.recording_shortcut
            && matches!(
                event.raw_event(),
                Event::TypedCharacters { .. } | Event::SetMarkedText { .. }
            )
        {
            return true;
        }
        if self.child.dispatch_event(event, ctx, app) {
            return true;
        }
        let action = self.action_for_event(event.raw_event());
        if let Some(action) = action {
            ctx.dispatch_typed_action(action);
            true
        } else {
            false
        }
    }
}

impl Keyboard {
    pub(super) fn action_for_event(&self, event: &Event) -> Option<Action> {
        match event {
            Event::DragAndDropFiles { paths, .. } if self.accept_attachments => Some(
                Action::AmooDropFiles(paths.iter().map(PathBuf::from).collect()),
            ),
            Event::TypedCharacters { chars } if self.active => Some(Action::Insert(chars.clone())),
            Event::SetMarkedText { marked_text, .. } if self.active => {
                Some(Action::Preedit(marked_text.clone()))
            }
            Event::ClearMarkedText if self.active => Some(Action::Preedit(String::new())),
            Event::KeyDown {
                keystroke: k,
                is_composing: false,
                ..
            } if !self.composing => {
                let cmd = k.ctrl || k.cmd;
                let typing = self.active;
                match k.key.as_str() {
                    "1" if cmd => Some(Action::Page(Page::Overview)),
                    // Ctrl+2 始终打开唯一的 AI 入口「Amoo 对话」；文件处理（Page::Work）
                    // 与「我的作品」（Page::Studio）是次要入口，不分配数字快捷键。
                    "2" if cmd => Some(Action::Page(Page::Amoo)),
                    "3" if cmd => Some(Action::Page(Page::Sms)),
                    "4" if cmd => Some(Action::Page(Page::Files)),
                    "5" if cmd => Some(Action::Page(Page::Apps)),
                    "6" if cmd => Some(Action::Page(Page::Connect)),
                    "7" if cmd => Some(Action::Page(Page::Tools)),
                    "8" if cmd => Some(Action::Page(Page::BackScreen)),
                    "," if cmd => Some(Action::Page(Page::Settings)),
                    "k" if cmd => Some(Action::FocusSearch),
                    "r" if cmd => Some(Action::ContextRefresh),
                    "f5" => Some(Action::ContextRefresh),
                    "m" if cmd => Some(Action::Mirror),
                    "t" if cmd => Some(Action::CycleTheme),
                    "f" if cmd => Some(Action::FocusSearch),
                    "n" if cmd && !typing => Some(Action::OpenNewFolder),
                    "escape" => Some(Action::Escape),
                    "tab" => Some(Action::NextField(k.shift)),
                    "enter" | "return" if typing && k.shift && self.multiline => {
                        Some(Action::Insert("\n".into()))
                    }
                    "enter" | "return" if typing => Some(Action::SubmitFocused),
                    "enter" | "return" => Some(Action::ActivateSelectedFile),
                    "up" if !typing => Some(Action::MoveFileSelection(-1)),
                    "down" if !typing => Some(Action::MoveFileSelection(1)),
                    "left" if !typing => Some(Action::MoveHorizontalSelection(-1)),
                    "right" if !typing => Some(Action::MoveHorizontalSelection(1)),
                    "backspace" if !typing => Some(Action::ParentDirectory),
                    "delete" if !typing => Some(Action::Trash),
                    "f2" if !typing => Some(Action::OpenRename),
                    "z" if cmd && k.shift && typing => Some(Action::RedoInput),
                    "z" if cmd && typing => Some(Action::UndoInput),
                    "y" if cmd && typing => Some(Action::RedoInput),
                    "v" if cmd && typing => Some(Action::Paste),
                    "c" if cmd && typing => Some(Action::CopyInput),
                    "c" if cmd => Some(Action::Copy),
                    "x" if cmd && typing => Some(Action::CutInput),
                    "a" if cmd && typing => Some(Action::SelectAll),
                    "backspace" if cmd => Some(Action::BackspaceLine),
                    "backspace" => Some(Action::Backspace),
                    "delete" => Some(Action::Delete),
                    "left" if k.shift && typing => {
                        Some(Action::ExtendSelection(if cmd { -2 } else { -1 }))
                    }
                    "right" if k.shift && typing => {
                        Some(Action::ExtendSelection(if cmd { 2 } else { 1 }))
                    }
                    "home" if k.shift && typing => Some(Action::ExtendSelection(-2)),
                    "end" if k.shift && typing => Some(Action::ExtendSelection(2)),
                    "left" if k.cmd && typing => Some(Action::Home),
                    "right" if k.cmd && typing => Some(Action::End),
                    "left" if typing => Some(Action::Left),
                    "right" if typing => Some(Action::Right),
                    "home" if typing => Some(Action::Home),
                    "end" if typing => Some(Action::End),
                    _ => None,
                }
            }
            _ => None,
        }
    }
}

#[cfg(test)]
mod input_tests {
    use super::*;

    fn keyboard(composing: bool) -> Keyboard {
        Keyboard {
            accept_attachments: false,
            context_menu: false,
            child: Empty::new().finish(),
            shortcut: Default::default(),
            recording_shortcut: false,
            active: true,
            multiline: true,
            composing,
        }
    }

    fn key(name: &str, is_composing: bool) -> Event {
        Event::KeyDown {
            keystroke: facloud_ui::keymap::Keystroke {
                key: name.into(),
                ..Default::default()
            },
            chars: String::new(),
            details: Default::default(),
            is_composing,
        }
    }

    #[test]
    fn ime_candidate_keys_do_not_submit_or_edit_committed_text() {
        for name in [
            "enter",
            "return",
            "backspace",
            "left",
            "right",
            "escape",
            "tab",
        ] {
            assert!(keyboard(true).action_for_event(&key(name, false)).is_none());
            assert!(keyboard(false).action_for_event(&key(name, true)).is_none());
        }
        assert!(matches!(
            keyboard(false).action_for_event(&key("enter", false)),
            Some(Action::SubmitFocused)
        ));
    }

    #[test]
    fn enter_and_shift_enter_share_input_rules() {
        for name in ["enter", "return"] {
            let mut event = key(name, false);
            assert!(matches!(
                keyboard(false).action_for_event(&event),
                Some(Action::SubmitFocused)
            ));
            if let Event::KeyDown { keystroke, .. } = &mut event {
                keystroke.shift = true;
            }
            assert!(
                matches!(keyboard(false).action_for_event(&event), Some(Action::Insert(text)) if text == "\n")
            );
            assert!(keyboard(true).action_for_event(&event).is_none());
        }
    }

    #[test]
    fn dropped_files_only_enter_an_available_composer() {
        let event = Event::DragAndDropFiles {
            paths: vec!["/tmp/example.csv".into()],
            location: Vector2F::new(20., 20.),
        };
        let mut input = keyboard(false);
        assert!(input.action_for_event(&event).is_none());
        input.accept_attachments = true;
        assert!(
            matches!(input.action_for_event(&event), Some(Action::AmooDropFiles(paths)) if paths == vec![PathBuf::from("/tmp/example.csv")])
        );
    }

    #[test]
    fn chinese_commit_replaces_selection_once_and_preserves_utf8_cursor() {
        let mut input = Input::default();
        input.set("旧内容");
        input.all = true;
        let action = keyboard(true)
            .action_for_event(&Event::SetMarkedText {
                marked_text: "ni hao".into(),
                selected_range: 0..6,
            })
            .unwrap();
        if let Action::Preedit(text) = action {
            input.preedit = text;
        } else {
            panic!("expected preedit");
        }
        assert_eq!(input.text, "旧内容");
        let action = keyboard(true)
            .action_for_event(&Event::TypedCharacters {
                chars: "你好🙂".into(),
            })
            .unwrap();
        if let Action::Insert(text) = action {
            input.insert(&text);
        } else {
            panic!("expected commit");
        }
        assert_eq!(input.text, "你好🙂");
        assert!(input.preedit.is_empty());
        input.cursor = input.previous();
        input.insert("，Amoo");
        assert_eq!(input.text, "你好，Amoo🙂");
        assert!(input.text.is_char_boundary(input.cursor));
    }
}
