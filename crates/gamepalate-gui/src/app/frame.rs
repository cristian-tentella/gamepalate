use std::time::{Duration, Instant};

use gamepalate_app::SavedWindow;
use iced::{
    Element, Fill, Length, Size, Task, mouse, padding,
    widget::{button, column, container, mouse_area, row, space, text},
    window::{self, Direction},
};

use super::{App, BAR_ICON_SCALE, Message, UNDERLINE_HEIGHT};
use crate::{font, style, window_icon};

const RESIZE_BORDER: f32 = 5.0;
const DOUBLE_CLICK: Duration = Duration::from_millis(500);
pub(super) const MINIMIZE: char = '\u{E15B}';
pub(super) const MAXIMIZE: char = '\u{E3C6}';
pub(super) const RESTORE: char = '\u{E3E0}';
pub(super) const CLOSE: char = '\u{E5CD}';
const DEFAULT_SIZE: Size = Size::new(1280.0, 800.0);
const MIN_SIZE: Size = Size::new(960.0, 600.0);

#[derive(Clone, Copy, Debug)]
pub enum WindowAction {
    Minimize,
    ToggleMaximize,
    Close,
    Resize(Direction),
}

pub(super) fn window_button<'a>(icon: char, action: WindowAction) -> Element<'a, Message> {
    // The dash and the cross of Material Symbols are thinner than the square.
    let thin = icon == MINIMIZE || icon == CLOSE;
    let size = font::icon(10.0) * BAR_ICON_SCALE * if thin { 1.4 } else { 1.0 };
    button(
        container(
            text(icon)
                .font(font::ICONS)
                .size(size)
                .width(Fill)
                .height(Fill)
                .center(),
        )
        .padding(padding::bottom(UNDERLINE_HEIGHT)),
    )
    .width(46)
    .height(Fill)
    .padding(0)
    .style(if matches!(action, WindowAction::Close) {
        style::close_button
    } else {
        style::window_button
    })
    .on_press(Message::Window(action))
    .into()
}

pub(super) fn rescaled((_, event): (window::Id, window::Event)) -> Option<Message> {
    match event {
        window::Event::Rescaled(scale) => Some(Message::Rescaled(scale)),
        _ => None,
    }
}

pub fn window_settings(saved: Option<SavedWindow>) -> window::Settings {
    #[cfg_attr(not(windows), allow(unused_mut))]
    let mut settings = window::Settings {
        size: saved_size(saved),
        maximized: saved.is_some_and(|saved| saved.maximized),
        position: window::Position::Centered,
        min_size: Some(MIN_SIZE),
        decorations: false,
        exit_on_close_request: false,
        ..window::Settings::default()
    };
    #[cfg(windows)]
    {
        settings.platform_specific = window::settings::PlatformSpecific {
            undecorated_shadow: true,
            corner_preference: window::settings::platform::CornerPreference::Round,
            ..window::settings::PlatformSpecific::default()
        };
    }
    settings
}

pub(super) fn saved_size(saved: Option<SavedWindow>) -> Size {
    saved.map_or(DEFAULT_SIZE, |saved| {
        Size::new(
            (saved.width as f32).max(MIN_SIZE.width),
            (saved.height as f32).max(MIN_SIZE.height),
        )
    })
}

pub(super) fn restored(size: Size, maximized: bool) -> Option<Size> {
    (!maximized && size.width >= MIN_SIZE.width && size.height >= MIN_SIZE.height).then_some(size)
}

pub(super) fn resize_borders<'a>() -> Element<'a, Message> {
    column![
        row![
            resize_border(Direction::NorthWest),
            resize_border(Direction::North),
            resize_border(Direction::NorthEast),
        ],
        row![
            resize_border(Direction::West),
            space().width(Fill),
            resize_border(Direction::East),
        ]
        .height(Fill),
        row![
            resize_border(Direction::SouthWest),
            resize_border(Direction::South),
            resize_border(Direction::SouthEast),
        ],
    ]
    .into()
}

fn resize_border<'a>(direction: Direction) -> Element<'a, Message> {
    let (width, height, interaction): (Length, Length, _) = match direction {
        Direction::North | Direction::South => (
            Fill,
            RESIZE_BORDER.into(),
            mouse::Interaction::ResizingVertically,
        ),
        Direction::East | Direction::West => (
            RESIZE_BORDER.into(),
            Fill,
            mouse::Interaction::ResizingHorizontally,
        ),
        Direction::NorthWest | Direction::SouthEast => (
            RESIZE_BORDER.into(),
            RESIZE_BORDER.into(),
            mouse::Interaction::ResizingDiagonallyDown,
        ),
        Direction::NorthEast | Direction::SouthWest => (
            RESIZE_BORDER.into(),
            RESIZE_BORDER.into(),
            mouse::Interaction::ResizingDiagonallyUp,
        ),
    };

    mouse_area(space().width(width).height(height))
        .interaction(interaction)
        .on_press(Message::Window(WindowAction::Resize(direction)))
        .into()
}

impl App {
    pub(super) fn update_frame(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::WindowFound(id) => {
                self.window = id;
                match id {
                    Some(id) => Task::batch([
                        window::scale_factor(id).map(Message::Rescaled),
                        window::run(id, window_icon::use_exe_icon).discard(),
                    ]),
                    None => Task::none(),
                }
            }
            Message::Rescaled(scale) => {
                self.scale = scale;
                Task::none()
            }
            Message::Window(action) => self.window_action(action),
            Message::BarPressed => self.bar_pressed(),
            Message::Resized(size) => {
                self.resized = size;
                self.search.resized();
                if let Some(page) = &mut self.game_page {
                    page.resized();
                }
                match self.window {
                    Some(id) => window::is_maximized(id).map(Message::Maximized),
                    None => Task::none(),
                }
            }
            Message::Maximized(maximized) => {
                self.maximized = maximized;
                if let Some(size) = restored(self.resized, maximized) {
                    self.size = size;
                }
                Task::none()
            }
            _ => Task::none(),
        }
    }

    pub(super) fn save_window(&self) -> Task<Message> {
        let window = SavedWindow {
            width: self.size.width.round() as u32,
            height: self.size.height.round() as u32,
            maximized: self.maximized,
        };
        self.change_gui_settings(move |settings| settings.window = Some(window))
    }

    fn window_action(&self, action: WindowAction) -> Task<Message> {
        if matches!(action, WindowAction::Close) {
            return self.save_window().chain(iced::exit());
        }
        let Some(id) = self.window else {
            return Task::none();
        };

        match action {
            WindowAction::Minimize => window::minimize(id, true),
            WindowAction::ToggleMaximize => window::toggle_maximize(id),
            WindowAction::Resize(direction) => window::drag_resize(id, direction),
            WindowAction::Close => Task::none(),
        }
    }

    fn bar_pressed(&mut self) -> Task<Message> {
        let Some(id) = self.window else {
            return Task::none();
        };
        let now = Instant::now();
        let double = self
            .last_bar_press
            .is_some_and(|last| now.duration_since(last) < DOUBLE_CLICK);
        self.last_bar_press = (!double).then_some(now);

        if double {
            window::toggle_maximize(id)
        } else {
            window::drag(id)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_window_opens_as_it_was_saved() {
        let saved = |width, height, maximized| {
            Some(SavedWindow {
                width,
                height,
                maximized,
            })
        };

        let first = window_settings(None);
        assert_eq!(first.size, DEFAULT_SIZE);
        assert!(!first.maximized);
        assert!(matches!(first.position, window::Position::Centered));
        assert!(!first.exit_on_close_request);

        let large = window_settings(saved(1440, 900, true));
        assert_eq!(large.size, Size::new(1440.0, 900.0));
        assert!(large.maximized);

        assert_eq!(window_settings(saved(400, 300, false)).size, MIN_SIZE);
    }

    #[test]
    fn keeps_the_size_of_the_window_when_it_is_not_maximized_or_minimized() {
        let size = Size::new(1440.0, 900.0);

        assert_eq!(restored(size, false), Some(size));
        assert_eq!(restored(Size::new(2560.0, 1392.0), true), None);
        assert_eq!(restored(Size::ZERO, false), None);
    }
}
