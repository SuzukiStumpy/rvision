# ADR 0021 — Context-sensitive help: window-scoped topics via `CM_HELP`

- **Status:** Accepted
- **Date:** 2026-07-03

## Context

`docs/roadmap.md`'s backlog and `help_window.md`'s Open Questions both named
the same gap: `HelpWindow::build` always opens on `contents.home()` — jumping
straight to a topic id for "whatever currently has focus" was cut from v1 as
application-level, "not designed here to avoid a speculative parameter with no
current caller." `CM_HELP` (`Command(6)`, `command.rs`) has sat reserved and
unused since it was first named, documented only as "the framework
standardises the id so the `Shell` and a bespoke app driver can share it" —
nothing acts on it yet.

A design discussion settled the scope before any code was written. Turbo
Vision's actual mechanism — a `HelpCtx` field on every `TView`, with `F1`
walking the focus chain to find the nearest one set — was considered and
rejected for now: it needs a new method on the `View` trait answered (or
defaulted) by every view in the tree, a cross-cutting commitment for one
feature, before `rvision` has a second consumer to validate the design against
(the roadmap's own still-open "second consumer" question). Scoping down to
per-*window* granularity instead avoids that commitment while still covering
the roadmap item's literal ask.

A first draft of this ADR left *where* `CM_HELP` gets caught vague —
"whichever layer catches it bubbling out of `Shell`." Tracing the actual
dispatch path showed that layer doesn't exist: `Root::dispatch` (`app.rs`)
drains every posted command and re-dispatches all but `CM_QUIT` straight back
into the tree, discarding the `EventResult` of each re-dispatch — there is no
"it reached the top unhandled" state left for anything above `Root` to
observe, and `Application::run` never hands control back between frames for
an outer wrapper to poll one anyway. Whatever reacts to `CM_HELP` has to do so
*inside* the `Shell` → `Desktop` dispatch itself, which means `Shell` — the
only thing in that chain that owns the `Desktop` a `HelpWindow` opens into.
This isn't a new kind of thing for `Shell` to do: it already reaches into
`widgets::` and constructs a specific widget itself in reaction to something
bubbling up, for the context-menu request `Shell::handle_mouse` drains
(ADR 0019).

This also clarifies what `CM_HELP`'s original doc comment meant by "the
`Shell` **and** a bespoke app driver can share it" — not two layers in the
same chain, but two *alternatives*: an app built on `rvision`'s `Shell`/
`Desktop` gets this for free once it hands `Shell` some help content; `edit`,
which still runs its own bespoke MDI chrome rather than `Shell`/`Desktop`
(`roadmap.md`), is the "bespoke app driver" — this ADR doesn't reach it as-is.
The expectation is that `edit` migrates onto `Shell`/`Desktop` at some future
point (its own undertaking, out of scope here), at which point it inherits
this mechanism rather than needing a second one.

Modal dialogs (`exec_view`) were considered and excluded outright, not merely
deferred: `exec_view` (`app.rs`) never holds a `Desktop` reference, and its
loop only recognises commands a dialog declares as *ending*
(`Window::ends_on`) — anything else posted, `CM_HELP` included, is
re-dispatched into the same dialog forever (`dispatch_modal`). A modal owns
input exclusively until dismissed, by definition, so it was never going to be
able to defer to a non-modal `HelpWindow` mid-run regardless of what `Window`
gains here. An application wanting "open help after a dialog closes" already
has everything it needs without framework changes: inspect the `Command`
`exec_view` returns, then open a `HelpWindow` itself as a separate, subsequent
step.

## Decision

**`Window` carries an optional help topic; `Frame` shows a glyph for it;
resolution is a read at the point `CM_HELP` is caught, not a payload on the
command.**

- `Window` gains a field, `help_topic: Option<String>` — an opaque topic id
  (a `HelpContents` key, meaningless to `rvision` beyond being a string) — set
  via a builder, `with_help_topic(mut self, topic: impl Into<String>) -> Self`
  (named `with_*`, not `help_topic(...)`, to avoid the setter/getter name
  clash the same way `with_default`/`ends_on` already do for `default_cmd`),
  and read via `help_topic(&self) -> Option<&str>`.
- `Frame` gains a third title-bar glyph, drawn immediately **left of the
  existing zoom glyph**, shown iff the owning window's `help_topic` is
  `Some` — `Window` pushes that visibility down the same way it already
  pushes `closable`/`zoomable` into the frame at construction/builder time.
- Clicking the glyph posts the **existing** `CM_HELP` — handled in
  `Window::handle_event` exactly like the close/zoom glyph clicks already are
  (`window.rs`, close/zoom click arms). No new `Command` is minted; `F1`
  (wherever an app binds it, e.g. a `StatusItem`) posts the same `CM_HELP`.
- **No payload rides on `CM_HELP`. `Shell` itself catches it and resolves the
  topic by reading, not by anything carried on the command.** `Shell` gains
  an optional field, `help: Option<HelpContents>`, set once via a builder
  (`with_help(mut self, contents: HelpContents) -> Self`). Its `handle_event`
  (the `Event::Command(_)` arm, currently a blind forward to
  `self.desktop.handle_event`) special-cases `CM_HELP` when `help` is `Some`:
  resolves the topic via `self.desktop.window(self.desktop.active_id()?)?
  .help_topic()` — `Some(id)` opens (or retargets) a `HelpWindow` on that
  topic; `None` (no `help_topic` set on the active window, or no active
  window at all) falls back to the home topic — then opens it into
  `self.desktop` (below). If `help` is `None` (the app never called
  `with_help`), `CM_HELP` falls through to `self.desktop.handle_event`
  exactly as every other command does today — zero cost, zero behaviour
  change, for a `Shell` that never opts in. `Desktop` needs no new API for
  this — `active_id()` and `window(id)` already exist.
- **`HelpWindow` gains a second entry point that opens straight to a topic**,
  the general-purpose seam both this feature and any other app-level caller
  need (e.g. a button whose action is "open help to topic X"): resolves `id`
  via the existing `HelpContents::topic_index`, mirroring how link activation
  already resolves a target (ADR 0020) — an unresolvable id is a silent
  fall-back to home, same miss-handling as everywhere else topic ids are
  resolved. `HelpWindow::build` itself is untouched (still starts at home, the
  common case); this is an additive sibling, not a breaking signature change.
- **The help window is a singleton, enforced by `Shell`, not by `Desktop` or
  `HelpWindow` themselves.** Neither has any notion of "the help window" as
  distinct from any other window — `HelpWindow::build`/`build_at` hand back a
  plain `Window`, immediately erased into `Desktop`'s `Box<dyn View>`-backed
  interior once opened, so there is no typed handle left to retarget later.
  `Shell` gains a second new field, `help_window: Option<WindowId>`, holding
  the id of the last help window it opened (if any). On `CM_HELP` it checks
  whether that id still resolves (`self.desktop.window(id)`) — if so, it
  **closes that window and opens a fresh one at the resolved topic**, reusing
  the closed window's own `bounds()` so position/size the user left it at
  survives; if not (never opened, or the user already closed it), it just
  opens one and remembers the new id. `Desktop::open` already raises what it
  opens to the top and makes it active (`raise` in `open`), so "bring the
  existing instance to the foreground" falls out of reopening for free — no
  separate focus step needed.
- **Narrow-frame behaviour stays all-or-nothing.** The existing
  `glyphs_shown(width)` gate (close+zoom disappear together below a width
  threshold) is extended to budget for three glyphs, still one boolean — a
  frame shows all of close/zoom/help or none of them; no per-glyph dropping.
- **Scoped to `Desktop`-hosted windows only.** A dialog run via `exec_view`
  could still set `help_topic`, but nothing shows or acts on it while running
  modally — inert, not rejected, since special-casing it away would cost more
  than leaving it unreachable.

## Consequences

- **Small, additive surface.** One `Option<String>` field plus two methods on
  `Window`, one new glyph span in `Frame`, one new match arm in
  `Window::handle_event`, one new `HelpWindow` entry point, and two new fields
  (`help`, `help_window`) plus a builder on `Shell`. `CM_HELP` goes from
  reserved-but-unused to load-bearing, making good on its own doc comment.
- **`Shell` gains a dependency on `help::HelpContents`/`widgets::HelpWindow`,
  but only pays for it when asked.** Consistent with what `Shell` already
  depends on (`widgets::{ContextMenu, Desktop, MenuBar, StatusLine, Window}`)
  — not a new category of coupling, just one more member of it. A `Shell`
  that never calls `with_help` carries a permanently-`None` field and takes
  the exact code path it does today; the cost and the dependency are both
  opt-in.
- **`edit` isn't reached by this yet.** It runs its own bespoke MDI chrome,
  not `Shell`/`Desktop` (`roadmap.md`), so nothing here changes it today.
  Migrating `edit` onto `Shell`/`Desktop` is its own, separate, future
  undertaking; once it happens, `edit` inherits context-sensitive help for
  free rather than needing a second mechanism built for its bespoke chrome.
- **`rvision` still owns no notion of what a topic id *means*.** It's an
  opaque string, matched later against whatever `HelpContents` the
  *application* supplies when it builds the `HelpWindow` — no `HelpContents`/
  `HelpTopic` dependency enters `Window` or `Frame`. The "framework owns
  format/viewer, application owns content" split (ADR 0013) holds one layer
  further down.
- **No per-*view* help context.** Only whole-window granularity exists;
  "F1 shows help for whatever control has focus inside this window" is not
  buildable on this decision alone. Accepted as the deliberate scope cut
  above — revisit only once a `View`-trait-wide mechanism has more than one
  feature (and, per the roadmap, more than one consumer) to justify it.
- **Modal dialogs get no context-sensitive help, by construction.** Not a gap
  left open — a boundary, since `exec_view` already cannot defer to a
  `Desktop` mid-run for any reason. "Help after a dialog" remains an
  application-composed sequence (inspect `exec_view`'s returned `Command`,
  then open a `HelpWindow` separately), needing nothing further from
  `rvision`.
- **Reopening in place of retargeting trades a little state for a lot of
  simplicity.** A rebuilt `HelpWindow` restarts its internal focus
  (list vs. pane) and the topic list's own scroll position, rather than
  preserving them — a real but minor regression against "the same instance,
  truly retargeted," accepted because the alternative (giving `Window` a way
  to reach a concrete `HelpWindow` back out of its `Box<dyn View>` interior,
  e.g. via `Any` downcasting) would add new cross-cutting surface to `View`
  for this one case, the exact cost already rejected above for `HelpCtx`.
  Position and size (`bounds()`) are preserved, since those live on `Window`
  itself and are captured before the close.

## Alternatives considered

- **Leave resolution entirely to "whichever layer catches `CM_HELP`," outside
  `Shell`.** This ADR's own first draft. Rejected once traced through the
  actual dispatch code: `Root::dispatch` discards the result of every
  re-dispatched command but `CM_QUIT`, and `Application::run` never yields
  control between frames for an outer wrapper to poll anything — there is no
  layer above `Shell` in the successful path capable of catching it. `Shell`
  catching it directly isn't a fallback position; it's the only mechanically
  workable one for apps using `Shell`/`Desktop` at all.
- **`HelpCtx`-on-`View`, Turbo-Vision-faithful.** Rejected for now: a new
  `View` trait method every view must answer or inherit a default for,
  justified by a single feature, before a second `rvision` consumer exists to
  pressure-test the design. Revisit if that changes.
- **Threading the topic id through `CM_HELP` itself** (a richer `Command`
  variant, or a side-channel like `Context::open_context_menu`/ADR 0019's
  pending-request field). Unnecessary: unlike a context-menu request (which
  can originate arbitrarily deep in the tree) or a followed help link (ADR
  0020, where `Command` genuinely cannot carry the string), the topic here is
  already sitting in already-tracked state (`Desktop::active_id` +
  `Window::help_topic`) by the time `CM_HELP` is caught. A plain read is
  simpler than adding a queue nothing else needs.
- **Progressive glyph dropping under a narrow frame** (help first, then
  zoom, then close). More graceful, but adds per-glyph width branching to a
  gate that has been a single boolean since `Frame` shipped, for an edge case
  (narrow *and* has a help topic) nothing exercises today. Rejected in favour
  of extending the existing all-or-nothing budget.
- **Giving `exec_view` a `Desktop` reference (or its own escape hatch) so
  modal dialogs could open non-modal help mid-run.** Real added scope for a
  case that contradicts a modal's own contract (exclusive input until
  dismissed) — rejected outright, not deferred.
- **`Any`-based downcasting on `View` so a driver could reach a concrete
  `HelpWindow` back out of `Window`'s boxed interior and retarget it in
  place**, avoiding the close/reopen state loss above. Rejected: it is new
  surface on the one trait every view in the crate implements, paid for by a
  single feature — the same objection already made to `HelpCtx`-on-`View`.
  Close-and-reopen gets the same user-visible result (same position/size, a
  window that jumps to the requested topic and comes to the front) without it.
