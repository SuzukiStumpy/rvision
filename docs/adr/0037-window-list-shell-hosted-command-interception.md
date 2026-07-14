# ADR 0037 — Window list: unconditional `Shell` command interception, read back via `Any` downcast

- **Status:** Accepted
- **Date:** 2026-07-14

## Context

Roadmap #2's newest sub-item, the window list dialog, needs to let a user
bring any open window to the front (dismissing the list) or close one
outright (list stays open, refreshed) — see `docs/specs/window_list.md`.
Every existing standard dialog (`ColorPicker`, `ThemePicker`, `FileDialog`)
is a modal `Application::exec_view` window: a blocking, one-shot loop where
the caller's background `Program` "receives no events while the window is
up" (`src/app.rs`, `exec_view`'s own doc comment). That shape can't survive
a `Close`-and-stay-open requirement — there's no point inside `exec_view`'s
loop where outside code gets to touch `Desktop` and feed a refreshed
snapshot back into the still-running dialog. The non-modal, `Desktop`-hosted
shape `HelpWindow`/`ThemeEditor` already use fits instead (ADR 0016, ADR
0021, ADR 0026).

That leaves two sub-problems ADR 0021 and ADR 0026 each solved once, in
slightly different shapes, neither an exact fit here:

- **Who opens it, and on what trigger?** ADR 0021's `CM_HELP` is caught by
  `Shell` only when the app opts in via `with_help(contents)`, because
  `Shell` needs app-supplied `HelpContents` it has no other way to get.
  `WindowList` needs no such data — `Desktop`, which `Shell` already owns by
  value, is the entire input. An opt-in method would exist solely to toggle
  behaviour `Shell` can always safely provide, for free, the same way
  `Desktop` itself always handles `CM_CLOSE`/`CM_ZOOM`/`CM_NEXT`/`CM_PREV`
  with no opt-in at all.
- **How does closing/activating a window it didn't open reach `Desktop`?**
  `ThemeEditor`'s Fg/Bg buttons (ADR 0026) face the structurally same
  problem — a view needs something only its host can do — and solved it
  with two new framework commands the *hosting `Program`* intercepts, plus
  a shared `Rc<RefCell<...>>` handle carrying the actual data (position,
  which field). That handle idiom is the right shape for a *leaf* dialog
  handing back one final value once (`ColorPickerResult`/`FileDialogResult`/
  `ThemePickerResult` all do the same). `WindowList` isn't a leaf: ADR 0036
  (accepted the day before this one) added exactly the seam this needs
  instead — `Desktop::content_mut::<T>(id)`, letting app-level code reach a
  window's live concrete content by id from *outside* dispatch. Introducing
  a second, parallel handle mechanism when ADR 0036 already generalizes the
  same access would be the redundant option here, not the conservative one.

## Decision

**`Shell` handles `CM_WINDOW_LIST` unconditionally** — no `with_window_list`
opt-in — next to the existing `CM_HELP` check in `Shell::handle_event`'s
`Event::Command` arm. It builds a snapshot from `Desktop::windows()`
(excluding the list's own previous window, if reopening), and opens a
`WindowList::build(...)` window, singleton-style: closes any prior instance
first, same as `open_help` already does for `HelpWindow`.

**Two new framework commands, `CM_WINDOW_LIST_ACTIVATE`/`CM_WINDOW_LIST_CLOSE`**
(`src/command.rs`, below `CM_USER`, alongside `CM_EDIT_FG`/`CM_EDIT_BG`).
`WindowList` posts one of these instead of acting on anything itself — it
has no `Desktop` reference to act with (ADR 0003). Each carries no payload
(`Command` is a bare `u16` newtype); `WindowList` records *which* window
under a plain internal field instead.

**`Shell` reads that field back via `Desktop::content_mut::<WindowList>`
(ADR 0036), not a new shared handle.** On either command, `Shell` calls
`content_mut::<WindowList>(list_id).take_pending()` and acts on `self.desktop`
directly: `Activate(id)` → `focus(id)` then close the list window itself
("bring to top + dismiss"); `Close(id)` → `close(id, ctx)` then push a
refreshed snapshot back into the still-open list via `set_entries(...)`
("terminate, list stays open").

## Consequences

- `WindowList` stays a fully passive, `Desktop`-ignorant `View` — same
  boundary `ThemePicker`/`ColorPicker` already keep (a widget never reaches
  into `Desktop`/`rvision::resource` itself) — even though, unlike them,
  it's driving *live*, repeated `Desktop` mutation. The mutation itself
  lives entirely in `Shell`, where the `Desktop` reference actually is.
- No new "shared result handle" type joins `ColorPickerResult`/
  `FileDialogResult`/`ThemePickerResult`. `Desktop::content_mut` already
  existing (ADR 0036) is what makes skipping it possible here — this ADR
  would likely have reinvented that same shape under a different name had
  ADR 0036 not just landed.
- `Shell` grows a second unconditional command interception (alongside
  `CM_CLOSE`/`CM_ZOOM`/`CM_NEXT`/`CM_PREV` already living in `Desktop`) and
  a second unconditionally-true unconditional command dispatch outside the
  `with_help`-style opt-in family. That's an intentional asymmetry, not an
  oversight: opt-in exists specifically for behaviour that needs app-supplied
  data `Shell` has no other way to get (`HelpContents`); nothing here needs
  data the app must supply, so gating it behind a builder method would only
  add a step every consumer has to remember, for no actual flexibility
  gained — mirrors why `Desktop` itself never gated `CM_CLOSE` behind an
  opt-in either.
- Any future `Shell`-hosted, `Desktop`-content-mutating widget has two
  precedents to choose between now: ADR 0026's shared-handle shape (a leaf,
  one-shot dialog opening *another* dialog) and this ADR's downcast-readback
  shape (a persistent widget whose host must reach back into live `Desktop`
  state on its behalf, repeatedly). Picking between them going forward
  should turn on exactly that distinction — leaf/one-shot vs.
  persistent/repeated — not preference.

## Alternatives considered

- **A shared `Rc<RefCell<WindowListAction>>` handle**, mirroring ADR 0026
  exactly. Rejected: would duplicate what `Desktop::content_mut` (ADR 0036)
  already does, for no benefit — the whole reason that ADR exists is this
  exact "app code reaching a window's content by id from outside dispatch"
  need.
- **`Desktop` itself intercepts `CM_WINDOW_LIST_ACTIVATE`/`_CLOSE`**, the
  same place `CM_CLOSE`/`CM_ZOOM`/`CM_NEXT`/`CM_PREV` already live.
  Rejected: those four are generic window-chrome commands meaningful for
  *any* window `Desktop` hosts. Reading a specific widget's concrete type
  via downcast to find its pending action is a `WindowList`-specific
  concern, and `Desktop` staying ignorant of any particular widget it hosts
  is worth preserving — `Shell` (which already composes `Desktop` with
  concrete, named chrome like `MenuBar`/`StatusLine`) is the layer that's
  already allowed to know about specific widgets.
- **Gate `CM_WINDOW_LIST` behind a `with_window_list()` opt-in**, mirroring
  `with_help` exactly for API-shape consistency. Rejected: opt-in exists to
  cover a real asymmetry (`Shell` cannot conjure `HelpContents` on its own);
  no equivalent gap exists here, so the mirrored API would be consistency
  for its own sake at the cost of one more thing every consumer must
  remember to call.
