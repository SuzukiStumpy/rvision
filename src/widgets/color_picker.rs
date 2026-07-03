//! A control for picking one concrete [`Color`] (`docs/specs/color_picker.md`).
//!
//! An 8×2 CGA swatch grid, plus — only when the terminal supports it
//! ([`ColorProfile::Truecolor`]) — toggleable RGB-field/hex custom entry, both
//! representations kept in sync against one canonical `(u8, u8, u8)`. Never
//! produces [`Color::Default`]: a caller wanting a "reset to terminal
//! default" affordance adds it around the dialog, not inside it.

use std::cell::RefCell;
use std::rc::Rc;

use crate::canvas::Canvas;
use crate::cell::{Cell, Grapheme};
use crate::color::{Color, Color16, ColorProfile, Style};
use crate::command::{CM_CANCEL, CM_OK};
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{Button, InputLine, Window};

/// Grid layout: 8 columns × 2 rows, in [`Color16::ALL`] discriminant order —
/// row 0 is indices 0–7, row 1 is indices 8–15 (`docs/specs/color_picker.md`).
const GRID_COLS: usize = 8;
const GRID_ROWS: usize = 2;

/// The `Color16` swatch at `index` (`0..16`).
fn color16_at(index: usize) -> Color16 {
    Color16::ALL[index]
}

/// `index`'s (column, row) position in the 8×2 grid.
fn grid_pos(index: usize) -> (usize, usize) {
    (index % GRID_COLS, index / GRID_COLS)
}

/// The grid index at (column, row), inverse of [`grid_pos`].
fn grid_index(col: usize, row: usize) -> usize {
    row * GRID_COLS + col
}

/// Moves `index` one step in `direction`, clamped at the grid edges (no
/// wraparound, matching `RadioButtons`' clamped `Up`/`Down`).
fn grid_move(index: usize, direction: GridDirection) -> usize {
    let (col, row) = grid_pos(index);
    let (col, row) = match direction {
        GridDirection::Left => (col.saturating_sub(1), row),
        GridDirection::Right => ((col + 1).min(GRID_COLS - 1), row),
        GridDirection::Up => (col, row.saturating_sub(1)),
        GridDirection::Down => (col, (row + 1).min(GRID_ROWS - 1)),
    };
    grid_index(col, row)
}

/// A grid navigation step.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GridDirection {
    Left,
    Right,
    Up,
    Down,
}

/// Parses one RGB channel field's text: an in-range (`0..=255`) decimal
/// integer with no extraneous characters. Empty, non-digit, or out-of-range
/// text parses to `None` — the caller leaves the canonical value unchanged
/// rather than clearing it (`docs/specs/color_picker.md`).
fn parse_channel(text: &str) -> Option<u8> {
    text.parse::<u16>()
        .ok()
        .filter(|v| *v <= 255)
        .map(|v| v as u8)
}

/// Parses a hex colour field's text: `RRGGBB`, with or without a leading `#`.
/// Anything else (wrong length, non-hex digits) parses to `None`.
fn parse_hex(text: &str) -> Option<(u8, u8, u8)> {
    let text = text.strip_prefix('#').unwrap_or(text);
    if text.len() != 6 || !text.is_ascii() {
        return None;
    }
    let r = u8::from_str_radix(&text[0..2], 16).ok()?;
    let g = u8::from_str_radix(&text[2..4], 16).ok()?;
    let b = u8::from_str_radix(&text[4..6], 16).ok()?;
    Some((r, g, b))
}

/// Which subsystem was touched most recently — decides whether the tentative
/// colour reads back as `Named` or `Rgb` (`docs/specs/color_picker.md`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Source {
    Grid,
    Custom,
}

/// The picker's tentative selection: a grid index plus a canonical RGB
/// triple, with `source` deciding which one [`TentativeColor::color`] reads
/// back from. Kept separate from the `View`-facing [`ColorPicker`] so the
/// selection logic is exercised without any drawing/event machinery.
struct TentativeColor {
    grid_index: usize,
    rgb: (u8, u8, u8),
    source: Source,
}

impl TentativeColor {
    /// Seeds from `initial`: `Named(c)` seeds the grid at `c` and mirrors its
    /// RGB into the custom fields; `Rgb(r, g, b)` seeds the custom fields
    /// directly (exact grid-cursor placement in that case is left at index 0
    /// — an open question per the spec, doesn't affect the result).
    fn new(initial: Color) -> Self {
        match initial {
            Color::Named(c) => {
                let index = Color16::ALL.iter().position(|&x| x == c).unwrap_or(0);
                Self {
                    grid_index: index,
                    rgb: c.to_rgb(),
                    source: Source::Grid,
                }
            }
            Color::Rgb(r, g, b) => Self {
                grid_index: 0,
                rgb: (r, g, b),
                source: Source::Custom,
            },
            Color::Default => Self {
                grid_index: 0,
                rgb: Color16::Black.to_rgb(),
                source: Source::Grid,
            },
        }
    }

    /// Selects grid swatch `index`: becomes the grid's colour and the source.
    fn select_grid(&mut self, index: usize) {
        self.grid_index = index;
        self.source = Source::Grid;
    }

    /// Records a new canonical RGB triple from the custom entry: becomes the
    /// source (even if it happens to equal a swatch).
    fn set_custom_rgb(&mut self, rgb: (u8, u8, u8)) {
        self.rgb = rgb;
        self.source = Source::Custom;
    }

    /// The tentative colour: `Named` if the grid was touched most recently,
    /// `Rgb` if custom entry was.
    fn color(&self) -> Color {
        match self.source {
            Source::Grid => Color::Named(color16_at(self.grid_index)),
            Source::Custom => Color::Rgb(self.rgb.0, self.rgb.1, self.rgb.2),
        }
    }
}

/// Which custom-entry representation is currently in the focus/tab order
/// (`Truecolor` only).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum EntryMode {
    Rgb,
    Hex,
}

// --- Layout (local coordinates; the owning Window supplies border/title) ---

const SWATCH_W: i16 = 3;
const GRID_W: i16 = GRID_COLS as i16 * SWATCH_W;
const GRID_H: i16 = GRID_ROWS as i16;
const PREVIEW_X: i16 = GRID_W + 1;
const PREVIEW_W: i16 = 4;

const CUSTOM_Y: i16 = 3;
/// One column wider than the longest value each field ever holds
/// (`"255"`/`"RRGGBB"`) — `InputLine` reserves a trailing column for the
/// caret when it sits at end-of-text, so a field sized to content-length
/// exactly clips that last character (see its `ensure_visible`).
const CHANNEL_FIELD_W: i16 = 4;
const HEX_FIELD_W: i16 = 7;
const R_X: i16 = 2;
const G_X: i16 = 9;
const B_X: i16 = 16;
const TOGGLE_X: i16 = 22;
const TOGGLE_W: i16 = 5;

const WIDTH: i16 = 30;
const BUTTON_W: i16 = 10;

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// Where focus currently sits, driving both which control routes a key and
/// which is drawn in [`Role::ButtonFocused`]/highlighted.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Focus {
    Grid,
    /// Index into the active representation's fields: `0..3` for R/G/B in
    /// [`EntryMode::Rgb`], just `0` for the single hex field in
    /// [`EntryMode::Hex`].
    Custom(usize),
    ModeToggle,
    Ok,
    Cancel,
}

/// The Truecolor-only custom-entry section (absent entirely under `Cga16`,
/// per `docs/specs/color_picker.md`'s gating rule).
struct CustomEntry {
    mode: EntryMode,
    r: InputLine,
    g: InputLine,
    b: InputLine,
    hex: InputLine,
    toggle_focused: bool,
}

impl CustomEntry {
    fn new(theme: &Theme) -> Self {
        Self {
            mode: EntryMode::Rgb,
            r: InputLine::new(rect(R_X, CUSTOM_Y, CHANNEL_FIELD_W, 1), theme),
            g: InputLine::new(rect(G_X, CUSTOM_Y, CHANNEL_FIELD_W, 1), theme),
            b: InputLine::new(rect(B_X, CUSTOM_Y, CHANNEL_FIELD_W, 1), theme),
            hex: InputLine::new(rect(R_X, CUSTOM_Y, HEX_FIELD_W, 1), theme),
            toggle_focused: false,
        }
    }

    /// The label the toggle shows — the mode a click would switch *to*
    /// (`docs/specs/color_picker.md`'s "plain relabelling button").
    fn toggle_label(&self) -> &'static str {
        match self.mode {
            EntryMode::Rgb => "Hex",
            EntryMode::Hex => "RGB",
        }
    }

    fn toggle_rect(&self) -> Rect {
        rect(TOGGLE_X, CUSTOM_Y, TOGGLE_W, 1)
    }

    /// Copies `rgb` into whichever fields the active mode shows — the
    /// canonical-to-display direction only (`docs/specs/color_picker.md`).
    fn sync_active_from(&mut self, rgb: (u8, u8, u8)) {
        match self.mode {
            EntryMode::Rgb => {
                self.r.set_text(&rgb.0.to_string());
                self.g.set_text(&rgb.1.to_string());
                self.b.set_text(&rgb.2.to_string());
            }
            EntryMode::Hex => {
                self.hex
                    .set_text(&format!("{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2));
            }
        }
    }

    /// Copies `rgb` into every field, active or not — used when the grid
    /// (not custom entry) sets a new canonical value, so whichever
    /// representation is toggled to next is already correct.
    fn sync_all_from(&mut self, rgb: (u8, u8, u8)) {
        self.r.set_text(&rgb.0.to_string());
        self.g.set_text(&rgb.1.to_string());
        self.b.set_text(&rgb.2.to_string());
        self.hex
            .set_text(&format!("{:02X}{:02X}{:02X}", rgb.0, rgb.1, rgb.2));
    }

    /// Re-parses the active representation's field(s); `Some` only when every
    /// field involved currently parses.
    fn parse_active(&self) -> Option<(u8, u8, u8)> {
        match self.mode {
            EntryMode::Rgb => {
                let r = parse_channel(self.r.text())?;
                let g = parse_channel(self.g.text())?;
                let b = parse_channel(self.b.text())?;
                Some((r, g, b))
            }
            EntryMode::Hex => parse_hex(self.hex.text()),
        }
    }

    fn apply_focus(&mut self, focus: Focus) {
        match self.mode {
            EntryMode::Rgb => {
                self.r.set_focused(focus == Focus::Custom(0));
                self.g.set_focused(focus == Focus::Custom(1));
                self.b.set_focused(focus == Focus::Custom(2));
                self.hex.set_focused(false);
            }
            EntryMode::Hex => {
                self.hex.set_focused(focus == Focus::Custom(0));
                self.r.set_focused(false);
                self.g.set_focused(false);
                self.b.set_focused(false);
            }
        }
        self.toggle_focused = focus == Focus::ModeToggle;
    }

    /// How many `Focus::Custom(_)` stops the active mode has.
    fn field_count(&self) -> usize {
        match self.mode {
            EntryMode::Rgb => 3,
            EntryMode::Hex => 1,
        }
    }
}

/// The interior control (`docs/specs/color_picker.md`): an 8×2 CGA swatch
/// grid, plus — only when constructed with [`ColorProfile::Truecolor`] —
/// toggleable RGB-field/hex custom entry. Embeddable directly, or via
/// [`ColorPicker::pick`]'s modal [`Window`](super::Window) wrapper.
pub struct ColorPicker {
    theme: Theme,
    profile: ColorProfile,
    tentative: TentativeColor,
    custom: Option<CustomEntry>,
    focus: Focus,
    grid_focused: bool,
    ok: Button,
    cancel: Button,
    /// Written by [`accept`](Self::accept); the seam [`ColorPicker::pick`]'s
    /// [`ColorPickerResult`] reads through (same shared-cell idiom as
    /// `FileDialogResult`).
    result: Rc<RefCell<Color>>,
}

impl ColorPicker {
    /// `profile` decides whether custom RGB/hex entry exists at all
    /// (`Truecolor`) or the grid is the whole picker (`Cga16`) — passed in,
    /// not detected internally, the same testability seam as everywhere else
    /// [`ColorProfile`] is consumed (ADR 0023).
    pub fn new(initial: Color, profile: ColorProfile, theme: &Theme) -> Self {
        let tentative = TentativeColor::new(initial);
        let buttons_y = buttons_y(profile);
        let mut custom = match profile {
            ColorProfile::Truecolor => Some(CustomEntry::new(theme)),
            ColorProfile::Cga16 => None,
        };
        if let Some(custom) = &mut custom {
            custom.sync_all_from(tentative.rgb);
        }
        let mut picker = Self {
            theme: theme.clone(),
            profile,
            tentative,
            custom,
            focus: Focus::Grid,
            grid_focused: false,
            ok: Button::new(rect(WIDTH - 24, buttons_y, BUTTON_W, 1), "OK", CM_OK, theme)
                .default(true),
            cancel: Button::new(
                rect(WIDTH - 12, buttons_y, BUTTON_W, 1),
                "Cancel",
                CM_CANCEL,
                theme,
            ),
            result: Rc::new(RefCell::new(Color::Default)),
        };
        picker.apply_focus();
        picker
    }

    /// The current tentative colour (grid highlight or custom entry,
    /// whichever was touched most recently).
    pub fn color(&self) -> Color {
        self.tentative.color()
    }

    /// The order focus cycles through: the grid, then (Truecolor only) the
    /// active representation's field(s) and the mode toggle, then OK/Cancel.
    fn focus_order(&self) -> Vec<Focus> {
        let mut order = vec![Focus::Grid];
        if let Some(custom) = &self.custom {
            for i in 0..custom.field_count() {
                order.push(Focus::Custom(i));
            }
            order.push(Focus::ModeToggle);
        }
        order.push(Focus::Ok);
        order.push(Focus::Cancel);
        order
    }

    fn apply_focus(&mut self) {
        self.grid_focused = self.focus == Focus::Grid;
        if let Some(custom) = &mut self.custom {
            custom.apply_focus(self.focus);
        }
        self.ok.set_focused(self.focus == Focus::Ok);
        self.cancel.set_focused(self.focus == Focus::Cancel);
    }

    fn move_focus(&mut self, delta: isize) {
        let order = self.focus_order();
        let n = order.len() as isize;
        let current = order.iter().position(|&f| f == self.focus).unwrap_or(0) as isize;
        let next = (((current + delta) % n) + n) % n;
        self.focus = order[next as usize];
        self.apply_focus();
    }

    /// Selects grid swatch `index`, syncing custom entry's fields (all of
    /// them, active or not) to match the new canonical value.
    fn select_grid(&mut self, index: usize) {
        self.tentative.select_grid(index);
        if let Some(custom) = &mut self.custom {
            custom.sync_all_from(self.tentative.rgb);
        }
    }

    /// Re-parses the active custom representation; if every field involved
    /// currently parses, adopts it as the new canonical value.
    fn reparse_custom(&mut self) {
        if let Some(custom) = &self.custom {
            if let Some(rgb) = custom.parse_active() {
                self.tentative.set_custom_rgb(rgb);
            }
        }
    }

    fn toggle_mode(&mut self) {
        if let Some(custom) = &mut self.custom {
            custom.mode = match custom.mode {
                EntryMode::Rgb => EntryMode::Hex,
                EntryMode::Hex => EntryMode::Rgb,
            };
            custom.sync_active_from(self.tentative.rgb);
            custom.apply_focus(self.focus);
        }
    }

    /// Records the tentative colour into the result handle and posts `CM_OK`.
    fn accept(&mut self, ctx: &mut Context) {
        *self.result.borrow_mut() = self.tentative.color();
        ctx.post(CM_OK);
    }

    fn on_enter(&mut self, ctx: &mut Context) -> EventResult {
        match self.focus {
            Focus::Cancel => {
                ctx.post(CM_CANCEL);
                EventResult::Consumed
            }
            Focus::ModeToggle => {
                self.toggle_mode();
                EventResult::Consumed
            }
            _ => {
                self.accept(ctx);
                EventResult::Consumed
            }
        }
    }

    /// Routes a non-navigation key to the focused control, re-parsing custom
    /// entry afterwards if a field was touched.
    /// The active custom-entry field's text, or `None` when focus isn't on
    /// one — used by [`route`](Self::route) to detect an actual edit (versus
    /// e.g. a `Home`/arrow that only moves the cursor).
    fn active_field_text(&self) -> Option<String> {
        let custom = self.custom.as_ref()?;
        match (self.focus, custom.mode) {
            (Focus::Custom(0), EntryMode::Hex) => Some(custom.hex.text().to_string()),
            (Focus::Custom(0), EntryMode::Rgb) => Some(custom.r.text().to_string()),
            (Focus::Custom(1), EntryMode::Rgb) => Some(custom.g.text().to_string()),
            (Focus::Custom(2), EntryMode::Rgb) => Some(custom.b.text().to_string()),
            _ => None,
        }
    }

    fn route(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let before = self.active_field_text();
        let result = match self.focus {
            Focus::Grid => EventResult::Ignored,
            Focus::Custom(0) if self.custom.as_ref().map(|c| c.mode) == Some(EntryMode::Hex) => {
                self.custom
                    .as_mut()
                    .map(|c| c.hex.handle_event(event, ctx))
                    .unwrap_or(EventResult::Ignored)
            }
            Focus::Custom(i) => self
                .custom
                .as_mut()
                .map(|c| match i {
                    0 => c.r.handle_event(event, ctx),
                    1 => c.g.handle_event(event, ctx),
                    _ => c.b.handle_event(event, ctx),
                })
                .unwrap_or(EventResult::Ignored),
            Focus::ModeToggle => match event {
                Event::Key(k) if matches!(k.code, KeyCode::Char(' ')) => {
                    self.toggle_mode();
                    EventResult::Consumed
                }
                _ => EventResult::Ignored,
            },
            Focus::Ok => self.ok.handle_event(event, ctx),
            Focus::Cancel => self.cancel.handle_event(event, ctx),
        };
        if self.active_field_text() != before {
            self.reparse_custom();
        }
        result
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
            KeyCode::Left if self.grid_focused => {
                self.select_grid(grid_move(self.tentative.grid_index, GridDirection::Left));
                EventResult::Consumed
            }
            KeyCode::Right if self.grid_focused => {
                self.select_grid(grid_move(self.tentative.grid_index, GridDirection::Right));
                EventResult::Consumed
            }
            KeyCode::Up if self.grid_focused => {
                self.select_grid(grid_move(self.tentative.grid_index, GridDirection::Up));
                EventResult::Consumed
            }
            KeyCode::Down if self.grid_focused => {
                self.select_grid(grid_move(self.tentative.grid_index, GridDirection::Down));
                EventResult::Consumed
            }
            _ => self.route(event, ctx),
        }
    }

    /// The grid index under local point `p`, if it lands on the grid.
    fn grid_hit(p: Point) -> Option<usize> {
        if p.x < 0 || p.x >= GRID_W || p.y < 0 || p.y >= GRID_H {
            return None;
        }
        let col = (p.x / SWATCH_W) as usize;
        let row = p.y as usize;
        Some(grid_index(col, row))
    }

    fn handle_mouse(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        if !matches!(m.kind, MouseKind::Down(MouseButton::Left)) {
            return EventResult::Ignored;
        }
        if let Some(index) = Self::grid_hit(m.pos) {
            self.focus = Focus::Grid;
            self.select_grid(index);
            self.apply_focus();
            return EventResult::Consumed;
        }
        let Some(custom) = &self.custom else {
            return self.click_buttons(m, ctx);
        };
        if custom.toggle_rect().contains(m.pos) {
            self.focus = Focus::ModeToggle;
            self.apply_focus();
            self.toggle_mode();
            return EventResult::Consumed;
        }
        let field_hit = match custom.mode {
            EntryMode::Rgb => [custom.r.bounds(), custom.g.bounds(), custom.b.bounds()]
                .iter()
                .position(|b| b.contains(m.pos)),
            EntryMode::Hex => {
                if custom.hex.bounds().contains(m.pos) {
                    Some(0)
                } else {
                    None
                }
            }
        };
        if let Some(i) = field_hit {
            self.focus = Focus::Custom(i);
            self.apply_focus();
            return EventResult::Consumed;
        }
        self.click_buttons(m, ctx)
    }

    fn click_buttons(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        if self.ok.bounds().contains(m.pos) {
            self.focus = Focus::Ok;
            self.apply_focus();
            // Not `self.ok.handle_event(...)`: that only posts `CM_OK` (a
            // plain `Button` doesn't know about `accept`'s extra step of
            // writing the tentative colour into `result`) -- the same
            // accept path `on_enter` uses for the keyboard.
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

/// The row OK/Cancel sit on: right below the grid+preview under `Cga16`, or
/// below the custom-entry row under `Truecolor`.
fn buttons_y(profile: ColorProfile) -> i16 {
    match profile {
        ColorProfile::Cga16 => 3,
        ColorProfile::Truecolor => 5,
    }
}

fn height_for(profile: ColorProfile) -> i16 {
    buttons_y(profile) + 1
}

/// A handle to the colour a [`ColorPicker`] was accepted with, readable
/// after [`exec_view`](crate::app::Application::exec_view) returns `CM_OK`.
/// Mirrors `FileDialogResult`: the same shared-cell idiom, since
/// `ColorPicker` itself becomes the window's boxed, type-erased interior
/// (ADR 0003).
#[derive(Clone)]
pub struct ColorPickerResult(Rc<RefCell<Color>>);

impl ColorPickerResult {
    /// The colour the picker was last accepted with (`Color::Default` if
    /// never accepted).
    pub fn color(&self) -> Color {
        *self.0.borrow()
    }
}

impl ColorPicker {
    /// A centred, fixed, `Esc`-cancels [`Window`] ending on `CM_OK`/
    /// `CM_CANCEL`, same shape as [`FileDialog::open`](super::FileDialog::open)/
    /// [`MessageBox`](super::MessageBox)'s builders.
    pub fn pick(
        title: &str,
        initial: Color,
        profile: ColorProfile,
        theme: &Theme,
    ) -> (Window, ColorPickerResult) {
        let picker = Self::new(initial, profile, theme);
        let result = ColorPickerResult(Rc::clone(&picker.result));
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

impl View for ColorPicker {
    fn bounds(&self) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(WIDTH, height_for(self.profile)))
    }

    fn draw(&self, canvas: &mut Canvas) {
        let body = self.theme.style(Role::DialogBackground);
        canvas.fill(canvas.bounds(), &Cell::blank(body));

        for i in 0..16 {
            let (col, row) = grid_pos(i);
            let x = col as i16 * SWATCH_W;
            let y = row as i16;
            let swatch = Style::new().bg(Color::Named(color16_at(i)));
            canvas.fill(rect(x, y, SWATCH_W, 1), &Cell::blank(swatch));
            if i == self.tentative.grid_index {
                let hl = self.theme.style(Role::Selection);
                canvas.set(Point::new(x, y), Cell::new(Grapheme::new("["), hl));
                canvas.set(
                    Point::new(x + SWATCH_W - 1, y),
                    Cell::new(Grapheme::new("]"), hl),
                );
            }
        }

        let preview = Style::new().bg(self.tentative.color());
        canvas.fill(rect(PREVIEW_X, 0, PREVIEW_W, GRID_H), &Cell::blank(preview));

        if let Some(custom) = &self.custom {
            match custom.mode {
                EntryMode::Rgb => {
                    canvas.put_str(Point::new(R_X - 2, CUSTOM_Y), "R:", body);
                    canvas.put_str(Point::new(G_X - 2, CUSTOM_Y), "G:", body);
                    canvas.put_str(Point::new(B_X - 2, CUSTOM_Y), "B:", body);
                    for field in [&custom.r, &custom.g, &custom.b] {
                        let mut child = canvas.child(field.bounds());
                        field.draw(&mut child);
                    }
                }
                EntryMode::Hex => {
                    canvas.put_str(Point::new(R_X - 2, CUSTOM_Y), "#:", body);
                    let mut child = canvas.child(custom.hex.bounds());
                    custom.hex.draw(&mut child);
                }
            }
            let toggle_style = if custom.toggle_focused {
                self.theme.style(Role::ButtonFocused)
            } else {
                self.theme.style(Role::ButtonNormal)
            };
            let toggle_rect = custom.toggle_rect();
            canvas.fill(toggle_rect, &Cell::blank(toggle_style));
            let label = custom.toggle_label();
            let x = toggle_rect.origin().x
                + ((toggle_rect.width() - label.chars().count() as i16) / 2).max(0);
            canvas.put_str(Point::new(x, toggle_rect.origin().y), label, toggle_style);
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

#[cfg(test)]
mod tests {
    use super::*;

    // --- Grid index <-> (col, row), and clamped movement ---

    #[test]
    fn grid_pos_maps_index_to_column_and_row() {
        assert_eq!(grid_pos(0), (0, 0));
        assert_eq!(grid_pos(7), (7, 0));
        assert_eq!(grid_pos(8), (0, 1));
        assert_eq!(grid_pos(15), (7, 1));
    }

    #[test]
    fn grid_index_is_the_inverse_of_grid_pos() {
        for i in 0..16 {
            let (col, row) = grid_pos(i);
            assert_eq!(grid_index(col, row), i);
        }
    }

    #[test]
    fn color16_at_matches_the_canonical_all_order() {
        for i in 0..16 {
            assert_eq!(color16_at(i), Color16::ALL[i]);
        }
    }

    #[test]
    fn grid_move_steps_and_clamps_at_every_edge() {
        assert_eq!(grid_move(0, GridDirection::Left), 0, "clamped at left edge");
        assert_eq!(grid_move(0, GridDirection::Up), 0, "clamped at top edge");
        assert_eq!(
            grid_move(7, GridDirection::Right),
            7,
            "clamped at right edge"
        );
        assert_eq!(
            grid_move(15, GridDirection::Down),
            15,
            "clamped at bottom edge"
        );

        assert_eq!(grid_move(0, GridDirection::Right), 1);
        assert_eq!(grid_move(0, GridDirection::Down), 8);
        assert_eq!(grid_move(15, GridDirection::Left), 14);
        assert_eq!(grid_move(15, GridDirection::Up), 7);
    }

    // --- RGB channel field parsing ---

    #[test]
    fn parse_channel_accepts_in_range_decimal() {
        assert_eq!(parse_channel("0"), Some(0));
        assert_eq!(parse_channel("255"), Some(255));
        assert_eq!(parse_channel("42"), Some(42));
    }

    #[test]
    fn parse_channel_rejects_out_of_range_empty_and_non_digit() {
        assert_eq!(parse_channel("256"), None, "out of range");
        assert_eq!(parse_channel(""), None, "empty");
        assert_eq!(parse_channel("12a"), None, "non-digit");
        assert_eq!(parse_channel("-1"), None, "negative");
    }

    // --- Hex field parsing ---

    #[test]
    fn parse_hex_accepts_with_and_without_leading_hash() {
        assert_eq!(parse_hex("aabbcc"), Some((0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex("#AABBCC"), Some((0xaa, 0xbb, 0xcc)));
        assert_eq!(parse_hex("#000000"), Some((0, 0, 0)));
        assert_eq!(parse_hex("ffffff"), Some((255, 255, 255)));
    }

    #[test]
    fn parse_hex_rejects_wrong_length_and_non_hex_chars() {
        assert_eq!(parse_hex("abc"), None, "too short");
        assert_eq!(parse_hex("aabbccdd"), None, "too long");
        assert_eq!(parse_hex("gggggg"), None, "non-hex digits");
        assert_eq!(parse_hex(""), None, "empty");
        assert_eq!(parse_hex("#"), None, "just the hash");
    }

    // --- Last-touched wins the result variant ---

    #[test]
    fn seeding_from_named_starts_at_that_swatch_sourced_from_the_grid() {
        let t = TentativeColor::new(Color::Named(Color16::Red));
        assert_eq!(t.grid_index, 4, "Red is Color16::ALL[4]");
        assert_eq!(t.color(), Color::Named(Color16::Red));
    }

    #[test]
    fn seeding_from_rgb_starts_sourced_from_custom_entry() {
        let t = TentativeColor::new(Color::Rgb(12, 34, 56));
        assert_eq!(t.rgb, (12, 34, 56));
        assert_eq!(t.color(), Color::Rgb(12, 34, 56));
    }

    #[test]
    fn selecting_a_swatch_then_editing_custom_switches_to_rgb() {
        let mut t = TentativeColor::new(Color::Named(Color16::Blue));
        t.set_custom_rgb((1, 2, 3));
        assert_eq!(t.color(), Color::Rgb(1, 2, 3));
    }

    #[test]
    fn editing_custom_then_selecting_a_swatch_switches_back_to_named() {
        let mut t = TentativeColor::new(Color::Rgb(1, 2, 3));
        t.select_grid(0);
        assert_eq!(t.color(), Color::Named(Color16::Black));
    }

    #[test]
    fn custom_rgb_equal_to_a_named_swatch_still_reads_back_as_rgb() {
        let mut t = TentativeColor::new(Color::Named(Color16::Black));
        // Black's canonical RGB is (0, 0, 0) — typing that in by hand should
        // still report Rgb, since custom entry (not the grid) was touched.
        t.set_custom_rgb((0, 0, 0));
        assert_eq!(t.color(), Color::Rgb(0, 0, 0));
    }

    // --- ColorPicker construction & custom-entry gating ---

    use crate::buffer::Buffer;
    use crate::command::{CM_USER, Command, CommandSet};
    use crate::event::{KeyEvent, Modifiers};

    fn cga16() -> ColorPicker {
        ColorPicker::new(
            Color::Named(Color16::Blue),
            ColorProfile::Cga16,
            &Theme::default(),
        )
    }

    fn truecolor() -> ColorPicker {
        ColorPicker::new(
            Color::Named(Color16::Blue),
            ColorProfile::Truecolor,
            &Theme::default(),
        )
    }

    fn press(p: &mut ColorPicker, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        p.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn type_str(p: &mut ColorPicker, s: &str) {
        for c in s.chars() {
            press(p, KeyCode::Char(c));
        }
    }

    #[test]
    fn cga16_has_no_custom_entry_section() {
        assert!(cga16().custom.is_none());
    }

    #[test]
    fn truecolor_has_a_custom_entry_section_seeded_from_initial() {
        let p = truecolor();
        let custom = p.custom.as_ref().unwrap();
        assert_eq!(custom.r.text(), "0");
        assert_eq!(custom.g.text(), "0");
        assert_eq!(custom.b.text(), "170", "Color16::Blue is (0, 0, 170)");
        assert_eq!(custom.hex.text(), "0000AA");
    }

    #[test]
    fn seeding_from_named_places_the_grid_highlight_on_that_swatch() {
        let p = cga16();
        assert_eq!(p.tentative.grid_index, 1, "Blue is Color16::ALL[1]");
    }

    // --- Grid navigation: arrows move + select immediately ---

    #[test]
    fn arrow_keys_move_the_grid_and_update_color_immediately() {
        let mut p = cga16(); // starts at Blue, index 1
        assert_eq!(press(&mut p, KeyCode::Right), EventResult::Consumed);
        assert_eq!(p.color(), Color::Named(Color16::Green), "index 2");
        press(&mut p, KeyCode::Down);
        assert_eq!(p.color(), Color::Named(Color16::LightGreen), "index 10");
    }

    #[test]
    fn grid_arrows_clamp_at_the_edges() {
        let mut p = cga16();
        for _ in 0..20 {
            press(&mut p, KeyCode::Left);
        }
        assert_eq!(p.tentative.grid_index, 0, "clamped to column 0, same row");
    }

    #[test]
    fn a_click_on_a_swatch_selects_it_directly() {
        let mut p = cga16();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // Row 1 ("DarkGray"...), column 3 -> index 8 + 3 = 11 (LightCyan).
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(3 * SWATCH_W, 1),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(p.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(p.color(), Color::Named(Color16::LightCyan));
    }

    // --- Custom entry: parsing, gating, last-touched ---

    #[test]
    fn typing_a_valid_rgb_value_updates_color_to_rgb() {
        let mut p = truecolor();
        press(&mut p, KeyCode::Tab); // grid -> R field
        press(&mut p, KeyCode::Home);
        // Replace the seeded "0" by clearing it first.
        press(&mut p, KeyCode::Delete);
        type_str(&mut p, "12");
        assert_eq!(
            p.color(),
            Color::Rgb(12, 0, 170),
            "R updated, G/B keep their seeded values"
        );
    }

    #[test]
    fn an_out_of_range_value_leaves_the_previous_color_in_place() {
        let mut p = truecolor();
        press(&mut p, KeyCode::Tab); // R field
        press(&mut p, KeyCode::Home);
        press(&mut p, KeyCode::Delete); // "0" -> "", never parseable
        assert_eq!(
            p.color(),
            Color::Named(Color16::Blue),
            "clearing the field never parses; the seeded value stands"
        );

        // A value that briefly passes through a valid state ("9") then goes
        // out of range ("99" is fine, but this picker only has one digit to
        // clear first): once genuinely out of range, the last-parseable
        // value sticks rather than reverting all the way to the seed.
        type_str(&mut p, "99");
        assert_eq!(
            p.color(),
            Color::Rgb(99, 0, 170),
            "99 is in range and commits"
        );
        press(&mut p, KeyCode::Char('9')); // "999": out of range
        assert_eq!(
            p.color(),
            Color::Rgb(99, 0, 170),
            "999 doesn't parse; the last in-range value (99) stands"
        );
    }

    #[test]
    fn toggling_mode_preserves_the_numeric_value_across_the_switch() {
        let mut p = truecolor();
        press(&mut p, KeyCode::Tab); // R
        press(&mut p, KeyCode::Tab); // G
        press(&mut p, KeyCode::Tab); // B
        press(&mut p, KeyCode::Tab); // mode toggle
        assert_eq!(p.focus, Focus::ModeToggle);
        press(&mut p, KeyCode::Enter);
        let custom = p.custom.as_ref().unwrap();
        assert_eq!(custom.mode, EntryMode::Hex);
        assert_eq!(custom.hex.text(), "0000AA", "carries the same colour over");
    }

    #[test]
    fn selecting_a_swatch_after_custom_entry_switches_back_to_named() {
        let mut p = truecolor();
        press(&mut p, KeyCode::Tab);
        press(&mut p, KeyCode::Home);
        press(&mut p, KeyCode::Delete);
        type_str(&mut p, "12");
        assert_eq!(p.color(), Color::Rgb(12, 0, 170));
        // Tab back around to the grid and move it: grid wins again. 7 focus
        // stops total (Grid, R, G, B, ModeToggle, Ok, Cancel); currently on
        // R (stop 1), so 6 more Tabs wraps back to Grid (stop 0).
        for _ in 0..6 {
            press(&mut p, KeyCode::Tab);
        }
        assert_eq!(p.focus, Focus::Grid);
        press(&mut p, KeyCode::Right);
        assert_eq!(p.color(), Color::Named(Color16::Green));
    }

    // --- Tab order & OK/Cancel ---

    #[test]
    fn tab_cycles_through_every_focus_stop_and_wraps() {
        let mut p = truecolor();
        let expected = [
            Focus::Custom(0),
            Focus::Custom(1),
            Focus::Custom(2),
            Focus::ModeToggle,
            Focus::Ok,
            Focus::Cancel,
            Focus::Grid,
        ];
        for want in expected {
            press(&mut p, KeyCode::Tab);
            assert_eq!(p.focus, want);
        }
    }

    #[test]
    fn cga16_tab_order_skips_custom_entry_entirely() {
        let mut p = cga16();
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.focus, Focus::Ok);
        press(&mut p, KeyCode::Tab);
        assert_eq!(p.focus, Focus::Cancel);
        press(&mut p, KeyCode::Tab);
        assert_eq!(
            p.focus,
            Focus::Grid,
            "wraps, never touching Custom/ModeToggle"
        );
    }

    #[test]
    fn enter_commits_the_tentative_color_into_the_result_handle_and_posts_ok() {
        let mut p = cga16();
        press(&mut p, KeyCode::Right); // Green
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = p.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
        assert_eq!(*p.result.borrow(), Color::Named(Color16::Green));
    }

    #[test]
    fn enter_on_cancel_posts_cancel_without_touching_the_result() {
        let mut p = cga16();
        press(&mut p, KeyCode::Tab); // Ok
        press(&mut p, KeyCode::Tab); // Cancel
        assert_eq!(p.focus, Focus::Cancel);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = p.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_CANCEL)]);
        assert_eq!(*p.result.borrow(), Color::Default, "never written");
    }

    #[test]
    fn a_click_on_ok_focuses_and_posts_it() {
        let mut p = cga16();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let bounds = p.ok.bounds();
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: bounds.origin(),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(p.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
    }

    #[test]
    fn a_click_on_ok_writes_the_tentative_color_into_the_result_handle() {
        // Regression: clicking OK routed the mouse event straight into the
        // `Button` widget, which only posts `CM_OK` -- it never calls
        // `ColorPicker::accept`, so `result` (read via `ColorPickerResult`
        // after the modal ends) stayed at its never-set `Color::Default`
        // regardless of what was actually selected. Enter went through
        // `on_enter` -> `accept` and worked correctly, which is what made
        // this so easy to miss with keyboard-only manual testing.
        let mut p = cga16(); // starts at Blue, index 1
        press(&mut p, KeyCode::Right); // -> Green
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: p.ok.bounds().origin(),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(p.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
        assert_eq!(*p.result.borrow(), Color::Named(Color16::Green));
    }

    // A stray application command must never be confused with the picker's
    // own CM_OK/CM_CANCEL.
    const CM_UNRELATED: Command = Command(CM_USER + 1);

    #[test]
    fn cm_ok_and_cm_cancel_are_the_only_commands_this_control_posts() {
        assert_ne!(CM_OK, CM_UNRELATED);
        assert_ne!(CM_CANCEL, CM_UNRELATED);
    }

    // --- pick(): the assembled Window (ADR 0016) ---

    #[test]
    fn pick_builds_a_centred_fixed_window_ending_on_ok_and_cancel() {
        let (w, _) = ColorPicker::pick(
            "Colour",
            Color::Named(Color16::Blue),
            ColorProfile::Cga16,
            &Theme::default(),
        );
        assert_eq!(w.placement(), crate::widgets::Placement::Centered);
        assert!(w.ends_on(CM_OK));
        assert!(w.ends_on(CM_CANCEL));
        assert!(!w.ends_on(CM_UNRELATED));
    }

    #[test]
    fn esc_cancels_via_the_window_not_the_interior() {
        let (mut w, _) = ColorPicker::pick(
            "Colour",
            Color::Named(Color16::Blue),
            ColorProfile::Cga16,
            &Theme::default(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
        assert_eq!(w.handle_event(&esc, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_CANCEL)]);
    }

    #[test]
    fn accepting_through_the_window_updates_the_result_handle() {
        let (mut w, result) = ColorPicker::pick(
            "Colour",
            Color::Named(Color16::Blue),
            ColorProfile::Cga16,
            &Theme::default(),
        );
        assert_eq!(result.color(), Color::Default, "never accepted yet");
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        w.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Right, Modifiers::NONE)),
            &mut ctx,
        );
        let r = w.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(result.color(), Color::Named(Color16::Green));
    }

    #[test]
    fn cancelling_through_the_window_leaves_the_result_handle_untouched() {
        let (mut w, result) = ColorPicker::pick(
            "Colour",
            Color::Named(Color16::Red),
            ColorProfile::Cga16,
            &Theme::default(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        w.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Esc, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(result.color(), Color::Default);
    }

    #[test]
    fn snapshot_pick_window() {
        let (w, _) = ColorPicker::pick(
            "Colour",
            Color::Named(Color16::Blue),
            ColorProfile::Cga16,
            &Theme::default(),
        );
        let mut buf = Buffer::new(w.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    // --- Render (snapshot) ---

    fn render(p: &ColorPicker) -> String {
        let mut buf = Buffer::new(p.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        p.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn snapshot_cga16_grid_only() {
        insta::assert_snapshot!(render(&cga16()));
    }

    #[test]
    fn snapshot_truecolor_rgb_mode() {
        insta::assert_snapshot!(render(&truecolor()));
    }

    #[test]
    fn snapshot_truecolor_hex_mode() {
        let mut p = truecolor();
        // Tab to the toggle and switch to hex mode.
        for _ in 0..4 {
            press(&mut p, KeyCode::Tab);
        }
        assert_eq!(p.focus, Focus::ModeToggle);
        press(&mut p, KeyCode::Enter);
        insta::assert_snapshot!(render(&p));
    }

    #[test]
    fn snapshot_grid_highlight_on_a_named_initial() {
        let p = ColorPicker::new(
            Color::Named(Color16::Yellow),
            ColorProfile::Cga16,
            &Theme::default(),
        );
        insta::assert_snapshot!(render(&p));
    }
}
