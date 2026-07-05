//! A small display-only slot for one line of view-supplied status text (a
//! `TextArea`'s cursor `<line> : <col>` and `INS`/`OVR` mode, say).
//!
//! `StatusPanel` doesn't compute or query anything itself — it just draws
//! whatever `String` its host last handed it via [`set_text`](StatusPanel::set_text).
//! Positioning is entirely the host's decision: a [`Window`](super::Window)
//! can host one on its own bottom border, and [`Shell`](crate::app::Shell)
//! can host one on the desktop status row, both pulling the text from
//! [`View::status_text`] on whatever interior view they're composing (ADR
//! 0032). See `docs/specs/status_panel.md`.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::geometry::Point;
use crate::geometry::Rect;
use crate::view::View;

/// A one-row status display, fed pre-formatted text by whoever hosts it.
pub struct StatusPanel {
    bounds: Rect,
    text: Option<String>,
    style: Style,
}

impl StatusPanel {
    /// The default reserved width in columns — fits `"9999 : 999   OVR"`
    /// with a little slack.
    pub const DEFAULT_WIDTH: i16 = 18;

    /// Creates a panel at `bounds`, initially blank, drawn in `style`.
    pub fn new(bounds: Rect, style: Style) -> Self {
        Self {
            bounds,
            text: None,
            style,
        }
    }

    /// Repositions/resizes the panel (a host calls this as its own layout
    /// changes, e.g. a terminal resize).
    pub fn set_bounds(&mut self, bounds: Rect) {
        self.bounds = bounds;
    }

    /// Replaces the displayed text, or blanks the row if `None`.
    pub fn set_text(&mut self, text: Option<String>) {
        self.text = text;
    }
}

impl View for StatusPanel {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        canvas.fill(area, &Cell::blank(self.style));
        if let Some(text) = &self.text {
            canvas.put_str(Point::new(1, 0), text, self.style);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::geometry::Size;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    #[test]
    fn set_bounds_updates_bounds() {
        let mut panel = StatusPanel::new(rect(0, 0, 18, 1), Style::new());
        panel.set_bounds(rect(5, 2, 18, 1));
        assert_eq!(panel.bounds(), rect(5, 2, 18, 1));
    }

    #[test]
    fn snapshot_populated_panel() {
        let mut panel = StatusPanel::new(rect(0, 0, 18, 1), Style::new());
        panel.set_text(Some("12 : 5   INS".to_string()));
        let mut buf = Buffer::new(Size::new(18, 1));
        let mut canvas = Canvas::new(&mut buf);
        panel.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn snapshot_blank_panel_when_text_is_none() {
        let panel = StatusPanel::new(rect(0, 0, 18, 1), Style::new());
        let mut buf = Buffer::new(Size::new(18, 1));
        let mut canvas = Canvas::new(&mut buf);
        panel.draw(&mut canvas);
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn text_wider_than_bounds_is_truncated_not_wrapped() {
        let mut panel = StatusPanel::new(rect(0, 0, 6, 1), Style::new());
        panel.set_text(Some("123456789".to_string()));
        let mut buf = Buffer::new(Size::new(6, 1));
        let mut canvas = Canvas::new(&mut buf);
        panel.draw(&mut canvas); // no panic; clipped to the 6-column box
        insta::assert_snapshot!(buf.to_text());
    }

    #[test]
    fn set_text_replaces_previous_value() {
        let mut panel = StatusPanel::new(rect(0, 0, 18, 1), Style::new());
        panel.set_text(Some("old".to_string()));
        panel.set_text(Some("new".to_string()));
        let mut buf = Buffer::new(Size::new(18, 1));
        let mut canvas = Canvas::new(&mut buf);
        panel.draw(&mut canvas);
        assert!(buf.to_text().contains("new"));
        assert!(!buf.to_text().contains("old"));
    }
}
