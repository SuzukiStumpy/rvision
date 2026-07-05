# ADR 0032 — Status content is a per-view protocol, hosted in two places

- **Status:** Accepted
- **Date:** 2026-07-05

## Context

Roadmap item 6 names a "status panel" — a small display for a text view's
cursor `<line> : <col>` and `INS`/`OVR` mode — without scoping it further.
The concrete need, once scoped: the panel must be usable either on the
desktop's status row (to the right of `StatusLine`'s hint list, driven by
whichever window is topmost) or on a window's own bottom border (to the left
of its horizontal `ScrollBar`, which must shrink to make room). Either way,
the composing owner (`Shell`, or `Window`) needs to ask an arbitrary interior
view "do you have status text to show?" without downcasting into its
concrete type (ADR 0003).

That's the same shape of problem ADR 0015 solved for scroll chrome: a child
that wants something hosted on its owner's border opts in through a
defaulted `View` trait method, queried every draw, with no cost to every
other view that doesn't implement it. Scroll needed a push-back channel too
(`set_scroll`, since the owner's chrome computes a new position the child
must accept); status content doesn't — it's read-only from the owner's
point of view, so a pull-only method is enough.

A second question this ADR resolves: what "row/col" means. `TextArea`
already tracks a *display* position (`display_pos`, private) for placing the
on-screen caret — the row within its word-wrapped `lines`, which shifts
whenever the window resizes and a line rewraps, even though the cursor
hasn't moved. That's the wrong number to show in a status panel: every
mainstream editor's line/column indicator is the *logical* position in the
underlying text (real newlines only), stable under reflow.

## Decision

Add one defaulted `View` trait method, in the same shape as
`scroll_metrics`/`drop_shadow`:

```rust
/// The status text this view wants shown in a hosting owner's status
/// panel, or `None` if it has none to offer (ADR 0032). Queried every
/// draw, like `scroll_metrics`. Pull-only: unlike scroll, there's nothing
/// for an owner to push back.
fn status_text(&self) -> Option<String> {
    None
}
```

`TextArea` implements it, formatting a *logical* line/column — counting real
`\n`s up to the cursor for the line, and graphemes since the last `\n` for
the column (both 1-based) — via a new public `cursor_line_col()`, plus a new
public `is_overtype()` for the existing private `overtype` flag:
`"{line} : {col}   {mode}"`, `mode` = `"INS"`/`"OVR"`.

Two owners host the resulting text through a new, purely-visual
`StatusPanel` widget (a display-only leaf, no query logic of its own —
`docs/specs/status_panel.md`):

- **`Window`** (opt-in via `.status_panel(true)`, default `false`) pulls
  `self.interior.status_text()`, draws a `StatusPanel` on its own bottom
  border to the left, and shrinks/shifts `horizontal_scroll_bar()` by
  exactly that width — mirroring how `vertical_scroll_bar`/
  `horizontal_scroll_bar` already host scroll chrome by querying
  `scroll_metrics`. `Window` also implements `status_text` itself,
  delegating to its interior unconditionally, independent of the frame
  flag — so the second owner below can read it regardless.
- **`Shell`** (opt-in via `.with_status_panel()`) hosts one on the desktop
  status row, to the right of `StatusLine`, refreshed from whichever window
  is topmost (`desktop.active_id()` → `desktop.window(id)` →
  `Window::status_text`, the same chain `Shell::open_help` already uses to
  resolve the active window's help topic). The refresh runs on every
  `Event::Idle`/`Event::Broadcast` tick (the shell's existing per-tick
  fan-out point), so it updates both on a window raise and live as the user
  types — a superset of "refreshes when a new window is raised," at no extra
  mechanism cost.

The two hosting modes are independent and composable, not mutually
exclusive in code: nothing stops a window from enabling its own frame panel
*and* a `Shell`-level one showing the same content while it's active. That's
a valid (if redundant) combination an app author might choose; this ADR
doesn't add a guard against it, matching ADR 0015's own "opt-in both ways"
stance on scroll chrome.

## Consequences

- One status-content mechanism, reusable by any future view, not just
  `TextArea` — the same generalization ADR 0015 bought for scroll chrome.
- `StatusPanel` carries no layout policy of its own (width, position): both
  hosts decide that themselves, the same division of labour `ScrollBar`
  already has with its hosts.
- The status panel's line/column is logical, not the on-screen wrapped row —
  correct per this decision, but a reader comparing it against the caret's
  actual screen row in a heavily-wrapped `TextArea` will see them diverge;
  that's intentional, not a bug.
- `examples/help_builder.rs`'s `TextArea` never reports horizontal scroll
  metrics (it word-wraps instead of scrolling sideways), so its demo never
  visibly exercises the scrollbar-shrink path — that path is real and
  generically correct for any future horizontally-scrolling interior, and is
  covered by a `Window` unit test with a stub interior instead.

## Alternatives considered

- **Bake formatting into `StatusPanel` itself** (have it query `View` and
  format text internally). Rejected: it would need to know about `TextArea`
  specifically to format anything, breaking the no-downcast rule the same
  way `Window` reaching into a concrete scrollable widget would (ADR 0015).
  Keeping `StatusPanel` a dumb display leaf, fed a pre-formatted `String` by
  whoever queried `status_text`, keeps the widget reusable for any future
  status content, not just line/col/mode.
- **Show the wrapped display row instead of the logical line.** Simpler (no
  new `TextArea` method — `display_pos` already exists), but it changes
  value under a pure resize with the cursor untouched, which reads as a bug
  to anyone used to a conventional editor's status line.
- **A single shared `Option<Rc<RefCell<StatusPanel>>>` between `Window` and
  `Shell`.** Rejected: needless shared mutable state for two owners that
  never need to see each other's copy — each hosts its own `StatusPanel`
  fed from the same pull, exactly as two `ScrollBar`s on two different
  `Window`s each get their own instance today.
