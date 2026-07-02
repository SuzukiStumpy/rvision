# ADR 0019 — Right-click context menus: a `Context` anchor request, Shell-owned overlay

- **Status:** Accepted
- **Date:** 2026-07-02

## Context

The roadmap's backlog names right-click context menus as the natural next
step after cascading menus (ADR 0018): "a pull-down anchored at the pointer,
populated per context... reuses the `Menu` overlay and its open/closed/modal
state machine; the trigger becomes a right-press hit-test rather than the
menu bar." Unlike a `MenuBar` pull-down, though, a context menu's *content*
depends on what's under the pointer — which may be an arbitrarily nested
view (a control inside a `Window` inside a `Desktop`), not something `Shell`
can enumerate up front the way it enumerates its own menu titles.

Four questions had to be settled before a spec could gel:

1. How general is v1 — can any nested view offer its own context menu
   content, or is this scoped to fixed built-ins (e.g. a `Window` system
   menu)?
2. Where does the open menu's state live, and who drives it modally?
3. Do nested submenus inside it pop one level at a time (ADR 0018's rule) or
   always close outright?
4. Is a keyboard trigger in scope for v1?

A fifth question surfaced only once (1) was answered: if any nested view can
request a menu, the request's anchor point starts out in *that view's own
local coordinate space* — several translations deep from the screen. Nothing
posted through `Context` has ever needed a position before (a `Command` is
just an id), so `Context` had no mechanism to carry one correctly across
nesting depth.

## Decision

**Scope (1): general, via a new `Context` method, not a new `View` method.**
`Context::open_context_menu(menu, at)` lets any view offer a menu from
inside whatever positional handling it already does for a right-click —
reusing the existing post/bubble-up idiom (ADR 0003) rather than adding a
recursive query method (`View::context_menu(&self, at) -> Option<Menu>`)
that every container (`Group`, `Desktop`, `Window`) would need to forward
through to arbitrary depth, duplicating exactly the coordinate-translation
work positional dispatch already does. A right-click is simply an ordinary
`MouseKind::Down(MouseButton::Right)` event through the existing positional
chain; the view under the point decides whether to call the new method,
same as it would `ctx.post(...)`.

**Ownership (2): a new field on `Shell`, not `Desktop`.** `context_menu:
Option<ContextMenu>` mirrors `MenuBar`'s shape exactly: first refusal on
every key/mouse event while `Some` (same as `menu_bar.is_open()`), drawn as a
full-frame overlay last (after the menu bar's own, so the rare case of both
open at once stacks correctly). This keeps the "modal overlay drawn last"
pattern in the one place ADR 0009 already established it, rather than
splitting it between `Shell` and `Desktop`.

**Dismiss rule (3): reuse ADR 0018 verbatim.** `Left`/`Esc` pop one cascade
level, closing outright only once at the root. No new rule to learn; a
context menu's submenus behave exactly like a pull-down's.

**Keyboard trigger (4): out of scope for v1.** There is no existing notion
of "what's focused wants a context menu" independent of a pointer position —
inventing one is a bigger question than this feature needs to answer today.
Noted as a follow-up in the spec's open questions, not decided here.

**The anchor problem: `Context` gains a small accumulated-offset mechanism.**
Every container that already translates a positional event and recurses
into a child — `Group::dispatch_positional`, `Desktop::handle_mouse`,
`Window`'s interior dispatch, `Shell`'s region carve-up — now pushes that
same offset onto `Context` around the recursive call, and pops it after.
`open_context_menu(menu, at)` sums the currently accumulated offset onto
`at` before stashing the request, so by the time `Shell` drains it the point
is already in true screen coordinates, regardless of how deep the requesting
view sat. This is a small, mechanical addition at four call sites that
already do the matching translation for `MouseEvent.pos` — not a new
concept, just the existing translation also applied to this one new request.

The request itself lives in a field on `Context` separate from `posted`,
and is never expressed as an `Event` variant: `Root`'s drain-and-redispatch
loop (ADR 0003) exists to re-inject posted commands/broadcasts back into the
tree, which is meaningless for a context-menu-open request — nothing in the
tree expects to *handle* one as an incoming event. Only `Shell`, which owns
the `ContextMenu` state the request is for, ever drains this field, right
after delivering the triggering mouse event down its own dispatch chain.

**`ContextMenu` is a new sibling type, not a `MenuBar` generalization —**
**but most of the cascade geometry turned out to be shareable anyway.** It
has no bar row, no top-level sibling cycling, no `Alt`-hot-key open path, and
its level-0 anchor is an arbitrary screen point rather than a bar title's
column — `path[0]` here means "the highlighted item in the one root `Menu`",
not `MenuBar::path[0]`'s "which sibling bar menu is open." That index shift
only touches the *root* level, though: everything past it (a cascaded level's
box geometry, deepest-first hit-testing, and the per-level draw loop) never
actually depended on which indexing scheme got it there. Pulled out of
`MenuBar` as free functions in `menu.rs` (`cascade_area`, `hit_test`,
`draw_cascade`, `pulldown_width`, and a small `MenuStyles` bundle),
`MenuBar` itself now calls them too — a better factoring than anticipated
while drafting the spec, which expected to duplicate that geometry. Only the
small path-bookkeeping methods that differ by the index shift itself
(`open_menus`, `hover`, `choose`, `already_expanded`, and the top-level
`handle_key`/`handle_mouse`) stay separate per type, since unifying those
across the two `path` semantics would cost more than the few dozen lines
each duplicates.

## Consequences

- A context menu's content is entirely application-defined; the framework
  supplies only the mechanism (anchor resolution, cascade, overlay,
  dismissal), not any built-in menu content.
- `Group`, `Desktop`, `Window`, and `Shell` each gain a few lines of offset
  bookkeeping around dispatch they already do; no behavioural change to
  anything that doesn't touch context menus.
- `Context`'s public surface grows by one method and, internally, the offset
  accumulator; existing callers (`ctx.post`, `ctx.broadcast`) are untouched.
- `menu.rs` gains a sibling type rather than a generalization of `MenuBar`,
  but also gains five shared free functions (`cascade_area`, `hit_test`,
  `draw_cascade`, `pulldown_width`, `MenuStyles`) that `MenuBar` itself now
  calls too — the one place `MenuBar` *did* change, as a pure refactor with
  no behavioural difference (its existing test suite, snapshots included,
  passes unchanged).
- `Shell` also gains a stored `Theme` clone, since unlike its three chrome
  pieces (each constructed once upfront with a theme in hand) a
  `ContextMenu` is built later, on demand, with nowhere else to resolve its
  styles from.
- Keyboard-triggered context menus remain an open, undecided follow-up.

## Alternatives considered

- **A recursive `View::context_menu(&self, at) -> Option<Menu>` query**,
  mirroring `drop_shadow`/`scroll_metrics` (ADR 0011/0015). Rejected: unlike
  those two protocols, which are queried on an *immediate* child by its
  direct owner during draw, this would need to reach arbitrary depth,
  requiring every container to add forwarding logic that re-derives the same
  coordinate translation positional dispatch already performs — strictly
  more code than reusing the existing dispatch/bubble-up path.
- **Scope v1 to fixed built-ins** (a `Window` system menu of Close/Zoom/
  Next/Prev, a `Desktop` backdrop menu), deferring the general mechanism.
  Rejected against the roadmap's own "populated per context" framing, and
  because the general mechanism, once the anchor problem is solved, is not
  meaningfully more code than a special-cased built-in would be.
- **`Desktop` owns the `ContextMenu` field instead of `Shell`.** Rejected:
  most of the value of a general mechanism is that a context menu can be
  requested from *outside* the desktop's own subtree too (in principle, the
  menu bar's own row, or the status line); anchoring the overlay/modality at
  `Shell` keeps one place responsible for "the thing drawn last, modally,
  over everything," matching `MenuBar`'s existing precedent exactly.
- **Route the request through `Event`/`posted`, re-dispatched like a
  command.** Rejected: `Root`'s redispatch loop exists to deliver posted
  events *back into the view tree*; a context-menu-open request has no
  handler in the tree waiting for it — it is consumed exactly once, by
  `Shell`, which is not what `Event`/`posted` is for.
- **Duplicate `MenuBar`'s cascade geometry wholesale in `ContextMenu`,**
  rather than trying to share any of it, on the assumption that the two
  types' differing `path` indexing would make a shared abstraction awkward.
  Dropped once implementation showed the indexing difference is confined to
  the root level: the box/hit-test/draw logic for every level past it is a
  pure function of `(menus, areas, highlights)` regardless of how `path`
  produced them, so it factors out cleanly as free functions with no
  awkwardness — the CLAUDE.md "prefer duplication" default doesn't apply
  once sharing turns out to cost nothing.
