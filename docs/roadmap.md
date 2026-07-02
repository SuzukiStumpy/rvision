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
- **Full hypertext help.** The `{label|target}` link syntax is already reserved
  in the help markup (ADR 0013); the v1 parser keeps only the label. Making
  links followable — jumping to another topic — is its own later phase, and
  would extend `HelpWindow` to move the list selection when a link activates
  (see that spec's Open Questions).
- **Context-sensitive help.** `open_help`-style entry points already take a
  starting topic id; wiring real context-sensitivity (F1 opening the topic for
  whatever currently has focus) is application-level — `HelpWindow::build`
  itself always starts on the home topic today (a deliberate v1 scope cut, see
  [`docs/specs/help_window.md`](specs/help_window.md)'s Open Questions).
- ~~**Cascading menus (submenus).**~~ Landed: a `MenuItem::submenu` opens a
  nested pull-down instead of posting a command, cascading to arbitrary depth.
  The `MenuBar` state machine generalized `open: Option<usize>` + `highlight`
  into a `path: Vec<usize>` stack; the overlay draws and hit-tests every open
  level (ADR 0018, extending the ADR 0009 overlay). See
  [`docs/specs/menu.md`](specs/menu.md) and the `chrome` example's File ▸
  Export item.
- **Right-click context menus.** A pull-down anchored at the pointer, populated
  per context. Reuses the `Menu` overlay and its open/closed/modal state
  machine; the trigger becomes a right-press hit-test rather than the menu bar.
  Best explored *after* submenus — they share the nesting/anchoring machinery.

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
