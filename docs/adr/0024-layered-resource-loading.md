# ADR 0024 — Layered resource loading: shared path resolution, per-kind format & merge

- **Status:** Accepted
- **Date:** 2026-07-03

## Context

This item was raised while scoping true colour support (#1, ADR 0023): both
themes and help content (ADR 0013) want to move from Rust-embedded data toward
files loaded at runtime, with three levels of override precedence — framework
defaults, overlaid by application defaults, overlaid by user customisation —
auto-loaded at bootstrap.

`edit` already solved the adjacent problem for a single app, single layer (its
own user config only): a hand-rolled `key = value` format and per-OS
config-directory resolution (XDG on Linux, `Application Support` on macOS,
`%APPDATA%` on Windows), both pure and injectable for testing, no `serde`/`dirs`
crate (crate budget, rvision ADR 0001/0006; see `edit`'s ADR 0025 and
`edit::settings`). This ADR generalizes the *reusable* half of that pattern into
`rvision` — but the three layers turn out not to be symmetric:

- `rvision` is a library crate. It has no install location of its own once
  compiled into a consuming app's binary, so any "framework-shipped" resource
  can only be embedded at compile time (`include_str!`) — exactly what
  `Theme::default()`/`help.rs` already do. There is no runtime path to
  discover for this layer.
- An app's own default resources depend entirely on how that app is packaged
  (a Linux package's `/usr/share`, a macOS bundle's `Resources` folder, files
  sitting beside a Windows `.exe`) — conventions `rvision` cannot guess, and
  guessing would cut against the project's existing explicit-over-magic style
  (`Shell::new` already takes an explicit `&Theme` rather than reaching for a
  global).
- Only the user-customisation layer has a genuinely universal, OS-level
  convention (the platform config directory) — exactly what `edit::settings`
  already solved.

## Decision

`rvision::resource` provides the one genuinely generic piece: locating and
reading the **app** and **user** layers as raw text, given an app name, a
per-resource-kind file name, and (for the app layer) an app-author-supplied
directory. The **framework** layer isn't modelled by this module at all — it
stays exactly what it is today (a Rust value, or an `include_str!`'d parse),
since embedding is the only distribution mechanism a library crate has.

```rust
pub struct ResourceLayers { pub app: Option<String>, pub user: Option<String> }
pub fn load_layers(app_name: &str, file_name: &str, app_dir: Option<&Path>) -> io::Result<ResourceLayers>;
pub fn user_resource_path(app_name: &str, file_name: &str) -> Option<PathBuf>;
```

`user_resource_path` generalizes `edit::settings::config_path` with explicit
`app_name`/`file_name` parameters in place of `edit`'s hardcoded `APP_DIR`/
`FILE_NAME` constants; the per-OS resolvers and env-var injection carry over
directly.

Each resource **kind** owns its own file format, parser, and merge rule on top
of these raw layers — no generic `Resource`/`parse`/`merge` trait. `Theme`
already has exactly the merge primitive needed (`Theme::with(role, style)`,
applied once per override found in a layer); a theme file format is a natural
extension of `edit`'s dotted `key = value` convention (e.g.
`editor_text.fg = rgb(30,30,46)`). Help content's merge (by topic `id`) doesn't
exist yet and is designed when help moves onto this loader. Building a shared
trait now, for two consumers with genuinely different merge shapes, is exactly
the premature abstraction the project avoids elsewhere (three similar lines
beat a guessed-at abstraction) — revisit if a third consumer makes the
commonality concrete.

Per the scoping discussion that produced this ADR, the user layer is **one
file per resource kind** (`~/.config/<app>/theme`, `~/.config/<app>/help-overrides`,
...) rather than one omnibus sectioned file — keeps each kind's format
independent and lets a user edit or delete just one thing.

## Consequences

- Reuses a proven pattern (`edit` ADR 0025) instead of inventing path
  resolution from scratch; the per-OS logic, env-var injection for
  testability, and "missing file/dir is not an error" leniency all carry over.
- No new crate; format parsing stays hand-rolled per kind, consistent with the
  rest of the framework.
- The loader is agnostic to *what* it's loading — themes, help, and any future
  kind all go through the same `load_layers`, keeping the generic surface
  small and stable while each kind's format/merge logic evolves independently.
- Left open, deliberately, for the next scoping pass: the exact theme file
  format and its merge function; help's topic-level merge; whether an
  env-var override (mirroring `edit`'s `$EDIT_CONFIG_PATH`) is worth
  generalizing per-kind or per-app; and whether/how `edit`'s own existing
  bespoke `settings.rs` might eventually migrate onto this (not required —
  mirrors ADR 0016's stance that `edit` adopts new `rvision` facilities at its
  own pace).

## Addendum (2026-07-03): write-back, and the env-var override question closed

Two of the items this ADR left open turned out to need resolving before the
spec ([`docs/specs/resource.md`](../specs/resource.md)) could go into TDD:

**Write-back was missing entirely.** The original decision only covered
*reading* — but #2's theme editor writes to the user layer and #3's theme
builder writes to the app-defaults layer, so a resource loader that can't
write is only half the tool those two items need. Resolution: the **user**
layer gets a symmetrical `write_user_resource`, mirroring
`edit::settings::save`'s "create the directory, then write" shape, because it
pairs with real, non-trivial path-resolution logic (`user_resource_path`'s
per-OS rules) that's already centralized on the read side — a caller
shouldn't have to re-resolve that path and hand-roll `create_dir_all` +
`fs::write` itself just to save what it loaded. The **app-defaults** layer
gets no such helper: its caller (an app author, e.g. the theme builder)
already holds the exact directory it supplied as `app_dir` — writing there is
a bare `fs::write`, not resource-loading logic, and two known callers doing
three lines each is the "three similar lines" the project already prefers
over a guessed-at abstraction.

```rust
pub fn write_user_resource(app_name: &str, file_name: &str, contents: &str) -> io::Result<()>;
```

A no-resolvable-path case (same condition under which `user_resource_path`
returns `None`) is a silent no-op, exactly matching
`edit::settings::Settings::save`'s existing behaviour, not an error.

**The env-var override: decided against.** `edit::settings::config_path`
supports `$EDIT_CONFIG_PATH`, an explicit whole-file-path override, useful
both as a power-user escape hatch and as edit's own test seam. Generalizing
it into `rvision::resource` would mean deriving an env-var name from a
runtime `app_name` string (e.g. `{APP_NAME}_RESOURCE_DIR`) — a naming
convention `rvision` would be inventing and imposing, not one an app author
chose. That cuts against this ADR's own explicit-over-magic stance (the same
reasoning that already ruled out `rvision` auto-discovering the app layer's
location). It's also unnecessary: `user_resource_path` and
`write_user_resource` already take explicit `app_name`/`file_name`
parameters, so any app that wants an override can check its own env var
first and only fall through to `user_resource_path` when unset — no module
support required. Left as documented guidance for callers, not a feature of
`rvision::resource` itself.

## Alternatives considered

- **A generic `Resource` trait (`parse` + `merge`) with a loader generic over
  `T`.** More "finished-looking," but premature: only two consumers exist
  today with different merge shapes (per-role vs. per-topic), so the
  abstraction's shape would be a guess. Revisit once a third consumer exists.
- **One omnibus per-app config file, sectioned by kind.** Rejected: forces
  unrelated formats (theme's flat key-values vs. help's block markup) into
  one grammar, and one hand-edit mistake risks the whole file rather than one
  kind.
- **`rvision` auto-discovering the app layer's location** (e.g. next to the
  executable). Rejected: packaging conventions vary too much across
  platforms/distribution methods for `rvision` to guess correctly, and it cuts
  against the project's existing explicit-over-magic style.
