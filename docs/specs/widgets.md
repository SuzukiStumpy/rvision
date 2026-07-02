# Module spec: `rvision::widgets`

- **Status:** Done for the backdrop/status-line/menu-bar chrome here; `Window`
  and `Desktop` themselves moved out to their own specs (Draft) — see below
- **Phase:** 4 (Application chrome) — kept current through Phases 5–6
- **Related ADRs:** 0003 (retained tree, commands up / broadcasts down), 0004 (three-phase dispatch), 0005 (colour roles), 0008 (owner-relative coords + `Canvas`), 0009 (application shell + menu overlay)

## Purpose

The Phase 4 chrome widget family: the concrete `View`s that make a screen look
like TurboVision — a desktop backdrop, framed windows, a status line, and a menu
bar with pull-downs. Reusable, editor-agnostic. These are the *furniture* around
the focus-and-content widgets.

**This file specs the backdrop/status-line/menu-bar chrome only.** `Window`
and `Desktop` — originally chrome specced here too — moved to their own specs
once ADR 0016 made them a dynamic MDI container absorbing `Dialog`:

- **`Window`, `MessageBox`, `FileDialog`** (the framed box, and its modal
  configurations) — see [`window.md`](window.md).
- **`Desktop`** (the dynamic window stack: open/close/drag/resize/focus) —
  see [`desktop.md`](desktop.md).
- **Controls (Phase 5)** — `Label`, `Button`, `InputLine`, `CheckBox`,
  `RadioButtons`, `ListBox`/`ListViewer`, `ScrollBar`: see
  [`controls.md`](controls.md).
- The editor view itself lives in the `edit` crate, not here
  ([`editor.md`](editor.md)).

It is **not** the application root: the layout, draw-ordering, menu overlay, and
accelerator routing that tie these together live in `app::Shell` (ADR 0009,
[`shell.md`](shell.md)).

## Public interface

```rust
// --- Background: a backdrop fill ---
pub struct Background { bounds: Rect, cell: Cell }
impl Background {
    pub fn new(bounds: Rect, cell: Cell) -> Self;     // e.g. '░' in DesktopBackground
}

// --- Frame: a window border with title + close/zoom glyphs ---
// A drawing helper, not an independent View — it always paints the whole
// canvas it is handed (a window's outer rect); Window owns one. close_span/
// zoom_span expose the glyphs' column ranges so an owner can hit-test a
// click without re-deriving the layout (ADR 0007) — see window.md.
pub struct Frame { title: String, active: bool, maximized: bool, style: Style, title_style: Style }
impl Frame {
    pub fn new(title: &str, style: Style, title_style: Style) -> Self;
    pub fn active(self, active: bool) -> Self;         // builder; active = doubled corners
    pub fn maximized(self, maximized: bool) -> Self;   // builder; swaps the zoom glyph to ↕
    pub fn set_active(&mut self, active: bool);
    pub fn close_span(width: i16) -> Option<Range<i16>>;  // None if too narrow to show glyphs
    pub fn zoom_span(width: i16) -> Option<Range<i16>>;
}

// --- Window, MessageBox, FileDialog, Desktop: see window.md / desktop.md ---
// (moved out of this file — ADR 0016 made them a dynamic MDI container
// absorbing the old Dialog, too large to stay chrome-file furniture)

// --- StatusLine: global hot-key items (carved to a region by the shell) ---
pub struct StatusItem { hint: String, label: String, key: KeyEvent, command: Command }
impl StatusItem {
    pub fn new(hint: &str, label: &str, key: KeyEvent, command: Command) -> Self;
}
pub struct StatusLine { bounds: Rect, items: Vec<StatusItem>, style: Style, key_style: Style }
impl StatusLine {
    pub fn new(bounds: Rect, items: Vec<StatusItem>, style: Style, key_style: Style) -> Self;
    pub fn set_bounds(&mut self, bounds: Rect);
}
impl View for StatusLine { /* a matching KeyEvent posts its (enabled) command */ }

// --- MenuBar + Menu: titles across the top, pull-downs below ---
pub struct MenuItem { label: String, command: Command, shortcut: Option<String>, hotkey: Option<char> }
impl MenuItem {
    pub fn new(label: &str, command: Command) -> Self;    // hotkey defaults to label's first letter
    pub fn with_shortcut(self, shortcut: &str) -> Self;   // display-only label
    pub fn with_hotkey(self, hotkey: char) -> Self;        // override, e.g. Save/Save As collision
}
pub struct Menu { title: String, items: Vec<MenuItem>, hotkey: Option<char> }
impl Menu {
    pub fn new(title: &str, items: Vec<MenuItem>) -> Self; // hotkey defaults to title's first letter
    pub fn with_hotkey(self, hotkey: char) -> Self;         // override the Alt hot-key
}
pub struct MenuBar { bounds: Rect, menus: Vec<Menu>, open: Option<usize>, highlight: usize, .. }
impl MenuBar {
    pub fn new(bounds: Rect, menus: Vec<Menu>, theme: &Theme) -> Self;
    pub fn set_bounds(&mut self, bounds: Rect);
    pub fn is_open(&self) -> bool;
    pub fn close(&mut self);
    pub fn draw_overlay(&self, canvas: &mut Canvas);      // the pull-down, full-frame canvas
}
impl View for MenuBar { /* draws the bar; handle_event runs the menu state machine */ }
```

> The chrome constructors take their `bounds` because `app::Shell`/`edit::app`
> carve a region per widget from the live terminal size each frame and re-seat them
> via `set_bounds` on resize (ADR 0009). `Background` (a plain backdrop fill) is the
> exception — it is a leaf used where a static fill is wanted.

## Behaviour & invariants

- **Drawing.** Every chrome widget draws into the canvas it is handed, sized to
  its assigned region (the shell carves these from the live terminal size — ADR
  0009 — so widgets do not lay themselves out from their own `bounds`). All writes
  clip (ADR 0008).
- **Frame.** Single-line box; the title is centred-ish on the top border with a
  space either side; an *active* frame uses doubled-corner glyphs; close `[■]` and
  zoom `[↑]`/`[↕]` (maximised) glyphs sit on the top border, only when the frame
  is wide enough (`close_span`/`zoom_span` return `None` below that width) —
  `Window` is what turns a click landing in one of those spans into an action
  (see [`window.md`](window.md)). Degrades without panic for tiny rects.
- **Window, MessageBox, FileDialog, Desktop.** Specced separately — see
  [`window.md`](window.md) and [`desktop.md`](desktop.md).
- **StatusLine.** A `Key` whose code equals an item's `key` posts that item's
  command (enabled-gated by `Context`, ADR 0003) and is consumed; other events are
  ignored. Drawn left→right, each item's key glyph in `key_style`.
- **MenuBar / Menu.** A small state machine (ADR 0009), no modal loop yet:
  - *Closed:* consumes `Alt`+a title's hot-key (opens that menu) and `F10` (opens
    the first menu). Every other event is ignored, so it never eats the editor's
    keys.
  - *Open:* modal — consumes every `Key`. `Left`/`Right` switch the open menu
    (wrap), `Up`/`Down` move the highlight (wrap), `Enter` posts the highlighted
    item's command and closes, a plain letter matching an item's hot-key posts
    that item's command and closes (no `Up`/`Down` needed), `Esc` closes. A
    disabled item's command is gated by `Context` (never posted); selecting it
    (via `Enter` or its hot-key) still closes the menu like TV.
  - Each `Menu`/`MenuItem`'s hot-key defaults to its title/label's first letter
    (case-insensitive) and is highlighted in `Role::MenuHotkey`; the app overrides
    it with `with_hotkey` once two items in the same menu would otherwise collide
    (e.g. "Cut" and "Copy" both defaulting to `c`).
  - The bar draws titles separated by spaces; the open title is highlighted.
    `draw_overlay` draws the pull-down box under the open title with items, their
    shortcuts right-aligned, the highlight in `MenuSelected`, disabled items in
    `MenuDisabled` (with no hot-key highlight — a letter that can't be pressed
    isn't singled out). The overlay is the shell's last draw, over the whole frame.

## Collaborators

- `Canvas`/`Buffer` (draw), `geometry` (`Rect`/`Point`/`Size`), `cell::Cell`.
- `theme::{Role, Theme}` (colours by role, ADR 0005), `color::Style`.
- `view::{View, Group, Context}`, `command::{Command, CommandSet}`, `event` types.
- Widgets never reference one another: a control posts via `Context`; the shell
  routes events to them and draws them (ADR 0003, 0009).

## Test plan (write these first)

- **Render (snapshot):** backdrop fill; an active/inactive frame with title +
  glyphs; a status line; the menu bar closed and with one menu open (bar +
  pull-down overlay). (`Window`/`Desktop` rendering: [`window.md`](window.md)/
  [`desktop.md`](desktop.md).)
- **Interaction (scripted events):** status-line key posts the right command and a
  disabled one does not; menu opens on `Alt`-letter/`F10`, `Left`/`Right` and
  `Up`/`Down` move within wrap, `Enter` posts + closes, a hot-key letter posts +
  closes without `Up`/`Down`, an unmatched letter is swallowed, `Esc` closes; a
  closed menu bar ignores ordinary keys.
- **Manual:** the `chrome` example on a real terminal (see [`shell.md`](shell.md)).

## Open questions

- Focus-aware frame styling reads a stored `active` flag set by the desktop; the
  general focus-in-draw question (ADR-tracked in `view.md`) is otherwise unchanged.
