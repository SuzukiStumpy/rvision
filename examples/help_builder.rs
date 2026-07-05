//! Roadmap #3's help authoring tool: a raw markup source pane (`TextArea`)
//! beside a live preview through the existing `HelpPane` renderer (via
//! `HelpWindow`), for iterating on an ADR 0013 `.help` file. No new format —
//! `HelpContents::parse` was designed with exactly this in mind.
//!
//! Run with `cargo run --example help_builder [-- <path>]`. If `<path>` is
//! given and exists, its contents seed the source pane and every plain Save
//! writes back there with no prompt; otherwise the first Save (or File ▸
//! Save As..., any time) opens a Save dialog to pick a name. Saving is never
//! a quit signal — partial progress can be saved repeatedly while the tool
//! stays open.
//!
//! - `Ctrl+O` / File ▸ Open... loads an existing file into the source pane,
//!   replacing whatever was there (no unsaved-changes prompt, same
//!   simplicity as Exit below) and becoming the path subsequent `Ctrl+S`
//!   saves write to. The preview is *not* auto-refreshed on load — Refresh
//!   is always an explicit, separate step, the same as after any other edit.
//! - `Ctrl+R` / File ▸ Refresh Preview closes and reopens the Preview window
//!   from a fresh parse of the source text — deliberately not live on every
//!   keystroke (docs/roadmap.md's #3 entry explains why), so a half-typed
//!   `{label|target}` link is never shown broken mid-edit, and following a
//!   link in the (real, unmodified) preview window genuinely works.
//! - `Ctrl+S` / File ▸ Save writes to the last-used path with no prompt, once
//!   one is known.
//! - File ▸ Save As... always prompts, so incremental, differently-named
//!   versions can be saved on demand.
//! - The source window's title grows a trailing `*` while there are unsaved
//!   edits, clearing on save (or a fresh Open), the same convention `edit`
//!   (the editor this framework was extracted from) uses for dirty files.
//! - `Alt-X` / File ▸ Exit quits (no unsaved-changes prompt).
//! - A permanent "Guide" window sits in Preview's bottom-right corner: this
//!   tool's own usage guide plus a primer on the `.help` format, baked in
//!   with `include_str!` the same way `edit`'s built-in help is
//!   (`crates/edit/src/help.rs`). It has no close glyph and no menu entry —
//!   it isn't something to open or dismiss, just always there — but it's
//!   still a plain moveable `Window`, so drag its title bar aside if it's
//!   covering Preview content underneath.

use std::cell::RefCell;
use std::collections::VecDeque;
use std::fs;
use std::io;
use std::path::PathBuf;
use std::rc::Rc;
use std::time::Duration;

use rvision::app::{Application, Program, Shell};
use rvision::backend::Backend;
use rvision::buffer::Buffer;
use rvision::canvas::Canvas;
use rvision::cell::Cell;
use rvision::command::{Accelerator, CM_CANCEL, CM_OK, CM_QUIT, CM_USER, Command, CommandSet};
use rvision::crossterm_backend::CrosstermBackend;
use rvision::event::{Event, EventResult, KeyCode, KeyEvent, Modifiers};
use rvision::geometry::{Point, Rect, Size};
use rvision::help::HelpContents;
use rvision::theme::{Role, Theme};
use rvision::view::{Context, ScrollMetrics, View};
use rvision::widgets::{
    Desktop, FileDialog, FileDialogResult, HelpWindow, Menu, MenuBar, MenuItem, StatusItem,
    StatusLine, TextArea, Window, WindowId,
};

const CM_SAVE: Command = Command(CM_USER + 1);
const CM_SAVE_AS: Command = Command(CM_USER + 2);
const CM_REFRESH: Command = Command(CM_USER + 3);
const CM_OPEN: Command = Command(CM_USER + 4);

/// This tool's own usage guide and `.help`-format primer, baked into the
/// permanent Guide window — the same `include_str!` convention `edit` uses
/// for its own built-in help (`crates/edit/src/help.rs`'s `HELP_TEXT`).
const GUIDE_TEXT: &str = include_str!("help_builder.help");

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

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

/// A `TextArea` shared between the `Window` that owns it (as a `Box<dyn
/// View>`) and this demo's driver, which needs to read its live text back out
/// on Save/Refresh — a plain forwarding shim local to this example, not a new
/// library type: unlike `ThemeEditorHandle`, nothing here needs to *write
/// into* the widget from outside, only read from it.
struct SharedTextArea(Rc<RefCell<TextArea>>);

impl View for SharedTextArea {
    fn bounds(&self) -> Rect {
        self.0.borrow().bounds()
    }

    fn draw(&self, canvas: &mut Canvas) {
        self.0.borrow().draw(canvas)
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        self.0.borrow_mut().handle_event(event, ctx)
    }

    fn focusable(&self) -> bool {
        self.0.borrow().focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.0.borrow_mut().set_focused(focused)
    }

    fn scroll_metrics(&self) -> Option<ScrollMetrics> {
        self.0.borrow().scroll_metrics()
    }

    fn set_scroll(&mut self, offset: Point) {
        self.0.borrow_mut().set_scroll(offset)
    }

    fn status_text(&self) -> Option<String> {
        self.0.borrow().status_text()
    }

    fn set_bounds(&mut self, bounds: Rect) {
        self.0.borrow_mut().set_bounds(bounds)
    }
}

/// Which of the two `FileDialog` flows a pending dialog belongs to — `CM_OK`/
/// `CM_CANCEL` are shared between them, so the driver needs to remember which
/// one it opened (ADR 0026's "driver holds the disambiguating state" idiom,
/// mirroring `examples/theme_editor.rs`'s `PendingPicker`).
enum PendingAction {
    Save,
    Open,
}

/// The Open or Save As dialog currently on screen, if any — only one of
/// either kind can be open at a time (each guarded by `pending.is_some()`),
/// so a single field covers both.
struct PendingDialog {
    window_id: WindowId,
    result: FileDialogResult,
    action: PendingAction,
}

struct HelpBuilderDemo {
    shell: Shell,
    commands: CommandSet,
    theme: Theme,
    source: Rc<RefCell<TextArea>>,
    source_id: WindowId,
    preview_id: WindowId,
    preview_area: Rect,
    current_path: Option<PathBuf>,
    last_saved_text: String,
    starting_dir: PathBuf,
    pending: Option<PendingDialog>,
    finished: bool,
}

impl HelpBuilderDemo {
    fn is_dirty(&self) -> bool {
        self.source.borrow().text() != self.last_saved_text.as_str()
    }

    /// Reflects the dirty flag in the source window's title (a trailing
    /// `" *"`) — cheap enough to just recompute and reassign every tick
    /// rather than tracking whether it actually flipped.
    fn sync_title(&mut self) {
        let title = if self.is_dirty() {
            "Source *"
        } else {
            "Source"
        };
        if let Some(window) = self.shell.desktop_mut().window_mut(self.source_id) {
            window.set_title(title);
        }
    }

    fn starting_dir_for_dialog(&self) -> PathBuf {
        self.current_path
            .as_ref()
            .and_then(|p| p.parent())
            .map(PathBuf::from)
            .unwrap_or_else(|| self.starting_dir.clone())
    }

    /// Opens `window` (a freshly built `FileDialog`), positioned and recorded
    /// as `pending` — the shared second half of `open_save_as`/`open_open`.
    fn open_dialog(&mut self, mut window: Window, result: FileDialogResult, action: PendingAction) {
        window.set_bounds(centered(
            window.bounds().size(),
            self.shell.desktop_mut().bounds().size(),
        ));
        let window_id = self.shell.desktop_mut().open(window);
        self.pending = Some(PendingDialog {
            window_id,
            result,
            action,
        });
    }

    fn open_save_as(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (window, result) =
            FileDialog::save("Save Help As", self.starting_dir_for_dialog(), &self.theme);
        self.open_dialog(window, result, PendingAction::Save);
    }

    fn open_open(&mut self) {
        if self.pending.is_some() {
            return;
        }
        let (window, result) = FileDialog::open(
            "Open Help File",
            self.starting_dir_for_dialog(),
            &self.theme,
        );
        self.open_dialog(window, result, PendingAction::Open);
    }

    fn write_to(&mut self, path: PathBuf) {
        let text = self.source.borrow().text().to_string();
        if fs::write(&path, &text).is_ok() {
            self.current_path = Some(path);
            self.last_saved_text = text;
        }
    }

    /// Loads `path` into the source pane, replacing whatever was there — no
    /// unsaved-changes prompt, matching this tool's equally simple Exit (see
    /// the module doc comment). The preview is deliberately left alone until
    /// an explicit Refresh, same as after any other edit to the source text.
    fn load_from(&mut self, path: PathBuf) {
        let text = fs::read_to_string(&path).unwrap_or_default();
        self.source.borrow_mut().set_text(&text);
        self.current_path = Some(path);
        self.last_saved_text = text;
    }

    fn on_save(&mut self) {
        if self.pending.is_some() {
            return;
        }
        match self.current_path.clone() {
            Some(path) => self.write_to(path),
            None => self.open_save_as(),
        }
    }

    fn on_ok(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let path = pending.result.path();
        let mut ctx = Context::new(&self.commands);
        self.shell.desktop_mut().close(pending.window_id, &mut ctx);
        match pending.action {
            PendingAction::Save => self.write_to(path),
            PendingAction::Open => self.load_from(path),
        }
    }

    fn on_cancel(&mut self) {
        let Some(pending) = self.pending.take() else {
            return;
        };
        let mut ctx = Context::new(&self.commands);
        self.shell.desktop_mut().close(pending.window_id, &mut ctx);
    }

    fn refresh_preview(&mut self) {
        let mut ctx = Context::new(&self.commands);
        self.shell.desktop_mut().close(self.preview_id, &mut ctx);
        let contents = HelpContents::parse(self.source.borrow().text());
        let window =
            HelpWindow::build(contents, self.preview_area, "Preview", &self.theme).closable(false);
        self.preview_id = self.shell.desktop_mut().open(window);
        // `Desktop::open` makes the freshly (re)opened preview active, which
        // would otherwise steal focus away from the source pane after every
        // refresh — surprising mid-edit-refresh-edit flow, found while
        // manually exercising this tool.
        self.shell.desktop_mut().focus(self.source_id);
    }

    /// Delivers one event to the shell, queueing whatever it posts.
    fn deliver(&mut self, event: &Event, queue: &mut VecDeque<Event>) -> EventResult {
        let mut ctx = Context::new(&self.commands);
        let result = self.shell.handle_event(event, &mut ctx);
        queue.extend(ctx.take_posted());
        result
    }

    /// Dispatches `event`, then drains posted commands, re-dispatching each
    /// from the top — mirroring `examples/theme_editor.rs`'s `dispatch`, plus
    /// interception for this tool's own open/save/refresh commands and the
    /// shared `CM_OK`/`CM_CANCEL` (disambiguated via `pending`, ADR 0026's
    /// idiom).
    fn dispatch(&mut self, event: &Event) -> EventResult {
        let mut queue = VecDeque::new();
        let result = self.deliver(event, &mut queue);
        let mut budget = 1024;
        while let Some(posted) = queue.pop_front() {
            match posted {
                Event::Command(CM_OPEN) => self.open_open(),
                Event::Command(CM_SAVE) => self.on_save(),
                Event::Command(CM_SAVE_AS) => self.open_save_as(),
                Event::Command(CM_REFRESH) => self.refresh_preview(),
                Event::Command(CM_OK) => self.on_ok(),
                Event::Command(CM_CANCEL) => self.on_cancel(),
                Event::Command(CM_QUIT) => self.finished = true,
                _ => {
                    if budget == 0 {
                        break;
                    }
                    budget -= 1;
                    self.deliver(&posted, &mut queue);
                }
            }
        }
        self.sync_title();
        result
    }
}

impl Program for HelpBuilderDemo {
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

    let arg_path = std::env::args().nth(1).map(PathBuf::from);
    let (current_path, initial_text) = match &arg_path {
        Some(path) => (
            Some(path.clone()),
            fs::read_to_string(path).unwrap_or_default(),
        ),
        None => (None, String::new()),
    };
    let starting_dir = current_path
        .as_ref()
        .and_then(|p| p.parent().map(PathBuf::from))
        .unwrap_or_else(|| std::env::current_dir().unwrap_or_default());

    let menu_bar = MenuBar::new(
        rect(0, 0, size.width, 1),
        vec![Menu::new(
            "File",
            vec![
                MenuItem::new("Open...", CM_OPEN).with_shortcut("Ctrl-O"),
                MenuItem::new("Save", CM_SAVE).with_shortcut("Ctrl-S"),
                MenuItem::new("Save As...", CM_SAVE_AS)
                    .with_shortcut("Ctrl-A")
                    .with_hotkey('a'),
                MenuItem::new("Refresh Preview", CM_REFRESH).with_shortcut("Ctrl-R"),
                MenuItem::new("Exit", CM_QUIT)
                    .with_shortcut("Alt-X")
                    .with_hotkey('x'),
            ],
        )],
        &theme,
    );

    let desk_w = size.width;
    let desk_h = (size.height - 2).max(0);
    let left_w = desk_w / 2;
    let right_w = desk_w - left_w;

    let source_bounds = rect(0, 0, left_w, desk_h);
    let preview_area = rect(left_w, 0, right_w, (desk_h / 2).max(14));

    // The interior view must be sized to the window's *interior* — inset by
    // one cell on every side for the border — not the outer bounds (the same
    // gotcha `examples/text_area.rs` documents at length: getting this wrong
    // silently reflows/scrolls against a size that's 2 columns/rows too big).
    let interior = Rect::from_origin_size(
        Point::new(1, 1),
        Size::new((left_w - 2).max(0), (desk_h - 2).max(0)),
    );
    let text_area = Rc::new(RefCell::new(
        TextArea::new(interior, &theme).with_text(&initial_text),
    ));
    // No `Group` to auto-focus a lone interior (ADR 0010) — told directly, or
    // `TextArea::handle_event` ignores every key.
    text_area.borrow_mut().set_focused(true);
    let source_window = Window::new(
        source_bounds,
        "Source",
        &theme,
        Box::new(SharedTextArea(Rc::clone(&text_area))),
    )
    .closable(false)
    .status_panel(true);

    let mut desktop = Desktop::new(
        rect(0, 1, desk_w, desk_h),
        Cell::from_char('░', theme.style(Role::DesktopBackground)),
    );
    let source_id = desktop.open(source_window);

    let contents = HelpContents::parse(&initial_text);
    let preview_window =
        HelpWindow::build(contents, preview_area, "Preview", &theme).closable(false);
    let preview_id = desktop.open(preview_window);

    // A permanent reference window: this tool's own usage guide plus a
    // primer on the `.help` format it authors (ADR 0013) — baked in the same
    // way `edit`'s own built-in help is (`crates/edit/src/help.rs`). No menu
    // entry and no F1 wiring (`Shell::with_help`'s singleton, ADR 0021, is
    // opt-in and this example never calls it) — it's just always there.
    // `guide_area`'s x-origin stays within the right half's own column range
    // (clamped to `right_w`), so on a narrow terminal it shrinks rather than
    // ever creeping into Source's column.
    let guide_w = desk_w - left_w;
    let guide_h = (desk_h / 2).max(14);
    let guide_area = rect(
        left_w + (right_w - guide_w).max(0),
        (desk_h - guide_h).max(0),
        guide_w,
        guide_h,
    );
    let guide_window =
        HelpWindow::build(HelpContents::parse(GUIDE_TEXT), guide_area, "Guide", &theme)
            .closable(false);
    desktop.open(guide_window);

    // `Desktop::open` makes the newly opened window active — without this,
    // the Preview/Guide windows (opened last) would steal keyboard focus
    // from the source `TextArea`, even though it's individually
    // `set_focused(true)`. Focusing Source last also raises it to the top,
    // over Preview and Guide, while leaving Guide (opened after Preview)
    // above Preview in their shared corner.
    desktop.focus(source_id);

    let status = StatusLine::new(
        rect(0, size.height - 1, size.width, 1),
        vec![
            StatusItem::new(
                "Ctrl-O",
                "Open",
                Accelerator::new(
                    KeyEvent::new(KeyCode::Char('o'), Modifiers::CONTROL),
                    CM_OPEN,
                ),
            ),
            StatusItem::new(
                "Ctrl-S",
                "Save",
                Accelerator::new(
                    KeyEvent::new(KeyCode::Char('s'), Modifiers::CONTROL),
                    CM_SAVE,
                ),
            ),
            StatusItem::new(
                "Ctrl-R",
                "Refresh",
                Accelerator::new(
                    KeyEvent::new(KeyCode::Char('r'), Modifiers::CONTROL),
                    CM_REFRESH,
                ),
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

    let mut shell = Shell::new(size, menu_bar, desktop, status, &theme);
    // "Save As..." has no status-line slot (there's no room, and Ctrl-O/S/R
    // already cover the common path) but its Ctrl-A shortcut should still
    // work — bound directly onto the desktop (ADR 0028), with no StatusItem
    // involved. This used to be a real, silent gap: the menu showed
    // "Ctrl-A" as a reminder next to Save As... but nothing ever bound it.
    shell.desktop_mut().bind_accelerator(Accelerator::new(
        KeyEvent::new(KeyCode::Char('a'), Modifiers::CONTROL),
        CM_SAVE_AS,
    ));

    let mut demo = HelpBuilderDemo {
        shell,
        commands: CommandSet::new(),
        theme,
        source: text_area,
        source_id,
        preview_id,
        preview_area,
        current_path,
        last_saved_text: initial_text,
        starting_dir,
        pending: None,
        finished: false,
    };

    let mut app = Application::new(backend).with_timeout(Duration::from_millis(250));
    app.run(&mut demo)?;

    drop(app); // restores the terminal before we print to stdout
    match demo.current_path {
        Some(path) => println!("Saved to {path:?}"),
        None => println!("Not saved."),
    }
    Ok(())
}

// --- the shipped Guide content (a compile-in safety net, ADR 0013) ---
//
// Mirrors `edit`'s own shipped-content tests
// (`crates/edit/src/help.rs`), adapted to this crate's current
// `Block::Paragraph(Vec<Span>)` model (`edit`'s copy predates ADR 0020's
// `Span`-based links and assumes a bare `String`).

#[cfg(test)]
mod guide_content {
    use super::GUIDE_TEXT;
    use rvision::help::{Block, HelpContents, HelpTopic, Span};

    fn topic_text(t: &HelpTopic) -> String {
        let mut s = String::new();
        for block in &t.body {
            match block {
                Block::Paragraph(spans) => {
                    for span in spans {
                        match span {
                            Span::Text(text) => s.push_str(text),
                            Span::Link { label, .. } => s.push_str(label),
                        }
                    }
                    s.push('\n');
                }
                Block::Preformatted(lines) => {
                    for l in lines {
                        s.push_str(l);
                        s.push('\n');
                    }
                }
            }
        }
        s
    }

    /// Extracts every `{label|target}` target from raw markup (links are
    /// reduced to label text at parse time, so this scans the source) —
    /// skipping `<pre>`-fenced lines and any `\{`-escaped brace (ADR 0029),
    /// since the real parser never link-scans either (this Guide's own
    /// "links" topic shows the `{label|target}` syntax literally via
    /// `\{...}`, which isn't a real link to check).
    fn link_targets(src: &str) -> Vec<String> {
        let mut out = Vec::new();
        let mut in_pre = false;
        for line in src.split('\n') {
            let trimmed = line.trim();
            if trimmed == "<pre>" {
                in_pre = true;
                continue;
            }
            if trimmed == "</pre>" {
                in_pre = false;
                continue;
            }
            if in_pre {
                continue;
            }
            let mut rest = line;
            while let Some(o) = rest.find('{') {
                if o > 0 && rest.as_bytes()[o - 1] == b'\\' {
                    rest = &rest[o + 1..];
                    continue;
                }
                let after = &rest[o + 1..];
                if let Some(bar) = after.find('|') {
                    let ab = &after[bar + 1..];
                    if let Some(close) = ab.find('}') {
                        out.push(ab[..close].to_string());
                        rest = &ab[close + 1..];
                        continue;
                    }
                }
                rest = after;
            }
        }
        out
    }

    #[test]
    fn guide_parses_with_the_expected_topics() {
        let c = HelpContents::parse(GUIDE_TEXT);
        let ids: Vec<&str> = c.topics().iter().map(|t| t.id.as_str()).collect();
        assert_eq!(
            ids,
            ["overview", "usage", "format", "preformatted", "links"]
        );
    }

    #[test]
    fn guide_topic_ids_are_unique() {
        let c = HelpContents::parse(GUIDE_TEXT);
        let mut seen = std::collections::BTreeSet::new();
        for t in c.topics() {
            assert!(seen.insert(t.id.clone()), "duplicate topic id {:?}", t.id);
        }
    }

    #[test]
    fn usage_topic_documents_the_shortcuts() {
        let c = HelpContents::parse(GUIDE_TEXT);
        let text = topic_text(c.topic("usage").expect("a usage topic"));
        for key in ["Ctrl-O", "Ctrl-S", "Ctrl-A", "Ctrl-R", "Alt-X"] {
            assert!(text.contains(key), "{key} documented");
        }
    }

    #[test]
    fn every_link_target_resolves() {
        let c = HelpContents::parse(GUIDE_TEXT);
        for target in link_targets(GUIDE_TEXT) {
            assert!(
                c.topic(&target).is_some(),
                "dangling help link target {target:?}"
            );
        }
    }
}
