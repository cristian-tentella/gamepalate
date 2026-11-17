use gamepalate_app::{SteamId, steam_client::ClientAccount};
use iced::{
    Border, Center, ContentFit, Element, Fill, Theme,
    widget::{button, column, image, row, space, text},
};

use crate::{font, style};

const ROW_HEIGHT: f32 = 64.0;
const AVATAR_SIZE: f32 = 40.0;
const AVATAR_GLYPH: char = '\u{E853}';
const NO_NAME: &str = "Steam account";
const SIGNED_IN_LAST: &str = "Signed in last";
pub(crate) const USE_ANOTHER: &str = "Use another account";
pub(crate) const BACK_TO_LIST: &str = "Back to the accounts on this PC";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Message {
    Select(SteamId),
    UseAnother,
    BackToList,
}

#[derive(Debug)]
struct Row {
    account: ClientAccount,
    avatar: Option<image::Handle>,
}

#[derive(Debug, Default)]
pub(crate) struct Accounts {
    rows: Vec<Row>,
    selected: Option<SteamId>,
    another: bool,
}

impl Accounts {
    pub(crate) fn new(list: Vec<ClientAccount>, preferred: Option<SteamId>) -> Accounts {
        let rows: Vec<Row> = list
            .into_iter()
            .map(|account| Row {
                avatar: account.avatar.as_deref().map(image::Handle::from_path),
                account,
            })
            .collect();
        let known = |id: SteamId| rows.iter().any(|row| row.account.steam_id == id);
        let (selected, another) = match preferred {
            Some(id) if known(id) => (Some(id), false),
            Some(_) if !rows.is_empty() => (None, true),
            _ => (rows.first().map(|row| row.account.steam_id), false),
        };

        Accounts {
            rows,
            selected,
            another,
        }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.rows.is_empty()
    }

    pub(crate) fn listed(&self) -> bool {
        !self.rows.is_empty() && !self.another
    }

    pub(crate) fn selected(&self) -> Option<SteamId> {
        self.selected.filter(|_| self.listed())
    }

    pub(crate) fn update(&mut self, message: Message) {
        match message {
            Message::Select(id) if self.rows.iter().any(|row| row.account.steam_id == id) => {
                self.selected = Some(id);
            }
            Message::Select(_) => {}
            Message::UseAnother if !self.rows.is_empty() => self.another = true,
            Message::UseAnother => {}
            Message::BackToList => {
                self.another = false;
                if self.selected.is_none() {
                    self.selected = self.rows.first().map(|row| row.account.steam_id);
                }
            }
        }
    }

    pub(crate) fn view<'a, M: Clone + 'a>(&'a self, map: fn(Message) -> M) -> Element<'a, M> {
        let last = self.rows.iter().position(|row| row.account.most_recent);
        let rows = self.rows.iter().enumerate().map(|(index, row)| {
            let id = row.account.steam_id;
            row_view(
                row,
                last == Some(index),
                self.selected == Some(id),
                map(Message::Select(id)),
            )
        });

        column![
            column(rows).spacing(8),
            button(text(USE_ANOTHER).size(font::BAR_SIZE))
                .padding(0)
                .style(style::link)
                .on_press(map(Message::UseAnother)),
        ]
        .spacing(16)
        .into()
    }

    pub(crate) fn back_link<'a, M: Clone + 'a>(map: fn(Message) -> M) -> Element<'a, M> {
        button(text(BACK_TO_LIST).size(font::NOTE_SIZE))
            .padding(0)
            .style(style::link)
            .on_press(map(Message::BackToList))
            .into()
    }
}

fn row_view<'a, M: Clone + 'a>(
    row_of: &'a Row,
    signed_in_last: bool,
    selected: bool,
    message: M,
) -> Element<'a, M> {
    let avatar: Element<'a, M> = match &row_of.avatar {
        Some(handle) => image(handle.clone())
            .width(AVATAR_SIZE)
            .height(AVATAR_SIZE)
            .content_fit(ContentFit::Cover)
            .border_radius(AVATAR_SIZE / 2.0)
            .into(),
        None => text(AVATAR_GLYPH)
            .font(font::ICONS)
            .size(AVATAR_SIZE)
            .width(AVATAR_SIZE)
            .height(AVATAR_SIZE)
            .center()
            .style(style::muted)
            .into(),
    };
    let check = selected.then(|| {
        text(font::CHECK)
            .font(font::ICONS)
            .size(font::icon(font::INFO_SIZE))
    });

    button(
        row![
            avatar,
            column![
                text(shown_name(&row_of.account))
                    .size(font::BAR_SIZE)
                    .font(font::SEMIBOLD),
                text(id_line(&row_of.account, signed_in_last))
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
            ]
            .spacing(8),
            space().width(Fill),
        ]
        .push(check)
        .spacing(16)
        .align_y(Center),
    )
    .width(Fill)
    .height(ROW_HEIGHT)
    .padding([0, 16])
    .style(account_button(selected))
    .on_press(message)
    .into()
}

fn account_button(selected: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme, status| {
        let palette = theme.extended_palette();
        let hovered = matches!(status, button::Status::Hovered | button::Status::Pressed);
        let (color, width) = if selected {
            (palette.primary.base.color, 2.0)
        } else {
            (palette.background.strong.color, 1.0)
        };

        button::Style {
            background: hovered.then(|| palette.background.weak.color.into()),
            text_color: palette.background.base.text,
            border: Border {
                color,
                width,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        }
    }
}

fn shown_name(account: &ClientAccount) -> &str {
    account
        .persona_name
        .as_deref()
        .map(str::trim)
        .filter(|name| !name.is_empty())
        .unwrap_or(NO_NAME)
}

fn id_line(account: &ClientAccount, signed_in_last: bool) -> String {
    if signed_in_last {
        format!("{} · {SIGNED_IN_LAST}", account.steam_id.0)
    } else {
        account.steam_id.0.to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn account(id: u64, name: Option<&str>, most_recent: bool) -> ClientAccount {
        ClientAccount {
            steam_id: SteamId(id),
            persona_name: name.map(str::to_string),
            most_recent,
            avatar: None,
        }
    }

    fn two() -> Vec<ClientAccount> {
        vec![
            account(76_561_198_000_000_000, Some("Player"), true),
            account(76_561_198_000_000_001, None, false),
        ]
    }

    #[test]
    fn the_first_account_is_selected() {
        let accounts = Accounts::new(two(), None);

        assert!(accounts.listed());
        assert_eq!(accounts.selected(), Some(SteamId(76_561_198_000_000_000)));
    }

    #[test]
    fn a_single_account_is_already_selected() {
        let accounts = Accounts::new(vec![account(76_561_198_000_000_005, None, false)], None);

        assert!(accounts.listed());
        assert_eq!(accounts.selected(), Some(SteamId(76_561_198_000_000_005)));
    }

    #[test]
    fn a_click_changes_the_choice() {
        let mut accounts = Accounts::new(two(), None);

        accounts.update(Message::Select(SteamId(76_561_198_000_000_001)));
        assert_eq!(accounts.selected(), Some(SteamId(76_561_198_000_000_001)));

        accounts.update(Message::Select(SteamId(1)));
        assert_eq!(accounts.selected(), Some(SteamId(76_561_198_000_000_001)));
    }

    #[test]
    fn use_another_account_shows_the_field_and_back_returns_to_the_list() {
        let mut accounts = Accounts::new(two(), None);

        accounts.update(Message::UseAnother);
        assert!(!accounts.listed());
        assert_eq!(accounts.selected(), None);
        assert!(!accounts.is_empty());

        accounts.update(Message::BackToList);
        assert!(accounts.listed());
        assert_eq!(accounts.selected(), Some(SteamId(76_561_198_000_000_000)));
    }

    #[test]
    fn a_saved_account_is_selected_when_the_list_has_it() {
        let accounts = Accounts::new(two(), Some(SteamId(76_561_198_000_000_001)));

        assert_eq!(accounts.selected(), Some(SteamId(76_561_198_000_000_001)));
    }

    #[test]
    fn a_saved_account_that_is_not_in_the_list_opens_the_field() {
        let accounts = Accounts::new(two(), Some(SteamId(76_561_198_000_000_009)));

        assert!(!accounts.listed());
        assert!(!accounts.is_empty());
    }

    #[test]
    fn without_accounts_there_is_only_the_field() {
        let mut accounts = Accounts::new(Vec::new(), None);

        assert!(accounts.is_empty());
        assert!(!accounts.listed());
        accounts.update(Message::UseAnother);
        accounts.update(Message::BackToList);
        assert!(!accounts.listed());
        assert_eq!(accounts.selected(), None);
    }

    #[test]
    fn a_missing_name_is_steam_account() {
        assert_eq!(shown_name(&two()[0]), "Player");
        assert_eq!(shown_name(&two()[1]), "Steam account");
        assert_eq!(shown_name(&account(1, Some("  "), false)), "Steam account");
    }

    #[test]
    fn only_the_last_account_signed_in_says_so() {
        assert_eq!(
            id_line(&two()[0], true),
            "76561198000000000 · Signed in last"
        );
        assert_eq!(id_line(&two()[1], false), "76561198000000001");
    }
}
