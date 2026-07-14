//! Phase 5 manual check: modal dialogs and controls driven by the keyboard,
//! over a plain blue backdrop, through the real event loop.
//!
//! Run with `cargo run -p rvision --example dialogs`. It shows, in sequence:
//!
//! 1. a welcome message box (`Enter`/`Esc`);
//! 2. a *Settings* dialog exercising every control, organized into a
//!    `TabbedPages` strip — "General" (an input line, a check box) and
//!    "Formatting" (a radio group inside a titled `GroupBox`) — plus
//!    OK/Cancel buttons below it. Click a tab label, or use Left/Right while
//!    the strip is focused, to switch; `Tab`/`Shift-Tab` cycle strip ⇄ the
//!    active tab's content ⇄ OK/Cancel (escaping the whole widget once the
//!    active page's own focus is exhausted). The arrows also drive the radio
//!    group and (when focused) the input caret, `Space` toggles the check
//!    box, `Enter` is the default *OK*, `Esc` cancels regardless of which tab
//!    is showing;
//! 3. a file *Open* dialog — type a name or pick from the list; `Enter` on a
//!    folder navigates into it, `Enter` on a file (or *Open*) accepts;
//! 4. a colour picker (`docs/specs/color_picker.md`) seeded at Cyan — arrows
//!    move the swatch grid, and (only if `ColorProfile::detect()` reports
//!    `Truecolor` — try running with `COLORTERM=truecolor` set, and again
//!    unset, to see the gating) `Tab` reaches RGB/hex custom entry with a
//!    toggle between them;
//! 5. a closing box reporting what you picked.
//!
//! The chosen file, colour, and the settings result are printed after the
//! terminal is restored (the RAII backend, ADR 0001).

use std::io;
use std::path::PathBuf;
use std::time::Duration;

use rvision::app::{Application, Program};
use rvision::buffer::Buffer;
use rvision::cell::Cell;
use rvision::color::{Color, Color16, ColorProfile, Style};
use rvision::command::{CM_OK, Command};
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult};
use rvision::geometry::{Point, Rect, Size};
use rvision::theme::{Role, Theme};
use rvision::view::{Group, View};
use rvision::widgets::{
    Button, CheckBox, ColorPicker, FileDialog, GroupBox, InputLine, Label, MessageBox,
    RadioButtons, TabbedPages, Window,
};

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// The screen behind the dialogs: a blue desktop with a title strip. It only
/// ever draws — `exec_view` never feeds it events.
struct Backdrop {
    style: Style,
}

impl Program for Backdrop {
    fn draw(&mut self, frame: &mut Buffer) {
        frame.fill(frame.bounds(), &Cell::from_char('░', self.style));
        frame.put_str(
            Point::new(2, 0),
            " rvision — Phase 5 dialogs demo (Esc cancels) ",
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

/// A custom dialog wiring up an input line, a check box, a radio group, and the
/// OK/Cancel buttons — interior coordinates have `(0, 0)` just inside the border.
///
/// The input line/check box and the radio group sit on separate `TabbedPages`
/// tabs rather than all at once — "General" bundles the first two controls
/// into its own `Group` (a page needing more than one control is the
/// caller's own composition, not something `TabbedPages` builds for you);
/// "Formatting" reuses the same `GroupBox`-wrapped `RadioButtons` as before,
/// just repositioned to the tab's own local origin.
fn settings_dialog(theme: &Theme) -> Window {
    let cancel = Command(3); // CM_CANCEL

    // `.non_wrapping()` (ADR 0031) is essential here, not optional: without
    // it, a boundary Tab/Shift-Tab wraps back onto this Group's own two
    // focusable children (InputLine/CheckBox) forever and never reports
    // `Ignored`, so it can never escape back out to TabbedPages' strip —
    // found during the manual tmux pass (Tab worked fine on "Formatting"
    // only because GroupBox already makes its own interior non-wrapping).
    let general: Box<dyn View> = Box::new(
        Group::new(
            rect(0, 0, 28, 4),
            vec![
                Box::new(Label::new(rect(0, 0, 8, 1), "Name:", theme)),
                Box::new(InputLine::new(rect(8, 0, 20, 1), theme).with_text("untitled.txt")),
                Box::new(CheckBox::new(rect(0, 2, 28, 1), "Word wrap", theme).with_checked(true)),
            ],
        )
        .non_wrapping(),
    );
    let formatting: Box<dyn View> = Box::new(GroupBox::new(
        rect(0, 0, 20, 5),
        "Line endings",
        vec![Box::new(RadioButtons::new(
            rect(1, 0, 16, 3),
            &["Unix (LF)", "DOS (CRLF)", "Mac (CR)"],
            theme,
        ))],
        theme,
    ));
    // Dialog interior is 32 columns wide (size.width - 2); a 1-column left
    // margin (mirroring the original layout) plus this widget's own width
    // must not exceed that, or its right border clips against the Window's
    // own frame (found during the manual tmux pass).
    let tabs = TabbedPages::new(
        rect(1, 1, 30, 8),
        vec![("General", general), ("Formatting", formatting)],
        theme,
    );

    let controls: Vec<Box<dyn View>> = vec![
        Box::new(tabs),
        Box::new(Button::new(rect(8, 10, 10, 1), "OK", CM_OK, theme).default(true)),
        Box::new(Button::new(rect(20, 10, 10, 1), "Cancel", cancel, theme)),
    ];
    let size = Size::new(34, 13);
    let interior = rect(1, 1, size.width - 2, size.height - 2);
    let group = Group::new(interior, controls);
    Window::dialog(
        Rect::from_origin_size(Point::new(0, 0), size),
        "Settings",
        theme,
        Box::new(group),
    )
    .centered()
    .resizable(false)
    .zoomable(false)
    .closable(false)
    .esc_cancels(true)
    .with_default(CM_OK)
    .also_ends_on(CM_OK)
    .also_ends_on(cancel)
}

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new()?;
    let theme = Theme::default();
    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    let mut backdrop = Backdrop {
        style: theme.style(Role::DesktopBackground),
    };

    let mut welcome = MessageBox::ok("Welcome", "A tour of rvision dialogs.", &theme);
    app.exec_view(&mut backdrop, &mut welcome)?;

    let mut settings = settings_dialog(&theme);
    let settings_result = app.exec_view(&mut backdrop, &mut settings)?;

    let start = std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."));
    let (mut picker, picker_result) = FileDialog::open("Open File", start, &theme);
    let picked = if app.exec_view(&mut backdrop, &mut picker)? == CM_OK {
        Some(picker_result.path())
    } else {
        None
    };

    let profile = ColorProfile::detect();
    let (mut color_picker, color_result) = ColorPicker::pick(
        "Pick a Colour",
        Color::Named(Color16::Cyan),
        profile,
        &theme,
    );
    let picked_color = if app.exec_view(&mut backdrop, &mut color_picker)? == CM_OK {
        Some(color_result.color())
    } else {
        None
    };

    let summary = match &picked {
        Some(path) => format!("Opened: {}", path.display()),
        None => "No file opened.".to_string(),
    };
    let mut goodbye = MessageBox::ok("Done", &summary, &theme);
    app.exec_view(&mut backdrop, &mut goodbye)?;

    drop(app); // restores the terminal before we print to stdout
    println!("Settings closed with command {settings_result:?}");
    if let Some(path) = picked {
        println!("Picked file: {}", path.display());
    }
    println!("Colour profile detected: {profile:?}");
    if let Some(color) = picked_color {
        println!("Picked colour: {color:?}");
    }
    Ok(())
}
