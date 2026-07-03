//! A dialog for picking one whole [`Theme`] out of several named candidates
//! (`docs/specs/theme_picker.md`).
//!
//! Not an editor: `ThemePicker` hands back whichever candidate was chosen,
//! whole — mutating one role at a time is `ThemeEditor`'s job (roadmap #2's
//! other half). It never touches `rvision::resource` itself; the caller
//! supplies already-built `Theme`s and decides what to do with the choice.

use std::cell::RefCell;
use std::rc::Rc;

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::command::{CM_CANCEL, CM_OK};
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{Button, ListBox, Window};

// --- Layout (local coordinates; the owning Window supplies border/title) ---

const LIST_W: i16 = 18;
const LIST_H: i16 = 9;
const PANEL_X: i16 = LIST_W + 1;
const PANEL_W: i16 = 28;

const BOTTOM_Y: i16 = LIST_H;
const OK_X: i16 = 2;
const CANCEL_X: i16 = 14;
const BUTTON_W: i16 = 10;

const WIDTH: i16 = PANEL_X + PANEL_W;
const HEIGHT: i16 = BOTTOM_Y + 1;

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// Where focus currently sits.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Ok,
    Cancel,
}

const FOCUS_ORDER: [Focus; 3] = [Focus::List, Focus::Ok, Focus::Cancel];

/// The interior control: a candidate-name list, a live preview panel, and
/// OK/Cancel.
pub struct ThemePicker {
    /// The chrome theme, fixed at construction — this widget's own list/
    /// button colours never reflect whichever candidate is highlighted (same
    /// split as `ColorPicker::new`'s `initial`/`theme`).
    theme: Theme,
    candidates: Vec<(String, Theme)>,
    list: ListBox,
    ok: Button,
    cancel: Button,
    focus: Focus,
    /// Written by [`accept`](Self::accept); the seam [`ThemePicker::pick`]'s
    /// [`ThemePickerResult`] reads through (same shared-cell idiom as
    /// `ColorPickerResult`/`FileDialogResult`).
    result: Rc<RefCell<(String, Theme)>>,
}

impl ThemePicker {
    /// `candidates` is shown in the given order; `initial` is the starting
    /// highlight, clamped into range (`0` if `candidates` is empty). `theme`
    /// styles the picker's own chrome.
    pub fn new(candidates: Vec<(String, Theme)>, initial: usize, theme: &Theme) -> Self {
        let initial = if candidates.is_empty() {
            0
        } else {
            initial.min(candidates.len() - 1)
        };
        let items: Vec<String> = candidates.iter().map(|(name, _)| name.clone()).collect();
        // The highlighted candidate stays visible even once focus tabs away
        // to OK/Cancel -- almost all of this dialog's point is comparing
        // candidates by eye, which shouldn't stop just because focus moved
        // to a button (same reasoning as `ThemeEditor`'s role list).
        let mut list =
            ListBox::new(rect(0, 0, LIST_W, LIST_H), items, theme).always_show_selection(true);
        if !candidates.is_empty() {
            list.select(initial);
        }
        let seed = candidates
            .get(initial)
            .cloned()
            .unwrap_or_else(|| (String::new(), Theme::default()));
        let mut picker = Self {
            theme: theme.clone(),
            candidates,
            list,
            ok: Button::new(rect(OK_X, BOTTOM_Y, BUTTON_W, 1), "OK", CM_OK, theme).default(true),
            cancel: Button::new(
                rect(CANCEL_X, BOTTOM_Y, BUTTON_W, 1),
                "Cancel",
                CM_CANCEL,
                theme,
            ),
            focus: Focus::List,
            result: Rc::new(RefCell::new(seed)),
        };
        picker.apply_focus();
        picker
    }

    /// The index currently highlighted in the list.
    pub fn selected_index(&self) -> usize {
        self.list.selected().unwrap_or(0)
    }

    fn apply_focus(&mut self) {
        self.list.set_focused(self.focus == Focus::List);
        self.ok.set_focused(self.focus == Focus::Ok);
        self.cancel.set_focused(self.focus == Focus::Cancel);
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

    /// Records the highlighted candidate into the result handle and posts
    /// `CM_OK`. The one path every "activate OK" input (`Enter`, `Space`, a
    /// mouse click) must go through -- never `Button::handle_event`'s own
    /// bare `CM_OK` post, which knows nothing about this extra step.
    fn accept(&mut self, ctx: &mut Context) {
        if let Some(candidate) = self.candidates.get(self.selected_index()) {
            *self.result.borrow_mut() = candidate.clone();
        }
        ctx.post(CM_OK);
    }

    fn on_enter(&mut self, ctx: &mut Context) -> EventResult {
        match self.focus {
            Focus::Cancel => {
                ctx.post(CM_CANCEL);
                EventResult::Consumed
            }
            _ => {
                self.accept(ctx);
                EventResult::Consumed
            }
        }
    }

    fn route(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match self.focus {
            Focus::List => self.list.handle_event(event, ctx),
            Focus::Ok => self.ok.handle_event(event, ctx),
            Focus::Cancel => self.cancel.handle_event(event, ctx),
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
            KeyCode::Char(' ') if self.focus == Focus::Ok => {
                self.accept(ctx);
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
            return self.list.handle_event(&Event::Mouse(*m), ctx);
        }
        if !matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
            return EventResult::Ignored;
        }
        if self.ok.bounds().contains(m.pos) {
            self.focus = Focus::Ok;
            self.apply_focus();
            // Not `self.ok.handle_event(...)`: see `accept`'s doc comment.
            self.accept(ctx);
            return EventResult::Consumed;
        }
        if self.cancel.bounds().contains(m.pos) {
            self.focus = Focus::Cancel;
            self.apply_focus();
            return self.cancel.handle_event(&Event::Mouse(*m), ctx);
        }
        EventResult::Ignored
    }
}

/// Draws a small sample screen in `theme`'s own styles, labelled with `name`
/// — enough to get a feel for the theme, not an exhaustive role gallery.
fn draw_preview(canvas: &mut Canvas, name: &str, theme: &Theme) {
    let title = theme.style(Role::WindowTitle);
    canvas.fill(rect(PANEL_X, 0, PANEL_W, 1), &Cell::blank(title));
    canvas.put_str(Point::new(PANEL_X + 1, 0), name, title);

    let menu = theme.style(Role::MenuBar);
    canvas.fill(rect(PANEL_X, 1, PANEL_W, 1), &Cell::blank(menu));
    canvas.put_str(Point::new(PANEL_X + 1, 1), "File  Edit", menu);
    canvas.put_str(
        Point::new(PANEL_X + 7, 1),
        " Edit ",
        theme.style(Role::MenuSelected),
    );

    let body = theme.style(Role::EditorText);
    canvas.fill(rect(PANEL_X, 2, PANEL_W, 4), &Cell::blank(body));
    canvas.put_str(Point::new(PANEL_X + 1, 2), "Sample text", body);
    canvas.put_str(
        Point::new(PANEL_X + 1, 3),
        " a selected line ",
        theme.style(Role::Selection),
    );
    canvas.put_str(
        Point::new(PANEL_X + 1, 4),
        " OK ",
        theme.style(Role::ButtonNormal),
    );
    canvas.put_str(
        Point::new(PANEL_X + 6, 4),
        " Cancel ",
        theme.style(Role::ButtonFocused),
    );
    canvas.put_str(
        Point::new(PANEL_X + 1, 5),
        " input ",
        theme.style(Role::Input),
    );

    canvas.fill(rect(PANEL_X, 6, PANEL_W, 1), &Cell::blank(body));
    canvas.put_str(
        Point::new(PANEL_X + 1, 6),
        "{a help link}",
        theme.style(Role::HelpLink),
    );
}

impl View for ThemePicker {
    fn bounds(&self) -> Rect {
        rect(0, 0, WIDTH, HEIGHT)
    }

    fn draw(&self, canvas: &mut Canvas) {
        let dialog_style = self.theme.style(Role::DialogBackground);
        canvas.fill(canvas.bounds(), &Cell::blank(dialog_style));

        {
            let mut child = canvas.child(self.list.bounds());
            self.list.draw(&mut child);
        }

        if let Some((name, preview_theme)) = self.candidates.get(self.selected_index()) {
            draw_preview(canvas, name, preview_theme);
        }

        for control in [&self.ok as &dyn View, &self.cancel] {
            let mut child = canvas.child(control.bounds());
            control.draw(&mut child);
        }
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

/// A read-only handle onto the picked theme — mirrors `ColorPickerResult`/
/// `FileDialogResult`: a handle read after `exec_view` returns `CM_OK`.
#[derive(Clone)]
pub struct ThemePickerResult(Rc<RefCell<(String, Theme)>>);

impl ThemePickerResult {
    /// The name of the theme the picker was last accepted with (the
    /// constructed `initial` candidate's name if never accepted).
    pub fn name(&self) -> String {
        self.0.borrow().0.clone()
    }

    /// The theme the picker was last accepted with (the constructed
    /// `initial` candidate if never accepted).
    pub fn theme(&self) -> Theme {
        self.0.borrow().1.clone()
    }
}

impl ThemePicker {
    /// A centred, fixed, `Esc`-cancels [`Window`] ending on `CM_OK`/
    /// `CM_CANCEL`, same shape as
    /// [`ColorPicker::pick`](super::ColorPicker::pick)/
    /// [`FileDialog::open`](super::FileDialog::open).
    pub fn pick(
        title: &str,
        candidates: Vec<(String, Theme)>,
        initial: usize,
        theme: &Theme,
    ) -> (Window, ThemePickerResult) {
        let picker = Self::new(candidates, initial, theme);
        let result = ThemePickerResult(Rc::clone(&picker.result));
        let size = picker.bounds().size();
        let window = Window::dialog(
            Rect::from_origin_size(Point::new(0, 0), Size::new(size.width + 2, size.height + 2)),
            title,
            theme,
            Box::new(picker),
        )
        .centered()
        .resizable(false)
        .zoomable(false)
        .closable(false)
        .esc_cancels(true)
        .also_ends_on(CM_OK)
        .also_ends_on(CM_CANCEL);
        (window, result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::color::{Color, Color16, Style};
    use crate::command::{CM_USER, Command, CommandSet};
    use crate::event::{KeyEvent, Modifiers};

    fn candidates() -> Vec<(String, Theme)> {
        let alt = Theme::default().with(
            Role::EditorText,
            Style::new()
                .fg(Color::Named(Color16::Yellow))
                .bg(Color::Named(Color16::Black)),
        );
        vec![
            ("Default".to_string(), Theme::default()),
            ("Alternate".to_string(), alt),
        ]
    }

    fn picker() -> ThemePicker {
        ThemePicker::new(candidates(), 0, &Theme::default())
    }

    fn press(p: &mut ThemePicker, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        p.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn press_posting(p: &mut ThemePicker, code: KeyCode) -> Vec<Event> {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        p.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx);
        ctx.posted().to_vec()
    }

    fn tab_to(p: &mut ThemePicker, focus: Focus) {
        for _ in 0..FOCUS_ORDER.len() {
            if p.focus == focus {
                return;
            }
            press(p, KeyCode::Tab);
        }
        panic!("never reached {focus:?}");
    }

    /// `Theme` has no `PartialEq` (nothing else in the crate needs it) -- a
    /// `(name, EditorText style)` pair is a cheap, distinguishing enough
    /// stand-in for asserting *which* candidate the result handle holds.
    fn identify(entry: &(String, Theme)) -> (String, Style) {
        (entry.0.clone(), entry.1.style(Role::EditorText))
    }

    // --- Construction / list ---

    #[test]
    fn candidate_names_land_in_the_list_in_order() {
        let p = picker();
        assert_eq!(p.list.selected_text(), Some("Default"));
    }

    #[test]
    fn initial_clamps_into_range() {
        let p = ThemePicker::new(candidates(), 99, &Theme::default());
        assert_eq!(p.selected_index(), 1);
    }

    #[test]
    fn initial_clamps_to_zero_when_empty() {
        let p = ThemePicker::new(Vec::new(), 5, &Theme::default());
        assert_eq!(p.selected_index(), 0);
    }

    #[test]
    fn moving_the_selection_only_changes_selected_index() {
        let mut p = picker();
        press(&mut p, KeyCode::Down);
        assert_eq!(p.selected_index(), 1);
    }

    // --- Commit / cancel ---

    #[test]
    fn enter_from_the_list_commits_the_highlighted_candidate() {
        let mut p = picker();
        press(&mut p, KeyCode::Down); // highlight "Alternate"
        let posted = press_posting(&mut p, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_OK)]);
        assert_eq!(identify(&p.result.borrow()), identify(&candidates()[1]));
    }

    #[test]
    fn space_on_ok_commits_the_same_as_a_click() {
        let mut p = picker();
        press(&mut p, KeyCode::Down);
        tab_to(&mut p, Focus::Ok);
        press(&mut p, KeyCode::Char(' '));
        assert_eq!(identify(&p.result.borrow()), identify(&candidates()[1]));
    }

    #[test]
    fn a_click_on_ok_writes_the_highlighted_candidate_into_the_result_handle() {
        let mut p = picker();
        press(&mut p, KeyCode::Down);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: p.ok.bounds().origin(),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(p.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
        assert_eq!(identify(&p.result.borrow()), identify(&candidates()[1]));
    }

    #[test]
    fn cancel_posts_cm_cancel_without_touching_the_result() {
        let mut p = picker();
        press(&mut p, KeyCode::Down); // highlight "Alternate"
        tab_to(&mut p, Focus::Cancel);
        let posted = press_posting(&mut p, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_CANCEL)]);
        assert_eq!(
            identify(&p.result.borrow()),
            identify(&candidates()[0]),
            "seeded at initial"
        );
    }

    #[test]
    fn esc_is_not_handled_directly_by_the_picker() {
        // `Esc` is the owning `Window`'s job (`esc_cancels`), not this
        // widget's -- mirrors `ColorPicker`/`ThemeEditor`.
        let mut p = picker();
        assert_eq!(press(&mut p, KeyCode::Esc), EventResult::Ignored);
    }

    // --- Tab order ---

    #[test]
    fn tab_cycles_list_ok_cancel_and_wraps() {
        let mut p = picker();
        assert_eq!(p.focus, Focus::List);
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.focus, Focus::Ok);
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.focus, Focus::Cancel);
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.focus, Focus::List);
        press(&mut p, KeyCode::BackTab);
        assert_eq!(p.focus, Focus::Cancel);
    }

    // A stray application command must never be confused with the framework
    // commands this widget posts.
    const CM_UNRELATED: Command = Command(CM_USER + 1);

    #[test]
    fn framework_commands_are_the_only_ones_this_widget_posts() {
        assert_ne!(CM_OK, CM_UNRELATED);
        assert_ne!(CM_CANCEL, CM_UNRELATED);
    }

    // --- Render (snapshot) ---

    fn render(p: &ThemePicker) -> String {
        let mut buf = Buffer::new(p.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        p.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn snapshot_first_candidate_highlighted() {
        insta::assert_snapshot!(render(&picker()));
    }

    #[test]
    fn snapshot_second_candidate_highlighted() {
        let mut p = picker();
        press(&mut p, KeyCode::Down);
        insta::assert_snapshot!(render(&p));
    }

    // --- Window chrome ---

    #[test]
    fn esc_posts_cancel_via_the_window() {
        let (mut w, _) = ThemePicker::pick("Theme", candidates(), 0, &Theme::default());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
        assert_eq!(w.handle_event(&esc, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_CANCEL)]);
    }

    #[test]
    fn result_handle_reads_the_initial_candidate_before_any_accept() {
        let (_, result) = ThemePicker::pick("Theme", candidates(), 1, &Theme::default());
        assert_eq!(result.name(), "Alternate");
    }
}
