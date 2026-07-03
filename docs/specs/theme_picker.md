# Module spec: `rvision::widgets::theme_picker`

- **Status:** Done. `examples/theme_picker.rs` offers "CGA (default)" always
  and "Truecolour" (`examples/themes/truecolour.theme`) only when
  `ColorProfile::detect()` reports `Truecolor`.
- **Phase:** unscheduled (roadmap backlog #2)
- **Related ADRs:** 0005 (semantic roles), 0010 (modal dialogs & focus-aware
  controls), 0016 (unified `Window`), 0025 (theme file format/merge)

## Purpose

A dialog for picking one [`Theme`](crate::theme::Theme) out of several named
candidates, with a live preview of each — not an editor: it hands back
whichever whole `Theme` was chosen, unlike `ThemeEditor` (roadmap #2's other
half), which mutates one role at a time. Where the candidates come from
(files on disk, `Theme::default()` plus a bundled truecolour theme, …) and
what happens with the choice afterwards (apply in memory, persist via
`rvision::resource::write_user_resource`, …) is entirely the caller's concern
— this widget only ever sees already-built `Theme` values plus display names,
mirroring `rvision::resource`'s own boundary (ADR 0024: the framework
locates/reads raw text; it does not maintain a catalogue of named resources).

## Public interface

```rust
pub struct ThemePicker { /* ... */ }

impl ThemePicker {
    /// `candidates` is shown in the given order; `initial` is the starting
    /// highlight (clamped into range). `theme` styles the picker's own
    /// chrome (list/buttons) — fixed at construction, independent of
    /// whichever candidate is highlighted (same split as `ColorPicker::new`'s
    /// `initial`/`theme`, `ThemeEditor::new`'s `base`/`theme`).
    pub fn new(candidates: Vec<(String, Theme)>, initial: usize, theme: &Theme) -> Self;

    /// The index currently highlighted in the list.
    pub fn selected_index(&self) -> usize;
}

impl View for ThemePicker { /* focusable; List/OK/Cancel */ }

/// Mirrors `ColorPickerResult`/`FileDialogResult`: a handle read after
/// `exec_view` returns `CM_OK`.
#[derive(Clone)]
pub struct ThemePickerResult(Rc<RefCell<(String, Theme)>>);

impl ThemePickerResult {
    pub fn name(&self) -> String;
    pub fn theme(&self) -> Theme;
}

impl ThemePicker {
    /// A centred, fixed, `Esc`-cancels `Window` ending on `CM_OK`/`CM_CANCEL`,
    /// same shape as `ColorPicker::pick`/`FileDialog::open`.
    pub fn pick(
        title: &str,
        candidates: Vec<(String, Theme)>,
        initial: usize,
        theme: &Theme,
    ) -> (Window, ThemePickerResult);
}
```

## Behaviour & invariants

- **List drives a live preview.** Moving the list highlight (`Up`/`Down`/
  `Home`/`End`/a click on a row) immediately redraws the preview panel from
  that candidate's own `Theme` — no separate activate step, the same
  immediate-update idiom as `ColorPicker`'s grid and `ThemeEditor`'s role
  list. The list stays visibly selected even once focus tabs away to OK/
  Cancel (`always_show_selection(true)`, same reasoning as `ThemeEditor`'s
  role list: most of this dialog's point is comparing candidates by eye,
  which shouldn't stop just because focus moved to a button).
- **Preview is self-contained.** A fixed handful of roles rendered as a small
  sample screen (title bar showing the candidate's own name, a menu row, body
  text, a selected line, a button pair, an input swatch, a help link) — drawn
  entirely in the *highlighted candidate's* styles. It exists to give a feel
  for the theme, not to be exhaustive (that's what actually applying it, or
  `ThemeEditor`, is for).
- **Chrome never reflects the candidate being browsed.** The picker's own
  list/OK/Cancel colours come from `theme` (the caller's current theme, fixed
  at construction) — browsing candidates changes only the preview panel, so
  the dialog itself never flickers as the highlight moves.
- **Commit is explicit.** `OK` writes the *currently highlighted* candidate's
  `(name, Theme)` into the result handle and posts `CM_OK`; `Cancel`/`Esc`
  posts `CM_CANCEL` without touching it. The handle is seeded at
  construction with the `initial` candidate, so a cancelled dialog's result
  still reads as something sane (mirrors `ColorPicker`/`FileDialog`'s
  "untouched" value being the starting one, not an empty/default one — there
  is no natural empty `Theme` the way `Color::Default`/an empty `PathBuf`
  serve those two).
- **OK's side effect is reachable by every input path.** `ColorPicker`
  shipped with (and later fixed) a bug where clicking `OK` posted `CM_OK` via
  the plain `Button` widget without running the extra "write the tentative
  value into the result handle" step that only `Enter` triggered. `ThemePicker`
  closes the equivalent gap from the start: `Enter`, `Space`, and a mouse
  click on `OK` all route through one `accept` method, never through
  `Button::handle_event`'s own bare `CM_OK` post.
- **Degenerate input.** Empty `candidates` doesn't panic (an empty list, a
  preview that draws nothing, `OK` posts `CM_OK` without writing a result) —
  but isn't a scenario the dialog tries to make useful; callers are expected
  to supply at least one candidate. `initial` out of range clamps to the last
  valid index (or `0` when `candidates` is empty).

## Collaborators

- [`Theme`]/[`Role`] — every candidate is a full, already-resolved `Theme`;
  the widget only ever calls `.style(role)` on them, never `.merge`/`.with`.
- [`ListBox`] — the candidate list, reused as-is (no new capability needed).
- [`Button`] — `OK`/`Cancel`.
- [`Window`] (ADR 0016) — the modal chrome behind `ThemePicker::pick`, same
  shape as `ColorPicker::pick`/`FileDialog::open`/`MessageBox`'s builders.
- An `Rc<RefCell<(String, Theme)>>` result handle — the same shared-cell
  idiom as `ColorPickerResult`/`FileDialogResult`.
- Deliberately **not** a collaborator: `rvision::resource`. Loading candidate
  themes from disk and persisting the choice both stay at the application
  layer (see `examples/theme_picker.rs`), matching `ThemeEditor`/
  `ColorPicker`'s own examples, which call `rvision::resource` themselves
  rather than the widget doing so.

## Test plan (write these first)

- **Logic:** candidate names/order land in the list unchanged; `initial`
  clamps into `0..candidates.len()` (and to `0` when empty); moving the
  selection touches nothing but `selected_index()`.
- **Render (snapshot):** a picker with two candidates, default highlight;
  after moving the highlight to the second candidate (confirms the preview
  panel actually redraws from the newly highlighted theme, not a cached
  one).
- **Interaction (scripted events):** `Down`/`Up` move the highlight; `Enter`
  from `List` commits the highlighted candidate (not just from `OK`); `Space`
  on `OK` commits, matching a mouse click on `OK` (the two paths `ColorPicker`
  once diverged on); a mouse click on `OK` commits; `Esc`/`Cancel` leaves the
  result at its constructed seed; `Tab`/`BackTab` cycle List→OK→Cancel→List
  and back.
- **Manual:** `examples/theme_picker.rs`, offering `Theme::default()` and
  `examples/themes/truecolour.theme` (merged, gated on
  `ColorProfile::detect()`) as the two candidates — confirms the preview
  panel actually looks different between them on a truecolour terminal.

## Open questions

None outstanding — scope intentionally kept to "browse a caller-supplied list
and hand back the choice," matching what `rvision::resource`/ADR 0024 already
decided is and isn't the framework's job.
