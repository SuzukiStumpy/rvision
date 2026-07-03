# Module spec: `rvision::color`

- **Status:** In progress
- **Phase:** 1
- **Related ADRs:** 0005 (semantic roles over a truecolour-ready type), 0023
  (truecolour capability detection)

## Purpose

Represent colours and text styling in a way that is **truecolour-ready from day
one** but ships 16-colour CGA values first. Pure value types; no terminal I/O and
no role/theme logic (that lives in `theme`). Also carries the one fact a future
theme resolver needs to choose between a truecolour and 16-colour rendering of
itself: whether the terminal is believed to support 24-bit colour.

## Public interface

```rust
pub enum Color16 { Black, Blue, Green, Cyan, Red, Magenta, Brown, LightGray,
                   DarkGray, LightBlue, LightGreen, LightCyan, LightRed,
                   LightMagenta, Yellow, White }
impl Color16 { fn to_rgb(self) -> (u8, u8, u8); }   // canonical CGA values

pub enum Color { Default, Named(Color16), Rgb(u8, u8, u8) }  // Default = terminal default
impl Color { fn resolve_rgb(self) -> Option<(u8, u8, u8)>; } // Default -> None

pub struct Attributes(/* u8 bitset */);
impl Attributes {
    const NONE/BOLD/DIM/ITALIC/UNDERLINE/REVERSE/BLINK: Self;
    fn empty()/is_empty(self)/contains(self, other)/union(self, other);
    // BitOr for ergonomic `BOLD | UNDERLINE`
}

pub struct Style { pub fg: Color, pub bg: Color, pub attrs: Attributes }
impl Style { fn new(); fn fg(self, Color); fn bg(self, Color); fn attrs(self, Attributes); }

pub enum ColorProfile { Truecolor, Cga16 }
impl ColorProfile { fn detect() -> Self; } // reads COLORTERM / TERM once (ADR 0023)
```

## Behaviour & invariants

- The 16 named colours store their **canonical CGA RGB** so the aesthetic is
  identical whether a backend emits 16-colour or truecolour escapes.
- `Color::Default` means "the terminal's default" and has no fixed RGB →
  `resolve_rgb` returns `None`.
- `Attributes` is a bitset: `union` combines, `contains(x)` is true only if all
  bits of `x` are set; `NONE`/`empty()` contains nothing.
- `Style` default is `Default` fg/bg with no attributes; builder methods are
  chainable and return a new `Style`.
- `ColorProfile::detect()` treats `COLORTERM` of `truecolor`/`24bit`
  (case-insensitive) as authoritative; failing that, a `TERM` containing
  `direct` (e.g. `xterm-direct`) also counts as truecolour. Anything else is
  `Cga16` — detection is deliberately conservative (a false "no" degrades to a
  smaller palette; a false "yes" would risk garbled escapes).
- The env-reading part of `detect()` is a thin, untested-by-necessity wrapper;
  the actual decision is a pure function taking both variables as `Option<&str>`,
  so the policy is unit-tested without touching real process environment
  (same shape as `crossterm_backend`'s clock-injected double-click detection).

## Collaborators

Consumed by `theme` (roles → `Style`), by `cell` (each cell carries a `Style`),
and by the backend (which turns `Color`/`Attributes` into escape sequences).
`ColorProfile` has no consumer inside `rvision` yet — it's a standalone fact for
the future resource loader (roadmap backlog #9) or a hand-authored theme pair to
consult when choosing between a theme's truecolour and fallback styles. Depends
on nothing.

## Test plan (vertical slices)

1. (tracer) `Color16::to_rgb` returns canonical CGA values (spot-check several).
2. `Color::resolve_rgb`: Named via Color16; Rgb returns itself; Default -> None.
3. `Attributes`: empty contains nothing; union + contains; BitOr sugar.
4. `Style`: default is blank; chained builders set fg/bg/attrs.
5. `ColorProfile`'s pure decision function: `COLORTERM=truecolor`/`24bit`
   (and case variants) -> `Truecolor`; a `direct`-containing `TERM` with no
   `COLORTERM` -> `Truecolor`; absence of both, or an unrelated value of
   either -> `Cga16`.

## Open questions

- ANSI 0..15 index mapping for `Color16` was deferred to the backend; resolved
  in `crossterm_backend::to_ct_color`, which maps each `Color16` to crossterm's
  named palette (so the user's own terminal theme applies) rather than a fixed
  ANSI index.
