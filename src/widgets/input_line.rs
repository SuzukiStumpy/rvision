//! A one-line editable text field (TurboVision's `TInputLine`).
//!
//! A focusable [control](super) for a [`Dialog`](super::Dialog): it holds a
//! `String`, a grapheme cursor, and a horizontal scroll offset so a value longer
//! than the field stays usable. Editing and cursor motion step by **grapheme
//! cluster** (`unicode-segmentation`, ADR 0006/0008), never by byte, via the
//! shared helpers in [`super::text_edit`]. When focused it draws a caret (a
//! reverse-video cell, ADR 0010); a real hardware cursor waits for the editor
//! (Phase 6).

use super::text_edit;
use crate::canvas::Canvas;
use crate::cell::{Cell, Grapheme};
use crate::color::{Attributes, Style};
use crate::event::{Event, EventResult, KeyCode, Modifiers, MouseButton, MouseKind};
use crate::geometry::{Point, Rect};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};
use unicode_segmentation::UnicodeSegmentation;

/// A single-line text input field.
pub struct InputLine {
    bounds: Rect,
    text: String,
    /// Cursor position as a grapheme index in `0..=len`.
    cursor: usize,
    /// The other end of an in-progress selection (Shift+navigation), if any.
    selection_anchor: Option<usize>,
    /// Leftmost visible grapheme index.
    scroll: usize,
    focused: bool,
    style: Style,
    selection_style: Style,
    /// `Insert` toggles this: on, a printable `Char` overwrites the grapheme
    /// under the cursor instead of pushing it right.
    overtype: bool,
}

impl InputLine {
    /// Creates an empty input field at `bounds`, in the theme's [`Role::Input`]
    /// colour.
    pub fn new(bounds: Rect, theme: &Theme) -> Self {
        Self {
            bounds,
            text: String::new(),
            cursor: 0,
            selection_anchor: None,
            scroll: 0,
            focused: false,
            style: theme.style(Role::Input),
            selection_style: theme.style(Role::Selection),
            overtype: false,
        }
    }

    /// Seeds the field with `text`, placing the cursor at the end.
    pub fn with_text(mut self, text: &str) -> Self {
        self.set_text(text);
        self
    }

    /// Replaces the value with `text`, placing the cursor at the end and
    /// re-scrolling to show it (e.g. when a file picker mirrors a list selection).
    pub fn set_text(&mut self, text: &str) {
        self.text = text.to_string();
        self.cursor = self.len();
        self.selection_anchor = None;
        self.scroll = 0;
        self.ensure_visible();
    }

    /// The current value.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The cursor position (a grapheme index in `0..=len`).
    pub fn cursor(&self) -> usize {
        self.cursor
    }

    /// The currently selected text, if any.
    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = text_edit::selection_range(self.selection_anchor, self.cursor)?;
        let starts = text_edit::grapheme_starts(&self.text);
        Some(&self.text[starts[start]..starts[end]])
    }

    /// The number of grapheme clusters in the value.
    fn len(&self) -> usize {
        text_edit::grapheme_len(&self.text)
    }

    /// The grapheme index under local display column `x` — the inverse of the
    /// caret placement in [`draw`](View::draw), so a click lands on the grapheme it
    /// points at (clamped to `len` past the value's end).
    fn grapheme_at(&self, x: i16) -> usize {
        let graphemes: Vec<&str> = self.text.graphemes(true).collect();
        let target = col_of(&graphemes, self.scroll) + x.max(0);
        let mut idx = self.scroll;
        while idx < graphemes.len() && col_of(&graphemes, idx + 1) <= target {
            idx += 1;
        }
        idx
    }

    /// Deletes the active selection (if any), clearing the anchor and moving
    /// the cursor to where the selection started. A no-op when nothing is
    /// selected.
    fn delete_selection(&mut self) {
        let sel = text_edit::selection_range(self.selection_anchor, self.cursor);
        self.cursor = text_edit::collapse_selection(&mut self.text, sel, self.cursor);
        self.selection_anchor = None;
    }

    /// Scrolls so the cursor column is visible within the field width.
    fn ensure_visible(&mut self) {
        let width = self.bounds.width().max(1);
        if self.cursor < self.scroll {
            self.scroll = self.cursor;
            return;
        }
        let graphemes: Vec<&str> = self.text.graphemes(true).collect();
        while col_of(&graphemes, self.cursor) - col_of(&graphemes, self.scroll) >= width {
            self.scroll += 1;
        }
    }

    /// Moves the cursor via `mv`, updating the selection anchor to match
    /// whether `shift` was held (ADR-free shared rule, see
    /// [`text_edit::next_anchor`]), then re-scrolls to keep it visible.
    fn navigate(&mut self, shift: bool, mv: impl FnOnce(&mut Self)) {
        let old = self.cursor;
        mv(self);
        self.selection_anchor = text_edit::next_anchor(self.selection_anchor, shift, old);
        self.ensure_visible();
    }
}

/// Display width of one grapheme cluster, in columns (≥ 0).
fn width_of(grapheme: &str) -> i16 {
    Grapheme::new(grapheme).width() as i16
}

/// Total display width of `graphemes[..idx]`.
fn col_of(graphemes: &[&str], idx: usize) -> i16 {
    graphemes[..idx.min(graphemes.len())]
        .iter()
        .map(|g| width_of(g))
        .sum()
}

impl View for InputLine {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        let width = area.width();
        canvas.fill(area, &Cell::blank(self.style));

        let selection = text_edit::selection_range(self.selection_anchor, self.cursor);
        let graphemes: Vec<&str> = self.text.graphemes(true).collect();
        let mut col = 0;
        let mut idx = self.scroll;
        while idx < graphemes.len() && col < width {
            let w = width_of(graphemes[idx]).max(1);
            if col + w > width {
                break; // never split a wide grapheme across the right edge
            }
            let style = match selection {
                Some((start, end)) if idx >= start && idx < end => self.selection_style,
                _ => self.style,
            };
            canvas.put_str(Point::new(col, 0), graphemes[idx], style);
            col += w;
            idx += 1;
        }

        // The caret over the grapheme at the cursor (or a blank past the
        // end), drawn only when focused (ADR 0010): a reverse-video block in
        // overtype mode (what's about to be replaced), an underline in
        // insert mode (what's about to be pushed right) — the same
        // block-vs-bar convention as a real terminal cursor, since this caret
        // is hand-drawn rather than the hardware one (Phase 6).
        if self.focused {
            let caret_col = col_of(&graphemes, self.cursor) - col_of(&graphemes, self.scroll);
            if caret_col >= 0 && caret_col < width {
                let caret_attrs = if self.overtype {
                    Attributes::REVERSE
                } else {
                    Attributes::UNDERLINE
                };
                let style = self.style.attrs(caret_attrs);
                let cell = if self.cursor < graphemes.len() {
                    Cell::new(Grapheme::new(graphemes[self.cursor]), style)
                } else {
                    Cell::blank(style)
                };
                canvas.set(Point::new(caret_col, 0), cell);
            }
        }
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
        // A click places the caret under the pointer (the group focuses the field
        // first), regardless of the prior focus, and clears any selection (no
        // drag-select yet).
        if let Event::Mouse(m) = event {
            if matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
                self.cursor = self.grapheme_at(m.pos.x);
                self.selection_anchor = None;
                self.ensure_visible();
                return EventResult::Consumed;
            }
            return EventResult::Ignored;
        }
        // A bracketed paste drops its text in at the caret (replacing any
        // selection first); a single-line field takes only the printable
        // characters, flattening any newlines (ADR 0012).
        if let Event::Paste(text) = event {
            if !self.focused {
                return EventResult::Ignored;
            }
            self.delete_selection();
            for c in text.chars().filter(|c| !c.is_control()) {
                self.cursor = text_edit::insert(&mut self.text, self.cursor, c);
            }
            self.ensure_visible();
            return EventResult::Consumed;
        }
        let Event::Key(key) = event else {
            return EventResult::Ignored;
        };
        if !self.focused {
            return EventResult::Ignored;
        }
        let shift = key.modifiers.contains(Modifiers::SHIFT);
        let ctrl = key.modifiers.contains(Modifiers::CONTROL);
        match key.code {
            KeyCode::Char(c)
                if !c.is_control() && !ctrl && !key.modifiers.contains(Modifiers::ALT) =>
            {
                self.delete_selection();
                self.cursor = if self.overtype {
                    text_edit::overwrite(&mut self.text, self.cursor, c)
                } else {
                    text_edit::insert(&mut self.text, self.cursor, c)
                };
                self.ensure_visible();
                EventResult::Consumed
            }
            KeyCode::Insert => {
                self.overtype = !self.overtype;
                EventResult::Consumed
            }
            KeyCode::Backspace => {
                if self.selection_anchor.is_some() {
                    self.delete_selection();
                } else {
                    self.cursor = text_edit::backspace(&mut self.text, self.cursor);
                }
                self.ensure_visible();
                EventResult::Consumed
            }
            KeyCode::Delete => {
                if self.selection_anchor.is_some() {
                    self.delete_selection();
                } else {
                    text_edit::delete(&mut self.text, self.cursor);
                }
                self.ensure_visible();
                EventResult::Consumed
            }
            KeyCode::Left => {
                self.navigate(shift, |s| {
                    s.cursor = if ctrl {
                        text_edit::word_left(&s.text, s.cursor)
                    } else {
                        s.cursor.saturating_sub(1)
                    };
                });
                EventResult::Consumed
            }
            KeyCode::Right => {
                self.navigate(shift, |s| {
                    s.cursor = if ctrl {
                        text_edit::word_right(&s.text, s.cursor)
                    } else {
                        (s.cursor + 1).min(s.len())
                    };
                });
                EventResult::Consumed
            }
            // Home/End (and their Ctrl variants, which have nothing further to
            // reach on a single line) go to the start/end of the value.
            KeyCode::Home => {
                self.navigate(shift, |s| s.cursor = 0);
                EventResult::Consumed
            }
            KeyCode::End => {
                self.navigate(shift, |s| s.cursor = s.len());
                EventResult::Consumed
            }
            // Tab / Enter / Esc and anything else bubble to the dialog.
            _ => EventResult::Ignored,
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::CommandSet;
    use crate::event::KeyEvent;
    use crate::geometry::Size;

    fn rect(w: i16) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(w, 1))
    }

    fn focused(width: i16) -> InputLine {
        let mut input = InputLine::new(rect(width), &Theme::default());
        input.set_focused(true);
        input
    }

    fn press(input: &mut InputLine, code: KeyCode) -> EventResult {
        key(input, code, Modifiers::NONE)
    }

    fn key(input: &mut InputLine, code: KeyCode, modifiers: Modifiers) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        input.handle_event(&Event::Key(KeyEvent::new(code, modifiers)), &mut ctx)
    }

    fn click(input: &mut InputLine, x: i16) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        input.handle_event(
            &Event::Mouse(crate::event::MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                pos: Point::new(x, 0),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        )
    }

    #[test]
    fn a_bracketed_paste_inserts_printables_and_flattens_newlines() {
        let mut input = focused(20);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = input.handle_event(&Event::Paste("a\nb c".to_string()), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(input.text(), "ab c", "newline dropped, the space kept");
    }

    #[test]
    fn clicking_places_the_caret_at_that_column() {
        let mut input = focused(10);
        type_str(&mut input, "hello"); // caret at end (5)
        assert_eq!(click(&mut input, 2), EventResult::Consumed);
        assert_eq!(input.cursor(), 2);
        // A click past the value's end clamps to its end.
        click(&mut input, 9);
        assert_eq!(input.cursor(), 5);
    }

    fn type_str(input: &mut InputLine, s: &str) {
        for c in s.chars() {
            press(input, KeyCode::Char(c));
        }
    }

    fn render(input: &InputLine, width: i16) -> String {
        let mut buf = Buffer::new(Size::new(width, 1));
        let mut canvas = Canvas::new(&mut buf);
        input.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn typing_inserts_and_advances_the_cursor() {
        let mut input = focused(20);
        type_str(&mut input, "abc");
        assert_eq!(input.text(), "abc");
        assert_eq!(input.cursor(), 3);
    }

    #[test]
    fn inserts_at_the_cursor_not_just_the_end() {
        let mut input = focused(20);
        type_str(&mut input, "ac");
        press(&mut input, KeyCode::Left); // between a and c
        press(&mut input, KeyCode::Char('b'));
        assert_eq!(input.text(), "abc");
        assert_eq!(input.cursor(), 2);
    }

    #[test]
    fn backspace_and_delete_remove_around_the_cursor() {
        let mut input = focused(20);
        type_str(&mut input, "abc"); // cursor at 3
        press(&mut input, KeyCode::Backspace);
        assert_eq!(input.text(), "ab");
        press(&mut input, KeyCode::Home);
        press(&mut input, KeyCode::Delete);
        assert_eq!(input.text(), "b");
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn cursor_moves_by_grapheme_and_to_the_ends() {
        let mut input = focused(20);
        type_str(&mut input, "abc");
        press(&mut input, KeyCode::Home);
        assert_eq!(input.cursor(), 0);
        press(&mut input, KeyCode::Right);
        assert_eq!(input.cursor(), 1);
        press(&mut input, KeyCode::End);
        assert_eq!(input.cursor(), 3);
        press(&mut input, KeyCode::Left);
        assert_eq!(input.cursor(), 2);
        // Left at the start and Right at the end do not run off.
        press(&mut input, KeyCode::Home);
        press(&mut input, KeyCode::Left);
        assert_eq!(input.cursor(), 0);
    }

    #[test]
    fn a_wide_grapheme_is_one_cursor_step() {
        let mut input = focused(20);
        type_str(&mut input, "a世b");
        assert_eq!(input.text(), "a世b");
        assert_eq!(input.cursor(), 3, "the wide char counts as one grapheme");
    }

    #[test]
    fn an_unfocused_field_ignores_typing() {
        let mut input = InputLine::new(rect(20), &Theme::default());
        assert_eq!(press(&mut input, KeyCode::Char('x')), EventResult::Ignored);
        assert_eq!(input.text(), "");
    }

    #[test]
    fn insert_key_toggles_overtype_and_typing_overwrites() {
        let mut input = focused(20);
        type_str(&mut input, "abc");
        press(&mut input, KeyCode::Home);
        assert_eq!(press(&mut input, KeyCode::Insert), EventResult::Consumed);
        press(&mut input, KeyCode::Char('X'));
        assert_eq!(input.text(), "Xbc");
        assert_eq!(input.cursor(), 1);
    }

    #[test]
    fn overtype_at_the_end_still_appends() {
        let mut input = focused(20);
        type_str(&mut input, "ab");
        press(&mut input, KeyCode::Insert); // cursor already at the end
        press(&mut input, KeyCode::Char('c'));
        assert_eq!(input.text(), "abc");
    }

    #[test]
    fn a_second_insert_press_toggles_back_to_insert_mode() {
        let mut input = focused(20);
        type_str(&mut input, "abc");
        press(&mut input, KeyCode::Home);
        press(&mut input, KeyCode::Insert);
        press(&mut input, KeyCode::Insert);
        press(&mut input, KeyCode::Char('X'));
        assert_eq!(input.text(), "Xabc");
    }

    #[test]
    fn enter_and_tab_bubble_so_the_dialog_can_use_them() {
        let mut input = focused(20);
        assert_eq!(press(&mut input, KeyCode::Enter), EventResult::Ignored);
        assert_eq!(press(&mut input, KeyCode::Tab), EventResult::Ignored);
    }

    #[test]
    fn horizontal_scroll_keeps_the_cursor_visible() {
        // Field width 5; type 8 chars. The window scrolls to show the tail and
        // the caret sits in the last column.
        let mut input = focused(5);
        type_str(&mut input, "abcdefgh");
        assert_eq!(render(&input, 5), "efgh ", "shows the tail with the caret");
        // Home scrolls back to the start.
        press(&mut input, KeyCode::Home);
        assert_eq!(render(&input, 5), "abcde");
    }

    #[test]
    fn the_caret_is_underlined_in_insert_mode_and_shown_only_when_focused() {
        let mut input = focused(10).with_text("hi");
        input.set_focused(true);
        let mut buf = Buffer::new(Size::new(10, 1));
        let mut canvas = Canvas::new(&mut buf);
        input.draw(&mut canvas);
        // Cursor is at the end (column 2): an underlined blank caret — the
        // thin, TurboVision-style insert-mode cursor, distinct from
        // overtype's reverse-video block (see the test below).
        assert!(
            buf.get(Point::new(2, 0))
                .unwrap()
                .style()
                .attrs
                .contains(Attributes::UNDERLINE)
        );

        // Unfocused: no caret anywhere.
        input.set_focused(false);
        let mut buf = Buffer::new(Size::new(10, 1));
        let mut canvas = Canvas::new(&mut buf);
        input.draw(&mut canvas);
        for x in 0..10 {
            let attrs = buf.get(Point::new(x, 0)).unwrap().style().attrs;
            assert!(!attrs.contains(Attributes::UNDERLINE));
            assert!(!attrs.contains(Attributes::REVERSE));
        }
    }

    #[test]
    fn overtype_mode_draws_a_reverse_video_block_caret() {
        let mut input = focused(10).with_text("hi");
        press(&mut input, KeyCode::Insert);
        let mut buf = Buffer::new(Size::new(10, 1));
        let mut canvas = Canvas::new(&mut buf);
        input.draw(&mut canvas);
        let attrs = buf.get(Point::new(2, 0)).unwrap().style().attrs;
        assert!(attrs.contains(Attributes::REVERSE));
        assert!(!attrs.contains(Attributes::UNDERLINE));
    }

    // --- Word motion (Ctrl+Left/Ctrl+Right) ---

    #[test]
    fn ctrl_right_and_left_jump_by_word() {
        let mut input = focused(30).with_text("hello world");
        press(&mut input, KeyCode::Home);
        key(&mut input, KeyCode::Right, Modifiers::CONTROL);
        assert_eq!(input.cursor(), 6, "lands at the start of 'world'");
        key(&mut input, KeyCode::Right, Modifiers::CONTROL);
        assert_eq!(input.cursor(), 11, "lands at the end");
        key(&mut input, KeyCode::Left, Modifiers::CONTROL);
        assert_eq!(input.cursor(), 6);
    }

    // --- Selection (Shift+navigation) ---

    #[test]
    fn shift_right_extends_a_selection_from_the_starting_cursor() {
        let mut input = focused(30).with_text("hello");
        press(&mut input, KeyCode::Home);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        assert_eq!(input.selected_text(), Some("he"));
    }

    #[test]
    fn a_bare_arrow_after_shift_selecting_collapses_it() {
        let mut input = focused(30).with_text("hello");
        press(&mut input, KeyCode::Home);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        press(&mut input, KeyCode::Right); // no shift: collapses
        assert_eq!(input.selected_text(), None);
    }

    #[test]
    fn selection_range_is_ordered_regardless_of_extend_direction() {
        let mut input = focused(30).with_text("hello");
        press(&mut input, KeyCode::End);
        key(&mut input, KeyCode::Left, Modifiers::SHIFT);
        key(&mut input, KeyCode::Left, Modifiers::SHIFT);
        assert_eq!(input.selected_text(), Some("lo"));
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut input = focused(30).with_text("hello");
        press(&mut input, KeyCode::Home);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        press(&mut input, KeyCode::Char('X'));
        assert_eq!(input.text(), "Xllo");
        assert_eq!(input.selected_text(), None);
    }

    #[test]
    fn backspace_over_a_selection_deletes_the_whole_range() {
        let mut input = focused(30).with_text("hello");
        press(&mut input, KeyCode::Home);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        press(&mut input, KeyCode::Backspace);
        assert_eq!(input.text(), "llo");
    }

    #[test]
    fn shift_home_and_end_select_to_the_line_bounds() {
        let mut input = focused(30).with_text("hello"); // cursor starts at the end
        key(&mut input, KeyCode::Home, Modifiers::SHIFT);
        assert_eq!(input.selected_text(), Some("hello"));
        press(&mut input, KeyCode::End); // bare: collapses, cursor to end
        assert_eq!(input.selected_text(), None);
    }

    #[test]
    fn a_click_clears_an_active_selection() {
        let mut input = focused(30).with_text("hello");
        press(&mut input, KeyCode::Home);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        click(&mut input, 4);
        assert_eq!(input.selected_text(), None);
    }

    #[test]
    fn pasting_over_a_selection_replaces_it() {
        let mut input = focused(30).with_text("hello");
        press(&mut input, KeyCode::Home);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        key(&mut input, KeyCode::Right, Modifiers::SHIFT);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        input.handle_event(&Event::Paste("HI".to_string()), &mut ctx);
        assert_eq!(input.text(), "HIllo");
    }
}
