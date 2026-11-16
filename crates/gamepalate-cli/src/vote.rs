use std::path::Path;

use anyhow::Context;
use clap::{Args, ValueEnum};
use gamepalate_core::{
    ids::AppId,
    taste::game_name,
    timestamp::Timestamp,
    votes::{Vote, VoteKind, cast},
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
    /// Not now: it comes back in the recommendations after 7 days
    Later,
    /// Not for you: it leaves the recommendations
    NotForMe,
}

impl VoteChoice {
    fn kind(self) -> VoteKind {
        match self {
            VoteChoice::WouldTry => VoteKind::WouldTry,
            VoteChoice::Later => VoteKind::Later,
            VoteChoice::NotForMe => VoteKind::NotForMe,
        }
    }

    fn name(self) -> String {
        self.to_possible_value()
            .map(|value| value.get_name().to_string())
            .unwrap_or_default()
    }
}

impl VoteArgs {
    fn vote(&self, date: String, at: Timestamp) -> Vote {
        Vote {
            kind: self.vote.kind(),
            date,
            at: Some(at),
            bet: self.bet,
            searched: false,
        }
    }
}

pub fn run(args: &VoteArgs, files: &DataFiles, data_dir: &Path) -> anyhow::Result<()> {
    let id = AppId(args.appid);
    let vote = args.vote(
        jiff::Zoned::now().date().to_string(),
        Timestamp(jiff::Timestamp::now().as_millisecond()),
    );
    let mut votes = files.votes.clone();
    cast(&mut votes, id, vote);

    let path = data_dir.join("votes.toml");
    save_votes(&path, &votes, |id| {
        let name = game_name(id, &files.catalog, &files.off_catalog);
        (name != format!("#{}", id.0)).then_some(name)
    })
    .with_context(|| format!("in {}", path.display()))?;

    let bet = if votes[&id].bet { " (bet)" } else { "" };
    println!(
        "Voted {}: {}{bet}.",
        game_label(id, &files.catalog, &files.off_catalog),
        args.vote.name()
    );
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn choice(name: &str) -> Option<VoteKind> {
        VoteChoice::from_str(name, false).ok().map(VoteChoice::kind)
    }

    #[test]
    fn later_is_a_vote_and_maybe_is_not_any_more() {
        assert_eq!(choice("later"), Some(VoteKind::Later));
        assert_eq!(choice("would_try"), Some(VoteKind::WouldTry));
        assert_eq!(choice("maybe"), None);
    }
}
