//! Manual check for the theme picker (`docs/specs/theme_picker.md`,
//! roadmap backlog #2): browse named `Theme` candidates with a live preview,
//! then actually apply whichever one was picked — proving `ColorProfile`
//! detection, the bundled truecolour theme (`examples/truecolour.rs`), and
//! `widgets::ThemePicker` all work together.
//!
//! Run with `cargo run --example theme_picker`. Offers "CGA (default)"
//! always, plus "Truecolour" only when `ColorProfile::detect()` reports
//! `Truecolor` (try with and without `COLORTERM=truecolor` set to see the
//! gating — same idea as `examples/dialogs.rs`'s colour picker). `Up`/`Down`
//! move the highlight and update the preview live; `Enter`/`OK` picks;
//! `Esc`/`Cancel` backs out. The closing screen is drawn in whichever theme
//! you picked, not the one the dialog itself started in.

use std::io;
use std::time::Duration;

use rvision::app::{Application, Program};
use rvision::buffer::Buffer;
use rvision::cell::Cell;
use rvision::color::{ColorProfile, Style};
use rvision::command::CM_OK;
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult};
use rvision::geometry::Point;
use rvision::theme::{Role, Theme};
use rvision::widgets::{MessageBox, ThemePicker};

/// The same bundled palette `examples/truecolour.rs` demonstrates on its own
/// — reused here as the picker's second candidate rather than duplicated.
const TRUECOLOUR_THEME: &str = include_str!("themes/truecolour.theme");

/// The screen behind the dialogs. Its style is swapped after a pick, so the
/// closing message box actually renders in the chosen theme.
struct Backdrop {
    style: Style,
}

impl Program for Backdrop {
    fn draw(&mut self, frame: &mut Buffer) {
        frame.fill(frame.bounds(), &Cell::from_char('░', self.style));
        frame.put_str(
            Point::new(2, 0),
            " rvision — theme picker demo (Esc cancels) ",
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

fn main() -> io::Result<()> {
    let backend = CrosstermBackend::new()?;
    let theme = Theme::default();
    let profile = ColorProfile::detect();

    let mut candidates = vec![("CGA (default)".to_string(), Theme::default())];
    if profile == ColorProfile::Truecolor {
        candidates.push((
            "Truecolour".to_string(),
            Theme::default().merge(TRUECOLOUR_THEME),
        ));
    }

    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    let mut backdrop = Backdrop {
        style: theme.style(Role::DesktopBackground),
    };

    let (mut picker, result) = ThemePicker::pick("Pick a Theme", candidates, 0, &theme);
    let picked = if app.exec_view(&mut backdrop, &mut picker)? == CM_OK {
        Some((result.name(), result.theme()))
    } else {
        None
    };

    let summary = match &picked {
        Some((name, chosen)) => {
            backdrop.style = chosen.style(Role::DesktopBackground);
            format!("Applied: {name}")
        }
        None => "Cancelled — kept the starting theme.".to_string(),
    };
    let closing_theme = picked.as_ref().map(|(_, t)| t).unwrap_or(&theme);
    let mut goodbye = MessageBox::ok("Done", &summary, closing_theme);
    app.exec_view(&mut backdrop, &mut goodbye)?;

    drop(app); // restores the terminal before we print to stdout
    println!("Colour profile detected: {profile:?}");
    match picked {
        Some((name, _)) => println!("Picked theme: {name}"),
        None => println!("No theme picked."),
    }
    Ok(())
}
