use std::{collections::BTreeSet, fmt::Write as _, io::ErrorKind, path::Path};

use gamepalate_core::lists::{ListId, Lists};
use serde::Deserialize;

use crate::atomic::write_atomically;

const HEADER: &str = "\
# The lists of saved games, oldest first. votes.toml puts a game in a list with its id.
";

#[derive(Debug, thiserror::Error)]
pub enum ListsError {
    #[error("could not read the lists")]
    Io(#[from] std::io::Error),

    #[error("could not save the lists")]
    Write(#[source] std::io::Error),

    #[error("the lists file is not valid")]
    Toml(#[from] toml::de::Error),

    #[error("two lists have id {0}")]
    SameId(u32),

    #[error("two lists are called {0:?}")]
    SameName(String),

    #[error("the list with id {0} has no name")]
    NoName(u32),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListsFile {
    #[serde(default)]
    list: Vec<ListEntry>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ListEntry {
    id: u32,
    name: String,
}

/// The lists in `lists.toml`. Without the file there are none.
pub fn load_lists(path: &Path) -> Result<Lists, ListsError> {
    match std::fs::read_to_string(path) {
        Ok(text) => parse_lists(&text),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(Lists::new()),
        Err(error) => Err(error.into()),
    }
}

pub fn save_lists(path: &Path, lists: &Lists) -> Result<(), ListsError> {
    let text = lists_text(lists);
    parse_lists(&text)?;
    write_atomically(path, &text).map_err(ListsError::Write)
}

fn lists_text(lists: &Lists) -> String {
    let mut text = HEADER.to_string();
    for (id, name) in lists {
        let _ = write!(
            text,
            "\n[[list]]\nid = {}\nname = {}\n",
            id.0,
            toml::Value::String(name.clone())
        );
    }
    text
}

fn parse_lists(text: &str) -> Result<Lists, ListsError> {
    let file: ListsFile = toml::from_str(text)?;

    let mut lists = Lists::new();
    let mut names = BTreeSet::new();
    for entry in file.list {
        if entry.name.trim().is_empty() {
            return Err(ListsError::NoName(entry.id));
        }
        if !names.insert(entry.name.to_lowercase()) {
            return Err(ListsError::SameName(entry.name));
        }
        if lists.insert(ListId(entry.id), entry.name).is_some() {
            return Err(ListsError::SameId(entry.id));
        }
    }
    Ok(lists)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn two_lists() -> Lists {
        Lists::from([
            (ListId(2), "Co-op".to_string()),
            (ListId(5), "On sale \"under 10 €\"".to_string()),
        ])
    }

    #[test]
    fn the_lists_survive_saving_and_loading() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("lists.toml");

        save_lists(&path, &two_lists()).unwrap();

        assert_eq!(load_lists(&path).unwrap(), two_lists());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(HEADER));
        assert!(text.contains("[[list]]\nid = 2\nname = \"Co-op\"\n"));
    }

    #[test]
    fn the_lists_are_read_in_the_order_of_their_ids() {
        let lists = parse_lists(
            r#"
            [[list]]
            id = 5
            name = "On sale"

            [[list]]
            id = 2
            name = "Co-op"
            "#,
        )
        .unwrap();

        assert_eq!(
            lists.keys().copied().collect::<Vec<_>>(),
            [ListId(2), ListId(5)]
        );
    }

    #[test]
    fn without_the_file_there_are_no_lists() {
        assert_eq!(
            load_lists(Path::new("does-not-exist/lists.toml")).unwrap(),
            Lists::new()
        );
        assert_eq!(parse_lists("").unwrap(), Lists::new());
    }

    #[test]
    fn two_lists_cannot_have_the_same_id() {
        let result = parse_lists(
            r#"
            [[list]]
            id = 2
            name = "Co-op"
            [[list]]
            id = 2
            name = "On sale"
            "#,
        );
        assert!(matches!(result, Err(ListsError::SameId(2))));
    }

    #[test]
    fn two_lists_cannot_have_the_same_name_whatever_the_case() {
        let result = parse_lists(
            r#"
            [[list]]
            id = 2
            name = "Co-op"
            [[list]]
            id = 5
            name = "CO-OP"
            "#,
        );
        assert!(matches!(result, Err(ListsError::SameName(name)) if name == "CO-OP"));
    }

    #[test]
    fn a_list_needs_a_name() {
        for name in ["", "  "] {
            let result = parse_lists(&format!("[[list]]\nid = 3\nname = \"{name}\""));
            assert!(matches!(result, Err(ListsError::NoName(3))));
        }
    }

    #[test]
    fn an_unknown_key_is_refused() {
        let result = parse_lists("[[list]]\nid = 3\nname = \"Co-op\"\ncolor = \"red\"");
        assert!(matches!(result, Err(ListsError::Toml(_))));
    }
}
