//! Shared window-arrangement geometry (ADR 0033): chrome hit-testing,
//! move/resize sessions, cascade/tile layout, bounds clamping. Plain
//! functions over [`Rect`]/[`Point`]/[`Size`] — no knowledge of [`View`](crate::view::View),
//! `Window`, or any concrete document type, so it serves both `rvision`'s own
//! `widgets::Desktop`/`widgets::Window` and a future non-`rvision` caller
//! (`edit`'s bespoke MDI) without either shaping it to their own needs.

use crate::geometry::{Point, Rect, Size};
use crate::widgets::Frame;

/// Which of a window's optional chrome affordances are active — the flags
/// [`chrome_hit`] gates its classification on, grouped so a call site can't
/// transpose two same-typed bools.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ChromeFlags {
    /// Whether a title-bar press starts a move session.
    pub moveable: bool,
    /// Whether a bottom-right-corner press starts a resize session.
    pub resizable: bool,
    /// Whether the close glyph is live.
    pub closable: bool,
    /// Whether the zoom glyph is live.
    pub zoomable: bool,
    /// Whether a help glyph is drawn (shifts the close/zoom glyphs'
    /// narrow-frame visibility threshold, ADR 0021).
    pub has_help: bool,
}

/// Where a press at `pos` landed on a window occupying `bounds` — the
/// column/row span [`chrome_hit`] classified a press into.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChromeHit {
    /// The close glyph.
    Close,
    /// The zoom glyph.
    Zoom,
    /// The help glyph.
    Help,
    /// The title bar, clear of any glyph — starts a move session.
    Move,
    /// The bottom-right corner — starts a resize session.
    Resize,
    /// Interior, plain border, or a glyph region whose flag is off.
    None,
}

/// Classifies a press at `pos` against a window occupying `bounds` (same
/// coordinate space as `pos` — the caller's choice, this module has no
/// opinion). Fully gated on `flags`: a geometric hit on a disabled
/// affordance (e.g. the resize corner when `!flags.resizable`) is
/// [`ChromeHit::None`], never the geometric variant — callers never need a
/// second flag check. Close/zoom/help are tested first (each only when its
/// own flag is set), before falling through to the resize-corner/title-row
/// test, so a glyph hit is never also read as a move.
pub fn chrome_hit(bounds: Rect, pos: Point, flags: ChromeFlags) -> ChromeHit {
    let width = bounds.width();
    let height = bounds.height();
    let local_x = pos.x - bounds.origin().x;
    let local_y = pos.y - bounds.origin().y;

    if local_y == 0 {
        if flags.closable
            && Frame::close_span(width, flags.has_help).is_some_and(|s| s.contains(&local_x))
        {
            return ChromeHit::Close;
        }
        if flags.zoomable
            && Frame::zoom_span(width, flags.has_help).is_some_and(|s| s.contains(&local_x))
        {
            return ChromeHit::Zoom;
        }
        if flags.has_help && Frame::help_span(width).is_some_and(|s| s.contains(&local_x)) {
            return ChromeHit::Help;
        }
    }

    if local_x == width - 1 && local_y == height - 1 {
        return if flags.resizable {
            ChromeHit::Resize
        } else {
            ChromeHit::None
        };
    }
    if local_y == 0 && local_x >= 1 && local_x < width - 1 {
        return if flags.moveable {
            ChromeHit::Move
        } else {
            ChromeHit::None
        };
    }
    ChromeHit::None
}

/// What an in-progress [`ArrangeSession`] is doing to the window bounds it
/// grabbed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ArrangeKind {
    /// Translates the window's origin.
    Move,
    /// Resizes the window in place, from its bottom-right corner.
    Resize,
}

/// An in-progress title-bar move or corner resize: an opaque anchor point
/// plus the window's bounds when the session started.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ArrangeSession {
    kind: ArrangeKind,
    anchor: Point,
    start_bounds: Rect,
}

/// Starts a session of `kind` for a window occupying `bounds`, grabbed at
/// `anchor` (same coordinate space as `bounds`).
pub fn start_session(kind: ArrangeKind, bounds: Rect, anchor: Point) -> ArrangeSession {
    ArrangeSession {
        kind,
        anchor,
        start_bounds: bounds,
    }
}

/// The session's window bounds at pointer `pos`: [`ArrangeKind::Move`]
/// translates `start_bounds` by the delta between `anchor` and `pos`;
/// [`ArrangeKind::Resize`] grows/shrinks `start_bounds` by that same delta,
/// floored (each dimension independently) at `min_size` — supplied by the
/// caller rather than a hardcoded floor, since `Desktop` and `edit` already
/// disagree on it. No ceiling and no clamping to any outer bounds — a caller
/// that wants the result kept on-screen composes [`clamp_rect`] itself.
pub fn continue_session(session: &ArrangeSession, pos: Point, min_size: Size) -> Rect {
    let dx = pos.x - session.anchor.x;
    let dy = pos.y - session.anchor.y;
    match session.kind {
        ArrangeKind::Move => session.start_bounds.offset(dx, dy),
        ArrangeKind::Resize => {
            let width = (session.start_bounds.width() + dx).max(min_size.width);
            let height = (session.start_bounds.height() + dy).max(min_size.height);
            Rect::from_origin_size(session.start_bounds.origin(), Size::new(width, height))
        }
    }
}

/// `rect` clamped to fit within a `bounds`-sized area at the origin: the
/// size is capped and the origin pulled back so the rectangle stays fully
/// within `bounds`.
pub fn clamp_rect(rect: Rect, bounds: Size) -> Rect {
    let w = rect.width().clamp(0, bounds.width.max(0));
    let h = rect.height().clamp(0, bounds.height.max(0));
    let x = rect.origin().x.clamp(0, (bounds.width - w).max(0));
    let y = rect.origin().y.clamp(0, (bounds.height - h).max(0));
    Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
}

/// A cascade slot (desktop-local) for the `index`-th window on a
/// `desktop`-sized area: stepped down-right from the top-left, wrapping
/// every 8 so a long stack never marches off-screen. `min_size` caps how far
/// the step can push the slot's origin, and the result is always clamped to
/// fit `desktop`.
pub fn cascade_slot(desktop: Size, index: usize, min_size: Size) -> Rect {
    let step = (index % 8) as i16;
    let x = (step * 2).min((desktop.width - min_size.width).max(0));
    let y = step.min((desktop.height - min_size.height).max(0));
    clamp_rect(
        Rect::from_origin_size(
            Point::new(x, y),
            Size::new(desktop.width - x, desktop.height - y),
        ),
        desktop,
    )
}

/// An even grid of `count` rects filling `desktop`: a roughly square layout
/// (`cols` the smallest value with `cols * cols >= count`), with the last
/// row/column absorbing the integer-division remainder so the grid exactly
/// fills `desktop` with no gap or overhang.
pub fn tile(desktop: Size, count: usize) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let cols = (1..=count).find(|c| c * c >= count).unwrap_or(1);
    let rows = count.div_ceil(cols);
    (0..count)
        .map(|i| {
            let row = i / cols;
            let col = i % cols;
            let cols_in_row = if row + 1 == rows {
                count - cols * row
            } else {
                cols
            };
            let cell_w = desktop.width / cols_in_row as i16;
            let cell_h = desktop.height / rows as i16;
            let x = cell_w * col as i16;
            let y = cell_h * row as i16;
            let w = if col + 1 == cols_in_row {
                desktop.width - x
            } else {
                cell_w
            };
            let h = if row + 1 == rows {
                desktop.height - y
            } else {
                cell_h
            };
            Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    // A window wide/tall enough to show close+zoom (>=10) but not help (>=13).
    const NARROW_HELP: Rect = Rect::from_origin_size(Point::new(0, 0), Size::new(10, 5));
    // Wide enough for all three.
    const WIDE: Rect = Rect::from_origin_size(Point::new(0, 0), Size::new(20, 8));

    fn all_flags() -> ChromeFlags {
        ChromeFlags {
            moveable: true,
            resizable: true,
            closable: true,
            zoomable: true,
            has_help: true,
        }
    }

    fn no_help_flags() -> ChromeFlags {
        ChromeFlags {
            has_help: false,
            ..all_flags()
        }
    }

    // --- chrome_hit: glyphs ---

    #[test]
    fn close_glyph_hit_when_closable() {
        let span = Frame::close_span(WIDE.width(), true).unwrap();
        let pos = Point::new(span.start, 0);
        assert_eq!(chrome_hit(WIDE, pos, all_flags()), ChromeHit::Close);
    }

    #[test]
    fn close_glyph_column_is_ordinary_title_bar_when_not_closable() {
        // No glyph is drawn there when `!closable` (Frame's own gate mirrors
        // this), so the column is just an ordinary — moveable — part of the
        // title bar, not a dead zone.
        let span = Frame::close_span(WIDE.width(), true).unwrap();
        let pos = Point::new(span.start, 0);
        let flags = ChromeFlags {
            closable: false,
            ..all_flags()
        };
        assert_eq!(chrome_hit(WIDE, pos, flags), ChromeHit::Move);
    }

    #[test]
    fn zoom_glyph_hit_when_zoomable() {
        let span = Frame::zoom_span(WIDE.width(), true).unwrap();
        let pos = Point::new(span.start, 0);
        assert_eq!(chrome_hit(WIDE, pos, all_flags()), ChromeHit::Zoom);
    }

    #[test]
    fn zoom_glyph_column_is_ordinary_title_bar_when_not_zoomable() {
        let span = Frame::zoom_span(WIDE.width(), true).unwrap();
        let pos = Point::new(span.start, 0);
        let flags = ChromeFlags {
            zoomable: false,
            ..all_flags()
        };
        assert_eq!(chrome_hit(WIDE, pos, flags), ChromeHit::Move);
    }

    #[test]
    fn help_glyph_hit_when_has_help() {
        let span = Frame::help_span(WIDE.width()).unwrap();
        let pos = Point::new(span.start, 0);
        assert_eq!(chrome_hit(WIDE, pos, all_flags()), ChromeHit::Help);
    }

    #[test]
    fn help_glyph_miss_when_no_help_topic() {
        // Same column as where the help glyph would be, but has_help off —
        // no help glyph is drawn there at all, so it reads as a title-bar
        // Move instead (matching Frame's own drawing: no help glyph, so no
        // gap in the title bar for it).
        let span = Frame::help_span(WIDE.width()).unwrap();
        let pos = Point::new(span.start, 0);
        assert_eq!(chrome_hit(WIDE, pos, no_help_flags()), ChromeHit::Move);
    }

    #[test]
    fn narrow_frame_with_help_drops_all_three_glyphs() {
        // width 10 clears the no-help threshold (>=10) but not the
        // has-help one (>=13): ADR 0021's all-or-nothing gate means none of
        // the three glyphs are shown, so every glyph column reads as Move.
        let close = Frame::close_span(10, false).unwrap();
        let pos = Point::new(close.start, 0);
        assert_eq!(chrome_hit(NARROW_HELP, pos, all_flags()), ChromeHit::Move);
    }

    #[test]
    fn narrow_frame_without_help_still_shows_close_and_zoom() {
        let close = Frame::close_span(10, false).unwrap();
        let pos = Point::new(close.start, 0);
        assert_eq!(
            chrome_hit(NARROW_HELP, pos, no_help_flags()),
            ChromeHit::Close
        );
    }

    // --- chrome_hit: move / resize / none ---

    #[test]
    fn title_bar_hit_when_moveable() {
        let pos = Point::new(WIDE.origin().x + 5, WIDE.origin().y);
        assert_eq!(chrome_hit(WIDE, pos, all_flags()), ChromeHit::Move);
    }

    #[test]
    fn title_bar_miss_when_not_moveable() {
        let pos = Point::new(WIDE.origin().x + 5, WIDE.origin().y);
        let flags = ChromeFlags {
            moveable: false,
            ..all_flags()
        };
        assert_eq!(chrome_hit(WIDE, pos, flags), ChromeHit::None);
    }

    #[test]
    fn corner_hit_when_resizable() {
        let br = WIDE.bottom_right();
        let pos = Point::new(br.x - 1, br.y - 1);
        assert_eq!(chrome_hit(WIDE, pos, all_flags()), ChromeHit::Resize);
    }

    #[test]
    fn corner_miss_when_not_resizable() {
        let br = WIDE.bottom_right();
        let pos = Point::new(br.x - 1, br.y - 1);
        let flags = ChromeFlags {
            resizable: false,
            ..all_flags()
        };
        assert_eq!(chrome_hit(WIDE, pos, flags), ChromeHit::None);
    }

    #[test]
    fn interior_is_none() {
        let pos = Point::new(WIDE.origin().x + 3, WIDE.origin().y + 3);
        assert_eq!(chrome_hit(WIDE, pos, all_flags()), ChromeHit::None);
    }

    #[test]
    fn outside_bounds_is_none() {
        let pos = Point::new(WIDE.origin().x - 1, WIDE.origin().y);
        assert_eq!(chrome_hit(WIDE, pos, all_flags()), ChromeHit::None);
    }

    #[test]
    fn same_hit_test_translates_with_a_non_zero_origin() {
        // chrome_hit must work in whatever coordinate space bounds/pos share
        // — not assume a zero origin.
        let shifted = rect(100, 50, 20, 8);
        let close = Frame::close_span(20, true).unwrap();
        let pos = Point::new(100 + close.start, 50);
        assert_eq!(chrome_hit(shifted, pos, all_flags()), ChromeHit::Close);
    }

    // --- continue_session ---

    #[test]
    fn move_translates_by_the_delta_since_anchor() {
        let start = rect(5, 5, 10, 4);
        let session = start_session(ArrangeKind::Move, start, Point::new(5, 5));
        let moved = continue_session(&session, Point::new(8, 3), Size::new(1, 1));
        assert_eq!(moved, rect(8, 3, 10, 4));
    }

    #[test]
    fn resize_grows_and_shrinks_from_start_bounds() {
        let start = rect(5, 5, 10, 4);
        let session = start_session(ArrangeKind::Resize, start, Point::new(14, 8));
        // Corner moves +3 columns, -1 row from the anchor.
        let resized = continue_session(&session, Point::new(17, 7), Size::new(1, 1));
        assert_eq!(resized, rect(5, 5, 13, 3));
    }

    #[test]
    fn resize_floors_each_axis_independently_at_min_size() {
        let start = rect(5, 5, 10, 4);
        let session = start_session(ArrangeKind::Resize, start, Point::new(14, 8));
        // Shrink width a lot, height a little — each axis floors on its own.
        let resized = continue_session(&session, Point::new(0, 7), Size::new(6, 3));
        assert_eq!(resized, rect(5, 5, 6, 3));
    }

    #[test]
    fn grab_point_invariant_matches_offset_from_corner_when_grabbed_exactly_on_the_cell() {
        // Two ways to compute the same resize: `Desktop`'s delta-from-anchor
        // (continue_session) vs. `edit`'s offset-from-corner formulation.
        // They agree only because the grab lands exactly on the
        // bottom-right cell (chrome_hit's own contract) — this test is that
        // invariant made explicit, per ADR 0033 point 4.
        let start = rect(2, 2, 12, 6);
        let corner = start.bottom_right().offset(-1, -1);
        let grab = corner; // chrome_hit only ever starts a Resize here.
        let pos = Point::new(20, 10);

        let session = start_session(ArrangeKind::Resize, start, grab);
        let via_anchor_delta = continue_session(&session, pos, Size::new(1, 1));

        // edit::app::drag_to's Resize arm, inlined: w/h from the corner's
        // *offset* (dx = grab.x - corner.x, here always 0) rather than a
        // delta since anchor.
        let dx = grab.x - corner.x;
        let dy = grab.y - corner.y;
        let w = (pos.x - dx - start.origin().x + 1).max(1);
        let h = (pos.y - dy - start.origin().y + 1).max(1);
        let via_offset_from_corner = Rect::from_origin_size(start.origin(), Size::new(w, h));

        assert_eq!(via_anchor_delta, via_offset_from_corner);
    }

    // --- clamp_rect ---

    #[test]
    fn clamp_rect_leaves_an_in_bounds_rect_unchanged() {
        let bounds = Size::new(80, 24);
        let r = rect(2, 2, 10, 5);
        assert_eq!(clamp_rect(r, bounds), r);
    }

    #[test]
    fn clamp_rect_caps_oversized_dimensions_independently() {
        let bounds = Size::new(80, 24);
        let r = rect(0, 0, 100, 30);
        assert_eq!(clamp_rect(r, bounds), rect(0, 0, 80, 24));
    }

    #[test]
    fn clamp_rect_pulls_a_far_or_negative_origin_back_into_bounds() {
        let bounds = Size::new(80, 24);
        assert_eq!(clamp_rect(rect(90, 30, 10, 5), bounds), rect(70, 19, 10, 5));
        assert_eq!(clamp_rect(rect(-5, -5, 10, 5), bounds), rect(0, 0, 10, 5));
    }

    #[test]
    fn clamp_rect_against_empty_bounds_collapses_without_panicking() {
        let r = rect(2, 2, 10, 5);
        assert_eq!(clamp_rect(r, Size::new(0, 0)), rect(0, 0, 0, 0));
    }

    // --- cascade_slot ---

    #[test]
    fn cascade_slot_steps_down_right_by_two_and_one() {
        let desktop = Size::new(80, 24);
        let min = Size::new(10, 3);
        assert_eq!(cascade_slot(desktop, 0, min), rect(0, 0, 80, 24));
        assert_eq!(cascade_slot(desktop, 1, min), rect(2, 1, 78, 23));
        assert_eq!(cascade_slot(desktop, 2, min), rect(4, 2, 76, 22));
    }

    #[test]
    fn cascade_slot_wraps_every_eight() {
        let desktop = Size::new(80, 24);
        let min = Size::new(10, 3);
        assert_eq!(cascade_slot(desktop, 0, min), cascade_slot(desktop, 8, min));
        assert_eq!(
            cascade_slot(desktop, 3, min),
            cascade_slot(desktop, 11, min)
        );
    }

    #[test]
    fn cascade_slot_step_never_pushes_origin_past_desktop_minus_min_size() {
        // A tiny desktop forces the step to cap well before wrapping.
        let desktop = Size::new(15, 5);
        let min = Size::new(10, 3);
        let slot = cascade_slot(desktop, 7, min);
        assert!(slot.origin().x <= desktop.width - min.width);
        assert!(slot.origin().y <= desktop.height - min.height);
    }

    // --- tile ---

    fn union_all(rects: &[Rect]) -> Rect {
        rects
            .iter()
            .copied()
            .reduce(|a, b| a.union(b))
            .unwrap_or_default()
    }

    #[test]
    fn tile_one_window_fills_the_desktop() {
        let desktop = Size::new(80, 24);
        let rects = tile(desktop, 1);
        assert_eq!(rects, vec![rect(0, 0, 80, 24)]);
    }

    #[test]
    fn tile_two_windows_splits_side_by_side() {
        let desktop = Size::new(80, 24);
        let rects = tile(desktop, 2);
        assert_eq!(rects.len(), 2);
        assert_eq!(union_all(&rects), rect(0, 0, 80, 24));
        assert_eq!(rects[0], rect(0, 0, 40, 24));
        assert_eq!(rects[1], rect(40, 0, 40, 24));
    }

    #[test]
    fn tile_three_windows_uneven_row_absorbs_remainder() {
        let desktop = Size::new(80, 24);
        let rects = tile(desktop, 3);
        assert_eq!(rects.len(), 3);
        assert_eq!(union_all(&rects), rect(0, 0, 80, 24));
        // 3 -> cols=2, rows=2: row 0 has 2 slots, row 1 has 1 (stretched).
        assert_eq!(rects[2], rect(0, 12, 80, 12));
    }

    #[test]
    fn tile_four_windows_perfect_square() {
        let desktop = Size::new(80, 24);
        let rects = tile(desktop, 4);
        assert_eq!(rects.len(), 4);
        assert_eq!(union_all(&rects), rect(0, 0, 80, 24));
        assert_eq!(rects[0], rect(0, 0, 40, 12));
        assert_eq!(rects[3], rect(40, 12, 40, 12));
    }
}
