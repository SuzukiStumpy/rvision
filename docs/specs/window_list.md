# Module spec: `rvision::widgets::window_list`

- **Status:** Done
- **Phase:** roadmap #2, "New/updated standard dialog boxes"
- **Related ADRs:** 0003 (owner chain, no back-references), 0010 (modal
  dialogs — the shape this deliberately does *not* use), 0016 (dynamic
  `Desktop`), 0021 (`CM_HELP`/`Shell`-hosted singleton — the precedent this
  follows), 0026 (bubbled command, hosting code acts), 0036 (`Any` downcast
  for a window's content), 0037 (this module's own Shell-interception shape)

## Purpose

A live view of every window currently open on a `Desktop` — TurboVision's
"Window List." Lets a user bring any window to the front (and dismiss the
list), terminate one outright, or reset one's size/position back onto the
desktop, without hunting for it under overlapping chrome. Reset exists as
the manual recovery path GitHub issue #9 asked for: `Desktop::continue_drag`
never clamps a dragged/resized window to the desktop's own bounds, so a
window can in principle end up off-screen or oversized with no other way
back. It is **not** a modal picker (contrast `ThemePicker`/`ColorPicker`):
it must stay open and reflect `Desktop`'s state across a `Close`/`Reset`, so
several windows can be fixed or closed in one visit. It is **not** itself
where `Desktop` mutation happens — `WindowList` only records what the user
asked for; the code hosting it (`Shell`) is the one with a `Desktop` to act
on (ADR 0003, ADR 0037).

## Public interface

```rust
/// What the user asked `WindowList` to do, read (and cleared) by whatever
/// hosts it via `Desktop::content_mut` (ADR 0036) after a
/// `CM_WINDOW_LIST_ACTIVATE`/`CM_WINDOW_LIST_CLOSE` bubbles up.
pub enum WindowListAction {
    Activate(WindowId),
    Close(WindowId),
    /// Clamp this window's bounds back onto the desktop and bring it to
    /// front; the list itself stays open (mirrors `Close`, not `Activate`).
    Reset(WindowId),
}

/// A titles-only list of open windows plus Close/Reset buttons.
pub struct WindowList { /* ListBox + parallel Vec<WindowId> + Close/Reset Buttons */ }

impl WindowList {
    /// `entries` is shown in list order, id-per-row. `theme` styles the
    /// widget's own chrome.
    pub fn new(entries: Vec<(WindowId, String)>, theme: &Theme) -> Self;

    /// Rebuilds the displayed rows from a fresh snapshot (there is no
    /// in-place item-replace on `ListBox`), used after a `Close` removes a
    /// window. Selection is cleared if the previously-selected id isn't in
    /// the new snapshot, kept (by id, not index) otherwise.
    pub fn set_entries(&mut self, entries: Vec<(WindowId, String)>);

    /// Reads and clears the pending action, if any.
    pub fn take_pending(&mut self) -> Option<WindowListAction>;

    /// A `Window` titled `title`, sized to fit and centred within `area`
    /// (`Desktop::open` doesn't consult `Placement`, so centring happens
    /// here once, at construction — mirrors `HelpWindow::build`). An
    /// ordinary resizable/moveable/closable/zoomable window, not a
    /// chrome-locked dialog: this is a persistent utility window like
    /// `HelpWindow`, not a one-shot pick like `ColorPicker::pick`.
    pub fn build(entries: Vec<(WindowId, String)>, area: Rect, title: &str, theme: &Theme) -> Window;
}

impl View for WindowList { /* ... */ }
```

## Behaviour & invariants

- A plain click on a row selects it (`ListBox`'s own behaviour); no action
  is recorded.
- `DoubleClick` on a row, or `Enter` while the list is focused, records
  `Activate(id)` for the highlighted row and posts `CM_WINDOW_LIST_ACTIVATE`
  — mirrors `FileDialog`'s "double-click on the list = select + commit"
  (`file_dialog.rs`).
- Clicking **Close** (or `Enter`/`Space` while Close is focused) with a row
  selected records `Close(id)` and posts `CM_WINDOW_LIST_CLOSE`. With
  nothing selected (only possible on an empty list), Close is a no-op that
  posts nothing.
- Clicking **Reset** (or `Enter`/`Space` while Reset is focused) with a row
  selected records `Reset(id)` and posts `CM_WINDOW_LIST_RESET`; empty-list
  behaviour mirrors Close (a no-op, nothing posted).
- `Tab`/`BackTab` cycles `List ⇄ Close ⇄ Reset` and wraps, same shape as
  `ThemePicker`'s `Focus`/`FOCUS_ORDER` minus the OK/Cancel pair.
  `take_pending` never has a bearing on where focus is.
- `take_pending` returns `None` and leaves state untouched when nothing is
  pending; a second call right after a first never returns the same value
  twice (read-and-clear).
- `set_entries` never panics on an empty `Vec` — draws a plain "No open
  windows" message instead of a blank list, and Close stays a no-op.
- `WindowList` never reads or writes any global/shared state itself — every
  `WindowId` it hands back is exactly one it was constructed or refreshed
  with; it has no way to reach `Desktop` and never tries to (ADR 0003).

## Collaborators

- `ListBox`, `Button` (existing widgets) compose the interior, same
  precedent as `ThemePicker`/`ComboBox`.
- `Window` — `build` wraps the interior the same way `HelpWindow::build`
  does.
- Hosted and driven entirely by `Shell` (`src/app.rs`): `Shell` builds the
  initial snapshot from `Desktop::windows()`, opens it via `Desktop::open`,
  and — on `CM_WINDOW_LIST_ACTIVATE`/`CM_WINDOW_LIST_CLOSE`/
  `CM_WINDOW_LIST_RESET` bubbling back up — reads `take_pending()` through
  `Desktop::content_mut::<WindowList>` (ADR 0036) and acts on `Desktop`
  directly. `Activate` → `show`+close the list; `Close` → `close`+refresh;
  `Reset` → clamp the target's bounds via `arrange::clamp_rect`, `show` the
  target, then `show` the list itself again immediately after — `Desktop::
  show`/`raise` unconditionally reassigns `active` (keyboard routing)
  regardless of any `topmost` flag, so without that second `show` the list
  would silently lose keyboard focus to the just-fixed window the instant
  it stayed open, defeating the point of a "stays open" action. See ADR
  0037 for why this lives in `Shell` unconditionally rather than behind an
  opt-in like `with_help`.

## Test plan (write these first)

- **Logic:** entries land in list order; single click only moves highlight;
  `Enter`/`DoubleClick` on the list sets `pending = Activate(selected)` and
  posts `CM_WINDOW_LIST_ACTIVATE`; a plain click never sets `pending`;
  Close sets `pending = Close(selected)` and posts `CM_WINDOW_LIST_CLOSE`;
  Reset sets `pending = Reset(selected)` and posts `CM_WINDOW_LIST_RESET`;
  Close/Reset with an empty list post nothing and leave `pending` `None`;
  `take_pending` clears after reading; `set_entries` rebuilds rows and
  drops a selection whose id is no longer present; Tab/BackTab cycles
  `List ⇄ Close ⇄ Reset` and wraps; framework commands posted are only the
  three above (no stray app-numbered command confusion, mirroring
  `ThemePicker`'s own guard test).
- **Render (snapshot):** populated list, list focused; Close focused; Reset
  focused; empty-list state.
- **Interaction (scripted events):** a `DoubleClick` on a row followed by
  reading `take_pending` end to end; a `Close`-click → `set_entries` refresh
  round trip; a `Reset`-click on an out-of-bounds window → `Shell` clamps
  its bounds, raises it, and the list keeps keyboard focus.
- **Manual:** `examples/chrome.rs`'s Window ▸ Window List... — open several
  windows, double-click a background one (front + dialog closes), reopen,
  select another + Close (terminates, dialog stays open, entry gone); drag
  a window far off-screen, open the list, select it + Reset (snaps back
  on-screen, raised, list still has keyboard focus without an extra click).

## Open questions

None outstanding — the "type" column originally proposed was cut (asked and
confirmed): `Window`/`Desktop` track no such concept today, and adding one
speculatively for this alone isn't justified.

A manual pass (`examples/mdi.rs`, tmux) surfaced one real gap, since fixed:
`Activate` must resolve through `Desktop::show`, not `Desktop::focus` —
`focus` deliberately no-ops on a hidden window (needed so `CM_NEXT`/
`CM_PREV` cycling skips one correctly), but picking a hidden window from
this list should always bring it to front. `WindowList` itself is unchanged
by this — the fix lives entirely in `Shell::resolve_window_list_action`.

**Known limitation, not fixed (GitHub issue #9 partial fix):** `Reset`
clamps a window's *current* bounds back onto the desktop; it doesn't touch
`Window`'s own `maximized`/restore bookkeeping. `CM_ZOOM` sets a maximized
window's bounds to exactly the desktop's size at zoom time, so `Reset` is a
no-op against the *same* desktop size — but if the terminal shrinks after a
window is maximized (`Desktop::set_bounds` never re-fits an already-
maximized window), `Reset`'s clamp would shrink its bounds without updating
the zoom glyph/toggle state, which could then look stale. Same spirit as
this work leaving `Desktop::continue_drag`'s missing clamp ceiling
unfixed — a follow-up, not part of this action's scope.
