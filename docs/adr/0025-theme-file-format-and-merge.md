# ADR 0025 — Theme file format: dotted `key = value`, infallible merge via `Theme::with`

- **Status:** Accepted
- **Date:** 2026-07-03

## Context

ADR 0024 generalized *where* a resource's app/user override layers live
(`rvision::resource`), deliberately leaving each kind's own file format and
merge rule for its own pass — for themes, "a theme file format is a natural
extension of `edit`'s dotted `key = value` convention (e.g.
`editor_text.fg = rgb(30,30,46)`)... `Theme` already has exactly the merge
primitive needed (`Theme::with(role, style)`)". This ADR nails that down:
roadmap backlog #9's last open piece before the theme editor (#2) and theme
builder (#3) can actually load an app/user theme.

`rvision` already has an in-repo precedent for how a hand-authored, user-facing
text format should fail: help content's `HelpContents::parse` (ADR 0013) is
**infallible** — "malformed input degrades gracefully (an unknown directive
becomes text...); authoring mistakes... are caught by a content test, not a
runtime error." A theme file is the same kind of artifact (hand-edited prose,
not a wire format between programs), so the same failure philosophy applies:
one bad line should cost that one override, not the whole theme.

## Decision

`Theme::merge(self, text: &str) -> Self` on `theme.rs`, line-oriented:

```
# a full-line comment; blank lines are also ignored
editor_text.fg = rgb(30, 30, 46)
editor_text.bg = light_gray
selection.attrs = bold|reverse
window_title.attrs = none
```

- One override per line: `<role_key>.<field> = <value>`, split on the first
  `.` (role keys and field names never contain one) and the first `=`;
  whitespace around either is trimmed.
- `role_key` is `Role`'s variant name in `snake_case` (`EditorText` →
  `editor_text`), looked up via a small hand-written table — not a runtime
  `PascalCase`→`snake_case` conversion (no case-conversion crate; the mapping
  is fixed and small, so a table is both cheaper and more obviously correct
  than a general algorithm).
- `field` is `fg`, `bg`, or `attrs`.
- A colour value (`fg`/`bg`) is `default` (→ `Color::Default`), a `Color16`
  variant's `snake_case` name (→ `Color::Named`, e.g. `light_gray`), or
  `rgb(r, g, b)` with decimal `u8` components (→ `Color::Rgb`) — the exact
  form ADR 0024's own example already used.
- An `attrs` value is `none` or a `|`-combined list of `bold`, `dim`,
  `italic`, `underline`, `reverse`, `blink`, matching `Attributes`'
  constants.
- **Merge is per-field, not per-role wholesale.** For each parseable line,
  `merge` reads the role's *current* style (from `self` — the layer
  underneath), overwrites just the one field the line names, and calls
  `Theme::with(role, style)`. A file that sets only `editor_text.fg` leaves
  `editor_text`'s background and attributes exactly as the layer beneath left
  them; it does not reset them to `Style::default()`. This is what makes
  layering actually useful (a user file overriding one colour doesn't have to
  restate the other two fields).
- **Infallible, matching `HelpContents::parse`'s precedent.** A line that
  doesn't split on `.`/`=`, names an unrecognised role/field, or has an
  unparseable value is skipped — that one override doesn't apply, nothing
  else is affected, and there's no error type to define, propagate, or test
  for exhaustiveness. Authoring mistakes are a content-correctness concern
  (a future theme editor/linter can flag them), not a runtime failure mode.
- Applying a layer is `theme = theme.merge(&layer_text)` per layer, in order
  (framework default, then app, then user) — exactly the fold `load_layers`'s
  two optional strings are for; `rvision::resource` itself still knows
  nothing about this format (ADR 0024 stands).

## Consequences

- The theme editor (#2) and theme builder (#3) can now turn
  `resource::load_layers`'s raw `app`/`user` strings into an actual `Theme`:
  `Theme::default().merge(app.as_deref().unwrap_or("")).merge(user.as_deref().unwrap_or(""))`.
- Still open, deliberately deferred to #2/#3's own spec: how an editor
  *produces* this format's text to hand to `resource::write_user_resource` —
  a full dump of all 19 roles' 3 fields, or a diff against the layer beneath
  so a saved file only records what the user actually changed. `merge`'s
  contract doesn't care which a caller chooses; this ADR only fixes the
  reading/parsing half, matching what roadmap #9 actually left open ("the
  theme file format and its merge function").
- Help's topic-level merge remains its own separately-deferred design (ADR
  0024), unrelated to this ADR beyond sharing the same infallible-parsing
  philosophy.
- A theme file with a typo (`editor_txt.fg = ...`) silently drops that one
  line rather than erroring the whole load. Accepted deliberately, mirroring
  `HelpContents::parse` exactly — the alternative (a strict parser) means one
  bad line in a hand-edited file blanks an entire layer back to the layer
  beneath, which is a worse failure mode for this kind of artifact.

## Alternatives considered

- **A strict `Result<Theme, ThemeParseError>` parser.** More conventional for
  a "real" file format, but inconsistent with `HelpContents::parse`'s
  established precedent in this same codebase, and a strictly worse failure
  mode for a hand-edited text file: one typo aborts the whole layer instead of
  costing one override.
- **One line per role with all three fields** (e.g.
  `editor_text = rgb(255,255,255), rgb(0,0,170), bold`). Rejected: less
  diffable/hand-editable (a one-colour tweak still touches a line describing
  three things), and doesn't match ADR 0024's own worked example, which was
  already per-field.
- **`serde`-backed TOML/JSON.** Rejected outright by the crate budget (ADR
  0001/0006) without its own ADR to add a dependency; also heavier than a
  format this simple needs.
- **Runtime `PascalCase`→`snake_case` conversion for role keys** (e.g. via a
  small hand-rolled algorithm) instead of a lookup table. Rejected: `Role` has
  19 fixed variants that essentially never change shape; a table is more
  obviously correct (no acronym/boundary edge cases to get right) for a
  one-time, small mapping.
