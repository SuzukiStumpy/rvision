# Module spec: `rvision::widgets::status_panel`

- **Status:** Done
- **Phase:** Roadmap #6 (new widgets)
- **Related ADRs:** 0003, 0008, 0015, 0032

## Purpose

A small display-only widget that shows one line of view-supplied status text
(e.g. a `TextArea`'s cursor `<line> : <col>` and `INS`/`OVR` mode). It does
not compute or own that text — it just draws whatever `String` its host last
gave it. It is not responsible for deciding *where* it sits: a `Window` can
host one on its own bottom border, and `Shell` can host one on the desktop
status row (ADR 0032); `StatusPanel` itself knows nothing about either.

## Public interface

```rust
pub struct StatusPanel {
    // ...
}

impl StatusPanel {
    /// The default reserved width in columns — fits "9999 : 999   OVR"
    /// with a little slack.
    pub const DEFAULT_WIDTH: i16 = 18;

    pub fn new(bounds: Rect, style: Style) -> Self;
    pub fn set_bounds(&mut self, bounds: Rect);
    pub fn set_text(&mut self, text: Option<String>);
}

impl View for StatusPanel {
    fn bounds(&self) -> Rect;
    fn draw(&self, canvas: &mut Canvas);
}
```

## Behaviour & invariants

- `draw` always fills its whole bounds with `style` first (so stale text
  never bleeds through when it shrinks), then, if `text` is `Some`, writes it
  left-aligned starting at local `(1, 0)` — matching `StatusLine`'s own
  one-column left margin.
- Text is clipped, never wrapped: `Canvas::put_str` naturally clips at the
  canvas edge (same as every other single-row widget in this crate), so a
  string longer than the bounds is silently truncated rather than pushed
  onto a second row.
- `text: None` draws a blank (filled) row — a host that has nothing to show
  still gets its reserved column back as visually blank chrome, not garbage
  from a previous frame.
- Zero-size bounds draw nothing (covered generically by `Canvas`/`Buffer`
  clipping elsewhere in the crate; no special case needed here).

## Collaborators

- `View` (bounds/draw only — not focusable, no event handling, same as
  `StatusLine`).
- `Canvas`/`Cell`/`Style` for drawing.
- Hosted by `Window` (own bottom border, left of `horizontal_scroll_bar`)
  and by `Shell` (desktop status row, right of `StatusLine`) — both pull the
  text to feed it from `View::status_text` (ADR 0032) on whatever interior
  view they're composing; `StatusPanel` itself never queries anything.

## Test plan (write these first)

- **Logic:** `set_bounds`/`set_text` update state; `bounds()` reflects the
  latest `set_bounds`.
- **Render (snapshot):** a populated panel; a `None` panel (blank row); text
  wider than bounds (truncates, doesn't wrap or panic).
- **Manual:** `examples/help_builder.rs`'s Source window bottom-left corner.

## Open questions

- None currently — width/positioning are entirely host-decided (see ADR
  0032), so this module has no layout policy of its own to resolve.
