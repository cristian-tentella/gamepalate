use std::time::{Duration, Instant};

use iced::{
    Element, Event, Length, Rectangle, Size, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree, tree},
    },
    keyboard,
    window::{self, RedrawRequest},
};

const LINE: f32 = 60.0;
const TIME_CONSTANT: Duration = Duration::from_millis(40);
const REST: f32 = 0.5;

pub fn smooth<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Smooth<'a, Message, Theme, Renderer> {
    Smooth {
        content: content.into(),
    }
}

pub struct Smooth<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

#[derive(Default)]
struct State {
    remaining: Vector,
    last: Option<Instant>,
    shift: bool,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Smooth<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &Renderer,
        operation: &mut dyn Operation,
    ) {
        let state = tree.state.downcast_mut::<State>();
        state.remaining = Vector::ZERO;
        state.last = None;

        self.content
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        let state = tree.state.downcast_mut::<State>();
        let content = &mut tree.children[0];

        match event {
            Event::Keyboard(keyboard::Event::ModifiersChanged(modifiers)) => {
                state.shift = modifiers.shift();
            }
            Event::Mouse(mouse::Event::WheelScrolled { delta })
                if cursor.is_over(layout.bounds()) =>
            {
                if let Some(movement) = notch(*delta, state.shift) {
                    state.remaining += movement;
                    state.last.get_or_insert_with(Instant::now);
                    shell.capture_event();
                    shell.request_redraw();
                    return;
                }
            }
            Event::Window(window::Event::RedrawRequested(now)) => {
                self.content.as_widget_mut().update(
                    content, event, layout, cursor, renderer, clipboard, shell, viewport,
                );

                let Some(last) = state.last else {
                    return;
                };
                let (passed, left) = step(state.remaining, now.saturating_duration_since(last));
                state.remaining = left;
                state.last = (left != Vector::ZERO).then_some(*now);

                let scrolled = Event::Mouse(mouse::Event::WheelScrolled {
                    delta: mouse::ScrollDelta::Pixels {
                        x: passed.x,
                        y: passed.y,
                    },
                });
                let mut messages = Vec::new();
                let mut inner = Shell::new(&mut messages);
                self.content.as_widget_mut().update(
                    content, &scrolled, layout, cursor, renderer, clipboard, &mut inner, viewport,
                );
                let redraw = inner.redraw_request();
                let layout_invalid = inner.is_layout_invalid();
                let widgets_invalid = inner.are_widgets_invalid();
                drop(inner);

                for message in messages {
                    shell.publish(message);
                }
                if layout_invalid {
                    shell.invalidate_layout();
                }
                if widgets_invalid {
                    shell.invalidate_widgets();
                }
                match redraw {
                    RedrawRequest::NextFrame => shell.request_redraw(),
                    RedrawRequest::At(at) => shell.request_redraw_at(at),
                    RedrawRequest::Wait => {}
                }
                if state.last.is_some() {
                    shell.request_redraw();
                }
                return;
            }
            _ => {}
        }

        self.content.as_widget_mut().update(
            content, event, layout, cursor, renderer, clipboard, shell, viewport,
        );
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.content.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.content.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, Theme, Renderer>> {
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            layout,
            renderer,
            viewport,
            translation,
        )
    }
}

impl<'a, Message, Theme, Renderer> From<Smooth<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(smooth: Smooth<'a, Message, Theme, Renderer>) -> Self {
        Element::new(smooth)
    }
}

fn notch(delta: mouse::ScrollDelta, shift: bool) -> Option<Vector> {
    let mouse::ScrollDelta::Lines { x, y } = delta else {
        return None;
    };
    let (x, y) = if cfg!(target_os = "macos") && shift {
        (y, x)
    } else {
        (x, y)
    };
    let movement = if shift {
        Vector::new(y, x)
    } else {
        Vector::new(x, y)
    };

    Some(movement * LINE)
}

fn step(remaining: Vector, elapsed: Duration) -> (Vector, Vector) {
    let left = remaining * (-elapsed.as_secs_f32() / TIME_CONSTANT.as_secs_f32()).exp();

    if left.x.abs() < REST && left.y.abs() < REST {
        (remaining, Vector::ZERO)
    } else {
        (remaining - left, left)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FRAME: Duration = Duration::from_millis(16);

    fn lines(y: f32) -> mouse::ScrollDelta {
        mouse::ScrollDelta::Lines { x: 0.0, y }
    }

    #[test]
    fn a_notch_is_sixty_pixels_and_sideways_with_shift() {
        assert_eq!(notch(lines(1.0), false), Some(Vector::new(0.0, 60.0)));
        assert_eq!(notch(lines(-1.0), true), Some(Vector::new(-60.0, 0.0)));
    }

    #[test]
    fn touchpad_pixels_are_not_animated() {
        assert_eq!(
            notch(mouse::ScrollDelta::Pixels { x: 0.0, y: 7.0 }, false),
            None
        );
    }

    #[test]
    fn a_notch_slows_down_and_is_done_within_150_ms() {
        let (_, left) = step(Vector::new(0.0, 60.0), FRAME);
        assert!((left.y / 60.0 - 0.67).abs() < 0.01);

        let (_, left) = step(Vector::new(0.0, 60.0), Duration::from_millis(150));
        assert!(left.y < 60.0 * 0.03);
    }

    #[test]
    fn the_frames_add_up_to_the_whole_notch() {
        let mut remaining = Vector::new(0.0, 60.0);
        let mut total = 0.0;
        let mut frames = 0;

        while remaining != Vector::ZERO {
            let (passed, left) = step(remaining, FRAME);
            total += passed.y;
            remaining = left;
            frames += 1;
        }

        assert!((total - 60.0).abs() < 1e-3);
        assert!(frames <= 12);
    }

    #[test]
    fn notches_add_up_and_cancel_out() {
        let down = notch(lines(1.0), false).unwrap();
        let up = notch(lines(-1.0), false).unwrap();

        assert_eq!(down + down, Vector::new(0.0, 120.0));
        assert_eq!(down + up, Vector::ZERO);
    }

    #[test]
    fn under_half_a_pixel_it_stops() {
        assert_eq!(
            step(Vector::new(0.0, 0.4), Duration::ZERO),
            (Vector::new(0.0, 0.4), Vector::ZERO)
        );
    }
}
