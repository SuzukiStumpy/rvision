//! The retained-mode view tree: the [`View`] trait, the owning [`Group`], the
//! handler [`Context`], and the [`StaticText`] leaf (ADR 0003, 0004).
//!
//! A [`View`] knows its owner-relative [`bounds`](View::bounds), draws itself
//! through a [`Canvas`] in local coordinates (ADR 0008), and handles events. A
//! [`Group`] owns its children (`Vec<Box<dyn View>>`, parent-owns-child), draws
//! them in z-order, and runs the three-phase dispatch — *positional* (mouse →
//! the view under the cursor), *focused* (keys/commands → the focus chain), and
//! *broadcast* (everyone) — plus Tab/Shift-Tab focus traversal.
//!
//! Views never reference one another. A view emits a command by posting it to the
//! [`Context`]; commands then bubble **up** the owner chain as the recursive
//! dispatch unwinds, and broadcasts travel **down** (ADR 0003).

use crate::canvas::Canvas;
use crate::color::Style;
use crate::command::{Command, CommandSet};
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect};
use crate::widgets::Menu;

/// One node of the UI tree.
///
/// The default [`handle_event`](View::handle_event) ignores everything and the
/// default [`focusable`](View::focusable) is `false`, so a passive leaf (like
/// [`StaticText`]) only implements [`bounds`](View::bounds) and
/// [`draw`](View::draw).
pub trait View {
    /// The view's rectangle in its **owner's** coordinate space.
    fn bounds(&self) -> Rect;

    /// Draws the view through `canvas`, which is already offset and clipped to
    /// the view's bounds: draw at local `(0, 0)` (ADR 0008).
    fn draw(&self, canvas: &mut Canvas);

    /// The drop shadow this view casts onto its **owner's** surface, or `None`
    /// (the default) if it casts none (ADR 0011).
    ///
    /// A shadow falls *outside* the view's own bounds — down its right and
    /// bottom edges — so a view cannot paint it through its own clipped
    /// [`draw`](Self::draw) canvas. Instead the owner reads this and paints
    /// `canvas.shadow(child.bounds(), style)` on its own surface *before* drawing
    /// the child on top, so each floating view sits over its own shadow and a
    /// higher sibling's shadow falls on a lower one. A floating widget — a
    /// [`Window`](crate::widgets::Window), tree-resident or run modally
    /// (ADR 0016) — returns the [`Role::Shadow`](crate::theme::Role::Shadow)
    /// style it resolved at construction; flush views (controls, the desktop
    /// backdrop) keep `None`.
    fn drop_shadow(&self) -> Option<Style> {
        None
    }

    /// Whether this view wants z-order priority over its siblings right now
    /// — drawn after every non-requesting sibling (so it paints on top
    /// regardless of vector position) and hit-tested before them (ADR
    /// 0030). Queried every frame/event, like [`drop_shadow`](Self::drop_shadow).
    ///
    /// For a transient popup that can grow past its "natural" footprint —
    /// [`ComboBox`](crate::widgets::ComboBox)'s open drop-down is the
    /// motivating case — insertion order alone doesn't guarantee it wins
    /// against a later sibling (e.g. a dialog's OK/Cancel buttons) that
    /// happens to occupy the same screen area. The default `false` leaves
    /// ordinary insertion-order z-order exactly as it was.
    fn wants_topmost(&self) -> bool {
        false
    }

    /// Handles one event, returning whether it was consumed (ADR 0004). A view
    /// emits its own commands by posting them to `ctx`; it never references
    /// another view.
    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let _ = (event, ctx);
        EventResult::Ignored
    }

    /// Whether this view can hold the keyboard focus.
    fn focusable(&self) -> bool {
        false
    }

    /// Notifies the view that it has gained (`true`) or lost (`false`) the
    /// keyboard focus, so a focusable view can draw itself focused (ADR 0010).
    ///
    /// The owning [`Group`] pushes this as focus moves; the default ignores it,
    /// so a view that does not draw differently when focused need not implement
    /// it. A `Group` forwards a `set_focused` it receives to its own focused
    /// child, so the signal composes through nested groups.
    fn set_focused(&mut self, focused: bool) {
        let _ = focused;
    }

    /// What this view needs scrolled right now, or `None` if nothing does (ADR
    /// 0015). Queried every draw, like [`drop_shadow`](Self::drop_shadow): a
    /// composing owner (a [`Window`](crate::widgets::Window) first) reserves
    /// and draws a border [`ScrollBar`](crate::widgets::ScrollBar) per axis
    /// that needs one, and routes clicks/drags on it back through
    /// [`set_scroll`](Self::set_scroll). A view that manages its own scroll
    /// chrome (or doesn't scroll) keeps the default.
    fn scroll_metrics(&self) -> Option<ScrollMetrics> {
        None
    }

    /// Pushes a new scroll position an owner's chrome computed on this view's
    /// behalf (ADR 0015), mirroring [`set_focused`](Self::set_focused)'s push
    /// shape. The default ignores it.
    fn set_scroll(&mut self, offset: Point) {
        let _ = offset;
    }

    /// The status text this view wants shown in a hosting owner's status
    /// panel, or `None` if it has none to offer (ADR 0032). Queried every
    /// draw, like [`scroll_metrics`](Self::scroll_metrics). Pull-only:
    /// unlike scrolling, there's nothing for an owner to push back.
    fn status_text(&self) -> Option<String> {
        None
    }

    /// Whether it is currently OK to act on `command` (TurboVision's
    /// `TView::valid`) — e.g. close, quit, zoom (ADR 0016). Default: always
    /// OK. A view that needs to refuse (unsaved changes) can also post a
    /// follow-up command through `ctx` in the same call — e.g. to ask its
    /// owner to run a confirmation flow — and try again once that resolves.
    /// A view never gets to run its own modal loop directly (ADR 0003): only
    /// whoever owns a concrete `Application` can do that.
    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
        let _ = (command, ctx);
        true
    }

    /// Tells this view its area changed — a resize (drag or a
    /// [`Window`](crate::widgets::Window) propagating its own `set_bounds`),
    /// a zoom/restore, or any other repositioning an owner decides to push
    /// down (ADR 0017). The default is a no-op: only a view whose own layout
    /// is a cached function of its size (wrapped lines, a scrolled-into-view
    /// offset) needs to override it and recompute that cache.
    fn set_bounds(&mut self, bounds: Rect) {
        let _ = bounds;
    }
}

/// What a scrollable view needs scrolled, per axis (ADR 0015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScrollMetrics {
    /// The horizontal axis's range, or `None` if this view doesn't scroll
    /// sideways.
    pub horizontal: Option<AxisMetrics>,
    /// The vertical axis's range, or `None` if this view doesn't scroll up/down.
    pub vertical: Option<AxisMetrics>,
}

/// One axis's scroll range: `total` units, `visible` of them on screen at
/// once, the first shown being `pos` (ADR 0015).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AxisMetrics {
    /// The total number of scrollable units (rows, columns, ...).
    pub total: usize,
    /// How many are visible on screen at once.
    pub visible: usize,
    /// The index of the first unit currently shown.
    pub pos: usize,
}

/// A pending request to open a context menu, made via
/// [`Context::open_context_menu`] and drained via
/// [`Context::take_context_menu_request`] by whoever owns the overlay
/// (`Shell`, ADR 0019).
pub struct ContextMenuRequest {
    /// The menu to show.
    pub menu: Menu,
    /// Where to anchor it, in true screen coordinates — already resolved
    /// through however many nested [`Context::translated`] scopes stood
    /// between the requesting view and the screen.
    pub at: Point,
}

/// A handler's outbound channel: how a view posts commands and queries command
/// state without holding a reference to anyone (ADR 0003).
///
/// One `Context` is threaded through a whole event's dispatch. Posted events
/// accumulate; the application loop drains them with [`take_posted`](Self::take_posted)
/// and re-injects them from the root.
pub struct Context<'a> {
    posted: Vec<Event>,
    commands: &'a CommandSet,
    /// The local-to-screen offset accumulated so far by nested
    /// [`translated`](Self::translated) scopes (ADR 0019) — zero at the
    /// screen's own top level.
    offset: Point,
    context_menu_request: Option<ContextMenuRequest>,
    /// Set by [`request_mouse_capture`](Self::request_mouse_capture); drained
    /// by [`take_mouse_capture_request`](Self::take_mouse_capture_request).
    capture_requested: bool,
}

impl<'a> Context<'a> {
    /// Creates a context over the current command-enable state.
    pub fn new(commands: &'a CommandSet) -> Self {
        Self {
            posted: Vec::new(),
            commands,
            offset: Point::default(),
            context_menu_request: None,
            capture_requested: false,
        }
    }

    /// Posts a command to bubble up the owner chain — but only if it is enabled,
    /// so a disabled control's command never fires (ADR 0003).
    pub fn post(&mut self, command: Command) {
        if self.commands.is_enabled(command) {
            self.posted.push(Event::Command(command));
        }
    }

    /// Posts a broadcast to travel down to every view. Not gated by command
    /// state — a broadcast is a notification, not an action.
    pub fn broadcast(&mut self, command: Command) {
        self.posted.push(Event::Broadcast(command));
    }

    /// Whether `command` is currently enabled (e.g. so a control can grey itself).
    pub fn is_enabled(&self, command: Command) -> bool {
        self.commands.is_enabled(command)
    }

    /// The current command-enable state, e.g. to snapshot a gate at
    /// construction time (a [`ContextMenu`](crate::widgets::ContextMenu) built
    /// on demand does this, mirroring how a statically-held widget is instead
    /// pushed one via `sync_enabled`).
    pub fn commands(&self) -> &CommandSet {
        self.commands
    }

    /// The events posted so far during this dispatch.
    pub fn posted(&self) -> &[Event] {
        &self.posted
    }

    /// Takes the posted events, leaving the queue empty.
    pub fn take_posted(&mut self) -> Vec<Event> {
        std::mem::take(&mut self.posted)
    }

    /// Runs `f` with this context's accumulated offset increased by `(dx,
    /// dy)` for the duration of the call, then restores it — wraps one step
    /// of a translate-and-recurse positional dispatch (`Group`, `Desktop`,
    /// `Window`, `Shell`) so that an [`open_context_menu`](Self::open_context_menu)
    /// call made by a view nested arbitrarily deep resolves to true screen
    /// coordinates, without any intermediate container needing to know a
    /// context menu exists (ADR 0019).
    ///
    /// `(dx, dy)` is the amount that converts the child's local coordinates
    /// back to this scope's — i.e. the child's `bounds.origin()` itself, the
    /// *positive* of what a caller separately subtracts to compute the
    /// child-local `MouseEvent.pos` it dispatches.
    pub fn translated<T>(&mut self, dx: i16, dy: i16, f: impl FnOnce(&mut Self) -> T) -> T {
        let previous = self.offset;
        self.offset = self.offset.offset(dx, dy);
        let result = f(self);
        self.offset = previous;
        result
    }

    /// Requests a context menu showing `menu`'s items, anchored at `at` — in
    /// the caller's own local coordinates, resolved here to true screen
    /// coordinates using the offset accumulated by any enclosing
    /// [`translated`](Self::translated) scopes (ADR 0019). Replaces any
    /// request already pending from earlier in this same dispatch.
    pub fn open_context_menu(&mut self, menu: Menu, at: Point) {
        self.context_menu_request = Some(ContextMenuRequest {
            menu,
            at: at.offset(self.offset.x, self.offset.y),
        });
    }

    /// Takes the pending context-menu request, if any, leaving it clear.
    pub fn take_context_menu_request(&mut self) -> Option<ContextMenuRequest> {
        self.context_menu_request.take()
    }

    /// Asks whoever ultimately owns positional mouse dispatch (`Desktop`) to
    /// keep forwarding every subsequent mouse event straight to the window
    /// this call originated in, regardless of where the pointer moves, until
    /// the button is released — a scroll-bar thumb drag's use case, phrased
    /// generically (not `ScrollBar`-specific) so any future continuous-drag
    /// interaction can reuse it without `Desktop` needing to know it exists.
    /// Mirrors [`open_context_menu`](Self::open_context_menu)'s "child asks
    /// owner for something it can't do itself" idiom (ADR 0019); simpler,
    /// since no position needs resolving through [`translated`](Self::translated).
    pub fn request_mouse_capture(&mut self) {
        self.capture_requested = true;
    }

    /// Takes the pending capture request, if any, leaving it clear.
    pub fn take_mouse_capture_request(&mut self) -> bool {
        std::mem::take(&mut self.capture_requested)
    }
}

/// A non-interactive line of text. The simplest concrete [`View`]: it draws its
/// string at its top-left and ignores every event.
pub struct StaticText {
    bounds: Rect,
    text: String,
    style: Style,
}

impl StaticText {
    /// Creates a static-text view occupying `bounds`, rendering `text` in `style`.
    pub fn new(bounds: Rect, text: &str, style: Style) -> Self {
        Self {
            bounds,
            text: text.to_string(),
            style,
        }
    }
}

impl View for StaticText {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        canvas.put_str(Point::new(0, 0), &self.text, self.style);
    }
}

/// A view that owns child views and routes events to them (ADR 0003, 0004).
///
/// Children are stored bottom-to-top: index 0 draws first (underneath), the last
/// child draws last (on top). Focus order is the same vector order among
/// [`focusable`](View::focusable) children.
pub struct Group {
    bounds: Rect,
    children: Vec<Box<dyn View>>,
    focused: Option<usize>,
    wraps: bool,
}

impl Group {
    /// Creates a group occupying `bounds` that owns `children`. Focus starts on
    /// the first focusable child, or `None` if there is none; that child is told
    /// it holds the focus (ADR 0010).
    pub fn new(bounds: Rect, mut children: Vec<Box<dyn View>>) -> Self {
        let focused = children.iter().position(|child| child.focusable());
        if let Some(index) = focused {
            children[index].set_focused(true);
        }
        Self {
            bounds,
            children,
            focused,
            wraps: true,
        }
    }

    /// Opts this group out of wrapping its own Tab/Shift-Tab traversal at the
    /// ends (ADR 0031): instead of cycling back to its first/last focusable
    /// child, a boundary Tab reports `Ignored` so it escapes to whatever owns
    /// this group's own dispatch — for a `Group` embedded as one child inside
    /// another `Group` (e.g. [`GroupBox`](crate::widgets::GroupBox)'s
    /// interior), so the *outer* group's traversal decides where focus goes
    /// once this nested group's own stops are exhausted, rather than this
    /// group wrapping locally and swallowing the Tab. A top-level dialog
    /// `Group` (the common case, with no owning `Group` above it) leaves
    /// wrapping on — the default `Group::new` gives every existing caller.
    pub fn non_wrapping(mut self) -> Self {
        self.wraps = false;
        self
    }

    /// The index of the currently focused child, if any.
    pub fn focused(&self) -> Option<usize> {
        self.focused
    }

    /// Indices in the order [`draw`](View::draw) should visit them: every
    /// child *not* requesting topmost priority first (original relative
    /// order, unchanged), then every child that does (also its original
    /// relative order) — so a requesting child always paints last, over any
    /// ordinary sibling (ADR 0030).
    fn draw_order(&self) -> Vec<usize> {
        let n = self.children.len();
        let mut order: Vec<usize> = (0..n)
            .filter(|&i| !self.children[i].wants_topmost())
            .collect();
        order.extend((0..n).filter(|&i| self.children[i].wants_topmost()));
        order
    }

    /// Indices in the order [`dispatch_positional`](Self::dispatch_positional)
    /// should hit-test them: topmost-requesting children first (reverse of
    /// their relative order, so two such children still resolve topmost-first
    /// between themselves), then the ordinary reverse-order scan of the rest
    /// — the mirror image of [`draw_order`](Self::draw_order) (ADR 0030).
    fn hit_test_order(&self) -> Vec<usize> {
        let n = self.children.len();
        let mut order: Vec<usize> = (0..n)
            .filter(|&i| self.children[i].wants_topmost())
            .collect();
        order.reverse();
        let mut rest: Vec<usize> = (0..n)
            .filter(|&i| !self.children[i].wants_topmost())
            .collect();
        rest.reverse();
        order.extend(rest);
        order
    }

    /// Positional phase: deliver to the topmost child under the pointer, with the
    /// position translated into that child's local coordinates (ADR 0004). A
    /// left-press first moves focus to the clicked child if it can take it, so
    /// clicking a control focuses it the way TurboVision does.
    fn dispatch_positional(&mut self, mouse: MouseEvent, ctx: &mut Context) -> EventResult {
        for i in self.hit_test_order() {
            let bounds = self.children[i].bounds();
            if bounds.contains(mouse.pos) {
                if matches!(mouse.kind, MouseKind::Down(MouseButton::Left))
                    && self.children[i].focusable()
                {
                    self.set_focus(i);
                }
                let origin = bounds.origin();
                let local = MouseEvent {
                    pos: mouse.pos.offset(-origin.x, -origin.y),
                    ..mouse
                };
                let children = &mut self.children;
                return ctx.translated(origin.x, origin.y, |ctx| {
                    children[i].handle_event(&Event::Mouse(local), ctx)
                });
            }
        }
        EventResult::Ignored
    }

    /// Moves focus to child `index`, telling the old and new children (defaulted
    /// no-ops unless they draw themselves focused, ADR 0010).
    fn set_focus(&mut self, index: usize) {
        if self.focused != Some(index) {
            if let Some(old) = self.focused {
                self.children[old].set_focused(false);
            }
            self.children[index].set_focused(true);
            self.focused = Some(index);
        }
    }

    /// Focused phase: the focused child gets first crack; if it ignores a
    /// Tab/Shift-Tab, this group moves its own focus. Anything else ignored
    /// returns `Ignored` so it bubbles up the owner chain (ADR 0003).
    fn dispatch_focused(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        if let Some(index) = self.focused {
            if let Some(child) = self.children.get_mut(index) {
                if child.handle_event(event, ctx).is_consumed() {
                    return EventResult::Consumed;
                }
            }
        }
        if let Event::Key(key) = event {
            match key.code {
                KeyCode::Tab => return self.move_focus(true),
                KeyCode::BackTab => return self.move_focus(false),
                _ => {}
            }
        }
        EventResult::Ignored
    }

    /// Broadcast phase: deliver to every child; broadcasts don't stop, so the
    /// group reports `Ignored` (ADR 0004).
    fn dispatch_broadcast(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        for child in &mut self.children {
            child.handle_event(event, ctx);
        }
        EventResult::Ignored
    }

    /// Advances (or, if `!forward`, retreats) focus to the next focusable child,
    /// wrapping at the ends unless [`non_wrapping`](Self::non_wrapping) opted
    /// this group out (ADR 0031) — in which case a boundary Tab/Shift-Tab (one
    /// that would otherwise wrap) is left `Ignored` instead, so it escapes to
    /// whatever owns this group's own dispatch. Consumes the event, or ignores
    /// it if no child can take focus.
    fn move_focus(&mut self, forward: bool) -> EventResult {
        let focusable: Vec<usize> = (0..self.children.len())
            .filter(|&i| self.children[i].focusable())
            .collect();
        if focusable.is_empty() {
            return EventResult::Ignored;
        }
        let current = self
            .focused
            .and_then(|f| focusable.iter().position(|&i| i == f));
        let len = focusable.len();
        if !self.wraps {
            let at_boundary = match current {
                Some(p) => (forward && p + 1 == len) || (!forward && p == 0),
                None => false,
            };
            if at_boundary {
                return EventResult::Ignored;
            }
        }
        let next = match current {
            Some(p) if forward => (p + 1) % len,
            Some(p) => (p + len - 1) % len,
            None if forward => 0,
            None => len - 1,
        };
        self.set_focus(focusable[next]);
        EventResult::Consumed
    }
}

impl View for Group {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        for i in self.draw_order() {
            let child = &self.children[i];
            // A floating child casts its drop shadow on this group's surface
            // before it — and any higher sibling — is drawn on top (ADR 0011).
            if let Some(style) = child.drop_shadow() {
                canvas.shadow(child.bounds(), style);
            }
            let mut sub = canvas.child(child.bounds());
            child.draw(&mut sub);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Mouse(mouse) => self.dispatch_positional(*mouse, ctx),
            // Paste, like a key, goes to the focused child (e.g. an input line).
            Event::Key(_) | Event::Command(_) | Event::Paste(_) => {
                self.dispatch_focused(event, ctx)
            }
            Event::Broadcast(_) | Event::Resize(_) | Event::Idle => {
                self.dispatch_broadcast(event, ctx)
            }
        }
    }

    fn focusable(&self) -> bool {
        // A group can take focus iff it has a focusable child to delegate to.
        self.focused.is_some()
    }

    fn set_focused(&mut self, focused: bool) {
        // Forward to the focused child so the signal composes through nesting
        // (ADR 0010): an outer group telling this one it (lost) gained focus
        // (un)focuses whichever of our children currently holds it.
        if let Some(index) = self.focused {
            self.children[index].set_focused(focused);
        }
    }

    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
        // Fans out to every child, not just the focused one (ADR 0016,
        // mirroring TurboVision's TGroup::valid) — a fold, not a
        // short-circuiting `all()`, so two refusing children both get asked
        // (and so both get the chance to post their own follow-up) rather
        // than the second being skipped once the first has already refused.
        self.children
            .iter_mut()
            .fold(true, |ok, child| child.valid(command, ctx) && ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::cell::Cell;
    use crate::command::{CM_CANCEL, CM_OK};
    use crate::event::{KeyEvent, Modifiers, MouseButton, MouseKind};
    use crate::geometry::Size;
    use crate::widgets::Menu;
    use std::cell::RefCell;
    use std::rc::Rc;

    /// A shared event log: `(probe id, event)` in dispatch order. Lets a test
    /// inspect what a boxed, type-erased child saw.
    type Log = Rc<RefCell<Vec<(u16, Event)>>>;

    /// A focusable test view: records every event it handles into a shared log,
    /// fills its area with its id digit (so z-order is visible in snapshots), and
    /// optionally posts a command when a given key arrives.
    struct Probe {
        id: u16,
        bounds: Rect,
        focusable: bool,
        log: Log,
        post: Option<(KeyCode, Command)>,
        topmost: bool,
    }

    impl Probe {
        fn new(id: u16, bounds: Rect, focusable: bool, log: &Log) -> Self {
            Self {
                id,
                bounds,
                focusable,
                log: Rc::clone(log),
                post: None,
                topmost: false,
            }
        }

        fn posting(mut self, key: KeyCode, command: Command) -> Self {
            self.post = Some((key, command));
            self
        }

        /// Opts into ADR 0030's z-order priority (`View::wants_topmost`).
        fn topmost(mut self, yes: bool) -> Self {
            self.topmost = yes;
            self
        }

        fn boxed(self) -> Box<dyn View> {
            Box::new(self)
        }
    }

    impl View for Probe {
        fn bounds(&self) -> Rect {
            self.bounds
        }

        fn draw(&self, canvas: &mut Canvas) {
            let digit = char::from_digit(self.id as u32 % 10, 10).unwrap_or('?');
            let area = canvas.bounds();
            canvas.fill(area, &Cell::from_char(digit, Style::new()));
        }

        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            self.log.borrow_mut().push((self.id, event.clone()));
            if let (Event::Key(key), Some((code, command))) = (event, self.post) {
                if key.code == code {
                    ctx.post(command);
                    return EventResult::Consumed;
                }
            }
            EventResult::Ignored
        }

        fn focusable(&self) -> bool {
            self.focusable
        }

        fn wants_topmost(&self) -> bool {
            self.topmost
        }
    }

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    fn mouse_down_at(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, Modifiers::NONE))
    }

    fn right_click_at(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Right),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    /// A leaf that offers a context menu, anchored at whatever local position
    /// a right-click reaches it at, on the same event that triggers it
    /// (ADR 0019).
    struct Offerer {
        bounds: Rect,
    }

    impl View for Offerer {
        fn bounds(&self) -> Rect {
            self.bounds
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            if let Event::Mouse(mouse) = event {
                if mouse.kind == MouseKind::Down(MouseButton::Right) {
                    ctx.open_context_menu(Menu::new("M", vec![]), mouse.pos);
                    return EventResult::Consumed;
                }
            }
            EventResult::Ignored
        }
    }

    // --- StaticText (tracer bullet) ---

    #[test]
    fn static_text_draws_its_string_and_is_not_focusable() {
        let text = StaticText::new(rect(0, 0, 10, 1), "hello", Style::new());
        assert!(!text.focusable());

        let mut buf = Buffer::new(Size::new(10, 1));
        let mut canvas = Canvas::new(&mut buf);
        let mut sub = canvas.child(text.bounds());
        text.draw(&mut sub);
        assert_eq!(buf.to_text(), "hello     ");
    }

    // --- Positional dispatch ---

    #[test]
    fn mouse_goes_to_the_child_under_it_in_local_coords() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(1, 1, 5, 5), false, &log).boxed(),
                Probe::new(2, rect(10, 1, 5, 5), false, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        // A click inside child 2's box reaches child 2 with a translated point.
        group.handle_event(&mouse_down_at(11, 2), &mut ctx);
        let seen = log.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, 2);
        assert_eq!(seen[0].1, mouse_down_at(1, 1));
    }

    #[test]
    fn mouse_goes_to_the_topmost_of_overlapping_children() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(0, 0, 10, 10), false, &log).boxed(),
                Probe::new(2, rect(2, 2, 5, 5), false, &log).boxed(), // drawn last = on top
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        group.handle_event(&mouse_down_at(3, 3), &mut ctx);
        let seen = log.borrow();
        assert_eq!(seen.len(), 1, "only the topmost child receives the click");
        assert_eq!(seen[0].0, 2);
        assert_eq!(seen[0].1, mouse_down_at(1, 1));
    }

    #[test]
    fn mouse_outside_every_child_is_ignored() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![Probe::new(1, rect(1, 1, 3, 3), false, &log).boxed()],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        let result = group.handle_event(&mouse_down_at(15, 8), &mut ctx);
        assert_eq!(result, EventResult::Ignored);
        assert!(log.borrow().is_empty());
    }

    #[test]
    fn a_context_menu_request_from_a_child_resolves_to_screen_coordinates() {
        // The child sits at (10, 1); it offers a menu anchored at its own
        // local (2, 3). Group must translate that back out through the same
        // offset it applied on the way in (ADR 0019), so the request Group's
        // caller sees is in Group's own coordinate space: (12, 4).
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![Box::new(Offerer {
                bounds: rect(10, 1, 5, 5),
            })],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        group.handle_event(&right_click_at(12, 4), &mut ctx);
        let req = ctx.take_context_menu_request().unwrap();
        assert_eq!(req.at, Point::new(12, 4));
    }

    #[test]
    fn a_left_press_focuses_the_clicked_focusable_child() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(1, 1, 5, 5), true, &log).boxed(),
                Probe::new(2, rect(10, 1, 5, 5), true, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        group.handle_event(&mouse_down_at(11, 2), &mut ctx); // inside child 2
        assert_eq!(group.focused(), Some(1));
        group.handle_event(&mouse_down_at(2, 2), &mut ctx); // inside child 1
        assert_eq!(group.focused(), Some(0));
    }

    #[test]
    fn a_press_on_a_non_focusable_child_leaves_focus_alone() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(1, 1, 5, 5), true, &log).boxed(),
                Probe::new(2, rect(10, 1, 5, 5), false, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        group.handle_event(&mouse_down_at(2, 2), &mut ctx); // focus the focusable child 1
        assert_eq!(group.focused(), Some(0));
        group.handle_event(&mouse_down_at(11, 2), &mut ctx); // click the non-focusable child 2
        assert_eq!(group.focused(), Some(0), "focus stays on child 1");
    }

    // --- Focus traversal ---

    #[test]
    fn initial_focus_is_the_first_focusable_child() {
        let log: Log = Log::default();
        let group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Box::new(StaticText::new(rect(0, 0, 5, 1), "x", Style::new())),
                Probe::new(1, rect(0, 1, 5, 1), true, &log).boxed(),
                Probe::new(2, rect(0, 2, 5, 1), true, &log).boxed(),
            ],
        );
        assert_eq!(group.focused(), Some(1));
    }

    #[test]
    fn tab_and_back_tab_cycle_focus_skipping_static_text() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Box::new(StaticText::new(rect(0, 0, 5, 1), "x", Style::new())),
                Probe::new(1, rect(0, 1, 5, 1), true, &log).boxed(),
                Probe::new(2, rect(0, 2, 5, 1), true, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert_eq!(group.focused(), Some(1));
        assert_eq!(
            group.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(group.focused(), Some(2));
        // Tab wraps past the end back to the first focusable child.
        group.handle_event(&key(KeyCode::Tab), &mut ctx);
        assert_eq!(group.focused(), Some(1));
        // Shift-Tab goes the other way, wrapping past the start.
        group.handle_event(&key(KeyCode::BackTab), &mut ctx);
        assert_eq!(group.focused(), Some(2));
    }

    #[test]
    fn keys_reach_only_the_focused_child() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(0, 0, 5, 1), true, &log).boxed(),
                Probe::new(2, rect(0, 1, 5, 1), true, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        // Focus is on child 1; a plain character key reaches only it.
        group.handle_event(&key(KeyCode::Char('a')), &mut ctx);
        let seen = log.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, 1);
        assert_eq!(seen[0].1, key(KeyCode::Char('a')));
    }

    #[test]
    fn group_with_no_focusable_children_ignores_tab() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![Probe::new(1, rect(0, 0, 5, 1), false, &log).boxed()],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert_eq!(group.focused(), None);
        assert_eq!(
            group.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Ignored
        );
    }

    // --- Non-wrapping (nested-scope) Tab boundary (ADR 0031) ---

    #[test]
    fn a_non_wrapping_group_with_one_focusable_child_ignores_boundary_tab() {
        // Reproduces the bug a `GroupBox` with a single focusable child (one
        // `RadioButtons`) hit when embedded in an outer `Group`: an ordinary
        // wrapping group would treat Tab as "wrap back to myself" and consume
        // it, swallowing the keypress so it could never reach a sibling past
        // the group box.
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![Probe::new(1, rect(0, 0, 5, 1), true, &log).boxed()],
        )
        .non_wrapping();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert_eq!(group.focused(), Some(0));
        assert_eq!(
            group.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Ignored,
            "a lone focusable child is always at the boundary in both directions"
        );
        assert_eq!(group.focused(), Some(0), "focus did not move");
        assert_eq!(
            group.handle_event(&key(KeyCode::BackTab), &mut ctx),
            EventResult::Ignored
        );
    }

    #[test]
    fn a_non_wrapping_group_still_cycles_internally_before_the_boundary() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(0, 0, 5, 1), true, &log).boxed(),
                Probe::new(2, rect(0, 1, 5, 1), true, &log).boxed(),
            ],
        )
        .non_wrapping();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert_eq!(group.focused(), Some(0));
        assert_eq!(
            group.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed,
            "moving to the second child is still a normal, internal move"
        );
        assert_eq!(group.focused(), Some(1));
        assert_eq!(
            group.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Ignored,
            "now at the last child, forward Tab escapes instead of wrapping"
        );
        assert_eq!(group.focused(), Some(1), "focus stayed put, not wrapped");
    }

    #[test]
    fn a_wrapping_group_is_the_default_and_unaffected() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![Probe::new(1, rect(0, 0, 5, 1), true, &log).boxed()],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            group.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed,
            "plain Group::new still wraps a lone focusable child to itself"
        );
    }

    #[test]
    fn empty_group_ignores_everything() {
        let mut group = Group::new(rect(0, 0, 20, 10), vec![]);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert!(group.focused().is_none());
        assert_eq!(
            group.handle_event(&key(KeyCode::Char('a')), &mut ctx),
            EventResult::Ignored
        );
        assert_eq!(
            group.handle_event(&mouse_down_at(1, 1), &mut ctx),
            EventResult::Ignored
        );
    }

    // --- Focus notification (ADR 0010) ---

    /// A focusable leaf that records its current focus flag, set by its owner.
    struct FocusSpy {
        bounds: Rect,
        focused: Rc<RefCell<bool>>,
    }

    impl View for FocusSpy {
        fn bounds(&self) -> Rect {
            self.bounds
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn focusable(&self) -> bool {
            true
        }
        fn set_focused(&mut self, focused: bool) {
            *self.focused.borrow_mut() = focused;
        }
    }

    #[test]
    fn group_tells_its_initial_focused_child_it_has_focus() {
        let a = Rc::new(RefCell::new(false));
        let b = Rc::new(RefCell::new(false));
        let _group = Group::new(
            rect(0, 0, 10, 3),
            vec![
                Box::new(StaticText::new(rect(0, 0, 5, 1), "x", Style::new())),
                Box::new(FocusSpy {
                    bounds: rect(0, 1, 5, 1),
                    focused: Rc::clone(&a),
                }),
                Box::new(FocusSpy {
                    bounds: rect(0, 2, 5, 1),
                    focused: Rc::clone(&b),
                }),
            ],
        );
        assert!(
            *a.borrow(),
            "the first focusable child is told it has focus"
        );
        assert!(!*b.borrow(), "a later child is not");
    }

    #[test]
    fn tab_moves_the_focus_flag_between_children() {
        let a = Rc::new(RefCell::new(false));
        let b = Rc::new(RefCell::new(false));
        let mut group = Group::new(
            rect(0, 0, 10, 3),
            vec![
                Box::new(FocusSpy {
                    bounds: rect(0, 0, 5, 1),
                    focused: Rc::clone(&a),
                }),
                Box::new(FocusSpy {
                    bounds: rect(0, 1, 5, 1),
                    focused: Rc::clone(&b),
                }),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        group.handle_event(&key(KeyCode::Tab), &mut ctx);
        assert!(!*a.borrow(), "the old focus child is told it lost focus");
        assert!(*b.borrow(), "the new focus child is told it gained focus");
    }

    #[test]
    fn a_group_forwards_focus_to_its_focused_child() {
        // An outer container telling a nested group it lost focus must reach the
        // grandchild that actually holds it (composition, ADR 0010).
        let leaf = Rc::new(RefCell::new(false));
        let mut group = Group::new(
            rect(0, 0, 10, 3),
            vec![Box::new(FocusSpy {
                bounds: rect(0, 0, 5, 1),
                focused: Rc::clone(&leaf),
            })],
        );
        assert!(*leaf.borrow());
        group.set_focused(false);
        assert!(!*leaf.borrow(), "the unfocus reached the grandchild");
        group.set_focused(true);
        assert!(*leaf.borrow());
    }

    // --- Commands: posting, gating, and bubbling ---

    #[test]
    fn focused_child_posts_a_command() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(0, 0, 5, 1), true, &log)
                    .posting(KeyCode::Enter, CM_OK)
                    .boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        let result = group.handle_event(&key(KeyCode::Enter), &mut ctx);
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
    }

    #[test]
    fn a_disabled_command_is_not_posted() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(0, 0, 5, 1), true, &log)
                    .posting(KeyCode::Enter, CM_OK)
                    .boxed(),
            ],
        );
        let mut cs = CommandSet::new();
        cs.disable(CM_OK);
        let mut ctx = Context::new(&cs);

        group.handle_event(&key(KeyCode::Enter), &mut ctx);
        assert!(ctx.posted().is_empty(), "a disabled command never fires");
    }

    #[test]
    fn a_command_routes_down_the_focus_chain_and_bubbles_back_up() {
        // Outer group -> inner group -> leaf probe. A Command travels down the
        // focus chain to the leaf; the leaf ignores it, and the Ignored unwinds
        // back up the owner chain — that unwinding is the bubble (ADR 0003).
        let log: Log = Log::default();
        let inner = Group::new(
            rect(2, 1, 10, 8),
            vec![Probe::new(7, rect(0, 0, 5, 1), true, &log).boxed()],
        );
        let mut outer = Group::new(rect(0, 0, 20, 10), vec![Box::new(inner)]);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        let result = outer.handle_event(&Event::Command(CM_OK), &mut ctx);
        assert_eq!(
            result,
            EventResult::Ignored,
            "unhandled command bubbles out the top"
        );
        let seen = log.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0], (7, Event::Command(CM_OK)), "it reached the leaf");
    }

    // --- Broadcast & z-order draw ---

    #[test]
    fn broadcast_reaches_every_child() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(0, 0, 5, 1), false, &log).boxed(),
                Probe::new(2, rect(0, 1, 5, 1), false, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        group.handle_event(&Event::Broadcast(CM_OK), &mut ctx);
        let seen = log.borrow();
        let ids: Vec<u16> = seen.iter().map(|(id, _)| *id).collect();
        assert_eq!(ids, vec![1, 2], "both children saw the broadcast");
    }

    // --- Drop-shadow protocol (ADR 0011) ---

    #[test]
    fn a_plain_view_casts_no_shadow() {
        let text = StaticText::new(rect(0, 0, 5, 1), "x", Style::new());
        assert_eq!(text.drop_shadow(), None);
    }

    /// A view that fills itself with 'X' and casts a shadow in a given style.
    struct ShadowBox {
        bounds: Rect,
        shadow: Style,
    }

    impl View for ShadowBox {
        fn bounds(&self) -> Rect {
            self.bounds
        }
        fn draw(&self, canvas: &mut Canvas) {
            let area = canvas.bounds();
            canvas.fill(area, &Cell::from_char('X', Style::new()));
        }
        fn drop_shadow(&self) -> Option<Style> {
            Some(self.shadow)
        }
    }

    #[test]
    fn a_group_paints_a_childs_drop_shadow_outside_the_child() {
        let shadow = crate::theme::Theme::default().style(crate::theme::Role::Shadow);
        // A 5×3 child at (2, 2): its right strip starts at x = 7, its body at x = 2.
        let group = Group::new(
            rect(0, 0, 20, 10),
            vec![Box::new(ShadowBox {
                bounds: rect(2, 2, 5, 3),
                shadow,
            })],
        );
        let mut buf = Buffer::new(Size::new(20, 10));
        let mut canvas = Canvas::new(&mut buf);
        group.draw(&mut canvas);

        // The right-edge shadow cell is repainted in the shadow style…
        assert_eq!(buf.get(Point::new(7, 3)).unwrap().style(), shadow);
        // …the child's own body sits on top, undimmed…
        let body = buf.get(Point::new(3, 3)).unwrap();
        assert_eq!(body.grapheme().to_string(), "X");
        assert_eq!(body.style(), Style::new());
        // …and a cell clear of both is untouched.
        assert_eq!(buf.get(Point::new(0, 0)).unwrap().style(), Style::new());
    }

    // --- Topmost priority protocol (ADR 0030) ---

    #[test]
    fn a_plain_view_does_not_want_topmost() {
        let text = StaticText::new(rect(0, 0, 5, 1), "x", Style::new());
        assert!(!text.wants_topmost());
    }

    #[test]
    fn a_topmost_child_draws_over_a_later_ordinary_sibling_at_the_same_spot() {
        // Child 0 requests topmost priority; child 1 is an ordinary later
        // sibling occupying the same cell. Insertion order alone would have
        // child 1 (drawn last) win — `wants_topmost` must override that.
        let log: Log = Log::default();
        let group = Group::new(
            rect(0, 0, 5, 5),
            vec![
                Probe::new(1, rect(0, 0, 3, 3), false, &log)
                    .topmost(true)
                    .boxed(),
                Probe::new(2, rect(0, 0, 3, 3), false, &log).boxed(),
            ],
        );
        let mut buf = Buffer::new(Size::new(5, 5));
        let mut canvas = Canvas::new(&mut buf);
        group.draw(&mut canvas);
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().grapheme().to_string(),
            "1",
            "the topmost-requesting child painted last, over its ordinary sibling"
        );
    }

    #[test]
    fn a_topmost_child_is_hit_tested_before_a_later_ordinary_sibling() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 5, 5),
            vec![
                Probe::new(1, rect(0, 0, 3, 3), false, &log)
                    .topmost(true)
                    .boxed(),
                Probe::new(2, rect(0, 0, 3, 3), false, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        group.handle_event(&mouse_down_at(1, 1), &mut ctx);
        let seen = log.borrow();
        assert_eq!(seen.len(), 1, "only the topmost-requesting child was hit");
        assert_eq!(seen[0].0, 1);
    }

    #[test]
    fn two_topmost_children_still_resolve_topmost_first_between_themselves() {
        // Both request priority; ordinary z-order (last-drawn-wins) still
        // decides which of the *two* requesting children is on top.
        let log: Log = Log::default();
        let group = Group::new(
            rect(0, 0, 5, 5),
            vec![
                Probe::new(1, rect(0, 0, 3, 3), false, &log)
                    .topmost(true)
                    .boxed(),
                Probe::new(2, rect(0, 0, 3, 3), false, &log)
                    .topmost(true)
                    .boxed(),
                Probe::new(3, rect(0, 0, 3, 3), false, &log).boxed(),
            ],
        );
        let mut buf = Buffer::new(Size::new(5, 5));
        let mut canvas = Canvas::new(&mut buf);
        group.draw(&mut canvas);
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().grapheme().to_string(),
            "2",
            "the later of the two topmost-requesting children wins"
        );
    }

    #[test]
    fn without_any_topmost_request_ordinary_z_order_is_unchanged() {
        let log: Log = Log::default();
        let group = Group::new(
            rect(0, 0, 5, 5),
            vec![
                Probe::new(1, rect(0, 0, 3, 3), false, &log).boxed(),
                Probe::new(2, rect(0, 0, 3, 3), false, &log).boxed(),
            ],
        );
        let mut buf = Buffer::new(Size::new(5, 5));
        let mut canvas = Canvas::new(&mut buf);
        group.draw(&mut canvas);
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().grapheme().to_string(),
            "2",
            "plain insertion-order z-order, exactly as before ADR 0030"
        );
    }

    #[test]
    fn snapshot_children_draw_in_z_order() {
        let log: Log = Log::default();
        let group = Group::new(
            rect(0, 0, 8, 4),
            vec![
                Probe::new(1, rect(0, 0, 8, 4), false, &log).boxed(), // fills '1'
                Probe::new(2, rect(2, 1, 4, 2), false, &log).boxed(), // '2' on top
            ],
        );
        let mut buf = Buffer::new(Size::new(8, 4));
        let mut canvas = Canvas::new(&mut buf);
        group.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    // --- Scroll-chrome protocol (ADR 0015) ---

    #[test]
    fn a_plain_view_reports_no_scroll_metrics_and_ignores_set_scroll() {
        let mut text = StaticText::new(rect(0, 0, 5, 1), "x", Style::new());
        assert_eq!(text.scroll_metrics(), None);
        text.set_scroll(Point::new(3, 4)); // no panic, no-op
    }

    // --- Status-content protocol (ADR 0032) ---

    #[test]
    fn a_plain_view_reports_no_status_text() {
        let text = StaticText::new(rect(0, 0, 5, 1), "x", Style::new());
        assert_eq!(text.status_text(), None);
    }

    /// A view that reports a fixed vertical range and records the last
    /// `set_scroll` offset it was pushed.
    struct Scrollable {
        bounds: Rect,
        metrics: Option<ScrollMetrics>,
        pushed: Rc<RefCell<Option<Point>>>,
    }

    impl View for Scrollable {
        fn bounds(&self) -> Rect {
            self.bounds
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn scroll_metrics(&self) -> Option<ScrollMetrics> {
            self.metrics
        }
        fn set_scroll(&mut self, offset: Point) {
            *self.pushed.borrow_mut() = Some(offset);
        }
    }

    #[test]
    fn a_scrollable_view_reports_metrics_and_accepts_a_pushed_offset() {
        let pushed = Rc::new(RefCell::new(None));
        let metrics = ScrollMetrics {
            horizontal: None,
            vertical: Some(AxisMetrics {
                total: 10,
                visible: 4,
                pos: 2,
            }),
        };
        let mut view = Scrollable {
            bounds: rect(0, 0, 5, 4),
            metrics: Some(metrics),
            pushed: Rc::clone(&pushed),
        };
        assert_eq!(view.scroll_metrics(), Some(metrics));
        view.set_scroll(Point::new(0, 5));
        assert_eq!(*pushed.borrow(), Some(Point::new(0, 5)));
    }

    // --- `valid` veto protocol (ADR 0016) ---

    #[test]
    fn a_plain_view_is_always_valid() {
        let mut text = StaticText::new(rect(0, 0, 5, 1), "x", Style::new());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(text.valid(CM_OK, &mut ctx));
    }

    /// A view that refuses one specific command, optionally posting a
    /// follow-up command through `ctx` when it does.
    struct Vetoer {
        bounds: Rect,
        refuses: Command,
        follow_up: Option<Command>,
        asked: Rc<RefCell<Vec<Command>>>,
    }

    impl View for Vetoer {
        fn bounds(&self) -> Rect {
            self.bounds
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
            self.asked.borrow_mut().push(command);
            if command == self.refuses {
                if let Some(follow_up) = self.follow_up {
                    ctx.post(follow_up);
                }
                false
            } else {
                true
            }
        }
    }

    #[test]
    fn a_refusing_view_can_post_a_follow_up_command() {
        let asked = Rc::new(RefCell::new(Vec::new()));
        let mut vetoer = Vetoer {
            bounds: rect(0, 0, 5, 1),
            refuses: CM_OK,
            follow_up: Some(CM_CANCEL),
            asked: Rc::clone(&asked),
        };
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!vetoer.valid(CM_OK, &mut ctx));
        assert_eq!(ctx.posted(), &[Event::Command(CM_CANCEL)]);
    }

    #[test]
    fn group_valid_is_true_when_every_child_agrees() {
        let log: Log = Log::default();
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Probe::new(1, rect(0, 0, 5, 1), false, &log).boxed(),
                Probe::new(2, rect(0, 1, 5, 1), false, &log).boxed(),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(group.valid(CM_OK, &mut ctx));
    }

    #[test]
    fn one_refusing_child_makes_the_whole_group_refuse() {
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![Box::new(Vetoer {
                bounds: rect(0, 0, 5, 1),
                refuses: CM_OK,
                follow_up: None,
                asked: Rc::new(RefCell::new(Vec::new())),
            })],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!group.valid(CM_OK, &mut ctx));
    }

    #[test]
    fn every_child_is_asked_even_after_one_refuses() {
        // Not a short-circuiting `all()`: two refusing children both get the
        // chance to post their own follow-up in the same pass.
        let asked_a: Rc<RefCell<Vec<Command>>> = Rc::new(RefCell::new(Vec::new()));
        let asked_b: Rc<RefCell<Vec<Command>>> = Rc::new(RefCell::new(Vec::new()));
        let mut group = Group::new(
            rect(0, 0, 20, 10),
            vec![
                Box::new(Vetoer {
                    bounds: rect(0, 0, 5, 1),
                    refuses: CM_OK,
                    follow_up: None,
                    asked: Rc::clone(&asked_a),
                }),
                Box::new(Vetoer {
                    bounds: rect(0, 1, 5, 1),
                    refuses: CM_OK,
                    follow_up: None,
                    asked: Rc::clone(&asked_b),
                }),
            ],
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!group.valid(CM_OK, &mut ctx));
        assert_eq!(*asked_a.borrow(), vec![CM_OK], "the first child was asked");
        assert_eq!(
            *asked_b.borrow(),
            vec![CM_OK],
            "the second child was asked too, not skipped"
        );
    }

    // --- Context: offset accumulator & context-menu requests (ADR 0019) ---

    fn probe_menu() -> Menu {
        Menu::new("Ctx", vec![])
    }

    #[test]
    fn open_context_menu_with_no_offset_resolves_to_the_given_point() {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        ctx.open_context_menu(probe_menu(), Point::new(5, 3));
        let req = ctx.take_context_menu_request().unwrap();
        assert_eq!(req.at, Point::new(5, 3));
    }

    #[test]
    fn translated_shifts_a_request_made_inside_it() {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        ctx.translated(10, 2, |ctx| {
            ctx.open_context_menu(probe_menu(), Point::new(1, 1));
        });
        let req = ctx.take_context_menu_request().unwrap();
        assert_eq!(
            req.at,
            Point::new(11, 3),
            "the local point is shifted by the pushed offset"
        );
    }

    #[test]
    fn nested_translated_scopes_compose() {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        ctx.translated(10, 2, |ctx| {
            ctx.translated(3, 1, |ctx| {
                ctx.open_context_menu(probe_menu(), Point::new(0, 0));
            });
        });
        let req = ctx.take_context_menu_request().unwrap();
        assert_eq!(
            req.at,
            Point::new(13, 3),
            "nested offsets add up, screen coordinates regardless of depth"
        );
    }

    #[test]
    fn translated_restores_the_previous_offset_after_returning() {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        ctx.translated(10, 2, |ctx| {
            ctx.open_context_menu(probe_menu(), Point::new(0, 0));
        });
        ctx.take_context_menu_request(); // drain the first request
        // A second request made outside any `translated` scope is unshifted,
        // proving the offset didn't leak past the closure that pushed it.
        ctx.open_context_menu(probe_menu(), Point::new(4, 4));
        let req = ctx.take_context_menu_request().unwrap();
        assert_eq!(req.at, Point::new(4, 4));
    }

    #[test]
    fn take_context_menu_request_clears_the_pending_request() {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(ctx.take_context_menu_request().is_none());
        ctx.open_context_menu(probe_menu(), Point::new(1, 1));
        assert!(ctx.take_context_menu_request().is_some());
        assert!(
            ctx.take_context_menu_request().is_none(),
            "taking clears the pending request"
        );
    }

    #[test]
    fn a_later_request_replaces_an_earlier_one() {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        ctx.open_context_menu(probe_menu(), Point::new(1, 1));
        ctx.open_context_menu(probe_menu(), Point::new(9, 9));
        let req = ctx.take_context_menu_request().unwrap();
        assert_eq!(req.at, Point::new(9, 9), "the later request wins");
    }

    #[test]
    fn request_mouse_capture_sets_a_flag_take_clears_it() {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!ctx.take_mouse_capture_request(), "none requested yet");
        ctx.request_mouse_capture();
        assert!(ctx.take_mouse_capture_request(), "a request was made");
        assert!(
            !ctx.take_mouse_capture_request(),
            "taking clears the pending request"
        );
    }

    #[test]
    fn commands_accessor_exposes_the_same_enabled_state() {
        let mut cs = CommandSet::new();
        cs.disable(CM_OK);
        let ctx = Context::new(&cs);
        assert!(!ctx.commands().is_enabled(CM_OK));
        assert!(ctx.commands().is_enabled(CM_CANCEL));
    }
}
