use std::path::{Path, PathBuf};

use gamepalate_core::{
    ids::AppId,
    lists::{ListId, ListNameError, Lists, MAX_NAME_CHARS, list_name, next_id},
    votes::Votes,
};
use gamepalate_store::{
    lists::{ListsError, save_lists},
    votes::save_votes,
};

use super::{Gamepalate, VoteError};
use crate::log::failed;

pub(crate) const LISTS_FILE: &str = "lists.toml";

/// Why a list was not made, renamed or deleted.
#[derive(Debug, thiserror::Error)]
pub enum ListError {
    #[error("a list needs a name")]
    NoName,
    #[error("a list name can have at most {MAX_NAME_CHARS} characters")]
    TooLong,
    #[error("A list with this name already exists")]
    Taken,
    #[error("the list is gone")]
    Gone,
    #[error("no new list fits after the list with the highest id in lists.toml")]
    Full,
    #[error("could not save the lists in {}", path.display())]
    Lists {
        path: PathBuf,
        #[source]
        source: ListsError,
    },
    #[error(transparent)]
    Vote(#[from] VoteError),
}

impl From<ListNameError> for ListError {
    fn from(error: ListNameError) -> ListError {
        match error {
            ListNameError::Empty => ListError::NoName,
            ListNameError::TooLong => ListError::TooLong,
            ListNameError::Taken => ListError::Taken,
        }
    }
}

fn saved(path: &Path, lists: Lists) -> Result<Lists, ListError> {
    save_lists(path, &lists).map_err(|source| ListError::Lists {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(lists)
}

/// Adds a list called `name` at the end, and gives its id.
fn created_list(path: &Path, lists: &mut Lists, name: &str) -> Result<ListId, ListError> {
    let name = list_name(lists, name, None)?;
    let id = next_id(lists).ok_or(ListError::Full)?;
    let mut changed = lists.clone();
    changed.insert(id, name);
    *lists = saved(path, changed)?;
    Ok(id)
}

/// Gives the list `id` a new name. Only `lists.toml` changes: the votes name the list by its id.
fn renamed_list(path: &Path, lists: &mut Lists, id: ListId, name: &str) -> Result<(), ListError> {
    let old = lists.get(&id).ok_or(ListError::Gone)?;
    let name = list_name(lists, name, Some(id))?;
    if *old == name {
        return Ok(());
    }
    let mut changed = lists.clone();
    changed.insert(id, name);
    *lists = saved(path, changed)?;
    Ok(())
}

/// Takes the list `id` out of every vote, then out of `lists.toml`: an id in the votes is
/// always in the lists, and a stop in between leaves at most a list without games. The games
/// stay saved. Each of `votes` and `lists` changes once its file is saved.
fn deleted_list(
    votes_path: &Path,
    lists_path: &Path,
    votes: &mut Votes,
    lists: &mut Lists,
    id: ListId,
    name: impl Fn(AppId) -> Option<String>,
) -> Result<(), ListError> {
    if !lists.contains_key(&id) {
        return Ok(());
    }
    if votes.values().any(|vote| vote.lists.contains(&id)) {
        let mut changed = votes.clone();
        for vote in changed.values_mut() {
            vote.lists.remove(&id);
        }
        save_votes(votes_path, &changed, name).map_err(|source| VoteError {
            path: votes_path.to_path_buf(),
            source,
        })?;
        *votes = changed;
    }
    let mut changed = lists.clone();
    changed.remove(&id);
    *lists = saved(lists_path, changed)?;
    Ok(())
}

impl Gamepalate {
    /// The lists of saved games, oldest first.
    pub fn lists(&self) -> Lists {
        self.lock_lists().clone()
    }

    /// Makes a new list, last of all, and gives its id. The name loses the spaces around it; an
    /// empty one, a long one or one in use is refused.
    pub fn create_list(&self, name: &str) -> Result<ListId, ListError> {
        let mut lists = self.write_lists();
        created_list(&self.data_dir.join(LISTS_FILE), &mut lists, name)
            .inspect_err(failed("could not create the list"))
    }

    pub fn rename_list(&self, id: ListId, name: &str) -> Result<(), ListError> {
        let mut lists = self.write_lists();
        renamed_list(&self.data_dir.join(LISTS_FILE), &mut lists, id, name)
            .inspect_err(failed("could not rename the list"))
    }

    /// Deletes the list. Its games stay saved. A list that is gone already changes nothing.
    pub fn delete_list(&self, id: ListId) -> Result<(), ListError> {
        let mut votes = self.write_votes();
        let mut lists = self.write_lists();
        deleted_list(
            &self.data_dir.join("votes.toml"),
            &self.data_dir.join(LISTS_FILE),
            &mut votes,
            &mut lists,
            id,
            |id| self.saved_name(id),
        )
        .inspect_err(failed("could not delete the list"))
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gamepalate_core::votes::{Vote, VoteKind};
    use gamepalate_store::{lists::load_lists, votes::load_votes};

    use super::*;

    fn co_op() -> Lists {
        Lists::from([
            (ListId(2), "Co-op".to_string()),
            (ListId(5), "On sale".to_string()),
        ])
    }

    #[test]
    fn a_new_list_goes_last_and_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LISTS_FILE);
        let mut lists = co_op();

        let id = created_list(&path, &mut lists, "  Steam Deck ").unwrap();

        assert_eq!(id, ListId(6));
        assert_eq!(lists[&id], "Steam Deck");
        assert_eq!(load_lists(&path).unwrap(), lists);
    }

    #[test]
    fn the_first_list_writes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LISTS_FILE);
        let mut lists = Lists::new();

        let id = created_list(&path, &mut lists, "Co-op").unwrap();

        assert_eq!(id, ListId(1));
        assert_eq!(load_lists(&path).unwrap(), lists);
    }

    #[test]
    fn no_list_is_made_after_an_id_that_cannot_grow() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LISTS_FILE);
        let mut lists = Lists::from([(ListId(u32::MAX), "Written by hand".to_string())]);

        let error = created_list(&path, &mut lists, "Co-op").unwrap_err();

        assert!(matches!(error, ListError::Full));
        assert_eq!(lists.len(), 1);
        assert!(!path.exists());
    }

    #[test]
    fn a_name_that_cannot_be_a_list_is_refused_and_nothing_is_saved() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LISTS_FILE);
        let mut lists = co_op();

        for (name, refused) in [
            ("  ", "a list needs a name"),
            (
                &"a".repeat(41),
                "a list name can have at most 40 characters",
            ),
            ("co-op", "A list with this name already exists"),
        ] {
            let error = created_list(&path, &mut lists, name).unwrap_err();
            assert_eq!(error.to_string(), refused);
            let error = renamed_list(&path, &mut lists, ListId(5), name).unwrap_err();
            assert_eq!(error.to_string(), refused);
        }
        assert_eq!(lists, co_op());
        assert!(!path.exists());
    }

    #[test]
    fn a_renamed_list_keeps_its_id() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LISTS_FILE);
        let mut lists = co_op();

        renamed_list(&path, &mut lists, ListId(2), "Co-op with Marco").unwrap();

        assert_eq!(lists[&ListId(2)], "Co-op with Marco");
        assert_eq!(load_lists(&path).unwrap(), lists);
    }

    #[test]
    fn a_list_can_be_renamed_to_another_case_of_its_name() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LISTS_FILE);
        let mut lists = co_op();

        renamed_list(&path, &mut lists, ListId(2), "CO-OP").unwrap();

        assert_eq!(lists[&ListId(2)], "CO-OP");
    }

    #[test]
    fn renaming_to_the_same_name_or_a_gone_list_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LISTS_FILE);
        let mut lists = co_op();

        renamed_list(&path, &mut lists, ListId(2), " Co-op ").unwrap();
        let error = renamed_list(&path, &mut lists, ListId(9), "Steam Deck").unwrap_err();

        assert!(matches!(error, ListError::Gone));
        assert!(!path.exists());
    }

    fn saved_in(lists: &[u32]) -> Vote {
        Vote {
            saved: true,
            lists: lists.iter().copied().map(ListId).collect(),
            ..Vote::new(VoteKind::WouldTry, "2026-11-25")
        }
    }

    fn three_votes() -> Votes {
        Votes::from([
            (AppId(10), saved_in(&[2, 5])),
            (AppId(20), saved_in(&[2])),
            (AppId(30), saved_in(&[5])),
        ])
    }

    #[test]
    fn a_deleted_list_leaves_every_vote_and_its_games_keep_the_save() {
        let dir = tempfile::tempdir().unwrap();
        let votes_path = dir.path().join("votes.toml");
        let lists_path = dir.path().join(LISTS_FILE);
        let mut votes = three_votes();
        let mut lists = co_op();

        deleted_list(
            &votes_path,
            &lists_path,
            &mut votes,
            &mut lists,
            ListId(2),
            |_| None,
        )
        .unwrap();

        assert_eq!(votes[&AppId(10)].lists, BTreeSet::from([ListId(5)]));
        assert!(votes[&AppId(20)].lists.is_empty());
        assert!(votes.values().all(|vote| vote.saved));
        assert_eq!(load_votes(&votes_path).unwrap().votes, votes);
        assert_eq!(
            load_lists(&lists_path).unwrap(),
            Lists::from([(ListId(5), "On sale".to_string())])
        );
    }

    #[test]
    fn an_empty_list_is_deleted_without_touching_the_votes() {
        let dir = tempfile::tempdir().unwrap();
        let votes_path = dir.path().join("votes.toml");
        let lists_path = dir.path().join(LISTS_FILE);
        let mut votes = Votes::from([(AppId(30), saved_in(&[5]))]);
        let mut lists = co_op();

        deleted_list(
            &votes_path,
            &lists_path,
            &mut votes,
            &mut lists,
            ListId(2),
            |_| None,
        )
        .unwrap();

        assert!(!votes_path.exists());
        assert!(!lists.contains_key(&ListId(2)));
    }

    #[test]
    fn deleting_a_gone_list_saves_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let votes_path = dir.path().join("votes.toml");
        let lists_path = dir.path().join(LISTS_FILE);
        let mut votes = three_votes();
        let mut lists = co_op();

        deleted_list(
            &votes_path,
            &lists_path,
            &mut votes,
            &mut lists,
            ListId(9),
            |_| None,
        )
        .unwrap();

        assert_eq!(votes, three_votes());
        assert!(!votes_path.exists() && !lists_path.exists());
    }

    #[test]
    fn when_the_lists_cannot_be_saved_the_votes_already_left_the_list() {
        let dir = tempfile::tempdir().unwrap();
        let votes_path = dir.path().join("votes.toml");
        let lists_path = dir.path().join("missing").join(LISTS_FILE);
        let mut votes = three_votes();
        let mut lists = co_op();

        let error = deleted_list(
            &votes_path,
            &lists_path,
            &mut votes,
            &mut lists,
            ListId(2),
            |_| None,
        )
        .unwrap_err();

        assert!(matches!(error, ListError::Lists { .. }));
        assert!(votes.values().all(|vote| !vote.lists.contains(&ListId(2))));
        assert_eq!(load_votes(&votes_path).unwrap().votes, votes);
        assert_eq!(lists, co_op());
    }

    #[test]
    fn when_the_votes_cannot_be_saved_nothing_changes() {
        let dir = tempfile::tempdir().unwrap();
        let votes_path = dir.path().join("missing").join("votes.toml");
        let lists_path = dir.path().join(LISTS_FILE);
        let mut votes = three_votes();
        let mut lists = co_op();

        let error = deleted_list(
            &votes_path,
            &lists_path,
            &mut votes,
            &mut lists,
            ListId(2),
            |_| None,
        )
        .unwrap_err();

        assert!(matches!(error, ListError::Vote(_)));
        assert_eq!(votes, three_votes());
        assert_eq!(lists, co_op());
        assert!(!lists_path.exists());
    }
}
