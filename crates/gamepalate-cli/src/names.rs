use std::path::Path;

use anyhow::bail;
pub use gamepalate_app::names::{family_problem_text, game_label, tag_name};
use gamepalate_core::{families::Families, filters::TagNameError, search::SearchInputs};
use gamepalate_store::data_files::DataFiles;

pub fn is_automatic(name: &str, families: &Families) -> bool {
    families
        .families
        .iter()
        .any(|family| family.name == name && family.automatic.is_some())
}

pub fn family_label(name: &str, families: &Families) -> String {
    if is_automatic(name, families) {
        format!("{name} (automatic)")
    } else {
        name.to_string()
    }
}

pub fn check_families(
    files: &DataFiles,
    inputs: &SearchInputs,
    data_dir: &Path,
) -> anyhow::Result<()> {
    let problems = files.families.problems(
        &files.catalog,
        &files.off_catalog,
        &files.corrections.tag_multipliers,
    );
    if problems.is_empty() {
        return Ok(());
    }
    let lines: Vec<String> = problems
        .iter()
        .map(|problem| format!("  - {}", family_problem_text(problem, inputs)))
        .collect();
    bail!(
        "problems in {}:\n{}",
        data_dir.join("families.toml").display(),
        lines.join("\n")
    )
}

pub fn tag_name_error_text(error: &TagNameError) -> String {
    match error {
        TagNameError::NotFound(name) => format!("no tag named \"{name}\""),
        TagNameError::Ambiguous(name) => {
            format!("more than one tag matches \"{name}\": write the full name")
        }
    }
}

#[cfg(test)]
mod tests {
    use gamepalate_core::families::Family;

    use super::*;

    #[test]
    fn an_automatic_family_says_so_after_its_name() {
        let family = |name: &str, automatic: Option<&str>| Family {
            name: name.to_string(),
            games: Vec::new(),
            automatic: automatic.map(str::to_string),
        };
        let families = Families {
            families: vec![
                family("Puzzle · Relaxing", Some("2026-11-10")),
                family("mysteries", None),
            ],
            held_out: Vec::new(),
        };

        assert_eq!(
            family_label("Puzzle · Relaxing", &families),
            "Puzzle · Relaxing (automatic)"
        );
        assert_eq!(family_label("mysteries", &families), "mysteries");
        assert_eq!(family_label("Signals", &families), "Signals");
    }
}
