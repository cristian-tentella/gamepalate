use std::path::Path;

use anyhow::{Context, bail};
use clap::{Args, CommandFactory, error::ErrorKind};
use gamepalate_app::library::{GameCorrectionError, check_game_multiplier};
use gamepalate_core::{
    corrections::Ban,
    filters::find_tag,
    ids::{AppId, TagId},
    taste::game_name,
};
use gamepalate_store::{
    corrections_edit::{CorrectionsEditor, EntryValue, Section},
    data_files::DataFiles,
};

use crate::{
    format::{multiplier, parse_number},
    names::{game_label, tag_name_error_text},
};

#[derive(Args)]
pub struct PlayedElsewhereArgs {
    /// Steam appid of the game
    #[arg(value_parser = clap::value_parser!(u32).range(1..))]
    appid: u32,

    /// Stop counting the game as played elsewhere
    #[arg(long)]
    remove: bool,
}

#[derive(Args)]
pub struct BanArgs {
    /// Ban the tag only for games that have it within their first N tags
    #[arg(long, value_name = "N", value_parser = clap::value_parser!(u32).range(1..))]
    within_first: Option<u32>,

    /// Tag name, in English or Italian
    #[arg(required = true, value_name = "NAME")]
    words: Vec<String>,
}

#[derive(Args)]
pub struct MultiplierArgs {
    /// Correct a game of your signals instead of a tag
    #[arg(long, value_name = "APPID")]
    game: Option<u32>,

    /// Tag name, in English or Italian, then an optional factor
    #[arg(
        required_unless_present = "game",
        allow_negative_numbers = true,
        value_name = "NAME [FACTOR]"
    )]
    words: Vec<String>,
}

#[derive(Args)]
pub struct RemoveArgs {
    /// Tag name, in English or Italian, or the appid of a game or the id of a tag
    #[arg(required = true, value_name = "NAME|ID")]
    words: Vec<String>,
}

#[derive(Clone, Copy, PartialEq)]
pub enum Direction {
    Boost,
    Dampen,
}

impl Direction {
    fn command(self) -> &'static str {
        match self {
            Direction::Boost => "boost",
            Direction::Dampen => "dampen",
        }
    }
}

pub fn hide(appid: u32, files: &DataFiles, data_dir: &Path) -> anyhow::Result<()> {
    let id = AppId(appid);
    let label = game_label(id, &files.catalog, &files.off_catalog);
    if files.corrections.hidden_games.contains(&id) {
        println!("Game {label} was already hidden.");
        return Ok(());
    }

    let mut editor = open(data_dir)?;
    editor.set(
        Section::HiddenGames,
        appid,
        EntryValue::Bool(true),
        game_comment(id, files).as_deref(),
    );
    save(editor, data_dir)?;

    println!("Hidden game {label}.");
    Ok(())
}

pub fn played_elsewhere(
    args: &PlayedElsewhereArgs,
    files: &DataFiles,
    data_dir: &Path,
) -> anyhow::Result<()> {
    println!("{}", change_played_elsewhere(args, files, data_dir)?);
    Ok(())
}

fn change_played_elsewhere(
    args: &PlayedElsewhereArgs,
    files: &DataFiles,
    data_dir: &Path,
) -> anyhow::Result<String> {
    let id = AppId(args.appid);
    let label = game_label(id, &files.catalog, &files.off_catalog);
    let counted = files.corrections.played_elsewhere.contains(&id);
    if counted != args.remove {
        return Ok(if args.remove {
            format!("Game {label} was not played elsewhere.")
        } else {
            format!("Game {label} was already played elsewhere.")
        });
    }

    let mut editor = open(data_dir)?;
    if args.remove {
        editor.remove(Section::PlayedElsewhere, args.appid);
    } else {
        editor.set(
            Section::PlayedElsewhere,
            args.appid,
            EntryValue::Bool(true),
            game_comment(id, files).as_deref(),
        );
    }
    save(editor, data_dir)?;

    Ok(if args.remove {
        format!("Removed {label} from the games played elsewhere.")
    } else {
        format!("Added {label} as played elsewhere.")
    })
}

pub fn ban(args: &BanArgs, files: &DataFiles, data_dir: &Path) -> anyhow::Result<()> {
    let (id, name) = tag(&args.words.join(" "), files)?;
    let label = format!("{name} ({})", id.0);
    let ban = args.within_first.map_or(Ban::Everywhere, Ban::TopTags);
    let scope = match ban {
        Ban::Everywhere => String::new(),
        Ban::TopTags(limit) => format!(" within the first {limit} tags"),
    };
    if files.corrections.banned_tags.get(&id) == Some(&ban) {
        println!("Tag {label} was already banned{scope}.");
        return Ok(());
    }

    let value = match ban {
        Ban::Everywhere => EntryValue::Bool(true),
        Ban::TopTags(limit) => EntryValue::Integer(i64::from(limit)),
    };
    let mut editor = open(data_dir)?;
    editor.set(Section::BannedTags, id.0, value, Some(&name));
    save(editor, data_dir)?;

    println!("Banned tag {label}{scope}.");
    Ok(())
}

pub fn correct_multiplier(
    direction: Direction,
    args: &MultiplierArgs,
    files: &DataFiles,
    data_dir: &Path,
) -> anyhow::Result<()> {
    match args.game {
        Some(appid) => correct_game(direction, appid, &args.words, files, data_dir),
        None => correct_tag(direction, &args.words, files, data_dir),
    }
}

fn correct_tag(
    direction: Direction,
    words: &[String],
    files: &DataFiles,
    data_dir: &Path,
) -> anyhow::Result<()> {
    let (name, factor) = split_factor(words);
    let factor = factor.unwrap_or(match direction {
        Direction::Boost => files.config.corrections.boost,
        Direction::Dampen => files.config.corrections.dampen,
    });
    if factor <= 0.0 {
        usage_error(
            direction.command(),
            "the factor of a tag must be a number > 0",
        );
    }

    let (id, name) = tag(&name, files)?;
    let label = format!("{name} ({})", id.0);
    if files.corrections.tag_multipliers.get(&id) == Some(&factor) {
        println!("Tag {label} was already at {}.", multiplier(factor));
        return Ok(());
    }

    let mut editor = open(data_dir)?;
    editor.set(
        Section::TagMultipliers,
        id.0,
        EntryValue::Float(factor),
        Some(&name),
    );
    save(editor, data_dir)?;

    let verb = match direction {
        Direction::Boost => "Boosted",
        Direction::Dampen => "Dampened",
    };
    println!("{verb} tag {label} to {}.", multiplier(factor));
    Ok(())
}

fn correct_game(
    direction: Direction,
    appid: u32,
    words: &[String],
    files: &DataFiles,
    data_dir: &Path,
) -> anyhow::Result<()> {
    let families_path = data_dir.join("families.toml");
    if direction == Direction::Boost {
        usage_error(
            "boost",
            format!(
                "a game cannot be boosted: to make it count, put it in a family in {}",
                families_path.display()
            ),
        );
    }

    let factor = match words {
        [] => files.config.corrections.dampen,
        [factor] => parse_number(factor).unwrap_or_else(|error| usage_error("dampen", error)),
        _ => usage_error("dampen", "after --game only a factor can follow"),
    };
    let id = AppId(appid);
    match check_game_multiplier(id, factor, &files.search_inputs(&[])) {
        Ok(()) => {}
        Err(GameCorrectionError::FactorOutOfRange { factor }) => {
            let message = format!(
                "the factor of a game must be from 0 to 1, where 0 means it does not count: to make a game count more, put it in a family in {}",
                families_path.display()
            );
            if words.is_empty() {
                bail!("corrections.dampen of the config is {factor}: {message}");
            }
            usage_error("dampen", message);
        }
        Err(error) => bail!(error),
    }
    let label = game_label(id, &files.catalog, &files.off_catalog);

    if files.corrections.game_multipliers.get(&id) == Some(&factor) {
        println!("Game {label} was already at {}.", multiplier(factor));
        return Ok(());
    }

    let mut editor = open(data_dir)?;
    editor.set(
        Section::GameMultipliers,
        appid,
        EntryValue::Float(factor),
        game_comment(id, files).as_deref(),
    );
    save(editor, data_dir)?;

    println!("Dampened game {label} to {}.", multiplier(factor));
    Ok(())
}

pub fn remove(args: &RemoveArgs, files: &DataFiles, data_dir: &Path) -> anyhow::Result<()> {
    let text = args.words.join(" ");
    let number = text
        .parse::<u32>()
        .ok()
        .filter(|_| text.chars().all(|c| c.is_ascii_digit()));

    if let Some(appid) = number {
        let id = AppId(appid);
        let mut removed = Vec::new();
        if files.corrections.hidden_games.contains(&id) {
            removed.push("hidden".to_string());
        }
        if let Some(&factor) = files.corrections.game_multipliers.get(&id) {
            removed.push(multiplier(factor));
        }
        if !removed.is_empty() {
            let mut editor = open(data_dir)?;
            editor.remove(Section::HiddenGames, appid);
            editor.remove(Section::GameMultipliers, appid);
            save(editor, data_dir)?;
            println!(
                "Removed the corrections of game {}: {}.",
                game_label(id, &files.catalog, &files.off_catalog),
                removed.join(", ")
            );
            return Ok(());
        }
    }

    let id = match number {
        Some(tagid) => TagId(tagid),
        None => tag(&text, files)?.0,
    };
    let label = match files.catalog.tags.get(&id) {
        Some(tag) => format!("{} ({})", tag.english, id.0),
        None => format!("#{}", id.0),
    };

    let mut removed = Vec::new();
    match files.corrections.banned_tags.get(&id) {
        Some(Ban::Everywhere) => removed.push("banned".to_string()),
        Some(Ban::TopTags(limit)) => removed.push(format!("banned within the first {limit} tags")),
        None => {}
    }
    if let Some(&factor) = files.corrections.tag_multipliers.get(&id) {
        removed.push(multiplier(factor));
    }
    if removed.is_empty() {
        match number {
            Some(_) => bail!("no correction on a game or a tag with id {}", id.0),
            None => bail!("no correction on tag {label}"),
        }
    }

    let mut editor = open(data_dir)?;
    editor.remove(Section::BannedTags, id.0);
    editor.remove(Section::TagMultipliers, id.0);
    save(editor, data_dir)?;
    println!(
        "Removed the corrections of tag {label}: {}.",
        removed.join(", ")
    );
    Ok(())
}

fn split_factor(words: &[String]) -> (String, Option<f64>) {
    if let [name @ .., last] = words
        && !name.is_empty()
        && let Ok(factor) = parse_number(last)
    {
        return (name.join(" "), Some(factor));
    }
    (words.join(" "), None)
}

fn usage_error(command: &str, message: impl std::fmt::Display) -> ! {
    let mut cli = crate::Cli::command();
    cli.build();
    match cli.find_subcommand_mut(command) {
        Some(command) => command.error(ErrorKind::ValueValidation, message).exit(),
        None => cli.error(ErrorKind::ValueValidation, message).exit(),
    }
}

fn tag(name: &str, files: &DataFiles) -> anyhow::Result<(TagId, String)> {
    let id = find_tag(name, &files.catalog)
        .map_err(|error| anyhow::anyhow!(tag_name_error_text(&error)))?;
    let name = files.catalog.tags[&id].english.clone();
    Ok((id, name))
}

fn game_comment(id: AppId, files: &DataFiles) -> Option<String> {
    let name = game_name(id, &files.catalog, &files.off_catalog);
    (name != format!("#{}", id.0)).then_some(name)
}

fn open(data_dir: &Path) -> anyhow::Result<CorrectionsEditor> {
    let path = data_dir.join("corrections.toml");
    CorrectionsEditor::open(&path).with_context(|| format!("in {}", path.display()))
}

fn save(editor: CorrectionsEditor, data_dir: &Path) -> anyhow::Result<()> {
    let path = data_dir.join("corrections.toml");
    editor
        .save()
        .with_context(|| format!("in {}", path.display()))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use gamepalate_core::{
        catalog::{Catalog, Freshness},
        config::Config,
        corrections::Corrections,
        currency::Currency,
        families::Families,
        game::{Game, Price, Reviews},
        off_catalog::OffCatalogGames,
        region::Region,
        signals::Signals,
        timestamp::Timestamp,
        votes::Votes,
    };

    use super::*;

    fn files(corrections: Corrections) -> DataFiles {
        let game = Game {
            id: AppId(1426210),
            name: "It Takes Two".to_string(),
            price: Price {
                original_cents: 3999,
                discount: None,
            },
            reviews: Reviews {
                total: 100,
                positive: 95,
            },
            is_early_access: false,
            tags: Vec::new(),
            developers: Vec::new(),
            franchises: Vec::new(),
        };
        DataFiles {
            config: Config::default(),
            catalog: Catalog::new(
                vec![game],
                BTreeMap::new(),
                BTreeMap::new(),
                Freshness {
                    prices_updated_at: Timestamp(0),
                    tags_updated_at: Timestamp(0),
                    region: Region::ITALY,
                },
                Currency::italian(),
            ),
            corrections,
            families: Families::default(),
            signals: Signals {
                read_at: Timestamp(0),
                library: Vec::new(),
                borrowed: Vec::new(),
                recent: Vec::new(),
                wishlist: Vec::new(),
                followed: Vec::new(),
            },
            votes: Votes::new(),
            off_catalog: OffCatalogGames::new(),
            known_converted: None,
        }
    }

    fn args(remove: bool) -> PlayedElsewhereArgs {
        PlayedElsewhereArgs {
            appid: 1426210,
            remove,
        }
    }

    #[test]
    fn a_game_is_added_as_played_elsewhere_and_removed() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");

        let message =
            change_played_elsewhere(&args(false), &files(Corrections::default()), dir.path())
                .unwrap();

        assert_eq!(message, "Added It Takes Two (1426210) as played elsewhere.");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains("1426210 = true # It Takes Two"), "{text}");

        let corrections = gamepalate_store::corrections::load_corrections(&path).unwrap();
        let message =
            change_played_elsewhere(&args(true), &files(corrections), dir.path()).unwrap();

        assert_eq!(
            message,
            "Removed It Takes Two (1426210) from the games played elsewhere."
        );
        assert!(!std::fs::read_to_string(&path).unwrap().contains("1426210"));
    }

    #[test]
    fn a_change_that_is_already_there_writes_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let counted = Corrections {
            played_elsewhere: [AppId(1426210)].into(),
            ..Corrections::default()
        };

        let again = change_played_elsewhere(&args(false), &files(counted), dir.path()).unwrap();
        let missing =
            change_played_elsewhere(&args(true), &files(Corrections::default()), dir.path())
                .unwrap();

        assert_eq!(
            again,
            "Game It Takes Two (1426210) was already played elsewhere."
        );
        assert_eq!(
            missing,
            "Game It Takes Two (1426210) was not played elsewhere."
        );
        assert!(!dir.path().join("corrections.toml").exists());
    }
}
