# Module spec: `rvision::widgets::context_menu`

- **Status:** Done
- **Phase:** Backlog — roadmap's "Right-click context menus" (landed)
- **Related ADRs:** 0009 (application shell + menu overlay — the draw-last-
  overlay/modal-first-refusal shape this reuses), 0018 (cascading menus — the
  submenu path-stack rules this reuses verbatim), 0019 (new: the anchor
  request path — `Context`'s offset accumulator, and why `ContextMenu` is a
  sibling type rather than a `MenuBar` generalization)

## Purpose

A pointer-anchored pull-down, triggered by a right-click rather than a menu
bar title, that any view can request with its own items. Reuses `Menu`/
`MenuItem` (unchanged) and the cascading-submenu rules (ADR 0018) wholesale;
adds the plumbing for a right-click landing on an arbitrary, arbitrarily
nested view to open one, correctly anchored in screen coordinates.

Not covered here: a keyboard trigger (Shift-F10 or similar) — deferred,
mouse-only for v1, since there is no existing notion of "what's focused wants
a context menu" independent of a pointer position. Not covered: any built-in
context menu content (e.g. a `Window` system menu of Close/Zoom/Next/Prev) —
the general mechanism this spec adds lets an application wire that up itself;
the framework does not pre-populate one.

## Public interface

```rust
// view.rs — Context grows a second outbound channel, parallel to `posted`
// but never re-dispatched as an Event (only Shell drains it), plus the
// offset-accumulator primitive any container needs to make it resolve
// correctly (ADR 0019).
pub struct ContextMenuRequest { pub menu: Menu, pub at: Point } // `at` already
    // resolved to true screen coordinates by the time it's stashed.
impl<'a> Context<'a> {
    pub fn open_context_menu(&mut self, menu: Menu, at: Point); // `at` in the
        // caller's own local coordinates, resolved here using the
        // accumulated offset.
    pub fn take_context_menu_request(&mut self) -> Option<ContextMenuRequest>;
    pub fn translated<T>(&mut self, dx: i16, dy: i16, f: impl FnOnce(&mut Self) -> T) -> T;
        // wraps one translate-and-recurse dispatch step; Group, Desktop,
        // Window, and Shell each call this around the child dispatch they
        // already do.
    pub fn commands(&self) -> &CommandSet; // lets a on-demand ContextMenu
        // snapshot the live enable state at construction time.
}

// widgets/context_menu.rs (new file) — crate-internal, not part of the
// public API; Shell is the only consumer.
struct ContextMenu { /* anchor, menu, path: Vec<usize>, styles, commands, closed */ }
impl ContextMenu {
    fn new(menu: Menu, at: Point, theme: &Theme, commands: &CommandSet) -> Self;
    fn is_closed(&self) -> bool; // Shell polls this after each dispatch and
        // drops its Option<ContextMenu> once true, rather than this type
        // signalling its own removal any other way.
    fn handle_event(&mut self, event: &Event, screen: Size, ctx: &mut Context) -> EventResult;
    fn draw_overlay(&self, canvas: &mut Canvas);
}

// menu.rs — the pure geometry/hit-test/draw pieces of MenuBar's cascade
// turned out to have no dependency on its bar-specific path[0] indexing,
// so they became free functions shared by both types, rather than being
// duplicated (a better factoring than planned — see ADR 0019):
pub(crate) fn pulldown_width(menu: &Menu) -> i16;
pub(crate) fn cascade_area(parent_area: Rect, parent_highlight: usize, menu: &Menu, screen_w: i16) -> Rect;
pub(crate) fn hit_test(pos: Point, menus: &[&Menu], areas: &[Rect]) -> Option<(usize, usize)>;
pub(crate) fn draw_cascade(canvas: &mut Canvas, menus: &[&Menu], areas: &[Rect], highlights: &[usize], commands: &CommandSet, styles: MenuStyles);
pub(crate) struct MenuStyles { pub bar: Style, pub selected: Style, pub disabled: Style, pub hotkey_fg: Color }

// app.rs — Shell grows two fields
pub struct Shell { /* ..., theme: Theme, context_menu: Option<ContextMenu> */ }
```

## Behaviour & invariants

- A right-click (`MouseKind::Down(MouseButton::Right)`) is an ordinary mouse
  event through the existing positional dispatch chain (`Shell` → `Desktop` →
  `Window` → its interior) — no new `View` trait method, no recursive query
  pass. A view that wants to offer a menu at that position calls
  `ctx.open_context_menu(menu, its_own_local_click_point)` and returns
  `EventResult::Consumed`, same as posting any other command. A view that
  doesn't recognise the position returns `Ignored` as usual.
- **Anchor resolution (ADR 0019).** `Context` accumulates the offset each
  container already computes when it translates `MouseEvent.pos` and recurses
  into a child (`Group::dispatch_positional`, `Desktop::handle_mouse`,
  `Window`'s interior dispatch, `Shell`'s region carve-up) — the same
  subtraction, now also pushed onto `Context` around that same call and
  popped after. `open_context_menu` sums the current accumulated offset onto
  `at` before stashing the request, so it resolves to true screen coordinates
  regardless of nesting depth, without any container needing to know a
  context menu exists.
- The request is a field on `Context` separate from `posted` — never an
  `Event` variant, never re-dispatched into the tree. Only `Shell` drains it
  (immediately after delivering a `Right`-`Down` mouse event down its own
  dispatch chain), so `Root`/`Group`/`Desktop` need no awareness of what it's
  for, only of the offset bookkeeping.
- If more than one request is somehow made during a single event's dispatch
  (should not happen — positional dispatch reaches exactly one leaf per
  event), the last one wins. Not a validated invariant, just documented
  fallback behaviour.
- Taking a request replaces whatever `Shell.context_menu` already held —
  right-clicking elsewhere while one is open swaps it for the new one.
- While `context_menu` is `Some`, `Shell` gives it first refusal on every
  key/mouse event, exactly mirroring `menu_bar.is_open()`'s modality; drawn as
  a full-frame overlay last (after the menu bar's own overlay, so the two
  stack correctly on the rare occasion both are open).
- Cascading submenus inside a `ContextMenu` follow ADR 0018's rules verbatim:
  `Right`/`Enter` on an enabled `Submenu` item opens the next level; `Left`/
  `Esc` pops one level, only closing entirely once at the root; a leaf
  `Command` item posts (gated by `Context`, ADR 0003) and closes every level;
  hover only ever moves the highlight, never opens a level on its own; item
  hot-key letters still choose/open immediately, same convenience as a
  pull-down. There is no sibling cycling (no bar titles to cycle between) and
  no `Alt`-hot-key open path (opened only by a right-click).
- **Anchor geometry.** Level 0's box opens with its top-left at the click
  point, clamped (slid back, not flipped to the opposite side) so it never
  runs off the right or bottom of the screen — the two-axis generalisation of
  `pulldown_area`'s single-axis clamp, since a context menu can open anywhere
  on screen, not just under a fixed top row. Deeper levels cascade exactly as
  ADR 0018, via the same shared `cascade_area` free function `MenuBar` itself
  now calls: right of the parent's box at the parent item's row, flipping to
  the parent's left edge if it would overflow.
- `Left`-`Down` on any point outside every currently open box, or `Esc` at
  the root level, dismisses the whole `ContextMenu` (clears
  `Shell.context_menu`) — mirroring `clicking_off_an_open_pulldown_dismisses_it`.
- `ContextMenu` is a new sibling type, **not** a generalization of `MenuBar`:
  no bar row, no top-level sibling cycling, no `Alt`-hot-key open trigger,
  and its level-0 anchor is an arbitrary screen point rather than a bar
  title's column — `path[0]` here is simply "the highlighted item in the one
  root `Menu`", not "which sibling bar menu is open" as `MenuBar::path[0]` is.
  That index shift is exactly the seam along which the two types split: the
  pure geometry/hit-test/draw pieces (`cascade_area`, `hit_test`,
  `draw_cascade`, `pulldown_width`, `MenuStyles`) never actually depended on
  `MenuBar`'s bar-index indirection, so they became shared free functions in
  `menu.rs` — a better factoring than the "duplicate the ~100 lines" call
  anticipated while drafting this spec. Only the small path-bookkeeping
  methods (`open_menus`, `hover`, `choose`, `already_expanded`,
  `handle_key`/`handle_mouse`) stay separate per type, since unifying *those*
  across the two differing `path` semantics would need something more
  convoluted than the few dozen lines each duplicates.

## Collaborators

`Menu`/`MenuItem` (reused as-is, no changes — a context menu is built from the
same types a pull-down is). `Context` (extended with the offset accumulator
and the new request field). `Group`, `Desktop`, `Window` (each gains a small
push/pop of that offset around the translate-and-recurse dispatch they
already do — no behavioural change otherwise). `Shell` (new `context_menu`
and `theme` fields — the latter kept solely so a `ContextMenu` can be built
on demand, since unlike the three chrome pieces it has no upfront
construction site to resolve styles from a theme at; overlay draw,
first-refusal routing — same shape ADR 0009 already gave `MenuBar`).
`MenuBar` itself, indirectly: it now calls the same shared `cascade_area`/
`hit_test`/`draw_cascade`/`pulldown_width`/`MenuStyles` this spec's
implementation factored out of it. `Theme`/`Role`: reuses `MenuBar`/
`MenuSelected`/`MenuDisabled`/`MenuHotkey` — no new roles.

## Test plan (write these first)

- **Logic:** `Context`'s offset accumulates correctly through nested
  push/pop scopes (a direct unit test against `Context`, faking a couple of
  nested "push offset, call, pop" scopes) and resolves to the plain point
  when no offset was ever pushed; a `Right`-`Down` on a view that never calls
  `open_context_menu` leaves `Shell.context_menu` `None`; a second
  right-click request while one is already open replaces it; `Esc`/`Left`
  pop one cascade level (reusing ADR 0018's own test shape) rather than
  closing outright, except at the root; choosing a `Command` item posts and
  closes entirely at any depth; a disabled item (via `with_gate`, same as
  `MenuItem` already supports) behaves like `MenuBar`'s disabled item —
  greyed, un-openable, closes without posting if chosen anyway.
- **Render (snapshot):** a single-level context menu opened mid-screen; one
  opened close enough to the right/bottom edge that it flips on one or both
  axes to stay on screen; a two-level cascade opened through a context menu,
  matching `menu.rs`'s existing cascade snapshot style.
- **Interaction (scripted events):** the key regression this feature exists
  to prove — a right-click on a view nested inside a `Window` inside a
  `Desktop` inside `Shell` opens its requested menu anchored at the *true
  screen position* of the click, not some partially-translated local point;
  clicking an item posts its command through the normal bubble-up path and
  closes the menu; a plain right-click on a view that offers nothing leaves
  ordinary dispatch (`Ignored`) undisturbed, so nothing opens.
- **Manual:** extend the `chrome` example with a right-click context menu
  somewhere in the window interior (e.g. a couple of harmless commands) to
  exercise real anchoring and dismissal by eye.

## Open questions

None blocking a coherent Draft. Two follow-ups noted for later, out of scope
for v1:

- A keyboard trigger (Shift-F10-style), deferred per this round's decision —
  there is no existing notion of "what's focused wants a context menu"
  independent of a pointer position yet.
- Whether the framework should eventually offer a built-in convenience
  constructor for a `Window` "system menu" (Close/Zoom/Next/Prev on its
  title bar) now that the general mechanism exists to build one trivially at
  the application level. Not required for v1; an app can already do this
  itself once this spec lands.
