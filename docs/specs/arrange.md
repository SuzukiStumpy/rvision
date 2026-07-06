# Module spec: `rvision::arrange`

- **Status:** Done
- **Phase:** post-extraction rework (SDI/MDI convergence)
- **Related ADRs:** 0033 (extract window-arrangement geometry), 0016 (unify
  `Window`/`Dialog`, dynamic desktop), 0003 (owner chain, no shared refs)

## Purpose

Plain `Rect`/`Point`/`Size` functions for the window-arrangement math that
`Desktop` and `edit::app::EditorApp` each need — chrome hit-testing, in-progress
move/resize sessions, cascade/tile layout, bounds clamping — with no knowledge
of `View`, `Window`, or any concrete document type. It is **not** a trait
callers implement against; it is not the thing that decides *what happens* on
a hit (posting a command, starting a session) — that stays with each caller.

## Public interface

```rust
pub struct ChromeFlags {
    pub moveable: bool,
    pub resizable: bool,
    pub closable: bool,
    pub zoomable: bool,
    pub has_help: bool,
}

pub enum ChromeHit { Close, Zoom, Help, Move, Resize, None }

pub fn chrome_hit(bounds: Rect, pos: Point, flags: ChromeFlags) -> ChromeHit;

pub enum ArrangeKind { Move, Resize }

pub struct ArrangeSession { /* private: kind, anchor, start_bounds */ }

pub fn start_session(kind: ArrangeKind, bounds: Rect, anchor: Point) -> ArrangeSession;
pub fn continue_session(session: &ArrangeSession, pos: Point, min_size: Size) -> Rect;

pub fn clamp_rect(rect: Rect, bounds: Size) -> Rect;
pub fn cascade_slot(desktop: Size, index: usize, min_size: Size) -> Rect;
pub fn tile(desktop: Size, count: usize) -> Vec<Rect>;
```

## Behaviour & invariants

- `chrome_hit` is fully gated on `flags`: a geometric hit on a disabled
  affordance (e.g. the resize corner when `resizable` is `false`) returns
  `None`, never the geometric variant — callers never need a second flag
  check of their own.
- Close/zoom/help are tested first (each only when its own flag is set),
  before falling through to the resize-corner/title-row test, so a glyph hit
  is never also read as a move — matches `Desktop`'s current
  `!on_close && !on_zoom && !on_help` exclusion, just expressed as one
  ordered classification instead of a separate bool per caller.
- `bounds`/`pos` must be in the same coordinate space; `chrome_hit` has no
  opinion on which (screen-absolute for `Desktop`/`edit` today; a caller with
  desktop-local window rects would pass those consistently instead).
- `continue_session`'s `Move` translates `start_bounds` by the delta between
  `anchor` and `pos`; `Resize` grows/shrinks `start_bounds` by that same
  delta, floored (both dimensions independently) at `min_size`. No ceiling
  and no clamping to any outer bounds — a caller that wants the result kept
  on-screen composes `clamp_rect` afterward itself.
- **Grab-point invariant:** the delta-from-anchor formulation above and an
  "offset from the window's origin/corner at grab time" formulation (as
  `edit::app::start_move`/`start_resize` computes it) produce identical
  results *only* because both `Desktop` and `edit` constrain the initial grab
  to land on exactly one cell (the title row for move, the bottom-right
  corner for resize) before a session ever starts. `chrome_hit` is what
  enforces that constraint for both callers now; this module's own tests
  assert the two formulations agree under that constraint so a future change
  loosening either caller's hit-test to a margin rather than one cell would
  be caught here, not discovered as a drift bug later.
- `cascade_slot` steps down-right from the top-left by 2 columns / 1 row per
  index, wrapping every 8 so a long stack never marches off-screen; the step
  is capped so the slot's origin never pushes past `desktop - min_size`, and
  the resulting rect is always clamped (via `clamp_rect`) to fit `desktop`.
- `tile` lays `count` rects out in a roughly square grid (`cols` = smallest
  `c` with `c*c >= count`, `rows = count.div_ceil(cols)`); the last row/column
  absorbs the integer-division remainder so the grid always exactly fills
  `desktop` with no gap or overhang. No `min_size`: unlike cascade there is no
  stepping to cap, and a `desktop` smaller than `count` cells can produce
  empty rects — the caller's problem, not this function's (same as
  `clamp_rect` producing an empty rect for a zero-or-negative `bounds`).

## Collaborators

- `geometry::{Point, Rect, Size}` — the only types this module knows about.
- `widgets::Frame`'s public `close_span`/`zoom_span`/`help_span` — `chrome_hit`
  calls into these for glyph geometry rather than re-deriving it, so the two
  never drift.
- Callers: `widgets::Desktop` (`start_session_if_applicable`/`continue_drag`,
  and the new `cascade`/`tile` methods), `widgets::Window` (its close/zoom/
  help glyph handling in `handle_event`). `edit::app::EditorApp` is a
  prospective future caller, not built against this yet (separate, later
  `edit`-side work).

## Test plan (write these first)

- **Logic:**
  - `chrome_hit`: each glyph (close/zoom/help) hit and miss, each gated off by
    its own flag (disabled → `None` even on a geometric hit), the has-help
    width-threshold shift (a frame wide enough for two glyphs but not three),
    title-row hit when `moveable`/not, resize-corner hit when `resizable`/not,
    plain interior/border → `None`, a glyph hit never falls through to
    `Move`.
  - `continue_session`: `Move` translates by the pos-since-anchor delta in
    both axes independently; `Resize` grows/shrinks and floors at `min_size`
    on both axes independently; the grab-point invariant test above.
  - `clamp_rect`: an in-bounds rect is unchanged; oversized width/height each
    clamp independently; a negative-origin or past-the-far-edge origin pulls
    back to fit; a `bounds` of zero collapses the result to empty without
    panicking.
  - `cascade_slot`: slots 0..7 step by (2,1) each; index 8 wraps back to the
    same slot as index 0; the step never pushes the origin past
    `desktop - min_size`; a `desktop` smaller than `min_size` still returns
    without panicking (clamped to empty/whatever fits).
  - `tile`: `count` 1 (fills `desktop` whole), 2 (side by side), 3 (uneven —
    one row of 2, one of 1, both absorbing remainder correctly), 4 (perfect
    2x2 square); every returned rect's union exactly equals `desktop` with no
    gap for each case.
- **Manual:** none directly (no drawing) — exercised indirectly through
  `examples/mdi.rs` via `Desktop`'s drag/resize/close/zoom/help/cascade/tile.

## Open questions

None outstanding — `edit`'s own adoption of this module is explicitly
out of scope for this pass (see ADR 0033).
