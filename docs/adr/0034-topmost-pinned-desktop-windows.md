# ADR 0034 — `topmost`-pinned `Desktop` windows

- **Status:** Accepted
- **Date:** 2026-07-06

## Context

Manually driving `examples/mdi.rs`'s new `Window ▸ Cascade`/`Tile` commands
(ADR 0033) surfaced a second, related gap: showing the demo's docked toolbox
and then raising an ordinary document window (a click, or `Window ▸ Next`)
let that document climb visually above the toolbox — nothing in `Desktop`
ever kept a window pinned above the rest regardless of raise order. A docked
utility panel (a toolbox, an inspector) is a common enough shape that this is
worth a real mechanism rather than an app working around it.

**This is not the same "topmost" ADR 0030 already added.** ADR 0030's
`View::wants_topmost` is a `Group`-scoped mechanism for a transient popup (a
`ComboBox`'s open dropdown) staying drawn above its *sibling views within the
same owner* — checked by `Group`'s own draw-order/dispatch logic, with no
knowledge of `Desktop`'s window stack at all. This ADR is about `Desktop`'s
own, separate z-order: which of its resident `Window`s sits above which
other one, regardless of which is active. The two never interact — a
`Window`'s *interior* could use ADR 0030's mechanism internally (as
`ComboBox` already does) independently of whether the `Window` itself is
`topmost` here.

## Decision

**`Window` gains a `topmost` flag** (`.topmost(yes)` / `is_topmost()`,
default `false`) — an axis independent of `moveable`/`resizable`/`closable`/
`zoomable` (which gate interactive affordances) and of `arrangeable` (ADR
0033, which gates `cascade`/`tile`'s bulk sweep): `topmost` gates `Desktop`'s
own z-order bookkeeping.

**`Desktop::raise` sends a `topmost` window to the true end of the stack,
and any other window only as far as just below the first `topmost`
entry.** This keeps `self.windows`' vec order doing exactly what it always
did — serving as the complete, literal z-order for both `draw` and
positional hit-testing — with no second, parallel notion of "visual
priority" to keep in sync. A `topmost` window can never be climbed above by
raising something else; raising *it* (a direct click, `open`/`show`/`focus`)
can still move it above another `topmost` window, preserving relative order
within that tier.

**Raising a window — `topmost` or not — still always makes it active.**
`topmost` is purely about z-order, not a "can't receive focus" flag: a
direct click on a pinned toolbox activates it exactly like clicking any
other window would. What changes is only the two places `Desktop` *guesses*
which window should become active next, without an explicit raise:

- **`activate_topmost_visible`** (`close`/`hide`'s fallback when the active
  window is removed) prefers the topmost visible *ordinary* (non-`topmost`)
  window, falling back to a `topmost` one only when nothing ordinary is left
  visible. Otherwise, closing or hiding some unrelated window would silently
  hand focus to a pinned toolbox sitting above it — the toolbox visually
  covering the desktop doesn't mean the user wanted to start typing into it.
- **`cycle_focus`** (`CM_NEXT`/`CM_PREV`) excludes `topmost` windows from its
  candidates entirely — the same shape as ADR 0033's `arrangeable` exclusion
  from `cascade`/`tile`: a pinned utility panel isn't one of "the windows"
  Tab-style cycling steps through.

`examples/mdi.rs`'s toolbox sets both `arrangeable(false)` and
`topmost(true)`: never swept into a cascade/tile layout, and never covered
by a document window once shown.

## Consequences

- `Desktop` gains a real "always on top" primitive with no new bookkeeping
  field — it's entirely expressed as *where in the existing vec* a window's
  entry sits, so every place that already trusted vec order (draw,
  positional hit-testing) keeps working unchanged.
- Multiple `topmost` windows are supported for free (they stack among
  themselves, all above every ordinary window) without being a design goal
  in itself — it falls out of "insert before the first `topmost` entry"
  rather than needing special-casing for exactly one.
- A `topmost` window can still be dragged/resized/closed/zoomed like any
  other, independently — this ADR doesn't couple `topmost` to the other
  flags; an application combines whichever it needs, as `mdi.rs`'s toolbox
  already does with `resizable(false)`/`zoomable(false)`/`closable(false)`.

## Alternatives considered

- **Reuse ADR 0030's `View::wants_topmost` for `Desktop`'s own windows.**
  Rejected: it's checked by `Group`'s dispatch over `Box<dyn View>` children
  in general, with no concept of `Desktop`'s `WindowId`-keyed stack,
  hide/show, or active-window bookkeeping — plumbing it through would mean
  either `Desktop` reimplementing `Group`'s logic anyway or a much larger
  change to unify two genuinely different owners' child-ordering concerns
  for no real gain over a dedicated, small `Window` flag.
- **A separate `z_index`/priority number instead of a bool.** Rejected as
  unneeded generality — nothing today asks for more than two tiers
  (ordinary, pinned-on-top); a numeric priority would need its own ordering
  rule for ties and give `cascade`/`tile`-style callers a much larger space
  to reason about for no concrete requirement driving it (CLAUDE.md: don't
  design for hypothetical future need).
- **Let `topmost` also imply `arrangeable(false)`,** since the one concrete
  use (a toolbox) wants both. Rejected: the two answer different questions
  (z-order vs. bulk-layout eligibility) and a future `topmost` window might
  still want to participate in cascade/tile — collapsing them would need
  undoing later for no cost saved now beyond one call-site keystroke.
- **Make `activate_topmost_visible`/`cycle_focus` configurable per-call
  instead of driven by the `Window` flag.** Rejected: the window itself, not
  the caller, is what knows whether it's a pinned utility panel; a per-call
  flag would need every call site (menu commands, accelerators, `Shell`) to
  independently get this right instead of it being a property of the window
  once, at construction.
