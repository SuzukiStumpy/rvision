//! An editable text field with a filtered drop-down of suggestions
//! (`docs/specs/combo_box.md`).
//!
//! Composes a real [`InputLine`] (the text) and a real [`ListBox`] (the
//! suggestion rows) rather than reimplementing either. There is no overlay
//! protocol: `bounds()` simply reports a taller rectangle while the drop-down
//! is open (one row for the field, plus one row per visible suggestion), and
//! ordinary positional dispatch/draw — which query a child's `bounds()`
//! fresh every event/frame (`view.rs`) — take care of the rest. See the
//! spec's "key design decision" section for the trade-off this accepts (a
//! sibling occupying the drop-down's screen area can obscure or steal clicks
//! from it; the caller must leave room, the same discipline a drop shadow
//! already asks for, ADR 0011).

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{InputLine, ListBox};

/// How many suggestion rows show at once unless overridden via
/// [`ComboBox::max_visible`].
const DEFAULT_MAX_VISIBLE: usize = 8;

/// An editable field offering a filtered drop-down of candidate strings.
/// Typing free text is always accepted — the drop-down is completion, not a
/// constraint (`docs/specs/combo_box.md`).
pub struct ComboBox {
    theme: Theme,
    style: Style,
    input: InputLine,
    list: ListBox,
    items: Vec<String>,
    open: bool,
    /// `ComboBox`'s own idea of which filtered row is previewed, reset to
    /// `None` on every rebuild. Driven independently of `ListBox`'s own
    /// `selected` (see the spec: delegating `Up`/`Down` straight to
    /// `ListBox` would fight its "row 0 auto-selected on construction"
    /// behaviour).
    highlight: Option<usize>,
    /// The last text the user genuinely typed (as opposed to a navigation
    /// preview) — what `Esc` reverts to.
    last_typed_text: String,
    max_visible: usize,
    /// The candidate strings as of the last [`rebuild_list`](Self::rebuild_list)
    /// call — frozen at that point, not re-derived from live `text()`.
    /// `navigate` indexes into this, never into a fresh
    /// [`filtered`](Self::filtered): once a preview overwrites the field,
    /// re-filtering from that same field would collapse the candidate list
    /// to just the preview itself.
    matches: Vec<String>,
    /// The one-row-tall bounds when closed; open bounds are derived from
    /// this plus `list.bounds().height()`.
    closed_bounds: Rect,
    /// Whether typing narrows the candidate list (`true`, the default) or
    /// leaves it full and just jumps `highlight` to the first match
    /// (`false`) — see [`filterable`](Self::filterable).
    filterable: bool,
    /// `true` locks the field to values from `items`: printable keys drive
    /// [`search`](Self::search) instead of free editing — see
    /// [`select_only`](Self::select_only).
    select_only: bool,
    /// Accumulated typed characters since the drop-down last opened, used
    /// only in `select_only` mode (an editable field already has real text
    /// to search by; this stands in for it when there is none).
    search: String,
}

impl ComboBox {
    /// Creates an empty combo box at `bounds` (closed height forced to one
    /// row) offering `items` as suggestions.
    pub fn new(bounds: Rect, items: Vec<String>, theme: &Theme) -> Self {
        let closed_bounds = Rect::from_origin_size(bounds.origin(), Size::new(bounds.width(), 1));
        let input = InputLine::new(input_rect(closed_bounds.width()), theme);
        let list = ListBox::new(
            Rect::from_origin_size(Point::new(0, 1), Size::new(closed_bounds.width(), 0)),
            vec![],
            theme,
        );
        let mut combo = Self {
            theme: theme.clone(),
            style: theme.style(Role::Input),
            input,
            list,
            items,
            open: false,
            highlight: None,
            last_typed_text: String::new(),
            max_visible: DEFAULT_MAX_VISIBLE,
            matches: Vec::new(),
            closed_bounds,
            filterable: true,
            select_only: false,
            search: String::new(),
        };
        combo.rebuild_list();
        combo
    }

    /// Seeds the field's text as if typed, cursor at the end.
    pub fn with_text(mut self, text: &str) -> Self {
        self.set_text(text);
        self
    }

    /// Caps how many suggestion rows show at once (clamped to at least one).
    pub fn max_visible(mut self, n: usize) -> Self {
        self.max_visible = n.max(1);
        self.rebuild_list();
        self
    }

    /// Whether typing narrows the candidate list (`true`, the default). When
    /// `false`, the drop-down always shows every candidate and typing simply
    /// jumps `highlight` to the first one starting with the typed text —
    /// classic list "type-ahead," rather than filtering.
    pub fn filterable(mut self, yes: bool) -> Self {
        self.filterable = yes;
        self
    }

    /// Locks the field to a value from `items`: printable keys/clicks can
    /// only navigate and pick a suggestion, never insert free text. `Esc`
    /// still backs out of a preview the same way (there's just nothing else
    /// for it to revert *to* beyond the empty string until something has
    /// actually been picked).
    pub fn select_only(mut self, yes: bool) -> Self {
        self.select_only = yes;
        self
    }

    /// Replaces the field's text as if typed, and closes the drop-down.
    pub fn set_text(&mut self, text: &str) {
        self.input.set_text(text);
        self.last_typed_text = text.to_string();
        self.open = false;
        self.highlight = None;
    }

    /// The current field text — the value, whether or not it matches a
    /// listed item.
    pub fn text(&self) -> &str {
        self.input.text()
    }

    /// The index into the original `items` this text exactly matches
    /// (case-insensitive), or `None` for free text matching nothing.
    pub fn selected_index(&self) -> Option<usize> {
        self.items
            .iter()
            .position(|item| item.eq_ignore_ascii_case(self.text()))
    }

    /// Whether the drop-down is currently showing.
    pub fn is_open(&self) -> bool {
        self.open
    }

    fn width(&self) -> i16 {
        self.closed_bounds.width()
    }

    /// `items` starting with `needle`, case-insensitively (`""` matches
    /// everything) — unless `filterable` is off, in which case every
    /// candidate always shows regardless of `needle` (typing then only
    /// jumps `highlight`, via [`jump_to_first_match`](Self::jump_to_first_match),
    /// rather than narrowing what's shown).
    fn filtered_by(&self, needle: &str) -> Vec<String> {
        if !self.filterable {
            return self.items.clone();
        }
        let needle = needle.to_lowercase();
        self.items
            .iter()
            .filter(|item| item.to_lowercase().starts_with(&needle))
            .cloned()
            .collect()
    }

    /// Rebuilds the drop-down's `ListBox` from [`filtered_by`](Self::filtered_by)`(needle)`
    /// — cheap, and there is no `ListBox::set_items` to mutate one in place
    /// instead. Freezes the result into `self.matches` (see its doc comment)
    /// and always clears `highlight`: a fresh filter has no preview yet.
    fn rebuild_list_for(&mut self, needle: &str) {
        let matches = self.filtered_by(needle);
        let rows = matches.len().min(self.max_visible) as i16;
        let bounds = Rect::from_origin_size(Point::new(0, 1), Size::new(self.width(), rows));
        let mut list = ListBox::new(bounds, matches.clone(), &self.theme);
        list.set_focused(true);
        self.list = list;
        self.matches = matches;
        self.highlight = None;
    }

    /// [`rebuild_list_for`](Self::rebuild_list_for) using the field's current text.
    fn rebuild_list(&mut self) {
        let needle = self.text().to_string();
        self.rebuild_list_for(&needle);
    }

    /// Rebuilds the candidate list from `needle`, then — instead of leaving
    /// nothing highlighted, the way an ordinary rebuild does — jumps
    /// `highlight` straight to the first candidate starting with `needle`,
    /// if any. `copy_into_field` controls whether that match's text is also
    /// written into the field: `false` for an editable, non-filtering combo
    /// box (the field already holds exactly what was typed — searching by
    /// it must not overwrite it); `true` for `select_only` (there is no
    /// separately-typed text to preserve, so the match *is* the display
    /// value, the same idiom [`navigate`](Self::navigate) already uses for
    /// arrow-preview).
    fn jump_to_first_match(&mut self, needle: &str, copy_into_field: bool) {
        self.rebuild_list_for(needle);
        let needle = needle.to_lowercase();
        let Some(idx) = self
            .matches
            .iter()
            .position(|m| m.to_lowercase().starts_with(&needle))
        else {
            // No candidate matches: show nothing highlighted, rather than
            // `ListBox::new`'s own construction-default row 0 (ADR-free —
            // see `ListBox::deselect`'s doc comment).
            self.list.deselect();
            return;
        };
        self.highlight = Some(idx);
        self.list.select(idx);
        if copy_into_field {
            let text = self.matches[idx].clone();
            self.input.set_text(&text);
        }
    }

    /// `Down`/`Up`: opens if closed (only `Down` does; see `handle_key`),
    /// then moves `highlight` over `self.matches` (frozen at the last
    /// rebuild — never a fresh [`filtered`](Self::filtered), which would
    /// re-derive from the very text this preview is about to overwrite),
    /// clamped at the ends, copying the new preview into the field.
    fn navigate(&mut self, forward: bool) {
        if !self.open {
            self.open = true;
            self.rebuild_list();
        }
        if self.matches.is_empty() {
            return;
        }
        let next = match self.highlight {
            None => 0,
            Some(h) if forward => (h + 1).min(self.matches.len() - 1),
            Some(h) => h.saturating_sub(1),
        };
        self.highlight = Some(next);
        self.list.select(next);
        let text = self.matches[next].clone();
        self.input.set_text(&text);
    }

    /// Closes the drop-down; `revert` restores the last genuinely-typed text
    /// (undoing any navigation preview) — `Esc`'s behaviour, not a plain
    /// close's.
    fn close(&mut self, revert: bool) {
        if revert {
            let text = self.last_typed_text.clone();
            self.input.set_text(&text);
        }
        self.open = false;
        self.highlight = None;
        self.search.clear();
    }

    /// Delegates to the embedded `InputLine`, diffing its text before/after
    /// (the same idiom `ColorPicker::route` uses for its custom-entry
    /// fields) to detect an actual edit versus a cursor-only move. An edit
    /// opens the drop-down (if closed) and becomes the new "last typed"
    /// baseline; what happens to the candidate list then depends on
    /// `filterable` — narrow-and-clear-highlight (`rebuild_list`) or
    /// leave-full-and-jump (`jump_to_first_match`, not overwriting the
    /// field — see its doc comment).
    fn delegate_to_input(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let before = self.input.text().to_string();
        let result = self.input.handle_event(event, ctx);
        if self.input.text() != before {
            self.last_typed_text = self.input.text().to_string();
            self.open = true;
            if self.filterable {
                self.rebuild_list();
            } else {
                let needle = self.input.text().to_string();
                self.jump_to_first_match(&needle, false);
            }
        }
        result
    }

    /// The `select_only` counterpart to [`delegate_to_input`](Self::delegate_to_input):
    /// printable keys extend [`search`](Self::search) instead of editing any
    /// text directly; `Backspace` shortens it. Either way the candidate list
    /// (narrowed or not, per `filterable`) is rebuilt from `search`, and
    /// `highlight` jumps to the first match, copied into the field — there
    /// is no separately-typed text to preserve in this mode, so the match
    /// itself is the display value (mirrors `navigate`'s arrow-preview).
    fn handle_select_only_key(&mut self, code: KeyCode) -> EventResult {
        match code {
            KeyCode::Char(c) if !c.is_control() => {
                if !self.open {
                    self.open = true;
                    self.search.clear();
                }
                self.search.push(c);
                let needle = self.search.clone();
                self.jump_to_first_match(&needle, true);
                EventResult::Consumed
            }
            KeyCode::Backspace if self.open => {
                self.search.pop();
                let needle = self.search.clone();
                self.jump_to_first_match(&needle, true);
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }

    fn handle_key(&mut self, event: &Event, code: KeyCode, ctx: &mut Context) -> EventResult {
        match code {
            KeyCode::Down => {
                self.navigate(true);
                EventResult::Consumed
            }
            KeyCode::Up if self.open => {
                self.navigate(false);
                EventResult::Consumed
            }
            KeyCode::Enter if self.open => {
                self.close(false);
                EventResult::Consumed
            }
            KeyCode::Esc if self.open => {
                self.close(true);
                EventResult::Consumed
            }
            _ if self.select_only => self.handle_select_only_key(code),
            _ => self.delegate_to_input(event, ctx),
        }
    }

    fn handle_mouse(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        if m.pos.y == 0 {
            return self.handle_header_click(m, ctx);
        }
        if !self.open {
            return EventResult::Ignored;
        }
        let local = MouseEvent {
            pos: Point::new(m.pos.x, m.pos.y - 1),
            ..*m
        };
        let result = self.list.handle_event(&Event::Mouse(local), ctx);
        if matches!(
            m.kind,
            MouseKind::Down(MouseButton::Left) | MouseKind::DoubleClick(MouseButton::Left)
        ) {
            if let Some(text) = self.list.selected_text() {
                let text = text.to_string();
                self.input.set_text(&text);
                self.highlight = self.list.selected();
            }
            self.open = false;
        }
        result
    }

    fn handle_header_click(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        if m.pos.x == self.width() - 1 {
            if !matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
                return EventResult::Ignored;
            }
            if self.open {
                self.close(false);
            } else {
                self.open = true;
                self.rebuild_list();
            }
            return EventResult::Consumed;
        }
        if self.select_only {
            if matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
                if !self.open {
                    self.open = true;
                    self.rebuild_list();
                }
                return EventResult::Consumed;
            }
            return EventResult::Ignored;
        }
        self.input.handle_event(&Event::Mouse(*m), ctx)
    }
}

/// The embedded `InputLine`'s local bounds: every column except the
/// right-most, which the drop-arrow indicator owns.
fn input_rect(width: i16) -> Rect {
    Rect::from_origin_size(Point::new(0, 0), Size::new((width - 1).max(0), 1))
}

impl View for ComboBox {
    fn bounds(&self) -> Rect {
        if self.open {
            let height = 1 + self.list.bounds().height();
            Rect::from_origin_size(self.closed_bounds.origin(), Size::new(self.width(), height))
        } else {
            self.closed_bounds
        }
    }

    /// While open, the drop-down must win z-order over any sibling — an
    /// ordinary `Group` child sitting later in the vector (e.g. a dialog's
    /// OK/Cancel buttons) would otherwise draw over it and steal its clicks
    /// (ADR 0030).
    fn wants_topmost(&self) -> bool {
        self.open
    }

    fn draw(&self, canvas: &mut Canvas) {
        canvas.fill(
            Rect::from_origin_size(Point::new(0, 0), Size::new(self.width(), 1)),
            &Cell::blank(self.style),
        );
        let mut field = canvas.child(input_rect(self.width()));
        self.input.draw(&mut field);

        let arrow = if self.open { "▲" } else { "▼" };
        canvas.put_str(Point::new(self.width() - 1, 0), arrow, self.style);

        if self.open {
            let mut list = canvas.child(self.list.bounds());
            self.list.draw(&mut list);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Mouse(m) => self.handle_mouse(m, ctx),
            Event::Key(k) => self.handle_key(event, k.code, ctx),
            // A paste inserts free text, which `select_only` categorically
            // disallows — a no-op there rather than routed through `search`
            // (a pasted string isn't "typed" in the type-ahead sense).
            Event::Paste(_) if self.select_only => EventResult::Ignored,
            Event::Paste(_) => self.delegate_to_input(event, ctx),
            _ => EventResult::Ignored,
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        // No caret in `select_only`: there's nothing here to edit, and a
        // blinking caret would misleadingly suggest otherwise.
        if !self.select_only {
            self.input.set_focused(focused);
        }
        if !focused {
            self.close(false);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::{CM_OK, CommandSet};
    use crate::event::{KeyEvent, Modifiers};
    use crate::view::Group;
    use crate::widgets::Button;

    fn items(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|s| s.to_string()).collect()
    }

    fn rect(w: i16) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(w, 1))
    }

    fn combo(w: i16, labels: &[&str]) -> ComboBox {
        let mut cb = ComboBox::new(rect(w), items(labels), &Theme::default());
        cb.set_focused(true);
        cb
    }

    fn key(cb: &mut ComboBox, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        cb.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn type_str(cb: &mut ComboBox, s: &str) {
        for c in s.chars() {
            key(cb, KeyCode::Char(c));
        }
    }

    fn mouse(cb: &mut ComboBox, kind: MouseKind, x: i16, y: i16) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        cb.handle_event(
            &Event::Mouse(MouseEvent {
                kind,
                pos: Point::new(x, y),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        )
    }

    // --- Logic: filtering, selected_index, bounds ---

    #[test]
    fn filtered_matches_by_prefix_case_insensitively() {
        let mut cb = combo(20, &["Green", "Grey", "Blue"]);
        cb.set_text("gr");
        let mut got = cb.filtered_by(cb.text());
        got.sort();
        assert_eq!(got, vec!["Green".to_string(), "Grey".to_string()]);
    }

    #[test]
    fn empty_text_filters_to_everything() {
        let cb = combo(20, &["Green", "Grey", "Blue"]);
        assert_eq!(cb.filtered_by(cb.text()).len(), 3);
    }

    #[test]
    fn selected_index_is_an_exact_case_insensitive_match() {
        let mut cb = combo(20, &["Green", "Grey"]);
        cb.set_text("GREEN");
        assert_eq!(cb.selected_index(), Some(0));
        cb.set_text("gre");
        assert_eq!(cb.selected_index(), None, "prefix alone isn't a match");
        cb.set_text("something else");
        assert_eq!(cb.selected_index(), None);
    }

    #[test]
    fn bounds_height_reflects_open_state_and_visible_row_cap() {
        let mut cb = combo(10, &["a", "b", "c"]).max_visible(2);
        assert_eq!(cb.bounds().height(), 1, "closed is always one row");
        key(&mut cb, KeyCode::Down);
        assert_eq!(cb.bounds().height(), 3, "1 header + 2 capped rows");
    }

    #[test]
    fn bounds_stays_one_row_open_with_zero_matches() {
        let mut cb = combo(10, &["a", "b"]);
        cb.set_text("zzz"); // matches nothing
        key(&mut cb, KeyCode::Down);
        assert!(cb.is_open());
        assert_eq!(cb.bounds().height(), 1, "no rows to add");
    }

    #[test]
    fn empty_items_never_panics() {
        let mut cb = combo(10, &[]);
        assert_eq!(key(&mut cb, KeyCode::Down), EventResult::Consumed);
        assert_eq!(cb.selected_index(), None);
    }

    // --- Typing vs. navigation: the two ways text changes ---

    #[test]
    fn typing_opens_and_narrows_without_moving_text_itself() {
        let mut cb = combo(20, &["Green", "Grey", "Blue"]);
        assert!(!cb.is_open());
        type_str(&mut cb, "gr");
        assert!(cb.is_open());
        assert_eq!(cb.text(), "gr", "typing never rewrites what was typed");
        assert_eq!(cb.filtered_by(cb.text()).len(), 2);
    }

    #[test]
    fn down_when_closed_opens_and_previews_the_first_match_in_one_press() {
        let mut cb = combo(20, &["Green", "Grey", "Blue"]);
        key(&mut cb, KeyCode::Down);
        assert!(cb.is_open());
        assert_eq!(cb.text(), "Green");
    }

    #[test]
    fn down_and_up_move_the_preview_and_clamp_at_the_ends() {
        let mut cb = combo(20, &["Aa", "Bb", "Cc"]);
        key(&mut cb, KeyCode::Down);
        assert_eq!(cb.text(), "Aa");
        key(&mut cb, KeyCode::Down);
        assert_eq!(cb.text(), "Bb");
        key(&mut cb, KeyCode::Down);
        assert_eq!(cb.text(), "Cc");
        key(&mut cb, KeyCode::Down); // clamped at the last
        assert_eq!(cb.text(), "Cc");
        key(&mut cb, KeyCode::Up);
        assert_eq!(cb.text(), "Bb");
    }

    #[test]
    fn down_then_up_returns_to_the_starting_preview() {
        // Proves ComboBox's own index math drives this, not ListBox's: a
        // freshly-rebuilt ListBox auto-selects row 0, which would make the
        // very first Down skip straight to row 1 if delegated directly.
        let mut cb = combo(20, &["Aa", "Bb", "Cc"]);
        key(&mut cb, KeyCode::Down); // -> Aa (row 0)
        key(&mut cb, KeyCode::Down); // -> Bb (row 1)
        key(&mut cb, KeyCode::Up); // back to Aa (row 0)
        assert_eq!(cb.text(), "Aa");
    }

    #[test]
    fn up_while_closed_is_ignored() {
        let mut cb = combo(20, &["Aa", "Bb"]);
        assert_eq!(key(&mut cb, KeyCode::Up), EventResult::Ignored);
        assert!(!cb.is_open());
    }

    // --- Enter / Esc ---

    #[test]
    fn enter_while_open_closes_without_reverting_and_is_consumed() {
        let mut cb = combo(20, &["Green", "Grey"]);
        key(&mut cb, KeyCode::Down); // preview "Green"
        assert_eq!(key(&mut cb, KeyCode::Enter), EventResult::Consumed);
        assert!(!cb.is_open());
        assert_eq!(cb.text(), "Green");
    }

    #[test]
    fn enter_while_closed_is_ignored_and_bubbles() {
        let mut cb = combo(20, &["Green", "Grey"]);
        assert_eq!(key(&mut cb, KeyCode::Enter), EventResult::Ignored);
    }

    #[test]
    fn esc_while_open_reverts_a_navigation_preview() {
        let mut cb = combo(20, &["Green", "Grey"]);
        type_str(&mut cb, "gr");
        key(&mut cb, KeyCode::Down); // previews "Green" or "Grey"
        assert_ne!(cb.text(), "gr");
        assert_eq!(key(&mut cb, KeyCode::Esc), EventResult::Consumed);
        assert!(!cb.is_open());
        assert_eq!(cb.text(), "gr", "reverted to what was actually typed");
    }

    #[test]
    fn esc_after_typing_with_no_navigation_is_a_no_op_revert() {
        let mut cb = combo(20, &["Green", "Grey"]);
        type_str(&mut cb, "gr");
        key(&mut cb, KeyCode::Esc);
        assert!(!cb.is_open());
        assert_eq!(cb.text(), "gr", "nothing to undo, text unchanged");
    }

    #[test]
    fn esc_while_closed_is_ignored_and_bubbles() {
        let mut cb = combo(20, &["Green", "Grey"]);
        assert_eq!(key(&mut cb, KeyCode::Esc), EventResult::Ignored);
    }

    // --- filterable(false): type-ahead jump instead of narrowing ---

    #[test]
    fn non_filterable_typing_never_narrows_and_preserves_what_was_typed() {
        let mut cb = combo(20, &["Green", "Grey", "Blue"]).filterable(false);
        type_str(&mut cb, "gr");
        assert!(cb.is_open());
        assert_eq!(cb.text(), "gr", "typed text is never overwritten");
        assert_eq!(
            cb.matches.len(),
            3,
            "the full list stays, never narrowed to matches"
        );
    }

    #[test]
    fn non_filterable_typing_jumps_the_highlight_to_the_first_match() {
        let mut cb = combo(20, &["Blue", "Green", "Grey"]).filterable(false);
        type_str(&mut cb, "gr");
        assert_eq!(
            cb.list.selected_text(),
            Some("Green"),
            "highlight jumped to the first candidate starting with 'gr'"
        );
        // Confirmed via the list's own visual selection, not the field text
        // (which stays exactly what was typed, per the test above).
        assert_eq!(cb.text(), "gr");
    }

    #[test]
    fn non_filterable_with_no_match_leaves_the_highlight_alone() {
        let mut cb = combo(20, &["Blue", "Green"]).filterable(false);
        type_str(&mut cb, "z");
        assert_eq!(cb.text(), "z");
        assert_eq!(cb.list.selected_text(), None);
    }

    #[test]
    fn filterable_defaults_to_true_and_is_unaffected_by_the_new_flag_unless_set() {
        let mut cb = combo(20, &["Green", "Grey", "Blue"]);
        type_str(&mut cb, "gr");
        assert_eq!(cb.matches.len(), 2, "default behaviour still narrows");
    }

    // --- select_only(true): locked to a value from `items` ---

    #[test]
    fn select_only_ignores_printable_keys_as_free_text() {
        let mut cb = combo(20, &["Green", "Grey"]).select_only(true);
        type_str(&mut cb, "x");
        assert_ne!(cb.text(), "x", "never inserted as free text");
    }

    #[test]
    fn select_only_typing_opens_and_jumps_to_the_first_match() {
        let mut cb = combo(20, &["Blue", "Green", "Grey"]).select_only(true);
        key(&mut cb, KeyCode::Char('g'));
        assert!(cb.is_open());
        assert_eq!(cb.text(), "Green", "the match itself is the display value");
    }

    #[test]
    fn select_only_backspace_shortens_the_search_and_rejumps() {
        let mut cb = combo(20, &["Blue", "Green", "Grey"]).select_only(true);
        key(&mut cb, KeyCode::Char('g'));
        key(&mut cb, KeyCode::Char('r'));
        key(&mut cb, KeyCode::Char('e'));
        key(&mut cb, KeyCode::Char('y')); // "grey" matches only "Grey"
        assert_eq!(cb.text(), "Grey");
        key(&mut cb, KeyCode::Backspace); // back to "gre" -> first match "Green"
        assert_eq!(cb.text(), "Green");
    }

    #[test]
    fn select_only_search_resets_on_reopen() {
        let mut cb = combo(20, &["Blue", "Green", "Grey"]).select_only(true);
        type_str(&mut cb, "gre"); // -> "Green"
        key(&mut cb, KeyCode::Enter); // closes, keeps "Green"
        assert!(!cb.is_open());
        key(&mut cb, KeyCode::Char('b')); // fresh search, not "greb"
        assert_eq!(cb.text(), "Blue");
    }

    #[test]
    fn select_only_combined_with_non_filterable_still_shows_the_full_list() {
        let mut cb = combo(20, &["Blue", "Green", "Grey"])
            .select_only(true)
            .filterable(false);
        key(&mut cb, KeyCode::Char('g'));
        assert_eq!(cb.text(), "Green");
        assert_eq!(cb.matches.len(), 3, "full list, never narrowed");
    }

    #[test]
    fn select_only_paste_is_ignored() {
        let mut cb = combo(20, &["Green"]).select_only(true);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = cb.handle_event(&Event::Paste("Green".to_string()), &mut ctx);
        assert_eq!(r, EventResult::Ignored);
        assert_eq!(cb.text(), "");
    }

    #[test]
    fn select_only_click_in_the_text_area_opens_instead_of_placing_a_cursor() {
        let mut cb = combo(20, &["Green", "Grey"]).select_only(true);
        assert!(!cb.is_open());
        mouse(&mut cb, MouseKind::Down(MouseButton::Left), 2, 0);
        assert!(cb.is_open());
    }

    // --- Mouse ---

    #[test]
    fn a_click_on_a_suggestion_row_commits_its_text_and_closes() {
        let mut cb = combo(20, &["Aa", "Bb", "Cc"]);
        key(&mut cb, KeyCode::Down); // opens, 3 rows now under the header
        assert!(cb.is_open());
        // Row 1 (local list y = 1) is "Bb"; header is at y = 0.
        mouse(&mut cb, MouseKind::Down(MouseButton::Left), 0, 2);
        assert_eq!(cb.text(), "Bb");
        assert!(!cb.is_open());
    }

    #[test]
    fn a_click_on_the_drop_arrow_opens_with_no_preview_then_closes_on_second_click() {
        let mut cb = combo(10, &["Aa", "Bb"]);
        let arrow_x = 9; // width 10, arrow in the last column
        mouse(&mut cb, MouseKind::Down(MouseButton::Left), arrow_x, 0);
        assert!(cb.is_open());
        assert_eq!(cb.text(), "", "browsing only, nothing previewed yet");
        mouse(&mut cb, MouseKind::Down(MouseButton::Left), arrow_x, 0);
        assert!(!cb.is_open());
        assert_eq!(cb.text(), "");
    }

    #[test]
    fn a_click_inside_the_text_only_moves_the_cursor_never_toggling_open() {
        let mut cb = combo(20, &["Aa", "Bb"]);
        type_str(&mut cb, "hello");
        assert!(cb.is_open(), "typing opened it");
        key(&mut cb, KeyCode::Esc); // close it again, text reverts to "hello"
        assert!(!cb.is_open());
        mouse(&mut cb, MouseKind::Down(MouseButton::Left), 2, 0);
        assert!(
            !cb.is_open(),
            "a click in the text never opens the dropdown"
        );
    }

    #[test]
    fn the_wheel_over_an_open_list_pans_without_changing_text_or_highlight() {
        let labels: Vec<String> = (0..10).map(|i| format!("Item{i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut cb = combo(20, &refs).max_visible(3);
        key(&mut cb, KeyCode::Down); // opens, previews "Item0"
        assert_eq!(cb.text(), "Item0");
        mouse(&mut cb, MouseKind::ScrollDown, 0, 1);
        assert_eq!(cb.text(), "Item0", "the wheel never changes the preview");
        assert!(cb.is_open());
    }

    // --- Focus ---

    #[test]
    fn losing_focus_while_open_closes_without_reverting() {
        let mut cb = combo(20, &["Green", "Grey"]);
        type_str(&mut cb, "gr");
        assert!(cb.is_open());
        cb.set_focused(false);
        assert!(!cb.is_open());
        assert_eq!(cb.text(), "gr");
    }

    // --- Z-order over an ordinary sibling (ADR 0030) ---

    #[test]
    fn wants_topmost_tracks_open() {
        let mut cb = combo(20, &["Aa"]);
        assert!(!cb.wants_topmost());
        key(&mut cb, KeyCode::Down);
        assert!(cb.wants_topmost());
    }

    #[test]
    fn an_open_drop_down_draws_over_and_is_clicked_ahead_of_a_later_sibling_button() {
        // A Button positioned directly under the combo box, exactly where its
        // one-row drop-down would land once open — an ordinary `Group` would
        // draw the later-inserted Button on top and hand it the click.
        let theme = Theme::default();
        let mut cb = ComboBox::new(rect(20), items(&["Aa"]), &theme);
        cb.set_focused(true);
        key(&mut cb, KeyCode::Down); // opens: bounds now 2 rows tall
        assert!(cb.is_open());

        let button_area = Rect::from_origin_size(Point::new(0, 1), Size::new(20, 1));
        let button = Button::new(button_area, "OK", CM_OK, &theme);
        let mut group = Group::new(
            Rect::from_origin_size(Point::new(0, 0), Size::new(20, 2)),
            vec![Box::new(cb), Box::new(button)],
        );

        let mut buf = Buffer::new(Size::new(20, 2));
        let mut canvas = Canvas::new(&mut buf);
        group.draw(&mut canvas);
        assert_eq!(
            buf.get(Point::new(0, 1)).unwrap().grapheme().to_string(),
            "A",
            "the drop-down row painted over the button beneath it"
        );

        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        group.handle_event(
            &Event::Mouse(MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                pos: Point::new(0, 1),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        );
        assert!(
            ctx.posted().is_empty(),
            "the click picked the suggestion row, not the button underneath it"
        );
    }

    // --- Builders ---

    #[test]
    fn with_text_seeds_the_field_closed() {
        let cb = ComboBox::new(rect(20), items(&["Green", "Grey"]), &Theme::default())
            .with_text("Green");
        assert_eq!(cb.text(), "Green");
        assert!(!cb.is_open());
        assert_eq!(cb.selected_index(), Some(0));
    }

    // --- Render (snapshot) ---

    fn render(cb: &ComboBox) -> String {
        let size = cb.bounds().size();
        let mut buf = Buffer::new(size);
        let mut canvas = Canvas::new(&mut buf);
        cb.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn snapshot_closed_field() {
        let cb = combo(16, &["Green", "Grey", "Blue"]);
        insta::assert_snapshot!(render(&cb));
    }

    #[test]
    fn snapshot_open_with_filtered_rows_and_a_preview() {
        let mut cb = combo(16, &["Green", "Grey", "Blue"]);
        type_str(&mut cb, "gr");
        key(&mut cb, KeyCode::Down);
        insta::assert_snapshot!(render(&cb));
    }

    #[test]
    fn snapshot_open_scrolled_past_max_visible() {
        let labels: Vec<String> = (0..10).map(|i| format!("Item{i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut cb = combo(16, &refs).max_visible(3);
        key(&mut cb, KeyCode::Down);
        key(&mut cb, KeyCode::Down);
        key(&mut cb, KeyCode::Down);
        key(&mut cb, KeyCode::Down); // scrolls the embedded ListBox
        insta::assert_snapshot!(render(&cb));
    }

    #[test]
    fn snapshot_open_with_zero_matches() {
        let mut cb = combo(16, &["Green", "Grey"]);
        type_str(&mut cb, "zzz");
        insta::assert_snapshot!(render(&cb));
    }
}
