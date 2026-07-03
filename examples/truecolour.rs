//! Roadmap backlog #1's "ship a real theme" half: `ColorProfile::detect`
//! (ADR 0023) paired with an actual truecolour [`Theme`], not just the bare
//! capability check. `examples/themes/truecolour.theme` is hand-authored
//! app-layer data (ADR 0025's file format) — merged onto `Theme::default()`
//! only when the terminal reports truecolour support; a `Cga16` terminal
//! keeps the plain CGA theme untouched, proving the fallback is graceful
//! rather than assumed.
//!
//! Run with `cargo run --example truecolour` in a truecolour terminal
//! (`COLORTERM=truecolor`) vs. a plain one to compare. `cargo run --example
//! truecolour -- --dump-theme` prints every role's resolved colour and exits
//! without touching the terminal — useful for verifying a theme file parses
//! as intended, or diffing two runs headlessly.

use std::io;
use std::time::Duration;

use rvision::app::{Application, Program};
use rvision::buffer::Buffer;
use rvision::cell::Cell;
use rvision::color::ColorProfile;
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult, KeyCode, Modifiers};
use rvision::geometry::{Point, Rect, Size};
use rvision::theme::{Field, Role, Theme};

/// The palette this demo offers when [`ColorProfile::detect`] reports
/// [`ColorProfile::Truecolor`] — parsed through the same `Theme::merge` a
/// real app would use to apply an app- or user-resource layer (ADR 0024).
const TRUECOLOUR_THEME: &str = include_str!("themes/truecolour.theme");

const SPINNER: [char; 4] = ['|', '/', '-', '\\'];

/// Resolves the theme this demo (and `--dump-theme`) should use for
/// `profile`: the bundled truecolour palette merged over the CGA default, or
/// the bare CGA default unchanged.
fn theme_for(profile: ColorProfile) -> Theme {
    match profile {
        ColorProfile::Truecolor => Theme::default().merge(TRUECOLOUR_THEME),
        ColorProfile::Cga16 => Theme::default(),
    }
}

/// Prints every role's resolved `fg`/`bg`, reusing `Theme::format_field`
/// (ADR 0026) so the output is exactly the theme-file line a role/field
/// would serialize to — handy for spotting a typo'd key that `Theme::merge`
/// silently dropped (ADR 0025's infallible parsing).
fn dump_theme(profile: ColorProfile, theme: &Theme) {
    println!("# detected: {profile:?}");
    for role in Role::ALL {
        println!("{}", theme.format_field(role, Field::Fg));
        println!("{}", theme.format_field(role, Field::Bg));
    }
}

struct Demo {
    theme: Theme,
    profile: ColorProfile,
    finished: bool,
    ticks: u64,
}

impl Demo {
    fn new(profile: ColorProfile, theme: Theme) -> Self {
        Self {
            theme,
            profile,
            finished: false,
            ticks: 0,
        }
    }
}

impl Program for Demo {
    fn draw(&mut self, frame: &mut Buffer) {
        let size = frame.size();
        frame.fill(
            frame.bounds(),
            &Cell::blank(self.theme.style(Role::DesktopBackground)),
        );

        let w = 56.min(size.width);
        let h = 17.min(size.height);
        if w < 4 || h < 3 {
            return; // too small to bother
        }
        let origin = Point::new((size.width - w) / 2, (size.height - h) / 2);
        let window = Rect::from_origin_size(origin, Size::new(w, h));
        let frame_style = self.theme.style(Role::WindowFrame);
        frame.fill(window, &Cell::blank(frame_style));
        frame.draw_box(window, frame_style);

        let title_style = self.theme.style(Role::WindowTitle);
        frame.put_str(origin.offset(2, 0), " rvision · truecolour ", title_style);

        // A one-row menu bar, with one item highlighted as if selected.
        let menu_style = self.theme.style(Role::MenuBar);
        frame.fill(
            Rect::from_origin_size(origin.offset(1, 1), Size::new(w - 2, 1)),
            &Cell::blank(menu_style),
        );
        frame.put_str(origin.offset(2, 1), "File   Edit   Window", menu_style);
        frame.put_str(
            origin.offset(9, 1),
            " Edit ",
            self.theme.style(Role::MenuSelected),
        );

        let body = origin.offset(2, 3);
        let editor_style = self.theme.style(Role::EditorText);
        frame.put_str(body, "Profile detected:", editor_style);
        let profile_text = match self.profile {
            ColorProfile::Truecolor => "Truecolor -- truecolour.theme applied",
            ColorProfile::Cga16 => "Cga16 -- falling back to the CGA default",
        };
        frame.put_str(body.offset(0, 1), profile_text, editor_style);

        frame.put_str(
            body.offset(0, 3),
            " a selected line of text  ",
            self.theme.style(Role::Selection),
        );
        frame.put_str(
            body.offset(0, 4),
            " an inactive selection    ",
            self.theme.style(Role::SelectionInactive),
        );

        frame.put_str(
            body.offset(0, 6),
            "a {help link|topic} in prose",
            editor_style,
        );
        frame.put_str(
            body.offset(11, 6),
            "{help link|topic}",
            self.theme.style(Role::HelpLink),
        );

        frame.put_str(
            body.offset(0, 8),
            " an input field ",
            self.theme.style(Role::Input),
        );

        frame.put_str(
            body.offset(0, 10),
            " OK ",
            self.theme.style(Role::ButtonNormal),
        );
        frame.put_str(
            body.offset(6, 10),
            " Cancel ",
            self.theme.style(Role::ButtonFocused),
        );

        frame.put_str(
            body.offset(0, 12),
            " drop shadow ",
            self.theme.style(Role::Shadow),
        );

        let status_style = self.theme.style(Role::StatusBar);
        frame.fill(
            Rect::from_origin_size(Point::new(0, size.height - 1), Size::new(size.width, 1)),
            &Cell::blank(status_style),
        );
        frame.put_str(
            Point::new(1, size.height - 1),
            "Esc",
            self.theme.style(Role::StatusKey),
        );
        frame.put_str(Point::new(5, size.height - 1), "Quit", status_style);
        frame.put_str(
            Point::new(12, size.height - 1),
            &format!("idle {}", SPINNER[(self.ticks % 4) as usize]),
            status_style,
        );
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        match event {
            Event::Idle => {
                self.ticks += 1;
                EventResult::Consumed
            }
            Event::Key(key) => {
                let quit = matches!(key.code, KeyCode::Esc)
                    || (matches!(key.code, KeyCode::Char('q') | KeyCode::Char('Q'))
                        && key.modifiers.contains(Modifiers::CONTROL));
                if quit {
                    self.finished = true;
                    return EventResult::Consumed;
                }
                EventResult::Ignored
            }
            _ => EventResult::Ignored,
        }
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

fn main() -> io::Result<()> {
    let profile = ColorProfile::detect();
    let theme = theme_for(profile);

    if std::env::args().any(|a| a == "--dump-theme") {
        dump_theme(profile, &theme);
        return Ok(());
    }

    let backend = CrosstermBackend::new()?;
    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    let mut demo = Demo::new(profile, theme);
    app.run(&mut demo)
}
