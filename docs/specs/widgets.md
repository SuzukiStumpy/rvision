# Module spec: `rvision::widgets`

- **Status:** Done for the backdrop/status-line chrome here; `Window`,
  `Desktop`, and `MenuBar`/`Menu`/`MenuItem` themselves moved out to their own
  specs — see below
- **Phase:** 4 (Application chrome) — kept current through Phases 5–6
- **Related ADRs:** 0003 (retained tree, commands up / broadcasts down), 0004 (three-phase dispatch), 0005 (colour roles), 0008 (owner-relative coords + `Canvas`), 0009 (application shell + menu overlay), 0028 (`StatusLine` unified with the global accelerator table)

## Purpose

The Phase 4 chrome widget family: the concrete `View`s that make a screen look
like TurboVision — a desktop backdrop, framed windows, a status line, and a menu
bar with pull-downs. Reusable, editor-agnostic. These are the *furniture* around
the focus-and-content widgets.

**This file specs the backdrop/status-line chrome only.** `Window`, `Desktop`,
and `MenuBar` — originally chrome specced here too — moved to their own specs
once they grew past "furniture":

- **`Window`, `MessageBox`, `FileDialog`** (the framed box, and its modal
  configurations) — see [`window.md`](window.md) (moved once ADR 0016 made
  them a dynamic MDI container absorbing `Dialog`).
- **`Desktop`** (the dynamic window stack: open/close/drag/resize/focus) —
  see [`desktop.md`](desktop.md).
- **`MenuBar`, `Menu`, `MenuItem`** (the top bar and its cascading
  pull-downs) — see [`menu.md`](menu.md) (moved once submenus grew its state
  machine and overlay drawing past chrome-file size).
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

// --- StatusLine: hot-key hints (carved to a region by the shell) ---
// Purely a display widget (ADR 0028) — the Accelerator each item carries is
// harvested by Shell::new into Desktop's global accelerator table, which is
// what actually fires it; see command.md / desktop.md.
pub struct StatusItem { hint: String, label: String, accelerator: Accelerator }
impl StatusItem {
    pub fn new(hint: &str, label: &str, accelerator: Accelerator) -> Self;
}
pub struct StatusLine { bounds: Rect, items: Vec<StatusItem>, style: Style, key_style: Style }
impl StatusLine {
    pub fn new(bounds: Rect, items: Vec<StatusItem>, style: Style, key_style: Style) -> Self;
    pub fn set_bounds(&mut self, bounds: Rect);
    // pub(crate) fn accelerators(&self) -> impl Iterator<Item = Accelerator> + '_;
}
impl View for StatusLine { /* draw only — no handle_event override */ }

// --- MenuBar, Menu, MenuItem: see menu.md ---
// (moved out of this file once cascading submenus grew the state machine and
// overlay drawing too large to stay chrome-file furniture)
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
- **StatusLine.** A pure display widget (ADR 0028): drawn left→right, each
  item's key glyph in `key_style`. It no longer intercepts keys itself —
  each item's `Accelerator` is harvested into `Desktop`'s global accelerator
  table by `Shell::new`, which is what actually posts the (enabled-gated)
  command; see [`desktop.md`](desktop.md).
- **MenuBar / Menu.** Specced separately — see [`menu.md`](menu.md).

## Collaborators

- `Canvas`/`Buffer` (draw), `geometry` (`Rect`/`Point`/`Size`), `cell::Cell`.
- `theme::{Role, Theme}` (colours by role, ADR 0005), `color::Style`.
- `view::{View, Group, Context}`, `command::{Command, CommandSet}`, `event` types.
- Widgets never reference one another: a control posts via `Context`; the shell
  routes events to them and draws them (ADR 0003, 0009).

## Test plan (write these first)

- **Render (snapshot):** backdrop fill; an active/inactive frame with title +
  glyphs; a status line. (`Window`/`Desktop` rendering: [`window.md`](window.md)/
  [`desktop.md`](desktop.md); `MenuBar` rendering: [`menu.md`](menu.md).)
- **Interaction (scripted events):** status-line key posts the right command and a
  disabled one does not. (`MenuBar` interaction: [`menu.md`](menu.md).)
- **Manual:** the `chrome` example on a real terminal (see [`shell.md`](shell.md)).

## Open questions

- Focus-aware frame styling reads a stored `active` flag set by the desktop; the
  general focus-in-draw question (ADR-tracked in `view.md`) is otherwise unchanged.
