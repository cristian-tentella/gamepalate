use std::path::Path;

use anyhow::Context;
use clap::{Args, ValueEnum};
use gamepalate_core::{
    ids::AppId,
    taste::game_name,
    votes::{Vote, VoteKind},
};
use gamepalate_store::{data_files::DataFiles, votes::save_votes};

use crate::names::game_label;

#[derive(Args)]
pub struct VoteArgs {
    /// Steam appid of the game
    #[arg(value_parser = clap::value_parser!(u32).range(1..))]
    appid: u32,

    /// Your vote on the game
    vote: VoteChoice,

    /// The game was a bet in the recommendations
    #[arg(long)]
    bet: bool,
}

#[derive(Clone, Copy, ValueEnum)]
#[value(rename_all = "snake_case")]
enum VoteChoice {
    /// You would try it: it counts as a signal
    WouldTry,
    /// You might try it
    Maybe,
    /// Not for you: it leaves the recommendations
    NotForMe,
    /// You already know it: it leaves the recommendations
    Known,
}

impl VoteChoice {
    fn kind(self) -> VoteKind {
        match self {
            VoteChoice::WouldTry => VoteKind::WouldTry,
            VoteChoice::Maybe => VoteKind::Maybe,
            VoteChoice::NotForMe => VoteKind::NotForMe,
            VoteChoice::Known => VoteKind::Known,
        }
    }

    fn name(self) -> String {
        self.to_possible_value()
            .map(|value| value.get_name().to_string())
            .unwrap_or_default()
    }
}

pub fn run(args: &VoteArgs, files: &DataFiles, data_dir: &Path) -> anyhow::Result<()> {
    let id = AppId(args.appid);
    let was_bet = files.votes.get(&id).is_some_and(|vote| vote.bet);

    let mut votes = files.votes.clone();
    votes.insert(
        id,
        Vote {
            kind: args.vote.kind(),
            date: jiff::Zoned::now().date().to_string(),
            bet: args.bet || was_bet,
        },
    );

    let path = data_dir.join("votes.toml");
    save_votes(&path, &votes, |id| {
        let name = game_name(id, &files.catalog, &files.off_catalog);
        (name != format!("#{}", id.0)).then_some(name)
    })
    .with_context(|| format!("in {}", path.display()))?;

    let bet = if args.bet || was_bet { " (bet)" } else { "" };
    println!(
        "Voted {}: {}{bet}.",
        game_label(id, &files.catalog, &files.off_catalog),
        args.vote.name()
    );
    Ok(())
}
