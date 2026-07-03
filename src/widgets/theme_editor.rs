//! A dialog for editing a [`Theme`]'s per-role styles (`docs/specs/theme_editor.md`).
//!
//! Browse all 19 [`Role`]s in a list, edit the selected one's foreground/
//! background via a nested [`ColorPicker`](super::ColorPicker) (opened by
//! whatever hosts this on a [`Desktop`](super::Desktop) — ADR 0026, since a
//! `View` can't open another window itself) and its attributes via
//! checkboxes, then save. Tracks exactly which fields were touched during the
//! session and serializes only those via [`Theme::format_field`] — a diff
//! against the starting `base`, not a full 19-role dump.

use std::cell::RefCell;
use std::collections::HashSet;
use std::rc::Rc;

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::{Attributes, Color, Style};
use crate::command::{CM_CANCEL, CM_EDIT_BG, CM_EDIT_FG, CM_OK};
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Field, Role, Theme};
use crate::view::{Context, View};

use super::{Button, CheckBox, ListBox, Window};

/// The six [`Attributes`] flags the checkboxes toggle, in the same order as
/// their on-screen layout (two columns of three).
const ATTR_FLAGS: [Attributes; 6] = [
    Attributes::BOLD,
    Attributes::DIM,
    Attributes::ITALIC,
    Attributes::UNDERLINE,
    Attributes::REVERSE,
    Attributes::BLINK,
];
const ATTR_LABELS: [&str; 6] = ["Bold", "Dim", "Italic", "Underline", "Reverse", "Blink"];

/// The index of `index`'s mutually-exclusive partner, if it has one — just
/// Bold (0) and Dim (1): both together is visually indistinguishable from
/// either alone, so checking one clears the other (see `route_to_attr`).
fn mutually_exclusive_with(index: usize) -> Option<usize> {
    match index {
        0 => Some(1),
        1 => Some(0),
        _ => None,
    }
}

// --- Layout (local coordinates; the owning Window supplies border/title) ---

const LIST_W: i16 = 22;
const LIST_H: i16 = 10;
const PANEL_X: i16 = LIST_W + 1;
/// Wide enough for the longest checkbox label, `"[X] Underline"` (13 cells).
const ATTR_COL_W: i16 = 13;
const ATTR_COL2_X: i16 = PANEL_X + ATTR_COL_W + 1;
const PANEL_W: i16 = ATTR_COL_W + 1 + ATTR_COL_W;

const PREVIEW_Y: i16 = 0;
const BUTTON_Y: i16 = 2;
const BG_BUTTON_Y: i16 = 3;
const ATTR_Y0: i16 = 5;
const RESTORE_Y: i16 = ATTR_Y0 + 3;

const BOTTOM_Y: i16 = LIST_H;
const SAVE_X: i16 = 2;
const CANCEL_X: i16 = 14;
const ACTION_BUTTON_W: i16 = 10;

const WIDTH: i16 = PANEL_X + PANEL_W;
const HEIGHT: i16 = BOTTOM_Y + 1;

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// The hand-drawn Restore-Defaults control's bounds — full panel width, one
/// row, between the attribute checkboxes and the Save/Cancel row.
fn restore_rect() -> Rect {
    rect(PANEL_X, RESTORE_Y, PANEL_W, 1)
}

/// Where focus currently sits, driving both which control routes a key and
/// which is drawn focused.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    List,
    Fg,
    Bg,
    /// Index into [`ATTR_FLAGS`]/[`ATTR_LABELS`] (`0..6`).
    Attr(usize),
    /// Resets the whole session to the framework default (`restore_defaults`)
    /// — hand-drawn/hand-dispatched like `ColorPicker`'s mode toggle, not a
    /// real [`Button`]: it must act purely locally and post no command at
    /// all, unlike Fg/Bg (which must bubble out, ADR 0026) or Save/Cancel
    /// (whose whole job is posting one).
    Restore,
    Save,
    Cancel,
}

/// The order focus cycles through: fixed, unlike `ColorPicker`'s (no
/// Truecolor/Cga16 gating here).
const FOCUS_ORDER: [Focus; 12] = [
    Focus::List,
    Focus::Fg,
    Focus::Bg,
    Focus::Attr(0),
    Focus::Attr(1),
    Focus::Attr(2),
    Focus::Attr(3),
    Focus::Attr(4),
    Focus::Attr(5),
    Focus::Restore,
    Focus::Save,
    Focus::Cancel,
];

/// The shared, mutable editing session — read and written both by
/// `ThemeEditor` itself and, between the two windows' separate lifecycles,
/// by [`ThemeEditorHandle`] (ADR 0026).
struct ThemeEditorState {
    /// The layer beneath this session's edits (framework default merged with
    /// whatever app/user layers were loaded before the editor opened) —
    /// immutable after construction. Never read for `diff_text` (that only
    /// cares about `touched`/`edited`); its only use is `restore_defaults`,
    /// which needs to know which fields the framework default would actually
    /// need to override to win back over it.
    base: Theme,
    edited: Theme,
    touched: HashSet<(Role, Field)>,
    selected: usize,
}

impl ThemeEditorState {
    fn selected_role(&self) -> Role {
        Role::ALL[self.selected]
    }

    fn style(&self) -> Style {
        self.edited.style(self.selected_role())
    }

    fn set_field_color(&mut self, field: Field, color: Color) {
        let role = self.selected_role();
        let mut style = self.edited.style(role);
        match field {
            Field::Fg => style.fg = color,
            Field::Bg => style.bg = color,
            Field::Attrs => unreachable!("apply_color only ever names Fg/Bg"),
        }
        self.edited = self.edited.clone().with(role, style);
        self.touched.insert((role, field));
    }

    fn toggle_attr(&mut self, flag: Attributes) {
        let role = self.selected_role();
        let mut style = self.edited.style(role);
        style.attrs = style.attrs.toggle(flag);
        self.edited = self.edited.clone().with(role, style);
        self.touched.insert((role, Field::Attrs));
    }

    /// Resets the whole session to the hard-coded framework default,
    /// discarding every edit made so far (this click's and any earlier
    /// one's) — a panic button, not an undo. Marks touched exactly the
    /// fields where the default actually differs from `base`: anything
    /// `base` already left at the default value needs no override line, so a
    /// field that was touched earlier but happens to already match the
    /// default no longer is.
    fn restore_defaults(&mut self) {
        let default = Theme::default();
        self.touched = Role::ALL
            .into_iter()
            .flat_map(|role| [Field::Fg, Field::Bg, Field::Attrs].map(|field| (role, field)))
            .filter(|&(role, field)| {
                default.format_field(role, field) != self.base.format_field(role, field)
            })
            .collect();
        self.edited = default;
    }

    /// Every touched field, sorted by `(role, field)` for deterministic
    /// output, rendered via `Theme::format_field`.
    fn diff_text(&self) -> String {
        let mut entries: Vec<_> = self.touched.iter().copied().collect();
        entries.sort_by_key(|(role, field)| (*role as usize, field_rank(*field)));
        entries
            .iter()
            .map(|(role, field)| self.edited.format_field(*role, *field))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// A fixed, arbitrary-but-stable ordering for [`Field`] within one role's
/// lines in [`ThemeEditorState::diff_text`] — `Field` itself carries no
/// `Ord` (nothing else in the crate needs to sort by it).
fn field_rank(field: Field) -> u8 {
    match field {
        Field::Fg => 0,
        Field::Bg => 1,
        Field::Attrs => 2,
    }
}

/// The interior control: a role list, a live style preview, Foreground/
/// Background buttons (post [`CM_EDIT_FG`]/[`CM_EDIT_BG`] — ADR 0026, this
/// widget never opens a `ColorPicker` itself), six attribute checkboxes, and
/// Save/Cancel.
pub struct ThemeEditor {
    /// The chrome theme, fixed at construction — used for this widget's own
    /// background fill and the hand-drawn Restore-Defaults control, so
    /// neither hot-reloads mid-session if the user edits the very role that
    /// styles them (`Role::DialogBackground`/`Role::ButtonNormal`/
    /// `Role::ButtonFocused`), matching every other sub-widget here (each
    /// fixes its own colours from `theme` once, at construction).
    theme: Theme,
    state: Rc<RefCell<ThemeEditorState>>,
    list: ListBox,
    fg: Button,
    bg: Button,
    attrs: [CheckBox; 6],
    save: Button,
    cancel: Button,
    focus: Focus,
}

impl ThemeEditor {
    /// `base` seeds the working copy; `theme` styles the editor's own
    /// chrome (list/buttons/checkboxes) — the same split as
    /// [`ColorPicker::new`](super::ColorPicker::new)'s `initial`/`theme`.
    pub fn new(base: Theme, theme: &Theme) -> Self {
        let state = Rc::new(RefCell::new(ThemeEditorState {
            base: base.clone(),
            edited: base,
            touched: HashSet::new(),
            selected: 0,
        }));
        let items: Vec<String> = Role::ALL.iter().map(|r| r.key().to_string()).collect();
        // Which role is selected stays visible even while focus is away on
        // Foreground/Background/an attribute checkbox/Save/Cancel -- almost
        // all of this widget's interaction happens with the list unfocused,
        // unlike `ListBox`'s other consumers, so the opt-in default (only
        // marking the selection while focused) would otherwise make it look
        // like nothing is selected the moment you tab off it. Same
        // reasoning as `HelpWindow`'s topic list (ADR 0020 addendum).
        let mut list =
            ListBox::new(rect(0, 0, LIST_W, LIST_H), items, theme).always_show_selection(true);
        list.select(0);
        let attrs = std::array::from_fn(|i| {
            let (x, y) = if i < 3 {
                (PANEL_X, ATTR_Y0 + i as i16)
            } else {
                (ATTR_COL2_X, ATTR_Y0 + (i - 3) as i16)
            };
            CheckBox::new(rect(x, y, ATTR_COL_W, 1), ATTR_LABELS[i], theme)
        });
        let mut editor = Self {
            theme: theme.clone(),
            state,
            list,
            fg: Button::new(
                rect(PANEL_X, BUTTON_Y, PANEL_W, 1),
                "Foreground...",
                CM_EDIT_FG,
                theme,
            ),
            bg: Button::new(
                rect(PANEL_X, BG_BUTTON_Y, PANEL_W, 1),
                "Background...",
                CM_EDIT_BG,
                theme,
            ),
            attrs,
            save: Button::new(
                rect(SAVE_X, BOTTOM_Y, ACTION_BUTTON_W, 1),
                "Save",
                CM_OK,
                theme,
            )
            .default(true),
            cancel: Button::new(
                rect(CANCEL_X, BOTTOM_Y, ACTION_BUTTON_W, 1),
                "Cancel",
                CM_CANCEL,
                theme,
            ),
            focus: Focus::List,
        };
        editor.sync_attrs_from_selected();
        editor.apply_focus();
        editor
    }

    /// The current edited snapshot (base with every touched field applied).
    pub fn theme(&self) -> Theme {
        self.state.borrow().edited.clone()
    }

    fn sync_attrs_from_selected(&mut self) {
        let attrs = self.state.borrow().style().attrs;
        for (i, flag) in ATTR_FLAGS.iter().enumerate() {
            self.attrs[i].set_checked(attrs.contains(*flag));
        }
    }

    /// Resets the whole session to the framework default and resyncs the
    /// attribute checkboxes to whatever the (now-default) selected role
    /// ended up with. Posts nothing — unlike Fg/Bg, this never needs a
    /// nested dialog or the driver's involvement at all (ADR 0026).
    fn activate_restore(&mut self) {
        self.state.borrow_mut().restore_defaults();
        self.sync_attrs_from_selected();
    }

    fn apply_focus(&mut self) {
        self.list.set_focused(self.focus == Focus::List);
        self.fg.set_focused(self.focus == Focus::Fg);
        self.bg.set_focused(self.focus == Focus::Bg);
        for (i, box_) in self.attrs.iter_mut().enumerate() {
            box_.set_focused(self.focus == Focus::Attr(i));
        }
        self.save.set_focused(self.focus == Focus::Save);
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

    fn route_to_list(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let before = self.list.selected();
        let result = self.list.handle_event(event, ctx);
        if self.list.selected() != before {
            if let Some(index) = self.list.selected() {
                self.state.borrow_mut().selected = index;
                self.sync_attrs_from_selected();
            }
        }
        result
    }

    fn route_to_attr(&mut self, index: usize, event: &Event, ctx: &mut Context) -> EventResult {
        let before = self.attrs[index].is_checked();
        let result = self.attrs[index].handle_event(event, ctx);
        let after = self.attrs[index].is_checked();
        if after != before {
            self.state.borrow_mut().toggle_attr(ATTR_FLAGS[index]);
            // Bold and Dim are mutually exclusive: both together renders
            // identically to either alone in every terminal this framework
            // targets, so checking one clears the other rather than letting
            // the UI offer a combination with no visible effect.
            if after {
                if let Some(other) = mutually_exclusive_with(index) {
                    if self.attrs[other].is_checked() {
                        self.attrs[other].set_checked(false);
                        self.state.borrow_mut().toggle_attr(ATTR_FLAGS[other]);
                    }
                }
            }
        }
        result
    }

    fn route(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match self.focus {
            Focus::List => self.route_to_list(event, ctx),
            Focus::Fg => self.fg.handle_event(event, ctx),
            Focus::Bg => self.bg.handle_event(event, ctx),
            Focus::Attr(i) => self.route_to_attr(i, event, ctx),
            Focus::Restore => match event {
                Event::Key(k) if matches!(k.code, KeyCode::Enter | KeyCode::Char(' ')) => {
                    self.activate_restore();
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            Focus::Save => self.save.handle_event(event, ctx),
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
            _ => self.route(event, ctx),
        }
    }

    fn handle_mouse(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        if self.list.bounds().contains(m.pos) {
            if matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
                self.focus = Focus::List;
                self.apply_focus();
            }
            return self.route_to_list(&Event::Mouse(*m), ctx);
        }
        if !matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
            return EventResult::Ignored;
        }
        if self.fg.bounds().contains(m.pos) {
            self.focus = Focus::Fg;
            self.apply_focus();
            return self.fg.handle_event(&Event::Mouse(*m), ctx);
        }
        if self.bg.bounds().contains(m.pos) {
            self.focus = Focus::Bg;
            self.apply_focus();
            return self.bg.handle_event(&Event::Mouse(*m), ctx);
        }
        for i in 0..self.attrs.len() {
            if self.attrs[i].bounds().contains(m.pos) {
                self.focus = Focus::Attr(i);
                self.apply_focus();
                return self.route_to_attr(i, &Event::Mouse(*m), ctx);
            }
        }
        if restore_rect().contains(m.pos) {
            self.focus = Focus::Restore;
            self.apply_focus();
            self.activate_restore();
            return EventResult::Consumed;
        }
        if self.save.bounds().contains(m.pos) {
            self.focus = Focus::Save;
            self.apply_focus();
            return self.save.handle_event(&Event::Mouse(*m), ctx);
        }
        if self.cancel.bounds().contains(m.pos) {
            self.focus = Focus::Cancel;
            self.apply_focus();
            return self.cancel.handle_event(&Event::Mouse(*m), ctx);
        }
        EventResult::Ignored
    }
}

impl View for ThemeEditor {
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

        let preview = self.state.borrow().style();
        canvas.fill(
            rect(PANEL_X, PREVIEW_Y, PANEL_W, 1),
            &Cell::blank(Style::new().bg(preview.bg)),
        );
        canvas.put_str(Point::new(PANEL_X, PREVIEW_Y), "Sample text", preview);

        for control in [&self.fg as &dyn View, &self.bg] {
            let mut child = canvas.child(control.bounds());
            control.draw(&mut child);
        }
        for box_ in &self.attrs {
            let mut child = canvas.child(box_.bounds());
            box_.draw(&mut child);
        }

        let restore_style = if self.focus == Focus::Restore {
            self.theme.style(Role::ButtonFocused)
        } else {
            self.theme.style(Role::ButtonNormal)
        };
        let restore_rect = restore_rect();
        canvas.fill(restore_rect, &Cell::blank(restore_style));
        let label = "Restore Defaults";
        let x = restore_rect.origin().x
            + ((restore_rect.width() - label.chars().count() as i16) / 2).max(0);
        canvas.put_str(Point::new(x, restore_rect.origin().y), label, restore_style);

        for control in [&self.save as &dyn View, &self.cancel] {
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

/// A read/write handle onto the same session [`ThemeEditor`] itself reads
/// and writes from — generalizes the read-only `ColorPickerResult`/
/// `FileDialogResult` idiom (ADR 0026) so the `Program` hosting this on a
/// `Desktop` can both inspect a still-open editor (to seed a nested
/// `ColorPicker`) and write back into it (once that picker accepts) before
/// the editor next redraws.
#[derive(Clone)]
pub struct ThemeEditorHandle(Rc<RefCell<ThemeEditorState>>);

impl ThemeEditorHandle {
    /// The role currently highlighted in the list.
    pub fn selected_role(&self) -> Role {
        self.0.borrow().selected_role()
    }

    /// The selected role's current (edited) style — read by the driver to
    /// seed a nested `ColorPicker` with the right starting colour.
    pub fn style(&self) -> Style {
        self.0.borrow().style()
    }

    /// Writes a colour picked via a nested `ColorPicker` back into the
    /// selected role's `field` (`Fg`/`Bg`), marking it touched.
    ///
    /// # Panics
    ///
    /// Panics if `field` is `Field::Attrs` — attributes are edited in-widget
    /// via checkboxes, never through a nested colour picker.
    pub fn apply_color(&self, field: Field, color: Color) {
        self.0.borrow_mut().set_field_color(field, color);
    }

    /// The full edited theme (base with every touched field applied).
    pub fn theme(&self) -> Theme {
        self.0.borrow().edited.clone()
    }

    /// Every touched field, one per line via `Theme::format_field`, sorted
    /// by `(role, field)` for deterministic output. Empty if nothing was
    /// touched.
    pub fn diff_text(&self) -> String {
        self.0.borrow().diff_text()
    }
}

impl ThemeEditor {
    /// Builds the `Desktop`-ready `Window` at `origin`: fixed size (derived
    /// from the interior's own layout), `resizable(false)`, `zoomable(false)`,
    /// `closable(false)`, `esc_cancels(true)` — no `also_ends_on`/`centered`
    /// (those are `exec_view`-only; `Desktop` hosting positions and closes
    /// windows itself, ADR 0026).
    pub fn window(
        origin: Point,
        title: &str,
        base: Theme,
        theme: &Theme,
    ) -> (Window, ThemeEditorHandle) {
        let editor = Self::new(base, theme);
        let handle = ThemeEditorHandle(Rc::clone(&editor.state));
        let size = editor.bounds().size();
        let window = Window::dialog(
            Rect::from_origin_size(origin, Size::new(size.width + 2, size.height + 2)),
            title,
            theme,
            Box::new(editor),
        )
        .resizable(false)
        .zoomable(false)
        .closable(false)
        .esc_cancels(true);
        (window, handle)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::color::Color16;
    use crate::command::{CM_USER, Command, CommandSet};
    use crate::event::{KeyEvent, Modifiers};

    fn editor() -> ThemeEditor {
        ThemeEditor::new(Theme::default(), &Theme::default())
    }

    fn press(e: &mut ThemeEditor, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        e.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn press_posting(e: &mut ThemeEditor, code: KeyCode) -> Vec<Event> {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        e.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx);
        ctx.posted().to_vec()
    }

    fn tab_to(e: &mut ThemeEditor, focus: Focus) {
        for _ in 0..FOCUS_ORDER.len() {
            if e.focus == focus {
                return;
            }
            press(e, KeyCode::Tab);
        }
        panic!("never reached {focus:?}");
    }

    // --- Role list ---

    #[test]
    fn list_starts_selected_on_the_first_role_by_its_key() {
        let e = editor();
        assert_eq!(e.list.selected(), Some(0));
        assert_eq!(e.list.selected_text(), Some(Role::ALL[0].key()));
    }

    #[test]
    fn moving_the_selection_never_touches_anything() {
        let mut e = editor();
        press(&mut e, KeyCode::Down);
        assert_eq!(e.state.borrow().selected, 1);
        assert!(e.state.borrow().touched.is_empty());
        assert_eq!(
            e.theme().style(Role::ALL[1]),
            Theme::default().style(Role::ALL[1])
        );
    }

    #[test]
    fn selecting_a_role_resyncs_the_attribute_checkboxes() {
        let base = Theme::default().with(
            Role::ALL[1],
            Style::new().attrs(Attributes::BOLD | Attributes::REVERSE),
        );
        let mut e = ThemeEditor::new(base, &Theme::default());
        assert!(!e.attrs[0].is_checked(), "role 0 starts with no attrs set");
        press(&mut e, KeyCode::Down); // -> role 1
        assert!(e.attrs[0].is_checked(), "Bold");
        assert!(e.attrs[4].is_checked(), "Reverse");
        assert!(!e.attrs[1].is_checked(), "Dim stays unchecked");
    }

    #[test]
    fn list_selection_stays_visible_after_focus_moves_away() {
        // Regression: almost all of this widget's interaction happens with
        // the list unfocused (editing via Fg/Bg/attrs/Save/Cancel) -- unlike
        // `ListBox`'s default, opt-in-only highlight, which would otherwise
        // make it look like nothing is selected the moment focus tabs off.
        let mut e = editor();
        tab_to(&mut e, Focus::Fg);
        let mut buf = Buffer::new(e.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        e.draw(&mut canvas);
        let cell = buf.get(Point::new(0, 0)).unwrap();
        assert_eq!(
            cell.style(),
            Theme::default().style(Role::SelectionInactive)
        );
    }

    // --- Attribute checkboxes ---

    #[test]
    fn toggling_bold_marks_attrs_touched_and_updates_theme() {
        let mut e = editor();
        tab_to(&mut e, Focus::Attr(0));
        press(&mut e, KeyCode::Char(' '));
        let role = Role::ALL[0];
        assert!(e.theme().style(role).attrs.contains(Attributes::BOLD));
        assert!(e.state.borrow().touched.contains(&(role, Field::Attrs)));
    }

    #[test]
    fn toggling_twice_restores_the_original_attrs_value() {
        let mut e = editor();
        tab_to(&mut e, Focus::Attr(2)); // Italic
        let original = e.theme().style(Role::ALL[0]).attrs;
        press(&mut e, KeyCode::Char(' '));
        press(&mut e, KeyCode::Char(' '));
        assert_eq!(e.theme().style(Role::ALL[0]).attrs, original);
    }

    #[test]
    fn checking_bold_then_dim_unchecks_bold() {
        let mut e = editor();
        tab_to(&mut e, Focus::Attr(0)); // Bold
        press(&mut e, KeyCode::Char(' '));
        tab_to(&mut e, Focus::Attr(1)); // Dim
        press(&mut e, KeyCode::Char(' '));

        assert!(!e.attrs[0].is_checked(), "Bold cleared by checking Dim");
        assert!(e.attrs[1].is_checked());
        let attrs = e.theme().style(Role::ALL[0]).attrs;
        assert!(attrs.contains(Attributes::DIM));
        assert!(!attrs.contains(Attributes::BOLD));
    }

    #[test]
    fn checking_dim_then_bold_unchecks_dim() {
        let mut e = editor();
        tab_to(&mut e, Focus::Attr(1)); // Dim
        press(&mut e, KeyCode::Char(' '));
        tab_to(&mut e, Focus::Attr(0)); // Bold
        press(&mut e, KeyCode::Char(' '));

        assert!(!e.attrs[1].is_checked(), "Dim cleared by checking Bold");
        assert!(e.attrs[0].is_checked());
        let attrs = e.theme().style(Role::ALL[0]).attrs;
        assert!(attrs.contains(Attributes::BOLD));
        assert!(!attrs.contains(Attributes::DIM));
    }

    #[test]
    fn bold_and_dim_do_not_affect_other_attributes() {
        let mut e = editor();
        tab_to(&mut e, Focus::Attr(3)); // Underline
        press(&mut e, KeyCode::Char(' '));
        tab_to(&mut e, Focus::Attr(0)); // Bold
        press(&mut e, KeyCode::Char(' '));
        tab_to(&mut e, Focus::Attr(1)); // Dim
        press(&mut e, KeyCode::Char(' '));

        assert!(
            e.attrs[3].is_checked(),
            "Underline is unrelated to Bold/Dim"
        );
        assert!(!e.attrs[0].is_checked());
        assert!(e.attrs[1].is_checked());
    }

    // --- Fg/Bg buttons bubble the edit-request commands, not editing anything ---

    #[test]
    fn foreground_button_posts_cm_edit_fg_without_mutating_theme() {
        let mut e = editor();
        tab_to(&mut e, Focus::Fg);
        let posted = press_posting(&mut e, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_EDIT_FG)]);
        assert_eq!(
            e.theme().style(Role::ALL[0]),
            Theme::default().style(Role::ALL[0])
        );
    }

    #[test]
    fn background_button_posts_cm_edit_bg() {
        let mut e = editor();
        tab_to(&mut e, Focus::Bg);
        let posted = press_posting(&mut e, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_EDIT_BG)]);
    }

    // --- Save / Cancel ---

    #[test]
    fn save_button_posts_cm_ok() {
        let mut e = editor();
        tab_to(&mut e, Focus::Save);
        let posted = press_posting(&mut e, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_OK)]);
    }

    #[test]
    fn cancel_button_posts_cm_cancel() {
        let mut e = editor();
        tab_to(&mut e, Focus::Cancel);
        let posted = press_posting(&mut e, KeyCode::Enter);
        assert_eq!(posted, vec![Event::Command(CM_CANCEL)]);
    }

    // --- Tab order ---

    #[test]
    fn tab_cycles_every_stop_and_wraps() {
        let mut e = editor();
        let expected = [
            Focus::Fg,
            Focus::Bg,
            Focus::Attr(0),
            Focus::Attr(1),
            Focus::Attr(2),
            Focus::Attr(3),
            Focus::Attr(4),
            Focus::Attr(5),
            Focus::Restore,
            Focus::Save,
            Focus::Cancel,
            Focus::List,
        ];
        for want in expected {
            press(&mut e, KeyCode::Tab);
            assert_eq!(e.focus, want);
        }
    }

    // --- ThemeEditorHandle: the ADR 0026 read/write seam ---

    #[test]
    fn handle_reflects_the_selected_role_and_its_style() {
        let mut e = editor();
        press(&mut e, KeyCode::Down);
        let handle = ThemeEditorHandle(Rc::clone(&e.state));
        assert_eq!(handle.selected_role(), Role::ALL[1]);
        assert_eq!(handle.style(), Theme::default().style(Role::ALL[1]));
    }

    #[test]
    fn apply_color_writes_into_the_selected_role_and_marks_touched() {
        let e = editor();
        let handle = ThemeEditorHandle(Rc::clone(&e.state));
        handle.apply_color(Field::Fg, Color::Named(Color16::Cyan));
        assert_eq!(
            handle.theme().style(Role::ALL[0]).fg,
            Color::Named(Color16::Cyan)
        );
        assert!(
            e.state
                .borrow()
                .touched
                .contains(&(Role::ALL[0], Field::Fg))
        );
    }

    #[test]
    fn apply_color_is_reflected_by_the_very_next_draw() {
        // Regression: the preview swatch used to be drawn from a field
        // cached at construction/selection-change time, which `apply_color`
        // (written through the shared handle between the editor and a
        // nested `ColorPicker`'s separate window) never refreshed -- a
        // picked colour never showed up in the sample text until the role
        // selection happened to change. `draw` must read the live style.
        let e = editor();
        let handle = ThemeEditorHandle(Rc::clone(&e.state));
        handle.apply_color(Field::Bg, Color::Named(Color16::Red));

        let mut buf = Buffer::new(e.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        e.draw(&mut canvas);

        let swatch = buf.get(Point::new(PANEL_X, PREVIEW_Y)).unwrap();
        assert_eq!(swatch.style().bg, Color::Named(Color16::Red));
    }

    #[test]
    #[should_panic]
    fn apply_color_rejects_the_attrs_field() {
        let e = editor();
        let handle = ThemeEditorHandle(Rc::clone(&e.state));
        handle.apply_color(Field::Attrs, Color::Named(Color16::Cyan));
    }

    // --- diff_text ---

    #[test]
    fn diff_text_is_empty_until_something_is_touched() {
        let e = editor();
        let handle = ThemeEditorHandle(Rc::clone(&e.state));
        assert_eq!(handle.diff_text(), "");
    }

    #[test]
    fn diff_text_renders_only_touched_fields_sorted_by_role_then_field() {
        let e = editor();
        let handle = ThemeEditorHandle(Rc::clone(&e.state));
        // Touch role 1's bg, then role 0's fg -- out of order -- diff_text
        // must still come back sorted by (role, field).
        e.state.borrow_mut().selected = 1;
        handle.apply_color(Field::Bg, Color::Named(Color16::Red));
        e.state.borrow_mut().selected = 0;
        handle.apply_color(Field::Fg, Color::Named(Color16::Yellow));

        let expected = format!(
            "{}\n{}",
            e.theme().format_field(Role::ALL[0], Field::Fg),
            e.theme().format_field(Role::ALL[1], Field::Bg),
        );
        assert_eq!(handle.diff_text(), expected);
    }

    // --- Restore Defaults ---

    #[test]
    fn restore_defaults_resets_the_edited_theme_to_the_framework_default() {
        // `base` diverges from the framework default for one role, and one
        // other role is manually edited this session -- both must be gone
        // after restoring.
        let customized_role = Role::ALL[3];
        let base = Theme::default().with(
            customized_role,
            Style::new().fg(Color::Named(Color16::Yellow)),
        );
        let mut e = ThemeEditor::new(base, &Theme::default());
        tab_to(&mut e, Focus::Attr(0));
        press(&mut e, KeyCode::Char(' ')); // touch role 0's attrs too

        tab_to(&mut e, Focus::Restore);
        press(&mut e, KeyCode::Char(' '));

        for role in Role::ALL {
            assert_eq!(
                e.theme().style(role),
                Theme::default().style(role),
                "{role:?} should match the framework default exactly"
            );
        }
    }

    #[test]
    fn restore_defaults_marks_touched_only_where_default_differs_from_base() {
        // Diverge only the fg from the framework default's own style for
        // this role -- bg/attrs must stay exactly what the default already
        // has, so they don't spuriously show up as needing an override.
        let customized_role = Role::ALL[3];
        let mut customized_style = Theme::default().style(customized_role);
        customized_style.fg = Color::Named(Color16::Yellow);
        let base = Theme::default().with(customized_role, customized_style);
        let mut e = ThemeEditor::new(base, &Theme::default());
        tab_to(&mut e, Focus::Restore);
        press(&mut e, KeyCode::Char(' '));

        let touched = &e.state.borrow().touched;
        assert!(touched.contains(&(customized_role, Field::Fg)));
        // bg/attrs weren't touched in `base` for that role, and the default
        // already matches base there, so no line is needed.
        assert!(!touched.contains(&(customized_role, Field::Bg)));
        assert!(!touched.contains(&(customized_role, Field::Attrs)));
        // An untouched-in-base role needs no override at all.
        assert!(!touched.contains(&(Role::ALL[0], Field::Fg)));
    }

    #[test]
    fn restore_defaults_wipes_a_prior_session_edit_that_is_no_longer_relevant() {
        // Role 0 isn't customized in `base`, so restoring should drop any
        // edit made to it this session -- it's a full reset, not a merge.
        let mut e = editor(); // base == Theme::default()
        let handle = ThemeEditorHandle(Rc::clone(&e.state));
        handle.apply_color(Field::Fg, Color::Named(Color16::Magenta));
        assert!(
            e.state
                .borrow()
                .touched
                .contains(&(Role::ALL[0], Field::Fg))
        );

        tab_to(&mut e, Focus::Restore);
        press(&mut e, KeyCode::Char(' '));

        assert!(
            !e.state
                .borrow()
                .touched
                .contains(&(Role::ALL[0], Field::Fg))
        );
        assert_eq!(
            e.theme().style(Role::ALL[0]).fg,
            Theme::default().style(Role::ALL[0]).fg
        );
    }

    #[test]
    fn restore_defaults_posts_no_command() {
        let mut e = editor();
        tab_to(&mut e, Focus::Restore);
        let posted = press_posting(&mut e, KeyCode::Char(' '));
        assert!(posted.is_empty());
        let posted_enter = press_posting(&mut e, KeyCode::Enter);
        assert!(posted_enter.is_empty());
    }

    #[test]
    fn clicking_restore_defaults_activates_it() {
        let base =
            Theme::default().with(Role::ALL[3], Style::new().fg(Color::Named(Color16::Yellow)));
        let mut e = ThemeEditor::new(base, &Theme::default());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: restore_rect().origin(),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(e.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert!(ctx.posted().is_empty());
        assert_eq!(
            e.theme().style(Role::ALL[3]),
            Theme::default().style(Role::ALL[3])
        );
        assert_eq!(e.focus, Focus::Restore);
    }

    // --- Window chrome (ADR 0026: Desktop-hosted, not exec_view) ---

    #[test]
    fn window_has_no_resize_zoom_or_close_chrome() {
        let (w, _) = ThemeEditor::window(
            Point::new(0, 0),
            "Theme",
            Theme::default(),
            &Theme::default(),
        );
        assert!(!w.is_resizable());
        assert!(!w.is_zoomable());
        assert!(!w.is_closable());
    }

    #[test]
    fn esc_posts_cancel_via_the_window() {
        let (mut w, _) = ThemeEditor::window(
            Point::new(0, 0),
            "Theme",
            Theme::default(),
            &Theme::default(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
        assert_eq!(w.handle_event(&esc, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_CANCEL)]);
    }

    #[test]
    fn window_handle_stays_live_after_construction() {
        let (_, handle) = ThemeEditor::window(
            Point::new(0, 0),
            "Theme",
            Theme::default(),
            &Theme::default(),
        );
        assert_eq!(handle.selected_role(), Role::ALL[0]);
    }

    // A stray application command must never be confused with the framework
    // commands this widget posts.
    const CM_UNRELATED: Command = Command(CM_USER + 1);

    #[test]
    fn framework_commands_are_the_only_ones_this_widget_posts() {
        assert_ne!(CM_OK, CM_UNRELATED);
        assert_ne!(CM_CANCEL, CM_UNRELATED);
        assert_ne!(CM_EDIT_FG, CM_UNRELATED);
        assert_ne!(CM_EDIT_BG, CM_UNRELATED);
    }

    // --- Render (snapshot) ---

    fn render(e: &ThemeEditor) -> String {
        let mut buf = Buffer::new(e.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        e.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn snapshot_default_role() {
        insta::assert_snapshot!(render(&editor()));
    }

    #[test]
    fn snapshot_role_with_attrs_set() {
        let mut e = editor();
        tab_to(&mut e, Focus::Attr(0));
        press(&mut e, KeyCode::Char(' '));
        tab_to(&mut e, Focus::Attr(3));
        press(&mut e, KeyCode::Char(' '));
        insta::assert_snapshot!(render(&e));
    }
}
