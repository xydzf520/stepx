//! Native text selection. UTF-8 byte offsets are kept separate from shaped character indices.
use super::*;
use facloud_ui::{platform::Cursor, text::IsRect, text_layout::TextStyle};
use pathfinder_geometry::rect::RectF;
use std::ops::Range;

impl Input {
    pub(super) fn place_pointer(&mut self, position: usize, extend: bool, clicks: u32) -> bool {
        if !self.preedit.is_empty() {
            return false;
        }
        let mut pos = position.min(self.text.len());
        while !self.text.is_char_boundary(pos) {
            pos -= 1;
        }
        if extend {
            self.anchor.get_or_insert(self.cursor);
        } else {
            self.anchor = Some(pos);
        }
        self.cursor = pos;
        self.last_edit = None;
        self.all = false;
        self.dragging = true;
        if clicks >= 3 {
            self.all = true;
            self.anchor = Some(0);
            self.cursor = self.text.len();
        } else if clicks == 2 {
            let start = self.text[..pos]
                .char_indices()
                .rev()
                .find(|(_, c)| c.is_whitespace())
                .map(|(i, c)| i + c.len_utf8())
                .unwrap_or(0);
            let end = self.text[pos..]
                .char_indices()
                .find(|(_, c)| c.is_whitespace())
                .map(|(i, _)| pos + i)
                .unwrap_or(self.text.len());
            self.anchor = Some(start);
            self.cursor = end;
        }
        true
    }
    pub(super) fn checkpoint(&mut self, group: bool) {
        if !group
            || self
                .last_edit
                .is_none_or(|at| at.elapsed() > Duration::from_millis(650))
        {
            self.undo.push((self.text.clone(), self.cursor));
            if self.undo.len() > 32 {
                self.undo.remove(0);
            }
        }
        self.last_edit = Some(Instant::now());
        self.redo.clear();
    }
    pub(super) fn restore_edit(&mut self, redo: bool) {
        let previous = if redo {
            self.redo.pop()
        } else {
            self.undo.pop()
        };
        if let Some((text, cursor)) = previous {
            let old = (std::mem::replace(&mut self.text, text), self.cursor);
            if redo {
                self.undo.push(old);
            } else {
                self.redo.push(old);
            }
            self.cursor = cursor;
            self.all = false;
            self.anchor = None;
            self.preedit.clear();
            self.last_edit = None;
        }
    }
    pub(super) fn selection_range(&self) -> Option<Range<usize>> {
        if self.all {
            return Some(0..self.text.len());
        }
        self.anchor
            .filter(|a| *a != self.cursor)
            .map(|a| a.min(self.cursor)..a.max(self.cursor))
    }
    pub(super) fn delete_selection(&mut self) -> bool {
        let Some(range) = self.selection_range() else {
            self.anchor = None;
            return false;
        };
        self.cursor = range.start;
        self.text.replace_range(range, "");
        self.all = false;
        self.anchor = None;
        true
    }
    pub(super) fn extend(&mut self, direction: i8) {
        self.anchor.get_or_insert(self.cursor);
        self.cursor = match direction {
            -2 => 0,
            2 => self.text.len(),
            -1 => self.previous(),
            _ => self.next(),
        };
        self.all = false;
    }
}
impl Companion {
    pub(super) fn clear_text_interaction(&mut self) {
        self.text_selection.clear();
        self.selected_text = None;
        self.context_menu = None;
        self.wheel_motions.borrow_mut().clear();
    }
    pub(super) fn selectable_messages(&self, child: widgets::El) -> widgets::El {
        let origin = std::rc::Rc::new(std::cell::Cell::new(vec2f(0., 0.)));
        let click_origin = origin.clone();
        let area = SelectableArea::new(
            self.text_selection.clone(),
            |args, ctx, _| {
                ctx.dispatch_typed_action(Action::MessageSelection(args.selection));
            },
            child,
        )
        .on_selection_right_click(move |ctx, at| {
            ctx.dispatch_typed_action(Action::ContextOpen(
                context_menu::Target::Selection,
                click_origin.get() + at,
            ));
        })
        .finish();
        let area = SelectionRegion {
            child: area,
            origin,
        }
        .finish();
        Hoverable::new(self.mouse_state("selectable-message-body"), move |_| area)
            .with_cursor(Cursor::IBeam)
            .with_defer_events_to_children()
            .finish()
    }
    pub(super) fn handle_text_action(
        &mut self,
        action: &Action,
        ctx: &mut ViewContext<Self>,
    ) -> bool {
        match action {
            Action::MessageSelection(text) => {
                self.selected_text = text.clone().filter(|s| !s.is_empty());
                // SelectableArea also reports None for clicks outside its bounds.
                // That notification must not steal focus from an editor below it.
                if self.selected_text.is_some() {
                    self.focus = None;
                }
            }
            Action::InputPointer(index, position, extend, clicks) => {
                if *index == F_GLOBAL_SEARCH && !self.search.open && self.page != Page::Search {
                    self.handle_search(&SearchAction::Open, ctx);
                }
                let input = &mut self.fields[*index];
                if !input.place_pointer(*position, *extend, *clicks) {
                    return true;
                }
                self.focus = Some(*index);
                self.selected_text = None;
                self.text_selection.clear();
                ctx.focus_self();
            }
            Action::InputPointerUp => {
                for f in &mut self.fields {
                    f.dragging = false;
                }
            }
            Action::ExtendSelection(direction) => {
                if let Some(i) = self.focus {
                    self.fields[i].extend(*direction);
                }
            }
            _ => return false,
        }
        true
    }
    /// Wrap a shaped paragraph. Pointer hit testing uses the actual font layout, not estimated widths.
    pub(super) fn editable_line(
        &self,
        mut text: Text,
        line: &str,
        offset: usize,
        index: usize,
        caret: Option<usize>,
    ) -> widgets::El {
        let f = &self.fields[index];
        let focused = self.input_has_focus(index);
        let insertion = if focused { f.preedit.len() } else { 0 };
        let masked = (index == F_PAIR_CODE && !self.show_secret) || index == F_LAB_KEY;
        let mask_map = masked.then(|| {
            masked_byte_map(
                &f.text,
                f.cursor,
                if focused {
                    f.preedit.chars().count()
                } else {
                    0
                },
            )
        });
        if let Some(r) = f.selection_range().filter(|_| focused) {
            let positions = line
                .char_indices()
                .enumerate()
                .filter_map(|(i, (byte, _))| {
                    let display = offset + byte;
                    let original = mask_map
                        .as_ref()
                        .and_then(|m| m.get(i))
                        .copied()
                        .unwrap_or_else(|| display_to_original(display, f.cursor, insertion));
                    (r.contains(&original)
                        && !(display >= f.cursor && display < f.cursor + insertion))
                        .then_some(i)
                })
                .collect();
            text = text.with_single_highlight(
                Highlight::new()
                    .with_text_style(TextStyle::new().with_background_color(self.p().accent_soft)),
                positions,
            );
        }
        let count = line.chars().count();
        let mut caret_info = None;
        if let Some(c) = caret.filter(|_| focused) {
            let at_end = c >= count && count > 0;
            let id = format!("input-glyph:{index}");
            text = text.with_saved_char_position(if at_end { count - 1 } else { c }, id.clone());
            caret_info = Some((id, at_end));
        }
        EditableLine {
            text,
            mask_map,
            index,
            offset,
            cursor: f.cursor,
            insertion,
            dragging: f.dragging,
            composing: !f.preedit.is_empty(),
            caret: caret_info,
            draw_caret: focused && f.selection_range().is_none(),
            caret_color: self.p().text,
        }
        .finish()
    }
}
fn masked_byte_map(text: &str, cursor: usize, preedit_chars: usize) -> Vec<usize> {
    let source: Vec<usize> = text
        .char_indices()
        .map(|(i, _)| i)
        .chain(std::iter::once(text.len()))
        .collect();
    let caret = text[..cursor].chars().count();
    (0..source.len() + preedit_chars)
        .map(|i| source[display_to_original(i, caret, preedit_chars).min(source.len() - 1)])
        .collect()
}
fn display_to_original(display: usize, cursor: usize, insertion: usize) -> usize {
    if display <= cursor {
        display
    } else {
        display.saturating_sub(insertion).max(cursor)
    }
}
// SelectableArea reports block-local coordinates; retain its painted origin for the popup.
struct SelectionRegion {
    child: widgets::El,
    origin: std::rc::Rc<std::cell::Cell<Vector2F>>,
}
impl Element for SelectionRegion {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        self.child.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.child.after_layout(ctx, app)
    }
    fn paint(&mut self, o: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        self.origin.set(o);
        self.child.paint(o, ctx, app)
    }
    fn size(&self) -> Option<Vector2F> {
        self.child.size()
    }
    fn origin(&self) -> Option<Point> {
        self.child.origin()
    }
    fn dispatch_event(
        &mut self,
        e: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        self.child.dispatch_event(e, ctx, app)
    }
}
struct EditableLine {
    text: Text,
    mask_map: Option<Vec<usize>>,
    index: usize,
    offset: usize,
    cursor: usize,
    insertion: usize,
    dragging: bool,
    composing: bool,
    caret: Option<(String, bool)>,
    draw_caret: bool,
    caret_color: facloud_ui::color::ColorU,
}
impl Element for EditableLine {
    fn layout(&mut self, c: SizeConstraint, ctx: &mut LayoutContext, app: &AppContext) -> Vector2F {
        self.text.layout(c, ctx, app)
    }
    fn after_layout(&mut self, ctx: &mut AfterLayoutContext, app: &AppContext) {
        self.text.after_layout(ctx, app)
    }
    fn paint(&mut self, o: Vector2F, ctx: &mut PaintContext, app: &AppContext) {
        // Commit the shaped glyph bounds before painting a layout-independent caret.
        ctx.position_cache.start();
        self.text.paint(o, ctx, app);
        ctx.position_cache.end();
        if let Some((id, at_end)) = &self.caret {
            let glyph = ctx
                .position_cache
                .get_position(id)
                .unwrap_or(RectF::new(o, vec2f(0., 20.)));
            // Ellipsized text may still report the original, offscreen glyph
            // position. Keep the caret (and native IME anchor) in the editor,
            // away from the adjacent microphone and action buttons.
            let size = self.text.size().unwrap_or(vec2f(1.5, 20.));
            let height = (glyph.height() - 2.).max(14.).min(size.y());
            let caret = RectF::new(
                vec2f(
                    (glyph.min_x() + if *at_end { glyph.width() } else { 0. })
                        .clamp(o.x(), o.x() + (size.x() - 1.5).max(0.)),
                    (glyph.min_y() + 1.)
                        .clamp(o.y(), o.y() + (size.y() - height).max(0.)),
                ),
                vec2f(1.5, height),
            );
            ctx.position_cache.start();
            ctx.position_cache
                .cache_position_indefinitely(format!("input-caret:{}", self.index), caret);
            ctx.position_cache.end();
            if self.draw_caret {
                ctx.scene
                    .draw_rect_without_hit_recording(caret)
                    .with_background(Fill::Solid(self.caret_color));
            }
        }
    }
    fn size(&self) -> Option<Vector2F> {
        self.text.size()
    }
    fn origin(&self) -> Option<Point> {
        self.text.origin()
    }
    fn dispatch_event(
        &mut self,
        event: &DispatchedEvent,
        ctx: &mut EventContext,
        app: &AppContext,
    ) -> bool {
        if matches!(event.raw_event(), Event::LeftMouseUp { .. }) && self.dragging {
            ctx.dispatch_typed_action(Action::InputPointerUp);
            return false;
        }
        let (position, extend, clicks) = match event.raw_event() {
            Event::LeftMouseDown {
                position,
                modifiers,
                click_count,
                ..
            } => (*position, modifiers.shift, *click_count),
            Event::LeftMouseDragged { position, .. } if self.dragging => (*position, true, 1),
            Event::RightMouseDown { .. } => {
                return false;
            }
            _ => return self.text.dispatch_event(event, ctx, app),
        };
        let (Some(origin), Some(size)) = (self.origin(), self.size()) else {
            return false;
        };
        if position.y() < origin.y()
            || position.y() > origin.y() + size.y()
            || ctx.is_covered(Point::from_vec2f(position, origin.z_index()))
        {
            return false;
        }
        if !extend && (position.x() < origin.x() || position.x() > origin.x() + size.x().max(24.)) {
            return false;
        }
        if self.composing {
            return true;
        }
        let prefix = self
            .text
            .get_selection(
                vec2f(origin.x() - 1., origin.y() - 1.),
                position,
                IsRect::False,
            )
            .map(|v| v.iter().map(|f| f.text.len()).sum::<usize>())
            .unwrap_or(0);
        let byte = self
            .mask_map
            .as_ref()
            .map(|m| m[(prefix / "•".len()).min(m.len() - 1)])
            .unwrap_or_else(|| {
                display_to_original(self.offset + prefix, self.cursor, self.insertion)
            });
        ctx.dispatch_typed_action(Action::InputPointer(self.index, byte, extend, clicks));
        true
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn unicode_selection_replaces_only_selected_range() {
        let mut f = Input::default();
        f.set("你好🌟 world");
        f.cursor = "你好🌟".len();
        f.anchor = Some("你".len());
        assert_eq!(&f.text[f.selection_range().unwrap()], "好🌟");
        f.insert("测试");
        assert_eq!(f.text, "你测试 world");
        f.extend(-1);
        assert_eq!(&f.text[f.selection_range().unwrap()], "试");
        f.delete_selection();
        assert_eq!(f.text, "你测 world");
        f.all = true;
        f.insert("替换");
        assert_eq!(f.text, "替换");
    }
    #[test]
    fn mask_selection_maps_to_original_characters() {
        assert_eq!(masked_byte_map("a中🌟", 1, 0), vec![0, 1, 4, 8]);
        assert_eq!(masked_byte_map("a中🌟", 1, 2), vec![0, 1, 1, 1, 4, 8]);
    }
    #[test]
    fn undo_redo_preserve_unicode_and_clear_on_new_draft() {
        let mut f = Input::default();
        f.set("你好🌟");
        f.checkpoint(false);
        f.insert("测试");
        f.restore_edit(false);
        assert_eq!(f.text, "你好🌟");
        f.restore_edit(true);
        assert_eq!(f.text, "你好🌟测试");
        f.clear();
        assert!(f.undo.is_empty() && f.redo.is_empty());
    }
    #[test]
    fn caret_is_not_part_of_copied_text() {
        assert_eq!(display_to_original(9, 3, 3), 6);
        assert_eq!(display_to_original(4, 3, 3), 3);
    }
}
