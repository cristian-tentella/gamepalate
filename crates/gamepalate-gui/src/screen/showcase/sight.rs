use iced::advanced::layout::{self, Layout};
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, mouse};
use iced::{Element, Event, Fill, Length, Rectangle, Size, Theme, window};

/// The part of a video's height that must show in the sheet for it to play by itself.
const IN_SIGHT: f32 = 0.25;

/// A transparent layer that tells, at every redraw, whether more than a quarter of the height
/// under it shows in the scrolling of the sheet. It speaks once when it appears, then at every
/// change, so the state of a sheet built again starts clean.
pub(super) fn sight<'a, M: 'a>(on_change: impl Fn(bool) -> M + 'a) -> Element<'a, M> {
    Element::new(Sight {
        on_change: Box::new(on_change),
    })
}

/// The part of the height of `bounds` inside `viewport`, from 0 to 1. The width is not counted:
/// a video of the description always takes the whole column.
fn shown(bounds: Rectangle, viewport: Rectangle) -> f32 {
    if bounds.height <= 0.0 {
        return 0.0;
    }
    let top = bounds.y.max(viewport.y);
    let bottom = (bounds.y + bounds.height).min(viewport.y + viewport.height);

    ((bottom - top) / bounds.height).clamp(0.0, 1.0)
}

fn in_sight(bounds: Rectangle, viewport: Rectangle) -> bool {
    shown(bounds, viewport) > IN_SIGHT
}

struct Sight<'a, M> {
    on_change: Box<dyn Fn(bool) -> M + 'a>,
}

impl<M> Widget<M, Theme, iced::Renderer> for Sight<'_, M> {
    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<Option<bool>>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(None::<bool>)
    }

    fn size(&self) -> Size<Length> {
        Size::new(Length::Fill, Length::Fill)
    }

    fn layout(
        &mut self,
        _tree: &mut Tree,
        _renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        layout::atomic(limits, Fill, Fill)
    }

    fn draw(
        &self,
        _tree: &Tree,
        _renderer: &mut iced::Renderer,
        _theme: &Theme,
        _style: &renderer::Style,
        _layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _viewport: &Rectangle,
    ) {
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        _cursor: mouse::Cursor,
        _renderer: &iced::Renderer,
        _clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        if !matches!(event, Event::Window(window::Event::RedrawRequested(_))) {
            return;
        }
        let now = in_sight(layout.bounds(), *viewport);
        let last = tree.state.downcast_mut::<Option<bool>>();
        if *last != Some(now) {
            *last = Some(now);
            shell.publish((self.on_change)(now));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A video 200 px high at `y`, in a sheet that shows from 0 to 600.
    fn video_at(y: f32) -> f32 {
        let bounds = Rectangle::new([0.0, y].into(), Size::new(680.0, 200.0));
        let viewport = Rectangle::new([0.0, 0.0].into(), Size::new(680.0, 600.0));
        shown(bounds, viewport)
    }

    #[test]
    fn a_video_all_inside_shows_whole() {
        assert_eq!(video_at(100.0), 1.0);
    }

    #[test]
    fn a_video_entering_from_the_bottom_shows_its_top() {
        assert_eq!(video_at(550.0), 0.25);
        assert_eq!(video_at(500.0), 0.5);
    }

    #[test]
    fn a_video_entering_from_the_top_shows_its_bottom() {
        assert_eq!(video_at(-150.0), 0.25);
        assert_eq!(video_at(-60.0), 0.7);
    }

    #[test]
    fn a_video_outside_shows_nothing() {
        assert_eq!(video_at(700.0), 0.0);
        assert_eq!(video_at(-400.0), 0.0);
    }

    #[test]
    fn a_video_taller_than_the_sheet_shows_the_part_inside() {
        let bounds = Rectangle::new([0.0, -100.0].into(), Size::new(680.0, 1000.0));
        let viewport = Rectangle::new([0.0, 0.0].into(), Size::new(680.0, 600.0));
        assert_eq!(shown(bounds, viewport), 0.6);
    }

    #[test]
    fn a_video_with_no_height_shows_nothing() {
        let bounds = Rectangle::new([0.0, 100.0].into(), Size::new(680.0, 0.0));
        let viewport = Rectangle::new([0.0, 0.0].into(), Size::new(680.0, 600.0));
        assert_eq!(shown(bounds, viewport), 0.0);
    }

    #[test]
    fn plays_only_above_a_quarter() {
        let viewport = Rectangle::new([0.0, 0.0].into(), Size::new(680.0, 600.0));
        let at = |y: f32| Rectangle::new([0.0, y].into(), Size::new(680.0, 200.0));
        assert!(!in_sight(at(550.0), viewport));
        assert!(in_sight(at(549.0), viewport));
        assert!(!in_sight(at(-150.0), viewport));
        assert!(in_sight(at(-149.0), viewport));
    }
}
