//! A two-pane help browser: a topic list beside the selected topic's page,
//! composed as one [`Window`] interior (ADR 0013's "topic-list +
//! scrollable-page viewer", unblocked by ADR 0016's dynamic [`Desktop`](super::Desktop)
//! and proving out ADR 0017's resize-propagation protocol for a genuinely
//! resizable composite interior — the first one in the crate).
//!
//! [`HelpWindow::build`] returns a plain, fully capable `Window` — resizable,
//! moveable, closable, zoomable — meant to be opened non-modally via
//! [`Desktop::open`](super::Desktop::open), not run through
//! [`Application::exec_view`](crate::app::Application::exec_view): nothing
//! about a help browser calls for that path's ending-command/`Esc`-cancels
//! policy.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::event::{Event, EventResult, KeyCode, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::help::HelpContents;
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{HelpPane, ListBox, Window};

/// The topic-list column's width, up to which it grows; narrower windows
/// shrink it (never the page pane, down to `MIN_PANE_WIDTH`) to fit.
const LIST_WIDTH: i16 = 22;
/// The page pane's floor: the list column gives way to this first, so an
/// extremely narrow window still leaves the actual content readable rather
/// than handing every column to the sidebar.
const MIN_PANE_WIDTH: i16 = 10;
/// The purely cosmetic column between the list and the page.
const DIVIDER_WIDTH: i16 = 1;
/// The window's own default size, before it's centred within the caller's
/// area (typically a `Desktop`'s bounds) and clamped to fit inside it.
const DEFAULT_WIDTH: i16 = 66;
const DEFAULT_HEIGHT: i16 = 20;

/// Which of the two child widgets currently holds keyboard focus.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Pane,
}

/// The `Window` interior: a [`ListBox`] of topic titles and a [`HelpPane`]
/// for the selected one's body, kept in sync (ADR 0013).
pub struct HelpWindow {
    contents: HelpContents,
    list: ListBox,
    pane: HelpPane,
    bounds: Rect,
    focus: Focus,
    /// The list index the pane is currently showing, so a selection change
    /// (and only a selection change) re-triggers `HelpPane::show`.
    shown: Option<usize>,
    divider_style: Style,
}

impl HelpWindow {
    /// Builds a `Window` titled `title`, sized and centred within `area`
    /// (`Desktop::open` doesn't consult [`Placement`](super::Placement) the
    /// way `exec_view` does, so centring happens here, once, at construction
    /// — see `docs/specs/desktop.md`), showing `contents.home()`.
    pub fn build(contents: HelpContents, area: Rect, title: &str, theme: &Theme) -> Window {
        let bounds = default_bounds(area);
        let interior = Self::new(contents, bounds.size(), theme);
        Window::new(bounds, title, theme, Box::new(interior))
    }

    /// As [`build`](Self::build), but shows `topic` instead of the home topic
    /// (ADR 0021) — resolved via [`HelpContents::topic_index`], mirroring how
    /// a followed link resolves its target (ADR 0020). An unresolvable
    /// `topic` (no such id) falls back to `contents.home()` silently, same as
    /// [`build`](Self::build) itself.
    pub fn build_at(
        contents: HelpContents,
        area: Rect,
        title: &str,
        theme: &Theme,
        topic: &str,
    ) -> Window {
        let bounds = default_bounds(area);
        let mut interior = Self::new(contents, bounds.size(), theme);
        if let Some(idx) = interior.contents.topic_index(topic) {
            interior.list.select(idx);
            interior.shown = Some(idx);
            if let Some(t) = interior.contents.topics().get(idx) {
                interior.pane.show(t);
            }
        }
        Window::new(bounds, title, theme, Box::new(interior))
    }

    fn new(contents: HelpContents, size: Size, theme: &Theme) -> Self {
        let (list_rect, pane_rect) = split(size);
        let titles: Vec<String> = contents.titles().into_iter().map(str::to_string).collect();
        // Which topic is showing should stay answerable at a glance even
        // while the page pane holds focus (ADR 0020 addendum).
        let mut list = ListBox::new(list_rect, titles, theme).always_show_selection(true);
        list.set_focused(true);
        let mut pane = HelpPane::new(pane_rect, theme);
        let shown = contents.home().map(|home| {
            pane.show(home);
            0
        });
        Self {
            contents,
            list,
            pane,
            bounds: Rect::from_origin_size(Point::new(0, 0), size),
            focus: Focus::List,
            shown,
            divider_style: theme.style(Role::WindowFrame),
        }
    }

    /// Pushes the focus flag to whichever child now holds it (ADR 0010).
    fn apply_focus(&mut self) {
        self.list.set_focused(self.focus == Focus::List);
        self.pane.set_focused(self.focus == Focus::Pane);
    }

    /// Toggles focus between the two children (there are only two, so
    /// "move by delta" just flips it either direction).
    fn move_focus(&mut self) {
        self.focus = match self.focus {
            Focus::List => Focus::Pane,
            Focus::Pane => Focus::List,
        };
        self.apply_focus();
    }

    /// Shows whatever topic the list now has selected, if that's changed
    /// since the pane last updated.
    fn sync_pane_from_list(&mut self) {
        let selected = self.list.selected();
        if selected == self.shown {
            return;
        }
        self.shown = selected;
        if let Some(topic) = selected.and_then(|i| self.contents.topics().get(i)) {
            self.pane.show(topic);
        }
    }

    /// Routes `event` into the list, then re-syncs the pane if the selection
    /// moved — the one path both key and mouse routing share.
    fn route_to_list(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let result = self.list.handle_event(event, ctx);
        self.sync_pane_from_list();
        result
    }

    /// Routes `event` into the pane, then jumps the list (and thus the page)
    /// to whatever topic a link activation asked for — the mirror of
    /// `route_to_list`/`sync_pane_from_list` in the opposite direction
    /// (ADR 0020).
    fn route_to_pane(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let result = self.pane.handle_event(event, ctx);
        self.sync_list_from_pane_link();
        result
    }

    /// Jumps the list selection (and re-shows the page) to whatever topic a
    /// link activation in the pane queued, if any. An unresolvable target —
    /// a dangling link, which shouldn't happen for well-formed content per
    /// ADR 0013's "caught by a content test" stance — is a silent no-op.
    /// Focus is left untouched either way: activating a link moves the
    /// selection and page content, not keyboard focus.
    fn sync_list_from_pane_link(&mut self) {
        let Some(target) = self.pane.take_link_activation() else {
            return;
        };
        let Some(idx) = self.contents.topic_index(&target) else {
            return;
        };
        self.list.select(idx);
        self.shown = Some(idx);
        if let Some(topic) = self.contents.topics().get(idx) {
            self.pane.show(topic);
        }
    }

    /// Routes a mouse event (already in this interior's own local
    /// coordinates — the owning `Window` translates into it, ADR 0016) to
    /// whichever child pane the pointer landed on, focusing it first on a
    /// press. The divider column, and anywhere outside both panes, is
    /// ignored.
    fn handle_mouse(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        let p = m.pos;
        let (focus, bounds) = if self.list.bounds().contains(p) {
            (Focus::List, self.list.bounds())
        } else if self.pane.bounds().contains(p) {
            (Focus::Pane, self.pane.bounds())
        } else {
            return EventResult::Ignored;
        };
        let pressed = matches!(m.kind, MouseKind::Down(_) | MouseKind::DoubleClick(_));
        if pressed && self.focus != focus {
            self.focus = focus;
            self.apply_focus();
        }
        let local = Event::Mouse(MouseEvent {
            pos: p.offset(-bounds.origin().x, -bounds.origin().y),
            ..*m
        });
        match focus {
            Focus::List => self.route_to_list(&local, ctx),
            Focus::Pane => self.route_to_pane(&local, ctx),
        }
    }
}

/// Splits `size` into the list column's rect and the page pane's rect, both
/// in this interior's own local (0, 0)-based coordinates. The list column
/// grows up to `LIST_WIDTH` but gives way first on a narrow window, down to
/// nothing, so the pane keeps `MIN_PANE_WIDTH` for as long as the total
/// width allows one; below that, the pane shrinks too (never negative —
/// clamped to zero either way).
fn split(size: Size) -> (Rect, Rect) {
    let height = size.height.max(0);
    let available_for_list = (size.width - DIVIDER_WIDTH - MIN_PANE_WIDTH).max(0);
    let list_w = LIST_WIDTH.min(available_for_list);
    let pane_x = list_w + DIVIDER_WIDTH;
    let pane_w = (size.width - pane_x).max(0);
    (
        Rect::from_origin_size(Point::new(0, 0), Size::new(list_w, height)),
        Rect::from_origin_size(Point::new(pane_x, 0), Size::new(pane_w, height)),
    )
}

/// `DEFAULT_WIDTH`x`DEFAULT_HEIGHT` (clamped to fit), centred within `area`.
fn default_bounds(area: Rect) -> Rect {
    let w = DEFAULT_WIDTH.min(area.width()).max(0);
    let h = DEFAULT_HEIGHT.min(area.height()).max(0);
    let x = area.origin().x + (area.width() - w) / 2;
    let y = area.origin().y + (area.height() - h) / 2;
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

impl View for HelpWindow {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        {
            let mut child = canvas.child(self.list.bounds());
            self.list.draw(&mut child);
        }
        let divider = Rect::from_origin_size(
            Point::new(self.list.bounds().width(), 0),
            Size::new(DIVIDER_WIDTH, self.bounds.height().max(0)),
        );
        canvas.fill(divider, &Cell::from_char('│', self.divider_style));
        {
            let mut child = canvas.child(self.pane.bounds());
            self.pane.draw(&mut child);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Mouse(m) => self.handle_mouse(m, ctx),
            Event::Key(key) => match key.code {
                KeyCode::Tab | KeyCode::BackTab => {
                    self.move_focus();
                    EventResult::Consumed
                }
                _ => match self.focus {
                    Focus::List => self.route_to_list(event, ctx),
                    Focus::Pane => self.route_to_pane(event, ctx),
                },
            },
            _ => EventResult::Ignored,
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        let (list_rect, pane_rect) = split(bounds.size());
        self.list.set_bounds(list_rect);
        self.pane.set_bounds(pane_rect);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::CommandSet;
    use crate::event::{KeyEvent, Modifiers, MouseButton};
    use crate::help::HelpContents;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    const SOURCE: &str = "\
@topic intro Introduction
Welcome to the thing. This is the first, home topic. See {the Mouse
topic|mouse} for more.

@topic keys Keyboard
Arrow keys move around.

<pre>
Ctrl+S   Save
F3       Next
</pre>

@topic mouse Mouse
Click things.
";

    fn contents() -> HelpContents {
        HelpContents::parse(SOURCE)
    }

    fn browser(w: i16, h: i16) -> HelpWindow {
        HelpWindow::new(contents(), Size::new(w, h), &Theme::default())
    }

    fn press(b: &mut HelpWindow, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        b.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn ctrl(b: &mut HelpWindow, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        b.handle_event(
            &Event::Key(KeyEvent::new(code, Modifiers::CONTROL)),
            &mut ctx,
        )
    }

    fn click(b: &mut HelpWindow, x: i16, y: i16) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        b.handle_event(
            &Event::Mouse(MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                pos: Point::new(x, y),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        )
    }

    fn wheel(b: &mut HelpWindow, x: i16, y: i16, kind: MouseKind) {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        b.handle_event(
            &Event::Mouse(MouseEvent {
                kind,
                pos: Point::new(x, y),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        );
    }

    // --- Construction ---

    #[test]
    fn build_starts_on_the_home_topic() {
        let b = browser(50, 10);
        assert_eq!(b.list.selected(), Some(0));
        assert_eq!(b.shown, Some(0));
    }

    #[test]
    fn the_list_marks_its_current_topic_even_while_the_pane_holds_focus() {
        // Usability gap found in manual testing: `ListBox`'s own default is
        // to hide its highlight while unfocused, which left the topic list
        // blank the moment focus moved to the page. `HelpWindow` opts its
        // list into `always_show_selection` (ADR 0020 addendum) so the
        // current topic stays visible regardless of which pane has focus.
        let mut b = browser(50, 10);
        press(&mut b, KeyCode::Tab); // focus the pane
        assert_eq!(b.focus, Focus::Pane);
        let theme = Theme::default();
        let mut buf = Buffer::new(Size::new(50, 10));
        let mut canvas = Canvas::new(&mut buf);
        b.draw(&mut canvas);
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().style(),
            theme.style(Role::SelectionInactive),
            "the current topic row still reads as current, just dimmer"
        );
    }

    #[test]
    fn empty_contents_builds_without_panicking() {
        let b = browser(50, 10);
        let empty = HelpWindow::new(
            HelpContents::default(),
            Size::new(50, 10),
            &Theme::default(),
        );
        assert_eq!(empty.list.selected(), None);
        assert_eq!(empty.shown, None);
        let mut buf = Buffer::new(Size::new(50, 10));
        let mut canvas = Canvas::new(&mut buf);
        empty.draw(&mut canvas); // no panic
        let _ = b; // silence unused in case of future edits
    }

    #[test]
    fn build_wraps_a_fully_capable_resizable_window() {
        let w = HelpWindow::build(contents(), rect(0, 0, 80, 24), "Help", &Theme::default());
        assert!(w.is_resizable());
        assert!(w.is_moveable());
        assert!(w.is_closable());
        assert!(w.is_zoomable());
    }

    #[test]
    fn build_centres_the_default_size_within_the_area() {
        let w = HelpWindow::build(contents(), rect(0, 0, 80, 24), "Help", &Theme::default());
        let b = w.bounds();
        assert_eq!(b.size(), Size::new(DEFAULT_WIDTH, DEFAULT_HEIGHT));
        assert_eq!(b.origin().x, (80 - DEFAULT_WIDTH) / 2);
        assert_eq!(b.origin().y, (24 - DEFAULT_HEIGHT) / 2);
    }

    #[test]
    fn build_clamps_to_a_smaller_area_without_panicking() {
        let w = HelpWindow::build(contents(), rect(0, 0, 12, 5), "Help", &Theme::default());
        assert_eq!(w.bounds().size(), Size::new(12, 5));
    }

    // --- build_at: opening straight to a given topic (ADR 0021) ---

    #[test]
    fn build_at_selects_the_given_topic_instead_of_home() {
        let area = rect(0, 0, 80, 24);
        let w = HelpWindow::build_at(contents(), area, "Help", &Theme::default(), "mouse");
        let mut buf = Buffer::new(area.size());
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        let text = buf.to_text();
        assert!(text.contains("Click things."), "shows the Mouse topic");
        assert!(
            !text.contains("Welcome to the thing"),
            "not left on the home topic"
        );
    }

    #[test]
    fn build_at_falls_back_to_home_for_an_unresolvable_topic() {
        let area = rect(0, 0, 80, 24);
        let w = HelpWindow::build_at(contents(), area, "Help", &Theme::default(), "nonexistent");
        let mut buf = Buffer::new(area.size());
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        assert!(
            buf.to_text().contains("Welcome to the thing"),
            "an unresolvable topic id falls back to home, not a panic"
        );
    }

    // --- Selection drives the page ---

    #[test]
    fn arrow_keys_in_the_list_change_the_shown_topic() {
        let mut b = browser(50, 10);
        assert_eq!(b.shown, Some(0));
        press(&mut b, KeyCode::Down);
        assert_eq!(b.list.selected(), Some(1));
        assert_eq!(b.shown, Some(1));
    }

    #[test]
    fn clicking_a_different_list_row_changes_the_shown_topic() {
        let mut b = browser(50, 10);
        click(&mut b, 2, 2); // row 2: "Mouse"
        assert_eq!(b.list.selected(), Some(2));
        assert_eq!(b.shown, Some(2));
    }

    #[test]
    fn scrolling_the_pane_never_changes_the_list_selection() {
        let mut b = browser(50, 10);
        let list_w = b.list.bounds().width();
        wheel(&mut b, list_w + 2, 1, MouseKind::ScrollDown);
        assert_eq!(b.list.selected(), Some(0));
        assert_eq!(b.shown, Some(0));
    }

    // --- Focus ---

    #[test]
    fn tab_and_back_tab_toggle_focus_between_list_and_pane() {
        let mut b = browser(50, 10);
        assert_eq!(b.focus, Focus::List);
        assert!(b.list.selected().is_some()); // sanity: list starts non-empty
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.focus, Focus::Pane);
        press(&mut b, KeyCode::Tab);
        assert_eq!(b.focus, Focus::List);
        press(&mut b, KeyCode::BackTab);
        assert_eq!(b.focus, Focus::Pane);
    }

    #[test]
    fn clicking_the_pane_focuses_it_without_touching_the_list_selection() {
        let mut b = browser(50, 10);
        let list_w = b.list.bounds().width();
        click(&mut b, list_w + 2, 1);
        assert_eq!(b.focus, Focus::Pane);
        assert_eq!(
            b.list.selected(),
            Some(0),
            "click on the pane, not the list"
        );
    }

    #[test]
    fn a_click_on_the_divider_column_is_ignored() {
        let mut b = browser(50, 10);
        let list_w = b.list.bounds().width();
        let before = b.focus;
        assert_eq!(click(&mut b, list_w, 1), EventResult::Ignored);
        assert_eq!(b.focus, before);
    }

    // --- Link activation (ADR 0020) ---

    #[test]
    fn following_a_link_in_the_pane_jumps_the_list_and_keeps_focus_there() {
        let mut b = browser(50, 10);
        press(&mut b, KeyCode::Tab); // focus the pane
        assert_eq!(b.focus, Focus::Pane);
        press(&mut b, KeyCode::Enter); // "intro"'s one link, already current
        assert_eq!(b.list.selected(), Some(2), "jumped to the Mouse topic");
        assert_eq!(b.shown, Some(2));
        assert_eq!(
            b.focus,
            Focus::Pane,
            "activation doesn't move keyboard focus"
        );
    }

    #[test]
    fn clicking_a_link_in_the_pane_jumps_the_list_and_focuses_the_pane() {
        let src = "@topic a A\n{go|b}\n@topic b B\nThere.";
        let mut b = HelpWindow::new(
            HelpContents::parse(src),
            Size::new(50, 10),
            &Theme::default(),
        );
        let list_w = b.list.bounds().width();
        // The pane starts right past the divider; its first line is just the
        // link's own two-character label, so (0, 0) pane-local lands on it.
        click(&mut b, list_w + 1, 0);
        assert_eq!(b.list.selected(), Some(1));
        assert_eq!(b.shown, Some(1));
        assert_eq!(b.focus, Focus::Pane);
    }

    #[test]
    fn an_unresolvable_link_target_is_a_no_op() {
        let src = "@topic a A\nSee {missing|nowhere}.\n@topic b B\nMore.";
        let mut b = HelpWindow::new(
            HelpContents::parse(src),
            Size::new(50, 10),
            &Theme::default(),
        );
        press(&mut b, KeyCode::Tab); // focus the pane
        press(&mut b, KeyCode::Enter); // the one link, targeting a topic id that doesn't exist
        assert_eq!(
            b.list.selected(),
            Some(0),
            "no topic in `contents` resolves the target"
        );
        assert_eq!(b.shown, Some(0));
    }

    #[test]
    fn ctrl_down_cycles_links_in_the_pane_without_touching_the_list() {
        let mut b = browser(50, 10);
        press(&mut b, KeyCode::Tab); // focus the pane
        ctrl(&mut b, KeyCode::Down); // "intro" has one link: wraps back to itself
        assert_eq!(
            b.list.selected(),
            Some(0),
            "cycling alone doesn't activate anything"
        );
        assert_eq!(b.shown, Some(0));
    }

    // --- Resize propagation (ADR 0017) ---

    #[test]
    fn set_bounds_redivides_the_columns() {
        let mut b = browser(50, 10);
        assert_eq!(b.list.bounds().width(), LIST_WIDTH);
        b.set_bounds(rect(0, 0, 80, 30));
        assert_eq!(b.list.bounds().width(), LIST_WIDTH);
        assert_eq!(b.pane.bounds().width(), 80 - LIST_WIDTH - DIVIDER_WIDTH);
        assert_eq!(b.pane.bounds().height(), 30);
    }

    #[test]
    fn set_bounds_shrinks_the_list_column_before_the_pane_goes_negative() {
        let mut b = browser(50, 10);
        b.set_bounds(rect(0, 0, 10, 10));
        assert!(b.list.bounds().width() < LIST_WIDTH);
        assert!(b.pane.bounds().width() >= 0);
    }

    #[test]
    fn set_bounds_via_the_view_trait_cascades_to_both_children() {
        let mut b = browser(50, 10);
        let view: &mut dyn View = &mut b;
        view.set_bounds(rect(0, 0, 40, 6));
        assert_eq!(b.pane.bounds().height(), 6);
    }

    #[test]
    fn resizing_through_a_window_relayouts_the_interior_live() {
        // The end-to-end path: Window::set_bounds propagates through the
        // ADR 0017 protocol without HelpWindow needing to know why its
        // bounds changed.
        let mut w = HelpWindow::build(contents(), rect(0, 0, 80, 24), "Help", &Theme::default());
        w.set_bounds(rect(1, 1, 90, 30));
        let mut buf = Buffer::new(Size::new(90, 30));
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas); // no panic; proves the cascade actually ran
    }

    // --- Draw ---

    #[test]
    fn snapshot_two_pane_layout() {
        let b = browser(40, 8);
        let mut buf = Buffer::new(Size::new(40, 8));
        let mut canvas = Canvas::new(&mut buf);
        b.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn snapshot_narrow_window_shrinks_the_list_column() {
        let b = browser(14, 6);
        let mut buf = Buffer::new(Size::new(14, 6));
        let mut canvas = Canvas::new(&mut buf);
        b.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn snapshot_empty_contents_draws_blank_panes_without_panicking() {
        let b = HelpWindow::new(HelpContents::default(), Size::new(30, 6), &Theme::default());
        let mut buf = Buffer::new(Size::new(30, 6));
        let mut canvas = Canvas::new(&mut buf);
        b.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }
}
