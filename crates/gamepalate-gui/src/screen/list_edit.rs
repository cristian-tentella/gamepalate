use gamepalate_app::{ListId, Lists};
use iced::{
    Center, Element, Event, Fill, Subscription, Task, event,
    keyboard::{self, Key, key::Named},
    widget::{Id, button, column, container, operation, row, text, text_input},
    window,
};

use super::save_menu;
use crate::{font, format, style};

const WIDTH: f32 = 240.0;
const PADDING: f32 = 6.0;
const FIELD: Id = Id::new("list-edit-name");

/// What a right click on the title of a section of Saved gives: Rename and Delete, then the field
/// of the new name or the question before deleting.
#[derive(Default)]
pub struct ListEdit {
    open: Option<(ListId, Edit)>,
}

#[derive(Debug, PartialEq)]
enum Edit {
    Actions,
    Renaming { name: String, error: Option<String> },
    Deleting,
}

#[derive(Clone, Debug)]
pub enum Message {
    Open(ListId),
    Close,
    Escape,
    Enter,
    Rename,
    Delete,
    Name(String),
    Submit,
    Confirm,
}

/// What Votes does with the list.
#[derive(Debug, PartialEq)]
pub enum Action {
    Rename(ListId, String),
    Delete(ListId),
}

impl ListEdit {
    pub fn is_open(&self) -> bool {
        self.open.is_some()
    }

    /// The list has its new name: the popup closes.
    pub fn renamed(&mut self) {
        self.open = None;
    }

    /// The facade refused the name: the popup says why under the field.
    pub fn refused(&mut self, refusal: String) {
        if let Some((_, Edit::Renaming { error, .. })) = &mut self.open {
            *error = Some(refusal);
        }
    }

    /// `games` counts all the games in a list, whatever Votes shows.
    pub fn update(
        &mut self,
        message: Message,
        lists: &Lists,
        games: impl Fn(ListId) -> usize,
    ) -> (Option<Action>, Task<Message>) {
        let none = (None, Task::none());
        if let Message::Open(list) = message {
            if lists.contains_key(&list) {
                self.open = Some((list, Edit::Actions));
            }
            return none;
        }
        let Some((list, edit)) = self.open.as_mut() else {
            return none;
        };
        let list = *list;
        match (message, edit) {
            (Message::Close, _) | (Message::Escape, Edit::Actions) => self.open = None,
            (Message::Escape, edit) => *edit = Edit::Actions,
            (Message::Rename, edit @ Edit::Actions) => {
                *edit = Edit::Renaming {
                    name: lists.get(&list).cloned().unwrap_or_default(),
                    error: None,
                };
                return (None, operation::focus(FIELD));
            }
            (Message::Delete, edit @ Edit::Actions) => {
                if games(list) > 0 {
                    *edit = Edit::Deleting;
                } else {
                    self.open = None;
                    return (Some(Action::Delete(list)), Task::none());
                }
            }
            (Message::Confirm | Message::Enter, Edit::Deleting) => {
                self.open = None;
                return (Some(Action::Delete(list)), Task::none());
            }
            (Message::Name(new), Edit::Renaming { name, error }) => {
                *name = new;
                *error = None;
            }
            (Message::Submit, Edit::Renaming { name, .. }) => {
                let name = name.trim();
                return (
                    (!name.is_empty()).then(|| Action::Rename(list, name.to_string())),
                    Task::none(),
                );
            }
            _ => {}
        }
        none
    }

    /// `title`, with the popup under it while it is open on `list`. `map` turns the popup's
    /// messages into the screen's.
    pub fn title<'a, M: Clone + 'a>(
        &'a self,
        title: Element<'a, M>,
        list: ListId,
        name: &'a str,
        games: usize,
        map: impl Fn(Message) -> M + Clone + 'a,
    ) -> Element<'a, M> {
        let popup = self
            .open
            .as_ref()
            .filter(|(open, _)| *open == list)
            .map(|(_, edit)| popup(edit, name, games).map(map.clone()));

        save_menu::anchored(title, popup, map(Message::Close))
    }
}

fn popup<'a>(edit: &'a Edit, name: &'a str, games: usize) -> Element<'a, Message> {
    let content: Element<'a, Message> = match edit {
        Edit::Actions => column![
            line("Rename", Message::Rename),
            line("Delete", Message::Delete)
        ]
        .spacing(2)
        .into(),
        Edit::Renaming { name, error } => renaming(name, error.as_deref()),
        Edit::Deleting => deleting(name, games),
    };

    container(content)
        .width(WIDTH)
        .padding(PADDING)
        .style(style::popup)
        .into()
}

fn line<'a>(label: &'a str, message: Message) -> Element<'a, Message> {
    button(text(label).size(font::BAR_SIZE))
        .width(Fill)
        .padding([6, 10])
        .style(style::list_row(false))
        .on_press(message)
        .into()
}

fn renaming<'a>(name: &'a str, error: Option<&'a str>) -> Element<'a, Message> {
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
    .push(error.map(|error| text(error).size(font::NOTE_SIZE).style(text::danger)))
    .spacing(4)
    .into()
}

fn deleting<'a>(name: &str, games: usize) -> Element<'a, Message> {
    container(
        column![
            text(format!("Delete {name}?"))
                .size(font::BAR_SIZE)
                .font(font::SEMIBOLD),
            text(stay_saved(games))
                .size(font::NOTE_SIZE)
                .style(style::muted),
            row![
                button(text("Delete").size(font::NOTE_SIZE))
                    .padding([4, 12])
                    .style(style::danger)
                    .on_press(Message::Confirm),
                button(text("Cancel").size(font::NOTE_SIZE))
                    .padding([4, 8])
                    .style(style::text_button)
                    .on_press(Message::Escape),
            ]
            .spacing(8)
            .align_y(Center),
        ]
        .spacing(6),
    )
    .padding([6, 10])
    .into()
}

fn stay_saved(games: usize) -> String {
    if games == 1 {
        "Its game stays saved".to_string()
    } else {
        format!("Its {} games stay saved", format::count(games as u32))
    }
}

/// The keys of the open popup. The page's own keys are quiet meanwhile. Esc is heard whatever
/// took it, since the field keeps it for itself; Enter only when nothing else did.
pub fn keys() -> Subscription<Message> {
    event::listen_with(edit_key)
}

fn edit_key(event: Event, status: event::Status, _: window::Id) -> Option<Message> {
    let Event::Keyboard(keyboard::Event::KeyPressed { key, modifiers, .. }) = event else {
        return None;
    };
    if key == Key::Named(Named::Escape) {
        return Some(Message::Escape);
    }
    if status == event::Status::Captured || modifiers.command() {
        return None;
    }
    (key == Key::Named(Named::Enter)).then_some(Message::Enter)
}

#[cfg(test)]
mod tests {
    use iced::keyboard::key;

    use super::*;

    const CO_OP: ListId = ListId(2);
    const ON_SALE: ListId = ListId(5);

    fn lists() -> Lists {
        Lists::from([
            (CO_OP, "Co-op".to_string()),
            (ON_SALE, "On sale".to_string()),
        ])
    }

    fn games(list: ListId) -> usize {
        if list == CO_OP { 3 } else { 0 }
    }

    fn press(edit: &mut ListEdit, message: Message) -> Option<Action> {
        edit.update(message, &lists(), games).0
    }

    fn opened(list: ListId) -> ListEdit {
        let mut edit = ListEdit::default();
        press(&mut edit, Message::Open(list));
        edit
    }

    fn edit(edit: &ListEdit) -> &Edit {
        &edit.open.as_ref().unwrap().1
    }

    #[test]
    fn a_right_click_gives_rename_and_delete_on_a_list_that_is_there() {
        assert_eq!(edit(&opened(CO_OP)), &Edit::Actions);
        assert_eq!(
            opened(CO_OP).open.as_ref().map(|(list, _)| *list),
            Some(CO_OP)
        );
        assert!(!opened(ListId(9)).is_open());
    }

    #[test]
    fn rename_puts_the_name_in_the_field_and_enter_saves_it() {
        let mut list_edit = opened(CO_OP);
        press(&mut list_edit, Message::Rename);
        assert_eq!(
            edit(&list_edit),
            &Edit::Renaming {
                name: "Co-op".to_string(),
                error: None
            }
        );

        press(&mut list_edit, Message::Name("  Co-op games ".to_string()));
        assert_eq!(
            press(&mut list_edit, Message::Submit),
            Some(Action::Rename(CO_OP, "Co-op games".to_string()))
        );
        assert!(list_edit.is_open());
        list_edit.renamed();
        assert!(!list_edit.is_open());
    }

    #[test]
    fn an_empty_name_renames_nothing() {
        let mut list_edit = opened(CO_OP);
        press(&mut list_edit, Message::Rename);
        press(&mut list_edit, Message::Name("  ".to_string()));
        assert_eq!(press(&mut list_edit, Message::Submit), None);
    }

    #[test]
    fn a_refused_name_shows_under_the_field_until_it_changes() {
        let mut list_edit = opened(CO_OP);
        press(&mut list_edit, Message::Rename);
        list_edit.refused("A list with this name already exists".to_string());
        assert!(matches!(
            edit(&list_edit),
            Edit::Renaming { error: Some(_), .. }
        ));

        press(&mut list_edit, Message::Name("Co-op!".to_string()));
        assert!(matches!(
            edit(&list_edit),
            Edit::Renaming { error: None, .. }
        ));
    }

    #[test]
    fn esc_goes_back_to_rename_and_delete_and_then_closes() {
        let mut list_edit = opened(CO_OP);
        press(&mut list_edit, Message::Rename);
        press(&mut list_edit, Message::Escape);
        assert_eq!(edit(&list_edit), &Edit::Actions);

        press(&mut list_edit, Message::Escape);
        assert!(!list_edit.is_open());
    }

    #[test]
    fn delete_asks_first_when_the_list_has_games() {
        let mut list_edit = opened(CO_OP);
        assert_eq!(press(&mut list_edit, Message::Delete), None);
        assert_eq!(edit(&list_edit), &Edit::Deleting);

        press(&mut list_edit, Message::Escape);
        assert_eq!(edit(&list_edit), &Edit::Actions);

        press(&mut list_edit, Message::Delete);
        assert_eq!(
            press(&mut list_edit, Message::Enter),
            Some(Action::Delete(CO_OP))
        );
        assert!(!list_edit.is_open());

        let mut list_edit = opened(CO_OP);
        press(&mut list_edit, Message::Delete);
        assert_eq!(
            press(&mut list_edit, Message::Confirm),
            Some(Action::Delete(CO_OP))
        );
    }

    #[test]
    fn an_empty_list_goes_without_asking() {
        let mut list_edit = opened(ON_SALE);
        assert_eq!(
            press(&mut list_edit, Message::Delete),
            Some(Action::Delete(ON_SALE))
        );
        assert!(!list_edit.is_open());
    }

    #[test]
    fn a_click_outside_closes() {
        let mut list_edit = opened(CO_OP);
        press(&mut list_edit, Message::Rename);
        press(&mut list_edit, Message::Close);
        assert!(!list_edit.is_open());
    }

    #[test]
    fn the_confirmation_counts_the_games() {
        assert_eq!(stay_saved(4), "Its 4 games stay saved");
        assert_eq!(stay_saved(1), "Its game stays saved");
    }

    fn pressed(key: Key, status: event::Status) -> Option<Message> {
        edit_key(
            Event::Keyboard(keyboard::Event::KeyPressed {
                key: key.clone(),
                modified_key: key,
                physical_key: key::Physical::Unidentified(key::NativeCode::Unidentified),
                location: keyboard::Location::Standard,
                modifiers: keyboard::Modifiers::default(),
                text: None,
                repeat: false,
            }),
            status,
            window::Id::unique(),
        )
    }

    #[test]
    fn the_keys_of_the_popup() {
        let ignored = event::Status::Ignored;
        assert!(matches!(
            pressed(Key::Named(Named::Enter), ignored),
            Some(Message::Enter)
        ));
        assert!(pressed(Key::Named(Named::Enter), event::Status::Captured).is_none());
        assert!(matches!(
            pressed(Key::Named(Named::Escape), event::Status::Captured),
            Some(Message::Escape)
        ));
        assert!(pressed(Key::Named(Named::ArrowDown), ignored).is_none());
    }
}
