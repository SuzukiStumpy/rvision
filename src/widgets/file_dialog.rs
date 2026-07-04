//! A file Open/Save dialog (TurboVision's `TFileDialog`).
//!
//! Composes a [`ListBox`](super::ListBox) of directory entries, an
//! [`InputLine`](super::InputLine) for the file name, and *Open*/*Save* +
//! *Cancel* [`Button`](super::Button)s into a [`Window`](super::Window)'s
//! interior (ADR 0016). `Enter` on a directory navigates into it; `Enter` on a
//! file (or the default button) accepts the path; `Esc` cancels (via the
//! window's `esc_cancels`, not `FileDialog` itself). A left **double-click**
//! on a list entry does the same as `Enter` on it — open the file or step into
//! the folder (ADR 0007). After
//! [`exec_view`](crate::app::Application::exec_view) returns `CM_OK`,
//! [`FileDialogResult::path`] is the chosen file.
//!
//! Directory listing is read through an injected closure (real `std::fs` by
//! default), so navigation is testable without touching the filesystem.
//!
//! The embedded [`ListBox`] no longer draws or hit-tests its own scroll bar
//! (ADR 0015): `FileDialog` hosts one in the column it already reserves,
//! querying [`View::scroll_metrics`] and routing hits back through
//! [`View::set_scroll`] — the second, non-`Window` proof of the protocol
//! alongside `ListBox` itself.

use std::cell::RefCell;
use std::fs;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use crate::canvas::Canvas;
use crate::command::{CM_CANCEL, CM_OK};
use crate::event::{Event, EventResult, KeyCode, KeyEvent, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{Button, InputLine, ListBox, ScrollBar, ScrollPart, Window};

/// One directory entry: a name and whether it is a sub-directory.
#[derive(Clone)]
struct Entry {
    name: String,
    is_dir: bool,
}

/// How a [`FileDialog`] lists a directory's own entries (no `..`; the dialog adds
/// that itself). The default is [`read_dir_entries`]; tests inject a fake.
type Reader = Box<dyn Fn(&Path) -> Vec<Entry>>;

const FOCUS_LIST: usize = 0;
const FOCUS_INPUT: usize = 1;
const FOCUS_OPEN: usize = 2;
const FOCUS_CANCEL: usize = 3;
const FOCUS_COUNT: usize = 4;

/// The interior's own size — the window built around it is this plus one cell
/// of border on every side.
const WIDTH: i16 = 44;
const HEIGHT: i16 = 16;

/// A file picker's controls — the [`Window`](super::Window) interior built by
/// [`open`](FileDialog::open)/[`save`](FileDialog::save). Draws and dispatches
/// in its own **local** coordinates (`(0, 0)` is its own top-left corner); it
/// has no border, title, or shadow of its own — the `Window` around it
/// supplies all of that (ADR 0016).
pub struct FileDialog {
    theme: Theme,
    reader: Reader,
    dir: PathBuf,
    entries: Vec<Entry>,
    list: ListBox,
    list_rect: Rect,
    input: InputLine,
    open: Button,
    cancel: Button,
    focus: usize,
    /// Shared with the [`FileDialogResult`] handle returned alongside the
    /// window: written whenever a path is accepted (ADR 0016).
    result: Rc<RefCell<PathBuf>>,
    /// Whether the list's own hosted scroll bar's thumb is mid-drag (ADR
    /// 0027) — set on a `Down` hit on it, alongside a mouse-capture request
    /// so `Desktop` keeps forwarding events here regardless of pointer
    /// position; cleared on `Up`.
    dragging_thumb: bool,
}

/// A handle to the path a [`FileDialog`] was accepted with, readable after
/// [`exec_view`](crate::app::Application::exec_view) returns `CM_OK`.
/// `FileDialog` itself becomes the window's boxed, type-erased interior
/// (ADR 0003), so this is the narrow seam back out — the same shared-cell
/// idiom used throughout the crate's own tests, not a new pattern.
#[derive(Clone)]
pub struct FileDialogResult(Rc<RefCell<PathBuf>>);

impl FileDialogResult {
    /// The path the dialog was last accepted with (empty if never accepted).
    pub fn path(&self) -> PathBuf {
        self.0.borrow().clone()
    }
}

impl FileDialog {
    /// An *Open* dialog titled `title`, starting in `dir`: a centred, fixed,
    /// `Esc`-cancels [`Window`](super::Window) ending on `CM_OK`/`CM_CANCEL`,
    /// plus the handle to read the chosen path from.
    pub fn open(title: &str, dir: impl Into<PathBuf>, theme: &Theme) -> (Window, FileDialogResult) {
        Self::build(title, dir.into(), "Open", theme, Box::new(read_dir_entries))
    }

    /// A *Save* dialog, mirroring [`open`](Self::open).
    pub fn save(title: &str, dir: impl Into<PathBuf>, theme: &Theme) -> (Window, FileDialogResult) {
        Self::build(title, dir.into(), "Save", theme, Box::new(read_dir_entries))
    }

    /// Assembles the window around a fresh [`FileDialog`] interior (the
    /// `reader` seam makes navigation testable).
    fn build(
        title: &str,
        dir: PathBuf,
        accept: &str,
        theme: &Theme,
        reader: Reader,
    ) -> (Window, FileDialogResult) {
        let dialog = Self::with_reader(dir, accept, theme, reader);
        let result = FileDialogResult(Rc::clone(&dialog.result));
        let window = Window::dialog(
            Rect::from_origin_size(Point::new(0, 0), Size::new(WIDTH + 2, HEIGHT + 2)),
            title,
            theme,
            Box::new(dialog),
        )
        .centered()
        .resizable(false)
        .zoomable(false)
        // No system box — TV file dialogs don't show one either, and neither
        // CM_CLOSE nor CM_ZOOM is a registered ending command here (ADR 0016).
        .closable(false)
        .esc_cancels(true)
        .also_ends_on(CM_OK)
        .also_ends_on(CM_CANCEL);
        (window, result)
    }

    /// The shared constructor behind [`open`](Self::open)/[`save`](Self::save).
    fn with_reader(dir: PathBuf, accept: &str, theme: &Theme, reader: Reader) -> Self {
        let list_rect = rect(0, 4, WIDTH, HEIGHT - 6);

        let mut dialog = Self {
            theme: theme.clone(),
            reader,
            dir: PathBuf::new(),
            entries: Vec::new(),
            list: ListBox::new(list_rect, Vec::new(), theme),
            list_rect,
            input: InputLine::new(rect(0, 1, WIDTH, 1), theme),
            open: Button::new(rect(WIDTH - 24, HEIGHT - 1, 10, 1), accept, CM_OK, theme)
                .default(true),
            cancel: Button::new(
                rect(WIDTH - 12, HEIGHT - 1, 10, 1),
                "Cancel",
                CM_CANCEL,
                theme,
            ),
            focus: FOCUS_LIST,
            result: Rc::new(RefCell::new(PathBuf::new())),
            dragging_thumb: false,
        };
        dialog.set_dir(dir);
        dialog
    }

    /// The path the dialog currently points at: the directory joined with the
    /// name field.
    pub fn path(&self) -> PathBuf {
        self.dir.join(self.input.text())
    }

    /// Records `path()` as the accepted result and posts `CM_OK`.
    fn accept(&mut self, ctx: &mut Context) {
        *self.result.borrow_mut() = self.path();
        ctx.post(CM_OK);
    }

    /// Reads `dir`, rebuilds the list (with a leading `..` unless at the root),
    /// clears the name field, and returns focus to the list.
    fn set_dir(&mut self, dir: PathBuf) {
        let mut entries = (self.reader)(&dir);
        if dir.parent().is_some() {
            entries.insert(
                0,
                Entry {
                    name: "..".to_string(),
                    is_dir: true,
                },
            );
        }
        let display: Vec<String> = entries.iter().map(display_of).collect();
        self.list = ListBox::new(self.list_rect, display, &self.theme);
        self.entries = entries;
        self.dir = dir;
        self.input.set_text("");
        self.focus = FOCUS_LIST;
        self.apply_focus();
    }

    /// Pushes the focus flag to whichever control now holds it (ADR 0010).
    fn apply_focus(&mut self) {
        self.list.set_focused(self.focus == FOCUS_LIST);
        self.input.set_focused(self.focus == FOCUS_INPUT);
        self.open.set_focused(self.focus == FOCUS_OPEN);
        self.cancel.set_focused(self.focus == FOCUS_CANCEL);
    }

    /// Moves focus `delta` steps round the four controls.
    fn move_focus(&mut self, delta: isize) {
        let n = FOCUS_COUNT as isize;
        self.focus = (((self.focus as isize + delta) % n + n) % n) as usize;
        self.apply_focus();
    }

    /// Navigates into entry `index` (a directory or `..`).
    fn navigate(&mut self, index: usize) {
        let entry = &self.entries[index];
        let new_dir = if entry.name == ".." {
            self.dir
                .parent()
                .map(Path::to_path_buf)
                .unwrap_or_else(|| self.dir.clone())
        } else {
            self.dir.join(&entry.name)
        };
        self.set_dir(new_dir);
    }

    /// Mirrors the highlighted list entry into the name field (blank for a
    /// directory), the way a real file picker does.
    fn sync_input_from_list(&mut self) {
        if let Some(i) = self.list.selected() {
            let entry = &self.entries[i];
            let text = if entry.is_dir { "" } else { &entry.name };
            self.input.set_text(text);
        }
    }

    /// The scroll bar hosting the list's overflow, if it currently has any
    /// (ADR 0015) — positioned in the list's own local coordinates, along its
    /// rightmost column.
    fn list_scroll_bar(&self) -> Option<ScrollBar> {
        let vertical = self.list.scroll_metrics()?.vertical?;
        let bounds = self.list.bounds();
        let column = Rect::from_origin_size(
            Point::new(bounds.width() - 1, 0),
            Size::new(1, bounds.height()),
        );
        let mut bar = ScrollBar::new(column, self.theme.style(Role::DialogBackground));
        bar.set_metrics(vertical.total, vertical.visible, vertical.pos);
        Some(bar)
    }

    /// Scrolls the list by `delta` rows (negative = up), clamped, via the
    /// scroll protocol (`View::set_scroll`) rather than reaching into it.
    fn scroll_list_by(&mut self, delta: isize) {
        let Some(vertical) = self.list.scroll_metrics().and_then(|m| m.vertical) else {
            return;
        };
        let max_top = vertical.total.saturating_sub(vertical.visible) as isize;
        let new_top = (vertical.pos as isize + delta).clamp(0, max_top);
        self.list.set_scroll(Point::new(0, new_top as i16));
    }

    /// If `local` (in the list's own local coordinates) is a press/double-click
    /// landing on the hosted scroll bar, scrolls accordingly and reports `true`
    /// — the caller skips forwarding the event into the list itself. `false`
    /// means "not a bar hit," including when the list has no bar to hit. A hit
    /// on the thumb itself starts a drag (ADR 0027) instead of scrolling
    /// directly: marks `dragging_thumb` and asks `Desktop` (via `ctx`) to keep
    /// forwarding events here regardless of pointer position, until release.
    fn handle_list_bar_hit(&mut self, local: Point, kind: MouseKind, ctx: &mut Context) -> bool {
        if !matches!(
            kind,
            MouseKind::Down(MouseButton::Left) | MouseKind::DoubleClick(MouseButton::Left)
        ) {
            return false;
        }
        let Some(bar) = self.list_scroll_bar() else {
            return false;
        };
        let Some(part) = bar.hit(local) else {
            return false;
        };
        if part == ScrollPart::Thumb {
            self.dragging_thumb = true;
            ctx.request_mouse_capture();
            return true;
        }
        let visible = self
            .list
            .scroll_metrics()
            .and_then(|m| m.vertical)
            .map(|v| v.visible)
            .unwrap_or(1);
        let page = visible.max(1) as isize;
        match part {
            ScrollPart::LineUp => self.scroll_list_by(-1),
            ScrollPart::LineDown => self.scroll_list_by(1),
            ScrollPart::PageUp => self.scroll_list_by(-page),
            ScrollPart::PageDown => self.scroll_list_by(page),
            ScrollPart::Thumb => unreachable!("handled above"),
        }
        true
    }

    /// `Enter`: navigate into a directory, accept a file/typed path, or
    /// activate the focused button.
    fn on_enter(&mut self, ctx: &mut Context) -> EventResult {
        match self.focus {
            FOCUS_LIST => match self.list.selected() {
                Some(i) if self.entries[i].is_dir => {
                    self.navigate(i);
                    EventResult::Consumed
                }
                Some(i) => {
                    self.input.set_text(&self.entries[i].name);
                    self.accept(ctx);
                    EventResult::Consumed
                }
                None => EventResult::Ignored,
            },
            FOCUS_CANCEL => {
                ctx.post(CM_CANCEL);
                EventResult::Consumed
            }
            // The name field or the Open/Save button: accept the typed path.
            _ => {
                self.accept(ctx);
                EventResult::Consumed
            }
        }
    }

    /// Routes a non-navigation key to the focused control.
    fn route(&mut self, key: KeyEvent, ctx: &mut Context) -> EventResult {
        let event = Event::Key(key);
        match self.focus {
            FOCUS_LIST => {
                let result = self.list.handle_event(&event, ctx);
                self.sync_input_from_list();
                result
            }
            FOCUS_INPUT => self.input.handle_event(&event, ctx),
            FOCUS_OPEN => self.open.handle_event(&event, ctx),
            FOCUS_CANCEL => self.cancel.handle_event(&event, ctx),
            _ => EventResult::Ignored,
        }
    }

    /// Routes a mouse event (already in this interior's own local
    /// coordinates — the owning `Window` translates into it, ADR 0016) to the
    /// control under the pointer, focusing it on a left-press. Clicking the
    /// list mirrors arrow navigation by syncing the name field to the new
    /// selection.
    fn handle_mouse(&mut self, m: &MouseEvent, ctx: &mut Context) -> EventResult {
        // A thumb drag in progress (ADR 0027) takes every event ahead of the
        // usual positional lookup below — `Desktop`'s mouse capture keeps
        // delivering these regardless of where the pointer strayed, so
        // position isn't re-checked here either.
        if matches!(m.kind, MouseKind::Up(MouseButton::Left)) {
            let was_dragging = self.dragging_thumb;
            self.dragging_thumb = false;
            return if was_dragging {
                EventResult::Consumed
            } else {
                EventResult::Ignored
            };
        }
        if self.dragging_thumb {
            if let MouseKind::Drag(MouseButton::Left) = m.kind {
                if let Some(bar) = self.list_scroll_bar() {
                    let list_origin = self.list.bounds().origin();
                    let local = m.pos.offset(-list_origin.x, -list_origin.y);
                    let target = bar.pos_at(local);
                    let current = self
                        .list
                        .scroll_metrics()
                        .and_then(|sm| sm.vertical)
                        .map(|v| v.pos)
                        .unwrap_or(0);
                    self.scroll_list_by(target as isize - current as isize);
                }
                return EventResult::Consumed;
            }
        }

        let p = m.pos;
        let bounds = [
            self.list.bounds(),
            self.input.bounds(),
            self.open.bounds(),
            self.cancel.bounds(),
        ];
        let Some(i) = bounds.iter().position(|b| b.contains(p)) else {
            return EventResult::Ignored;
        };
        let pressed = matches!(
            m.kind,
            MouseKind::Down(MouseButton::Left) | MouseKind::DoubleClick(MouseButton::Left)
        );
        if pressed {
            self.focus = i;
            self.apply_focus();
        }
        let b = bounds[i];
        let local_pos = p.offset(-b.origin().x, -b.origin().y);
        let local = Event::Mouse(MouseEvent {
            pos: local_pos,
            ..*m
        });
        match i {
            FOCUS_LIST => {
                let result = if self.handle_list_bar_hit(local_pos, m.kind, ctx) {
                    EventResult::Consumed
                } else {
                    self.list.handle_event(&local, ctx)
                };
                self.sync_input_from_list();
                // A double-click on the list is "select and accept": run the same
                // navigate-into-a-directory / open-a-file path as Enter (ADR 0007).
                if matches!(m.kind, MouseKind::DoubleClick(MouseButton::Left)) {
                    return self.on_enter(ctx);
                }
                result
            }
            FOCUS_INPUT => self.input.handle_event(&local, ctx),
            FOCUS_OPEN => self.open.handle_event(&local, ctx),
            FOCUS_CANCEL => self.cancel.handle_event(&local, ctx),
            _ => EventResult::Ignored,
        }
    }
}

/// How an entry shows in the list: `..`, a directory `name/`, or a file `name`.
fn display_of(entry: &Entry) -> String {
    if entry.name == ".." {
        "..".to_string()
    } else if entry.is_dir {
        format!("{}/", entry.name)
    } else {
        entry.name.clone()
    }
}

/// Reads a directory's own entries from the real filesystem — sub-directories
/// first, then files, each group sorted by name. Unreadable directories list
/// empty rather than erroring.
fn read_dir_entries(path: &Path) -> Vec<Entry> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    if let Ok(read) = fs::read_dir(path) {
        for entry in read.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let is_dir = entry.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if is_dir {
                dirs.push(Entry { name, is_dir });
            } else {
                files.push(Entry { name, is_dir });
            }
        }
    }
    dirs.sort_by(|a, b| a.name.cmp(&b.name));
    files.sort_by(|a, b| a.name.cmp(&b.name));
    dirs.into_iter().chain(files).collect()
}

fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

impl View for FileDialog {
    fn bounds(&self) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(WIDTH, HEIGHT))
    }

    fn draw(&self, canvas: &mut Canvas) {
        canvas.put_str(
            Point::new(0, 0),
            "Name:",
            self.theme.style(Role::DialogBackground),
        );
        canvas.put_str(
            Point::new(0, 3),
            "Files:",
            self.theme.style(Role::DialogBackground),
        );
        for control in [&self.input as &dyn View, &self.open, &self.cancel] {
            let mut child = canvas.child(control.bounds());
            control.draw(&mut child);
        }
        {
            let mut child = canvas.child(self.list.bounds());
            self.list.draw(&mut child);
            // Host the list's scroll bar in the column it draws over, if it
            // currently has overflow to show (ADR 0015) — drawn last so it
            // sits on top of whatever the list painted underneath.
            if let Some(bar) = self.list_scroll_bar() {
                let mut bar_canvas = child.child(bar.bounds());
                bar.draw(&mut bar_canvas);
            }
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        let key = match event {
            Event::Key(key) => key,
            Event::Mouse(m) => return self.handle_mouse(m, ctx),
            _ => return EventResult::Ignored,
        };
        match key.code {
            KeyCode::Tab => {
                self.move_focus(1);
                EventResult::Consumed
            }
            KeyCode::BackTab => {
                self.move_focus(-1);
                EventResult::Consumed
            }
            KeyCode::Enter => self.on_enter(ctx),
            _ => self.route(*key, ctx),
        }
    }

    fn focusable(&self) -> bool {
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::{CM_USER, Command, CommandSet};
    use crate::event::Modifiers;

    fn entry(name: &str, is_dir: bool) -> Entry {
        Entry {
            name: name.to_string(),
            is_dir,
        }
    }

    /// A fake filesystem: `/root` holds a sub-dir and two files; `/root/sub`
    /// holds one file.
    fn fake_reader() -> Reader {
        // Match by `Path` equality, not the raw string: navigating builds child
        // paths with `PathBuf::join`, which uses the platform separator, so on
        // Windows `/root` + `sub` is `/root\sub`. `Path` comparison treats `/`
        // and `\` as equivalent there, keeping this fake filesystem cross-platform.
        Box::new(|path: &Path| {
            if path == Path::new("/root") {
                vec![
                    entry("sub", true),
                    entry("a.txt", false),
                    entry("b.txt", false),
                ]
            } else if path == Path::new("/root/sub") {
                vec![entry("c.txt", false)]
            } else {
                vec![]
            }
        })
    }

    /// A bare interior over the fake `/root` filesystem, driven directly (its
    /// own local coordinates — no border, `(0, 0)` is its own corner).
    fn dialog() -> FileDialog {
        FileDialog::with_reader(
            PathBuf::from("/root"),
            "Open",
            &Theme::default(),
            fake_reader(),
        )
    }

    /// A fake filesystem with more files than the list's rows can show at
    /// once, so its embedded `ListBox` reports scroll overflow (ADR 0015).
    fn fake_reader_many() -> Reader {
        Box::new(|path: &Path| {
            if path == Path::new("/many") {
                (0..15)
                    .map(|i| entry(&format!("f{i:02}.txt"), false))
                    .collect()
            } else {
                vec![]
            }
        })
    }

    fn dialog_with_many_files() -> FileDialog {
        FileDialog::with_reader(
            PathBuf::from("/many"),
            "Open",
            &Theme::default(),
            fake_reader_many(),
        )
    }

    fn press(d: &mut FileDialog, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        d.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn names(d: &FileDialog) -> Vec<String> {
        d.entries.iter().map(|e| e.name.clone()).collect()
    }

    #[test]
    fn lists_directories_first_with_a_parent_entry() {
        let d = dialog();
        assert_eq!(names(&d), vec!["..", "sub", "a.txt", "b.txt"]);
        assert!(d.entries[1].is_dir, "sub is a directory");
        assert!(!d.entries[2].is_dir, "a.txt is a file");
    }

    #[test]
    fn clicking_a_file_in_the_list_selects_it_and_fills_the_name() {
        let mut d = dialog(); // entries: .., sub, a.txt, b.txt
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // The list starts at local (0, 4); "a.txt" is its row 2 → y = 6.
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(2, 6),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(d.focus, FOCUS_LIST);
        assert_eq!(d.list.selected(), Some(2));
        assert_eq!(d.input.text(), "a.txt", "the name field follows the click");
    }

    #[test]
    fn double_clicking_a_file_accepts_it() {
        let mut d = dialog(); // entries: .., sub, a.txt, b.txt
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // "a.txt" is the list's row 2 → local y = 6. Double-click = open.
        let dc = Event::Mouse(MouseEvent {
            kind: MouseKind::DoubleClick(MouseButton::Left),
            pos: Point::new(2, 6),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&dc, &mut ctx), EventResult::Consumed);
        assert_eq!(d.list.selected(), Some(2));
        assert_eq!(d.input.text(), "a.txt");
        assert_eq!(
            ctx.take_posted(),
            vec![Event::Command(CM_OK)],
            "double-clicking a file accepts, like select + Enter"
        );
        assert_eq!(d.path(), PathBuf::from("/root/a.txt"));
    }

    #[test]
    fn double_clicking_a_directory_navigates_into_it() {
        let mut d = dialog();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        // "sub" is the list's row 1 → local y = 5.
        let dc = Event::Mouse(MouseEvent {
            kind: MouseKind::DoubleClick(MouseButton::Left),
            pos: Point::new(2, 5),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&dc, &mut ctx), EventResult::Consumed);
        assert_eq!(d.dir, PathBuf::from("/root/sub"));
        assert_eq!(names(&d), vec!["..", "c.txt"]);
        assert!(
            ctx.take_posted().is_empty(),
            "navigating into a folder accepts nothing"
        );
    }

    #[test]
    fn enter_on_a_directory_navigates_into_it() {
        let mut d = dialog();
        press(&mut d, KeyCode::Down); // select "sub"
        press(&mut d, KeyCode::Enter);
        assert_eq!(d.dir, PathBuf::from("/root/sub"));
        assert_eq!(names(&d), vec!["..", "c.txt"]);
    }

    #[test]
    fn dotdot_navigates_to_the_parent() {
        let mut d = dialog();
        press(&mut d, KeyCode::Down);
        press(&mut d, KeyCode::Enter); // into /root/sub
        assert_eq!(d.dir, PathBuf::from("/root/sub"));
        // ".." is the first entry; Enter on it goes back up.
        press(&mut d, KeyCode::Enter);
        assert_eq!(d.dir, PathBuf::from("/root"));
    }

    #[test]
    fn enter_on_a_file_accepts_with_its_path() {
        let mut d = dialog();
        press(&mut d, KeyCode::Down); // sub
        press(&mut d, KeyCode::Down); // a.txt
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = d.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
        assert_eq!(d.path(), PathBuf::from("/root/a.txt"));
    }

    #[test]
    fn typing_in_the_name_field_builds_the_path() {
        let mut d = dialog();
        press(&mut d, KeyCode::Tab); // focus the name field
        assert_eq!(d.focus, FOCUS_INPUT);
        for c in "new.txt".chars() {
            press(&mut d, KeyCode::Char(c));
        }
        assert_eq!(d.path(), PathBuf::from("/root/new.txt"));
    }

    #[test]
    fn tab_cycles_focus_through_the_four_controls() {
        let mut d = dialog();
        assert_eq!(d.focus, FOCUS_LIST);
        press(&mut d, KeyCode::Tab);
        assert_eq!(d.focus, FOCUS_INPUT);
        press(&mut d, KeyCode::Tab);
        assert_eq!(d.focus, FOCUS_OPEN);
        press(&mut d, KeyCode::Tab);
        assert_eq!(d.focus, FOCUS_CANCEL);
        press(&mut d, KeyCode::Tab);
        assert_eq!(d.focus, FOCUS_LIST, "wraps");
        press(&mut d, KeyCode::BackTab);
        assert_eq!(d.focus, FOCUS_CANCEL, "and back the other way");
    }

    #[test]
    fn snapshot_file_dialog_interior() {
        let d = dialog();
        let mut buf = Buffer::new(d.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        d.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    // --- Hosting the embedded list's scroll bar (ADR 0015) ---

    #[test]
    fn a_short_listing_has_no_scroll_bar_to_host() {
        let d = dialog(); // /root: only 4 entries, well under the list's rows
        assert!(d.list_scroll_bar().is_none());
    }

    #[test]
    fn a_long_listing_gets_a_hosted_scroll_bar() {
        let d = dialog_with_many_files();
        assert!(
            d.list_scroll_bar().is_some(),
            "15 entries overflow the list's visible rows"
        );
    }

    #[test]
    fn snapshot_file_dialog_interior_with_scroll_bar() {
        let d = dialog_with_many_files();
        let mut buf = Buffer::new(d.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        d.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn clicking_the_hosted_bars_down_arrow_scrolls_without_selecting_or_navigating() {
        let mut d = dialog_with_many_files();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let before_dir = d.dir.clone();
        let before_selection = d.list.selected();
        let before_pos = d.list.scroll_metrics().unwrap().vertical.unwrap().pos;

        // The bar sits in the list's rightmost column, its foot on the list's
        // last visible row — both derived from the same rect the dialog lays
        // the list out in, so this stays correct if that layout ever changes.
        let bar = d.list_scroll_bar().expect("15 entries overflow 10 rows");
        let bounds = d.list.bounds();
        let foot = Point::new(bounds.width() - 1, bounds.height() - 1);
        assert_eq!(bar.hit(foot), Some(ScrollPart::LineDown));
        let list_origin = d.list.bounds().origin();
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: foot.offset(list_origin.x, list_origin.y),
            modifiers: Modifiers::NONE,
        });

        assert_eq!(d.handle_event(&click, &mut ctx), EventResult::Consumed);
        let after_pos = d.list.scroll_metrics().unwrap().vertical.unwrap().pos;
        assert_eq!(after_pos, before_pos + 1, "the down arrow scrolled by one");
        assert_eq!(d.dir, before_dir, "a bar click never navigates");
        assert_eq!(
            d.list.selected(),
            before_selection,
            "a bar click never changes the selection"
        );
        assert!(ctx.posted().is_empty(), "and never posts a command");
    }

    #[test]
    fn dragging_the_hosted_bars_thumb_scrolls_the_list() {
        let mut d = dialog_with_many_files();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let before_selection = d.list.selected();
        let list_origin = d.list.bounds().origin();

        // List-local (43, 1): the thumb at scroll pos 0 (one row under the up
        // arrow, on a 10-row-visible/16-entry list) — list-local to
        // interior-local via the list's own origin, same as the bar-arrow
        // test above.
        let down = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(43, 1).offset(list_origin.x, list_origin.y),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&down, &mut ctx), EventResult::Consumed);
        assert_eq!(
            d.list.scroll_metrics().unwrap().vertical.unwrap().pos,
            0,
            "anchors only, no scroll yet"
        );
        assert!(
            ctx.take_mouse_capture_request(),
            "asks Desktop to keep delivering events regardless of position"
        );

        let drag = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(43, 4).offset(list_origin.x, list_origin.y),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&drag, &mut ctx), EventResult::Consumed);
        assert_eq!(d.list.scroll_metrics().unwrap().vertical.unwrap().pos, 3);
        assert_eq!(
            d.list.selected(),
            before_selection,
            "a thumb drag never changes the selection"
        );

        let up = Event::Mouse(MouseEvent {
            kind: MouseKind::Up(MouseButton::Left),
            pos: Point::new(43, 4).offset(list_origin.x, list_origin.y),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&up, &mut ctx), EventResult::Consumed);

        let stray = Event::Mouse(MouseEvent {
            kind: MouseKind::Drag(MouseButton::Left),
            pos: Point::new(43, 9).offset(list_origin.x, list_origin.y),
            modifiers: Modifiers::NONE,
        });
        d.handle_event(&stray, &mut ctx);
        assert_eq!(
            d.list.scroll_metrics().unwrap().vertical.unwrap().pos,
            3,
            "no further scroll after Up"
        );
    }

    #[test]
    fn clicking_a_row_far_from_the_bar_selects_it_even_while_scrolled() {
        // Regression: a click anywhere in the bar's *row* range — even nowhere
        // near its column — used to be misread as a scroll-bar hit
        // (`ScrollBar::hit` didn't check the cross-axis coordinate), reported
        // as clicking a file in a real, overflowing directory listing
        // appearing to "reset" the view instead of selecting what was
        // actually clicked.
        let mut d = dialog_with_many_files();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        d.scroll_list_by(3); // top = 3: row 0 now shows f02.txt
        let list_origin = d.list.bounds().origin();
        let click = Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(2, 2).offset(list_origin.x, list_origin.y),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&click, &mut ctx), EventResult::Consumed);
        assert_eq!(
            d.list.scroll_metrics().unwrap().vertical.unwrap().pos,
            3,
            "a plain content click never scrolls"
        );
        assert_eq!(d.list.selected(), Some(5), "row top(3) + local y(2)");
        assert_eq!(d.input.text(), "f04.txt");
    }

    #[test]
    fn double_clicking_a_row_far_from_the_bar_accepts_it_even_while_scrolled() {
        let mut d = dialog_with_many_files();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        d.scroll_list_by(3);
        let list_origin = d.list.bounds().origin();
        let dc = Event::Mouse(MouseEvent {
            kind: MouseKind::DoubleClick(MouseButton::Left),
            pos: Point::new(2, 2).offset(list_origin.x, list_origin.y),
            modifiers: Modifiers::NONE,
        });
        assert_eq!(d.handle_event(&dc, &mut ctx), EventResult::Consumed);
        assert_eq!(d.list.selected(), Some(5));
        assert_eq!(ctx.take_posted(), vec![Event::Command(CM_OK)]);
        assert_eq!(d.path(), PathBuf::from("/many/f04.txt"));
    }

    // --- The assembled Window (ADR 0016): chrome, ending, Esc ---

    /// The real assembly path (`build`, same as `open`/`save`), but over the
    /// fake `/root` filesystem so these tests are deterministic regardless of
    /// what's really on disk.
    fn window() -> (Window, FileDialogResult) {
        FileDialog::build(
            "Open",
            PathBuf::from("/root"),
            "Open",
            &Theme::default(),
            fake_reader(),
        )
    }

    #[test]
    fn open_builds_a_centred_fixed_window_ending_on_ok_and_cancel() {
        let (w, _) = window();
        assert_eq!(w.placement(), crate::widgets::Placement::Centered);
        assert!(w.ends_on(CM_OK));
        assert!(w.ends_on(CM_CANCEL));
        assert!(!w.ends_on(Command(CM_USER + 1)));
    }

    #[test]
    fn esc_cancels_via_the_window_not_the_interior() {
        let (mut w, _) = window();
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let esc = Event::Key(KeyEvent::new(KeyCode::Esc, Modifiers::NONE));
        assert_eq!(w.handle_event(&esc, &mut ctx), EventResult::Consumed);
        assert_eq!(ctx.posted(), &[Event::Command(CM_CANCEL)]);
    }

    #[test]
    fn accepting_a_file_updates_the_result_handle() {
        let (mut w, result) = window();
        assert_eq!(result.path(), PathBuf::new());
        // Down twice (past "..", "sub") to "a.txt", then Enter accepts it.
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        w.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Down, Modifiers::NONE)),
            &mut ctx,
        );
        w.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Down, Modifiers::NONE)),
            &mut ctx,
        );
        let r = w.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::Enter, Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(r, EventResult::Consumed);
        assert_eq!(result.path(), PathBuf::from("/root/a.txt"));
    }

    #[test]
    fn snapshot_file_dialog_window() {
        let (w, _) = window();
        let mut buf = Buffer::new(w.bounds().size());
        let mut canvas = Canvas::new(&mut buf);
        w.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }
}
