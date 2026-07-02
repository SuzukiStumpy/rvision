# Module spec: `rvision::widgets::menu`

- **Status:** Done
- **Phase:** Backlog — roadmap's "Cascading menus (submenus)" (landed)
- **Related ADRs:** 0009 (application shell + menu overlay — the state
  machine and draw-last-as-overlay decision this spec extends), 0018
  (cascading menus — the path stack, anchor rule, hover behaviour, nesting
  depth, and item-level gating this spec now bakes in)

## Purpose

`MenuBar`/`Menu`/`MenuItem`: the top-row bar and its pull-downs. Today a
pull-down is one level deep — every item either posts a `Command` or is
disabled. This spec covers **cascading**: an item that opens a nested
pull-down (a submenu) instead of posting a command, so choosing it drills
down rather than closing the bar.

Not covered here: right-click context menus (a separate backlog item that
reuses this same nesting/anchoring machinery once it exists — best built
after this) or the `exec_view` modal loop (Phase 5; menus stay the
hand-rolled state machine ADR 0009 chose, not a modal view).

This spec supersedes the `MenuBar`/`Menu`/`MenuItem` section previously in
[`widgets.md`](widgets.md) — that file now points here.

## Public interface

```rust
pub struct MenuItem { label: String, shortcut: Option<String>, hotkey: Option<char>, action: MenuAction, gate: Option<Command> }
enum MenuAction { Command(Command), Submenu(Menu) }
impl MenuItem {
    pub fn new(label: &str, command: Command) -> Self;     // unchanged: posts `command`; gate = Some(command)
    pub fn submenu(label: &str, menu: Menu) -> Self;        // new: opens `menu` instead; gate = None (always enabled)
    pub fn with_shortcut(self, shortcut: &str) -> Self;     // unchanged; not meaningful on a submenu item
    pub fn with_hotkey(self, hotkey: char) -> Self;         // unchanged
    pub fn with_gate(self, command: Command) -> Self;       // new: ties this item's enabled state to
                                                             // `command`'s CommandSet entry (ADR 0018) —
                                                             // for opting a Submenu item into disabling
}

pub struct Menu { title: String, items: Vec<MenuItem>, hotkey: Option<char> }
impl Menu {
    pub fn new(title: &str, items: Vec<MenuItem>) -> Self;  // unchanged
    pub fn with_hotkey(self, hotkey: char) -> Self;         // unchanged
}

// `open: Option<usize>` + `highlight: usize` generalize to a path (see Behaviour).
pub struct MenuBar { bounds: Rect, menus: Vec<Menu>, path: Vec<usize>, .. }
impl MenuBar {
    pub fn new(bounds: Rect, menus: Vec<Menu>, theme: &Theme) -> Self; // unchanged
    pub fn set_bounds(&mut self, bounds: Rect);   // unchanged
    pub fn is_open(&self) -> bool;                // unchanged: true at any path depth > 0
    pub fn close(&mut self);                      // unchanged: clears the whole path
    pub fn draw_overlay(&self, canvas: &mut Canvas); // now draws every open level, cascaded
}
impl View for MenuBar { /* unchanged signatures */ }
```

## Behaviour & invariants

Unchanged from today (moved here from `widgets.md`, still true at every
depth unless a bullet below says otherwise):

- *Closed:* consumes `Alt`+a title's hot-key (opens that menu) and `F10`
  (opens the first menu). Every other event is ignored.
- Each `Menu`/`MenuItem`'s hot-key defaults to its title/label's first
  letter (case-insensitive), highlighted in `Role::MenuHotkey`; overridden
  with `with_hotkey` on collision.
- A disabled item's command is gated by `Context` (never posted); choosing
  it (`Enter`/hot-key/click) still closes like TV.
- The bar draws titles separated by spaces; the open title is highlighted.
  Item rows draw shortcuts right-aligned, the highlight in `MenuSelected`,
  disabled in `MenuDisabled`.

New, for cascading (state-machine shape and rules decided in ADR 0018):

- A `MenuItem` is either a `Command` item or a `Submenu` item, never both.
  A `Submenu` item ignores `with_shortcut` — its right-aligned slot instead
  shows a fixed cascade mark (`▸`) so the item reads as "opens something."
- Every item, `Command` or `Submenu`, is enabled-gated the same way, through
  `gate: Option<Command>` checked against the `CommandSet` pushed in by
  `sync_enabled` — no separate mechanism for the two item shapes. `new`
  sets `gate = Some(command)` (today's behaviour, unchanged); `submenu`
  defaults `gate = None` (always enabled), opted into disabling via
  `with_gate`. A disabled `Submenu` item draws greyed (`Role::MenuDisabled`,
  no hot-key highlight, same as a disabled `Command` item) and its cascade
  never opens; choosing it (`Enter`/hot-key/click) still closes/pops like a
  disabled `Command` item does. A branch's availability is never derived
  from its descendants — only the item itself carries the gate (ADR 0018).
- `open: Option<usize>` + `highlight: usize` generalize to `path: Vec<usize>`.
  `path[0]` is the bar-level menu index, exactly as `open` was. `path[i]`
  for `i > 0` is the highlighted item within the submenu opened by
  `path[i - 1]`. The **last** entry is the focused level — `Up`/`Down`,
  hot-key matching, and mouse hover/hit-testing all act on it alone. `path`
  may grow to any depth with no artificial cap — a `Submenu`'s `Menu` can
  itself contain `Submenu` items (ADR 0018).
- `Right` or `Enter` on a focused, highlighted, enabled `Submenu` item
  pushes a new `0` onto `path` and opens that submenu (focus moves to it).
  `Enter`/a hot-key on a `Command` item posts it and clears `path` entirely
  (closes every level), same as today. Opening a submenu is never triggered
  by hover — `MouseKind::Moved` only ever moves the highlight, exactly as
  it does today; there is no hover-delay auto-open (ADR 0018).
- `Left` or `Esc` at depth > 0 (`path.len() > 1`) pops the last entry,
  closing the deepest submenu and returning focus/highlight to its parent
  item. At depth 0 they keep today's meaning: `Left`/`Right` cycle sibling
  top-level menus (wrap), `Esc` closes the bar.
- Every ancestor level along `path` stays drawn while a descendant is open
  — TV cascades all open boxes at once, not one at a time. Clicking or
  hovering an item on an ancestor level truncates `path` to that level
  before acting (so switching a sibling mid-drill collapses everything
  below it first).
- `draw_overlay` draws each level of `path` left to right, one box per
  depth, anchored beside the parent item that opened it: to its right,
  top-aligned with its row, flipping to the parent's left edge if it would
  run off the right of the screen (ADR 0018) — the same clamp shape
  `pulldown_area` already applies to a single top-level box, evaluated once
  per open level.
- Hit-testing (`item_at` and friends) tests the open levels **deepest
  first**: a screen point under two overlapping boxes resolves to the
  descendant, since it draws on top.

## Collaborators

Unchanged: `Canvas`/`Buffer` (draw), `geometry::{Rect, Point, Size}`,
`theme::{Role, Theme}`, `command::{Command, CommandSet}`,
`view::{View, Context}`, `event` types. A `Submenu` action embeds a `Menu`
by value — still no cross-widget references (ADR 0003).

## Test plan (write these first)

- **Logic:** opening a `Submenu` item pushes a `path` level with the right
  parent index; nesting two levels deep behaves the same way a second time
  (proving `path` genuinely recurses, not just one hard-coded level);
  `Left`/`Esc` at depth > 0 pop one level without closing the bar; `Left`/
  `Esc` at depth 0 keep today's behaviour; choosing a `Command` item at any
  depth posts + clears the whole `path`; acting on a sibling item at an
  ancestor level while a descendant is open truncates `path` first; a
  disabled `Submenu` item (via `with_gate`) can't be opened and its choice
  closes/pops without opening anything, mirroring a disabled `Command`
  item.
- **Render (snapshot):** a bar with one open top-level menu whose
  highlighted item is a `Submenu`, drawn open beside it, anchored right and
  top-aligned (two cascaded boxes, cascade mark on the parent item); the
  same but flipped to the parent's left edge when the box would run off
  the right of the screen; a disabled `Submenu` item's mark and greyed
  styling vs. an enabled one's.
- **Interaction (scripted events):** keyboard-drill two levels deep and
  back out with `Left`/`Esc`; `Enter` on a leaf `Command` item posts and
  closes every level; mouse click opens a submenu, a click elsewhere
  collapses the whole `path`; hovering back onto an ancestor's item
  truncates `path` to it; hovering (without clicking) a `Submenu` item only
  moves the highlight and never opens it; a disabled `Submenu` item's
  hot-key/`Enter`/click closes without opening or posting.
- **Manual:** the `chrome` example, extended with a submenu-bearing item.

## Open questions

None currently. The five items raised while drafting this spec (anchor
rule, hover-to-open, nesting depth, branch-level disabling, and whether
this needed its own ADR) were resolved in ADR 0018; the decisions are
folded into Behaviour & invariants above.
