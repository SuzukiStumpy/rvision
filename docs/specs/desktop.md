# Module spec: `rvision::widgets::Desktop`

- **Status:** Draft
- **Phase:** post-extraction rework (SDI/MDI convergence)
- **Related ADRs:** 0016 (unify `Window`/`Dialog`, dynamic desktop), 0007
  (mouse), 0009 (`Shell`), 0003/0004 (tree + dispatch), 0027 (generic mouse
  capture for continuous drag interactions), 0028 (global keyboard
  accelerator table), 0033 (shared `arrange` geometry: chrome hit-testing,
  drag sessions, cascade/tile), 0034 (`topmost`-pinned windows)

## Purpose

The backdrop plus a *dynamic* stack of [`Window`](window.md)s — TurboVision's
`TDesktop`, made real. Owns `Window`s concretely (not `Box<dyn View>`, so it
can mark the active one and reach every window for the `valid` fan-out below).
Turns `Desktop` from "draws whatever fixed `Vec<Window>` it was built with"
into an actually usable MDI container: open/close, click-to-front, drag,
resize, hide/show, keyboard window-cycling — all built once here instead of
once per consuming application (ADR 0016).

It is **not** the modal path — a window run via `Application::exec_view`
never touches `Desktop` at all (see [`window.md`](window.md), [`app.md`](app.md)).

## Public interface

```rust
/// An opaque handle to a window `Desktop` owns, from a monotonic counter
/// `Desktop` keeps internally. No locking: the event loop is single-threaded
/// by design (CLAUDE.md), so there is nothing to race (ADR 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

pub struct Desktop { bounds: Rect, backdrop: Cell, /* windows + active + drag/capture session + accelerators, see below */ }

impl Desktop {
    /// An empty desktop occupying `bounds`, filled with `backdrop`.
    pub fn new(bounds: Rect, backdrop: Cell) -> Self;

    /// Registers a global keyboard shortcut (ADR 0028): pressing
    /// `accelerator`'s key, whenever nothing more specific already claims
    /// it, posts its command. Works with no window open, and needs no
    /// `StatusLine` at all — `Shell::new` feeds one in per `StatusItem`
    /// automatically, but this is also the way to bind one with no
    /// status-bar slot.
    pub fn bind_accelerator(&mut self, accelerator: Accelerator);

    /// Adds `window` to the stack, raised to the top and made active.
    pub fn open(&mut self, window: Window) -> WindowId;

    /// Asks `id`'s `valid(CM_CLOSE, ctx)`; if it agrees, removes and returns
    /// the window (transferring active to the next visible one in stack
    /// order, or `None`). A refusal, or an unknown `id`, leaves the desktop
    /// unchanged and returns `None` — the window may still have posted a
    /// follow-up through `ctx` (e.g. "confirm discard").
    pub fn close(&mut self, id: WindowId, ctx: &mut Context) -> Option<Window>;

    pub fn hide(&mut self, id: WindowId);   // no-op on an unknown id
    pub fn show(&mut self, id: WindowId);   // raises to top + activates, like click-to-front

    /// Raises `id` to the top and makes it active (click-to-front,
    /// programmatically). No-op if `id` is hidden or unknown.
    pub fn focus(&mut self, id: WindowId);

    /// Moves active to the next (or, if `!forward`, previous) *visible*
    /// window in stack order, wrapping, and raises it to the top — the
    /// keyboard equivalent of click-to-front (`CM_NEXT`/`CM_PREV`, below).
    pub fn cycle_focus(&mut self, forward: bool);

    /// Repositions every visible, non-maximized, arrangeable window into a
    /// cascade (`arrange::cascade_slot`, ADR 0033), in current stack order —
    /// resized to its slot if `resizable`, moved to the slot's origin but
    /// kept its own size otherwise. Z-order and the active window are
    /// unchanged — only bounds move.
    pub fn cascade(&mut self);

    /// Repositions every visible, non-maximized, arrangeable window into an
    /// even grid filling the desktop (`arrange::tile`, ADR 0033). Same
    /// exclusions and resizable/non-resizable distinction as `cascade`.
    pub fn tile(&mut self);

    pub fn active_id(&self) -> Option<WindowId>;
    pub fn window(&self, id: WindowId) -> Option<&Window>;
    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window>;

    pub fn set_bounds(&mut self, bounds: Rect);   // unchanged: the shell relays out on resize
}
impl View for Desktop { /* see below; valid() fans out to every window */ }
```

New framework-reserved commands in `command.rs` (below `CM_USER`, alongside
`CM_QUIT`/`CM_OK`/…): `CM_CLOSE`, `CM_ZOOM`, `CM_NEXT`, `CM_PREV`. `Window`
posts `CM_CLOSE`/`CM_ZOOM` from its own glyph clicks (see
[`window.md`](window.md)); an application's menu/status-line/accelerator can
post any of the four directly — `Desktop` is what gives them effect.

## Behaviour & invariants

- **Z-order / visibility.** Windows are stored bottom-to-top as today; the
  active window is the topmost **visible, ordinary** one (see `topmost`
  below — ADR 0034 changes what "topmost" means for picking a fallback
  active window, not for draw order). Any operation that raises a window
  (`open`, `show`, `focus`, `cycle_focus`, click-to-front) physically moves
  it to the end of its own tier of the stack — the true end if it's
  `topmost`, otherwise just below the first `topmost` entry — so `self`'s
  vec order always doubles as the complete, literal z-order for drawing and
  hit-testing, regardless of hidden windows interleaved earlier in it.
  `draw`, positional hit-testing, and keyboard dispatch-if-active all skip a
  window with `!is_visible()` outright (ADR 0016); broadcast/resize still
  reach every window, hidden or not, so a hidden window's state stays
  current for when it's shown again.
- **Hiding the active window** transfers active to the next visible window in
  stack order (or `None`), the same bookkeeping `close` needs, without
  removing the window or invalidating its `WindowId`.
- **Command interception (ADR 0016).** `handle_event(Event::Command(_))` now
  looks at the command *before* forwarding to the active window (previously
  it forwarded blindly):
  - `CM_CLOSE` → `self.close(active_id, ctx)` if the active window is
    `closable`; a non-closable active window (or no active window) ignores
    it silently.
  - `CM_ZOOM` → `active_window.toggle_zoom(self.bounds)` if `zoomable`.
  - `CM_NEXT`/`CM_PREV` → `cycle_focus(true)`/`cycle_focus(false)`.
  - Anything else falls through to the active window as today, whose
    `Ignored` bubbles out (ADR 0003).
- **Click-to-front (ADR 0016).** Any `MouseKind::Down` on a **visible**
  window, anywhere on it, raises and activates it *before* the click is
  otherwise acted on — this never fires while a modal `exec_view` is running,
  since `Desktop` receives no events at all then (already true today; no new
  mechanism needed).
- **`topmost`-pinned windows (ADR 0034).** A `Window::topmost(true)` window
  (a docked toolbox) is kept above every non-`topmost` window regardless of
  raise/click-to-front order: `raise` sends a `topmost` window to the true
  end of the stack, and any *other* (non-`topmost`) window it raises only as
  far as just below the first `topmost` entry, never past it. This is purely
  about z-order, not about stealing focus: raising a window — `topmost` or
  not — still always makes *it* the active one (a direct click on the
  toolbox activates it exactly like any other window would); what changes is
  only the two places that otherwise *guess* which window should become
  active next:
  - **`activate_topmost_visible`** (the `close`/`hide` fallback) prefers the
    topmost visible *ordinary* window, falling back to a `topmost` one only
    when nothing ordinary is left visible — so closing/hiding some other
    window never silently hands focus to a pinned toolbox sitting above it.
  - **`cycle_focus`** (`CM_NEXT`/`CM_PREV`) excludes `topmost` windows from
    its candidates entirely, the same way `cascade`/`tile` exclude non-
    `arrangeable` ones — a pinned utility panel isn't one of "the windows"
    Tab-style cycling is meant to step through.
- **Drag/resize sessions**, owned by `Desktop` directly — mirroring how
  `MenuBar` owns its own open/closed state machine across a sequence of
  events, rather than each event being handled statelessly. The
  classification and session math are `arrange::chrome_hit`/`start_session`/
  `continue_session` (ADR 0033), shared with `Window`'s own close/zoom/help
  glyph hit-testing:
  - A `Down(Left)` at a screen-absolute position that `arrange::chrome_hit`
    classifies as the **title bar** (row 0, outside the close/zoom/help
    glyph spans) starts a *move* session if the window is `moveable`; one
    classified as the **bottom-right corner** starts a *resize* session if
    `resizable`. Either way the down-click itself is consumed by `Desktop`
    (never forwarded into the window) once a session starts.
  - A down-click classified as a close/zoom/help glyph, or landing inside the
    interior, is *not* a session start — after the click-to-front raise, it
    is forwarded into the window exactly as positional dispatch does today,
    letting `Window` itself handle the glyphs (see [`window.md`](window.md),
    also `arrange::chrome_hit`-backed) or route on into the interior.
  - While a session is active, `Desktop` consumes every `Drag(Left)` itself
    (`arrange::continue_session` maps pointer movement since the anchor to a
    new `bounds`, applied via `Window::set_bounds` — moved for a move
    session, resized and floored at `MIN_SIZE` for a resize one — with no
    ceiling: `Desktop` still never clamps a dragged/resized window to its
    own bounds, deliberately kept as-is by ADR 0033) and never forwards to
    any window; `Up(Left)` ends the session (consumed) and dispatch returns
    to normal.
  - No session is active for more than one window at a time; a session in
    progress makes `Desktop` ignore new `Down` events until it ends (matches
    single-pointer terminal input; no multi-touch to arbitrate).
- **Cascade/tile** (ADR 0033): `cascade`/`tile` reposition every currently
  visible, non-maximized, *arrangeable* window (`arrange::cascade_slot`/
  `arrange::tile`) in current stack order; a maximized window already fills
  the desktop and is left untouched rather than force-restored, a hidden
  window is skipped, and a non-`arrangeable` window (a docked toolbox) is
  skipped too — entirely untouched, not repositioned or resized, regardless
  of visibility. A skipped window's slot in the cascade/tile sequence isn't
  reserved either: the remaining arrangeable windows lay out as if it were
  never open at all. Among the windows that do participate, a non-`resizable`
  one is moved to its computed slot's *origin* but keeps its own current
  size rather than being resized to fill the slot — `resizable` means
  "nobody changes my size," not just "no interactive corner-drag." Neither
  operation touches z-order or the active window — only bounds move. Plain
  methods, not framework `Command`s — matching `open`/`hide`/`show`/`focus`'s
  existing precedent that an operation needing no target-window data beyond
  what's already on `self` doesn't need to travel as a bubbled `Command`.
- **Mouse capture (ADR 0027).** A view anywhere in a window's tree can ask
  (via `Context::request_mouse_capture`) to keep receiving every subsequent
  mouse event straight through to its own window — regardless of where the
  pointer moves — until the button is released. `Desktop` tracks this as
  `captured: Option<WindowId>`, orthogonal to the `Move`/`Resize` drag
  session above: capture is checked first in `handle_mouse` (bypassing
  ordinary positional dispatch entirely while active) and cleared on `Up`,
  once the event has been forwarded. `Desktop` never learns *why* something
  wanted capture (a scroll-bar thumb drag is the only caller today) — purely
  a generic "keep forwarding" primitive, the same shape as the context-menu
  anchor request (ADR 0019).
- **Global keyboard accelerators (ADR 0028).** `Desktop` owns an
  `Accelerators` table (`command.rs`), populated via `bind_accelerator` —
  directly, or automatically per `StatusItem` when `Shell::new` harvests
  `StatusLine::accelerators()`. `handle_event`'s `Event::Key` arm tries the
  active window's own dispatch first, unchanged; only on `Ignored` does it
  resolve the key against the table and `ctx.post` the bound command
  (already gated by `CommandSet`, so a disabled command's key still
  consumes but posts nothing). Works with no active window. `Event::Paste`
  carries no `KeyEvent` and only ever reaches the active window, never the
  table.
- **`valid` fans out to every window, not just the active one** (ADR 0016,
  mirroring TV's `TGroup::valid`). Like `Group`'s own fan-out (`view.md`),
  every window is asked — not a short-circuiting `all()` — so several
  unsaved windows can each post their own confirmation follow-up in the same
  pass rather than only the first-refused one getting the chance:
  ```rust
  fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
      self.windows_mut().fold(true, |ok, w| w.valid(command, ctx) && ok)
  }
  ```
  This is what lets `Root` ask "is it OK to `CM_QUIT`?" of a whole desktop of
  windows through one opaque call on its single root `View` (see
  [`app.md`](app.md)) without knowing `Desktop` exists — the composition is
  generic, the same shape `Group` would need if it ever owned vetoable
  children directly.
- **`focusable()`** unchanged: `true` iff there is an active window.

## Collaborators

- [`Window`](window.md) (owned by value; `set_bounds`/`hide`/`show`/`toggle_zoom`/
  `valid` are what `Desktop` drives).
- `arrange` (ADR 0033): `chrome_hit`/`ChromeFlags`/`ChromeHit` classify a
  press; `start_session`/`continue_session`/`ArrangeSession`/`ArrangeKind`
  drive the drag/resize session; `cascade_slot`/`tile` back `cascade`/`tile`.
  `Desktop` supplies its own `MIN_SIZE` at the call sites that need a floor.
- `view::{View, Context}`, `command::{Command, CM_CLOSE, CM_ZOOM, CM_NEXT, CM_PREV,
  Accelerator, Accelerators}`, `event::{Event, MouseEvent, MouseKind}`.
- `app::Root` (asks `valid(CM_QUIT, ctx)` before honouring a posted quit —
  see [`app.md`](app.md)); `app::Shell` (owns a `Desktop`, needs a
  `desktop_mut()` accessor so application code can call `open` — see
  [`shell.md`](shell.md)).

## Test plan (write these first)

- **Logic:** `open` returns distinct, increasing `WindowId`s and activates;
  `close` on a `valid`-refusing window is a no-op and returns `None`; `close`
  on the active window transfers active to the next visible window in stack
  order; `hide`/`show` toggle visibility and reassign active correctly; an
  unknown `WindowId` is a safe no-op everywhere; `cascade`/`tile` position
  visible windows per `arrange::cascade_slot`/`arrange::tile`, skip hidden,
  maximized, and non-`arrangeable` windows (without reserving those a slot),
  keep a non-`resizable` window's own size while still moving it to its
  slot's origin, and leave z-order/the active window untouched.
- **Render (snapshot):** a hidden window is not drawn (no shadow, no chrome)
  even though it is still resident; z-order after a `focus`/`cycle_focus`
  raise matches the new stack order.
- **Interaction (scripted events):** click-to-front raises + activates a
  background window on any click on it, including one that also lands on its
  interior (the click still reaches the interior after raising); a title-bar
  drag moves a `moveable` window and is a no-op on a non-moveable one; a
  corner drag resizes a `resizable` window; `CM_CLOSE`/`CM_ZOOM`/`CM_NEXT`/
  `CM_PREV` each act on the active window and respect its `closable`/
  `zoomable` flags; `CM_QUIT` reaching `Desktop::valid` polls every window,
  not just the active one, and a single refusal anywhere vetoes it; a thumb
  drag keeps scrolling even once the pointer strays outside the window's own
  bounds, ending only on `Up` (ADR 0027); an unclaimed key resolves a bound
  accelerator and posts its command, the active window's own handling always
  wins over one bound to the same key, an accelerator fires with no active
  window, and an unbound key with no active claim still bubbles `Ignored`
  (ADR 0028); a `topmost` window stays hit-testable above an ordinary one
  even after that ordinary window is raised, and raising one `topmost`
  window still moves it above another `topmost` one (ADR 0034); closing/
  hiding the active window falls back to the topmost visible *ordinary*
  window over a `topmost` one, falling back further to a `topmost` window
  only when nothing ordinary is left visible; `CM_NEXT`/`CM_PREV` skip
  `topmost` windows entirely when cycling.
- **End-to-end (through `Application`):** opening a window via `desktop_mut().open(...)`
  from application code between `run` turns shows it on the next draw;
  `exec_view` run over a `Shell` with open desktop windows leaves them
  resident and unresponsive until the modal returns (background draws, no
  events reach them).
- **Manual:** a `desktop`/`mdi` example — open several windows, drag, resize,
  close, cycle with `CM_NEXT`, hide/show a `topmost`, non-`arrangeable`
  toolbox-style window from a menu command (staying visually on top of, and
  never swept into a cascade/tile with, the document windows), cascade and
  tile from a menu command.

## Open questions

- **Tiling/cascade layout commands.** Landed (ADR 0033): `cascade`/`tile`,
  backed by `arrange::cascade_slot`/`arrange::tile`, exercised manually via
  `examples/mdi.rs`'s Window ▸ Cascade/Tile commands.
- **Resize granularity** (corner-only vs. full-edge) — see
  [`window.md`](window.md)'s matching open question; both are decided
  together since `Desktop` is what detects the grab point.
- **Minimum window size during resize.** Needs a floor (at least enough for
  the frame's corners plus the close/zoom glyphs) so a drag can't collapse a
  window to something `interior_bounds` treats as empty forever; the exact
  floor is an implementation detail, not a design question.
- **`HelpWindow`.** Landed: a `Window` wrapping a composed `ListBox` +
  `HelpPane` interior, opened non-modally via `desktop.open(...)` exactly as
  anticipated here — see [`help_window.md`](help_window.md). It has since
  become more than just "the first real consumer": `Shell`'s ADR 0021
  context-help handling drives the same `open`/`close`/`window`/`active_id`
  surface to enforce a singleton help window (close the old one, reopen at a
  newly-resolved topic, reusing its `bounds()`) — nothing further was needed
  from `Desktop` itself for that (see [`shell.md`](shell.md)).
- **Double-click the title bar to zoom/restore.** Windows convention, not yet
  supported: today a title-bar click (outside the close/zoom glyph spans)
  only ever starts a move session (`start_session_if_applicable`), regardless
  of `MouseKind`. Recognising `DoubleClick(Left)` there and calling
  `Window::toggle_zoom` instead — the same glyph-span exclusion already
  computed, one more arm — would sit alongside the existing zoom glyph/
  `CM_ZOOM` as a second way to reach it. Logged, not scheduled.
