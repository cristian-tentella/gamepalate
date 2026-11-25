use gamepalate_app::{ListId, StarredList};
use iced::{
    Center, Element, Event, Fill, Subscription, Task, event,
    keyboard::{self, Key, key::Named},
    widget::{
        Id, button, column, container, mouse_area, operation, row, text, text::Wrapping, text_input,
    },
    window,
};

use super::star_menu;
use crate::{font, format, style};

const WIDTH: f32 = 240.0;
const PADDING: f32 = 6.0;
const FIELD: Id = Id::new("list-menu-name");

/// The menu of the Starred segment of Votes: all the starred games, one list, or the starred
/// games in no list. A right click on a list renames it or deletes it.
#[derive(Default)]
pub struct ListMenu {
    open: Option<Open>,
}

struct Open {
    row: usize,
    edit: Option<Edit>,
}

#[derive(Debug, PartialEq)]
enum Edit {
    /// Rename and Delete, after a right click.
    Actions(ListId),
    Renaming {
        list: ListId,
        name: String,
        error: Option<String>,
    },
    Deleting(ListId),
}

#[derive(Clone, Debug)]
pub enum Message {
    Close,
    Up,
    Down,
    Enter,
    Escape,
    Click(usize),
    RightClick(usize),
    Rename,
    Delete,
    Name(String),
    Submit,
    Confirm,
    Cancel,
}

/// What Votes does with a row of the menu.
#[derive(Debug, PartialEq)]
pub enum Action {
    Choose(StarredList),
    Rename(ListId, String),
    Delete(ListId),
}

/// The rows of the menu, with the games each shows.
#[derive(Debug, Default)]
pub struct Choices {
    pub all: usize,
    pub lists: Vec<ListChoice>,
    /// Only when some starred games are in no list, or No list is the one chosen.
    pub no_list: Option<usize>,
}

#[derive(Debug)]
pub struct ListChoice {
    pub id: ListId,
    pub name: String,
    /// The games the segment would show: the filters of Votes count.
    pub shown: usize,
    /// All the games in the list, which the confirmation of Delete counts.
    pub games: usize,
}

impl Choices {
    fn rows(&self) -> Vec<StarredList> {
        std::iter::once(StarredList::All)
            .chain(self.lists.iter().map(|list| StarredList::List(list.id)))
            .chain(self.no_list.map(|_| StarredList::NoList))
            .collect()
    }

    fn list(&self, id: ListId) -> Option<&ListChoice> {
        self.lists.iter().find(|list| list.id == id)
    }
}

impl ListMenu {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// Opens the menu on the row of the list chosen.
    pub fn open(&mut self, choices: &Choices, chosen: StarredList) {
        let row = choices
            .rows()
            .iter()
            .position(|&row| row == chosen)
            .unwrap_or(0);
        self.open = Some(Open { row, edit: None });
    }

    /// The list has its new name: the row shows it again.
    pub fn renamed(&mut self) {
        if let Some(open) = &mut self.open {
            open.edit = None;
        }
    }

    /// The facade refused the name: the menu says why under the field.
    pub fn refused(&mut self, refusal: String) {
        if let Some(Edit::Renaming { error, .. }) =
            self.open.as_mut().and_then(|open| open.edit.as_mut())
        {
            *error = Some(refusal);
        }
    }

    pub fn update(
        &mut self,
        message: Message,
        choices: &Choices,
    ) -> (Option<Action>, Task<Message>) {
        let rows = choices.rows();
        let Some(open) = self.open.as_mut() else {
            return (None, Task::none());
        };
        let none = (None, Task::none());
        let row = match (message, &mut open.edit) {
            (Message::Close, _) => {
                self.open = None;
                return none;
            }
            (Message::Escape | Message::Cancel, Some(_)) => {
                open.edit = None;
                return none;
            }
            (Message::Escape, None) => {
                self.open = None;
                return none;
            }
            (Message::Up, None | Some(Edit::Actions(_))) => {
                open.edit = None;
                open.row = open.row.saturating_sub(1);
                return none;
            }
            (Message::Down, None | Some(Edit::Actions(_))) => {
                open.edit = None;
                open.row = (open.row + 1).min(rows.len() - 1);
                return none;
            }
            (Message::Enter, None) => open.row,
            (Message::Click(row), _) if row < rows.len() => row,
            (Message::RightClick(row), _) => {
                if let Some(&StarredList::List(list)) = rows.get(row) {
                    open.row = row;
                    open.edit = Some(Edit::Actions(list));
                }
                return none;
            }
            (Message::Rename, Some(Edit::Actions(list))) => {
                let list = *list;
                let name = choices
                    .list(list)
                    .map_or_else(String::new, |list| list.name.clone());
                open.edit = Some(Edit::Renaming {
                    list,
                    name,
                    error: None,
                });
                return (None, operation::focus(FIELD));
            }
            (Message::Delete, Some(Edit::Actions(list))) => {
                let list = *list;
                if choices.list(list).is_some_and(|list| list.games > 0) {
                    open.edit = Some(Edit::Deleting(list));
                    return none;
                }
                self.open = None;
                return (Some(Action::Delete(list)), Task::none());
            }
            (Message::Confirm | Message::Enter, Some(Edit::Deleting(list))) => {
                let list = *list;
                self.open = None;
                return (Some(Action::Delete(list)), Task::none());
            }
            (Message::Name(new), Some(Edit::Renaming { name, error, .. })) => {
                *name = new;
                *error = None;
                return none;
            }
            (Message::Submit, Some(Edit::Renaming { list, name, .. })) => {
                let name = name.trim();
                return (
                    (!name.is_empty()).then(|| Action::Rename(*list, name.to_string())),
                    Task::none(),
                );
            }
            _ => return none,
        };

        self.open = None;
        (Some(Action::Choose(rows[row])), Task::none())
    }

    /// The segment, with the menu under it when it is open. `map` turns the menu's messages into
    /// the screen's.
    pub fn segment<'a, M: Clone + 'a>(
        &'a self,
        segment: Element<'a, M>,
        choices: &Choices,
        chosen: StarredList,
        map: impl Fn(Message) -> M + Clone + 'a,
    ) -> Element<'a, M> {
        let menu = self
            .open
            .as_ref()
            .map(|open| menu(open, choices, chosen).map(map.clone()));

        star_menu::anchored(segment, menu, map(Message::Close))
    }
}

fn menu<'a>(open: &Open, choices: &Choices, chosen: StarredList) -> Element<'a, Message> {
    let rows = choices.rows();
    let line = |index: usize, label: String, games: usize| -> Element<'a, Message> {
        let lit = open.edit.is_none() && open.row == index;
        let label = text(label)
            .size(font::BAR_SIZE)
            .wrapping(Wrapping::None)
            .font(if rows[index] == chosen {
                font::SEMIBOLD
            } else {
                font::REGULAR
            });
        button(
            row![
                container(label).width(Fill).clip(true),
                text(format::count(games as u32))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .spacing(8)
            .align_y(Center),
        )
        .width(Fill)
        .padding([6, 10])
        .style(style::list_row(lit))
        .on_press(Message::Click(index))
        .into()
    };

    let all = line(0, "All starred".to_string(), choices.all);
    let lists = choices.lists.iter().enumerate().map(|(place, list)| {
        let index = place + 1;
        match &open.edit {
            Some(Edit::Actions(id)) if *id == list.id => actions(&list.name),
            Some(Edit::Renaming {
                list: id,
                name,
                error,
            }) if *id == list.id => renaming(name, error.as_deref()),
            Some(Edit::Deleting(id)) if *id == list.id => deleting(list),
            _ => mouse_area(line(index, list.name.clone(), list.shown))
                .on_right_press(Message::RightClick(index))
                .into(),
        }
    });
    let no_list = choices
        .no_list
        .map(|games| line(rows.len() - 1, "No list".to_string(), games));

    let mut menu = column![all, style::separator()].spacing(4);
    if !choices.lists.is_empty() {
        menu = menu.push(column(lists).spacing(2));
    }
    if let Some(no_list) = no_list {
        menu = menu.push(style::separator()).push(no_list);
    }

    container(menu)
        .width(WIDTH)
        .padding(PADDING)
        .style(style::popup)
        .into()
}

fn small_button<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(font::NOTE_SIZE))
        .padding([4, 8])
        .style(style::text_button)
        .on_press(message)
        .into()
}

fn actions<'a>(name: &str) -> Element<'a, Message> {
    container(
        row![
            container(
                text(name.to_string())
                    .size(font::BAR_SIZE)
                    .wrapping(Wrapping::None)
            )
            .width(Fill)
            .clip(true),
            small_button("Rename", Message::Rename),
            small_button("Delete", Message::Delete),
        ]
        .spacing(4)
        .align_y(Center),
    )
    .padding([2, 10])
    .into()
}

fn renaming<'a>(name: &str, error: Option<&str>) -> Element<'a, Message> {
    column![
        text_input("List name", name)
            .id(FIELD)
            .on_input(Message::Name)
            .on_submit(Message::Submit)
            .size(font::BAR_SIZE)
            .padding([6, 10])
            .style(if error.is_some() {
                style::wrong_field
            } else {
                style::field
            }),
    ]
    .push(error.map(|error| {
        text(error.to_string())
            .size(font::NOTE_SIZE)
            .style(text::danger)
    }))
    .spacing(4)
    .into()
}

fn deleting<'a>(list: &ListChoice) -> Element<'a, Message> {
    container(
        column![
            text(format!("Delete {}?", list.name))
                .size(font::BAR_SIZE)
                .font(font::SEMIBOLD),
            text(keep_their_star(list.games))
                .size(font::NOTE_SIZE)
                .style(style::muted),
            row![
                button(text("Delete").size(font::NOTE_SIZE))
                    .padding([4, 12])
                    .style(style::danger)
                    .on_press(Message::Confirm),
                small_button("Cancel", Message::Cancel),
            ]
            .spacing(8)
            .align_y(Center),
        ]
        .spacing(6),
    )
    .padding([6, 10])
    .into()
}

fn keep_their_star(games: usize) -> String {
    if games == 1 {
        "Its game keeps its star".to_string()
    } else {
        format!("Its {} games keep their star", format::count(games as u32))
    }
}

/// The keys of the open menu. The page's own keys are quiet meanwhile. Esc is heard whatever took
/// it, since the field keeps it for itself; the rest only when nothing else did.
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
    match key {
        Key::Named(Named::ArrowUp) => Some(Message::Up),
        Key::Named(Named::ArrowDown) => Some(Message::Down),
        Key::Named(Named::Enter) => Some(Message::Enter),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key;

    use super::*;

    const CO_OP: ListId = ListId(2);
    const ON_SALE: ListId = ListId(5);

    fn choices() -> Choices {
        Choices {
            all: 6,
            lists: vec![
                ListChoice {
                    id: CO_OP,
                    name: "Co-op".to_string(),
                    shown: 2,
                    games: 3,
                },
                ListChoice {
                    id: ON_SALE,
                    name: "On sale".to_string(),
                    shown: 0,
                    games: 0,
                },
            ],
            no_list: Some(1),
        }
    }

    fn opened(chosen: StarredList) -> ListMenu {
        let mut menu = ListMenu::default();
        menu.open(&choices(), chosen);
        menu
    }

    fn press(menu: &mut ListMenu, message: Message) -> Option<Action> {
        menu.update(message, &choices()).0
    }

    fn row(menu: &ListMenu) -> usize {
        menu.open.as_ref().unwrap().row
    }

    fn edit(menu: &ListMenu) -> Option<&Edit> {
        menu.open.as_ref().unwrap().edit.as_ref()
    }

    #[test]
    fn the_rows_are_all_starred_the_lists_and_no_list() {
        assert_eq!(
            choices().rows(),
            [
                StarredList::All,
                StarredList::List(CO_OP),
                StarredList::List(ON_SALE),
                StarredList::NoList
            ]
        );
        let without = Choices {
            no_list: None,
            ..choices()
        };
        assert_eq!(without.rows().last(), Some(&StarredList::List(ON_SALE)));
    }

    #[test]
    fn the_menu_opens_on_the_list_chosen() {
        assert_eq!(row(&opened(StarredList::All)), 0);
        assert_eq!(row(&opened(StarredList::List(ON_SALE))), 2);
        assert_eq!(row(&opened(StarredList::NoList)), 3);
        assert_eq!(row(&opened(StarredList::List(ListId(9)))), 0);
    }

    #[test]
    fn the_arrows_stop_at_the_ends_and_enter_chooses() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::Up);
        assert_eq!(row(&menu), 0);
        for _ in 0..6 {
            press(&mut menu, Message::Down);
        }
        assert_eq!(row(&menu), 3);
        press(&mut menu, Message::Up);
        assert_eq!(
            press(&mut menu, Message::Enter),
            Some(Action::Choose(StarredList::List(ON_SALE)))
        );
        assert!(!menu.is_open());
    }

    #[test]
    fn a_click_chooses_and_esc_closes_without_choosing() {
        let mut menu = opened(StarredList::All);
        assert_eq!(
            press(&mut menu, Message::Click(3)),
            Some(Action::Choose(StarredList::NoList))
        );
        assert!(!menu.is_open());

        let mut menu = opened(StarredList::All);
        assert_eq!(press(&mut menu, Message::Escape), None);
        assert!(!menu.is_open());
    }

    #[test]
    fn a_right_click_on_a_list_gives_rename_and_delete() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(1));
        assert_eq!(edit(&menu), Some(&Edit::Actions(CO_OP)));
        assert_eq!(row(&menu), 1);

        // All starred and No list are not lists
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(0));
        press(&mut menu, Message::RightClick(3));
        assert_eq!(edit(&menu), None);
    }

    #[test]
    fn rename_puts_the_name_in_the_field_and_enter_saves_it() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(1));
        assert_eq!(press(&mut menu, Message::Rename), None);
        assert_eq!(
            edit(&menu),
            Some(&Edit::Renaming {
                list: CO_OP,
                name: "Co-op".to_string(),
                error: None
            })
        );
        // The arrows and the Enter of the page wait while the field is open
        press(&mut menu, Message::Down);
        assert_eq!(press(&mut menu, Message::Enter), None);
        press(&mut menu, Message::Name("  Co-op with Marco ".to_string()));
        assert_eq!(
            press(&mut menu, Message::Submit),
            Some(Action::Rename(CO_OP, "Co-op with Marco".to_string()))
        );
        assert!(menu.is_open());
        menu.renamed();
        assert_eq!(edit(&menu), None);
    }

    #[test]
    fn an_empty_name_renames_nothing() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(1));
        press(&mut menu, Message::Rename);
        press(&mut menu, Message::Name("  ".to_string()));
        assert_eq!(press(&mut menu, Message::Submit), None);
    }

    #[test]
    fn a_refused_name_shows_under_the_field_until_it_changes() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(1));
        press(&mut menu, Message::Rename);
        menu.refused("A list with this name already exists".to_string());
        let error = |menu: &ListMenu| match edit(menu) {
            Some(Edit::Renaming { error, .. }) => error.clone(),
            _ => panic!("not renaming"),
        };
        assert_eq!(
            error(&menu).as_deref(),
            Some("A list with this name already exists")
        );
        press(&mut menu, Message::Name("On sale 2".to_string()));
        assert_eq!(error(&menu), None);
    }

    #[test]
    fn esc_in_the_field_cancels_the_rename_and_esc_again_closes() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(1));
        press(&mut menu, Message::Rename);
        press(&mut menu, Message::Name("Something else".to_string()));
        assert_eq!(press(&mut menu, Message::Escape), None);
        assert!(menu.is_open());
        assert_eq!(edit(&menu), None);
        press(&mut menu, Message::Escape);
        assert!(!menu.is_open());
    }

    #[test]
    fn delete_asks_first_when_the_list_has_games() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(1));
        assert_eq!(press(&mut menu, Message::Delete), None);
        assert_eq!(edit(&menu), Some(&Edit::Deleting(CO_OP)));
        assert_eq!(press(&mut menu, Message::Cancel), None);
        assert_eq!(edit(&menu), None);

        press(&mut menu, Message::RightClick(1));
        press(&mut menu, Message::Delete);
        assert_eq!(
            press(&mut menu, Message::Confirm),
            Some(Action::Delete(CO_OP))
        );
        assert!(!menu.is_open());

        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(1));
        press(&mut menu, Message::Delete);
        assert_eq!(
            press(&mut menu, Message::Enter),
            Some(Action::Delete(CO_OP))
        );
    }

    #[test]
    fn an_empty_list_goes_without_asking() {
        let mut menu = opened(StarredList::All);
        press(&mut menu, Message::RightClick(2));
        assert_eq!(
            press(&mut menu, Message::Delete),
            Some(Action::Delete(ON_SALE))
        );
        assert!(!menu.is_open());
    }

    #[test]
    fn the_confirmation_counts_the_games() {
        assert_eq!(keep_their_star(4), "Its 4 games keep their star");
        assert_eq!(keep_their_star(1), "Its game keeps its star");
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
    fn the_keys_of_the_menu() {
        let ignored = event::Status::Ignored;
        assert!(matches!(
            pressed(Key::Named(Named::ArrowUp), ignored),
            Some(Message::Up)
        ));
        assert!(matches!(
            pressed(Key::Named(Named::ArrowDown), ignored),
            Some(Message::Down)
        ));
        assert!(matches!(
            pressed(Key::Named(Named::Enter), ignored),
            Some(Message::Enter)
        ));
        assert!(matches!(
            pressed(Key::Named(Named::Escape), event::Status::Captured),
            Some(Message::Escape)
        ));
        assert!(pressed(Key::Named(Named::Enter), event::Status::Captured).is_none());
        // The keys of the page are quiet while the menu is open
        for key in ["s", "1", "j", "/"] {
            assert!(pressed(Key::Character(key.into()), ignored).is_none());
        }
    }
}
