# ADR 0016 — Unify `Window` and `Dialog`; a capable, dynamic desktop

- **Status:** Accepted
- **Date:** 2026-07-02

## Context

`rvision` currently has two unrelated ways to show a bordered box of content.
[`Window`] (`src/widgets/window.rs`) lives concretely in `Desktop`'s
`Vec<Window>`, drawn and dispatched every frame alongside its siblings, but
with no way to open or close one dynamically, no drag, no resize, and no
z-order reshuffling on click — `Desktop`'s mouse handling today is purely
positional: find the topmost window under the pointer, forward the translated
event, done. [`Dialog`] (`src/widgets/dialog.rs`) never joins the tree at all;
it is driven entirely by [`Application::exec_view`]'s own nested, exclusive
event pump, and carries policy `Window` has no concept of — which posted
commands end it (`ending`/`ends_on`), a default button (`default_cmd`), `Esc`
always cancelling. The two duplicate almost everything about drawing a
bordered, titled box with an interior and a shadow.

This split has a real cost: `edit` doesn't use `Desktop`/`Window` at all. It
owns its documents and its own bespoke MDI, drag, resize, and modal driver loop
entirely itself (the editor's ADR 0018), specifically because `Window` wraps
`Box<dyn View>`, and reaching a concrete `Document` behind that box would force
a downcast or `Rc<RefCell>` — both already rejected by ADR 0003. Each step was
locally reasonable, but the result is two windowing implementations, and this
crate's has atrophied to the point of missing basic MDI mechanics. The help
system was the first feature to want a real, non-modal window container and
hit the gap directly; ADR 0013 deferred a `HelpWindow` as a result.

This ADR does **not** attempt to solve that concrete-access problem, and does
not require `edit` to adopt anything here. `edit` keeps its own solution for
now, pinned to `rvision v0.1.0` (tagged immediately before this work) until it
migrates at its own pace, as its own later decision made in the `edit`
codebase. What this ADR *is* about: making `rvision`'s own `Desktop`/`Window`
a genuinely capable desktop metaphor — open, close, move, resize, focus,
z-order, a coherent single-window concept — so that *any* consumer, present or
future, can actually use it instead of reaching for a bespoke replacement the
way `edit` had to.

Turbo Vision itself already draws exactly the line this decision needs:
`TDialog` **is** a `TWindow` subclass (same chrome, different default flags —
no `wfGrow`/`wfZoom`, `ofCentered`), and `TGroup::execView` is a generic nested
pump that can run *any* `TView` modally, entirely separate from `insertView`,
which is how a view joins a group for ordinary dispatch. Even TV never merged
those two execution paths — they are genuinely different concerns: one is
long-lived and coexists with siblings, the other is short-lived and exclusive.

## Decision

### One `Window` type, not two

`Dialog` is absorbed into `Window` as a configuration, not a separate struct.
`Window` gains optional policy alongside its existing
`bounds`/`frame`/`interior`/shadow fields:

```rust
pub struct Window {
    // existing: bounds, frame, active, interior_fill, shadow_style,
    // casts_shadow, interior: Box<dyn View>
    resizable: bool,
    moveable: bool,
    closable: bool,
    zoomable: bool,
    placement: Placement,          // Positioned | Centered
    ending: Vec<Command>,          // which posted commands end a run
    default_cmd: Option<Command>,  // Enter's fallback target
    esc_cancels: bool,
    visible: bool,                 // resident but not drawn/dispatched when false
}

pub enum Placement { Positioned, Centered }
```

A "dialog" is just `Window::new(...)` configured as fixed-size, centred, not
resizable/zoomable (TV dialogs can still be dragged by their title even though
they can't resize or zoom), `.ending([CM_OK, CM_CANCEL])`,
`.with_default(cmd)`, `.esc_cancels(true)`. `MessageBox`/`FileDialog` build on
the same `Window` constructor `Dialog` used to provide.

`Group`'s existing Tab-cycling focus behaviour is untouched and unaffected — it
was never `Dialog`-specific; any `Window` whose interior is a `Group` already
gets it for free.

### Two ways to run one, not merged

- **`Desktop::open(window: Window) -> WindowId`** — joins the desktop's stack,
  ordinary per-frame draw and dispatch alongside siblings, raised to active on
  open.
- **`Application::exec_view(&mut self, background: &mut dyn Program, window: &mut Window) -> io::Result<Command>`**
  — unchanged in spirit from today: a nested, exclusive pump; `background`
  draws but receives no events while it runs. `exec_view` now takes
  `&mut Window` directly. **The `Modal` trait is deleted** — `Window` already
  carries everything it required (`size` via `bounds`, `ends_on`, plus the new
  `valid` below) and more; there is exactly one concrete type that plays that
  role now, so the trait added a layer of indirection with nothing left to
  abstract over.

This mirrors TV's `insertView`/`execView` split deliberately, not as a
compromise — merging the two execution models would be a separate, far riskier
project with no clear payoff even in the software this crate is modelled on.

### Opening is an API call; everything else on an existing window is a command

A `Command` is a bare `Command(u16)` tag — it cannot carry a `Box<dyn View>`.
So "open a window with this content" can never be a bubbled command; it is
always `desktop.open(window)`, called by whoever holds a concrete
`&mut Desktop` and constructed the interior — the same escape hatch `edit`'s
bespoke loop already relies on, and squarely an application concern (ADR 0003).

Everything that acts on a window *already open* needs no new data, so it stays
inside `Desktop`'s/`Window`'s own machinery and never needs to leave the tree:

- **Close, cycle-focus, maximise/restore** are commands `Desktop` intercepts
  itself in its `handle_event(Event::Command)` (which today just forwards
  blindly to the active window — it grows a first look at the command before
  forwarding).
- **Click-to-front:** any mouse-down on a window, anywhere on it, raises it to
  the top of the stack and makes it active before whatever the click was
  actually for proceeds — unless a modal `exec_view` is running, which is
  already enforced for free: `Desktop` receives no events at all while that
  nested pump owns input.
- **Drag/resize** are owned by `Desktop` directly, mirroring how `MenuBar`
  already owns its own open/closed state machine across a sequence of events:
  a mouse-down on a title bar (if `moveable`) or a border/corner (if
  `resizable`) starts a session `Desktop` tracks (which window, anchor point,
  move-or-resize) across the following `Mouse::Move`/`Mouse::Up` events,
  mutating that window's bounds each step. `Window` gains a `set_bounds`,
  mirroring the existing `Desktop::set_bounds`.

### Window identity

`Desktop::open` returns a `WindowId(u64)` — an opaque id from a monotonically
increasing counter `Desktop` owns internally, used for `close(id)`/`focus(id)`
and any future window-list API. No lock is needed: `rvision`'s event loop is
single-threaded by design (`CLAUDE.md`'s non-negotiable — no async, no tokio),
so there is nothing to race. A client application going multithreaded in its
own critical sections doesn't change that; only `Application`/`Desktop`
themselves would need to, and that is a much larger rearchitecture this
decision doesn't need to pre-pay for.

### Reuse without rebuilding: hiding, and holding a window by value

Two different cases want "build once, show repeatedly," and they get two
different answers rather than one flag doing double duty.

**A window run modally (`exec_view`) already supports this for free.**
`exec_view` borrows `&mut Window`; it never takes ownership. An application
that wants a reusable "Find" or "Settings" dialog simply holds one `Window`
value itself (e.g. a field on its own app struct) and calls `exec_view` on it
every time the user asks — no reconstruction between shows. Whatever state the
interior wants to carry between appearances (the last search term, the last
selected radio option) just sits in that same `Window`'s interior, untouched
between calls, exactly like any other retained Rust value; nothing new is
needed for this half.

**A window resident on the desktop needs an actual flag**, because `Desktop`
owns these once opened (`Desktop::open`), and today has no notion of "still
resident but not shown." `Window` gains `visible: bool` (default `true`) with
`hide()`/`show()` — TurboVision's own method names for exactly this
(`TView::hide`/`show`, toggling its `sfVisible` state flag), not new
terminology. `Desktop` skips a hidden window in `draw`, in mouse hit-testing,
and in keyboard dispatch if it happens to be the active one. Hiding the active
window transfers active status to the next visible window in stack order (or
`None` if none are visible) — the same bookkeeping `close` already needs, just
without removing the window or invalidating its `WindowId`. Showing a hidden
window raises it to the top and makes it active, identically to click-to-front.

This is the mechanism a reusable *desktop* dialog (a toolbox, an inspector
panel someone can toggle from a menu) uses; a modal one uses the retained
`&mut Window` pattern above instead, since it never lives on the desktop in
the first place.

### Closing — and any state change — is vetoable

`View` gains one more defaulted method, in the same shape as
`drop_shadow`/`set_focused`/`scroll_metrics` (ADR 0011/0010/0015):

```rust
/// Whether it is currently OK to act on `command` (TurboVision's
/// `TView::valid`) — e.g. close, quit, zoom. Default: always OK. A view
/// that needs to refuse (unsaved changes) can also post a follow-up
/// command through `ctx` in the same call — e.g. to ask its owner to run
/// a confirmation flow — and try again once that resolves. A view never
/// gets to run its own modal loop directly (ADR 0003): only whoever owns
/// a concrete `Application` can do that.
fn valid(&mut self, command: Command, ctx: &mut Context) -> bool { true }
```

This is TV's own hook, not an invention — `TView::valid` exists for exactly
this. `ending`/`ends_on` decides *which* posted commands are close-candidates
for a window; `valid` is the final gate asked with that command immediately
before it takes effect, by whichever mechanism is about to act on it
(`Desktop::close` for a tree-resident window, `exec_view` before it returns for
a modal one).

This generalises past single-window close: before `Root` honours `CM_QUIT`, it
asks `valid(CM_QUIT, ctx)` of **every** open window, not just the active one —
mirroring TV's `TGroup::valid` propagating the check to all its subviews.
Quitting with several unsaved documents open is exactly the "state change the
client layer needs to verify" case, multiplied.

## Consequences

- One chrome/frame/interior/shadow/focus implementation instead of two;
  `Dialog`, `MessageBox`, `FileDialog` all become configurations of `Window`
  rather than a parallel struct duplicating its drawing and event-routing
  code.
- `rvision`'s `Desktop` becomes an actually usable MDI container — dynamic
  open/close, drag, resize, click-to-front, keyboard window-cycling, all built
  once in the framework instead of once per application. `HelpWindow` (ADR
  0013's deferred item) is unblocked: it becomes a `Window` wrapping a
  composed `ListBox` + `HelpPane` interior, opened non-modally via
  `Desktop::open`.
- Expensive-to-build or stateful dialogs (a Find/Replace box, a Settings
  dialog) pay their construction cost once rather than on every show — either
  retained by value and re-`exec_view`'d, or opened once on the desktop and
  toggled with `hide`/`show`.
- `edit` is unaffected until it chooses to migrate — it is pinned to
  `rvision v0.1.0` and stays there until that pin is bumped deliberately.
  Migrating `edit` onto this is a separate, later, `edit`-side decision, not a
  consequence forced by this ADR.
- Real, non-trivial engineering: every current `Dialog`/`MessageBox`/
  `FileDialog` call site, `exec_view`'s signature, and `Window`'s own event
  handling all move in the same change. This is deliberately the ambitious
  cut, not a shallow one — see the TV precedent above for why the boundary is
  drawn where it is rather than merging the execution models too.
- The `valid`/`Context`-posting shape means a refused close is a two-step
  dance (deny + request a follow-up UI + retry once approved) rather than TV's
  synchronous in-place dialog. Slightly more ceremony; the alternative breaks
  ADR 0003.

## Alternatives considered

- **Solve `edit`'s concrete-access problem in the same pass (full
  convergence).** Rejected for scope: conflates two hard problems (windowing
  mechanics vs. how an owner reaches a concrete `Document`) into one decision,
  risking a worse outcome on both. `edit` explicitly keeps its own solution
  (see Context).
- **Merge `Desktop::open` and `exec_view` into one execution model.** Rejected
  — not even TV does this; the two are genuinely different concerns, and
  merging them is a much larger, separate, riskier project than unifying the
  type.
- **Keep `Modal` as a trait for future chrome-less modals.** Rejected as
  speculative — no current need, and cheap to reintroduce later if one
  appears (`CLAUDE.md`: don't design for hypothetical requirements).
- **UUIDv7 for `WindowId`, hedging a multi-threaded future.** Rejected —
  conflicts with the single-threaded non-negotiable, and would need a new
  crate dependency (crate budget, ADR 0001) for a scenario the architecture
  has already ruled out.
- **Let `Window`/`Desktop` downcast into a child's concrete type for
  close-confirmation etc.** Rejected — exactly the move ADR 0003 already ruled
  out, and the reason `edit` forked its own windowing in the first place.
- **Always rebuild a dialog/window from scratch on every show, no `visible`
  flag at all.** The simplest possible rule, and adequate for a one-shot
  message box, but wasteful for anything stateful or non-trivial to construct,
  and TV already solved this with `hide`/`show` rather than accepting the
  rebuild cost.
