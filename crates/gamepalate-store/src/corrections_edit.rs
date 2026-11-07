use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use gamepalate_core::corrections::Corrections;
use toml_edit::{DocumentMut, Item, Table, Value};

use crate::corrections::{CorrectionsError, parse_corrections};

const NEW_FILE: &str = "\
# Corrections to the recommendations. Every key is a Steam id, the comment after it is only a reminder.
#
# [banned_tags]       tagid = true bans a tag, tagid = N bans it only within a game's first N tags
# [tag_multipliers]   tagid = factor, above 1 boosts a tag, below 1 dampens it
# [hidden_games]      appid = true hides a game from the recommendations
# [game_multipliers]  appid = factor from 0 to 1 dampens a game of your signals
# [played_elsewhere]  appid = true counts a game you played outside Steam

[banned_tags]

[tag_multipliers]

[hidden_games]

[game_multipliers]

[played_elsewhere]
";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
    BannedTags,
    TagMultipliers,
    HiddenGames,
    GameMultipliers,
    PlayedElsewhere,
}

impl Section {
    pub fn name(self) -> &'static str {
        match self {
            Section::BannedTags => "banned_tags",
            Section::TagMultipliers => "tag_multipliers",
            Section::HiddenGames => "hidden_games",
            Section::GameMultipliers => "game_multipliers",
            Section::PlayedElsewhere => "played_elsewhere",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum EntryValue {
    Bool(bool),
    Integer(i64),
    Float(f64),
}

impl From<EntryValue> for Value {
    fn from(value: EntryValue) -> Value {
        match value {
            EntryValue::Bool(value) => value.into(),
            EntryValue::Integer(value) => value.into(),
            EntryValue::Float(value) => value.into(),
        }
    }
}

pub struct CorrectionsEditor {
    path: PathBuf,
    document: DocumentMut,
}

impl CorrectionsEditor {
    pub fn open(path: &Path) -> Result<CorrectionsEditor, CorrectionsError> {
        let text = match std::fs::read_to_string(path) {
            Ok(text) => text,
            Err(error) if error.kind() == ErrorKind::NotFound => NEW_FILE.to_string(),
            Err(error) => return Err(error.into()),
        };
        parse_corrections(&text)?;
        let document = text.parse::<DocumentMut>()?;
        Ok(CorrectionsEditor {
            path: path.to_path_buf(),
            document,
        })
    }

    pub fn set(&mut self, section: Section, id: u32, value: EntryValue, comment: Option<&str>) {
        let key = id.to_string();
        let mut value = Value::from(value);
        let table = self.section(section);
        match table.get_mut(&key) {
            Some(Item::Value(old)) => {
                *value.decor_mut() = old.decor().clone();
                *old = value;
            }
            _ => {
                if let Some(comment) = comment {
                    value
                        .decor_mut()
                        .set_suffix(format!(" # {}", comment.replace(['\r', '\n'], " ")));
                }
                table.insert(&key, Item::Value(value));
            }
        }
    }

    pub fn remove(&mut self, section: Section, id: u32) -> bool {
        let key = id.to_string();
        let Some(table) = self
            .document
            .get_mut(section.name())
            .and_then(Item::as_table_like_mut)
        else {
            return false;
        };
        let Some(prefix) = table
            .key(&key)
            .map(|key| decor_prefix(key.leaf_decor()).to_string())
        else {
            return false;
        };

        let next = table
            .iter()
            .map(|(other, _)| other.to_string())
            .skip_while(|other| *other != key)
            .nth(1);
        table.remove(&key);

        if prefix.contains('#')
            && let Some(mut next) = next.and_then(|next| table.key_mut(&next))
        {
            let joined = format!("{prefix}{}", decor_prefix(next.leaf_decor()));
            next.leaf_decor_mut().set_prefix(joined);
        }
        true
    }

    pub fn save(self) -> Result<Corrections, CorrectionsError> {
        let text = self.document.to_string();
        let corrections = parse_corrections(&text)?;

        let mut temporary = self.path.clone().into_os_string();
        temporary.push(".tmp");
        let temporary = PathBuf::from(temporary);
        std::fs::write(&temporary, &text).map_err(CorrectionsError::Write)?;
        std::fs::rename(&temporary, &self.path).map_err(|error| {
            let _ = std::fs::remove_file(&temporary);
            CorrectionsError::Write(error)
        })?;

        Ok(corrections)
    }

    fn section(&mut self, section: Section) -> &mut dyn toml_edit::TableLike {
        let empty = self.document.to_string().trim().is_empty();
        let item = self.document.entry(section.name()).or_insert_with(|| {
            let mut table = Table::new();
            if !empty {
                table.decor_mut().set_prefix("\n");
            }
            Item::Table(table)
        });
        item.as_table_like_mut()
            .expect("a section that parsed as corrections is a table")
    }
}

fn decor_prefix(decor: &toml_edit::Decor) -> &str {
    decor
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .unwrap_or("")
}

#[cfg(test)]
mod tests {
    use gamepalate_core::ids::AppId;

    use super::*;

    const WITH_COMMENTS: &str = "# my corrections

[hidden_games]
# a comment
100 = true

[played_elsewhere]
# the co-op games
200 = true
";

    fn written(text: &str) -> (tempfile::TempDir, PathBuf) {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("corrections.toml");
        std::fs::write(&path, text).unwrap();
        (dir, path)
    }

    #[test]
    fn a_game_played_elsewhere_is_added_without_touching_the_comments() {
        let (_dir, path) = written(WITH_COMMENTS);

        let mut editor = CorrectionsEditor::open(&path).unwrap();
        editor.set(Section::PlayedElsewhere, 300, EntryValue::Bool(true), None);
        let corrections = editor.save().unwrap();

        assert_eq!(
            corrections.played_elsewhere.iter().collect::<Vec<_>>(),
            [&AppId(200), &AppId(300)]
        );
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            format!("{WITH_COMMENTS}300 = true\n")
        );
    }

    #[test]
    fn a_game_played_elsewhere_is_removed_without_touching_the_comments() {
        let (_dir, path) = written(WITH_COMMENTS);

        let mut editor = CorrectionsEditor::open(&path).unwrap();
        assert!(editor.remove(Section::PlayedElsewhere, 200));
        let corrections = editor.save().unwrap();

        assert!(corrections.played_elsewhere.is_empty());
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.starts_with("# my corrections\n\n[hidden_games]\n# a comment\n100 = true"));
        assert!(text.contains("[played_elsewhere]"));
        assert!(!text.contains("200"));
    }

    #[test]
    fn the_section_is_created_in_a_file_without_it() {
        let (_dir, path) = written("[hidden_games]\n100 = true\n");

        let mut editor = CorrectionsEditor::open(&path).unwrap();
        editor.set(Section::PlayedElsewhere, 300, EntryValue::Bool(true), None);
        editor.save().unwrap();

        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "[hidden_games]\n100 = true\n\n[played_elsewhere]\n300 = true\n"
        );
    }
}
