use std::cmp::Reverse;

use comfy_table::{ContentArrangement, Table, presets::UTF8_FULL_CONDENSED};
use gamepalate_core::{taste::game_name, votes::VoteKind};
use gamepalate_store::data_files::DataFiles;

use crate::{format::integer, names::game_label};

pub fn run(files: &DataFiles) {
    if files.votes.is_empty() {
        println!("No votes.");
        return;
    }

    let mut rows: Vec<_> = files
        .votes
        .iter()
        .map(|(&id, vote)| {
            let unnamed = game_name(id, &files.catalog, &files.off_catalog) == format!("#{}", id.0);
            let mut game = game_label(id, &files.catalog, &files.off_catalog);
            if vote.bet {
                game.push_str(" [bet]");
            }
            if files.corrections.hidden_games.contains(&id) {
                game.push_str(" [hidden]");
            }
            (vote, unnamed, game)
        })
        .collect();
    rows.sort_by_cached_key(|(vote, unnamed, game)| {
        (Reverse(vote.date.clone()), *unnamed, game.to_lowercase())
    });

    let mut table = Table::new();
    table
        .load_style(UTF8_FULL_CONDENSED)
        .set_content_arrangement(ContentArrangement::Dynamic)
        .set_header(["Date", "Vote", "Game"]);
    for (vote, _, game) in rows {
        table.add_row([vote.date.clone(), kind_name(vote.kind).to_string(), game]);
    }
    println!("{table}");

    let bets = files.votes.values().filter(|vote| vote.bet).count();
    println!(
        "{} votes, {} of them on bets.",
        integer(files.votes.len() as u64),
        integer(bets as u64)
    );
}

fn kind_name(kind: VoteKind) -> &'static str {
    match kind {
        VoteKind::WouldTry => "would_try",
        VoteKind::Maybe => "maybe",
        VoteKind::NotForMe => "not_for_me",
        VoteKind::Known => "known",
    }
}
