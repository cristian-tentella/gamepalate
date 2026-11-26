use std::collections::BTreeSet;

use gamepalate_app::{AppId, Gamepalate, ListId, Lists, SaveChange};
use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::{Operation, Tree};
use iced::advanced::{Clipboard, Renderer as _, Shell, Widget, mouse, overlay, renderer};
use iced::{
    Center, Element, Event, Fill, Length, Point, Rectangle, Size, Subscription, Task, Theme,
    Vector, event,
    keyboard::{self, Key, key::Named},
    widget::{Id, button, checkbox, column, container, operation, row, text, text_input},
    window,
};

use super::game;
use crate::{font, style};

const WIDTH: f32 = 240.0;
const PADDING: f32 = 6.0;
/// Between the button and the menu.
const GAP: f32 = 6.0;
const FIELD: Id = Id::new("save-menu-name");
/// The lists of a game not saved.
pub static NO_LISTS: BTreeSet<ListId> = BTreeSet::new();

/// The menu of Save: Save alone, a box for each list, and a new list. It lists the lists
/// as they were when the facade last changed.
pub struct SaveMenu {
    lists: Lists,
    revision: u64,
    open: Option<Open>,
}

struct Open {
    game: AppId,
    row: usize,
    naming: Option<Naming>,
}

#[derive(Default)]
struct Naming {
    name: String,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    Close,
    Up,
    Down,
    Enter,
    Escape,
    Click(usize),
    Name(String),
    Submit,
}

/// What the screen does with the game the menu is open for.
#[derive(Debug, PartialEq)]
pub enum Action {
    Save(SaveChange),
    /// Make the list and put the game in it: first `create_list`, then [`SaveMenu::created`] and
    /// Save with `SaveChange::AddTo`, so the list is written before the vote.
    Create(String),
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Row {
    Save,
    List(ListId),
    New,
}

impl SaveMenu {
    pub fn new(gamepalate: &Gamepalate) -> SaveMenu {
        SaveMenu {
            lists: gamepalate.lists(),
            revision: gamepalate.revision(),
            open: None,
        }
    }

    /// Reads the lists again if the facade changed since.
    pub fn follow(&mut self, gamepalate: &Gamepalate) {
        let revision = gamepalate.revision();
        if revision != self.revision {
            self.revision = revision;
            self.lists = gamepalate.lists();
        }
    }

    pub fn lists(&self) -> &Lists {
        &self.lists
    }

    pub fn name(&self, list: ListId) -> &str {
        self.lists.get(&list).map_or("", String::as_str)
    }

    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The game the menu is open for.
    pub fn game(&self) -> Option<AppId> {
        self.open.as_ref().map(|open| open.game)
    }

    /// Opens the menu with the first row lit, so S and Enter do what S did alone.
    pub fn open(&mut self, gamepalate: &Gamepalate, game: AppId) {
        self.follow(gamepalate);
        self.open = Some(Open {
            game,
            row: 0,
            naming: None,
        });
    }

    pub fn close(&mut self) {
        self.open = None;
    }

    /// Closes the menu if the screen moved to another game.
    pub fn keep_for(&mut self, game: Option<AppId>) {
        if self.game() != game {
            self.open = None;
        }
    }

    /// The list is made: the menu has it and closes.
    pub fn created(&mut self, gamepalate: &Gamepalate) {
        self.follow(gamepalate);
        self.open = None;
    }

    /// The facade refused the name: the menu says why under the field.
    pub fn refused(&mut self, error: String) {
        if let Some(naming) = self.open.as_mut().and_then(|open| open.naming.as_mut()) {
            naming.error = Some(error);
        }
    }

    /// `saved` and `lists` are Save on the game as the screen knows it.
    pub fn update(
        &mut self,
        message: Message,
        saved: bool,
        lists: &BTreeSet<ListId>,
    ) -> (Option<Action>, Task<Message>) {
        let rows = self.rows();
        let Some(open) = self.open.as_mut() else {
            return (None, Task::none());
        };
        let (row, keyboard) = match message {
            Message::Close => {
                self.open = None;
                return (None, Task::none());
            }
            Message::Escape if open.naming.is_some() => {
                open.naming = None;
                return (None, Task::none());
            }
            Message::Escape => {
                self.open = None;
                return (None, Task::none());
            }
            Message::Up if open.naming.is_none() => {
                open.row = open.row.saturating_sub(1);
                return (None, Task::none());
            }
            Message::Down if open.naming.is_none() => {
                open.row = (open.row + 1).min(rows.len() - 1);
                return (None, Task::none());
            }
            Message::Enter if open.naming.is_none() => (open.row, true),
            Message::Click(row) if row < rows.len() => {
                open.row = row;
                open.naming = None;
                (row, false)
            }
            Message::Name(name) => {
                if let Some(naming) = &mut open.naming {
                    *naming = Naming { name, error: None };
                }
                return (None, Task::none());
            }
            Message::Submit => {
                let name = open.naming.as_ref().map(|naming| naming.name.trim());
                return (
                    name.filter(|name| !name.is_empty())
                        .map(|name| Action::Create(name.to_string())),
                    Task::none(),
                );
            }
            Message::Up | Message::Down | Message::Enter | Message::Click(_) => {
                return (None, Task::none());
            }
        };

        match rows[row] {
            Row::Save => {
                self.open = None;
                let change = if saved {
                    SaveChange::Unsave
                } else {
                    SaveChange::Save
                };
                (Some(Action::Save(change)), Task::none())
            }
            Row::List(list) => {
                if keyboard {
                    self.open = None;
                }
                let change = if lists.contains(&list) {
                    SaveChange::RemoveFrom(list)
                } else {
                    SaveChange::AddTo(list)
                };
                (Some(Action::Save(change)), Task::none())
            }
            Row::New => {
                open.naming = Some(Naming::default());
                (None, operation::focus(FIELD))
            }
        }
    }

    fn rows(&self) -> Vec<Row> {
        std::iter::once(Row::Save)
            .chain(self.lists.keys().map(|&list| Row::List(list)))
            .chain([Row::New])
            .collect()
    }

    /// The Save button, with the menu under it when it is open. `map` turns the menu's messages
    /// into the screen's.
    pub fn save<'a, M: Clone + 'a>(
        &'a self,
        saved: bool,
        lists: &BTreeSet<ListId>,
        on_press: Option<M>,
        map: impl Fn(Message) -> M + Clone + 'a,
    ) -> Element<'a, M> {
        let button = game::save_button(saved, on_press);
        let menu = self
            .open
            .as_ref()
            .map(|open| self.menu(open, saved, lists).map(map.clone()));

        anchored(button, menu, map(Message::Close))
    }

    fn menu(&self, open: &Open, saved: bool, lists: &BTreeSet<ListId>) -> Element<'_, Message> {
        let rows = self.rows();
        let line =
            |index: usize, content: Element<'static, Message>| -> Element<'static, Message> {
                let lit = open.naming.is_none() && open.row == index;
                button(content)
                    .width(Fill)
                    .padding([6, 10])
                    .style(style::list_row(lit))
                    .on_press(Message::Click(index))
                    .into()
            };

        let save = line(
            0,
            row![
                text(font::BOOKMARK).font(font::ICONS).size(font::icon(9.0)),
                text(if saved { "Unsave" } else { "Save" }).size(font::BAR_SIZE),
            ]
            .spacing(8)
            .align_y(Center)
            .into(),
        );
        let boxes = rows.iter().enumerate().filter_map(|(index, row)| {
            let Row::List(list) = *row else {
                return None;
            };
            let name = self.name(list).to_string();
            Some(line(
                index,
                checkbox(lists.contains(&list))
                    .label(name)
                    .text_size(font::BAR_SIZE)
                    .width(Fill)
                    .on_toggle(move |_| Message::Click(index))
                    .into(),
            ))
        });
        let new_list: Element<'_, Message> = match &open.naming {
            Some(naming) => column![
                text_input("List name", &naming.name)
                    .id(FIELD)
                    .on_input(Message::Name)
                    .on_submit(Message::Submit)
                    .size(font::BAR_SIZE)
                    .padding([6, 10])
                    .style(if naming.error.is_some() {
                        style::wrong_field
                    } else {
                        style::field
                    }),
            ]
            .push(naming.error.as_ref().map(|error| {
                text(error.clone())
                    .size(font::NOTE_SIZE)
                    .style(text::danger)
            }))
            .spacing(4)
            .into(),
            None => line(
                rows.len() - 1,
                row![
                    text("+").size(font::BAR_SIZE),
                    text("New list").size(font::BAR_SIZE)
                ]
                .spacing(8)
                .into(),
            ),
        };

        let lists_part = (!self.lists.is_empty()).then(|| {
            let title = container(text("Save to").size(font::NOTE_SIZE).style(style::muted))
                .padding([2, 10]);
            column![title].extend(boxes).spacing(2)
        });
        let mut menu = column![save, style::separator()].spacing(4);
        if let Some(lists_part) = lists_part {
            menu = menu.push(lists_part).push(style::separator());
        }

        container(menu.push(new_list))
            .width(WIDTH)
            .padding(PADDING)
            .style(style::popup)
            .into()
    }
}

/// The keys of the open menu. The page's own keys are quiet meanwhile. Esc is heard whatever took
/// it, since the field keeps it for itself; the rest only when nothing else did, so S in the field
/// is a letter.
pub fn keys() -> Subscription<Message> {
    event::listen_with(menu_key)
}

fn menu_key(event: Event, status: event::Status, _: window::Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    if key == Key::Named(Named::Escape) {
        return Some(Message::Escape);
    }
    if status == event::Status::Captured || modifiers.command() {
        return None;
    }
    key_message(&key)
}

fn key_message(key: &Key) -> Option<Message> {
    match key.as_ref() {
        Key::Named(Named::ArrowUp) => Some(Message::Up),
        Key::Named(Named::ArrowDown) => Some(Message::Down),
        Key::Named(Named::Enter) => Some(Message::Enter),
        Key::Character(key) if game::save_key(key) => Some(Message::Close),
        _ => None,
    }
}

/// `button` and, while there is one, `menu` under it in an overlay over the whole window: a click
/// outside the menu sends `on_close`, and the page under it is out of reach.
pub(super) fn anchored<'a, M: Clone + 'a>(
    button: Element<'a, M>,
    menu: Option<Element<'a, M>>,
    on_close: M,
) -> Element<'a, M> {
    Element::new(Anchored {
        button,
        menu,
        on_close,
    })
}

/// Where the menu goes: under its button, or above it when there is no room below, and inside the
/// window.
fn place(anchor: Rectangle, menu: Size, window: Size) -> Point {
    let x = anchor.x.min(window.width - menu.width).max(0.0);
    let below = anchor.y + anchor.height + GAP;
    let y = if below + menu.height <= window.height {
        below
    } else {
        (anchor.y - GAP - menu.height).max(0.0)
    };

    Point::new(x, y)
}

/// See [`anchored`].
struct Anchored<'a, M> {
    button: Element<'a, M>,
    menu: Option<Element<'a, M>>,
    on_close: M,
}

impl<M: Clone> Widget<M, Theme, iced::Renderer> for Anchored<'_, M> {
    fn children(&self) -> Vec<Tree> {
        std::iter::once(&self.button)
            .chain(&self.menu)
            .map(Tree::new)
            .collect()
    }

    fn diff(&self, tree: &mut Tree) {
        match &self.menu {
            Some(menu) => tree.diff_children(&[&self.button, menu]),
            None => tree.diff_children(&[&self.button]),
        }
    }

    fn size(&self) -> Size<Length> {
        self.button.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        self.button
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits)
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        self.button.as_widget().draw(
            &tree.children[0],
            renderer,
            theme,
            style,
            layout,
            cursor,
            viewport,
        );
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
        viewport: &Rectangle,
    ) {
        self.button.as_widget_mut().update(
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
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        self.button.as_widget().mouse_interaction(
            &tree.children[0],
            layout,
            cursor,
            viewport,
            renderer,
        )
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        self.button
            .as_widget_mut()
            .operate(&mut tree.children[0], layout, renderer, operation);
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        offset: Vector,
    ) -> Option<overlay::Element<'b, M, Theme, iced::Renderer>> {
        let [button_tree, rest @ ..] = &mut tree.children[..] else {
            return None;
        };
        match (&mut self.menu, rest.first_mut()) {
            (Some(menu), Some(menu_tree)) => Some(overlay::Element::new(Box::new(Menu {
                menu,
                tree: menu_tree,
                anchor: layout.bounds() + offset,
                on_close: self.on_close.clone(),
            }))),
            _ => {
                self.button
                    .as_widget_mut()
                    .overlay(button_tree, layout, renderer, viewport, offset)
            }
        }
    }
}

struct Menu<'a, 'b, M> {
    menu: &'a mut Element<'b, M>,
    tree: &'a mut Tree,
    anchor: Rectangle,
    on_close: M,
}

impl<M: Clone> overlay::Overlay<M, Theme, iced::Renderer> for Menu<'_, '_, M> {
    fn layout(&mut self, renderer: &iced::Renderer, bounds: Size) -> layout::Node {
        let limits = layout::Limits::new(Size::ZERO, bounds);
        let node = self
            .menu
            .as_widget_mut()
            .layout(self.tree, renderer, &limits);
        let position = place(self.anchor, node.size(), bounds);

        layout::Node::with_children(bounds, vec![node.move_to(position)])
    }

    fn draw(
        &self,
        renderer: &mut iced::Renderer,
        theme: &Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
    ) {
        let Some(menu_layout) = layout.children().next() else {
            return;
        };
        let bounds = layout.bounds();
        renderer.with_layer(bounds, |renderer| {
            self.menu.as_widget().draw(
                self.tree,
                renderer,
                theme,
                style,
                menu_layout,
                cursor,
                &bounds,
            );
        });
    }

    fn operate(
        &mut self,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        if let Some(menu_layout) = layout.children().next() {
            self.menu
                .as_widget_mut()
                .operate(self.tree, menu_layout, renderer, operation);
        }
    }

    fn update(
        &mut self,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, M>,
    ) {
        let Some(menu_layout) = layout.children().next() else {
            return;
        };
        self.menu.as_widget_mut().update(
            self.tree,
            event,
            menu_layout,
            cursor,
            renderer,
            clipboard,
            shell,
            &layout.bounds(),
        );
        if matches!(event, Event::Mouse(mouse::Event::ButtonPressed(_)))
            && !cursor.is_over(menu_layout.bounds())
        {
            shell.publish(self.on_close.clone());
            shell.capture_event();
        }
    }

    fn mouse_interaction(
        &self,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        let inside = layout
            .children()
            .next()
            .map_or(mouse::Interaction::None, |menu_layout| {
                self.menu.as_widget().mouse_interaction(
                    self.tree,
                    menu_layout,
                    cursor,
                    &layout.bounds(),
                    renderer,
                )
            });
        // Anything but None keeps the page under the menu out of reach of the cursor
        inside.max(mouse::Interaction::Idle)
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key;

    use super::*;

    const CO_OP: ListId = ListId(2);
    const ON_SALE: ListId = ListId(5);

    fn menu() -> SaveMenu {
        SaveMenu {
            lists: Lists::from([
                (CO_OP, "Co-op".to_string()),
                (ON_SALE, "On sale".to_string()),
            ]),
            revision: 0,
            open: Some(Open {
                game: AppId(10),
                row: 0,
                naming: None,
            }),
        }
    }

    fn press(menu: &mut SaveMenu, message: Message) -> Option<Action> {
        menu.update(message, false, &BTreeSet::new()).0
    }

    fn row(menu: &SaveMenu) -> usize {
        menu.open.as_ref().unwrap().row
    }

    #[test]
    fn the_arrows_stop_at_the_ends() {
        let mut menu = menu();
        press(&mut menu, Message::Up);
        assert_eq!(row(&menu), 0);
        for _ in 0..5 {
            press(&mut menu, Message::Down);
        }
        assert_eq!(row(&menu), 3);
        press(&mut menu, Message::Up);
        assert_eq!(row(&menu), 2);
    }

    #[test]
    fn enter_on_the_first_row_saves_or_unsaves_and_closes() {
        let mut menu = menu();
        assert_eq!(
            press(&mut menu, Message::Enter),
            Some(Action::Save(SaveChange::Save))
        );
        assert!(!menu.is_open());

        let mut menu = self::menu();
        let (action, _) = menu.update(Message::Enter, true, &BTreeSet::from([CO_OP]));
        assert_eq!(action, Some(Action::Save(SaveChange::Unsave)));
        assert!(!menu.is_open());
    }

    #[test]
    fn enter_on_a_list_puts_the_game_in_it_or_takes_it_out_and_closes() {
        let mut menu = menu();
        press(&mut menu, Message::Down);
        assert_eq!(
            press(&mut menu, Message::Enter),
            Some(Action::Save(SaveChange::AddTo(CO_OP)))
        );
        assert!(!menu.is_open());

        let mut menu = self::menu();
        press(&mut menu, Message::Down);
        press(&mut menu, Message::Down);
        let (action, _) = menu.update(Message::Enter, true, &BTreeSet::from([ON_SALE]));
        assert_eq!(action, Some(Action::Save(SaveChange::RemoveFrom(ON_SALE))));
        assert!(!menu.is_open());
    }

    #[test]
    fn a_click_on_a_list_keeps_the_menu_open_and_one_on_the_save_closes_it() {
        let mut menu = menu();
        assert_eq!(
            press(&mut menu, Message::Click(2)),
            Some(Action::Save(SaveChange::AddTo(ON_SALE)))
        );
        assert!(menu.is_open());
        assert_eq!(row(&menu), 2);
        assert_eq!(
            press(&mut menu, Message::Click(0)),
            Some(Action::Save(SaveChange::Save))
        );
        assert!(!menu.is_open());
    }

    #[test]
    fn new_list_opens_the_field_and_enter_there_makes_the_list() {
        let mut menu = menu();
        for _ in 0..3 {
            press(&mut menu, Message::Down);
        }
        assert_eq!(press(&mut menu, Message::Enter), None);
        assert!(menu.open.as_ref().unwrap().naming.is_some());
        // The arrows and the Enter of the page wait while the field is open
        press(&mut menu, Message::Up);
        assert_eq!(press(&mut menu, Message::Enter), None);
        press(&mut menu, Message::Name("  Steam Deck ".to_string()));
        assert_eq!(
            press(&mut menu, Message::Submit),
            Some(Action::Create("Steam Deck".to_string()))
        );
        assert!(menu.is_open());
    }

    #[test]
    fn an_empty_name_makes_nothing() {
        let mut menu = menu();
        press(&mut menu, Message::Click(3));
        press(&mut menu, Message::Name("   ".to_string()));
        assert_eq!(press(&mut menu, Message::Submit), None);
    }

    #[test]
    fn a_refused_name_shows_under_the_field_until_it_changes() {
        let mut menu = menu();
        press(&mut menu, Message::Click(3));
        menu.refused("A list with this name already exists".to_string());
        let naming = |menu: &SaveMenu| {
            menu.open
                .as_ref()
                .unwrap()
                .naming
                .as_ref()
                .unwrap()
                .error
                .clone()
        };
        assert_eq!(
            naming(&menu).as_deref(),
            Some("A list with this name already exists")
        );
        press(&mut menu, Message::Name("Co-op 2".to_string()));
        assert_eq!(naming(&menu), None);
    }

    #[test]
    fn esc_in_the_field_goes_back_to_the_rows_and_esc_there_closes() {
        let mut menu = menu();
        press(&mut menu, Message::Click(3));
        press(&mut menu, Message::Escape);
        assert!(menu.is_open() && menu.open.as_ref().unwrap().naming.is_none());
        assert_eq!(row(&menu), 3);
        press(&mut menu, Message::Escape);
        assert!(!menu.is_open());
    }

    #[test]
    fn without_lists_the_menu_has_the_save_and_new_list() {
        let mut menu = SaveMenu {
            lists: Lists::new(),
            ..menu()
        };
        press(&mut menu, Message::Down);
        press(&mut menu, Message::Down);
        assert_eq!(row(&menu), 1);
        assert_eq!(press(&mut menu, Message::Enter), None);
        assert!(menu.open.as_ref().unwrap().naming.is_some());
    }

    #[test]
    fn the_menu_closes_when_the_screen_moves_to_another_game() {
        let mut menu = menu();
        menu.keep_for(Some(AppId(10)));
        assert!(menu.is_open());
        menu.keep_for(Some(AppId(11)));
        assert!(!menu.is_open());
    }

    #[test]
    fn the_keys_of_the_menu() {
        assert!(matches!(
            key_message(&Key::Named(Named::ArrowUp)),
            Some(Message::Up)
        ));
        assert!(matches!(
            key_message(&Key::Named(Named::ArrowDown)),
            Some(Message::Down)
        ));
        assert!(matches!(
            key_message(&Key::Named(Named::Enter)),
            Some(Message::Enter)
        ));
        for s in ["s", "S"] {
            assert!(matches!(
                key_message(&Key::Character(s.into())),
                Some(Message::Close)
            ));
        }
        assert!(key_message(&Key::Character("1".into())).is_none());
        // The keys of the page are quiet while the menu is open
        for key in [
            Key::Named(Named::Space),
            Key::Named(Named::Backspace),
            Key::Named(Named::ArrowRight),
            Key::Character("j".into()),
            Key::Character("d".into()),
        ] {
            assert!(key_message(&key).is_none());
        }
    }

    fn pressed(key: Key, status: event::Status) -> Option<Message> {
        let event = Event::Keyboard(keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
            location: keyboard::Location::Standard,
            modifiers: keyboard::Modifiers::empty(),
            text: None,
            repeat: false,
        });
        menu_key(event, status, window::Id::unique())
    }

    #[test]
    fn esc_is_heard_even_from_the_field_but_s_there_is_a_letter() {
        let escape = Key::Named(Named::Escape);
        assert!(matches!(
            pressed(escape, event::Status::Captured),
            Some(Message::Escape)
        ));
        assert!(pressed(Key::Character("s".into()), event::Status::Captured).is_none());
        assert!(matches!(
            pressed(Key::Character("s".into()), event::Status::Ignored),
            Some(Message::Close)
        ));
    }

    #[test]
    fn the_menu_goes_under_the_save_or_above_it_inside_the_window() {
        let window = Size::new(1000.0, 700.0);
        let menu = Size::new(240.0, 200.0);
        let save = |x, y| Rectangle::new(Point::new(x, y), Size::new(36.0, 36.0));

        assert_eq!(
            place(save(400.0, 100.0), menu, window),
            Point::new(400.0, 142.0)
        );
        assert_eq!(
            place(save(400.0, 640.0), menu, window),
            Point::new(400.0, 434.0)
        );
        assert_eq!(place(save(900.0, 100.0), menu, window).x, 760.0);
        assert_eq!(
            place(save(0.0, 300.0), Size::new(240.0, 900.0), window),
            Point::new(0.0, 0.0)
        );
    }
}
