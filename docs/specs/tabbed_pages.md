# Module spec: `rvision::widgets::tabbed_pages`

- **Status:** Done. Manual pass done (tmux, `examples/dialogs.rs`): confirmed
  strip rendering, click-to-switch, Left/Right while the strip is focused,
  and Tab/Shift-Tab traversal in and out of each tab's content and past the
  whole widget to OK/Cancel. That pass surfaced two real gaps, both fixed
  during it: (1) the widget's own trailing `│` drawn after the *last* tab
  even with nothing to divide from (draw-only fix, `tabbed_pages.rs`); (2)
  the demo's "General" page was a plain (wrapping) `Group`, which — being
  more than one child — never reports `Ignored` at the Tab/Shift-Tab
  boundary, so focus could never escape it back to the strip. Fixed by
  building it `.non_wrapping()`, same as `GroupBox` already does internally
  for its own interior. (2) is not a `TabbedPages` bug — it confirms the
  Collaborators section's warning below: a page's escape behaviour is
  entirely the caller's responsibility, and easy to get wrong by omission.
- **Phase:** unscheduled (roadmap backlog #6, "New widgets")
- **Related ADRs:** 0003 (commands up / broadcasts down, views never
  reference siblings), 0004 (three-phase dispatch), 0008 (owner-relative
  coordinates + `Canvas`), 0010 (focus-aware controls), 0016 (border+interior
  composition and arbitrary-`View` content — the two idioms `Window`/
  `GroupBox` already establish, both reused here), 0017 (resize propagation
  per-view), 0031 (non-wrapping nested focus groups — the boundary-escape
  *semantics* this widget's hand-rolled two-slot focus arbitration mirrors),
  0036 (`Any` downcast access to a view's concrete type)

## Purpose

A titled tab strip over a set of pages, one page shown at a time —
TurboVision-adjacent "property sheet"/notebook control for organizing related
controls in a dialog that doesn't have room to show them all at once (e.g. a
Settings dialog's "General"/"Formatting" sections). Distinct from
[`GroupBox`](group_box.md) (a single always-visible bordered group — no
switching, no chrome beyond a title) and from [`Desktop`](desktop.md)/
[`WindowList`](window_list.md) (window management — this widget has no
`Desktop` awareness and never will). Each page owns exactly one arbitrary
`View`; a page needing several controls is the caller's own `Group`/
`GroupBox`, built before it's handed to `TabbedPages` — this widget's own job
is strictly the strip and which one page is currently shown.

## Public interface

```rust
pub struct TabbedPages { /* private */ }

impl TabbedPages {
    /// Creates a tab strip + bordered page area at `bounds`, one page per
    /// `(title, view)` pair in `tabs`, in that order — index 0 starts
    /// active, and the strip (not the page) starts holding keyboard focus.
    /// Mirrors `WindowList::new`'s `Vec<(id, label)>` pairing shape, so
    /// title and content can never fall out of sync via parallel vecs.
    /// Border/strip/interior fill resolve `Role::DialogBackground`, the
    /// active tab resolves `Role::Selection`/`Role::SelectionInactive`
    /// (same roles `ListBox`'s "always show current item" mode uses) —
    /// no new theme role.
    pub fn new(bounds: Rect, tabs: Vec<(&str, Box<dyn View>)>, theme: &Theme) -> Self;

    /// Sets the initially active page (clamped to the tab count).
    pub fn with_current(self, index: usize) -> Self;

    /// The index of the currently active/shown page.
    pub fn current(&self) -> usize;

    /// Switches the active page (clamped; a no-op if already current).
    /// Pure UI state, like `RadioButtons::selected`/`ComboBox::selected_index`
    /// — no command posted (contrast `WindowList`, which posts commands
    /// because it must ask an owner to mutate a `Desktop` it has no access
    /// to; `TabbedPages` has no such need).
    pub fn select(&mut self, index: usize);

    /// The number of tabs/pages.
    pub fn tab_count(&self) -> usize;

    /// Reaches page `index`'s view (any concrete type via `AsAny`, ADR
    /// 0036), `None` out of range.
    pub fn page(&self, index: usize) -> Option<&dyn View>;
    pub fn page_mut(&mut self, index: usize) -> Option<&mut dyn View>;

    /// The tab strip's rectangle in local coordinates: full width, row 0.
    pub fn strip_bounds(&self) -> Rect;

    /// The bordered page area in local coordinates — `GroupBox`-shaped,
    /// offset down by the strip row. Collapses to empty for a widget too
    /// small to have one.
    pub fn interior_bounds(&self) -> Rect;
}

impl View for TabbedPages { /* ... */ }
```

## Behaviour & invariants

- **Layout.** Row 0 is the strip (full width). Rows below are a single-line
  border box (no title of its own — the strip *is* the title), reusing
  `GroupBox`'s box-drawing glyphs verbatim, offset down by one row.
  `interior_bounds()` is that box inset by one cell on every side, same as
  `GroupBox::interior_bounds`. Each page's view is laid out in
  interior-local coordinates (`(0, 0)` one cell in from the border, two rows
  down from the widget's own top).
- **Two focus-participating slots, not N.** Unlike `Group`/`GroupBox`
  (N children, plain Tab cycling), `TabbedPages` has exactly two things that
  can hold keyboard focus: the strip itself, and the active page's view.
  This is hand-rolled rather than built from a real `Group`, because the
  strip is chrome this widget draws/hit-tests directly — it is never a
  stored `View` in a vec (mirrors `Frame`/`Window`'s glyphs, which also
  aren't separate `View`s).
- **Initial and construction-time focus.** `new` starts with the strip
  holding focus, never a page — mirrors `Group::new`'s "focus starts on the
  first focusable slot" rule, treating the strip as always-slot-0. A page's
  `set_focused(true)` is only ever called once the user actually Tabs or
  clicks into it.
- **Strip is always focusable** iff `tab_count() > 0` (mirrors `RadioButtons`
  always being focusable with at least one label).
- **Mouse.** A left-click anywhere in `strip_bounds()` focuses the strip
  (unfocusing the active page first if it held focus) and, if it landed
  within a specific tab's hit-test span (`tab_columns`, computed from label
  widths in `new`/`set_bounds`), switches to that tab — always `Consumed`,
  mirroring `RadioButtons`' "any click within bounds consumes." A left-click
  inside `interior_bounds()` translates into the active page's local
  coordinates exactly like `GroupBox::handle_event`'s Mouse arm, and — if
  the page is focusable — moves focus onto it first (mirrors
  `Group::dispatch_positional`'s pre-forward `set_focus`). A click on the
  border, or outside the widget, is `Ignored` (mirrors `GroupBox`'s
  "click on the border is ignored").
- **Keyboard, strip focused.** Left/Right move `current` by one, **clamped**
  at the ends (no wrap) — the literal `RadioButtons` Up/Down algorithm,
  transposed to a horizontal axis; confirmed by reading `radio_buttons.rs`
  directly rather than assumed. Tab moves focus onto the active page if it's
  focusable, else is `Ignored` (boundary escape — the strip is the last slot
  when the page can't take focus). BackTab is always `Ignored` — the strip
  is unconditionally the first slot, so Shift-Tab from it always escapes
  outward, non-wrapping in the same *sense* as ADR 0031 (though hand-rolled,
  since `Group::non_wrapping()` only applies to a real `Vec<Box<dyn View>>`
  child list, and the strip isn't one).
- **Keyboard, page focused.** The active page's view gets first crack.
  If it ignores the event and the event is specifically BackTab, focus
  returns to the strip (`Consumed`). Forward Tab is never handled here —
  the page is the last slot, so a boundary forward Tab always escapes as
  `Ignored`, letting it bubble to whatever owns this widget's own dispatch
  (the load-bearing case this widget must not regress on, per `GroupBox`'s
  own history with ADR 0031). Left/Right reaching here (ignored by the page)
  are **not** intercepted — arrow-based tab switching is scoped to the strip
  only, so a page's own content (e.g. a `TextArea`) can use arrow keys
  undisturbed.
- **Command | Paste.** Always forwarded straight to the active page's view,
  with no focus gating — mirrors `Window`'s catch-all forwarding to its one
  interior: there is exactly one sensible destination regardless of which of
  the two slots currently holds keyboard focus.
- **Broadcast | Resize | Idle.** Delivered to **every** page's view, active
  or not (`Ignored` returned) — the literal `Desktop::handle_event` fan-out
  for hidden windows, so an inactive page's own state (a scroll position, a
  cached wrap) stays current for when its tab is next selected, without
  being rebuilt from scratch. Inactive pages are never rebuilt on switch —
  each is a persistently-held `Box<dyn View>` for the widget's whole
  lifetime, so a page's internal focus/scroll/state survives being hidden
  and shown again for free.
- **`valid`.** Folds (does not short-circuit) over every page, mirroring
  `Group::valid`/`Desktop::valid` — an inactive page with unsaved state can
  still veto a Cancel/Close the same way an inactive `Window` can.
- **`set_bounds`.** Recomputes `tab_columns` and `interior_bounds()`, then
  pushes the new interior rect to **every** page via `View::set_bounds`
  (ADR 0017) — not just the active one, generalising `Window::set_bounds`'s
  single-interior propagation to N pages, consistent with the
  broadcast-reaches-everyone rule above.
- **Drawing.** Strip row, then the border box, then the interior filled with
  `interior_fill`, then **only** the active page's view is drawn — never the
  inactive ones (the "skip for draw" half of the `Desktop` hidden-window
  precedent). The active tab's label uses `Role::Selection` while the strip
  holds focus, `Role::SelectionInactive` otherwise (whenever a page holds
  focus, or the whole widget doesn't) — permanently on, no opt-out, since a
  tab bar that hides which page is active defeats its own purpose.
- **Tab label rendering.** Each renders `" {title} "` (mirrors `GroupBox`'s
  title padding), separated by a single `│`, truncated to fit if a label
  would overflow. A tab that doesn't fit in the strip at all past that point
  is simply not drawn and gets no `tab_columns` entry (see Open questions —
  no strip scrolling in v1; Left/Right can still reach it by index).
- **Degenerate sizes.** No tabs: not focusable, ignores every event, draws
  an empty/blank widget without panic. A widget too small for a strip row
  plus a usable interior (`interior_bounds()` collapses) degrades without
  panic, mirroring `GroupBox`'s tiny-area handling.

## Collaborators

- [`Group`](view.md)'s dispatch-phase *shape* (positional/focused/broadcast)
  and its `move_focus`/non-wrapping-boundary algorithm are the templates for
  this widget's own two-slot arbitration — not reused by composition (the
  strip has no `View` identity), reimplemented deliberately in the same
  spirit as `GroupBox` reimplements nothing but *does* reuse `Group` for its
  interior.
- `Canvas`/`Buffer`, `geometry`, `cell::Cell`, `theme::{Role, Theme}`,
  `color::Style` — drawing, reusing `GroupBox`'s border-glyph approach and
  `ListBox`'s `Role::Selection`/`SelectionInactive` precedent.
- `view::{View, Context}`, `event` types — same seam as every other
  container; posts no commands of its own (mirrors `RadioButtons`/`ComboBox`,
  contrast `WindowList`).
- Each page is caller-supplied (`Box<dyn View>`) — may itself be a `Group`,
  `GroupBox`, or a single control. Whether a page's own internal Tab/
  Shift-Tab ever escapes back out to the strip depends entirely on how the
  *caller* built that page's content (e.g. whether its own `Group` was built
  `.non_wrapping()`) — `TabbedPages` has no way to impose that on
  caller-supplied content, exactly as `GroupBox` can't impose it on anything
  but its own directly-owned interior `Group`.

## Test plan (write these first)

- **Logic:** no tabs is not focusable and ignores every event (mirrors
  `empty_group_ignores_everything`); first tab is current by default;
  `with_current`/`select` clamp to the tab count; `strip_bounds` is the full
  width, row 0; `interior_bounds` is inset below the strip and collapses to
  empty for a too-small widget; `tab_columns` are recomputed after
  `set_bounds` widens or narrows the strip.
- **Interaction (scripted events):** a click on a tab label switches the
  current page and focuses the strip; a click on blank strip space focuses
  the strip but leaves the current page unchanged; a click on the border is
  ignored (mirrors `GroupBox`); a click inside the interior reaches the
  active page at translated coordinates, and focuses it if it's focusable;
  Left/Right move the current page while the strip is focused, clamped at
  the ends (mirrors `RadioButtons`' clamp test); arrows do nothing when a
  page (not the strip) holds focus; Tab from the strip moves focus onto a
  focusable page, or is ignored if the active page isn't focusable; BackTab
  from the strip always escapes; keys reach the focused page's content first
  (mirrors `GroupBox`'s "keys reach the focused child"); BackTab from an
  exhausted page returns focus to the strip; **forward Tab from an exhausted
  page escapes `TabbedPages` entirely** to reach a later sibling in an outer
  `Group` — structurally identical to `group_box.rs`'s
  `tab_escapes_a_group_box_with_one_focusable_child_to_reach_a_later_sibling`,
  the load-bearing regression test; switching tabs while a page holds focus
  unfocuses the old page and focuses the new one; a command posted by the
  active page's content bubbles out; Command/Paste reach the active page
  regardless of whether the strip or a page holds focus; a broadcast reaches
  every page including inactive ones (mirrors `broadcast_reaches_every_child`
  + `Desktop`'s hidden-window precedent); Resize/Idle also reach every page;
  `valid` folds over every page, not just the active one (mirrors
  `Group`/`Desktop::valid`'s non-short-circuiting fold); `set_bounds`
  recomputes geometry and propagates the new interior to every page
  including inactive ones.
- **Render (snapshot):** strip + bordered interior around the active page;
  the active tab is bright when the strip is focused vs. dim when a page or
  nothing holds focus (`Selection`/`SelectionInactive`); switching tabs draws
  only the newly active page's content; a too-small widget degrades without
  panic (mirrors `GroupBox`'s tiny-area test); a long tab title truncates to
  fit.
- **Manual:** `examples/dialogs.rs`'s Settings dialog, restructured into two
  tabs ("General": `Name:`/`InputLine` + `Word wrap` `CheckBox`;
  "Formatting": the existing `GroupBox`-wrapped `RadioButtons`, unchanged).
  Run in a real terminal (tmux): click each tab label, use Left/Right while
  the strip is focused, Tab/Shift-Tab in and out of each tab's content and
  past the whole widget to OK/Cancel, confirm switching tabs never loses a
  page's own control state (e.g. the input line's cursor position/text).

## Open questions

- No strip scrolling: a set of tabs whose labels don't all fit the widget's
  width simply stops drawing (and hit-testing) the ones that overflow.
  Revisit only if a real use case needs more tabs than comfortably fit one
  dialog's width — no evidence of that yet.
- Re-entering a page via Tab from the strip always lands wherever that
  page's own `Group`/content last left its internal focus (or its own
  natural default if never visited) — not necessarily "first control" on
  every re-entry. Mirrors `GroupBox`'s own open question about re-entry
  landing spot; not addressed here either.
