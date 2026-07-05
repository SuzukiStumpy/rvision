//! A scrollable single-selection list (TurboVision's `TListBox`/`TListViewer`).
//!
//! A focusable [control](super) showing one item per row. `Up`/`Down` move the
//! selection by one, `PageUp`/`PageDown` by a screenful, `Home`/`End` to the
//! ends; the view always scrolls to keep the selection visible. `Enter` is left
//! to bubble so the dialog's default button (e.g. *Open*) acts on the selected
//! item. A left **double-click** selects the row like a single click; turning
//! that into an "activate" (the Enter equivalent) is the container's job — see
//! [`FileDialog`](super::FileDialog) (ADR 0007).
//!
//! `ListBox` does not draw or hit-test its own scroll bar: it reports
//! [`scroll_metrics`](View::scroll_metrics) and accepts
//! [`set_scroll`](View::set_scroll) instead, so whoever composes it (e.g.
//! [`FileDialog`](super::FileDialog)) hosts a [`ScrollBar`](super::ScrollBar)
//! for it (ADR 0015). The mouse wheel still pans the list directly — only the
//! *bar* moved out, not wheel scrolling.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{AxisMetrics, Context, ScrollMetrics, View};

/// A scrollable list with one selected row.
pub struct ListBox {
    bounds: Rect,
    items: Vec<String>,
    selected: Option<usize>,
    top: usize,
    focused: bool,
    /// Whether the selected row stays visibly marked even while unfocused
    /// (dimmer than the focused highlight) rather than reading as an
    /// ordinary row — opt-in, so existing consumers are unaffected
    /// (ADR 0020 addendum).
    always_show_selection: bool,
    style: Style,
    focus_style: Style,
    inactive_focus_style: Style,
}

impl ListBox {
    /// Creates a list at `bounds` over `items`; the first item starts selected
    /// (or nothing, if the list is empty).
    pub fn new(bounds: Rect, items: Vec<String>, theme: &Theme) -> Self {
        let selected = (!items.is_empty()).then_some(0);
        Self {
            bounds,
            items,
            selected,
            top: 0,
            focused: false,
            always_show_selection: false,
            style: theme.style(Role::Input),
            focus_style: theme.style(Role::Selection),
            inactive_focus_style: theme.style(Role::SelectionInactive),
        }
    }

    /// Keeps the selected row visibly marked (dimmer than the focused
    /// highlight) even while this list isn't focused, instead of the default
    /// of reading as an ordinary row — for a list whose "what's current
    /// here?" answer matters to more than just itself, e.g.
    /// [`HelpWindow`](super::HelpWindow)'s topic list while its page pane
    /// holds focus (ADR 0020 addendum).
    pub fn always_show_selection(mut self, yes: bool) -> Self {
        self.always_show_selection = yes;
        self
    }

    /// The index of the selected item, if any.
    pub fn selected(&self) -> Option<usize> {
        self.selected
    }

    /// The text of the selected item, if any.
    pub fn selected_text(&self) -> Option<&str> {
        self.selected
            .and_then(|i| self.items.get(i))
            .map(String::as_str)
    }

    /// Clears the selection outright — unlike `new` (which auto-selects the
    /// first item) and `select` (which always lands on a real index), this
    /// leaves nothing highlighted. For a consumer that rebuilds a `ListBox`
    /// from scratch as its content changes (e.g.
    /// [`ComboBox`](super::ComboBox)'s type-ahead search) and needs to show
    /// "no candidate matches" rather than a construction-default row 0.
    pub fn deselect(&mut self) {
        self.selected = None;
    }

    /// Selects item `index` (clamped to the last item), scrolling it into view.
    /// A no-op on an empty list.
    pub fn select(&mut self, index: usize) {
        if self.items.is_empty() {
            return;
        }
        self.selected = Some(index.min(self.items.len() - 1));
        self.ensure_visible();
    }

    /// The number of fully visible rows.
    fn rows(&self) -> usize {
        self.bounds.height().max(0) as usize
    }

    /// Moves the selection by `delta` rows (clamped), then scrolls it into view.
    fn move_by(&mut self, delta: isize) {
        if self.items.is_empty() {
            return;
        }
        let last = self.items.len() as isize - 1;
        let current = self.selected.unwrap_or(0) as isize;
        let next = (current + delta).clamp(0, last) as usize;
        self.selected = Some(next);
        self.ensure_visible();
    }

    /// Scrolls so the selected row is within the visible window.
    fn ensure_visible(&mut self) {
        let rows = self.rows().max(1);
        if let Some(sel) = self.selected {
            if sel < self.top {
                self.top = sel;
            } else if sel >= self.top + rows {
                self.top = sel + 1 - rows;
            }
        }
    }

    /// Repositions/resizes the list, keeping the selection visible and
    /// clamping the scroll offset to what the new height can show (ADR 0017)
    /// — the same relayout-on-resize shape as [`HelpPane::set_bounds`](super::HelpPane::set_bounds).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        let max_top = self.items.len().saturating_sub(self.rows());
        self.top = self.top.min(max_top);
        self.ensure_visible();
    }

    /// Scrolls the view by `delta` rows (negative = up) **without** moving the
    /// selection — the wheel and scroll bar pan the list. Clamped so the last
    /// screenful never scrolls off the bottom.
    fn scroll_by(&mut self, delta: isize) {
        let max_top = self.items.len().saturating_sub(self.rows()) as isize;
        self.top = ((self.top as isize) + delta).clamp(0, max_top) as usize;
    }

    /// Handles a mouse event (in the list's local coordinates): a click selects
    /// the row under the pointer; the wheel pans the view a row at a time.
    /// Scroll-bar hit-testing is a host's job now (ADR 0015), not the list's.
    fn handle_mouse(&mut self, m: &MouseEvent) -> EventResult {
        match m.kind {
            // A double-click selects like a click; the container turns it into an
            // "activate" (its Enter path).
            MouseKind::Down(MouseButton::Left) | MouseKind::DoubleClick(MouseButton::Left) => {
                if m.pos.y >= 0 {
                    let idx = self.top + m.pos.y as usize;
                    if idx < self.items.len() {
                        self.selected = Some(idx);
                    }
                }
                EventResult::Consumed
            }
            MouseKind::ScrollDown => {
                self.scroll_by(1);
                EventResult::Consumed
            }
            MouseKind::ScrollUp => {
                self.scroll_by(-1);
                EventResult::Consumed
            }
            _ => EventResult::Ignored,
        }
    }
}

impl View for ListBox {
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

        for r in 0..rows {
            let idx = self.top + r;
            if idx >= self.items.len() {
                break;
            }
            let row_style = if self.selected != Some(idx) {
                self.style
            } else if self.focused {
                self.focus_style
            } else if self.always_show_selection {
                self.inactive_focus_style
            } else {
                self.style
            };
            let row = Rect::from_origin_size(Point::new(0, r as i16), Size::new(area.width(), 1));
            canvas.fill(row, &Cell::blank(row_style));
            canvas.put_str(Point::new(0, r as i16), &self.items[idx], row_style);
        }
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
        if let Event::Mouse(m) = event {
            return self.handle_mouse(m);
        }
        if let Event::Key(key) = event {
            if self.focused {
                let page = self.rows().max(1) as isize;
                match key.code {
                    KeyCode::Up => {
                        self.move_by(-1);
                        return EventResult::Consumed;
                    }
                    KeyCode::Down => {
                        self.move_by(1);
                        return EventResult::Consumed;
                    }
                    KeyCode::PageUp => {
                        self.move_by(-page);
                        return EventResult::Consumed;
                    }
                    KeyCode::PageDown => {
                        self.move_by(page);
                        return EventResult::Consumed;
                    }
                    KeyCode::Home => {
                        self.move_by(isize::MIN / 2);
                        return EventResult::Consumed;
                    }
                    KeyCode::End => {
                        self.move_by(isize::MAX / 2);
                        return EventResult::Consumed;
                    }
                    _ => {}
                }
            }
        }
        EventResult::Ignored
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn scroll_metrics(&self) -> Option<ScrollMetrics> {
        let rows = self.rows();
        if self.items.len() <= rows {
            return None;
        }
        Some(ScrollMetrics {
            horizontal: None,
            vertical: Some(AxisMetrics {
                total: self.items.len(),
                visible: rows,
                pos: self.top,
            }),
        })
    }

    fn set_scroll(&mut self, offset: Point) {
        let max_top = self.items.len().saturating_sub(self.rows());
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
    use crate::event::{KeyEvent, Modifiers};

    fn items(labels: &[&str]) -> Vec<String> {
        labels.iter().map(|s| s.to_string()).collect()
    }

    fn rect(w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(w, h))
    }

    fn list(w: i16, h: i16, labels: &[&str]) -> ListBox {
        let mut lb = ListBox::new(rect(w, h), items(labels), &Theme::default());
        lb.set_focused(true);
        lb
    }

    fn press(lb: &mut ListBox, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        lb.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn mouse(lb: &mut ListBox, kind: MouseKind, x: i16, y: i16) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        lb.handle_event(
            &Event::Mouse(MouseEvent {
                kind,
                pos: Point::new(x, y),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        )
    }

    #[test]
    fn clicking_a_row_selects_that_item() {
        let mut lb = list(10, 4, &["a", "b", "c", "d"]);
        mouse(&mut lb, MouseKind::Down(MouseButton::Left), 2, 2);
        assert_eq!(lb.selected(), Some(2));
        assert_eq!(lb.selected_text(), Some("c"));
    }

    #[test]
    fn double_clicking_a_row_selects_it_like_a_click() {
        // The "activate" (Enter-equivalent) is the container's job; the list just
        // makes sure the right row is selected when the double-click lands.
        let mut lb = list(10, 4, &["a", "b", "c", "d"]);
        mouse(&mut lb, MouseKind::DoubleClick(MouseButton::Left), 2, 1);
        assert_eq!(lb.selected(), Some(1));
    }

    #[test]
    fn clicking_below_the_last_item_does_nothing() {
        let mut lb = list(10, 6, &["a", "b"]);
        mouse(&mut lb, MouseKind::Down(MouseButton::Left), 0, 4); // blank row
        assert_eq!(lb.selected(), Some(0), "selection unchanged");
    }

    #[test]
    fn the_wheel_scrolls_the_list_without_moving_the_selection() {
        let mut lb = list(10, 3, &["a", "b", "c", "d", "e", "f"]);
        assert_eq!(lb.top, 0);
        mouse(&mut lb, MouseKind::ScrollDown, 5, 1);
        assert_eq!(lb.top, 1);
        assert_eq!(
            lb.selected(),
            Some(0),
            "the wheel does not move the selection"
        );
    }

    #[test]
    fn a_click_in_the_former_bar_column_now_just_selects_that_row() {
        // ListBox no longer reserves or hit-tests a bar column of its own
        // (ADR 0015) — every column, including the last, is content.
        let mut lb = list(10, 3, &["a", "b", "c", "d", "e", "f"]);
        mouse(&mut lb, MouseKind::Down(MouseButton::Left), 9, 2);
        assert_eq!(lb.selected(), Some(2));
        assert_eq!(lb.top, 0, "a plain click never scrolls");
    }

    #[test]
    fn scroll_metrics_is_none_under_a_page_and_some_once_it_overflows() {
        let short = list(10, 4, &["a", "b"]);
        assert_eq!(short.scroll_metrics(), None);

        let long = list(10, 3, &["a", "b", "c", "d", "e", "f"]);
        assert_eq!(
            long.scroll_metrics(),
            Some(ScrollMetrics {
                horizontal: None,
                vertical: Some(AxisMetrics {
                    total: 6,
                    visible: 3,
                    pos: 0,
                }),
            })
        );
    }

    #[test]
    fn set_scroll_clamps_and_moves_top_without_touching_selection() {
        let mut lb = list(10, 3, &["a", "b", "c", "d", "e", "f"]);
        lb.set_scroll(Point::new(0, 2));
        assert_eq!(lb.top, 2);
        assert_eq!(
            lb.selected(),
            Some(0),
            "set_scroll never moves the selection"
        );

        lb.set_scroll(Point::new(0, 99)); // clamps to the last full page
        assert_eq!(lb.top, 3);
        lb.set_scroll(Point::new(0, -5)); // clamps at zero, no panic/underflow
        assert_eq!(lb.top, 0);
    }

    fn render(lb: &ListBox, w: i16, h: i16) -> String {
        let mut buf = Buffer::new(Size::new(w, h));
        let mut canvas = Canvas::new(&mut buf);
        lb.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn select_sets_the_index_clamped_and_scrolls_into_view() {
        let mut lb = list(6, 3, &["a", "b", "c", "d", "e", "f"]);
        lb.select(4);
        assert_eq!(lb.selected(), Some(4));
        assert!(lb.top > 0, "scrolled to keep item 4 visible");
        lb.select(99); // clamps to the last
        assert_eq!(lb.selected(), Some(5));
        let mut empty = ListBox::new(rect(6, 3), vec![], &Theme::default());
        empty.select(2); // no panic, still nothing selected
        assert_eq!(empty.selected(), None);
    }

    #[test]
    fn an_unfocused_list_draws_no_highlight_by_default() {
        let mut lb = list(10, 4, &["a", "b", "c"]);
        lb.set_focused(false);
        let text = render(&lb, 10, 4);
        // No easy way to assert "no highlight" from `to_text()` alone (it
        // drops style); assert via the same style-inspection approach the
        // opted-in case below uses.
        let mut buf = Buffer::new(Size::new(10, 4));
        let mut canvas = Canvas::new(&mut buf);
        lb.draw(&mut canvas);
        let theme = Theme::default();
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().style(),
            theme.style(Role::Input),
            "unfocused, opted-out: the selected row reads as an ordinary row"
        );
        let _ = text;
    }

    #[test]
    fn always_show_selection_dims_the_current_row_instead_of_hiding_it() {
        let mut lb = ListBox::new(rect(10, 4), items(&["a", "b", "c"]), &Theme::default())
            .always_show_selection(true);
        lb.set_focused(false);
        let theme = Theme::default();
        let mut buf = Buffer::new(Size::new(10, 4));
        let mut canvas = Canvas::new(&mut buf);
        lb.draw(&mut canvas);
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().style(),
            theme.style(Role::SelectionInactive),
            "unfocused, opted-in: still visibly the current row, just dimmer"
        );

        lb.set_focused(true);
        let mut buf2 = Buffer::new(Size::new(10, 4));
        let mut canvas2 = Canvas::new(&mut buf2);
        lb.draw(&mut canvas2);
        assert_eq!(
            buf2.get(Point::new(0, 0)).unwrap().style(),
            theme.style(Role::Selection),
            "focused: the ordinary bright highlight, unchanged"
        );
    }

    #[test]
    fn first_item_is_selected_and_empty_has_none() {
        assert_eq!(list(10, 4, &["a", "b"]).selected(), Some(0));
        let empty = ListBox::new(rect(10, 4), vec![], &Theme::default());
        assert_eq!(empty.selected(), None);
        assert_eq!(empty.selected_text(), None);
    }

    #[test]
    fn deselect_clears_the_construction_default_selection() {
        let mut lb = list(10, 4, &["a", "b", "c"]);
        assert_eq!(lb.selected(), Some(0), "new() auto-selects the first item");
        lb.deselect();
        assert_eq!(lb.selected(), None);
        assert_eq!(lb.selected_text(), None);
    }

    #[test]
    fn deselect_then_select_lands_on_a_real_index_again() {
        let mut lb = list(10, 4, &["a", "b", "c"]);
        lb.deselect();
        lb.select(1);
        assert_eq!(lb.selected(), Some(1));
    }

    #[test]
    fn arrows_move_the_selection_and_clamp() {
        let mut lb = list(10, 4, &["a", "b", "c"]);
        press(&mut lb, KeyCode::Down);
        assert_eq!(lb.selected(), Some(1));
        press(&mut lb, KeyCode::Down);
        press(&mut lb, KeyCode::Down); // clamps at the last
        assert_eq!(lb.selected(), Some(2));
        assert_eq!(lb.selected_text(), Some("c"));
        press(&mut lb, KeyCode::Up);
        assert_eq!(lb.selected(), Some(1));
    }

    #[test]
    fn home_end_and_page_jump() {
        let mut lb = list(10, 3, &["a", "b", "c", "d", "e", "f"]);
        press(&mut lb, KeyCode::End);
        assert_eq!(lb.selected(), Some(5));
        press(&mut lb, KeyCode::Home);
        assert_eq!(lb.selected(), Some(0));
        press(&mut lb, KeyCode::PageDown); // a page is 3 rows
        assert_eq!(lb.selected(), Some(3));
    }

    #[test]
    fn scrolls_to_keep_the_selection_visible() {
        // 6 items in a 3-row box: End shows the tail (d, e, f), full width —
        // no bar of its own to make room for (ADR 0015).
        let mut lb = list(6, 3, &["aa", "bb", "cc", "dd", "ee", "ff"]);
        press(&mut lb, KeyCode::End);
        let text = render(&lb, 6, 3);
        let rows: Vec<&str> = text.lines().collect();
        assert!(rows[0].starts_with("dd"));
        assert!(rows[1].starts_with("ee"));
        assert!(rows[2].starts_with("ff"));
    }

    #[test]
    fn enter_bubbles_for_the_default_button() {
        let mut lb = list(10, 4, &["a", "b"]);
        assert_eq!(press(&mut lb, KeyCode::Enter), EventResult::Ignored);
    }

    #[test]
    fn snapshot_list_full_width_no_bar_of_its_own() {
        let lb = list(12, 3, &["alpha", "beta", "gamma", "delta", "epsilon"]);
        insta::assert_snapshot!(render(&lb, 12, 3));
    }

    // --- Resize propagation (ADR 0017) ---

    #[test]
    fn set_bounds_updates_the_row_count() {
        let mut lb = list(10, 3, &["a", "b", "c", "d", "e", "f"]);
        assert_eq!(lb.rows(), 3);
        lb.set_bounds(rect(10, 6));
        assert_eq!(lb.rows(), 6);
    }

    #[test]
    fn set_bounds_clamps_top_when_the_list_grows_past_its_scrolled_offset() {
        let labels: Vec<String> = (0..20).map(|i| format!("L{i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut lb = list(10, 5, &refs);
        press(&mut lb, KeyCode::End); // scrolls to the bottom: top = 15
        assert_eq!(lb.top, 15);
        // Growing tall enough to show every item leaves nothing to scroll.
        lb.set_bounds(rect(10, 20));
        assert_eq!(lb.top, 0);
    }

    #[test]
    fn set_bounds_keeps_the_selection_visible_when_the_list_shrinks() {
        let labels: Vec<String> = (0..20).map(|i| format!("L{i}")).collect();
        let refs: Vec<&str> = labels.iter().map(String::as_str).collect();
        let mut lb = list(10, 10, &refs);
        lb.select(9);
        assert_eq!(lb.top, 0, "item 9 already fit in 10 rows");
        lb.set_bounds(rect(10, 3));
        assert_eq!(
            lb.top, 7,
            "scrolled so item 9 is still the last visible row"
        );
    }

    #[test]
    fn set_bounds_via_the_view_trait_reaches_the_same_logic() {
        let mut lb = list(10, 5, &["a", "b", "c", "d", "e", "f"]);
        let view: &mut dyn View = &mut lb;
        view.set_bounds(rect(10, 2));
        assert_eq!(lb.rows(), 2);
    }
}
