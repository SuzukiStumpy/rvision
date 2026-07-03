# Module spec: `rvision::resource`

- **Status:** Done. Landed 2026-07-03 as [`resource.rs`](../../src/resource.rs)
  per the test plan below: per-OS user-config-dir resolution
  (`unix_config_dir`/`macos_config_dir`/`windows_config_dir`, pure and
  unit-tested on every host regardless of target OS), `load_layers`/
  `write_user_resource`/`user_resource_path` each split into a thin
  env-reading wrapper plus a fully-testable core taking already-resolved
  paths (`load_layers_from`, `write_user_resource_to`) — this crate forbids
  `unsafe`, which rules out `std::env::set_var` as a test seam, so real-env
  behaviour is exercised only through the pure per-OS functions, mirroring
  `ColorProfile::detect`/`profile_from_env` (ADR 0023)'s impure/pure split.
- **Phase:** unscheduled (roadmap backlog #9)
- **Related ADRs:** 0024 (layered resource loading, see its 2026-07-03
  addendum for write-back and the env-var override decision)

## Purpose

Locate and read the **app** and **user** override layers of a named resource
(as raw text), so a resource kind (theme, help content, ...) can overlay them
onto its own framework-embedded default. This module knows nothing about any
resource kind's file format, parsing, or merge rules — only where its bytes
live. The framework layer is out of scope entirely: it's whatever the calling
kind already embeds at compile time (`Theme::default()`, an `include_str!`'d
help document), since a library crate has no runtime install location of its
own to discover (ADR 0024).

## Public interface

```rust
// pub struct ResourceLayers { pub app: Option<String>, pub user: Option<String> }
//
// pub fn load_layers(
//     app_name: &str,
//     file_name: &str,
//     app_dir: Option<&Path>,
// ) -> io::Result<ResourceLayers>;
//
// pub fn user_resource_path(app_name: &str, file_name: &str) -> Option<PathBuf>;
//
// pub fn write_user_resource(
//     app_name: &str,
//     file_name: &str,
//     contents: &str,
// ) -> io::Result<()>;
```

`app_dir` is supplied by the calling application (its own resources
directory) — this module never guesses an app's install layout. `user_resource_path`
generalizes `edit::settings::config_path` (see `edit`'s ADR 0025) with explicit
`app_name`/`file_name` in place of that module's hardcoded constants.
`write_user_resource` is the symmetrical write-back for that same layer,
generalizing `edit::settings::save`'s "create the directory, then write"
shape — resolved via the same path logic as `user_resource_path`, so a
caller never has to re-derive it. The app-defaults layer has no write helper:
its caller already holds the exact directory it passed as `app_dir`, so
writing there is a bare `fs::write`, not path-resolution logic worth
centralizing (ADR 0024 addendum, 2026-07-03).

## Behaviour & invariants

- A missing app or user file is not an error: the corresponding `ResourceLayers`
  field is `None`, exactly as a missing settings file is `Settings::default` in
  `edit::settings::load`.
- `app_dir: None` means the app supplied no resources directory at all — `app`
  is `None` unconditionally, no path is even attempted.
- User path resolution follows `edit`'s per-OS rules: Linux/BSD prefers
  `$XDG_CONFIG_HOME`, falling back to `$HOME/.config`; macOS uses
  `$HOME/Library/Application Support`; Windows uses `%APPDATA%`. An empty env
  var counts as unset (same as `edit::settings::nonempty`).
- Path resolution is a pure function of injected environment values (no direct
  `std::env` calls outside one thin wrapper), so every OS's rules are
  unit-tested on every host, matching `edit::settings`'s existing test shape.
- One file per resource kind (`file_name` is the whole kind, e.g. `"theme"`,
  `"help-overrides"`) — no omnibus multi-kind file (ADR 0024 scoping decision).
- `write_user_resource` creates the user layer's directory (`create_dir_all`
  on the resolved path's parent) before writing, exactly as
  `edit::settings::save_to` does. When `user_resource_path` resolves to
  `None` (no config directory found in the environment), `write_user_resource`
  is a silent no-op returning `Ok(())`, not an error — mirroring
  `edit::settings::Settings::save`.
- No env-var path override lives in this module (e.g. nothing generalizing
  `edit`'s `$EDIT_CONFIG_PATH`). Decided against: it would mean `rvision`
  inventing an env-var naming convention from a runtime `app_name` string,
  which is exactly the kind of guessed-at, app-imposed convention ADR 0024
  otherwise avoids. An app that wants one checks its own env var first and
  falls through to `user_resource_path`/`write_user_resource` when unset — no
  module support needed, since both already take explicit
  `app_name`/`file_name` (ADR 0024 addendum, 2026-07-03).

## Collaborators

Consumed by whatever loads a specific resource kind (a future theme loader,
a future help-content loader) — those own their file's format, `parse`, and
merge-onto-base logic. Depends on nothing in `rvision` itself; mirrors (but
does not depend on, and is not depended on by) `edit::settings`.

## Test plan (write these first)

- **Logic:**
  - Per-OS user path resolution (Unix XDG-then-HOME, macOS, Windows), each
    with injected env — mirror `edit::settings`'s test list, generalized for
    the `app_name`/`file_name` parameters.
  - Empty env vars count as unset.
  - `load_layers` with `app_dir: None` never touches the filesystem for the
    app layer.
  - `load_layers` where neither file exists returns both fields `None`, not
    an error.
  - `load_layers` propagates a real I/O error (not "not found") from either
    layer.
  - `write_user_resource` then `load_layers`/direct read round-trips the
    written contents (mirrors `edit::settings`'s
    `save_then_load_round_trips_through_a_temp_dir`).
  - `write_user_resource` creates a not-yet-existing directory (exercises
    `create_dir_all`, same shape as `edit::settings`'s equivalent test).
  - `write_user_resource` is a silent `Ok(())` no-op when no config
    directory resolves from the injected environment.
- **Manual:** none — no rendering or interaction surface.

## Open questions

- The theme file format and its `Theme`-merge function (extends `edit`'s
  dotted `key = value` convention; role → style, with `Theme::with` as the
  per-override primitive) — designed in its own pass, not here.
- Help content's topic-level merge (by `id`) — likewise, its own pass.
- Whether `edit`'s own `settings.rs` ever migrates onto this module — not
  required (ADR 0016 precedent: adoption at `edit`'s own pace).
