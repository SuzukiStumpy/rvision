# ADR 0017 — Resize propagation is a per-view protocol

- **Status:** Accepted
- **Date:** 2026-07-02

## Context

[`Window`] hosts a boxed interior (`Box<dyn View>`) but never tells it about a
size change. `Window::set_bounds` (a drag-resize session, ADR 0016) and
`Window::toggle_zoom` (maximise/restore) both mutate only `self.bounds`; the
interior finds out its area changed, if at all, by the differently-sized
`Canvas` it's handed on the next `draw` — but `draw` takes `&self`, so a
widget whose layout is a *cached* function of its size (wrapped lines, a
scrolled-into-view offset) has no way to recompute that cache from inside
`draw`. Every interior in the tree today either ignores its size entirely
(static content) or is hosted in a `Window` locked `.resizable(false)`
([`FileDialog`], [`MessageBox`]), so the gap has never mattered — until a
`HelpWindow` (`docs/specs/desktop.md`'s open question, `roadmap.md`) wants to
compose [`ListBox`] and [`HelpPane`] into a *resizable* two-pane interior.
`HelpPane` already carries a tested `set_bounds` (added for its own resizing
when it was a bare, directly-driven widget) that does exactly the right
thing — relayout its wrapped lines and clamp scroll — but nothing has ever
called it when it's hosted inside a `Window`.

This is the same shape of problem [`drop_shadow`] (ADR 0011) and
[`scroll_metrics`]/[`set_scroll`] (ADR 0015) already solved: something a
*child* needs to receive that only the *owner* can supply (because the owner
holds the concrete size, and ADR 0003 forbids downcasting into the child to
push it directly).

## Decision

Add one more defaulted `View` trait method, in the same shape as the two
protocols already on the trait:

```rust
pub trait View {
    // ...existing methods...

    /// Tells this view its area changed — a resize (drag or terminal resize
    /// cascading down through an owner), a zoom/restore, or any other
    /// repositioning an owner decides to propagate (ADR 0017). The default is
    /// a no-op: only a view whose own layout is a size-dependent, cached
    /// function needs to override it (e.g. `HelpPane`, which already does).
    fn set_bounds(&mut self, bounds: Rect) { let _ = bounds; }
}
```

`Window::set_bounds` and `Window::toggle_zoom` both call
`self.interior.set_bounds(self.interior_bounds())` immediately after updating
their own `bounds` — one call site each, covering every way a `Window`'s size
can change (drag-resize, zoom, and a direct `set_bounds` call from
application code). `Desktop`'s drag-resize session already calls
`Window::set_bounds` on every pointer-move while a resize is in progress
(`desktop.rs`'s `continue_drag`), so live relayout during a drag falls out for
free — no change needed there.

**Migration, as the proof.** `ListBox` gains a real `set_bounds` (it
previously took its bounds only at construction, with no way to change them
short of rebuilding the whole list) that updates its stored bounds and
re-clamps `top` so the selection stays visible — mirroring the clamping
`HelpPane::set_bounds` already does. This is exercised for real by the
upcoming `HelpWindow` composite interior (`docs/specs/help_window.md`), which
implements `set_bounds` itself to redivide its width between the list column
and the page pane, then cascades to each child in turn — the same
"receive, recompute, cascade to children" shape a `Group` would need if it
ever grew a resizable composite child.

## Consequences

- Any interior that cares about its size gets correct relayout on drag-resize
  and zoom/restore "for free" the moment it implements `set_bounds` — no
  `Window`-side special-casing per widget, matching how `scroll_metrics`
  needed no per-widget change on the `Window` side either.
- Interiors that don't care (the overwhelming majority today — static text,
  fixed-size dialogs) pay nothing: the default is a no-op, and no existing
  call site changes behaviour.
- `Window` itself still never resizes or repositions its interior's *bounds
  rectangle* on the interior's behalf beyond telling it the new area — an
  interior that wants to reflow (wrap text, redistribute columns) does that
  itself inside `set_bounds`, exactly as `HelpPane` and the new `ListBox`
  impl do. `Window` doesn't know or care what "resize" means to its interior,
  consistent with ADR 0003's boxed-trait-object opacity.
- A composite interior with more than one child (like the upcoming
  `HelpWindow`) is now responsible for its own layout-splitting logic inside
  `set_bounds` — a small, local cost paid once per composite type, not once
  per scrollable/resizable leaf widget.

## Alternatives considered

- **Recompute layout unconditionally inside `draw`, from `canvas.bounds()`,
  with no cached state.** Avoids a trait change entirely, but `draw` takes
  `&self` — any interior that currently caches its layout (`HelpPane`'s
  wrapped `lines`, built once in `layout()` and read many times) would need
  that cache moved behind a `Cell`/`RefCell` purely to keep working when
  hosted resizably, reworking an already-shipped, already-tested widget to
  accommodate one new caller. Rejected: strictly more invasive than adding one
  defaulted trait method that `HelpPane` already half-expects (its
  `set_bounds` predates this ADR).
- **Let `Window` downcast into known resizable widget types and call a
  concrete method.** Rejected outright by ADR 0003, for the same reason the
  `edit`/`rvision` windowing split existed before ADR 0016.
- **Thread the new bounds through `Event::Resize` instead of a new trait
  method.** Rejected: `Event::Resize` already carries a specific, different
  meaning (the *terminal's* size, delivered top-down from `Application::run`)
  and is not emitted at all for a drag-resize session or a zoom toggle — reusing
  it would conflate "the terminal changed size" with "your own area changed,"
  which are genuinely different events with different owners.

[`Window`]: ../specs/window.md
[`ListBox`]: ../specs/controls.md
[`HelpPane`]: ../specs/help_window.md
[`drop_shadow`]: ../adr/0011-drop-shadows-per-view-protocol.md
[`scroll_metrics`]: ../adr/0015-scroll-chrome-per-view-protocol.md
