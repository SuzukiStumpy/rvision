//! The status line: a bottom row of hot-key hints (TurboVision's
//! `TStatusLine`).
//!
//! Each [`StatusItem`] pairs a shown hint (`F1`, `Alt-X`) and label (`Help`,
//! `Exit`) with the [`Accelerator`] that actually fires it — pairing them at
//! construction makes a shown hint structurally incapable of lacking a real
//! binding behind it. `StatusLine` itself is a pure display widget: it draws
//! the hints and nothing more. The binding + firing lives on
//! [`Desktop`](super::Desktop)'s global accelerator table instead
//! (ADR 0028) — `Shell::new` feeds every item's `Accelerator` into it, so
//! building a `StatusLine`, as before, is enough to get a working shortcut,
//! but a shortcut no longer *needs* a status-line slot to work at all.

use crate::canvas::Canvas;
use crate::color::Style;
use crate::command::Accelerator;
use crate::geometry::{Point, Rect};
use crate::view::View;

/// One labelled hot-key hint on the status line.
pub struct StatusItem {
    hint: String,
    label: String,
    accelerator: Accelerator,
}

impl StatusItem {
    /// Creates an item shown as `hint` + `label` (e.g. `"F1"`, `"Help"`),
    /// backed by `accelerator` — the key that actually fires it, registered
    /// separately into `Desktop`'s table (ADR 0028).
    pub fn new(hint: &str, label: &str, accelerator: Accelerator) -> Self {
        Self {
            hint: hint.to_string(),
            label: label.to_string(),
            accelerator,
        }
    }
}

/// A row of [`StatusItem`]s.
pub struct StatusLine {
    bounds: Rect,
    items: Vec<StatusItem>,
    style: Style,
    key_style: Style,
}

impl StatusLine {
    /// Creates a status line at `bounds` from `items`, drawing labels in `style`
    /// and the key hints in `key_style`.
    pub fn new(bounds: Rect, items: Vec<StatusItem>, style: Style, key_style: Style) -> Self {
        Self {
            bounds,
            items,
            style,
            key_style,
        }
    }

    /// Repositions the status line (the shell calls this as the terminal resizes).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    /// Every item's binding, in order — `Shell::new` feeds these into
    /// `Desktop`'s global accelerator table (ADR 0028) at construction.
    pub(crate) fn accelerators(&self) -> impl Iterator<Item = Accelerator> + '_ {
        self.items.iter().map(|item| item.accelerator)
    }
}

impl View for StatusLine {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        canvas.fill(area, &crate::cell::Cell::blank(self.style));
        // " hint label   hint label  …" — the hint in key_style, the label in the
        // bar style, two spaces between items.
        let mut x = 1;
        for item in &self.items {
            x = canvas.put_str(Point::new(x, 0), &item.hint, self.key_style);
            x = canvas.put_str(Point::new(x, 0), " ", self.style);
            x = canvas.put_str(Point::new(x, 0), &item.label, self.style);
            x += 2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::{CM_QUIT, Command, CommandSet};
    use crate::event::{Event, EventResult, KeyCode, KeyEvent, Modifiers};
    use crate::geometry::Size;
    use crate::view::Context;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    const CM_HELP: Command = Command(crate::command::CM_USER + 1);

    fn line(bounds: Rect) -> StatusLine {
        StatusLine::new(
            bounds,
            vec![
                StatusItem::new(
                    "F1",
                    "Help",
                    Accelerator::new(KeyEvent::new(KeyCode::F(1), Modifiers::NONE), CM_HELP),
                ),
                StatusItem::new(
                    "Alt-X",
                    "Exit",
                    Accelerator::new(KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT), CM_QUIT),
                ),
            ],
            Style::new(),
            Style::new(),
        )
    }

    #[test]
    fn snapshot_status_line() {
        let sl = line(rect(0, 0, 30, 1));
        let mut buf = Buffer::new(Size::new(30, 1));
        let mut canvas = Canvas::new(&mut buf);
        sl.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn status_line_no_longer_intercepts_keys_itself() {
        // Binding + firing moved to Desktop's global accelerator table (ADR
        // 0028); StatusLine is now purely a display widget.
        let mut sl = line(rect(0, 0, 30, 1));
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let r = sl.handle_event(
            &Event::Key(KeyEvent::new(KeyCode::F(1), Modifiers::NONE)),
            &mut ctx,
        );
        assert_eq!(r, EventResult::Ignored);
        assert!(ctx.posted().is_empty());
    }

    #[test]
    fn accelerators_yields_every_items_binding_in_order() {
        let sl = line(rect(0, 0, 30, 1));
        let f1 = Accelerator::new(KeyEvent::new(KeyCode::F(1), Modifiers::NONE), CM_HELP);
        let alt_x = Accelerator::new(KeyEvent::new(KeyCode::Char('x'), Modifiers::ALT), CM_QUIT);
        assert_eq!(sl.accelerators().collect::<Vec<_>>(), vec![f1, alt_x]);
    }
}
