# Module spec: `rvision::widgets::theme_editor`

- **Status:** Done
- **Phase:** unscheduled (roadmap backlog #2)
- **Related ADRs:** 0005 (semantic roles), 0009/0016 (`Desktop` dynamic
  windows), 0010 (modal dialogs — deliberately *not* used here), 0021 (a
  precedent for a non-modal, singleton-ish `Desktop` window: `HelpWindow`),
  0024 (resource layers), 0025 (theme file format/merge), 0026 (this
  module's own composition decision)

## Purpose

A dialog for editing a [`Theme`](crate::theme::Theme)'s per-role styles from
within a running application: browse all 19 [`Role`](crate::theme::Role)s,
edit a role's foreground/background via a nested
[`ColorPicker`](super::ColorPicker) and its attributes via checkboxes, then
save. Not a theme *picker* (choosing between existing themes) and not the
roadmap's theme *builder* (a thin app-defaults-layer wrapper around this same
editor, out of scope for this module).

Produces a **diff**, not a full theme dump: only the fields a user actually
touches during a session are serialized, via
[`Theme::format_field`](crate::theme::Theme::format_field), so saving a
one-colour tweak doesn't blank out everything else the layer beneath already
set (ADR 0025's whole point — a diff-based save is what makes that useful in
practice).

## Public interface

```rust
pub struct ThemeEditor { /* interior View: role list, preview, Fg/Bg
    buttons, 6 attribute checkboxes, a hand-drawn Restore Defaults control,
    Save/Cancel */ }

impl ThemeEditor {
    /// `base` seeds the working copy; `theme` styles the editor's own chrome
    /// (list/buttons/checkboxes) — same split as `ColorPicker::new`'s
    /// `initial`/`theme`.
    pub fn new(base: Theme, theme: &Theme) -> Self;

    /// The current edited snapshot (base with every touched field applied).
    pub fn theme(&self) -> Theme;
}

impl View for ThemeEditor { /* bounds/draw/handle_event/focusable */ }

/// A read/write handle onto the same `Rc<RefCell<ThemeEditorState>>`
/// `ThemeEditor` itself reads/writes from — generalizes the read-only
/// `ColorPickerResult`/`FileDialogResult` idiom so the hosting `Program`
/// (ADR 0026) can both inspect and mutate a still-open editor.
#[derive(Clone)]
pub struct ThemeEditorHandle(/* Rc<RefCell<ThemeEditorState>> */);

impl ThemeEditorHandle {
    /// The role currently highlighted in the list.
    pub fn selected_role(&self) -> Role;

    /// The selected role's current (edited) style — read by the driver to
    /// seed the nested `ColorPicker` with the right starting colour.
    pub fn style(&self) -> Style;

    /// Writes a colour picked via a nested `ColorPicker` back into the
    /// selected role's `field`, marking it touched. Called by the driver
    /// after its own nested `exec_view`/`Desktop` picker flow accepts.
    pub fn apply_color(&self, field: Field, color: Color);

    /// The full edited theme (base with every touched field applied).
    pub fn theme(&self) -> Theme;

    /// Every touched field, rendered one per line via `Theme::format_field`,
    /// sorted by `(role, field)` for deterministic output. Empty if nothing
    /// was touched.
    pub fn diff_text(&self) -> String;
}

impl ThemeEditor {
    /// Builds the `Desktop`-ready `Window` at `origin`: fixed size (derived
    /// from the interior's own layout — no auto-centring; `Desktop` doesn't
    /// have `exec_view`'s, ADR 0026), `resizable(false)`, `zoomable(false)`,
    /// `closable(false)`, `esc_cancels(true)`.
    pub fn window(origin: Point, title: &str, base: Theme, theme: &Theme)
        -> (Window, ThemeEditorHandle);
}
```

## Behaviour & invariants

- **Role list.** A `ListBox` of all 19 `Role::ALL` entries, labelled by
  `Role::key()` (the same `snake_case` string the theme-file format uses —
  one less vocabulary for a user to learn). `Up`/`Down`/click only change
  *which* role is previewed/edited; they never mutate anything.
  `always_show_selection(true)` (ADR 0020 addendum): almost all of this
  widget's interaction happens with focus away from the list (Foreground/
  Background/an attribute checkbox/Save/Cancel), so `ListBox`'s default of
  only marking the selection while it's the focused control would otherwise
  make it look like nothing is selected the moment focus tabs off — the same
  reasoning as `HelpWindow`'s topic list.
- **Preview.** A small swatch/sample showing the selected role's *current
  edited* style, read fresh from the shared state on every `draw` — never
  cached, since `apply_color` (called through `ThemeEditorHandle` from
  outside, between this window's own redraws) must be reflected on the very
  next frame. Its own starting `theme`'s chrome is unaffected — the editor
  doesn't hot-reload its own colours as you edit them, deliberately; see Open
  questions.
- **Foreground/Background.** Two buttons post `CM_EDIT_FG`/`CM_EDIT_BG`
  (`command.rs`) instead of editing anything themselves — per ADR 0026, a
  view hosted on a `Desktop` can't open another window itself. The hosting
  `Program` is responsible for opening a `ColorPicker::pick(...)` seeded from
  `handle.style()`'s matching field, and for calling `handle.apply_color`
  when it accepts.
- **Attribute checkboxes.** Six `CheckBox`es — Bold/Dim/Italic/Underline/
  Reverse/Blink — toggle directly in-widget via `Attributes::toggle` (no
  nested dialog needed, unlike colour), updating the edited style and
  marking `(role, Field::Attrs)` touched immediately. **Bold and Dim are
  mutually exclusive**: checking one clears the other (both in the checkbox
  UI and the underlying bit), since both set together renders identically to
  either alone in every terminal this framework targets and would otherwise
  offer a combination with no visible effect. The other four attributes have
  no such interaction.
- **Touched-tracking.** A `HashSet<(Role, Field)>`, global across the whole
  session: switching the selected role never loses another role's edits, and
  a role can be revisited and re-edited without losing its earlier touch.
- **Restore Defaults.** A hand-drawn, hand-dispatched control (like
  `ColorPicker`'s mode toggle — not a real `Button`, since it must act
  purely locally and post no command at all, unlike everything else in this
  widget). Resets the *whole* session to `Theme::default()` — a panic
  button, not a per-role undo — and recomputes `touched` from scratch as
  exactly the fields where the framework default differs from `base` (the
  layer beneath this session, stored at construction): a field `base`
  already left at the default value needs no override line, so a field
  touched earlier in the session but now back at the default is no longer
  marked. Still requires `Save` afterward to persist, like any other edit.
- **`diff_text()`.** Collects the touched set, sorts by `(role as usize,
  field-rank)` (`Fg` < `Bg` < `Attrs`) for deterministic output, and joins
  each `Theme::format_field(role, field)` line with `\n`. Empty string if
  nothing was touched.
- **Save vs. Cancel.** The Save button posts `CM_OK`; Cancel (and `Esc`, via
  `esc_cancels`) posts `CM_CANCEL`. `ThemeEditor` itself attaches no special
  meaning beyond posting the command — per ADR 0026 the *driver* decides
  what either means (write-and-close vs. discard-and-close), since only the
  driver knows whether a nested picker is currently pending (a `CM_OK`/
  `CM_CANCEL` while one is pending belongs to the *picker*, not the editor).
- **No live self-restyling.** `ThemeEditor` retains the chrome `theme` passed
  to `new`/`window` as its own fixed field, used for its own background fill
  and the Restore Defaults control, matching every other sub-widget here
  (each fixes its own colours from `theme` once, at construction) — editing
  the very role that styles the editor's own chrome
  (`DialogBackground`/`ButtonNormal`/`ButtonFocused`) never hot-reloads the
  editor mid-session, only the preview. A per-role reset (as opposed to
  Restore Defaults' whole-session reset) and a full-theme-dump save mode
  remain out of scope (see Open questions).

## Collaborators

- [`ColorPicker`](super::ColorPicker) — opened by the *driver*, not by
  `ThemeEditor` itself (ADR 0026).
- [`ListBox`](super::ListBox), [`CheckBox`](super::CheckBox),
  [`Button`](super::Button), [`Window`](super::Window) — composed directly
  (own `Focus` enum + tab order, not `view::Group` — the role list, preview,
  and checkboxes all need to resync each other on role-selection change,
  which `Group`'s generic children can't do; `ColorPicker` already
  established this "own `Focus`, delegate-then-resync" idiom for the same
  reason).
- [`Theme::format_field`]/[`Role::key`]/`Field` (`theme.rs`) — the
  serialization primitives this module assembles a diff from; owned by
  `theme.rs` since they're the inverse of `Theme::merge`'s own parsing, not
  editor-specific.
- [`Attributes::toggle`] (`color.rs`) — flips one attrs bit per checkbox.

## Test plan (write these first)

- **Logic:**
  - Touched-set tracking: toggling an attribute checkbox marks exactly
    `(selected_role, Field::Attrs)`; `apply_color` marks exactly
    `(selected_role, field)`; selecting a different role and editing it adds
    to, never replaces, the touched set.
  - `diff_text()`: empty when nothing touched; renders only touched fields;
    sorted deterministically regardless of edit order; matches
    `Theme::format_field`'s exact syntax.
  - `theme()` reflects every touched field's current value, unedited fields
    matching `base`.
  - `restore_defaults`: resets `edited` to `Theme::default()` exactly (every
    role); marks touched only fields where the default differs from `base`
    (an untouched-in-base role gets no override at all); wipes an unrelated
    field touched earlier this session if it's no longer relevant (a full
    reset, not a merge with prior touches).
- **Interaction (scripted events):**
  - Tab order cycles role list → Fg button → Bg button → 6 checkboxes →
    Restore Defaults → Save → Cancel → wraps.
  - Activating Restore Defaults (`Enter`/`Space`/click) posts no command at
    all, unlike every other focus stop.
  - Pressing/clicking Fg posts `CM_EDIT_FG`; Bg posts `CM_EDIT_BG`; neither
    mutates `theme()`.
  - Toggling a checkbox updates `theme()` immediately, no command posted.
  - Checking Bold then Dim (and vice versa) clears the first; the other four
    attributes are unaffected by either.
  - Save posts `CM_OK`; Cancel and `Esc` post `CM_CANCEL`.
  - Changing the selected role updates the preview/checkbox states to that
    role's current (possibly already-touched) style.
  - `apply_color` (via `ThemeEditorHandle`, simulating the driver's write-back
    after a nested `ColorPicker` accepts) is reflected by the very next
    `draw` — regression coverage for the preview having once been drawn from
    a stale, construction-time-cached style instead of the live shared state.
  - The selected role's highlight in the list survives focus moving away to
    Fg/Bg/an attribute checkbox/Save/Cancel — regression coverage for
    `always_show_selection`.
- **Render (snapshot):** the editor with a role at its default style; a role
  with some attributes checked; the list showing `Role::key()` labels.
- **Manual:** `examples/theme_editor.rs` — full round trip against a real
  `Desktop`: edit a colour via the nested picker (confirm it opens
  raised/focused), toggle an attribute, Save, confirm the printed diff text;
  run twice to confirm the resource-loader round trip; confirm Cancel at
  either the editor or the nested picker discards cleanly and is routed to
  the right one via the driver's `pending_picker`.

## Open questions

- **No per-role "reset to base" affordance.** Restore Defaults (added after
  first landing) resets the *whole* session, not one role at a time —
  deliberately deferred, the touched-set model would support a per-role
  reset cheaply (clear just that role's entries and reapply `base`'s own
  value for it) if a future pass wants it; not building it until asked for.
- **Keyboard-only role browsing when the list is short/tall relative to the
  window** — `ListBox` already handles scrolling; no new behaviour needed
  here, noted only because it wasn't separately test-listed above.
