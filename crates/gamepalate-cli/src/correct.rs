use std::path::Path;

use anyhow::{Context, bail};
use clap::Args;
use gamepalate_core::{
    corrections::Ban,
    effective_hours::effective_hours,
    filters::find_tag,
    ids::{AppId, TagId},
    taste::game_name,
};
use gamepalate_store::{
    corrections_edit::{CorrectionsEditor, EntryValue, Section},
    data_files::DataFiles,
};

use crate::{
    format::{decimal, multiplier, parse_number},
    names::{game_label, tag_name_error_text},
};

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
    #[arg(required_unless_present = "game", value_name = "NAME [FACTOR]")]
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
        bail!("the factor of a tag must be a number > 0");
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
    if direction == Direction::Boost {
        bail!(
            "a game cannot be boosted: to make it count, put it in a family in {}",
            data_dir.join("families.toml").display()
        );
    }

    let factor = match words {
        [] => files.config.corrections.dampen,
        [factor] => parse_number(factor).map_err(anyhow::Error::msg)?,
        _ => bail!("after --game only a factor can follow"),
    };
    if !(0.0..=1.0).contains(&factor) {
        bail!(
            "the factor of a game must be from 0 to 1, where 0 means it does not count: to make a game count more, put it in a family in {}",
            data_dir.join("families.toml").display()
        );
    }

    let id = AppId(appid);
    let label = game_label(id, &files.catalog, &files.off_catalog);
    let min_hours = files.config.signals.min_hours;
    let hours = effective_hours(&files.signals, &files.votes, &files.config.signals);
    match hours.get(&id) {
        None => bail!("{label} is not in your signals: dampening it would have no effect"),
        Some(&game_hours) if game_hours < min_hours => bail!(
            "{label} has {} effective hours, below the threshold of {min_hours} hours: dampening it would have no effect",
            decimal(game_hours)
        ),
        Some(_) => {}
    }

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
