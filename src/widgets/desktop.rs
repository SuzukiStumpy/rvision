//! The desktop: a backdrop with a dynamic stack of windows on top of it
//! (TurboVision's `TDesktop`, made real — ADR 0016).
//!
//! Windows are opened and closed at runtime through opaque [`WindowId`]s
//! rather than a fixed list built once. They are stored bottom-to-top: index
//! 0 draws first, the last **visible** one draws on top and is active. Any
//! operation that raises a window (`open`/`show`/`focus`/`cycle_focus`/
//! click-to-front) physically moves it to the end of the stack, so draw order
//! stays a true z-order. The desktop fills its whole area with the backdrop
//! first, so the gaps between and around windows always show the blue field.
//!
//! `Desktop` also owns its own drag/resize sessions — mirroring how
//! [`MenuBar`](super::MenuBar) owns its own open/closed state machine across a
//! sequence of events rather than handling each one statelessly — and
//! intercepts `CM_CLOSE`/`CM_ZOOM`/`CM_NEXT`/`CM_PREV` before they would
//! otherwise just fall through to the active window.

use crate::arrange::{self, ChromeFlags, ChromeHit};
use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::command::{Accelerator, Accelerators, CM_CLOSE, CM_NEXT, CM_PREV, CM_ZOOM, Command};
use crate::event::{Event, EventResult, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::view::{Context, View};

use super::Window;
// Only the test module below still derives glyph spans directly (to compute
// expected click positions); production code now goes through `arrange`.
#[cfg(test)]
use super::Frame;

/// An opaque handle to a window `Desktop` owns, from a monotonic counter kept
/// internally. No locking: the event loop is single-threaded by design
/// (CLAUDE.md), so there is nothing to race (ADR 0016).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct WindowId(u64);

/// The smallest a window may shrink to during a resize session: enough width
/// for the frame's close/zoom glyphs ([`Frame::glyphs_shown`]'s threshold for
/// a window with no help topic) and enough height for the border plus one
/// interior row. A window with a help topic (ADR 0021) needs a wider frame
/// still to keep showing all three glyphs, but this floor doesn't chase
/// that — such a window's glyphs can disappear before a resize hits this
/// floor, same as any other narrow-frame glyph drop. An implementation
/// detail, not a design question (see `docs/specs/desktop.md`'s open
/// questions).
const MIN_SIZE: Size = Size::new(10, 3);

/// An in-progress title-bar move or corner resize, owned by `Desktop` for as
/// long as the mouse button stays down — the geometry itself lives in
/// [`arrange::ArrangeSession`] (ADR 0033), shared with `Window`'s own
/// chrome hit-testing and (prospectively) `edit`'s bespoke MDI.
struct DragSession {
    window: WindowId,
    session: arrange::ArrangeSession,
}

/// A backdrop plus a dynamic stack of windows.
pub struct Desktop {
    bounds: Rect,
    backdrop: Cell,
    windows: Vec<(WindowId, Window)>,
    active: Option<WindowId>,
    next_id: u64,
    drag: Option<DragSession>,
    /// A window that asked (via `Context::request_mouse_capture`) to keep
    /// receiving every mouse event regardless of pointer position, until
    /// release — a scroll-bar thumb drag's use case, generalised so
    /// `Desktop` needs no `ScrollBar` knowledge of its own (ADR 0027). Kept
    /// separate from `drag`: `Move`/`Resize` are a `Desktop`-owned concept
    /// (ADR 0016) where `Desktop` computes the new bounds itself; this is
    /// the opposite — `Desktop` computes nothing, just keeps forwarding.
    captured: Option<WindowId>,
    /// The system-level global keyboard shortcut table (ADR 0028): checked
    /// as a fallback whenever the active window's own key handling declines
    /// a key, independent of whether anything displays a hint for it (unlike
    /// the app-specific `StatusLine` hint it replaces).
    accelerators: Accelerators,
}

impl Desktop {
    /// An empty desktop occupying `bounds`, filled with `backdrop`.
    pub fn new(bounds: Rect, backdrop: Cell) -> Self {
        Self {
            bounds,
            backdrop,
            windows: Vec::new(),
            active: None,
            next_id: 0,
            drag: None,
            captured: None,
            accelerators: Accelerators::new(),
        }
    }

    /// Registers a global keyboard shortcut (ADR 0028): pressing
    /// `accelerator`'s key, whenever nothing more specific already claims it
    /// (the active window's own handling always gets first refusal), posts
    /// its command. Works with no window open at all, and needs no visible
    /// status-line hint — `Shell::new` feeds one in per `StatusItem`
    /// automatically, but this is also how an app binds one with no status
    /// bar slot at all (`shell.desktop_mut().bind_accelerator(...)`).
    pub fn bind_accelerator(&mut self, accelerator: Accelerator) {
        self.accelerators.bind(accelerator);
    }

    /// Adds `window` to the stack, raised to the top and made active.
    pub fn open(&mut self, window: Window) -> WindowId {
        let id = WindowId(self.next_id);
        self.next_id += 1;
        self.windows.push((id, window));
        self.raise(id);
        id
    }

    /// Asks `id`'s `valid(CM_CLOSE, ctx)`; if it agrees, removes and returns
    /// the window (transferring active to the next visible one in stack
    /// order, or `None`). A refusal, or an unknown `id`, leaves the desktop
    /// unchanged and returns `None` — the window may still have posted a
    /// follow-up through `ctx` (e.g. "confirm discard").
    pub fn close(&mut self, id: WindowId, ctx: &mut Context) -> Option<Window> {
        let pos = self.windows.iter().position(|(wid, _)| *wid == id)?;
        if !self.windows[pos].1.valid(CM_CLOSE, ctx) {
            return None;
        }
        let (_, window) = self.windows.remove(pos);
        if self.active == Some(id) {
            self.active = None;
            self.activate_topmost_visible();
        }
        Some(window)
    }

    /// Hides `id` (no-op on an unknown id). Reassigns active to the next
    /// visible window in stack order if `id` was active, without removing it
    /// or invalidating its `WindowId` (TurboVision's `TView::hide`).
    pub fn hide(&mut self, id: WindowId) {
        let Some((_, window)) = self.windows.iter_mut().find(|(wid, _)| *wid == id) else {
            return;
        };
        window.hide();
        window.set_active(false);
        if self.active == Some(id) {
            self.active = None;
            self.activate_topmost_visible();
        }
    }

    /// Shows `id` again, raised to the top and made active — like
    /// click-to-front, but programmatic. No-op on an unknown id.
    pub fn show(&mut self, id: WindowId) {
        let Some((_, window)) = self.windows.iter_mut().find(|(wid, _)| *wid == id) else {
            return;
        };
        window.show();
        self.raise(id);
    }

    /// Raises `id` to the top and makes it active. No-op if `id` is hidden or
    /// unknown.
    pub fn focus(&mut self, id: WindowId) {
        let Some((_, window)) = self.windows.iter().find(|(wid, _)| *wid == id) else {
            return;
        };
        if !window.is_visible() {
            return;
        }
        self.raise(id);
    }

    /// Moves active to the next (or, if `!forward`, previous) *visible*
    /// window in the current stack order, wrapping, and raises it to the top
    /// — the keyboard equivalent of click-to-front (`CM_NEXT`/`CM_PREV`). A
    /// safe no-op with fewer than two visible windows.
    pub fn cycle_focus(&mut self, forward: bool) {
        let visible: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.is_visible())
            .map(|(id, _)| *id)
            .collect();
        if visible.len() < 2 {
            return;
        }
        let current = self
            .active
            .and_then(|id| visible.iter().position(|&v| v == id));
        let next = match current {
            Some(i) if forward => (i + 1) % visible.len(),
            Some(i) => (i + visible.len() - 1) % visible.len(),
            None => 0,
        };
        self.raise(visible[next]);
    }

    /// Repositions every visible, non-maximized window into a cascade —
    /// stepped down-right from the top-left, wrapping every 8
    /// ([`arrange::cascade_slot`], ADR 0033) — in current stack order. A
    /// maximized window already fills the desktop and is left untouched
    /// rather than force-restored; a hidden window is skipped too. Z-order
    /// and the active window are unchanged — only bounds move.
    pub fn cascade(&mut self) {
        let desktop = self.bounds.size();
        let mut index = 0;
        for (_, window) in self.windows.iter_mut() {
            if !window.is_visible() || window.is_maximized() {
                continue;
            }
            window.set_bounds(arrange::cascade_slot(desktop, index, MIN_SIZE));
            index += 1;
        }
    }

    /// Repositions every visible, non-maximized window into an even grid
    /// filling the desktop ([`arrange::tile`], ADR 0033), in current stack
    /// order. Same maximized/hidden exclusions as [`cascade`](Self::cascade).
    pub fn tile(&mut self) {
        let desktop = self.bounds.size();
        let ids: Vec<WindowId> = self
            .windows
            .iter()
            .filter(|(_, w)| w.is_visible() && !w.is_maximized())
            .map(|(id, _)| *id)
            .collect();
        let slot_count = ids.len();
        for (id, slot) in ids.into_iter().zip(arrange::tile(desktop, slot_count)) {
            if let Some(window) = self.window_mut(id) {
                window.set_bounds(slot);
            }
        }
    }

    /// The id of the active (topmost visible) window, or `None` if the
    /// desktop has no visible windows.
    pub fn active_id(&self) -> Option<WindowId> {
        self.active
    }

    /// A reference to `id`'s window, or `None` if `id` is unknown.
    pub fn window(&self, id: WindowId) -> Option<&Window> {
        self.windows
            .iter()
            .find(|(wid, _)| *wid == id)
            .map(|(_, w)| w)
    }

    /// A mutable reference to `id`'s window, or `None` if `id` is unknown.
    pub fn window_mut(&mut self, id: WindowId) -> Option<&mut Window> {
        self.windows
            .iter_mut()
            .find(|(wid, _)| *wid == id)
            .map(|(_, w)| w)
    }

    /// Repositions the desktop (the shell calls this as the terminal resizes).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    /// Moves `id` to the top of the stack and makes it active, deactivating
    /// whichever window was previously active. No-op if `id` is unknown.
    fn raise(&mut self, id: WindowId) {
        let Some(pos) = self.windows.iter().position(|(wid, _)| *wid == id) else {
            return;
        };
        let (_, mut window) = self.windows.remove(pos);
        window.set_active(true);
        self.windows.push((id, window));
        if let Some(previous) = self.active {
            if previous != id {
                if let Some((_, w)) = self.windows.iter_mut().find(|(wid, _)| *wid == previous) {
                    w.set_active(false);
                }
            }
        }
        self.active = Some(id);
    }

    /// Reassigns `active` to the topmost visible window in stack order (or
    /// `None`) without moving anything — the next-topmost-visible window is
    /// already correctly positioned. Used by [`close`](Self::close)/
    /// [`hide`](Self::hide), which have already cleared or removed the
    /// previous active window's own flag.
    fn activate_topmost_visible(&mut self) {
        let next = self
            .windows
            .iter()
            .rev()
            .find(|(_, w)| w.is_visible())
            .map(|(id, _)| *id);
        if let Some(id) = next {
            if let Some((_, w)) = self.windows.iter_mut().find(|(wid, _)| *wid == id) {
                w.set_active(true);
            }
        }
        self.active = next;
    }

    /// The topmost visible window whose bounds contain `pos`, if any.
    fn topmost_visible_at(&self, pos: Point) -> Option<WindowId> {
        self.windows
            .iter()
            .rev()
            .find(|(_, w)| w.is_visible() && w.bounds().contains(pos))
            .map(|(id, _)| *id)
    }

    /// Starts a move or resize session if `pos` (screen-absolute, same space
    /// as `id`'s own bounds) is a valid grab point: the title bar (row 0),
    /// clear of any drawn close/zoom/help glyph (ADR 0021), for a move; the
    /// bottom-right corner for a resize — [`arrange::chrome_hit`]'s
    /// classification (ADR 0033). Returns whether a session was started — a
    /// `false` return leaves the click to be forwarded into the window as
    /// usual.
    fn start_session_if_applicable(&mut self, id: WindowId, pos: Point) -> bool {
        let Some(window) = self.window(id) else {
            return false;
        };
        let bounds = window.bounds();
        let flags = ChromeFlags {
            moveable: window.is_moveable(),
            resizable: window.is_resizable(),
            closable: window.is_closable(),
            zoomable: window.is_zoomable(),
            has_help: window.help_topic().is_some(),
        };
        let kind = match arrange::chrome_hit(bounds, pos, flags) {
            ChromeHit::Move => arrange::ArrangeKind::Move,
            ChromeHit::Resize => arrange::ArrangeKind::Resize,
            ChromeHit::Close | ChromeHit::Zoom | ChromeHit::Help | ChromeHit::None => {
                return false;
            }
        };
        self.drag = Some(DragSession {
            window: id,
            session: arrange::start_session(kind, bounds, pos),
        });
        true
    }

    /// Applies the in-progress session's window to `pos`'s movement since the
    /// anchor. No-op if no session is active.
    fn continue_drag(&mut self, pos: Point) {
        let Some(session) = self.drag.take() else {
            return;
        };
        let new_bounds = arrange::continue_session(&session.session, pos, MIN_SIZE);
        if let Some(window) = self.window_mut(session.window) {
            window.set_bounds(new_bounds);
        }
        self.drag = Some(session);
    }

    /// Translates `mouse` into `id`'s local coordinates and forwards it,
    /// restoring the offset afterward (so a nested `open_context_menu`
    /// resolves correctly) — the shared tail of ordinary positional dispatch
    /// and of forwarding a captured drag straight through (ADR 0027), which
    /// skips the positional lookup this would otherwise be part of.
    fn dispatch_to_window(
        &mut self,
        id: WindowId,
        mouse: MouseEvent,
        ctx: &mut Context,
    ) -> EventResult {
        let Some(window) = self.window_mut(id) else {
            return EventResult::Ignored;
        };
        let origin = window.bounds().origin();
        let translated = MouseEvent {
            pos: mouse.pos.offset(-origin.x, -origin.y),
            ..mouse
        };
        ctx.translated(origin.x, origin.y, |ctx| {
            window.handle_event(&Event::Mouse(translated), ctx)
        })
    }

    /// Forwards a `Key`/`Paste` event straight to the active window, if any
    /// (ADR 0009's "focused" pass) — the shared tail `handle_event` builds
    /// its `Key` fallback to the accelerator table (ADR 0028) on top of.
    fn dispatch_to_active(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match self.active {
            Some(id) => match self.window_mut(id) {
                Some(window) => window.handle_event(event, ctx),
                None => EventResult::Ignored,
            },
            None => EventResult::Ignored,
        }
    }

    /// Positional dispatch, click-to-front, and drag/resize/capture session
    /// handling for a mouse event (ADR 0016, ADR 0027).
    fn handle_mouse(&mut self, mouse: MouseEvent, ctx: &mut Context) -> EventResult {
        // A view somewhere in this window's tree asked to keep receiving
        // every event regardless of where the pointer wanders (ADR 0027) —
        // takes priority over ordinary positional dispatch entirely, the
        // same way a `Move`/`Resize` session below does.
        if let Some(id) = self.captured {
            let result = self.dispatch_to_window(id, mouse, ctx);
            if matches!(mouse.kind, MouseKind::Up(_)) {
                self.captured = None;
            }
            return result;
        }

        if self.drag.is_some() {
            return match mouse.kind {
                MouseKind::Drag(MouseButton::Left) => {
                    self.continue_drag(mouse.pos);
                    EventResult::Consumed
                }
                MouseKind::Up(MouseButton::Left) => {
                    self.drag = None;
                    EventResult::Consumed
                }
                // No multi-touch to arbitrate: swallow everything else too,
                // rather than let it leak through to a window mid-session.
                _ => EventResult::Consumed,
            };
        }

        let Some(id) = self.topmost_visible_at(mouse.pos) else {
            return EventResult::Ignored;
        };

        // Click-to-front: any Down raises and activates the window it landed
        // on, before the click is otherwise acted on (ADR 0016).
        if matches!(mouse.kind, MouseKind::Down(_)) {
            self.raise(id);
        }

        if self.window(id).is_none() {
            return EventResult::Ignored;
        }

        if matches!(mouse.kind, MouseKind::Down(MouseButton::Left))
            && self.start_session_if_applicable(id, mouse.pos)
        {
            return EventResult::Consumed;
        }

        let result = self.dispatch_to_window(id, mouse, ctx);
        if ctx.take_mouse_capture_request() {
            self.captured = Some(id);
        }
        result
    }

    /// Command interception: `CM_CLOSE`/`CM_ZOOM`/`CM_NEXT`/`CM_PREV` act
    /// here rather than falling straight through to the active window
    /// (ADR 0016); anything else still does.
    fn handle_command(&mut self, command: Command, ctx: &mut Context) -> EventResult {
        if command == CM_NEXT {
            self.cycle_focus(true);
            return EventResult::Consumed;
        }
        if command == CM_PREV {
            self.cycle_focus(false);
            return EventResult::Consumed;
        }
        let Some(active) = self.active else {
            return EventResult::Ignored;
        };
        if command == CM_CLOSE {
            if !self.window(active).is_some_and(Window::is_closable) {
                return EventResult::Ignored;
            }
            self.close(active, ctx);
            return EventResult::Consumed;
        }
        if command == CM_ZOOM {
            if !self.window(active).is_some_and(Window::is_zoomable) {
                return EventResult::Ignored;
            }
            // `self.bounds` is parent-relative (ADR 0008) — a zoomed window
            // must fill the desktop's own *local* frame instead, since that's
            // the space window bounds are drawn/hit-tested in (`Shell`
            // already translates by `self.bounds`'s own origin before
            // handing events to `Desktop`). Reusing `self.bounds` directly
            // here would double-apply that offset.
            let local_bounds = Rect::from_origin_size(Point::new(0, 0), self.bounds.size());
            if let Some(window) = self.window_mut(active) {
                window.toggle_zoom(local_bounds);
            }
            return EventResult::Consumed;
        }
        match self.window_mut(active) {
            Some(window) => window.handle_event(&Event::Command(command), ctx),
            None => EventResult::Ignored,
        }
    }
}

impl View for Desktop {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        canvas.fill(area, &self.backdrop);
        for (_, window) in &self.windows {
            if !window.is_visible() {
                continue;
            }
            // The window casts its drop shadow on the backdrop (or a lower
            // window) before it is drawn on top of that shadow (ADR 0011).
            if let Some(style) = window.drop_shadow() {
                canvas.shadow(window.bounds(), style);
            }
            let mut sub = canvas.child(window.bounds());
            window.draw(&mut sub);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Mouse(mouse) => self.handle_mouse(*mouse, ctx),
            Event::Command(command) => self.handle_command(*command, ctx),
            // A key the active window declines falls back to the global
            // accelerator table (ADR 0028) before bubbling out any further —
            // the active window's own handling always gets first refusal, so
            // an accelerator never hijacks a keystroke a control already
            // means something by.
            Event::Key(key) => self.dispatch_to_active(event, ctx).or_else(|| {
                match self.accelerators.resolve(key) {
                    Some(command) => {
                        ctx.post(command);
                        EventResult::Consumed
                    }
                    None => EventResult::Ignored,
                }
            }),
            // Paste carries no KeyEvent, so it can't resolve against the
            // accelerator table — just the active window, as before.
            Event::Paste(_) => self.dispatch_to_active(event, ctx),
            // Broadcast / resize / idle: every window, hidden or not, so a
            // hidden window's state stays current for when it's shown again.
            Event::Broadcast(_) | Event::Resize(_) | Event::Idle => {
                for (_, window) in &mut self.windows {
                    window.handle_event(event, ctx);
                }
                EventResult::Ignored
            }
        }
    }

    fn focusable(&self) -> bool {
        self.active.is_some()
    }

    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
        // Every window is asked, not just the active one — a non-short-
        // circuiting fold, like Group's own fan-out, so several unsaved
        // windows can each post their own follow-up in one pass (ADR 0016).
        self.windows
            .iter_mut()
            .fold(true, |ok, (_, w)| w.valid(command, ctx) && ok)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::color::Style;
    use crate::command::{CM_HELP, CM_OK, CM_USER, CommandSet};
    use crate::event::{KeyCode, KeyEvent, Modifiers, MouseButton, MouseKind};
    use crate::geometry::Size;
    use crate::theme::{Role, Theme};
    use crate::view::{AxisMetrics, ScrollMetrics, StaticText};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    fn blank() -> Box<dyn View> {
        Box::new(StaticText::new(rect(0, 0, 1, 1), "", Style::new()))
    }

    fn blank_window_at(bounds: Rect) -> Window {
        Window::new(bounds, "W", &Theme::default(), blank())
    }

    /// An interior that records every event it sees and posts a command on Enter.
    struct Recorder {
        tag: u16,
        log: Rc<RefCell<Vec<(u16, Event)>>>,
        command: Command,
    }

    impl View for Recorder {
        fn bounds(&self) -> Rect {
            rect(0, 0, 100, 100)
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            self.log.borrow_mut().push((self.tag, event.clone()));
            if matches!(event, Event::Key(k) if k.code == KeyCode::Enter) {
                ctx.post(self.command);
                return EventResult::Consumed;
            }
            EventResult::Ignored
        }
    }

    fn recorder_window(tag: u16, bounds: Rect, log: &Rc<RefCell<Vec<(u16, Event)>>>) -> Window {
        Window::new(
            bounds,
            "W",
            &Theme::default(),
            Box::new(Recorder {
                tag,
                log: Rc::clone(log),
                command: CM_OK,
            }),
        )
    }

    // --- Basic rendering/dispatch (pre-existing behaviour, adapted to the
    // dynamic open() API) ---

    #[test]
    fn empty_desktop_just_paints_the_backdrop() {
        let desk = Desktop::new(rect(0, 0, 3, 2), Cell::from_char('░', Style::new()));
        assert_eq!(desk.active_id(), None);
        assert!(!desk.focusable());
        let mut buf = Buffer::new(Size::new(3, 2));
        let mut canvas = Canvas::new(&mut buf);
        desk.draw(&mut canvas);
        assert_eq!(buf.to_text(), "░░░\n░░░");
    }

    #[test]
    fn a_window_casts_its_drop_shadow_on_the_backdrop() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let shadow = Theme::default().style(Role::Shadow);
        let mut desk = Desktop::new(rect(0, 0, 20, 10), Cell::from_char('░', Style::new()));
        desk.open(recorder_window(1, rect(2, 1, 8, 4), &log));
        let mut buf = Buffer::new(Size::new(20, 10));
        let mut canvas = Canvas::new(&mut buf);
        desk.draw(&mut canvas);

        let shadowed = buf.get(Point::new(10, 2)).unwrap();
        assert_eq!(shadowed.grapheme().to_string(), "░");
        assert_eq!(shadowed.style(), shadow);
        assert_eq!(buf.get(Point::new(0, 9)).unwrap().style(), Style::new());
    }

    #[test]
    fn topmost_window_is_active() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(recorder_window(1, rect(0, 0, 10, 5), &log));
        let b = desk.open(recorder_window(2, rect(5, 2, 10, 5), &log));
        assert_eq!(desk.active_id(), Some(b));
    }

    #[test]
    fn keys_reach_only_the_active_window() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(recorder_window(1, rect(0, 0, 10, 5), &log));
        desk.open(recorder_window(2, rect(12, 0, 10, 5), &log));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        desk.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('a'), Modifiers::NONE)),
            &mut ctx,
        );
        let seen = log.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, 2, "only the active (top) window saw the key");
    }

    #[test]
    fn a_command_from_the_active_window_bubbles_to_the_context() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(recorder_window(1, rect(0, 0, 10, 5), &log));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        desk.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
    }

    // --- Global accelerator table (ADR 0028) ---

    #[test]
    fn an_unclaimed_key_resolves_a_bound_accelerator_and_posts_its_command() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(recorder_window(1, rect(0, 0, 10, 5), &log));
        desk.bind_accelerator(Accelerator::new(
            KeyEvent::new(KeyCode::Char('o'), Modifiers::CONTROL),
            CM_HELP,
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let result = desk.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('o'), Modifiers::CONTROL)),
            &mut ctx,
        );
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_HELP)]);
    }

    #[test]
    fn the_active_windows_own_key_handling_wins_over_a_bound_accelerator() {
        // Recorder claims Enter itself (posting CM_OK); an accelerator also
        // bound to Enter must never get a look-in.
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(recorder_window(1, rect(0, 0, 10, 5), &log));
        desk.bind_accelerator(Accelerator::new(
            KeyEvent::new(KeyCode::Enter, Modifiers::NONE),
            CM_HELP,
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        desk.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(
            ctx.posted(),
            &[Event::Command(CM_OK)],
            "the window's own claim wins; the accelerator never fires"
        );
    }

    #[test]
    fn an_accelerator_fires_even_with_no_active_window() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.bind_accelerator(Accelerator::new(
            KeyEvent::new(KeyCode::Char('o'), Modifiers::CONTROL),
            CM_HELP,
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let result = desk.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('o'), Modifiers::CONTROL)),
            &mut ctx,
        );
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_HELP)]);
    }

    #[test]
    fn an_unbound_key_with_no_active_claim_bubbles_ignored() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(recorder_window(1, rect(0, 0, 10, 5), &log));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let result = desk.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('z'), Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(result, EventResult::Ignored);
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn a_disabled_accelerator_command_still_consumes_the_key_but_posts_nothing() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.bind_accelerator(Accelerator::new(
            KeyEvent::new(KeyCode::Char('o'), Modifiers::CONTROL),
            CM_HELP,
        ));
        let mut cs = CommandSet::new();
        cs.disable(CM_HELP);
        let mut ctx = Context::new(&cs);
        let result = desk.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('o'), Modifiers::CONTROL)),
            &mut ctx,
        );
        assert_eq!(result, EventResult::Consumed, "the key is still swallowed");
        assert!(
            ctx.posted().is_empty(),
            "but the disabled command never fires"
        );
    }

    #[test]
    fn a_click_goes_to_the_topmost_window_under_it() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(recorder_window(1, rect(0, 0, 12, 6), &log));
        desk.open(recorder_window(2, rect(3, 2, 12, 6), &log));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        desk.handle_event(
            &Event::Mouse(MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                pos: Point::new(5, 4),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        );
        let seen = log.borrow();
        assert_eq!(seen.len(), 1);
        assert_eq!(seen[0].0, 2);
    }

    // --- Dynamic open/close/hide/show/focus/cycle_focus (ADR 0016) ---

    #[test]
    fn open_returns_distinct_ids_and_activates_each_in_turn() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let a = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        assert_eq!(desk.active_id(), Some(a));
        let b = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        assert_ne!(a, b);
        assert_eq!(desk.active_id(), Some(b));
    }

    #[test]
    fn close_on_a_refusing_window_is_a_no_op() {
        struct Vetoer;
        impl View for Vetoer {
            fn bounds(&self) -> Rect {
                rect(0, 0, 5, 1)
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn valid(&mut self, command: Command, _ctx: &mut Context) -> bool {
                command != CM_CLOSE
            }
        }
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let id = desk.open(Window::new(
            rect(0, 0, 10, 4),
            "W",
            &Theme::default(),
            Box::new(Vetoer),
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(desk.close(id, &mut ctx).is_none());
        assert!(desk.window(id).is_some());
    }

    #[test]
    fn close_on_the_active_window_transfers_active_to_the_next_visible_window() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let a = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        let b = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        assert_eq!(desk.active_id(), Some(b));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(desk.close(b, &mut ctx).is_some());
        assert_eq!(desk.active_id(), Some(a));
        assert!(desk.window(b).is_none());
    }

    #[test]
    fn hide_reassigns_active_and_show_restores_it_on_top() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let a = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        let b = desk.open(blank_window_at(rect(0, 0, 10, 4)));

        desk.hide(b);
        assert!(!desk.window(b).unwrap().is_visible());
        assert_eq!(desk.active_id(), Some(a));

        desk.show(b);
        assert!(desk.window(b).unwrap().is_visible());
        assert_eq!(desk.active_id(), Some(b));
    }

    #[test]
    fn unknown_window_id_is_a_safe_no_op_everywhere() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let real = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        let fake = WindowId(9999);

        desk.hide(fake);
        desk.show(fake);
        desk.focus(fake);
        assert!(desk.window(fake).is_none());
        assert!(desk.window_mut(fake).is_none());

        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(desk.close(fake, &mut ctx).is_none());
        assert_eq!(desk.active_id(), Some(real));
    }

    #[test]
    fn cascade_positions_visible_windows_per_arrange_cascade_slot() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let a = desk.open(blank_window_at(rect(20, 8, 10, 4)));
        let b = desk.open(blank_window_at(rect(0, 0, 10, 4)));

        desk.cascade();

        assert_eq!(
            desk.window(a).unwrap().bounds(),
            crate::arrange::cascade_slot(Size::new(40, 12), 0, Size::new(10, 3))
        );
        assert_eq!(
            desk.window(b).unwrap().bounds(),
            crate::arrange::cascade_slot(Size::new(40, 12), 1, Size::new(10, 3))
        );
        // Z-order/active are untouched by a pure re-layout.
        assert_eq!(desk.active_id(), Some(b));
    }

    #[test]
    fn cascade_skips_hidden_and_maximized_windows() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let visible = desk.open(blank_window_at(rect(5, 5, 10, 4)));
        let hidden = desk.open(blank_window_at(rect(1, 1, 10, 4)));
        desk.hide(hidden);
        let maximized = desk.open(blank_window_at(rect(2, 2, 10, 4)));
        desk.window_mut(maximized)
            .unwrap()
            .toggle_zoom(rect(0, 0, 40, 12));

        desk.cascade();

        // The only eligible window lands at cascade slot 0 (the desktop
        // itself), not shifted by the hidden/maximized ones it skipped.
        assert_eq!(
            desk.window(visible).unwrap().bounds(),
            crate::arrange::cascade_slot(Size::new(40, 12), 0, Size::new(10, 3))
        );
        assert_eq!(desk.window(hidden).unwrap().bounds(), rect(1, 1, 10, 4));
        assert!(desk.window(maximized).unwrap().is_maximized());
        assert_eq!(desk.window(maximized).unwrap().bounds(), rect(0, 0, 40, 12));
    }

    #[test]
    fn tile_lays_out_visible_windows_per_arrange_tile() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let a = desk.open(blank_window_at(rect(20, 8, 10, 4)));
        let b = desk.open(blank_window_at(rect(0, 0, 10, 4)));

        desk.tile();

        let slots = crate::arrange::tile(Size::new(40, 12), 2);
        assert_eq!(desk.window(a).unwrap().bounds(), slots[0]);
        assert_eq!(desk.window(b).unwrap().bounds(), slots[1]);
        assert_eq!(desk.active_id(), Some(b));
    }

    #[test]
    fn tile_skips_hidden_and_maximized_windows() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let visible = desk.open(blank_window_at(rect(5, 5, 10, 4)));
        let hidden = desk.open(blank_window_at(rect(1, 1, 10, 4)));
        desk.hide(hidden);
        let maximized = desk.open(blank_window_at(rect(2, 2, 10, 4)));
        desk.window_mut(maximized)
            .unwrap()
            .toggle_zoom(rect(0, 0, 40, 12));

        desk.tile();

        assert_eq!(desk.window(visible).unwrap().bounds(), rect(0, 0, 40, 12));
        assert_eq!(desk.window(hidden).unwrap().bounds(), rect(1, 1, 10, 4));
        assert!(desk.window(maximized).unwrap().is_maximized());
    }

    #[test]
    fn focus_changes_which_overlapping_window_is_on_top() {
        let log = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let a = desk.open(recorder_window(1, rect(0, 0, 12, 6), &log));
        desk.open(recorder_window(2, rect(3, 2, 12, 6), &log));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(5, 4),
            modifiers: Modifiers::NONE,
        });

        desk.handle_event(&click, &mut ctx);
        assert_eq!(log.borrow().last().unwrap().0, 2, "b is on top by default");

        desk.focus(a);
        desk.handle_event(&click, &mut ctx);
        assert_eq!(
            log.borrow().last().unwrap().0,
            1,
            "focusing a brings it to the top"
        );
    }

    // --- Command interception (ADR 0016) ---

    #[test]
    fn cm_close_closes_the_active_window_when_closable() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let id = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            desk.handle_event(&Event::Command(CM_CLOSE), &mut ctx),
            EventResult::Consumed
        );
        assert!(desk.window(id).is_none());
        assert_eq!(desk.active_id(), None);
    }

    #[test]
    fn cm_close_is_ignored_when_the_active_window_is_not_closable() {
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        let id = desk
            .open(Window::new(rect(0, 0, 10, 4), "W", &Theme::default(), blank()).closable(false));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            desk.handle_event(&Event::Command(CM_CLOSE), &mut ctx),
            EventResult::Ignored
        );
        assert!(desk.window(id).is_some());
    }

    #[test]
    fn cm_zoom_toggles_the_active_window_when_zoomable() {
        let desktop_bounds = rect(0, 0, 40, 20);
        let mut desk = Desktop::new(desktop_bounds, Cell::default());
        let id = desk.open(blank_window_at(rect(2, 1, 10, 5)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert_eq!(
            desk.handle_event(&Event::Command(CM_ZOOM), &mut ctx),
            EventResult::Consumed
        );
        assert!(desk.window(id).unwrap().is_maximized());
        assert_eq!(desk.window(id).unwrap().bounds(), desktop_bounds);

        desk.handle_event(&Event::Command(CM_ZOOM), &mut ctx);
        assert!(!desk.window(id).unwrap().is_maximized());
        assert_eq!(desk.window(id).unwrap().bounds(), rect(2, 1, 10, 5));
    }

    #[test]
    fn cm_zoom_fills_the_desktops_own_local_frame_even_when_the_desktop_itself_is_offset() {
        // A `Desktop` hosted in a real `Shell` always sits at a non-zero
        // origin (below the menu bar) — `Desktop::bounds()` is parent-relative
        // (ADR 0008), but window bounds are desktop-*local*: `Shell` already
        // translates the canvas/mouse position by that offset before handing
        // events to `Desktop`. `CM_ZOOM` must fill the desktop's own local
        // frame (origin (0, 0), matching its own size) rather than carrying
        // its parent-relative origin over onto the window — otherwise the
        // zoomed window ends up shifted by that same offset a second time,
        // uncovering rows at the top and overflowing past the bottom.
        let mut desk = Desktop::new(rect(0, 1, 40, 20), Cell::default());
        let id = desk.open(blank_window_at(rect(2, 1, 10, 5)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        desk.handle_event(&Event::Command(CM_ZOOM), &mut ctx);

        assert_eq!(desk.window(id).unwrap().bounds(), rect(0, 0, 40, 20));
    }

    #[test]
    fn cm_zoom_is_ignored_when_the_active_window_is_not_zoomable() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk
            .open(Window::new(rect(2, 1, 10, 5), "W", &Theme::default(), blank()).zoomable(false));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            desk.handle_event(&Event::Command(CM_ZOOM), &mut ctx),
            EventResult::Ignored
        );
        assert!(!desk.window(id).unwrap().is_maximized());
    }

    #[test]
    fn cm_next_and_cm_prev_cycle_focus_and_raise() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let a = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        desk.open(blank_window_at(rect(0, 0, 10, 4))); // b
        let c = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        assert_eq!(desk.active_id(), Some(c));

        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            desk.handle_event(&Event::Command(CM_NEXT), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(
            desk.active_id(),
            Some(a),
            "wraps from the top back to the bottom"
        );

        assert_eq!(
            desk.handle_event(&Event::Command(CM_PREV), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(
            desk.active_id(),
            Some(c),
            "steps back one in the stack order left after raising a"
        );
    }

    #[test]
    fn cm_next_is_a_safe_no_op_with_fewer_than_two_visible_windows() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk.open(blank_window_at(rect(0, 0, 10, 4)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(
            desk.handle_event(&Event::Command(CM_NEXT), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(desk.active_id(), Some(id));
    }

    // --- valid() fans out to every window (ADR 0016) ---

    #[test]
    fn valid_polls_every_window_and_a_single_refusal_vetoes() {
        struct ValidSpy {
            tag: u16,
            refuses: bool,
            seen: Rc<RefCell<Vec<u16>>>,
        }
        impl View for ValidSpy {
            fn bounds(&self) -> Rect {
                rect(0, 0, 5, 1)
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn valid(&mut self, _command: Command, _ctx: &mut Context) -> bool {
                self.seen.borrow_mut().push(self.tag);
                !self.refuses
            }
        }
        const CM_APPLY: Command = Command(CM_USER + 1);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 12), Cell::default());
        desk.open(Window::new(
            rect(0, 0, 10, 5),
            "A",
            &Theme::default(),
            Box::new(ValidSpy {
                tag: 1,
                refuses: true,
                seen: Rc::clone(&seen),
            }),
        ));
        desk.open(Window::new(
            rect(15, 0, 10, 5),
            "B",
            &Theme::default(),
            Box::new(ValidSpy {
                tag: 2,
                refuses: false,
                seen: Rc::clone(&seen),
            }),
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!desk.valid(CM_APPLY, &mut ctx));
        assert_eq!(
            *seen.borrow(),
            vec![1, 2],
            "both windows were asked, not short-circuited"
        );
    }

    // --- Rendering: hidden windows are skipped entirely (ADR 0016) ---

    #[test]
    fn a_hidden_window_is_not_drawn() {
        let mut desk = Desktop::new(rect(0, 0, 10, 4), Cell::from_char('.', Style::new()));
        let id = desk.open(Window::new(
            rect(0, 0, 10, 4),
            "W",
            &Theme::default(),
            Box::new(StaticText::new(rect(0, 0, 5, 1), "hi", Style::new())),
        ));
        desk.hide(id);
        let mut buf = Buffer::new(Size::new(10, 4));
        let mut canvas = Canvas::new(&mut buf);
        desk.draw(&mut canvas);
        assert!(
            buf.to_text()
                .chars()
                .filter(|c| *c != '\n')
                .all(|c| c == '.'),
            "a hidden window draws no chrome, shadow, or interior"
        );
    }

    // --- Drag/resize sessions (ADR 0016) ---

    #[test]
    fn title_bar_drag_moves_a_moveable_window() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk.open(blank_window_at(rect(2, 1, 10, 5)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        // Window-local (5, 0): plain title bar, clear of the close/zoom
        // glyph spans on a 10-wide frame (close: 2..5, zoom: 6..9).
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(7, 1),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&down, &mut ctx), EventResult::Consumed);
        assert_eq!(
            desk.window(id).unwrap().bounds(),
            rect(2, 1, 10, 5),
            "no move yet, just anchored"
        );

        let drag = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(10, 3),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&drag, &mut ctx), EventResult::Consumed);
        assert_eq!(desk.window(id).unwrap().bounds(), rect(5, 3, 10, 5));

        let up = Event::Mouse(MouseEvent {
            kind: MouseKind::Up(MouseButton::Left),
            pos: Point::new(10, 3),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&up, &mut ctx), EventResult::Consumed);

        // The session has ended: a stray drag with no preceding Down does nothing.
        let stray = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(20, 10),
            modifiers: Modifiers::NONE,
        });
        desk.handle_event(&stray, &mut ctx);
        assert_eq!(desk.window(id).unwrap().bounds(), rect(5, 3, 10, 5));
    }

    #[test]
    fn title_bar_drag_is_a_no_op_on_a_non_moveable_window() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk
            .open(Window::new(rect(2, 1, 10, 5), "W", &Theme::default(), blank()).moveable(false));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(7, 1),
            modifiers: Modifiers::NONE,
        });
        // Forwarded into the window, which ignores a bare title-bar click
        // (its own test coverage) since no session was started.
        assert_eq!(desk.handle_event(&down, &mut ctx), EventResult::Ignored);
        let drag = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(10, 3),
            modifiers: Modifiers::NONE,
        });
        desk.handle_event(&drag, &mut ctx);
        assert_eq!(desk.window(id).unwrap().bounds(), rect(2, 1, 10, 5));
    }

    #[test]
    fn corner_drag_resizes_a_resizable_window() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk.open(blank_window_at(rect(2, 1, 10, 5)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // Window-local (9, 4): bottom-right corner.
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(11, 5),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&down, &mut ctx), EventResult::Consumed);
        let drag = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(14, 7),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&drag, &mut ctx), EventResult::Consumed);
        assert_eq!(desk.window(id).unwrap().bounds(), rect(2, 1, 13, 7));
    }

    #[test]
    fn corner_drag_is_a_no_op_on_a_non_resizable_window() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk
            .open(Window::new(rect(2, 1, 10, 5), "W", &Theme::default(), blank()).resizable(false));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(11, 5),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&down, &mut ctx), EventResult::Ignored);
        let drag = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(14, 7),
            modifiers: Modifiers::NONE,
        });
        desk.handle_event(&drag, &mut ctx);
        assert_eq!(desk.window(id).unwrap().bounds(), rect(2, 1, 10, 5));
    }

    #[test]
    fn resize_drag_floors_at_the_minimum_size() {
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk.open(blank_window_at(rect(2, 1, 10, 5)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(11, 5),
            modifiers: Modifiers::NONE,
        });
        desk.handle_event(&down, &mut ctx);
        let drag = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(-20, -20),
            modifiers: Modifiers::NONE,
        });
        desk.handle_event(&drag, &mut ctx);
        let bounds = desk.window(id).unwrap().bounds();
        assert_eq!(bounds.size(), MIN_SIZE);
        assert_eq!(bounds.origin(), Point::new(2, 1));
    }

    // --- Scroll-bar thumb dragging via generic mouse capture (ADR 0027) ---

    /// An interior that reports a fixed vertical overflow and records every
    /// offset pushed to it — mirrors `window.rs`'s own private test double of
    /// the same shape (not shared across files, test-only code).
    struct Scrollable {
        metrics: ScrollMetrics,
        pushed: Rc<RefCell<Vec<Point>>>,
    }

    impl View for Scrollable {
        fn bounds(&self) -> Rect {
            rect(0, 0, 100, 100)
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn scroll_metrics(&self) -> Option<ScrollMetrics> {
            Some(self.metrics)
        }
        fn set_scroll(&mut self, offset: Point) {
            self.pushed.borrow_mut().push(offset);
        }
    }

    fn vertical_metrics(total: usize, visible: usize, pos: usize) -> ScrollMetrics {
        ScrollMetrics {
            horizontal: None,
            vertical: Some(AxisMetrics {
                total,
                visible,
                pos,
            }),
        }
    }

    #[test]
    fn thumb_drag_moves_the_scroll_position_even_outside_the_windows_bounds() {
        let pushed = Rc::new(RefCell::new(Vec::new()));
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        desk.open(Window::new(
            rect(2, 1, 12, 8),
            "W",
            &Theme::default(),
            Box::new(Scrollable {
                metrics: vertical_metrics(20, 6, 0),
                pushed: Rc::clone(&pushed),
            }),
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        // Window-local (11, 2): the vertical bar's thumb, one row under the
        // up arrow at scroll pos 0 (bar spans window-local rows 1..7, column
        // 11 on a 12-wide/8-tall window).
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(13, 3),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&down, &mut ctx), EventResult::Consumed);
        assert!(
            pushed.borrow().is_empty(),
            "anchors the drag, no scroll yet"
        );

        // Far outside the window's own bounds entirely (the window spans
        // absolute rows 1..9) — proving this is a real Desktop-level mouse
        // capture, not ordinary positional dispatch, which would have
        // ignored a point outside every window.
        let drag = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(13, 15),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&drag, &mut ctx), EventResult::Consumed);
        assert_eq!(
            *pushed.borrow(),
            vec![Point::new(0, 14)],
            "dragging far past the track clamps to the maximum scroll position"
        );

        let up = Event::Mouse(MouseEvent {
            kind: MouseKind::Up(MouseButton::Left),
            pos: Point::new(13, 15),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&up, &mut ctx), EventResult::Consumed);

        // The capture has ended: a stray drag with no preceding Down does nothing.
        let stray = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(13, 3),
            modifiers: Modifiers::NONE,
        });
        desk.handle_event(&stray, &mut ctx);
        assert_eq!(pushed.borrow().len(), 1, "no further pushes after Up");
    }

    #[test]
    fn a_click_on_a_glyph_does_not_start_a_move_session() {
        // Row 0, but on the close glyph: raises the window and forwards the
        // click into it (which itself posts CM_CLOSE) rather than starting a
        // move session — Window handles its own glyphs (ADR 0016).
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk.open(blank_window_at(rect(2, 1, 10, 5)));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let close_x = Frame::close_span(10, false).unwrap().start;
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(2 + close_x, 1),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&down, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_CLOSE)]);
        assert!(
            desk.window(id).unwrap().bounds() == rect(2, 1, 10, 5),
            "the click acted on the glyph, not a drag"
        );
    }

    #[test]
    fn a_click_on_the_help_glyph_does_not_start_a_move_session_either() {
        // Same as the close-glyph case above, but for the new ADR 0021 help
        // glyph: a width-13 frame is the narrowest that still shows all three
        // glyphs (Frame's all-or-nothing gate for a help-enabled window).
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        let id = desk.open(
            Window::new(rect(2, 1, 13, 5), "W", &Theme::default(), blank())
                .with_help_topic("intro"),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let help_x = Frame::help_span(13).unwrap().start;
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(2 + help_x, 1),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(desk.handle_event(&down, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_HELP)]);
        assert!(
            desk.window(id).unwrap().bounds() == rect(2, 1, 13, 5),
            "the click acted on the glyph, not a drag"
        );
    }

    // --- Context menu anchor propagation (ADR 0019) ---

    /// An interior that offers a context menu anchored at its own local
    /// right-click position.
    struct Offerer;

    impl View for Offerer {
        fn bounds(&self) -> Rect {
            rect(0, 0, 100, 100)
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            if let Event::Mouse(mouse) = event {
                if mouse.kind == MouseKind::Down(MouseButton::Right) {
                    ctx.open_context_menu(crate::widgets::Menu::new("M", vec![]), mouse.pos);
                    return EventResult::Consumed;
                }
            }
            EventResult::Ignored
        }
    }

    #[test]
    fn a_context_menu_request_from_a_window_interior_resolves_to_desktop_coordinates() {
        // The window sits at (5, 2), sized 10x5; its interior (inset one cell
        // for the border on each side) spans desktop-absolute (6, 3)..(14, 6).
        // A right-click at desktop (10, 4) lands inside it, at interior-local
        // (4, 1); Desktop must translate the request back out to desktop
        // coordinates (10, 4), proving the offset composes correctly across a
        // real Window, not just a synthetic offset.
        let mut desk = Desktop::new(rect(0, 0, 40, 20), Cell::default());
        desk.open(Window::new(
            rect(5, 2, 10, 5),
            "W",
            &Theme::default(),
            Box::new(Offerer),
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let right_click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Right),
            pos: Point::new(10, 4),
            modifiers: Modifiers::NONE,
        });
        desk.handle_event(&right_click, &mut ctx);
        let req = ctx.take_context_menu_request().unwrap();
        assert_eq!(req.at, Point::new(10, 4));
    }
}
