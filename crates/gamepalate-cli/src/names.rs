use std::path::Path;

use anyhow::bail;
pub use gamepalate_app::names::{family_problem_text, game_label, tag_name};
use gamepalate_core::{filters::TagNameError, search::SearchInputs};
use gamepalate_store::data_files::DataFiles;

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
