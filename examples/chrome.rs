//! Phase 4 manual check: the full TurboVision-style application chrome — a menu
//! bar, a blue desktop with one framed window, and a status line — driven by the
//! keyboard through the real event loop.
//!
//! Run with `cargo run -p rvision --example chrome`. Things to try:
//!
//! - `Alt-F` / `Alt-E` / `Alt-S` (or `F10`) open a menu; `←`/`→` switch menus,
//!   `↑`/`↓` move the highlight, `Enter` chooses, `Esc` closes.
//! - File ▸ Export cascades into a submenu (ADR 0018): `→` or `Enter` opens it,
//!   `←` or `Esc` backs out one level without closing the bar.
//! - Right-click inside the window opens a context menu, anchored at the
//!   click (ADR 0019); it cascades and dismisses exactly like a pull-down's
//!   submenu (`←`/`Esc` backs out one level, a click elsewhere or a fresh
//!   right-click elsewhere dismisses/replaces it).
//! - `Alt-X` (or File ▸ Exit) quits; the terminal is always restored, even on a
//!   panic, thanks to the RAII backend (ADR 0001).
//! - Resize the window: the menu bar, desktop, and status line relay out.

use std::io;
use std::time::Duration;

use rvision::app::{Application, Root, Shell};
use rvision::backend::Backend;
use rvision::canvas::Canvas;
use rvision::cell::Cell;
use rvision::color::Style;
use rvision::command::{CM_QUIT, CM_USER, Command};
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult, KeyCode, KeyEvent, Modifiers, MouseButton, MouseKind};
use rvision::geometry::{Point, Rect, Size};
use rvision::theme::{Role, Theme};
use rvision::view::{Context, View};
use rvision::widgets::{Desktop, Menu, MenuBar, MenuItem, StatusItem, StatusLine, Window};

// Application command ids, numbered from the framework/app boundary (ADR 0003).
// Only CM_QUIT is wired to an effect here; the rest post harmlessly — this is a
// chrome demo, not the editor (Phase 6).
const CM_NEW: Command = Command(CM_USER + 1);
const CM_OPEN: Command = Command(CM_USER + 2);
const CM_SAVE: Command = Command(CM_USER + 3);
const CM_CUT: Command = Command(CM_USER + 4);
const CM_COPY: Command = Command(CM_USER + 5);
const CM_PASTE: Command = Command(CM_USER + 6);
const CM_FIND: Command = Command(CM_USER + 7);
const CM_REPLACE: Command = Command(CM_USER + 8);
const CM_EXPORT_PDF: Command = Command(CM_USER + 9);
const CM_EXPORT_PNG: Command = Command(CM_USER + 10);
const CM_WORD_COUNT: Command = Command(CM_USER + 11);
const CM_ABOUT_RVISION: Command = Command(CM_USER + 12);
const CM_ABOUT_EDIT: Command = Command(CM_USER + 13);

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// A placeholder window interior: a few lines of help that fit any window (each
/// line is short and clipped by the window, never spilling). Stands in for the
/// editor view that arrives in Phase 6.
struct Hint {
    style: Style,
}

impl View for Hint {
    fn bounds(&self) -> Rect {
        // Unused: the window fills the interior canvas and draws this into it.
        rect(0, 0, 1, 1)
    }

    fn draw(&self, canvas: &mut Canvas) {
        let lines = [
            "Empty document.",
            "",
            "Alt / F10   open a menu",
            "Right-click open a context menu",
            "Alt-X       exit",
        ];
        for (row, line) in lines.iter().enumerate() {
            canvas.put_str(Point::new(1, row as i16 + 1), line, self.style);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let Event::Mouse(mouse) = event else {
            return EventResult::Ignored;
        };
        if mouse.kind != MouseKind::Down(MouseButton::Right) {
            return EventResult::Ignored;
        }
        let menu = Menu::new(
            "Context",
            vec![
                MenuItem::new("Word Count", CM_WORD_COUNT),
                MenuItem::submenu(
                    "About",
                    Menu::new(
                        "About",
                        vec![
                            MenuItem::new("rvision...", CM_ABOUT_RVISION),
                            MenuItem::new("edit...", CM_ABOUT_EDIT),
                        ],
                    ),
                ),
            ],
        );
        ctx.open_context_menu(menu, mouse.pos);
        EventResult::Consumed
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
                vec![
                    MenuItem::new("New", CM_NEW).with_shortcut("Ctrl-N"),
                    MenuItem::new("Open...", CM_OPEN).with_shortcut("Ctrl-O"),
                    MenuItem::new("Save", CM_SAVE).with_shortcut("Ctrl-S"),
                    MenuItem::submenu(
                        "Export",
                        Menu::new(
                            "Export",
                            vec![
                                MenuItem::new("PDF...", CM_EXPORT_PDF),
                                MenuItem::new("PNG...", CM_EXPORT_PNG),
                            ],
                        ),
                    ),
                    MenuItem::new("Exit", CM_QUIT).with_shortcut("Alt-X"),
                ],
            ),
            Menu::new(
                "Edit",
                vec![
                    MenuItem::new("Cut", CM_CUT).with_shortcut("Ctrl-X"),
                    MenuItem::new("Copy", CM_COPY).with_shortcut("Ctrl-C"),
                    MenuItem::new("Paste", CM_PASTE).with_shortcut("Ctrl-V"),
                ],
            ),
            Menu::new(
                "Search",
                vec![
                    MenuItem::new("Find...", CM_FIND),
                    MenuItem::new("Replace...", CM_REPLACE),
                ],
            ),
        ],
        &theme,
    );

    // One window, centred in the desktop region (rows 1..h-1).
    let desk_w = size.width;
    let desk_h = (size.height - 2).max(0);
    let win_w = 50.min(desk_w - 4).max(4);
    let win_h = 14.min(desk_h - 2).max(3);
    let win = rect((desk_w - win_w) / 2, (desk_h - win_h) / 2, win_w, win_h);
    let window = Window::new(
        win,
        "Untitled",
        &theme,
        Box::new(Hint {
            style: theme.style(Role::WindowFrame),
        }),
    );
    let mut desktop = Desktop::new(
        rect(0, 1, desk_w, desk_h),
        Cell::from_char('░', theme.style(Role::DesktopBackground)),
    );
    desktop.open(window);

    let status = StatusLine::new(
        rect(0, size.height - 1, size.width, 1),
        vec![
            StatusItem::new(
                "F1",
                "Help",
                KeyEvent::new(KeyCode::F(1), Modifiers::NONE),
                CM_FIND,
            ),
            StatusItem::new(
                "Alt-X",
                "Exit",
                KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT),
                CM_QUIT,
            ),
            StatusItem::new(
                "F10",
                "Menu",
                KeyEvent::new(KeyCode::F(10), Modifiers::NONE),
                CM_NEW,
            ),
        ],
        theme.style(Role::StatusBar),
        theme.style(Role::StatusKey),
    );

    let shell = Shell::new(size, menu_bar, desktop, status, &theme);
    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    let mut root = Root::new(Box::new(shell));
    app.run(&mut root)
    // `app` (and the backend) drops here, restoring the terminal.
}
