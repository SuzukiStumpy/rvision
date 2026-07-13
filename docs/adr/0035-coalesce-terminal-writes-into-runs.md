# ADR 0035 — Coalesce `CrosstermBackend::present`'s writes into runs

- **Status:** Accepted
- **Date:** 2026-07-14

## Context

`CrosstermBackend::present` (`src/crossterm_backend.rs`) walks `frame.diff(&self.front)`
— the changed cells since the last present, in row-major order (ADR 0002) — and
for **every single cell**, unconditionally queues five separate crossterm
commands: `MoveTo`, `SetAttribute(Attribute::Reset)`, `SetForegroundColor`,
`SetBackgroundColor`, `queue_attrs` (itself up to six more), and `Print` of one
grapheme. There is no batching: two adjacent, identically-styled changed cells
on the same row each pay the full cost independently, even though a real
terminal write only needs one cursor move and one style change to cover both,
followed by a single, longer `Print`. `docs/specs/backend.md`'s own "Open
questions" already named this exact gap ("`present` re-specifies fg/bg/attrs
for every changed cell; tracking the current terminal style to emit fewer
escapes is a later optimisation") — it was known and deliberately deferred,
not an oversight.

Found concretely while investigating a reported "floaty" window-drag feel in
`edit`, a downstream consumer: dragging a window rapidly redraws a
content-dense region behind it every frame. In-process measurements in `edit`
(frame composition, and `Buffer::diff`'s changed-cell *count*) ruled out
`edit`'s own code as the bottleneck — both were symmetric, sub-millisecond
costs regardless of scenario. That left the one remaining, unmeasured link in
the chain: the actual terminal write this ADR addresses. `CrosstermBackend`
hardcodes `io::stdout()` (not swappable in tests), so its real write cost
can't be timed headlessly — this fix proceeds on the strength of the
structural gap being real and worth closing regardless, not on a headless
before/after timing.

## Decision

Insert a pure grouping step between `frame.diff(&self.front)` and the write
loop: `coalesce_runs(diff: Vec<(Point, &Cell)>) -> Vec<Run>`, where a `Run` is
a maximal sequence of cells that are on the **same row**, **column-contiguous**
(the next cell's `x` equals the running run's `end_col`, accounting for wide
graphemes occupying two columns), and **identically styled**. `present` then
writes one `MoveTo` + one style-set (`SetAttribute(Reset)` +
`SetForegroundColor` + `SetBackgroundColor` + `queue_attrs`) + one `Print` of
the run's concatenated graphemes, per run, instead of per cell.

This mirrors the module's own established pattern (`map_event`): a pure
function factored out of the I/O-bound `present`/`poll_event` methods so it's
unit-testable without a TTY, with the actual terminal write remaining a
manual-verification concern (`examples/`, live use). `coalesce_runs` takes
`Vec<(Point, &Cell)>` (`Buffer::diff`'s exact return shape) and returns owned
`Run`s (each owns a `String` of its concatenated graphemes, since a run's text
doesn't exist as a contiguous slice anywhere in the source `Buffer`).

Width-0 continuation cells (a wide grapheme's second column, ADR 0006) are
filtered before run-building, exactly as `present` already did — they carry
no glyph of their own, and the grapheme to their left already advances
`end_col` by 2 to account for the column they cover.

`Buffer::diff`'s output contract, `Backend`'s public trait, and `Style`'s
shape are all unchanged — this is entirely internal to `present`'s write
loop.

## Consequences

- Two adjacent same-style changed cells cost one `MoveTo`/style-set/`Print`
  instead of two; a long uniform run (blank space, or same-coloured text)
  costs one regardless of its length. Cost is now proportional to the number
  of *runs* in a frame's diff, not the number of changed *cells* — a real
  reduction whenever a diff has any adjacent same-style cells at all, which
  ordinary UI content (menu bars, window borders, blocks of plainly-styled
  text) does constantly.
- `coalesce_runs` is unit-tested directly (empty diff, a same-style contiguous
  run merges, a style change or a column gap or a row change each starts a
  new run, a wide grapheme's continuation cell is skipped and doesn't break
  contiguity) — real regression coverage for a change that used to be
  impossible to test without a TTY.
- `docs/specs/backend.md`'s "Open questions" entry on this is resolved and
  moves into the spec's `CrosstermBackend` behaviour section.
- Whether this is the *complete* explanation for the `edit` report that
  prompted it is still open — the actual terminal-write timing couldn't be
  measured headlessly. The structural gap is real and closed either way;
  confirming the full effect needs a live run.

## Alternatives considered

- **Track only the last-emitted style, without requiring column contiguity**
  (skip re-emitting `SetForegroundColor`/`SetBackgroundColor`/attrs if
  unchanged from the previous *write*, regardless of whether the cells are
  adjacent on screen). Rejected as strictly weaker: it saves the style-set
  calls but not the per-cell `MoveTo`/`Print` overhead, and diffs are
  frequently non-contiguous (`Buffer::diff` skips unchanged cells), so this
  would miss most of the actual win a dense, mixed-position diff offers.
  `coalesce_runs`'s contiguity requirement is what lets multiple cells share
  one `Print` at all.
- **Batch across rows** (treat the whole diff as one run-finding pass
  ignoring row boundaries, relying on `MoveTo` semantics). Rejected: crossterm
  positions are absolute per `MoveTo`, and a run spanning a row wrap has no
  natural single `Print` target — the added complexity buys nothing since
  `Buffer::diff` already only ever needs a new `MoveTo` at a row boundary
  regardless of batching.
- **Leave it deferred, as `docs/specs/backend.md` already had it.** Rejected
  now that a concrete report gave it a measured (if incomplete) motivation
  and the fix is a small, self-contained, well-tested pure function — the
  kind of thing that's more expensive to defer a second time than to do.
