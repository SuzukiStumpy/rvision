# Module spec: `rvision::view`

- **Status:** Done; extended (Draft) for the ADR 0015/0016/0017 protocols below
- **Phase:** 3 (View system); scroll/valid/resize protocols added post-extraction
- **Related ADRs:** 0003 (retained tree, parent-owns-children, commands up / broadcasts down), 0004 (three-phase dispatch, `EventResult`), 0008 (owner-relative coords + `Canvas`), 0015 (scroll chrome protocol), 0016 (unify `Window`/`Dialog`, `valid` veto protocol, `Modal` trait removed), 0017 (resize propagation protocol)

## Purpose

The retained-mode view tree. A [`View`] is the unit of the UI: it knows its
owner-relative `bounds`, draws itself through a [`Canvas`], and handles events. A
[`Group`] owns child views (`Vec<Box<dyn View>>`), draws them in z-order, and
implements the three-phase dispatch (positional → focused → broadcast) plus the
focus chain. [`Context`] is the handler's outbound channel: how a view posts a
command without holding a reference to anyone (ADR 0003).

It is **not** the widget library (buttons, windows — Phases 4/5) nor the app loop
(`app`). [`StaticText`] is the one concrete leaf here, plus a minimal focusable
test view to exercise dispatch.

## Public interface

```rust
pub trait View {
    fn bounds(&self) -> Rect;                 // in the owner's coordinates
    fn draw(&self, canvas: &mut Canvas);      // local (0,0) coords (ADR 0008)
    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let _ = (event, ctx);
        EventResult::Ignored                  // leaves that ignore everything
    }
    fn focusable(&self) -> bool { false }     // can this view hold focus?
    fn set_focused(&mut self, focused: bool) {}  // owner pushes focus (ADR 0010)
    fn drop_shadow(&self) -> Option<Style> { None }  // shadow the owner paints (ADR 0011)

    // What this view needs scrolled, or None; queried every draw (ADR 0015).
    fn scroll_metrics(&self) -> Option<ScrollMetrics> { None }
    // An owner's scroll chrome pushing a new position it computed (ADR 0015).
    fn set_scroll(&mut self, offset: Point) { let _ = offset; }

    // An owner telling this view its area changed — resize, zoom/restore
    // (ADR 0017). Default no-op; only a view whose layout is a cached
    // function of its size needs to override it.
    fn set_bounds(&mut self, bounds: Rect) { let _ = bounds; }

    // Whether it is OK to act on `command` right now — TurboVision's
    // `TView::valid` (ADR 0016). Default: always OK. A view that must refuse
    // (e.g. unsaved changes) can also post a follow-up command through `ctx`
    // in the same call, to ask its owner to run a confirmation flow, and try
    // again once that resolves. A view never runs its own modal loop
    // directly (ADR 0003) — only whoever owns a concrete `Application` can.
    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool { true }
}

pub struct Context<'a> { /* posted queue + &CommandSet */ }
impl<'a> Context<'a> {
    pub fn new(commands: &'a CommandSet) -> Self;
    pub fn post(&mut self, command: Command);      // enabled-gated (ADR 0003)
    pub fn broadcast(&mut self, command: Command);
    pub fn is_enabled(&self, command: Command) -> bool;
    pub fn take_posted(&mut self) -> Vec<Event>;   // the app loop re-dispatches
}

pub struct StaticText { /* bounds, text, style */ }
impl StaticText { pub fn new(bounds: Rect, text: &str, style: Style) -> Self; }

pub struct Group { /* bounds, Vec<Box<dyn View>>, focused: Option<usize> */ }
impl Group {
    pub fn new(bounds: Rect, children: Vec<Box<dyn View>>) -> Self; // focuses first focusable
    pub fn focused(&self) -> Option<usize>;
}
```

## Behaviour & invariants

- **Coordinates.** `bounds` is owner-relative; `draw` is handed a `Canvas`
  already offset+clipped to those bounds, so a view draws at local `(0, 0)` and
  cannot paint outside its box (ADR 0008).
- **Z-order.** `Group` draws children in vector order: index 0 is bottom, last is
  top; later children overwrite earlier ones where they overlap. Focus order is
  the same vector order among `focusable` children.
- **Drop shadows (ADR 0011).** A shadow falls *outside* a view's clipped canvas,
  so the view only *declares* it via `drop_shadow() -> Option<Style>` (default
  `None`); the owner paints `canvas.shadow(child.bounds(), style)` just before
  drawing each `Some` child. Painted per-child in z-order, so a view sits over its
  own shadow and a higher sibling's shadow falls on a lower one. The view supplies
  the style (resolved from its own `Role::Shadow`), keeping `Group` theme-free.
- **Three-phase dispatch (per group, by event class — ADR 0004):**
  - *Positional* (`Mouse`): delivered to the **topmost** child whose `bounds`
    contain the pointer, with the position translated into that child's local
    coordinates. If it ignores, the group ignores (no redelivery to occluded
    siblings).
  - *Focused* (`Key`, `Command`): delivered to the focused child first. If the
    child ignores a `Tab`/`BackTab`, the group advances/retreats its own focus
    (wrapping among focusable children) and consumes it. Any other ignored event
    returns `Ignored` so it bubbles up the owner chain — that unwinding *is* the
    bubble (ADR 0003).
  - *Broadcast* (`Broadcast`, `Resize`, `Idle`): delivered to **all** children;
    the group returns `Ignored` (broadcasts don't stop).
- **`valid` fans out to every child, not just the focused one** (ADR 0016,
  mirroring TurboVision's `TGroup::valid`). Every child is asked — the fold
  is *not* short-circuiting `all()` — so two refusing children both get the
  chance to post their own follow-up in the same pass, rather than the
  second being skipped once the first has already refused:
  ```rust
  fn valid(&mut self, c: Command, ctx: &mut Context) -> bool {
      self.children.iter_mut().fold(true, |ok, v| v.valid(c, ctx) && ok)
  }
  ```
  A plain leaf's default (`true`) makes this a no-op for a `Group` of
  ordinary controls; it matters once something inside can refuse (e.g. a
  `Window`'s interior refusing `CM_CLOSE`, see [`window.md`](window.md)) —
  and it composes through nesting the same way `set_focused` already does.
- **Initial focus.** `Group::new` focuses the first `focusable` child, or `None`
  if there are none. A group with no focusable children ignores `Tab`.
- **Posting.** `Context::post` queues an `Event::Command` only if the command is
  enabled (a disabled control's command never fires); `broadcast` is not gated.
  The app loop later drains `take_posted` and re-injects them from the root.
- **Edge cases:** empty group (no children) ignores everything; a `Mouse` outside
  every child is ignored; `draw` of an off-screen child writes nothing (the
  `Canvas` clip handles it).

## Collaborators

- [`Canvas`] (draw), [`Buffer`] (via Canvas), `geometry` (`Rect`/`Point`).
- `event` (`Event`/`EventResult`/`KeyCode`), `command` (`Command`/`CommandSet`).
- Siblings never hold references to one another: a child posts a command via
  `Context`; the group routes events down; results unwind up (ADR 0003).

## Test plan (write these first)

- **Logic:** positional hit-testing picks the topmost containing child and
  translates the point; a click outside all children is ignored; initial focus is
  the first focusable child.
- **Interaction:** `Tab`/`BackTab` cycle focus among focusable children (skipping
  `StaticText`), wrapping at the ends; the focused child receives keys; a command
  a child posts appears in `Context` and (when the child ignores it) bubbles up.
- **`valid` fan-out:** a `Group` of all-default children reports `valid` as
  `true`; one refusing child makes the whole `Group` refuse, regardless of
  which child is focused; every child is asked (not just the first refusal
  short-circuiting) so a refusing child can still post its own follow-up.
- **Render (snapshot):** a group of two `StaticText`s plus an overlapping child
  draws in z-order at the right offsets.
- **Manual:** later, via the Phase 4 chrome demo.

## Open questions

- **Focus-aware drawing** (a focused button looking different): *resolved in Phase
  5* (ADR 0010). `draw` keeps its `&self`/`Canvas` signature; focus is a stored
  flag pushed by the owner via the new defaulted `View::set_focused`, which `Group`
  calls as focus moves (and forwards to its focused child). A richer draw context
  (theme/cursor too) is still open if a future need appears.
- **`Modal` trait: removed**, not just resolved (ADR 0016). It briefly existed
  to give `exec_view` something to run generically (`size` + `ends_on`); once
  `Window` absorbed `Dialog` there was exactly one concrete type that ever
  implemented it, so the trait added indirection with nothing left to
  abstract over. `exec_view` now takes `&mut Window` directly — see
  [`window.md`](window.md), [`app.md`](app.md).
- **Scroll chrome, the `valid` veto, and resize propagation: resolved** as
  defaulted `View` methods, same shape as `drop_shadow`/`set_focused` — see
  ADR 0015, ADR 0016, ADR 0017, and [`window.md`](window.md)/
  [`desktop.md`](desktop.md) for where they're actually consumed (`Window`
  hosts scroll chrome and propagates `set_bounds` to its interior on resize/
  zoom; `Window`/`Group`/`Desktop` all implement `valid` fan-out or
  delegation).
- **Integer view IDs** (ADR 0003) for targeted messages: not needed for bubbling
  or Tab traversal; add when a command must address a specific view.
- **Cross-group Tab boundary**: focus currently wraps within a group; handing off
  to the parent at the boundary is a Phase 4/5 refinement.
- **Theme threading** into `draw`: `StaticText` takes a concrete `Style` for now;
  resolving a `Role` against a `Theme` at draw time arrives with the chrome.
