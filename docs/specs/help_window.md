# Module spec: `rvision::widgets::HelpWindow`

- **Status:** Draft
- **Phase:** post-extraction rework (help system, unblocked by SDI/MDI convergence)
- **Related ADRs:** 0013 (help format and topic model), 0016 (unify
  `Window`/`Dialog`, dynamic desktop), 0017 (resize propagation protocol),
  0015 (scroll chrome protocol), 0009 (`Shell`), 0007 (mouse)

## Purpose

The non-modal desktop window `docs/specs/desktop.md`'s "Open questions" and
`roadmap.md` both named as the first real `Desktop` consumer: a topic list
down the left, the selected topic's page on the right, composed into one
[`Window`](window.md) interior (ADR 0013's "topic-list + scrollable-page
viewer"). It owns a [`ListBox`](controls.md) of topic titles and a
[`HelpPane`](help_pane.md — n/a, see `help.rs`/`help_pane.rs`) side by side,
and keeps them in sync: moving the list selection shows that topic's page.

It is **not** a modal help viewer (`edit`'s own, ADR 0013's "viewer surface"
question) and **not** the format/parser (`HelpContents`/`HelpTopic`/`Block`,
already done — ADR 0013). It is also not context-sensitive (jumping straight
to a topic for whatever has focus) — that's application-level and still
waits on nothing this module provides beyond an opened window (roadmap).

## Public interface

```rust
/// A two-pane help browser: a topic list and the selected topic's page,
/// composed as one `Window` interior (ADR 0013, ADR 0016).
pub struct HelpWindow {
    contents: HelpContents,
    list: ListBox,
    pane: HelpPane,
    bounds: Rect,
    focus: HelpFocus,   // List | Pane
}

impl HelpWindow {
    /// Builds a `Window` titled `title`, sized and centred within `area`
    /// (typically the caller's `Desktop` bounds — `Desktop::open` does not
    /// consult `Placement`, unlike `exec_view`, so centring happens here,
    /// once, at construction), showing `contents.home()`. Resizable,
    /// moveable, closable, zoomable — a plain, fully capable `Window`, not a
    /// dialog: nothing about a help browser calls for `exec_view`'s ending/
    /// `Esc`-cancels policy.
    pub fn build(contents: HelpContents, area: Rect, title: &str, theme: &Theme) -> Window;
}

impl View for HelpWindow {
    // bounds/draw/handle_event/focusable as below.
    fn set_bounds(&mut self, bounds: Rect);   // ADR 0017: redivide the columns, cascade to both children
}
```

`HelpFocus` is a private two-value enum (`List`/`Pane`), not part of the
public surface — mirrors `FileDialog`'s private `FOCUS_*` constants, just
with two targets instead of four.

## Behaviour & invariants

- **Layout.** The interior splits its width into a fixed-width list column
  (up to `LIST_WIDTH`, shrinking if the window is narrower than that plus a
  minimum pane width — no proportional split, matching the simplicity of a
  fixed sidebar over a resizable content pane), a one-column divider, and the
  page pane taking the remainder; both panes fill the full interior height
  (no separate title-header row — the list's own selection highlight is
  what shows which topic is current, keeping this module to exactly two
  child widgets). Recomputed by `set_bounds`.
- **Resizing (ADR 0017).** `HelpWindow::set_bounds` is the composite's own
  cascade point: it re-splits the new size into list/pane rectangles and
  calls `self.list.set_bounds(...)` / `self.pane.set_bounds(...)`, each of
  which already knows how to relayout itself (`ListBox`'s row count/scroll
  clamp, `HelpPane`'s re-wrap). `Window::set_bounds`/`toggle_zoom` call this
  automatically on every drag-resize step and on zoom/restore — `HelpWindow`
  itself never needs to know *why* its bounds changed, only that they did.
- **Selection drives the page.** After every event routed into the list (key
  or mouse) that could change `list.selected()`, the interior compares the
  new selection against what the pane is currently showing; on a change it
  calls `self.pane.show(&topic)` — which, per `HelpPane`'s own contract,
  resets that pane's scroll to the top-left. Scrolling *within* a topic never
  touches the list, so it never re-triggers this.
- **Focus.** Two focus targets, cycled by `Tab`/`BackTab` (wrapping): the
  list and the pane. A left-click on either pane's area focuses it first,
  mirroring `FileDialog`'s click-to-focus, before the click is routed in.
  Neither the divider column nor anywhere outside both panes is hit — a
  click there falls to `Ignored` and is not a focus change.
- **Empty contents.** A `HelpContents` with no topics leaves the list empty
  (`ListBox`'s own empty-list handling: `selected() == None`) and the pane
  blank (`HelpPane`'s own just-constructed empty state) — `HelpWindow::build`
  never calls `pane.show` if `contents.home()` is `None`. No panic, no
  special-cased draw path.
- **Window configuration.** `resizable(true).moveable(true).closable(true)
  .zoomable(true)` — every flag left at `Window::new`'s fully-capable
  default; none of `esc_cancels`/`also_ends_on`/`with_default`/`centered()`
  are set, since those are `exec_view`/modal-only policy this window never
  runs under (`Desktop::open` ignores `Placement` entirely — see
  [`desktop.md`](desktop.md)).

## Collaborators

- [`ListBox`](controls.md) (topic titles; `set_bounds`, `selected`,
  `scroll_metrics`/`set_scroll` already exist — ADR 0015/0017) and
  [`HelpPane`](../adr/0013-help-format-and-model.md) (the page; `show`,
  `set_bounds` already exist).
- `help::{HelpContents, HelpTopic}` (ADR 0013) — read-only; `HelpWindow` owns
  a clone, never mutates it.
- [`Window`](window.md) (what `build` returns; hosts no scroll chrome of its
  own for this interior, since neither `ListBox` nor `HelpPane` reports
  `scroll_metrics` upward — each manages its own scrolling internally/via its
  own hosted bar, exactly as they do standalone).
- [`Desktop`](desktop.md) (`desktop.open(help_window)` is the intended call
  site; `HelpWindow` has no dependency on `Desktop` itself, only on being a
  plain `Window`).

## Test plan (write these first)

- **Logic:** `build` selects `contents.home()`'s index in the list and shows
  its body in the pane; an empty `HelpContents` builds without panicking and
  leaves the list/pane empty; `set_bounds` re-splits list/pane widths
  (narrow window shrinks the list column, never the reverse) and cascades to
  both children (a fake/spy is unnecessary here — `ListBox::rows()`/
  `HelpPane::content_height()` behaviour after resize is directly observable,
  as their own test suites already establish the pattern for).
- **Interaction (scripted events):** `Down`/`Up` in the list (focused) moves
  the pane to the newly selected topic; a click on a different list row does
  the same; scrolling *within* the pane (arrows/wheel/`PageDown`) never
  changes the list's selection; `Tab`/`BackTab` cycle list ↔ pane, wrapping;
  a click on the pane side focuses it without changing the list's selection.
- **Render (snapshot):** the two columns with their divider at a representative
  size; a narrow window that shrinks the list column; an empty-contents
  window (blank list, blank pane, no panic).
- **End-to-end (through `Window`/`Desktop`):** the built `Window` is
  resizable/moveable/closable/zoomable; a `Desktop`-driven corner-drag resize
  relayouts both panes live (via the ADR 0017 propagation, not anything
  `HelpWindow`-specific); `CM_CLOSE` (closable) and `CM_ZOOM` (zoomable, via
  `Window::toggle_zoom`) both work through `Desktop` exactly as any other
  window's do.
- **Manual:** a `Help` menu item in the `mdi` example (or a small dedicated
  one) opening a `HelpWindow` built from a short embedded `HelpContents`,
  demonstrating resize, topic switching, and zoom/restore together.

## Open questions

- **Starting topic.** `build` always starts at `contents.home()` (the first
  declared topic). Opening straight to a specific topic id (context-
  sensitive help, `roadmap.md`) is application-level and deferred there, per
  ADR 0013's own scope cut — not designed here to avoid a speculative
  parameter with no current caller.
- **A visible page-title header row.** Considered and cut for v1 (see
  Behaviour & invariants) to keep the composite to exactly two child widgets
  and one layout split; revisit only if the list's selection highlight
  proves insufficient feedback for which topic is showing in practice.
- **Full hypertext (followable `{label|target}` links).** Unaffected by this
  module either way — `HelpPane` already renders only the label (ADR 0013);
  `HelpWindow` would gain a way to jump the list selection when a link is
  activated, once that phase lands, without changing today's layout/focus
  model.
