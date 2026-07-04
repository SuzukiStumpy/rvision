# Module spec: `rvision::widgets::Window` + `MessageBox` + `FileDialog`

- **Status:** Draft
- **Phase:** post-extraction rework (SDI/MDI convergence)
- **Related ADRs:** 0016 (unify `Window`/`Dialog`, dynamic desktop), 0015 (scroll
  chrome protocol), 0017 (resize propagation protocol), 0011 (drop shadows),
  0010 (`exec_view` + focus-aware drawing), 0009 (`Shell`), 0007 (mouse),
  0003/0004 (tree + dispatch), 0021 (window-scoped context help)

## Purpose

The one bordered-box-with-an-interior type in the framework: a [`Frame`]
around a `Box<dyn View>` interior, optionally configured as fixed-size,
centred, and modal-mannered (ADR 0016). `Dialog` no longer exists as a
separate struct — a "dialog" is a `Window` built with that configuration.
`MessageBox` and `FileDialog` are factories that build one.

It is **not** where a window lives — that's [`Desktop`](desktop.md) for a
tree-resident window, `Application::exec_view` for a modally-run one (see
[`app.md`](app.md)). `Window` itself has no opinion about which; the same
value can be handed to either.

**SDI is still a first-class use, not a mode this design leaves behind.**
`resizable`/`moveable`/`closable`/`zoomable` are independent flags, not parts
of an MDI/SDI switch — an application that only ever wants one, fixed,
undismissable window opens a single `Window::new(bounds, ...)` sized to fill
the desktop with `.resizable(false).moveable(false)` and never calls
`Desktop::open` again; `Desktop`'s drag/resize sessions simply never start
(see [`desktop.md`](desktop.md)) and there is nothing else to converge. An
application that wants *no* frame/chrome at all can also skip `Window`/
`Desktop` entirely and hand `Root::new` a single full-screen `View` directly
— exactly what `edit` already does for unrelated reasons (its own ADR 0018)
— untouched by anything in ADR 0016.

## Public interface

```rust
pub enum Placement { Positioned, Centered }

pub struct Window {
    bounds: Rect,
    frame: Frame,
    active: bool,
    maximized: bool,
    restore_bounds: Option<Rect>,   // pre-zoom bounds, while maximized
    interior_fill: Cell,
    shadow_style: Style,
    casts_shadow: bool,
    interior: Box<dyn View>,
    resizable: bool,
    moveable: bool,
    closable: bool,
    zoomable: bool,
    help_topic: Option<String>,        // ADR 0021: opaque HelpContents topic id
    placement: Placement,
    ending: Vec<Command>,
    default_cmd: Option<Command>,
    esc_cancels: bool,
    visible: bool,
}

impl Window {
    /// A window at `bounds`, fully capable by default: resizable, moveable,
    /// closable, zoomable, `Positioned`, no ending commands, visible.
    pub fn new(bounds: Rect, title: &str, theme: &Theme, interior: Box<dyn View>) -> Self;

    // Construction-time configuration (consuming builders, existing style).
    pub fn resizable(self, yes: bool) -> Self;
    pub fn moveable(self, yes: bool) -> Self;
    pub fn closable(self, yes: bool) -> Self;
    pub fn zoomable(self, yes: bool) -> Self;
    pub fn centered(self) -> Self;                    // placement = Centered
    pub fn with_default(self, command: Command) -> Self;
    pub fn also_ends_on(self, command: Command) -> Self;
    pub fn esc_cancels(self, yes: bool) -> Self;
    pub fn with_help_topic(self, topic: impl Into<String>) -> Self;  // ADR 0021

    // Queries.
    pub fn ends_on(&self, command: Command) -> bool;
    pub fn interior_bounds(&self) -> Rect;             // unchanged
    pub fn is_active(&self) -> bool;
    pub fn is_visible(&self) -> bool;
    pub fn is_maximized(&self) -> bool;
    pub fn placement(&self) -> Placement;
    pub fn help_topic(&self) -> Option<&str>;          // ADR 0021

    // Runtime mutation (existing style: plain setters).
    pub fn set_active(&mut self, active: bool);         // unchanged
    pub fn set_casts_shadow(&mut self, casts: bool);     // unchanged
    pub fn set_title(&mut self, title: &str);           // forwards to Frame::set_title; cosmetic only
    pub fn set_bounds(&mut self, bounds: Rect);          // drag/resize; propagates to the interior (ADR 0017)
    pub fn hide(&mut self);
    pub fn show(&mut self);
    pub fn toggle_zoom(&mut self, desktop_bounds: Rect); // fills/restores against the caller's area
}

impl View for Window {
    // draw/handle_event as today, extended below; focusable() unchanged (true).
    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
        self.interior.valid(command, ctx)   // delegates; Window has no opinion of its own
    }
}

// --- MessageBox: builds a centred, fixed, ending Window ---
pub struct MessageBox;
impl MessageBox {
    pub fn ok(title: &str, message: &str, theme: &Theme) -> Window;
    pub fn ok_cancel(title: &str, message: &str, theme: &Theme) -> Window;
    pub fn yes_no(title: &str, message: &str, theme: &Theme) -> Window;
}

// --- FileDialog: builds a centred, fixed, ending Window; result via a shared handle ---
pub struct FileDialog { /* unchanged internals: reader, dir, entries, list, input, open, cancel, focus */ }
impl FileDialog {
    pub fn open(title: &str, dir: impl Into<PathBuf>, theme: &Theme) -> (Window, FileDialogResult);
    pub fn save(title: &str, dir: impl Into<PathBuf>, theme: &Theme) -> (Window, FileDialogResult);
}
/// The chosen path, readable after `exec_view` returns `CM_OK`. `FileDialog`
/// itself becomes the window's boxed, type-erased interior (ADR 0003), so
/// this is the narrow, single-purpose seam back out — an `Rc<RefCell<_>>`
/// the interior writes into on accept, the same shared-cell idiom already
/// used throughout the crate's own tests (`Recorder`, `FocusSpy`, ...), not
/// a new pattern.
pub struct FileDialogResult(Rc<RefCell<PathBuf>>);
impl FileDialogResult {
    pub fn path(&self) -> PathBuf;
}
```

## Behaviour & invariants

- **Draw.** Frame first (title, border doubled iff `active`, close glyph iff
  `closable`, zoom glyph iff `zoomable`, **help glyph iff `help_topic` is
  `Some`, drawn immediately left of the zoom glyph (ADR 0021)** — reflecting
  `maximized` via `Frame::maximized`/`Frame::set_maximized` mirroring
  `set_active`), then the interior through the inset sub-canvas, as today.
  The existing `glyphs_shown(width)` all-or-nothing gate is extended to
  budget for three glyphs instead of two, still a single boolean — a narrow
  frame shows all of close/zoom/help or none of them (ADR 0021 rejected
  per-glyph dropping). **Scroll chrome
  (ADR 0015):** if `interior.scroll_metrics()` returns `Some`, `Window`
  reserves one column/row per axis that needs one just inside the border,
  draws a `ScrollBar` there, and on a click/drag landing in that gutter calls
  `interior.set_scroll(...)` with the result — the same host pattern
  `FileDialog` proves for its own embedded `ListBox` (ADR 0015), generalised
  to any interior. No current interior in this crate needs both a window
  frame *and* this hosting yet (`FileDialog`'s own scrolling is handled one
  level down, between it and its `ListBox`); this is the seam ADR 0015 built
  for a future `TextEdit`/`HelpWindow` interior, proven here with a fake
  scrollable test interior.
- **Border-glyph clicks.** A mouse-down at row 0 within `Frame::close_span`
  posts `CM_CLOSE` (only if `closable`) and consumes; within
  `Frame::zoom_span` posts `CM_ZOOM` (only if `zoomable`) and consumes; within
  the new help-glyph span posts the existing `CM_HELP` (only if `help_topic`
  is `Some`) and consumes (ADR 0021) — no new `Command` id, and resolution of
  *which* topic happens later, when whatever catches `CM_HELP` reads
  `help_topic()` back off the active window (see [`shell.md`](shell.md)).
  No glyph is interactive when its condition is off — each is simply not
  drawn then (see Draw), so there is nothing to hit.
- **Interior routing.** Unchanged: a mouse inside `interior_bounds` is
  translated and forwarded; keys/commands/broadcasts/paste go to the interior
  and its `Ignored` results bubble out (ADR 0003).
- **Everything else on the border** (title bar, resize corner) is left
  `Ignored` by `Window` itself — that silence is deliberate: it is exactly
  what tells [`Desktop`](desktop.md) (or `exec_view`'s caller) that no session
  should start there *unless* the owner recognises it as one. `Window` has no
  concept of a drag session; that state lives one level up, mirroring how
  `MenuBar` (not a menu item) owns its own open/closed machinery.
- **Esc / default command (from the old `Dialog`, unchanged in effect).** If
  `esc_cancels`, `Esc` posts `CM_CANCEL` before the interior sees it. If the
  interior ignores `Enter` and `default_cmd` is `Some`, that command is
  posted. A plain `Window::new()` has `esc_cancels: false` and
  `default_cmd: None`, so it behaves exactly as today's `Window` — these are
  additive, not a new mode.
- **`ends_on`.** `true` for anything in `ending` (empty by default). Only
  consulted by `exec_view` (see [`app.md`](app.md)); a tree-resident `Window`
  with an empty `ending` simply never has anything to end.
- **`valid` delegates to the interior** (see interface). A `Window` never
  vetoes on its own behalf; whatever it wraps decides (TV's `TView::valid`
  default, composed one level).
- **`visible`.** A `Window`'s own `draw`/`handle_event` do **not** check it —
  visibility is [`Desktop`](desktop.md)'s concern (it skips a hidden window
  entirely in draw, hit-testing, and active-window dispatch) and is
  meaningless to `exec_view` (a window being run modally is shown for the
  run's duration regardless of the flag). `hide`/`show` just toggle the flag
  and, for `show`, are TurboVision's own naming (`TView::hide`/`show`,
  `sfVisible`) — the *raise-to-top* half of `show`'s effect is `Desktop`'s
  job, not `Window`'s (a `Window` doesn't know its own stack position).
- **`toggle_zoom`.** Fills `desktop_bounds`, remembering the prior `bounds` in
  `restore_bounds`; called again, restores it and clears `restore_bounds`.
  Sets `casts_shadow(false)` while maximized (a shadow off the edge of the
  desktop is pointless — same reasoning `set_casts_shadow`'s doc already
  gives) and restores the prior shadow setting on restore.
- **Resize propagation to the interior (ADR 0017).** Both `set_bounds` and
  `toggle_zoom` call `self.interior.set_bounds(self.interior_bounds())`
  immediately after updating `self.bounds` — the one and only place `Window`
  tells its interior "your area changed." Most interiors ignore it (the
  default `View::set_bounds` is a no-op); one that lays out a size-dependent
  cache (e.g. [`HelpWindow`](help_window.md)'s composite `ListBox`+`HelpPane`
  interior) overrides it to relayout and cascade to its own children. A
  `Window` locked `.resizable(false)` (`FileDialog`, `MessageBox`) never
  triggers the resize half of this in practice, but the zoom half still can —
  those interiors just don't override `set_bounds`, so it's a no-op either
  way.
- **`MessageBox`.** Unchanged behaviour from `Dialog`-based `MessageBox`,
  rebuilt on `Window`: `.centered().resizable(false).zoomable(false)`, message
  lines + buttons as a `Group` interior, first button `.with_default`, every
  button's command `.also_ends_on`.
- **`FileDialog`.** Unchanged navigation/selection logic (Tab-cycled
  list/input/Open/Cancel, `Enter`/double-click semantics) but now *is* the
  boxed interior rather than the whole modal view: no more `size`/`title`/
  outer `style`/`drop_shadow`/`Modal` impl on `FileDialog` itself — those move
  to the `Window` its `open`/`save` constructors build
  (`.centered().resizable(false).zoomable(false).with_default(CM_OK).esc_cancels(true)`).
  Its embedded `ListBox` is hosted via the ADR 0015 scroll protocol instead of
  building its own `ScrollBar` inline (see [`controls.md`](controls.md)).

## Collaborators

- `Frame` (border/glyphs/title; the help glyph is a third span alongside
  `close_span`/`zoom_span`, ADR 0021).
  `ScrollBar`/`ScrollMetrics`/`AxisMetrics` (ADR 0015, hosting).
- `view::{View, Group, Context}`, `command::{Command, CommandSet, CM_OK,
  CM_CANCEL, CM_YES, CM_NO, CM_CLOSE, CM_ZOOM, CM_HELP}` (`CM_CLOSE`/`CM_ZOOM`
  are framework-reserved ids, below `CM_USER`, alongside `CM_QUIT` et al. —
  `Desktop` acts on `CM_CLOSE`/`CM_ZOOM`; `CM_HELP` is acted on by `Shell`
  (ADR 0021, see [`shell.md`](shell.md)); `Window` only ever posts any of them,
  never acts on one itself).
- `widgets::{Button, Label, InputLine, ListBox}` (what `MessageBox`/
  `FileDialog` compose as interiors).
- `Desktop` (owns tree-resident `Window`s, drives drag/resize/hide/show) and
  `Application::exec_view` (runs one modally) — see [`desktop.md`](desktop.md)
  and [`app.md`](app.md). Neither is a dependency `Window` itself has; both
  depend on `Window`.

## Test plan (write these first)

- **Logic:** default flags from `Window::new`; builder toggles; `ends_on`
  covers `ending` plus additions; `toggle_zoom` round-trips bounds and shadow;
  `hide`/`show` toggle `is_visible` (raising is `Desktop`'s test, not this
  one).
- **Render (snapshot):** close/zoom glyphs present/absent per flag; help
  glyph present only when `help_topic` is `Some`, positioned left of zoom;
  maximized glyph swap; all three glyphs drop together (not individually) on
  a frame too narrow for them; a scroll-hosting interior gets a border
  `ScrollBar` in the reserved gutter, a non-scrolling one doesn't.
- **Interaction (scripted events):** click on `close_span` posts `CM_CLOSE`
  only when `closable`; same for `CM_ZOOM`/`zoomable`; same for the help span
  and `CM_HELP`/`help_topic` (ADR 0021); `Esc` posts
  `CM_CANCEL` only when `esc_cancels`; `Enter` falls back to `default_cmd`
  only when the interior ignores it; a click in the scroll gutter calls
  `set_scroll` on the interior with the expected offset; `valid` delegates to
  (and returns) the interior's answer, including a refusal that also posts a
  follow-up via `ctx`; `set_bounds` and `toggle_zoom` each call
  `interior.set_bounds` with the new `interior_bounds()` (ADR 0017, via a fake
  interior that records what it was told).
- **FileDialog-specific:** unchanged tests from today (navigation, `..`,
  double-click accept, Tab cycling), rehomed onto the interior type;
  `FileDialogResult::path` reflects what the interior wrote after `CM_OK`.
- **Manual:** the `dialogs` example; a resizable/moveable window on the
  desktop example once [`desktop.md`](desktop.md) lands.

## Open questions

- **Resize handle shape.** Spec assumes a single bottom-right corner grab
  point (matching `Frame`'s existing dedicated corner glyphs and classic TV
  behaviour), not full-edge resize. Revisit only if a corner turns out too
  fiddly in practice on a real terminal.
- **`FileDialogResult`'s shared-cell shape.** `Rc<RefCell<PathBuf>>` is the
  minimal version; if a second stateful dialog needs the same "read a result
  after the window closes" pattern, consider a small generic helper instead
  of hand-rolling another one-off wrapper.
- **Per-window command gating** (carried over from the old `Dialog` spec,
  still unresolved): `exec_view` runs under a fresh all-enabled `CommandSet`;
  greying a button until a field validates would need a `Window` to own its
  own `CommandSet`, or a validity hook of its own. Deferred.
- **Hardware cursor** for a focused input line — drawn as a cell now (ADR
  0010), a real terminal cursor when the editor needs one (Phase 6, `edit`-side).
