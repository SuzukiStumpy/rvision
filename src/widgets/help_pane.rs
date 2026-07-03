//! The help page pane: a read-only renderer for one
//! [`HelpTopic`](crate::help::HelpTopic) that scrolls both ways (ADR 0013,
//! `docs/specs/help_window.md`).
//!
//! It reflows [`Paragraph`](crate::help::Block::Paragraph) prose to its current
//! width and emits [`Preformatted`](crate::help::Block::Preformatted) lines
//! verbatim, so prose adapts to the pane while keybinding tables and other
//! fixed-format blocks stay aligned. Because a `<pre>` block can be wider than
//! the pane, the pane scrolls **horizontally** as well as vertically: a
//! [`ScrollBar`](super::ScrollBar) appears down the right edge when the page is
//! too tall and along the bottom when a line is too wide, each only when needed
//! (the two interact — one steals a row/column, which can call for the other, so
//! they are decided together). Arrow keys, the wheel, and the bars' arrows/track
//! all scroll. It is the reusable part shared by the framework's (future) help
//! window and the editor's modal help viewer — neither owns the rendering.
//!
//! A paragraph's [`Span::Link`](crate::help::Span::Link)s are followable
//! (ADR 0020): `Ctrl+Down`/`Ctrl+Up` cycle a "current link" highlight through
//! the page, wrapping and scrolling it into view; `Enter` follows the current
//! link; a direct click follows a link immediately. Following a link doesn't
//! change *what* is followed by this module — it queues the target topic id,
//! drained via [`take_link_activation`](HelpPane::take_link_activation) by
//! whoever owns both this pane and the topic list
//! ([`HelpWindow`](super::HelpWindow)), mirroring how that owner already polls
//! the list's own selection.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::event::{Event, EventResult, KeyCode, Modifiers, MouseButton, MouseEvent, MouseKind};
use crate::geometry::{Point, Rect, Size};
use crate::help::{Block, HelpTopic, Span};
use crate::theme::{Role, Theme};
use crate::view::{Context, View};

use super::{ScrollBar, ScrollPart};
use unicode_width::UnicodeWidthStr;

/// Lines panned per wheel notch — matches the editor's feel.
const WHEEL_STEP: isize = 3;

/// A read-only view of one help topic's body that scrolls in both axes.
pub struct HelpPane {
    bounds: Rect,
    /// The current topic's blocks, kept so the pane can re-lay-out on resize.
    body: Vec<Block>,
    /// The laid-out display lines at the current text width.
    lines: Vec<String>,
    /// Every followable link at the current text width, in document order
    /// (ADR 0020) — rebuilt alongside `lines` in `layout`.
    links: Vec<PaneLink>,
    /// Index into `links` of the keyboard "current link" highlight, or `None`
    /// if the topic has no links. `Some(0)` by default whenever `show` finds
    /// any.
    current_link: Option<usize>,
    /// The target topic id a link activation (Enter or a direct click) is
    /// waiting to be drained via `take_link_activation`.
    pending_activation: Option<String>,
    /// Index of the topmost visible line.
    top: usize,
    /// Leftmost visible display column (horizontal scroll offset).
    left: usize,
    /// Whether each scroll bar is currently shown (decided together in `layout`).
    needs_vbar: bool,
    needs_hbar: bool,
    focused: bool,
    style: Style,
    /// A link at rest, whether or not this pane holds focus (ADR 0020).
    link_style: Style,
    /// The *current* link, only while this pane is focused — mirrors
    /// `ListBox`'s own focused-row highlight, one level more granular.
    link_focus_style: Style,
}

impl HelpPane {
    /// Creates an empty pane at `bounds`.
    pub fn new(bounds: Rect, theme: &Theme) -> Self {
        Self {
            bounds,
            body: Vec::new(),
            lines: Vec::new(),
            links: Vec::new(),
            current_link: None,
            pending_activation: None,
            top: 0,
            left: 0,
            needs_vbar: false,
            needs_hbar: false,
            focused: false,
            style: theme.style(Role::DialogBackground),
            link_style: theme.style(Role::HelpLink),
            link_focus_style: theme.style(Role::Selection),
        }
    }

    /// Shows `topic`: lays its body out for the current size, scrolls to the
    /// top-left, and — if the topic has any links — highlights the first one
    /// as the current link.
    pub fn show(&mut self, topic: &HelpTopic) {
        self.body = topic.body.clone();
        self.top = 0;
        self.left = 0;
        self.current_link = None;
        self.pending_activation = None;
        self.layout();
        self.current_link = (!self.links.is_empty()).then_some(0);
    }

    /// Repositions/resizes the pane, re-laying-out if the size changed.
    pub fn set_bounds(&mut self, bounds: Rect) {
        let resized = bounds.size() != self.bounds.size();
        self.bounds = bounds;
        if resized {
            self.layout();
        } else {
            self.clamp_top();
            self.clamp_left();
        }
    }

    /// Drains the target topic id a link activation (Enter on the current
    /// link, or a direct click) is waiting to jump to, if any. Polled by
    /// [`HelpWindow`](super::HelpWindow) after routing an event into this
    /// pane, the same way it already polls its list's own selection.
    pub fn take_link_activation(&mut self) -> Option<String> {
        self.pending_activation.take()
    }

    /// The total number of laid-out lines (for sizing / overflow checks).
    pub fn content_height(&self) -> i16 {
        self.lines.len() as i16
    }

    /// The widest laid-out line in display columns (for sizing).
    pub fn content_width(&self) -> i16 {
        max_line_width(&self.lines)
    }

    /// The pane's full height in rows (before any horizontal bar is subtracted).
    fn rows_total(&self) -> usize {
        self.bounds.height().max(0) as usize
    }

    /// Visible text rows — the height minus the horizontal bar's row, if shown.
    fn text_rows(&self) -> usize {
        self.rows_total()
            .saturating_sub(usize::from(self.needs_hbar))
    }

    /// Visible text columns — the width minus the vertical bar's column, if shown.
    fn text_w(&self) -> i16 {
        (self.bounds.width() - i16::from(self.needs_vbar)).max(0)
    }

    /// Decides which scroll bars are needed and lays the body out to match. The
    /// two bars interact — a vertical bar narrows the text (which can make a line
    /// overflow → horizontal bar) and a horizontal bar shortens it (which can push
    /// the line count over → vertical bar) — so they are found together by a short
    /// fixed-point iteration (it converges in a step or two; the cap just bounds
    /// it). Only the *width* changes the wrapped line content, so the body is
    /// re-rendered once at the settled text width.
    fn layout(&mut self) {
        let width = self.bounds.width().max(0);
        let total_rows = self.rows_total();
        let (mut vbar, mut hbar) = (false, false);
        for _ in 0..4 {
            let text_w = (width - i16::from(vbar)).max(0);
            let text_rows = total_rows.saturating_sub(usize::from(hbar));
            let lines = render_blocks(&self.body, text_w as u16).lines;
            let new_vbar = lines.len() > text_rows && width > 1;
            let new_hbar = max_line_width(&lines) > text_w && total_rows > 1;
            if (new_vbar, new_hbar) == (vbar, hbar) {
                break;
            }
            vbar = new_vbar;
            hbar = new_hbar;
        }
        self.needs_vbar = vbar;
        self.needs_hbar = hbar;
        let text_w = (width - i16::from(vbar)).max(0);
        let laid = render_blocks(&self.body, text_w as u16);
        self.lines = laid.lines;
        self.links = laid.links;
        self.clamp_top();
        self.clamp_left();
        self.clamp_current_link();
    }

    /// Keeps `current_link` a valid index into the just-rebuilt `links`,
    /// mirroring `clamp_top`/`clamp_left`'s best-effort (not
    /// identity-preserving) contract across a relayout — a link's total
    /// count is a property of `body` alone (unaffected by resize), so this
    /// only ever narrows an already-`Some` index, never invents one out of
    /// `None` (ADR 0020).
    fn clamp_current_link(&mut self) {
        if let Some(i) = self.current_link {
            self.current_link = if self.links.is_empty() {
                None
            } else {
                Some(i.min(self.links.len() - 1))
            };
        }
    }

    /// Moves the current-link highlight by `delta` links, **wrapping**
    /// (unlike scrolling's clamp — matches `HelpWindow`'s own wrapping
    /// Tab/BackTab convention instead), then scrolls it into view. A no-op
    /// if the topic has no links.
    fn cycle_link(&mut self, delta: isize) {
        if self.links.is_empty() {
            return;
        }
        let len = self.links.len() as isize;
        let current = self.current_link.unwrap_or(0) as isize;
        self.current_link = Some((current + delta).rem_euclid(len) as usize);
        self.reveal_current_link();
    }

    /// Scrolls so the current link's line is visible, and resets `left` to
    /// 0 — a paragraph line always fits `text_w` at `left == 0` (only a
    /// `<pre>` block scrolls horizontally, and `<pre>` never contains
    /// links), so a nonzero `left` left over from scrolling one could
    /// otherwise hide the link just selected.
    fn reveal_current_link(&mut self) {
        let Some(link) = self.current_link.and_then(|i| self.links.get(i)) else {
            return;
        };
        let line = link.line;
        let text_rows = self.text_rows();
        if line < self.top {
            self.top = line;
        } else if line >= self.top + text_rows {
            self.top = line + 1 - text_rows.max(1);
        }
        self.left = 0;
    }

    /// Queues the current link's target for `take_link_activation`, if there
    /// is one.
    fn follow_current_link(&mut self) {
        if let Some(link) = self.current_link.and_then(|i| self.links.get(i)) {
            self.pending_activation = Some(link.target.clone());
        }
    }

    /// The index into `links` whose rendered run contains `pos` (pane-local
    /// coordinates, already accounting for scroll), if any.
    fn link_at(&self, pos: Point) -> Option<usize> {
        if pos.y < 0 || pos.x < 0 {
            return None;
        }
        let line = self.top + pos.y as usize;
        let col = self.left + pos.x as usize;
        self.links.iter().position(|link| {
            link.line == line
                && self.lines.get(link.line).is_some_and(|text| {
                    let start = text[..link.start_byte].width();
                    let end = text[..link.end_byte].width();
                    (start..end).contains(&col)
                })
        })
    }

    /// The largest valid `top`, keeping the last screenful in view.
    fn max_top(&self) -> usize {
        self.lines.len().saturating_sub(self.text_rows())
    }

    /// The largest valid `left`, keeping the rightmost column reachable.
    fn max_left(&self) -> usize {
        (self.content_width().max(0) as usize).saturating_sub(self.text_w().max(0) as usize)
    }

    fn clamp_top(&mut self) {
        self.top = self.top.min(self.max_top());
    }

    fn clamp_left(&mut self) {
        self.left = self.left.min(self.max_left());
    }

    /// Scrolls by `delta` lines (negative = up), clamped.
    fn scroll_by(&mut self, delta: isize) {
        let max = self.max_top() as isize;
        self.top = ((self.top as isize) + delta).clamp(0, max) as usize;
    }

    /// Scrolls by `delta` columns (negative = left), clamped.
    fn scroll_h_by(&mut self, delta: isize) {
        let max = self.max_left() as isize;
        self.left = ((self.left as isize) + delta).clamp(0, max) as usize;
    }

    /// Handles a mouse event in the pane's local coordinates: the wheel pans
    /// vertically, and (when shown) each scroll bar's arrows/track scroll its axis.
    /// Works regardless of focus, so the wheel acts under the pointer.
    fn handle_mouse(&mut self, m: &MouseEvent) -> EventResult {
        let text_w = self.text_w();
        let text_rows = self.text_rows();
        let vpage = text_rows.max(1) as isize;
        let hpage = text_w.max(1) as isize;
        match m.kind {
            MouseKind::ScrollDown => {
                self.scroll_by(WHEEL_STEP);
                EventResult::Consumed
            }
            MouseKind::ScrollUp => {
                self.scroll_by(-WHEEL_STEP);
                EventResult::Consumed
            }
            // The horizontal bar's row (checked first so the shared corner goes to
            // the vertical bar, matching the draw order).
            MouseKind::Down(MouseButton::Left)
                if self.needs_hbar && m.pos.y == text_rows as i16 && m.pos.x < text_w =>
            {
                let mut bar = ScrollBar::horizontal(
                    Rect::from_origin_size(Point::new(0, text_rows as i16), Size::new(text_w, 1)),
                    self.style,
                );
                bar.set_metrics(
                    self.content_width().max(0) as usize,
                    text_w as usize,
                    self.left,
                );
                match bar.hit(m.pos) {
                    Some(ScrollPart::LineUp) => self.scroll_h_by(-1),
                    Some(ScrollPart::LineDown) => self.scroll_h_by(1),
                    Some(ScrollPart::PageUp) => self.scroll_h_by(-hpage),
                    Some(ScrollPart::PageDown) => self.scroll_h_by(hpage),
                    _ => {}
                }
                EventResult::Consumed
            }
            MouseKind::Down(MouseButton::Left) if self.needs_vbar && m.pos.x == text_w => {
                let mut bar = ScrollBar::new(
                    Rect::from_origin_size(Point::new(text_w, 0), Size::new(1, text_rows as i16)),
                    self.style,
                );
                bar.set_metrics(self.lines.len(), text_rows, self.top);
                match bar.hit(m.pos) {
                    Some(ScrollPart::LineUp) => self.scroll_by(-1),
                    Some(ScrollPart::LineDown) => self.scroll_by(1),
                    Some(ScrollPart::PageUp) => self.scroll_by(-vpage),
                    Some(ScrollPart::PageDown) => self.scroll_by(vpage),
                    _ => {}
                }
                EventResult::Consumed
            }
            // A direct hit on a link's own glyphs follows it immediately,
            // regardless of the current keyboard-cycle state (ADR 0020) —
            // checked last among the left-click arms so it never shadows the
            // scroll-bar hits above (disjoint regions: bars sit exactly on
            // the `text_rows`/`text_w` boundary this can't reach).
            MouseKind::Down(MouseButton::Left) => match self.link_at(m.pos) {
                Some(i) => {
                    self.current_link = Some(i);
                    self.pending_activation = Some(self.links[i].target.clone());
                    EventResult::Consumed
                }
                None => EventResult::Ignored,
            },
            _ => EventResult::Ignored,
        }
    }
}

/// The widest line in `lines`, in display columns.
fn max_line_width(lines: &[String]) -> i16 {
    lines.iter().map(|l| l.width() as i16).max().unwrap_or(0)
}

/// One followable link at its current layout: `line` indexes the pane's
/// `lines`, `start_byte..end_byte` is a byte range *into that line's own
/// `String`* (not a display-column range — columns are derived on demand via
/// `UnicodeWidthStr`, both for drawing and for mouse hit-testing), ADR 0020.
/// A link that wraps across a line boundary produces two `PaneLink`s, one per
/// line, sharing a `target` — never merged across lines.
#[derive(Debug, Clone, PartialEq, Eq)]
struct PaneLink {
    line: usize,
    start_byte: usize,
    end_byte: usize,
    target: String,
}

/// A body laid out to a text width: the display lines, plus every link's
/// position within them.
struct LaidOutPage {
    lines: Vec<String>,
    links: Vec<PaneLink>,
}

/// Renders `body` to display lines at `width` columns: each block is reflowed
/// (paragraphs, via [`wrap_spans`]) or kept verbatim (preformatted, never
/// linked), with one blank line between blocks.
fn render_blocks(body: &[Block], width: u16) -> LaidOutPage {
    let mut lines = Vec::new();
    let mut links = Vec::new();
    for (i, block) in body.iter().enumerate() {
        if i > 0 {
            lines.push(String::new());
        }
        match block {
            Block::Paragraph(spans) => {
                let base = lines.len();
                let laid = wrap_spans(spans, width);
                links.extend(laid.links.into_iter().map(|l| PaneLink {
                    line: l.line + base,
                    ..l
                }));
                lines.extend(laid.lines);
            }
            Block::Preformatted(pre) => lines.extend(pre.iter().cloned()),
        }
    }
    LaidOutPage { lines, links }
}

/// Word-wraps `spans` to `width` display columns, using the exact greedy
/// packing policy [`wrap::wrap`](crate::wrap::wrap) uses for plain text, but
/// tracking which link (if any) produced each word.
///
/// A `Block::Paragraph`'s spans never contain `'\n'` (the parser already
/// joins a paragraph's source lines with `" "` before cutting spans), so
/// unlike the general `wrap::wrap` this never needs to handle embedded hard
/// breaks.
///
/// Spans are flattened into one continuous string before tokenizing, rather
/// than word-wrapping each span independently — tokenizing per span would
/// insert a phantom space wherever a link is immediately followed by
/// abutting punctuation with no separating space (`{x|y}.` would wrongly
/// wrap as `"x ."`). Each emitted word's link target is attributed by
/// whichever span produced the word's *first* byte, so a link immediately
/// preceded by abutting text with no separator isn't independently
/// clickable — an accepted limitation (ADR 0020), since no authored content
/// does this today.
fn wrap_spans(spans: &[Span], width: u16) -> LaidOutPage {
    let width = width as usize;
    let (flat, link_ranges) = flatten_spans(spans);

    let mut lines = Vec::new();
    let mut links = Vec::new();
    let mut current = String::new();
    // The link run (if any) still being extended on `current`: its
    // line-local byte range plus which `spans` index (not just target — two
    // different links can share a target) it came from, so only a truly
    // contiguous continuation of the *same* link span coalesces.
    let mut open_run: Option<(usize, usize, usize)> = None;

    for word_range in word_ranges(&flat) {
        let word = &flat[word_range.clone()];
        let owner = owner_span_at(word_range.start, &link_ranges);

        if current.is_empty() {
            current.push_str(word);
        } else if current.width() + 1 + word.width() <= width {
            current.push(' ');
            current.push_str(word);
        } else {
            flush_open_run(&mut open_run, &mut links, spans, lines.len());
            lines.push(std::mem::take(&mut current));
            current.push_str(word);
        }

        let word_start = current.len() - word.len();
        let word_end = current.len();
        match (owner, open_run) {
            (Some(idx), Some((start, end, prev_idx)))
                if idx == prev_idx && end + 1 == word_start =>
            {
                open_run = Some((start, word_end, prev_idx));
            }
            (Some(idx), _) => {
                flush_open_run(&mut open_run, &mut links, spans, lines.len());
                open_run = Some((word_start, word_end, idx));
            }
            (None, _) => {
                flush_open_run(&mut open_run, &mut links, spans, lines.len());
            }
        }
    }
    flush_open_run(&mut open_run, &mut links, spans, lines.len());
    lines.push(current);
    LaidOutPage { lines, links }
}

/// Closes off `open_run`, if any, pushing it as a `PaneLink` on `line`.
fn flush_open_run(
    open_run: &mut Option<(usize, usize, usize)>,
    links: &mut Vec<PaneLink>,
    spans: &[Span],
    line: usize,
) {
    if let Some((start_byte, end_byte, span_idx)) = open_run.take() {
        if let Span::Link { target, .. } = &spans[span_idx] {
            links.push(PaneLink {
                line,
                start_byte,
                end_byte,
                target: target.clone(),
            });
        }
    }
}

/// Concatenates every span's shown text (a link's `label`, a run's plain
/// text) into one string, alongside the flat byte range each *link* span
/// landed at (its index into `spans`) — text spans need no entry, since
/// [`owner_span_at`] treats "not covered by any range" as plain text.
fn flatten_spans(spans: &[Span]) -> (String, Vec<(std::ops::Range<usize>, usize)>) {
    let mut flat = String::new();
    let mut link_ranges = Vec::new();
    for (idx, span) in spans.iter().enumerate() {
        let start = flat.len();
        match span {
            Span::Text(t) => flat.push_str(t),
            Span::Link { label, .. } => {
                flat.push_str(label);
                link_ranges.push((start..flat.len(), idx));
            }
        }
    }
    (flat, link_ranges)
}

/// The `spans` index of the link range containing `byte`, if any.
fn owner_span_at(byte: usize, link_ranges: &[(std::ops::Range<usize>, usize)]) -> Option<usize> {
    link_ranges
        .iter()
        .find(|(range, _)| range.contains(&byte))
        .map(|(_, idx)| *idx)
}

/// The byte ranges of `s`'s whitespace-delimited words, in order — the same
/// boundaries `str::split_whitespace` uses, exposed as ranges so
/// [`wrap_spans`] can attribute each word back to the span it came from.
fn word_ranges(s: &str) -> Vec<std::ops::Range<usize>> {
    let mut out = Vec::new();
    let mut start: Option<usize> = None;
    for (i, c) in s.char_indices() {
        if c.is_whitespace() {
            if let Some(st) = start.take() {
                out.push(st..i);
            }
        } else if start.is_none() {
            start = Some(i);
        }
    }
    if let Some(st) = start {
        out.push(st..s.len());
    }
    out
}

impl View for HelpPane {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        canvas.fill(area, &Cell::blank(self.style));
        let text_rows = self.text_rows();
        let text_w = self.text_w();
        if text_rows == 0 || text_w <= 0 {
            return;
        }

        {
            let mut text = canvas.child(Rect::from_origin_size(
                Point::new(0, 0),
                Size::new(text_w, text_rows as i16),
            ));
            // Draw each line shifted left by the scroll offset; the child canvas
            // clips the off-screen columns on both sides (`Canvas::set`).
            for r in 0..text_rows {
                let idx = self.top + r;
                if idx >= self.lines.len() {
                    break;
                }
                let line = &self.lines[idx];
                text.put_str(Point::new(-(self.left as i16), r as i16), line, self.style);
                // Redraw each link on this line on top, in its own style —
                // the current one (only while focused) gets the same
                // highlight `ListBox` gives its focused selected row;
                // every other link still reads as clickable at rest
                // (ADR 0020).
                for (i, link) in self.links.iter().enumerate() {
                    if link.line != idx {
                        continue;
                    }
                    let start_col = line[..link.start_byte].width() as i16;
                    let style = if self.focused && self.current_link == Some(i) {
                        self.link_focus_style
                    } else {
                        self.link_style
                    };
                    text.put_str(
                        Point::new(start_col - self.left as i16, r as i16),
                        &line[link.start_byte..link.end_byte],
                        style,
                    );
                }
            }
        }

        if self.needs_vbar {
            let mut bar = ScrollBar::new(
                Rect::from_origin_size(Point::new(0, 0), Size::new(1, text_rows as i16)),
                self.style,
            );
            bar.set_metrics(self.lines.len(), text_rows, self.top);
            let mut sub = canvas.child(Rect::from_origin_size(
                Point::new(text_w, 0),
                Size::new(1, text_rows as i16),
            ));
            bar.draw(&mut sub);
        }

        if self.needs_hbar {
            let mut bar = ScrollBar::horizontal(
                Rect::from_origin_size(Point::new(0, 0), Size::new(text_w, 1)),
                self.style,
            );
            bar.set_metrics(
                self.content_width().max(0) as usize,
                text_w as usize,
                self.left,
            );
            let mut sub = canvas.child(Rect::from_origin_size(
                Point::new(0, text_rows as i16),
                Size::new(text_w, 1),
            ));
            bar.draw(&mut sub);
        }
    }

    fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
        if let Event::Mouse(m) = event {
            return self.handle_mouse(m);
        }
        if let Event::Key(key) = event {
            if !self.focused {
                return EventResult::Ignored;
            }
            let vpage = self.text_rows().max(1) as isize;
            match key.code {
                // Dedicated link-cycle keys (ADR 0020) — guarded arms ahead
                // of the plain Up/Down below, so Ctrl+Up/Down on a linkless
                // topic falls through to ordinary scrolling rather than
                // going inert.
                KeyCode::Down
                    if key.modifiers.contains(Modifiers::CONTROL) && !self.links.is_empty() =>
                {
                    self.cycle_link(1)
                }
                KeyCode::Up
                    if key.modifiers.contains(Modifiers::CONTROL) && !self.links.is_empty() =>
                {
                    self.cycle_link(-1)
                }
                KeyCode::Enter if self.current_link.is_some() => self.follow_current_link(),
                KeyCode::Up => self.scroll_by(-1),
                KeyCode::Down => self.scroll_by(1),
                KeyCode::PageUp => self.scroll_by(-vpage),
                KeyCode::PageDown => self.scroll_by(vpage),
                KeyCode::Left => self.scroll_h_by(-1),
                KeyCode::Right => self.scroll_h_by(1),
                // Home returns to the top-left corner; End jumps to the last line.
                KeyCode::Home => {
                    self.top = 0;
                    self.left = 0;
                }
                KeyCode::End => self.top = self.max_top(),
                _ => return EventResult::Ignored,
            }
            return EventResult::Consumed;
        }
        EventResult::Ignored
    }

    fn focusable(&self) -> bool {
        true
    }

    fn set_focused(&mut self, focused: bool) {
        self.focused = focused;
    }

    fn set_bounds(&mut self, bounds: Rect) {
        self.set_bounds(bounds);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::CommandSet;
    use crate::event::{KeyEvent, Modifiers};
    use crate::help::Block;

    fn rect(w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(0, 0), Size::new(w, h))
    }

    fn topic(body: Vec<Block>) -> HelpTopic {
        HelpTopic {
            id: "t".into(),
            title: "T".into(),
            body,
        }
    }

    fn pane(w: i16, h: i16, body: Vec<Block>) -> HelpPane {
        let mut p = HelpPane::new(rect(w, h), &Theme::default());
        p.show(&topic(body));
        p.set_focused(true);
        p
    }

    fn press(p: &mut HelpPane, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        p.handle_event(&Event::Key(KeyEvent::new(code, Modifiers::NONE)), &mut ctx)
    }

    fn wheel(p: &mut HelpPane, kind: MouseKind) {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        p.handle_event(
            &Event::Mouse(MouseEvent {
                kind,
                pos: Point::new(1, 1),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        );
    }

    fn click(p: &mut HelpPane, x: i16, y: i16) {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        p.handle_event(
            &Event::Mouse(MouseEvent {
                kind: MouseKind::Down(MouseButton::Left),
                pos: Point::new(x, y),
                modifiers: Modifiers::NONE,
            }),
            &mut ctx,
        );
    }

    fn ctrl(p: &mut HelpPane, code: KeyCode) -> EventResult {
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        p.handle_event(
            &Event::Key(KeyEvent::new(code, Modifiers::CONTROL)),
            &mut ctx,
        )
    }

    fn render(p: &HelpPane, w: i16, h: i16) -> String {
        let mut buf = Buffer::new(Size::new(w, h));
        let mut canvas = Canvas::new(&mut buf);
        p.draw(&mut canvas);
        buf.to_text()
    }

    #[test]
    fn prose_reflows_but_preformatted_stays_verbatim() {
        // Pane 14 wide: prose wraps at the space boundary, the 13-wide table fits.
        let p = pane(
            14,
            10,
            vec![
                Block::text("the quick brown fox jumps"),
                Block::Preformatted(vec!["Ctrl+S   Save".into(), "F3       Next".into()]),
            ],
        );
        let text = render(&p, 14, 10);
        let rows: Vec<&str> = text.lines().collect();
        // Prose wrapped to <= 12 columns.
        assert_eq!(rows[0].trim_end(), "the quick");
        assert_eq!(rows[1].trim_end(), "brown fox");
        assert_eq!(rows[2].trim_end(), "jumps");
        // A blank line between blocks, then the table verbatim (columns intact).
        assert_eq!(rows[3].trim_end(), "");
        assert_eq!(rows[4].trim_end(), "Ctrl+S   Save");
        assert_eq!(rows[5].trim_end(), "F3       Next");
    }

    #[test]
    fn a_short_page_shows_no_scroll_bar() {
        let p = pane(20, 6, vec![Block::text("one short line")]);
        assert!(!p.needs_vbar);
        assert!(!p.needs_hbar);
        let text = render(&p, 20, 6);
        // No bar glyphs in the last column.
        for row in text.lines() {
            assert!(!row.ends_with('▲') && !row.ends_with('▼'));
        }
    }

    #[test]
    fn an_overflowing_page_shows_a_scroll_bar() {
        let body = vec![Block::Preformatted(
            (0..20).map(|i| format!("line {i}")).collect(),
        )];
        let p = pane(12, 5, body);
        assert!(p.needs_vbar);
        assert_eq!(p.content_height(), 20);
        let rows: Vec<String> = render(&p, 12, 5).lines().map(str::to_string).collect();
        assert!(rows[0].ends_with('▲'), "up arrow at the top of the bar");
        assert!(rows[4].ends_with('▼'), "down arrow at the foot");
    }

    #[test]
    fn keys_scroll_and_clamp() {
        let body = vec![Block::Preformatted(
            (0..20).map(|i| format!("L{i}")).collect(),
        )];
        let mut p = pane(10, 5, body); // 20 lines, 5 rows → max_top 15
        assert_eq!(p.top, 0);
        press(&mut p, KeyCode::Down);
        assert_eq!(p.top, 1);
        press(&mut p, KeyCode::PageDown); // + 5 rows
        assert_eq!(p.top, 6);
        press(&mut p, KeyCode::End);
        assert_eq!(p.top, 15);
        press(&mut p, KeyCode::Down); // clamps at the bottom
        assert_eq!(p.top, 15);
        press(&mut p, KeyCode::Home);
        assert_eq!(p.top, 0);
        press(&mut p, KeyCode::Up); // clamps at the top
        assert_eq!(p.top, 0);
    }

    #[test]
    fn the_wheel_pans_the_page() {
        let body = vec![Block::Preformatted(
            (0..20).map(|i| format!("L{i}")).collect(),
        )];
        let mut p = pane(10, 5, body);
        wheel(&mut p, MouseKind::ScrollDown);
        assert_eq!(p.top, WHEEL_STEP as usize);
        wheel(&mut p, MouseKind::ScrollUp);
        assert_eq!(p.top, 0);
    }

    #[test]
    fn keys_are_ignored_when_unfocused_so_they_bubble() {
        let body = vec![Block::Preformatted(
            (0..20).map(|i| format!("L{i}")).collect(),
        )];
        let mut p = pane(10, 5, body);
        p.set_focused(false);
        assert_eq!(press(&mut p, KeyCode::Down), EventResult::Ignored);
        assert_eq!(p.top, 0);
    }

    #[test]
    fn show_resets_scroll_to_the_top_left() {
        let long = vec![Block::Preformatted(vec![
            "a very wide preformatted line".into(),
        ])];
        let mut p = pane(10, 5, long);
        press(&mut p, KeyCode::Right);
        assert!(p.left > 0);
        p.show(&topic(vec![Block::text("fresh")]));
        assert_eq!(p.top, 0);
        assert_eq!(p.left, 0);
    }

    // --- horizontal scrolling (the wide-`<pre>` case) ---

    #[test]
    fn a_wide_preformatted_line_shows_a_horizontal_bar_and_scrolls() {
        // One 20-column line in an 8-wide pane: a horizontal bar, no vertical one.
        let body = vec![Block::Preformatted(vec!["0123456789ABCDEFGHIJ".into()])];
        let mut p = pane(8, 4, body);
        assert!(p.needs_hbar, "wide line needs a horizontal bar");
        assert!(!p.needs_vbar, "one line needs no vertical bar");
        assert_eq!(p.content_width(), 20);

        // The bar occupies the bottom row; the text rows are above it.
        let first = render(&p, 8, 4);
        let row0 = first.lines().next().unwrap();
        assert!(row0.starts_with("01234"), "left edge visible: {row0:?}");

        // Right scrolls the content; left of the line scrolls out of view.
        press(&mut p, KeyCode::Right);
        press(&mut p, KeyCode::Right);
        assert_eq!(p.left, 2);
        let scrolled = render(&p, 8, 4);
        let row0 = scrolled.lines().next().unwrap();
        assert!(row0.starts_with("23456"), "scrolled view: {row0:?}");

        // End-of-line clamp via repeated Right, then Home returns to the start.
        for _ in 0..50 {
            press(&mut p, KeyCode::Right);
        }
        assert_eq!(p.left, p.max_left());
        assert_eq!(p.left, 20 - p.text_w() as usize);
        press(&mut p, KeyCode::Home);
        assert_eq!(p.left, 0);
    }

    #[test]
    fn the_horizontal_bar_arrows_scroll_on_click() {
        let body = vec![Block::Preformatted(vec!["0123456789ABCDEFGHIJ".into()])];
        let mut p = pane(8, 4, body);
        // text_w 8, bar on the bottom row (y = rows_total - 1 = 3). Right arrow at
        // the bar's right end.
        let text_w = p.text_w();
        assert_eq!(p.text_rows(), 3); // one row given to the bar
        click(&mut p, text_w - 1, 3);
        assert_eq!(p.left, 1, "the right arrow advances one column");
    }

    #[test]
    fn set_bounds_via_the_view_trait_reaches_the_same_relayout_logic() {
        // The inherent `set_bounds` (used directly by callers that hold a
        // concrete `HelpPane`, e.g. `Application::exec_view`-style drivers)
        // and the `View::set_bounds` override a `Window` calls through a
        // `Box<dyn View>` (ADR 0017) must relayout identically.
        let body = vec![Block::Preformatted(vec!["0123456789ABCDEFGHIJ".into()])];
        let mut p = pane(8, 4, body);
        assert!(p.needs_hbar);
        let view: &mut dyn View = &mut p;
        view.set_bounds(rect(20, 4));
        assert!(!p.needs_hbar, "widened past the content, so no bar needed");
    }

    #[test]
    fn both_bars_appear_for_a_tall_and_wide_page() {
        // Many wide lines: needs both bars; each steals a row/column.
        let body = vec![Block::Preformatted(
            (0..30)
                .map(|i| format!("{i:04}-wide-line-XXXXXXXXXXXXXXXX"))
                .collect(),
        )];
        let p = pane(10, 6, body);
        assert!(p.needs_vbar && p.needs_hbar);
        assert_eq!(p.text_w(), 9); // one column for the vertical bar
        assert_eq!(p.text_rows(), 5); // one row for the horizontal bar
    }

    // --- Followable links (ADR 0020): wrap_spans/PaneLink ---

    #[test]
    fn wrap_spans_wraps_plain_text_like_wrap_wrap() {
        // No links: byte-for-byte parity with `wrap::wrap`'s own greedy
        // packing (its `a_long_line_breaks_on_spaces_within_width` test).
        let spans = vec![Span::Text("the quick brown fox jumps".into())];
        let laid = wrap_spans(&spans, 9);
        assert_eq!(laid.lines, vec!["the quick", "brown fox", "jumps"]);
        assert!(laid.links.is_empty());
    }

    #[test]
    fn wrap_spans_does_not_insert_a_space_before_abutting_punctuation() {
        // The exact fixture that broke a naive per-span tokenizer during
        // design: no space between a link and the following ".".
        let spans = vec![
            Span::Text("See ".into()),
            Span::Link {
                label: "the keys".into(),
                target: "keyboard".into(),
            },
            Span::Text(" and ".into()),
            Span::Link {
                label: "paste".into(),
                target: "clipboard".into(),
            },
            Span::Text(".".into()),
        ];
        let laid = wrap_spans(&spans, 80);
        assert_eq!(laid.lines, vec!["See the keys and paste."]);
    }

    #[test]
    fn a_multi_word_link_label_coalesces_into_one_pane_link_within_a_line() {
        let spans = vec![Span::Link {
            label: "ab cd ef".into(),
            target: "x".into(),
        }];
        // Width 5: "ab cd" (5 cols) fits one line, "ef" wraps to the next —
        // proving both coalescing (within a line) and the no-cross-line-merge
        // rule (across the wrap) in one shot.
        let laid = wrap_spans(&spans, 5);
        assert_eq!(laid.lines, vec!["ab cd", "ef"]);
        assert_eq!(
            laid.links,
            vec![
                PaneLink {
                    line: 0,
                    start_byte: 0,
                    end_byte: 5,
                    target: "x".into()
                },
                PaneLink {
                    line: 1,
                    start_byte: 0,
                    end_byte: 2,
                    target: "x".into()
                },
            ]
        );
    }

    #[test]
    fn adjacent_links_with_no_separator_merge_into_the_earlier_ones_run() {
        // Accepted limitation (ADR 0020): a link immediately preceded by
        // abutting text/another link with no separating space isn't
        // independently clickable — the whole word attributes to whichever
        // span produced its first byte.
        let spans = vec![
            Span::Link {
                label: "a".into(),
                target: "x".into(),
            },
            Span::Link {
                label: "b".into(),
                target: "y".into(),
            },
        ];
        let laid = wrap_spans(&spans, 80);
        assert_eq!(laid.lines, vec!["ab"]);
        assert_eq!(
            laid.links,
            vec![PaneLink {
                line: 0,
                start_byte: 0,
                end_byte: 2,
                target: "x".into()
            }]
        );
    }

    #[test]
    fn render_blocks_offsets_link_lines_by_preceding_blocks() {
        let body = vec![
            Block::Preformatted(vec!["row0".into(), "row1".into()]),
            Block::Paragraph(vec![Span::Link {
                label: "go".into(),
                target: "t".into(),
            }]),
        ];
        let laid = render_blocks(&body, 40);
        // 2 preformatted rows, a blank separator, then the paragraph's line.
        assert_eq!(laid.lines, vec!["row0", "row1", "", "go"]);
        assert_eq!(
            laid.links,
            vec![PaneLink {
                line: 3,
                start_byte: 0,
                end_byte: 2,
                target: "t".into()
            }]
        );
    }

    // --- Followable links: HelpPane state & interaction ---

    #[test]
    fn show_defaults_current_link_to_the_first_link_or_none() {
        let mut p = pane(20, 6, vec![Block::text("no links here")]);
        assert_eq!(p.current_link, None);
        p.show(&topic(vec![Block::Paragraph(vec![
            Span::Text("go to ".into()),
            Span::Link {
                label: "there".into(),
                target: "there".into(),
            },
        ])]));
        assert_eq!(p.current_link, Some(0));
    }

    #[test]
    fn ctrl_down_up_cycle_the_current_link_and_wrap() {
        let body = vec![Block::Paragraph(vec![
            Span::Link {
                label: "one".into(),
                target: "a".into(),
            },
            Span::Text(" ".into()),
            Span::Link {
                label: "two".into(),
                target: "b".into(),
            },
        ])];
        let mut p = pane(40, 6, body);
        assert_eq!(p.current_link, Some(0));
        ctrl(&mut p, KeyCode::Down);
        assert_eq!(p.current_link, Some(1));
        ctrl(&mut p, KeyCode::Down); // wraps back to the first
        assert_eq!(p.current_link, Some(0));
        ctrl(&mut p, KeyCode::Up); // wraps backward to the last
        assert_eq!(p.current_link, Some(1));
    }

    #[test]
    fn ctrl_down_falls_through_to_scroll_when_the_topic_has_no_links() {
        let body = vec![Block::Preformatted(
            (0..20).map(|i| format!("L{i}")).collect(),
        )];
        let mut p = pane(10, 5, body);
        assert!(p.current_link.is_none());
        assert_eq!(p.top, 0);
        ctrl(&mut p, KeyCode::Down);
        assert_eq!(
            p.top, 1,
            "Ctrl+Down with no links scrolls instead of going inert"
        );
    }

    #[test]
    fn enter_queues_the_current_links_target_and_drains_once() {
        let body = vec![Block::Paragraph(vec![Span::Link {
            label: "go".into(),
            target: "there".into(),
        }])];
        let mut p = pane(20, 6, body);
        assert_eq!(p.take_link_activation(), None);
        press(&mut p, KeyCode::Enter);
        assert_eq!(p.take_link_activation(), Some("there".to_string()));
        assert_eq!(p.take_link_activation(), None, "drains only once");
    }

    #[test]
    fn enter_is_ignored_when_the_topic_has_no_links() {
        let mut p = pane(20, 6, vec![Block::text("no links")]);
        assert_eq!(press(&mut p, KeyCode::Enter), EventResult::Ignored);
    }

    #[test]
    fn a_direct_click_on_a_link_follows_it_immediately() {
        let body = vec![Block::Paragraph(vec![
            Span::Text("go ".into()),
            Span::Link {
                label: "there".into(),
                target: "place".into(),
            },
        ])];
        let mut p = pane(20, 6, body);
        click(&mut p, 3, 0); // inside "there"
        assert_eq!(p.current_link, Some(0));
        assert_eq!(p.take_link_activation(), Some("place".to_string()));
    }

    #[test]
    fn a_click_elsewhere_in_the_text_does_not_activate_a_link() {
        let body = vec![Block::Paragraph(vec![
            Span::Text("go ".into()),
            Span::Link {
                label: "there".into(),
                target: "place".into(),
            },
        ])];
        let mut p = pane(20, 6, body);
        click(&mut p, 0, 0); // inside "go", not the link
        assert_eq!(p.take_link_activation(), None);
    }

    #[test]
    fn cycling_scrolls_an_offscreen_link_into_view() {
        let mut body = vec![Block::Preformatted(
            (0..10).map(|i| format!("line {i}")).collect(),
        )];
        body.push(Block::Paragraph(vec![Span::Link {
            label: "target".into(),
            target: "t".into(),
        }]));
        let mut p = pane(20, 5, body);
        assert_eq!(p.top, 0);
        ctrl(&mut p, KeyCode::Down); // one link: cycles to itself, but reveals it
        assert!(p.top > 0, "scrolled to reveal the off-screen link");
    }

    #[test]
    fn cycling_resets_horizontal_scroll_to_zero() {
        let body = vec![
            Block::Preformatted(vec!["0123456789ABCDEFGHIJ".into()]),
            Block::Paragraph(vec![Span::Link {
                label: "target".into(),
                target: "t".into(),
            }]),
        ];
        let mut p = pane(8, 6, body);
        press(&mut p, KeyCode::Right);
        press(&mut p, KeyCode::Right);
        assert!(p.left > 0);
        ctrl(&mut p, KeyCode::Down);
        assert_eq!(p.left, 0);
    }

    #[test]
    fn a_link_draws_in_the_link_style_and_the_current_one_highlights_when_focused() {
        let body = vec![Block::Paragraph(vec![
            Span::Text("go ".into()),
            Span::Link {
                label: "there".into(),
                target: "place".into(),
            },
        ])];
        let mut p = pane(20, 6, body); // `pane()` leaves it focused.
        let theme = Theme::default();

        let mut buf = Buffer::new(Size::new(20, 6));
        let mut canvas = Canvas::new(&mut buf);
        p.draw(&mut canvas);
        assert_eq!(
            buf.get(Point::new(0, 0)).unwrap().style(),
            theme.style(Role::DialogBackground)
        );
        assert_eq!(
            buf.get(Point::new(3, 0)).unwrap().style(),
            theme.style(Role::Selection),
            "the current link, focused, gets the same highlight ListBox gives its selected row"
        );

        p.set_focused(false);
        let mut buf2 = Buffer::new(Size::new(20, 6));
        let mut canvas2 = Canvas::new(&mut buf2);
        p.draw(&mut canvas2);
        assert_eq!(
            buf2.get(Point::new(3, 0)).unwrap().style(),
            theme.style(Role::HelpLink),
            "unfocused: still reads as a link, just not the current-selection highlight"
        );
    }
}
