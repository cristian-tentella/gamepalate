use std::collections::BTreeSet;

use crate::{catalog::Catalog, ids::AppId, off_catalog::OffCatalogGames, taste::game_tags};

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Families {
    pub families: Vec<Family>,
    pub held_out: Vec<AppId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Family {
    pub name: String,
    pub games: Vec<AppId>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FamilyProblem {
    NameWithoutLetters { family: String },
    Empty { family: String },
    DuplicateGame { family: String, game: AppId },
    GameWithoutTags { family: String, game: AppId },
    DuplicateHeldOut { game: AppId },
    HeldOutInFamily { game: AppId, family: String },
}

impl FamilyProblem {
    pub fn family(&self) -> Option<&str> {
        match self {
            FamilyProblem::NameWithoutLetters { family }
            | FamilyProblem::Empty { family }
            | FamilyProblem::DuplicateGame { family, .. }
            | FamilyProblem::GameWithoutTags { family, .. }
            | FamilyProblem::HeldOutInFamily { family, .. } => Some(family),
            FamilyProblem::DuplicateHeldOut { .. } => None,
        }
    }
}

impl Families {
    // TODO: also report a family whose tag profile is empty.
    pub fn problems(&self, catalog: &Catalog, off_catalog: &OffCatalogGames) -> Vec<FamilyProblem> {
        let mut problems = Vec::new();

        for family in &self.families {
            if !family.name.chars().any(char::is_alphabetic) {
                problems.push(FamilyProblem::NameWithoutLetters {
                    family: family.name.clone(),
                });
            }
            if family.games.is_empty() {
                problems.push(FamilyProblem::Empty {
                    family: family.name.clone(),
                });
            }

            let mut seen = BTreeSet::new();
            for &game in &family.games {
                if !seen.insert(game) {
                    problems.push(FamilyProblem::DuplicateGame {
                        family: family.name.clone(),
                        game,
                    });
                } else if !has_tags(game, catalog, off_catalog) {
                    problems.push(FamilyProblem::GameWithoutTags {
                        family: family.name.clone(),
                        game,
                    });
                }
            }
        }

        let mut seen = BTreeSet::new();
        for &game in &self.held_out {
            if !seen.insert(game) {
                problems.push(FamilyProblem::DuplicateHeldOut { game });
                continue;
            }
            for family in &self.families {
                if family.games.contains(&game) {
                    problems.push(FamilyProblem::HeldOutInFamily {
                        game,
                        family: family.name.clone(),
                    });
                }
            }
        }

        problems
    }

    pub fn without_problems(mut self, problems: &[FamilyProblem]) -> Families {
        self.families.retain(|family| {
            !problems
                .iter()
                .any(|problem| problem.family() == Some(family.name.as_str()))
        });
        self
    }
}

fn has_tags(game: AppId, catalog: &Catalog, off_catalog: &OffCatalogGames) -> bool {
    game_tags(game, catalog, off_catalog).is_some_and(|tags| !tags.is_empty())
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeMap;

    use super::*;
    use crate::{
        catalog::Freshness,
        game::{Game, GameTag, Price, Reviews},
        ids::TagId,
        off_catalog::OffCatalogGame,
        timestamp::Timestamp,
    };

    fn tags(count: u32) -> Vec<GameTag> {
        (0..count)
            .map(|id| GameTag {
                id: TagId(id),
                weight: 1.0,
            })
            .collect()
    }

    // In the catalog, 1 has tags and 2 has none. Off the catalog, 10 has tags and 20 has none.
    fn catalog() -> Catalog {
        let game = |id, tag_count| Game {
            id: AppId(id),
            name: format!("game {id}"),
            price: Price {
                original_cents: 0,
                discount: None,
            },
            reviews: Reviews {
                total: 0,
                positive: 0,
            },
            is_early_access: false,
            tags: tags(tag_count),
        };
        Catalog::new(
            vec![game(1, 2), game(2, 0)],
            BTreeMap::new(),
            BTreeMap::new(),
            Freshness {
                prices_updated_at: Timestamp(0),
                tags_updated_at: Timestamp(0),
            },
        )
    }

    fn off_catalog() -> OffCatalogGames {
        let game = |tag_count| OffCatalogGame {
            tags: tags(tag_count),
            name: None,
        };
        BTreeMap::from([(AppId(10), game(1)), (AppId(20), game(0))])
    }

    fn family(name: &str, games: &[u32]) -> Family {
        Family {
            name: name.to_string(),
            games: games.iter().copied().map(AppId).collect(),
        }
    }

    fn problems_of(families: Vec<Family>, held_out: &[u32]) -> Vec<FamilyProblem> {
        let families = Families {
            families,
            held_out: held_out.iter().copied().map(AppId).collect(),
        };
        families.problems(&catalog(), &off_catalog())
    }

    #[test]
    fn problems_of_good_families_are_none() {
        let problems = problems_of(vec![family("mysteries", &[1, 10])], &[1000]);
        assert!(problems.is_empty());
    }

    #[test]
    fn problems_find_an_empty_family_and_a_name_without_letters() {
        let problems = problems_of(vec![family("2024", &[])], &[]);
        assert_eq!(
            problems,
            [
                FamilyProblem::NameWithoutLetters {
                    family: "2024".to_string()
                },
                FamilyProblem::Empty {
                    family: "2024".to_string()
                },
            ]
        );
    }

    #[test]
    fn problems_find_a_duplicate_game() {
        let problems = problems_of(vec![family("mysteries", &[1, 1])], &[]);
        assert_eq!(
            problems,
            [FamilyProblem::DuplicateGame {
                family: "mysteries".to_string(),
                game: AppId(1),
            }]
        );
    }

    #[test]
    fn problems_find_games_without_tags_in_and_off_the_catalog() {
        let problems = problems_of(vec![family("mysteries", &[2, 20, 99])], &[]);
        let games: Vec<AppId> = problems
            .iter()
            .map(|problem| match problem {
                FamilyProblem::GameWithoutTags { game, .. } => *game,
                other => panic!("unexpected problem {other:?}"),
            })
            .collect();
        assert_eq!(games, [AppId(2), AppId(20), AppId(99)]);
    }

    #[test]
    fn problems_find_held_out_games_that_are_doubled_or_in_a_family() {
        let problems = problems_of(vec![family("mysteries", &[1])], &[1, 5, 5]);
        assert_eq!(
            problems,
            [
                FamilyProblem::HeldOutInFamily {
                    game: AppId(1),
                    family: "mysteries".to_string(),
                },
                FamilyProblem::DuplicateHeldOut { game: AppId(5) },
            ]
        );
    }

    #[test]
    fn without_problems_drops_only_the_families_with_a_problem() {
        let families = Families {
            families: vec![family("good", &[1]), family("bad", &[])],
            held_out: vec![AppId(5), AppId(5)],
        };
        let problems = families.problems(&catalog(), &off_catalog());

        let kept = families.without_problems(&problems);
        assert_eq!(kept.families, [family("good", &[1])]);
    }
}
