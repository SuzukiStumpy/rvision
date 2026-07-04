//! Roadmap #6 manual check: `TextArea` hosted on a real `Desktop`, driven by
//! the real event loop, so its scroll bar is hosted generically (ADR 0015)
//! and the window's drag/resize/close chrome actually works — a lone
//! `exec_view` run has none of that (`Window` has no concept of a drag
//! session outside a `Desktop`, per ADR 0016), which was the wrong choice
//! for the first cut of this example.
//!
//! Run with `cargo run -p rvision --example text_area`. Things to try:
//!
//! - Type across the right edge and watch it reflow — multiple spaces are
//!   never collapsed, unlike the read-only `wrap`/`HelpPane` path.
//! - `Ctrl+Left`/`Ctrl+Right` jump by word; `Home`/`End` stay on the current
//!   wrapped line; `Ctrl+Home`/`Ctrl+End` jump to the whole text.
//! - Hold `Shift` with any of the above to select, then type/Backspace/Delete
//!   to replace or remove the selection.
//! - Drag the title bar to move the window; drag the bottom-right corner to
//!   resize it and watch the text reflow live; scroll with the wheel or the
//!   hosted scroll bar once the text runs past the bottom.
//! - The window's close glyph (top-left) removes the window from the
//!   desktop, same as any other `rvision` window — it does not quit the
//!   app. `Alt-X` (or File ▸ Exit) always quits.

use std::io;
use std::time::Duration;

use rvision::app::{Application, Root, Shell};
use rvision::backend::Backend;
use rvision::cell::Cell;
use rvision::command::{Accelerator, CM_QUIT};
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{KeyCode, KeyEvent, Modifiers};
use rvision::geometry::{Point, Rect, Size};
use rvision::theme::{Role, Theme};
use rvision::view::View;
use rvision::widgets::{
    Desktop, Menu, MenuBar, MenuItem, StatusItem, StatusLine, TextArea, Window,
};

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

const SAMPLE: &str = "TextArea (roadmap #6) — a scrollable, focusable \
multi-line text field.\n\nIt reflows long lines to the current width but \
never collapses whitespace: three    spaces    survive right here, exactly \
as typed.\n\nDrag the bottom-right corner to resize this window and watch it \
rewrap live, and scroll with the wheel or the bar on the right once the text \
runs past the bottom.\n\nCtrl+Left/Ctrl+Right jump by word. Home/End stay on \
this wrapped line; Ctrl+Home/Ctrl+End jump to the very start or end of the \
whole text. Hold Shift with any of those to select.";

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new()?;
    let size = backend.size();
    let theme = Theme::default();

    let menu_bar = MenuBar::new(
        rect(0, 0, size.width, 1),
        vec![Menu::new(
            "File",
            vec![MenuItem::new("Exit", CM_QUIT).with_shortcut("Alt-X")],
        )],
        &theme,
    );

    let desk_w = size.width;
    let desk_h = (size.height - 2).max(0);
    let win_w = 50.min(desk_w - 4).max(4);
    let win_h = 16.min(desk_h - 2).max(3);
    let win = rect((desk_w - win_w) / 2, (desk_h - win_h) / 2, win_w, win_h);
    // The interior view must be sized to the window's *interior* — inset by
    // one cell on every side for the border — not the window's own outer
    // bounds. `Window::new`/`styled` don't do this for the caller (only a
    // later resize/zoom re-propagates via `set_bounds`, ADR 0017): sizing
    // the interior correctly up front is the caller's job, the same way
    // `HelpWindow::build` computes its own `interior_size` before
    // constructing what it wraps. Getting this wrong doesn't panic or even
    // look obviously wrong at a glance — it silently reflows/scrolls against
    // a size that's 2 columns/rows too big in each dimension, which is
    // exactly what happened here the first time (wrapped words truncated at
    // the true right edge, and no scroll bar/wheel until a resize forced a
    // correct `set_bounds`).
    let interior = Rect::from_origin_size(Point::new(1, 1), Size::new(win_w - 2, win_h - 2));
    let mut text_area = TextArea::new(interior, &theme).with_text(SAMPLE);
    // A `Window`'s interior isn't auto-focused just by its window being the
    // desktop's active one (that's `Window::set_active`'s frame styling,
    // a separate concept from a view's own `focused` flag) — a lone
    // interior (no `Group` to do this for its first child, ADR 0010) has to
    // be told directly, or `TextArea::handle_event` ignores every key.
    text_area.set_focused(true);
    let window = Window::new(win, "TextArea", &theme, Box::new(text_area));

    let mut desktop = Desktop::new(
        rect(0, 1, desk_w, desk_h),
        Cell::from_char('░', theme.style(Role::DesktopBackground)),
    );
    desktop.open(window);

    let status = StatusLine::new(
        rect(0, size.height - 1, size.width, 1),
        vec![StatusItem::new(
            "Alt-X",
            "Exit",
            Accelerator::new(KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT), CM_QUIT),
        )],
        theme.style(Role::StatusBar),
        theme.style(Role::StatusKey),
    );

    let shell = Shell::new(size, menu_bar, desktop, status, &theme);
    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    let mut root = Root::new(Box::new(shell));
    app.run(&mut root)
    // `app` (and the backend) drops here, restoring the terminal.
}
