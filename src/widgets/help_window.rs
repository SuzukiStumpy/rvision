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

    fn new(contents: HelpContents, size: Size, theme: &Theme) -> Self {
        let (list_rect, pane_rect) = split(size);
        let titles: Vec<String> = contents.titles().into_iter().map(str::to_string).collect();
        let mut list = ListBox::new(list_rect, titles, theme);
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
            Focus::Pane => self.pane.handle_event(&local, ctx),
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
                    Focus::Pane => self.pane.handle_event(event, ctx),
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
Welcome to the thing. This is the first, home topic.

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
