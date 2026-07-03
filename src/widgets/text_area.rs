//! A scrollable, focusable multi-line text-entry field.
//!
//! Generalizes [`InputLine`](super::InputLine)'s single line the way
//! [`ListBox`](super::ListBox) generalizes a single choice: a flat `String`
//! (real `'\n'`s as hard breaks) with a grapheme cursor over the whole text,
//! reflowed to the current width via this module's own whitespace-preserving
//! [`reflow`] rather than [`crate::wrap::wrap`] (which collapses space runs —
//! correct for read-only prose, wrong for an editable buffer). Vertical
//! scrolling follows the [`View::scroll_metrics`]/[`View::set_scroll`]
//! protocol (ADR 0015) exactly like `ListBox` — no bar of its own; a host
//! draws one. Editing, word motion, and selection share
//! [`super::text_edit`]'s free functions with `InputLine`.

use super::text_edit;
use crate::canvas::Canvas;
use crate::cell::{Cell, Grapheme};
use crate::color::{Attributes, Style};
use crate::event::{Event, EventResult, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect};
use crate::theme::{Role, Theme};
use crate::view::{AxisMetrics, Context, ScrollMetrics, View};
use crate::wrap::word_offsets;
use unicode_segmentation::UnicodeSegmentation;
use unicode_width::UnicodeWidthStr;

/// Where [`TextArea::set_text`]/[`TextArea::with_text`] place the cursor in
/// the new text.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorPosition {
    /// The very start of the text (the default — the more common affordance
    /// outside a single-line field).
    Start,
    /// The very end of the text (matches [`InputLine::set_text`](super::InputLine::set_text)).
    End,
}

/// A scrollable multi-line text-entry field.
pub struct TextArea {
    bounds: Rect,
    text: String,
    /// Cursor position as a grapheme index over the whole `text`.
    cursor: usize,
    /// The other end of an in-progress selection (Shift+navigation), if any.
    selection_anchor: Option<usize>,
    /// The laid-out display lines at the current width: `(start_byte,
    /// verbatim slice of `text`)`. Rebuilt by [`relayout`](Self::relayout).
    lines: Vec<(usize, String)>,
    /// Index of the topmost visible display line.
    top: usize,
    focused: bool,
    style: Style,
    selection_style: Style,
    /// `Insert` toggles this, same as `InputLine`.
    overtype: bool,
}

impl TextArea {
    /// Creates an empty text area at `bounds`, in the theme's [`Role::Input`]
    /// colour.
    pub fn new(bounds: Rect, theme: &Theme) -> Self {
        let mut area = Self {
            bounds,
            text: String::new(),
            cursor: 0,
            selection_anchor: None,
            lines: Vec::new(),
            top: 0,
            focused: false,
            style: theme.style(Role::Input),
            selection_style: theme.style(Role::Selection),
            overtype: false,
        };
        area.relayout();
        area
    }

    /// Seeds the field with `text`, cursor at the start.
    pub fn with_text(mut self, text: &str) -> Self {
        self.set_text(text);
        self
    }

    /// Seeds the field with `text`, cursor at `at`.
    pub fn with_text_at(mut self, text: &str, at: CursorPosition) -> Self {
        self.set_text_at(text, at);
        self
    }

    /// Replaces the value with `text`, placing the cursor at its start.
    pub fn set_text(&mut self, text: &str) {
        self.set_text_at(text, CursorPosition::Start);
    }

    /// Replaces the value with `text`, placing the cursor at `at`.
    pub fn set_text_at(&mut self, text: &str, at: CursorPosition) {
        self.text = text.to_string();
        self.selection_anchor = None;
        self.top = 0;
        self.cursor = match at {
            CursorPosition::Start => 0,
            CursorPosition::End => text_edit::grapheme_len(&self.text),
        };
        self.relayout();
    }

    /// The current value.
    pub fn text(&self) -> &str {
        &self.text
    }

    /// The currently selected text, if any.
    pub fn selected_text(&self) -> Option<&str> {
        let (start, end) = text_edit::selection_range(self.selection_anchor, self.cursor)?;
        let starts = text_edit::grapheme_starts(&self.text);
        Some(&self.text[starts[start]..starts[end]])
    }

    /// The number of visible rows.
    fn rows(&self) -> usize {
        self.bounds.height().max(0) as usize
    }

    /// The width reflow packs lines to — one less than the box's own width,
    /// permanently reserving its last column so the caret always has a real
    /// column to sit in at true end-of-line. `TextArea` never scrolls
    /// horizontally (unlike `InputLine`), so this is the only mechanism that
    /// keeps the caret visible there — not a per-case fallback for a line
    /// that happens to land exactly at the edge, but why one never does.
    fn wrap_width(&self) -> u16 {
        (self.bounds.width() - 1).max(0) as u16
    }

    /// Recomputes `lines` for the current width, then re-clamps `top` and
    /// re-scrolls to keep the cursor visible (ADR 0017: called on every text
    /// change and on a width-changing resize).
    fn relayout(&mut self) {
        self.lines = reflow(&self.text, self.wrap_width());
        let max_top = self.lines.len().saturating_sub(self.rows());
        self.top = self.top.min(max_top);
        self.ensure_row_visible();
    }

    /// Repositions/resizes the field (ADR 0017): a width change reflows, a
    /// height-only change just re-clamps the scroll — the same split
    /// `ListBox::set_bounds` uses.
    pub fn set_bounds(&mut self, bounds: Rect) {
        let width_changed = bounds.width() != self.bounds.width();
        self.bounds = bounds;
        if width_changed {
            self.relayout();
        } else {
            let max_top = self.lines.len().saturating_sub(self.rows());
            self.top = self.top.min(max_top);
            self.ensure_row_visible();
        }
    }

    /// Scrolls `top` so the cursor's display row is visible.
    fn ensure_row_visible(&mut self) {
        let (row, _) = self.display_pos(self.cursor);
        let rows = self.rows().max(1);
        if row < self.top {
            self.top = row;
        } else if row >= self.top + rows {
            self.top = row + 1 - rows;
        }
    }

    /// Maps a grapheme cursor index to its `(display_row, display_column)`.
    fn display_pos(&self, cursor: usize) -> (usize, i16) {
        let starts = text_edit::grapheme_starts(&self.text);
        let byte = starts[cursor.min(starts.len() - 1)];
        let mut row = 0;
        for (i, (off, _)) in self.lines.iter().enumerate() {
            if *off <= byte {
                row = i;
            } else {
                break;
            }
        }
        let (line_off, line) = &self.lines[row];
        let within = byte.saturating_sub(*line_off);
        let mut col = 0i16;
        let mut consumed = 0usize;
        for g in line.graphemes(true) {
            if consumed >= within {
                break;
            }
            col += width_of(g);
            consumed += g.len();
        }
        (row, col)
    }

    /// The inverse of [`display_pos`](Self::display_pos): the grapheme cursor
    /// index at `(row, col)`, clamping `row` to the last line and `col` past
    /// a short line's end.
    fn cursor_from_display(&self, row: usize, col: i16) -> usize {
        let row = row.min(self.lines.len().saturating_sub(1));
        let (line_off, line) = &self.lines[row];
        let mut acc = 0i16;
        let mut byte_in_line = 0usize;
        let target = col.max(0);
        for g in line.graphemes(true) {
            let w = width_of(g);
            if acc + w > target {
                break;
            }
            acc += w;
            byte_in_line += g.len();
        }
        text_edit::byte_to_grapheme(&self.text, line_off + byte_in_line)
    }

    /// Moves the cursor by `delta` display rows (clamped), preserving column.
    fn move_display_row(&mut self, delta: isize) {
        let (row, col) = self.display_pos(self.cursor);
        let last = self.lines.len() as isize - 1;
        let new_row = (row as isize + delta).clamp(0, last.max(0)) as usize;
        self.cursor = self.cursor_from_display(new_row, col);
    }

    /// Moves the cursor to the start of its current *display* line.
    fn line_home(&mut self) {
        let (row, _) = self.display_pos(self.cursor);
        let (line_off, _) = &self.lines[row];
        self.cursor = text_edit::byte_to_grapheme(&self.text, *line_off);
    }

    /// Moves the cursor to the end of its current *display* line.
    fn line_end(&mut self) {
        let (row, _) = self.display_pos(self.cursor);
        let (line_off, line) = &self.lines[row];
        self.cursor = text_edit::byte_to_grapheme(&self.text, line_off + line.len());
    }

    /// Applies a navigation move, updating the selection anchor to match
    /// whether `shift` was held (see [`text_edit::next_anchor`]), then
    /// rescrolls to keep the cursor visible.
    fn navigate(&mut self, shift: bool, mv: impl FnOnce(&mut Self)) {
        let old = self.cursor;
        mv(self);
        self.selection_anchor = text_edit::next_anchor(self.selection_anchor, shift, old);
        self.ensure_row_visible();
    }

    /// Deletes the active selection (if any), clearing the anchor and moving
    /// the cursor to where it started. A no-op when nothing is selected.
    fn delete_selection(&mut self) {
        let sel = text_edit::selection_range(self.selection_anchor, self.cursor);
        self.cursor = text_edit::collapse_selection(&mut self.text, sel, self.cursor);
        self.selection_anchor = None;
    }

    /// Whether the grapheme sitting at the cursor is the line-break `'\n'` —
    /// overtype must never delete one (see [`Self::insert_or_overwrite`]).
    fn cursor_on_newline(&self) -> bool {
        let starts = text_edit::grapheme_starts(&self.text);
        self.cursor + 1 < starts.len()
            && &self.text[starts[self.cursor]..starts[self.cursor + 1]] == "\n"
    }

    /// Inserts `c` at the cursor, or in overtype mode replaces the grapheme
    /// there — except a `'\n'`, which overtype always pushes right (falling
    /// back to insert) rather than deleting the line break.
    fn insert_or_overwrite(&mut self, c: char) -> usize {
        if self.overtype && !self.cursor_on_newline() {
            text_edit::overwrite(&mut self.text, self.cursor, c)
        } else {
            text_edit::insert(&mut self.text, self.cursor, c)
        }
    }

    /// Handles a mouse event in the field's local coordinates: a click places
    /// the cursor (clearing any selection); the wheel pans `top`.
    fn handle_mouse(&mut self, m: &MouseEvent) -> EventResult {
        match m.kind {
            MouseKind::Down(MouseButton::Left) => {
                let row = self.top + m.pos.y.max(0) as usize;
                self.cursor = self.cursor_from_display(row, m.pos.x);
                self.selection_anchor = None;
                self.ensure_row_visible();
                EventResult::Consumed
            }
            MouseKind::ScrollDown => {
                let max_top = self.lines.len().saturating_sub(self.rows());
                self.top = (self.top + 1).min(max_top);
                EventResult::Consumed
            }
            MouseKind::ScrollUp => {
                self.top = self.top.saturating_sub(1);
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

/// Breaks `text` into display lines of at most `width` columns, preserving
/// every byte verbatim — unlike [`crate::wrap::wrap`], no whitespace run is
/// ever collapsed *or erased*. A gap (whether between two words or trailing
/// at the end of a hard line) is never atomic and never elided: as much of
/// it as fits joins the current line, and any part that doesn't wraps onto
/// its own continuation line(s), the same as a hard-to-place word's
/// surroundings would. Interior gaps count at their true display width, so
/// packing reflects what a monospace terminal actually renders. Only a
/// *word* (or leading indentation glued to the first word of a hard line)
/// is atomic — wider than `width`, it still overflows rather than splitting.
///
/// An earlier version of this function elided a whole gap outright once it
/// stopped fitting (matching `wrap::wrap`'s own single-space-separator
/// convention). That is wrong here for two reasons: it can hide an
/// arbitrary amount of typed whitespace at once rather than the one column
/// `wrap` ever elides, and — worse — a cursor sitting inside an elided gap
/// has no display column to map to at all, so it reads as stuck in place no
/// matter how much more whitespace is typed into it, only jumping once a
/// following word finally forces a new wrap. Splitting instead of eliding
/// means every byte of `text` always has a real, distinct display position.
fn reflow(text: &str, width: u16) -> Vec<(usize, String)> {
    let width = width as usize;
    let mut out = Vec::new();
    let mut hard_start = 0usize;
    for hard_line in text.split('\n') {
        let words = word_offsets(hard_line);
        let mut line_start = 0usize;
        let mut cur_width = 0usize;
        let mut pos = 0usize;
        for &(wstart, word) in &words {
            place_gap(
                hard_line,
                &mut pos,
                wstart,
                width,
                &mut cur_width,
                &mut line_start,
                hard_start,
                &mut out,
            );
            let word_w = word.width();
            if cur_width == 0 {
                // First token on a fresh line: placed unconditionally, even
                // if it alone exceeds `width` (overflow allowed, never
                // split).
                cur_width = word_w;
            } else if cur_width + word_w <= width {
                cur_width += word_w;
            } else {
                out.push((
                    hard_start + line_start,
                    hard_line[line_start..wstart].to_string(),
                ));
                line_start = wstart;
                cur_width = word_w;
            }
            pos = wstart + word.len();
        }
        // The tail: everything after the last word, or the whole hard line
        // if it has no words at all — just another gap, by the same rule.
        place_gap(
            hard_line,
            &mut pos,
            hard_line.len(),
            width,
            &mut cur_width,
            &mut line_start,
            hard_start,
            &mut out,
        );
        out.push((hard_start + line_start, hard_line[line_start..].to_string()));
        hard_start += hard_line.len() + 1; // +1 for the '\n' just consumed
    }
    out
}

/// Places the gap `hard_line[*pos..gap_end)` (either between two words or
/// the trailing tail), packing as much of it as fits onto the current line
/// and wrapping any remainder onto its own continuation line(s) — never
/// eliding any of it (see [`reflow`]'s doc comment for why). Advances `*pos`
/// to `gap_end`, updating `*cur_width`/`*line_start` and pushing any flushed
/// lines to `out` along the way.
#[allow(clippy::too_many_arguments)]
fn place_gap(
    hard_line: &str,
    pos: &mut usize,
    gap_end: usize,
    width: usize,
    cur_width: &mut usize,
    line_start: &mut usize,
    hard_start: usize,
    out: &mut Vec<(usize, String)>,
) {
    while *pos < gap_end {
        let room = width.saturating_sub(*cur_width);
        let mut taken = 0usize;
        let mut taken_w = 0usize;
        for g in hard_line[*pos..gap_end].graphemes(true) {
            let w = g.width().max(1);
            if taken_w + w > room {
                break;
            }
            taken_w += w;
            taken += g.len();
        }
        if taken == 0 {
            if *cur_width == 0 {
                // Even a fresh line has no room at all (width == 0): force
                // one grapheme through anyway, mirroring the
                // first-token-placed-unconditionally overflow rule, so this
                // always makes progress.
                let g = hard_line[*pos..gap_end].graphemes(true).next().unwrap();
                taken = g.len();
                taken_w = g.width().max(1);
            } else {
                out.push((
                    hard_start + *line_start,
                    hard_line[*line_start..*pos].to_string(),
                ));
                *line_start = *pos;
                *cur_width = 0;
                continue;
            }
        }
        *cur_width += taken_w;
        *pos += taken;
    }
}

/// Display width of one grapheme cluster, in columns (≥ 0).
fn width_of(grapheme: &str) -> i16 {
    Grapheme::new(grapheme).width() as i16
}

impl View for TextArea {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        canvas.fill(area, &Cell::blank(self.style));
        let rows = self.rows();
        if rows == 0 || area.width() <= 0 {
            return;
        }

        let starts = text_edit::grapheme_starts(&self.text);
        let sel_bytes = text_edit::selection_range(self.selection_anchor, self.cursor)
            .map(|(s, e)| (starts[s], starts[e]));

        for r in 0..rows {
            let idx = self.top + r;
            if idx >= self.lines.len() {
                break;
            }
            let (line_off, line) = &self.lines[idx];
            let mut col = 0i16;
            let mut byte_in_line = 0usize;
            for g in line.graphemes(true) {
                let w = width_of(g).max(1);
                if col + w > area.width() {
                    break;
                }
                let abs = line_off + byte_in_line;
                let style = match sel_bytes {
                    Some((s, e)) if abs >= s && abs < e => self.selection_style,
                    _ => self.style,
                };
                canvas.put_str(Point::new(col, r as i16), g, style);
                col += w;
                byte_in_line += g.len();
            }
        }

        // The caret, drawn only when focused (ADR 0010) — same
        // underline/reverse-block convention as `InputLine`. `wrap_width`
        // always reserves the box's last column (see its own doc comment),
        // so a display line's content never reaches `area.width()` and the
        // caret always has a real column to sit in at true end-of-line —
        // no scrolling, no per-case rolling/clamping needed here.
        if self.focused {
            let (row, col) = self.display_pos(self.cursor);
            if row >= self.top && row < self.top + rows && col >= 0 && col < area.width() {
                let local_row = (row - self.top) as i16;
                let (line_off, line) = &self.lines[row];
                let within = starts[self.cursor].saturating_sub(*line_off);
                let mut consumed = 0usize;
                let glyph = line.graphemes(true).find(|g| {
                    let at = consumed;
                    consumed += g.len();
                    at == within
                });
                let caret_attrs = if self.overtype {
                    Attributes::REVERSE
                } else {
                    Attributes::UNDERLINE
                };
                let style = self.style.attrs(caret_attrs);
                let cell = match glyph {
                    Some(g) => Cell::new(Grapheme::new(g), style),
                    None => Cell::blank(style),
                };
                canvas.set(Point::new(col, local_row), cell);
            }
        }
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
        if let Event::Mouse(m) = event {
            return self.handle_mouse(m);
        }
        if let Event::Paste(text) = event {
            if !self.focused {
                return EventResult::Ignored;
            }
            self.delete_selection();
            for c in text.chars().filter(|c| !c.is_control() || *c == '\n') {
                self.cursor = text_edit::insert(&mut self.text, self.cursor, c);
            }
            self.relayout();
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
                self.cursor = self.insert_or_overwrite(c);
                self.relayout();
                EventResult::Consumed
            }
            KeyCode::Enter => {
                self.delete_selection();
                self.cursor = text_edit::insert(&mut self.text, self.cursor, '\n');
                self.relayout();
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
                self.relayout();
                EventResult::Consumed
            }
            KeyCode::Delete => {
                if self.selection_anchor.is_some() {
                    self.delete_selection();
                } else {
                    text_edit::delete(&mut self.text, self.cursor);
                }
                self.relayout();
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
                        (s.cursor + 1).min(text_edit::grapheme_len(&s.text))
                    };
                });
                EventResult::Consumed
            }
            KeyCode::Up => {
                self.navigate(shift, |s| s.move_display_row(-1));
                EventResult::Consumed
            }
            KeyCode::Down => {
                self.navigate(shift, |s| s.move_display_row(1));
                EventResult::Consumed
            }
            KeyCode::PageUp => {
                let page = self.rows().max(1) as isize;
                self.navigate(shift, |s| s.move_display_row(-page));
                EventResult::Consumed
            }
            KeyCode::PageDown => {
                let page = self.rows().max(1) as isize;
                self.navigate(shift, |s| s.move_display_row(page));
                EventResult::Consumed
            }
            // Home/End are line-scoped; Ctrl+Home/Ctrl+End reach the whole text.
            KeyCode::Home if ctrl => {
                self.navigate(shift, |s| s.cursor = 0);
                EventResult::Consumed
            }
            KeyCode::End if ctrl => {
                self.navigate(shift, |s| s.cursor = text_edit::grapheme_len(&s.text));
                EventResult::Consumed
            }
            KeyCode::Home => {
                self.navigate(shift, |s| s.line_home());
                EventResult::Consumed
            }
            KeyCode::End => {
                self.navigate(shift, |s| s.line_end());
                EventResult::Consumed
            }
            // Tab bubbles so the dialog can use it.
            _ => EventResult::Ignored,
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn scroll_metrics(&self) -> Option<ScrollMetrics> {
        let rows = self.rows();
        if self.lines.len() <= rows {
            return None;
        }
        Some(ScrollMetrics {
            horizontal: None,
            vertical: Some(AxisMetrics {
                total: self.lines.len(),
                visible: rows,
                pos: self.top,
            }),
        })
    }

    fn set_scroll(&mut self, offset: Point) {
        let max_top = self.lines.len().saturating_sub(self.rows());
        self.top = (offset.y.max(0) as usize).min(max_top);
    }

    fn set_bounds(&mut self, bounds: Rect) {
        self.set_bounds(bounds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::CommandSet;
    use crate::event::KeyEvent;
    use crate::geometry::Size;

    fn rect(w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(w, h))
    }

    fn focused(w: i16, h: i16) -> TextArea {
        let mut t = TextArea::new(rect(w, h), &Theme::default());
        t.set_focused(true);
        t
    }

    fn key(t: &mut TextArea, code: KeyCode, modifiers: Modifiers) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        t.handle_event(&Event::Key(KeyEvent::new(code, modifiers)), &mut ctx)
    }

    fn press(t: &mut TextArea, code: KeyCode) -> EventResult {
        key(t, code, Modifiers::NONE)
    }

    fn type_str(t: &mut TextArea, s: &str) {
        for c in s.chars() {
            if c == '\n' {
                press(t, KeyCode::Enter);
            } else {
                press(t, KeyCode::Char(c));
            }
        }
    }

    fn click(t: &mut TextArea, x: i16, y: i16) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        t.handle_event(
            &Event::Mouse(MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                pos: Point::new(x, y),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        )
    }

    fn render(t: &TextArea, w: i16, h: i16) -> String {
        let mut buf = Buffer::new(Size::new(w, h));
        let mut canvas = Canvas::new(&mut buf);
        t.draw(&mut canvas);
        buf.to_text()
    }

    // --- reflow (whitespace-preserving) ---

    #[test]
    fn reflow_preserves_multiple_spaces_when_they_fit() {
        assert_eq!(reflow("a    b", 40), vec![(0, "a    b".to_string())]);
    }

    #[test]
    fn reflow_splits_rather_than_elides_a_gap_at_the_wrap_point() {
        // "hello" (5) fits width 5; "world" would need 5+1+5, so it wraps —
        // but the single separator space isn't discarded, just given its
        // own line, so every byte of the source stays addressable.
        let out = reflow("hello world", 5);
        assert_eq!(
            out,
            vec![
                (0, "hello".to_string()),
                (5, " ".to_string()),
                (6, "world".to_string()),
            ]
        );
    }

    #[test]
    fn reflow_preserves_leading_indentation() {
        assert_eq!(
            reflow("  indented", 40),
            vec![(0, "  indented".to_string())]
        );
    }

    #[test]
    fn reflow_counts_true_gap_width_not_a_collapsed_single_space() {
        // "a" + 10 spaces + "b": at width 5 the true gap width alone (10)
        // already exceeds what's left after "a", so "b" wraps well past it
        // (a `wrap::wrap`-style collapse to one space would instead have let
        // "a b" fit on one line) — and the 10 spaces are split across their
        // own lines rather than discarded, so `b`'s line ends up starting
        // with the one space that didn't fit anywhere else.
        let out = reflow("a          b", 5);
        assert_eq!(
            out,
            vec![
                (0, "a    ".to_string()),
                (5, "     ".to_string()),
                (10, " b".to_string()),
            ]
        );
    }

    #[test]
    fn reflow_never_splits_an_over_long_word() {
        let out = reflow("hi superlongword ok", 6);
        assert_eq!(
            out,
            vec![
                (0, "hi ".to_string()),
                (3, "superlongword".to_string()),
                (16, " ok".to_string()),
            ]
        );
        assert_eq!(
            out.iter().map(|(_, l)| l.as_str()).collect::<String>(),
            "hi superlongword ok",
            "splitting the surrounding gaps never drops a byte"
        );
    }

    #[test]
    fn reflow_preserves_blank_and_whitespace_only_lines() {
        let out = reflow("one\n\n   \ntwo", 40);
        assert_eq!(
            out,
            vec![
                (0, "one".to_string()),
                (4, String::new()),
                (5, "   ".to_string()),
                (9, "two".to_string()),
            ]
        );
    }

    #[test]
    fn reflow_wraps_trailing_whitespace_that_overflows_the_width() {
        // "ab" (2) + 10 trailing spaces, width 5: the trailing run isn't a
        // word, so it wraps across continuation lines rather than growing
        // the first line to 12 columns unboundedly.
        let out = reflow("ab          ", 5);
        assert_eq!(
            out,
            vec![
                (0, "ab   ".to_string()),
                (5, "     ".to_string()),
                (10, "  ".to_string()),
            ]
        );
        for (_, line) in &out {
            assert!(line.width() <= 5, "{line:?} fits the width");
        }
    }

    #[test]
    fn reflow_wraps_an_all_whitespace_hard_line_that_overflows_the_width() {
        // No words at all on this hard line — same rule applies.
        let out = reflow("          ", 4); // 10 spaces, width 4
        assert_eq!(
            out,
            vec![
                (0, "    ".to_string()),
                (4, "    ".to_string()),
                (8, "  ".to_string()),
            ]
        );
    }

    // --- Logic: cursor motion & editing ---

    #[test]
    fn insert_backspace_delete_across_a_line_break() {
        let mut t = focused(20, 5);
        t.set_text_at("ab\ncd", CursorPosition::End);
        press(&mut t, KeyCode::Backspace); // removes 'd'
        assert_eq!(t.text(), "ab\nc");
        press(&mut t, KeyCode::Home); // start of the "c" display line
        press(&mut t, KeyCode::Backspace); // removes the '\n', joining lines
        assert_eq!(t.text(), "abc");
    }

    #[test]
    fn enter_splits_a_line() {
        let mut t = focused(20, 5).with_text("ab");
        press(&mut t, KeyCode::Right); // between a and b
        press(&mut t, KeyCode::Enter);
        assert_eq!(t.text(), "a\nb");
    }

    #[test]
    fn left_right_cross_the_line_break() {
        let mut t = focused(20, 5).with_text("ab\ncd");
        for _ in 0..3 {
            press(&mut t, KeyCode::Right);
        }
        assert_eq!(t.cursor, 3);
        press(&mut t, KeyCode::Left);
        assert_eq!(t.cursor, 2);
    }

    #[test]
    fn up_down_preserve_column_and_clamp_on_shorter_lines() {
        let mut t = focused(20, 5).with_text("abcdef\nab\nabcdef");
        t.cursor = 4; // 'e' on the first line, column 4
        press(&mut t, KeyCode::Down);
        assert_eq!(t.cursor, 9, "clamped to the end of the short 'ab' line");
        press(&mut t, KeyCode::Down);
        assert_eq!(
            t.cursor, 12,
            "column stays at 2 (where it got clamped), not restored to 4 \
             — no sticky goal column, by design"
        );
    }

    #[test]
    fn home_end_are_confined_to_the_display_line() {
        let mut t = focused(20, 5).with_text("abc\ndef");
        t.cursor = 5; // 'e' on the second line
        press(&mut t, KeyCode::Home);
        assert_eq!(t.cursor, 4, "start of 'def', not the whole buffer");
        press(&mut t, KeyCode::End);
        assert_eq!(t.cursor, 7, "end of 'def', not the whole buffer");
    }

    #[test]
    fn ctrl_home_and_end_reach_the_whole_text() {
        let mut t = focused(20, 5).with_text("abc\ndef");
        t.cursor = 5;
        key(&mut t, KeyCode::Home, Modifiers::CONTROL);
        assert_eq!(t.cursor, 0);
        key(&mut t, KeyCode::End, Modifiers::CONTROL);
        assert_eq!(t.cursor, 7);
    }

    #[test]
    fn ctrl_left_and_right_jump_by_word_across_a_line_break() {
        let mut t = focused(20, 5).with_text("hello\nworld");
        t.cursor = 0;
        key(&mut t, KeyCode::Right, Modifiers::CONTROL);
        assert_eq!(
            t.cursor, 6,
            "lands at the start of 'world', past the newline"
        );
        key(&mut t, KeyCode::Left, Modifiers::CONTROL);
        assert_eq!(t.cursor, 0);
    }

    #[test]
    fn page_up_and_down_move_by_a_screenful() {
        let text = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut t = focused(20, 3).with_text(&text);
        t.cursor = 0;
        press(&mut t, KeyCode::PageDown); // 3 rows
        assert_eq!(t.display_pos(t.cursor).0, 3);
        press(&mut t, KeyCode::PageUp);
        assert_eq!(t.display_pos(t.cursor).0, 0);
    }

    #[test]
    fn overtype_toggle_and_fallback_at_a_line_break_and_true_end() {
        let mut t = focused(20, 5).with_text("ab\ncd");
        t.cursor = 2; // right before the '\n'
        press(&mut t, KeyCode::Insert);
        press(&mut t, KeyCode::Char('X'));
        // Overwriting the '\n' falls back to insert (never deletes the break).
        assert_eq!(t.text(), "abX\ncd");

        let mut t2 = focused(20, 5).with_text("ab");
        t2.cursor = 2; // true end
        press(&mut t2, KeyCode::Insert);
        press(&mut t2, KeyCode::Char('c'));
        assert_eq!(t2.text(), "abc");
    }

    #[test]
    fn set_text_defaults_to_start_and_at_end_variant_lands_at_the_end() {
        let t1 = TextArea::new(rect(20, 5), &Theme::default()).with_text("hello");
        assert_eq!(t1.cursor, 0);
        let t2 = TextArea::new(rect(20, 5), &Theme::default())
            .with_text_at("hello", CursorPosition::End);
        assert_eq!(t2.cursor, 5);
    }

    #[test]
    fn typing_preserves_multiple_spaces_and_reflow_round_trips_on_resize() {
        let mut t = focused(4, 5);
        type_str(&mut t, "a    b");
        assert_eq!(
            t.text(),
            "a    b",
            "no space silently collapsed while typing"
        );
        t.set_bounds(rect(40, 5));
        assert_eq!(t.text(), "a    b", "resize never touches stored text");
    }

    #[test]
    fn typing_trailing_spaces_past_the_margin_advances_the_cursor_immediately() {
        // wrap_width is 5 (box width 6, minus the reserved column); "abcde"
        // fills it exactly. Each further space must wrap (and the cursor
        // must visibly advance) on its own keystroke — not stay put until a
        // later non-space character forces a reflow.
        let mut t = focused(6, 3).with_text("abcde");
        press(&mut t, KeyCode::End);
        assert_eq!(t.display_pos(t.cursor), (0, 5));
        press(&mut t, KeyCode::Char(' '));
        assert_eq!(
            t.lines.len(),
            2,
            "the space already wrapped to its own line"
        );
        assert_eq!(t.display_pos(t.cursor), (1, 1));
        press(&mut t, KeyCode::Char(' '));
        assert_eq!(t.display_pos(t.cursor), (1, 2));
    }

    #[test]
    fn typing_into_an_interior_gap_advances_the_cursor_immediately() {
        // Inserting spaces *between* two existing words (an interior gap,
        // not the trailing tail) used to leave the cursor's display
        // position stuck at the end of the previous line for every
        // keystroke inside a gap that had been elided wholesale — only
        // jumping once a later edit forced a new wrap. Splitting instead of
        // eliding means every one of these positions is a distinct, real
        // column.
        let mut t = focused(10, 3).with_text("hello world");
        t.cursor = 5; // right after "hello", before the space+"world"
        let start = t.display_pos(t.cursor);
        press(&mut t, KeyCode::Char(' '));
        let after_one = t.display_pos(t.cursor);
        assert_ne!(after_one, start, "the cursor moved on the very first space");
        press(&mut t, KeyCode::Char(' '));
        let after_two = t.display_pos(t.cursor);
        assert_ne!(
            after_two, after_one,
            "and again on the second — no batching until a later character"
        );
    }

    // --- Selection ---

    #[test]
    fn shift_navigation_extends_a_selection() {
        let mut t = focused(20, 5).with_text("hello");
        t.cursor = 0;
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        assert_eq!(t.selected_text(), Some("he"));
    }

    #[test]
    fn reversing_direction_shrinks_and_flips_the_range() {
        let mut t = focused(20, 5).with_text("hello");
        t.cursor = 2;
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        assert_eq!(t.selected_text(), Some("ll"));
        // The anchor stays fixed at 2; walking the cursor back past it
        // flips which side of the anchor the selection reads from.
        for _ in 0..4 {
            key(&mut t, KeyCode::Left, Modifiers::SHIFT);
        }
        assert_eq!(
            t.selected_text(),
            Some("he"),
            "the range flips but stays ordered"
        );
    }

    #[test]
    fn a_bare_arrow_collapses_a_selection() {
        let mut t = focused(20, 5).with_text("hello");
        t.cursor = 0;
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        press(&mut t, KeyCode::Right);
        assert_eq!(t.selected_text(), None);
    }

    #[test]
    fn typing_over_a_selection_replaces_it() {
        let mut t = focused(20, 5).with_text("hello");
        t.cursor = 0;
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        press(&mut t, KeyCode::Char('X'));
        assert_eq!(t.text(), "Xllo");
    }

    #[test]
    fn enter_over_a_selection_replaces_it() {
        let mut t = focused(20, 5).with_text("hello");
        t.cursor = 0;
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        press(&mut t, KeyCode::Enter);
        assert_eq!(t.text(), "\nllo");
    }

    #[test]
    fn backspace_and_delete_over_a_selection_remove_the_whole_range() {
        let mut t = focused(20, 5).with_text("hello");
        t.cursor = 0;
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        press(&mut t, KeyCode::Backspace);
        assert_eq!(t.text(), "llo");
    }

    #[test]
    fn a_click_clears_an_active_selection() {
        let mut t = focused(20, 5).with_text("hello");
        t.cursor = 0;
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        key(&mut t, KeyCode::Right, Modifiers::SHIFT);
        click(&mut t, 3, 0);
        assert_eq!(t.selected_text(), None);
    }

    #[test]
    fn shift_end_then_shift_down_then_backspace_deletes_the_expected_span() {
        let mut t = focused(20, 5).with_text("abc\ndef");
        t.cursor = 0;
        key(&mut t, KeyCode::End, Modifiers::SHIFT); // select "abc"
        key(&mut t, KeyCode::Down, Modifiers::SHIFT); // extend onto "def"
        press(&mut t, KeyCode::Backspace);
        // Selection ran from 0 to wherever column 3 lands on "def" (index 3
        // within that line): "abc\ndef" cursor at col3 on row1 -> grapheme
        // index 7 (the end, since "def" is exactly 3 chars) -> whole text
        // deleted.
        assert_eq!(t.text(), "");
    }

    // --- Interaction: paste, scroll ---

    #[test]
    fn bracketed_paste_keeps_newlines() {
        let mut t = focused(20, 5);
        t.set_focused(true);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        t.handle_event(&Event::Paste("a\nb".to_string()), &mut ctx);
        assert_eq!(t.text(), "a\nb");
    }

    #[test]
    fn scroll_metrics_is_none_under_a_page_and_some_once_it_overflows() {
        let short = focused(20, 5).with_text("a\nb");
        assert_eq!(short.scroll_metrics(), None);

        let text = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let long = focused(20, 3).with_text(&text);
        assert_eq!(
            long.scroll_metrics(),
            Some(ScrollMetrics {
                horizontal: None,
                vertical: Some(AxisMetrics {
                    total: 10,
                    visible: 3,
                    pos: 0,
                }),
            })
        );
    }

    #[test]
    fn set_scroll_clamps_and_moves_top_without_touching_the_cursor() {
        let text = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut t = focused(20, 3).with_text(&text);
        let cursor_before = t.cursor;
        t.set_scroll(Point::new(0, 4));
        assert_eq!(t.top, 4);
        assert_eq!(t.cursor, cursor_before);
        t.set_scroll(Point::new(0, 99));
        assert_eq!(t.top, 7, "clamps to the last full page (10 - 3)");
    }

    #[test]
    fn wheel_scrolls_without_moving_the_cursor() {
        let text = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut t = focused(20, 3).with_text(&text);
        let cursor_before = t.cursor;
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        t.handle_event(
            &Event::Mouse(MouseEvent {
                kind: MouseKind::ScrollDown,
                pos: Point::new(0, 0),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        );
        assert_eq!(t.top, 1);
        assert_eq!(t.cursor, cursor_before);
    }

    // --- Render ---

    #[test]
    fn snapshot_multiline_wrapped_content_with_caret() {
        let mut t = focused(6, 3).with_text("hello world");
        t.cursor = 11;
        insta::assert_snapshot!(render(&t, 6, 3));
    }

    #[test]
    fn wrap_width_always_reserves_the_boxs_last_column() {
        // A 6-wide box reflows to 5 columns: "abcde" (5) fits the reserved
        // width exactly, and "fghij" — which would fit the box's full 6
        // columns — still wraps to its own line, because the box's own
        // width is never what reflow packs to.
        let t = focused(6, 3).with_text("abcde fghij");
        assert_eq!(
            t.lines,
            vec![
                (0, "abcde".to_string()),
                (5, " ".to_string()),
                (6, "fghij".to_string()),
            ]
        );
    }

    #[test]
    fn the_caret_is_visible_at_true_end_of_line_with_no_special_casing_needed() {
        // Before `wrap_width` reserved a column, this exact content used to
        // need a roll/clamp fallback to keep the caret on screen — now the
        // reserved column means it just... has somewhere to go.
        let mut t = focused(6, 1);
        t.set_text_at("abcde", CursorPosition::End);
        let mut buf = Buffer::new(Size::new(6, 1));
        let mut canvas = Canvas::new(&mut buf);
        t.draw(&mut canvas);
        assert!(
            buf.get(Point::new(5, 0))
                .unwrap()
                .style()
                .attrs
                .contains(Attributes::UNDERLINE),
            "the reserved last column holds the caret, not just clamped or invisible"
        );
    }

    #[test]
    fn caret_is_underlined_in_insert_and_reverse_in_overtype() {
        let mut t = focused(10, 1).with_text_at("hi", CursorPosition::End);
        let mut buf = Buffer::new(Size::new(10, 1));
        let mut canvas = Canvas::new(&mut buf);
        t.draw(&mut canvas);
        assert!(
            buf.get(Point::new(2, 0))
                .unwrap()
                .style()
                .attrs
                .contains(Attributes::UNDERLINE)
        );

        press(&mut t, KeyCode::Insert);
        let mut buf2 = Buffer::new(Size::new(10, 1));
        let mut canvas2 = Canvas::new(&mut buf2);
        t.draw(&mut canvas2);
        assert!(
            buf2.get(Point::new(2, 0))
                .unwrap()
                .style()
                .attrs
                .contains(Attributes::REVERSE)
        );
    }

    #[test]
    fn a_scrolled_view_shows_the_lower_lines() {
        let text = (0..10)
            .map(|i| format!("line{i}"))
            .collect::<Vec<_>>()
            .join("\n");
        let mut t = focused(20, 3).with_text(&text);
        t.set_scroll(Point::new(0, 4));
        let text_rendered = render(&t, 20, 3);
        let rows: Vec<&str> = text_rendered.lines().collect();
        assert!(rows[0].starts_with("line4"));
    }

    #[test]
    fn a_selection_spanning_a_line_break_draws_in_the_selection_role() {
        let mut t = focused(20, 3).with_text("abc\ndef");
        t.cursor = 0;
        key(&mut t, KeyCode::End, Modifiers::SHIFT);
        key(&mut t, KeyCode::Down, Modifiers::SHIFT);
        let mut buf = Buffer::new(Size::new(20, 3));
        let mut canvas = Canvas::new(&mut buf);
        t.draw(&mut canvas);
        let theme = Theme::default();
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().style(),
            theme.style(Role::Selection),
            "'a' on the first row is inside the selection"
        );
    }
}
