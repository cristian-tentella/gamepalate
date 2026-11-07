use std::path::{Path, PathBuf};

use gamepalate_app::{
    setup::error_chain,
    steam_client::{answer_steam_client, steam_client_question},
};
use iced::{
    Center, Element, Fill, Task,
    widget::{button, column, container, row, space, text},
};

use crate::{font, style};

const TITLE: &str = "Games you played but don't own";
const WHY: &str = "Steam leaves out of your library the games borrowed through Family Sharing, free weekends and free games you removed. Their hours are in a file the Steam app keeps on this PC:";
const WHAT: &str = "gamepalate can read it at every update and keep only the hours of each game. Nothing is sent anywhere and the file is never changed.";

const ELSEWHERE: &str = "Played games outside Steam? Add them from ";

#[derive(Clone, Copy, Debug, PartialEq)]
pub(crate) enum ElsewhereHint<M> {
    Link(M),
    Text,
    None,
}

#[derive(Debug)]
pub(crate) struct SteamClientQuestion {
    file: PathBuf,
    saving: bool,
    error: Option<String>,
}

impl SteamClientQuestion {
    pub(crate) fn ask(data_dir: &Path) -> Option<SteamClientQuestion> {
        steam_client_question(data_dir).map(SteamClientQuestion::about)
    }

    pub(crate) fn about(file: PathBuf) -> SteamClientQuestion {
        SteamClientQuestion {
            file,
            saving: false,
            error: None,
        }
    }

    pub(crate) fn answer(&mut self, data_dir: PathBuf, read: bool) -> Task<Result<(), String>> {
        if self.saving {
            return Task::none();
        }
        self.saving = true;
        self.error = None;
        Task::perform(
            async move { answer_steam_client(&data_dir, read).map_err(|error| error_chain(&error)) },
            |result| result,
        )
    }

    pub(crate) fn failed(&mut self, error: String) {
        self.saving = false;
        self.error = Some(error);
    }

    pub(crate) fn view<'a, M: Clone + 'a>(
        &'a self,
        answer: fn(bool) -> M,
        hint: ElsewhereHint<M>,
    ) -> Element<'a, M> {
        let buttons = row![
            space().width(Fill),
            button(text("Don't read it").size(font::BAR_SIZE))
                .style(style::secondary)
                .padding([8, 20])
                .on_press_maybe((!self.saving).then(|| answer(false))),
            button(
                text(if self.saving {
                    "Reading…"
                } else {
                    "Read the hours"
                })
                .size(font::BAR_SIZE)
            )
            .style(style::primary)
            .padding([8, 20])
            .on_press_maybe((!self.saving).then(|| answer(true))),
        ]
        .spacing(12)
        .align_y(Center);
        let error = self
            .error
            .as_ref()
            .map(|error| text(error).size(font::NOTE_SIZE).style(text::danger));

        container(
            column![
                text(TITLE).size(font::INFO_SIZE).font(font::SEMIBOLD),
                text(WHY).size(font::BAR_SIZE),
                text(self.file.display().to_string())
                    .size(font::NOTE_SIZE)
                    .style(style::muted),
                text(WHAT).size(font::BAR_SIZE),
            ]
            .push(error)
            .push(buttons)
            .push(hint_view(hint))
            .spacing(10),
        )
        .padding(20)
        .width(Fill)
        .style(style::card)
        .into()
    }
}

fn hint_view<'a, M: Clone + 'a>(hint: ElsewhereHint<M>) -> Option<Element<'a, M>> {
    let library: Element<'a, M> = match hint {
        ElsewhereHint::None => return None,
        ElsewhereHint::Text => text("Library").size(font::NOTE_SIZE).into(),
        ElsewhereHint::Link(open) => button(text("Library").size(font::NOTE_SIZE))
            .padding(0)
            .style(style::link)
            .on_press(open)
            .into(),
    };

    Some(
        row![
            text(ELSEWHERE).size(font::NOTE_SIZE).style(style::muted),
            library,
            text(".").size(font::NOTE_SIZE).style(style::muted),
        ]
        .align_y(Center)
        .into(),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_hint_about_games_played_elsewhere_shows_only_when_asked() {
        assert!(hint_view::<()>(ElsewhereHint::None).is_none());
        assert!(hint_view::<()>(ElsewhereHint::Text).is_some());
        assert!(hint_view(ElsewhereHint::Link(())).is_some());
        assert_eq!(
            format!("{ELSEWHERE}Library."),
            "Played games outside Steam? Add them from Library."
        );
    }
}
