# ADR 0030 — Per-view topmost priority for transient popups (`View::wants_topmost`)

- **Status:** Accepted
- **Date:** 2026-07-05

## Context

`ComboBox` (`docs/specs/combo_box.md`) avoids the `Shell`-owned overlay
machinery (ADR 0009/0019) by exploiting something already true of `Group`:
both `draw` and `dispatch_positional` query a child's `bounds()` fresh every
frame/event, so `ComboBox::bounds()` simply reports a taller rectangle while
its drop-down is open, and ordinary dispatch/draw take care of the rest —
no new protocol needed for *geometry*.

Manual testing (a dialog: a `Label`, a `ComboBox`, then OK/Cancel `Button`s,
all children of one `Group`) surfaced the piece that idea doesn't cover:
*z-order*. `Group` draws children in vector order — index 0 bottom, last on
top — and `dispatch_positional` hit-tests in the reverse of that, so a later
sibling always wins where it overlaps an earlier one, regardless of which
one is bigger right now. With enough candidates to fill the drop-down, its
lower rows fell under the dialog's own OK/Cancel buttons: the buttons drew
over it, and a click meant for a suggestion row landed on a button instead.

`docs/specs/combo_box.md`'s first draft called this an accepted trade-off
("the dialog author must leave room below it") — the same framing ADR 0011's
drop shadow and ADR 0026's obscured-control note both use. Reported back,
though, this one wasn't accepted: a combo box whose own drop-down can be
covered by ordinary sibling chrome any time the candidate list is long
enough is a real usability gap in the widget itself, not a layout nicety to
leave to callers.

## Decision

Add one more defaulted `View` method, the same shape as `drop_shadow`
(ADR 0011) and `scroll_metrics` (ADR 0015) — a property a view declares,
acted on by its owner:

```rust
trait View {
    fn wants_topmost(&self) -> bool {
        false
    }
}
```

`Group` queries it on every child, for both passes:

- **`draw`**: every child reporting `false` draws first, in its original
  relative order (unchanged from today); every child reporting `true` draws
  *after*, also in its original relative order. A requesting child now
  always paints over an ordinary sibling, wherever either sits in the
  vector.
- **`dispatch_positional`**: `true`-reporting children are hit-tested first
  (in the reverse of their relative order, so two such children still
  resolve topmost-first among themselves), before falling through to the
  ordinary reverse-order scan of the rest.

Default `false` means every existing widget — and every existing `Group`
test — is unaffected; the reordering only activates for a child that opts
in, and typically only while it actually has something transient open (for
`ComboBox`, `wants_topmost()` just returns `self.open`).

Only `Group` is touched. `Desktop` already has its own, separate z-order
mechanism (click-to-front window ordering, ADR 0016) at a different scale —
a `ComboBox` living directly on a `Desktop` rather than inside a `Window`'s
interior `Group` isn't addressed here, deferred until something actually
needs it (YAGNI).

## Consequences

- `ComboBox`'s drop-down now wins z-order over *any* sibling in the same
  `Group` while open, not just ones physically laid out below it — a dialog
  author no longer has to reserve blank rows or order controls carefully to
  keep it from being obscured. `docs/specs/combo_box.md`'s "key design
  decision" section is updated accordingly: the trade-off is resolved, not
  merely documented.
- The `Window`/`Canvas` clip at the dialog's own edge is untouched by this —
  a drop-down still can't paint past its host window's border; that's a
  separate, pre-existing clipping limit (ADR 0008), not what this ADR
  addresses.
- A small, generic, reusable mechanism: any future control that needs to
  transiently outrank its siblings (not just draw outside its own bounds,
  which `drop_shadow` already covers) can opt in the same way, without a
  bespoke solution per widget.

## Alternatives considered

- **Generalise the `Shell`-owned overlay (ADR 0009/0019) to arbitrary
  nesting depth**, so any view could request a true screen-level overlay.
  Rejected: the actual gap was a *local* z-order question, fully solvable
  within the one `Group` a `ComboBox` sits in — reaching for `Context`'s
  offset accumulator and a `Shell`-drained request queue would be
  materially more machinery for the same outcome.
- **Leave it as an accepted trade-off**, per `combo_box.md`'s first draft.
  Rejected once reported back as real friction rather than acceptable
  layout discipline — unlike a drop shadow (which only ever needs a couple
  of rows/columns of margin, cheap to reserve) an open combo box's
  footprint scales with however many candidates match, which a dialog
  author can't size around in advance.
