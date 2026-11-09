use std::{
    io::ErrorKind,
    path::{Path, PathBuf},
};

use gamepalate_core::{families::Families, ids::AppId};
use toml_edit::{Array, DocumentMut, Item, Table, TableLike, Value};

use crate::{
    atomic::write_atomically,
    families::{FamiliesError, parse_families},
};

pub const NEW_FILE: &str = "\
# Your families. Each one is a taste, made of the Steam appids of the games that stand for it.
# Discover takes turns among them. [held_out] lists games you like, kept out on purpose to test it.
";

const INDENT: &str = "    ";
const AUTOMATIC: &str = "automatic";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FamiliesSnapshot(Option<String>);

pub struct FamiliesEditor {
    path: PathBuf,
    before: FamiliesSnapshot,
    document: DocumentMut,
    leading: String,
}

impl FamiliesEditor {
    pub fn open(path: &Path) -> Result<FamiliesEditor, FamiliesError> {
        let before = match std::fs::read_to_string(path) {
            Ok(text) => Some(text),
            Err(error) if error.kind() == ErrorKind::NotFound => None,
            Err(error) => return Err(error.into()),
        };
        let text = match before.as_deref() {
            Some(text) if !text.trim().is_empty() => text,
            _ => NEW_FILE,
        };
        parse_families(text)?;
        let mut document = text.parse::<DocumentMut>()?;

        let mut leading = String::new();
        if document.is_empty() {
            leading = document.trailing().as_str().unwrap_or("").to_string();
            document.set_trailing("");
        }

        Ok(FamiliesEditor {
            path: path.to_path_buf(),
            before: FamiliesSnapshot(before),
            document,
            leading,
        })
    }

    pub fn add_game(&mut self, family: &str, id: AppId, comment: &str) {
        if let Some(games) = self.family_games(family) {
            push_game(games, id, comment);
            self.make_own(family);
            return;
        }
        let table = self.new_table(id, comment);
        self.families().insert(family, Item::Table(table));
    }

    pub fn remove_game(&mut self, family: &str, id: AppId) -> bool {
        let removed = self
            .family_games(family)
            .is_some_and(|games| remove_game(games, id));
        if removed {
            self.make_own(family);
        }
        removed
    }

    pub fn rename(&mut self, from: &str, to: &str) -> bool {
        let families = self.families();
        let Some(item) = families.remove(from) else {
            return false;
        };
        families.insert(to, item);
        self.make_own(to);
        true
    }

    pub fn set_automatic(&mut self, family: &str, date: &str) -> bool {
        let Some(table) = self.family_table(family) else {
            return false;
        };
        table.insert(AUTOMATIC, toml_edit::value(date));
        true
    }

    // A comment line at the top of a new file, under the usual ones.
    pub fn add_heading(&mut self, line: &str) {
        if self.document.is_empty() {
            self.leading
                .push_str(&format!("# {}\n", line.replace(['\r', '\n'], " ")));
        }
    }

    pub fn delete(&mut self, family: &str) -> bool {
        self.families().remove(family).is_some()
    }

    pub fn hold_out(&mut self, id: AppId, comment: &str) {
        if let Some(games) = self.held_out_games() {
            push_game(games, id, comment);
            return;
        }
        let table = self.new_table(id, comment);
        self.document.insert("held_out", Item::Table(table));
    }

    pub fn release(&mut self, id: AppId) -> bool {
        self.held_out_games()
            .is_some_and(|games| remove_game(games, id))
    }

    pub fn save(mut self) -> Result<(Families, FamiliesSnapshot), FamiliesError> {
        if self.document.is_empty() {
            self.document.set_trailing(self.leading);
        }
        let text = self.document.to_string();
        let families = parse_families(&text)?;
        write_atomically(&self.path, &text).map_err(FamiliesError::Write)?;
        Ok((families, self.before))
    }

    fn families(&mut self) -> &mut dyn TableLike {
        self.document
            .entry("families")
            .or_insert_with(|| {
                let mut table = Table::new();
                table.set_implicit(true);
                Item::Table(table)
            })
            .as_table_like_mut()
            .expect("families that parsed are a table")
    }

    fn family_table(&mut self, family: &str) -> Option<&mut dyn TableLike> {
        self.families().get_mut(family)?.as_table_like_mut()
    }

    fn family_games(&mut self, family: &str) -> Option<&mut Array> {
        games(self.family_table(family)?)
    }

    // A family the user changed is theirs: it loses the day gamepalate made it.
    fn make_own(&mut self, family: &str) {
        if let Some(table) = self.family_table(family) {
            table.remove(AUTOMATIC);
        }
    }

    fn held_out_games(&mut self) -> Option<&mut Array> {
        let table = self.document.get_mut("held_out")?.as_table_like_mut()?;
        games(table)
    }

    fn new_table(&mut self, id: AppId, comment: &str) -> Table {
        let mut table = Table::new();
        let prefix = if self.leading.is_empty() && self.document.is_empty() {
            String::new()
        } else {
            format!("{}\n", std::mem::take(&mut self.leading))
        };
        table.decor_mut().set_prefix(prefix);
        let mut games = Array::new();
        push_game(&mut games, id, comment);
        table.insert("games", Item::Value(Value::Array(games)));
        table
    }
}

pub fn restore_families(
    path: &Path,
    snapshot: &FamiliesSnapshot,
) -> Result<Families, FamiliesError> {
    match &snapshot.0 {
        Some(text) => {
            let families = parse_families(text)?;
            write_atomically(path, text).map_err(FamiliesError::Write)?;
            Ok(families)
        }
        None => match std::fs::remove_file(path) {
            Ok(()) => Ok(Families::default()),
            Err(error) if error.kind() == ErrorKind::NotFound => Ok(Families::default()),
            Err(error) => Err(FamiliesError::Write(error)),
        },
    }
}

fn games(table: &mut dyn TableLike) -> Option<&mut Array> {
    table
        .entry("games")
        .or_insert(Item::Value(Value::Array(Array::new())))
        .as_array_mut()
}

fn push_game(games: &mut Array, id: AppId, comment: &str) {
    let multiline = games.is_empty() || games.iter().any(|game| prefix(game).contains('\n'));
    if !multiline {
        games.push(i64::from(id.0));
        return;
    }

    let indent = games
        .iter()
        .last()
        .map_or(INDENT, |game| after_last_line(prefix(game)));
    let trailing = games.trailing().as_str().unwrap_or("");
    let (head, rest) = split_first_line(trailing);
    let rest = if rest.is_empty() { "\n" } else { rest };

    let mut value = Value::from(i64::from(id.0));
    value.decor_mut().set_prefix(format!("{head}\n{indent}"));
    value.decor_mut().set_suffix("");
    let trailing = if comment.is_empty() {
        rest.to_string()
    } else {
        format!(" # {}{rest}", comment.replace(['\r', '\n'], " "))
    };

    games.push_formatted(value);
    games.set_trailing(trailing);
    games.set_trailing_comma(true);
}

fn remove_game(games: &mut Array, id: AppId) -> bool {
    let Some(index) = games
        .iter()
        .position(|game| game.as_integer() == Some(i64::from(id.0)))
    else {
        return false;
    };
    let removed = prefix(&games.remove(index)).to_string();

    let Some(line) = removed.rfind('\n') else {
        if let Some(next) = games.get_mut(index) {
            next.decor_mut().set_prefix(removed);
        }
        return true;
    };
    let (kept, own_line) = removed.split_at(line);

    match games.get_mut(index) {
        Some(next) => {
            let (_, rest) = split_first_line(prefix(next));
            let rest = if rest.is_empty() { own_line } else { rest };
            let joined = format!("{kept}{rest}");
            next.decor_mut().set_prefix(joined);
        }
        None => {
            let (_, rest) = split_first_line(games.trailing().as_str().unwrap_or(""));
            let joined = format!("{kept}{rest}");
            games.set_trailing(joined);
        }
    }
    true
}

fn prefix(value: &Value) -> &str {
    value
        .decor()
        .prefix()
        .and_then(|prefix| prefix.as_str())
        .unwrap_or("")
}

fn split_first_line(text: &str) -> (&str, &str) {
    text.find('\n')
        .map_or((text, ""), |line| text.split_at(line))
}

fn after_last_line(text: &str) -> &str {
    text.rfind('\n').map_or(text, |line| &text[line + 1..])
}

#[cfg(test)]
mod tests {
    use super::*;

    const FILE: &str = "\
# My tastes

# Slow games
[families.cozy]
games = [
    413150, # Stardew Valley
    1055540, # A Short Hike
]

[families.mysteries]
games = [10, 20]

[held_out]
games = [
    30, # Outer Wilds
]
";

    fn edited(text: &str, edit: impl FnOnce(&mut FamiliesEditor)) -> String {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");
        if !text.is_empty() {
            std::fs::write(&path, text).unwrap();
        }
        let mut editor = FamiliesEditor::open(&path).unwrap();
        edit(&mut editor);
        editor.save().unwrap();
        std::fs::read_to_string(&path).unwrap()
    }

    #[test]
    fn a_game_goes_at_the_end_of_its_family_with_its_name() {
        let text = edited(FILE, |editor| {
            editor.add_game("cozy", AppId(504230), "Celeste");
        });

        assert!(text.contains(
            "games = [
    413150, # Stardew Valley
    1055540, # A Short Hike
    504230, # Celeste
]"
        ));
        assert!(text.starts_with("# My tastes\n\n# Slow games\n[families.cozy]"));
    }

    #[test]
    fn a_one_line_list_stays_on_one_line() {
        let text = edited(FILE, |editor| {
            editor.add_game("mysteries", AppId(40), "Return of the Obra Dinn");
        });
        assert!(text.contains("games = [10, 20, 40]"));
    }

    #[test]
    fn a_removed_game_takes_its_comment_with_it() {
        let text = edited(FILE, |editor| {
            assert!(editor.remove_game("cozy", AppId(413150)));
            assert!(!editor.remove_game("cozy", AppId(1)));
        });
        assert!(text.contains("games = [\n    1055540, # A Short Hike\n]"));

        let text = edited(FILE, |editor| {
            assert!(editor.remove_game("cozy", AppId(1055540)));
        });
        assert!(text.contains("games = [\n    413150, # Stardew Valley\n]"));

        let text = edited(FILE, |editor| {
            assert!(editor.remove_game("mysteries", AppId(10)));
        });
        assert!(text.contains("games = [20]"));
    }

    #[test]
    fn a_new_family_goes_at_the_end() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");
        std::fs::write(&path, FILE).unwrap();
        let mut editor = FamiliesEditor::open(&path).unwrap();
        editor.add_game("slow puzzles", AppId(50), "The Witness");
        let (families, _) = editor.save().unwrap();

        let names: Vec<&str> = families.families.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["cozy", "mysteries", "slow puzzles"]);
        assert!(
            std::fs::read_to_string(&path)
                .unwrap()
                .contains("[families.\"slow puzzles\"]\ngames = [\n    50, # The Witness\n]")
        );
    }

    #[test]
    fn a_renamed_family_keeps_its_place_and_comments() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");
        std::fs::write(&path, FILE).unwrap();
        let mut editor = FamiliesEditor::open(&path).unwrap();
        assert!(editor.rename("cozy", "calm"));
        assert!(!editor.rename("puzzles", "riddles"));
        let (families, _) = editor.save().unwrap();

        assert_eq!(families.families[0].name, "calm");
        assert_eq!(families.families[0].games, [AppId(413150), AppId(1055540)]);
        let text = std::fs::read_to_string(&path).unwrap();
        assert_eq!(text, FILE.replace("[families.cozy]", "[families.calm]"));
    }

    #[test]
    fn a_deleted_family_leaves_the_others() {
        let text = edited(FILE, |editor| {
            assert!(editor.delete("cozy"));
            assert!(!editor.delete("cozy"));
        });
        assert!(!text.contains("cozy"));
        assert!(!text.contains("Slow games"));
        assert!(text.contains("[families.mysteries]\ngames = [10, 20]"));
    }

    #[test]
    fn held_out_games_are_added_and_released() {
        let text = edited(FILE, |editor| {
            editor.hold_out(AppId(60), "Tunic");
            assert!(editor.release(AppId(30)));
        });
        assert!(text.ends_with("[held_out]\ngames = [\n    60, # Tunic\n]\n"));
    }

    #[test]
    fn a_new_file_starts_with_the_comments() {
        let text = edited("", |editor| {
            editor.add_game("cozy", AppId(413150), "Stardew Valley");
            editor.hold_out(AppId(30), "Outer Wilds");
        });
        assert_eq!(
            text,
            format!(
                "{NEW_FILE}\n[families.cozy]\ngames = [\n    413150, # Stardew Valley\n]\n\n[held_out]\ngames = [\n    30, # Outer Wilds\n]\n"
            )
        );
    }

    #[test]
    fn a_game_without_a_name_has_no_comment() {
        let text = edited(FILE, |editor| editor.hold_out(AppId(70), ""));
        assert!(text.ends_with("    30, # Outer Wilds\n    70,\n]\n"));
    }

    #[test]
    fn a_family_without_games_gets_its_list() {
        let text = edited("[families.empty]\n", |editor| {
            editor.add_game("empty", AppId(1), "One");
        });
        assert!(text.contains("[families.empty]\ngames = [\n    1, # One\n]"));
    }

    const AUTOMATIC_FILE: &str = "\
[families.cozy]
games = [
    1, # One
    2, # Two
]
automatic = \"2026-11-10\"

[families.mysteries]
games = [10, 20]
automatic = \"2026-11-10\"
";

    #[test]
    fn a_new_file_gets_its_heading_and_the_day_of_the_automatic_families() {
        let text = edited("", |editor| {
            editor.add_heading("Made from the games you play on 2026-11-10.");
            editor.add_game("cozy", AppId(1), "One");
            editor.add_game("cozy", AppId(2), "Two");
            assert!(editor.set_automatic("cozy", "2026-11-10"));
            assert!(!editor.set_automatic("roguelike", "2026-11-10"));
        });
        assert_eq!(
            text,
            format!(
                "{NEW_FILE}# Made from the games you play on 2026-11-10.\n\n[families.cozy]\ngames = [\n    1, # One\n    2, # Two\n]\nautomatic = \"2026-11-10\"\n"
            )
        );
    }

    #[test]
    fn changing_an_automatic_family_makes_it_your_own() {
        let without_day = |edit: fn(&mut FamiliesEditor)| {
            let text = edited(AUTOMATIC_FILE, edit);
            parse_families(&text)
                .unwrap()
                .families
                .iter()
                .map(|family| family.automatic.is_none())
                .collect::<Vec<bool>>()
        };

        assert_eq!(
            without_day(|editor| editor.add_game("cozy", AppId(3), "Three")),
            [true, false]
        );
        assert_eq!(
            without_day(|editor| {
                editor.remove_game("cozy", AppId(1));
            }),
            [true, false]
        );
        assert_eq!(
            without_day(|editor| {
                editor.rename("mysteries", "riddles");
            }),
            [false, true]
        );
        assert_eq!(
            without_day(|editor| {
                editor.remove_game("cozy", AppId(99));
                editor.hold_out(AppId(30), "Outer Wilds");
            }),
            [false, false]
        );
    }

    #[test]
    fn a_change_keeps_the_comments_of_an_automatic_family() {
        let text = edited(AUTOMATIC_FILE, |editor| {
            editor.remove_game("cozy", AppId(1));
        });
        assert!(
            text.starts_with("[families.cozy]\ngames = [\n    2, # Two\n]\n\n[families.mysteries]")
        );
    }

    #[test]
    fn restoring_puts_back_the_text_or_removes_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");
        std::fs::write(&path, FILE).unwrap();
        let mut editor = FamiliesEditor::open(&path).unwrap();
        editor.delete("cozy");
        let (_, before) = editor.save().unwrap();

        let families = restore_families(&path, &before).unwrap();
        assert_eq!(families.families.len(), 2);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), FILE);

        let new = dir.path().join("new.toml");
        let mut editor = FamiliesEditor::open(&new).unwrap();
        editor.add_game("cozy", AppId(1), "One");
        let (_, before) = editor.save().unwrap();
        assert_eq!(
            restore_families(&new, &before).unwrap(),
            Families::default()
        );
        assert!(!new.exists());
    }

    #[test]
    fn an_invalid_file_is_not_opened() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("families.toml");
        std::fs::write(&path, "[kept_aside]\ngames = [1]").unwrap();
        assert!(FamiliesEditor::open(&path).is_err());
    }
}
