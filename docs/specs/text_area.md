# Module spec: `rvision::widgets::text_area`

- **Status:** Done
- **Phase:** roadmap #6 (new widgets)
- **Related ADRs:** 0003 (commands up), 0004 (three-phase dispatch), 0005
  (colour roles), 0006 (grapheme-aware Unicode), 0008 (owner-relative coords +
  `Canvas`), 0010 (focus-aware drawing), 0012 (bracketed paste), 0015 (scroll
  chrome per-view protocol), 0017 (resize propagation per-view protocol)

## Purpose

A scrollable, focusable multi-line text-entry [control](controls.md):
generalizes `InputLine`'s single line the way `ListBox` generalizes a single
choice. Reusable UI furniture, not editor-specific (per `CLAUDE.md`: "no
editor knowledge" means document/buffer/syntax concepts, not a plain
multi-row text field).

Not: syntax highlighting, undo/redo, multiple cursors, clipboard
copy/cut wiring, mouse drag-select, or a word-wrap on/off toggle (it always
reflows) — all out of scope for this landing (see Open questions).

## Public interface

```rust
/// Where `set_text`/`with_text` place the cursor in the new text.
pub enum CursorPosition { Start, End }

pub struct TextArea {
    bounds, text: String,
    cursor: usize,                        // grapheme idx over the whole text
    /// The other end of an in-progress selection, if any (Shift+navigation).
    selection_anchor: Option<usize>,
    top: usize,                           // display-line scroll
    focused, overtype, style, ..
}
impl TextArea {
    pub fn new(bounds: Rect, theme: &Theme) -> Self;

    // Cursor lands at the Start by default — the more common affordance
    // (unlike InputLine, which always lands at the End) — with an explicit
    // variant when a caller wants the other end.
    pub fn with_text(self, text: &str) -> Self;                          // Start
    pub fn with_text_at(self, text: &str, at: CursorPosition) -> Self;
    pub fn set_text(&mut self, text: &str);                              // Start
    pub fn set_text_at(&mut self, text: &str, at: CursorPosition);

    pub fn text(&self) -> &str;
    /// The selected text, if any (`selection_anchor` set and != `cursor`).
    pub fn selected_text(&self) -> Option<&str>;
}
impl View for TextArea {
    // focusable; multi-line edit; scroll_metrics/set_scroll (ADR 0015,
    // vertical only, in *display* lines); set_bounds reflows (ADR 0017).
}
```

`TextArea` reflows with its **own** private `reflow` function — not
`wrap::wrap`/`wrap_with_offsets`. `wrap` deliberately collapses whitespace
runs to a single separator, which is correct for read-only prose
(`HelpPane`, `MessageBox`) but wrong for an editable buffer: a user typing
two spaces would watch one vanish, and the displayed text would stop being
byte-for-byte what's stored. `reflow` shares only [`wrap::word_offsets`]
(word boundaries with byte offsets) with `wrap.rs`; everything about how it
packs and slices is its own:

```rust
/// Breaks `text` into display lines of at most `width` columns, preserving
/// every byte verbatim — unlike `wrap::wrap`, no whitespace run is ever
/// collapsed *or dropped*: a gap (interior or trailing) that doesn't fit is
/// split across continuation lines, never erased. Interior gaps count at
/// their *true* display width when deciding whether the next word fits —
/// real columns, not a collapsed stand-in — so layout matches what a
/// monospace terminal actually renders. A lone word (or leading
/// indentation glued to it) wider than `width` still overflows rather than
/// splitting — a word is the only atomic thing here, mirroring `wrap`'s
/// own rule for it.
fn reflow(text: &str, width: u16) -> Vec<(usize, String)>;
```

## Reflow algorithm (`reflow`)

For each hard (`'\n'`-separated) line, find its word ranges via
`wrap::word_offsets`, then walk **gap, word, gap, word, ..., gap** left to
right (the last gap — the "tail" — has no word after it; a hard line with no
words at all is just that one gap). Two kinds of token, two rules:

- **A word is atomic.** The first word placed on a fresh display line goes
  down unconditionally, even if it alone exceeds `width` (overflow allowed,
  never split). Any later word is placed if `cur_width + word width <=
  width`; otherwise the line is flushed as the verbatim slice
  `text[line_start..word_start]` and a new display line starts at the
  word's own offset.
- **A gap is never atomic and never elided** (`place_gap`). As much of it as
  still fits the current line joins it, one grapheme at a time; once it
  wouldn't, the line is flushed and the rest continues on its own
  continuation line(s), repeating until the gap is exhausted — whether it's
  an interior gap (between two words) or the trailing tail makes no
  difference to this rule.

Because every flushed/continued line is always a **verbatim slice** of
`text` (never a rebuilt string) and gaps are only ever split, never erased,
every single byte of `text` ends up in exactly one display line — nothing
is ever dropped from view.

**This wasn't the first design.** An earlier version elided an interior gap
*wholesale* once it stopped fitting, mirroring `wrap::wrap`'s own
single-space-separator convention: cheap, and it matches what a `wrap`\-based
prose viewer already does. It turned out wrong for two compounding reasons.
First, unlike `wrap`'s own collapsed single column, an interior gap here can
be arbitrarily long (multiple typed spaces), so "elide it" could silently
discard far more than one column at once. Second, and the one that actually
surfaced as a bug: a cursor sitting inside an elided gap has **no display
column to map to at all** — [`display_pos`](#behaviour--invariants) falls
back to the end of the *previous* line for every position inside the gap,
so typing more whitespace there did nothing visible, no matter how much was
typed, until a following word finally forced a new wrap and the cursor
jumped all at once. Splitting instead of eliding removes the whole class of
bug: every byte has a real column, so there's nothing left to fall back
from.

## Behaviour & invariants

- **Storage.** One flat `String` with real `'\n'`s as hard breaks — no
  per-line `Vec`, so `wrap.rs`'s existing hard-break handling applies
  unmodified. Cursor is a grapheme index over the *whole* text (same
  convention as `InputLine`), with `'\n'` an ordinary one-grapheme step —
  Left/Right cross line boundaries with no special-casing.
- **Reflow, never horizontal scroll.** Every logical line is reflowed to
  `bounds.width() - 1` (`wrap_width`, never the box's full width — see the
  caret invariant below) via the whitespace-preserving `reflow` above — the
  opposite trade from `InputLine`, which never wraps and instead scrolls
  horizontally. Cached as `lines: Vec<(usize, String)>` (offset, display
  line — each a verbatim slice, never rebuilt/collapsed), rebuilt by a
  private `relayout()` whenever text or width changes (same
  cache-on-width-change shape as `HelpPane::layout`).
- **Vertical motion.** Up/Down move the cursor by one *display* line,
  preserving display column as closely as possible (clamped to the target
  line's length) — map cursor → (display_row, col) via `lines`, then back to
  a grapheme index on the target row. PageUp/PageDown move by a screenful of
  display lines, same shape as `ListBox::move_by`.
- **Line vs. document motion.** Home/End go to the start/end of the current
  *display* line only, not the logical paragraph or the whole buffer
  (conventional word-wrap-editor behaviour: Home/End are line-scoped).
  `Ctrl+Home`/`Ctrl+End` jump to the very start/end of the whole text
  (grapheme `0`/`len`), scrolling `top` to match.
- **Word motion.** `Ctrl+Left`/`Ctrl+Right` move the cursor to the previous/
  next word boundary — the GNU readline `backward-word`/`forward-word`
  convention (a grapheme counts as a word character by its first `char`
  being alphanumeric or `_`; everything else, punctuation included, is a
  plain separator), not full UAX #29 segmentation. Left skips any separator
  run immediately to the left then lands at the start of the word run before
  it; Right skips the current word run then any separator run, landing at
  the start of the next word. Operates over the flat `text` (crosses line
  breaks like plain Left/Right does). **Shared with `InputLine`**, which
  gains the same binding (see [`controls.md`](controls.md)) — the
  `word_left`/`word_right` helpers live in `text_edit.rs` alongside the
  grapheme ops for exactly that reason.
- **Selection (Shift+navigation).** Holding Shift with any navigation key
  (`Left`/`Right`/`Up`/`Down`/`Home`/`End`/`Ctrl` variants/`PageUp`/
  `PageDown`) extends a selection: if `selection_anchor` is `None` it is set
  to the cursor's position *before* the move, then the cursor moves exactly
  as it would unshifted. The selected range is always
  `min(anchor, cursor)..max(anchor, cursor)` regardless of travel direction.
  A navigation key pressed **without** Shift clears `selection_anchor`
  (collapses the selection) and then moves normally from the current
  cursor — a deliberate simplification versus some GUI editors' "collapse to
  the near edge of the old selection first" behaviour; flagged in Open
  questions in case it surprises. A mouse click always clears the selection
  and places the cursor (no drag-select yet). Typing a printable character
  (or `Enter`) while a selection is active deletes the selected range first,
  then inserts, same as `Backspace`/`Delete` deleting the whole selection
  instead of one grapheme — the minimum for a selection to be *usable*, not
  just visible. Selected cells draw in `Role::Selection` (same role
  `ListBox` uses for its own highlight) regardless of focus. **Shared with
  `InputLine`** (see [`controls.md`](controls.md)): the same anchor/collapse/
  replace-on-type rules apply there too, minus the vertical/multi-line
  variants that don't exist on a single line.
- **Editing.** Enter inserts `'\n'` (just another `insert`). Backspace /
  Delete / insert / overwrite are grapheme ops identical to `InputLine`'s,
  factored into a shared free-function helper (`src/widgets/text_edit.rs`) so
  both controls call the same `insert`/`overwrite`/`backspace`/`delete`/
  `byte_to_grapheme`/`grapheme_starts` — the roadmap's own call to share
  "grapheme-based cursor advance, the insert/overtype toggle, bracketed-paste
  handling" as free functions rather than a merged type (precedent: menu
  cascade/hit-test helpers in `menu.rs`). Overwrite at a `'\n'` falls back to
  insert (never deletes the line break) — the same "fall back to insert"
  rule `InputLine` already uses at true end-of-text.
- **Insert/overtype.** `Insert` toggles overtype exactly like `InputLine`
  (default off); the caret's attribute follows the same underline/reverse
  convention.
- **Paste (ADR 0012).** Inserts every non-control character *including*
  `'\n'` — unlike `InputLine`, which flattens newlines. The one deliberate
  behavioural difference between the two paste call sites, expressed as a
  `strip_newlines: bool` parameter on the shared helper, not two copies.
- **Tab** bubbles (dialog focus-cycles) rather than inserting a literal tab —
  matches `InputLine`'s convention.
- **Scrolling (ADR 0015).** Vertical only, over *display* lines, via
  `scroll_metrics`/`set_scroll` exactly like `ListBox` — no bar of its own; a
  host hosts one. `total`/`visible`/`pos` count display lines, not logical
  lines.
- **Resize (ADR 0017).** `set_bounds` on a width change calls `relayout()`
  (recomputes wrap, re-maps the cursor's display position, clamps `top`); on
  a height-only change it just clamps `top`, like `ListBox::set_bounds`.
- **The caret always has a column.** `TextArea` reflows to
  `bounds.width() - 1`, not the box's full width — permanently reserving
  its last column. A display line can therefore never reach the true right
  edge, so the caret at true end-of-line always lands on a real, visible
  column; there is nothing left to special-case. (An earlier version of
  this control reflowed to the full width and rolled/clamped the caret
  reactively when a line happened to land exactly on the edge — replaced
  outright once the reservation made that situation impossible to reach.)
- Degrades without panic for empty text, zero-size bounds, a cursor at the
  very end, and a single over-long word or gap (`reflow` allows that to
  overflow; the canvas clips it, same as `ListBox`/`InputLine`'s existing
  overflow handling).

## Collaborators

- `Canvas`/`Buffer`, `geometry`, `cell::Cell`, `theme::{Role, Theme}`,
  `color::Style`.
- `view::{View, Context, ScrollMetrics, AxisMetrics}`, `event` types.
- `wrap::word_offsets` (made `pub(crate)`) for word boundaries, feeding this
  module's own `reflow`; the `widgets::text_edit` free functions (shared with
  `InputLine`) for grapheme editing and word motion.
- `unicode_width::UnicodeWidthStr` for measuring word/gap display width
  during reflow (same crate `wrap.rs` already uses).
- Posts no `Command` of its own (same as `InputLine`); never references
  `ListBox`/`ScrollBar` directly — a host hosts its scroll bar the way
  `FileDialog` hosts `ListBox`'s.

## Test plan (write these first)

- **Logic:** insert/backspace/delete across a line break; Enter splits a
  line; Left/Right cross the boundary; Up/Down preserve column and clamp on
  shorter lines; Home/End confined to the display line; Ctrl+Home/Ctrl+End
  jump to the whole text's start/end; Ctrl+Left/Ctrl+Right land on word
  boundaries (across multiple words, across a line break, at start/end of
  text); PageUp/PageDown move by a screenful; overtype toggle + fallback at
  a line break and at true end; `set_text`/`with_text` default cursor to
  Start, `_at(.., End)` variants land at the end; `reflow` preserves exact
  whitespace (typing multiple spaces keeps all of them; `text()` round-trips
  byte-for-byte through a resize/rewrap) and never drops a byte from
  display — a gap that doesn't fit splits across continuation lines rather
  than disappearing.
- **Selection logic:** Shift+arrow sets the anchor once and extends;
  reversing direction shrinks/flips the range correctly; a bare arrow
  (no Shift) collapses it; typing/Enter/Backspace/Delete with an active
  selection replaces/removes the whole range; `selected_text` reflects the
  current range in document order regardless of which end the cursor is at.
- **Render (snapshot):** multi-line wrapped content with caret; caret
  underline (insert) vs reverse block (overtype); a scrolled view (`top >
  0`); a selection spanning a wrapped line boundary drawn in
  `Role::Selection`.
- **Interaction (scripted events):** typing across a wrap boundary; arrow
  up/down through a reflowed paragraph; bracketed paste keeps newlines;
  `scroll_metrics`/`set_scroll` parity with `ListBox`'s own tests; a
  Shift+End … Shift+Down … Backspace sequence deletes the expected span.
- **Manual:** a `text_area` example (or an added field in `dialogs.rs`);
  confirm reflow-on-resize and a hosted scroll bar in a real terminal.

## Open questions

- Undo/redo: out of scope for this landing (`InputLine` doesn't have it
  either).
- Clipboard copy/cut: not wired up here. `Ctrl+C`/`Ctrl+X` posting
  `selected_text()` through `Application::set_clipboard` (OSC 52) is a
  natural follow-up but a separate concern (command routing, not this
  control's own state) — left for a future item.
- Un-shifted arrow-away-from-a-selection collapses from the *current*
  cursor position rather than snapping to the selection's near edge first
  (some GUI editors do the latter) — simpler, may feel slightly off if a
  future manual pass disagrees.
- Mouse drag-select and Shift+click-to-extend: not implemented; a click
  always collapses to a plain cursor placement.
