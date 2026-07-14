//! A titled, bordered box for visually grouping related controls
//! (`docs/specs/group_box.md`).
//!
//! Distinct from [`Frame`](super::Frame) (window-chrome border drawing, with
//! close/zoom/help glyphs, not an independent [`View`]) and from
//! [`Window`](super::Window) (a whole floating/dockable box with move/resize/
//! close policy). `GroupBox` is a plain, static, in-dialog control: a
//! single-line border with a left-aligned embedded title, owning a real
//! [`Group`] of children placed inside it — the same "bordered box wrapping
//! an interior" composition `Window` already uses (ADR 0016), specialised to
//! an interior that is always a `Group` rather than any `View`.

use crate::canvas::Canvas;
use crate::cell::Cell;
use crate::color::Style;
use crate::event::{Event, EventResult, MouseEvent};
use crate::geometry::{Point, Rect, Size};
use crate::theme::{Role, Theme};
use crate::view::{Context, Group, View};

/// A titled border around a group of child views.
pub struct GroupBox {
    bounds: Rect,
    title: String,
    style: Style,
    interior_fill: Cell,
    interior: Group,
}

impl GroupBox {
    /// Creates a group box at `bounds` titled `title`, owning `children` —
    /// laid out in interior-local coordinates, `(0, 0)` sitting one cell in
    /// from the border (mirrors [`Window::interior_bounds`](super::Window::interior_bounds)).
    /// Border and title both resolve [`Role::DialogBackground`] — the same
    /// role [`RadioButtons`](super::RadioButtons)/[`CheckBox`](super::CheckBox)/
    /// [`Label`](super::Label) already use for their own body text, so a
    /// group box matches whatever dialog it sits in without a dedicated
    /// theme role.
    pub fn new(bounds: Rect, title: &str, children: Vec<Box<dyn View>>, theme: &Theme) -> Self {
        let style = theme.style(Role::DialogBackground);
        let interior_bounds = Self::compute_interior_bounds(bounds);
        Self {
            bounds,
            title: title.to_string(),
            style,
            interior_fill: Cell::blank(style),
            // Non-wrapping (ADR 0031): once Tab/Shift-Tab exhausts this box's
            // own children, it must escape to whatever `Group` owns this
            // `GroupBox`, not wrap back onto itself — otherwise a box with a
            // single focusable child (a lone `RadioButtons`, the whole point
            // of this widget) would swallow Tab forever.
            interior: Group::new(interior_bounds, children).non_wrapping(),
        }
    }

    /// The interior rectangle in the box's own local coordinates: the whole
    /// box inset by one cell on every side (the border). Collapses to empty
    /// for a box too small to have one.
    pub fn interior_bounds(&self) -> Rect {
        Self::compute_interior_bounds(self.bounds)
    }

    fn compute_interior_bounds(bounds: Rect) -> Rect {
        let Size { width, height } = bounds.size();
        Rect::from_origin_size(
            Point::new(1, 1),
            Size::new((width - 2).max(0), (height - 2).max(0)),
        )
    }

    /// Strokes the single-line box and, if `title` is non-empty, embeds it
    /// left-aligned on the top edge (`┌ Title ────┐`), truncated to fit.
    fn draw_border(&self, canvas: &mut Canvas, area: Rect) {
        let br = area.bottom_right();
        let (left, top) = (area.origin().x, area.origin().y);
        let (right, bottom) = (br.x - 1, br.y - 1);

        let h = Cell::from_char('─', self.style);
        let v = Cell::from_char('│', self.style);
        for x in left..=right {
            canvas.set(Point::new(x, top), h.clone());
            canvas.set(Point::new(x, bottom), h.clone());
        }
        for y in top..=bottom {
            canvas.set(Point::new(left, y), v.clone());
            canvas.set(Point::new(right, y), v.clone());
        }
        canvas.set(Point::new(left, top), Cell::from_char('┌', self.style));
        canvas.set(Point::new(right, top), Cell::from_char('┐', self.style));
        canvas.set(Point::new(left, bottom), Cell::from_char('└', self.style));
        canvas.set(Point::new(right, bottom), Cell::from_char('┘', self.style));

        if self.title.is_empty() || right - left < 2 {
            return;
        }
        let span = (right - left - 1) as usize;
        let label = format!(" {} ", self.title);
        let shown: String = label.chars().take(span).collect();
        canvas.put_str(Point::new(left + 1, top), &shown, self.style);
    }
}

impl View for GroupBox {
    fn bounds(&self) -> Rect {
        self.bounds
    }

    fn draw(&self, canvas: &mut Canvas) {
        let area = canvas.bounds();
        if area.width() < 2 || area.height() < 2 {
            return;
        }
        self.draw_border(canvas, area);

        let interior = self.interior_bounds();
        if !interior.is_empty() {
            let mut sub = canvas.child(interior);
            let fill_area = sub.bounds();
            sub.fill(fill_area, &self.interior_fill);
            self.interior.draw(&mut sub);
        }
    }

    fn handle_event(&mut self, event: &Event, ctx: &mut Context) -> EventResult {
        match event {
            Event::Mouse(mouse) => {
                let interior = self.interior_bounds();
                if interior.contains(mouse.pos) {
                    let origin = interior.origin();
                    let local = MouseEvent {
                        pos: mouse.pos.offset(-origin.x, -origin.y),
                        ..*mouse
                    };
                    ctx.translated(origin.x, origin.y, |ctx| {
                        self.interior.handle_event(&Event::Mouse(local), ctx)
                    })
                } else {
                    EventResult::Ignored
                }
            }
            _ => self.interior.handle_event(event, ctx),
        }
    }

    fn focusable(&self) -> bool {
        self.interior.focusable()
    }

    fn set_focused(&mut self, focused: bool) {
        self.interior.set_focused(focused);
    }

    fn reset_focus(&mut self) {
        self.interior.reset_focus();
    }

    fn valid(&mut self, command: crate::command::Command, ctx: &mut Context) -> bool {
        self.interior.valid(command, ctx)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::buffer::Buffer;
    use crate::command::{CM_OK, CommandSet};
    use crate::event::{KeyCode, KeyEvent, Modifiers, MouseButton, MouseKind};
    use crate::view::StaticText;

    fn rect(x: i16, y: i16, w: i16, h: i16) -> Rect {
        Rect::from_origin_size(Point::new(x, y), Size::new(w, h))
    }

    fn theme() -> Theme {
        Theme::default()
    }

    fn key(code: KeyCode) -> Event {
        Event::Key(KeyEvent::new(code, Modifiers::NONE))
    }

    fn mouse_down_at(x: i16, y: i16) -> Event {
        Event::Mouse(MouseEvent {
            kind: MouseKind::Down(MouseButton::Left),
            pos: Point::new(x, y),
            modifiers: Modifiers::NONE,
        })
    }

    fn render(gb: &GroupBox, w: i16, h: i16) -> String {
        let mut buf = Buffer::new(Size::new(w, h));
        let mut canvas = Canvas::new(&mut buf);
        gb.draw(&mut canvas);
        buf.to_text()
    }

    // --- Geometry ---

    #[test]
    fn interior_bounds_inset_by_one_on_each_side() {
        let gb = GroupBox::new(rect(0, 0, 20, 8), "Box", vec![], &theme());
        assert_eq!(gb.interior_bounds(), rect(1, 1, 18, 6));
    }

    #[test]
    fn interior_bounds_collapses_to_empty_for_a_too_small_box() {
        let gb = GroupBox::new(rect(0, 0, 1, 1), "Box", vec![], &theme());
        assert!(gb.interior_bounds().is_empty());
    }

    // --- Rendering ---

    #[test]
    fn snapshot_titled_box_around_children() {
        let children: Vec<Box<dyn View>> = vec![Box::new(StaticText::new(
            rect(0, 0, 10, 1),
            "(•) Left",
            Style::new(),
        ))];
        let gb = GroupBox::new(rect(0, 0, 16, 4), "Alignment", children, &theme());
        insta::assert_snapshot!(render(&gb, 16, 4));
    }

    #[test]
    fn empty_title_draws_a_plain_unbroken_box() {
        let gb = GroupBox::new(rect(0, 0, 10, 3), "", vec![], &theme());
        let text = render(&gb, 10, 3);
        assert!(!text.contains('T'), "no title glyphs leaked in");
        let top = text.lines().next().unwrap();
        assert!(top.chars().skip(1).take(8).all(|c| c == '─'));
    }

    #[test]
    fn a_long_title_truncates_to_fit() {
        let gb = GroupBox::new(
            rect(0, 0, 10, 3),
            "A Very Long Title Indeed",
            vec![],
            &theme(),
        );
        let text = render(&gb, 10, 3);
        let top = text.lines().next().unwrap();
        assert_eq!(top.chars().count(), 10, "still exactly the box width");
    }

    #[test]
    fn tiny_area_does_not_panic() {
        let gb = GroupBox::new(rect(0, 0, 1, 1), "X", vec![], &theme());
        let text = render(&gb, 1, 1);
        assert_eq!(text, " ");
    }

    // --- Interaction ---

    #[test]
    fn a_click_inside_the_interior_reaches_the_right_child_at_translated_coords() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct Probe {
            bounds: Rect,
            seen: Rc<RefCell<Vec<Event>>>,
        }
        impl View for Probe {
            fn bounds(&self) -> Rect {
                self.bounds
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
                self.seen.borrow_mut().push(event.clone());
                EventResult::Consumed
            }
            fn focusable(&self) -> bool {
                true
            }
        }

        let seen = Rc::new(RefCell::new(Vec::new()));
        let probe = Probe {
            bounds: rect(2, 1, 5, 1),
            seen: Rc::clone(&seen),
        };
        let gb = GroupBox::new(rect(0, 0, 20, 6), "Box", vec![Box::new(probe)], &theme());
        let mut gb = gb;
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        // Box interior starts at (1, 1); the child sits at (2, 1) within it,
        // so screen (1 + 2 + 1, 1 + 1) = (4, 2) lands inside the child at
        // local (1, 0).
        let result = gb.handle_event(&mouse_down_at(4, 2), &mut ctx);
        assert_eq!(result, EventResult::Consumed);
        assert_eq!(seen.borrow().as_slice(), &[mouse_down_at(1, 0)]);
    }

    #[test]
    fn a_click_on_the_border_itself_is_ignored() {
        let gb_children: Vec<Box<dyn View>> = vec![];
        let mut gb = GroupBox::new(rect(0, 0, 10, 4), "Box", gb_children, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        let result = gb.handle_event(&mouse_down_at(0, 0), &mut ctx);
        assert_eq!(result, EventResult::Ignored);
    }

    #[test]
    fn keys_reach_the_focused_child() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct Probe {
            bounds: Rect,
            seen: Rc<RefCell<Vec<Event>>>,
        }
        impl View for Probe {
            fn bounds(&self) -> Rect {
                self.bounds
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn handle_event(&mut self, event: &Event, _ctx: &mut Context) -> EventResult {
                self.seen.borrow_mut().push(event.clone());
                EventResult::Consumed
            }
            fn focusable(&self) -> bool {
                true
            }
        }

        let seen = Rc::new(RefCell::new(Vec::new()));
        let probe = Probe {
            bounds: rect(0, 0, 5, 1),
            seen: Rc::clone(&seen),
        };
        let mut gb = GroupBox::new(rect(0, 0, 20, 6), "Box", vec![Box::new(probe)], &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        gb.handle_event(&key(KeyCode::Char('a')), &mut ctx);
        assert_eq!(seen.borrow().as_slice(), &[key(KeyCode::Char('a'))]);
    }

    #[test]
    fn tab_and_back_tab_cycle_focus_among_children() {
        let children: Vec<Box<dyn View>> = vec![
            Box::new(StaticText::new(rect(0, 0, 5, 1), "static", Style::new())),
            Box::new(super::super::Button::new(
                rect(0, 1, 8, 1),
                "One",
                CM_OK,
                &theme(),
            )),
            Box::new(super::super::Button::new(
                rect(0, 2, 8, 1),
                "Two",
                CM_OK,
                &theme(),
            )),
        ];
        let mut gb = GroupBox::new(rect(0, 0, 20, 8), "Box", children, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert!(gb.focusable());
        assert_eq!(
            gb.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(
            gb.handle_event(&key(KeyCode::BackTab), &mut ctx),
            EventResult::Consumed
        );
    }

    // --- Tab escapes to an owning `Group` once exhausted (ADR 0031) ---
    //
    // Reproduces a real bug found manually driving the `dialogs` example: a
    // `GroupBox` holding a single `RadioButtons` (this widget's whole
    // motivating case, e.g. a set of radio buttons under "Alignment:")
    // swallowed every subsequent Tab once focus reached it, because an
    // ordinary wrapping `Group` treats "only one focusable child" as
    // "wrap Tab back to myself" and consumes it — so focus could never reach
    // a sibling control (an OK/Cancel button) laid out after the box.

    #[test]
    fn tab_escapes_a_group_box_with_one_focusable_child_to_reach_a_later_sibling() {
        let radio: Vec<Box<dyn View>> = vec![Box::new(super::super::RadioButtons::new(
            rect(0, 0, 12, 2),
            &["A", "B"],
            &theme(),
        ))];
        let gb = GroupBox::new(rect(0, 0, 20, 4), "Alignment", radio, &theme());
        let after = super::super::Button::new(rect(0, 5, 8, 1), "OK", CM_OK, &theme());
        let mut outer =
            crate::view::Group::new(rect(0, 0, 20, 10), vec![Box::new(gb), Box::new(after)]);
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);

        assert_eq!(outer.focused(), Some(0), "starts on the group box");
        assert_eq!(
            outer.handle_event(&key(KeyCode::Tab), &mut ctx),
            EventResult::Consumed,
            "the outer group itself still consumes and advances"
        );
        assert_eq!(
            outer.focused(),
            Some(1),
            "Tab escaped the group box to reach the OK button, not swallowed"
        );
        assert_eq!(
            outer.handle_event(&key(KeyCode::BackTab), &mut ctx),
            EventResult::Consumed
        );
        assert_eq!(
            outer.focused(),
            Some(0),
            "Shift-Tab returns to the group box"
        );
    }

    #[test]
    fn focusable_reflects_whether_the_interior_has_a_focusable_child() {
        let gb = GroupBox::new(rect(0, 0, 10, 4), "Box", vec![], &theme());
        assert!(!gb.focusable());

        let children: Vec<Box<dyn View>> = vec![Box::new(super::super::Button::new(
            rect(0, 0, 8, 1),
            "One",
            CM_OK,
            &theme(),
        ))];
        let gb = GroupBox::new(rect(0, 0, 10, 4), "Box", children, &theme());
        assert!(gb.focusable());
    }

    #[test]
    fn set_focused_forwards_to_the_interior_child_that_holds_focus() {
        use std::cell::RefCell;
        use std::rc::Rc;

        struct FocusSpy {
            bounds: Rect,
            focused: Rc<RefCell<bool>>,
        }
        impl View for FocusSpy {
            fn bounds(&self) -> Rect {
                self.bounds
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn focusable(&self) -> bool {
                true
            }
            fn set_focused(&mut self, focused: bool) {
                *self.focused.borrow_mut() = focused;
            }
        }

        let flag = Rc::new(RefCell::new(false));
        let mut gb = GroupBox::new(
            rect(0, 0, 10, 4),
            "Box",
            vec![Box::new(FocusSpy {
                bounds: rect(0, 0, 5, 1),
                focused: Rc::clone(&flag),
            })],
            &theme(),
        );
        assert!(*flag.borrow(), "initial focusable child told it has focus");
        gb.set_focused(false);
        assert!(!*flag.borrow());
        gb.set_focused(true);
        assert!(*flag.borrow());
    }

    #[test]
    fn reset_focus_forwards_to_the_interior_group() {
        let children: Vec<Box<dyn View>> = vec![
            Box::new(super::super::Button::new(
                rect(0, 0, 8, 1),
                "One",
                CM_OK,
                &theme(),
            )),
            Box::new(super::super::Button::new(
                rect(0, 1, 8, 1),
                "Two",
                CM_OK,
                &theme(),
            )),
        ];
        let mut gb = GroupBox::new(rect(0, 0, 20, 4), "Box", children, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        gb.handle_event(&key(KeyCode::Tab), &mut ctx);
        assert_eq!(gb.interior.focused(), Some(1));

        gb.reset_focus();

        assert_eq!(
            gb.interior.focused(),
            Some(0),
            "reset_focus reached the interior Group"
        );
    }

    #[test]
    fn a_childs_posted_command_bubbles_out() {
        let children: Vec<Box<dyn View>> = vec![Box::new(super::super::Button::new(
            rect(0, 0, 8, 1),
            "OK",
            CM_OK,
            &theme(),
        ))];
        let mut gb = GroupBox::new(rect(0, 0, 20, 4), "Box", children, &theme());
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        gb.handle_event(&key(KeyCode::Enter), &mut ctx);
        assert_eq!(ctx.posted(), &[Event::Command(CM_OK)]);
    }

    #[test]
    fn valid_fans_out_to_every_child() {
        struct Vetoer {
            bounds: Rect,
        }
        impl View for Vetoer {
            fn bounds(&self) -> Rect {
                self.bounds
            }
            fn draw(&self, _canvas: &mut Canvas) {}
            fn valid(&mut self, _command: crate::command::Command, _ctx: &mut Context) -> bool {
                false
            }
        }
        let mut gb = GroupBox::new(
            rect(0, 0, 10, 4),
            "Box",
            vec![Box::new(Vetoer {
                bounds: rect(0, 0, 5, 1),
            })],
            &theme(),
        );
        let cs = CommandSet::new();
        let mut ctx = Context::new(&cs);
        assert!(!gb.valid(CM_OK, &mut ctx));
    }
}
