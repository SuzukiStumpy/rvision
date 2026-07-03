# ADR 0023 — Truecolour capability detection via environment, no new crate

- **Status:** Accepted
- **Date:** 2026-07-03

## Context

ADR 0005 made `Color` truecolour-ready (`Color::Rgb(u8, u8, u8)`) and left the
backend passthrough as "an unused enum variant plus a backend match arm" — that
back pocket is now spent: `crossterm_backend::to_ct_color` already forwards
`Color::Rgb` straight to crossterm's `Color::Rgb`. ADR 0005 explicitly deferred
two things under "Full truecolour now": capability detection and a downgrade
fallback.

This ADR closes the detection half. The fallback's actual *consumer* — a real
truecolour theme, chosen between its RGB and 16-colour forms — is out of scope
here: themes are moving from Rust-embedded data to files loaded at runtime (see
the roadmap's resource-loader backlog item), so there is no theme yet to fall
back from, and `Theme`'s shape shouldn't be guessed at ahead of that design.
This ADR gives that future work (or any hand-authored theme pair in the
meantime) a decision it can rely on, landed now because it doesn't need to wait
on anything else.

Crate budget (ADR 0001/0006) rules out a capability-detection crate (e.g.
`supports-color`) without its own ADR. The fact needed — "does this terminal
understand 24-bit colour" — is knowable from two environment variables
terminals already set, so no crate is needed.

## Decision

Add `ColorProfile { Truecolor, Cga16 }` to `color.rs`, with
`ColorProfile::detect()` reading `COLORTERM` (`truecolor`/`24bit`,
case-insensitive) and, failing that, a `TERM` containing `direct` (e.g.
`xterm-direct`).

The decision logic is a pure function, `profile_from_env(colorterm: Option<&str>,
term: Option<&str>) -> ColorProfile`, taking both variables as parameters;
`detect()` is a thin wrapper reading the real environment once. This mirrors the
existing pattern in `crossterm_backend.rs` (`apply_double_click` takes
`now: Instant` rather than calling `Instant::now()` internally) so the policy
stays unit-tested without mutating process environment in tests.

No other module changes. `Theme`, `Style`, `Cell`, and the backend are
untouched — `ColorProfile` is a standalone fact for a future caller to consult;
it does not decide anything on its own yet.

## Consequences

- A real, unit-tested answer to "can I use RGB here" exists ahead of any code
  that needs it, at zero crate cost.
- Detection is necessarily heuristic (env vars, not a terminfo query): a
  terminal that supports truecolour but doesn't set `COLORTERM`/`TERM` as
  expected is treated as `Cga16`. Accepted deliberately — under-detection only
  costs an unnecessarily cautious fallback, never broken output.
- `ColorProfile` has no caller inside `rvision` itself yet. That's expected for
  a library primitive landed ahead of its consumer (the resource loader,
  roadmap backlog #9), not a half-finished feature — `detect`/`profile_from_env`
  are each complete and fully tested in their own right.

## Alternatives considered

- **A capability-detection crate** (e.g. `supports-color`) — more thorough (can
  probe terminfo, handle more edge cases), but needs its own ADR against the
  crate budget for a fact two env vars already give us.
- **Landing the theme-resolution side now, guessing at `Theme`'s future
  loaded shape** — would mean designing part of the resource loader's data
  model before that ADR exists, risking rework once the loader's actual file
  format and override semantics are decided.
- **No detection; always trust the theme** — the option seriously considered
  earlier in this discussion, but it pushes the "does this terminal actually
  support this" question onto every app author with no framework help at all.
