//! The menu bar and its cascading pull-downs (TurboVision's `TMenuBar` /
//! `TMenuBox`).
//!
//! The bar shows menu titles across the top row; opening one drops a pull-down
//! listing its items. An item can itself open a nested pull-down (a submenu)
//! instead of posting a command, cascading sideways as far as the app nests
//! them (ADR 0018). There is no modal loop yet (that is Phase 5's `exec_view`):
//! the open/highlight state lives on the [`MenuBar`] and the application shell
//! drives it — feeding it keys first (so it can claim `Alt`-hot-keys and, while
//! open, run modally) and drawing its pull-downs last, as an overlay over the
//! whole frame (ADR 0009).

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::{Color, Style};
use crate::command::{Command, CommandSet};
use crate::event::{Event, EventResult, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

/// The trailing glyph shown in place of a shortcut on a `Submenu` item,
/// marking that choosing it opens another pull-down rather than posting a
/// command (ADR 0018).
const SUBMENU_MARK: &str = "▸";

/// What choosing a [`MenuItem`] does: post a command, or open a nested
/// pull-down.
enum MenuAction {
    Command(Command),
    Submenu(Menu),
}

/// One entry in a pull-down menu: a label and what choosing it does (post a
/// command, or cascade into a nested pull-down), plus an optional shortcut
/// shown right-aligned (the accelerator itself is the status line's / app's
/// job; this is only the reminder text).
pub struct MenuItem {
    label: String,
    action: MenuAction,
    shortcut: Option<String>,
    /// The accelerator letter: highlighted in the drawn label and, while this
    /// item's menu is open, chosen by pressing it (no `Alt`) without needing
    /// `Up`/`Down` first. Defaults to `label`'s first character; override with
    /// [`with_hotkey`](Self::with_hotkey) once two items in the same menu would
    /// otherwise collide (e.g. "Save" / "Save As").
    hotkey: Option<char>,
    /// The `Command` whose `CommandSet` entry gates this item's enabled state
    /// — checked by [`enabled`](Self::enabled), never posted for a `Submenu`
    /// item. A `Command` item's gate is always its own command; a `Submenu`
    /// item has none by default (always enabled) unless opted in via
    /// [`with_gate`](Self::with_gate) (ADR 0018) — a submenu's availability is
    /// never derived from its descendants, only stated directly on the item.
    gate: Option<Command>,
}

impl MenuItem {
    /// Creates an item labelled `label` that posts `command` when chosen.
    pub fn new(label: &str, command: Command) -> Self {
        Self {
            label: label.to_string(),
            action: MenuAction::Command(command),
            shortcut: None,
            hotkey: label.chars().next().map(|c| c.to_ascii_lowercase()),
            gate: Some(command),
        }
    }

    /// Creates an item labelled `label` that opens `menu` as a cascading
    /// submenu when chosen, instead of posting a command. Enabled by default;
    /// see [`with_gate`](Self::with_gate) to disable the whole branch.
    pub fn submenu(label: &str, menu: Menu) -> Self {
        Self {
            label: label.to_string(),
            action: MenuAction::Submenu(menu),
            shortcut: None,
            hotkey: label.chars().next().map(|c| c.to_ascii_lowercase()),
            gate: None,
        }
    }

    /// Adds the right-aligned shortcut reminder (e.g. `"Ctrl-N"`). Not
    /// meaningful on a [`submenu`](Self::submenu) item — its trailing slot
    /// always shows the cascade mark instead.
    pub fn with_shortcut(mut self, shortcut: &str) -> Self {
        self.shortcut = Some(shortcut.to_string());
        self
    }

    /// Overrides the accelerator letter (case-insensitive) used to highlight
    /// and choose this item while its menu is open.
    pub fn with_hotkey(mut self, hotkey: char) -> Self {
        self.hotkey = Some(hotkey.to_ascii_lowercase());
        self
    }

    /// Ties this item's enabled state to `command`'s `CommandSet` entry
    /// (ADR 0018). Meaningful on a [`submenu`](Self::submenu) item, which has
    /// no command of its own to gate on otherwise; a disabled item's cascade
    /// never opens, greys out, and closes the bar like TV if chosen anyway.
    pub fn with_gate(mut self, command: Command) -> Self {
        self.gate = Some(command);
        self
    }

    /// The accelerator letter, if any.
    fn hotkey(&self) -> Option<char> {
        self.hotkey
    }

    /// The command this item posts, or `None` for a `Submenu` item.
    fn command(&self) -> Option<Command> {
        match self.action {
            MenuAction::Command(command) => Some(command),
            MenuAction::Submenu(_) => None,
        }
    }

    /// The nested menu this item opens, or `None` for a `Command` item.
    fn submenu_ref(&self) -> Option<&Menu> {
        match &self.action {
            MenuAction::Submenu(menu) => Some(menu),
            MenuAction::Command(_) => None,
        }
    }

    /// Whether this item is currently enabled, per its `gate` (`None` always
    /// reads as enabled — the default for a `Submenu` item).
    fn enabled(&self, commands: &CommandSet) -> bool {
        self.gate.is_none_or(|c| commands.is_enabled(c))
    }
}

/// One pull-down: a title and its items. The title's accelerator letter is its
/// `Alt`-hot-key (case-insensitive) and is highlighted in the drawn title.
pub struct Menu {
    title: String,
    items: Vec<MenuItem>,
    /// Defaults to `title`'s first character; override with
    /// [`with_hotkey`](Self::with_hotkey) once two menus would otherwise collide.
    hotkey: Option<char>,
}

impl Menu {
    /// Creates a menu titled `title` listing `items`.
    pub fn new(title: &str, items: Vec<MenuItem>) -> Self {
        Self {
            title: title.to_string(),
            items,
            hotkey: title.chars().next().map(|c| c.to_ascii_lowercase()),
        }
    }

    /// Overrides the `Alt`-hot-key (case-insensitive) that opens this menu and
    /// is highlighted in its title.
    pub fn with_hotkey(mut self, hotkey: char) -> Self {
        self.hotkey = Some(hotkey.to_ascii_lowercase());
        self
    }

    /// The `Alt`-hot-key that opens this menu.
    fn hotkey(&self) -> Option<char> {
        self.hotkey
    }
}

/// The top-row menu bar.
pub struct MenuBar {
    bounds: Rect,
    menus: Vec<Menu>,
    /// The open cascade, root first: `path[0]` is the bar-level menu index
    /// (as a single-level `open: Option<usize>` used to be); `path[i]` for
    /// `i > 0` is the highlighted item within the pull-down opened by
    /// `path[i - 1]` — which doubles as "the item whose submenu is open at
    /// the next depth," so one `usize` per level carries both meanings
    /// (ADR 0018). Empty means closed. The **last** entry is always the
    /// focused level: `Up`/`Down`, hot-keys, hover, and hit-testing act on it
    /// alone. Opening a menu always seeds two entries (`[index, 0]`) since a
    /// highlight always exists the moment a pull-down is showing.
    path: Vec<usize>,
    bar_style: Style,
    selected_style: Style,
    disabled_style: Style,
    /// The accelerator letter's foreground ([`Role::MenuHotkey`]), composed onto
    /// whichever background a title or item is currently drawn in.
    hotkey_fg: Color,
    /// Which commands are live, pushed in before a draw so disabled items can grey
    /// themselves (the same state-in-draw "push" as `View::set_focused`). Empty by
    /// default, so every item is enabled until the app says otherwise. Also the
    /// only plumbing a `Submenu` item's [`with_gate`](MenuItem::with_gate) has to
    /// check against — unlike a `Command` item, it has no command to post through
    /// `Context`, so there is no separate gate at that end (ADR 0018).
    commands: CommandSet,
}

impl MenuBar {
    /// Creates a menu bar at `bounds` from `menus`, taking its colours from
    /// `theme` ([`Role::MenuBar`], [`Role::MenuSelected`], [`Role::MenuDisabled`],
    /// [`Role::MenuHotkey`]).
    pub fn new(bounds: Rect, menus: Vec<Menu>, theme: &Theme) -> Self {
        Self {
            bounds,
            menus,
            path: Vec::new(),
            bar_style: theme.style(Role::MenuBar),
            selected_style: theme.style(Role::MenuSelected),
            disabled_style: theme.style(Role::MenuDisabled),
            hotkey_fg: theme.style(Role::MenuHotkey).fg,
            commands: CommandSet::new(),
        }
    }

    /// Pushes the current command-enabled state in before a draw, so a pull-down
    /// can grey the items whose command is disabled (ADR 0003/0004). Call it from
    /// the same place the app keeps its [`CommandSet`] up to date; dispatch already
    /// gates disabled commands, so this is purely the visual half.
    pub fn sync_enabled(&mut self, commands: &CommandSet) {
        self.commands = commands.clone();
    }

    /// Repositions the bar (the shell calls this as the terminal resizes).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    /// Whether a pull-down is currently open, at any nesting depth (the shell
    /// routes all keys here while it is, ADR 0009).
    pub fn is_open(&self) -> bool {
        !self.path.is_empty()
    }

    /// Closes the whole cascade, at every depth.
    pub fn close(&mut self) {
        self.path.clear();
    }

    /// The starting column of each menu title's text, laid out left to right with
    /// one space of padding on each side.
    fn title_starts(&self) -> Vec<i16> {
        let mut xs = Vec::with_capacity(self.menus.len());
        let mut x = 0;
        for menu in &self.menus {
            x += 1;
            xs.push(x);
            x += menu.title.chars().count() as i16 + 1;
        }
        xs
    }

    /// The menu whose title (with its one-space padding) sits under bar column `x`,
    /// or `None` for an empty stretch of the bar. Matches the highlighted ` Title `
    /// region drawn by [`draw`](Self::draw).
    fn title_at(&self, x: i16) -> Option<usize> {
        let starts = self.title_starts();
        self.menus.iter().enumerate().find_map(|(i, menu)| {
            let start = starts[i];
            let len = menu.title.chars().count() as i16;
            (x >= start - 1 && x <= start + len).then_some(i)
        })
    }

    /// Opens bar-level menu `index` (clamped), discarding any previous cascade
    /// and seeding a fresh highlight at its first item.
    fn open_menu(&mut self, index: usize) {
        if index < self.menus.len() {
            self.path = vec![index, 0];
        }
    }

    /// The `Menu` at each currently open level, root first, deepest (focused)
    /// last. Empty when closed. `path[1..]` (all but the root index) doubles as
    /// both "the highlight to draw at that level" and — for every level but the
    /// last — "which item's submenu is the next level," so walking one level
    /// short of `path`'s end reconstructs the whole chain (ADR 0018).
    fn open_menus(&self) -> Vec<&Menu> {
        let mut menus = Vec::new();
        let Some(&root) = self.path.first() else {
            return menus;
        };
        menus.push(&self.menus[root]);
        for &idx in &self.path[1..self.path.len() - 1] {
            let parent = *menus.last().expect("just pushed the root menu");
            let next = parent
                .items
                .get(idx)
                .and_then(|item| item.submenu_ref())
                .expect("path invariant: an ancestor highlight always names a Submenu item");
            menus.push(next);
        }
        menus
    }

    /// The box (border included) for each currently open level, root first, in
    /// the same screen coordinates the overlay draws in. Level 0 anchors under
    /// the bar, exactly as the single-level pull-down always has; level `d > 0`
    /// anchors to the right of level `d - 1`'s box, aligned with the row of the
    /// item that opened it, flipping to that box's left edge if it would run
    /// off the right of the screen (ADR 0018). The single source of the cascade
    /// geometry, shared by [`draw_overlay`](Self::draw_overlay) and
    /// hit-testing.
    fn level_areas(&self, screen_w: i16) -> Vec<Rect> {
        let menus = self.open_menus();
        let mut areas: Vec<Rect> = Vec::with_capacity(menus.len());
        for (level, menu) in menus.iter().enumerate() {
            let area = if level == 0 {
                self.pulldown_area(self.path[0], menu, screen_w)
            } else {
                let parent_area = areas[level - 1];
                let parent_highlight = self.path[level];
                let box_w = self.pulldown_width(menu);
                let box_h = menu.items.len() as i16 + 2;
                let top = parent_area.origin().y + 1 + parent_highlight as i16;
                let right_open_left = parent_area.origin().x + parent_area.width();
                let left = if right_open_left + box_w <= screen_w {
                    right_open_left
                } else {
                    (parent_area.origin().x - box_w).max(0)
                };
                Rect::from_origin_size(Point::new(left, top), Size::new(box_w, box_h))
            };
            areas.push(area);
        }
        areas
    }

    /// The `(level, item index)` under `pos` (screen coordinates), testing open
    /// levels **deepest first** so a point under two overlapping boxes resolves
    /// to the descendant, since it draws on top. `None` if `pos` is outside
    /// every open box.
    fn level_item_at(&self, pos: Point, screen_w: i16) -> Option<(usize, usize)> {
        let menus = self.open_menus();
        if menus.is_empty() {
            return None;
        }
        let areas = self.level_areas(screen_w);
        for level in (0..menus.len()).rev() {
            let menu = menus[level];
            if menu.items.is_empty() {
                continue;
            }
            let area = areas[level];
            let left = area.origin().x;
            let box_w = area.width();
            if pos.x <= left || pos.x >= left + box_w - 1 {
                continue;
            }
            let first_row = area.origin().y + 1;
            let row = pos.y - first_row;
            if row >= 0 && (row as usize) < menu.items.len() {
                return Some((level, row as usize));
            }
        }
        None
    }

    /// Whether item `idx` at `level` is already expanded — its submenu is
    /// genuinely open as the next level, not just highlighted. Matching the
    /// highlight alone isn't enough: navigating onto an item with `Up`/`Down`
    /// sets `path[level + 1]` to it before its submenu has ever been opened,
    /// and that must *not* read as "already open" (ADR 0018).
    fn already_expanded(&self, level: usize, idx: usize) -> bool {
        self.path.get(level + 1) == Some(&idx) && self.path.len() > level + 2
    }

    /// Moves the highlight to item `idx` at level `level`, truncating any
    /// deeper cascade first — used by hover, which only ever relocates the
    /// highlight and never opens a submenu (ADR 0018). A no-op if `idx` is
    /// already expanded: the cursor rests over a submenu's *parent* row (the
    /// child box opens beside it, not over it), so the very next hover after
    /// opening a submenu would otherwise immediately collapse it again.
    fn hover(&mut self, level: usize, idx: usize) {
        if self.already_expanded(level, idx) {
            return;
        }
        self.path.truncate(level + 1);
        self.path.push(idx);
    }

    /// Acts on item `idx` at level `level`: a `Command` item posts (gated by
    /// `Context`, ADR 0003) and closes the whole cascade; an enabled `Submenu`
    /// item opens as the next level (truncating anything deeper first, so
    /// re-choosing a different sibling replaces whatever was open below it) —
    /// unless it is already expanded, in which case choosing it again is a
    /// no-op, preserving whatever was navigated inside; a disabled `Submenu`
    /// item closes the whole cascade without opening, mirroring a disabled
    /// `Command` item (ADR 0018). Shared by `Enter`, a hot-key letter, and a
    /// mouse click, at any level.
    fn choose(&mut self, level: usize, idx: usize, ctx: &mut Context) {
        let (is_submenu, command, enabled) = {
            let menus = self.open_menus();
            let item = &menus[level].items[idx];
            (
                item.submenu_ref().is_some(),
                item.command(),
                item.enabled(&self.commands),
            )
        };
        if is_submenu {
            if !enabled {
                self.close();
            } else if !self.already_expanded(level, idx) {
                self.path.truncate(level + 1);
                self.path.push(idx);
                self.path.push(0);
            }
        } else {
            let command = command.expect("a non-submenu MenuItem always holds a Command");
            self.close();
            ctx.post(command); // Gated by Context: a disabled item posts nothing (ADR 0003).
        }
    }

    /// Cycles the bar-level open menu to its next (`+1`) or previous (`-1`)
    /// sibling, wrapping, and resets to a fresh pull-down (no submenu carried
    /// over) — only reached at the root of the cascade.
    fn cycle_top_level(&mut self, direction: i16) {
        let n = self.menus.len() as i16;
        if n == 0 {
            return;
        }
        let current = self.path[0] as i16;
        let next = (current + direction).rem_euclid(n);
        self.path = vec![next as usize, 0];
    }

    /// `Right`: opens the focused item's submenu if it has one and is enabled;
    /// otherwise, at the root of the cascade, cycles to the next top-level menu
    /// (today's behaviour); otherwise (nested, no submenu here) does nothing.
    fn handle_right(&mut self) {
        let submenu_enabled = {
            let menus = self.open_menus();
            let focused = menus.last().expect("handle_right called while closed");
            focused
                .items
                .get(*self.path.last().unwrap())
                .is_some_and(|item| item.submenu_ref().is_some() && item.enabled(&self.commands))
        };
        if submenu_enabled {
            self.path.push(0);
        } else if self.path.len() == 2 {
            self.cycle_top_level(1);
        }
    }

    /// `Up`/`Down`: moves the highlight within the focused (deepest) level,
    /// wrapping.
    fn move_highlight(&mut self, direction: i16) {
        let items = {
            let menus = self.open_menus();
            menus
                .last()
                .expect("move_highlight called while closed")
                .items
                .len()
        };
        if items == 0 {
            return;
        }
        let last = self
            .path
            .last_mut()
            .expect("move_highlight called while closed");
        *last = (*last as i16 + direction).rem_euclid(items as i16) as usize;
    }

    /// `Enter`: chooses the focused level's highlighted item, if any.
    fn choose_highlighted(&mut self, ctx: &mut Context) {
        let level = self.path.len() - 2;
        let idx = *self
            .path
            .last()
            .expect("choose_highlighted called while closed");
        let has_items = !self.open_menus()[level].items.is_empty();
        if has_items {
            self.choose(level, idx, ctx);
        }
    }

    /// A hot-key letter: chooses the focused level's item whose hot-key matches
    /// `c`, if any.
    fn choose_hotkey(&mut self, c: char, ctx: &mut Context) {
        let c = c.to_ascii_lowercase();
        let found = {
            let level = self.path.len() - 2;
            let menus = self.open_menus();
            menus[level]
                .items
                .iter()
                .position(|item| item.hotkey() == Some(c))
                .map(|idx| (level, idx))
        };
        if let Some((level, idx)) = found {
            self.choose(level, idx, ctx);
        }
    }

    /// Runs the modal key handling while any level is open: arrows navigate
    /// and cascade, `Enter` chooses, a hot-key letter jumps straight to and
    /// chooses its item, `Esc` closes one level (or the whole bar at the
    /// root); every other key is swallowed so nothing leaks to the editor
    /// underneath. Returns the result (always `Consumed` while open).
    fn handle_open(&mut self, code: KeyCode, ctx: &mut Context) -> EventResult {
        match code {
            KeyCode::Esc => {
                if self.path.len() > 2 {
                    self.path.pop(); // close the deepest submenu, refocus its parent item
                } else {
                    self.close();
                }
            }
            KeyCode::Left => {
                if self.path.len() > 2 {
                    self.path.pop();
                } else {
                    self.cycle_top_level(-1);
                }
            }
            KeyCode::Right => self.handle_right(),
            KeyCode::Up => self.move_highlight(-1),
            KeyCode::Down => self.move_highlight(1),
            KeyCode::Enter => self.choose_highlighted(ctx),
            KeyCode::Char(c) => self.choose_hotkey(c, ctx),
            _ => {}
        }
        EventResult::Consumed
    }

    /// Tries to open a menu from a closed bar: `Alt`+a title's first letter, or
    /// `F10` for the first menu. Returns whether it claimed the key.
    fn handle_closed(&mut self, code: KeyCode, modifiers: Modifiers) -> EventResult {
        match code {
            KeyCode::F(10) if !self.menus.is_empty() => {
                self.open_menu(0);
                EventResult::Consumed
            }
            KeyCode::Char(c) if modifiers.contains(Modifiers::ALT) => {
                let c = c.to_ascii_lowercase();
                if let Some(index) = self.menus.iter().position(|m| m.hotkey() == Some(c)) {
                    self.open_menu(index);
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    /// The pull-down box (border included) for the bar-level menu `index`, in
    /// screen coordinates: anchored a row below the bar, its left edge under
    /// the title but pulled back so the box stays on screen. Level 0 of
    /// [`level_areas`](Self::level_areas); deeper levels anchor off it.
    fn pulldown_area(&self, index: usize, menu: &Menu, screen_w: i16) -> Rect {
        let starts = self.title_starts();
        let box_w = self.pulldown_width(menu);
        let box_h = menu.items.len() as i16 + 2;
        let left = (starts[index] - 1).min(screen_w - box_w).max(0);
        Rect::from_origin_size(Point::new(left, 1), Size::new(box_w, box_h))
    }

    /// Routes a mouse event (positions in screen coordinates, the same space the
    /// bar and its overlay draw in — the bar sits on row `bounds.y`, the
    /// pull-downs the rows below and beside). Clicking a title opens it (or
    /// toggles it shut); clicking an item at any open level chooses it;
    /// clicking anywhere else dismisses the whole cascade. While open, moving
    /// over a title or item tracks the highlight (truncating anything deeper,
    /// TV feel) but never opens a submenu on its own (ADR 0018).
    fn handle_mouse(&mut self, mouse: &MouseEvent, ctx: &mut Context) -> EventResult {
        let on_bar = mouse.pos.y == self.bounds.origin().y;
        let screen_w = self.bounds.width();
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) => {
                if on_bar {
                    return match self.title_at(mouse.pos.x) {
                        Some(i) if self.path.first() == Some(&i) => {
                            self.close(); // a second click on the open title shuts it
                            EventResult::Consumed
                        }
                        Some(i) => {
                            self.open_menu(i);
                            EventResult::Consumed
                        }
                        // Bare stretch of the bar: dismiss the whole cascade.
                        None if self.is_open() => {
                            self.close();
                            EventResult::Consumed
                        }
                        None => EventResult::Ignored,
                    };
                }
                if self.is_open() {
                    if let Some((level, idx)) = self.level_item_at(mouse.pos, screen_w) {
                        self.choose(level, idx, ctx);
                    } else {
                        self.close(); // click off every open box dismisses the cascade
                    }
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            MouseKind::Moved | MouseKind::Drag(MouseButton::Left) if self.is_open() => {
                if let Some((level, idx)) = self.level_item_at(mouse.pos, screen_w) {
                    self.hover(level, idx);
                    EventResult::Consumed
                } else if on_bar {
                    if let Some(i) = self.title_at(mouse.pos.x) {
                        if self.path.first() != Some(&i) {
                            self.open_menu(i); // slide across the bar with the button down
                        }
                        return EventResult::Consumed;
                    }
                    EventResult::Ignored
                } else {
                    EventResult::Ignored
                }
            }
            _ => EventResult::Ignored,
        }
    }

    /// Draws every open level over the whole frame, cascaded left to right. The
    /// shell calls this after everything else so the boxes sit on top
    /// (ADR 0009); a closed bar draws nothing.
    pub fn draw_overlay(&self, canvas: &mut Canvas) {
        if self.path.is_empty() {
            return;
        }
        let menus = self.open_menus();
        let areas = self.level_areas(canvas.size().width);
        for (level, menu) in menus.iter().enumerate() {
            if menu.items.is_empty() {
                continue;
            }
            let area = areas[level];
            let left = area.origin().x;
            let box_w = area.width();
            let highlight = self.path[level + 1];

            canvas.fill(area, &Cell::blank(self.bar_style));
            canvas.draw_box(area, self.bar_style);
            for (i, item) in menu.items.iter().enumerate() {
                let row = area.origin().y + 1 + i as i16;
                let disabled = !item.enabled(&self.commands);
                // A disabled item can't light up: it stays greyed even on the
                // highlight row, so the whole line (fill included) reads as
                // unavailable.
                let style = if disabled {
                    self.disabled_style
                } else if i == highlight {
                    self.selected_style
                } else {
                    self.bar_style
                };
                // Repaint the interior row so the highlight is a full-width bar.
                let inner =
                    Rect::from_origin_size(Point::new(left + 1, row), Size::new(box_w - 2, 1));
                canvas.fill(inner, &Cell::blank(style));
                if disabled {
                    // No hot-key highlight either: a letter that can't be
                    // pressed shouldn't be singled out.
                    canvas.put_str(Point::new(left + 2, row), &item.label, style);
                } else {
                    self.put_hotkey_str(
                        canvas,
                        Point::new(left + 2, row),
                        &item.label,
                        style,
                        item.hotkey(),
                    );
                }
                match &item.action {
                    MenuAction::Command(_) => {
                        if let Some(shortcut) = &item.shortcut {
                            let sx = left + box_w - 2 - shortcut.chars().count() as i16;
                            canvas.put_str(Point::new(sx, row), shortcut, style);
                        }
                    }
                    MenuAction::Submenu(_) => {
                        let sx = left + box_w - 2 - SUBMENU_MARK.chars().count() as i16;
                        canvas.put_str(Point::new(sx, row), SUBMENU_MARK, style);
                    }
                }
            }
        }
    }

    /// Draws `text` at `at` in `style`, except its hot-key character (if any),
    /// which is drawn with the foreground swapped to [`Role::MenuHotkey`]'s
    /// colour while keeping `style`'s background and attributes. Shared by a bar
    /// title and an enabled pull-down item; a disabled item skips this and calls
    /// [`Canvas::put_str`] directly instead. Returns the ending column, like
    /// [`Canvas::put_str`].
    fn put_hotkey_str(
        &self,
        canvas: &mut Canvas,
        at: Point,
        text: &str,
        style: Style,
        hotkey: Option<char>,
    ) -> i16 {
        let Some((pre, key, post)) = hotkey.and_then(|hk| split_at_hotkey(text, hk)) else {
            return canvas.put_str(at, text, style);
        };
        let mut x = canvas.put_str(at, pre, style);
        x = canvas.put_str(Point::new(x, at.y), key, style.fg(self.hotkey_fg));
        canvas.put_str(Point::new(x, at.y), post, style)
    }

    /// The pull-down box width: widest "` label  trailer `" line (trailer being
    /// a shortcut for a `Command` item or the cascade mark for a `Submenu`
    /// item), plus borders.
    fn pulldown_width(&self, menu: &Menu) -> i16 {
        let label_w = menu
            .items
            .iter()
            .map(|it| it.label.chars().count())
            .max()
            .unwrap_or(0);
        let trailer_w = menu
            .items
            .iter()
            .map(|it| match &it.action {
                MenuAction::Command(_) => {
                    it.shortcut.as_ref().map(|s| s.chars().count()).unwrap_or(0)
                }
                MenuAction::Submenu(_) => SUBMENU_MARK.chars().count(),
            })
            .max()
            .unwrap_or(0);
        let gap = if trailer_w > 0 { trailer_w + 2 } else { 0 };
        // 1 leading + label + gap + 1 trailing, then +2 for the borders.
        (1 + label_w + gap + 1 + 2) as i16
    }
}

/// Splits `text` around the first case-insensitive occurrence of `hotkey`, for
/// drawing that one character in a different style. `None` if `hotkey` does not
/// occur in `text` (e.g. a `with_hotkey` override that names a letter its own
/// label doesn't contain), in which case the caller draws `text` plain.
fn split_at_hotkey(text: &str, hotkey: char) -> Option<(&str, &str, &str)> {
    let (i, ch) = text
        .char_indices()
        .find(|(_, c)| c.to_ascii_lowercase() == hotkey)?;
    let end = i + ch.len_utf8();
    Some((&text[..i], &text[i..end], &text[end..]))
}

impl View for MenuBar {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        canvas.fill(area, &Cell::blank(self.bar_style));
        let starts = self.title_starts();
        let open_title = self.path.first().copied();
        for (i, menu) in self.menus.iter().enumerate() {
            let start = starts[i];
            if open_title == Some(i) {
                // Highlight the open title together with its surrounding spaces.
                let label = format!(" {} ", menu.title);
                self.put_hotkey_str(
                    canvas,
                    Point::new(start - 1, 0),
                    &label,
                    self.selected_style,
                    menu.hotkey(),
                );
            } else {
                self.put_hotkey_str(
                    canvas,
                    Point::new(start, 0),
                    &menu.title,
                    self.bar_style,
                    menu.hotkey(),
                );
            }
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Key(key) if self.is_open() => self.handle_open(key.code, ctx),
            Event::Key(key) => self.handle_closed(key.code, key.modifiers),
            Event::Mouse(mouse) => self.handle_mouse(mouse, ctx),
            _ => EventResult::Ignored,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::{CM_USER, CommandSet};
    use crate::event::KeyEvent;

    const CM_NEW: Command = Command(CM_USER + 1);
    const CM_OPEN: Command = Command(CM_USER + 2);
    const CM_COPY: Command = Command(CM_USER + 3);
    const CM_PDF: Command = Command(CM_USER + 4);
    const CM_PNG: Command = Command(CM_USER + 5);
    const CM_EXPORT_GATE: Command = Command(CM_USER + 6);

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    fn bar() -> MenuBar {
        MenuBar::new(
            rect(0, 0, 40, 1),
            vec![
                Menu::new(
                    "File",
                    vec![
                        MenuItem::new("New", CM_NEW).with_shortcut("Ctrl-N"),
                        MenuItem::new("Open...", CM_OPEN).with_shortcut("Ctrl-O"),
                    ],
                ),
                Menu::new(
                    "Edit",
                    vec![MenuItem::new("Copy", CM_COPY).with_shortcut("Ctrl-C")],
                ),
            ],
            &Theme::default(),
        )
    }

    /// A bar whose File menu's last item, "Export", is a submenu of its own
    /// (PDF/PNG), for exercising cascading behaviour.
    fn bar_with_submenu() -> MenuBar {
        MenuBar::new(
            rect(0, 0, 40, 1),
            vec![Menu::new(
                "File",
                vec![
                    MenuItem::new("New", CM_NEW).with_shortcut("Ctrl-N"),
                    MenuItem::submenu(
                        "Export",
                        Menu::new(
                            "Export",
                            vec![MenuItem::new("PDF", CM_PDF), MenuItem::new("PNG", CM_PNG)],
                        ),
                    ),
                ],
            )],
            &Theme::default(),
        )
    }

    fn key(code: KeyCode, mods: Modifiers) -> Event {
        Event::Key(KeyEvent::new(code, mods))
    }

    /// Renders the bar like the shell does: the bar into a one-row sub-canvas at
    /// the top, then the pull-down overlay over the whole frame.
    fn render(bar: &MenuBar, w: i16, h: i16) -> String {
        let mut buf = Buffer::new(Size::new(w, h));
        let mut root = Canvas::new(&mut buf);
        {
            let mut barc = root.child(rect(0, 0, w, 1));
            bar.draw(&mut barc);
        }
        bar.draw_overlay(&mut root);
        buf.to_text()
    }

    // --- Drawing ---

    #[test]
    fn snapshot_closed_bar() {
        insta::assert_snapshot!(render(&bar(), 40, 6));
    }

    #[test]
    fn snapshot_open_file_menu() {
        let mut bar = bar();
        bar.handle_event(
            &key(KeyCode::Char('f'), Modifiers::ALT),
            &mut Context::new(&CommandSet::new()),
        );
        insta::assert_snapshot!(render(&bar, 40, 6));
    }

    // --- Opening / accelerators ---

    #[test]
    fn alt_letter_opens_the_matching_menu() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!bar.is_open());
        let r = bar.handle_event(&key(KeyCode::Char('e'), Modifiers::ALT), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(bar.path, vec![1, 0]); // Edit
    }

    #[test]
    fn f10_opens_the_first_menu() {
        let mut bar = bar();
        bar.handle_event(
            &key(KeyCode::F(10), Modifiers::NONE),
            &mut Context::new(&CommandSet::new()),
        );
        assert_eq!(bar.path, vec![0, 0]);
    }

    #[test]
    fn a_closed_bar_ignores_ordinary_keys() {
        // Plain letters (no Alt) must pass through to the editor.
        let mut bar = bar();
        let r = bar.handle_event(
            &key(KeyCode::Char('f'), Modifiers::NONE),
            &mut Context::new(&CommandSet::new()),
        );
        assert_eq!(r, EventResult::Ignored);
        assert!(!bar.is_open());
    }

    // --- Navigation while open ---

    #[test]
    fn left_right_switch_menus_and_wrap() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![1, 0]); // Edit
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![0, 0], "wraps back to File");
        bar.handle_event(&key(KeyCode::Left, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![1, 0], "and back the other way");
    }

    #[test]
    fn up_down_move_the_highlight_and_wrap() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File: 2 items
        assert_eq!(bar.path, vec![0, 0]);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![0, 1]);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![0, 0], "wraps");
        bar.handle_event(&key(KeyCode::Up, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![0, 1], "wraps the other way");
    }

    #[test]
    fn switching_menus_resets_the_highlight() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![0, 1]);
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![1, 0]);
    }

    // --- Choosing / dismissing ---

    #[test]
    fn enter_posts_the_highlighted_command_and_closes() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx); // Open...
        let r = bar.handle_event(&key(KeyCode::Enter, Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(!bar.is_open(), "choosing closes the menu");
        assert_eq!(ctx.posted(), &[Event::Command(CM_OPEN)]);
    }

    #[test]
    fn a_disabled_items_command_is_not_posted() {
        let mut bar = bar();
        let mut cs = CommandSet::new();
        cs.disable(CM_NEW);
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File, item 0 = New
        bar.handle_event(&key(KeyCode::Enter, Modifiers::NONE), &mut ctx);
        assert!(!bar.is_open(), "still closes, like TurboVision");
        assert!(
            ctx.posted().is_empty(),
            "but the disabled command never fires"
        );
    }

    #[test]
    fn a_disabled_item_draws_greyed() {
        // The visual half of disabled commands: an item whose command is disabled
        // draws in Role::MenuDisabled, even when it is the highlighted row (a
        // disabled item can't light up). Enabled items are unaffected.
        let mut bar = bar();
        let mut cs = CommandSet::new();
        cs.disable(CM_NEW); // File item 0 = New (also the row highlighted on open)
        bar.sync_enabled(&cs);
        bar.handle_event(
            &key(KeyCode::F(10), Modifiers::NONE),
            &mut Context::new(&cs),
        );

        let mut buf = Buffer::new(Size::new(40, 6));
        let mut root = Canvas::new(&mut buf);
        bar.draw_overlay(&mut root);

        let theme = Theme::default();
        // New on row 2 is disabled -> greyed despite being highlighted.
        assert_eq!(
            buf.get(Point::new(2, 2)).unwrap().style(),
            theme.style(Role::MenuDisabled)
        );
        // Open... on row 3 is enabled -> ordinary bar style (column 3, past its
        // hot-key 'O' at column 2, which now draws in Role::MenuHotkey).
        assert_eq!(
            buf.get(Point::new(3, 3)).unwrap().style(),
            theme.style(Role::MenuBar)
        );
    }

    // --- Hot-key letters (accelerators) ---

    #[test]
    fn hotkey_letter_selects_and_activates_the_item() {
        // Pressing an item's hot-key while its menu is open chooses it
        // immediately, like Enter, without needing Up/Down first.
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File
        let r = bar.handle_event(&key(KeyCode::Char('o'), Modifiers::NONE), &mut ctx); // Open...'s hot-key
        assert_eq!(r, EventResult::Consumed);
        assert!(!bar.is_open(), "choosing closes the menu");
        assert_eq!(ctx.posted(), &[Event::Command(CM_OPEN)]);
    }

    #[test]
    fn a_disabled_items_hotkey_closes_but_posts_nothing() {
        let mut bar = bar();
        let mut cs = CommandSet::new();
        cs.disable(CM_NEW);
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File, New = 'n'
        bar.handle_event(&key(KeyCode::Char('n'), Modifiers::NONE), &mut ctx);
        assert!(
            !bar.is_open(),
            "still closes, like Enter on a disabled item"
        );
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn an_unmatched_letter_is_swallowed_not_leaked() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File: 'n', 'o'
        let r = bar.handle_event(&key(KeyCode::Char('z'), Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(bar.is_open(), "no matching item, so nothing is chosen");
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn with_hotkey_overrides_the_default_first_letter() {
        let mut bar = MenuBar::new(
            rect(0, 0, 40, 1),
            vec![Menu::new(
                "File",
                vec![
                    MenuItem::new("Save", CM_NEW),
                    MenuItem::new("Save As...", CM_OPEN).with_hotkey('a'),
                ],
            )],
            &Theme::default(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        let r = bar.handle_event(&key(KeyCode::Char('a'), Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(
            ctx.posted(),
            &[Event::Command(CM_OPEN)],
            "'a' picks Save As..., not its default first letter 's'"
        );
    }

    #[test]
    fn menu_with_hotkey_overrides_the_titles_default_first_letter() {
        let mut bar = MenuBar::new(
            rect(0, 0, 40, 1),
            vec![
                Menu::new("File", vec![]),
                Menu::new("Search", vec![]).with_hotkey('r'),
            ],
            &Theme::default(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = bar.handle_event(&key(KeyCode::Char('r'), Modifiers::ALT), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(
            bar.path,
            vec![1, 0],
            "'r' opens Search via its overridden hot-key"
        );
    }

    // --- Hot-key drawing ---

    #[test]
    fn a_titles_hotkey_letter_draws_in_the_hotkey_colour() {
        let bar = bar();
        let mut buf = Buffer::new(Size::new(40, 6));
        let mut root = Canvas::new(&mut buf);
        bar.draw(&mut root.child(rect(0, 0, 40, 1)));

        let theme = Theme::default();
        // "File" starts at column 1 (column 0 is the leading pad space); 'F' is
        // its hot-key.
        assert_eq!(
            buf.get(Point::new(1, 0)).unwrap().style(),
            theme
                .style(Role::MenuBar)
                .fg(theme.style(Role::MenuHotkey).fg)
        );
        // The rest of the title is untouched.
        assert_eq!(
            buf.get(Point::new(2, 0)).unwrap().style(),
            theme.style(Role::MenuBar)
        );
    }

    #[test]
    fn an_items_hotkey_letter_draws_in_the_hotkey_colour_when_enabled() {
        let mut bar = bar();
        bar.handle_event(
            &key(KeyCode::F(10), Modifiers::NONE),
            &mut Context::new(&CommandSet::new()),
        ); // File open, item 0 = New (row 2), item 1 = Open... (row 3)

        let mut buf = Buffer::new(Size::new(40, 6));
        let mut root = Canvas::new(&mut buf);
        bar.draw_overlay(&mut root);

        let theme = Theme::default();
        // "New" on row 2 is selected (row 0, highlighted): 'N' still stands out
        // against the selected background, not the plain bar one.
        assert_eq!(
            buf.get(Point::new(2, 2)).unwrap().style(),
            theme
                .style(Role::MenuSelected)
                .fg(theme.style(Role::MenuHotkey).fg)
        );
        // "Open..." on row 3 is enabled but not highlighted: 'O' uses the plain
        // bar background.
        assert_eq!(
            buf.get(Point::new(2, 3)).unwrap().style(),
            theme
                .style(Role::MenuBar)
                .fg(theme.style(Role::MenuHotkey).fg)
        );
    }

    #[test]
    fn a_disabled_items_hotkey_letter_is_not_highlighted() {
        let mut bar = bar();
        let mut cs = CommandSet::new();
        cs.disable(CM_NEW); // File item 0 = New, also the row highlighted on open
        bar.sync_enabled(&cs);
        bar.handle_event(
            &key(KeyCode::F(10), Modifiers::NONE),
            &mut Context::new(&cs),
        );

        let mut buf = Buffer::new(Size::new(40, 6));
        let mut root = Canvas::new(&mut buf);
        bar.draw_overlay(&mut root);

        let theme = Theme::default();
        // 'N' in "New" reads as plain disabled style, not the hot-key colour.
        assert_eq!(
            buf.get(Point::new(2, 2)).unwrap().style(),
            theme.style(Role::MenuDisabled)
        );
    }

    #[test]
    fn esc_closes_without_posting() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        let r = bar.handle_event(&key(KeyCode::Esc, Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(!bar.is_open());
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn an_open_menu_swallows_unrelated_keys() {
        // While open the menu is modal: a stray letter is consumed, not leaked.
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        let r = bar.handle_event(&key(KeyCode::Char('z'), Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(bar.is_open());
    }

    // --- Mouse (Phase 9b) ---
    //
    // Layout of `bar()`: titles `File` (cols 0..=5) and `Edit` (cols 6..=11) on
    // row 0; the File pull-down is a 19-wide box at (0, 1), items `New` on row 2
    // and `Open...` on row 3.

    fn click(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    #[test]
    fn clicking_a_title_opens_that_menu() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = bar.handle_event(&click(7, 0), &mut ctx); // the Edit title
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(bar.path, vec![1, 0]);
    }

    #[test]
    fn clicking_the_open_title_again_closes_it() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File
        assert_eq!(bar.path, vec![0, 0]);
        bar.handle_event(&click(1, 0), &mut ctx); // click it again
        assert!(!bar.is_open());
    }

    #[test]
    fn clicking_another_title_switches_menus() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // File
        bar.handle_event(&click(7, 0), &mut ctx); // Edit
        assert_eq!(bar.path, vec![1, 0]);
    }

    #[test]
    fn clicking_an_item_posts_its_command_and_closes() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File
        let r = bar.handle_event(&click(3, 3), &mut ctx); // the Open... row
        assert_eq!(r, EventResult::Consumed);
        assert!(!bar.is_open());
        assert_eq!(ctx.posted(), &[Event::Command(CM_OPEN)]);
    }

    #[test]
    fn clicking_off_an_open_pulldown_dismisses_it() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File
        let r = bar.handle_event(&click(30, 5), &mut ctx); // empty desktop below
        assert_eq!(r, EventResult::Consumed);
        assert!(!bar.is_open());
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn clicking_a_disabled_item_posts_nothing_but_closes() {
        let mut bar = bar();
        let mut cs = CommandSet::new();
        cs.disable(CM_NEW);
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File
        bar.handle_event(&click(3, 2), &mut ctx); // the New row (disabled)
        assert!(!bar.is_open());
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn clicking_a_bare_part_of_a_closed_bar_is_ignored() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = bar.handle_event(&click(20, 0), &mut ctx); // past the last title
        assert_eq!(r, EventResult::Ignored);
        assert!(!bar.is_open());
    }

    #[test]
    fn moving_over_an_item_tracks_the_highlight() {
        let mut bar = bar();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File, highlight = 0
        let moved = Event::Mouse(MouseEvent {
            kind: MouseKind::Moved,
            pos: Point::new(3, 3), // hover the Open... row
            modifiers: Modifiers::NONE,
        });
        bar.handle_event(&moved, &mut ctx);
        assert_eq!(bar.path, vec![0, 1]);
    }

    // --- Cascading submenus (ADR 0018) ---
    //
    // Layout of `bar_with_submenu()`: one title "File" (cols 0..=5); its
    // pull-down lists "New" (row 2) then "Export" (row 3, a submenu). The File
    // box is 13 wide (1 + "Export".len()=6 + gap 3 + 1 + 2 borders) at (0, 1),
    // so its right edge is at column 13; the Export submenu opens beside its
    // own row (row 3) starting at column 13.

    #[test]
    fn right_on_a_submenu_item_opens_it() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx); // Export
        let r = bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(bar.path, vec![0, 1, 0], "opened Export, highlighting PDF");
    }

    #[test]
    fn enter_on_a_submenu_item_opens_it_rather_than_posting() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx); // Export
        bar.handle_event(&key(KeyCode::Enter, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![0, 1, 0]);
        assert!(
            ctx.posted().is_empty(),
            "a submenu item has no command to post"
        );
        assert!(bar.is_open(), "opening a submenu never closes the bar");
    }

    #[test]
    fn nests_two_levels_deep() {
        // A submenu whose own item is itself a submenu: path grows to depth 3,
        // proving the recursion isn't hard-coded to one level (ADR 0018).
        let mut bar = MenuBar::new(
            rect(0, 0, 40, 1),
            vec![Menu::new(
                "File",
                vec![MenuItem::submenu(
                    "Export",
                    Menu::new(
                        "Export",
                        vec![MenuItem::submenu(
                            "Image",
                            Menu::new("Image", vec![MenuItem::new("PNG", CM_PNG)]),
                        )],
                    ),
                )],
            )],
            &Theme::default(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx); // -> Export
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx); // -> Image
        assert_eq!(bar.path, vec![0, 0, 0, 0], "three levels deep");
        let r = bar.handle_event(&key(KeyCode::Enter, Modifiers::NONE), &mut ctx); // PNG
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_PNG)]);
        assert!(
            !bar.is_open(),
            "choosing the leaf command closes every level"
        );
    }

    #[test]
    fn left_pops_one_level_without_closing_the_bar() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx); // Export
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx); // open it
        assert_eq!(bar.path, vec![0, 1, 0]);
        let r = bar.handle_event(&key(KeyCode::Left, Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(
            bar.path,
            vec![0, 1],
            "back to File with Export still highlighted"
        );
        assert!(bar.is_open(), "Left at depth > 0 doesn't close the bar");
    }

    #[test]
    fn esc_pops_one_level_at_depth_then_closes_at_the_root() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx); // Export open
        bar.handle_event(&key(KeyCode::Esc, Modifiers::NONE), &mut ctx);
        assert_eq!(bar.path, vec![0, 1], "one Esc pops the submenu");
        bar.handle_event(&key(KeyCode::Esc, Modifiers::NONE), &mut ctx);
        assert!(
            !bar.is_open(),
            "a second Esc, now at the root, closes the bar"
        );
    }

    #[test]
    fn choosing_a_sibling_while_a_submenu_is_open_replaces_it() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx); // Export
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx); // Export open
        assert_eq!(bar.path, vec![0, 1, 0]);
        // Hover back onto New (the ancestor level's other item) truncates the
        // open Export submenu.
        let moved = Event::Mouse(MouseEvent {
            kind: MouseKind::Moved,
            pos: Point::new(3, 2), // New's row
            modifiers: Modifiers::NONE,
        });
        bar.handle_event(&moved, &mut ctx);
        assert_eq!(
            bar.path,
            vec![0, 0],
            "highlight moved to New, and Export's submenu is gone"
        );
    }

    #[test]
    fn hovering_a_submenu_item_only_moves_the_highlight_never_opens_it() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File, highlight New
        let moved = Event::Mouse(MouseEvent {
            kind: MouseKind::Moved,
            pos: Point::new(3, 3), // Export's row
            modifiers: Modifiers::NONE,
        });
        bar.handle_event(&moved, &mut ctx);
        assert_eq!(
            bar.path,
            vec![0, 1],
            "hover only highlights Export, it does not open its submenu"
        );
    }

    #[test]
    fn clicking_a_submenu_item_opens_it() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File
        let r = bar.handle_event(&click(3, 3), &mut ctx); // Export's row
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(bar.path, vec![0, 1, 0]);
        assert!(bar.is_open());
    }

    #[test]
    fn hovering_back_onto_the_row_that_opened_a_submenu_does_not_close_it() {
        // Regression: the cursor rests on Export's own row in the parent box
        // right after the click that opened its submenu (the child box opens
        // *beside* that row, not over it), so the very next hover naturally
        // re-resolves to that same parent item. That must not collapse the
        // submenu it just opened.
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File
        bar.handle_event(&click(3, 3), &mut ctx); // click Export -> opens its submenu
        assert_eq!(bar.path, vec![0, 1, 0]);
        let moved = Event::Mouse(MouseEvent {
            kind: MouseKind::Moved,
            pos: Point::new(3, 3), // still resting on Export's row
            modifiers: Modifiers::NONE,
        });
        let r = bar.handle_event(&moved, &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(
            bar.path,
            vec![0, 1, 0],
            "hovering the already-open parent row leaves the submenu open"
        );
    }

    #[test]
    fn clicking_a_leaf_item_in_an_open_submenu_posts_and_closes_everything() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&click(1, 0), &mut ctx); // open File
        bar.handle_event(&click(3, 3), &mut ctx); // open Export
        // File's box is 18 wide (its "Ctrl-N" shortcut sets the trailer
        // width), at x=0, so Export's box starts at x=18; PDF is its first
        // item row (Export's top border sits on row 3, aligned with its own
        // row in File's box, so PDF is one row below at y=4).
        let r = bar.handle_event(&click(20, 4), &mut ctx); // PDF
        assert_eq!(r, EventResult::Consumed);
        assert!(!bar.is_open());
        assert_eq!(ctx.posted(), &[Event::Command(CM_PDF)]);
    }

    #[test]
    fn a_disabled_submenu_item_cannot_be_opened() {
        let mut bar = MenuBar::new(
            rect(0, 0, 40, 1),
            vec![Menu::new(
                "File",
                vec![
                    MenuItem::submenu(
                        "Export",
                        Menu::new("Export", vec![MenuItem::new("PDF", CM_PDF)]),
                    )
                    .with_gate(CM_EXPORT_GATE),
                ],
            )],
            &Theme::default(),
        );
        let mut cs = CommandSet::new();
        cs.disable(CM_EXPORT_GATE);
        bar.sync_enabled(&cs);
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx); // File, Export highlighted
        let r = bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(
            bar.path,
            vec![0, 0],
            "Right does nothing: the branch is disabled"
        );
        assert!(bar.is_open());

        let r = bar.handle_event(&key(KeyCode::Enter, Modifiers::NONE), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(
            !bar.is_open(),
            "choosing a disabled submenu item closes the bar, like a disabled Command item"
        );
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn a_disabled_submenu_item_draws_greyed_with_its_cascade_mark() {
        let mut bar = MenuBar::new(
            rect(0, 0, 40, 1),
            vec![Menu::new(
                "File",
                vec![
                    MenuItem::submenu(
                        "Export",
                        Menu::new("Export", vec![MenuItem::new("PDF", CM_PDF)]),
                    )
                    .with_gate(CM_EXPORT_GATE),
                ],
            )],
            &Theme::default(),
        );
        let mut cs = CommandSet::new();
        cs.disable(CM_EXPORT_GATE);
        bar.sync_enabled(&cs);
        bar.handle_event(
            &key(KeyCode::F(10), Modifiers::NONE),
            &mut Context::new(&cs),
        );

        let mut buf = Buffer::new(Size::new(40, 6));
        let mut root = Canvas::new(&mut buf);
        bar.draw_overlay(&mut root);

        let theme = Theme::default();
        assert_eq!(
            buf.get(Point::new(2, 2)).unwrap().style(),
            theme.style(Role::MenuDisabled),
            "the disabled Export row is greyed"
        );
    }

    // --- Cascade rendering (ADR 0018) ---

    #[test]
    fn snapshot_cascaded_submenu() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx); // Export
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx); // open it
        insta::assert_snapshot!(render(&bar, 40, 8));
    }

    #[test]
    fn a_submenu_box_flips_to_the_parents_left_edge_near_the_right_of_the_screen() {
        // A leading dummy title pushes File's own pull-down away from column
        // 0 (so its un-clamped left, 12, is unambiguous), then a screen just
        // wide enough for File's box (left 12, width 18 -> right edge 30) but
        // too narrow for Export's box (width 7) beside it: 30 + 7 = 37 > 32.
        let mut bar = MenuBar::new(
            rect(0, 0, 60, 1),
            vec![
                Menu::new("Aaaaaaaaaa", vec![]),
                Menu::new(
                    "File",
                    vec![
                        MenuItem::new("New", CM_NEW).with_shortcut("Ctrl-N"),
                        MenuItem::submenu(
                            "Export",
                            Menu::new(
                                "Export",
                                vec![MenuItem::new("PDF", CM_PDF), MenuItem::new("PNG", CM_PNG)],
                            ),
                        ),
                    ],
                ),
            ],
            &Theme::default(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.open_menu(1); // File
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx); // Export
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx); // open it
        let areas = bar.level_areas(32);
        assert_eq!(
            areas[0].origin().x,
            12,
            "File sits under its title, unclamped"
        );
        assert_eq!(
            areas[1].origin().x,
            5,
            "Export flips to File's left edge (12 - 7 = 5) instead of overflowing"
        );
        assert_eq!(
            areas[1].origin().x + areas[1].width(),
            areas[0].origin().x,
            "Export's right edge touches File's left edge exactly"
        );
    }

    #[test]
    fn a_submenu_box_opens_to_the_right_when_there_is_room() {
        let mut bar = bar_with_submenu();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        bar.handle_event(&key(KeyCode::F(10), Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Down, Modifiers::NONE), &mut ctx);
        bar.handle_event(&key(KeyCode::Right, Modifiers::NONE), &mut ctx);
        let areas = bar.level_areas(80);
        assert_eq!(
            areas[1].origin().x,
            areas[0].origin().x + areas[0].width(),
            "Export opens flush to the right of File"
        );
        assert_eq!(
            areas[1].origin().y,
            areas[0].origin().y + 1 + 1, // File's box top + border + Export's row (1)
            "Export is top-aligned with the row it opened from"
        );
    }
}
