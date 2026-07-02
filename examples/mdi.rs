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
    CM_CLOSE, CM_NEXT, CM_PREV, CM_QUIT, CM_USER, CM_ZOOM, Command, CommandSet,
};
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult, KeyCode, KeyEvent, Modifiers};
use rvision::geometry::{Point, Rect, Size};
use rvision::theme::{Role, Theme};
use rvision::view::{Context, View};
use rvision::widgets::{
    Desktop, Menu, MenuBar, MenuItem, StatusItem, StatusLine, Window, WindowId,
};

// Application command ids, numbered from the framework/app boundary (ADR
// 0003). Everything else this demo triggers (close/zoom/next/prev) is a
// framework-reserved command Desktop itself already acts on.
const CM_NEW_WINDOW: Command = Command(CM_USER + 1);
const CM_TOGGLE_TOOLBOX: Command = Command(CM_USER + 2);

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
        );
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
    .closable(false);
    let toolbox_id = desktop.open(toolbox);
    desktop.hide(toolbox_id);

    let status = StatusLine::new(
        rect(0, size.height - 1, size.width, 1),
        vec![
            StatusItem::new(
                "Ctrl-N",
                "New",
                KeyEvent::new(KeyCode::Char('n'), Modifiers::CONTROL),
                CM_NEW_WINDOW,
            ),
            StatusItem::new(
                "Ctrl-W",
                "Close",
                KeyEvent::new(KeyCode::Char('w'), Modifiers::CONTROL),
                CM_CLOSE,
            ),
            StatusItem::new(
                "F5",
                "Zoom",
                KeyEvent::new(KeyCode::F(5), Modifiers::NONE),
                CM_ZOOM,
            ),
            StatusItem::new(
                "F6",
                "Next",
                KeyEvent::new(KeyCode::F(6), Modifiers::NONE),
                CM_NEXT,
            ),
            StatusItem::new(
                "F9",
                "Toolbox",
                KeyEvent::new(KeyCode::F(9), Modifiers::NONE),
                CM_TOGGLE_TOOLBOX,
            ),
            StatusItem::new(
                "Alt-X",
                "Exit",
                KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT),
                CM_QUIT,
            ),
        ],
        theme.style(Role::StatusBar),
        theme.style(Role::StatusKey),
    );

    let shell = Shell::new(size, menu_bar, desktop, status);
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
