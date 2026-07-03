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

- ~~**Versioning & release.**~~ Resolved by ADR 0022: `release-please` now
  runs on every push to `main`, maintaining an open release PR from
  Conventional Commits that bumps `Cargo.toml`'s version and `CHANGELOG.md`;
  merging it tags `vX.Y.Z` and cuts a GitHub Release. The first cut is
  `v1.0.0`, bootstrapped via a `Release-As` commit footer over the existing
  `v0.1.0` tag — `edit` depends on `rvision` through a pinned git reference
  and isn't tracking its HEAD, so the jump doesn't force any downstream
  change. See [`docs/adr/0022-release-process-and-versioning.md`](adr/0022-release-process-and-versioning.md),
  `release-please-config.json`, and `.github/workflows/release.yml`.
- **crates.io publishing.** Not yet published. `Cargo.toml`'s `repository`
  field already points here; a `publish` step (gated on release-please's
  `release_created` output, alongside a documentation pass — `cargo doc`,
  crate-level docs) would be the remaining gap. Now that tagging/changelog
  automation is in place (ADR 0022), this is the only piece of release
  automation still missing.
- **A second consumer.** `edit` is still the only known consumer. Gaining a
  second consumer (or publishing) was the other named trigger for going
  independent — worth revisiting how much API stability to promise once one
  exists, since today the API can move freely to suit `edit` alone. `v1.0.0`
  has now been cut (ADR 0022) on the strength of `edit`'s pinned dependency
  not being exposed to breakage; this question is now specifically about how
  that ongoing stability promise shapes future changes, not about whether the
  initial commitment was premature.
- **Apple Silicon macOS verification.** `edit`'s Phase 10 exit criteria left
  this "pending hardware" — CI builds it, but no manual terminal-quirk pass has
  happened on that platform. Still open, low urgency, blocks nothing.

## Backlog (new, unscheduled — raised 2026-07-03)

Raised in a backlog-planning conversation, not yet detailed into phases.
Listed in the order raised; #1 is called out as gating several of the others.
Once these are solidified (detail filled in, dependencies worked out), this
section and the "Backlog (inherited, unscheduled)" section above will be
retired in favour of a fresh roadmap with proper phases/milestones.

1. **True colour support.** Currently limited to the original EGA/CGA
   palette. ADR 0005 already left the cell colour type truecolour-ready;
   themes ship 16-colour CGA first. Needs to land before the colour/theme
   dialogs below, which depend on the underlying colour model.
   - Scoped 2026-07-03: most of the mechanism already exists —
     `Color::Rgb(u8,u8,u8)` and the crossterm backend's passthrough are
     already in place (ADR 0005's "back pocket" is spent). What's actually
     left is capability detection with a graceful 16-colour fallback (the
     scope ADR 0005 explicitly deferred), plus a *new* truecolour theme (or
     themes) to exercise it. The embedded `Theme::default()` CGA theme stays
     exactly as-is in framework code — it is not being replaced. Any new
     truecolour theme is data for the app layer to supply/inject at
     runtime, not a new hardcoded Rust constructor, which makes this item
     dependent on #9 (resource loader) for the "ship a real theme" half;
     the capability-detection half does not need to wait.
   - Landed 2026-07-03: `ColorProfile::detect()` in `color.rs` (ADR 0023) —
     reads `COLORTERM`/`TERM` to decide `Truecolor` vs. `Cga16` via a pure,
     unit-tested decision function. No consumer yet by design; `Theme` is
     untouched pending #9. Still open: the resource loader itself, and the
     actual truecolour theme(s) it will let an app supply.
2. **New/updated standard dialog boxes.**
   - Colour/palette picker — don't have one yet.
   - Theme picker — essential once more themes exist.
   - Theme editor — a user-facing dialog for creating/editing themes from
     within a running application; a distinct tool from #3's theme
     builder (see below), not a naming duplicate of it.
   - ~~Standardised file open/save dialogs.~~ Already have one:
     [`widgets::FileDialog`](../src/widgets/file_dialog.rs), covering both
     Open and Save, built on ADR 0016's unified `Window`.
   - Scoped 2026-07-03: the file dialog sub-item is resolved (struck
     above), leaving three sub-items with a real dependency order. The
     colour/palette picker is ungated — `Color`/`Color16`/`Style`
     (ADR 0005) and `ColorProfile` (ADR 0023) already give it everything
     it needs (a 16-swatch CGA grid always available; RGB entry offered
     when `ColorProfile::detect()` reports `Truecolor`) — so it's the
     piece that can start now. The theme picker stays blocked: it
     has nothing to pick *between* until #9 lands enough to load a second
     theme and #1 has an actual truecolour theme as data to offer. The
     theme editor writes a created/edited theme to the *user* resource
     layer (ADR 0024's auto-discovered per-user path) — its UI/model can
     be scoped and built against an in-memory `Theme` before #9's
     still-open theme file format is settled; only the actual *save*
     needs that format to land first. It will want the colour picker as a
     building block, so realistically follows it rather than the other
     way round.
   - ~~Colour/palette picker.~~ Landed 2026-07-03:
     [`widgets::ColorPicker`](../src/widgets/color_picker.rs) — an 8×2 CGA
     swatch grid always present, plus toggleable RGB-fields/hex custom entry
     gated on `ColorProfile::Truecolor`, both representations kept in sync
     against one canonical value, grid-vs-custom "last touched wins" deciding
     `Named` vs. `Rgb` on accept. `ColorPicker::pick()` wraps it in the usual
     centred, Esc-cancels `Window`, mirroring `FileDialog`. Wired into the
     `dialogs` example. See [`docs/specs/color_picker.md`](specs/color_picker.md)
     (manual terminal pass still open). The theme picker and theme editor
     sub-items remain blocked as scoped above.
3. **Utility programs.** Help authoring tool; theme builder; possibly more.
   - Scoped 2026-07-03: the theme builder is a developer-facing tool for
     authoring a theme to ship *with* an application, most likely a thin
     wrapper reusing #2's theme editor dialog/component but pointed at
     the app-defaults resource layer (ADR 0024) instead of the user layer
     the in-app editor writes to by default. Same editing surface, two
     different output layers — see #2's scoping note.
   - Scoped 2026-07-03: the help authoring tool needs no new format — ADR
     0013 already designed the `parse(&str) -> HelpContents` boundary
     specifically so a future authoring tool could target it, and the
     tool live-previews through the existing `HelpPane` renderer. Same
     shape as the theme builder: a thin wrapper that saves through #9's
     resource loader to the app-defaults layer. Its editing pane needs a
     multi-line text-entry control, which doesn't exist yet — see #6.
4. **Python bindings.** Write `rvision` applications with Python as the
   application layer calling into the library.
5. **TypeScript/JavaScript bindings.** Similar goal to the Python bindings;
   likely lower priority.
   - Scoped 2026-07-03 (both #4 and #5, deferred — not resolved): the
     crate-structure question is easy — bindings live in their own,
     separate crate(s), so ADR 0001's runtime crate budget (which governs
     `rvision` itself) doesn't apply to them. The real blocker is a
     genuine FFI design problem, not a roadmap bullet: `rvision`'s `View`
     is a Rust trait dispatched via `Box<dyn View>` (ADR 0003) — a
     Python/JS app layer needs to *implement* that shape and be called
     back into every frame across the boundary (PyO3 for Python; a Node
     native addon via `napi-rs`/`neon` for JS, since a real TTY rules out
     WASM/browser). Needs its own dedicated design session — which
     binding mechanism, how `View` dispatch crosses the FFI boundary,
     whether the host language or Rust drives the single-threaded
     poll/read loop — before either item can be scoped further.
6. **New widgets.** Combo box; group box; tab bar; status panel; TextArea
   (multi-line text entry); possibly others.
   - Scoped 2026-07-03: "frame" renamed to **group box** — a titled,
     bordered box for visually grouping related controls (e.g. a set of
     radio buttons under "Alignment:"), distinct from the existing
     `widgets::Frame` (window-chrome border drawing helper, not an
     independent `View` — see `src/widgets/frame.rs`). TextArea added: a
     general-purpose scrollable multi-line text-entry control,
     generalizing `InputLine`'s single line the way `ListBox`
     generalizes a single choice. It's reusable UI furniture, not
     `edit`-specific domain knowledge, so it belongs here rather than
     staying editor-only — CLAUDE.md's "no editor knowledge" line is
     about document/buffer/syntax concepts, not a plain multi-row text
     field. #3's help authoring tool is blocked on this landing.
     TextArea is a separate `View` from `InputLine`, not one widget
     configured into two modes: the internal shapes genuinely differ (a
     single `String` + one cursor index vs. a multi-line buffer with
     vertical scrolling via the existing `scroll_metrics`/`ScrollBar`
     protocol, ADR 0015, and reflow via the existing `wrap.rs` already
     used by `HelpPane`) — folding both into one struct means branching
     on a mode flag through draw/event/scroll throughout. Follows the
     precedent already set by `MenuBar`/`ContextMenu`: separate types,
     sharing only the genuinely common mechanics (grapheme-based cursor
     advance, the insert/overtype toggle from #7, bracketed-paste
     handling) as free functions, the way cascade/geometry/hit-testing
     became shared free functions in `menu.rs` rather than a merged type.
7. **Insert/overtype support** for text entry controls.
   - Scoped 2026-07-03: low-ambiguity. `KeyCode::Insert` already exists
     in the event model (`src/event.rs`), just unhandled. Wire it to
     toggle insert/overwrite on `InputLine` now; the new TextArea (#6)
     picks up the same toggle once it lands.
8. **End-user/developer documentation.** Possibly a GitHub wiki.
   - Scoped 2026-07-03: narrower than first framed. The developer half is
     already largely covered — crate-level rustdoc (`src/lib.rs`) carries
     an architecture-at-a-glance overview, and `docs/adr`/`docs/specs`/
     `CLAUDE.md` cover the *why* and *how the framework works*. The real
     gap is an end-user getting-started tutorial: zero-to-running walked
     in prose, narrating what `examples/hello.rs` already demonstrates in
     code, through to wiring a `Shell`/`Desktop` with a dialog. Venue: no
     wiki — it would be a second, unversioned, non-PR-reviewed home for
     documentation, breaking the pattern every other doc in this project
     already follows (ADRs, specs, roadmap: all in-repo, versioned,
     reviewed). Lands as `docs/getting-started.md` (or an expanded
     README) instead. The API-reference half arrives for free via
     docs.rs once crates.io publishing happens — already tracked above
     under "crates.io publishing," which already names "a documentation
     pass" as part of that remaining gap; the two should merge into one
     line item once either is actually scheduled.
9. **Generic resource loader.** Raised 2026-07-03 while scoping #1: themes
   and help content (ADR 0013) both want to move from Rust-embedded data
   (`include_str!` etc.) to files loaded at runtime, layered
   framework-defaults → application-defaults → user-customisation,
   auto-loaded at bootstrap. Help content already has a clean
   `parse(&str) -> Model` boundary designed with exactly this swap in mind
   (ADR 0013), so this generalises an already-anticipated seam rather than
   inventing one. Gated (now partially cleared, see below) the "ship a real
   theme" half of #1 and the theme/help authoring tools in #3.
   - Scoped 2026-07-03 (ADR 0024, `docs/specs/resource.md`, design only —
     no code yet): the three layers aren't symmetric. Framework defaults stay
     compile-time embedded (`rvision` has no runtime install location of its
     own); the app layer's directory is supplied explicitly by the app
     author (no auto-discovery — packaging conventions vary too much); only
     the user layer gets real auto-discovered path resolution, generalizing
     `edit::settings::config_path` (`edit` ADR 0025) with an app-name and
     per-kind file-name parameter. One file per resource kind, not an
     omnibus config. No generic `Resource`/merge trait — each kind (theme,
     help) keeps its own format and merge rule; `rvision::resource` only
     locates and reads raw text for the app/user layers. Still open: the
     theme file format and its merge function, help's topic-level merge, and
     an `edit`-style env-var override.
   - Design nailed down 2026-07-03 (ADR 0024 addendum, `docs/specs/resource.md`
     — still no code): closed the two items the first pass left open.
     Write-back was missing outright — added `write_user_resource`,
     symmetrical with `user_resource_path`/`load_layers`, needed by #2's
     theme editor and #3's theme builder to actually save what they edit.
     The app-defaults layer gets no equivalent helper — its caller already
     holds the exact directory it supplied, so writing there is a bare
     `fs::write`, not path-resolution logic worth centralizing. The
     `edit`-style env-var override is decided **against**: generalizing it
     would mean `rvision` inventing an env-var name from a runtime
     `app_name` string, which is the same kind of guessed-at convention
     ADR 0024 already ruled out for app-layer auto-discovery. An app that
     wants one checks its own env var first and falls through to
     `user_resource_path`/`write_user_resource` when unset — no module
     support needed. What's still open (deliberately, its own future pass):
     the theme file format/merge function and help's topic-level merge.
     Ready for TDD.
   - ~~Path resolution + read/write layer, landed 2026-07-03.~~
     [`rvision::resource`](../src/resource.rs) — `ResourceLayers`,
     `load_layers`, `user_resource_path`, `write_user_resource`, exactly per
     the spec above. Still open, deliberately deferred: the theme file
     format/merge function and help's topic-level merge — this item was
     always scoped to path resolution and raw read/write only, not those
     per-kind formats. Unblocks the theme editor's save path (#2) and both
     authoring tools (#3) as soon as a theme file format lands; the theme
     picker still additionally needs #1's truecolour theme as data to offer.

## Adding a phase

When work here gets scheduled, add a `## Phase N` section following the format
`edit`'s roadmap used: the modules it touches, an interface sketch, and the
tests to write first — then copy `docs/module-spec-template.md` per module
before writing any code (ADR 0014).
