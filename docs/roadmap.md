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

- **Framework windowing vs. `edit`'s bespoke windowing** *(needs its own
  grilling)*. `edit` owns its documents and chrome concretely — its own MDI,
  drag, resize, modal driver — instead of using `rvision`'s `Desktop`/`Window`,
  because those wrap `Box<dyn View>` and acting on a concrete document behind
  one would force a downcast or `Rc<RefCell>` (ADR 0003). Each step was locally
  reasonable, but the result is **two windowing implementations**, and this
  crate's has atrophied: no dynamic open/close, no drag in the widget itself.
  The help system was the first feature to want a mature framework window (a
  non-modal help window) and tripped over the gap — that's why a default
  `HelpWindow` container is deferred (ADR 0013). Converging the two (IDs + a
  registry? a window-kind enum? generics over the interior?) touches ADRs 0003,
  0009, 0010 and the `Shell` design, so it deserves a dedicated grilling before
  any code.
- **`HelpWindow`.** A non-modal desktop window wrapping `HelpPane` + a topic
  list, once the windowing question above is settled. Until then, consuming
  applications build their own modal viewer (as `edit` does).
- **Full hypertext help.** The `{label|target}` link syntax is already reserved
  in the help markup (ADR 0013); the v1 parser keeps only the label. Making
  links followable — jumping to another topic — is its own later phase.
- **Context-sensitive help.** `open_help`-style entry points already take a
  starting topic id; wiring real context-sensitivity (F1 opening the topic for
  whatever currently has focus) is application-level, and also waits on the
  windowing question.
- **Cascading menus (submenus).** A `MenuItem` that opens a nested pull-down
  instead of posting a command. Extends the `MenuBar` state machine (the open
  path becomes a stack) and the overlay draw + hit-testing (ADR 0009).
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
