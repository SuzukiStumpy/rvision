# Module spec: `rvision::widgets::combo_box`

- **Status:** Done. Manual pass done: the `combo_box` example (filtering,
  type-ahead, and select-only combo boxes together) exercised on a real
  terminal — typing/narrowing, Down-preview, Esc-revert, mouse row-pick, the
  z-order fix (ADR 0030) confirmed against a dialog whose OK/Cancel buttons
  the filtering combo's drop-down reaches, and both new flags' behaviour
  (type-ahead jump, select-only search/jump) all observed working as
  designed. Two real gaps surfaced and were fixed during the pass, not just
  noted: the drop-down's z-order (ADR 0030) and `Window::esc_cancels`'s
  interaction with a combo box's own `Esc` handling (documented below,
  addressed by leaving `esc_cancels` off in the example).
- **Phase:** unscheduled (roadmap backlog #6, "New widgets")
- **Related ADRs:** 0003 (commands up / broadcasts down, views never reference
  siblings), 0004 (three-phase dispatch), 0006/0008 (grapheme editing +
  owner-relative `Canvas`), 0010 (focus-aware drawing), 0011 (drop shadows —
  the sibling precedent for "the owner acts on what a child declares"), 0017
  (resize propagation), 0030 (per-view topmost priority — the drop-down's
  z-order fix)

## Purpose

An editable text field with an attached drop-down of suggestions —
TurboVision has no native equivalent; this is closer to a Windows
`CBS_DROPDOWN` combo box or a browser `<input>` with a `<datalist>`. By
default it picks one string value that may or may not be one of a fixed
candidate list: typing free text is always accepted, and the drop-down is
discoverability/completion, never a hard constraint. Two independent flags
narrow that default: `filterable(false)` swaps "typing narrows the list" for
"typing jumps the highlight to the first match, full list still shown"
(classic list type-ahead); `select_only(true)` removes free text entirely,
locking the value to one of `items`. With both, or with `select_only` alone,
it *is* a pure "pick one of N" selector — but arrives there as a
configuration of the general control, not a separate widget, since the two
behaviours share every other mechanism (drop-down composition, z-order,
navigation). Not a filesystem-specific picker (that's
[`FileDialog`](window.md)).

## The key design decision: no overlay

Every other "pops up something bigger than itself" widget in this codebase
(`MenuBar`'s pull-down, `ContextMenu`) needs `Shell` to draw a full-frame
overlay after everything else, because that mechanism (ADR 0009/0019) is
scoped to `Shell`'s own three permanent chrome children — reaching it from an
arbitrary `ComboBox` sitting at arbitrary depth inside a `Window`'s interior
`Group` would mean generalising that overlay protocol to any nesting depth,
which ADR 0009 explicitly deferred as speculative.

`ComboBox` avoids needing that entirely by exploiting something already true
of the engine: both `Group::draw` and `Group::dispatch_positional` call
`child.bounds()` **fresh, every frame/event** (`view.rs`) — nothing caches it.
So `ComboBox::bounds()` simply reports a *taller* rectangle while its
drop-down is open (one row for the field, plus one row per visible
suggestion) and a one-row rectangle while closed. Ordinary positional
dispatch then routes clicks on the suggestion rows to `ComboBox` with no new
protocol, and ordinary `Group::draw` hands it a correspondingly taller
`Canvas` to draw the list into. This is the same trick `drop_shadow` (ADR
0011) uses in spirit — let the existing owner-queries-child-every-frame
machinery do the work — taken one step further: instead of the *owner*
painting something outside the child's bounds, the child's bounds simply
grow to cover it, so nothing outside `ComboBox` itself needs to know.

**The z-order gap this left, and its fix.** Geometry alone isn't enough:
`Group` still draws/hit-tests children in plain vector order, so a sibling
sitting later in the vector (a dialog's OK/Cancel buttons, laid out below
the combo box) would draw over an open drop-down big enough to reach them,
and steal its clicks — first drafted here as an accepted trade-off ("a
dialog author must leave room"), the same shape as a drop shadow's margin
requirement. Manual testing showed that framing didn't hold up: an open
combo box's footprint scales with however many candidates match, which a
dialog author can't reliably lay out around in advance. Resolved instead by
ADR 0030: `View::wants_topmost() -> bool` (default `false`), which
`ComboBox` reports as `self.open`. `Group` draws every `true`-reporting
child last (over ordinary siblings, wherever it sits in the vector) and
hit-tests it first — so the open drop-down now always wins against an
ordinary sibling occupying the same area, with no layout discipline
required of the caller. The one limit that *is* still just geometry, not
z-order: a drop-down still can't paint past its own host window's edge
(`Canvas` clipping, ADR 0008) — a dialog does still need to be tall enough
to show it, same as any other content.

## Public interface

```rust
pub struct ComboBox { /* input: InputLine, list: ListBox, items: Vec<String>, .. */ }

impl ComboBox {
    /// Creates an empty combo box at `bounds` (closed height: exactly one
    /// row) offering `items` as suggestions.
    pub fn new(bounds: Rect, items: Vec<String>, theme: &Theme) -> Self;

    /// Seeds the field's text (as if typed), cursor at the end.
    pub fn with_text(self, text: &str) -> Self;

    /// Replaces the field's text (as if typed) and closes the drop-down.
    pub fn set_text(&mut self, text: &str);

    /// The current field text — the value, whether or not it matches a
    /// listed item.
    pub fn text(&self) -> &str;

    /// The index into the original `items` this text exactly matches
    /// (case-insensitive), or `None` for free text that matches nothing.
    pub fn selected_index(&self) -> Option<usize>;

    /// Whether the drop-down is currently showing.
    pub fn is_open(&self) -> bool;

    /// Caps how many suggestion rows show at once (default 8).
    pub fn max_visible(self, n: usize) -> Self;

    /// `true` (default): typing narrows the drop-down. `false`: the
    /// drop-down always shows every candidate; typing jumps `highlight` to
    /// the first match instead (list type-ahead).
    pub fn filterable(self, yes: bool) -> Self;

    /// Locks the value to one of `items`: printable keys/paste never insert
    /// free text, only search-and-jump (`false` by default).
    pub fn select_only(self, yes: bool) -> Self;
}

impl View for ComboBox {
    // bounds(): 1 row closed; 1 + visible_rows() while open (see above).
    // focusable(): true.
    // set_focused(false): closes the drop-down (no revert) and forwards to `input`.
}
```

## Behaviour & invariants

- **Composition, not reinvention.** `ComboBox` embeds a real
  [`InputLine`](controls.md) for the text (grapheme cursor, selection,
  horizontal scroll, insert/overtype — all free) and a real
  [`ListBox`](list_box.md) for the suggestion rows (scrolling, row
  hit-testing — all free), exactly the composition precedent
  `ColorPicker`/`ThemeEditor` already established. `ComboBox`'s own code is
  the filter, the two data flows into/out of those controls, and the
  open/closed state machine — no duplicated cursor or scroll logic.
- **Filtering.** The drop-down always reflects `items` filtered to those
  starting with the field's current text, case-insensitively (`""` matches
  everything, so an empty field's drop-down is the full list — the "just a
  picker" case falls out of the general one for free). Rebuilt (a fresh
  `ListBox`, not a mutated one — cheap, and there is no
  `ListBox::set_items`) every time the field's text actually changes.
- **Two distinct ways the drop-down changes `text`, never confused:**
  - *Typing* (any key/paste delegated straight to the embedded `InputLine`,
    detected by diffing `input.text()` before/after — the same idiom
    `ColorPicker::route` already uses for its custom-entry fields) edits the
    field directly, opens the drop-down if closed, re-filters, and clears any
    highlighted suggestion. This must **never** feed back into rewriting the
    field — it is the source of truth the filter reacts to, not a target of
    it.
  - *Navigating* (`Up`/`Down` while open, or a mouse click/double-click on a
    row) moves a highlight over the **filtered** list and immediately copies
    that row's text into the field (mirrors `ColorPicker`'s grid: "moving the
    highlight *is* selecting, no separate activate step") — a live preview,
    not a commit. `Up`/`Down` are handled by `ComboBox` itself (a small
    clamped index over the filtered list), not delegated to
    `ListBox::handle_event`: a freshly-rebuilt `ListBox` auto-selects row 0
    (its own documented construction behaviour), which would make the first
    `Down` skip straight to row 1 if delegated. `ComboBox` tracks its own
    `highlight: Option<usize>` (reset to `None` on every rebuild) and calls
    `ListBox::select` to keep the visual list in sync — `ListBox` is the
    renderer/scroller of record, not the source of the highlight index.
  - The field always remembers the last text the user actually *typed*
    (updated only by the typing path, never by navigation-preview) so `Esc`
    can revert a preview without erasing real typing (see below).
- **Opening.** Closed → open on: `Down` (also moves the highlight to row 0
  immediately — a single keypress both reveals and previews the top match),
  a click on the drop arrow (opens only — no highlight yet, "browse" rather
  than "navigate"), or any edit that changes the text. A click inside the
  text portion (not the arrow column) never opens it — it just places the
  cursor, like a plain `InputLine`.
- **Closing.**
  - `Enter` while open: closes, keeping whatever text is currently shown
    (typed or previewed). Consumed — it does not also bubble to a dialog's
    default button on the same keypress. `Enter` while closed: `Ignored`,
    bubbling exactly like a bare `InputLine`'s does.
  - `Esc` while open: closes **and reverts** the field to the last
    genuinely-typed text, undoing any navigation preview. Consumed. `Esc`
    while closed: `Ignored`, bubbles (e.g. to a dialog's Cancel).
  - A click on a suggestion row: copies its text and closes (commits
    immediately — unlike a bare `ListBox` row click, which only selects and
    leaves activation to the container, a combo box's whole point is that
    picking a suggestion is the activation).
  - The drop arrow, clicked while already open: closes, keeping current text
    (a plain toggle).
  - Losing focus (`set_focused(false)`, pushed by the owning `Group` when
    focus moves elsewhere): closes, keeping current text — a blur commits,
    it does not revert. `wants_topmost` (ADR 0030, see above) means a click
    *inside* the open drop-down's own footprint now always reaches
    `ComboBox` first, even if a sibling sits underneath it — but a click on
    a focusable sibling that doesn't overlap the drop-down at all (e.g. a
    Label off to the side, or OK/Cancel buttons positioned clear of it) is
    still simply routed to that sibling by ordinary positional dispatch, the
    same as any two non-overlapping views; `ComboBox` never sees that click
    directly. This defensive close is what keeps it in a consistent state
    once that happens.
- **`bounds()`.** One row closed. Open: one row plus
  `list.bounds().height()` — `ListBox`'s own bounds is the single source of
  truth for "how many suggestion rows," set once per rebuild to
  `min(filtered.len(), max_visible)`; `ComboBox` never recomputes it
  independently.
- **Mouse wheel over the open list:** delegated straight to
  `ListBox::handle_event`, which pans without touching its selection (its own
  documented invariant) — `ComboBox`'s `highlight`/field text are untouched
  either.
- **`selected_index()`** is derived, not stored: `items.iter().position(|i|
  i.eq_ignore_ascii_case(text()))`. No separate "committed selection" field
  to keep in sync with the text — the text *is* the value.
- Degrades without panic on an empty `items` list (drop-down never shows
  anything; `Down`/clicking the arrow still toggles `open` harmlessly) and on
  zero filter matches (drop-down opens with zero rows — same footprint as
  closed).
- **`filterable(false)`: type-ahead instead of narrowing.** The drop-down's
  candidate set becomes `items` unconditionally (`filtered_by` ignores its
  needle) — typing never removes a row. What typing *does* do is jump
  `highlight` to the first candidate starting with the field's current text
  (`jump_to_first_match`, `copy_into_field: false` — the field already holds
  exactly what was typed; a search must not overwrite it), or clear the
  visual selection outright if nothing matches (`ListBox::deselect`, added
  for this — a freshly-rebuilt `ListBox` otherwise auto-selects row 0 by its
  own construction default, which would misleadingly show a "match" that
  isn't one). `Up`/`Down`/`Enter`/`Esc`/mouse picking are all unchanged —
  only what typing does to the *list* differs; the field's own edit
  behaviour (still a real `InputLine`) is untouched either way.
- **`select_only(true)`: no free text, ever.** Printable `Char` keys no
  longer reach `InputLine::handle_event` at all; they extend a separate
  `search: String` buffer instead (there being no real typed text to search
  by, unlike the case above), `Backspace` shortens it, and each change
  re-runs `jump_to_first_match` with `copy_into_field: true` — the matched
  item's text *is* the display value, the same idiom `navigate`'s
  arrow-preview already uses, just driven by a search instead of a step.
  `search` resets whenever the drop-down closes (and thus fresh on every
  reopen — no timeout-based reset; see Open questions). A `Paste` is simply
  ignored (free text by another route). No caret is drawn
  (`set_focused` skips forwarding to `input`) — nothing here is directly
  editable, so a blinking insertion point would mislead. A click in the text
  portion opens the drop-down instead of placing a cursor, since there is no
  cursor to place. Orthogonal to `filterable`: combined with
  `filterable(false)` the drop-down shows every candidate and jumps as you
  search (classic native `<select>`); combined with the `filterable`
  default it narrows *and* jumps (a "searchable select") — both still
  produce only values from `items`.

## Collaborators

- [`InputLine`] — the text portion, used as-is; no new capability needed on
  it.
- [`ListBox`] — the suggestion rows, used as-is except for one small
  addition: `deselect()`, clearing the selection outright (distinct from
  `select`, which always lands on a real index) — needed once `filterable`
  and `select_only` introduced a real "nothing matches" case that must
  visually show nothing highlighted, rather than `new`'s construction
  default of row 0. Otherwise unchanged (confirmed while drafting
  this spec: reusing its own `Up`/`Down` handling directly would fight its
  "row 0 auto-selected on construction" behaviour, so `ComboBox` drives
  `select()` itself instead of delegating key events to it — see above).
- `Canvas`/`Buffer`, `geometry`, `theme::{Role, Theme}` — drawing, styled like
  `InputLine` (`Role::Input`) plus a one-column arrow-glyph indicator.
- `view::{View, Context}`, `event` types — same seam as every other control;
  posts no commands of its own (mirrors `ListBox`/`RadioButtons`: the value is
  read via `text()`/`selected_index()` when the host needs it, not announced).
- **`Window::esc_cancels`, incompatible when the drop-down is open.** Found
  during the manual pass: `Window::handle_event` posts `CM_CANCEL` on `Esc`
  *before* its interior ever sees the key when built with
  `.esc_cancels(true)` (ADR 0016, "from the old `Dialog`, folded in
  unchanged") — every other widget hosted this way (`ColorPicker`,
  `FileDialog`, `MessageBox`) is fine with that, since none of them gives
  `Esc` a meaning of its own. `ComboBox` is the first one that does, so a
  dialog embedding it needs to leave `esc_cancels` off (a Cancel button is
  the only way out) if it wants `Esc` to ever reach the combo box's own
  close-the-drop-down handling — otherwise `Esc` always cancels the whole
  dialog, even while just trying to back out of a preview. Not a `ComboBox`
  bug; a real constraint on how it composes with existing `Window` chrome,
  worth knowing before wiring one into a dialog. See the `combo_box` example,
  which leaves `esc_cancels` off for exactly this reason.

## Test plan (write these first)

- **Logic:** case-insensitive prefix filter (empty text matches everything;
  narrows correctly as more is typed); `selected_index` exact-match
  (case-insensitive) vs. free text → `None`; `bounds()` height closed vs. open
  vs. open-with-more-items-than-`max_visible` vs. open-with-zero-matches.
- **Interaction (scripted events):** typing opens the drop-down and narrows
  it, never itself moving `text` beyond what was typed; `Down` when closed
  opens *and* previews row 0 in one keypress; further `Down`/`Up` move the
  preview and clamp at the ends; a second `Down`-then-`Up` sequence returns to
  the same preview it started from (proving `ComboBox`'s own index math, not
  `ListBox`'s, drives this); `Enter` while open closes without reverting and
  is consumed (does not also bubble); `Enter` while closed is `Ignored`
  (bubbles); `Esc` while open reverts to the last typed text and is consumed;
  `Esc` after typing with no navigation is a no-op revert (closes, text
  unchanged); `Esc` while closed is `Ignored`; a click on a suggestion row
  commits its text and closes; a click on the drop arrow opens with no
  preview, and a second click closes without changing text; a click inside
  the text portion only moves the cursor, never toggling `open`; the wheel
  over an open list pans without changing `text`/`highlight`; losing focus
  while open closes without reverting; `wants_topmost()` is `true` iff open,
  and (a `Group`-level test, in `view.rs`) a `Group` containing an
  open-reporting child followed by an ordinary sibling occupying the same
  area draws the former last and hit-tests it first; `filterable(false)`
  never narrows `matches` and jumps `highlight` to the first match without
  touching `text`, leaving nothing highlighted when nothing matches;
  `select_only(true)` ignores printable keys/paste as free text, opens and
  jumps to the first match on the first keystroke, `Backspace` shortens the
  search and re-jumps, the search resets on reopen, and a click in the text
  portion opens rather than placing a cursor; the two flags combine
  (`select_only` + non-`filterable` still shows the full list while
  jumping).
- **Render (snapshot):** closed field with placeholder-empty text and the
  closed-arrow glyph; open with a handful of filtered rows under it, one
  previewed/highlighted; open with more matches than `max_visible` (scrolled,
  via `ListBox`'s own scrolling); open with zero matches (one row tall, same
  as closed).
- **Manual:** a `combo_box` example (or an addition to `dialogs`) on a real
  terminal — typing, arrowing through suggestions, mouse-picking a row,
  Esc-reverting a preview, and a dialog with OK/Cancel buttons positioned
  where a long candidate list's drop-down reaches them, confirming it now
  draws over and still receives clicks meant for it (ADR 0030) rather than
  losing to the buttons.

## Open questions

- No `PageUp`/`PageDown` paging through the suggestion list in v1 (only
  `Up`/`Down`, mirroring `ColorPicker`'s grid not bothering with paging for a
  small 16-swatch grid) — revisit if a real use case wants long lists.
- Resolved: the z-order/click trade-off first drafted here as accepted is
  fixed by ADR 0030 (`View::wants_topmost`) — see "The key design decision"
  above.
- `select_only`'s `search` buffer resets only when the drop-down closes, not
  after a pause — there is no per-keystroke timeout (the framework's
  `Event::Idle` tick could drive one, but that's real added scope for a
  nicety: typing quickly to disambiguate two similarly-prefixed items still
  works today; only a stale, minutes-old buffer from a session that never
  closed would be a real problem, and reopening already clears it). Revisit
  if a real use case wants it.
- Both new flags are per-instance (set at construction via the builders),
  not something a caller can flip on a live `ComboBox` after the fact —
  consistent with `max_visible`'s own shape, and nothing so far has needed
  otherwise.
