# Module spec: `rvision::command`

- **Status:** Done
- **Phase:** 3 (View system)
- **Related ADRs:** 0003 (commands bubble up the owner chain), 0004
  (`Event::Command`), 0028 (`Accelerator`/`Accelerators`, global keyboard
  shortcuts)

## Purpose

The vocabulary of UI **commands**, which of them are currently **enabled**,
and which keys fire them. A [`Command`] is the integer id of an action
(TurboVision's `cmXxx`); a [`CommandSet`] tracks the enabled/disabled state
so a control can grey itself and a disabled command never fires; an
[`Accelerator`] pairs a key with the command it should fire, and
[`Accelerators`] is the table `Desktop` resolves an unclaimed key against
(ADR 0028).

It is **not** the dispatcher: routing a command up the owner chain is the
`view::Group`'s job (ADR 0003), and resolving a key against the accelerator
table is `widgets::Desktop`'s (ADR 0028). This module is pure data — no
views, no events beyond re-exporting the `Command`/`KeyEvent` types `event`
already defines.

## Public interface

```rust
pub use crate::event::Command;

// Standard command ids (TV reserves the low numbers).
pub const CM_QUIT: Command;
pub const CM_OK: Command;
pub const CM_CANCEL: Command;

// The framework/application boundary: ids below CM_USER are framework
// standard commands; an app numbers its own from here up (TV's cmUser).
pub const CM_USER: u16 = 100;

pub struct CommandSet { /* set of *disabled* ids; empty => all enabled */ }

impl CommandSet {
    pub fn new() -> Self;                       // everything enabled
    pub fn enable(&mut self, command: Command);
    pub fn disable(&mut self, command: Command);
    pub fn is_enabled(&self, command: Command) -> bool;
}

/// One keyboard shortcut: `key` fires `command` (ADR 0028).
pub struct Accelerator { /* key: KeyEvent, command: Command */ }
impl Accelerator {
    pub fn new(key: KeyEvent, command: Command) -> Self;
}

// pub(crate): Desktop's own resolver, not part of the public surface.
struct Accelerators { /* bindings: Vec<Accelerator> */ }
impl Accelerators {
    fn new() -> Self;
    fn bind(&mut self, accelerator: Accelerator);
    fn resolve(&self, key: &KeyEvent) -> Option<Command>; // first bound match wins
}
```

## Behaviour & invariants

- **Open, partitioned namespace.** A `Command` is just a `u16` — an open id space
  (ADR 0003). The framework defines only the standard ids its own widgets emit and
  handle (all below `CM_USER`); an application numbers its commands from `CM_USER`
  up, and the framework routes those opaquely without ever naming them. App
  extensibility is *adding ids*, not editing the framework or the `Event` enum.
- **Enabled by default.** A freshly-`new` set enables every command; you disable
  the exceptions. (Most commands are live most of the time; storing the disabled
  ones keeps the common case allocation-free-ish and the whole `u16` space usable.)
- `enable`/`disable` are idempotent; enabling a never-disabled command is a no-op.
- `is_enabled` is the single query everything else builds on (a control's draw, a
  `Context`'s decision whether to post a command).
- **`Accelerators::resolve` does no gating of its own** (ADR 0028) — it's a
  pure lookup; `Desktop` posts the resolved command through `Context::post`,
  which already checks `CommandSet`, so a disabled command's key still
  consumes the keystroke but posts nothing. Binding the same key twice is
  not an error: the first bound entry wins, silently — a collision is a
  programming mistake, not a runtime condition worth detecting.

## Collaborators

- `event::Command` — the id type, re-exported so callers say `command::Command`.
- `view::Context` — consults `is_enabled` so a disabled command posted by a view
  is silently dropped (a disabled button can't fire), and exposes it so a control
  can render itself greyed.

## Test plan (write these first)

- **Logic:** new set enables an arbitrary id; `disable` then `is_enabled` is false;
  `enable` re-enables; idempotent repeats; two ids are independent.
- **Constants:** the standard ids are distinct and non-zero.
- **`Accelerators`:** an empty table resolves nothing; a bound key resolves to
  its command; an unbound key resolves to `None`; binding the same key twice
  resolves to the first one bound.

## Open questions

- A 256-bit bitset (like TV) vs a `BTreeSet`: the set is tiny and rarely touched,
  so `BTreeSet` is fine until a profile says otherwise.
