# ADR 0028 — A system-level global keyboard accelerator table

- **Status:** Accepted
- **Date:** 2026-07-04

## Context

A keyboard shortcut only ever worked if it was registered as a `StatusItem`
on `StatusLine` — which both draws a visible hint/label *and* matches the
raw key in its own `handle_event`. `MenuItem::with_shortcut("Ctrl-O")` is
purely cosmetic text drawn next to a pull-down item; it binds nothing.

This produced a real, confirmed bug: `examples/help_builder.rs` drew
`"Save As...".with_shortcut("Ctrl-A")` but never registered a matching
`StatusItem`, so `Ctrl-A` silently did nothing. Found while answering a
question about how to wire a menu item's shortcut at all, once it became
clear `MenuItem`/`StatusItem` had no shared source of truth.

The user's framing of the fix (their words): *"Accelerator keys should be a
system-level construct. So we would have a global table of accelerators and
their message-bindings. It would then be up to the desktop to emit these
commands into the global message pump and allow them to percolate down to
whichever view claims them and actions them. That way we sidestep the issue
that they're an app-specific customisation, allows the framework to work
with them even if we don't have a statusbar enabled."* — and, when asked how
the new table should relate to `StatusLine`: **unify** them, so a shown
shortcut hint is structurally incapable of lacking a real binding behind it.

## Decision

**`Desktop` owns a global `key -> command` table** (`Accelerators`,
`src/command.rs`, alongside the existing `CommandSet` — both are "commands
and their state," just one flavour is enabled/disabled and the other is
key-triggered). `Accelerator { key: KeyEvent, command: Command }` is the
public unit apps construct; the table itself (`pub(crate) Accelerators`) is
an internal resolver, first-bound-match-wins, no conflict detection (a
collision is a programming mistake, not a runtime condition).

`Desktop::handle_event`'s `Event::Key` arm tries the active window's own
dispatch first — unchanged — and only on `Ignored` falls back to
`self.accelerators.resolve(key)`, posting the resolved command via
`ctx.post` (which already gates on `CommandSet`, so a disabled command's key
still consumes but posts nothing). `Event::Paste` carries no `KeyEvent` and
was split out of the combined match arm it used to share with `Key`, since
it can't participate.

**No new dispatch mechanism was needed to make this "percolate down."**
`Root::dispatch` (and any app's hand-rolled equivalent, e.g.
`examples/help_builder.rs`'s own `HelpBuilderDemo::dispatch`) already drains
whatever a `Context` posts and re-dispatches each posted event from the
root until the queue empties. So `Desktop` posting the resolved command is
the entire mechanism: it flows back down through `Shell -> Desktop::handle_command`
-> the active window's focus chain (any control handling `Event::Command(X)`
claims it there, exactly like any other command) -> or bubbles all the way
back to `Ignored`, which an app's own top-level driver can catch as the
final fallback. This is precisely "percolate down to whichever view claims
them ... or the main event loop," for free, off the existing command-bubble
machinery (no new `View` trait method, no per-control registration
protocol).

**`StatusItem` is unified with `Accelerator`, not merely coexisting with
it:** `StatusItem::new(hint, label, accelerator: Accelerator)` embeds the
binding it displays a hint for, rather than taking a separate `key`/
`command` pair that could drift out of sync with what's actually bound (the
Ctrl-A bug's exact shape). `StatusLine::handle_event` is deleted entirely —
it is now a pure display widget. `StatusLine::accelerators()` exposes every
item's binding; `Shell::new` feeds each one into `Desktop::bind_accelerator`
at construction, so building a `StatusLine`, as before, is *sufficient* to
get a working shortcut. A shortcut that shouldn't take a status-line slot at
all — the concrete Ctrl-A fix — is bound directly:
`shell.desktop_mut().bind_accelerator(Accelerator::new(key, command))`, no
`StatusItem` involved, no new `Shell`-level API needed since `desktop_mut()`
already existed.

## Consequences

- `MenuItem::with_shortcut` remains deliberately cosmetic text — unifying it
  with `Accelerator` too was considered and rejected (see Alternatives): a
  menu item's displayed shortcut and its working binding still have to be
  kept in sync by the app author, same as before. Only the `StatusLine`/
  `Desktop` side is now structurally safe against drifting apart.
- A shortcut works with **no window open at all** (`Desktop`'s active-window
  check simply misses and falls through to the table either way) and **with
  no `StatusLine`/status bar in the chrome at all** — the concrete
  motivation for calling this "system-level" rather than an app/`StatusLine`
  customisation.
- The active window's own key handling **always** wins over an accelerator —
  preserves existing contracts (an open menu pull-down's modal claim, a
  focused control's own meaningful use of a key) without any new ordering
  rule: accelerators simply inherit `StatusLine`'s old post-process slot in
  `Shell`'s three-pass chain, just resolved one level lower, inside `Desktop`.
- Accelerators don't reach a modal `Application::exec_view` dialog, since
  that loop runs a bare `Window` and never touches `Desktop`/`Shell` at all
  — unchanged from `StatusLine`'s own reach today, not a regression.
- `StatusItem::new`'s signature is a breaking public API change
  (`(hint, label, key, command)` -> `(hint, label, accelerator)`).

## Alternatives considered

- **Teach `Desktop` about `ScrollBar`-style per-widget registration**, i.e. a
  new `View` trait method (`fn accelerator(&self, key: &KeyEvent) ->
  Option<Command>`) so an unfocused, buried control could claim a key
  directly, mirroring how a context-menu/mouse-capture request works.
  Rejected for this pass: no concrete control in this codebase needs it —
  every real shortcut so far is app-level (owned by the driver/`Shell`, not
  any specific widget) — and the existing command-bubble machinery already
  gets a claiming control "for free" once `Desktop` posts the resolved
  command, without a new protocol. Revisit only once something concrete
  needs a control to own its own accelerator independent of app wiring.
- **Also fold `MenuItem::with_shortcut` into `Accelerator`** so a menu item's
  displayed text is generated from (and can't drift from) a real binding.
  Appealing, but a separate, smaller concern from the architectural gap this
  ADR closes (menu items don't intercept keys at all, `StatusLine`/`Desktop`
  do) — left as a possible follow-up, not required to fix the reported bug.
- **Keep `StatusLine` intercepting keys itself, alongside a separate
  `Desktop`-level table for "invisible" ones.** Two places a shortcut could
  be bound, with no structural link between them, reintroduces exactly the
  "shown hint, no binding" divergence risk this ADR closes. Rejected in
  favour of full unification.
