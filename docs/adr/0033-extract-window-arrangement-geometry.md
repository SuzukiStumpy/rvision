# ADR 0033 — Extract shared window-arrangement geometry into `rvision::arrange`

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

`rvision::widgets::Desktop` and `edit::app::EditorApp` (`edit`'s own bespoke
MDI, ADR 0016's Context) each implement the same window-arrangement algorithm
twice: classify a press as a title-bar move, a corner resize, or a
close/zoom/help glyph; track an in-progress move/resize session as an anchor
point plus the window's starting bounds; apply pointer movement to produce new
bounds each step; raise-on-click z-order. None of it is specific to what's
*inside* a window — it's pure `Rect`/`Point`/`Size` arithmetic, implemented
once per `Box<dyn View>` window in `rvision`, once per concrete `Document` in
`edit`, because `edit` can't use `Desktop`/`Window` directly: `Window` wraps
`Box<dyn View>`, and reaching a concrete `Document` behind one would force a
downcast or `Rc<RefCell>`, both already rejected by ADR 0003. That constraint
is real and unavoidable and this ADR does not attempt to remove it (same
stance ADR 0016 already took).

Filed as [rvision#8](https://github.com/SuzukiStumpy/rvision/issues/8) once
building a second resident window kind in `edit` (a non-modal help overlay,
`edit`'s ADR 0027) made the duplication's shape concrete and measurable.
Reviewing the actual code in both repos (not just the issue's own sketch)
before implementing surfaced four points worth deciding explicitly, logged as
a comment on #8 and a follow-up backlog issue,
[rvision#9](https://github.com/SuzukiStumpy/rvision/issues/9):

1. `Desktop::start_session_if_applicable` only ever needs a bool ("is this a
   glyph, so don't start a move") — the code that actually posts
   `CM_CLOSE`/`CM_ZOOM`/`CM_HELP` by re-deriving the same glyph spans is
   `Window::handle_event`, a *third* copy the issue's original scope didn't
   name. Refactoring only `Desktop` would leave a unified `ChromeHit`'s
   `Close`/`Zoom`/`Help` variants unexercised by any real caller.
2. `cascade_slot`/`tile` have no existing `Desktop` counterpart to extract —
   `Desktop` has no cascade/tile at all today (`docs/specs/desktop.md` had
   explicitly deferred this). They're new capabilities motivated by `edit`
   already having them, not a deduplication.
3. `Desktop`'s drag/resize today never clamps a window to the desktop's own
   bounds (no ceiling on resize, no clamp at all on move); `edit`'s
   `clamp_rect` runs on every drag step. A real behavioural difference beyond
   the two (`MIN_SIZE` vs `MIN_WINDOW`, who posts the close/zoom command) the
   issue itself named.
4. The two implementations' move/resize math (`Desktop`'s "delta since
   anchor" vs. `edit`'s "offset from origin/corner at grab time") are
   algebraically identical only because both hit-tests constrain the initial
   grab to exactly one cell (the title row, or the exact bottom-right cell).
   Implicit today in both codebases; worth an explicit test once shared.

## Decision

**A new top-level module, `rvision::arrange`** (`src/arrange.rs`, sibling to
`geometry`/`command`/`event` — not nested under `widgets`): stateless plain
functions over `Rect`/`Point`/`Size`, no trait, no generics, no knowledge of
`View`/`Window`/any concrete document type. See `docs/specs/arrange.md` for
the full interface, invariants, and test plan; in summary:

- `ChromeFlags` (a small struct — `moveable`/`resizable`/`closable`/
  `zoomable`/`has_help` — grouped so a call site can't transpose two
  same-typed bools) and `chrome_hit(bounds, pos, flags) -> ChromeHit`
  (`Close | Zoom | Help | Move | Resize | None`), fully gated on `flags`
  internally — a geometric hit on a disabled affordance is `None`, not the
  geometric variant, so no caller needs a second flag check.
- `ArrangeKind`/`ArrangeSession`/`start_session`/`continue_session`: an
  opaque in-progress move/resize session, `min_size` supplied by the caller
  at `continue_session` time rather than a hardcoded floor (`Desktop` and
  `edit` already disagree on it: `Size::new(10, 3)` vs `Size::new(3, 3)`). No
  ceiling/bounds clamp — that stays a separate, explicit `clamp_rect` a
  caller composes in if it wants it (resolves point 3 above by making the
  difference a caller choice rather than a silent divergence).
- `clamp_rect`, `cascade_slot` (now also caller-supplied `min_size`, for the
  same reason as `continue_session`'s), `tile` — `edit::app`'s existing
  layout functions, generalised.

**Both `Desktop` and `Window` are refactored onto `chrome_hit`** (resolves
point 1): `Desktop::start_session_if_applicable` for the Move/Resize path,
`Window::handle_event`'s mouse arm for Close/Zoom/Help. Each existing test
suite passing unchanged is the proof the extracted classification is
sufficient for its real caller — between the two, every `ChromeHit` variant
is exercised by a consumer that already existed before this change, not just
by the new module's own unit tests.

**`Desktop` gains real `cascade`/`tile` methods** (resolves point 2) — plain
methods, not new framework-reserved `Command`s, matching `open`/`hide`/`show`/
`focus`'s existing precedent (ADR 0016) that an operation needing no target-
window data beyond what's already on `self` doesn't need to travel as a bubbled
`Command`. Both skip hidden and maximized windows (a maximized window already
fills the desktop; left alone rather than force-restored) and reposition the
rest via `arrange::cascade_slot`/`tile`, backed by their own tests plus a
manual run-through added to `examples/mdi.rs`'s existing desktop demo. `edit`
adopting these stays separate, later, `edit`-side work — not part of this.

**`Window` gains an `arrangeable` flag** (default `true`), independent of
`moveable`/`resizable`/`closable`/`zoomable`: those gate *interactive*
affordances (a drag, a glyph click), while `arrangeable` gates whether
`cascade`/`tile`'s bulk sweep may touch this window at all. A docked,
fixed-position window (`examples/mdi.rs`'s toolbox) sets it `false` to sit
out of both operations entirely, regardless of visibility — and without
reserving it a slot, so the windows that *do* participate lay out as if it
were never open. Discovered from actually using the `mdi` demo's new
Cascade/Tile commands: the toolbox — already `resizable(false)`/
`zoomable(false)`/`closable(false)` precisely because it's meant to stay put
— still got swept into both, since neither flag was ever about *this* kind
of repositioning. A single flag rather than separate `cascadable`/`tileable`
ones: cascade and tile are both "bulk auto-layout," the same category the
`arrange` module's own name already groups them under, and no concrete
caller wants to opt out of one but not the other yet (CLAUDE.md: don't split
until a real need shows up).

**A non-`resizable` window participating in cascade/tile is moved, never
resized.** The same demo surfaced this too: `resizable(false)` reads as "my
size never changes," but cascade/tile resized every participating window
regardless, since the flag had only ever gated interactive corner-drag
before. Both now move such a window to its computed slot's *origin* while
keeping its own current size (clamped to the desktop so the kept size can't
push it off-screen) — `arranged_bounds` in `desktop.rs` is the one place
this distinction is made, shared by both methods.

**`Desktop`'s current no-bounds-clamping drag/resize behaviour is kept
as-is** (resolves point 3) rather than folded in as a side effect of this
refactor — `continue_session` deliberately has no ceiling, matching what
`Desktop` does today; a caller wanting `edit`'s clamped behaviour composes
`clamp_rect` itself. The "a window could become hard to reach" concern this
raises is tracked separately as rvision#9 (a Window List dialog to find/reset
one), backlog, not blocking here.

**The grab-point invariant (point 4) gets an explicit test** in
`arrange`'s own suite, asserting `continue_session`'s delta-from-anchor
result matches an offset-from-corner computation, specifically under the
constraint both callers' `chrome_hit`-driven session start already enforces.

## Consequences

- One classification/session implementation instead of `Desktop`'s and
  `Window`'s separately-re-derived glyph math, with `edit` positioned to
  adopt the same functions later without `rvision` needing to change again.
- `Desktop` becomes capable of cascade/tile for the first time — a real
  feature addition riding along with the deduplication, not just a refactor.
- `Desktop`'s and `edit`'s drag/resize behaviour stays exactly as different
  as it is today (bounds-clamping, minimum size) — this ADR deliberately
  doesn't unify behaviour, only the code that computes it; a future ADR can
  revisit clamping if rvision#9 or something else makes it worth doing.
- `edit` migrating onto `rvision::arrange` is unblocked but not required or
  scheduled by this decision — its own later, `edit`-side ADR, exactly as
  ADR 0016 already treated `edit`'s adoption of `Desktop`/`Window` themselves.

## Alternatives considered

- **A trait-based abstraction** (`Window`/`Document` both implement some
  `Arrangeable` interface the module is generic over). Rejected: invites
  every consumer to shape its own variant, which is exactly the
  fork-into-incompatible-copies outcome this ADR exists to end. Plain
  functions over primitive geometry can't be implemented differently by
  different consumers — there's only one way to call them.
- **Leave the duplication as-is.** Rejected — it's the actual, measured,
  growing problem `edit`'s recent help-overlay work made concrete.
- **`edit` adopts `rvision::Desktop`/`Window` directly instead**, resolving
  the tension the other way. Rejected as the fix for *this* problem: it
  doesn't touch the underlying concrete-access constraint, so it would cost a
  large `edit`-side rewrite for no gain on the actual blocker; extracting the
  arrangement *math* — never the part requiring concrete access — gets the
  deduplication without that cost.
- **Fold `Desktop`'s bounds-clamping to match `edit`'s as part of this
  refactor.** Rejected as unnecessary, undiscussed scope creep on a pass that
  was about deduplicating code, not changing behaviour; kept as a caller
  choice (`clamp_rect`) instead, revisitable later on its own merits.
