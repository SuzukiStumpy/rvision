# ADR 0038 — Per-view `reset_focus` protocol for stale nested-group re-entry

- **Status:** Accepted
- **Date:** 2026-07-14

## Context

ADR 0031 gave `Group` a `non_wrapping()` opt-out so a boundary Tab/Shift-Tab
can escape a nested composite instead of wrapping locally, and named an
explicit open question at the time: "Re-entry doesn't reset to the 'natural'
end for the entry direction... it lands wherever the nested group's focus
was left... revisit if a real use case wants that ergonomic polish; nothing
so far has needed it." It stayed theoretical because every existing
non-wrapping consumer (`GroupBox`) wraps exactly one focusable child, where
"resume last position" and "reset to the start" are identical — there was
nothing to distinguish.

`widgets::TabbedPages` (`docs/specs/tabbed_pages.md`) is the first composite
whose page content can have *two or more* focusable children in a
non-wrapping `Group`, repeatedly escaped and re-entered as the user tabs
around a dialog. Manually driving `examples/dialogs.rs`'s Settings dialog
(General tab: an `InputLine` + a `CheckBox`) surfaced the gap for real: once
Tab escaped the page via the `CheckBox` (the last focusable child), the
page's `Group` permanently remembered `CheckBox` as its focus target.
`TabbedPages::focus_page()` re-enters a page purely via
`View::set_focused(true)`, which only ever forwards to "whichever child I
last remembered" — so every subsequent re-entry landed straight back on
`CheckBox`, and `InputLine` became permanently unreachable by keyboard.

Fixing this at `Group::move_focus`'s boundary-escape point (resetting
`self.focused` the moment a boundary Tab escapes) was considered and
rejected: an existing test
(`a_non_wrapping_group_still_cycles_internally_before_the_boundary`)
explicitly encodes today's "focus stays put, only the event escapes" as the
intended ADR 0031 contract. Changing that silently would break a tested,
deliberate decision for every existing `non_wrapping` consumer, not just fix
the new one.

`View::set_focused(bool)` also has no way to express "this is a **fresh**
arrival, start from the top" versus "resume where you left off" — both look
identical to a `Group` receiving `set_focused(true)`. That distinction has to
come from somewhere; a container can't infer it from the boolean alone.

## Decision

Add a new per-view protocol method, mirroring the shape ADR 0015/0030/0032
already established for `scroll_metrics`/`wants_topmost`/`status_text` —
pulled on demand, default no-op, opt-in per composite:

```rust
impl View {
    /// Forgets any remembered internal focus position and snaps back to the
    /// natural starting point (`Group`'s first focusable child), so a
    /// future `set_focused(true)` begins fresh instead of resuming wherever
    /// it last was. The default is a no-op — only a composite that
    /// remembers an internal cursor needs to override it. Does not itself
    /// claim focus (does not call `set_focused(true)` on the new target):
    /// the caller decides whether/when this view actually gains focus.
    fn reset_focus(&mut self) {}
}
```

`Group` implements it by unfocusing whichever child currently holds its
`focused` index (if any) and repointing that index at the first focusable
child — exactly `Group::new`'s own initial pick, just re-run on demand.
`GroupBox` forwards to its interior `Group`, mirroring how it already
forwards `focusable`/`set_focused`/`valid`.

The caller decides *when* to call it — this method only resets the target's
memory, it doesn't decide re-entry policy. `TabbedPages` calls
`tab.view.reset_focus()` at exactly the moment a page's own boundary forward
Tab escapes (`Focus::Page`'s fallthrough to `Ignored` in
`tabbed_pages.rs::handle_key`) — the same place ADR 0031's escape already
happens, so the fix is one line at the existing hand-off point. A **backward**
escape (Shift-Tab exhausting at the *first* focusable child) needs no such
call: the boundary the group escapes from *is* the natural forward-re-entry
target already, so there's nothing to correct.

## Consequences

- Fixes the concrete bug: a `TabbedPages` page with two or more focusable
  children (`examples/dialogs.rs`'s "General" tab: `InputLine` + `CheckBox`)
  can be Tab-escaped and later fully re-entered by keyboard any number of
  times without ever stranding a control. Confirmed by a unit test
  (`tabbed_pages.rs`) and manually, re-running `examples/dialogs.rs` in tmux.
- `Group::non_wrapping`'s own boundary-escape behaviour (ADR 0031) is
  completely unchanged — "focus stays put, only the event escapes" remains
  true at the instant of escape. `reset_focus` is a separate, explicit,
  additive step a caller opts into, not a change to what escaping itself
  does.
- Resolves ADR 0031's "revisit if a real use case wants that ergonomic
  polish" open question — that ADR's Open Questions section is updated to
  point here.
- New, tiny, backward-compatible trait surface: every existing `View`
  implementor is unaffected by the default no-op. Only `Group` and
  `GroupBox` implement it meaningfully today; a future composite with the
  same "remembers an internal focus cursor" shape gets the same fix by
  overriding it, without a bespoke per-widget workaround.
- `Window` was deliberately **not** given a forwarding override: unlike
  `GroupBox`, `Window` is not driven through a plain `Group`'s child-cycling
  protocol in the first place (`Desktop` manages window activation its own
  way, not via `View::set_focused` on a sibling list), so there is no
  exercised call site that would ever invoke it.

## Alternatives considered

- **Fix `Group::move_focus` to reset `self.focused` at the boundary-escape
  point itself.** Rejected: breaks the existing, deliberately-tested ADR
  0031 contract ("focus stays put") for every current `non_wrapping`
  consumer, to fix a problem only some of them (2+ focusable children) ever
  have.
- **Thread direction through `set_focused`** (e.g.
  `set_focused(focused: bool, entering_forward: Option<bool>)`), so a `Group`
  could pick its first child on a forward entry and its last on a backward
  one. Rejected as a bigger, more invasive signature change to a method
  every existing `View` already implements, for a refinement
  (`ADR 0031`'s own "natural end per direction" nicety) beyond what the
  concrete bug needs — plain "reset to the start" already fixes the actual
  reported defect, and directional entry can still be layered on later
  behind the same method if a real case needs it.
- **Give `TabbedPages` special-cased knowledge of `Group`** (downcast a page
  to `Group` via `AsAny` and call a `Group`-only reset method directly).
  Rejected: only works when a page happens to be a bare `Group`, silently
  doing nothing for a `GroupBox`-wrapped or any other composite page —
  exactly the kind of type-specific special-casing `View`'s per-view
  protocol methods (`scroll_metrics`, `status_text`, `wants_topmost`) exist
  to avoid.

## Open questions

- `reset_focus` always snaps to the *first* focusable child, regardless of
  which direction the reset is conceptually "for." A backward re-entry
  landing at the *last* child (ADR 0031's original "natural end per
  direction" framing) would need direction threaded through, which the
  Alternatives above defer until a real case asks for it.
