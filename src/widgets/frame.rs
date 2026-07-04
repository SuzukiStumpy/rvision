//! A window border: a single- or double-line box with a title and the
//! close/zoom glyphs on its top edge.
//!
//! `Frame` is a drawing helper, not an independent [`View`](crate::view::View):
//! it always paints the *whole* canvas it is handed — a window's outer rectangle —
//! so it has no bounds of its own. [`Window`](super::Window) owns one and draws it
//! before its interior.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::geometry::{Point, Rect};
use std::ops::Range;

/// The six glyphs of a box border: the four corners and the two edges.
struct BorderGlyphs {
    top_left: char,
    top_right: char,
    bottom_left: char,
    bottom_right: char,
    horizontal: char,
    vertical: char,
}

/// Single-line border (inactive windows).
const SINGLE: BorderGlyphs = BorderGlyphs {
    top_left: '┌',
    top_right: '┐',
    bottom_left: '└',
    bottom_right: '┘',
    horizontal: '─',
    vertical: '│',
};

/// Double-line border (the active window stands out, as in TurboVision).
const DOUBLE: BorderGlyphs = BorderGlyphs {
    top_left: '╔',
    top_right: '╗',
    bottom_left: '╚',
    bottom_right: '╝',
    horizontal: '═',
    vertical: '║',
};

/// The close glyph drawn near the top-left corner.
const CLOSE: &str = "[■]";
/// The zoom glyph drawn near the top-right corner when the window is at its
/// normal size: a single up-arrow inviting the user to maximise it.
const ZOOM: &str = "[↑]";
/// The zoom glyph when the window is maximised: a double-headed arrow inviting a
/// restore back to its normal size. Same width as [`ZOOM`], so the hit-test span
/// is unchanged.
const ZOOM_MAXIMIZED: &str = "[↕]";

/// The help glyph, drawn immediately left of the zoom glyph when the window
/// has a help topic (ADR 0021) — same width as the others, so it follows the
/// same span/layout math.
const HELP: &str = "[?]";

/// The resize-handle glyph drawn in the bottom-right corner in place of the
/// plain border corner when the frame is resizable — a purely visual
/// affordance (`Desktop` already treats that corner as the resize grab point
/// regardless of what's drawn there, ADR 0016).
const RESIZE_HANDLE: char = '◢';

/// A window frame: border, centred title, and close/zoom glyphs.
pub struct Frame {
    title: String,
    active: bool,
    maximized: bool,
    closable: bool,
    zoomable: bool,
    resizable: bool,
    help: bool,
    style: Style,
    title_style: Style,
}

impl Frame {
    /// Creates an (inactive) frame titled `title`, with `style` for the border and
    /// glyphs and `title_style` for the title text. Draws both the close and
    /// zoom glyphs by default (ADR 0016) — `closable`/`zoomable` turn either off.
    pub fn new(title: &str, style: Style, title_style: Style) -> Self {
        Self {
            title: title.to_string(),
            active: false,
            maximized: false,
            closable: true,
            zoomable: true,
            resizable: true,
            help: false,
            style,
            title_style,
        }
    }

    /// Marks the frame active (a doubled border) or not (a single one).
    pub fn active(mut self, active: bool) -> Self {
        self.active = active;
        self
    }

    /// Marks the window maximised, so the zoom glyph shows a restore (↕) arrow
    /// instead of the maximise (↑) one.
    pub fn maximized(mut self, maximized: bool) -> Self {
        self.maximized = maximized;
        self
    }

    /// Sets whether the close glyph is drawn at all (ADR 0016) — a window with
    /// `closable(false)` has nothing there to hit, so it's simply not drawn.
    pub fn closable(mut self, closable: bool) -> Self {
        self.closable = closable;
        self
    }

    /// Sets whether the zoom glyph is drawn at all, mirroring [`closable`](Self::closable).
    pub fn zoomable(mut self, zoomable: bool) -> Self {
        self.zoomable = zoomable;
        self
    }

    /// Sets whether the help glyph is drawn at all (ADR 0021) — `false` by
    /// default, unlike `closable`/`zoomable`: a window only gets one when it
    /// actually has a help topic ([`Window::with_help_topic`](super::Window::with_help_topic)).
    pub fn help(mut self, help: bool) -> Self {
        self.help = help;
        self
    }

    /// Sets whether the bottom-right corner shows a resize-handle glyph in
    /// place of the plain border corner (default `true`) — purely visual;
    /// the corner is always the resize grab point regardless of what's drawn
    /// there (`Desktop` decides that from the window's own `resizable` flag,
    /// ADR 0016).
    pub fn resizable(mut self, resizable: bool) -> Self {
        self.resizable = resizable;
        self
    }

    /// Sets the closable flag in place, mirroring [`set_active`](Self::set_active).
    pub fn set_closable(&mut self, closable: bool) {
        self.closable = closable;
    }

    /// Sets the zoomable flag in place, mirroring [`set_active`](Self::set_active).
    pub fn set_zoomable(&mut self, zoomable: bool) {
        self.zoomable = zoomable;
    }

    /// Sets the help flag in place, mirroring [`set_active`](Self::set_active)
    /// — [`Window::with_help_topic`](super::Window::with_help_topic) calls
    /// this (ADR 0021).
    pub fn set_help(&mut self, help: bool) {
        self.help = help;
    }

    /// Sets the resizable flag in place, mirroring [`set_active`](Self::set_active).
    pub fn set_resizable(&mut self, resizable: bool) {
        self.resizable = resizable;
    }

    /// Sets the active flag in place — the desktop calls this through its
    /// [`Window`](super::Window) as the focused window changes.
    pub fn set_active(&mut self, active: bool) {
        self.active = active;
    }

    /// Sets the maximised flag in place, mirroring [`set_active`](Self::set_active)
    /// — a [`Window`](super::Window) calls this as it toggles zoom (ADR 0016).
    pub fn set_maximized(&mut self, maximized: bool) {
        self.maximized = maximized;
    }

    /// Sets the title in place, mirroring [`set_active`](Self::set_active) —
    /// purely cosmetic, `draw_title` already truncates to fit dynamically, so
    /// there is nothing else derived from the title to recompute.
    pub fn set_title(&mut self, title: &str) {
        self.title = title.to_string();
    }

    /// Whether a `width`-wide frame is wide enough to draw its glyphs — three
    /// (close/zoom/help) when `help` is set, two otherwise (ADR 0021's
    /// all-or-nothing gate: a narrow frame drops every glyph it would need
    /// together, never just one). `help` reflects a specific frame's own
    /// flag, not a global constant, so a plain window's threshold is
    /// unchanged from before ADR 0021.
    fn glyphs_shown(width: i16, help: bool) -> bool {
        if help { width >= 13 } else { width >= 10 }
    }

    /// The column span the close glyph occupies on a `width`-wide frame's top edge,
    /// or `None` when the frame is too narrow to show it (`help` widens the
    /// narrow-frame threshold to also fit the help glyph, ADR 0021). Lets a
    /// window turn a click into the close action without re-deriving the
    /// glyph layout (ADR 0007).
    pub fn close_span(width: i16, help: bool) -> Option<Range<i16>> {
        Self::glyphs_shown(width, help).then(|| 2..2 + CLOSE.chars().count() as i16)
    }

    /// The column span the zoom glyph occupies, mirroring [`close_span`](Self::close_span).
    pub fn zoom_span(width: i16, help: bool) -> Option<Range<i16>> {
        Self::glyphs_shown(width, help).then(|| {
            let len = ZOOM.chars().count() as i16;
            (width - 1 - len)..(width - 1)
        })
    }

    /// The column span the help glyph occupies, immediately left of
    /// [`zoom_span`](Self::zoom_span) with no gap between them (ADR 0021).
    /// Only meaningful when the frame actually has a help topic — a caller
    /// gates on that itself, the same way `close_span`/`zoom_span` are only
    /// consulted when `closable`/`zoomable`.
    pub fn help_span(width: i16) -> Option<Range<i16>> {
        Self::glyphs_shown(width, true).then(|| {
            let zoom_len = ZOOM.chars().count() as i16;
            let help_len = HELP.chars().count() as i16;
            let zoom_start = width - 1 - zoom_len;
            (zoom_start - help_len)..zoom_start
        })
    }

    /// Draws the frame over the whole canvas it is handed. Degrades without panic
    /// for areas too small to hold a box (anything narrower or shorter than 2).
    pub fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        let (w, h) = (area.width(), area.height());
        if w < 2 || h < 2 {
            return;
        }
        let g = if self.active { &DOUBLE } else { &SINGLE };
        self.draw_border(canvas, area, g);

        // The close/zoom/help glyphs sit just inside the corners; only drawn
        // when the frame is wide enough *and* the window is closable/zoomable/
        // has a help topic (ADR 0016, ADR 0021) — a narrow or unconfigured
        // frame keeps a clean border instead of a clipped or dead-looking
        // glyph. The title is centred in the span between whichever glyphs are
        // actually drawn (or the whole top edge when none are) and truncated
        // to fit, so it never overdraws a glyph.
        let top = 0;
        let mut left = 1;
        let mut right = w - 1;
        if self.closable {
            if let Some(close) = Self::close_span(w, self.help) {
                canvas.put_str(Point::new(close.start, top), CLOSE, self.style);
                left = close.end;
            }
        }
        if self.zoomable {
            if let Some(zoom) = Self::zoom_span(w, self.help) {
                let zoom_glyph = if self.maximized { ZOOM_MAXIMIZED } else { ZOOM };
                canvas.put_str(Point::new(zoom.start, top), zoom_glyph, self.style);
                right = zoom.start;
            }
        }
        if self.help {
            if let Some(help) = Self::help_span(w) {
                canvas.put_str(Point::new(help.start, top), HELP, self.style);
                right = help.start;
            }
        }
        self.draw_title(canvas, top, left, right);
    }

    /// Draws the title centred in the half-open column span `[left, right)`,
    /// truncated to fit. Does nothing if the span or the title is empty.
    fn draw_title(&self, canvas: &mut Canvas, row: i16, left: i16, right: i16) {
        if self.title.is_empty() || right <= left {
            return;
        }
        let span = (right - left) as usize;
        let label = format!(" {} ", self.title);
        let shown: String = label.chars().take(span).collect();
        let len = shown.chars().count() as i16;
        let x = left + (right - left - len) / 2;
        canvas.put_str(Point::new(x, row), &shown, self.title_style);
    }

    /// Strokes the four edges and overwrites the corners.
    fn draw_border(&self, canvas: &mut Canvas, area: Rect, g: &BorderGlyphs) {
        let br = area.bottom_right();
        let (left, top) = (area.origin().x, area.origin().y);
        let (right, bottom) = (br.x - 1, br.y - 1);

        let h = Cell::from_char(g.horizontal, self.style);
        let v = Cell::from_char(g.vertical, self.style);
        for x in left..=right {
            canvas.set(Point::new(x, top), h.clone());
            canvas.set(Point::new(x, bottom), h.clone());
        }
        for y in top..=bottom {
            canvas.set(Point::new(left, y), v.clone());
            canvas.set(Point::new(right, y), v.clone());
        }
        canvas.set(
            Point::new(left, top),
            Cell::from_char(g.top_left, self.style),
        );
        canvas.set(
            Point::new(right, top),
            Cell::from_char(g.top_right, self.style),
        );
        canvas.set(
            Point::new(left, bottom),
            Cell::from_char(g.bottom_left, self.style),
        );
        let bottom_right = if self.resizable {
            RESIZE_HANDLE
        } else {
            g.bottom_right
        };
        canvas.set(
            Point::new(right, bottom),
            Cell::from_char(bottom_right, self.style),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::geometry::Size;

    fn render(frame: &Frame, w: i16, h: i16) -> String {
        let mut buf = Buffer::new(Size::new(w, h));
        let mut canvas = Canvas::new(&mut buf);
        frame.draw(&mut canvas);
        buf.to_text()
    }

    // Tracer bullet: an inactive frame is a single-line box with a centred title
    // and the close/zoom glyphs on the top edge.
    #[test]
    fn snapshot_inactive_frame() {
        let frame = Frame::new("Untitled", Style::new(), Style::new());
        insta::assert_snapshot!(render(&frame, 20, 5));
    }

    #[test]
    fn snapshot_active_frame_has_doubled_border() {
        let frame = Frame::new("Untitled", Style::new(), Style::new()).active(true);
        insta::assert_snapshot!(render(&frame, 20, 5));
    }

    #[test]
    fn narrow_frame_drops_glyphs_but_keeps_a_clean_box() {
        // Too narrow (8 < 10) for close/zoom; still a tidy single-line box.
        let frame = Frame::new("X", Style::new(), Style::new());
        insta::assert_snapshot!(render(&frame, 8, 3));
    }

    #[test]
    fn the_zoom_glyph_reflects_the_maximised_state() {
        let normal = Frame::new("Doc", Style::new(), Style::new());
        assert!(
            render(&normal, 20, 3).contains('↑'),
            "normal shows the ↑ glyph"
        );
        assert!(!render(&normal, 20, 3).contains('↕'));

        let maxed = Frame::new("Doc", Style::new(), Style::new()).maximized(true);
        assert!(
            render(&maxed, 20, 3).contains('↕'),
            "maximised shows the ↕ glyph"
        );
        assert!(!render(&maxed, 20, 3).contains('↑'));
    }

    #[test]
    fn set_maximized_toggles_the_zoom_glyph_in_place() {
        let mut frame = Frame::new("Doc", Style::new(), Style::new());
        assert!(render(&frame, 20, 3).contains('↑'));
        frame.set_maximized(true);
        assert!(render(&frame, 20, 3).contains('↕'));
        assert!(!render(&frame, 20, 3).contains('↑'));
        frame.set_maximized(false);
        assert!(render(&frame, 20, 3).contains('↑'));
    }

    #[test]
    fn set_title_changes_the_drawn_title_in_place() {
        let mut frame = Frame::new("Old", Style::new(), Style::new());
        assert!(render(&frame, 20, 3).contains("Old"));
        frame.set_title("New");
        let text = render(&frame, 20, 3);
        assert!(text.contains("New"));
        assert!(!text.contains("Old"));
    }

    #[test]
    fn resizable_frame_shows_a_handle_in_the_bottom_right_corner() {
        let frame = Frame::new("Doc", Style::new(), Style::new());
        let rows: Vec<String> = render(&frame, 20, 5).lines().map(str::to_string).collect();
        assert_eq!(rows[4].chars().last(), Some('◢'));
    }

    #[test]
    fn a_non_resizable_frame_keeps_a_plain_corner() {
        let frame = Frame::new("Doc", Style::new(), Style::new()).resizable(false);
        let rows: Vec<String> = render(&frame, 20, 5).lines().map(str::to_string).collect();
        assert_eq!(
            rows[4].chars().last(),
            Some('┘'),
            "plain single-line corner"
        );
    }

    #[test]
    fn set_resizable_toggles_the_corner_handle_in_place() {
        let mut frame = Frame::new("Doc", Style::new(), Style::new());
        assert!(render(&frame, 20, 5).contains('◢'));
        frame.set_resizable(false);
        assert!(!render(&frame, 20, 5).contains('◢'));
        frame.set_resizable(true);
        assert!(render(&frame, 20, 5).contains('◢'));
    }

    #[test]
    fn tiny_area_does_not_panic() {
        let frame = Frame::new("nope", Style::new(), Style::new());
        // 1-wide / 1-tall: below the box minimum; draws nothing, no panic.
        assert_eq!(render(&frame, 1, 4), " \n \n \n ");
        assert_eq!(render(&frame, 4, 1), "    ");
    }

    // --- Help glyph (ADR 0021) ---

    #[test]
    fn no_help_glyph_by_default() {
        let frame = Frame::new("Doc", Style::new(), Style::new());
        assert!(!render(&frame, 20, 3).contains('?'));
    }

    #[test]
    fn help_glyph_shown_when_enabled_and_sits_left_of_zoom() {
        let frame = Frame::new("Doc", Style::new(), Style::new()).help(true);
        let text = render(&frame, 20, 3);
        assert!(text.contains('?'), "help glyph drawn");
        let row = text.lines().next().unwrap();
        let help_col = row.find('?').unwrap();
        let zoom_col = row.find('↑').unwrap();
        assert!(help_col < zoom_col, "help glyph sits left of zoom");
    }

    #[test]
    fn help_span_is_immediately_left_of_zoom_span_with_no_gap() {
        let zoom = Frame::zoom_span(20, true).unwrap();
        let help = Frame::help_span(20).unwrap();
        assert_eq!(help.end, zoom.start);
    }

    #[test]
    fn a_help_enabled_frame_needs_a_wider_frame_before_showing_any_glyph() {
        // 12 is plenty for the old two-glyph threshold (10) but not enough to
        // also fit the help glyph (ADR 0021's all-or-nothing gate): every
        // glyph drops together, not just help.
        let frame = Frame::new("X", Style::new(), Style::new()).help(true);
        let narrow = render(&frame, 12, 3);
        assert!(!narrow.contains('['), "no glyph at all when too narrow");

        let wide = render(&frame, 13, 3);
        assert!(wide.contains('['), "all glyphs return once wide enough");
    }

    #[test]
    fn a_plain_frames_glyph_threshold_is_unaffected_by_adr_0021() {
        // A frame that never turns help on keeps exactly the old threshold —
        // ADR 0021 only widens the gate for a frame that actually uses it.
        let frame = Frame::new("X", Style::new(), Style::new());
        assert!(render(&frame, 10, 3).contains('['));
    }

    #[test]
    fn close_and_zoom_spans_widen_their_threshold_only_when_help_is_set() {
        assert!(Frame::close_span(10, false).is_some());
        assert!(Frame::close_span(12, true).is_none());
        assert!(Frame::close_span(13, true).is_some());
        assert!(Frame::zoom_span(12, true).is_none());
        assert!(Frame::zoom_span(13, true).is_some());
    }
}
