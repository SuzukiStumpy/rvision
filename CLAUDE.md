# CLAUDE.md

Working guide for this repository. Read `docs/adr/` for *why* the big
decisions went the way they did.

## What this is

A hand-rolled TurboVision-style terminal UI framework (`rvision`), extracted
from the [`edit`](https://github.com/SuzukiStumpy/edit) text editor once it
needed a life of its own. Still a Rust learning project at heart: build as
much as practical ourselves; reach for a crate only at the OS/terminal
boundary or for Unicode data tables. No editor knowledge belongs here — this
crate stays reusable by any terminal application.

## Layout

- `src/` — the framework.
- `examples/` — manual-verification demos (`cargo run --example <name>`).
- `docs/adr/` — one numbered Architecture Decision Record per major decision.
- `docs/adr/README.md` - index of ADRs.  Update this whenever creating or
  amending ADRs.
- `docs/specs/` — one spec per module.
- `docs/module-spec-template.md` — copy this before building any new module.

## Non-negotiables (the decisions, in short)

- **Crate budget.** Runtime: `crossterm`, `unicode-width`,
  `unicode-segmentation`. Dev: `insta`. Adding anything else needs a new ADR.
  (ADR 0001, 0006.)
- **The seam above crossterm.** The framework never calls crossterm directly.
  It draws into an in-memory `Cell` back-buffer; a `Backend` flushes it and an
  `EventSource` supplies events. `CrosstermBackend` for real use, `TestBackend`
  for tests. (ADR 0002.)
- **Retained-mode view tree.** Parent owns children (`Vec<Box<dyn View>>`).
  Views never hold references to each other: commands bubble **up** the owner
  chain, broadcasts travel **down**, identity is via integer IDs. (ADR 0003.)
- **Three-phase events.** Positional → focused → broadcast; modal dialogs run
  via `exec_view`; "handled" is a returned `EventResult`, never a mutated event.
  (ADR 0004.)
- **Colour by role.** Views ask for semantic roles resolved against a `Theme`;
  the cell colour type is truecolour-ready but themes ship 16-colour CGA first.
  (ADR 0005.)
- **Full Unicode.** A cell holds a grapheme cluster + display width; cursor
  movement steps by grapheme. (ADR 0006.)
- **No `unsafe`.** `rvision` sets `#![forbid(unsafe_code)]`; crossterm owns the
  FFI.
- **Panic-safe terminal.** Startup installs an RAII guard + panic hook so a
  crash always restores the terminal (cooked mode, leave alternate screen).
- **Single-threaded sync loop.** `poll(timeout)` → `read()`; the timeout drives
  idle/blink/resize. No async, no tokio.

## How we work

- **TDD, always.** Red → green → refactor. Write the failing test first.
  - *Logic* (geometry, buffers, dispatch): plain `#[test]`.
  - *Rendering*: draw into a `TestBackend` and assert with `insta` snapshots.
  - *Interaction*: feed a scripted event sequence, assert screen + model state.
  - *Manual*: `examples/` demos, run for real (colours, feel).
- **Per-module process.** Copy `docs/module-spec-template.md`, fill it in
  (purpose, public interface, invariants, test list), *then* write tests, *then*
  code. Record any design decision worth keeping as a new ADR.
- **Docs.** Rustdoc on every public item (`#![warn(missing_docs)]` is on).
  Keep the relevant ADR/spec updated when a decision changes.
- **Commits.** Conventional Commits — `feat: …`, `fix: …`, `feat!:` / a
  `BREAKING CHANGE:` footer for a major bump.

## Commands

```sh
cargo test                 # everything
cargo clippy --all-targets # lints
cargo fmt                  # format
cargo doc --open           # API docs
cargo insta review         # review pending snapshot changes
```

## Releasing

`release-please` runs on every push to `main`, maintaining an open release PR
from Conventional Commits that bumps `Cargo.toml`'s version and
`CHANGELOG.md`. Merging that PR tags `vX.Y.Z` and cuts a GitHub Release —
that's the entire release act today; crates.io publishing isn't wired in yet
(see `docs/roadmap.md`). Details: ADR 0022.

## Style

- Match the surrounding code's idiom and comment density. Comments explain
  *why*, not *what*.
- Hand-rolled error types implementing `std::error::Error` — no `thiserror` /
  `anyhow`.
- Keep `rvision` free of any editor-specific concepts.
