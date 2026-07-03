//! A framed window: a [`Frame`] around an interior [`View`] (ADR 0016).
//!
//! `Window` is the one bordered-box-with-an-interior type in the framework —
//! what used to be split between `Window` (desktop-resident, no policy) and
//! `Dialog` (modal-only, ran via [`Application::exec_view`](crate::app::Application::exec_view))
//! is now one type carrying optional policy: `resizable`/`moveable`/
//! `closable`/`zoomable` flags, a [`Placement`], which posted commands end a
//! modal run (`ending`/`ends_on`), a fallback for `Enter` (`default_cmd`),
//! and whether `Esc` cancels. A "dialog" is just a `Window` configured with
//! that policy — see [`MessageBox`](super::MessageBox) and
//! [`FileDialog`](super::FileDialog).
//!
//! `Window` itself has no opinion about *how* it is run: [`Desktop`](super::Desktop)
//! runs one long-lived, alongside siblings, in the ordinary tree; `exec_view`
//! runs one modally, nested and exclusive. Both just need `&mut Window`.
//!
//! **SDI is still a first-class use.** `resizable`/`moveable`/`closable`/
//! `zoomable` are independent flags, not parts of an MDI/SDI switch — an
//! application that wants exactly one, fixed, undismissable window opens a
//! single `Window` sized to fill the desktop with `.resizable(false).moveable(false)`
//! and never opens a second one; `Desktop`'s drag/resize sessions simply never
//! start for it. An application that wants no chrome at all can skip `Window`/
//! `Desktop` entirely and hand `Root::new` a full-screen `View` directly.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::command::{CM_CANCEL, CM_CLOSE, CM_HELP, CM_ZOOM, Command};
use crate::event::{Event, EventResult, KeyCode, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{Frame, ScrollBar, ScrollPart};

/// Where [`exec_view`](crate::app::Application::exec_view) (or [`Desktop::open`](super::Desktop::open))
/// positions a window at the start of its run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Placement {
    /// Left exactly where its `bounds` say.
    Positioned,
    /// Centred within the area it runs over, keeping only its `bounds`' size.
    Centered,
}

/// A window: a titled frame plus an interior view, optionally configured as a
/// fixed-size, centred, modal-mannered "dialog" (ADR 0016).
pub struct Window {
    bounds: Rect,
    frame: Frame,
    active: bool,
    interior_fill: Cell,
    frame_style: Style,
    shadow_style: Style,
    casts_shadow: bool,
    interior: Box<dyn View>,
    resizable: bool,
    moveable: bool,
    closable: bool,
    zoomable: bool,
    /// The opaque `HelpContents` topic id this window's help glyph/`F1`
    /// targets, if any (ADR 0021) — `None` means no help glyph is drawn and
    /// `F1` falls back to the home topic.
    help_topic: Option<String>,
    placement: Placement,
    ending: Vec<Command>,
    default_cmd: Option<Command>,
    esc_cancels: bool,
    visible: bool,
    maximized: bool,
    /// The bounds and `casts_shadow` setting to restore on the next zoom
    /// toggle, set while `maximized`.
    restore: Option<(Rect, bool)>,
}

impl Window {
    /// Creates a window at `bounds` titled `title`, taking its frame/title colours
    /// from `theme`'s window roles, wrapping `interior`. Inactive until raised;
    /// fully capable by default (resizable, moveable, closable, zoomable,
    /// [`Positioned`](Placement::Positioned), no ending commands, visible) — a
    /// "dialog" opts back out of some of that with the builders below.
    pub fn new(bounds: Rect, title: &str, theme: &Theme, interior: Box<dyn View>) -> Self {
        Self::styled(
            bounds,
            title,
            theme.style(Role::WindowFrame),
            theme.style(Role::WindowTitle),
            theme,
            interior,
        )
    }

    /// Creates a window styled as a dialog: frame, title, and interior
    /// background all resolve [`Role::DialogBackground`] — TurboVision's
    /// separate, distinct dialog palette, rather than the window frame/title
    /// roles [`new`](Self::new) uses. [`MessageBox`](super::MessageBox) and
    /// [`FileDialog`](super::FileDialog) build on this (ADR 0016) so a "dialog"
    /// keeps its classic look even though it's the same `Window` type.
    pub fn dialog(bounds: Rect, title: &str, theme: &Theme, interior: Box<dyn View>) -> Self {
        let style = theme.style(Role::DialogBackground);
        Self::styled(bounds, title, style, style, theme, interior)
    }

    /// The shared constructor behind [`new`](Self::new)/[`dialog`](Self::dialog):
    /// every other field is the same fully-capable default either way.
    fn styled(
        bounds: Rect,
        title: &str,
        frame_style: Style,
        title_style: Style,
        theme: &Theme,
        interior: Box<dyn View>,
    ) -> Self {
        let frame = Frame::new(title, frame_style, title_style);
        Self {
            bounds,
            frame,
            active: false,
            interior_fill: Cell::blank(frame_style),
            frame_style,
            shadow_style: theme.style(Role::Shadow),
            casts_shadow: true,
            interior,
            resizable: true,
            moveable: true,
            closable: true,
            zoomable: true,
            help_topic: None,
            placement: Placement::Positioned,
            ending: Vec::new(),
            default_cmd: None,
            esc_cancels: false,
            visible: true,
            maximized: false,
            restore: None,
        }
    }

    /// Sets whether a title-bar drag can move the window (default `true`).
    pub fn moveable(mut self, yes: bool) -> Self {
        self.moveable = yes;
        self
    }

    /// Sets whether a corner drag can resize the window (default `true`) —
    /// also tells the frame whether to draw the resize-handle affordance in
    /// the bottom-right corner, so a locked-size window doesn't invite a drag
    /// that won't do anything (ADR 0016).
    pub fn resizable(mut self, yes: bool) -> Self {
        self.resizable = yes;
        self.frame.set_resizable(yes);
        self
    }

    /// Sets whether the close glyph/`CM_CLOSE` can close the window (default
    /// `true`) — also tells the frame not to draw the glyph at all when off,
    /// so there's nothing to hit (ADR 0016).
    pub fn closable(mut self, yes: bool) -> Self {
        self.closable = yes;
        self.frame.set_closable(yes);
        self
    }

    /// Sets whether the zoom glyph/`CM_ZOOM` can maximise/restore the window
    /// (default `true`), mirroring [`closable`](Self::closable).
    pub fn zoomable(mut self, yes: bool) -> Self {
        self.zoomable = yes;
        self.frame.set_zoomable(yes);
        self
    }

    /// Gives the window a help topic (ADR 0021): a help glyph appears on the
    /// title bar, immediately left of the zoom glyph, and both it and `F1`
    /// post the existing `CM_HELP` — resolving `topic` into an actual page is
    /// whatever catches `CM_HELP`'s job (see `docs/specs/shell.md`), not this
    /// window's. No help glyph is drawn at all without this (the default).
    pub fn with_help_topic(mut self, topic: impl Into<String>) -> Self {
        self.help_topic = Some(topic.into());
        self.frame.set_help(true);
        self
    }

    /// Marks the window to be centred within the area it runs over
    /// ([`Placement::Centered`]), keeping only its current `bounds`' size.
    pub fn centered(mut self) -> Self {
        self.placement = Placement::Centered;
        self
    }

    /// Sets the default command — what the window posts when `Enter` is
    /// pressed and the focused interior did not consume it.
    pub fn with_default(mut self, command: Command) -> Self {
        self.default_cmd = Some(command);
        self
    }

    /// Registers `command` as also ending a modal run (beyond any already
    /// added), e.g. `CM_YES`/`CM_NO`.
    pub fn also_ends_on(mut self, command: Command) -> Self {
        if !self.ending.contains(&command) {
            self.ending.push(command);
        }
        self
    }

    /// Sets whether `Esc` posts `CM_CANCEL` before the interior sees it
    /// (default `false` — a plain window leaves `Esc` to its interior).
    pub fn esc_cancels(mut self, yes: bool) -> Self {
        self.esc_cancels = yes;
        self
    }

    /// Whether posting `command` should end a modal run
    /// ([`exec_view`](crate::app::Application::exec_view)). Always `false`
    /// for a window with no `ending` commands registered.
    pub fn ends_on(&self, command: Command) -> bool {
        self.ending.contains(&command)
    }

    /// The interior rectangle in the window's **local** coordinates: the whole
    /// window inset by one cell on every side (the border). Collapses to empty for
    /// a window too small to have an interior.
    pub fn interior_bounds(&self) -> Rect {
        let Size { width, height } = self.bounds.size();
        Rect::from_origin_size(
            Point::new(1, 1),
            Size::new((width - 2).max(0), (height - 2).max(0)),
        )
    }

    /// Whether a title-bar drag can move the window — [`Desktop`](super::Desktop)
    /// consults this before starting a move session (ADR 0016).
    pub fn is_moveable(&self) -> bool {
        self.moveable
    }

    /// Whether a corner drag can resize the window, mirroring
    /// [`is_moveable`](Self::is_moveable).
    pub fn is_resizable(&self) -> bool {
        self.resizable
    }

    /// Whether the close glyph/`CM_CLOSE` can close the window — checked by
    /// [`Desktop`](super::Desktop) before acting on `CM_CLOSE`.
    pub fn is_closable(&self) -> bool {
        self.closable
    }

    /// Whether the zoom glyph/`CM_ZOOM` can maximise/restore the window,
    /// mirroring [`is_closable`](Self::is_closable).
    pub fn is_zoomable(&self) -> bool {
        self.zoomable
    }

    /// Whether the window is currently the active (focused) one.
    pub fn is_active(&self) -> bool {
        self.active
    }

    /// The help topic id this window's help glyph/`F1` targets, if any
    /// (ADR 0021) — read by whatever catches `CM_HELP` (see
    /// `docs/specs/shell.md`) to resolve an actual page.
    pub fn help_topic(&self) -> Option<&str> {
        self.help_topic.as_deref()
    }

    /// Marks the window active or not, switching its frame between the doubled
    /// (active) and single (inactive) border.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
        self.frame.set_active(active);
    }

    /// Sets whether this window casts a drop shadow on what lies behind it
    /// (default `true`). Turn it off for a window meant to sit flush — e.g. one
    /// maximised to fill the desktop, whose shadow would only fall off-screen
    /// (ADR 0011).
    pub fn set_casts_shadow(&mut self, casts: bool) {
        self.casts_shadow = casts;
    }

    /// Where [`exec_view`](crate::app::Application::exec_view)/[`Desktop::open`](super::Desktop::open)
    /// should position this window at the start of its run.
    pub fn placement(&self) -> Placement {
        self.placement
    }

    /// Whether the window is currently resident-but-shown. `Window`'s own
    /// `draw`/`handle_event` do not consult this — visibility is
    /// [`Desktop`](super::Desktop)'s concern, and meaningless to a modal run
    /// (shown for the run's duration regardless).
    pub fn is_visible(&self) -> bool {
        self.visible
    }

    /// Marks the window hidden — resident but not drawn or dispatched to by a
    /// [`Desktop`](super::Desktop) (TurboVision's `TView::hide`, ADR 0016).
    pub fn hide(&mut self) {
        self.visible = false;
    }

    /// Marks the window shown again. Raising it to the top of its desktop's
    /// stack is the desktop's job, not this method's — it doesn't know its
    /// own stack position.
    pub fn show(&mut self) {
        self.visible = true;
    }

    /// Whether the window is currently maximised.
    pub fn is_maximized(&self) -> bool {
        self.maximized
    }

    /// Repositions/resizes the window directly — a drag/resize session (owned
    /// by [`Desktop`](super::Desktop)) calls this; `Window` has no opinion
    /// about *why* its bounds changed. Propagates the new
    /// [`interior_bounds`](Self::interior_bounds) to the interior via
    /// [`View::set_bounds`] (ADR 0017), so an interior whose layout depends
    /// on its size (a wrapped, scrolled page) can relayout.
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
        self.interior.set_bounds(self.interior_bounds());
    }

    /// Toggles between the window's normal bounds and filling `desktop_bounds`
    /// (TurboVision's `cmZoom`/`TWindow::zoom`, ADR 0016). Remembers the prior
    /// bounds and `casts_shadow` setting (a maximised window sits flush, so it
    /// stops casting one — a shadow off the edge of the desktop is pointless)
    /// and restores both on the next toggle.
    pub fn toggle_zoom(&mut self, desktop_bounds: Rect) {
        match self.restore.take() {
            Some((bounds, casts_shadow)) => {
                self.bounds = bounds;
                self.casts_shadow = casts_shadow;
                self.maximized = false;
            }
            None => {
                self.restore = Some((self.bounds, self.casts_shadow));
                self.bounds = desktop_bounds;
                self.casts_shadow = false;
                self.maximized = true;
            }
        }
        self.frame.set_maximized(self.maximized);
        // Same propagation set_bounds does (ADR 0017) — a zoom/restore is
        // just another way the window's area changes.
        self.interior.set_bounds(self.interior_bounds());
    }

    /// The scroll bar hosting the interior's vertical overflow, if it has any
    /// (ADR 0015) — drawn and hit-tested along the window's own right border,
    /// in the window's local coordinates, replacing that segment of the plain
    /// border line (mirroring how `edit`'s bespoke editor window already drew
    /// scroll chrome directly on its frame).
    fn vertical_scroll_bar(&self) -> Option<ScrollBar> {
        let vertical = self.interior.scroll_metrics()?.vertical?;
        let height = self.bounds.height();
        if height <= 2 {
            return None;
        }
        let mut bar = ScrollBar::new(
            Rect::from_origin_size(
                Point::new(self.bounds.width() - 1, 1),
                Size::new(1, height - 2),
            ),
            self.frame_style,
        );
        bar.set_metrics(vertical.total, vertical.visible, vertical.pos);
        Some(bar)
    }

    /// The scroll bar hosting the interior's horizontal overflow, mirroring
    /// [`vertical_scroll_bar`](Self::vertical_scroll_bar) along the bottom border.
    fn horizontal_scroll_bar(&self) -> Option<ScrollBar> {
        let horizontal = self.interior.scroll_metrics()?.horizontal?;
        let width = self.bounds.width();
        if width <= 2 {
            return None;
        }
        let mut bar = ScrollBar::horizontal(
            Rect::from_origin_size(
                Point::new(1, self.bounds.height() - 1),
                Size::new(width - 2, 1),
            ),
            self.frame_style,
        );
        bar.set_metrics(horizontal.total, horizontal.visible, horizontal.pos);
        Some(bar)
    }

    /// Pushes a new combined scroll offset to the interior, changing only the
    /// axis `delta` is non-zero for and leaving the other where it was (`View::set_scroll`
    /// takes one `Point` for both axes, ADR 0015).
    fn nudge_scroll(&mut self, vertical_delta: isize, horizontal_delta: isize) {
        let Some(metrics) = self.interior.scroll_metrics() else {
            return;
        };
        let axis = |current: Option<crate::view::AxisMetrics>, delta: isize| -> i16 {
            match current {
                Some(a) if delta != 0 => {
                    let max = a.total.saturating_sub(a.visible) as isize;
                    (a.pos as isize + delta).clamp(0, max) as i16
                }
                Some(a) => a.pos as i16,
                None => 0,
            }
        };
        let x = axis(metrics.horizontal, horizontal_delta);
        let y = axis(metrics.vertical, vertical_delta);
        self.interior.set_scroll(Point::new(x, y));
    }

    /// Handles a click landing on a hosted scroll bar: `vertical` selects which
    /// axis's bar to test. Returns whether the click landed on it (and was
    /// therefore handled) at all.
    fn handle_scroll_bar_click(&mut self, pos: Point, kind: MouseKind, vertical: bool) -> bool {
        if !matches!(
            kind,
            MouseKind::Down(MouseButton::Left) | MouseKind::DoubleClick(MouseButton::Left)
        ) {
            return false;
        }
        let bar = if vertical {
            self.vertical_scroll_bar()
        } else {
            self.horizontal_scroll_bar()
        };
        let Some(bar) = bar else {
            return false;
        };
        if !bar.bounds().contains(pos) {
            return false;
        }
        if let Some(part) = bar.hit(pos) {
            let page = if vertical {
                self.interior
                    .scroll_metrics()
                    .and_then(|m| m.vertical)
                    .map(|a| a.visible)
                    .unwrap_or(1)
            } else {
                self.interior
                    .scroll_metrics()
                    .and_then(|m| m.horizontal)
                    .map(|a| a.visible)
                    .unwrap_or(1)
            }
            .max(1) as isize;
            let delta = match part {
                ScrollPart::LineUp => -1,
                ScrollPart::LineDown => 1,
                ScrollPart::PageUp => -page,
                ScrollPart::PageDown => page,
                ScrollPart::Thumb => 0, // dragging the thumb rides on window drag infra (Phase 9d)
            };
            if vertical {
                self.nudge_scroll(delta, 0);
            } else {
                self.nudge_scroll(0, delta);
            }
        }
        true
    }
}

impl View for Window {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        self.frame.draw(canvas);
        if let Some(bar) = self.vertical_scroll_bar() {
            let mut sub = canvas.child(bar.bounds());
            bar.draw(&mut sub);
        }
        if let Some(bar) = self.horizontal_scroll_bar() {
            let mut sub = canvas.child(bar.bounds());
            bar.draw(&mut sub);
        }
        let interior = self.interior_bounds();
        if !interior.is_empty() {
            let mut sub = canvas.child(interior);
            // Paint the window's own background so a non-filling interior (or none)
            // shows solid window colour, not the desktop bleeding through.
            let area = sub.bounds();
            sub.fill(area, &self.interior_fill);
            self.interior.draw(&mut sub);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Mouse(mouse) => {
                // The close/zoom glyphs sit on the top border row (ADR 0016);
                // neither is interactive when its flag is off (nor drawn there —
                // see Frame), so there is nothing to hit.
                if mouse.pos.y == 0 && matches!(mouse.kind, MouseKind::Down(MouseButton::Left)) {
                    let has_help = self.help_topic.is_some();
                    if self.closable {
                        if let Some(span) = Frame::close_span(self.bounds.width(), has_help) {
                            if span.contains(&mouse.pos.x) {
                                ctx.post(CM_CLOSE);
                                return EventResult::Consumed;
                            }
                        }
                    }
                    if self.zoomable {
                        if let Some(span) = Frame::zoom_span(self.bounds.width(), has_help) {
                            if span.contains(&mouse.pos.x) {
                                ctx.post(CM_ZOOM);
                                return EventResult::Consumed;
                            }
                        }
                    }
                    if has_help {
                        if let Some(span) = Frame::help_span(self.bounds.width()) {
                            if span.contains(&mouse.pos.x) {
                                ctx.post(CM_HELP);
                                return EventResult::Consumed;
                            }
                        }
                    }
                }
                if self.handle_scroll_bar_click(mouse.pos, mouse.kind, true)
                    || self.handle_scroll_bar_click(mouse.pos, mouse.kind, false)
                {
                    return EventResult::Consumed;
                }
                // Positional events inside the interior are translated into it;
                // clicks anywhere else on the border (title bar, resize corner)
                // are left Ignored — that silence is deliberate: it tells a
                // Desktop that no session should start there unless it
                // recognises the click as one (ADR 0016). Window has no concept
                // of a drag session.
                let interior = self.interior_bounds();
                if interior.contains(mouse.pos) {
                    let origin = interior.origin();
                    let local = MouseEvent {
                        pos: mouse.pos.offset(-origin.x, -origin.y),
                        ..*mouse
                    };
                    ctx.translated(origin.x, origin.y, |ctx| {
                        self.interior.handle_event(&Event::Mouse(local), ctx)
                    })
                } else {
                    EventResult::Ignored
                }
            }
            Event::Key(key) => {
                // Esc cancels before the interior sees it, but only if configured
                // to (a plain Window leaves Esc to its interior) — from the old
                // Dialog, folded in unchanged (ADR 0016).
                if self.esc_cancels && key.code == KeyCode::Esc {
                    ctx.post(CM_CANCEL);
                    return EventResult::Consumed;
                }
                if self.interior.handle_event(event, ctx).is_consumed() {
                    return EventResult::Consumed;
                }
                // Enter falls back to the default command, if any.
                if key.code == KeyCode::Enter {
                    if let Some(command) = self.default_cmd {
                        ctx.post(command);
                        return EventResult::Consumed;
                    }
                }
                EventResult::Ignored
            }
            // Everything else (commands, broadcasts, paste) goes straight to the
            // interior, whose ignored results bubble back up (ADR 0003).
            _ => self.interior.handle_event(event, ctx),
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn drop_shadow(&self) -> Option<Style> {
        self.casts_shadow.then_some(self.shadow_style)
    }

    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
        // A Window never vetoes on its own behalf; whatever it wraps decides
        // (TV's TView::valid default, composed one level, ADR 0016).
        self.interior.valid(command, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::canvas::Canvas;
    use crate::color::Style;
    use crate::command::{CM_HELP, CM_OK, CM_USER, CommandSet};
    use crate::event::{KeyCode, KeyEvent, Modifiers, MouseButton, MouseKind};
    use crate::view::{AxisMetrics, ScrollMetrics, StaticText};
    use std::cell::RefCell;
    use std::rc::Rc;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    fn theme() -> Theme {
        Theme::default()
    }

    fn plain(bounds: Rect, interior: Box<dyn View>) -> Window {
        Window::new(bounds, "T", &theme(), interior)
    }

    fn blank() -> Box<dyn View> {
        Box::new(StaticText::new(rect(0, 0, 1, 1), "", Style::new()))
    }

    /// An interior that records keys and posts `CM_OK` on Enter.
    struct Interior {
        seen: Rc<RefCell<Vec<Event>>>,
    }

    impl View for Interior {
        fn bounds(&self) -> Rect {
            rect(0, 0, 100, 100)
        }
        fn draw(&self, canvas: &mut Canvas) {
            canvas.put_str(Point::new(0, 0), "in", Style::new());
        }
        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            self.seen.borrow_mut().push(event.clone());
            if matches!(event, Event::Key(k) if k.code == KeyCode::Enter) {
                ctx.post(CM_OK);
                return EventResult::Consumed;
            }
            EventResult::Ignored
        }
    }

    #[test]
    fn interior_bounds_inset_by_one_on_each_side() {
        let w = plain(rect(5, 2, 20, 8), blank());
        assert_eq!(w.interior_bounds(), rect(1, 1, 18, 6));
    }

    #[test]
    fn casts_a_shadow_by_default_and_can_be_turned_off() {
        let theme = theme();
        let mut w = plain(rect(5, 2, 20, 8), blank());
        // Default: the window reports the theme's shadow style for its owner to
        // paint (ADR 0011).
        assert_eq!(w.drop_shadow(), Some(theme.style(Role::Shadow)));
        // Turning it off makes it sit flush — no shadow reported.
        w.set_casts_shadow(false);
        assert_eq!(w.drop_shadow(), None);
    }

    #[test]
    fn tiny_window_has_an_empty_interior() {
        let w = plain(rect(0, 0, 1, 1), blank());
        assert!(w.interior_bounds().is_empty());
    }

    #[test]
    fn snapshot_window_draws_frame_then_interior() {
        let w = Window::new(
            rect(0, 0, 28, 5),
            "Untitled",
            &theme(),
            Box::new(StaticText::new(rect(0, 0, 26, 3), "hello", Style::new())),
        );
        let mut buf = Buffer::new(Size::new(28, 5));
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn keys_route_into_the_interior_and_commands_bubble() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut w = plain(
            rect(0, 0, 16, 5),
            Box::new(Interior {
                seen: Rc::clone(&seen),
            }),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE));
        assert_eq!(w.handle_event(&enter, &mut ctx), EventResult::Consumed);
        assert_eq!(*seen.borrow(), vec![enter]);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
    }

    #[test]
    fn set_active_toggles_the_frame_and_flag() {
        let mut w = plain(rect(0, 0, 10, 4), blank());
        assert!(!w.is_active());
        w.set_active(true);
        assert!(w.is_active());
    }

    // --- New Window/Dialog-unification behaviour (ADR 0016) ---

    #[test]
    fn new_window_is_fully_capable_by_default() {
        let w = plain(rect(0, 0, 10, 4), blank());
        assert!(w.resizable);
        assert!(w.moveable);
        assert!(w.closable);
        assert!(w.zoomable);
        assert_eq!(w.placement(), Placement::Positioned);
        assert!(!w.ends_on(CM_OK), "no ending commands by default");
        assert!(!w.esc_cancels);
        assert!(w.is_visible());
        assert!(!w.is_maximized());
    }

    #[test]
    fn dialog_uses_the_dialog_background_role_throughout() {
        // TurboVision's dialogs keep a distinct palette from ordinary windows
        // (ADR 0016) — Window::dialog resolves Role::DialogBackground for
        // frame, title, and interior fill, not Role::WindowFrame/WindowTitle.
        let t = theme();
        let w = Window::dialog(rect(0, 0, 10, 4), "T", &t, blank());
        assert_eq!(w.frame_style, t.style(Role::DialogBackground));
        assert_eq!(
            w.interior_fill,
            Cell::blank(t.style(Role::DialogBackground))
        );

        let plain = Window::new(rect(0, 0, 10, 4), "T", &t, blank());
        assert_eq!(plain.frame_style, t.style(Role::WindowFrame));
    }

    #[test]
    fn builders_configure_a_dialog_shaped_window() {
        let w = plain(rect(0, 0, 10, 4), blank())
            .resizable(false)
            .zoomable(false)
            .centered()
            .with_default(CM_OK)
            .also_ends_on(CM_OK)
            .esc_cancels(true);
        assert!(!w.resizable);
        assert!(!w.zoomable);
        assert!(w.moveable, "unaffected builders keep their default");
        assert_eq!(w.placement(), Placement::Centered);
        assert!(w.ends_on(CM_OK));
        assert!(w.esc_cancels);
    }

    #[test]
    fn query_getters_mirror_the_builder_flags() {
        // Desktop is a sibling module and can't see the private fields the
        // tests above check directly — these public getters are its only way
        // to read them (ADR 0016).
        let w = plain(rect(0, 0, 10, 4), blank());
        assert!(w.is_moveable());
        assert!(w.is_resizable());
        assert!(w.is_closable());
        assert!(w.is_zoomable());

        let locked = plain(rect(0, 0, 10, 4), blank())
            .moveable(false)
            .resizable(false)
            .closable(false)
            .zoomable(false);
        assert!(!locked.is_moveable());
        assert!(!locked.is_resizable());
        assert!(!locked.is_closable());
        assert!(!locked.is_zoomable());
    }

    #[test]
    fn resizable_flag_reaches_the_frames_corner_handle() {
        let w = plain(rect(0, 0, 20, 5), blank());
        let mut buf = Buffer::new(Size::new(20, 5));
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        assert!(buf.to_text().contains('◢'), "resizable by default");

        let locked = plain(rect(0, 0, 20, 5), blank()).resizable(false);
        let mut buf = Buffer::new(Size::new(20, 5));
        let mut canvas = Canvas::new(&mut buf);
        locked.draw(&mut canvas);
        assert!(!buf.to_text().contains('◢'), "no handle when locked");
    }

    #[test]
    fn ends_on_covers_added_commands_only() {
        const CM_APPLY: Command = Command(CM_USER + 1);
        let w = plain(rect(0, 0, 10, 4), blank()).also_ends_on(CM_OK);
        assert!(w.ends_on(CM_OK));
        assert!(!w.ends_on(CM_APPLY));
    }

    #[test]
    fn esc_posts_cancel_only_when_configured() {
        let mut plain_w = plain(rect(0, 0, 10, 4), blank());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
        assert_eq!(plain_w.handle_event(&esc, &mut ctx), EventResult::Ignored);
        assert!(ctx.posted().is_empty());

        let mut cancelling = plain(rect(0, 0, 10, 4), blank()).esc_cancels(true);
        let mut ctx = Context::new(&cs);
        assert_eq!(
            cancelling.handle_event(&esc, &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(ctx.posted(), &[Event::Command(CM_CANCEL)]);
    }

    #[test]
    fn enter_falls_back_to_the_default_command_when_the_interior_ignores_it() {
        struct IgnoreEnter;
        impl View for IgnoreEnter {
            fn bounds(&self) -> Rect {
                rect(0, 0, 5, 1)
            }
            fn draw(&self, _canvas: &mut Canvas) {}
        }
        const CM_APPLY: Command = Command(CM_USER + 1);
        let mut w = plain(rect(0, 0, 20, 5), Box::new(IgnoreEnter)).with_default(CM_APPLY);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE));
        w.handle_event(&enter, &mut ctx);
        assert_eq!(ctx.posted(), &[Event::Command(CM_APPLY)]);
    }

    #[test]
    fn close_glyph_click_posts_cm_close_only_when_closable() {
        let mut w = plain(rect(0, 0, 20, 5), blank());
        let close_x = Frame::close_span(20, false).unwrap().start;
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(close_x, 0),
            modifiers: Modifiers::NONE,
        });
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(w.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_CLOSE)]);

        let mut not_closable = plain(rect(0, 0, 20, 5), blank()).closable(false);
        let mut ctx = Context::new(&cs);
        assert_eq!(
            not_closable.handle_event(&click, &mut ctx),
            EventResult::Ignored
        );
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn zoom_glyph_click_posts_cm_zoom_only_when_zoomable() {
        let mut w = plain(rect(0, 0, 20, 5), blank());
        let zoom_x = Frame::zoom_span(20, false).unwrap().start;
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(zoom_x, 0),
            modifiers: Modifiers::NONE,
        });
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(w.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_ZOOM)]);

        let mut not_zoomable = plain(rect(0, 0, 20, 5), blank()).zoomable(false);
        let mut ctx = Context::new(&cs);
        assert_eq!(
            not_zoomable.handle_event(&click, &mut ctx),
            EventResult::Ignored
        );
        assert!(ctx.posted().is_empty());
    }

    // --- Help topic / glyph (ADR 0021) ---

    #[test]
    fn no_help_topic_by_default() {
        let w = plain(rect(0, 0, 20, 5), blank());
        assert_eq!(w.help_topic(), None);
    }

    #[test]
    fn with_help_topic_sets_the_topic_and_shows_the_glyph() {
        let w = plain(rect(0, 0, 20, 5), blank()).with_help_topic("intro");
        assert_eq!(w.help_topic(), Some("intro"));
        let mut buf = Buffer::new(Size::new(20, 5));
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        assert!(buf.to_text().contains('?'), "help glyph is drawn");
    }

    #[test]
    fn help_glyph_click_posts_cm_help_only_when_a_topic_is_set() {
        let mut w = plain(rect(0, 0, 20, 5), blank()).with_help_topic("intro");
        let help_x = Frame::help_span(20).unwrap().start;
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(help_x, 0),
            modifiers: Modifiers::NONE,
        });
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(w.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_HELP)]);

        let mut no_topic = plain(rect(0, 0, 20, 5), blank());
        let mut ctx = Context::new(&cs);
        assert_eq!(
            no_topic.handle_event(&click, &mut ctx),
            EventResult::Ignored
        );
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn a_click_on_the_bare_title_bar_or_resize_corner_is_ignored() {
        // Deliberate silence: it's what tells a Desktop no session should
        // start there unless it recognises the click as one (ADR 0016).
        let mut w = plain(rect(0, 0, 20, 5), blank());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let title_bar = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(10, 0),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(w.handle_event(&title_bar, &mut ctx), EventResult::Ignored);
        let corner = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(19, 4),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(w.handle_event(&corner, &mut ctx), EventResult::Ignored);
    }

    #[test]
    fn hide_and_show_toggle_visibility() {
        let mut w = plain(rect(0, 0, 10, 4), blank());
        assert!(w.is_visible());
        w.hide();
        assert!(!w.is_visible());
        w.show();
        assert!(w.is_visible());
    }

    #[test]
    fn toggle_zoom_round_trips_bounds_and_shadow() {
        let original = rect(2, 1, 20, 8);
        let mut w = plain(original, blank());
        assert!(w.drop_shadow().is_some());

        let desktop_bounds = rect(0, 0, 80, 24);
        w.toggle_zoom(desktop_bounds);
        assert!(w.is_maximized());
        assert_eq!(w.bounds(), desktop_bounds);
        assert_eq!(w.drop_shadow(), None, "a maximised window sits flush");

        w.toggle_zoom(desktop_bounds);
        assert!(!w.is_maximized());
        assert_eq!(w.bounds(), original);
        assert!(
            w.drop_shadow().is_some(),
            "the shadow setting is restored too"
        );
    }

    #[test]
    fn set_bounds_repositions_the_window_directly() {
        let mut w = plain(rect(0, 0, 10, 4), blank());
        let moved = rect(5, 5, 12, 6);
        w.set_bounds(moved);
        assert_eq!(w.bounds(), moved);
    }

    #[test]
    fn valid_delegates_to_the_interior() {
        struct Vetoer {
            refuses: Command,
        }
        impl View for Vetoer {
            fn bounds(&self) -> Rect {
                rect(0, 0, 5, 1)
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn valid(&mut self, command: Command, _ctx: &mut Context) -> bool {
                command != self.refuses
            }
        }
        let mut w = plain(rect(0, 0, 10, 4), Box::new(Vetoer { refuses: CM_CLOSE }));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!w.valid(CM_CLOSE, &mut ctx));
        assert!(w.valid(CM_OK, &mut ctx));
    }

    // --- Scroll-chrome hosting on the border (ADR 0015) ---

    /// An interior that reports a fixed vertical/horizontal overflow and
    /// records the last offset pushed to it.
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
    fn a_non_scrolling_interior_gets_no_border_bar() {
        let w = plain(rect(0, 0, 20, 8), blank());
        let mut buf = Buffer::new(Size::new(20, 8));
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        assert!(!buf.to_text().contains('▲'), "no vertical bar drawn");
    }

    #[test]
    fn a_scrolling_interior_gets_a_vertical_bar_on_the_right_border() {
        let pushed = Rc::new(RefCell::new(Vec::new()));
        let w = plain(
            rect(0, 0, 20, 8),
            Box::new(Scrollable {
                metrics: vertical_metrics(20, 6, 0),
                pushed,
            }),
        );
        let mut buf = Buffer::new(Size::new(20, 8));
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        let text = buf.to_text();
        let rows: Vec<&str> = text.lines().collect();
        // Right column, row 1 (just inside the top border) is the up arrow.
        assert_eq!(rows[1].chars().last(), Some('▲'));
    }

    #[test]
    fn clicking_the_vertical_bars_down_arrow_scrolls_the_interior() {
        let pushed = Rc::new(RefCell::new(Vec::new()));
        let mut w = plain(
            rect(0, 0, 20, 8),
            Box::new(Scrollable {
                metrics: vertical_metrics(20, 6, 0),
                pushed: Rc::clone(&pushed),
            }),
        );
        // Right border column (x=19), bottom of the bar's track (y=6, just
        // above the bottom border at y=7) is the down arrow.
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(19, 6),
            modifiers: Modifiers::NONE,
        });
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert_eq!(w.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(*pushed.borrow(), vec![Point::new(0, 1)]);
    }

    // --- Resize propagation to the interior (ADR 0017) ---

    /// An interior that records every `bounds` it's told about via `set_bounds`.
    struct ResizeSpy {
        seen: Rc<RefCell<Vec<Rect>>>,
    }

    impl View for ResizeSpy {
        fn bounds(&self) -> Rect {
            rect(0, 0, 100, 100)
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn set_bounds(&mut self, bounds: Rect) {
            self.seen.borrow_mut().push(bounds);
        }
    }

    #[test]
    fn set_bounds_propagates_the_new_interior_area() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut w = plain(
            rect(0, 0, 20, 8),
            Box::new(ResizeSpy {
                seen: Rc::clone(&seen),
            }),
        );
        w.set_bounds(rect(2, 2, 30, 12));
        assert_eq!(*seen.borrow(), vec![rect(1, 1, 28, 10)]);
    }

    #[test]
    fn toggle_zoom_propagates_the_new_interior_area_each_way() {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let mut w = plain(
            rect(2, 1, 20, 8),
            Box::new(ResizeSpy {
                seen: Rc::clone(&seen),
            }),
        );
        let desktop_bounds = rect(0, 0, 80, 24);
        w.toggle_zoom(desktop_bounds);
        w.toggle_zoom(desktop_bounds);
        assert_eq!(
            *seen.borrow(),
            vec![
                rect(1, 1, 78, 22), // maximised: interior of the full desktop
                rect(1, 1, 18, 6),  // restored: interior of the original bounds
            ]
        );
    }
}
