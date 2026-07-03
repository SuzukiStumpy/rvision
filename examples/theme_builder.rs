//! Roadmap backlog #3's theme *builder*: a thin wrapper around the same
//! theme editor (`docs/specs/theme_editor.md`) an app author runs to build a
//! theme to *ship with* their application — the app-defaults resource layer
//! (ADR 0024), not the in-app editor's user layer. Same editing surface, two
//! different output layers (roadmap #3's own framing): seeded from
//! `Theme::default()` alone (nothing sits beneath the app layer but the
//! framework default), and saved with a bare `fs::write` per ADR 0024's
//! addendum — the app-defaults layer has no `write_user_resource`-style
//! helper, since the caller already holds the exact directory it wants.
//!
//! Run with `cargo run --example theme_builder -- <app-resources-dir>`.
//! Controls are identical to `examples/theme_editor.rs`.

use std::collections::VecDeque;
use std::io;
use std::path::PathBuf;
use std::time::Duration;

use rvision::app::{Application, Program};
use rvision::backend::Backend;
use rvision::buffer::Buffer;
use rvision::canvas::Canvas;
use rvision::cell::Cell;
use rvision::color::ColorProfile;
use rvision::command::{CM_CANCEL, CM_EDIT_BG, CM_EDIT_FG, CM_OK, CommandSet};
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult};
use rvision::geometry::{Point, Rect, Size};
use rvision::theme::{Field, Role, Theme};
use rvision::view::{Context, View};
use rvision::widgets::{
    ColorPicker, ColorPickerResult, Desktop, ThemeEditor, ThemeEditorHandle, WindowId,
};

const RESOURCE_NAME: &str = "theme";

/// See `examples/theme_editor.rs`'s identical helper — `Desktop` has no
/// auto-centring of its own (unlike `Application::exec_view`), so a window
/// that wants it positions itself.
fn centered(size: Size, within: Size) -> Rect {
    let w = size.width.clamp(0, within.width.max(0));
    let h = size.height.clamp(0, within.height.max(0));
    let x = ((within.width - w) / 2).max(0);
    let y = ((within.height - h) / 2).max(0);
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// See `examples/theme_editor.rs`'s identical type — duplicated rather than
/// shared, since this driving `Program` is application glue (a "utility
/// program", roadmap #3), not library code, and two call sites don't justify
/// a shared examples-support module.
struct PendingPicker {
    window_id: WindowId,
    field: Field,
    result: ColorPickerResult,
}

struct ThemeBuilderDemo {
    desktop: Desktop,
    commands: CommandSet,
    theme: Theme,
    profile: ColorProfile,
    editor_id: WindowId,
    handle: ThemeEditorHandle,
    pending_picker: Option<PendingPicker>,
    app_dir: PathBuf,
    saved_text: Option<String>,
    finished: bool,
}

impl ThemeBuilderDemo {
    fn open_picker(&mut self, field: Field) {
        let style = self.handle.style();
        let (initial, title) = match field {
            Field::Fg => (style.fg, "Foreground"),
            Field::Bg => (style.bg, "Background"),
            Field::Attrs => unreachable!("ThemeEditor never posts CM_EDIT_FG/BG for attrs"),
        };
        let (mut window, result) = ColorPicker::pick(title, initial, self.profile, &self.theme);
        window.set_bounds(centered(
            window.bounds().size(),
            self.desktop.bounds().size(),
        ));
        let window_id = self.desktop.open(window);
        self.pending_picker = Some(PendingPicker {
            window_id,
            field,
            result,
        });
    }

    fn on_ok(&mut self) {
        if let Some(pending) = self.pending_picker.take() {
            self.handle
                .apply_color(pending.field, pending.result.color());
            let mut ctx = Context::new(&self.commands);
            self.desktop.close(pending.window_id, &mut ctx);
            return;
        }
        let text = self.handle.diff_text();
        // The app-defaults layer has no `write_user_resource`-style helper
        // (ADR 0024 addendum): the caller already holds the exact directory
        // it wants, so writing there is a bare `fs::write`.
        let _ = std::fs::write(self.app_dir.join(RESOURCE_NAME), &text);
        self.saved_text = Some(text);
        let mut ctx = Context::new(&self.commands);
        self.desktop.close(self.editor_id, &mut ctx);
        self.finished = true;
    }

    fn on_cancel(&mut self) {
        let mut ctx = Context::new(&self.commands);
        if let Some(pending) = self.pending_picker.take() {
            self.desktop.close(pending.window_id, &mut ctx);
            return;
        }
        self.desktop.close(self.editor_id, &mut ctx);
        self.finished = true;
    }

    fn deliver(&mut self, event: &Event, queue: &mut VecDeque<Event>) -> EventResult {
        let mut ctx = Context::new(&self.commands);
        let result = self.desktop.handle_event(event, &mut ctx);
        queue.extend(ctx.take_posted());
        result
    }

    fn dispatch(&mut self, event: &Event) -> EventResult {
        let mut queue = VecDeque::new();
        let result = self.deliver(event, &mut queue);
        let mut budget = 1024;
        while let Some(posted) = queue.pop_front() {
            match posted {
                Event::Command(CM_EDIT_FG) => self.open_picker(Field::Fg),
                Event::Command(CM_EDIT_BG) => self.open_picker(Field::Bg),
                Event::Command(CM_OK) => self.on_ok(),
                Event::Command(CM_CANCEL) => self.on_cancel(),
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

impl Program for ThemeBuilderDemo {
    fn draw(&mut self, frame: &mut Buffer) {
        let mut canvas = Canvas::new(frame);
        self.desktop.draw(&mut canvas);
    }

    fn handle_event(&mut self, event: &Event) -> EventResult {
        self.dispatch(event)
    }

    fn is_finished(&self) -> bool {
        self.finished
    }
}

fn main() -> io::Result<()> {
    let app_dir = match std::env::args().nth(1) {
        Some(dir) => PathBuf::from(dir),
        None => {
            eprintln!("usage: theme_builder <app-resources-dir>");
            std::process::exit(2);
        }
    };
    std::fs::create_dir_all(&app_dir)?;

    let backend = CrosstermBackend::new()?;
    let size = backend.size();
    let theme = Theme::default();
    let profile = ColorProfile::detect();

    // Nothing sits beneath the app layer but the framework default -- unlike
    // the in-app editor, there's no user layer to merge in here.
    let base = Theme::default();

    let mut desktop = Desktop::new(
        Rect::from_origin_size(Point::new(0, 0), size),
        Cell::from_char('░', theme.style(Role::DesktopBackground)),
    );
    let (window, handle) = ThemeEditor::window(Point::new(2, 1), "Theme Builder", base, &theme);
    let editor_id = desktop.open(window);

    let mut demo = ThemeBuilderDemo {
        desktop,
        commands: CommandSet::new(),
        theme,
        profile,
        editor_id,
        handle,
        pending_picker: None,
        app_dir: app_dir.clone(),
        saved_text: None,
        finished: false,
    };

    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    app.run(&mut demo)?;

    drop(app); // restores the terminal before we print to stdout
    match demo.saved_text {
        Some(text) if text.is_empty() => println!("Saved: nothing was touched."),
        Some(text) => println!("Saved to {:?}:\n{text}", app_dir.join(RESOURCE_NAME)),
        None => println!("Cancelled — nothing saved."),
    }
    Ok(())
}
