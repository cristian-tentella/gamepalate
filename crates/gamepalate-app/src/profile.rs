use std::collections::BTreeMap;

use gamepalate_core::{
    config::SignalsConfig,
    corrections::Ban,
    effective_hours::{HoursSource, effective_hours},
    ids::AppId,
    opinion::{Labels, MIN_LABELS, labels},
    profile::{ProfileTag, corrected_games, dominant_game, family_profiles, profile},
    search::SearchInputs,
    taste::game_name,
    timestamp::Timestamp,
    votes::VoteKind,
};

use crate::names::{family_problem_text, tag_name};

const PROFILE_TAGS: usize = 25;
const SHAPING_GAMES: usize = 5;

#[derive(Debug, Clone, PartialEq)]
pub struct ProfileView {
    pub taste: Option<TasteView>,
    pub families: Vec<FamilyView>,
    pub held_out: usize,
    pub family_problems: Vec<String>,
    pub votes: VotesView,
    pub corrections: CorrectionsView,
    pub hours: SignalsConfig,
    pub catalog_games: usize,
    pub prices_updated_at: Timestamp,
    pub tags_updated_at: Timestamp,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TasteView {
    pub game_count: usize,
    pub recent_games: usize,
    pub sources: Vec<(HoursSource, f64)>,
    pub shaping_games: Vec<HoursGame>,
    pub tags: Vec<TagView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct HoursGame {
    pub id: AppId,
    pub name: String,
    pub hours: f64,
    pub source: HoursSource,
}

#[derive(Debug, Clone, PartialEq)]
pub struct TagView {
    pub name: String,
    pub weight: f64,
    pub weight_before: f64,
    pub multiplier: Option<f64>,
    pub banned: Option<Ban>,
    pub games: Vec<HoursGame>,
    pub game_count: usize,
    pub dominant: bool,
}

#[derive(Debug, Clone, PartialEq)]
pub struct FamilyView {
    pub name: String,
    pub games: Vec<AppId>,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VotesView {
    pub would_try: usize,
    pub maybe: usize,
    pub not_for_me: usize,
    pub known: usize,
    pub labels: Labels,
    pub min_labels: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct CorrectionsView {
    pub tag_multipliers: Vec<(String, f64)>,
    pub banned_tags: Vec<(String, Ban)>,
    pub game_multipliers: Vec<CorrectedGameView>,
    pub hidden_games: Vec<String>,
}

impl CorrectionsView {
    pub fn is_empty(&self) -> bool {
        self.tag_multipliers.is_empty()
            && self.banned_tags.is_empty()
            && self.game_multipliers.is_empty()
            && self.hidden_games.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct CorrectedGameView {
    pub name: String,
    pub multiplier: f64,
    pub in_effect: bool,
}

pub(crate) fn profile_view(inputs: &SearchInputs) -> ProfileView {
    let catalog = inputs.catalog;
    let name = |id| game_name(id, catalog, inputs.off_catalog);

    let taste = profile(inputs, PROFILE_TAGS).map(|profile| {
        let hours: BTreeMap<AppId, (f64, HoursSource)> = profile
            .games
            .iter()
            .map(|game| (game.id, (game.hours, game.source)))
            .collect();
        let hours_game = |id: AppId| {
            let (hours, source) = hours
                .get(&id)
                .copied()
                .unwrap_or((0.0, HoursSource::Played));
            HoursGame {
                id,
                name: name(id),
                hours,
                source,
            }
        };

        TasteView {
            game_count: profile.games.len(),
            recent_games: profile.recent_games,
            sources: profile.source_shares(),
            shaping_games: profile
                .games
                .iter()
                .take(SHAPING_GAMES)
                .map(|game| hours_game(game.id))
                .collect(),
            tags: profile
                .tags
                .iter()
                .map(|tag| tag_view(tag, inputs, &hours_game))
                .collect(),
        }
    });

    let family_problems = inputs.families.problems(
        catalog,
        inputs.off_catalog,
        &inputs.corrections.tag_multipliers,
    );

    ProfileView {
        taste,
        families: family_profiles(inputs, &family_problems)
            .into_iter()
            .map(|family| FamilyView {
                name: family.name,
                games: family.games,
                tags: family
                    .tags
                    .iter()
                    .map(|&(tag, _)| tag_name(tag, catalog))
                    .collect(),
            })
            .collect(),
        held_out: inputs.families.held_out.len(),
        family_problems: family_problems
            .iter()
            .map(|problem| family_problem_text(problem, inputs))
            .collect(),
        votes: votes_view(inputs),
        corrections: corrections_view(inputs),
        hours: inputs.config.signals,
        catalog_games: catalog.games().len(),
        prices_updated_at: catalog.freshness.prices_updated_at,
        tags_updated_at: catalog.freshness.tags_updated_at,
    }
}

fn tag_view(
    tag: &ProfileTag,
    inputs: &SearchInputs,
    hours_game: &impl Fn(AppId) -> HoursGame,
) -> TagView {
    TagView {
        name: tag_name(tag.id, inputs.catalog),
        weight: tag.weight,
        weight_before: tag.weight_before,
        multiplier: tag.correction.as_ref().and_then(|c| c.multiplier),
        banned: tag.correction.as_ref().and_then(|c| c.banned),
        games: tag.games.iter().map(|game| hours_game(game.id)).collect(),
        game_count: tag.game_count,
        dominant: dominant_game(tag).is_some(),
    }
}

fn votes_view(inputs: &SearchInputs) -> VotesView {
    let count = |kind| {
        inputs
            .votes
            .values()
            .filter(|vote| vote.kind == kind)
            .count()
    };

    VotesView {
        would_try: count(VoteKind::WouldTry),
        maybe: count(VoteKind::Maybe),
        not_for_me: count(VoteKind::NotForMe),
        known: count(VoteKind::Known),
        labels: labels(inputs.votes, inputs.catalog),
        min_labels: MIN_LABELS,
    }
}

fn corrections_view(inputs: &SearchInputs) -> CorrectionsView {
    let corrections = inputs.corrections;
    let catalog = inputs.catalog;
    let name = |id| game_name(id, catalog, inputs.off_catalog);
    let hours = effective_hours(inputs.signals, inputs.votes, &inputs.config.signals);

    let mut view = CorrectionsView {
        tag_multipliers: corrections
            .tag_multipliers
            .iter()
            .map(|(&tag, &multiplier)| (tag_name(tag, catalog), multiplier))
            .collect(),
        banned_tags: corrections
            .banned_tags
            .iter()
            .map(|(&tag, &ban)| (tag_name(tag, catalog), ban))
            .collect(),
        game_multipliers: corrected_games(&hours, inputs)
            .into_iter()
            .map(|game| CorrectedGameView {
                name: name(game.id),
                multiplier: game.multiplier,
                in_effect: game.in_effect,
            })
            .collect(),
        hidden_games: corrections
            .hidden_games
            .iter()
            .map(|&id| name(id))
            .collect(),
    };
    view.tag_multipliers
        .sort_by_cached_key(|(name, _)| name.to_lowercase());
    view.banned_tags
        .sort_by_cached_key(|(name, _)| name.to_lowercase());
    view.game_multipliers
        .sort_by_cached_key(|game| game.name.to_lowercase());
    view.hidden_games
        .sort_by_cached_key(|name| name.to_lowercase());
    view
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use gamepalate_core::{
        catalog::{Catalog, Freshness, TagName},
        config::Config,
        corrections::Corrections,
        currency::Currency,
        families::{Families, Family},
        game::{Game, GameTag, Price, Reviews},
        ids::TagId,
        off_catalog::OffCatalogGames,
        region::Region,
        signals::{LibraryGame, Signals},
        votes::{Vote, Votes},
    };

    use super::*;

    fn game(id: u32, name: &str, tags: &[u32]) -> Game {
        Game {
            id: AppId(id),
            name: name.to_string(),
            price: Price {
                original_cents: 999,
                discount: None,
            },
            reviews: Reviews {
                total: 5000,
                positive: 4500,
            },
            is_early_access: false,
            tags: tags
                .iter()
                .map(|&tag| GameTag {
                    id: TagId(tag),
                    weight: 1.0,
                })
                .collect(),
        }
    }

    fn catalog() -> Catalog {
        let names = [
            "Exploration",
            "Puzzle",
            "Pixel Graphics",
            "Roguelike",
            "JRPG",
        ];
        Catalog::new(
            vec![
                game(1, "Outer Wilds", &[1, 2]),
                game(2, "Aseprite", &[3]),
                game(3, "Portal 2", &[2]),
                game(4, "Hades", &[4]),
            ],
            names
                .iter()
                .enumerate()
                .map(|(i, &name)| {
                    (
                        TagId(i as u32 + 1),
                        TagName {
                            english: name.to_string(),
                            italian: name.to_string(),
                        },
                    )
                })
                .collect(),
            (1..=5).map(|id| (TagId(id), 1.0)).collect(),
            Freshness {
                prices_updated_at: Timestamp(10),
                tags_updated_at: Timestamp(20),
                region: Region::ITALY,
            },
            Currency::italian(),
        )
    }

    fn signals() -> Signals {
        Signals {
            read_at: Timestamp(0),
            library: vec![
                LibraryGame {
                    id: AppId(1),
                    total_hours: 50.0,
                },
                LibraryGame {
                    id: AppId(2),
                    total_hours: 30.0,
                },
            ],
            recent: Vec::new(),
            wishlist: vec![AppId(3)],
            followed: Vec::new(),
        }
    }

    fn vote(kind: VoteKind) -> Vote {
        Vote {
            kind,
            date: "2026-10-01".to_string(),
            bet: false,
        }
    }

    fn votes() -> Votes {
        Votes::from([
            (AppId(4), vote(VoteKind::WouldTry)),
            (AppId(3), vote(VoteKind::NotForMe)),
            (AppId(5), vote(VoteKind::Maybe)),
            (AppId(6), vote(VoteKind::Known)),
        ])
    }

    fn view_with(signals: &Signals, votes: &Votes) -> ProfileView {
        let catalog = catalog();
        let corrections = Corrections {
            game_multipliers: BTreeMap::from([(AppId(2), 0.5), (AppId(99), 2.0)]),
            tag_multipliers: BTreeMap::from([(TagId(4), 0.5)]),
            banned_tags: BTreeMap::from([(TagId(5), Ban::TopTags(8))]),
            hidden_games: BTreeSet::from([AppId(3)]),
        };
        let families = Families {
            families: vec![
                Family {
                    name: "misteri".to_string(),
                    games: vec![AppId(1), AppId(3)],
                },
                Family {
                    name: "vuota".to_string(),
                    games: Vec::new(),
                },
            ],
            held_out: vec![AppId(2)],
        };

        profile_view(&SearchInputs {
            catalog: &catalog,
            game_vectors: &[],
            signals,
            votes,
            corrections: &corrections,
            families: &families,
            off_catalog: &OffCatalogGames::new(),
            config: &Config::default(),
        })
    }

    fn view() -> ProfileView {
        view_with(&signals(), &votes())
    }

    fn tag<'a>(taste: &'a TasteView, name: &str) -> &'a TagView {
        taste.tags.iter().find(|tag| tag.name == name).unwrap()
    }

    #[test]
    fn the_taste_names_the_games_that_shape_it() {
        let taste = view().taste.unwrap();

        assert_eq!(taste.game_count, 4);
        assert_eq!(taste.shaping_games[0].name, "Outer Wilds");
        assert_eq!(taste.shaping_games[0].hours, 50.0);
        assert_eq!(taste.shaping_games[0].source, HoursSource::Played);
        let portal = taste
            .shaping_games
            .iter()
            .find(|game| game.name == "Portal 2")
            .unwrap();
        assert_eq!((portal.hours, portal.source), (5.0, HoursSource::Wishlist));
        assert_eq!(taste.sources.len(), 4);
        assert_eq!(taste.sources[0].0, HoursSource::Played);
    }

    #[test]
    fn a_tag_carried_by_one_game_is_dominant() {
        let taste = view().taste.unwrap();

        let pixel = tag(&taste, "Pixel Graphics");
        assert!(pixel.dominant);
        assert_eq!(pixel.game_count, 1);
        assert_eq!(pixel.games[0].name, "Aseprite");
        assert_eq!(pixel.games[0].hours, 30.0);
        assert!(!tag(&taste, "Puzzle").dominant);
        assert_eq!(tag(&taste, "Puzzle").game_count, 2);
    }

    #[test]
    fn a_tag_multiplier_keeps_the_weight_before() {
        let taste = view().taste.unwrap();

        let roguelike = tag(&taste, "Roguelike");
        assert_eq!(roguelike.multiplier, Some(0.5));
        assert_eq!(roguelike.weight, roguelike.weight_before * 0.5);
        assert_eq!(tag(&taste, "Puzzle").multiplier, None);
    }

    #[test]
    fn the_families_leave_out_those_with_problems() {
        let view = view();

        assert_eq!(view.families.len(), 1);
        assert_eq!(view.families[0].name, "misteri");
        assert_eq!(view.families[0].games, [AppId(1), AppId(3)]);
        assert_eq!(view.families[0].tags[0], "Puzzle");
        assert_eq!(view.held_out, 1);
        assert_eq!(view.family_problems, ["family \"vuota\" has no games"]);
    }

    #[test]
    fn the_votes_count_every_kind_and_the_labels_of_the_opinion() {
        assert_eq!(
            view().votes,
            VotesView {
                would_try: 1,
                maybe: 1,
                not_for_me: 1,
                known: 1,
                labels: Labels {
                    would_try: 1,
                    not_for_me: 1
                },
                min_labels: 10,
            }
        );
    }

    #[test]
    fn the_corrections_come_by_name() {
        let corrections = view().corrections;

        assert_eq!(
            corrections.tag_multipliers,
            [("Roguelike".to_string(), 0.5)]
        );
        assert_eq!(
            corrections.banned_tags,
            [("JRPG".to_string(), Ban::TopTags(8))]
        );
        assert_eq!(
            corrections.game_multipliers,
            [
                CorrectedGameView {
                    name: "#99".to_string(),
                    multiplier: 2.0,
                    in_effect: false,
                },
                CorrectedGameView {
                    name: "Aseprite".to_string(),
                    multiplier: 0.5,
                    in_effect: true,
                },
            ]
        );
        assert_eq!(corrections.hidden_games, ["Portal 2"]);
        assert!(!corrections.is_empty());
        assert!(CorrectionsView::default().is_empty());
    }

    #[test]
    fn without_a_taste_the_rest_is_still_there() {
        let signals = Signals {
            library: Vec::new(),
            wishlist: Vec::new(),
            ..signals()
        };

        let view = view_with(&signals, &Votes::new());

        assert_eq!(view.taste, None);
        assert_eq!(view.families.len(), 1);
        assert_eq!(view.catalog_games, 4);
        assert_eq!(view.prices_updated_at, Timestamp(10));
        assert_eq!(view.tags_updated_at, Timestamp(20));
        assert_eq!(view.hours.min_hours, 2.0);
    }
}
