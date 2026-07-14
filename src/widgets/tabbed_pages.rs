//! A tab strip over a set of pages, one shown at a time
//! (`docs/specs/tabbed_pages.md`).
//!
//! Distinct from [`GroupBox`](super::GroupBox) (a single always-visible
//! bordered group, no switching) and from [`Desktop`](super::Desktop)/
//! [`WindowList`](super::WindowList) (window management — this widget has
//! no `Desktop` awareness). Each page owns exactly one arbitrary [`View`];
//! a page needing several controls is the caller's own `Group`/`GroupBox`,
//! built before it's handed to `TabbedPages`.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::command::Command;
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};
use std::ops::Range;

/// One tab: its label and the page it shows.
struct Tab {
    title: String,
    view: Box<dyn View>,
}

/// Which of `TabbedPages`'s two focus-participating slots currently holds
/// keyboard focus: the strip itself, or the active page's content. The
/// strip is never a stored `View` — it's chrome this widget draws and
/// hit-tests directly, so this can't be a plain `Group` index.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Strip,
    Page,
}

/// A titled tab strip over one arbitrary page per tab.
pub struct TabbedPages {
    bounds: Rect,
    tabs: Vec<Tab>,
    current: usize,
    focus: Focus,
    /// Whether this widget itself currently holds the owner's keyboard
    /// focus (pushed via `set_focused`) — independent of `focus`, which
    /// only tracks *which slot* would take a key if this widget has it.
    has_focus: bool,
    /// Per-tab local-column hit-test/draw spans, recomputed whenever
    /// `bounds`'s width changes.
    tab_columns: Vec<Range<i16>>,
    style: Style,
    interior_fill: Cell,
    active_style: Style,
    active_inactive_style: Style,
}

impl TabbedPages {
    /// Creates a tab strip + bordered page area at `bounds`, one page per
    /// `(title, view)` pair in `tabs`, in that order — index 0 starts
    /// active, and the strip (not the page) starts holding keyboard focus,
    /// mirroring `Group::new`'s "focus starts on the first focusable slot"
    /// rule (the strip is always slot 0). Border/strip/interior fill
    /// resolve [`Role::DialogBackground`]; the active tab resolves
    /// [`Role::Selection`]/[`Role::SelectionInactive`] — the same roles
    /// [`ListBox`](super::ListBox)'s "always show current item" mode uses.
    pub fn new(bounds: Rect, tabs: Vec<(&str, Box<dyn View>)>, theme: &Theme) -> Self {
        let style = theme.style(Role::DialogBackground);
        let tabs: Vec<Tab> = tabs
            .into_iter()
            .map(|(title, view)| Tab {
                title: title.to_string(),
                view,
            })
            .collect();
        let mut widget = Self {
            bounds,
            tabs,
            current: 0,
            focus: Focus::Strip,
            has_focus: false,
            tab_columns: Vec::new(),
            style,
            interior_fill: Cell::blank(style),
            active_style: theme.style(Role::Selection),
            active_inactive_style: theme.style(Role::SelectionInactive),
        };
        widget.recompute_tab_columns();
        widget
    }

    /// Sets the initially active page (clamped to the tab count).
    pub fn with_current(mut self, index: usize) -> Self {
        self.current = Self::clamp_index(index, self.tabs.len());
        self
    }

    /// The index of the currently active/shown page.
    pub fn current(&self) -> usize {
        self.current
    }

    /// Switches the active page (clamped; a no-op if there are no tabs).
    /// Pure UI state, like `RadioButtons::selected`/`ComboBox::selected_index`
    /// — no command posted (contrast [`WindowList`](super::WindowList),
    /// which posts commands because it must ask an owner to mutate a
    /// `Desktop` it has no access to; this widget has no such need).
    pub fn select(&mut self, index: usize) {
        if self.tabs.is_empty() {
            return;
        }
        self.current = index.min(self.tabs.len() - 1);
    }

    /// The number of tabs/pages.
    pub fn tab_count(&self) -> usize {
        self.tabs.len()
    }

    /// Reaches page `index`'s view (any concrete type via `AsAny`, ADR
    /// 0036), `None` out of range.
    pub fn page(&self, index: usize) -> Option<&dyn View> {
        self.tabs.get(index).map(|tab| tab.view.as_ref())
    }

    /// The mutable counterpart of [`page`](Self::page).
    pub fn page_mut(&mut self, index: usize) -> Option<&mut dyn View> {
        self.tabs.get_mut(index).map(|tab| tab.view.as_mut())
    }

    /// The tab strip's rectangle in local coordinates: full width, row 0.
    pub fn strip_bounds(&self) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(self.bounds.width(), 1))
    }

    /// The bordered page area in local coordinates — `GroupBox`-shaped,
    /// offset down by the strip row. Collapses to empty for a widget too
    /// small to have one.
    pub fn interior_bounds(&self) -> Rect {
        Self::compute_interior_bounds(self.bounds)
    }

    fn clamp_index(index: usize, len: usize) -> usize {
        if len == 0 { 0 } else { index.min(len - 1) }
    }

    fn compute_interior_bounds(bounds: Rect) -> Rect {
        let Size { width, height } = bounds.size();
        Rect::from_origin_size(
            Point::new(1, 2),
            Size::new((width - 2).max(0), (height - 3).max(0)),
        )
    }

    /// Lays out each tab's `" title "` span left to right starting at
    /// local column 1 (aligned with the border box's left edge below),
    /// separated by one column for the `│` divider. A label too long to
    /// fit at all truncates to fill the remaining room if it's the first
    /// tab (mirrors `GroupBox`'s single-title truncation, so at least one
    /// tab always shows even in a narrow widget); a later tab that doesn't
    /// fit is simply not laid out (open question in the spec — no strip
    /// scrolling in v1).
    fn recompute_tab_columns(&mut self) {
        let width = self.bounds.width();
        let mut columns: Vec<Range<i16>> = Vec::with_capacity(self.tabs.len());
        let mut x: i16 = 1;
        for (i, tab) in self.tabs.iter().enumerate() {
            if x >= width {
                break;
            }
            let remaining = width - x;
            let full = tab.title.chars().count() as i16 + 2; // " title "
            let span = if full <= remaining {
                full
            } else if i == 0 {
                remaining
            } else {
                break;
            };
            columns.push(x..x + span);
            x += span + 1;
        }
        self.tab_columns = columns;
    }

    fn tab_at(&self, x: i16) -> Option<usize> {
        self.tab_columns.iter().position(|range| range.contains(&x))
    }

    fn strip_focused(&self) -> bool {
        self.has_focus && self.focus == Focus::Strip
    }

    /// Moves focus onto the strip, telling the currently-active page (if it
    /// held focus) that it lost it.
    fn focus_strip(&mut self) {
        if self.focus != Focus::Strip {
            if let Some(tab) = self.tabs.get_mut(self.current) {
                tab.view.set_focused(false);
            }
            self.focus = Focus::Strip;
        }
    }

    /// Moves focus onto the active page, telling it it gained focus.
    fn focus_page(&mut self) {
        if self.focus != Focus::Page {
            self.focus = Focus::Page;
            if let Some(tab) = self.tabs.get_mut(self.current) {
                tab.view.set_focused(true);
            }
        }
    }

    fn handle_mouse(&mut self, mouse: MouseEvent, ctx: &mut Context) -> EventResult {
        let strip = self.strip_bounds();
        if strip.contains(mouse.pos) {
            if matches!(mouse.kind, MouseKind::Down(MouseButton::Left)) {
                self.focus_strip();
                if let Some(i) = self.tab_at(mouse.pos.x) {
                    self.select(i);
                }
                return EventResult::Consumed;
            }
            return EventResult::Ignored;
        }

        let interior = self.interior_bounds();
        if interior.contains(mouse.pos) {
            if matches!(mouse.kind, MouseKind::Down(MouseButton::Left))
                && self
                    .tabs
                    .get(self.current)
                    .is_some_and(|tab| tab.view.focusable())
            {
                self.focus_page();
            }
            let origin = interior.origin();
            let local = MouseEvent {
                pos: mouse.pos.offset(-origin.x, -origin.y),
                ..mouse
            };
            let current = self.current;
            let tabs = &mut self.tabs;
            return ctx.translated(origin.x, origin.y, |ctx| {
                tabs[current].view.handle_event(&Event::Mouse(local), ctx)
            });
        }

        EventResult::Ignored
    }

    fn handle_key(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let code = match event {
            Event::Key(key) => key.code,
            _ => unreachable!("handle_key is only called for Event::Key"),
        };
        match self.focus {
            Focus::Page => {
                if let Some(tab) = self.tabs.get_mut(self.current) {
                    if tab.view.handle_event(event, ctx).is_consumed() {
                        return EventResult::Consumed;
                    }
                }
                if code == KeyCode::BackTab {
                    self.focus_strip();
                    return EventResult::Consumed;
                }
                // A boundary forward Tab is left `Ignored` here so it
                // escapes this widget entirely — the active page is
                // always the *last* of the two slots. Reset the page's own
                // remembered focus cursor right now (ADR 0038): otherwise a
                // future fresh re-entry via `focus_page()` would forward
                // `set_focused(true)` to whichever child the page last
                // happened to leave focused, permanently stranding any
                // focusable sibling that comes before it.
                if code == KeyCode::Tab {
                    if let Some(tab) = self.tabs.get_mut(self.current) {
                        tab.view.reset_focus();
                    }
                }
                EventResult::Ignored
            }
            Focus::Strip => match code {
                KeyCode::Left => {
                    if self.current > 0 {
                        self.select(self.current - 1);
                    }
                    EventResult::Consumed
                }
                KeyCode::Right => {
                    if self.current + 1 < self.tabs.len() {
                        self.select(self.current + 1);
                    }
                    EventResult::Consumed
                }
                KeyCode::Tab => {
                    let page_focusable = self
                        .tabs
                        .get(self.current)
                        .is_some_and(|tab| tab.view.focusable());
                    if page_focusable {
                        self.focus_page();
                        EventResult::Consumed
                    } else {
                        // The strip is the last focusable slot when the
                        // active page can't take focus: boundary escape.
                        EventResult::Ignored
                    }
                }
                // The strip is unconditionally the first slot, so
                // Shift-Tab from it always escapes outward.
                KeyCode::BackTab => EventResult::Ignored,
                _ => EventResult::Ignored,
            },
        }
    }

    fn draw_strip(&self, canvas: &mut Canvas, area: Rect) {
        let strip = Rect::from_origin_size(Point::new(0, 0), Size::new(area.width(), 1));
        canvas.fill(strip, &Cell::blank(self.style));
        let strip_focused = self.strip_focused();
        for (i, range) in self.tab_columns.iter().enumerate() {
            let style = if i == self.current {
                if strip_focused {
                    self.active_style
                } else {
                    self.active_inactive_style
                }
            } else {
                self.style
            };
            let label = format!(" {} ", self.tabs[i].title);
            let span = (range.end - range.start).max(0) as usize;
            let shown: String = label.chars().take(span).collect();
            canvas.put_str(Point::new(range.start, 0), &shown, style);
            // A divider between this tab and the next — not after the last
            // one, which would otherwise leave a stray trailing `│` (found
            // during the manual tmux pass against examples/dialogs.rs).
            let sep_x = range.end;
            if i + 1 < self.tab_columns.len() && sep_x < area.width() {
                canvas.set(Point::new(sep_x, 0), Cell::from_char('│', self.style));
            }
        }
    }

    /// Strokes a single-line box below the strip — the identical glyphs
    /// `GroupBox::draw_border` uses, offset down by the strip row, with no
    /// title of its own (the strip already shows the tab titles).
    fn draw_border(&self, canvas: &mut Canvas, area: Rect) {
        let top = 1i16;
        let bottom = area.height() - 1;
        let left = 0i16;
        let right = area.width() - 1;
        if bottom <= top || right <= left {
            return;
        }

        let h = Cell::from_char('─', self.style);
        let v = Cell::from_char('│', self.style);
        for x in left..=right {
            canvas.set(Point::new(x, top), h.clone());
            canvas.set(Point::new(x, bottom), h.clone());
        }
        for y in top..=bottom {
            canvas.set(Point::new(left, y), v.clone());
            canvas.set(Point::new(right, y), v.clone());
        }
        canvas.set(Point::new(left, top), Cell::from_char('┌', self.style));
        canvas.set(Point::new(right, top), Cell::from_char('┐', self.style));
        canvas.set(Point::new(left, bottom), Cell::from_char('└', self.style));
        canvas.set(Point::new(right, bottom), Cell::from_char('┘', self.style));
    }
}

impl View for TabbedPages {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        if area.width() <= 0 || area.height() <= 0 {
            return;
        }
        self.draw_strip(canvas, area);
        if area.height() < 2 {
            return;
        }
        self.draw_border(canvas, area);

        let interior = self.interior_bounds();
        if !interior.is_empty() {
            let mut sub = canvas.child(interior);
            let fill_area = sub.bounds();
            sub.fill(fill_area, &self.interior_fill);
            if let Some(tab) = self.tabs.get(self.current) {
                tab.view.draw(&mut sub);
            }
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        if self.tabs.is_empty() {
            return EventResult::Ignored;
        }
        match event {
            Event::Mouse(mouse) => self.handle_mouse(*mouse, ctx),
            Event::Key(_) => self.handle_key(event, ctx),
            Event::Command(_) | Event::Paste(_) => {
                if let Some(tab) = self.tabs.get_mut(self.current) {
                    tab.view.handle_event(event, ctx)
                } else {
                    EventResult::Ignored
                }
            }
            Event::Broadcast(_) | Event::Resize(_) | Event::Idle => {
                for tab in &mut self.tabs {
                    tab.view.handle_event(event, ctx);
                }
                EventResult::Ignored
            }
        }
    }

    fn focusable(&self) -> bool {
        !self.tabs.is_empty()
    }

    fn set_focused(&mut self, focused: bool) {
        if focused && !self.has_focus {
            // A fresh arrival from outside always starts at the strip —
            // this widget's canonical "front door" — regardless of which
            // slot last held focus during a previous visit. Otherwise pure
            // forward Tab cycling could resume deep page focus (e.g. via an
            // outer Group wrapping back around) and the strip would become
            // unreachable except by an explicit BackTab from a page.
            self.focus = Focus::Strip;
        }
        self.has_focus = focused;
        if self.focus == Focus::Page {
            if let Some(tab) = self.tabs.get_mut(self.current) {
                tab.view.set_focused(focused);
            }
        }
    }

    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
        self.tabs
            .iter_mut()
            .fold(true, |ok, tab| tab.view.valid(command, ctx) && ok)
    }

    fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.recompute_tab_columns();
        let interior = self.interior_bounds();
        for tab in &mut self.tabs {
            tab.view.set_bounds(interior);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::{CM_OK, CommandSet};
    use crate::event::{KeyEvent, Modifiers};
    use crate::view::StaticText;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    fn theme() -> Theme {
        Theme::default()
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, Modifiers::NONE))
    }

    fn mouse_down_at(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    fn page(text: &str) -> Box<dyn View> {
        Box::new(StaticText::new(rect(0, 0, 10, 1), text, Style::new()))
    }

    fn tabs<'a>(titles: &[&'a str]) -> Vec<(&'a str, Box<dyn View>)> {
        titles.iter().map(|t| (*t, page(t))).collect()
    }

    /// A focusable leaf that records every event it handles and its current
    /// focus flag, so tests can assert dispatch/focus precisely.
    struct Probe {
        bounds: Rect,
        focusable: bool,
        focused: std::rc::Rc<std::cell::RefCell<bool>>,
        seen: std::rc::Rc<std::cell::RefCell<Vec<Event>>>,
        consume: bool,
    }

    impl Probe {
        fn new(focusable: bool) -> Self {
            Self {
                bounds: rect(0, 0, 5, 1),
                focusable,
                focused: Default::default(),
                seen: Default::default(),
                consume: false,
            }
        }

        fn consuming(mut self) -> Self {
            self.consume = true;
            self
        }
    }

    impl View for Probe {
        fn bounds(&self) -> Rect {
            self.bounds
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            self.seen.borrow_mut().push(event.clone());
            match event {
                // A leaf occupying the whole interior consumes a click
                // within it unconditionally, mirroring GroupBox's own
                // interior-translation test probe.
                Event::Mouse(_) => EventResult::Consumed,
                Event::Key(_) if self.consume => {
                    ctx.post(CM_OK);
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            }
        }
        fn focusable(&self) -> bool {
            self.focusable
        }
        fn set_focused(&mut self, focused: bool) {
            *self.focused.borrow_mut() = focused;
        }
    }

    fn render(tp: &TabbedPages, w: i16, h: i16) -> String {
        let mut buf = Buffer::new(Size::new(w, h));
        let mut canvas = Canvas::new(&mut buf);
        tp.draw(&mut canvas);
        buf.to_text()
    }

    // --- Logic ---

    #[test]
    fn no_tabs_is_not_focusable_and_ignores_events() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), vec![], &theme());
        assert!(!tp.focusable());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            tp.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Ignored
        );
        assert_eq!(
            tp.handle_event(&mouse_down_at(1, 0), &mut ctx),
            EventResult::Ignored
        );
    }

    #[test]
    fn first_tab_is_current_by_default() {
        let tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A", "B"]), &theme());
        assert_eq!(tp.current(), 0);
    }

    #[test]
    fn with_current_sets_and_clamps() {
        let tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A", "B"]), &theme()).with_current(1);
        assert_eq!(tp.current(), 1);
        let tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A", "B"]), &theme()).with_current(99);
        assert_eq!(tp.current(), 1);
    }

    #[test]
    fn select_switches_and_is_a_no_op_out_of_range() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A", "B", "C"]), &theme());
        tp.select(2);
        assert_eq!(tp.current(), 2);
        tp.select(99);
        assert_eq!(
            tp.current(),
            2,
            "clamps to the last tab rather than panicking"
        );
    }

    #[test]
    fn strip_bounds_is_the_full_width_top_row() {
        let tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A"]), &theme());
        assert_eq!(tp.strip_bounds(), rect(0, 0, 20, 1));
    }

    #[test]
    fn interior_bounds_is_inset_below_the_strip_and_collapses_for_a_too_small_widget() {
        let tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A"]), &theme());
        assert_eq!(tp.interior_bounds(), rect(1, 2, 18, 5));

        let tiny = TabbedPages::new(rect(0, 0, 2, 3), tabs(&["A"]), &theme());
        assert!(tiny.interior_bounds().is_empty());
    }

    #[test]
    fn tab_columns_are_recomputed_after_set_bounds_changes_width() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["Alpha", "Beta"]), &theme());
        assert_eq!(tp.tab_columns.len(), 2, "sanity: both tabs fit at width 20");
        let beta_start = tp.tab_columns[1].start;
        assert_eq!(tp.tab_at(beta_start), Some(1));

        tp.set_bounds(rect(0, 0, 6, 8));
        // Narrowed enough that only the (truncated) first tab fits.
        assert_eq!(tp.tab_columns.len(), 1);
        assert_eq!(tp.tab_at(beta_start), None);
    }

    // --- Interaction ---

    #[test]
    fn click_on_a_tab_label_switches_the_current_page_and_focuses_the_strip() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["Alpha", "Beta"]), &theme());
        let beta_col = tp.tab_columns[1].start;
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let result = tp.handle_event(&mouse_down_at(beta_col, 0), &mut ctx);
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(tp.current(), 1);
    }

    #[test]
    fn click_on_blank_strip_space_focuses_the_strip_but_leaves_the_current_page() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["Alpha", "Beta"]), &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // Column 19 sits well past both short tab labels.
        let result = tp.handle_event(&mouse_down_at(19, 0), &mut ctx);
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(tp.current(), 0, "no tab was hit, so current is unchanged");
    }

    #[test]
    fn click_on_the_border_is_ignored() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A"]), &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let result = tp.handle_event(&mouse_down_at(0, 1), &mut ctx);
        assert_eq!(result, EventResult::Ignored);
    }

    #[test]
    fn click_inside_the_interior_reaches_the_active_page_at_translated_coords() {
        let probe = Probe::new(true);
        let seen = std::rc::Rc::clone(&probe.seen);
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(probe))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        // Interior starts at local (1, 2); a click at (3, 4) lands on the
        // probe (bounds (0,0,5,1) covers row 0 of the interior) at (2, 2).
        let result = tp.handle_event(&mouse_down_at(3, 4), &mut ctx);
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(seen.borrow().as_slice(), &[mouse_down_at(2, 2)]);
    }

    #[test]
    fn a_focusable_page_is_focused_when_its_interior_is_clicked() {
        let probe = Probe::new(true);
        let focused = std::rc::Rc::clone(&probe.focused);
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(probe))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(
            !*focused.borrow(),
            "page starts unfocused (strip holds focus)"
        );
        tp.handle_event(&mouse_down_at(3, 4), &mut ctx);
        assert!(*focused.borrow(), "clicking into the page focused it");
    }

    #[test]
    fn left_and_right_arrows_move_the_current_page_while_the_strip_is_focused_clamped() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A", "B", "C"]), &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(tp.current(), 0);
        tp.handle_event(&key(KeyCode::Left), &mut ctx);
        assert_eq!(tp.current(), 0, "left at the start stays put (no wrap)");
        tp.handle_event(&key(KeyCode::Right), &mut ctx);
        assert_eq!(tp.current(), 1);
        tp.handle_event(&key(KeyCode::Right), &mut ctx);
        assert_eq!(tp.current(), 2);
        tp.handle_event(&key(KeyCode::Right), &mut ctx);
        assert_eq!(tp.current(), 2, "right at the end stays put (no wrap)");
        tp.handle_event(&key(KeyCode::Left), &mut ctx);
        assert_eq!(tp.current(), 1);
    }

    #[test]
    fn arrows_do_nothing_when_a_page_holds_focus() {
        let tp_tabs: Vec<(&str, Box<dyn View>)> =
            vec![("A", Box::new(Probe::new(true))), ("B", page("B"))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        tp.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page
        let result = tp.handle_event(&key(KeyCode::Right), &mut ctx);
        assert_eq!(
            result,
            EventResult::Ignored,
            "the probe page ignores Right, and TabbedPages must not intercept it"
        );
        assert_eq!(tp.current(), 0, "current page unchanged");
    }

    #[test]
    fn regaining_focus_from_outside_always_resets_to_the_strip() {
        // Found via a real bug report driving examples/dialogs.rs: Tab from
        // the dialog's last button wrapped the outer Group's focus back
        // around to TabbedPages, but TabbedPages resumed whatever page
        // control it had last focused (e.g. the check box) instead of
        // landing back on the strip — so pure forward cycling could never
        // reach the strip again once any page had ever been visited.
        let probe = Probe::new(true);
        let focused = std::rc::Rc::clone(&probe.focused);
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(probe))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        tp.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page
        assert_eq!(tp.focus, Focus::Page);
        assert!(*focused.borrow());

        // An owner (e.g. an outer Group wrapping focus back around) tells
        // TabbedPages it lost, then regained, focus — with no click/Tab of
        // its own moving it back to the strip in between.
        tp.set_focused(false);
        assert!(!*focused.borrow(), "losing focus unfocuses the page too");
        tp.set_focused(true);

        assert_eq!(
            tp.focus,
            Focus::Strip,
            "a fresh arrival from outside always starts at the strip"
        );
        assert!(
            !*focused.borrow(),
            "the page is not silently refocused behind the strip"
        );
    }

    #[test]
    fn outer_group_wrap_around_lands_back_on_the_strip_not_deep_page_focus() {
        // The exact end-to-end shape of the bug report: TabbedPages plus a
        // later Button sibling in an outer (wrapping) Group. Tab into the
        // page, escape to the button, then Tab again to wrap back around.
        let probe = Probe::new(true);
        let focused = std::rc::Rc::clone(&probe.focused);
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(probe))];
        let tp = TabbedPages::new(rect(0, 0, 20, 4), tp_tabs, &theme());
        let button = super::super::Button::new(rect(0, 5, 8, 1), "OK", CM_OK, &theme());
        let mut outer =
            crate::view::Group::new(rect(0, 0, 20, 10), vec![Box::new(tp), Box::new(button)]);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page
        assert!(*focused.borrow());
        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // page escapes -> Button
        assert_eq!(outer.focused(), Some(1));
        assert!(!*focused.borrow(), "leaving TabbedPages unfocused its page");

        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // Button wraps -> TabbedPages
        assert_eq!(outer.focused(), Some(0), "wrapped back to TabbedPages");
        assert!(
            !*focused.borrow(),
            "wrap-around lands on the strip, not back on the page"
        );
    }

    #[test]
    fn a_page_with_two_focusable_children_is_not_stranded_on_the_second_after_one_full_cycle() {
        // The precise bug report: a page shaped like examples/dialogs.rs's
        // "General" tab (two focusable controls in a `.non_wrapping()`
        // `Group`) must not permanently strand focus on the second control
        // once the page has been escaped and re-entered once already.
        let probe_a = Probe::new(true);
        let focused_a = std::rc::Rc::clone(&probe_a.focused);
        let probe_b = Probe::new(true);
        let focused_b = std::rc::Rc::clone(&probe_b.focused);
        let page: Box<dyn View> = Box::new(
            crate::view::Group::new(
                rect(0, 0, 20, 4),
                vec![Box::new(probe_a), Box::new(probe_b)],
            )
            .non_wrapping(),
        );
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", page)];
        let tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let button = super::super::Button::new(rect(0, 9, 8, 1), "OK", CM_OK, &theme());
        let mut outer =
            crate::view::Group::new(rect(0, 0, 20, 12), vec![Box::new(tp), Box::new(button)]);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> probe_a
        assert!(*focused_a.borrow());
        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // probe_a -> probe_b
        assert!(*focused_b.borrow());
        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // page escapes -> Button
        assert_eq!(outer.focused(), Some(1));
        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // Button wraps -> TabbedPages (strip)

        // Re-enter the page: must land on probe_a again, not resume probe_b.
        outer.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page
        assert!(
            *focused_a.borrow(),
            "the first control is reachable again after a full cycle"
        );
        assert!(
            !*focused_b.borrow(),
            "the second control isn't left stuck focused from before"
        );
    }

    #[test]
    fn tab_from_the_strip_moves_focus_onto_a_focusable_page() {
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(Probe::new(true)))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            tp.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(tp.focus, Focus::Page);
    }

    #[test]
    fn tab_from_the_strip_escapes_when_the_active_page_is_not_focusable() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A"]), &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            tp.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Ignored
        );
        assert_eq!(tp.focus, Focus::Strip);
    }

    #[test]
    fn back_tab_from_the_strip_always_escapes() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tabs(&["A"]), &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            tp.handle_event(&key(KeyCode::BackTab), &mut ctx),
            EventResult::Ignored
        );
    }

    #[test]
    fn keys_reach_the_focused_pages_content_first() {
        let probe = Probe::new(true).consuming();
        let seen = std::rc::Rc::clone(&probe.seen);
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(probe))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        tp.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page
        let result = tp.handle_event(&key(KeyCode::Char('x')), &mut ctx);
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(seen.borrow().len(), 1);
    }

    #[test]
    fn back_tab_from_an_exhausted_page_returns_focus_to_the_strip() {
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(Probe::new(true)))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        tp.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page
        assert_eq!(tp.focus, Focus::Page);
        let result = tp.handle_event(&key(KeyCode::BackTab), &mut ctx);
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(tp.focus, Focus::Strip);
    }

    #[test]
    fn forward_tab_from_an_exhausted_page_escapes_tabbedpages_to_reach_a_later_sibling() {
        // Structurally identical to group_box.rs's
        // tab_escapes_a_group_box_with_one_focusable_child_to_reach_a_later_sibling.
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(Probe::new(true)))];
        let tp = TabbedPages::new(rect(0, 0, 20, 4), tp_tabs, &theme());
        let after = super::super::Button::new(rect(0, 5, 8, 1), "OK", CM_OK, &theme());
        let mut outer =
            crate::view::Group::new(rect(0, 0, 20, 10), vec![Box::new(tp), Box::new(after)]);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert_eq!(outer.focused(), Some(0), "starts on TabbedPages");
        // Strip -> Page.
        assert_eq!(
            outer.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(outer.focused(), Some(0), "still inside TabbedPages");
        // Page is exhausted: forward Tab must escape to the OK button.
        assert_eq!(
            outer.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed,
            "the outer group itself still consumes and advances"
        );
        assert_eq!(
            outer.focused(),
            Some(1),
            "Tab escaped TabbedPages to reach the OK button, not swallowed"
        );
    }

    #[test]
    fn switching_tabs_while_a_page_holds_focus_unfocuses_old_and_focuses_new() {
        let probe_a = Probe::new(true);
        let focused_a = std::rc::Rc::clone(&probe_a.focused);
        let probe_b = Probe::new(true);
        let focused_b = std::rc::Rc::clone(&probe_b.focused);
        let tp_tabs: Vec<(&str, Box<dyn View>)> =
            vec![("A", Box::new(probe_a)), ("B", Box::new(probe_b))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        tp.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page A
        assert!(*focused_a.borrow());

        let b_col = tp.tab_columns[1].start;
        tp.handle_event(&mouse_down_at(b_col, 0), &mut ctx);
        assert!(!*focused_a.borrow(), "old page told it lost focus");
        assert_eq!(tp.current(), 1);
        assert_eq!(
            tp.focus,
            Focus::Strip,
            "a tab click always lands focus on the strip"
        );
        assert!(
            !*focused_b.borrow(),
            "new page not yet focused until Tab/click reaches it"
        );
    }

    #[test]
    fn a_command_posted_by_the_active_pages_content_bubbles_out() {
        let probe = Probe::new(true).consuming();
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(probe))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        tp.handle_event(&key(KeyCode::Tab), &mut ctx); // strip -> page
        tp.handle_event(&key(KeyCode::Char('x')), &mut ctx);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
    }

    #[test]
    fn commands_and_paste_reach_the_active_page_regardless_of_focus_slot() {
        let probe = Probe::new(true);
        let seen = std::rc::Rc::clone(&probe.seen);
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![("A", Box::new(probe))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(tp.focus, Focus::Strip);
        tp.handle_event(&Event::Command(CM_OK), &mut ctx);
        assert_eq!(
            seen.borrow().len(),
            1,
            "reached the page even with the strip focused"
        );
    }

    #[test]
    fn a_broadcast_reaches_every_page_including_inactive_ones() {
        let probe_a = Probe::new(false);
        let seen_a = std::rc::Rc::clone(&probe_a.seen);
        let probe_b = Probe::new(false);
        let seen_b = std::rc::Rc::clone(&probe_b.seen);
        let tp_tabs: Vec<(&str, Box<dyn View>)> =
            vec![("A", Box::new(probe_a)), ("B", Box::new(probe_b))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        tp.handle_event(&Event::Broadcast(CM_OK), &mut ctx);
        assert_eq!(seen_a.borrow().len(), 1);
        assert_eq!(seen_b.borrow().len(), 1, "the inactive page saw it too");
    }

    #[test]
    fn resize_and_idle_also_reach_every_page() {
        let probe_a = Probe::new(false);
        let seen_a = std::rc::Rc::clone(&probe_a.seen);
        let probe_b = Probe::new(false);
        let seen_b = std::rc::Rc::clone(&probe_b.seen);
        let tp_tabs: Vec<(&str, Box<dyn View>)> =
            vec![("A", Box::new(probe_a)), ("B", Box::new(probe_b))];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        tp.handle_event(&Event::Idle, &mut ctx);
        assert_eq!(seen_a.borrow().len(), 1);
        assert_eq!(seen_b.borrow().len(), 1);
    }

    #[test]
    fn valid_folds_over_every_page_not_just_the_active_one() {
        struct Vetoer {
            bounds: Rect,
        }
        impl View for Vetoer {
            fn bounds(&self) -> Rect {
                self.bounds
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn valid(&mut self, _command: Command, _ctx: &mut Context) -> bool {
                false
            }
        }
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![
            ("A", page("A")),
            (
                "B",
                Box::new(Vetoer {
                    bounds: rect(0, 0, 5, 1),
                }),
            ),
        ];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(
            !tp.valid(CM_OK, &mut ctx),
            "the inactive second page still gets to veto"
        );
    }

    #[test]
    fn set_bounds_recomputes_geometry_and_propagates_the_new_interior_to_every_page() {
        struct BoundsSpy {
            bounds: Rect,
            last: std::rc::Rc<std::cell::RefCell<Option<Rect>>>,
        }
        impl View for BoundsSpy {
            fn bounds(&self) -> Rect {
                self.bounds
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn set_bounds(&mut self, bounds: Rect) {
                *self.last.borrow_mut() = Some(bounds);
            }
        }
        let last_a = std::rc::Rc::new(std::cell::RefCell::new(None));
        let last_b = std::rc::Rc::new(std::cell::RefCell::new(None));
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![
            (
                "A",
                Box::new(BoundsSpy {
                    bounds: rect(0, 0, 5, 1),
                    last: std::rc::Rc::clone(&last_a),
                }),
            ),
            (
                "B",
                Box::new(BoundsSpy {
                    bounds: rect(0, 0, 5, 1),
                    last: std::rc::Rc::clone(&last_b),
                }),
            ),
        ];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 8), tp_tabs, &theme());
        tp.set_bounds(rect(0, 0, 30, 12));
        assert_eq!(tp.interior_bounds(), rect(1, 2, 28, 9));
        assert_eq!(*last_a.borrow(), Some(rect(1, 2, 28, 9)));
        assert_eq!(
            *last_b.borrow(),
            Some(rect(1, 2, 28, 9)),
            "the inactive page's bounds were updated too"
        );
    }

    // --- Render (snapshot) ---

    #[test]
    fn snapshot_strip_and_bordered_interior_around_the_active_page() {
        let tp = TabbedPages::new(
            rect(0, 0, 26, 6),
            tabs(&["General", "Formatting"]),
            &theme(),
        );
        insta::assert_snapshot!(render(&tp, 26, 6));
    }

    #[test]
    fn active_tab_is_bright_when_the_strip_is_focused() {
        let mut tp = TabbedPages::new(rect(0, 0, 20, 6), tabs(&["A", "B"]), &theme());
        tp.set_focused(true);
        let mut buf = Buffer::new(Size::new(20, 6));
        let mut canvas = Canvas::new(&mut buf);
        tp.draw(&mut canvas);
        let cell = buf.get(Point::new(1, 0)).unwrap();
        assert_eq!(cell.style(), theme().style(Role::Selection));
    }

    #[test]
    fn active_tab_is_dim_when_nothing_holds_focus() {
        let tp = TabbedPages::new(rect(0, 0, 20, 6), tabs(&["A", "B"]), &theme());
        let mut buf = Buffer::new(Size::new(20, 6));
        let mut canvas = Canvas::new(&mut buf);
        tp.draw(&mut canvas);
        let cell = buf.get(Point::new(1, 0)).unwrap();
        assert_eq!(cell.style(), theme().style(Role::SelectionInactive));
    }

    #[test]
    fn switching_tabs_draws_only_the_newly_active_pages_content() {
        let tp_tabs: Vec<(&str, Box<dyn View>)> = vec![
            (
                "A",
                Box::new(StaticText::new(rect(0, 0, 5, 1), "AAAAA", Style::new())),
            ),
            (
                "B",
                Box::new(StaticText::new(rect(0, 0, 5, 1), "BBBBB", Style::new())),
            ),
        ];
        let mut tp = TabbedPages::new(rect(0, 0, 20, 6), tp_tabs, &theme());
        let text = render(&tp, 20, 6);
        assert!(text.contains("AAAAA"));
        assert!(!text.contains("BBBBB"));

        tp.select(1);
        let text = render(&tp, 20, 6);
        assert!(!text.contains("AAAAA"));
        assert!(text.contains("BBBBB"));
    }

    #[test]
    fn no_trailing_separator_is_drawn_after_the_last_tab() {
        // Found during the manual tmux pass against examples/dialogs.rs: a
        // stray `│` was drawn after the last tab's label even though there's
        // no further tab for it to divide from.
        let tp = TabbedPages::new(
            rect(0, 0, 26, 6),
            tabs(&["General", "Formatting"]),
            &theme(),
        );
        let text = render(&tp, 26, 6);
        let strip = text.lines().next().unwrap();
        let after_formatting = strip.split("Formatting").nth(1).unwrap();
        assert!(
            !after_formatting.contains('│'),
            "no divider after the last tab, got {strip:?}"
        );
    }

    #[test]
    fn a_too_small_widget_degrades_without_panic() {
        let tp = TabbedPages::new(rect(0, 0, 1, 1), tabs(&["A"]), &theme());
        let text = render(&tp, 1, 1);
        assert_eq!(text, " ");
    }

    #[test]
    fn a_long_tab_title_truncates_to_fit() {
        let tp = TabbedPages::new(
            rect(0, 0, 10, 4),
            tabs(&["A Very Long Title Indeed"]),
            &theme(),
        );
        let text = render(&tp, 10, 4);
        let top = text.lines().next().unwrap();
        assert_eq!(top.chars().count(), 10, "still exactly the widget width");
    }
}
