# Roadmap

> `rvision` was extracted from the [`edit`](https://github.com/SuzukiStumpy/edit)
> monorepo after Phase 10 of its phased build (rendering core, real terminal +
> event loop, view system, chrome, dialogs & controls, mouse, help system,
> clipboard, settings — see `edit`'s `docs/roadmap.md` for that history). Every
> decision from that period is recorded in [`docs/adr/`](adr/) and
> [`docs/specs/`](specs/); this file picks up from the extraction point and
> tracks what's next now that `rvision` stands on its own.
>
> Landed items below are kept to a one-line pointer — the full story (design
> tradeoffs, bugs found and fixed, follow-ups) lives in the linked ADR/spec
> and in git history, not here.

## Backlog (inherited, unscheduled) — all resolved

Carried over from `edit`'s backlog at the point of extraction.

- ~~**Framework windowing vs. `edit`'s bespoke windowing.**~~ Resolved by ADR
  0015/0016: `Desktop` supports dynamic open/close, drag/resize, hide/show,
  keyboard cycling. See [`docs/specs/desktop.md`](specs/desktop.md).
- ~~**`HelpWindow`.**~~ Landed: `widgets::HelpWindow`, a non-modal
  topic-list + page-pane window (ADR 0017). See
  [`docs/specs/help_window.md`](specs/help_window.md).
- ~~**Full hypertext help.**~~ Landed: followable `{label|target}` links
  (ADR 0020).
- ~~**Context-sensitive help.**~~ Landed: window-scoped `CM_HELP` via
  `Shell::with_help` (ADR 0021).
- ~~**Cascading menus (submenus).**~~ Landed: `MenuItem::submenu` (ADR 0018).
- ~~**Right-click context menus.**~~ Landed: `ContextMenu` (ADR 0019).

## Now that `rvision` is standalone — all resolved

Questions raised by extraction itself (`edit`'s ADR 0024).

- ~~**Versioning & release.**~~ Resolved: `release-please` tags `vX.Y.Z` on
  merge from Conventional Commits (ADR 0022).
- ~~**crates.io publishing.**~~ Landed 2026-07-05: automated `cargo publish`
  gated on a release (ADR 0022 addendum). The API-reference docs arrive for
  free via docs.rs now that this is live.
- ~~**A second consumer.**~~ Closed 2026-07-05: resolved by publishing
  itself — `rvision` is a real public crate, so the stability question
  stopped being hypothetical.
- ~~**Apple Silicon macOS verification.**~~ Confirmed 2026-07-14: a manual
  terminal pass found no quirks.

## Backlog (new, unscheduled — raised 2026-07-03)

Raised in a backlog-planning conversation; #1 originally gated several of
the others. Most of this list is now landed — what's left open is #4, #5,
and help's topic-level merge under #9.

1. ~~**True colour support.**~~ Landed 2026-07-03: `ColorProfile::detect()`
   (ADR 0023) chooses truecolour vs. 16-colour CGA from env vars;
   [`examples/truecolour.rs`](../examples/truecolour.rs) and
   [`examples/themes/truecolour.theme`](../examples/themes/truecolour.theme)
   demonstrate the graceful fallback end to end.
2. ~~**New/updated standard dialog boxes.**~~ All landed:
   [`FileDialog`](../src/widgets/file_dialog.rs) (Open/Save),
   [`ColorPicker`](../src/widgets/color_picker.rs),
   [`ThemeEditor`](../src/widgets/theme_editor.rs) (ADR 0026),
   [`ThemePicker`](../src/widgets/theme_picker.rs), and
   [`WindowList`](../src/widgets/window_list.rs) (ADR 0037, 2026-07-14).
   See each widget's spec under `docs/specs/`.
3. ~~**Utility programs.**~~ Landed:
   [`examples/theme_builder.rs`](../examples/theme_builder.rs) and
   [`examples/help_builder.rs`](../examples/help_builder.rs) (a `TextArea`
   source pane with a live `HelpPane` preview). The help-authoring work also
   added backslash-escape syntax to the `.help` format and fixed a
   `<pre>`-block parsing bug (ADR 0029).
4. **Python bindings.** Still unscheduled — needs its own FFI design session
   (how `View` dispatch crosses the boundary, who drives the poll/read
   loop) before it can be scoped further.
5. **TypeScript/JavaScript bindings.** Same story as #4, and likely lower
   priority; deferred for the same reason.
6. ~~**New widgets.**~~ All landed: combo box (`widgets::ComboBox`, ADR
   0030's `View::wants_topmost`), group box (`widgets::GroupBox`, ADR 0031's
   `Group::non_wrapping`), status panel (`widgets::StatusPanel`, ADR 0032's
   `View::status_text`), TextArea (`docs/specs/text_area.md`, sharing
   `widgets::text_edit` with `InputLine`), and tab bar
   (`widgets::TabbedPages`, `docs/specs/tabbed_pages.md`).
7. ~~**Insert/overtype support**~~ for text entry. Landed: `InputLine`/
   `TextArea` gained an `overtype` toggle (`Insert` key); the caret style
   follows the mode.
8. ~~**End-user/developer documentation.**~~ Landed:
   [`docs/getting-started.md`](../docs/getting-started.md), a zero-to-running
   tour. The API-reference half arrives via docs.rs now that crates.io
   publishing (above) is live.
9. **Generic resource loader.** Path resolution/read-write
   (`rvision::resource`, ADR 0024) and the theme file format/merge
   (`Theme::merge`, ADR 0025) are landed. **Help's own topic-level merge
   remains deliberately deferred, unscheduled.**

## Adding a phase

When work here gets scheduled, add a `## Phase N` section following the format
`edit`'s roadmap used: the modules it touches, an interface sketch, and the
tests to write first — then copy `docs/module-spec-template.md` per module
before writing any code (ADR 0014).
