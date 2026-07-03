# ADR 0026 — Theme editor: `Desktop`-hosted composition via bubbled commands and a read/write handle

- **Status:** Accepted
- **Date:** 2026-07-03

## Context

Roadmap backlog #2's last dialog is the theme editor: "a user-facing dialog
for creating/editing themes from within a running application." Its
dependencies all landed already — `ColorPicker` (colour editing), `Theme::merge`
(ADR 0025's file format, and the field-level style it edits), and
`rvision::resource` (where a saved theme's user layer lives, ADR 0024). The
one open design problem is architectural, not data-shape: the editor's
"Foreground..."/"Background..." actions need to open a `ColorPicker` — one
dialog launching another, which nothing in `rvision` does yet.

A `View`'s `handle_event` only ever receives `&mut Context` (ADR 0003: views
never hold references to their owners; commands only bubble *up*). It has no
way to reach the `Desktop`/`Shell` that hosts it, so it cannot call
`Desktop::open` itself. `Application::exec_view` (ADR 0010) has the same
shape from the other direction: it takes `&mut Application<T>`, which a view
never has access to either. Every existing "opens a `Window`" widget
(`ColorPicker::pick`, `FileDialog::open/save`, `MessageBox`) is a leaf,
driven by application code that calls `exec_view` once, sequentially
(`examples/dialogs.rs`) — there is no existing precedent for one dialog
triggering another.

There *is* a precedent, though, for the shape this needs: `examples/mdi.rs`'s
`Mdi` struct wraps `Shell` in a custom `Program` and intercepts its own
app-specific commands (`CM_NEW_WINDOW`, `CM_TOGGLE_TOOLBOX`) in `dispatch()`,
reacting by calling `self.shell.desktop_mut().open(...)`. That is exactly "a
bubbled command causes the hosting code to open a new window" — the theme
editor needs the identical shape, just for a command the *framework* names
(since a library widget, not the app, is the one requesting the picker).

## Decision

**`ThemeEditor` is an ordinary `Desktop`-resident `Window`, not run via
`exec_view`.** Its chrome is locked down (`resizable(false)`, `zoomable(false)`,
`closable(false)`, `esc_cancels(true)`) so it *reads* as a dialog, the same
trick `ColorPicker::pick`/`FileDialog` already apply to their `exec_view`
windows — `Window::esc_cancels` is handled inside `Window::handle_event`
itself, so it works identically under `Desktop` dispatch or `exec_view`.
This follows `HelpWindow`'s precedent (ADR 0016/0021: a non-modal `Desktop`
window, not exclusive, but with its own fixed identity) rather than
`ColorPicker`'s.

**Two new framework commands, `CM_EDIT_FG`/`CM_EDIT_BG` (`command.rs`).**
`ThemeEditor`'s Fg/Bg buttons post these instead of editing anything
themselves. They must be framework-reserved (below `CM_USER`), not
app-numbered: `ThemeEditor` itself picks the signal's identity, and every
hosting app needs to recognise the same constant — the same reasoning that
already makes `CM_CLOSE`/`CM_ZOOM` framework commands even though only
`Desktop` acts on them. Save/Cancel reuse plain `CM_OK`/`CM_CANCEL`; no new
constants needed there.

**The hosting `Program` intercepts them exactly like `Mdi` intercepts its own
commands**, and reacts by opening a `ColorPicker::pick(...)` window via
`desktop.open`, remembering `pending_picker: Option<(WindowId, Field)>` as
its *own* field. A later `CM_OK`/`CM_CANCEL` is disambiguated purely by that
field: `Some` means it came from the picker (apply the colour, close it,
clear the field); `None` means it's the theme editor's own Save/Cancel. No
new dispatch mechanism — this is the same "driver holds the disambiguating
state" idiom `Mdi` already uses to know what `CM_TOGGLE_TOOLBOX` refers to.
`desktop.open` already raises and activates the freshly-opened window, so in
normal use the picker has input priority; a stray click reaching an obscured
control underneath the picker is an accepted, pre-existing trade-off of
`Desktop`'s non-exclusive dispatch (ADR 0016) — not a new gap introduced
here. True exclusivity is what `exec_view`/ADR 0010 is for, and this feature
deliberately isn't using it, per the decision above.

**The existing read-only result-handle idiom generalizes to read/write.**
`ColorPickerResult`/`FileDialogResult` are `Rc<RefCell<T>>` cells a caller
reads once, after `exec_view` returns `CM_OK`. `ThemeEditorHandle` is the
same cell shape, but read from and written to *while the window is still
open*: `selected_role()`/`style()` let the driver see what's pending (to seed
the nested picker), and `apply_color(field, color)` lets it write the picked
colour back into the very `ThemeEditor` instance that's about to redraw —
necessary because `Window` boxes its interior as `Box<dyn View>` with no
external accessor, so there is no other way to mutate it from outside once
constructed.

## Consequences

- No new framework primitive is invented. This composes two things that
  already exist — `Desktop`'s dynamic window management (ADR 0009/0016) and
  the app-level command-interception idiom `mdi.rs` established — extended
  only by two new named commands and a read/write variant of an existing
  handle shape.
- Any future "one dialog opens another" need (the roadmap's theme *builder*
  and help authoring tool, both explicitly slated to reuse this editor's
  surface) follows the same shape: a framework command for the request, a
  `pending_*` field on the driver, a read/write handle.
- `ThemeEditor` cannot be driven via `exec_view` the way `ColorPicker`/
  `FileDialog` are — a host app must have a `Desktop` (or at least drive a
  custom `Program` that can call `Desktop::open`), not just a bare
  `Application`/`Backdrop` loop like `examples/dialogs.rs`'s simpler dialogs.
- Interaction with `ThemeEditor` while its nested `ColorPicker` is open is
  not exclusive (no input lock), unlike every other modal dialog in this
  codebase. Acceptable here because the picker is always raised/activated on
  open and the editor's own controls are inert without a role selected to
  act on, but worth remembering if a future consumer expects `exec_view`-like
  guarantees.

## Alternatives considered

- **Bake `CM_EDIT_FG`/`CM_EDIT_BG` handling into `Shell` itself**, the way
  `CM_HELP` is caught natively (ADR 0021). Rejected: `Shell` would become
  aware of one specific, optional widget's needs. Help is a core framework
  feature every `Shell`-based app is expected to want; `ThemeEditor` is an
  optional utility dialog, not core chrome.
- **Run `ThemeEditor` via a sequential (non-nested) `exec_view` loop**,
  matching `ColorPicker`/`FileDialog` exactly: the driving code calls
  `exec_view` on the editor, gets `CM_EDIT_FG`/`CM_EDIT_BG` back (registered
  via `also_ends_on`), calls `exec_view` again on a fresh `ColorPicker`
  window, then re-enters `exec_view` on the same editor `Window`. This gives
  true exclusivity and needs no `Desktop` at all. Rejected in favour of
  `Desktop` hosting: it forces every host app to give up `Desktop`/`Shell`
  chrome for this one dialog, and doesn't match the roadmap's likely
  eventual use of the editor inside a real, already-`Desktop`-based `Shell`
  app.
- **A generic modal stack owned by `Application`**, so any view could
  request a nested `exec_view` without app-level plumbing. Rejected as
  premature generalization: there is exactly one consumer today, and the
  `Desktop`-hosted shape already covers it using existing mechanism.
