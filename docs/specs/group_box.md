# Module spec: `rvision::widgets::group_box`

- **Status:** Done. Manual pass done: the `dialogs` example's Settings
  dialog wraps its "Line endings" `RadioButtons` in a `GroupBox` — run in a
  real terminal (tmux), the border/title drew correctly and `Tab`/arrows/
  mouse drove the radio group exactly as before. That same pass surfaced a
  real gap, fixed during it rather than just noted: once `Tab` reached the
  group box it could never leave, fixed by ADR 0031
  (`Group::non_wrapping`) — see Behaviour below.
- **Phase:** unscheduled (roadmap backlog #6, "New widgets")
- **Related ADRs:** 0003 (commands up / broadcasts down, views never
  reference siblings), 0004 (three-phase dispatch), 0008 (owner-relative
  coordinates + `Canvas`), 0010 (focus-aware controls), 0016 (established the
  "bordered box wrapping an interior" composition `Window` uses, reused here
  unchanged), 0031 (non-wrapping nested focus groups — this widget's interior
  `Group` is the motivating case)

## Purpose

A titled, bordered box for visually grouping related controls — e.g. a set
of `RadioButtons` under "Alignment:" — distinct from
[`Frame`](widgets.md) (window-chrome border drawing, with close/zoom/help
glyphs, not an independent `View`) and from [`Window`](window.md) (a whole
floating/dockable box with move/resize/close policy). `GroupBox` is a plain,
static, in-dialog control: no policy, no glyphs, nothing floating — just a
single-line border with an embedded title, owning the children placed inside
it.

## Public interface

```rust
pub struct GroupBox {
    bounds: Rect,
    title: String,
    style: Style,
    interior_fill: Cell,
    interior: Group,
}

impl GroupBox {
    /// Creates a group box at `bounds` titled `title`, owning `children`
    /// (laid out in interior-local coordinates: `(0, 0)` sits one cell in
    /// from the border, mirroring `Window::interior_bounds`). Border and
    /// title both resolve `Role::DialogBackground` — the same role
    /// `RadioButtons`/`CheckBox`/`Label` already use for their own body text,
    /// so a group box matches whatever dialog it sits in without a
    /// dedicated theme role.
    pub fn new(bounds: Rect, title: &str, children: Vec<Box<dyn View>>, theme: &Theme) -> Self;

    /// The interior rectangle in the box's own local coordinates: the whole
    /// box inset by one cell on every side (the border). Collapses to empty
    /// for a box too small to have one.
    pub fn interior_bounds(&self) -> Rect;
}

impl View for GroupBox {
    // bounds(): `self.bounds`, fixed at construction (mirrors `Group`, which
    // has no public resize either — see Behaviour).
    // draw(): border + left-aligned embedded title, then the filled
    // interior with `children` drawn over it.
    // handle_event(): positional mouse translated into the interior the same
    // way `Window` translates into its own; everything else forwarded
    // straight to the interior `Group` (focused phase / broadcast phase are
    // not positional, so no translation is needed there).
    // focusable() -> self.interior.focusable()
    // set_focused(focused) -> self.interior.set_focused(focused)
    // valid(command, ctx) -> self.interior.valid(command, ctx)
}
```

## Behaviour & invariants

- **Composition, not reinvention.** `GroupBox` owns a real
  [`Group`](view.md) for its children — focus traversal (Tab/Shift-Tab),
  positional/focused/broadcast dispatch, and the `valid` veto fan-out are all
  `Group`'s existing logic, not reimplemented. `GroupBox`'s own code is
  exactly the border/title drawing plus the one level of coordinate
  translation between its own local frame and the interior `Group`'s —
  the same shape `Window` already uses for its (arbitrary, single) interior
  view, specialised to an interior that is always a `Group` of children
  rather than any `View`.
- **Border + title.** A single-line box (no active/inactive distinction —
  a group box is never a window, so there is nothing to double the border
  for). The title is embedded on the top edge, **left-aligned** starting
  immediately after the top-left corner (`┌ Title ────┐`), padded with one
  space either side, truncated to fit — unlike `Frame`'s centred title,
  which sits under a title *bar* long enough that centring reads well; a
  narrow group box reads better with the label anchored at a fixed spot.
  An empty title draws a plain, unbroken box. Degrades without panic for
  areas narrower or shorter than 2 cells (draws nothing, same threshold as
  `Frame`).
- **Interior fill.** Before drawing `children`, `GroupBox` fills its
  interior with its own background style — mirrors `Window` (not the bare
  leaf-control precedent of `RadioButtons`/`CheckBox`, which each fill only
  their own bounds): `GroupBox` is a "border + interior" composite that may
  be placed directly on a bare `Group`/`Desktop` with nothing behind it
  pre-filling on its behalf, so it fills itself rather than assuming a
  `Window` ancestor already did.
- **No resize propagation.** `GroupBox` does not override `View::set_bounds`
  (the default no-op). This mirrors `Group` itself, which also has no public
  resize: a `Group`'s children sit at fixed relative positions, so
  repropagating a new size wouldn't reposition them anyway without also
  re-laying-out every child — a bigger feature this widget doesn't need
  (dialogs are sized once at construction). Revisit only if a real use case
  wants a resizable group box.
- Not focusable in its own right — `focusable()`/`set_focused`/`valid` all
  delegate straight to the interior `Group`, exactly the way a plain `Group`
  reports `self.focused.is_some()` and forwards `set_focused` to whichever
  child currently holds it (ADR 0010). A `GroupBox` with no focusable
  children is correctly reported as not focusable, so an owning `Group`'s
  own Tab traversal skips over it.
- **Tab escapes once exhausted, rather than wrapping locally (ADR 0031).**
  The interior `Group` is built `.non_wrapping()`. Found during the manual
  pass: an ordinary (wrapping) interior `Group` with only one focusable child
  — this widget's whole motivating case, a lone `RadioButtons` — treats every
  `Tab` as "wrap back to my one child" and consumes it unconditionally, so
  focus could never leave the box to reach a sibling laid out after it (an
  OK/Cancel button). `.non_wrapping()` makes a *boundary* Tab/Shift-Tab
  report `Ignored` instead, letting it bubble to whatever `Group` owns this
  `GroupBox`, which then advances its own focus past the box exactly as if
  it had ignored the key itself. A box with more than one focusable child
  still Tab-cycles through all of them internally first; only the boundary
  case escapes. See ADR 0031 for the general mechanism (not `GroupBox`-specific).

## Collaborators

- [`Group`](view.md) — the interior; owns/dispatches to `children` unchanged.
- `Canvas`/`Buffer`, `geometry`, `cell::Cell`, `theme::{Role, Theme}`,
  `color::Style` — drawing, styled like `RadioButtons`/`CheckBox`
  (`Role::DialogBackground`).
- `view::{View, Context}`, `event` types — same seam as every other
  container; posts no commands of its own.
- Reuses `Window`'s established "translate into an inset interior" idiom
  (ADR 0016) rather than introducing a new one.

## Test plan (write these first)

- **Logic:** `interior_bounds` inset-by-one on every side; collapses to
  empty for a too-small box (width or height under 3).
- **Render (snapshot):** a titled box around a couple of children (e.g.
  `RadioButtons`), title embedded left-aligned on the top border; an empty
  title draws a plain unbroken box; a too-narrow/short box degrades without
  panic; a long title truncates to fit.
- **Interaction (scripted events):** a click inside the interior reaches the
  right child at correctly-translated local coordinates; a click on the
  border itself (not the interior) is ignored; keys reach the focused child;
  Tab/Shift-Tab cycle focus among the children; `focusable()` reflects
  whether the interior currently has a focusable child; `set_focused`
  forwards to whichever interior child holds focus; a child's posted command
  bubbles out; a broadcast reaches every child; `valid` fans out to every
  child the same way `Group::valid` does. **Tab escape (ADR 0031):** a
  `GroupBox` with a single focusable child, embedded as one child of an
  *outer* `Group` alongside a later sibling button, lets `Tab` reach that
  sibling instead of being swallowed; `Shift-Tab` returns to the box.
- **Manual:** done — the `dialogs` example's Settings dialog wraps its
  "Line endings" `RadioButtons` in a `GroupBox` titled "Line endings" (in
  place of the plain `Label` it used before). Run in a real terminal (tmux):
  border/title drew correctly, arrows still moved the radio selection, and
  (once ADR 0031 landed) `Tab` correctly reached the OK/Cancel buttons past
  the box instead of getting stuck on it.

## Open questions

- No focus-ring or distinct styling on the box itself when a child inside it
  is focused — TurboVision-style group boxes are purely a passive label/
  border; revisit only if a real design wants the box to react visually.
- Re-entering the box via Tab/Shift-Tab lands on whichever interior child was
  last focused, not necessarily that direction's "natural" end (first child
  forward, last child backward) — see ADR 0031's own open question; not
  addressed here either.
