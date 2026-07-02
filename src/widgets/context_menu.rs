//! A pointer-anchored pull-down, triggered by a right-click rather than a
//! menu bar title (ADR 0019).
//!
//! `ContextMenu` shares `Menu`/`MenuItem` and the cascading-submenu rules
//! (ADR 0018) with [`MenuBar`](super::MenuBar) — the shared geometry,
//! hit-testing, and drawing live as free functions in `menu.rs` — but it is
//! not a generalisation of `MenuBar`: no bar row, no top-level sibling
//! cycling, no `Alt`-hot-key open trigger. Its level 0 anchors at an
//! arbitrary screen point rather than a bar title's column, so `path[0]`
//! here means "the highlighted item in the one root `Menu`", not
//! `MenuBar::path[0]`'s "which sibling bar menu is open."
//!
//! `Shell` is the only thing that constructs one (from a request drained via
//! [`Context::take_context_menu_request`](crate::view::Context::take_context_menu_request))
//! and gives it first refusal on every event while it is open, exactly
//! mirroring how it already treats `MenuBar`'s own open pull-down.

use crate::canvas::Canvas;
use crate::command::CommandSet;
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::Theme;
use crate::view::Context;

use super::menu::{Menu, MenuStyles, cascade_area, draw_cascade, hit_test, pulldown_width};

/// A context menu, open at whatever cascade depth its `path` currently
/// reaches.
pub struct ContextMenu {
    anchor: Point,
    menu: Menu,
    /// `path[0]` is the highlighted item in the root `Menu`; `path[i]` for
    /// `i > 0` is the highlighted item within the submenu opened by
    /// `path[i - 1]` — the same doubled meaning `MenuBar::path[1..]` carries,
    /// just with no bar-index entry in front (ADR 0018, ADR 0019). Never
    /// empty for the lifetime of a `ContextMenu`.
    path: Vec<usize>,
    styles: MenuStyles,
    commands: CommandSet,
    /// Set once `Esc`/`Left` at the root, a click outside every open box, or
    /// choosing a leaf item closes this menu entirely. `Shell` checks this
    /// after every dispatch and drops its `Option<ContextMenu>` once set,
    /// rather than this type trying to signal its own removal.
    closed: bool,
}

impl ContextMenu {
    /// Opens `menu` anchored at `at` (already-resolved screen coordinates,
    /// ADR 0019), taking its styles from `theme` and a snapshot of `commands`
    /// for item gating — frozen at construction time, mirroring how
    /// `MenuBar::sync_enabled` pushes one in for a statically-held widget.
    pub(crate) fn new(menu: Menu, at: Point, theme: &Theme, commands: &CommandSet) -> Self {
        Self {
            anchor: at,
            menu,
            path: vec![0],
            styles: MenuStyles::from_theme(theme),
            commands: commands.clone(),
            closed: false,
        }
    }

    /// Whether this menu should be discarded — checked by `Shell` after every
    /// dispatch.
    pub(crate) fn is_closed(&self) -> bool {
        self.closed
    }

    /// The `Menu` at each currently open level, root first, deepest (focused)
    /// last — mirrors [`MenuBar::open_menus`](super::MenuBar), minus the
    /// bar-index indirection at the root.
    fn open_menus(&self) -> Vec<&Menu> {
        let mut menus = vec![&self.menu];
        for &idx in &self.path[..self.path.len() - 1] {
            let parent = *menus.last().expect("just pushed the root menu");
            let next = parent.items()[idx]
                .submenu_ref()
                .expect("path invariant: an ancestor highlight always names a Submenu item");
            menus.push(next);
        }
        menus
    }

    /// Level 0's box: top-left at the anchor point, clamped so it never runs
    /// off the right or bottom of `screen` (ADR 0019) — the two-axis
    /// generalisation of `MenuBar::pulldown_area`'s single-axis clamp, since a
    /// context menu can open anywhere on screen, not just under a fixed top
    /// row.
    fn root_area(&self, menu: &Menu, screen: Size) -> Rect {
        let box_w = pulldown_width(menu);
        let box_h = menu.items().len() as i16 + 2;
        let x = self.anchor.x.min((screen.width - box_w).max(0)).max(0);
        let y = self.anchor.y.min((screen.height - box_h).max(0)).max(0);
        Rect::from_origin_size(Point::new(x, y), Size::new(box_w, box_h))
    }

    /// The box for each currently open level, root first — mirrors
    /// [`MenuBar::level_areas`](super::MenuBar), deferring to the shared
    /// [`cascade_area`] for every level past the root.
    ///
    /// Note the index shift from `MenuBar::level_areas`: a cascaded level
    /// anchors beside its *parent's* highlighted row, which is `path[level -
    /// 1]` here (there is no bar-index entry in front to absorb the
    /// difference, unlike `MenuBar`, where `path[level]` already *is* the
    /// parent's highlight thanks to that extra entry).
    fn level_areas(&self, screen: Size) -> Vec<Rect> {
        let menus = self.open_menus();
        let mut areas: Vec<Rect> = Vec::with_capacity(menus.len());
        for (level, menu) in menus.iter().enumerate() {
            let area = if level == 0 {
                self.root_area(menu, screen)
            } else {
                cascade_area(areas[level - 1], self.path[level - 1], menu, screen.width)
            };
            areas.push(area);
        }
        areas
    }

    /// The `(level, item index)` under `pos`, deepest level first.
    fn level_item_at(&self, pos: Point, screen: Size) -> Option<(usize, usize)> {
        let menus = self.open_menus();
        let areas = self.level_areas(screen);
        hit_test(pos, &menus, &areas)
    }

    /// Whether item `idx` at `level` is already expanded (ADR 0018). Note the
    /// index shift from [`MenuBar::already_expanded`](super::MenuBar): there
    /// is no bar-index entry in front here, so `path[level]` (not
    /// `path[level + 1]`) is the array slot this `level` lives at.
    fn already_expanded(&self, level: usize, idx: usize) -> bool {
        self.path.get(level) == Some(&idx) && self.path.len() > level + 1
    }

    /// Moves the highlight to item `idx` at `level`, truncating any deeper
    /// cascade first — mirrors
    /// [`MenuBar::hover`](super::MenuBar).
    fn hover(&mut self, level: usize, idx: usize) {
        if self.already_expanded(level, idx) {
            return;
        }
        self.path.truncate(level);
        self.path.push(idx);
    }

    /// Acts on item `idx` at `level` — mirrors
    /// [`MenuBar::choose`](super::MenuBar) exactly, except closing here means
    /// setting `closed`, not clearing a `path` back to empty (`ContextMenu`
    /// has no "closed but still exists" state).
    fn choose(&mut self, level: usize, idx: usize, ctx: &mut Context) {
        let (is_submenu, command, enabled) = {
            let menus = self.open_menus();
            let item = &menus[level].items()[idx];
            (
                item.is_submenu(),
                item.command(),
                item.enabled(&self.commands),
            )
        };
        if is_submenu {
            if !enabled {
                self.closed = true;
            } else if !self.already_expanded(level, idx) {
                self.path.truncate(level);
                self.path.push(idx);
                self.path.push(0);
            }
        } else {
            let command = command.expect("a non-submenu MenuItem always holds a Command");
            self.closed = true;
            ctx.post(command); // Gated by Context: a disabled item posts nothing (ADR 0003).
        }
    }

    /// `Up`/`Down`: moves the highlight within the focused (deepest) level,
    /// wrapping.
    fn move_highlight(&mut self, direction: i16) {
        let items = self.open_menus().last().expect("never empty").items().len();
        if items == 0 {
            return;
        }
        let last = self.path.last_mut().expect("never empty");
        *last = (*last as i16 + direction).rem_euclid(items as i16) as usize;
    }

    /// `Right`: opens the focused item's submenu if it has one and is
    /// enabled. Unlike `MenuBar`, there is no sibling to cycle to at the root
    /// — a `Right` on a plain `Command` item, or at the root with no
    /// submenu, is simply a no-op.
    fn handle_right(&mut self) {
        let submenu_enabled = {
            let menus = self.open_menus();
            let focused = menus.last().expect("never empty");
            focused
                .items()
                .get(*self.path.last().expect("never empty"))
                .is_some_and(|item| item.is_submenu() && item.enabled(&self.commands))
        };
        if submenu_enabled {
            self.path.push(0);
        }
    }

    /// `Enter`: chooses the focused level's highlighted item, if any.
    fn choose_highlighted(&mut self, ctx: &mut Context) {
        let level = self.path.len() - 1;
        let idx = *self.path.last().expect("never empty");
        let has_items = !self.open_menus()[level].items().is_empty();
        if has_items {
            self.choose(level, idx, ctx);
        }
    }

    /// A hot-key letter: chooses the focused level's item whose hot-key
    /// matches `c`, if any.
    fn choose_hotkey(&mut self, c: char, ctx: &mut Context) {
        let c = c.to_ascii_lowercase();
        let found = {
            let level = self.path.len() - 1;
            let menus = self.open_menus();
            menus[level]
                .items()
                .iter()
                .position(|item| item.hotkey() == Some(c))
                .map(|idx| (level, idx))
        };
        if let Some((level, idx)) = found {
            self.choose(level, idx, ctx);
        }
    }

    /// Keyboard handling while open: arrows navigate and cascade, `Enter`
    /// chooses, a hot-key letter jumps straight to and chooses its item,
    /// `Esc`/`Left` pop one level (closing entirely at the root — there is no
    /// bar to fall back to). Every other key is swallowed, matching
    /// `MenuBar`'s modality while a pull-down is open.
    fn handle_key(&mut self, code: KeyCode, ctx: &mut Context) -> EventResult {
        match code {
            KeyCode::Esc | KeyCode::Left => {
                if self.path.len() > 1 {
                    self.path.pop();
                } else {
                    self.closed = true;
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

    /// Mouse handling while open, in `screen`-sized screen coordinates
    /// (`Shell` never region-translates while a context menu is open, same
    /// exception `MenuBar`'s pull-down already gets). A click on an item
    /// chooses it; a click outside every open box, or anywhere not otherwise
    /// recognised, closes this menu; hover only ever moves the highlight
    /// (ADR 0018).
    fn handle_mouse(&mut self, mouse: &MouseEvent, screen: Size, ctx: &mut Context) -> EventResult {
        match mouse.kind {
            MouseKind::Down(MouseButton::Left) => {
                if let Some((level, idx)) = self.level_item_at(mouse.pos, screen) {
                    self.choose(level, idx, ctx);
                } else {
                    self.closed = true;
                }
                EventResult::Consumed
            }
            MouseKind::Moved | MouseKind::Drag(MouseButton::Left) => {
                if let Some((level, idx)) = self.level_item_at(mouse.pos, screen) {
                    self.hover(level, idx);
                }
                EventResult::Consumed
            }
            _ => EventResult::Consumed,
        }
    }

    /// Routes one event while this menu is open. `screen` is the terminal's
    /// current size, needed for anchor clamping and hit-testing (the same
    /// screen-coordinate space `MenuBar`'s own overlay works in).
    pub(crate) fn handle_event(
        &mut self,
        event: &Event,
        screen: Size,
        ctx: &mut Context,
    ) -> EventResult {
        match event {
            Event::Key(key) => self.handle_key(key.code, ctx),
            Event::Mouse(mouse) => self.handle_mouse(mouse, screen, ctx),
            _ => EventResult::Ignored,
        }
    }

    /// Draws every open level over the whole frame, cascaded left to right —
    /// mirrors [`MenuBar::draw_overlay`](super::MenuBar).
    pub(crate) fn draw_overlay(&self, canvas: &mut Canvas) {
        let menus = self.open_menus();
        let areas = self.level_areas(canvas.size());
        draw_cascade(
            canvas,
            &menus,
            &areas,
            &self.path,
            &self.commands,
            self.styles,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::canvas::Canvas;
    use crate::command::{CM_USER, Command, CommandSet};
    use crate::event::{KeyEvent, Modifiers};
    use crate::theme::{Role, Theme};
    use crate::widgets::MenuItem;

    const CM_PDF: Command = Command(CM_USER + 1);
    const CM_PNG: Command = Command(CM_USER + 2);
    const CM_COPY: Command = Command(CM_USER + 3);
    const CM_EXPORT_GATE: Command = Command(CM_USER + 4);

    const SCREEN: Size = Size::new(40, 20);

    fn menu() -> Menu {
        Menu::new(
            "Ctx",
            vec![
                MenuItem::new("Copy", CM_COPY),
                MenuItem::submenu(
                    "Export",
                    Menu::new(
                        "Export",
                        vec![MenuItem::new("PDF", CM_PDF), MenuItem::new("PNG", CM_PNG)],
                    ),
                ),
            ],
        )
    }

    fn ctx_menu_at(x: i16, y: i16) -> ContextMenu {
        ContextMenu::new(
            menu(),
            Point::new(x, y),
            &Theme::default(),
            &CommandSet::new(),
        )
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, Modifiers::NONE))
    }

    fn click(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    fn dispatch(menu: &mut ContextMenu, event: &Event, ctx: &mut Context) -> EventResult {
        menu.handle_event(event, SCREEN, ctx)
    }

    // --- Opening state / geometry ---

    #[test]
    fn a_new_context_menu_starts_highlighting_the_first_item() {
        let menu = ctx_menu_at(5, 5);
        assert_eq!(menu.path, vec![0]);
        assert!(!menu.is_closed());
    }

    #[test]
    fn root_area_opens_at_the_anchor_when_there_is_room() {
        let m = ctx_menu_at(5, 5);
        let areas = m.level_areas(SCREEN);
        assert_eq!(areas[0].origin(), Point::new(5, 5));
    }

    #[test]
    fn root_area_clamps_to_fit_the_right_and_bottom_of_the_screen() {
        // Anchored past where a wide/tall box would fit; it slides back to
        // stay fully on screen rather than running off the edge.
        let m = ctx_menu_at(39, 19);
        let areas = m.level_areas(SCREEN);
        let area = areas[0];
        assert!(area.origin().x + area.width() <= SCREEN.width);
        assert!(area.origin().y + area.height() <= SCREEN.height);
    }

    // --- Navigation ---

    #[test]
    fn up_down_move_the_highlight_and_wrap() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx);
        assert_eq!(m.path, vec![1]);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx);
        assert_eq!(m.path, vec![0], "wraps");
        dispatch(&mut m, &key(KeyCode::Up), &mut ctx);
        assert_eq!(m.path, vec![1], "wraps the other way");
    }

    #[test]
    fn right_on_a_submenu_item_opens_it() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx); // highlight Export
        let r = dispatch(&mut m, &key(KeyCode::Right), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(m.path, vec![1, 0], "opened Export, highlighting PDF");
    }

    #[test]
    fn right_on_a_plain_command_item_is_a_no_op() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // Highlighted item 0 ("Copy") has no submenu and no sibling to cycle
        // to (unlike MenuBar's Right, which would cycle top-level menus).
        dispatch(&mut m, &key(KeyCode::Right), &mut ctx);
        assert_eq!(m.path, vec![0]);
    }

    #[test]
    fn nests_two_levels_deep() {
        let mut m = ContextMenu::new(
            Menu::new(
                "Root",
                vec![MenuItem::submenu(
                    "A",
                    Menu::new(
                        "A",
                        vec![MenuItem::submenu(
                            "B",
                            Menu::new("B", vec![MenuItem::new("Leaf", CM_COPY)]),
                        )],
                    ),
                )],
            ),
            Point::new(0, 0),
            &Theme::default(),
            &CommandSet::new(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Right), &mut ctx);
        dispatch(&mut m, &key(KeyCode::Right), &mut ctx);
        assert_eq!(m.path, vec![0, 0, 0]);
    }

    #[test]
    fn a_cascaded_levels_box_anchors_on_the_parent_row_that_opened_it_not_its_own_highlight() {
        // Regression: a cascaded level must stay anchored beside the row
        // that opened it. `path[level]` (MenuBar's own formula) is off by
        // one here — ContextMenu has no bar-index entry in front to absorb
        // it — so the submenu's box was tracking its *own* highlight instead
        // and visibly jumped a row every time the highlight moved inside it.
        let mut m = ctx_menu_at(0, 0); // "Copy" (row 0), "Export" (row 1, submenu)
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx); // highlight Export
        dispatch(&mut m, &key(KeyCode::Right), &mut ctx); // open it: path = [1, 0]
        let opened_at = m.level_areas(SCREEN)[1];

        dispatch(&mut m, &key(KeyCode::Down), &mut ctx); // highlight PNG inside it: path = [1, 1]
        let after_moving = m.level_areas(SCREEN)[1];

        assert_eq!(
            opened_at, after_moving,
            "moving the highlight inside the open submenu must not move its box"
        );
        // And it must anchor beside Export's own row (index 1), not PDF's
        // default first-highlight row (index 0) at the moment it opened.
        let root_area = m.level_areas(SCREEN)[0];
        assert_eq!(opened_at.origin().y, root_area.origin().y + 1 + 1);
    }

    #[test]
    fn esc_pops_one_level_then_closes_at_the_root() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx); // Export
        dispatch(&mut m, &key(KeyCode::Right), &mut ctx); // open it
        assert_eq!(m.path, vec![1, 0]);
        dispatch(&mut m, &key(KeyCode::Esc), &mut ctx);
        assert_eq!(m.path, vec![1], "popped one level, still open");
        assert!(!m.is_closed());
        dispatch(&mut m, &key(KeyCode::Esc), &mut ctx);
        assert!(m.is_closed(), "closes entirely at the root");
    }

    #[test]
    fn left_behaves_the_same_as_esc() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Left), &mut ctx);
        assert!(m.is_closed(), "no sibling to cycle to, so Left just closes");
    }

    // --- Choosing ---

    #[test]
    fn enter_on_a_leaf_posts_and_closes() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = dispatch(&mut m, &key(KeyCode::Enter), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(m.is_closed());
        assert_eq!(ctx.posted(), &[Event::Command(CM_COPY)]);
    }

    #[test]
    fn enter_on_a_submenu_item_opens_it_rather_than_posting() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx); // Export
        dispatch(&mut m, &key(KeyCode::Enter), &mut ctx);
        assert_eq!(m.path, vec![1, 0]);
        assert!(!m.is_closed());
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn hotkey_letter_chooses_the_item() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = dispatch(&mut m, &key(KeyCode::Char('c')), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(m.is_closed());
        assert_eq!(ctx.posted(), &[Event::Command(CM_COPY)]);
    }

    #[test]
    fn a_disabled_submenu_item_cannot_be_opened_and_closes_if_chosen() {
        let export = Menu::new(
            "Export",
            vec![MenuItem::new("PDF", CM_PDF), MenuItem::new("PNG", CM_PNG)],
        );
        let root = Menu::new(
            "Ctx",
            vec![
                MenuItem::new("Copy", CM_COPY),
                MenuItem::submenu("Export", export).with_gate(CM_EXPORT_GATE),
            ],
        );
        let mut cs = CommandSet::new();
        cs.disable(CM_EXPORT_GATE);
        let mut m = ContextMenu::new(root, Point::new(0, 0), &Theme::default(), &cs);
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx); // Export
        dispatch(&mut m, &key(KeyCode::Right), &mut ctx);
        assert_eq!(m.path, vec![1], "disabled: Right does not open it");
        dispatch(&mut m, &key(KeyCode::Enter), &mut ctx);
        assert!(m.is_closed(), "choosing it anyway closes without opening");
        assert!(ctx.posted().is_empty());
    }

    // --- Mouse ---
    //
    // Layout: root box at (0, 0), width = pulldown_width("Ctx" menu). Items:
    // "Copy" on row 1, "Export" on row 2 (both offset by the top border).

    #[test]
    fn clicking_a_leaf_item_posts_and_closes() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = dispatch(&mut m, &click(2, 1), &mut ctx); // "Copy" row
        assert_eq!(r, EventResult::Consumed);
        assert!(m.is_closed());
        assert_eq!(ctx.posted(), &[Event::Command(CM_COPY)]);
    }

    #[test]
    fn clicking_a_submenu_item_opens_it() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &click(2, 2), &mut ctx); // "Export" row
        assert_eq!(m.path, vec![1, 0]);
        assert!(!m.is_closed());
    }

    #[test]
    fn clicking_off_every_open_box_closes_it() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = dispatch(&mut m, &click(30, 15), &mut ctx);
        assert_eq!(r, EventResult::Consumed);
        assert!(m.is_closed());
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn hovering_an_item_only_moves_the_highlight_never_opens_it() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let moved = Event::Mouse(MouseEvent {
            kind: MouseKind::Moved,
            pos: Point::new(2, 2), // "Export" row
            modifiers: Modifiers::NONE,
        });
        dispatch(&mut m, &moved, &mut ctx);
        assert_eq!(m.path, vec![1], "highlight moved");
        assert!(!m.is_closed(), "but nothing opened");
    }

    #[test]
    fn hovering_back_onto_the_row_that_opened_a_submenu_does_not_close_it() {
        let mut m = ctx_menu_at(0, 0);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &click(2, 2), &mut ctx); // opens Export
        assert_eq!(m.path, vec![1, 0]);
        let moved = Event::Mouse(MouseEvent {
            kind: MouseKind::Moved,
            pos: Point::new(2, 2), // still resting on Export's own row
            modifiers: Modifiers::NONE,
        });
        dispatch(&mut m, &moved, &mut ctx);
        assert_eq!(
            m.path,
            vec![1, 0],
            "hovering the already-open parent row leaves the submenu open"
        );
    }

    // --- Rendering ---

    fn render(menu: &ContextMenu, size: Size) -> String {
        let mut buf = Buffer::new(size);
        let mut canvas = Canvas::new(&mut buf);
        menu.draw_overlay(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn snapshot_single_level_context_menu() {
        let m = ctx_menu_at(5, 3);
        insta::assert_snapshot!(render(&m, SCREEN));
    }

    #[test]
    fn snapshot_cascaded_context_menu() {
        let mut m = ctx_menu_at(2, 2);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        dispatch(&mut m, &key(KeyCode::Down), &mut ctx); // Export
        dispatch(&mut m, &key(KeyCode::Right), &mut ctx); // open it
        insta::assert_snapshot!(render(&m, SCREEN));
    }

    #[test]
    fn snapshot_flips_to_fit_near_the_bottom_right_of_the_screen() {
        let small = Size::new(20, 10);
        let m = ctx_menu_at(19, 9);
        insta::assert_snapshot!(render(&m, small));
    }

    #[test]
    fn a_disabled_item_draws_greyed() {
        let root = Menu::new(
            "Ctx",
            vec![
                MenuItem::new("Copy", CM_COPY),
                MenuItem::new("Paste", CM_PDF),
            ],
        );
        let mut cs = CommandSet::new();
        cs.disable(CM_COPY);
        let m = ContextMenu::new(root, Point::new(0, 0), &Theme::default(), &cs);

        let mut buf = Buffer::new(SCREEN);
        let mut canvas = Canvas::new(&mut buf);
        m.draw_overlay(&mut canvas);

        let theme = Theme::default();
        assert_eq!(
            buf.get(Point::new(2, 1)).unwrap().style(),
            theme.style(Role::MenuDisabled)
        );
    }
}
