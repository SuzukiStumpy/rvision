//! Manual check for `ComboBox` (`docs/specs/combo_box.md`).
//!
//! Run with `cargo run --example combo_box`. Three combo boxes, one per mode:
//!
//! 1. **Filtering (default).** Type to narrow the suggestions (try "gr" for
//!    Green/Grey), `Down`/`Up` to preview a match, click a row or press
//!    `Enter` to accept it, `Esc` to back out of a preview.
//! 2. **Type-ahead (`filterable(false)`).** Type "gr" — the list stays full
//!    (every colour still shown), but the highlight jumps to the first
//!    match ("Green"); the field itself keeps showing exactly what you
//!    typed.
//! 3. **Select-only.** Printable keys never insert free text; each one
//!    searches `items` and jumps straight to (and displays) the first
//!    match, `Backspace` shortens the search. Try typing "d" then "o" for
//!    "DOS (CRLF)".
//!
//! `Tab` cycles between the three combo boxes and OK/Cancel; `Enter` on OK
//! (or the dialog's own `Enter`-while-closed bubble) finishes.

use std::io;
use std::time::Duration;

use rvision::app::{Application, Program};
use rvision::buffer::Buffer;
use rvision::cell::Cell;
use rvision::color::Style;
use rvision::command::CM_OK;
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult};
use rvision::geometry::{Point, Rect, Size};
use rvision::theme::{Role, Theme};
use rvision::view::{Group, View};
use rvision::widgets::{Button, ComboBox, Label, Window};

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

struct Backdrop {
    style: Style,
}

impl Program for Backdrop {
    fn draw(&mut self, frame: &mut Buffer) {
        frame.fill(frame.bounds(), &Cell::from_char('░', self.style));
        frame.put_str(
            Point::new(2, 0),
            " rvision — ComboBox demo (Esc cancels) ",
            self.style,
        );
    }

    fn handle_event(&mut self, _event: &Event) -> EventResult {
        EventResult::Ignored
    }

    fn is_finished(&self) -> bool {
        false
    }
}

const CM_CANCEL_LOCAL: rvision::command::Command = rvision::command::CM_CANCEL;

fn colour_names() -> Vec<String> {
    [
        "Black",
        "Blue",
        "Green",
        "Cyan",
        "Red",
        "Magenta",
        "Brown",
        "LightGray",
        "DarkGray",
        "LightBlue",
        "LightGreen",
        "LightCyan",
        "LightRed",
        "LightMagenta",
        "Yellow",
        "White",
        "Grey",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect()
}

fn colour_dialog(theme: &Theme) -> Window {
    let line_endings = ["Unix (LF)", "DOS (CRLF)", "Mac (CR)"]
        .iter()
        .map(|s| s.to_string())
        .collect();

    let controls: Vec<Box<dyn View>> = vec![
        Box::new(Label::new(rect(1, 1, 26, 1), "Colour (filter):", theme)),
        Box::new(ComboBox::new(rect(1, 2, 26, 1), colour_names(), theme)),
        Box::new(Label::new(rect(1, 3, 26, 1), "Colour (type-ahead):", theme)),
        Box::new(ComboBox::new(rect(1, 4, 26, 1), colour_names(), theme).filterable(false)),
        Box::new(Label::new(
            rect(1, 5, 26, 1),
            "Line ending (select only):",
            theme,
        )),
        Box::new(ComboBox::new(rect(1, 6, 26, 1), line_endings, theme).select_only(true)),
        Box::new(Button::new(rect(4, 8, 10, 1), "OK", CM_OK, theme).default(true)),
        Box::new(Button::new(
            rect(16, 8, 10, 1),
            "Cancel",
            CM_CANCEL_LOCAL,
            theme,
        )),
    ];
    let size = Size::new(30, 11);
    let interior = rect(1, 1, size.width - 2, size.height - 2);
    let group = Group::new(interior, controls);
    Window::dialog(
        Rect::from_origin_size(Point::new(0, 0), size),
        "Pick Some Values",
        theme,
        Box::new(group),
    )
    .centered()
    .resizable(false)
    .zoomable(false)
    .closable(false)
    // Deliberately NOT `.esc_cancels(true)`: that intercepts `Esc` before the
    // interior ever sees it (`Window::handle_event`, ADR 0016), which would
    // steal the keystroke `ComboBox` needs to close its own open drop-down
    // instead of cancelling the whole dialog (`docs/specs/combo_box.md`).
    // The Cancel button remains the only way out.
    .with_default(CM_OK)
    .also_ends_on(CM_OK)
    .also_ends_on(CM_CANCEL_LOCAL)
}

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new()?;
    let theme = Theme::default();
    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    let mut backdrop = Backdrop {
        style: theme.style(Role::DesktopBackground),
    };

    let mut dialog = colour_dialog(&theme);
    let result = app.exec_view(&mut backdrop, &mut dialog)?;

    drop(app); // restores the terminal before we print to stdout
    println!("Dialog closed with command {result:?}");
    Ok(())
}
