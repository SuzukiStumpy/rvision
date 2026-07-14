//! A live list of every window open on a `Desktop` — TurboVision's "Window
//! List" (`docs/specs/window_list.md`, ADR 0037).
//!
//! Unlike [`ThemePicker`](super::ThemePicker)/[`ColorPicker`](super::ColorPicker),
//! this is not a one-shot `exec_view` picker: it must stay open and reflect
//! `Desktop`'s state across a Close, so several windows can be closed in one
//! visit. `WindowList` itself never touches `Desktop` (ADR 0003) — it only
//! records what the user asked for; [`Application`](crate::app::Shell) reads
//! that back via [`Desktop::content_mut`](super::Desktop::content_mut)
//! (ADR 0036) and acts on `Desktop` directly.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::command::{CM_WINDOW_LIST_ACTIVATE, CM_WINDOW_LIST_CLOSE};
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{Button, ListBox, Window, WindowId};

// --- Layout (local coordinates; the owning Window supplies border/title) ---

const LIST_W: i16 = 30;
const LIST_H: i16 = 8;
const BOTTOM_Y: i16 = LIST_H;
const CLOSE_X: i16 = 2;
const CLOSE_W: i16 = 10;

const WIDTH: i16 = LIST_W;
const HEIGHT: i16 = BOTTOM_Y + 1;

/// The outer `Window` size `build` centres within `area` — the interior's
/// own fixed size plus the one-cell border inset on every side (mirrors
/// `HelpWindow`'s `default_bounds`/`interior_size` split: `Desktop::open`
/// doesn't consult `Placement`, so centring happens once, here).
const DEFAULT_WIDTH: i16 = WIDTH + 2;
const DEFAULT_HEIGHT: i16 = HEIGHT + 2;

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// What the user asked for, read (and cleared) by whatever hosts this widget
/// via `Desktop::content_mut` after a `CM_WINDOW_LIST_ACTIVATE`/
/// `CM_WINDOW_LIST_CLOSE` bubbles up (ADR 0036, ADR 0037).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WindowListAction {
    /// Bring this window to the front and dismiss the list.
    Activate(WindowId),
    /// Close this window; the list itself stays open, refreshed.
    Close(WindowId),
}

/// Where focus currently sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Close,
}

const FOCUS_ORDER: [Focus; 2] = [Focus::List, Focus::Close];

/// The interior control: a titles list plus a Close button.
pub struct WindowList {
    theme: Theme,
    ids: Vec<WindowId>,
    list: ListBox,
    close: Button,
    focus: Focus,
    pending: Option<WindowListAction>,
}

impl WindowList {
    /// `entries` is shown in the given order, one row per id. `theme` styles
    /// the widget's own chrome.
    pub fn new(entries: Vec<(WindowId, String)>, theme: &Theme) -> Self {
        let (ids, titles): (Vec<WindowId>, Vec<String>) = entries.into_iter().unzip();
        let list = ListBox::new(rect(0, 0, LIST_W, LIST_H), titles, theme);
        let mut widget = Self {
            theme: theme.clone(),
            ids,
            list,
            close: Button::new(
                rect(CLOSE_X, BOTTOM_Y, CLOSE_W, 1),
                "Close",
                CM_WINDOW_LIST_CLOSE,
                theme,
            ),
            focus: Focus::List,
            pending: None,
        };
        widget.apply_focus();
        widget
    }

    /// Rebuilds the displayed rows from a fresh snapshot (there is no
    /// in-place item-replace on `ListBox`). The previously-selected id is
    /// kept selected (by id, not index) if it's still present; dropped
    /// otherwise.
    pub fn set_entries(&mut self, entries: Vec<(WindowId, String)>) {
        let previous = self.list.selected().and_then(|i| self.ids.get(i)).copied();
        let (ids, titles): (Vec<WindowId>, Vec<String>) = entries.into_iter().unzip();
        self.list = ListBox::new(rect(0, 0, LIST_W, LIST_H), titles, &self.theme);
        self.ids = ids;
        match previous.and_then(|id| self.ids.iter().position(|&i| i == id)) {
            Some(idx) => self.list.select(idx),
            None => self.list.deselect(),
        }
        self.apply_focus();
    }

    /// Reads and clears the pending action, if any.
    pub fn take_pending(&mut self) -> Option<WindowListAction> {
        self.pending.take()
    }

    fn selected_id(&self) -> Option<WindowId> {
        self.list.selected().and_then(|i| self.ids.get(i)).copied()
    }

    fn apply_focus(&mut self) {
        self.list.set_focused(self.focus == Focus::List);
        self.close.set_focused(self.focus == Focus::Close);
    }

    fn move_focus(&mut self, delta: isize) {
        let n = FOCUS_ORDER.len() as isize;
        let current = FOCUS_ORDER
            .iter()
            .position(|&f| f == self.focus)
            .unwrap_or(0) as isize;
        let next = (((current + delta) % n) + n) % n;
        self.focus = FOCUS_ORDER[next as usize];
        self.apply_focus();
    }

    /// Records `Activate(selected)` and posts `CM_WINDOW_LIST_ACTIVATE` — a
    /// no-op (nothing recorded, nothing posted) on an empty list. The one
    /// path every "activate the highlighted row" input (`Enter`, a
    /// double-click) must go through.
    fn activate_selected(&mut self, ctx: &mut Context) {
        if let Some(id) = self.selected_id() {
            self.pending = Some(WindowListAction::Activate(id));
            ctx.post(CM_WINDOW_LIST_ACTIVATE);
        }
    }

    /// Records `Close(selected)` and posts `CM_WINDOW_LIST_CLOSE` — a no-op
    /// on an empty list. Never routed through `Button::handle_event`'s own
    /// bare command post, which knows nothing about recording the target
    /// (same reasoning as `ThemePicker::accept`).
    fn activate_close(&mut self, ctx: &mut Context) {
        if let Some(id) = self.selected_id() {
            self.pending = Some(WindowListAction::Close(id));
            ctx.post(CM_WINDOW_LIST_CLOSE);
        }
    }

    fn on_enter(&mut self, ctx: &mut Context) -> EventResult {
        match self.focus {
            Focus::List => self.activate_selected(ctx),
            Focus::Close => self.activate_close(ctx),
        }
        EventResult::Consumed
    }

    fn route(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match self.focus {
            Focus::List => self.list.handle_event(event, ctx),
            Focus::Close => EventResult::Ignored,
        }
    }

    fn handle_key(&mut self, event: &Event, code: KeyCode, ctx: &mut Context) -> EventResult {
        match code {
            KeyCode::Tab => {
                self.move_focus(1);
                EventResult::Consumed
            }
            KeyCode::BackTab => {
                self.move_focus(-1);
                EventResult::Consumed
            }
            KeyCode::Enter => self.on_enter(ctx),
            KeyCode::Char(' ') if self.focus == Focus::Close => {
                self.activate_close(ctx);
                EventResult::Consumed
            }
            _ => self.route(event, ctx),
        }
    }

    fn handle_mouse(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        if self.list.bounds().contains(m.pos) {
            if matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
                self.focus = Focus::List;
                self.apply_focus();
            }
            let result = self.list.handle_event(&Event::Mouse(*m), ctx);
            if matches!(m.kind, MouseKind::DoubleClick(MouseButton::Left)) {
                self.activate_selected(ctx);
            }
            return result;
        }
        if !matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
            return EventResult::Ignored;
        }
        if self.close.bounds().contains(m.pos) {
            self.focus = Focus::Close;
            self.apply_focus();
            self.activate_close(ctx);
            return EventResult::Consumed;
        }
        EventResult::Ignored
    }
}

impl View for WindowList {
    fn bounds(&self) -> Rect {
        rect(0, 0, WIDTH, HEIGHT)
    }

    fn draw(&self, canvas: &mut Canvas) {
        let dialog_style = self.theme.style(Role::DialogBackground);
        canvas.fill(canvas.bounds(), &Cell::blank(dialog_style));

        if self.ids.is_empty() {
            canvas.put_str(Point::new(1, 0), "No open windows.", dialog_style);
        } else {
            let mut child = canvas.child(self.list.bounds());
            self.list.draw(&mut child);
        }

        let mut child = canvas.child(self.close.bounds());
        self.close.draw(&mut child);
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Key(key) => self.handle_key(event, key.code, ctx),
            Event::Mouse(m) => self.handle_mouse(m, ctx),
            _ => EventResult::Ignored,
        }
    }

    fn focusable(&self) -> bool {
        true
    }
}

impl WindowList {
    /// A `Window` titled `title`, sized to fit and centred within `area`
    /// (`Desktop::open` doesn't consult `Placement`, so centring happens
    /// here, once, at construction — mirrors `HelpWindow::build`). An
    /// ordinary resizable/moveable/closable/zoomable window, not a
    /// chrome-locked dialog: this is a persistent utility window like
    /// `HelpWindow`, not a one-shot pick like `ColorPicker::pick`.
    pub fn build(
        entries: Vec<(WindowId, String)>,
        area: Rect,
        title: &str,
        theme: &Theme,
    ) -> Window {
        let bounds = default_bounds(area);
        let interior = Self::new(entries, theme);
        Window::new(bounds, title, theme, Box::new(interior))
    }
}

/// Centres a box sized `DEFAULT_WIDTH`x`DEFAULT_HEIGHT` (clamped to fit)
/// within `area` — mirrors `help_window.rs`'s `default_bounds`.
fn default_bounds(area: Rect) -> Rect {
    let w = DEFAULT_WIDTH.min(area.width()).max(0);
    let h = DEFAULT_HEIGHT.min(area.height()).max(0);
    let x = area.origin().x + (area.width() - w) / 2;
    let y = area.origin().y + (area.height() - h) / 2;
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::{CM_USER, Command, CommandSet};
    use crate::event::{KeyEvent, Modifiers};

    // `WindowId` has no public constructor (by design — an opaque handle
    // minted only by `Desktop::open`), so tests build a real `Desktop` and
    // open blank windows to mint real ids, matching how `desktop.rs`'s own
    // tests do it.
    fn entries() -> (crate::widgets::Desktop, Vec<(WindowId, String)>) {
        use crate::widgets::Desktop;
        let mut desk = Desktop::new(rect(0, 0, 80, 24), Cell::default());
        let a = desk.open(Window::new(
            rect(0, 0, 10, 4),
            "Alpha",
            &Theme::default(),
            Box::new(crate::view::StaticText::new(
                rect(0, 0, 1, 1),
                "",
                crate::color::Style::new(),
            )),
        ));
        let b = desk.open(Window::new(
            rect(0, 0, 10, 4),
            "Beta",
            &Theme::default(),
            Box::new(crate::view::StaticText::new(
                rect(0, 0, 1, 1),
                "",
                crate::color::Style::new(),
            )),
        ));
        (
            desk,
            vec![(a, "Alpha".to_string()), (b, "Beta".to_string())],
        )
    }

    fn widget() -> (crate::widgets::Desktop, WindowList) {
        let (desk, e) = entries();
        (desk, WindowList::new(e, &Theme::default()))
    }

    fn press(w: &mut WindowList, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        w.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn press_posting(w: &mut WindowList, code: KeyCode) -> Vec<Event> {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        w.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx);
        ctx.posted().to_vec()
    }

    fn tab_to(w: &mut WindowList, focus: Focus) {
        for _ in 0..FOCUS_ORDER.len() {
            if w.focus == focus {
                return;
            }
            press(w, KeyCode::Tab);
        }
        panic!("never reached {focus:?}");
    }

    // --- Construction / list ---

    #[test]
    fn entries_land_in_the_list_in_order() {
        let (_desk, w) = widget();
        assert_eq!(w.list.selected_text(), Some("Alpha"));
        assert_eq!(w.ids.len(), 2);
    }

    #[test]
    fn moving_the_selection_only_moves_the_highlight_and_sets_no_pending_action() {
        let (_desk, mut w) = widget();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        w.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Down, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(w.list.selected_text(), Some("Beta"));
        assert!(ctx.posted().is_empty());
        assert!(w.pending.is_none());
    }

    // --- Activate (Enter / double-click) ---

    #[test]
    fn enter_on_the_list_records_activate_and_posts_the_command() {
        let (desk, mut w) = widget();
        let target = desk.windows().nth(1).unwrap().0; // "Beta"
        press(&mut w, KeyCode::Down); // highlight Beta
        let posted = press_posting(&mut w, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_WINDOW_LIST_ACTIVATE)]);
        assert_eq!(w.take_pending(), Some(WindowListAction::Activate(target)));
    }

    #[test]
    fn double_click_on_a_row_selects_it_and_activates_in_one_step() {
        let (desk, mut w) = widget();
        let target = desk.windows().nth(1).unwrap().0; // "Beta"
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::DoubleClick(MouseButton::Left),
            pos: Point::new(0, 1), // row 1 = "Beta"
            modifiers: Modifiers::NONE,
        });
        w.handle_event(&click, &mut ctx);
        assert_eq!(w.list.selected_text(), Some("Beta"));
        assert_eq!(ctx.posted(), &[Event::Command(CM_WINDOW_LIST_ACTIVATE)]);
        assert_eq!(w.take_pending(), Some(WindowListAction::Activate(target)));
    }

    #[test]
    fn a_plain_single_click_never_activates() {
        let (_desk, mut w) = widget();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(0, 1),
            modifiers: Modifiers::NONE,
        });
        w.handle_event(&click, &mut ctx);
        assert!(ctx.posted().is_empty());
        assert!(w.pending.is_none());
    }

    // --- Close ---

    #[test]
    fn close_records_close_and_posts_the_command_leaving_the_list_untouched() {
        let (desk, mut w) = widget();
        let target = desk.windows().next().unwrap().0; // "Alpha", selected by default
        tab_to(&mut w, Focus::Close);
        let posted = press_posting(&mut w, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_WINDOW_LIST_CLOSE)]);
        assert_eq!(w.take_pending(), Some(WindowListAction::Close(target)));
        assert_eq!(
            w.list.selected_text(),
            Some("Alpha"),
            "the row is still shown"
        );
    }

    #[test]
    fn a_click_on_close_records_close_and_posts_the_command() {
        let (desk, mut w) = widget();
        let target = desk.windows().next().unwrap().0;
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: w.close.bounds().origin(),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(w.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_WINDOW_LIST_CLOSE)]);
        assert_eq!(w.take_pending(), Some(WindowListAction::Close(target)));
    }

    #[test]
    fn close_on_an_empty_list_posts_nothing() {
        let mut w = WindowList::new(Vec::new(), &Theme::default());
        tab_to(&mut w, Focus::Close);
        let posted = press_posting(&mut w, KeyCode::Enter);
        assert!(posted.is_empty());
        assert!(w.take_pending().is_none());
    }

    // --- take_pending ---

    #[test]
    fn take_pending_clears_after_reading() {
        let (_desk, mut w) = widget();
        press(&mut w, KeyCode::Enter);
        assert!(w.take_pending().is_some());
        assert!(w.take_pending().is_none());
    }

    // --- set_entries ---

    #[test]
    fn set_entries_keeps_the_previously_selected_id_selected_by_id_not_index() {
        let (desk, mut w) = widget();
        let beta = desk.windows().nth(1).unwrap().0;
        w.list.select(1); // highlight Beta
        // Refresh with Beta now first (Alpha closed elsewhere).
        w.set_entries(vec![(beta, "Beta".to_string())]);
        assert_eq!(w.list.selected_text(), Some("Beta"));
        assert_eq!(w.selected_id(), Some(beta));
    }

    #[test]
    fn set_entries_drops_the_selection_if_its_id_is_gone() {
        let (_desk, mut w) = widget();
        w.set_entries(Vec::new());
        assert_eq!(w.list.selected(), None);
        assert_eq!(w.selected_id(), None);
    }

    // --- Tab order ---

    #[test]
    fn tab_cycles_list_and_close_and_wraps() {
        let (_desk, mut w) = widget();
        assert_eq!(w.focus, Focus::List);
        press(&mut w, KeyCode::Tab);
        assert_eq!(w.focus, Focus::Close);
        press(&mut w, KeyCode::Tab);
        assert_eq!(w.focus, Focus::List);
        press(&mut w, KeyCode::BackTab);
        assert_eq!(w.focus, Focus::Close);
    }

    // A stray application command must never be confused with the framework
    // commands this widget posts.
    const CM_UNRELATED: Command = Command(CM_USER + 1);

    #[test]
    fn framework_commands_are_the_only_ones_this_widget_posts() {
        assert_ne!(CM_WINDOW_LIST_ACTIVATE, CM_UNRELATED);
        assert_ne!(CM_WINDOW_LIST_CLOSE, CM_UNRELATED);
    }

    // --- Render (snapshot) ---

    fn render(w: &WindowList) -> String {
        let mut buf = Buffer::new(w.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn snapshot_populated_list_focused() {
        let (_desk, w) = widget();
        insta::assert_snapshot!(render(&w));
    }

    #[test]
    fn tabbing_to_close_actually_moves_drawn_focus_there() {
        // A text snapshot can't distinguish this (focus only changes style,
        // not glyphs) — assert the styles directly, mirroring `Button`'s own
        // `focus_chooses_the_draw_colour`. This is plumbing coverage (does
        // `WindowList::apply_focus` really call through to both children),
        // not a re-test of `Button`/`ListBox`'s own already-tested colours.
        let (_desk, mut w) = widget();
        let theme = Theme::default();
        let close_label_x = w.close.bounds().origin().x + (CLOSE_W - "Close".len() as i16) / 2;
        let close_cell = |w: &WindowList| {
            let mut buf = Buffer::new(w.bounds().size());
            let mut c = Canvas::new(&mut buf);
            w.draw(&mut c);
            buf.get(Point::new(close_label_x, BOTTOM_Y))
                .unwrap()
                .style()
        };

        assert_eq!(close_cell(&w), theme.style(Role::ButtonNormal));
        tab_to(&mut w, Focus::Close);
        assert_eq!(close_cell(&w), theme.style(Role::ButtonFocused));
    }

    #[test]
    fn snapshot_empty_list() {
        let w = WindowList::new(Vec::new(), &Theme::default());
        insta::assert_snapshot!(render(&w));
    }

    // --- Window chrome ---

    #[test]
    fn build_produces_a_fully_capable_ordinary_window_centred_in_area() {
        let (_desk, e) = entries();
        let area = rect(0, 0, 80, 24);
        let window = WindowList::build(e, area, "Window List", &Theme::default());
        assert_eq!(window.title(), "Window List");
        let b = window.bounds();
        assert_eq!(b.width(), DEFAULT_WIDTH);
        assert_eq!(b.height(), DEFAULT_HEIGHT);
        // Centred: equal (or off-by-one) margin left/right, top/bottom.
        assert!((area.width() - b.width()) / 2 - b.origin().x <= 1);
    }
}
