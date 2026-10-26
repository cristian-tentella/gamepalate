use std::path::Path;

use anyhow::bail;
use gamepalate_core::{
    catalog::Catalog,
    families::FamilyProblem,
    filters::TagNameError,
    ids::{AppId, TagId},
    off_catalog::OffCatalogGames,
    search::SearchInputs,
    taste::game_name,
};
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

pub fn tag_name(id: TagId, catalog: &Catalog) -> String {
    catalog
        .tags
        .get(&id)
        .map_or_else(|| format!("#{}", id.0), |tag| tag.english.clone())
}

pub fn tag_name_error_text(error: &TagNameError) -> String {
    match error {
        TagNameError::NotFound(name) => format!("no tag named \"{name}\""),
        TagNameError::Ambiguous(name) => {
            format!("more than one tag matches \"{name}\": write the full name")
        }
    }
}

pub fn game_label(id: AppId, catalog: &Catalog, off_catalog: &OffCatalogGames) -> String {
    let name = game_name(id, catalog, off_catalog);
    if name == format!("#{}", id.0) {
        name
    } else {
        format!("{name} ({})", id.0)
    }
}

pub fn family_problem_text(problem: &FamilyProblem, inputs: &SearchInputs) -> String {
    let game = |id| game_label(id, inputs.catalog, inputs.off_catalog);
    match problem {
        FamilyProblem::NameWithoutLetters { family } => {
            format!("family \"{family}\": the name has no letters")
        }
        FamilyProblem::Empty { family } => format!("family \"{family}\" has no games"),
        FamilyProblem::DuplicateGame { family, game: id } => {
            format!("family \"{family}\": {} appears twice", game(*id))
        }
        FamilyProblem::GameWithoutTags { family, game: id } => {
            format!("family \"{family}\": {} has no tags", game(*id))
        }
        FamilyProblem::EmptyProfile { family } => {
            format!("family \"{family}\": its tags give an empty profile")
        }
        FamilyProblem::DuplicateHeldOut { game: id } => {
            format!("{} is held out twice", game(*id))
        }
        FamilyProblem::HeldOutInFamily { game: id, family } => {
            format!("{} is held out and also in family \"{family}\"", game(*id))
        }
    }
}
