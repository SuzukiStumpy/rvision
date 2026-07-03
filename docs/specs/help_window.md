# Module spec: `rvision::widgets::HelpWindow`

- **Status:** Draft
- **Phase:** post-extraction rework (help system, unblocked by SDI/MDI convergence)
- **Related ADRs:** 0013 (help format and topic model), 0016 (unify
  `Window`/`Dialog`, dynamic desktop), 0017 (resize propagation protocol),
  0015 (scroll chrome protocol), 0009 (`Shell`), 0007 (mouse), 0020
  (followable help links), 0021 (window-scoped context help)

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
already done — ADR 0013). It is also not *itself* what makes help
context-sensitive — deciding *which* topic to show is `Shell`'s job (ADR
0021, see [`shell.md`](shell.md)), reading a window's `help_topic`. This
module's own contribution to that is narrower and purely mechanical:
`build_at` (below), a way to construct the window already showing a given
topic instead of always the home one.

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

    /// As [`build`](Self::build), but shows `topic` instead of the home topic
    /// (ADR 0021) — resolved via the existing `HelpContents::topic_index`,
    /// mirroring how a followed link resolves its target (ADR 0020). An
    /// unresolvable `topic` (no such id) falls back to `contents.home()`
    /// silently, the same miss-handling as everywhere else a topic id is
    /// resolved — not a new failure mode to design around. Additive: `build`
    /// itself is unchanged, still the right call for "just open to home."
    pub fn build_at(contents: HelpContents, area: Rect, title: &str, theme: &Theme, topic: &str) -> Window;
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
- **Link activation drives the list, in the opposite direction (ADR 0020).**
  After every event routed into the pane, the interior drains
  `HelpPane::take_link_activation` — set when the pane's `Enter` follows its
  current link or a direct click lands on one; `Ctrl+Down`/`Ctrl+Up` only
  cycle which link is current and never activate one — and, if the target
  resolves via `HelpContents::topic_index`, mirrors `sync_pane_from_list`'s
  own shape: `list.select(idx)`, updates `shown`, and calls `pane.show`.
  Keyboard focus is untouched either way — activation moves the selection and
  page content, not focus. An unresolvable target (a dangling link, which a
  content test should already be catching per ADR 0013) is a silent no-op.
- **Focus.** Two focus targets, cycled by `Tab`/`BackTab` (wrapping): the
  list and the pane. A left-click on either pane's area focuses it first,
  mirroring `FileDialog`'s click-to-focus, before the click is routed in.
  Neither the divider column nor anywhere outside both panes is hit — a
  click there falls to `Ignored` and is not a focus change.
- **The current topic stays marked regardless of focus.** The list is built
  with `ListBox::always_show_selection(true)` (ADR 0020 addendum): while the
  pane holds focus, the current topic still draws — dimmer
  (`Role::SelectionInactive`) than the list's own focused highlight
  (`Role::Selection`) — rather than losing its highlight entirely, the way a
  plain `ListBox` would.
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
  leaves the list/pane empty; `build_at` selects the given topic's index and
  shows its body instead, and falls back to `contents.home()` for an
  unresolvable topic id (ADR 0021); `set_bounds` re-splits list/pane widths
  (narrow window shrinks the list column, never the reverse) and cascades to
  both children (a fake/spy is unnecessary here — `ListBox::rows()`/
  `HelpPane::content_height()` behaviour after resize is directly observable,
  as their own test suites already establish the pattern for).
- **Interaction (scripted events):** `Down`/`Up` in the list (focused) moves
  the pane to the newly selected topic; a click on a different list row does
  the same; scrolling *within* the pane (arrows/wheel/`PageDown`) never
  changes the list's selection; `Tab`/`BackTab` cycle list ↔ pane, wrapping;
  a click on the pane side focuses it without changing the list's selection;
  following a link (`Enter` on the current one, or a direct click) jumps the
  list selection and page to the target topic while keeping focus in the
  pane; `Ctrl+Down`/`Ctrl+Up` alone (no `Enter`) only cycles the pane's
  current link and never touches the list; an unresolvable link target is a
  no-op (ADR 0020).
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

- **Starting topic.** Resolved (ADR 0021): `build_at` opens straight to a
  given topic id, resolved the same way a followed link's target is
  (`HelpContents::topic_index`, ADR 0020), falling back to home on a miss.
  `build` itself is untouched and still the right call when there's no
  specific topic to open to. Deciding *which* topic id to pass — the actual
  "F1 for whatever's focused" behaviour — is `Shell`'s job, not this
  module's (ADR 0021, [`shell.md`](shell.md)).
- **A visible page-title header row.** Considered and cut for v1 (see
  Behaviour & invariants) to keep the composite to exactly two child widgets
  and one layout split; revisit only if the list's selection highlight
  proves insufficient feedback for which topic is showing in practice.
- **Full hypertext (followable `{label|target}` links).** Landed (ADR 0020):
  `HelpPane` tracks and renders links, cycles/activates them, and queues the
  target; `HelpWindow` drains it via `sync_list_from_pane_link`, exactly the
  "jump the list selection when a link is activated" this question
  anticipated — the layout/focus model above is otherwise unchanged.
