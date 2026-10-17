use std::{collections::BTreeMap, io::ErrorKind, path::Path};

use gamepalate_core::{
    ids::AppId,
    votes::{Vote, VoteKind, Votes},
};
use serde::Deserialize;

use crate::keys::parse_id_key;

#[derive(Debug, thiserror::Error)]
pub enum VotesError {
    #[error("could not read the votes")]
    Io(#[from] std::io::Error),

    #[error("the votes file is not valid")]
    Toml(#[from] toml::de::Error),

    #[error("{0:?} is not an appid")]
    InvalidAppId(String),

    #[error("the vote for {appid} has date {date:?}, expected YYYY-MM-DD")]
    InvalidDate { appid: u32, date: String },

    #[error("the vote for {0} has bet = false, which can only be true")]
    BetNotTrue(u32),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VotesFile {
    #[serde(default)]
    votes: BTreeMap<String, VoteEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct VoteEntry {
    vote: VoteKindEntry,
    date: String,
    bet: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum VoteKindEntry {
    WouldTry,
    Maybe,
    NotForMe,
    Known,
}

impl From<VoteKindEntry> for VoteKind {
    fn from(kind: VoteKindEntry) -> Self {
        match kind {
            VoteKindEntry::WouldTry => VoteKind::WouldTry,
            VoteKindEntry::Maybe => VoteKind::Maybe,
            VoteKindEntry::NotForMe => VoteKind::NotForMe,
            VoteKindEntry::Known => VoteKind::Known,
        }
    }
}

pub fn load_votes(path: &Path) -> Result<Votes, VotesError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_votes(&text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Votes::new()),
        Err(error) => Err(error.into()),
    }
}

fn parse_votes(text: &str) -> Result<Votes, VotesError> {
    let file: VotesFile = toml::from_str(text)?;

    file.votes
        .into_iter()
        .map(|(key, entry)| {
            let appid = parse_id_key(&key).ok_or(VotesError::InvalidAppId(key))?;
            Ok((AppId(appid), vote_from_entry(appid, entry)?))
        })
        .collect()
}

fn vote_from_entry(appid: u32, entry: VoteEntry) -> Result<Vote, VotesError> {
    if !is_date_shaped(&entry.date) {
        return Err(VotesError::InvalidDate {
            appid,
            date: entry.date,
        });
    }

    let bet = match entry.bet {
        None => false,
        Some(true) => true,
        Some(false) => return Err(VotesError::BetNotTrue(appid)),
    };

    Ok(Vote {
        kind: entry.vote.into(),
        date: entry.date,
        bet,
    })
}

fn is_date_shaped(date: &str) -> bool {
    let bytes = date.as_bytes();
    bytes.len() == 10
        && bytes.iter().enumerate().all(|(i, &b)| match i {
            4 | 7 => b == b'-',
            _ => b.is_ascii_digit(),
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_votes_reads_every_field() {
        let votes = parse_votes(
            r#"
            [votes]
            1234 = { vote = "would_try", date = "2026-10-07" }
            5678 = { vote = "not_for_me", date = "2026-10-08", bet = true } # a name
            "#,
        )
        .unwrap();

        assert_eq!(
            votes[&AppId(1234)],
            Vote {
                kind: VoteKind::WouldTry,
                date: "2026-10-07".to_string(),
                bet: false,
            }
        );
        assert_eq!(
            votes[&AppId(5678)],
            Vote {
                kind: VoteKind::NotForMe,
                date: "2026-10-08".to_string(),
                bet: true,
            }
        );
    }

    #[test]
    fn parse_votes_orders_appids_numerically() {
        let votes = parse_votes(
            r#"
            [votes]
            100 = { vote = "maybe", date = "2026-10-07" }
            20 = { vote = "known", date = "2026-10-07" }
            3 = { vote = "maybe", date = "2026-10-07" }
            "#,
        )
        .unwrap();

        let appids: Vec<AppId> = votes.keys().copied().collect();
        assert_eq!(appids, [AppId(3), AppId(20), AppId(100)]);
    }

    #[test]
    fn parse_votes_without_the_section_is_empty() {
        assert!(parse_votes("").unwrap().is_empty());
    }

    #[test]
    fn parse_votes_rejects_an_appid_with_a_leading_zero() {
        let result = parse_votes(r#"votes = { "012" = { vote = "maybe", date = "2026-10-07" } }"#);
        assert!(matches!(result, Err(VotesError::InvalidAppId(key)) if key == "012"));
    }

    #[test]
    fn parse_votes_rejects_a_badly_shaped_date() {
        let result = parse_votes(r#"votes = { 10 = { vote = "maybe", date = "7/10/2026" } }"#);
        assert!(matches!(
            result,
            Err(VotesError::InvalidDate { appid: 10, .. })
        ));
    }

    #[test]
    fn parse_votes_rejects_bet_false() {
        let result =
            parse_votes(r#"votes = { 10 = { vote = "maybe", date = "2026-10-07", bet = false } }"#);
        assert!(matches!(result, Err(VotesError::BetNotTrue(10))));
    }

    #[test]
    fn parse_votes_rejects_an_unknown_vote_kind() {
        let result = parse_votes(r#"votes = { 10 = { vote = "si", date = "2026-10-07" } }"#);
        assert!(matches!(result, Err(VotesError::Toml(_))));
    }

    #[test]
    fn parse_votes_rejects_an_unknown_key() {
        let result =
            parse_votes(r#"votes = { 10 = { vote = "maybe", date = "2026-10-07", note = "" } }"#);
        assert!(matches!(result, Err(VotesError::Toml(_))));
    }

    #[test]
    fn load_votes_without_a_file_is_empty() {
        let votes = load_votes(Path::new("does-not-exist/votes.toml")).unwrap();
        assert!(votes.is_empty());
    }
}
