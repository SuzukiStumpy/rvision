# ADR 0020 — Followable help links: spans, dedicated cycle keys, a pending-activation poll

- **Status:** Accepted
- **Date:** 2026-07-03

## Context

The help markup (ADR 0013) already parses `{label|target}` inline links, but
the v1 parser throws `target` away and keeps only `label` as plain text —
named there as a deliberate scope cut ("full hypertext... is its own later
phase"). `docs/roadmap.md`'s backlog and `docs/specs/help_window.md`'s Open
Questions both point at the same gap: `HelpWindow` has no way to jump the
list selection (and thus the page) when a link is activated. ADR 0013's own
Consequences section predicted the shape of the fix: "`Paragraph(String)`
becomes a span sequence... a contained change behind one block variant."

Two questions had to be settled before implementation:

1. How does a keyboard user select and follow a link, given `HelpPane`
   already claims every arrow key plus `PageUp`/`PageDown`/`Home`/`End` for
   scrolling, and `HelpWindow` already claims `Tab`/`BackTab` to switch focus
   between the topic list and the page?
2. How does the pane, several layers down from `HelpWindow`, tell it "jump
   to topic X" — `Command` (`command.rs`) is a bare `u16` id and cannot carry
   a target string.

## Decision

**Keyboard model (1): dedicated `Ctrl+Down`/`Ctrl+Up` cycle a "current link"
highlight; `Enter` follows it; a direct click follows immediately** —
decided explicitly to leave `HelpWindow`'s existing, tested `Tab`/`BackTab`
list⇄pane contract untouched, rather than overloading `Tab` to also cycle
links and fall through to the pane/list switch at the ends (which would have
required `HelpPane` to report "no more links this direction" back up to
`HelpWindow`, a new coordination edge for marginal benefit). `HelpPane`'s key
match already ignores modifiers (any modifier on `Down` scrolls today, by
accident), so the new bindings are guarded arms added *above* the existing
plain ones — `KeyCode::Down if modifiers.contains(CONTROL) &&
!self.links.is_empty()` — falling through to ordinary scrolling on a
linkless topic rather than going inert.

**Span model (part of 1's rendering, and 2's data source): `Block::Paragraph
(String)` becomes `Block::Paragraph(Vec<Span>)`,** exactly as ADR 0013
predicted. `Span` is `Text(String) | Link { label: String, target: String }`.
`Block::Preformatted` is untouched — the parser never link-parses `<pre>`
content, so links there stay literal, same as before.

**Wrapping stays out of `wrap.rs`.** `wrap::wrap(&str, u16)` remains generic
and help-agnostic; a private span-aware wrap function lives beside
`render_blocks` in `help_pane.rs` instead (which already hosts
`render_blocks` itself rather than `wrap.rs`, even though it calls into it).
Teaching the generic wrap utility about `Span` would leak help-markup
knowledge into a module with zero help dependency today — the same "no
editor knowledge belongs [in rvision]" logic applied one level down.

The wrap function tokenizes over one **flattened** stream (spans'
texts concatenated with a parallel byte-range→span map), not per-span
independently. Tokenizing per span was the first draft and is wrong: it
inserts a phantom space wherever a link is immediately followed by abutting
punctuation with no separating space (`{paste|clipboard}.` would wrongly
wrap as `"paste ."`) — a shape the repo's own existing help-content test
fixture already contains. Flattening first reproduces `wrap::wrap`'s exact
word-boundary/space-collapse behavior, then attributes each emitted word's
link target by whichever span produced the word's first byte. `render_blocks`
now returns both the laid-out lines and a `Vec<PaneLink>` (line index +
byte-range-into-that-line + target); a link that wraps across a line
boundary correctly produces two `PaneLink`s, one per line, sharing a target.

**Signalling (2): a polled `pending_activation` field on `HelpPane`, drained
by `HelpWindow`** — `take_link_activation(&mut self) -> Option<String>`,
mirroring the existing `sync_pane_from_list` (which already polls
`ListBox::selected()` after routing an event into the list) in the opposite
direction: `HelpWindow::sync_list_from_pane_link` polls the pane after
routing an event into it, resolves the target to a topic index via a new
`HelpContents::topic_index`, and on a hit calls `list.select(idx)` +
`pane.show(topic)`; on a miss (a dangling target — shouldn't happen for
well-formed content, per ADR 0013's "caught by a content test" stance) it's
a silent no-op. Focus is untouched either way — activation moves the list
selection and page content, not keyboard focus.

This is the same family of solution as `Context::open_context_menu`/
`take_context_menu_request` (ADR 0019): a pending request drained by
whoever's actually driving dispatch, deliberately *not* routed through
`Event`/`posted`, since nothing in the tree is waiting to *handle* "a link
was activated" as an incoming event. The one difference: ADR 0019's field
lives on `Context` because a context-menu request can originate arbitrarily
deep in the tree. Here `HelpWindow` is `HelpPane`'s sole direct owner, so a
local field polled synchronously — one level simpler, matching the existing
`list.selected()` poll's own shape — is enough.

**Drawing** reuses `Role::Selection` for the current+focused link (no second
new role), directly mirroring `ListBox::draw`'s `if self.focused &&
self.selected == Some(idx) { focus_style } else { style }` ternary, applied
per link instead of per row. One new role, `Role::HelpLink`, covers every
link at rest (shown regardless of focus, so mouse users can see clickable
text even when the list holds focus) — appended last in `Role`/`Role::ALL`
so no existing discriminant shifts.

## Consequences

- **Breaking API change.** `Block::Paragraph`'s payload type changes
  (`String` → `Vec<Span>`); the commit landing it uses `feat!:` or a
  `BREAKING CHANGE:` footer per CLAUDE.md's Conventional Commits rule.
- **Current-link identity isn't preserved across a live resize.**
  `current_link` clamps to the new link count after `layout()` re-runs, the
  same best-effort (not identity-preserving) contract `clamp_top`/
  `clamp_left` already have for scroll position — acceptable for the same
  reason those are: a resize is a rare, discrete event, and "some link
  becomes current" beats panicking or losing all keyboard-link access.
- **A link immediately preceded by abutting text with no separating space
  isn't independently clickable** — the word containing the boundary byte
  is attributed to whichever span produced its *first* byte, so `text{link}`
  with no space merges into the preceding span's word. No shipped content
  does this today (the parser always joins paragraph source lines with `" "`
  before spans are cut, so this only arises from adjacent tokens authored
  with zero whitespace between them); accepted rather than solved, since
  solving it (splitting words at internal span boundaries too) would double
  the bookkeeping for a case nothing exercises.
- `wrap.rs` gains no new public surface and no new dependency; the crate
  budget (ADR 0001) is unaffected — this is all internal restructuring plus
  one new `Theme` role.

## Alternatives considered

- **Tab cycles links, falling through to the list⇄pane switch at either
  end.** More browser-like, but requires `HelpPane` to report "nothing left
  to cycle to in this direction" back to `HelpWindow`, a new coordination
  edge, and changes an existing tested contract for a UX gain the simpler
  scheme (dedicated keys, mouse click) gets almost all of. Rejected — user
  call, made before implementation.
- **Mouse-only link following, no keyboard cursor.** Simplest by far, but
  leaves keyboard users unable to use the feature at all, at odds with ADR
  0007's "architect for mouse, build keyboard-first" stance for a feature
  whose entire point is jumping via links. Rejected.
- **A flat `String` plus a parallel `Vec<(Range<usize>, target)>` on
  `HelpTopic` or `Block`, instead of `Vec<Span>`.** Keeps `Block::Paragraph`'s
  shape unchanged, but the ranges would be byte offsets into *source* text
  that reflow (word-wrap) immediately invalidates — the wrap step would need
  span-like provenance internally regardless, so this only moves the
  complexity without avoiding it, while diverging from what ADR 0013 already
  named as the expected evolution. Rejected in favor of `Vec<Span>`.

## Addendum (2026-07-03): `ListBox::always_show_selection`

Manual verification of the feature above surfaced a usability gap: `ListBox`
only draws its selected row's highlight while it itself is focused (its
long-standing default, shared by every consumer). The moment focus moved
from `HelpWindow`'s topic list to its page pane — the normal, expected state
while reading a topic or following a link — the list went visually blank,
so "what topic am I looking at?" had no answer without tabbing back.

**Decision:** an opt-in `ListBox::always_show_selection(bool)` builder
method (default `false`, so every existing consumer — `FileDialog` included
— is unaffected) that, when set, keeps the selected row visibly marked while
unfocused, in a new dimmer role, `Role::SelectionInactive`, instead of the
default `Role::Selection`. This mirrors `Role::WindowTitle`/
`WindowTitleInactive`'s existing "same information, receded when not the
active one" relationship rather than inventing a new visual vocabulary.
`HelpWindow` opts its topic list in at construction; nothing else does.

Rejected: changing `ListBox`'s default behaviour outright (would alter
`FileDialog`'s look without that being asked for, and "always show" isn't
obviously correct for every list) and duplicating a "draw a dim current-item
marker" implementation inside `HelpWindow` itself (this is squarely
`ListBox`'s own drawing logic to own, per ADR 0005's colour-by-role
philosophy).
