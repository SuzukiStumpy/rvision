//! The application driver: owns the terminal seam and runs the main loop.
//!
//! Each turn the loop builds a fresh frame, lets the [`Program`] draw into it,
//! presents it (a minimal diff flush — ADR 0002), then waits up to a timeout for
//! the next event and hands it to the program. A timed-out wait becomes
//! [`Event::Idle`], so the timeout is the idle/blink cadence.
//!
//! The thing the loop drives is abstracted behind [`Program`] so the loop is
//! unit-testable against a scripted, headless terminal with no real TTY. In
//! Phase 3 the root view tree takes the [`Program`] role; in Phase 2 a demo or a
//! test does (ADR 0003, 0004).

use crate::backend::{Backend, EventSource};
use crate::buffer::Buffer;
use crate::canvas::Canvas;
use crate::command::{CM_HELP, CM_QUIT, Command, CommandSet};
use crate::event::{Event, EventResult, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::help::HelpContents;
use crate::theme::Theme;
use crate::view::{Context, View};
use crate::widgets::{
    ContextMenu, Desktop, HelpWindow, MenuBar, Placement, StatusLine, Window, WindowId,
};
use std::collections::VecDeque;
use std::io;
use std::time::Duration;

/// The default idle/blink cadence: how long [`Application::run`] waits for input
/// before synthesising an [`Event::Idle`].
const DEFAULT_TIMEOUT: Duration = Duration::from_millis(100);

/// Cap on the number of posted events processed per input event. A hang-guard:
/// a misbehaving view that posts a command in response to handling one drops
/// events past the cap rather than spinning the loop forever.
const MAX_POSTED_PER_EVENT: usize = 1024;

/// What the [`Application`] loop drives: something that can render itself, react
/// to events, and say when it is done. The Phase 3 view tree will implement this.
pub trait Program {
    /// Renders the current state into `frame` (a blank buffer of the terminal's
    /// current size).
    fn draw(&mut self, frame: &mut Buffer);

    /// Reacts to one event, returning whether it was consumed (ADR 0004).
    fn handle_event(&mut self, event: &Event) -> EventResult;

    /// Returns whether the loop should stop. Checked after each draw and after
    /// each handled event.
    fn is_finished(&self) -> bool;
}

/// Owns the terminal (a combined [`Backend`] + [`EventSource`]) and runs the loop.
///
/// Because the `Application` owns the terminal, any unwind through [`run`](Self::run)
/// drops it — and the real backend's `Drop` restores the terminal (ADR 0001).
pub struct Application<T> {
    terminal: T,
    timeout: Duration,
}

impl<T: Backend + EventSource> Application<T> {
    /// Creates an application over `terminal` with the default idle cadence.
    pub fn new(terminal: T) -> Self {
        Self {
            terminal,
            timeout: DEFAULT_TIMEOUT,
        }
    }

    /// Sets the idle/blink cadence (the poll timeout).
    pub fn with_timeout(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// The current idle/blink cadence.
    pub fn timeout(&self) -> Duration {
        self.timeout
    }

    /// Borrows the terminal (e.g. to inspect a test backend's screen).
    pub fn terminal(&self) -> &T {
        &self.terminal
    }

    /// Mutably borrows the terminal.
    pub fn terminal_mut(&mut self) -> &mut T {
        &mut self.terminal
    }

    /// Pushes `text` to the host system clipboard through the backend (OSC 52 on
    /// a real terminal, a no-op on a backend that cannot reach one — the editor's ADR 0021).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error from writing to the terminal.
    pub fn set_clipboard(&mut self, text: &str) -> io::Result<()> {
        self.terminal.set_clipboard(text)
    }

    /// Runs the loop until `program` reports it is finished.
    ///
    /// Each turn: build a frame at the terminal's current size, `draw`, `present`,
    /// stop if finished, else wait for an event (`None` ⇒ [`Event::Idle`]),
    /// handle it, stop if finished. The two finish checks bracket the wait so a
    /// program that finishes while handling an event exits without a spurious
    /// extra draw, while one that starts finished still paints once.
    ///
    /// # Errors
    ///
    /// Propagates any I/O error from presenting a frame or polling for events.
    pub fn run(&mut self, program: &mut impl Program) -> io::Result<()> {
        loop {
            let mut frame = Buffer::new(self.terminal.size());
            program.draw(&mut frame);
            self.terminal.present(&frame)?;
            if program.is_finished() {
                break;
            }

            let event = self
                .terminal
                .poll_event(self.timeout)?
                .unwrap_or(Event::Idle);
            program.handle_event(&event);
            if program.is_finished() {
                break;
            }
        }
        Ok(())
    }

    /// Runs `window` modally over `background`, returning the command that closed
    /// it (ADR 0010) — TurboVision's `execView`. `Modal` is gone (ADR 0016):
    /// `Window` already carries everything this needs (size via its `bounds`,
    /// `ends_on`, `valid`), so this now takes the one concrete type that ever
    /// played that role.
    ///
    /// Each turn: build a frame at the terminal's current size, let `background`
    /// **draw** (it receives no events while the window is up); if the window's
    /// [`Placement`] is
    /// [`Centered`](crate::widgets::Placement::Centered), reposition it to the
    /// centre of the terminal first (a [`Positioned`](crate::widgets::Placement::Positioned)
    /// one runs exactly where its own `bounds` say); draw it on top, present, then
    /// poll one event and hand it to the window. A positional event is translated
    /// into the window's local coordinates. The first *ending* command the window
    /// posts ([`Window::ends_on`]) returns from the loop; any other posted
    /// command/broadcast is re-dispatched into the window, exactly as [`Root`]
    /// drains the tree. `Esc` closes it as `CM_CANCEL` when the window is
    /// configured to (`esc_cancels`).
    ///
    /// The window never joins the application's view tree; it is the caller's,
    /// borrowed for the duration of the loop and untouched afterwards — so a
    /// caller that wants a reusable dialog just holds the `Window` itself and
    /// calls this again next time, no reconstruction needed (ADR 0016).
    ///
    /// # Errors
    ///
    /// Propagates any I/O error from presenting a frame or polling for events.
    pub fn exec_view(
        &mut self,
        background: &mut dyn Program,
        window: &mut Window,
    ) -> io::Result<Command> {
        let commands = CommandSet::new();
        loop {
            let size = self.terminal.size();
            let mut frame = Buffer::new(size);
            background.draw(&mut frame);

            if window.placement() == Placement::Centered {
                window.set_bounds(centered(window.bounds().size(), size));
            }
            let area = window.bounds();
            {
                let mut canvas = Canvas::new(&mut frame);
                // The modal casts its own drop shadow on the background it floats
                // over, through the per-view protocol (ADR 0011).
                if let Some(style) = window.drop_shadow() {
                    canvas.shadow(area, style);
                }
                let mut sub = canvas.child(area);
                window.draw(&mut sub);
            }
            self.terminal.present(&frame)?;

            let event = self
                .terminal
                .poll_event(self.timeout)?
                .unwrap_or(Event::Idle);
            // Translate a positional event into the window's local coordinates;
            // everything else passes through unchanged.
            let event = match event {
                Event::Mouse(mouse) => Event::Mouse(MouseEvent {
                    pos: mouse.pos.offset(-area.origin().x, -area.origin().y),
                    ..mouse
                }),
                other => other,
            };

            if let Some(command) = dispatch_modal(window, &event, &commands) {
                return Ok(command);
            }
        }
    }
}

/// Centres a box of `size` within `within`, clamped to fit.
fn centered(size: Size, within: Size) -> Rect {
    let w = size.width.clamp(0, within.width.max(0));
    let h = size.height.clamp(0, within.height.max(0));
    let x = ((within.width - w) / 2).max(0);
    let y = ((within.height - h) / 2).max(0);
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// Delivers one event to `window` and drains what it posts: a posted *ending*
/// command (`Window::ends_on`) is returned to stop the modal loop; any other
/// posted command/broadcast is re-dispatched into the window. Returns `None` if
/// nothing ended the loop.
fn dispatch_modal(window: &mut Window, event: &Event, commands: &CommandSet) -> Option<Command> {
    let mut queue = VecDeque::new();
    {
        let mut ctx = Context::new(commands);
        window.handle_event(event, &mut ctx);
        queue.extend(ctx.take_posted());
    }
    let mut budget = MAX_POSTED_PER_EVENT;
    while let Some(posted) = queue.pop_front() {
        if let Event::Command(command) = posted {
            if window.ends_on(command) {
                return Some(command);
            }
        }
        if budget == 0 {
            break;
        }
        budget -= 1;
        let mut ctx = Context::new(commands);
        window.handle_event(&posted, &mut ctx);
        queue.extend(ctx.take_posted());
    }
    None
}

/// The root of the view tree, adapted to the loop's [`Program`] contract.
///
/// `Root` owns the top-level [`View`] and the [`CommandSet`], and is where the
/// two halves of the command model meet (ADR 0003, 0004): an input event is
/// dispatched into the tree through a [`Context`]; whatever the tree *posts* in
/// response — commands bubbling up, broadcasts going down — is drained and
/// re-dispatched from the root until it settles. [`CM_QUIT`] is the one command
/// `Root` handles itself: it ends the loop. This replaces Phase 2's quit-flag
/// stepping stone with the real command path.
pub struct Root {
    view: Box<dyn View>,
    commands: CommandSet,
    finished: bool,
}

impl Root {
    /// Creates a root over `view`, with every command enabled.
    pub fn new(view: Box<dyn View>) -> Self {
        Self {
            view,
            commands: CommandSet::new(),
            finished: false,
        }
    }

    /// Starts from a given command-enable set (e.g. with some commands disabled).
    pub fn with_commands(mut self, commands: CommandSet) -> Self {
        self.commands = commands;
        self
    }

    /// The command-enable set.
    pub fn commands(&self) -> &CommandSet {
        &self.commands
    }

    /// The command-enable set, mutably — enable/disable as application state
    /// changes (a control reads this to grey itself).
    pub fn commands_mut(&mut self) -> &mut CommandSet {
        &mut self.commands
    }

    /// Delivers one event to the view tree, queueing whatever it posts.
    fn deliver(&mut self, event: &Event, queue: &mut VecDeque<Event>) -> EventResult {
        // Split-borrow the disjoint fields so the tree can be handled mutably
        // while the context reads the command set.
        let Self { view, commands, .. } = self;
        let mut ctx = Context::new(commands);
        let result = view.handle_event(event, &mut ctx);
        queue.extend(ctx.take_posted());
        result
    }

    /// Dispatches `event`, then drains posted commands/broadcasts, re-dispatching
    /// each from the root. [`CM_QUIT`] ends the loop, but only once the tree
    /// agrees it's `valid` (ADR 0016) — asked of the single root view, so a
    /// `Desktop`'s or `Group`'s own fan-out override reaches every window/child
    /// underneath without `Root` knowing either exists. A refusal leaves the
    /// loop running; any follow-up the refusing view posted (e.g. "confirm
    /// discard") is drained and re-dispatched like any other posted event.
    /// Everything else flows back into the tree. Returns the result of the
    /// original event.
    fn dispatch(&mut self, event: &Event) -> EventResult {
        let mut queue = VecDeque::new();
        let result = self.deliver(event, &mut queue);
        let mut budget = MAX_POSTED_PER_EVENT;
        while let Some(posted) = queue.pop_front() {
            if posted == Event::Command(CM_QUIT) {
                let mut ctx = Context::new(&self.commands);
                if self.view.valid(CM_QUIT, &mut ctx) {
                    self.finished = true;
                }
                queue.extend(ctx.take_posted());
                continue;
            }
            if budget == 0 {
                break;
            }
            budget -= 1;
            self.deliver(&posted, &mut queue);
        }
        result
    }
}

impl Program for Root {
    fn draw(&mut self, frame: &mut Buffer) {
        // The root view fills the terminal: hand it a canvas over the whole frame
        // so it can lay itself out against the live size (centre, reflow on
        // resize) via `Canvas::size`. Its own `bounds` is for nesting in an owner,
        // which the root has none of.
        let mut canvas = Canvas::new(frame);
        self.view.draw(&mut canvas);
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        self.dispatch(event)
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

/// The standard application screen: a menu bar across the top, a status line
/// across the bottom, and a desktop filling the space between — TurboVision's
/// `TProgram` (ADR 0009).
///
/// `Shell` is itself a [`View`], so it drops into a [`Root`] and runs in the
/// [`Application`] loop. Unlike a generic [`Group`](crate::view::Group) it:
///
/// - **lays out live**: each frame it carves the three regions from the terminal
///   size, so a resize relays them out (windows inside the desktop keep their
///   place);
/// - **draws the menu overlay last**: an open pull-down paints over the whole
///   frame, on top of everything (the menu bar's own row is only one tall);
/// - **routes keys in three local passes** — menu bar (pre-process: `Alt`-hot-keys
///   and, while open, modally), then the desktop's active window (focused), then
///   the status line (post-process: global function keys). The generic event
///   engine is untouched; these passes live only here, where the only views that
///   need them are.
pub struct Shell {
    menu_bar: MenuBar,
    desktop: Desktop,
    status_line: StatusLine,
    size: Size,
    /// Kept solely to build a [`ContextMenu`] on demand (ADR 0019) — unlike
    /// the three chrome pieces above, a context menu doesn't exist until a
    /// view requests one at runtime, so there is no upfront construction
    /// site to resolve its styles from a theme the way `MenuBar::new` etc.
    /// already do.
    theme: Theme,
    /// The open context menu, if any (ADR 0019). Mirrors `MenuBar`'s own
    /// open pull-down: first refusal on every key/mouse event while `Some`,
    /// drawn as a full-frame overlay last, after the menu bar's own.
    context_menu: Option<ContextMenu>,
    /// App-supplied help content, opted into via `with_help` (ADR 0021).
    /// `None` (the default) means `CM_HELP` is never caught here — it falls
    /// through to the desktop like any other command.
    help: Option<HelpContents>,
    /// The singleton help window's id, if one is currently open (ADR 0021) —
    /// `CM_HELP` closes and reopens this one, rather than letting duplicates
    /// accumulate.
    help_window: Option<WindowId>,
}

/// The three chrome regions for a terminal of `size`: the menu-bar row, the
/// desktop between, and the status-line row.
struct Regions {
    menu: Rect,
    desktop: Rect,
    status: Rect,
}

fn regions(size: Size) -> Regions {
    let w = size.width.max(0);
    let h = size.height.max(0);
    Regions {
        menu: Rect::from_origin_size(Point::new(0, 0), Size::new(w, 1)),
        desktop: Rect::from_origin_size(Point::new(0, 1), Size::new(w, (h - 2).max(0))),
        status: Rect::from_origin_size(Point::new(0, (h - 1).max(0)), Size::new(w, 1)),
    }
}

impl Shell {
    /// Assembles a shell for a terminal of `size` from its three chrome pieces,
    /// positioning each to the matching region. `theme` is kept to build a
    /// [`ContextMenu`] on demand (ADR 0019); the three chrome pieces have
    /// already resolved their own styles from it.
    ///
    /// Every `status_line` item's [`Accelerator`](crate::command::Accelerator)
    /// is fed into `desktop`'s global accelerator table (ADR 0028) — building
    /// a `StatusItem`, as before, is enough to get a working shortcut. A
    /// shortcut that shouldn't take a status-line slot at all still works:
    /// call [`desktop_mut`](Self::desktop_mut)`().bind_accelerator(...)`
    /// directly, with no `StatusItem` involved.
    pub fn new(
        size: Size,
        menu_bar: MenuBar,
        mut desktop: Desktop,
        status_line: StatusLine,
        theme: &Theme,
    ) -> Self {
        for accelerator in status_line.accelerators() {
            desktop.bind_accelerator(accelerator);
        }
        let mut shell = Self {
            menu_bar,
            desktop,
            status_line,
            size,
            theme: theme.clone(),
            context_menu: None,
            help: None,
            help_window: None,
        };
        shell.relayout(size);
        shell
    }

    /// Whether a menu pull-down is currently open (the menu bar runs modally then).
    pub fn menu_is_open(&self) -> bool {
        self.menu_bar.is_open()
    }

    /// A mutable reference to the desktop — the only way in from outside the
    /// view tree for application code to dynamically open/close/hide/show
    /// windows (ADR 0016), since `Shell` owns it by value.
    pub fn desktop_mut(&mut self) -> &mut Desktop {
        &mut self.desktop
    }

    /// Opts `Shell` into handling `CM_HELP` itself (ADR 0021): it resolves
    /// the active window's help topic and opens a singleton `HelpWindow`
    /// there. Without this, `CM_HELP` falls through to the desktop exactly
    /// like any other unrecognised command — zero cost, zero behaviour
    /// change.
    pub fn with_help(mut self, contents: HelpContents) -> Self {
        self.help = Some(contents);
        self
    }

    /// Resolves the active window's help topic (falling back to home with no
    /// active window, or one with no `help_topic`) and (re)opens the
    /// singleton help window there (ADR 0021). Only called from
    /// `handle_event` once `self.help` is confirmed `Some`; `contents` is the
    /// caller's clone of it, handed in rather than re-read to keep this
    /// function total over its inputs.
    fn open_help(&mut self, contents: HelpContents, ctx: &mut Context) {
        let topic = self
            .desktop
            .active_id()
            .and_then(|id| self.desktop.window(id))
            .and_then(Window::help_topic)
            .map(str::to_string);
        let area = self.desktop.bounds();
        let mut window = match &topic {
            Some(t) => HelpWindow::build_at(contents, area, "Help", &self.theme, t),
            None => HelpWindow::build(contents, area, "Help", &self.theme),
        };
        // The singleton: reuse the old window's position/size if one was
        // open, then close it — a fresh one just opens centred instead.
        if let Some(old_id) = self.help_window {
            if let Some(old) = self.desktop.window(old_id) {
                window.set_bounds(old.bounds());
            }
            self.desktop.close(old_id, ctx);
        }
        self.help_window = Some(self.desktop.open(window));
    }

    /// Repositions the three children for a terminal of `size`.
    fn relayout(&mut self, size: Size) {
        self.size = size;
        let r = regions(size);
        self.menu_bar.set_bounds(r.menu);
        self.desktop.set_bounds(r.desktop);
        self.status_line.set_bounds(r.status);
    }

    /// Three-pass key routing (ADR 0009): menu bar → active window → status line.
    /// An open context menu takes absolute priority over all three, exactly
    /// mirroring the menu bar's own open-pull-down modality (ADR 0019).
    fn handle_key(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        if let Some(menu) = &mut self.context_menu {
            let result = menu.handle_event(event, self.size, ctx);
            if menu.is_closed() {
                self.context_menu = None;
            }
            return result;
        }
        self.menu_bar
            .handle_event(event, ctx)
            .or_else(|| self.desktop.handle_event(event, ctx))
            .or_else(|| self.status_line.handle_event(event, ctx))
    }

    /// Positional routing: the region under the pointer, in that region's local
    /// coordinates. (Behaviour inside each region is mostly Phase 9; the seam is
    /// here from the start — ADR 0007.)
    ///
    /// **An open pull-down or context menu is the exception.** Both draw as a
    /// full-frame overlay below the bar's own one-row bounds (ADR 0009), so
    /// the region carve-up below would hand a click on an item to the desktop
    /// underneath it instead — the same "claim everything while open"
    /// modality `handle_key` already gives them for keys. A fresh right-click
    /// while a context menu is open always supersedes it, though: closing it
    /// first and falling through to ordinary dispatch lets whatever sits
    /// under the new point offer a fresh one in the same step (ADR 0019).
    fn handle_mouse(&mut self, mouse: MouseEvent, ctx: &mut Context) -> EventResult {
        if let Some(menu) = &mut self.context_menu {
            if matches!(mouse.kind, MouseKind::Down(MouseButton::Right)) {
                self.context_menu = None;
            } else {
                let result = menu.handle_event(&Event::Mouse(mouse), self.size, ctx);
                if menu.is_closed() {
                    self.context_menu = None;
                }
                return result;
            }
        }

        let result = if self.menu_bar.is_open() {
            self.menu_bar.handle_event(&Event::Mouse(mouse), ctx)
        } else {
            let mut result = EventResult::Ignored;
            let r = regions(self.size);
            for (region, target) in [
                (r.menu, &mut self.menu_bar as &mut dyn View),
                (r.status, &mut self.status_line as &mut dyn View),
                (r.desktop, &mut self.desktop as &mut dyn View),
            ] {
                if region.contains(mouse.pos) {
                    let origin = region.origin();
                    let local = MouseEvent {
                        pos: mouse.pos.offset(-origin.x, -origin.y),
                        ..mouse
                    };
                    result = ctx.translated(origin.x, origin.y, |ctx| {
                        target.handle_event(&Event::Mouse(local), ctx)
                    });
                    break;
                }
            }
            result
        };

        if let Some(req) = ctx.take_context_menu_request() {
            self.context_menu = Some(ContextMenu::new(
                req.menu,
                req.at,
                &self.theme,
                ctx.commands(),
            ));
        }
        result
    }
}

impl View for Shell {
    fn bounds(&self) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), self.size)
    }

    fn draw(&self, canvas: &mut Canvas) {
        let r = regions(canvas.size());
        // Each child draws through a sub-canvas scoped so its borrow ends before
        // the next one reborrows the frame.
        self.desktop.draw(&mut canvas.child(r.desktop));
        self.status_line.draw(&mut canvas.child(r.status));
        self.menu_bar.draw(&mut canvas.child(r.menu));
        // The open pull-down is the last thing drawn, over the whole frame, so it
        // sits on top of the desktop below the bar (ADR 0009). An open context
        // menu draws after that, so it stacks on top on the rare occasion both
        // are open (ADR 0019).
        self.menu_bar.draw_overlay(canvas);
        if let Some(menu) = &self.context_menu {
            menu.draw_overlay(canvas);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Key(_) => self.handle_key(event, ctx),
            Event::Mouse(mouse) => self.handle_mouse(*mouse, ctx),
            // A re-dispatched command (ADR 0003) goes to the active window; CM_QUIT
            // never reaches here — `Root` claims it before re-dispatch.
            // `CM_HELP` is the one exception, caught here rather than left to
            // fall through, whenever `help` is `Some` (ADR 0021) — see
            // `open_help`.
            Event::Command(command) => {
                if *command == CM_HELP {
                    if let Some(contents) = self.help.clone() {
                        self.open_help(contents, ctx);
                        return EventResult::Consumed;
                    }
                }
                self.desktop.handle_event(event, ctx)
            }
            // A paste goes to the active window, like a key (ADR 0012).
            Event::Paste(_) => self.desktop.handle_event(event, ctx),
            Event::Resize(size) => {
                self.relayout(*size);
                self.desktop.handle_event(event, ctx);
                EventResult::Ignored
            }
            Event::Broadcast(_) | Event::Idle => {
                self.menu_bar.handle_event(event, ctx);
                self.desktop.handle_event(event, ctx);
                self.status_line.handle_event(event, ctx);
                EventResult::Ignored
            }
        }
    }

    fn focusable(&self) -> bool {
        true
    }

    fn valid(&mut self, command: Command, ctx: &mut Context) -> bool {
        // Delegates to the desktop's own fan-out (ADR 0016) so Root's
        // CM_QUIT gate reaches every open window, not just the active one,
        // without Root needing to know Shell or Desktop exist at all.
        self.desktop.valid(command, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::backend::TestBackend;
    use crate::cell::Cell;
    use crate::color::Style;
    use crate::command::{Accelerator, CM_CANCEL, CM_OK, CM_USER, Command};
    use crate::event::{KeyCode, KeyEvent, Modifiers, MouseButton, MouseEvent, MouseKind};
    use crate::geometry::{Point, Rect, Size};
    use crate::theme::{Role, Theme};
    use crate::view::StaticText;
    use crate::widgets::{Menu, MenuItem, StatusItem, Window};
    use std::cell::RefCell;
    use std::collections::VecDeque;
    use std::rc::Rc;

    /// A headless terminal for driving the loop: delegates output to a
    /// [`TestBackend`] and replays a script of poll results. Each `poll_event`
    /// pops the next scripted result; a `Resize` updates the reported size, the
    /// way the real backend does. The script running dry is an error rather than a
    /// silent idle, so a loop that fails to finish surfaces as a failed test
    /// instead of a hang.
    struct ScriptedTerminal {
        backend: TestBackend,
        size: Size,
        script: VecDeque<Option<Event>>,
    }

    impl ScriptedTerminal {
        fn new(size: Size, script: Vec<Option<Event>>) -> Self {
            Self {
                backend: TestBackend::new(size),
                size,
                script: script.into_iter().collect(),
            }
        }

        fn screen_text(&self) -> String {
            self.backend.to_text()
        }

        fn screen(&self) -> &Buffer {
            self.backend.screen()
        }

        fn presents(&self) -> usize {
            self.backend.presents()
        }
    }

    impl Backend for ScriptedTerminal {
        fn size(&self) -> Size {
            self.size
        }

        fn present(&mut self, frame: &Buffer) -> io::Result<()> {
            self.backend.present(frame)
        }
    }

    impl EventSource for ScriptedTerminal {
        fn poll_event(&mut self, _timeout: Duration) -> io::Result<Option<Event>> {
            match self.script.pop_front() {
                Some(result) => {
                    if let Some(Event::Resize(size)) = result {
                        self.size = size;
                    }
                    Ok(result)
                }
                None => Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "scripted terminal ran out of events before the program finished",
                )),
            }
        }
    }

    /// A program that records what it sees, paints `HI`, and quits on Ctrl-Q.
    #[derive(Default)]
    struct Recorder {
        seen: Vec<Event>,
        draw_sizes: Vec<Size>,
        finished: bool,
    }

    impl Program for Recorder {
        fn draw(&mut self, frame: &mut Buffer) {
            self.draw_sizes.push(frame.size());
            frame.put_str(Point::new(0, 0), "HI", Style::new());
        }

        fn handle_event(&mut self, event: &Event) -> EventResult {
            self.seen.push(event.clone());
            if let Event::Key(KeyEvent {
                code: KeyCode::Char('q'),
                modifiers,
            }) = event
            {
                if modifiers.contains(Modifiers::CONTROL) {
                    self.finished = true;
                    return EventResult::Consumed;
                }
            }
            EventResult::Ignored
        }

        fn is_finished(&self) -> bool {
            self.finished
        }
    }

    fn ctrl_q() -> Event {
        Event::Key(KeyEvent::new(KeyCode::Char('q'), Modifiers::CONTROL))
    }

    #[test]
    fn draws_then_quits_on_the_quit_key() {
        let terminal = ScriptedTerminal::new(Size::new(6, 2), vec![Some(ctrl_q())]);
        let mut app = Application::new(terminal);
        let mut program = Recorder::default();

        app.run(&mut program).unwrap();

        assert!(program.finished);
        assert_eq!(program.seen, vec![ctrl_q()]);
        // The program's drawing reached the screen, and we presented at least once.
        assert!(app.terminal().screen_text().starts_with("HI"));
        assert!(app.terminal().presents() >= 1);
    }

    #[test]
    fn a_timed_out_poll_delivers_one_idle() {
        // `None` is a poll timeout; the loop turns it into exactly one Idle.
        let terminal = ScriptedTerminal::new(Size::new(6, 2), vec![None, Some(ctrl_q())]);
        let mut app = Application::new(terminal);
        let mut program = Recorder::default();

        app.run(&mut program).unwrap();

        assert_eq!(program.seen, vec![Event::Idle, ctrl_q()]);
    }

    #[test]
    fn a_resize_changes_the_next_draw_size() {
        let terminal = ScriptedTerminal::new(
            Size::new(6, 2),
            vec![Some(Event::Resize(Size::new(10, 3))), Some(ctrl_q())],
        );
        let mut app = Application::new(terminal);
        let mut program = Recorder::default();

        app.run(&mut program).unwrap();

        // First draw at the initial size, then at the resized size.
        assert_eq!(program.draw_sizes, vec![Size::new(6, 2), Size::new(10, 3)]);
        assert_eq!(program.seen[0], Event::Resize(Size::new(10, 3)));
    }

    // --- Root: the view-tree bridge to the loop (Phase 3) ---

    /// A focusable leaf for driving `Root`: posts `command` when it sees `on_key`,
    /// and records every command it is handed (without consuming it, so a
    /// re-dispatched command bubbles back out).
    struct Poster {
        bounds: Rect,
        on_key: KeyCode,
        command: Command,
        received: Rc<RefCell<Vec<Command>>>,
    }

    impl View for Poster {
        fn bounds(&self) -> Rect {
            self.bounds
        }

        fn draw(&self, canvas: &mut Canvas) {
            canvas.put_str(Point::new(0, 0), "P", Style::new());
        }

        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            match event {
                Event::Key(key) if key.code == self.on_key => {
                    ctx.post(self.command);
                    EventResult::Consumed
                }
                Event::Command(command) => {
                    self.received.borrow_mut().push(*command);
                    EventResult::Ignored
                }
                _ => EventResult::Ignored,
            }
        }

        fn focusable(&self) -> bool {
            true
        }
    }

    fn full(size: Size) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), size)
    }

    #[test]
    fn root_draws_its_view_through_a_canvas() {
        let mut root = Root::new(Box::new(StaticText::new(
            full(Size::new(8, 1)),
            "hello",
            Style::new(),
        )));
        let mut frame = Buffer::new(Size::new(8, 1));
        root.draw(&mut frame);
        assert_eq!(frame.to_text(), "hello   ");
    }

    #[test]
    fn posting_cm_quit_finishes_the_root() {
        // The view posts CM_QUIT on Ctrl-Q; Root drains it and ends the loop —
        // the real command path replacing Phase 2's quit flag.
        let mut root = Root::new(Box::new(Poster {
            bounds: full(Size::new(10, 3)),
            on_key: KeyCode::Char('q'),
            command: CM_QUIT,
            received: Rc::new(RefCell::new(Vec::new())),
        }));
        assert!(!root.is_finished());
        root.handle_event(&Event::Key(KeyEvent::new(
            KeyCode::Char('q'),
            Modifiers::NONE,
        )));
        assert!(root.is_finished());
    }

    #[test]
    fn a_posted_command_is_redispatched_into_the_tree() {
        // The view posts an app command on Enter; Root re-dispatches it from the
        // top, so the tree sees it come back as an Event::Command.
        let app_cmd = Command(CM_USER + 1);
        let received = Rc::new(RefCell::new(Vec::new()));
        let mut root = Root::new(Box::new(Poster {
            bounds: full(Size::new(10, 3)),
            on_key: KeyCode::Enter,
            command: app_cmd,
            received: Rc::clone(&received),
        }));

        root.handle_event(&Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)));
        assert_eq!(*received.borrow(), vec![app_cmd]);
        assert!(!root.is_finished(), "an app command must not end the loop");
    }

    #[test]
    fn a_disabled_command_is_neither_posted_nor_redispatched() {
        let app_cmd = Command(CM_USER + 2);
        let received = Rc::new(RefCell::new(Vec::new()));
        let mut commands = CommandSet::new();
        commands.disable(app_cmd);
        let mut root = Root::new(Box::new(Poster {
            bounds: full(Size::new(10, 3)),
            on_key: KeyCode::Enter,
            command: app_cmd,
            received: Rc::clone(&received),
        }))
        .with_commands(commands);

        root.handle_event(&Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)));
        assert!(
            received.borrow().is_empty(),
            "a disabled command never fires, so nothing is re-dispatched"
        );
    }

    #[test]
    fn a_refusing_root_view_leaves_cm_quit_unfinished() {
        // Root's CM_QUIT gate (ADR 0016) — a view that vetoes leaves the loop
        // running; one that agrees lets it through.
        struct Gate {
            refuses: bool,
        }
        impl View for Gate {
            fn bounds(&self) -> Rect {
                full(Size::new(4, 1))
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
                if matches!(event, Event::Key(k) if k.code == KeyCode::Char('q')) {
                    ctx.post(CM_QUIT);
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            fn valid(&mut self, command: Command, _ctx: &mut Context) -> bool {
                command != CM_QUIT || !self.refuses
            }
        }
        let quit_key = Event::Key(KeyEvent::new(KeyCode::Char('q'), Modifiers::NONE));

        let mut refusing = Root::new(Box::new(Gate { refuses: true }));
        refusing.handle_event(&quit_key);
        assert!(!refusing.is_finished(), "a refusal leaves the loop running");

        let mut agreeing = Root::new(Box::new(Gate { refuses: false }));
        agreeing.handle_event(&quit_key);
        assert!(agreeing.is_finished(), "agreeing lets CM_QUIT through");
    }

    #[test]
    fn application_runs_the_root_until_a_view_posts_cm_quit() {
        // End-to-end through the real loop: a key reaches the focused view, which
        // posts CM_QUIT; Root finishes; the loop exits after presenting.
        let quit_key = Event::Key(KeyEvent::new(KeyCode::Char('q'), Modifiers::CONTROL));
        let terminal = ScriptedTerminal::new(Size::new(8, 1), vec![Some(quit_key)]);
        let mut app = Application::new(terminal);
        let mut root = Root::new(Box::new(Poster {
            bounds: full(Size::new(8, 1)),
            on_key: KeyCode::Char('q'),
            command: CM_QUIT,
            received: Rc::new(RefCell::new(Vec::new())),
        }));

        app.run(&mut root).unwrap();

        assert!(root.is_finished());
        assert!(app.terminal().presents() >= 1);
        assert!(app.terminal().screen_text().starts_with('P'));
    }

    // --- Shell: the TProgram-style application root (Phase 4) ---

    const CM_PING: Command = Command(CM_USER + 10);

    /// Builds a shell with one window whose interior posts [`CM_PING`] on `a`, a
    /// File/Edit menu bar, and an F1/Alt-X status line.
    fn shell(size: Size, received: Rc<RefCell<Vec<Command>>>) -> Shell {
        use crate::theme::Role;
        let theme = Theme::default();
        let (w, h) = (size.width, size.height);

        let menu_bar = MenuBar::new(
            full(Size::new(w, 1)),
            vec![
                Menu::new("File", vec![MenuItem::new("Exit", CM_QUIT)]),
                Menu::new("Edit", vec![MenuItem::new("Copy", Command(CM_USER + 20))]),
            ],
            &theme,
        );
        let window = Window::new(
            Rect::from_origin_size(Point::new(2, 1), Size::new(30, 6)),
            "Untitled",
            &theme,
            Box::new(Poster {
                bounds: full(Size::new(28, 4)),
                on_key: KeyCode::Char('a'),
                command: CM_PING,
                received,
            }),
        );
        let mut desktop = Desktop::new(
            Rect::from_origin_size(Point::new(0, 1), Size::new(w, h - 2)),
            Cell::from_char('░', theme.style(Role::DesktopBackground)),
        );
        desktop.open(window);
        let status = StatusLine::new(
            Rect::from_origin_size(Point::new(0, h - 1), Size::new(w, 1)),
            vec![
                StatusItem::new(
                    "F1",
                    "Help",
                    Accelerator::new(KeyEvent::new(KeyCode::F(1), Modifiers::NONE), CM_HELP),
                ),
                StatusItem::new(
                    "Alt-X",
                    "Exit",
                    Accelerator::new(KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT), CM_QUIT),
                ),
            ],
            theme.style(Role::StatusBar),
            theme.style(Role::StatusKey),
        );
        Shell::new(size, menu_bar, desktop, status, &theme)
    }

    fn no_log() -> Rc<RefCell<Vec<Command>>> {
        Rc::new(RefCell::new(Vec::new()))
    }

    #[test]
    fn snapshot_shell_composes_the_full_screen() {
        let sh = shell(Size::new(40, 10), no_log());
        let mut frame = Buffer::new(Size::new(40, 10));
        let mut canvas = Canvas::new(&mut frame);
        sh.draw(&mut canvas);
        insta::assert_snapshot!(frame.to_text());
    }

    #[test]
    fn a_plain_key_reaches_the_active_window() {
        let mut sh = shell(Size::new(40, 10), no_log());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // Closed menu: 'a' flows past the bar to the active window, which posts ping.
        sh.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('a'), Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(ctx.posted(), &[Event::Command(CM_PING)]);
    }

    #[test]
    fn an_open_menu_swallows_typing_so_the_window_never_sees_it() {
        let mut sh = shell(Size::new(40, 10), no_log());
        let cs = CommandSet::new();
        sh.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('f'), Modifiers::ALT)),
            &mut Context::new(&cs),
        );
        assert!(sh.menu_is_open());
        // 'a' now goes to the modal menu, not the window: no ping is posted.
        let mut ctx = Context::new(&cs);
        sh.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('a'), Modifiers::NONE)),
            &mut ctx,
        );
        assert!(ctx.posted().is_empty());
        assert!(sh.menu_is_open());
    }

    #[test]
    fn clicking_a_pulldown_item_reaches_the_menu_bar_not_the_desktop_underneath() {
        // The pull-down draws as a full-frame overlay below the bar's own
        // one-row bounds (ADR 0009), so a click on an item lands in what the
        // region carve-up would otherwise treat as the desktop. The menu bar
        // must see it first, in screen coordinates, same as it already does
        // for keys while open.
        let received = Rc::new(RefCell::new(Vec::new()));
        let mut sh = shell(Size::new(40, 10), Rc::clone(&received));
        let cs = CommandSet::new();

        sh.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('f'), Modifiers::ALT)),
            &mut Context::new(&cs),
        );
        assert!(sh.menu_is_open());

        // File's only item, "Exit" (CM_QUIT), is the pull-down's first row —
        // screen (2, 2): box left 0, top 1, one border row above the item.
        let mut ctx = Context::new(&cs);
        sh.handle_event(
            &Event::Mouse(MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                pos: Point::new(2, 2),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        );

        assert_eq!(ctx.posted(), &[Event::Command(CM_QUIT)]);
        assert!(!sh.menu_is_open(), "choosing an item closes the pull-down");
        assert!(
            received.borrow().is_empty(),
            "the click never reached the window underneath"
        );
    }

    #[test]
    fn a_function_key_falls_through_to_the_status_line() {
        let mut sh = shell(Size::new(40, 10), no_log());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // The window's interior ignores F1, so the post-process status line claims it.
        sh.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::F(1), Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(ctx.posted(), &[Event::Command(CM_HELP)]);
    }

    #[test]
    fn shell_new_feeds_status_line_accelerators_into_the_desktop() {
        // Proves the harvest happened *into Desktop's own table* at
        // construction (ADR 0028), not merely "works when routed through
        // Shell" — dispatch directly at the desktop, bypassing Shell::handle_event.
        let mut sh = shell(Size::new(40, 10), no_log());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let result = sh.desktop_mut().handle_event(
            &Event::Key(KeyEvent::new(KeyCode::F(1), Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_HELP)]);
    }

    #[test]
    fn bind_accelerator_works_with_no_matching_status_item() {
        // The direct regression shape of the reported bug: a shortcut with
        // no StatusLine slot at all still fires, once bound straight onto
        // the desktop (ADR 0028) — no StatusItem involved.
        use crate::theme::Role;
        let theme = Theme::default();
        let size = Size::new(40, 10);
        let (w, h) = (size.width, size.height);
        let menu_bar = MenuBar::new(full(Size::new(w, 1)), vec![], &theme);
        let mut desktop = Desktop::new(
            Rect::from_origin_size(Point::new(0, 1), Size::new(w, h - 2)),
            Cell::from_char('░', theme.style(Role::DesktopBackground)),
        );
        desktop.open(Window::new(
            Rect::from_origin_size(Point::new(2, 1), Size::new(20, 4)),
            "Untitled",
            &theme,
            Box::new(StaticText::new(full(Size::new(18, 2)), "", Style::new())),
        ));
        let status = StatusLine::new(
            Rect::from_origin_size(Point::new(0, h - 1), Size::new(w, 1)),
            vec![],
            theme.style(Role::StatusBar),
            theme.style(Role::StatusKey),
        );
        let mut sh = Shell::new(size, menu_bar, desktop, status, &theme);
        let hidden = Command(CM_USER + 30);
        sh.desktop_mut().bind_accelerator(Accelerator::new(
            KeyEvent::new(KeyCode::Char('a'), Modifiers::CONTROL),
            hidden,
        ));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        sh.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Char('a'), Modifiers::CONTROL)),
            &mut ctx,
        );
        assert_eq!(ctx.posted(), &[Event::Command(hidden)]);
    }

    #[test]
    fn resize_relays_out_the_chrome() {
        let mut sh = shell(Size::new(20, 6), no_log());
        sh.handle_event(
            &Event::Resize(Size::new(30, 8)),
            &mut Context::new(&CommandSet::new()),
        );
        assert_eq!(sh.size, Size::new(30, 8));
        // Drawn at the new size, the status line lands on the new bottom row (7).
        let mut frame = Buffer::new(Size::new(30, 8));
        let mut canvas = Canvas::new(&mut frame);
        sh.draw(&mut canvas);
        let text = frame.to_text();
        let rows: Vec<&str> = text.lines().collect();
        assert!(rows[7].trim_start().starts_with("F1 Help"));
    }

    #[test]
    fn application_runs_the_shell_until_a_status_key_posts_cm_quit() {
        // End-to-end: Alt-X reaches the status line (the window ignores it), which
        // posts CM_QUIT; Root claims it and the loop exits.
        let alt_x = Event::Key(KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT));
        let terminal = ScriptedTerminal::new(Size::new(40, 10), vec![Some(alt_x)]);
        let mut app = Application::new(terminal);
        let mut root = Root::new(Box::new(shell(Size::new(40, 10), no_log())));

        app.run(&mut root).unwrap();

        assert!(root.is_finished());
        assert!(app.terminal().presents() >= 1);
    }

    #[test]
    fn shell_valid_delegates_to_the_desktop() {
        // Regression: Shell must override valid() to forward to its Desktop,
        // or Root's CM_QUIT gate silently never reaches any open window
        // (ADR 0016) — the default View::valid is unconditionally true.
        struct Vetoer {
            refuses: bool,
        }
        impl View for Vetoer {
            fn bounds(&self) -> Rect {
                full(Size::new(4, 1))
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn valid(&mut self, command: Command, _ctx: &mut Context) -> bool {
                command != CM_QUIT || !self.refuses
            }
        }
        let theme = Theme::default();
        let size = Size::new(20, 8);
        let menu_bar = MenuBar::new(full(Size::new(20, 1)), vec![], &theme);
        let mut desktop = Desktop::new(full(Size::new(20, 6)), Cell::default());
        desktop.open(Window::new(
            Rect::from_origin_size(Point::new(0, 0), Size::new(10, 4)),
            "W",
            &theme,
            Box::new(Vetoer { refuses: true }),
        ));
        let status = StatusLine::new(
            full(Size::new(20, 1)),
            vec![],
            theme.style(Role::StatusBar),
            theme.style(Role::StatusKey),
        );
        let mut shell = Shell::new(size, menu_bar, desktop, status, &theme);

        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(
            !shell.valid(CM_QUIT, &mut ctx),
            "the open window vetoes quitting"
        );
        assert!(
            shell.valid(CM_OK, &mut ctx),
            "the window only refuses CM_QUIT, so an unrelated command passes"
        );
    }

    // --- CM_HELP handling (ADR 0021) ---

    fn help_contents() -> HelpContents {
        HelpContents::parse("@topic home Home\nHome body.\n\n@topic other Other\nOther body.")
    }

    fn blank_interior() -> Box<dyn View> {
        Box::new(StaticText::new(full(Size::new(1, 1)), "", Style::new()))
    }

    /// A shell with one window (interior: `blank_interior`, or `interior` if
    /// given) and, unless `help` is `None`, opted into `CM_HELP` handling.
    fn shell_for_help(size: Size, help: Option<HelpContents>, window: Window) -> Shell {
        let theme = Theme::default();
        let menu_bar = MenuBar::new(full(Size::new(size.width, 1)), vec![], &theme);
        let mut desktop = Desktop::new(
            Rect::from_origin_size(Point::new(0, 1), Size::new(size.width, size.height - 2)),
            Cell::default(),
        );
        desktop.open(window);
        let status = StatusLine::new(
            Rect::from_origin_size(Point::new(0, size.height - 1), Size::new(size.width, 1)),
            vec![],
            theme.style(Role::StatusBar),
            theme.style(Role::StatusKey),
        );
        let mut shell = Shell::new(size, menu_bar, desktop, status, &theme);
        if let Some(contents) = help {
            shell = shell.with_help(contents);
        }
        shell
    }

    #[test]
    fn cm_help_falls_through_to_the_desktop_when_help_is_not_configured() {
        let received = Rc::new(RefCell::new(Vec::new()));
        let window = Window::new(
            Rect::from_origin_size(Point::new(0, 0), Size::new(10, 4)),
            "W",
            &Theme::default(),
            Box::new(Poster {
                bounds: full(Size::new(8, 2)),
                on_key: KeyCode::Char('x'), // unused: this test drives Command directly
                command: Command(CM_USER + 40),
                received: Rc::clone(&received),
            }),
        );
        let mut sh = shell_for_help(Size::new(40, 10), None, window);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        sh.handle_event(&Event::Command(CM_HELP), &mut ctx);
        assert_eq!(
            *received.borrow(),
            vec![CM_HELP],
            "with no help configured, CM_HELP reaches the desktop like any other command"
        );
    }

    #[test]
    fn cm_help_opens_a_help_window_on_the_active_windows_topic() {
        let window = Window::new(
            Rect::from_origin_size(Point::new(0, 0), Size::new(10, 4)),
            "W",
            &Theme::default(),
            blank_interior(),
        )
        .with_help_topic("other");
        let mut sh = shell_for_help(Size::new(40, 10), Some(help_contents()), window);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        sh.handle_event(&Event::Command(CM_HELP), &mut ctx);

        let mut frame = Buffer::new(Size::new(40, 10));
        let mut canvas = Canvas::new(&mut frame);
        sh.draw(&mut canvas);
        let text = frame.to_text();
        assert!(text.contains("Other body."), "opened on the resolved topic");
    }

    #[test]
    fn cm_help_falls_back_to_home_with_no_active_window() {
        // An empty desktop: active_id() is None, so the topic lookup itself
        // has nothing to read — home is the only sensible fallback.
        let theme = Theme::default();
        let menu_bar = MenuBar::new(full(Size::new(40, 1)), vec![], &theme);
        let desktop = Desktop::new(
            Rect::from_origin_size(Point::new(0, 1), Size::new(40, 8)),
            Cell::default(),
        );
        let status = StatusLine::new(
            Rect::from_origin_size(Point::new(0, 9), Size::new(40, 1)),
            vec![],
            theme.style(Role::StatusBar),
            theme.style(Role::StatusKey),
        );
        let mut sh = Shell::new(Size::new(40, 10), menu_bar, desktop, status, &theme)
            .with_help(help_contents());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        sh.handle_event(&Event::Command(CM_HELP), &mut ctx);

        let mut frame = Buffer::new(Size::new(40, 10));
        let mut canvas = Canvas::new(&mut frame);
        sh.draw(&mut canvas);
        assert!(frame.to_text().contains("Home body."));
    }

    #[test]
    fn a_second_cm_help_reopens_the_singleton_preserving_bounds_and_switching_topic() {
        let size = Size::new(60, 20);
        let theme = Theme::default();
        let menu_bar = MenuBar::new(full(Size::new(size.width, 1)), vec![], &theme);
        let mut desktop = Desktop::new(
            Rect::from_origin_size(Point::new(0, 1), Size::new(size.width, size.height - 2)),
            Cell::default(),
        );
        let a = desktop.open(
            Window::new(
                Rect::from_origin_size(Point::new(0, 0), Size::new(10, 4)),
                "A",
                &theme,
                blank_interior(),
            )
            .with_help_topic("home"),
        );
        let b = desktop.open(
            Window::new(
                Rect::from_origin_size(Point::new(12, 0), Size::new(10, 4)),
                "B",
                &theme,
                blank_interior(),
            )
            .with_help_topic("other"),
        );
        let status = StatusLine::new(
            Rect::from_origin_size(Point::new(0, size.height - 1), Size::new(size.width, 1)),
            vec![],
            theme.style(Role::StatusBar),
            theme.style(Role::StatusKey),
        );
        let mut sh = Shell::new(size, menu_bar, desktop, status, &theme).with_help(help_contents());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        sh.desktop_mut().focus(a);
        sh.handle_event(&Event::Command(CM_HELP), &mut ctx);
        let help_id_1 = sh.desktop_mut().active_id().unwrap();
        let bounds_1 = sh.desktop_mut().window(help_id_1).unwrap().bounds();

        // Simulate the user having dragged the help window elsewhere.
        let moved = Rect::from_origin_size(Point::new(5, 3), bounds_1.size());
        sh.desktop_mut()
            .window_mut(help_id_1)
            .unwrap()
            .set_bounds(moved);

        sh.desktop_mut().focus(b);
        sh.handle_event(&Event::Command(CM_HELP), &mut ctx);
        let help_id_2 = sh.desktop_mut().active_id().unwrap();

        assert_ne!(
            help_id_1, help_id_2,
            "closed and reopened, not retargeted in place"
        );
        assert!(
            sh.desktop_mut().window(help_id_1).is_none(),
            "the old help window is gone"
        );
        assert_eq!(
            sh.desktop_mut().window(help_id_2).unwrap().bounds(),
            moved,
            "position/size carried over from the closed one"
        );

        let mut frame = Buffer::new(size);
        let mut canvas = Canvas::new(&mut frame);
        sh.draw(&mut canvas);
        let text = frame.to_text();
        assert!(
            text.contains("Other body."),
            "shows the newly resolved topic"
        );
        assert!(!text.contains("Home body."), "not still on the old one");
    }

    // --- Context menu anchor propagation (ADR 0019) ---

    /// An interior that offers a context menu anchored at its own local
    /// right-click position.
    struct Offerer;

    impl View for Offerer {
        fn bounds(&self) -> Rect {
            full(Size::new(100, 100))
        }
        fn draw(&self, _canvas: &mut Canvas) {}
        fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
            if let Event::Mouse(mouse) = event {
                if mouse.kind == MouseKind::Down(MouseButton::Right) {
                    let menu = Menu::new("M", vec![MenuItem::new("Ping", Command(CM_USER + 30))]);
                    ctx.open_context_menu(menu, mouse.pos);
                    return EventResult::Consumed;
                }
            }
            EventResult::Ignored
        }
    }

    /// A shell with one window (at (2, 1), sized 20x8) whose interior offers
    /// a one-item context menu on right-click, anchored at the click.
    fn shell_with_offerer(size: Size) -> Shell {
        let theme = Theme::default();
        let menu_bar = MenuBar::new(full(Size::new(size.width, 1)), vec![], &theme);
        let mut desktop = Desktop::new(
            Rect::from_origin_size(Point::new(0, 1), Size::new(size.width, size.height - 2)),
            Cell::default(),
        );
        desktop.open(Window::new(
            Rect::from_origin_size(Point::new(2, 1), Size::new(20, 8)),
            "W",
            &theme,
            Box::new(Offerer),
        ));
        let status = StatusLine::new(
            Rect::from_origin_size(Point::new(0, size.height - 1), Size::new(size.width, 1)),
            vec![],
            theme.style(Role::StatusBar),
            theme.style(Role::StatusKey),
        );
        Shell::new(size, menu_bar, desktop, status, &theme)
    }

    fn right_click_at(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Right),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    fn left_click_at(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    /// Whether a box-drawing corner sits at `(x, y)` in `shell`'s drawn
    /// frame — the only externally observable sign a `ContextMenu` is open,
    /// since `Shell.context_menu` has no public accessor (it is
    /// Shell-internal, like `MenuBar`'s own open pull-down).
    fn corner_at(shell: &Shell, size: Size, x: i16, y: i16) -> bool {
        let mut frame = Buffer::new(size);
        let mut canvas = Canvas::new(&mut frame);
        shell.draw(&mut canvas);
        frame.get(Point::new(x, y)).unwrap().grapheme().to_string() == "┌"
    }

    #[test]
    fn a_right_click_nested_inside_a_window_inside_a_desktop_resolves_to_true_screen_coordinates() {
        // The regression this feature exists to prove: a request from a view
        // nested Shell -> Desktop -> Window -> interior must resolve to the
        // click's actual screen position, not some partially-translated
        // local point (ADR 0019). Window at (2, 1) sized 20x8; its interior
        // (inset one cell each side) spans screen-absolute (3, 2)..(21, 8).
        let size = Size::new(40, 10);
        let mut shell = shell_with_offerer(size);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // Screen (10, 5): inside the desktop region (rows 1..8), inside the
        // window, inside its interior.
        shell.handle_event(&right_click_at(10, 5), &mut ctx);
        assert!(
            corner_at(&shell, size, 10, 5),
            "the context menu's box is anchored at the click's true screen \
             position, not a partially-translated local one"
        );
    }

    #[test]
    fn clicking_the_offered_item_posts_its_command_and_closes_the_menu() {
        let size = Size::new(40, 10);
        let mut shell = shell_with_offerer(size);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        shell.handle_event(&right_click_at(10, 5), &mut ctx);
        assert!(corner_at(&shell, size, 10, 5));

        // The box is 2 rows tall (border, one item, border); its item row
        // sits one below the anchor, starting two columns in.
        shell.handle_event(&left_click_at(12, 6), &mut ctx);
        assert_eq!(ctx.posted(), &[Event::Command(Command(CM_USER + 30))]);
        assert!(
            !corner_at(&shell, size, 10, 5),
            "choosing the item closed the menu"
        );
    }

    #[test]
    fn clicking_elsewhere_dismisses_the_menu_without_reaching_the_window() {
        let size = Size::new(40, 10);
        let mut shell = shell_with_offerer(size);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        shell.handle_event(&right_click_at(10, 5), &mut ctx);
        assert!(corner_at(&shell, size, 10, 5));

        shell.handle_event(&left_click_at(30, 5), &mut ctx);
        assert!(
            !corner_at(&shell, size, 10, 5),
            "a click off the box dismisses it"
        );
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn a_second_right_click_replaces_the_open_menu() {
        let size = Size::new(40, 10);
        let mut shell = shell_with_offerer(size);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        shell.handle_event(&right_click_at(10, 5), &mut ctx);
        assert!(corner_at(&shell, size, 10, 5));

        shell.handle_event(&right_click_at(20, 6), &mut ctx);
        assert!(!corner_at(&shell, size, 10, 5), "the first menu is gone");
        assert!(
            corner_at(&shell, size, 20, 6),
            "a fresh one opened at the new click"
        );
    }

    // --- exec_view: the modal dialog loop (Phase 5, ADR 0010) ---

    /// A background program that just paints `BG` at the origin (outside any
    /// centred dialog), so a test can confirm the background was drawn underneath.
    #[derive(Default)]
    struct Backdrop;

    impl Program for Backdrop {
        fn draw(&mut self, frame: &mut Buffer) {
            frame.put_str(Point::new(0, 0), "BG", Style::new());
        }
        fn handle_event(&mut self, _event: &Event) -> EventResult {
            EventResult::Ignored
        }
        fn is_finished(&self) -> bool {
            false
        }
    }

    fn message_box() -> crate::widgets::Window {
        crate::widgets::MessageBox::ok_cancel("Confirm", "Proceed?", &Theme::default())
    }

    #[test]
    fn exec_view_returns_ok_when_the_default_button_is_pressed() {
        // Focus starts on the OK button; Enter activates it and ends the loop.
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE));
        let terminal = ScriptedTerminal::new(Size::new(40, 12), vec![Some(enter)]);
        let mut app = Application::new(terminal);
        let mut background = Backdrop;
        let mut dialog = message_box();

        let result = app.exec_view(&mut background, &mut dialog).unwrap();
        assert_eq!(result, CM_OK);
        // The background was painted under the centred dialog, and we presented.
        assert!(app.terminal().screen_text().starts_with("BG"));
        assert!(app.terminal().presents() >= 1);
    }

    #[test]
    fn exec_view_casts_a_drop_shadow_under_the_dialog() {
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE));
        let size = Size::new(40, 12);
        let terminal = ScriptedTerminal::new(size, vec![Some(enter)]);
        let mut app = Application::new(terminal);
        let mut background = Backdrop;
        let mut dialog = message_box();
        let area = centered(dialog.bounds().size(), size);

        app.exec_view(&mut background, &mut dialog).unwrap();

        // A cell just past the dialog's right edge, one row below its top, is dimmed
        // to the shadow style — the dialog floats over the background.
        let screen = app.terminal().screen();
        let shadow_cell = screen
            .get(Point::new(area.bottom_right().x, area.origin().y + 1))
            .unwrap();
        assert_eq!(shadow_cell.style(), Theme::default().style(Role::Shadow));
    }

    #[test]
    fn exec_view_returns_cancel_on_esc() {
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
        let terminal = ScriptedTerminal::new(Size::new(40, 12), vec![Some(esc)]);
        let mut app = Application::new(terminal);
        let mut background = Backdrop;
        let mut dialog = message_box();

        let result = app.exec_view(&mut background, &mut dialog).unwrap();
        assert_eq!(result, CM_CANCEL);
    }

    #[test]
    fn exec_view_idles_until_a_button_is_pressed() {
        // A timed-out poll (None ⇒ Idle) keeps the loop alive without ending it;
        // Tab then Enter selects Cancel.
        let tab = Event::Key(KeyEvent::new(KeyCode::Tab, Modifiers::NONE));
        let enter = Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE));
        let terminal = ScriptedTerminal::new(Size::new(40, 12), vec![None, Some(tab), Some(enter)]);
        let mut app = Application::new(terminal);
        let mut background = Backdrop;
        let mut dialog = message_box();

        let result = app.exec_view(&mut background, &mut dialog).unwrap();
        assert_eq!(
            result, CM_CANCEL,
            "Tab moved focus to Cancel, Enter chose it"
        );
    }
}
