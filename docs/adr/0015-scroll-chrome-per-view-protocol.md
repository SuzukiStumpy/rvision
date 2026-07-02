# ADR 0015 — Scroll chrome is a per-view protocol

- **Status:** Accepted
- **Date:** 2026-07-02

## Context

Content that scrolls (a list, a document, a help page) can currently only
manage its own scroll bar entirely by itself. [`ListBox`] builds and draws its
own [`ScrollBar`] internally, computing metrics (`total`, `visible`, `pos`) and
owning hit-testing for it inside its own `handle_event`. [`HelpPane`] does the
same independently, for both axes. `edit`'s own editor window hand-built an
equivalent mechanism *again*, bespoke, entirely outside `rvision`
(`ScrollMetrics::needs_vertical`/`needs_horizontal` driving both drawing and
mouse hit-testing along its window frame) — because `rvision`'s [`Window`]
gives an interior no way to report what it needs, and `Window` has no way to
draw scroll chrome for content it doesn't own concretely (ADR 0003: an owner
never reaches into a child's fields, only the `View` trait's own methods).

That's the same shape of problem [`drop_shadow`](crate::view::View::drop_shadow)
(ADR 0011) and [`set_focused`](crate::view::View::set_focused) (ADR 0010)
already solved for two other cross-cutting concerns: something a *child* needs
to report or receive that the *owner* draws or manages on its behalf, without
the owner ever downcasting into the child's concrete type. Scrolling needs the
same shape: a child that wants its scroll chrome hosted by whoever composes it
(a `Window`, a `Dialog`, a `FileDialog`) should be able to opt in, without every
future scrollable widget re-deriving `ScrollBar`-hosting from scratch — and a
widget that would rather stay self-contained should be free to keep doing that.

This groundwork is also the direct unblock for the larger SDI/MDI windowing
decision (ADR 0016): a `Window` interior — a future `TextEdit`, a
`HelpWindow`'s topic pane — needs a way to say "I have more content than fits;
here's my viewport," and the window's own border is the natural place for that
chrome to live, exactly as `edit`'s bespoke editor window already proved works.

## Decision

Add two defaulted `View` trait methods, in the same shape as the two protocols
already on the trait:

```rust
/// What a scrollable view needs scrolled, per axis (ADR 0015).
pub struct ScrollMetrics {
    pub horizontal: Option<AxisMetrics>,
    pub vertical: Option<AxisMetrics>,
}

/// One axis's scroll range: `total` units, `visible` of them on screen at
/// once, the first shown being `pos`.
pub struct AxisMetrics {
    pub total: usize,
    pub visible: usize,
    pub pos: usize,
}

pub trait View {
    // ...existing methods...

    /// What this view needs scrolled right now, or `None` if nothing does
    /// (ADR 0015). Queried every draw, like `drop_shadow`.
    fn scroll_metrics(&self) -> Option<ScrollMetrics> { None }

    /// Pushes a new scroll position an owner's chrome computed on this
    /// view's behalf (ADR 0015), mirroring `set_focused`'s push shape.
    fn set_scroll(&mut self, offset: Point) { let _ = offset; }
}
```

A composing owner (`Window` first; anything else that lays a child out in a
fixed area — `Dialog`, `FileDialog` — can do the same) checks `scroll_metrics`
each draw. If it returns `Some`, the owner reserves and draws a border
`ScrollBar` per axis that needs one (reusing the existing widget unchanged),
routes clicks/drags on that bar through `ScrollBar::hit`, and calls
`set_scroll` with the result. If it returns `None`, the owner draws no chrome
there and the child is free to scroll itself internally by whatever means it
likes.

`draw`'s signature is untouched — metrics are a separate query, not threaded
through the draw call, for the same reason ADR 0010 gave for rejecting a
`DrawContext`: only scrollable content needs this, and a defaulted trait method
costs nothing for the rest.

**Migration, as the proof.** [`ListBox`] and [`FileDialog`] are updated in the
same change to *prove* the protocol rather than add it as a purely theoretical
seam:

- `ListBox` drops its internally-owned `ScrollBar` and hit-testing, and
  implements `scroll_metrics`/`set_scroll` instead (`total = items.len()`,
  `visible = rows()`, `pos = top`, vertical axis only).
- `FileDialog` becomes the delegating *host* for its embedded list — querying
  `scroll_metrics`, drawing and hit-testing a `ScrollBar` in the column it
  already reserves, and pushing clicks back through `set_scroll` — showing the
  pattern working through a second, non-`Window` owner, not just the one call
  site the decision was designed around.

## Consequences

- One scroll-chrome implementation instead of three (`ListBox`'s, `HelpPane`'s
  eventual generalisation, and `edit`'s bespoke editor-window version) — new
  scrollable widgets opt in for free instead of re-deriving `ScrollBar` hosting
  each time.
- `HelpPane` is deliberately **not** migrated here — it manages both axes
  internally today and works fine; nothing about this decision forces a widget
  that's happy self-contained to change. The protocol is opt-in both ways.
- After migration, a bare `ListBox` used outside any delegating owner no longer
  has a scroll-bar fallback of its own — it relies on something hosting it. No
  current call site does this (both existing uses, standalone and inside
  `FileDialog`, gain a host), but it's a real trade-off to keep in mind for a
  future bare use.
- Slightly more work for any future owner that wants to host scrollable
  children (it must add the query-and-host loop), in exchange for paying that
  once per *owner type* rather than once per *scrollable widget*.

## Alternatives considered

- **Leave `ListBox`/`HelpPane` self-contained; add the protocol only for future
  widgets.** Lower risk, but proves nothing — the point of doing this now is
  validating the seam against a real, already-working widget before the far
  larger windowing decision (ADR 0016) leans on it.
- **Thread scroll state through `draw` via a context object.** Same objection
  ADR 0010 raised for focus: rewrites every `draw` in the framework for a need
  only some views have.
- **Let `Window` downcast into known scrollable widget types.** Rejected
  outright — breaks ADR 0003's no-downcast rule, for the same reason the whole
  `edit`/`rvision` windowing split exists in the first place (ADR 0016).
