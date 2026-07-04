//! Manual check for the dynamic MDI desktop (ADR 0016): open/close/hide/show/
//! focus/cycle windows, drag them by the title bar, resize from the
//! bottom-right corner, and zoom/restore — driven by the real event loop.
//!
//! Run with `cargo run -p rvision --example mdi`. Things to try:
//!
//! - **Window ▸ New Window** opens another window, cascaded from the last.
//! - Drag a window's title bar to move it; drag its bottom-right corner to
//!   resize it. Clicking anywhere on a window raises it to the top.
//! - `Ctrl-N` opens a window, `Ctrl-W` closes the active one, `F5` zooms/
//!   restores it, `F6` cycles focus forward (Window ▸ Next/Previous covers
//!   both directions).
//! - Window ▸ Toggle Toolbox hides/shows a fixed, non-closable toolbox
//!   window docked to the right — `Desktop::hide`/`show`, not `open`/`close`,
//!   since it stays resident either way.
//! - `F1` (or Help ▸ Contents, or a window's own help glyph — the one just
//!   left of its zoom glyph) opens a resizable two-pane `HelpWindow` (ADR
//!   0013/0017), targeted at whatever the *active* window is about (ADR
//!   0021): a document window opens to the Windows topic, the toolbox to its
//!   own Toolbox topic, and F1 with no window active (or one with no help
//!   topic) falls back to the Overview topic. Drag the help window's corner
//!   to resize it and watch both panes relayout live. Reopening while it's
//!   already up closes and reopens it at the newly resolved topic, reusing
//!   its position/size, rather than stacking a second one. The Overview
//!   topic's link is followable (ADR 0020): `Ctrl+Down`/`Ctrl+Up` cycle the
//!   page's current link, `Enter` follows it, or just click it — either
//!   jumps the list and page to the Windows topic.
//! - `Alt-X` (or File ▸ Exit) quits; the terminal is always restored, even on
//!   a panic, thanks to the RAII backend (ADR 0001).

use std::collections::VecDeque;
use std::io;
use std::time::Duration;

use rvision::app::{Application, Program, Shell};
use rvision::backend::Backend;
use rvision::buffer::Buffer;
use rvision::canvas::Canvas;
use rvision::cell::Cell;
use rvision::color::Style;
use rvision::command::{
    Accelerator, CM_CLOSE, CM_HELP, CM_NEXT, CM_PREV, CM_QUIT, CM_USER, CM_ZOOM, Command,
    CommandSet,
};
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult, KeyCode, KeyEvent, Modifiers};
use rvision::geometry::{Point, Rect, Size};
use rvision::help::HelpContents;
use rvision::theme::{Role, Theme};
use rvision::view::{Context, View};
use rvision::widgets::{
    Desktop, Menu, MenuBar, MenuItem, StatusItem, StatusLine, Window, WindowId,
};

// Application command ids, numbered from the framework/app boundary (ADR
// 0003). Everything else this demo triggers (close/zoom/next/prev/help) is a
// framework-reserved command Desktop/Shell themselves already act on.
const CM_NEW_WINDOW: Command = Command(CM_USER + 1);
const CM_TOGGLE_TOOLBOX: Command = Command(CM_USER + 2);

/// A small hand-authored help document (ADR 0013) just to give the demo's
/// `HelpWindow` something real to browse. Each document window and the
/// toolbox carry their own `help_topic` (ADR 0021), so `F1`/the help glyph
/// opens straight to whichever of these is about the active one.
const HELP_SOURCE: &str = "\
@topic overview Overview
This desktop hosts a handful of plain document windows plus a docked
toolbox. Drag a title bar to move a window, or its bottom-right corner
(marked \u{25e2}) to resize it. Clicking anywhere on a window raises it.
See the {Windows|windows} topic for the keyboard shortcuts. F1 (or a
window's own help glyph) opens straight to whichever topic is about the
active window (ADR 0021) — this one is the fallback when nothing more
specific applies.

@topic windows Windows
This is what a document window's own F1/help glyph opens to. Window >
New Window opens another document, cascaded from the last one. Ctrl-W
closes the active window; F5 zooms/restores it; F6 (or Window >
Next/Previous) cycles focus between them.

<pre>
Ctrl+N   New Window
Ctrl+W   Close
F5       Zoom
F6       Next
F9       Toggle Toolbox
</pre>

@topic toolbox Toolbox
This is what the toolbox window's own F1/help glyph opens to instead —
proof that two windows' `F1` can land on two different pages (ADR 0021).
F9 (or Window > Toggle Toolbox) shows or hides it; it stays resident
either way, so its own window state (position, size) survives a hide.

@topic help This Help Window
This window is itself resizable (ADR 0017): drag its corner and watch
the topic list and this page relayout live, independently of each
other. Tab moves focus between the list and the page.
";

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// A window interior that just shows a few lines of placeholder text —
/// stands in for whatever real content an application would host.
struct Info {
    lines: Vec<String>,
    style: Style,
}

impl View for Info {
    fn bounds(&self) -> Rect {
        rect(0, 0, 1, 1) // unused: the window fills the interior canvas
    }

    fn draw(&self, canvas: &mut Canvas) {
        for (row, line) in self.lines.iter().enumerate() {
            canvas.put_str(Point::new(1, row as i16 + 1), line, self.style);
        }
    }
}

/// Drives the demo. Spawning a new window or toggling the toolbox both need
/// a `Window` value, which only application code can build — `Desktop`
/// itself has no idea what a "new" window should contain (ADR 0016) — so
/// this is a small custom [`Program`] rather than the generic [`Root`]
/// (which only knows how to gate `CM_QUIT`, nothing app-specific).
struct Mdi {
    shell: Shell,
    commands: CommandSet,
    theme: Theme,
    finished: bool,
    opened: u32,
    toolbox: WindowId,
}

impl Mdi {
    fn open_new_window(&mut self) {
        self.opened += 1;
        let n = self.opened;
        let cascade = ((n - 1) % 6) as i16;
        let bounds = rect(4 + cascade * 3, 1 + cascade, 36, 10);
        let window = Window::new(
            bounds,
            &format!("Document {n}"),
            &self.theme,
            Box::new(Info {
                lines: vec![
                    format!("Window #{n}"),
                    String::new(),
                    "Drag title bar to move.".to_string(),
                    "Drag corner (◢) to resize.".to_string(),
                    "Click to raise.".to_string(),
                ],
                style: self.theme.style(Role::WindowFrame),
            }),
        )
        .with_help_topic("windows");
        self.shell.desktop_mut().open(window);
    }

    fn toggle_toolbox(&mut self) {
        let desktop = self.shell.desktop_mut();
        let visible = desktop.window(self.toolbox).is_some_and(|w| w.is_visible());
        if visible {
            desktop.hide(self.toolbox);
        } else {
            desktop.show(self.toolbox);
        }
    }

    /// Delivers one event to the shell, queueing whatever it posts.
    fn deliver(&mut self, event: &Event, queue: &mut VecDeque<Event>) -> EventResult {
        let mut ctx = Context::new(&self.commands);
        let result = self.shell.handle_event(event, &mut ctx);
        queue.extend(ctx.take_posted());
        result
    }

    /// Dispatches `event`, then drains posted commands, re-dispatching each
    /// from the top — mirroring `Root::dispatch`, plus interception for the
    /// two commands this demo owns (`Root` has no way to know about them).
    fn dispatch(&mut self, event: &Event) -> EventResult {
        let mut queue = VecDeque::new();
        let result = self.deliver(event, &mut queue);
        let mut budget = 1024;
        while let Some(posted) = queue.pop_front() {
            match posted {
                Event::Command(CM_QUIT) => {
                    let mut ctx = Context::new(&self.commands);
                    if self.shell.valid(CM_QUIT, &mut ctx) {
                        self.finished = true;
                    }
                    queue.extend(ctx.take_posted());
                }
                Event::Command(cmd) if cmd == CM_NEW_WINDOW => self.open_new_window(),
                Event::Command(cmd) if cmd == CM_TOGGLE_TOOLBOX => self.toggle_toolbox(),
                // CM_HELP isn't intercepted here at all — Shell::with_help
                // (below) catches it natively (ADR 0021), so it just falls
                // to the ordinary re-dispatch arm like ADR 0016's CM_CLOSE/
                // CM_ZOOM/CM_NEXT/CM_PREV already do.
                _ => {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    self.deliver(&posted, &mut queue);
                }
            }
        }
        result
    }
}

impl Program for Mdi {
    fn draw(&mut self, frame: &mut Buffer) {
        let mut canvas = Canvas::new(frame);
        self.shell.draw(&mut canvas);
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        self.dispatch(event)
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new()?;
    let size = backend.size();
    let theme = Theme::default();

    let menu_bar = MenuBar::new(
        rect(0, 0, size.width, 1),
        vec![
            Menu::new(
                "File",
                vec![MenuItem::new("Exit", CM_QUIT).with_shortcut("Alt-X")],
            ),
            Menu::new(
                "Window",
                vec![
                    MenuItem::new("New Window", CM_NEW_WINDOW).with_shortcut("Ctrl-N"),
                    MenuItem::new("Close", CM_CLOSE).with_shortcut("Ctrl-W"),
                    MenuItem::new("Zoom", CM_ZOOM).with_shortcut("F5"),
                    MenuItem::new("Next", CM_NEXT).with_shortcut("F6"),
                    MenuItem::new("Previous", CM_PREV).with_shortcut("Shift-F6"),
                    MenuItem::new("Toggle Toolbox", CM_TOGGLE_TOOLBOX).with_shortcut("F9"),
                ],
            ),
            Menu::new(
                "Help",
                vec![MenuItem::new("Contents", CM_HELP).with_shortcut("F1")],
            ),
        ],
        &theme,
    );

    let desk_w = size.width;
    let desk_h = (size.height - 2).max(0);
    let mut desktop = Desktop::new(
        rect(0, 1, desk_w, desk_h),
        Cell::from_char('░', theme.style(Role::DesktopBackground)),
    );

    // A fixed, non-closable toolbox docked to the right edge, hidden until
    // toggled on — Desktop::hide/show, not open/close, since it stays
    // resident either way (ADR 0016).
    let toolbox_w = 18.min(desk_w).max(0);
    let toolbox = Window::new(
        rect((desk_w - toolbox_w).max(0), 0, toolbox_w, desk_h.min(10)),
        "Toolbox",
        &theme,
        Box::new(Info {
            lines: vec!["(tools go".to_string(), "here)".to_string()],
            style: theme.style(Role::WindowFrame),
        }),
    )
    .resizable(false)
    .zoomable(false)
    .closable(false)
    .with_help_topic("toolbox");
    let toolbox_id = desktop.open(toolbox);
    desktop.hide(toolbox_id);

    let status = StatusLine::new(
        rect(0, size.height - 1, size.width, 1),
        vec![
            StatusItem::new(
                "Ctrl-N",
                "New",
                Accelerator::new(
                    KeyEvent::new(KeyCode::Char('n'), Modifiers::CONTROL),
                    CM_NEW_WINDOW,
                ),
            ),
            StatusItem::new(
                "Ctrl-W",
                "Close",
                Accelerator::new(
                    KeyEvent::new(KeyCode::Char('w'), Modifiers::CONTROL),
                    CM_CLOSE,
                ),
            ),
            StatusItem::new(
                "F5",
                "Zoom",
                Accelerator::new(KeyEvent::new(KeyCode::F(5), Modifiers::NONE), CM_ZOOM),
            ),
            StatusItem::new(
                "F6",
                "Next",
                Accelerator::new(KeyEvent::new(KeyCode::F(6), Modifiers::NONE), CM_NEXT),
            ),
            StatusItem::new(
                "F9",
                "Toolbox",
                Accelerator::new(
                    KeyEvent::new(KeyCode::F(9), Modifiers::NONE),
                    CM_TOGGLE_TOOLBOX,
                ),
            ),
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

    let shell = Shell::new(size, menu_bar, desktop, status, &theme)
        .with_help(HelpContents::parse(HELP_SOURCE));
    let mut demo = Mdi {
        shell,
        commands: CommandSet::new(),
        theme: theme.clone(),
        finished: false,
        opened: 0,
        toolbox: toolbox_id,
    };
    // Two starting windows, so there's immediately something to drag,
    // resize, and cycle between.
    demo.open_new_window();
    demo.open_new_window();

    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    app.run(&mut demo)
    // `app` (and the backend) drops here, restoring the terminal.
}
