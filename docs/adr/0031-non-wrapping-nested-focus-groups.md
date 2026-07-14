# ADR 0031 — Non-wrapping nested focus groups (`Group::non_wrapping`)

- **Status:** Accepted
- **Date:** 2026-07-05

## Context

`docs/specs/view.md` already named this gap as an open question: "focus
currently wraps within a group; handing off to the parent at the boundary is
a Phase 4/5 refinement." It stayed theoretical because nothing had ever
nested a `Group` as one child *inside* another `Group` for focus-traversal
purposes — every existing composite (`ColorPicker`, `ThemeEditor`, `ComboBox`)
holds its several controls directly in one flat `Group`, not a `Group` of its
own.

`GroupBox` (`docs/specs/group_box.md`) is the first widget to do that: a
titled border owning a real `Group` of children, itself placed as one child
of whatever dialog `Group` it sits in. Manually driving the `dialogs`
example's Settings dialog (an `InputLine`, a `CheckBox`, a `GroupBox` wrapping
a `RadioButtons`, then OK/Cancel buttons) surfaced the gap for real: once
`Tab` reached the group box, it could never leave. `Group::move_focus`
treats "no further focusable child in this direction" as "wrap back to
whichever end is next" and reports the key `Consumed` unconditionally once
`focusable` is non-empty — correct for a *top-level* dialog `Group` (which has
no owner to hand off to, and existing tests rely on exactly this wrap), but
wrong for `GroupBox`'s *interior* `Group`: with only one focusable child (a
lone `RadioButtons`, this widget's whole motivating case), every `Tab` "wraps"
to the same single child and is swallowed, so focus can never reach the
OK/Cancel buttons laid out after the box.

## Decision

Give `Group` a per-instance opt-out, defaulting to today's behaviour:

```rust
impl Group {
    pub fn non_wrapping(mut self) -> Self { self.wraps = false; self }
}
```

`move_focus` checks it before wrapping: if `!self.wraps` and the requested
move would need to wrap (already at the last focusable child going forward,
or the first going backward), it returns `EventResult::Ignored` **without**
changing `self.focused`, instead of consuming and wrapping. Everywhere else —
moving between two focusable children that aren't at the boundary yet — is
unchanged, still `Consumed`, so a non-wrapping group with several focusable
children still cycles through all of them internally before a boundary Tab
finally escapes.

Because `Consumed`/`Ignored` bubbling is already the entire mechanism ADR
0003/0004 built dispatch on, nothing else needs to change: when a nested,
non-wrapping `Group`'s boundary `Tab` comes back `Ignored`, its owner (here,
`GroupBox::handle_event`, which forwards non-mouse events straight to its
interior) simply returns that `Ignored` up one more level, and the *outer*
`Group`'s own `dispatch_focused` — seeing its currently-focused child (the
whole `GroupBox`) did not consume the key — falls through to its own
`move_focus` exactly as if that child were an ordinary leaf that ignored Tab.
This composes to arbitrary nesting depth for free; no new event, no new
`Context` plumbing, no protocol beyond the existing `EventResult`.

`wraps: true` is `Group`'s default (set unconditionally in `Group::new`), so
every existing caller — every top-level dialog/window interior — is
byte-for-byte unaffected; only `GroupBox` opts in, on its own interior:

```rust
interior: Group::new(interior_bounds, children).non_wrapping(),
```

## Consequences

- `GroupBox` composes correctly with an owning `Group`'s own Tab order: Tab
  cycles through a box's own children (if it has more than one) and then
  moves on to the next sibling past the box, exactly as if the box weren't
  there; Shift-Tab does the same in reverse, re-entering the box. Confirmed
  both by a unit test (`group_box.rs`,
  `tab_escapes_a_group_box_with_one_focusable_child_to_reach_a_later_sibling`)
  and manually, re-running the `dialogs` example's Settings dialog in a real
  terminal (tmux) after the fix.
- Re-entering a non-wrapping group (Tab forward into it, or Shift-Tab back
  into it) lands on whichever child it was last focused on, not necessarily
  that direction's "natural" end (its first child for forward entry, last for
  backward). Left as-is: a minor ergonomic nicety, not a correctness gap —
  see Open questions.
- Resolves `docs/specs/view.md`'s long-open "Cross-group Tab boundary"
  question; that file's Public interface/Behaviour sections are updated to
  document `non_wrapping`.
- A small, generic, reusable mechanism: any future composite that owns a
  nested `Group` of its own focusable children (not just `GroupBox`) gets the
  same correct hand-off by opting in the same way, without a bespoke fix per
  widget — the same shape ADR 0030's `View::wants_topmost` took for the
  z-order gap `ComboBox` surfaced.

## Alternatives considered

- **A new `View`-trait-wide method** (mirroring `wants_topmost`/
  `drop_shadow`), e.g. `fn focus_exhausted(&self, forward: bool) -> bool`,
  queried by an owning `Group` instead of relying on `Ignored` bubbling.
  Rejected: `Consumed`/`Ignored` already says exactly this ("nothing more for
  me to do with this key") for every other case in the dispatch engine: a
  second, parallel signal would duplicate a distinction the engine already
  makes for free, for no new capability.
- **Always non-wrapping** (drop the flag, change `Group`'s default). Rejected:
  breaks every top-level dialog/window `Group`, which has no owner to hand a
  boundary Tab off to and must keep cycling its own controls — several
  existing tests (`view.rs`'s `tab_and_back_tab_cycle_focus_skipping_static_text`
  and friends) encode exactly that wrap, and real dialogs (`dialogs.rs`)
  depend on Tab from the last button returning to the first field.
- **Reimplement focus cycling inside `GroupBox` directly**, bypassing a
  nested `Group` entirely. Rejected on the same "composition, not
  reinvention" grounds `docs/specs/combo_box.md` already argues from: `Group`
  already owns exactly this dispatch/traversal logic correctly; duplicating
  it inside `GroupBox` to work around one boundary case would be the
  reinvention this codebase's own precedent (ComboBox reusing `InputLine`/
  `ListBox` verbatim) argues against.

## Open questions

- ~~Re-entry doesn't reset to the "natural" end for the entry direction...~~
  Resolved by ADR 0038 (`View::reset_focus`) once `TabbedPages` became a
  real use case with 2+ focusable children in a non-wrapping group: a caller
  can now explicitly reset a group's remembered cursor back to its first
  focusable child before a future fresh entry. Direction-aware re-entry
  (landing at the *last* child for a backward entry) remains open — see
  ADR 0038's own Open Questions.
