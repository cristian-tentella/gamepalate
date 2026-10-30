use std::process::Command;

use gamepalate_app::{
    Gamepalate,
    setup::{FirstRun, Part, error_chain},
};
use iced::{
    Center, Element, Fill, padding,
    widget::{button, column, container, row, text},
};

use crate::{font, style};

const CHECK: char = '\u{E73E}';
const CROSS: char = '\u{E711}';
const WIDTH: f32 = 760.0;

pub struct Setup {
    first_run: FirstRun,
    error: Option<String>,
}

#[derive(Clone, Debug)]
pub enum Message {
    CheckAgain,
    OpenFolder,
}

#[derive(Debug, PartialEq)]
struct Line {
    label: &'static str,
    present: bool,
    hint: Option<String>,
}

pub fn start(first_run: FirstRun) -> Result<Gamepalate, Box<Setup>> {
    if !first_run.ready() {
        return Err(Box::new(Setup {
            first_run,
            error: None,
        }));
    }

    Gamepalate::open(first_run.data_dir.clone()).map_err(|error| {
        Box::new(Setup {
            first_run,
            error: Some(error_chain(&error)),
        })
    })
}

impl Setup {
    pub fn first_run(&self) -> &FirstRun {
        &self.first_run
    }

    pub fn update(&mut self, message: Message) -> Option<Gamepalate> {
        match message {
            Message::CheckAgain => match start(FirstRun::check(None)) {
                Ok(gamepalate) => Some(gamepalate),
                Err(setup) => {
                    *self = *setup;
                    None
                }
            },
            Message::OpenFolder => {
                if let Some(dir) = &self.first_run.data_dir {
                    let _ = std::fs::create_dir_all(dir);
                    let _ = Command::new("explorer").arg(dir).spawn();
                }
                None
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let title = text("gamepalate needs a few things before it can recommend games.")
            .size(font::PRICE_SIZE)
            .font(font::SEMIBOLD);
        let folder: Element<'_, Message> = match &self.first_run.data_dir {
            Some(dir) => row![
                text!("Data folder: {}", dir.display()).size(font::BAR_SIZE),
                button(text("Open folder").size(font::BAR_SIZE))
                    .padding(0)
                    .style(style::link)
                    .on_press(Message::OpenFolder),
            ]
            .spacing(16)
            .align_y(Center)
            .into(),
            None => text("Could not find the user data folder: set GAMEPALATE_DATA_DIR.")
                .size(font::BAR_SIZE)
                .style(text::danger)
                .into(),
        };
        let lines = column(lines(&self.first_run).into_iter().map(line_view)).spacing(16);
        let error = self.error.as_ref().map(|error| {
            text!("gamepalate could not open the data files: {error}")
                .size(font::BAR_SIZE)
                .style(text::danger)
        });
        let check = button(text("Check again").size(font::BAR_SIZE))
            .padding([8, 20])
            .on_press(Message::CheckAgain);

        container(
            column![title, folder, lines]
                .push(error)
                .push(check)
                .spacing(28)
                .max_width(WIDTH),
        )
        .padding(40)
        .center_x(Fill)
        .into()
    }
}

fn lines(first_run: &FirstRun) -> Vec<Line> {
    vec![
        part_line(
            "config.toml with your Steam ID",
            &first_run.steam_id,
            "add steam_id = \"<your SteamID64>\" to config.toml",
        ),
        Line {
            label: "Steam Web API key",
            present: first_run.api_key,
            hint: (!first_run.api_key).then(|| {
                "set STEAM_API_KEY: you can get one at steamcommunity.com/dev/apikey".to_string()
            }),
        },
        part_line(
            "Catalog of Steam games",
            &first_run.catalog,
            "run: gamepalate update (15-25 minutes the first time)",
        ),
        part_line(
            "Your Signals",
            &first_run.signals,
            "run: gamepalate profile",
        ),
    ]
}

fn part_line(label: &'static str, part: &Part, missing: &str) -> Line {
    Line {
        label,
        present: *part == Part::Present,
        hint: match part {
            Part::Present => None,
            Part::Missing => Some(missing.to_string()),
            Part::Invalid(message) => Some(message.clone()),
        },
    }
}

fn line_view<'a>(line: Line) -> Element<'a, Message> {
    let mark = text(if line.present { CHECK } else { CROSS })
        .font(font::ICONS)
        .size(12)
        .style(if line.present {
            text::success
        } else {
            text::danger
        });

    column![
        row![
            mark,
            text(line.label).size(font::INFO_SIZE).font(font::SEMIBOLD)
        ]
        .spacing(12)
        .align_y(Center)
    ]
    .push(line.hint.map(|hint| {
        container(text(hint).size(font::BAR_SIZE).style(style::muted)).padding(padding::left(24))
    }))
    .spacing(4)
    .into()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::*;

    fn first_run() -> FirstRun {
        FirstRun {
            data_dir: Some(PathBuf::from("data")),
            steam_id: Part::Present,
            api_key: true,
            catalog: Part::Present,
            signals: Part::Present,
        }
    }

    #[test]
    fn a_missing_part_opens_the_setup_without_trying_the_files() {
        let first_run = FirstRun {
            catalog: Part::Missing,
            ..first_run()
        };

        let Err(setup) = start(first_run.clone()) else {
            panic!("expected the setup");
        };

        assert_eq!(setup.first_run, first_run);
        assert_eq!(setup.error, None);
    }

    #[test]
    fn files_that_do_not_open_show_the_error() {
        let dir = tempfile::tempdir().unwrap();
        let first_run = FirstRun {
            data_dir: Some(dir.path().to_path_buf()),
            ..first_run()
        };

        let Err(setup) = start(first_run) else {
            panic!("expected the setup");
        };

        assert!(setup.error.unwrap().starts_with("config not found"));
    }

    #[test]
    fn everything_there_has_no_hints() {
        assert!(
            lines(&first_run())
                .iter()
                .all(|line| line.present && line.hint.is_none())
        );
    }

    #[test]
    fn a_missing_catalog_says_which_command_to_run() {
        let first_run = FirstRun {
            api_key: false,
            catalog: Part::Missing,
            ..first_run()
        };

        let lines = lines(&first_run);

        assert!(!lines[1].present);
        assert!(lines[1].hint.as_ref().unwrap().contains("STEAM_API_KEY"));
        assert_eq!(
            lines[2],
            Line {
                label: "Catalog of Steam games",
                present: false,
                hint: Some("run: gamepalate update (15-25 minutes the first time)".to_string()),
            }
        );
    }

    #[test]
    fn an_invalid_file_shows_its_message() {
        let first_run = FirstRun {
            signals: Part::Invalid("the signals file is not valid".to_string()),
            ..first_run()
        };

        let line = &lines(&first_run)[3];

        assert!(!line.present);
        assert_eq!(line.hint.as_deref(), Some("the signals file is not valid"));
    }
}
