use iced::{
    Element, Event, Length, Point, Rectangle, Size, Vector,
    advanced::{
        Clipboard, Layout, Shell, Widget, layout, mouse, overlay, renderer,
        widget::{Operation, Tree},
    },
};

const GAP: f32 = 6.0;
const MARGIN: f32 = 16.0;

/// The content with a menu right below it, over everything else. A click outside both closes it.
pub fn dropdown<'a, Message, Theme, Renderer>(
    content: impl Into<Element<'a, Message, Theme, Renderer>>,
    menu: Option<Element<'a, Message, Theme, Renderer>>,
    on_dismiss: Message,
) -> Dropdown<'a, Message, Theme, Renderer> {
    Dropdown {
        content: content.into(),
        menu,
        on_dismiss,
    }
}

pub struct Dropdown<'a, Message, Theme, Renderer> {
    content: Element<'a, Message, Theme, Renderer>,
    menu: Option<Element<'a, Message, Theme, Renderer>>,
    on_dismiss: Message,
}

impl<'a, Message, Theme, Renderer> Dropdown<'a, Message, Theme, Renderer> {
    fn widgets(&self) -> Vec<&dyn Widget<Message, Theme, Renderer>> {
        std::iter::once(&self.content)
            .chain(&self.menu)
            .map(Element::as_widget)
            .collect()
    }
}

impl<Message, Theme, Renderer> Widget<Message, Theme, Renderer>
    for Dropdown<'_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn children(&self) -> Vec<Tree> {
        self.widgets().into_iter().map(Tree::new).collect()
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&self.widgets());
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
        self.content.as_widget_mut().update(
            &mut tree.children[0],
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            viewport,
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
        let (content, rest) = tree.children.split_first_mut()?;
        let Some((menu, menu_tree)) = self.menu.as_mut().zip(rest.first_mut()) else {
            return self.content.as_widget_mut().overlay(
                content,
                layout,
                renderer,
                viewport,
                translation,
            );
        };

        Some(overlay::Element::new(Box::new(Menu {
            menu,
            tree: menu_tree,
            anchor: layout.bounds() + translation,
            on_dismiss: self.on_dismiss.clone(),
        })))
    }
}

impl<'a, Message, Theme, Renderer> From<Dropdown<'a, Message, Theme, Renderer>>
    for Element<'a, Message, Theme, Renderer>
where
    Message: Clone + 'a,
    Theme: 'a,
    Renderer: renderer::Renderer + 'a,
{
    fn from(dropdown: Dropdown<'a, Message, Theme, Renderer>) -> Self {
        Element::new(dropdown)
    }
}

struct Menu<'a, 'b, Message, Theme, Renderer> {
    menu: &'a mut Element<'b, Message, Theme, Renderer>,
    tree: &'a mut Tree,
    anchor: Rectangle,
    on_dismiss: Message,
}

impl<Message, Theme, Renderer> overlay::Overlay<Message, Theme, Renderer>
    for Menu<'_, '_, Message, Theme, Renderer>
where
    Message: Clone,
    Renderer: renderer::Renderer,
{
    fn layout(&mut self, renderer: &Renderer, bounds: Size) -> layout::Node {
        let top = self.anchor.y + self.anchor.height + GAP;
        let limits = layout::Limits::new(
            Size::ZERO,
            Size::new(
                (bounds.width - self.anchor.x - MARGIN).max(0.0),
                (bounds.height - top - MARGIN).max(0.0),
            ),
        );

        self.menu
            .as_widget_mut()
            .layout(self.tree, renderer, &limits)
            .move_to(Point::new(self.anchor.x, top))
    }

    fn draw(
        &self,
        renderer: &mut Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        self.menu.as_widget().draw(
            self.tree,
            renderer,
            theme,
            style,
            layout,
            cursor,
            &layout.bounds(),
        );
    }

    fn operate(&mut self, layout: Layout<'_>, renderer: &Renderer, operation: &mut dyn Operation) {
        self.menu
            .as_widget_mut()
            .operate(self.tree, layout, renderer, operation);
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
    ) {
        if let Event::Mouse(mouse::Event::ButtonPressed(_)) = event
            && !cursor.is_over(layout.bounds())
            && !cursor.is_over(self.anchor)
        {
            shell.publish(self.on_dismiss.clone());
        }

        self.menu.as_widget_mut().update(
            self.tree,
            event,
            layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &layout.bounds(),
        );
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &Renderer,
    ) -> mouse::Interaction {
        self.menu.as_widget().mouse_interaction(
            self.tree,
            layout,
            cursor,
            &layout.bounds(),
            renderer,
        )
    }
}
