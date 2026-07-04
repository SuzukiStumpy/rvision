# ADR 0027 — Generic mouse capture for continuous drag interactions

- **Status:** Accepted
- **Date:** 2026-07-04

## Context

Dragging a scroll bar's **thumb** did nothing anywhere in the framework —
found while manually verifying an unrelated `FileDialog` click-selection fix.
Tracing it: `Desktop`'s `DragKind` (ADR 0016) only has `Move` and `Resize`;
comments scattered across `scroll_bar.rs`/`window.rs`/`file_dialog.rs`
("dragging the thumb rides on window drag infra (Phase 9d)") were a forward
reference to a phase that was never actually built.

Scroll bars are hosted in two different places in this codebase:

- **On a `Window`'s own border**, generically, for whatever its sole interior
  reports via `scroll_metrics` (ADR 0015) — e.g. `TextArea`'s.
- **Inside a widget's own interior**, self-drawn and self-hit-tested by that
  widget — `FileDialog`'s embedded list, `HelpPane`'s own two bars.

`Desktop`'s existing drag-session machinery only recognises grab points on
the *window* itself (title row, resize corner) — it has no visibility into
either kind of bar, since the second kind isn't even part of window chrome.
A first design pass taught `Desktop` about `Window`'s own bars directly
(a third `DragKind` variant, two new `Window` accessor methods) while giving
`FileDialog`/`HelpPane` a separate, weaker local-only mechanism — which loses
the drag if the pointer leaves the *window* entirely, since nothing captures
the mouse at the `Desktop` level for those two. Asked directly, "could
`Desktop` capture the click and bubble it down to the widget instead" — yes,
and doing so removes the asymmetry and closes that gap for all three.

## Decision

**A generic mouse-capture request on `Context`**, mirroring a pattern the
codebase already has for exactly this shape of problem: ADR 0019's
`Context::open_context_menu`/`take_context_menu_request` ("a view hosted on a
`Desktop` can't open another window itself... this is the standard signal for
'please do X for me' that a hosting owner intercepts"). Capture needs no
position data, so it's simpler — a plain flag:

```rust
impl<'a> Context<'a> {
    pub fn request_mouse_capture(&mut self);
    pub fn take_mouse_capture_request(&mut self) -> bool;
}
```

**`Desktop` gains a second, orthogonal session concept: `captured: Option<WindowId>`**,
alongside the existing `drag: Option<DragSession>`. After ordinary positional
dispatch delivers an event to a window, `Desktop` checks
`ctx.take_mouse_capture_request()`; if set, every subsequent mouse event
(regardless of pointer position — the positional lookup is skipped entirely,
the same way it already is while `drag` is `Some`) is forwarded straight to
that window until `Up`. `Desktop` needs **no knowledge of `ScrollBar` at
all** — it is purely "someone asked to keep receiving events."

`Move`/`Resize` deliberately keep their existing, separate mechanism rather
than being rebuilt on top of this: window arrangement is a genuinely
`Desktop`-owned concept (ADR 0016) — `Desktop` computes the new bounds
itself. Mouse capture is the opposite shape: `Desktop` computes nothing, it
only forwards; the receiving view keeps its own drag state and does the
actual scrolling math.

**Each of the three bar hosts (`Window`, `FileDialog`, `HelpPane`) gets an
identical, fully local shape** — no shared trait or helper type, since each
already privately owns its own bar geometry and an existing *relative* scroll
primitive (`Window::nudge_scroll`, `FileDialog::scroll_list_by`,
`HelpPane::scroll_by`/`scroll_h_by`), reused as-is:

- A `Down` hit on a bar's `Thumb` (via `ScrollBar::hit`, whose cross-axis
  containment check landed in the same session as this fix) sets a local
  "currently dragging this axis" flag and calls `ctx.request_mouse_capture()`
  — no scroll happens yet, just anchoring.
- While that flag is set, `Drag` recomputes the bar fresh, calls
  `ScrollBar::pos_at(point)` for the absolute target position (deliberately
  *not* cross-axis-checked, unlike `hit` — a drag should keep tracking even
  if the pointer wanders off the bar's exact column/row, matching real
  scrollbar UX, and `pos_at` already clamps against a point far outside the
  track), diffs it against the current scroll position, and applies the
  delta through the existing relative primitive.
- `Up` clears the flag.

## Consequences

- `Desktop` stays exactly as ignorant of `ScrollBar` as it was before — any
  *future* continuous-drag interaction (nothing concrete planned) can reuse
  `request_mouse_capture`/`take_mouse_capture_request` without `Desktop`
  needing a new case for it.
- All three hosts get full robustness "for free": a drag that strays outside
  the *window's* own bounds (not just the bar's sub-region within it) keeps
  working, because capture bypasses positional dispatch entirely at the
  `Desktop` level. This was the specific gap the first design pass would have
  left open for `FileDialog`/`HelpPane`.
- Three near-identical small implementations (one per host) rather than one
  shared abstraction. Accepted: each host's bar geometry is already
  independently duplicated today (`Window::vertical_scroll_bar`,
  `FileDialog::list_scroll_bar`, `HelpPane`'s inline construction), so this
  follows the existing precedent rather than introducing a new one; a shared
  "drag-tracking mixin" would need either a trait `Window`/`FileDialog`/
  `HelpPane` all implement (more machinery than three ~15-line blocks
  warrant) or free functions with an awkward number of parameters (bar,
  current position, apply-delta closure) for what each already expresses
  clearly inline.
- Dragging past the *screen's* own edge isn't reachable in a terminal (the
  pointer can't leave the viewport) and isn't specially handled — not a gap
  in practice.

## Alternatives considered

- **Teach `Desktop` about `ScrollBar`/thumbs directly** (the first design
  pass): a third `DragKind::ScrollThumb(bool)`, plus `Window` methods
  `Desktop` calls to hit-test and continue a drag. Works for `Window`-hosted
  bars but has no way to reach `FileDialog`/`HelpPane`'s internally-hosted
  ones at all (they aren't part of window chrome `Desktop` can see), leaving
  the two-tier asymmetry and the "leaves the window" gap for those two.
  Rejected once the generic capture request proved simpler *and* more
  complete.
- **A `View` trait method** (e.g. `fn drag_capture_at(&self, local: Point) -> bool`)
  that `Desktop` could call generically before dispatch, rather than a
  `Context` request made *during* normal dispatch. Rejected: it would need
  `Desktop` to probe an arbitrary, unknown-depth interior tree for "does
  something here want to capture," duplicating the hit-testing every host
  already does in its own `handle_event`; the `Context`-request idiom lets
  the view that already recognized the hit (mid-dispatch) simply say so,
  exactly like a context-menu request already does.
- **Rebuild `Move`/`Resize` on the same generic capture mechanism.** Would
  fully unify session handling under one concept. Rejected as unnecessary
  scope: that mechanism already works and is well-tested; changing it wasn't
  asked for and isn't warranted by this fix.
