# ADR 0018 — Cascading menus: a path stack, right-anchored, item-level gating

- **Status:** Accepted
- **Date:** 2026-07-02

## Context

`MenuBar`'s pull-down is one level today (ADR 0009): `open: Option<usize>`
(which top-level menu is open) plus `highlight: usize` (which item within
it). The roadmap's backlog names cascading submenus — a `MenuItem` that
opens a nested pull-down instead of posting a command — as unscheduled work
whose shape was never pinned down. [`docs/specs/menu.md`] sketched the new
public interface but left five things open: how the state machine
generalizes, where a nested box anchors, whether hovering opens one,
whether nesting can go past one level, and whether a submenu branch can be
disabled.

## Decision

**State machine.** `Option<usize>` + `usize` generalizes to `path:
Vec<usize>`. `path[0]` is the bar-level menu index — exactly what `open`
was. `path[i]` for `i > 0` is the highlighted item within the menu opened
by `path[i - 1]`. The last entry is the *focused* level: `Up`/`Down`,
hot-key matching, hover, and hit-testing all act on it alone. `Right` or
`Enter` on a focused `Submenu` item pushes a new `0` and opens that
submenu; `Enter`/a hot-key on a `Command` item posts it and clears `path`
entirely, same as today. `Left`/`Esc` at depth > 0 pop one level; at depth
0 they keep today's meaning (`Left`/`Right` cycle sibling top-level menus,
`Esc` closes the bar). One `usize` per level is enough — the item
highlighted at depth *i* is the same item that opened depth *i + 1*, so it
doubles as both "the highlight to draw" and "the parent of the next level"
without a second parallel array.

**Anchor.** A nested box opens to the right of its parent item's row,
top-aligned with that row, and flips to the parent's left edge if it would
run off the right of the screen — the same clamp shape `pulldown_area`
already applies to the single top-level box today, just evaluated once per
open level.

**No hover-to-open.** Opening a submenu always requires an explicit
`Right`, `Enter`, or click. `MouseKind::Moved` keeps its one job — tracking
the highlight — unchanged.

**Nesting depth uncapped.** `path: Vec<usize>` supports arbitrary
recursion with no extra code; a depth cap would need its *own* enforcement
logic for strictly less capability. The behaviour is documented and tested
to depth 2, with no artificial limit built in.

**Item-level gating, not branch derivation.** A submenu's availability is
not derived by inspecting its descendants. Instead `MenuItem` grows an
optional gate — a `Command` checked through the same `CommandSet`/`Context`
plumbing a plain item's own command already uses (ADR 0003, 0004) — so a
`Submenu` item can be disabled (greyed, unreachable, its cascade never
opens) exactly like a `Command` item is. A `Command` item's gate stays its
own command, unchanged; a `Submenu` item's gate defaults to `None` (always
enabled) and opts in via a builder. No new enabled-state mechanism is
introduced — the existing one just applies to one more item shape.

## Consequences

- [`docs/specs/menu.md`] moves from "open questions block further design"
  to ready-to-build test-first; its interface sketch and behaviour section
  already reflect these five decisions.
- Cascading reuses the whole of ADR 0009's "draw last, as a full-frame
  overlay" decision unchanged: `draw_overlay` grows to iterate `path`'s
  levels instead of a single `Option`, but nothing about the shell's
  layout, draw-ordering, or accelerator routing changes.
- Every existing single-level behaviour and test keeps working as-is: a
  `path` with one entry is exactly today's `Option<usize>` + `usize`, just
  renamed.
- Right-click context menus (also on the backlog) inherit this same
  nesting/anchoring machinery once they're built — the roadmap already
  notes they're "best explored after submenus."

## Alternatives considered

- **Derive a branch's enabled state from whether any leaf command
  underneath it is enabled.** Rejected: it needs walking the nested `Menu`
  on every draw and every gate check, and produces a confusing UI — a
  branch greys out for an opaque reason ("something several levels down
  happens to be disabled") instead of one the app stated directly. A
  per-item gate is simpler and matches how Windows, macOS, and TurboVision
  itself hand this decision to the app, not the framework.
- **Auto-open a submenu on hover.** Rejected: needs a hover-delay timer —
  nothing in `rvision` has one, and the crate budget rules out async/tokio
  machinery just to build one — or every box the pointer passes over en
  route elsewhere flashes open. An explicit trigger keeps the interaction
  model identical to today's top-level pull-down.
- **Cap nesting at one level**, matching the roadmap's literal "a nested
  pull-down" wording, with an explicit depth check. Rejected: `Vec<usize>`
  already generalizes for free; a cap is strictly more code for less
  capability, and the check would need its own tests to prove it fires
  correctly.
- **A `Vec<(usize, usize)>` of (opened-at index, highlight) pairs** instead
  of a single `Vec<usize>`. Rejected as redundant: the item highlighted at
  a given depth already *is* the item whose submenu opened the next depth
  — a second array would just be two names for the same number.

[`docs/specs/menu.md`]: ../specs/menu.md
