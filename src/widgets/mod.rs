//! The widget family: the concrete [`View`](crate::view::View)s that make a
//! screen look like TurboVision.
//!
//! - **Chrome (Phase 4):** a desktop backdrop, framed windows, a status line, and
//!   a menu bar with pull-downs — the furniture, laid out and routed by
//!   [`crate::app::Shell`] (ADR 0009).
//! - **Windows & controls (Phase 5, unified ADR 0016):** [`Window`] (and its
//!   [`MessageBox`]/[`FileDialog`] configurations) runs either tree-resident on
//!   a [`Desktop`] or modally via
//!   [`Application::exec_view`](crate::app::Application::exec_view), holding the
//!   focusable controls [`Button`], [`Label`], [`InputLine`], [`CheckBox`],
//!   [`RadioButtons`], [`ListBox`], [`ScrollBar`], and [`GroupBox`] (a titled
//!   border grouping a `Group` of them). Focus-aware drawing is the
//!   `set_focused` push (ADR 0010).
//!
//! All are reusable and editor-agnostic; the editor view itself arrives in Phase 6.

mod background;
mod button;
mod check_box;
mod color_picker;
mod combo_box;
mod context_menu;
mod desktop;
mod file_dialog;
mod frame;
mod group_box;
mod help_pane;
mod help_window;
mod input_line;
mod label;
mod list_box;
mod menu;
mod message_box;
mod radio_buttons;
mod scroll_bar;
mod status;
mod status_panel;
mod text_area;
mod text_edit;
mod theme_editor;
mod theme_picker;
mod window;

pub use background::Background;
pub use button::Button;
pub use check_box::CheckBox;
pub use color_picker::{ColorPicker, ColorPickerResult};
pub use combo_box::ComboBox;
pub(crate) use context_menu::ContextMenu;
pub use desktop::{Desktop, WindowId};
pub use file_dialog::{FileDialog, FileDialogResult};
pub use frame::Frame;
pub use group_box::GroupBox;
pub use help_pane::HelpPane;
pub use help_window::HelpWindow;
pub use input_line::InputLine;
pub use label::Label;
pub use list_box::ListBox;
pub use menu::{Menu, MenuBar, MenuItem};
pub use message_box::MessageBox;
pub use radio_buttons::RadioButtons;
pub use scroll_bar::{Orientation, ScrollBar, ScrollPart};
pub use status::{StatusItem, StatusLine};
pub use status_panel::StatusPanel;
pub use text_area::{CursorPosition, TextArea};
pub use theme_editor::{ThemeEditor, ThemeEditorHandle};
pub use theme_picker::{ThemePicker, ThemePickerResult};
pub use window::{Placement, Window};
