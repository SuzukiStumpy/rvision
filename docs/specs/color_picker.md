# Module spec: `rvision::widgets::color_picker`

- **Status:** In progress (core control + `pick()` implemented and tested per
  the plan below; manual terminal pass still open)
- **Phase:** unscheduled (roadmap backlog #2)
- **Related ADRs:** 0005 (semantic roles over a truecolour-ready type), 0007
  (mouse), 0010 (modal dialogs & focus-aware controls), 0016 (unified
  `Window`), 0023 (truecolour capability detection)

## Purpose

A control for picking one concrete [`Color`](crate::color::Color) — a 16-swatch
CGA grid, plus (when the terminal supports it) custom RGB entry — and a modal
dialog built around it. Not a theme editor: `ColorPicker` picks a single
colour value and hands it back; pairing two of them for a role's fg/bg, and
persisting the result, is the theme editor's job (roadmap #2), which will use
this as a building block.

Deliberately never produces `Color::Default` — this round scoped the picker to
concrete colours only (`Named`/`Rgb`); a caller wanting a "reset to terminal
default" affordance adds it around the dialog, not inside it.

## Public interface

```rust
// New: Color16 gains a canonical enumeration, mirroring Role::ALL, so the
// grid can lay out swatches without duplicating the variant list.
impl Color16 {
    pub const ALL: [Color16; 16];
}

// The interior control — embeddable directly, not just via `pick`.
pub struct ColorPicker { /* ... */ }

impl ColorPicker {
    /// `profile` decides whether custom RGB/hex entry exists at all (Truecolor)
    /// or the grid is the whole picker (Cga16). Passed in, not detected
    /// internally — same testability seam as everywhere else `ColorProfile`
    /// is consumed (ADR 0023).
    pub fn new(initial: Color, profile: ColorProfile, theme: &Theme) -> Self;

    /// The current tentative colour (grid highlight or custom entry, whichever
    /// was touched most recently).
    pub fn color(&self) -> Color;
}

impl View for ColorPicker { /* ... */ }

/// Mirrors `FileDialogResult`: a handle read after `exec_view` returns `CM_OK`.
#[derive(Clone)]
pub struct ColorPickerResult(Rc<RefCell<Color>>);

impl ColorPickerResult {
    pub fn color(&self) -> Color;
}

impl ColorPicker {
    /// A centred, fixed, `Esc`-cancels `Window` ending on `CM_OK`/`CM_CANCEL`,
    /// same shape as `FileDialog::open`/`MessageBox::*`.
    pub fn pick(
        title: &str,
        initial: Color,
        profile: ColorProfile,
        theme: &Theme,
    ) -> (Window, ColorPickerResult);
}
```

## Behaviour & invariants

- **Grid.** 8 columns × 2 rows in `Color16` discriminant order: row 0 is
  indices 0–7 (`Black`…`LightGray`), row 1 is indices 8–15
  (`DarkGray`…`White`) — which is already the CGA "normal / bright" pairing
  (`Black`/`DarkGray`, `Blue`/`LightBlue`, … column-aligned), so no reordering
  is needed to get the classic layout. `Left`/`Right`/`Up`/`Down` move the
  highlight, clamped at the grid edges (no wraparound — matches
  `RadioButtons`' clamped `Up`/`Down`). Moving the highlight *is* selecting,
  immediately updating the tentative colour and preview — no separate
  navigate/activate step, again matching `RadioButtons` rather than
  `ListBox`/`FileDialog`'s select-then-accept split (there's no second control
  here for the grid to hand off to; `OK` ends the whole dialog, not the grid).
  A mouse click on a swatch selects it directly (ADR 0007).
- **Custom entry gating.** The RGB/hex section exists only when constructed
  with `ColorProfile::Truecolor` — entirely absent under `Cga16`, not merely
  disabled, so a 16-colour terminal never tab-stops into controls it can't
  render meaningfully.
- **Two synced representations, one canonical value.** Under `Truecolor`,
  custom entry offers three numeric fields (R/G/B, 0–255) *and* a single hex
  field (`RRGGBB`, with or without a leading `#`), both backed by one
  canonical `(u8, u8, u8)`. Only one representation is in the focus/tab order
  at a time; a dedicated toggle control switches which. Toggling copies the
  current canonical value into the newly active representation's field(s) —
  never the reverse — so a half-typed value in the mode being left doesn't
  leak into the other. Within the active representation, a keystroke that
  leaves it in a valid parse state updates the canonical value (and the live
  preview) immediately; one that doesn't (empty field, partial hex, digits
  out of `0..=255`) leaves the canonical value unchanged until parseable
  again.
- **Last touched wins the result shape.** Selecting a grid swatch sets the
  tentative colour to `Color::Named(c)`; subsequently editing either custom
  representation sets it to `Color::Rgb(r, g, b)` — even if the numbers
  happen to equal a named colour — and selecting a grid swatch afterwards
  switches it back to `Named`. Whichever subsystem was touched most recently
  decides the variant; under `Cga16`, the grid is the only subsystem, so the
  result is always `Named`.
- **Seeding.** `new`'s `initial` sets the starting tentative colour and, where
  possible, both subsystems' starting display: `Named(c)` seeds the grid
  highlight at `c` and the custom fields from `c.to_rgb()`; `Rgb(r, g, b)`
  seeds the custom fields directly. (Exact starting grid-cursor placement when
  seeded from an `Rgb` that doesn't exactly match any swatch is left open —
  see below.)
- **Commit/cancel.** `OK` writes the tentative colour into the
  `ColorPickerResult` handle; `Cancel`/`Esc` leaves it untouched (mirrors
  `FileDialogResult` — empty/previous if never accepted).

## Collaborators

- [`Color`]/[`Color16`]/[`Style`] (`color.rs`) — the value picked; `Color16`
  gains the `ALL` const described above.
- [`ColorProfile`] (`color.rs`, ADR 0023) — passed in by the caller (typically
  resolved once at startup via `ColorProfile::detect()`), not read from the
  environment by the picker itself.
- [`Theme`]/[`Role`] — style lookups (`DialogBackground` for the body,
  `Selection` for the grid highlight frame, `Input` for the entry fields).
- [`InputLine`] — the R/G/B and hex fields, reused as-is; digit
  filtering/clamping and hex parsing happen at the picker level reading
  `InputLine`'s text, the same seam `FileDialog` uses reading its path field.
  No new `InputLine` capability needed.
- [`Button`] — `OK`/`Cancel`, and the mode-toggle control (a plain relabelling
  button — "RGB" ⇄ "Hex" — is the cheapest option; see open questions).
- [`Window`] (ADR 0016) — the modal chrome behind `ColorPicker::pick`, same
  shape as `FileDialog::open`/`save` and `MessageBox`'s builders.
- An `Rc<RefCell<Color>>` result handle — the same shared-cell idiom as
  `FileDialogResult`.

## Test plan (write these first)

- **Logic:** `Color16::ALL` order and count; grid index ↔ `Color16` mapping;
  RGB field parsing (in-range, out-of-range, empty, non-digit → canonical
  unchanged vs. updated); hex parsing (valid with/without `#`, invalid chars
  → canonical unchanged vs. updated); "last touched" variant switching (grid
  then custom → `Rgb`; custom then grid → `Named`); toggling mode copies the
  canonical value into the newly active fields without touching the one being
  left.
- **Render (snapshot):** `Cga16` picker (grid only, no custom-entry section);
  `Truecolor` picker in RGB-field mode; `Truecolor` picker in hex mode; grid
  highlight rendered on the swatch matching a `Named` `initial`.
- **Interaction (scripted events):** arrow keys move the grid and update the
  preview immediately; the toggle switches mode and preserves the numeric
  value across the switch; typing digits in the R field updates the preview
  live; an out-of-range or partial value leaves the previous preview in
  place; `Enter` on `OK` commits into the result handle; `Esc` leaves the
  handle at its prior value; a mouse click on a swatch selects it directly.
- **Manual:** a `color_picker` example (or an addition to `dialogs`),
  eyeballed once with a truecolour terminal and once with `COLORTERM` unset,
  confirming `ColorProfile::detect()` gates the custom-entry section
  end-to-end and the swatch colours/hex/RGB values actually agree.

## Open questions

- Resolved: seeding from an `Rgb` that doesn't exactly match any swatch places
  the grid cursor at the fixed first cell (index 0) — confirmed as an
  implementation detail with no effect on the public interface or result
  correctness.
- Resolved: the mode toggle is a persistent, focusable, click-and-keyboard
  control in the tab order (not a hidden modifier-key shortcut) — mouse/
  discoverability parity won out (ADR 0007). It draws in
  `Role::ButtonFocused`/`Role::ButtonNormal` like a real button but is
  hand-rolled rather than an embedded [`Button`](crate::widgets::Button),
  since its activation must flip `ColorPicker`'s own internal mode rather
  than post a bubbling `Command` the way `Button` always does.
- Once the theme editor (#2) is actually scoped, whether it wants
  `ColorPicker` embedded inline (two side-by-side instances for fg/bg) or
  invoked modally per-field via `pick` — affects nothing here, deferred to
  that spec.
