use gamepalate_core::{
    corrections::Ban,
    effective_hours::effective_hours,
    ids::{AppId, TagId},
    profile::corrected_games,
};
use gamepalate_store::data_files::DataFiles;

use crate::format::multiplier;

struct Line {
    name: Option<String>,
    text: String,
}

pub fn run(files: &DataFiles) {
    let inputs = files.search_inputs(&[]);
    let corrections = &files.corrections;
    let hours = effective_hours(&files.signals, &files.votes, &files.config.signals);
    let in_effect: Vec<(AppId, bool)> = corrected_games(&hours, &inputs)
        .iter()
        .map(|game| (game.id, game.in_effect))
        .collect();

    let sections = [
        (
            "banned_tags",
            corrections
                .banned_tags
                .iter()
                .map(|(&id, ban)| {
                    let detail = match ban {
                        Ban::Everywhere => String::new(),
                        Ban::TopTags(limit) => format!(" within the first {limit} tags"),
                    };
                    tag_line(id, files, detail)
                })
                .collect::<Vec<_>>(),
        ),
        (
            "tag_multipliers",
            corrections
                .tag_multipliers
                .iter()
                .map(|(&id, &value)| tag_line(id, files, format!(" {}", multiplier(value))))
                .collect(),
        ),
        (
            "game_multipliers",
            corrections
                .game_multipliers
                .iter()
                .map(|(&id, &value)| {
                    let effect = in_effect
                        .iter()
                        .find(|(game, _)| *game == id)
                        .is_some_and(|(_, in_effect)| *in_effect);
                    let note = if effect { "" } else { " (no effect)" };
                    game_line(id, files, format!(" {}{note}", multiplier(value)))
                })
                .collect(),
        ),
        (
            "hidden_games",
            corrections
                .hidden_games
                .iter()
                .map(|&id| game_line(id, files, String::new()))
                .collect(),
        ),
    ];

    if sections.iter().all(|(_, lines)| lines.is_empty()) {
        println!("No active corrections.");
        return;
    }

    let mut first = true;
    for (title, mut lines) in sections {
        if lines.is_empty() {
            continue;
        }
        if !first {
            println!();
        }
        first = false;
        lines.sort_by_key(|line| {
            (
                line.name.is_none(),
                line.name.as_ref().map(|name| name.to_lowercase()),
            )
        });
        println!("[{title}]");
        for line in lines {
            println!("  {}", line.text);
        }
    }
}

fn tag_line(id: TagId, files: &DataFiles, detail: String) -> Line {
    let name = files.catalog.tags.get(&id).map(|tag| tag.english.clone());
    let label = match &name {
        Some(name) => format!("{name} ({})", id.0),
        None => format!("tag {} (not in the catalog)", id.0),
    };
    Line {
        name,
        text: format!("{label}{detail}"),
    }
}

fn game_line(id: AppId, files: &DataFiles, detail: String) -> Line {
    let name = files
        .catalog
        .game(id)
        .map(|game| game.name.clone())
        .or_else(|| {
            files
                .off_catalog
                .get(&id)
                .and_then(|game| game.name.clone())
        });
    let label = match &name {
        Some(name) => format!("{name} ({})", id.0),
        None => format!("game {} (not in the catalog)", id.0),
    };
    Line {
        name,
        text: format!("{label}{detail}"),
    }
}
