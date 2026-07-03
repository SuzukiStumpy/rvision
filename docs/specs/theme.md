# Module spec: `rvision::theme`

- **Status:** In progress
- **Phase:** 1 (core); roadmap backlog #9 (file format & merge)
- **Related ADRs:** 0005 (semantic roles over a swappable theme), 0024
  (layered resource loading), 0025 (theme file format & merge)

## Purpose

Map **semantic roles** (what a piece of UI *is*) to concrete [`Style`]s (how it
looks). Widgets ask for a role and never name colours, so swapping the theme
re-skins the whole UI. Ships one default 16-colour CGA theme.

## Public interface

```rust
pub enum Role { DesktopBackground, WindowFrame, WindowTitle, MenuBar,
                MenuSelected, MenuDisabled, StatusBar, StatusKey,
                ButtonNormal, ButtonFocused, EditorText, Selection }
impl Role {
    pub const ALL: [Role; _];     // every role, in discriminant order
    pub const COUNT: usize;       // == ALL.len()
}

pub struct Theme { /* [Style; Role::COUNT] */ }
impl Theme {
    fn style(&self, role: Role) -> Style;          // lookup
    fn with(self, role: Role, style: Style) -> Self; // override one role
}
impl Default for Theme { /* the CGA palette */ }

impl Theme {
    /// Applies dotted `role.field = value` overrides from a theme-file layer
    /// (ADR 0025) on top of `self`, one call per layer
    /// (`default().merge(app).merge(user)`). Infallible: unparseable/unknown
    /// lines are skipped, matching `HelpContents::parse`'s precedent (ADR
    /// 0013) — one bad line costs that one override, not the whole layer.
    pub fn merge(self, text: &str) -> Self;
}
```

## Behaviour & invariants

- A `Theme` stores one `Style` per role in an array indexed by `role as usize`;
  **`Role::ALL` must be in discriminant order** so the cast is a valid index
  (guarded by a test).
- `style` is total: every role resolves to a style.
- `with` returns a new theme with a single role replaced, others untouched.
- The default theme uses canonical CGA colours; values are provisional and may be
  tuned as widgets are built.
- `merge` overrides **per field, not per role**: a line only replaces the one
  field (`fg`/`bg`/`attrs`) it names, reading the role's other fields from
  `self` first — so a layer that sets only `editor_text.fg` doesn't reset
  `editor_text.bg`/`attrs` to `Style::default()` (ADR 0025).
- Comments (`#`, full-line) and blank lines are ignored; a line that fails to
  split on `.`/`=`, names an unrecognised role/field, or has an unparseable
  value is skipped entirely — no error, no partial application of that line.
- Role/colour keys are looked up via small hand-written `snake_case` tables
  (`Role::EditorText` ↔ `"editor_text"`, `Color16::LightGray` ↔
  `"light_gray"`), not a runtime case-conversion algorithm.

## Collaborators

Depends on `color` (`Style`, `Color`, `Color16`). Consumed by every widget when
drawing (a `&Theme` will be threaded through draw in later phases).

## Test plan (vertical slices)

1. (tracer) default theme resolves a few known roles to expected CGA styles.
2. `Role::ALL` is in discriminant order and `COUNT == ALL.len()` (index safety).
3. `with` overrides one role and leaves the rest unchanged.
4. `merge`: a single `fg`/`bg`/`attrs` line overrides just that field, leaving
   the role's other fields as `self` had them.
5. `merge`: `rgb(r, g, b)`, a `Color16` name, and `default` all parse as
   expected for `fg`/`bg`; out-of-range/malformed values (e.g. `rgb(999,0,0)`,
   `rgb(1,2)`, an unknown colour name) leave that field unchanged.
6. `merge`: `attrs` parses `none` and `|`-combined attribute lists; an unknown
   token anywhere in the list leaves `attrs` unchanged (all-or-nothing per
   line).
7. `merge`: comments (`#...`) and blank lines are ignored; a line with no `.`
   or no `=`, an unrecognised role key, or an unrecognised field name is
   skipped without affecting any other line.
8. `merge`: applying two layers in sequence
   (`default().merge(app_text).merge(user_text)`) lets the second layer
   override just one field the first layer also touched, while leaving a
   field only the first layer touched intact.

## Open questions

- The role set will grow as widgets arrive; additions append to `ALL` and the
  array resizes via `COUNT`.
- How the theme editor (#2)/theme builder (#3) *produce* this format's text
  to hand to `resource::write_user_resource` — full dump of every role vs. a
  diff against the layer beneath — is deliberately left to their own spec
  (ADR 0025).
