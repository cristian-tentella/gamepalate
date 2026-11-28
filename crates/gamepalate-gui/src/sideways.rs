//! Turns the vertical wheel sideways for a horizontal scrollable: the event reaches the
//! scrollable as if it were Shift+wheel, so it moves as far, frame by frame with the smooth
//! scrolling of the page. A `scroll_by` operation would not: every operation stops the smooth
//! scrolling, which then hands over only its first frame.

use iced::{
    Element, Event, Length, Rectangle, Size, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree},
    },
};

pub fn sideways<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
) -> Sideways<'a, Message, Theme, Renderer> {
    Sideways {
        content: content.into(),
    }
}

pub struct Sideways<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Sideways<'_, Message, Theme, Renderer>
where
    Renderer: renderer::Renderer,
{
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
        let content = &mut tree.children[0];

        if let Event::Mouse(mouse::Event::WheelScrolled { delta }) = event
            && cursor.is_over(layout.bounds())
        {
            let turned = Event::Mouse(mouse::Event::WheelScrolled {
                delta: turn(*delta),
            });
            self.content.as_widget_mut().update(
                content, &turned, layout, cursor, renderer, clipboard, shell, viewport,
            );
            // Over the row the page stays still, also when the row is at its end.
            shell.capture_event();
            return;
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

impl<'a, Message, Theme, Renderer> From<Sideways<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(sideways: Sideways<'a, Message, Theme, Renderer>) -> Self {
        Element::new(sideways)
    }
}

/// The wheel along its stronger axis, put on the horizontal one: down goes right, as with
/// Shift+wheel, and a horizontal touchpad swipe stays as it is.
fn turn(delta: mouse::ScrollDelta) -> mouse::ScrollDelta {
    let along = |x: f32, y: f32| if y.abs() >= x.abs() { y } else { x };
    match delta {
        mouse::ScrollDelta::Lines { x, y } => mouse::ScrollDelta::Lines {
            x: along(x, y),
            y: 0.0,
        },
        mouse::ScrollDelta::Pixels { x, y } => mouse::ScrollDelta::Pixels {
            x: along(x, y),
            y: 0.0,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mouse::ScrollDelta::{Lines, Pixels};

    #[test]
    fn the_vertical_wheel_turns_sideways_whole() {
        assert_eq!(turn(Lines { x: 0.0, y: -1.0 }), Lines { x: -1.0, y: 0.0 });
        assert_eq!(
            turn(Pixels { x: 0.0, y: 100.0 }),
            Pixels { x: 100.0, y: 0.0 }
        );
        assert_eq!(
            turn(Pixels { x: 0.0, y: -35.0 }),
            Pixels { x: -35.0, y: 0.0 }
        );
    }

    #[test]
    fn a_sideways_swipe_stays_as_it_is() {
        assert_eq!(
            turn(Pixels { x: -12.0, y: 3.0 }),
            Pixels { x: -12.0, y: 0.0 }
        );
        assert_eq!(turn(Lines { x: 1.0, y: 0.0 }), Lines { x: 1.0, y: 0.0 });
    }
}
