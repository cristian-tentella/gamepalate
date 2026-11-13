mod corrections;
mod families;
mod taste;

use std::{path::PathBuf, sync::Arc};

use gamepalate_app::{
    AppId, FamiliesChanged, FamiliesRemade, FamiliesUndo, FamilyChange, FamilyGame, Gamepalate,
    ProfileView, RemakeChanges, RemakePreview, VotesView,
};
use iced::{
    Center, Element, Fill, Subscription, Task,
    keyboard::{self, Key, key::Named},
    widget::{column, container, progress_bar, responsive, row, scrollable, stack, text},
};

use super::{
    Toasts, library, sentence,
    showcase::{self, Showcase},
    votes::Filter,
};
use crate::{font, style};
use families::{
    FAMILY_HEADERS, NO_NEW_FAMILIES, already_many, cover_width, families_text, remade_text,
    remake_dialog,
};
use taste::no_taste_text;

const STACK_WIDTH: f32 = 1100.0;
const NO_VOTES: &str = "No votes yet. Vote in Discover with keys 1–4.";
const LABEL_BAR_WIDTH: f32 = 120.0;
const COLUMN_WIDTH: f32 = 760.0;
const SIDE_WIDTH: f32 = 380.0;
const COLUMN_GAP: f32 = 32.0;
const PAGE_WIDTH: f32 = COLUMN_WIDTH + COLUMN_GAP + SIDE_WIDTH;
const PAGE_PADDING: f32 = 24.0;
const CLOSED: char = '\u{E5CC}';
const OPENED: char = '\u{E5CF}';
pub(crate) const HELD_OUT_TIP: &str =
    "Games you like, kept out of families on purpose to see if Discover finds them";

pub struct Profile {
    gamepalate: Arc<Gamepalate>,
    data_dir: PathBuf,
    view: ProfileView,
    open_correction: Option<Correction>,
    all_tags: bool,
    showcase: Showcase,
    editing: Option<Editing>,
    remake: Remake,
    remake_changes: Option<RemakeChanges>,
    new_games: Option<usize>,
    undo: Option<FamiliesUndo>,
    toasts: Toasts,
}

#[derive(Clone, Copy, Debug, PartialEq)]
enum Remake {
    Idle,
    Planning,
    Asking,
    Working,
}

impl Remake {
    fn after(self, message: &Message) -> Remake {
        match (self, message) {
            (Remake::Idle, Message::AskRemake) => Remake::Planning,
            (Remake::Planning, Message::Planned(RemakePreview::Changes(_))) => Remake::Asking,
            (Remake::Planning, Message::Planned(_)) => Remake::Idle,
            (Remake::Asking, Message::ConfirmRemake) => Remake::Working,
            (Remake::Asking, Message::CancelRemake | Message::CloseEditor) => Remake::Idle,
            (Remake::Working, Message::Remade(_)) => Remake::Idle,
            (state, _) => state,
        }
    }
}

struct Editing {
    family: String,
    name: String,
    games: Vec<FamilyGame>,
}

#[derive(Clone, Debug)]
pub enum Message {
    ToggleAllTags,
    DiscoverFamily(String),
    OpenVotes(Filter),
    OpenInLibrary(AppId),
    OpenLibrary(library::Filter, library::Sort),
    ToggleCorrection(Correction),
    OpenFamilies,
    Showcase(showcase::Message),
    EditFamily(String),
    EditName(String),
    RenameFamily,
    DoneEditing,
    CloseEditor,
    AskRemake,
    Planned(RemakePreview),
    ConfirmRemake,
    CancelRemake,
    Remade(Result<FamiliesRemade, String>),
    ChangeFamilies(FamilyChange),
    FamiliesSaved(FamilyChange, Result<FamiliesChanged, String>),
    Undo,
    Undone(Result<(), String>),
    HideToast(u64),
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Correction {
    Games,
    Hidden,
}

impl Profile {
    pub fn new(gamepalate: &Arc<Gamepalate>) -> (Profile, Task<Message>) {
        let mut profile = Profile {
            gamepalate: Arc::clone(gamepalate),
            data_dir: gamepalate.data_dir().to_path_buf(),
            view: gamepalate.profile_view(),
            open_correction: None,
            all_tags: false,
            showcase: Showcase::new(gamepalate),
            editing: None,
            remake: Remake::Idle,
            remake_changes: None,
            new_games: gamepalate.new_played_games(),
            undo: None,
            toasts: Toasts::default(),
        };
        let task = profile.load();

        (profile, task)
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        let task = self.handle(message);

        Task::batch([task, self.load()])
    }

    fn handle(&mut self, message: Message) -> Task<Message> {
        let before = self.remake;
        self.remake = before.after(&message);
        match message {
            Message::ToggleAllTags => self.all_tags = !self.all_tags,
            Message::DiscoverFamily(_)
            | Message::OpenVotes(_)
            | Message::OpenInLibrary(_)
            | Message::OpenLibrary(..) => {}
            Message::ToggleCorrection(correction) => {
                self.open_correction =
                    (self.open_correction != Some(correction)).then_some(correction);
            }
            Message::OpenFamilies => super::open_families(&self.data_dir),
            Message::Showcase(message) => {
                return self.showcase.update(message).map(Message::Showcase);
            }
            Message::EditFamily(family) => {
                self.editing = Some(Editing {
                    games: self.gamepalate.family_games(&family),
                    name: family.clone(),
                    family,
                });
            }
            Message::EditName(name) => {
                if let Some(editing) = &mut self.editing {
                    editing.name = name;
                }
            }
            Message::RenameFamily => return self.rename(),
            Message::DoneEditing => {
                let task = self.rename();
                self.editing = None;
                return task;
            }
            Message::CloseEditor => self.editing = None,
            Message::AskRemake => {
                if before == Remake::Idle {
                    return self.preview_remake();
                }
            }
            Message::Planned(RemakePreview::Changes(changes)) => {
                self.remake_changes = Some(changes);
            }
            Message::Planned(RemakePreview::NoNewFamilies) => {
                return self
                    .toasts
                    .show(NO_NEW_FAMILIES.to_string(), false, Message::HideToast);
            }
            Message::Planned(RemakePreview::AlreadyMany) => {
                return self.toasts.show(already_many(), false, Message::HideToast);
            }
            Message::CancelRemake => self.remake_changes = None,
            Message::ConfirmRemake => {
                if before == Remake::Asking {
                    self.remake_changes = None;
                    return self.remake_families();
                }
            }
            Message::Remade(Ok(remade)) => {
                let text = remade_text(&remade.outcome);
                let undoable = remade.undo.is_some();
                if let Some(undo) = remade.undo {
                    self.undo = Some(undo);
                }
                self.reload();
                return self.toasts.show(text, undoable, Message::HideToast);
            }
            Message::Remade(Err(error)) => {
                return self.toasts.show(error, false, Message::HideToast);
            }
            Message::ChangeFamilies(change) => return self.change_families(change),
            Message::FamiliesSaved(change, Ok(changed)) => {
                let games = self
                    .editing
                    .as_ref()
                    .map_or(&[][..], |editing| &editing.games);
                let text = families_text(&change, games, &changed.gone);
                if let (FamilyChange::Rename { to, .. }, Some(editing)) =
                    (&change, &mut self.editing)
                {
                    editing.family = to.trim().to_string();
                    editing.name = editing.family.clone();
                }
                self.undo = Some(changed.undo);
                self.reload();
                return self.toasts.show(text, true, Message::HideToast);
            }
            Message::FamiliesSaved(_, Err(error)) | Message::Undone(Err(error)) => {
                return self.toasts.show(error, false, Message::HideToast);
            }
            Message::Undo => {
                if let Some(undo) = self.undo.take() {
                    let gamepalate = Arc::clone(&self.gamepalate);
                    return Task::perform(
                        async move {
                            gamepalate
                                .undo_families(&undo)
                                .map_err(|error| sentence(&error.to_string()))
                        },
                        Message::Undone,
                    );
                }
            }
            Message::Undone(Ok(())) => {
                self.reload();
                return self
                    .toasts
                    .show("Undone".to_string(), false, Message::HideToast);
            }
            Message::HideToast(id) => self.toasts.hide(id),
        }

        Task::none()
    }

    fn reload(&mut self) {
        self.view = self.gamepalate.profile_view();
        self.new_games = self.gamepalate.new_played_games();
        let names = self.gamepalate.family_names();
        self.editing = self
            .editing
            .take()
            .filter(|editing| names.contains(&editing.family))
            .map(|mut editing| {
                editing.games = self.gamepalate.family_games(&editing.family);
                editing
            });
    }

    fn load(&mut self) -> Task<Message> {
        let ids: Vec<AppId> = self
            .view
            .taste
            .iter()
            .flat_map(|taste| &taste.shaping_games)
            .map(|game| game.id)
            .chain(
                self.view
                    .families
                    .iter()
                    .flat_map(|family| family.games.iter().take(FAMILY_HEADERS).copied()),
            )
            .chain(
                self.editing
                    .iter()
                    .flat_map(|editing| editing.games.iter().map(|game| game.id)),
            )
            .collect();

        self.showcase.load_headers(&ids).map(Message::Showcase)
    }

    pub fn subscription(keys: bool) -> Subscription<Message> {
        if !keys {
            return Subscription::none();
        }
        keyboard::listen().filter_map(key_message)
    }

    pub fn view(&self) -> Element<'_, Message> {
        responsive(move |size| self.layout(size.width)).into()
    }

    fn layout(&self, width: f32) -> Element<'_, Message> {
        let taste = match &self.view.taste {
            Some(taste) => self.taste(taste),
            None => text(no_taste_text(self.view.hours.min_hours))
                .size(font::BAR_SIZE)
                .into(),
        };

        let taste = column![section_title("Your taste"), taste]
            .spacing(14)
            .max_width(COLUMN_WIDTH);

        let side_width = if stacked(width) {
            width.min(PAGE_WIDTH + 2.0 * PAGE_PADDING) - 2.0 * PAGE_PADDING
        } else {
            SIDE_WIDTH
        };
        let side = column![self.families(cover_width(side_width))]
            .push(self.votes())
            .push((!self.view.corrections.is_empty()).then(|| self.corrections()))
            .spacing(32);

        let columns: Element<'_, Message> = if stacked(width) {
            column![taste, side].spacing(32).into()
        } else {
            row![
                container(taste).width(Fill),
                container(side).width(SIDE_WIDTH),
            ]
            .spacing(COLUMN_GAP)
            .into()
        };

        let page = scrollable(
            container(container(columns).width(Fill).max_width(PAGE_WIDTH))
                .padding(PAGE_PADDING)
                .center_x(Fill),
        )
        .height(Fill);

        stack![page]
            .push(self.editing.as_ref().map(|editing| self.editor(editing)))
            .push(
                self.remake_changes
                    .as_ref()
                    .filter(|_| self.remake == Remake::Asking)
                    .map(remake_dialog),
            )
            .push(self.toasts.view(Message::Undo))
            .into()
    }
}

impl Profile {
    fn votes(&self) -> Option<Element<'_, Message>> {
        let votes = &self.view.votes;
        if votes.labels.enough() {
            return None;
        }

        let total = votes.would_try + votes.not_for_me;
        let body: Element<'_, Message> = if total == 0 {
            text(NO_VOTES).size(font::BAR_SIZE).into()
        } else {
            let needed = votes.min_labels;
            column![
                text(opinion_progress(votes))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                label_bar("Would try", votes.labels.would_try, needed),
                label_bar("Not for me", votes.labels.not_for_me, needed),
            ]
            .spacing(6)
            .into()
        };

        Some(column![section_title("Votes"), body].spacing(14).into())
    }
}

fn label_bar<'a>(label: &'a str, count: usize, needed: usize) -> Element<'a, Message> {
    row![
        text(label).size(font::NOTE_SIZE).width(80),
        progress_bar(0.0..=needed as f32, count.min(needed) as f32)
            .girth(4)
            .length(LABEL_BAR_WIDTH)
            .style(style::meter),
        text!("{count} of {needed}").size(font::NOTE_SIZE),
    ]
    .spacing(8)
    .align_y(Center)
    .into()
}

fn opinion_progress(votes: &VotesView) -> String {
    format!(
        "Votes count from {n} Would try and {n} Not this game or Not my genre. You have {} and {}.",
        votes.labels.would_try,
        votes.labels.not_for_me,
        n = votes.min_labels
    )
}

fn key_message(event: keyboard::Event) -> Option<Message> {
    let keyboard::Event::KeyPressed { key, modifiers, .. } = event else {
        return None;
    };
    if modifiers.command() {
        return matches!(key.as_ref(), Key::Character("z" | "Z")).then_some(Message::Undo);
    }

    match key {
        Key::Named(Named::Escape) => Some(Message::CloseEditor),
        _ => None,
    }
}

fn stacked(width: f32) -> bool {
    width < STACK_WIDTH
}

fn section_title(title: &str) -> Element<'_, Message> {
    text(title)
        .size(font::PRICE_SIZE)
        .font(font::SEMIBOLD)
        .into()
}

#[cfg(test)]
mod fixtures {
    use super::*;

    pub(super) fn changes() -> RemakeChanges {
        RemakeChanges {
            kept: vec!["Mystery".to_string(), "JRPG".to_string()],
            made_again: Vec::new(),
            removed: vec!["Puzzle · Mystery".to_string()],
            added: vec!["Roguelike".to_string()],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::fixtures::*;
    use super::*;

    #[test]
    fn the_remake_asks_first_and_cancel_changes_nothing() {
        let planning = Remake::Idle.after(&Message::AskRemake);
        assert_eq!(planning, Remake::Planning);
        let asking = planning.after(&Message::Planned(RemakePreview::Changes(changes())));
        assert_eq!(asking, Remake::Asking);

        assert_eq!(asking.after(&Message::CancelRemake), Remake::Idle);
        assert_eq!(asking.after(&Message::CloseEditor), Remake::Idle);
        assert_eq!(asking.after(&Message::Undo), Remake::Asking);
        assert_eq!(Remake::Idle.after(&Message::ConfirmRemake), Remake::Idle);
    }

    #[test]
    fn nothing_to_change_asks_nothing() {
        let planning = Remake::Idle.after(&Message::AskRemake);

        assert_eq!(
            planning.after(&Message::Planned(RemakePreview::NoNewFamilies)),
            Remake::Idle
        );
        assert_eq!(
            planning.after(&Message::Planned(RemakePreview::AlreadyMany)),
            Remake::Idle
        );
    }

    #[test]
    fn the_remake_works_until_it_is_done_and_asks_once() {
        let working = Remake::Asking.after(&Message::ConfirmRemake);
        assert_eq!(working, Remake::Working);

        assert_eq!(working.after(&Message::AskRemake), Remake::Working);
        assert_eq!(working.after(&Message::CancelRemake), Remake::Working);
        assert_eq!(
            working.after(&Message::Remade(Err("failed".to_string()))),
            Remake::Idle
        );
    }

    fn labels(would_try: usize, not_for_me: usize) -> VotesView {
        VotesView {
            would_try: 0,
            later: 0,
            not_for_me: 0,
            labels: gamepalate_app::Labels {
                would_try,
                not_for_me,
            },
            min_labels: 10,
        }
    }

    #[test]
    fn the_opinion_says_how_many_votes_are_missing() {
        assert_eq!(
            opinion_progress(&labels(4, 7)),
            "Votes count from 10 Would try and 10 Not this game or Not my genre. You have 4 and 7."
        );
    }

    fn pressed(key: Key, modifiers: keyboard::Modifiers) -> keyboard::Event {
        keyboard::Event::KeyPressed {
            key: key.clone(),
            modified_key: key,
            physical_key: keyboard::key::Physical::Unidentified(
                keyboard::key::NativeCode::Unidentified,
            ),
            location: keyboard::Location::Standard,
            modifiers,
            text: None,
            repeat: false,
        }
    }

    #[test]
    fn the_keys_undo_and_close() {
        let key = |named| key_message(pressed(Key::Named(named), keyboard::Modifiers::empty()));

        assert!(matches!(key(Named::Escape), Some(Message::CloseEditor)));
        assert!(key(Named::ArrowDown).is_none());
        assert!(key(Named::Enter).is_none());
        assert!(
            key_message(pressed(
                Key::Named(Named::ArrowDown),
                keyboard::Modifiers::CTRL
            ))
            .is_none()
        );
        assert!(matches!(
            key_message(pressed(
                Key::Character("z".into()),
                keyboard::Modifiers::CTRL
            )),
            Some(Message::Undo)
        ));
    }

    #[test]
    fn a_narrow_window_puts_the_columns_one_under_the_other() {
        assert!(stacked(1000.0));
        assert!(!stacked(1100.0));
        assert!(!stacked(1280.0));
    }
}
