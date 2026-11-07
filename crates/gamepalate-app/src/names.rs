use gamepalate_core::{
    catalog::Catalog,
    families::FamilyProblem,
    ids::{AppId, TagId},
    off_catalog::OffCatalogGames,
    search::SearchInputs,
    taste::game_name,
};

pub fn tag_name(id: TagId, catalog: &Catalog) -> String {
    catalog
        .tags
        .get(&id)
        .map_or_else(|| format!("#{}", id.0), |tag| tag.english.clone())
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

pub fn folded(text: &str) -> String {
    text.chars()
        .flat_map(char::to_lowercase)
        .map(|c| match c {
            'à' | 'á' | 'â' | 'ã' | 'ä' | 'å' | 'ā' | 'ą' => 'a',
            'ç' | 'ć' | 'č' => 'c',
            'è' | 'é' | 'ê' | 'ë' | 'ē' | 'ę' | 'ě' => 'e',
            'ì' | 'í' | 'î' | 'ï' | 'ī' => 'i',
            'ñ' | 'ń' | 'ň' => 'n',
            'ò' | 'ó' | 'ô' | 'õ' | 'ö' | 'ø' | 'ō' => 'o',
            'ù' | 'ú' | 'û' | 'ü' | 'ū' | 'ů' => 'u',
            'ý' | 'ÿ' => 'y',
            'ś' | 'š' => 's',
            'ź' | 'ż' | 'ž' => 'z',
            'ł' => 'l',
            'ř' => 'r',
            other => other,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folding_ignores_case_and_accents() {
        assert_eq!(folded("Pokémon ÉTÉ Ørn Łódź"), "pokemon ete orn lodz");
        assert_eq!(folded("Hades II"), "hades ii");
    }
}
