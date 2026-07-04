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

1. ~~**True colour support.** Currently limited to the original EGA/CGA
   palette. ADR 0005 already left the cell colour type truecolour-ready;
   themes ship 16-colour CGA first. Needs to land before the colour/theme
   dialogs below, which depend on the underlying colour model.~~
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
   - Landed 2026-07-03: the "ship a real theme" half, now that #9 is done.
     [`examples/themes/truecolour.theme`](../examples/themes/truecolour.theme)
     is a hand-authored dark palette in ADR 0025's file format — 19 roles'
     `fg`/`bg` as `rgb(...)` overrides, each role's `attrs` left alone since
     the CGA layer beneath already sets what's needed (e.g. `WindowTitle`'s
     bold). It stays example/app-layer data, never a hardcoded Rust
     constructor, per this item's own scoping note above.
     [`examples/truecolour.rs`](../examples/truecolour.rs) wires it end to
     end: `ColorProfile::detect()` picks `Theme::default()` alone on
     `Cga16`, or `Theme::default().merge(text)` on `Truecolor` — a real
     graceful fallback, not just the capability check in isolation. Also
     gained a `--dump-theme` flag (prints every role's resolved colour via
     `Theme::format_field`, no terminal takeover) — used to diff the two
     profiles' output and confirm all 19 roles' `fg`/`bg` actually
     overrode (`Theme::merge`'s per-line parsing is infallible, ADR 0025, so
     a typo'd role/field key would otherwise fail silently rather than
     erroring). With this, roadmap item #1 is fully landed; the theme
     picker sub-item of #2 now has a second theme to pick between.
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
     — manual terminal pass done while building the theme editor below
     (ADR 0026, which drives this via mouse): found and fixed a bug where
     clicking *OK* posted `CM_OK` via the plain `Button` widget without ever
     calling `ColorPicker::accept`, so the result handle stayed at its
     never-set `Color::Default` regardless of the selected swatch — only
     `Enter` updated it correctly. The theme picker sub-item remains blocked
     as scoped above.
   - ~~Theme editor.~~ Landed 2026-07-03:
     [`widgets::ThemeEditor`](../src/widgets/theme_editor.rs) — browse all 19
     `Role`s in a list, edit the selected one's foreground/background via a
     nested `ColorPicker` and its attributes via checkboxes, then save.
     Tracks exactly which fields were touched and serializes only those via
     `Theme::format_field` (a diff against the starting theme, not a full
     19-role dump, per this item's earlier open question — resolved in
     favour of the diff). Hosted as an ordinary `Desktop` window (chrome
     locked to read as a dialog) rather than run via `exec_view`, since a
     `View` can't open the nested `ColorPicker` itself — see ADR 0026 for the
     composition mechanism (a bubbled `CM_EDIT_FG`/`CM_EDIT_BG`, and a
     read/write `ThemeEditorHandle` generalizing `ColorPickerResult`'s
     read-only idiom). See
     [`docs/specs/theme_editor.md`](specs/theme_editor.md) and the
     `theme_editor` example — manual terminal pass done: colour edits and
     attribute toggles reflect immediately, the selected role stays
     highlighted while focus is on Foreground/Background/Save/Cancel, and a
     saved diff round-trips through `rvision::resource` on the next run —
     including the example's *own* chrome (Desktop backdrop, the editor's own
     window/list/buttons), which loads and merges the saved layer at startup
     the same way a real application would, not just the value being edited.
     Also gained a **Restore Defaults** control (hand-drawn/hand-dispatched
     like `ColorPicker`'s mode toggle, posting no command): resets the whole
     session to the framework default in one step, marking touched only the
     fields that actually need an override to win back over `base` — a panic
     button, not a per-role undo.
   - Unblocked 2026-07-03: #1 now ships an actual second theme
     ([`examples/themes/truecolour.theme`](../examples/themes/truecolour.theme)),
     so the theme picker sub-item has something to pick *between* and is no
     longer scope-blocked as noted above — still unscheduled/undesigned,
     just no longer waiting on a dependency.
   - ~~Theme picker.~~ Landed 2026-07-03:
     [`widgets::ThemePicker`](../src/widgets/theme_picker.rs) — a candidate
     list (name + already-built `Theme`, supplied by the caller; the widget
     never touches `rvision::resource` itself, mirroring `ColorPicker`/
     `ThemeEditor`'s own boundary) with a live preview panel that redraws
     from the highlighted candidate's own styles as the highlight moves, no
     separate activate step. Not an editor — it hands back a whole `Theme`,
     unlike `ThemeEditor`'s per-role diff. Proactively closed the same gap
     `ColorPicker` shipped with and later had to fix (`Enter`, `Space`, and a
     mouse click on *OK* all route through one `accept`, never through
     `Button::handle_event`'s own bare `CM_OK` post). Built via `pick`/
     `exec_view` like `ColorPicker`/`FileDialog`, not `Desktop`-hosted like
     `ThemeEditor` — nothing here needs ADR 0026's nested-window composition
     since there's no second dialog to open. See
     [`docs/specs/theme_picker.md`](specs/theme_picker.md) and the
     `theme_picker` example, which offers "CGA (default)" always and
     "Truecolour" (`examples/themes/truecolour.theme`) only when
     `ColorProfile::detect()` reports `Truecolor` — mirroring
     `examples/dialogs.rs`'s colour-picker gating — and actually applies the
     picked theme to its own closing screen, not just to the dialog's own
     preview.
3. **Utility programs.** Help authoring tool; theme builder; possibly more.
   - Scoped 2026-07-03: the theme builder is a developer-facing tool for
     authoring a theme to ship *with* an application, most likely a thin
     wrapper reusing #2's theme editor dialog/component but pointed at
     the app-defaults resource layer (ADR 0024) instead of the user layer
     the in-app editor writes to by default. Same editing surface, two
     different output layers — see #2's scoping note.
   - ~~Theme builder.~~ Landed 2026-07-03: `examples/theme_builder.rs` — the
     same driving loop as the `theme_editor` example, seeded from
     `Theme::default()` alone (no user layer beneath the app layer) and
     saved with a bare `fs::write` to a CLI-supplied app-resources
     directory, exactly per this item's own scoping note. Some driving-loop
     duplication between the two examples is accepted deliberately
     (application glue, not library code).
   - Scoped 2026-07-03: the help authoring tool needs no new format — ADR
     0013 already designed the `parse(&str) -> HelpContents` boundary
     specifically so a future authoring tool could target it, and the
     tool live-previews through the existing `HelpPane` renderer. Same
     shape as the theme builder: a thin wrapper that saves through #9's
     resource loader to the app-defaults layer. Its editing pane needs a
     multi-line text-entry control, which doesn't exist yet — see #6.
   - ~~Help authoring tool.~~ Landed 2026-07-04: `examples/help_builder.rs` —
     a raw-markup `TextArea` source pane beside a live preview through the
     existing `HelpPane` renderer, via an ordinary, unmodified
     `HelpWindow::build` (not a new composite) closed and reopened on an
     explicit **Refresh Preview** (`Ctrl+R`) rather than reparsed on every
     keystroke — so a half-typed `{label|target}` link is never shown broken
     mid-edit, and link-following in the preview genuinely works via
     `HelpWindow`'s own already-tested mechanics, for free. Saving diverged
     from this item's own scoping note above once actually built: rather
     than a fixed `<app-resources-dir>`/`RESOURCE_NAME` pair mirroring the
     theme builder, it opens a `widgets::FileDialog` **Save As** dialog the
     first time (or any time, via the File menu), remembering the chosen
     path so subsequent `Ctrl+S` saves silently — saving is deliberately
     never a quit signal, so progress can be saved repeatedly mid-session.
     The source window's title grows a trailing `" *"` while there are
     unsaved edits (`edit`'s own dirty-file convention), which needed one
     small, generic library addition: `Frame::set_title`/`Window::set_title`
     (`docs/specs/window.md`) — a runtime title setter alongside the
     existing `set_active`-style setters, additive in the same family as
     `with_help_topic`, no new ADR. No new `src/widgets` composite either:
     unlike the theme editor, this item was only ever scoped as a utility
     program (#3), not a standard dialog (#2), so the whole tool is
     application-layer glue over already-tested pieces (`TextArea`,
     `HelpPane`/`HelpWindow`, `HelpContents::parse`, `FileDialog`), verified
     manually (including a real resize, drag, Save/Save-As/Cancel, and
     CLI-argument round-trip pass) rather than with `#[test]` — same
     precedent as the theme editor/builder examples. Still open: the preview
     only reflects the state as of the last refresh and always reopens on
     the home topic; there is no unsaved-changes prompt on quit — accepted
     simplifications, not oversights.
   - Follow-up 2026-07-04: the first cut only let a file be *loaded* via the
     CLI argument at startup — nothing let a user load a different existing
     file once the tool was already running. Added File ▸ Open... (`Ctrl+O`),
     a `widgets::FileDialog::open` opened/tracked the same way as Save As
     (unified into one `PendingDialog { window_id, result, action }` plus a
     `PendingAction::{Save, Open}` tag, replacing the Save-only `PendingSave`,
     since `CM_OK`/`CM_CANCEL` are shared and only one dialog kind can be
     pending at a time either way). Loading replaces the source pane's text
     outright and becomes the path subsequent `Ctrl+S` saves target — no
     unsaved-changes prompt, the same accepted simplicity as Exit above — and
     deliberately does *not* auto-refresh the preview, keeping Refresh the
     one explicit "reflect the source now" action everywhere, not just after
     typing.
   - Follow-up 2026-07-04: authoring `examples/help_builder.help` (a usage
     guide for the tool itself) surfaced a real gap in the *format*, not
     just the example — no way to show `@topic`/`<pre>`/`{label|target}`
     literally. Landed a backslash-escape syntax in the core parser
     (`src/help.rs`, ADR 0029): `\@`/`\<`/`\#` at line start and `\{`/`\\`
     inline, everything else left untouched, `<pre>` content never
     escape-processed. Flagged but explicitly deferred, larger scope: a
     Markdown-style code-span/code-block syntax (single backtick inline,
     triple-backtick multi-line) — not designed or scheduled.
   - Follow-up 2026-07-04: manual testing right after the above surfaced a
     real parser bug, not just a missing escape — a `<pre>` block that *is*
     properly closed a line or two later was still splitting into two
     topics because an `@topic`-shaped line inside it ended the block early
     regardless of the real close sitting right there. Fixed in
     `HelpContents::parse` (`src/help.rs`, ADR 0029's addendum): `<pre>` now
     scans forward for its own `</pre>`, however far away, tolerating one
     topic-shaped (or bare `<pre>`) line along the way before falling back
     to the original "genuinely unclosed, recover at the next real topic"
     behaviour — which a new regression test locks in, so a topic that
     forgets its own close still can't reach across and swallow a *later*
     topic's own well-formed `<pre>` block.
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
   - ~~TextArea.~~ Landed 2026-07-03:
     [`widgets::TextArea`](../src/widgets/text_area.rs) — see
     [`docs/specs/text_area.md`](specs/text_area.md). Two design points
     shifted during implementation, both recorded in the spec:
     - **Reflow is not `wrap.rs`.** The plan above named `wrap.rs` as the
       reflow mechanism, but `wrap::wrap` deliberately collapses runs of
       whitespace to one separator — correct for read-only prose
       (`HelpPane`, `MessageBox`), wrong for an editable buffer (typing two
       spaces would watch one vanish, and the displayed text would stop
       matching what's stored byte-for-byte). `TextArea` reflows with its
       own whitespace-preserving `reflow`, sharing only `wrap::word_offsets`
       (word boundaries with byte offsets, promoted to `pub(crate)`) with
       `wrap.rs` — one algorithm for word boundaries, two different
       policies for what happens around them. `wrap::wrap_with_offsets`
       (an offset-tracking sibling added mid-investigation, before the
       collapsing problem surfaced) stayed in `wrap.rs` regardless — `wrap`
       is now a thin map over it, so `HelpPane`/`MessageBox`'s own path
       gained one algorithm instead of two for free.
     - **Navigation grew past the original scope, and so did `InputLine`.**
       A backlog-planning follow-up added `Ctrl+Left`/`Ctrl+Right`
       word motion, `Ctrl+Home`/`Ctrl+End`, and Shift-extended selection —
       and asked for the same on `InputLine` too (except `InputLine::set_text`
       keeps resetting to the **end**; only `TextArea::set_text` defaults to
       the **start**, via a new `CursorPosition` enum with `_at` variants for
       the other end). The shared mechanics — grapheme ops, word motion,
       selection-range/collapse — moved out of `InputLine` into a new
       `widgets::text_edit` free-function module (per this item's own
       "share as free functions" call above), which `TextArea` builds on
       too. See the updated [`docs/specs/controls.md`](specs/controls.md).
     Manual pass: `examples/text_area.rs`, run in a real terminal (tmux) —
     typing/backspace, `Ctrl+Home`/`Ctrl+End`, Shift-select-then-replace,
     and the hosted scroll bar's thumb all confirmed working; `Window`
     hosts the bar generically via `scroll_metrics` (ADR 0015) with no
     `TextArea`-specific code in `Window` at all.
     - Follow-up 2026-07-03: the manual pass surfaced two real problems, one
       in the library and one in the example. **Example bug, fixed:** the
       first cut hosted the demo window via `Application::exec_view` (the
       `dialogs` example's modal pattern), where dragging, resizing, and the
       close glyph silently do nothing — `Window` "has no concept of a drag
       session" outside a `Desktop` (ADR 0016 — the framework's own
       comment, not a gap this landing introduced). Rebuilt on
       `Shell`/`Desktop`/`Root` (the `chrome` example's pattern) so those
       actually work, with `Alt-X`/File ▸ Exit as a guaranteed quit
       independent of the window's own close button (closing a window only
       removes it from the desktop; it was never going to quit the app).
       That rebuild initially reintroduced a *different* bug — forgetting
       `TextArea::set_focused(true)` on the lone interior view, since a
       `Window`'s interior isn't auto-focused just by its window being the
       desktop's active one (that's `Window::set_active`'s frame styling,
       a separate concept from a view's own `focused` flag — only a
       `Group` auto-focuses a first child, ADR 0010, and there's no `Group`
       here) — caught by the same re-run, not shipped. **Library bug,
       fixed:** a display line packed to *exactly* the box's width left the
       caret one column past the last visible one on `End` — invisible, not
       just misplaced.
     - Second follow-up 2026-07-03: a second manual pass (after the above)
       found the *example* still truncating a wrapped word mid-character
       ("this wind" instead of wrapping "window" whole) and no scroll bar or
       working wheel until the window was actually resized once. Root cause
       was one bug, not two: the example passed the *window's own* outer
       bounds to `TextArea::new` instead of `Window`'s inset-by-one
       *interior* bounds — `Window::new`/`styled` don't size the interior
       for the caller at construction (only a later resize/zoom
       re-propagates it via `set_bounds`, ADR 0017), so this is on the
       caller, the same way `HelpWindow::build` computes its own
       `interior_size` before constructing what it wraps. `TextArea` was
       reflowing/scrolling against a size two columns/rows bigger than what
       `Window` actually draws it into — explaining the truncation (reflow
       thought "window" fit) and the missing scroll bar (`scroll_metrics`
       thought the taller phantom height didn't need one) in one stroke.
       Fixed in the example. Separately, prompted by "there should ... at
       all times [be] one character padding": the previous roll/clamp caret
       fix was a reactive patch for a symptom, not the cause — replaced with
       a real invariant. `TextArea` now reflows to `bounds.width() - 1`,
       *always* reserving the box's last column, so a display line can
       never reach the true right edge in the first place and the caret
       always has a real column at true end-of-line. The roll/clamp code
       from the prior follow-up became unreachable dead weight once this
       landed and was removed outright; its two tests were replaced with
       ones asserting the actual invariant (a full-width-content line still
       wraps one word early; the caret needs no special-casing to be
       visible).
     - Third follow-up 2026-07-03: a third bug, this one a real correctness
       gap in `reflow` itself, not the example — typing several trailing
       spaces at the end of a line didn't visibly advance the cursor at all
       until a following non-whitespace character was typed, at which point
       everything "caught up" at once. Cause: `reflow`'s per-word packing
       loop only ever measured a gap that came *before* a word — trailing
       whitespace with no word after it (including a hard line that's
       nothing but whitespace) was copied onto the current display line
       verbatim with no width check at all, so it could grow arbitrarily far
       past the box's width. The canvas draw loop still clips at the true
       edge, so both the excess spaces and a cursor sitting among them
       simply stopped being drawn — until a new word finally gave the
       packing loop something to size against, forcing a real wrap that
       revealed everything in one jump. Fixed: the tail after the last word
       (or the whole hard line, unified as the same case when there are no
       words at all) is no longer copied in one shot — it's now packed one
       grapheme at a time and wrapped onto its own continuation line(s) once
       it would overflow, exactly like a real terminal cursor advancing past
       the right margin. Unlike a word, a run of whitespace has no reason to
       stay whole, so this is free to split at any point (interior gaps
       *between* two words were left unaffected at the time — still eliding
       wholesale at a wrap point, unchanged — which turned out to be wrong
       too; see the next follow-up). Two `reflow`-level tests plus one
       `TextArea`-level test added; 633 tests pass.
     - Fourth follow-up 2026-07-03: reported again — "still not behaving...
       exactly the same as before." The third follow-up's fix was real but
       incomplete: it only covered the *trailing* tail; typing whitespace
       into an *interior* gap (between two words already on the same line —
       e.g. placing the cursor right after "hello" in "hello world" and
       typing spaces there) hit the untouched wholesale-elision path and
       showed the identical symptom. Debugged by reproducing the exact
       scenario at the unit level (not just re-reading code): the model's
       `cursor`/`text` update correctly on every keystroke, but
       `display_pos` — which maps a grapheme cursor to a screen
       row/column — has no way to represent a position *inside* an elided
       gap, since those bytes have no display line of their own at all. It
       fell back to the end of the previous line for every such position,
       so the caret read as frozen no matter how many spaces went in, until
       a following word forced a genuine re-wrap and the caret jumped to
       wherever that word ended up landing — reading as everything
       "catching up at once." Once diagnosed, eliding *any* gap outright
       (not just a long one) was the wrong call from the start: `reflow`
       now never elides a gap, interior or trailing — `place_gap` (a helper
       used for both) splits whichever it's given across continuation lines
       the exact same way, so every byte of `text` always lands on a real,
       addressable display line. The visible cost is that a single
       separator that doesn't fit now gets a line of its own instead of
       silently vanishing (matches this control's own "never lose
       whitespace" principle better than the old wholesale-elision
       shortcut did anyway, which was borrowed from `wrap::wrap`'s
       coarser, collapsing prose model without re-examining whether it
       still fit a byte-exact one). Five existing `reflow`/`TextArea` tests
       whose expectations encoded the old elision updated; a new
       `TextArea`-level regression test asserts the caret's display
       position changes on the very first and second keystrokes typed into
       an interior gap, not just eventually. 634 tests pass.
7. ~~**Insert/overtype support** for text entry controls.~~ Landed
   2026-07-03: `InputLine` gained an `overtype: bool` field (default off,
   matching TurboVision); `KeyCode::Insert` toggles it. While on, a
   printable `Char` overwrites the grapheme under the cursor via a new
   `overwrite` op (delete-then-insert, so it reuses `insert`'s
   combining-mark/cursor-advance handling) instead of pushing text right,
   falling back to a plain insert past the end so overtype can still
   extend the line. See [`docs/specs/controls.md`](specs/controls.md).
   The new TextArea (#6) picks up the same toggle once it lands.
   - Follow-up 2026-07-03: manual pass surfaced that the two modes looked
     identical — the caret was always a reverse-video block, so nothing on
     screen showed which mode was active until you typed. Fixed: the
     caret's attribute now follows the mode, underline for insert and the
     original reverse-video block for overtype, matching a real terminal
     cursor's block-vs-bar convention.
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
   inventing one. Gated (now cleared, see below, for themes specifically —
   help's own topic-level merge is still its own deferred pass) the "ship a
   real theme" half of #1 and the theme/help authoring tools in #3.
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
   - ~~Theme file format + merge function, landed 2026-07-03.~~ ADR 0025:
     `Theme::merge(self, text: &str) -> Self` applies dotted
     `role.field = value` overrides (e.g. `editor_text.fg = rgb(30,30,46)`)
     per-field onto the layer beneath — layering is
     `Theme::default().merge(app).merge(user)`. Infallible, matching
     `HelpContents::parse`'s precedent (ADR 0013): an unparseable/unknown
     line is skipped rather than erroring the whole layer. Still open,
     deliberately out of this item's scope: how the theme editor (#2)/theme
     builder (#3) *serialize* an edited `Theme` back into this format to hand
     to `write_user_resource` (full dump vs. diff-against-the-layer-beneath)
     — left to their own spec, since `merge`'s contract doesn't care which a
     caller chooses. Help's topic-level merge remains separately deferred.
     With this, #9's own scope is complete; #2/#3 can now actually load a
     second theme, though the theme picker still needs #1's truecolour theme
     as data to offer before it has anything to pick between.

## Adding a phase

When work here gets scheduled, add a `## Phase N` section following the format
`edit`'s roadmap used: the modules it touches, an interface sketch, and the
tests to write first — then copy `docs/module-spec-template.md` per module
before writing any code (ADR 0014).
