use std::{
    collections::{BTreeMap, BTreeSet},
    fmt::Write as _,
    io::ErrorKind,
    path::{Path, PathBuf},
};

use gamepalate_core::{
    ids::AppId,
    timestamp::Timestamp,
    votes::{Vote, VoteKind, Votes},
};
use serde::Deserialize;

use crate::keys::parse_id_key;

const HEADER: &str = "\
# Votes on the recommended games. Every key is a Steam appid, the comment after it is only a reminder.
# gamepalate rewrites this file at every vote: comments added by hand are lost.
#
# would_try   you would try it, it counts as a signal
# later       not now: the game comes back in Discover after 30 days
# not_for_me  not for you, it leaves the recommendations
# bet = true  the vote was given to a bet
# genre_ok = true  a not_for_me given as Not this one: the genre is fine, the game is not
# at          when the vote was cast, in milliseconds since 1970 (for the last 24 hours and the 30 days of a later)
";

#[derive(Debug, thiserror::Error)]
pub enum VotesError {
    #[error("could not read the votes")]
    Io(#[from] std::io::Error),

    #[error("could not save the votes")]
    Write(#[source] std::io::Error),

    #[error("the votes file is not valid")]
    Toml(#[from] toml::de::Error),

    #[error("{0:?} is not an appid")]
    InvalidAppId(String),

    #[error("the vote for {appid} has date {date:?}, expected YYYY-MM-DD")]
    InvalidDate { appid: u32, date: String },

    #[error("the vote for {0} has bet = false, which can only be true")]
    BetNotTrue(u32),

    #[error("the vote for {0} has genre_ok = false, which can only be true")]
    GenreOkNotTrue(u32),

    #[error("the vote for {0} has genre_ok, which only a not_for_me can have")]
    GenreOkNotNotForMe(u32),
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
    at: Option<i64>,
    bet: Option<bool>,
    genre_ok: Option<bool>,
}

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum VoteKindEntry {
    WouldTry,
    #[serde(alias = "maybe")]
    Later,
    NotForMe,
    // An old vote, only read to be converted: see `known`.
    Known,
}

impl VoteKindEntry {
    fn kind(&self) -> Option<VoteKind> {
        match self {
            VoteKindEntry::WouldTry => Some(VoteKind::WouldTry),
            VoteKindEntry::Later => Some(VoteKind::Later),
            VoteKindEntry::NotForMe => Some(VoteKind::NotForMe),
            VoteKindEntry::Known => None,
        }
    }
}

#[derive(Debug, Default, PartialEq)]
pub struct ReadVotes {
    pub votes: Votes,
    /// The games of the old `known` votes, left out of `votes`.
    pub known: BTreeSet<AppId>,
}

pub fn load_votes(path: &Path) -> Result<ReadVotes, VotesError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_votes(&text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(ReadVotes::default()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_votes(
    path: &Path,
    votes: &Votes,
    name: impl Fn(AppId) -> Option<String>,
) -> Result<(), VotesError> {
    let text = votes_text(votes, name);
    parse_votes(&text)?;

    let mut temporary = path.as_os_str().to_owned();
    temporary.push(".tmp");
    let temporary = PathBuf::from(temporary);
    std::fs::write(&temporary, &text).map_err(VotesError::Write)?;
    std::fs::rename(&temporary, path).map_err(|error| {
        let _ = std::fs::remove_file(&temporary);
        VotesError::Write(error)
    })
}

fn votes_text(votes: &Votes, name: impl Fn(AppId) -> Option<String>) -> String {
    let mut text = format!("{HEADER}\n[votes]\n");
    for (&id, vote) in votes {
        let at = vote
            .at
            .map_or(String::new(), |at| format!(", at = {}", at.0));
        let bet = if vote.bet { ", bet = true" } else { "" };
        let genre_ok = if vote.genre_ok {
            ", genre_ok = true"
        } else {
            ""
        };
        let _ = write!(
            text,
            "{} = {{ vote = \"{}\", date = \"{}\"{at}{bet}{genre_ok} }}",
            id.0,
            kind_name(vote.kind),
            vote.date
        );
        if let Some(name) = name(id) {
            let _ = write!(text, " # {}", name.replace(['\r', '\n'], " "));
        }
        text.push('\n');
    }
    text
}

/// The name of the vote in `votes.toml`, which the CLI uses too.
pub fn kind_name(kind: VoteKind) -> &'static str {
    match kind {
        VoteKind::WouldTry => "would_try",
        VoteKind::Later => "later",
        VoteKind::NotForMe => "not_for_me",
    }
}

fn parse_votes(text: &str) -> Result<ReadVotes, VotesError> {
    let file: VotesFile = toml::from_str(text)?;

    let mut read = ReadVotes::default();
    for (key, entry) in file.votes {
        let appid = parse_id_key(&key).ok_or(VotesError::InvalidAppId(key))?;
        match vote_from_entry(appid, entry)? {
            Some(vote) => {
                read.votes.insert(AppId(appid), vote);
            }
            None => {
                read.known.insert(AppId(appid));
            }
        }
    }
    Ok(read)
}

fn vote_from_entry(appid: u32, entry: VoteEntry) -> Result<Option<Vote>, VotesError> {
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
    let genre_ok = match (entry.genre_ok, &entry.vote) {
        (None, _) => false,
        (Some(false), _) => return Err(VotesError::GenreOkNotTrue(appid)),
        (Some(true), VoteKindEntry::NotForMe) => true,
        (Some(true), _) => return Err(VotesError::GenreOkNotNotForMe(appid)),
    };

    Ok(entry.vote.kind().map(|kind| Vote {
        kind,
        date: entry.date,
        at: entry.at.map(Timestamp),
        bet,
        genre_ok,
    }))
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
        .unwrap()
        .votes;

        assert_eq!(
            votes[&AppId(1234)],
            Vote::new(VoteKind::WouldTry, "2026-10-07")
        );
        assert_eq!(
            votes[&AppId(5678)],
            Vote {
                bet: true,
                ..Vote::new(VoteKind::NotForMe, "2026-10-08")
            }
        );
    }

    #[test]
    fn the_time_of_a_vote_survives_saving_and_loading() {
        let mut votes = Votes::new();
        votes.insert(
            AppId(10),
            Vote {
                at: Some(Timestamp(1_762_500_000_000)),
                bet: true,
                ..Vote::new(VoteKind::Later, "2026-11-07")
            },
        );
        votes.insert(AppId(20), Vote::new(VoteKind::NotForMe, "2026-10-07"));

        let text = votes_text(&votes, |_| None);

        assert!(text.contains(
            "10 = { vote = \"later\", date = \"2026-11-07\", at = 1762500000000, bet = true }"
        ));
        assert_eq!(parse_votes(&text).unwrap().votes, votes);
    }

    #[test]
    fn not_this_one_survives_saving_and_loading() {
        let votes = parse_votes(
            r#"
            [votes]
            383870 = { vote = "not_for_me", date = "2026-11-13", at = 1794500000000, genre_ok = true } # Firewatch
            412020 = { vote = "not_for_me", date = "2026-11-13" }
            "#,
        )
        .unwrap()
        .votes;

        assert!(votes[&AppId(383870)].genre_ok);
        assert!(!votes[&AppId(412020)].genre_ok);
        let text = votes_text(&votes, |_| None);
        assert!(text.contains(
            "383870 = { vote = \"not_for_me\", date = \"2026-11-13\", at = 1794500000000, genre_ok = true }"
        ));
        assert!(text.contains("412020 = { vote = \"not_for_me\", date = \"2026-11-13\" }"));
        assert_eq!(parse_votes(&text).unwrap().votes, votes);
    }

    #[test]
    fn parse_votes_rejects_genre_ok_false() {
        let result = parse_votes(
            r#"votes = { 10 = { vote = "not_for_me", date = "2026-10-07", genre_ok = false } }"#,
        );
        assert!(matches!(result, Err(VotesError::GenreOkNotTrue(10))));
    }

    #[test]
    fn parse_votes_rejects_genre_ok_on_another_vote() {
        let result = parse_votes(
            r#"votes = { 10 = { vote = "would_try", date = "2026-10-07", genre_ok = true } }"#,
        );
        assert!(matches!(result, Err(VotesError::GenreOkNotNotForMe(10))));
    }

    #[test]
    fn an_old_maybe_is_read_as_a_later_and_saved_as_one() {
        let votes = parse_votes(
            r#"
            [votes]
            1496590 = { vote = "maybe", date = "2026-11-11", at = 1794404475152 } # Tchia
            2393760 = { vote = "later", date = "2026-11-11" }
            "#,
        )
        .unwrap()
        .votes;

        assert_eq!(votes[&AppId(1496590)].kind, VoteKind::Later);
        assert_eq!(votes[&AppId(2393760)].kind, VoteKind::Later);
        let text = votes_text(&votes, |_| None);
        assert!(
            text.contains(
                "1496590 = { vote = \"later\", date = \"2026-11-11\", at = 1794404475152 }"
            )
        );
        assert!(!text.contains("\"maybe\""));
    }

    #[test]
    fn an_old_known_is_read_apart_and_never_saved() {
        let read = parse_votes(
            r#"
            [votes]
            500 = { vote = "known", date = "2026-10-07", at = 1791000000000 } # Left 4 Dead
            620 = { vote = "would_try", date = "2026-10-07" }
            22370 = { vote = "known", date = "2026-10-08", bet = true }
            "#,
        )
        .unwrap();

        assert_eq!(read.known, BTreeSet::from([AppId(500), AppId(22370)]));
        assert_eq!(read.votes.keys().copied().collect::<Vec<_>>(), [AppId(620)]);
        let text = votes_text(&read.votes, |_| None);
        assert!(!text.contains("known"));
        assert!(!text.contains("500"));
    }

    #[test]
    fn parse_votes_orders_appids_numerically() {
        let votes = parse_votes(
            r#"
            [votes]
            100 = { vote = "later", date = "2026-10-07" }
            20 = { vote = "would_try", date = "2026-10-07" }
            3 = { vote = "later", date = "2026-10-07" }
            "#,
        )
        .unwrap()
        .votes;

        let appids: Vec<AppId> = votes.keys().copied().collect();
        assert_eq!(appids, [AppId(3), AppId(20), AppId(100)]);
    }

    #[test]
    fn parse_votes_without_the_section_is_empty() {
        assert_eq!(parse_votes("").unwrap(), ReadVotes::default());
    }

    #[test]
    fn parse_votes_rejects_an_appid_with_a_leading_zero() {
        let result = parse_votes(r#"votes = { "012" = { vote = "later", date = "2026-10-07" } }"#);
        assert!(matches!(result, Err(VotesError::InvalidAppId(key)) if key == "012"));
    }

    #[test]
    fn parse_votes_rejects_a_badly_shaped_date() {
        let result = parse_votes(r#"votes = { 10 = { vote = "later", date = "7/10/2026" } }"#);
        assert!(matches!(
            result,
            Err(VotesError::InvalidDate { appid: 10, .. })
        ));
    }

    #[test]
    fn parse_votes_rejects_bet_false() {
        let result =
            parse_votes(r#"votes = { 10 = { vote = "later", date = "2026-10-07", bet = false } }"#);
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
            parse_votes(r#"votes = { 10 = { vote = "later", date = "2026-10-07", note = "" } }"#);
        assert!(matches!(result, Err(VotesError::Toml(_))));
    }

    #[test]
    fn load_votes_without_a_file_is_empty() {
        let read = load_votes(Path::new("does-not-exist/votes.toml")).unwrap();
        assert_eq!(read, ReadVotes::default());
    }
}
