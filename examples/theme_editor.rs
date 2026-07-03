//! Manual check for the theme editor (`docs/specs/theme_editor.md`, ADR 0026):
//! browse every [`Role`], edit a role's colours via a nested [`ColorPicker`]
//! and its attributes via checkboxes, then save — proving the colour picker,
//! the resource loader, and the theme file format/merge all work together.
//!
//! Run with `cargo run --example theme_editor`. `Tab`/`Shift-Tab` move focus
//! through the role list, *Foreground.../Background...*, the six attribute
//! checkboxes, and *Save*/*Cancel*; `Space` toggles a checkbox; the
//! *Foreground.../Background...* buttons open a colour picker as a second
//! window (raised over the editor — click a swatch or arrow-key the grid,
//! then *OK*/*Cancel*); `Esc` cancels whichever window is topmost. Saving
//! writes only the fields you actually touched to this example's user
//! resource layer (ADR 0024/0025) and prints where — run it a second time to
//! see the change already applied.

use std::collections::VecDeque;
use std::io;
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
use rvision::resource;
use rvision::theme::{Field, Role, Theme};
use rvision::view::{Context, View};
use rvision::widgets::{
    ColorPicker, ColorPickerResult, Desktop, ThemeEditor, ThemeEditorHandle, WindowId,
};

const APP_NAME: &str = "rvision-examples";
const RESOURCE_NAME: &str = "theme";

/// Centres a box of `size` within `within`, clamped to fit — `Desktop` has
/// no auto-centring of its own (unlike `Application::exec_view`), so a
/// window that wants it positions itself.
fn centered(size: Size, within: Size) -> Rect {
    let w = size.width.clamp(0, within.width.max(0));
    let h = size.height.clamp(0, within.height.max(0));
    let x = ((within.width - w) / 2).max(0);
    let y = ((within.height - h) / 2).max(0);
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// The nested colour picker currently open, and which field of the selected
/// role it's editing — the "driver holds the disambiguating state" idiom
/// ADR 0026 borrows from `examples/mdi.rs`'s `Mdi::toolbox`.
struct PendingPicker {
    window_id: WindowId,
    field: Field,
    result: ColorPickerResult,
}

/// Drives the demo: a bare `Desktop` (no `Shell`/menu bar needed) hosting the
/// theme editor and, transiently, a nested colour picker.
struct ThemeEditorDemo {
    desktop: Desktop,
    commands: CommandSet,
    theme: Theme,
    profile: ColorProfile,
    editor_id: WindowId,
    handle: ThemeEditorHandle,
    pending_picker: Option<PendingPicker>,
    saved_text: Option<String>,
    finished: bool,
}

impl ThemeEditorDemo {
    fn open_picker(&mut self, field: Field) {
        let style = self.handle.style();
        let (initial, title) = match field {
            Field::Fg => (style.fg, "Foreground"),
            Field::Bg => (style.bg, "Background"),
            Field::Attrs => unreachable!("ThemeEditor never posts CM_EDIT_FG/BG for attrs"),
        };
        let (mut window, result) = ColorPicker::pick(title, initial, self.profile, &self.theme);
        // `ColorPicker::pick`'s `.centered()` is only honoured by
        // `Application::exec_view`'s own loop; `Desktop` never auto-centers,
        // so we position it ourselves, the same way `examples/mdi.rs`'s
        // `open_new_window` computes its own explicit bounds.
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
        let _ = resource::write_user_resource(APP_NAME, RESOURCE_NAME, &text);
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

    /// Delivers one event to the desktop, queueing whatever it posts.
    fn deliver(&mut self, event: &Event, queue: &mut VecDeque<Event>) -> EventResult {
        let mut ctx = Context::new(&self.commands);
        let result = self.desktop.handle_event(event, &mut ctx);
        queue.extend(ctx.take_posted());
        result
    }

    /// Dispatches `event`, then drains posted commands, re-dispatching each
    /// from the top — mirroring `examples/mdi.rs`'s `Mdi::dispatch`, plus
    /// interception for the two edit-request commands and the shared
    /// `CM_OK`/`CM_CANCEL` (disambiguated via `pending_picker`, ADR 0026).
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

impl Program for ThemeEditorDemo {
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
    let backend = CrosstermBackend::new()?;
    let size = backend.size();
    let profile = ColorProfile::detect();

    // A real application loads its theme once at startup and uses it for
    // everything, including its own chrome -- so the previously saved layer
    // must reach the Desktop backdrop and the editor's own window/list/
    // buttons, not just the in-editor preview. `theme` is deliberately the
    // *same* value for both the chrome (`ThemeEditor::window`'s `theme`
    // param) and the initial value being edited (`base`): there's nothing
    // else styling this demo, so there's no reason for them to diverge, and
    // keeping them the same is what makes a saved change visible immediately
    // on the next run.
    let layers = resource::load_layers(APP_NAME, RESOURCE_NAME, None)?;
    let theme = Theme::default().merge(layers.user.as_deref().unwrap_or(""));

    let mut desktop = Desktop::new(
        Rect::from_origin_size(Point::new(0, 0), size),
        Cell::from_char('░', theme.style(Role::DesktopBackground)),
    );
    let (window, handle) =
        ThemeEditor::window(Point::new(2, 1), "Theme Editor", theme.clone(), &theme);
    let editor_id = desktop.open(window);

    let mut demo = ThemeEditorDemo {
        desktop,
        commands: CommandSet::new(),
        theme,
        profile,
        editor_id,
        handle,
        pending_picker: None,
        saved_text: None,
        finished: false,
    };

    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    app.run(&mut demo)?;

    drop(app); // restores the terminal before we print to stdout
    match demo.saved_text {
        Some(text) if text.is_empty() => println!("Saved: nothing was touched."),
        Some(text) => {
            println!(
                "Saved to {:?}:\n{text}",
                resource::user_resource_path(APP_NAME, RESOURCE_NAME)
            );
        }
        None => println!("Cancelled — nothing saved."),
    }
    Ok(())
}
