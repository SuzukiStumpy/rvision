# Roadmap

> `rvision` was extracted from the [`edit`](https://github.com/SuzukiStumpy/edit)
> monorepo after Phase 10 of its phased build (rendering core, real terminal +
> event loop, view system, chrome, dialogs & controls, mouse, help system,
> clipboard, settings — see `edit`'s `docs/roadmap.md` for that history). Every
> decision from that period is recorded in [`docs/adr/`](adr/) and
> [`docs/specs/`](specs/); this file picks up from the extraction point and
> tracks what's next now that `rvision` stands on its own.

## Backlog (inherited, unscheduled)

Carried over from `edit`'s backlog at the point of extraction. None of these are
scheduled into a phase yet; roughly ordered by how much shared machinery they
need.

- ~~**Framework windowing vs. `edit`'s bespoke windowing.**~~ Resolved by
  ADR 0015 (scroll-chrome protocol) and ADR 0016 (unify `Window`/`Dialog`,
  dynamic `Desktop`): `Desktop` now supports dynamic open/close, click-to-
  front, drag, resize, hide/show, and keyboard window-cycling natively (see
  [`docs/specs/desktop.md`](specs/desktop.md)). ADR 0016 deliberately stops
  short of migrating `edit` itself — `edit` keeps its own bespoke MDI, pinned
  to `rvision v0.1.0`, and can adopt the new `Desktop` at its own pace — but
  the gap that blocked a framework-level `HelpWindow` is gone.
- ~~**`HelpWindow`.**~~ Landed: a non-modal desktop window (`widgets::HelpWindow`)
  composing a topic-list `ListBox` and a `HelpPane` side by side, opened via
  `Desktop::open` (see [`docs/specs/help_window.md`](specs/help_window.md)).
  Resizable, not just positioned — which surfaced and closed a real gap
  (`Window` never told a resizable interior its area changed) fixed as its
  own protocol, ADR 0017. `edit` still builds its own modal viewer for now
  (nothing requires it to migrate).
- ~~**Full hypertext help.**~~ Landed: `{label|target}` links (ADR 0013) are
  followable (ADR 0020). `HelpPane` parses a paragraph's spans, word-wraps
  them tracking each link's on-screen position, and lets `Ctrl+Down`/
  `Ctrl+Up` cycle a "current link" highlight (`Enter` follows it; a click
  follows immediately) — dedicated keys chosen specifically to leave
  `HelpWindow`'s existing `Tab`/`BackTab` list⇄pane contract untouched.
  Activating a link queues its target, drained by `HelpWindow` the same way
  it already polls the list's own selection, so the list and page jump to
  match. See [`docs/specs/help_window.md`](specs/help_window.md) and the
  `mdi` example's Overview topic.
- ~~**Context-sensitive help.**~~ Landed at the design level (ADR 0021;
  implementation next): scoped to *window*-level granularity rather than
  Turbo Vision's per-view `HelpCtx`, since a `View`-trait-wide mechanism isn't
  justified yet by one feature and one consumer. `Window` carries an optional
  `help_topic`; `Frame` shows a glyph for it beside the zoom glyph; both the
  glyph and `F1` post the existing, previously-unused `CM_HELP`; `Shell`
  (opted in via `with_help`) resolves the active window's topic and opens a
  singleton `HelpWindow` via its new `build_at` entry point. Modal dialogs
  (`exec_view`) are excluded outright, not deferred — a modal owns input
  exclusively until dismissed, so it was never going to be able to defer to a
  non-modal `HelpWindow` mid-run. `edit`'s own bespoke MDI chrome doesn't use
  `Shell`/`Desktop` yet, so it isn't reached by this until it migrates onto
  them (separate, future work). See [`docs/adr/0021-window-scoped-context-help.md`](adr/0021-window-scoped-context-help.md),
  [`docs/specs/window.md`](specs/window.md), [`docs/specs/help_window.md`](specs/help_window.md),
  and [`docs/specs/shell.md`](specs/shell.md).
- ~~**Cascading menus (submenus).**~~ Landed: a `MenuItem::submenu` opens a
  nested pull-down instead of posting a command, cascading to arbitrary depth.
  The `MenuBar` state machine generalized `open: Option<usize>` + `highlight`
  into a `path: Vec<usize>` stack; the overlay draws and hit-tests every open
  level (ADR 0018, extending the ADR 0009 overlay). See
  [`docs/specs/menu.md`](specs/menu.md) and the `chrome` example's File ▸
  Export item.
- ~~**Right-click context menus.**~~ Landed: a pull-down anchored at the
  pointer, populated per context. Reuses `Menu`/`MenuItem` and the ADR 0018
  cascade rules unchanged (most of the cascade geometry/hit-testing/drawing
  turned out to be shareable with `MenuBar` outright, as free functions in
  `menu.rs`); the trigger is an ordinary right-click through the existing
  positional dispatch, resolved to true screen coordinates via a small offset
  accumulator added to `Context` (ADR 0019), so any nested view can request one
  correctly anchored regardless of depth. See
  [`docs/specs/context_menu.md`](specs/context_menu.md) and the `chrome`
  example's right-click on the window interior.

## Now that `rvision` is standalone

Extraction is the trigger `edit`'s ADR 0024 named for `rvision` graduating "to
an independent semver line and its own repository" — that has now happened.
Open questions this raises, not yet decided:

- **Versioning & release.** CI (`test` + `lint` in `.github/workflows/ci.yml`)
  already runs on every push and PR, but there is no release automation yet —
  no tagging, no changelog, no crates.io publish. `edit` drove its lockstep
  workspace version with release-please over Conventional Commits; `rvision`
  needs its own call now that it versions independently.
- **crates.io publishing.** Not yet published. `Cargo.toml`'s `repository`
  field already points here; a `publish` step and a documentation pass
  (`cargo doc`, crate-level docs) would be the remaining gap.
- **A second consumer.** `edit` is still the only known consumer. Gaining a
  second consumer (or publishing) was the other named trigger for going
  independent — worth revisiting how much API stability to promise once one
  exists, since today the API can move freely to suit `edit` alone.
- **Apple Silicon macOS verification.** `edit`'s Phase 10 exit criteria left
  this "pending hardware" — CI builds it, but no manual terminal-quirk pass has
  happened on that platform. Still open, low urgency, blocks nothing.

## Adding a phase

When work here gets scheduled, add a `## Phase N` section following the format
`edit`'s roadmap used: the modules it touches, an interface sketch, and the
tests to write first — then copy `docs/module-spec-template.md` per module
before writing any code (ADR 0014).
