# Module spec: `rvision::app::Shell`

- **Status:** Done
- **Phase:** 4 (Application chrome)
- **Related ADRs:** 0009 (this composite), 0003/0004 (tree + dispatch it sits on),
  0008 (`Canvas`), 0005 (roles), 0018 (cascading menus), 0019 (context menus,
  `context_menu` field + drained anchor request), 0021 (window-scoped context
  help, `help`/`help_window` fields), 0028 (global keyboard accelerator
  table: `Shell::new` harvests `StatusLine`'s bindings into `Desktop`)

## Purpose

The standard application screen — TurboVision's `TProgram`: a menu bar across the
top, a status line across the bottom, and a [`Desktop`](widgets.md) between. It is
the one purpose-built root the generic [`Group`](view.md) cannot be (ADR 0009),
because it lays out live, draws the menu overlay last, and routes keys in three
local passes. It is an ordinary [`View`], so it lives inside [`Root`] and runs in
the [`Application`] loop unchanged.

It is **not** the loop (`Application`/`Program`) nor a widget — it is the thing
that arranges the Phase 4 widgets into a screen and wires their events.

## Public interface

```rust
pub struct Shell {
    menu_bar: MenuBar,
    desktop: Desktop,
    status_line: StatusLine,
    size: Size,
    theme: Theme,                        // kept to build ContextMenu/HelpWindow on demand
    context_menu: Option<ContextMenu>,    // ADR 0019: the open one, if any
    help: Option<HelpContents>,           // ADR 0021: app-supplied help content, opt-in
    help_window: Option<WindowId>,        // ADR 0021: last help window opened, for singleton reuse
}
impl Shell {
    /// Feeds every `status_line` item's `Accelerator` into `desktop`'s
    /// global accelerator table (ADR 0028) before the rest of construction.
    pub fn new(size: Size, menu_bar: MenuBar, desktop: Desktop, status_line: StatusLine, theme: &Theme) -> Self;
    pub fn menu_is_open(&self) -> bool;
    // So application code can dynamically open/close/hide/show desktop
    // windows (ADR 0016) — Shell owns the Desktop by value, so this is the
    // only way in from outside the view tree.
    pub fn desktop_mut(&mut self) -> &mut Desktop;
    /// Opts `Shell` into handling `CM_HELP` itself (ADR 0021). Without this,
    /// `CM_HELP` falls through to the desktop exactly as any other
    /// unrecognised command does — zero cost, zero behaviour change.
    pub fn with_help(self, contents: HelpContents) -> Self;
}
impl View for Shell { /* draws all four, routes events (below) */ }
```

## Behaviour & invariants

- **Live layout.** `regions(size)` carves three rectangles: menu bar `= row 0`,
  status line `= row h-1`, desktop `= rows 1..h-1` (all full width; heights clamp
  at small sizes). `draw` uses `canvas.size()`; `Event::Resize` calls `relayout`,
  which stores the size and `set_bounds` on each child. The stored size feeds
  positional routing between frames.
- **Draw order (ADR 0009).** desktop → status line → menu bar, then
  `MenuBar::draw_overlay` over the **whole** frame, so an open pull-down sits on
  top of the desktop just below the bar. The menu bar's own region is one row, so
  its pull-down can only be drawn as this final overlay.
- **Key routing — three local passes (TurboVision pre/focused/post):**
  1. *menu bar* (pre-process): claims `Alt`+a title letter and `F10`; while a menu
     is open it is **modal**, consuming every key so nothing leaks to the editor.
  2. *desktop* (focused, then a fallback of its own): the active window's
     interior gets first refusal; only once it declines does `Desktop`
     resolve the key against its own global accelerator table and post the
     bound command, if any (ADR 0028) — this is where `StatusLine`'s
     function-key/`Alt`-X hot-keys actually fire now, not in `status_line`
     itself.
  3. *status line* (post-process): purely a display pass at this point — a
     pure `View` with no `handle_event` override, so it always returns
     `Ignored` and this pass never actually claims anything; kept in the
     chain unchanged since nothing needs removing it.
  The first pass to consume wins (`EventResult::or_else` chaining, ADR 0004).
- **Other events.** `Mouse` → the region under the pointer in its local
  coordinates (behaviour mostly Phase 9; the seam exists now). A right-click
  drains `Context`'s anchor request into `context_menu` (ADR 0019), which
  then takes absolute priority over both key and mouse routing above until
  closed. `Command` (re-dispatched by `Root`) → `CM_HELP`, when `help` is
  `Some`, is caught here (below); every other command → the desktop's active
  window; `CM_QUIT` never reaches the shell, as `Root` claims it before
  re-dispatch. `Broadcast`/`Idle` → all three children. `Resize` → `relayout`
  + tell the desktop.
- **`CM_HELP` handling (ADR 0021), only when `help` is `Some`.** Resolves the
  topic by reading, not from anything carried on the command:
  `desktop.window(desktop.active_id()?)?.help_topic()` — `Some(id)` targets
  that topic, `None` (no active window, or its `help_topic` is unset) targets
  home. The help window is a singleton `Shell` itself enforces: if
  `help_window` still resolves (`desktop.window(id)`), that window is closed
  and a fresh one opened at the resolved topic reusing its old `bounds()`
  (position/size survive; internal list/pane focus and topic-list scroll
  don't — accepted, ADR 0021); otherwise a new one is just opened. Either way
  the new `WindowId` replaces `help_window`, and `Desktop::open`'s own
  raise-on-open already brings it to the front. When `help` is `None`,
  `CM_HELP` fails this check and falls through to the desktop exactly like
  any other command — no special-cased no-op branch.

## Collaborators

- [`widgets::{MenuBar, Desktop, StatusLine, ContextMenu, HelpWindow}`](widgets.md)
  (owned by value or built on demand), [`help::HelpContents`](help_window.md)
  (ADR 0021, opt-in via `with_help`), [`Canvas`] (draw), `geometry`
  (`Rect`/`Size`/`Point`), `event`/`command` (`CM_HELP`).
- Drops into [`Root`] (the loop bridge) as its root `View`; commands a child posts
  bubble to `Root`, which re-dispatches or claims `CM_QUIT` (ADR 0003). `Root`
  discards the result of every other re-dispatched command, which is exactly
  why `CM_HELP` must be caught here rather than by anything above `Shell`
  (ADR 0021) — mirrors why the ADR 0019 context-menu request is drained here
  too, not passed further up.

## Test plan (write these first)

- **Render (snapshot):** a full 40×10 screen — menu titles, blue backdrop, a
  framed active window, status line.
- **Interaction:** a plain key reaches the active window; `Alt`-letter opens a menu
  and then swallows typing (the window never sees it); a function key falls through
  to the status line; a resize relays the chrome out (status line on the new bottom
  row).
- **End-to-end:** through `Application`, `Alt`-X reaches the status line, which
  posts `CM_QUIT`, and the loop exits.
- **`CM_HELP` (ADR 0021):** with no `with_help` call, `CM_HELP` reaches the
  desktop like any other command and nothing opens; with help content set,
  it opens a `HelpWindow` on the active window's `help_topic` (falling back
  to home with no active window or an unset `help_topic`); a second
  `CM_HELP` while one is already open closes and reopens it at the newly
  resolved topic, at the same `bounds()`, active/foregrounded.
- **Manual:** `cargo run -p rvision --example chrome` — see [`widgets.md`](widgets.md).

## Open questions

- **Mouse behaviour** (click-to-open menus, click-outside-to-close) — the
  desktop's own drag/resize/click-to-front now exist
  ([`desktop.md`](desktop.md)); the menu bar's own click handling is still
  open.
- **`exec_view` reconciliation.** The modal loop now runs any `Window`
  ([`window.md`](window.md)), so the pull-down *could* become one; left as a
  later cleanup — the shell's layout/overlay and the `MenuBar` data are unchanged.
